use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Form, Path, Query, State,
    },
    http::{header, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Json, Router,
};
use dioxus::prelude::*;
use futures_util::{SinkExt, StreamExt};
use nanoid::nanoid;
use queensgame_shared::{
    append_mouse_recording, append_recording_frame, mouse_recording_times_are_sorted,
    normalize_display_name, recording_frame_is_valid, validate_solution, CellState,
    CreateRoomResponse, GameBootstrap, Puzzle, PuzzleFile, PuzzleNav, RoomBootstrap,
    RoomClientMessage, RoomMedalCounts, RoomMouseRecording, RoomPhase, RoomPlayerSnapshot,
    RoomPuzzleChoice, RoomRecording, RoomRecordingFrame, RoomServerMessage, RoomSnapshot,
    ValidateRequest, ValidateResponse, DISPLAY_NAME_MAX_CHARS, ROOM_MOUSE_EVENT_ENTER,
    ROOM_MOUSE_EVENT_LEAVE, ROOM_MOUSE_EVENT_PRIMARY_DOWN, ROOM_MOUSE_EVENT_PRIMARY_UP,
    ROOM_MOUSE_EVENT_SECONDARY_DOWN, ROOM_MOUSE_EVENT_SECONDARY_UP,
};
use rand::Rng;
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    net::SocketAddr,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{broadcast, Mutex};
use tower_http::{services::ServeDir, trace::TraceLayer};

const PUZZLE_DATA: &str = include_str!("../../../data/9x9-puzzles.json");
const STYLE_CSS: &str = include_str!("../../../static/style.css");
const QUEEN_SVG: &str = include_str!("../../../static/queen.svg");
const MAX_RECORDING_FRAMES: usize = 10_000;
const MAX_MOUSE_SAMPLES: usize = 100_000;
const MAX_MOUSE_EVENTS: usize = 100_000;

#[derive(Clone)]
struct AppState {
    puzzles: Arc<Vec<Puzzle>>,
    rooms: Arc<Mutex<BTreeMap<String, Room>>>,
}

struct Room {
    slug: String,
    puzzle_choice: RoomPuzzleChoice,
    active_puzzle_id: Option<usize>,
    played_puzzle_ids: BTreeSet<usize>,
    players: BTreeMap<String, RoomPlayer>,
    race_player_ids: Vec<String>,
    phase: ServerRoomPhase,
    tx: broadcast::Sender<String>,
}

struct RoomPlayer {
    id: String,
    name: String,
    ready: bool,
    connected: bool,
    finish_ms: Option<u64>,
    gave_up: bool,
    medals: RoomMedalCounts,
    recording: Option<RoomRecording>,
    mouse_recording: Option<RoomMouseRecording>,
    joined_order: u64,
}

enum ServerRoomPhase {
    Lobby,
    Countdown {
        starts_at_ms: u64,
    },
    Racing {
        started_at_ms: u64,
        started_at: Instant,
    },
    Complete {
        started_at_ms: u64,
    },
}

#[derive(Debug, Deserialize)]
struct JoinParams {
    player_id: String,
    name: String,
}

#[derive(Debug, Deserialize)]
struct CreateRoomForm {
    display_name: String,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "queensgame=info,tower_http=info".into()),
        )
        .init();

    let state = AppState {
        puzzles: Arc::new(load_puzzles()),
        rooms: Arc::new(Mutex::new(BTreeMap::new())),
    };
    let client_dist = client_dist_dir();

    let app = Router::new()
        .route("/", get(|| async { Redirect::temporary("/puzzles/9x9/1") }))
        .route("/puzzles", get(puzzles_index))
        .route("/puzzles/9x9", get(puzzles_index))
        .route("/puzzles/9x9/:id", get(puzzle_page))
        .route("/rooms", get(rooms_index).post(create_room_form))
        .route("/rooms/:slug", get(room_page))
        .route("/api/rooms", post(create_room_api))
        .route("/api/puzzles/9x9/:id", get(puzzle_api))
        .route("/api/validate", post(validate_api))
        .route("/ws/rooms/:slug", get(room_ws))
        .route("/static/style.css", get(static_css))
        .route("/static/queen.svg", get(static_queen_svg))
        .nest_service("/static/client", ServeDir::new(client_dist))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let addr = bind_addr();
    tracing::info!("listening on http://{addr}");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind HTTP listener");
    axum::serve(listener, app)
        .await
        .expect("HTTP server failed");
}

fn bind_addr() -> SocketAddr {
    std::env::var("QUEENSGAME_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:3000".to_string())
        .parse()
        .expect(
            "QUEENSGAME_ADDR must be a valid socket address, like 127.0.0.1:3000 or 0.0.0.0:3000",
        )
}

fn client_dist_dir() -> PathBuf {
    std::env::var_os("QUEENSGAME_CLIENT_DIST")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("dist/client"))
}

fn load_puzzles() -> Vec<Puzzle> {
    let data: PuzzleFile = serde_json::from_str(PUZZLE_DATA)
        .expect("data/9x9-puzzles.json must contain valid puzzle data");
    assert!(!data.puzzles.is_empty(), "puzzle data must not be empty");
    data.puzzles
}

async fn puzzles_index(State(state): State<AppState>) -> Result<Html<String>, AppError> {
    Ok(Html(render_puzzles_page(
        puzzle_nav(&state.puzzles, 0),
        state.puzzles.len(),
    )))
}

async fn puzzle_page(
    State(state): State<AppState>,
    Path(id): Path<usize>,
) -> Result<Html<String>, AppError> {
    let puzzle = find_puzzle(&state, id)?.clone();
    let bootstrap = GameBootstrap {
        puzzle: puzzle.clone(),
        puzzle_nav: puzzle_nav(&state.puzzles, id),
        total: state.puzzles.len(),
    };
    let bootstrap_json = serde_json::to_string(&bootstrap)?;

    Ok(Html(render_puzzle_page(&puzzle, bootstrap_json)))
}

async fn rooms_index() -> Html<String> {
    Html(render_rooms_page())
}

async fn create_room_form(
    State(state): State<AppState>,
    Form(form): Form<CreateRoomForm>,
) -> Result<Redirect, AppError> {
    let Some(display_name) = normalize_display_name(&form.display_name) else {
        return Err(AppError::BadRequest("Enter a display name.".to_string()));
    };
    let room = create_room(&state).await;
    Ok(Redirect::to(&format!(
        "/rooms/{}?name={}",
        room.slug,
        urlencoding::encode(&display_name)
    )))
}

async fn create_room_api(
    State(state): State<AppState>,
) -> Result<Json<CreateRoomResponse>, AppError> {
    let room = create_room(&state).await;
    Ok(Json(CreateRoomResponse {
        path: format!("/rooms/{}", room.slug),
        slug: room.slug,
    }))
}

async fn room_page(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Html<String>, AppError> {
    let snapshot = {
        let rooms = state.rooms.lock().await;
        let room = rooms.get(&slug).ok_or(AppError::NotFound)?;
        snapshot_room(room, &state.puzzles)
    };
    let bootstrap = RoomBootstrap {
        slug: slug.clone(),
        total_puzzles: state.puzzles.len(),
        snapshot,
    };
    let bootstrap_json = serde_json::to_string(&bootstrap)?;

    Ok(Html(render_room_page(&slug, bootstrap_json)))
}

async fn puzzle_api(
    State(state): State<AppState>,
    Path(id): Path<usize>,
) -> Result<Json<Puzzle>, AppError> {
    Ok(Json(find_puzzle(&state, id)?.clone()))
}

async fn validate_api(
    State(state): State<AppState>,
    Json(request): Json<ValidateRequest>,
) -> Result<Json<ValidateResponse>, AppError> {
    let puzzle = find_puzzle(&state, request.id)?;
    Ok(Json(validate_solution(puzzle, &request.queens)))
}

async fn room_ws(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(params): Query<JoinParams>,
    ws: WebSocketUpgrade,
) -> Result<Response, AppError> {
    if params.player_id.trim().is_empty() {
        return Err(AppError::BadRequest("Missing player id".to_string()));
    }
    if normalize_display_name(&params.name).is_none() {
        return Err(AppError::BadRequest("Missing display name".to_string()));
    }

    {
        let rooms = state.rooms.lock().await;
        if !rooms.contains_key(&slug) {
            return Err(AppError::NotFound);
        }
    }

    Ok(ws
        .on_upgrade(move |socket| handle_room_socket(socket, state, slug, params))
        .into_response())
}

async fn static_css() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        STYLE_CSS,
    )
}

async fn static_queen_svg() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "image/svg+xml; charset=utf-8")],
        QUEEN_SVG,
    )
}

async fn create_room(state: &AppState) -> CreateRoomResponse {
    let mut rooms = state.rooms.lock().await;
    let slug = loop {
        let candidate = nanoid!(8, &nanoid::alphabet::SAFE);
        if !rooms.contains_key(&candidate) {
            break candidate;
        }
    };
    let (tx, _) = broadcast::channel(64);
    rooms.insert(
        slug.clone(),
        Room {
            slug: slug.clone(),
            puzzle_choice: RoomPuzzleChoice::Random,
            active_puzzle_id: None,
            played_puzzle_ids: BTreeSet::new(),
            players: BTreeMap::new(),
            race_player_ids: Vec::new(),
            phase: ServerRoomPhase::Lobby,
            tx,
        },
    );

    CreateRoomResponse {
        path: format!("/rooms/{slug}"),
        slug,
    }
}

async fn handle_room_socket(socket: WebSocket, state: AppState, slug: String, params: JoinParams) {
    let player_id = params.player_id;
    let Some(player_name) = normalize_display_name(&params.name) else {
        return;
    };

    let Some((initial_snapshot, mut room_rx)) =
        join_room(&state, &slug, &player_id, player_name).await
    else {
        return;
    };

    let (mut sender, mut receiver) = socket.split();
    if sender.send(Message::Text(initial_snapshot)).await.is_err() {
        disconnect_player(&state, &slug, &player_id).await;
        return;
    }

    let send_task = tokio::spawn(async move {
        while let Ok(message) = room_rx.recv().await {
            if sender.send(Message::Text(message)).await.is_err() {
                break;
            }
        }
    });

    while let Some(Ok(message)) = receiver.next().await {
        match message {
            Message::Text(raw) => match serde_json::from_str::<RoomClientMessage>(&raw) {
                Ok(message) => handle_room_message(&state, &slug, &player_id, message).await,
                Err(error) => {
                    send_room_error(&state, &slug, format!("Invalid room message: {error}")).await
                }
            },
            Message::Close(_) => break,
            Message::Ping(_) | Message::Pong(_) | Message::Binary(_) => {}
        }
    }

    send_task.abort();
    disconnect_player(&state, &slug, &player_id).await;
}

async fn join_room(
    state: &AppState,
    slug: &str,
    player_id: &str,
    player_name: String,
) -> Option<(String, broadcast::Receiver<String>)> {
    let mut rooms = state.rooms.lock().await;
    let room = rooms.get_mut(slug)?;
    let joined_order = room.players.len() as u64 + 1;
    let reset_ready = matches!(room.phase, ServerRoomPhase::Lobby);
    room.players
        .entry(player_id.to_string())
        .and_modify(|player| {
            player.name = player_name.clone();
            player.connected = true;
            if reset_ready {
                player.ready = false;
            }
        })
        .or_insert_with(|| RoomPlayer {
            id: player_id.to_string(),
            name: player_name,
            ready: false,
            connected: true,
            finish_ms: None,
            gave_up: false,
            medals: RoomMedalCounts::default(),
            recording: None,
            mouse_recording: None,
            joined_order,
        });
    let initial_snapshot = room_snapshot_message(room, &state.puzzles);
    let _ = room.tx.send(initial_snapshot.clone());
    let rx = room.tx.subscribe();

    Some((initial_snapshot, rx))
}

async fn disconnect_player(state: &AppState, slug: &str, player_id: &str) {
    let mut rooms = state.rooms.lock().await;
    let Some(room) = rooms.get_mut(slug) else {
        return;
    };
    if let Some(player) = room.players.get_mut(player_id) {
        player.connected = false;
        if matches!(room.phase, ServerRoomPhase::Lobby) {
            player.ready = false;
        }
    }
    broadcast_room(room, &state.puzzles);
}

async fn handle_room_message(
    state: &AppState,
    slug: &str,
    player_id: &str,
    message: RoomClientMessage,
) {
    match message {
        RoomClientMessage::SelectPuzzle { puzzle_id } => {
            select_room_puzzle(state, slug, puzzle_id).await;
        }
        RoomClientMessage::SelectRandom => {
            select_random_puzzle(state, slug).await;
        }
        RoomClientMessage::SetReady { ready } => {
            set_player_ready(state, slug, player_id, ready).await;
        }
        RoomClientMessage::Finish { queens, recording } => {
            finish_player(state, slug, player_id, queens, recording).await;
        }
        RoomClientMessage::GiveUp => {
            give_up_player(state, slug, player_id).await;
        }
        RoomClientMessage::RecordingFrame { frame } => {
            store_recording_frame(state, slug, player_id, frame).await;
        }
        RoomClientMessage::MouseRecordingChunk { recording } => {
            store_mouse_recording_chunk(state, slug, player_id, recording).await;
        }
        RoomClientMessage::MouseRecording { recording } => {
            store_mouse_recording(state, slug, player_id, recording).await;
        }
    }
}

async fn select_room_puzzle(state: &AppState, slug: &str, puzzle_id: usize) {
    if find_puzzle_by_id(&state.puzzles, puzzle_id).is_none() {
        send_room_error(state, slug, format!("Puzzle {puzzle_id} does not exist.")).await;
        return;
    }

    let mut rooms = state.rooms.lock().await;
    let Some(room) = rooms.get_mut(slug) else {
        return;
    };
    if !room_accepts_next_race_setup(room) {
        return;
    }
    room.puzzle_choice = RoomPuzzleChoice::Puzzle { id: puzzle_id };
    reset_room_setup_for_selection(room);
    broadcast_room(room, &state.puzzles);
}

async fn select_random_puzzle(state: &AppState, slug: &str) {
    let mut rooms = state.rooms.lock().await;
    let Some(room) = rooms.get_mut(slug) else {
        return;
    };
    if !room_accepts_next_race_setup(room) {
        return;
    }
    room.puzzle_choice = RoomPuzzleChoice::Random;
    reset_room_setup_for_selection(room);
    broadcast_room(room, &state.puzzles);
}

async fn set_player_ready(state: &AppState, slug: &str, player_id: &str, ready: bool) {
    let mut starts_at_ms = None;
    {
        let mut rooms = state.rooms.lock().await;
        let Some(room) = rooms.get_mut(slug) else {
            return;
        };
        if !room_accepts_next_race_setup(room) {
            return;
        }
        if let Some(player) = room.players.get_mut(player_id) {
            player.ready = ready;
        }

        if room_all_connected_players_ready(room) {
            let start = now_ms() + 3_000;
            clear_room_race_results(room);
            room.phase = ServerRoomPhase::Countdown {
                starts_at_ms: start,
            };
            starts_at_ms = Some(start);
        }
        broadcast_room(room, &state.puzzles);
    }

    if let Some(starts_at_ms) = starts_at_ms {
        schedule_room_start(state.clone(), slug.to_string(), starts_at_ms);
    }
}

fn schedule_room_start(state: AppState, slug: String, starts_at_ms: u64) {
    tokio::spawn(async move {
        let delay_ms = starts_at_ms.saturating_sub(now_ms());
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;

        let mut rooms = state.rooms.lock().await;
        let Some(room) = rooms.get_mut(&slug) else {
            return;
        };
        if !matches!(
            room.phase,
            ServerRoomPhase::Countdown {
                starts_at_ms: active_start
            } if active_start == starts_at_ms
        ) {
            return;
        }

        let puzzle_id = match room.puzzle_choice {
            RoomPuzzleChoice::Puzzle { id } => id,
            RoomPuzzleChoice::Random => {
                let Some(id) = random_room_puzzle_id(&state.puzzles, &room.played_puzzle_ids)
                else {
                    return;
                };
                id
            }
        };
        room.active_puzzle_id = Some(puzzle_id);

        room.race_player_ids = room
            .players
            .values()
            .filter(|player| player.connected)
            .map(|player| player.id.clone())
            .collect();
        for player in room.players.values_mut() {
            player.ready = false;
            player.finish_ms = None;
            player.gave_up = false;
            player.recording = None;
            player.mouse_recording = None;
        }

        room.phase = ServerRoomPhase::Racing {
            started_at_ms: now_ms(),
            started_at: Instant::now(),
        };
        broadcast_room(room, &state.puzzles);
    });
}

async fn store_recording_frame(
    state: &AppState,
    slug: &str,
    player_id: &str,
    frame: RoomRecordingFrame,
) {
    let mut rooms = state.rooms.lock().await;
    let Some(room) = rooms.get_mut(slug) else {
        return;
    };
    if !matches!(room.phase, ServerRoomPhase::Racing { .. }) {
        return;
    }
    if !room.race_player_ids.iter().any(|id| id == player_id) {
        return;
    }
    let Some(puzzle_id) = room.active_puzzle_id else {
        return;
    };
    let Some(puzzle) = find_puzzle_by_id(&state.puzzles, puzzle_id) else {
        return;
    };
    let cell_count = puzzle.size.saturating_mul(puzzle.size);
    if !recording_frame_is_valid(&frame, cell_count) {
        return;
    }

    let Some(player) = room.players.get_mut(player_id) else {
        return;
    };
    if player.finish_ms.is_some() || player.gave_up {
        return;
    }
    let recording = player
        .recording
        .get_or_insert_with(|| RoomRecording { frames: Vec::new() });
    if recording.frames.len() >= MAX_RECORDING_FRAMES {
        return;
    }
    let broadcast_frame = frame.clone();
    if !append_recording_frame(recording, frame) {
        return;
    }

    let _ = room.tx.send(
        serde_json::to_string(&RoomServerMessage::RecordingFrame {
            player_id: player_id.to_string(),
            frame: broadcast_frame,
        })
        .expect("room recording frame must be serializable"),
    );
}

async fn finish_player(
    state: &AppState,
    slug: &str,
    player_id: &str,
    queens: Vec<[usize; 2]>,
    recording: RoomRecording,
) {
    let mut rooms = state.rooms.lock().await;
    let Some(room) = rooms.get_mut(slug) else {
        return;
    };
    let (started_at_ms, elapsed_ms) = match &room.phase {
        ServerRoomPhase::Racing {
            started_at_ms,
            started_at,
        } => (*started_at_ms, started_at.elapsed().as_millis() as u64),
        _ => return,
    };
    let Some(puzzle_id) = room.active_puzzle_id else {
        return;
    };
    let Some(puzzle) = find_puzzle_by_id(&state.puzzles, puzzle_id) else {
        return;
    };
    if !validate_solution(puzzle, &queens).complete {
        send_room_error_locked(
            room,
            &format!("Submitted solution for puzzle {puzzle_id} is not complete."),
        );
        return;
    }
    if !recording_matches_solution(puzzle, &queens, &recording) {
        send_room_error_locked(room, "Submitted replay does not match the finished board.");
        return;
    }

    if let Some(player) = room.players.get_mut(player_id) {
        if player.finish_ms.is_none() && !player.gave_up {
            player.finish_ms = Some(elapsed_ms);
            player.recording = Some(recording);
        }
    }

    if room_all_racers_done(room) {
        complete_room_race(room, &state.puzzles, started_at_ms);
    }
    broadcast_room(room, &state.puzzles);
}

async fn give_up_player(state: &AppState, slug: &str, player_id: &str) {
    let mut rooms = state.rooms.lock().await;
    let Some(room) = rooms.get_mut(slug) else {
        return;
    };
    let started_at_ms = match &room.phase {
        ServerRoomPhase::Racing { started_at_ms, .. } => *started_at_ms,
        ServerRoomPhase::Lobby
        | ServerRoomPhase::Countdown { .. }
        | ServerRoomPhase::Complete { .. } => return,
    };
    if !room.race_player_ids.iter().any(|id| id == player_id) {
        return;
    }

    if let Some(player) = room.players.get_mut(player_id) {
        if player.finish_ms.is_none() {
            player.gave_up = true;
        }
    }

    if room_all_racers_done(room) {
        complete_room_race(room, &state.puzzles, started_at_ms);
    }
    broadcast_room(room, &state.puzzles);
}

async fn store_mouse_recording_chunk(
    state: &AppState,
    slug: &str,
    player_id: &str,
    recording: RoomMouseRecording,
) {
    if recording.samples.is_empty() && recording.events.is_empty() {
        return;
    }

    let mut rooms = state.rooms.lock().await;
    let Some(room) = rooms.get_mut(slug) else {
        return;
    };
    if !matches!(room.phase, ServerRoomPhase::Racing { .. }) {
        return;
    }
    if !room.race_player_ids.iter().any(|id| id == player_id) {
        return;
    }
    let Some(puzzle_id) = room.active_puzzle_id else {
        return;
    };
    let Some(puzzle) = find_puzzle_by_id(&state.puzzles, puzzle_id) else {
        return;
    };
    if !mouse_recording_is_valid(puzzle, &recording) {
        return;
    }

    let Some(player) = room.players.get_mut(player_id) else {
        return;
    };
    if player.finish_ms.is_some() || player.gave_up {
        return;
    }
    let existing = player
        .mouse_recording
        .get_or_insert_with(|| RoomMouseRecording {
            samples: Vec::new(),
            events: Vec::new(),
        });
    if existing
        .samples
        .len()
        .saturating_add(recording.samples.len())
        > MAX_MOUSE_SAMPLES
        || existing.events.len().saturating_add(recording.events.len()) > MAX_MOUSE_EVENTS
    {
        return;
    }

    let broadcast_recording = recording.clone();
    if !append_mouse_recording(existing, recording) {
        return;
    }

    let _ = room.tx.send(
        serde_json::to_string(&RoomServerMessage::MouseRecordingChunk {
            player_id: player_id.to_string(),
            recording: broadcast_recording,
        })
        .expect("room mouse recording chunk must be serializable"),
    );
}

async fn store_mouse_recording(
    state: &AppState,
    slug: &str,
    player_id: &str,
    recording: RoomMouseRecording,
) {
    let mut rooms = state.rooms.lock().await;
    let Some(room) = rooms.get_mut(slug) else {
        return;
    };
    let Some(puzzle_id) = room.active_puzzle_id else {
        return;
    };
    let Some(puzzle) = find_puzzle_by_id(&state.puzzles, puzzle_id) else {
        return;
    };
    if !mouse_recording_is_valid(puzzle, &recording) {
        send_room_error_locked(room, "Submitted mouse replay data is invalid.");
        return;
    }
    let Some(player) = room.players.get_mut(player_id) else {
        return;
    };
    if player.finish_ms.is_none() {
        return;
    }
    player.mouse_recording = Some(recording);
    broadcast_room(room, &state.puzzles);
}

async fn send_room_error(state: &AppState, slug: &str, message: String) {
    let rooms = state.rooms.lock().await;
    let Some(room) = rooms.get(slug) else {
        return;
    };
    send_room_error_locked(room, &message);
}

fn recording_matches_solution(
    puzzle: &Puzzle,
    queens: &[[usize; 2]],
    recording: &RoomRecording,
) -> bool {
    let Some(last_frame) = recording.frames.last() else {
        return false;
    };
    let cell_count = puzzle.size * puzzle.size;
    if recording
        .frames
        .iter()
        .any(|frame| !recording_frame_is_valid(frame, cell_count))
    {
        return false;
    }
    if recording
        .frames
        .windows(2)
        .any(|frames| frames[0].elapsed_ms > frames[1].elapsed_ms)
    {
        return false;
    }

    let recorded_queens: Vec<[usize; 2]> = last_frame
        .states
        .iter()
        .enumerate()
        .filter_map(|(index, state)| {
            if CellState::from_storage_code(*state) == CellState::Queen {
                Some([index / puzzle.size, index % puzzle.size])
            } else {
                None
            }
        })
        .collect();
    let mut recorded_queens = recorded_queens;
    let mut submitted_queens = queens.to_vec();
    recorded_queens.sort_unstable();
    submitted_queens.sort_unstable();
    recorded_queens == submitted_queens
}

fn mouse_recording_is_valid(puzzle: &Puzzle, recording: &RoomMouseRecording) -> bool {
    if recording.samples.len() > MAX_MOUSE_SAMPLES || recording.events.len() > MAX_MOUSE_EVENTS {
        return false;
    }
    if !mouse_recording_times_are_sorted(recording) {
        return false;
    }

    let cell_count = puzzle.size.saturating_mul(puzzle.size);
    recording.events.iter().all(|event| {
        matches!(
            event.1,
            ROOM_MOUSE_EVENT_ENTER
                | ROOM_MOUSE_EVENT_LEAVE
                | ROOM_MOUSE_EVENT_PRIMARY_DOWN
                | ROOM_MOUSE_EVENT_PRIMARY_UP
                | ROOM_MOUSE_EVENT_SECONDARY_DOWN
                | ROOM_MOUSE_EVENT_SECONDARY_UP
        ) && event
            .4
            .map(|cell_index| usize::from(cell_index) < cell_count)
            .unwrap_or(true)
    })
}

fn room_accepts_next_race_setup(room: &Room) -> bool {
    matches!(
        room.phase,
        ServerRoomPhase::Lobby | ServerRoomPhase::Complete { .. }
    )
}

fn reset_room_setup_for_selection(room: &mut Room) {
    reset_room_ready_flags(room);
    if matches!(room.phase, ServerRoomPhase::Lobby) {
        clear_room_race_results(room);
    }
}

fn reset_room_ready_flags(room: &mut Room) {
    for player in room.players.values_mut() {
        player.ready = false;
    }
}

fn clear_room_race_results(room: &mut Room) {
    for player in room.players.values_mut() {
        player.finish_ms = None;
        player.gave_up = false;
        player.recording = None;
        player.mouse_recording = None;
    }
    room.race_player_ids.clear();
    room.active_puzzle_id = None;
}

fn room_all_connected_players_ready(room: &Room) -> bool {
    let mut connected_players = room
        .players
        .values()
        .filter(|player| player.connected)
        .peekable();
    if connected_players.peek().is_none() {
        return false;
    }
    connected_players.all(|player| player.ready)
}

fn room_all_racers_done(room: &Room) -> bool {
    !room.race_player_ids.is_empty()
        && room.race_player_ids.iter().all(|player_id| {
            room.players
                .get(player_id)
                .map(|player| player.finish_ms.is_some() || player.gave_up)
                .unwrap_or(false)
        })
}

fn complete_room_race(room: &mut Room, puzzles: &[Puzzle], started_at_ms: u64) {
    let completed_puzzle_id = room.active_puzzle_id;
    if let Some(puzzle_id) = completed_puzzle_id {
        room.played_puzzle_ids.insert(puzzle_id);
    }

    award_room_medals(room);

    if let (RoomPuzzleChoice::Puzzle { .. }, Some(puzzle_id)) =
        (&room.puzzle_choice, completed_puzzle_id)
    {
        if let Some(next_id) = next_puzzle_id(puzzles, puzzle_id) {
            room.puzzle_choice = RoomPuzzleChoice::Puzzle { id: next_id };
        }
    }

    room.phase = ServerRoomPhase::Complete { started_at_ms };
}

fn award_room_medals(room: &mut Room) {
    let mut placements = room
        .race_player_ids
        .iter()
        .filter_map(|player_id| {
            let player = room.players.get(player_id)?;
            player
                .finish_ms
                .map(|finish_ms| (player.id.clone(), finish_ms, player.joined_order))
        })
        .collect::<Vec<_>>();
    placements.sort_by_key(|(_, finish_ms, joined_order)| (*finish_ms, *joined_order));

    for (place, (player_id, _, _)) in placements.into_iter().take(3).enumerate() {
        let Some(player) = room.players.get_mut(&player_id) else {
            continue;
        };
        match place {
            0 => player.medals.gold += 1,
            1 => player.medals.silver += 1,
            2 => player.medals.bronze += 1,
            _ => {}
        }
    }
}

fn random_room_puzzle_id(puzzles: &[Puzzle], played_puzzle_ids: &BTreeSet<usize>) -> Option<usize> {
    let mut candidates = puzzles
        .iter()
        .filter(|puzzle| !played_puzzle_ids.contains(&puzzle.id))
        .map(|puzzle| puzzle.id)
        .collect::<Vec<_>>();

    if candidates.is_empty() {
        candidates = puzzles.iter().map(|puzzle| puzzle.id).collect();
    }
    if candidates.is_empty() {
        return None;
    }

    let index = rand::rng().random_range(0..candidates.len());
    candidates.get(index).copied()
}

fn next_puzzle_id(puzzles: &[Puzzle], current_id: usize) -> Option<usize> {
    puzzles
        .iter()
        .map(|puzzle| puzzle.id)
        .filter(|id| *id > current_id)
        .min()
        .or_else(|| puzzles.iter().map(|puzzle| puzzle.id).min())
}

fn broadcast_room(room: &Room, puzzles: &[Puzzle]) {
    let _ = room.tx.send(room_snapshot_message(room, puzzles));
}

fn room_snapshot_message(room: &Room, puzzles: &[Puzzle]) -> String {
    serde_json::to_string(&RoomServerMessage::Snapshot {
        snapshot: snapshot_room(room, puzzles),
    })
    .expect("room snapshot must be serializable")
}

fn send_room_error_locked(room: &Room, message: &str) {
    let _ = room.tx.send(
        serde_json::to_string(&RoomServerMessage::Error {
            message: message.to_string(),
        })
        .expect("room error must be serializable"),
    );
}

fn snapshot_room(room: &Room, puzzles: &[Puzzle]) -> RoomSnapshot {
    let puzzle = match room.phase {
        ServerRoomPhase::Racing { .. } | ServerRoomPhase::Complete { .. } => room
            .active_puzzle_id
            .and_then(|id| find_puzzle_by_id(puzzles, id).cloned()),
        ServerRoomPhase::Lobby | ServerRoomPhase::Countdown { .. } => None,
    };
    let winner_id = if matches!(room.phase, ServerRoomPhase::Complete { .. }) {
        room.players
            .values()
            .filter_map(|player| player.finish_ms.map(|finish_ms| (player, finish_ms)))
            .min_by_key(|(player, finish_ms)| (*finish_ms, player.joined_order))
            .map(|(player, _)| player.id.clone())
    } else {
        None
    };

    RoomSnapshot {
        slug: room.slug.clone(),
        phase: room.phase.as_snapshot_phase(),
        puzzle_choice: room.puzzle_choice.clone(),
        played_puzzle_ids: room.played_puzzle_ids.iter().copied().collect(),
        players: room
            .players
            .values()
            .map(|player| RoomPlayerSnapshot {
                id: player.id.clone(),
                name: player.name.clone(),
                ready: player.ready,
                connected: player.connected,
                finish_ms: player.finish_ms,
                gave_up: player.gave_up,
                medals: player.medals,
                recording: player.recording.clone(),
                mouse_recording: player.mouse_recording.clone(),
            })
            .collect(),
        puzzle,
        winner_id,
    }
}

impl ServerRoomPhase {
    fn as_snapshot_phase(&self) -> RoomPhase {
        match self {
            Self::Lobby => RoomPhase::Lobby,
            Self::Countdown { starts_at_ms } => RoomPhase::Countdown {
                starts_at_ms: *starts_at_ms,
            },
            Self::Racing { started_at_ms, .. } => RoomPhase::Racing {
                started_at_ms: *started_at_ms,
            },
            Self::Complete { started_at_ms } => RoomPhase::Complete {
                started_at_ms: *started_at_ms,
            },
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn find_puzzle(state: &AppState, id: usize) -> Result<&Puzzle, AppError> {
    find_puzzle_by_id(&state.puzzles, id).ok_or(AppError::NotFound)
}

fn find_puzzle_by_id(puzzles: &[Puzzle], id: usize) -> Option<&Puzzle> {
    puzzles.iter().find(|puzzle| puzzle.id == id)
}

fn puzzle_nav(puzzles: &[Puzzle], active_id: usize) -> Vec<PuzzleNav> {
    puzzles
        .iter()
        .map(|puzzle| PuzzleNav {
            id: puzzle.id,
            active: puzzle.id == active_id,
        })
        .collect()
}

fn render_document(title: &str, description: &str, client: bool, content: Element) -> String {
    let content = dioxus_ssr::render_element(content);
    let client_script = if client {
        r#"<script type="module">import init from "/static/client/queensgame_client.js"; init();</script>"#
    } else {
        ""
    };

    format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>{title}</title><meta name="description" content="{description}"><link rel="stylesheet" href="/static/style.css"></head><body>{content}{client_script}</body></html>"#
    )
}

fn render_puzzles_page(puzzle_nav: Vec<PuzzleNav>, total: usize) -> String {
    render_document(
        "9x9 Queens Puzzles",
        "Choose from 300 bundled 9x9 Queens puzzles.",
        false,
        rsx! {
            header { class: "site-header",
                a { class: "brand", href: "/puzzles/9x9/1", aria_label: "Queens Game home",
                    span { class: "brand-mark", "Q" }
                    span { class: "brand-name", "Queens Game" }
                }
                nav { class: "top-nav", aria_label: "Primary",
                    a { href: "/puzzles/9x9", "Puzzles" }
                    a { href: "/rooms", "Rooms" }
                }
            }
            main { class: "archive-page",
                section { class: "archive-hero",
                    p { class: "eyebrow", "Puzzle set" }
                    h1 { "9x9 Queens Puzzles" }
                    p { "{total} advanced boards with one queen per row, column, and colored region." }
                    a { class: "nav-button primary", href: "/puzzles/9x9/1", "Start Puzzle #1" }
                }
                section { class: "archive-list", aria_labelledby: "archive-title",
                    div { class: "selector-header",
                        p { class: "eyebrow", "Archive" }
                        h2 { id: "archive-title", "Select a puzzle" }
                    }
                    div { class: "puzzle-grid wide",
                        for nav in puzzle_nav {
                            a { href: "/puzzles/9x9/{nav.id}", "{nav.id}" }
                        }
                    }
                }
            }
        },
    )
}

fn render_puzzle_page(puzzle: &Puzzle, bootstrap_json: String) -> String {
    let title = format!("9x9 Queens Puzzle #{}", puzzle.id);

    render_document(
        &title,
        "Place one queen in every row, column, and colored region without diagonal touching.",
        true,
        rsx! {
            header { class: "site-header",
                a { class: "brand", href: "/puzzles/9x9/1", aria_label: "Queens Game home",
                    span { class: "brand-mark", "Q" }
                    span { class: "brand-name", "Queens Game" }
                }
                nav { class: "top-nav", aria_label: "Primary",
                    a { href: "/puzzles/9x9", "Puzzles" }
                    a { href: "/rooms", "Rooms" }
                    a { href: "/puzzles/9x9/{puzzle.id}", "9x9" }
                }
            }
            div { id: "game-root" }
            script { r#type: "application/json", id: "game-data", dangerous_inner_html: "{bootstrap_json}" }
        },
    )
}

fn render_rooms_page() -> String {
    render_document(
        "Queens Game Rooms",
        "Create a multiplayer Queens game room.",
        false,
        rsx! {
            header { class: "site-header",
                a { class: "brand", href: "/puzzles/9x9/1", aria_label: "Queens Game home",
                    span { class: "brand-mark", "Q" }
                    span { class: "brand-name", "Queens Game" }
                }
                nav { class: "top-nav", aria_label: "Primary",
                    a { href: "/puzzles/9x9", "Puzzles" }
                    a { href: "/rooms", "Rooms" }
                }
            }
            main { class: "archive-page",
                section { class: "archive-hero",
                    p { class: "eyebrow", "Multiplayer" }
                    h1 { "Game Rooms" }
                    p { "Create a room, send the link to other players, ready up, and race the same puzzle together." }
                    form { class: "display-name-form", method: "post", action: "/rooms",
                        label { r#for: "create-display-name", "Display name" }
                        input {
                            id: "create-display-name",
                            name: "display_name",
                            r#type: "text",
                            autocomplete: "nickname",
                            maxlength: "{DISPLAY_NAME_MAX_CHARS}",
                            required: true,
                            placeholder: "Your name"
                        }
                        button { r#type: "submit", class: "nav-button primary", "Create Room" }
                    }
                }
            }
        },
    )
}

fn render_room_page(slug: &str, bootstrap_json: String) -> String {
    let title = format!("Queens Room {slug}");

    render_document(
        &title,
        "Join a multiplayer Queens race room.",
        true,
        rsx! {
            header { class: "site-header",
                a { class: "brand", href: "/puzzles/9x9/1", aria_label: "Queens Game home",
                    span { class: "brand-mark", "Q" }
                    span { class: "brand-name", "Queens Game" }
                }
                nav { class: "top-nav", aria_label: "Primary",
                    a { href: "/puzzles/9x9", "Puzzles" }
                    a { href: "/rooms", "Rooms" }
                    a { href: "/rooms/{slug}", "Room" }
                }
            }
            div { id: "game-root" }
            script { r#type: "application/json", id: "room-data", dangerous_inner_html: "{bootstrap_json}" }
        },
    )
}

#[derive(Debug)]
enum AppError {
    NotFound,
    BadRequest(String),
    Json(serde_json::Error),
}

impl From<serde_json::Error> for AppError {
    fn from(error: serde_json::Error) -> Self {
        AppError::Json(error)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match self {
            AppError::NotFound => (StatusCode::NOT_FOUND, "Not found").into_response(),
            AppError::BadRequest(message) => (StatusCode::BAD_REQUEST, message).into_response(),
            AppError::Json(error) => {
                tracing::error!(%error, "JSON handling failed");
                (StatusCode::INTERNAL_SERVER_ERROR, "JSON handling failed").into_response()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_puzzle(id: usize) -> Puzzle {
        Puzzle {
            id,
            size: 1,
            colors: vec!["#ffffff".to_string()],
            regions: vec![vec![0]],
        }
    }

    fn test_room(puzzle_choice: RoomPuzzleChoice) -> Room {
        let (tx, _) = broadcast::channel(4);
        Room {
            slug: "ROOMTEST".to_string(),
            puzzle_choice,
            active_puzzle_id: None,
            played_puzzle_ids: BTreeSet::new(),
            players: BTreeMap::new(),
            race_player_ids: Vec::new(),
            phase: ServerRoomPhase::Lobby,
            tx,
        }
    }

    fn add_test_player(
        room: &mut Room,
        id: &str,
        finish_ms: Option<u64>,
        gave_up: bool,
        joined_order: u64,
    ) {
        room.players.insert(
            id.to_string(),
            RoomPlayer {
                id: id.to_string(),
                name: id.to_string(),
                ready: false,
                connected: true,
                finish_ms,
                gave_up,
                medals: RoomMedalCounts::default(),
                recording: None,
                mouse_recording: None,
                joined_order,
            },
        );
        room.race_player_ids.push(id.to_string());
    }

    #[test]
    fn puzzle_data_contains_9x9_puzzles() {
        let puzzles = load_puzzles();
        assert_eq!(puzzles.len(), 300);
        for puzzle in puzzles {
            assert_eq!(puzzle.size, 9);
            assert_eq!(puzzle.colors.len(), 9);
            assert_eq!(puzzle.regions.len(), 9);
            assert!(puzzle.regions.iter().all(|row| row.len() == 9));
        }
    }

    #[test]
    fn random_room_puzzle_id_uses_unplayed_puzzles_first() {
        let puzzles = vec![test_puzzle(1), test_puzzle(2), test_puzzle(3)];
        let played = BTreeSet::from([1, 2]);

        assert_eq!(random_room_puzzle_id(&puzzles, &played), Some(3));

        let played = BTreeSet::from([1, 2, 3]);
        let next = random_room_puzzle_id(&puzzles, &played);
        assert!(matches!(next, Some(1 | 2 | 3)));
    }

    #[test]
    fn completing_room_race_records_puzzle_and_awards_medals() {
        let puzzles = vec![
            test_puzzle(1),
            test_puzzle(2),
            test_puzzle(3),
            test_puzzle(4),
        ];
        let mut room = test_room(RoomPuzzleChoice::Puzzle { id: 2 });
        room.active_puzzle_id = Some(2);
        add_test_player(&mut room, "ada", Some(1_200), false, 1);
        add_test_player(&mut room, "bea", Some(900), false, 2);
        add_test_player(&mut room, "cam", None, true, 3);
        add_test_player(&mut room, "dee", Some(1_500), false, 4);

        assert!(room_all_racers_done(&room));
        complete_room_race(&mut room, &puzzles, 42);

        assert!(room.played_puzzle_ids.contains(&2));
        assert!(matches!(
            room.puzzle_choice,
            RoomPuzzleChoice::Puzzle { id: 3 }
        ));
        assert!(matches!(
            room.phase,
            ServerRoomPhase::Complete { started_at_ms: 42 }
        ));
        assert_eq!(room.players["bea"].medals.gold, 1);
        assert_eq!(room.players["ada"].medals.silver, 1);
        assert_eq!(room.players["dee"].medals.bronze, 1);
        assert_eq!(room.players["cam"].medals.total(), 0);
    }
}
