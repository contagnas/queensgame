use axum::{
    Json, Router,
    extract::{
        Form, Path, Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::StatusCode,
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use futures_util::{SinkExt, StreamExt};
use nanoid::nanoid;
use queensgame_server_assets::{
    load_puzzles, static_css, static_dseg7_classic_bold_woff2, static_mage_light_svg,
    static_mage_svg,
    static_minesweeper_flag_svg, static_minesweeper_mine_svg, static_queen_svg,
};
use queensgame_server_pages::render_app_page;
use queensgame_server_runtime::{bind_addr, client_dist_dir};
use queensgame_shared::normalize_display_name;
use queensgame_shared_minesweeper::{
    MinesweeperBoard, MinesweeperBootstrap, MinesweeperCellState, MinesweeperStatus,
    build_room_minesweeper_board, clamp_room_minesweeper_tile_axis,
    clamp_room_minesweeper_time_limit_seconds, default_room_minesweeper_tile_cols,
    default_room_minesweeper_tile_rows, default_room_minesweeper_time_limit_seconds,
};
use queensgame_shared_queens::{
    CellState, GameBootstrap, Puzzle, PuzzleArchiveBootstrap, PuzzleNav, ValidateRequest,
    ValidateResponse, validate_solution,
};
use queensgame_shared_room::{
    CreateRoomResponse, ROOM_MOUSE_EVENT_ENTER, ROOM_MOUSE_EVENT_LEAVE,
    ROOM_MOUSE_EVENT_PRIMARY_DOWN, ROOM_MOUSE_EVENT_PRIMARY_UP, ROOM_MOUSE_EVENT_SECONDARY_DOWN,
    ROOM_MOUSE_EVENT_SECONDARY_UP, RoomBootstrap, RoomClientMessage, RoomGameKind, RoomLivePointer,
    RoomMedalCounts, RoomMinesweeperCellSnapshot, RoomMinesweeperSnapshot, RoomMouseRecording,
    RoomPhase, RoomPlayerSnapshot, RoomPuzzleChoice, RoomRecording, RoomRecordingFrame,
    RoomServerMessage, RoomSnapshot, append_mouse_recording, append_recording_frame,
    mouse_recording_times_are_sorted, recording_frame_is_valid,
};
use rand::Rng;
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{Mutex, broadcast};
use tower_http::{services::ServeDir, trace::TraceLayer};

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
    game_kind: RoomGameKind,
    puzzle_choice: RoomPuzzleChoice,
    minesweeper_time_limit_seconds: u32,
    minesweeper_tile_rows: usize,
    minesweeper_tile_cols: usize,
    minesweeper: Option<ServerMinesweeperGame>,
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
    minesweeper_score: u32,
    minesweeper_eliminated: bool,
    minesweeper_last_score_ms: Option<u64>,
    minesweeper_flags: BTreeSet<usize>,
    pointer: Option<RoomLivePointer>,
    joined_order: u64,
}

struct ServerMinesweeperGame {
    board: MinesweeperBoard,
    starting_cells: Vec<usize>,
    cell_owners: BTreeMap<usize, String>,
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
        .route("/minesweeper", get(minesweeper_page))
        .route("/rooms", get(rooms_index).post(create_room_form))
        .route("/rooms/:slug", get(room_page))
        .route("/api/rooms", post(create_room_api))
        .route("/api/rooms/:slug", get(room_api))
        .route("/api/puzzles/9x9", get(puzzles_api))
        .route("/api/puzzles/9x9/:id", get(puzzle_api))
        .route("/api/validate", post(validate_api))
        .route("/ws/rooms/:slug", get(room_ws))
        .route("/favicon.svg", get(static_mage_svg))
        .route("/static/mage-light.svg", get(static_mage_light_svg))
        .route("/static/mage.svg", get(static_mage_svg))
        .route("/static/style.css", get(static_css))
        .route("/static/queen.svg", get(static_queen_svg))
        .route(
            "/static/minesweeper-flag.svg",
            get(static_minesweeper_flag_svg),
        )
        .route(
            "/static/minesweeper-mine.svg",
            get(static_minesweeper_mine_svg),
        )
        .route(
            "/static/fonts/dseg7-classic-bold.woff2",
            get(static_dseg7_classic_bold_woff2),
        )
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

async fn puzzles_index(State(state): State<AppState>) -> Result<Html<String>, AppError> {
    let app_json = app_json("puzzles", &puzzle_archive_bootstrap(&state))?;
    Ok(Html(render_app_page(
        "Boardmage - 9x9 Queens Puzzles",
        "Choose from 300 bundled 9x9 Queens boards.",
        &app_json,
    )))
}

async fn puzzle_page(
    State(state): State<AppState>,
    Path(id): Path<usize>,
) -> Result<Html<String>, AppError> {
    let bootstrap = puzzle_bootstrap(&state, id)?;
    let app_json = app_json("game", &bootstrap)?;

    Ok(Html(render_app_page(
        &format!("Boardmage - Queens Puzzle #{}", bootstrap.puzzle.id),
        "Place one queen in every row, column, and colored region without diagonal touching.",
        &app_json,
    )))
}

async fn minesweeper_page() -> Result<Html<String>, AppError> {
    let app_json = app_json("minesweeper", &MinesweeperBootstrap::default())?;

    Ok(Html(render_app_page(
        "Boardmage - Minesweeper",
        "Play expert Minesweeper.",
        &app_json,
    )))
}

async fn rooms_index() -> Result<Html<String>, AppError> {
    let app_json = app_empty_json("rooms");
    Ok(Html(render_app_page(
        "Boardmage Rooms",
        "Create a multiplayer Boardmage room.",
        &app_json,
    )))
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
    let bootstrap = room_bootstrap(&state, slug.clone()).await?;
    let app_json = app_json("room", &bootstrap)?;

    Ok(Html(render_app_page(
        &format!("Boardmage Room {slug}"),
        "Join a multiplayer Boardmage room.",
        &app_json,
    )))
}

async fn room_api(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Json<RoomBootstrap>, AppError> {
    Ok(Json(room_bootstrap(&state, slug).await?))
}

async fn puzzles_api(State(state): State<AppState>) -> Json<PuzzleArchiveBootstrap> {
    Json(puzzle_archive_bootstrap(&state))
}

async fn puzzle_api(
    State(state): State<AppState>,
    Path(id): Path<usize>,
) -> Result<Json<GameBootstrap>, AppError> {
    Ok(Json(puzzle_bootstrap(&state, id)?))
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
            game_kind: RoomGameKind::Queens,
            puzzle_choice: RoomPuzzleChoice::Random,
            minesweeper_time_limit_seconds: default_room_minesweeper_time_limit_seconds(),
            minesweeper_tile_rows: default_room_minesweeper_tile_rows(),
            minesweeper_tile_cols: default_room_minesweeper_tile_cols(),
            minesweeper: None,
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
    let personalized_initial_snapshot = room_message_for_player(&initial_snapshot, &player_id);
    if sender
        .send(Message::Text(personalized_initial_snapshot))
        .await
        .is_err()
    {
        disconnect_player(&state, &slug, &player_id).await;
        return;
    }

    let send_player_id = player_id.clone();
    let send_task = tokio::spawn(async move {
        while let Ok(message) = room_rx.recv().await {
            let message = room_message_for_player(&message, &send_player_id);
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
            minesweeper_score: 0,
            minesweeper_eliminated: false,
            minesweeper_last_score_ms: None,
            minesweeper_flags: BTreeSet::new(),
            pointer: None,
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
        player.pointer = None;
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
        RoomClientMessage::SelectGame { game_kind } => {
            select_room_game(state, slug, game_kind).await;
        }
        RoomClientMessage::SelectPuzzle { puzzle_id } => {
            select_room_puzzle(state, slug, puzzle_id).await;
        }
        RoomClientMessage::SelectRandom => {
            select_random_puzzle(state, slug).await;
        }
        RoomClientMessage::SetMinesweeperTimeLimit { seconds } => {
            set_room_minesweeper_time_limit(state, slug, seconds).await;
        }
        RoomClientMessage::SetMinesweeperTiles { rows, cols } => {
            set_room_minesweeper_tiles(state, slug, rows, cols).await;
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
        RoomClientMessage::MinesweeperReveal { index } => {
            reveal_room_minesweeper_cell(state, slug, player_id, index).await;
        }
        RoomClientMessage::MinesweeperToggleFlag { index } => {
            toggle_room_minesweeper_flag(state, slug, player_id, index).await;
        }
        RoomClientMessage::MinesweeperChord { index } => {
            chord_room_minesweeper_cell(state, slug, player_id, index).await;
        }
        RoomClientMessage::PointerUpdate { pointer } => {
            update_room_pointer(state, slug, player_id, pointer).await;
        }
    }
}

async fn select_room_game(state: &AppState, slug: &str, game_kind: RoomGameKind) {
    let mut rooms = state.rooms.lock().await;
    let Some(room) = rooms.get_mut(slug) else {
        return;
    };
    if !room_accepts_next_race_setup(room) {
        return;
    }
    if room.game_kind != game_kind {
        room.game_kind = game_kind;
        clear_room_race_results(room);
        room.phase = ServerRoomPhase::Lobby;
        reset_room_ready_flags(room);
    }
    broadcast_room(room, &state.puzzles);
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
    if room.game_kind != RoomGameKind::Queens || !room_accepts_next_race_setup(room) {
        return;
    }
    room.puzzle_choice = RoomPuzzleChoice::Puzzle { id: puzzle_id };
    reset_room_setup_for_selection(room);
    broadcast_room(room, &state.puzzles);
}

async fn set_room_minesweeper_time_limit(state: &AppState, slug: &str, seconds: u32) {
    update_room_minesweeper_setup(state, slug, |room| {
        room.minesweeper_time_limit_seconds = clamp_room_minesweeper_time_limit_seconds(seconds);
    })
    .await;
}

async fn set_room_minesweeper_tiles(state: &AppState, slug: &str, rows: usize, cols: usize) {
    update_room_minesweeper_setup(state, slug, |room| {
        room.minesweeper_tile_rows = clamp_room_minesweeper_tile_axis(rows);
        room.minesweeper_tile_cols = clamp_room_minesweeper_tile_axis(cols);
    })
    .await;
}

async fn update_room_minesweeper_setup(
    state: &AppState,
    slug: &str,
    update: impl FnOnce(&mut Room),
) {
    let mut rooms = state.rooms.lock().await;
    let Some(room) = rooms.get_mut(slug) else {
        return;
    };
    if room.game_kind != RoomGameKind::Minesweeper || !room_accepts_next_race_setup(room) {
        return;
    }
    update(room);
    reset_room_ready_flags(room);
    broadcast_room(room, &state.puzzles);
}

async fn select_random_puzzle(state: &AppState, slug: &str) {
    let mut rooms = state.rooms.lock().await;
    let Some(room) = rooms.get_mut(slug) else {
        return;
    };
    if room.game_kind != RoomGameKind::Queens || !room_accepts_next_race_setup(room) {
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
            clear_room_race_results(room);
            if room.game_kind == RoomGameKind::Minesweeper {
                prepare_room_minesweeper_game(room);
            }
            let countdown_ms = if room.game_kind == RoomGameKind::Minesweeper {
                5_000
            } else {
                3_000
            };
            let start = now_ms() + countdown_ms;
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

        match room.game_kind {
            RoomGameKind::Queens => {
                let puzzle_id = match room.puzzle_choice {
                    RoomPuzzleChoice::Puzzle { id } => id,
                    RoomPuzzleChoice::Random => {
                        let Some(id) =
                            random_room_puzzle_id(&state.puzzles, &room.played_puzzle_ids)
                        else {
                            return;
                        };
                        id
                    }
                };
                room.active_puzzle_id = Some(puzzle_id);
                begin_room_race_for_connected_players(room);
            }
            RoomGameKind::Minesweeper => {
                if room.minesweeper.is_none() {
                    prepare_room_minesweeper_game(room);
                }
                reveal_room_minesweeper_starts(room);
                begin_room_race_for_connected_players(room);
            }
        }

        room.phase = ServerRoomPhase::Racing {
            started_at_ms: now_ms(),
            started_at: Instant::now(),
        };
        if room.game_kind == RoomGameKind::Minesweeper {
            schedule_room_minesweeper_timeout(
                state.clone(),
                slug.clone(),
                room.phase
                    .as_snapshot_phase()
                    .race_started_at_ms()
                    .unwrap_or_default(),
                room.minesweeper_time_limit_seconds,
            );
        }
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
    if room.game_kind != RoomGameKind::Queens {
        return;
    }
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
    if room.game_kind != RoomGameKind::Queens {
        return;
    }
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

    if let Some(player) = room.players.get_mut(player_id)
        && player.finish_ms.is_none()
        && !player.gave_up
    {
        player.finish_ms = Some(elapsed_ms);
        player.recording = Some(recording);
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

    match room.game_kind {
        RoomGameKind::Queens => {
            if let Some(player) = room.players.get_mut(player_id)
                && player.finish_ms.is_none()
            {
                player.gave_up = true;
            }

            if room_all_racers_done(room) {
                complete_room_race(room, &state.puzzles, started_at_ms);
            }
        }
        RoomGameKind::Minesweeper => {
            if let Some(player) = room.players.get_mut(player_id) {
                player.minesweeper_eliminated = true;
                player.pointer = None;
            }
            if room_minesweeper_should_complete(room) {
                complete_room_race(room, &state.puzzles, started_at_ms);
            }
        }
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
    if room.game_kind != RoomGameKind::Queens {
        return;
    }
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
    if room.game_kind != RoomGameKind::Queens {
        return;
    }
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

async fn reveal_room_minesweeper_cell(state: &AppState, slug: &str, player_id: &str, index: usize) {
    let mut rooms = state.rooms.lock().await;
    let Some(room) = rooms.get_mut(slug) else {
        return;
    };
    let Some((started_at_ms, elapsed_ms)) = active_minesweeper_elapsed_ms(room) else {
        return;
    };
    if !room_minesweeper_player_can_act(room, player_id) {
        return;
    }
    let flagged = room
        .players
        .get(player_id)
        .map(|player| player.minesweeper_flags.contains(&index))
        .unwrap_or(false);
    if flagged {
        return;
    }
    let Some(game) = room.minesweeper.as_mut() else {
        return;
    };
    let Some(cell) = game.board.cells.get(index) else {
        return;
    };
    if cell.state == MinesweeperCellState::Revealed {
        return;
    }

    let mut score_delta = 0u32;
    let mut eliminated = false;
    if cell.mine {
        detonate_room_minesweeper_mine(game, player_id, index);
        eliminated = true;
    } else {
        let revealed = game.board.reveal_safe_cells(index);
        score_delta = claim_minesweeper_revealed_cells(game, player_id, &revealed);
    }

    apply_minesweeper_player_result(room, player_id, score_delta, elapsed_ms, eliminated);
    if room_minesweeper_should_complete(room) {
        complete_room_race(room, &state.puzzles, started_at_ms);
    }
    broadcast_room(room, &state.puzzles);
}

async fn toggle_room_minesweeper_flag(state: &AppState, slug: &str, player_id: &str, index: usize) {
    let mut rooms = state.rooms.lock().await;
    let Some(room) = rooms.get_mut(slug) else {
        return;
    };
    if active_minesweeper_elapsed_ms(room).is_none()
        || !room_minesweeper_player_can_act(room, player_id)
    {
        return;
    }
    let Some(game) = room.minesweeper.as_ref() else {
        return;
    };
    let Some(cell) = game.board.cells.get(index) else {
        return;
    };
    if cell.state == MinesweeperCellState::Revealed {
        return;
    }
    let Some(player) = room.players.get_mut(player_id) else {
        return;
    };
    if !player.minesweeper_flags.remove(&index) {
        player.minesweeper_flags.insert(index);
    }
    broadcast_room(room, &state.puzzles);
}

async fn chord_room_minesweeper_cell(state: &AppState, slug: &str, player_id: &str, index: usize) {
    let mut rooms = state.rooms.lock().await;
    let Some(room) = rooms.get_mut(slug) else {
        return;
    };
    let Some((started_at_ms, elapsed_ms)) = active_minesweeper_elapsed_ms(room) else {
        return;
    };
    if !room_minesweeper_player_can_act(room, player_id) {
        return;
    }
    let flags = room
        .players
        .get(player_id)
        .map(|player| player.minesweeper_flags.clone())
        .unwrap_or_default();
    let Some(game) = room.minesweeper.as_mut() else {
        return;
    };
    let Some(cell) = game.board.cells.get(index) else {
        return;
    };
    if cell.state != MinesweeperCellState::Revealed || cell.mine || cell.adjacent_mines == 0 {
        return;
    }
    let neighbors = game.board.neighbors(index);
    let flagged_neighbors = neighbors
        .iter()
        .filter(|neighbor| flags.contains(neighbor))
        .count();
    if flagged_neighbors != usize::from(cell.adjacent_mines) {
        return;
    }

    let targets = neighbors
        .into_iter()
        .filter(|neighbor| !flags.contains(neighbor))
        .filter(|neighbor| game.board.cells[*neighbor].state != MinesweeperCellState::Revealed)
        .collect::<Vec<_>>();
    let mut eliminated = false;
    let mut score_delta = 0u32;
    if let Some(mine) = targets
        .iter()
        .copied()
        .find(|target| game.board.cells[*target].mine)
    {
        detonate_room_minesweeper_mine(game, player_id, mine);
        eliminated = true;
    } else {
        for target in targets {
            let revealed = game.board.reveal_safe_cells(target);
            score_delta = score_delta
                .saturating_add(claim_minesweeper_revealed_cells(game, player_id, &revealed));
        }
    }

    apply_minesweeper_player_result(room, player_id, score_delta, elapsed_ms, eliminated);
    if room_minesweeper_should_complete(room) {
        complete_room_race(room, &state.puzzles, started_at_ms);
    }
    broadcast_room(room, &state.puzzles);
}

async fn update_room_pointer(
    state: &AppState,
    slug: &str,
    player_id: &str,
    pointer: Option<RoomLivePointer>,
) {
    let mut rooms = state.rooms.lock().await;
    let Some(room) = rooms.get_mut(slug) else {
        return;
    };
    if room.game_kind != RoomGameKind::Minesweeper {
        return;
    }
    if pointer.is_some() && !room_minesweeper_player_can_point(room, player_id) {
        return;
    }
    let Some(player) = room.players.get_mut(player_id) else {
        return;
    };
    let pointer = pointer.map(|mut pointer| {
        pointer.updated_at_ms = now_ms();
        pointer
    });
    player.pointer = pointer;
    let _ = room.tx.send(
        serde_json::to_string(&RoomServerMessage::PointerUpdate {
            player_id: player_id.to_string(),
            pointer,
        })
        .expect("room pointer update must be serializable"),
    );
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
        player.minesweeper_score = 0;
        player.minesweeper_eliminated = false;
        player.minesweeper_last_score_ms = None;
        player.minesweeper_flags.clear();
        player.pointer = None;
    }
    room.race_player_ids.clear();
    room.active_puzzle_id = None;
    room.minesweeper = None;
}

fn prepare_room_minesweeper_game(room: &mut Room) {
    let tile_rows = clamp_room_minesweeper_tile_axis(room.minesweeper_tile_rows);
    let tile_cols = clamp_room_minesweeper_tile_axis(room.minesweeper_tile_cols);
    room.minesweeper_tile_rows = tile_rows;
    room.minesweeper_tile_cols = tile_cols;
    let seed = now_ms()
        ^ (tile_rows as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
        ^ (tile_cols as u64).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    let built = build_room_minesweeper_board(tile_rows, tile_cols, seed);
    room.minesweeper = Some(ServerMinesweeperGame {
        board: built.board,
        starting_cells: built.starting_cells,
        cell_owners: BTreeMap::new(),
    });
}

fn begin_room_race_for_connected_players(room: &mut Room) {
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
        player.minesweeper_score = 0;
        player.minesweeper_eliminated = false;
        player.minesweeper_last_score_ms = None;
        player.minesweeper_flags.clear();
        player.pointer = None;
    }
}

fn reveal_room_minesweeper_starts(room: &mut Room) {
    let Some(game) = room.minesweeper.as_mut() else {
        return;
    };
    game.board.status = MinesweeperStatus::Playing;
    for start in game.starting_cells.clone() {
        let _ = game.board.reveal_safe_cells(start);
    }
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

fn active_minesweeper_elapsed_ms(room: &Room) -> Option<(u64, u64)> {
    if room.game_kind != RoomGameKind::Minesweeper {
        return None;
    }
    match &room.phase {
        ServerRoomPhase::Racing {
            started_at_ms,
            started_at,
        } => Some((*started_at_ms, started_at.elapsed().as_millis() as u64)),
        ServerRoomPhase::Lobby
        | ServerRoomPhase::Countdown { .. }
        | ServerRoomPhase::Complete { .. } => None,
    }
}

fn room_minesweeper_player_can_act(room: &Room, player_id: &str) -> bool {
    room.race_player_ids.iter().any(|id| id == player_id)
        && room
            .players
            .get(player_id)
            .map(|player| !player.minesweeper_eliminated)
            .unwrap_or(false)
}

fn room_minesweeper_player_can_point(room: &Room, player_id: &str) -> bool {
    let Some(player) = room.players.get(player_id) else {
        return false;
    };
    if !player.connected {
        return false;
    }
    match room.phase {
        ServerRoomPhase::Countdown { .. } => room.minesweeper.is_some(),
        ServerRoomPhase::Racing { .. } => !player.minesweeper_eliminated,
        ServerRoomPhase::Lobby | ServerRoomPhase::Complete { .. } => false,
    }
}

fn minesweeper_score_for_revealed_cells(board: &MinesweeperBoard, revealed: &[usize]) -> u32 {
    revealed
        .iter()
        .filter(|index| {
            board.cells[**index].state == MinesweeperCellState::Revealed
                && !board.cells[**index].mine
                && board.cells[**index].adjacent_mines > 0
        })
        .count() as u32
}

fn claim_minesweeper_revealed_cells(
    game: &mut ServerMinesweeperGame,
    player_id: &str,
    revealed: &[usize],
) -> u32 {
    for index in revealed {
        let Some(cell) = game.board.cells.get(*index) else {
            continue;
        };
        if cell.state == MinesweeperCellState::Revealed && !cell.mine && cell.adjacent_mines > 0 {
            game.cell_owners
                .entry(*index)
                .or_insert_with(|| player_id.to_string());
        }
    }
    minesweeper_score_for_revealed_cells(&game.board, revealed)
}

fn detonate_room_minesweeper_mine(game: &mut ServerMinesweeperGame, player_id: &str, index: usize) {
    let Some(cell) = game.board.cells.get_mut(index) else {
        return;
    };
    if !cell.mine {
        return;
    }
    cell.state = MinesweeperCellState::Revealed;
    cell.detonated = true;
    game.cell_owners
        .entry(index)
        .or_insert_with(|| player_id.to_string());
}

fn apply_minesweeper_player_result(
    room: &mut Room,
    player_id: &str,
    score_delta: u32,
    elapsed_ms: u64,
    eliminated: bool,
) {
    let Some(player) = room.players.get_mut(player_id) else {
        return;
    };
    if score_delta > 0 {
        player.minesweeper_score = player.minesweeper_score.saturating_add(score_delta);
        player.minesweeper_last_score_ms = Some(elapsed_ms);
    }
    if eliminated {
        player.minesweeper_eliminated = true;
        player.pointer = None;
    }
}

fn room_minesweeper_should_complete(room: &Room) -> bool {
    let solved = room
        .minesweeper
        .as_ref()
        .map(|game| game.board.all_safe_cells_revealed())
        .unwrap_or(false);
    solved || room_all_minesweeper_players_eliminated(room)
}

fn room_all_minesweeper_players_eliminated(room: &Room) -> bool {
    !room.race_player_ids.is_empty()
        && room.race_player_ids.iter().all(|player_id| {
            room.players
                .get(player_id)
                .map(|player| player.minesweeper_eliminated)
                .unwrap_or(true)
        })
}

fn schedule_room_minesweeper_timeout(
    state: AppState,
    slug: String,
    started_at_ms: u64,
    seconds: u32,
) {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(u64::from(seconds))).await;
        let mut rooms = state.rooms.lock().await;
        let Some(room) = rooms.get_mut(&slug) else {
            return;
        };
        if !matches!(
            room.phase,
            ServerRoomPhase::Racing {
                started_at_ms: active_start,
                ..
            } if active_start == started_at_ms
        ) || room.game_kind != RoomGameKind::Minesweeper
        {
            return;
        }
        complete_room_race(room, &state.puzzles, started_at_ms);
        broadcast_room(room, &state.puzzles);
    });
}

fn complete_room_race(room: &mut Room, puzzles: &[Puzzle], started_at_ms: u64) {
    match room.game_kind {
        RoomGameKind::Queens => {
            let completed_puzzle_id = room.active_puzzle_id;
            if let Some(puzzle_id) = completed_puzzle_id {
                room.played_puzzle_ids.insert(puzzle_id);
            }

            award_room_queens_medals(room);

            if let (RoomPuzzleChoice::Puzzle { .. }, Some(puzzle_id)) =
                (&room.puzzle_choice, completed_puzzle_id)
                && let Some(next_id) = next_puzzle_id(puzzles, puzzle_id)
            {
                room.puzzle_choice = RoomPuzzleChoice::Puzzle { id: next_id };
            }
        }
        RoomGameKind::Minesweeper => {
            award_room_minesweeper_medals(room);
        }
    }

    room.phase = ServerRoomPhase::Complete { started_at_ms };
}

fn award_room_queens_medals(room: &mut Room) {
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

fn award_room_minesweeper_medals(room: &mut Room) {
    for (place, player_id) in minesweeper_ranked_player_ids(room)
        .into_iter()
        .take(3)
        .enumerate()
    {
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

fn minesweeper_ranked_player_ids(room: &Room) -> Vec<String> {
    let mut players = room
        .race_player_ids
        .iter()
        .filter_map(|player_id| room.players.get(player_id))
        .collect::<Vec<_>>();
    players.sort_by(|left, right| {
        right
            .minesweeper_score
            .cmp(&left.minesweeper_score)
            .then_with(|| {
                left.minesweeper_last_score_ms
                    .unwrap_or(u64::MAX)
                    .cmp(&right.minesweeper_last_score_ms.unwrap_or(u64::MAX))
            })
            .then_with(|| left.joined_order.cmp(&right.joined_order))
    });
    players
        .into_iter()
        .map(|player| player.id.clone())
        .collect()
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

fn room_message_for_player(message: &str, player_id: &str) -> String {
    let Ok(RoomServerMessage::Snapshot { mut snapshot }) =
        serde_json::from_str::<RoomServerMessage>(message)
    else {
        return message.to_string();
    };

    for player in &mut snapshot.players {
        if player.id != player_id {
            player.minesweeper_flags.clear();
        }
    }

    serde_json::to_string(&RoomServerMessage::Snapshot { snapshot })
        .expect("personalized room snapshot must be serializable")
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
    let puzzle = match (room.game_kind, &room.phase) {
        (RoomGameKind::Queens, ServerRoomPhase::Racing { .. })
        | (RoomGameKind::Queens, ServerRoomPhase::Complete { .. }) => room
            .active_puzzle_id
            .and_then(|id| find_puzzle_by_id(puzzles, id).cloned()),
        _ => None,
    };
    let minesweeper = if room.game_kind == RoomGameKind::Minesweeper {
        room.minesweeper.as_ref().map(room_minesweeper_snapshot)
    } else {
        None
    };
    let winner_id = if matches!(room.phase, ServerRoomPhase::Complete { .. }) {
        match room.game_kind {
            RoomGameKind::Queens => room
                .players
                .values()
                .filter_map(|player| player.finish_ms.map(|finish_ms| (player, finish_ms)))
                .min_by_key(|(player, finish_ms)| (*finish_ms, player.joined_order))
                .map(|(player, _)| player.id.clone()),
            RoomGameKind::Minesweeper => minesweeper_ranked_player_ids(room).into_iter().next(),
        }
    } else {
        None
    };

    RoomSnapshot {
        slug: room.slug.clone(),
        game_kind: room.game_kind,
        phase: room.phase.as_snapshot_phase(),
        puzzle_choice: room.puzzle_choice.clone(),
        minesweeper_time_limit_seconds: room.minesweeper_time_limit_seconds,
        minesweeper_tile_rows: room.minesweeper_tile_rows,
        minesweeper_tile_cols: room.minesweeper_tile_cols,
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
                minesweeper_score: player.minesweeper_score,
                minesweeper_eliminated: player.minesweeper_eliminated,
                minesweeper_last_score_ms: player.minesweeper_last_score_ms,
                minesweeper_flags: player.minesweeper_flags.iter().copied().collect(),
                pointer: player.pointer,
            })
            .collect(),
        puzzle,
        minesweeper,
        winner_id,
    }
}

fn room_minesweeper_snapshot(game: &ServerMinesweeperGame) -> RoomMinesweeperSnapshot {
    let starting_cells = game.starting_cells.iter().copied().collect::<BTreeSet<_>>();
    let cells = game
        .board
        .cells
        .iter()
        .enumerate()
        .map(|(index, cell)| {
            let revealed = cell.state == MinesweeperCellState::Revealed;
            let start = starting_cells.contains(&index);
            RoomMinesweeperCellSnapshot {
                revealed,
                mine: cell.mine,
                detonated: cell.detonated,
                start,
                adjacent_mines: Some(cell.adjacent_mines),
                owner_id: game.cell_owners.get(&index).cloned(),
            }
        })
        .collect();

    RoomMinesweeperSnapshot {
        width: game.board.width,
        height: game.board.height,
        mines: game.board.mines,
        starting_cells: game.starting_cells.clone(),
        cells,
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

fn puzzle_bootstrap(state: &AppState, id: usize) -> Result<GameBootstrap, AppError> {
    let puzzle = find_puzzle(state, id)?.clone();
    Ok(GameBootstrap {
        puzzle,
        puzzle_nav: puzzle_nav(&state.puzzles, id),
        total: state.puzzles.len(),
    })
}

fn puzzle_archive_bootstrap(state: &AppState) -> PuzzleArchiveBootstrap {
    PuzzleArchiveBootstrap {
        puzzle_nav: puzzle_nav(&state.puzzles, 0),
        total: state.puzzles.len(),
    }
}

async fn room_bootstrap(state: &AppState, slug: String) -> Result<RoomBootstrap, AppError> {
    let snapshot = {
        let rooms = state.rooms.lock().await;
        let room = rooms.get(&slug).ok_or(AppError::NotFound)?;
        snapshot_room(room, &state.puzzles)
    };

    Ok(RoomBootstrap {
        slug,
        total_puzzles: state.puzzles.len(),
        snapshot,
    })
}

fn app_json<T: serde::Serialize>(kind: &str, data: &T) -> Result<String, AppError> {
    Ok(serde_json::json!({
        "kind": kind,
        "data": data,
    })
    .to_string())
}

fn app_empty_json(kind: &str) -> String {
    serde_json::json!({ "kind": kind }).to_string()
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
    use queensgame_shared::{
        MINESWEEPER_EXPERT_HEIGHT, MINESWEEPER_EXPERT_MINES, MINESWEEPER_EXPERT_WIDTH,
    };

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
            game_kind: RoomGameKind::Queens,
            puzzle_choice,
            minesweeper_time_limit_seconds: default_room_minesweeper_time_limit_seconds(),
            minesweeper_tile_rows: default_room_minesweeper_tile_rows(),
            minesweeper_tile_cols: default_room_minesweeper_tile_cols(),
            minesweeper: None,
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
                minesweeper_score: 0,
                minesweeper_eliminated: false,
                minesweeper_last_score_ms: None,
                minesweeper_flags: BTreeSet::new(),
                pointer: None,
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

    #[test]
    fn completing_minesweeper_race_awards_medals_by_score() {
        let puzzles = vec![test_puzzle(1)];
        let mut room = test_room(RoomPuzzleChoice::Random);
        room.game_kind = RoomGameKind::Minesweeper;
        add_test_player(&mut room, "ada", None, false, 1);
        add_test_player(&mut room, "bea", None, false, 2);
        add_test_player(&mut room, "cam", None, false, 3);

        room.players.get_mut("ada").unwrap().minesweeper_score = 12;
        room.players
            .get_mut("ada")
            .unwrap()
            .minesweeper_last_score_ms = Some(800);
        room.players.get_mut("bea").unwrap().minesweeper_score = 14;
        room.players
            .get_mut("bea")
            .unwrap()
            .minesweeper_last_score_ms = Some(900);
        room.players.get_mut("cam").unwrap().minesweeper_score = 12;
        room.players
            .get_mut("cam")
            .unwrap()
            .minesweeper_last_score_ms = Some(700);

        complete_room_race(&mut room, &puzzles, 99);

        assert_eq!(room.players["bea"].medals.gold, 1);
        assert_eq!(room.players["cam"].medals.silver, 1);
        assert_eq!(room.players["ada"].medals.bronze, 1);
        assert!(matches!(
            room.phase,
            ServerRoomPhase::Complete { started_at_ms: 99 }
        ));
    }

    #[test]
    fn minesweeper_snapshot_includes_hidden_board_data_for_optimistic_play() {
        let mut room = test_room(RoomPuzzleChoice::Random);
        room.game_kind = RoomGameKind::Minesweeper;
        add_test_player(&mut room, "ada", None, false, 1);
        prepare_room_minesweeper_game(&mut room);

        let snapshot =
            room_minesweeper_snapshot(room.minesweeper.as_ref().expect("minesweeper game"));

        assert!(
            snapshot
                .cells
                .iter()
                .any(|cell| !cell.revealed && cell.mine)
        );
        assert!(
            snapshot
                .cells
                .iter()
                .all(|cell| cell.adjacent_mines.is_some())
        );
    }

    #[test]
    fn minesweeper_room_defaults_to_one_tile_even_with_many_players() {
        let mut room = test_room(RoomPuzzleChoice::Random);
        room.game_kind = RoomGameKind::Minesweeper;
        for index in 0..5 {
            add_test_player(&mut room, &format!("p{index}"), None, false, index);
        }

        prepare_room_minesweeper_game(&mut room);
        let game = room.minesweeper.as_ref().expect("minesweeper game");

        assert_eq!(room.minesweeper_tile_rows, 1);
        assert_eq!(room.minesweeper_tile_cols, 1);
        assert_eq!(game.board.width, MINESWEEPER_EXPERT_WIDTH);
        assert_eq!(game.board.height, MINESWEEPER_EXPERT_HEIGHT);
        assert_eq!(game.starting_cells.len(), 1);
    }

    #[test]
    fn minesweeper_room_uses_configured_tile_grid() {
        let mut room = test_room(RoomPuzzleChoice::Random);
        room.game_kind = RoomGameKind::Minesweeper;
        room.minesweeper_tile_rows = 3;
        room.minesweeper_tile_cols = 2;
        add_test_player(&mut room, "ada", None, false, 1);

        prepare_room_minesweeper_game(&mut room);
        let game = room.minesweeper.as_ref().expect("minesweeper game");

        assert_eq!(game.board.width, MINESWEEPER_EXPERT_WIDTH * 2);
        assert_eq!(game.board.height, MINESWEEPER_EXPERT_HEIGHT * 3);
        assert_eq!(game.board.mines, MINESWEEPER_EXPERT_MINES * 6);
        assert_eq!(game.starting_cells.len(), 6);
    }

    #[test]
    fn minesweeper_snapshot_marks_number_owner_after_reveal() {
        let mut room = test_room(RoomPuzzleChoice::Random);
        room.game_kind = RoomGameKind::Minesweeper;
        add_test_player(&mut room, "ada", None, false, 1);
        prepare_room_minesweeper_game(&mut room);
        let game = room.minesweeper.as_mut().expect("minesweeper game");
        let index = game
            .board
            .cells
            .iter()
            .position(|cell| !cell.mine && cell.adjacent_mines > 0)
            .expect("numbered safe cell");

        let revealed = game.board.reveal_safe_cells(index);
        assert_eq!(claim_minesweeper_revealed_cells(game, "ada", &revealed), 1);
        let snapshot = room_minesweeper_snapshot(game);

        assert_eq!(snapshot.cells[index].owner_id.as_deref(), Some("ada"));
    }

    #[test]
    fn minesweeper_snapshot_marks_detonated_mine_owner() {
        let mut room = test_room(RoomPuzzleChoice::Random);
        room.game_kind = RoomGameKind::Minesweeper;
        add_test_player(&mut room, "ada", None, false, 1);
        prepare_room_minesweeper_game(&mut room);
        let game = room.minesweeper.as_mut().expect("minesweeper game");
        let index = game
            .board
            .cells
            .iter()
            .position(|cell| cell.mine)
            .expect("mine cell");

        detonate_room_minesweeper_mine(game, "ada", index);
        let snapshot = room_minesweeper_snapshot(game);

        assert!(snapshot.cells[index].revealed);
        assert!(snapshot.cells[index].detonated);
        assert_eq!(snapshot.cells[index].owner_id.as_deref(), Some("ada"));
    }

    #[test]
    fn minesweeper_players_can_point_during_countdown_before_race_ids_exist() {
        let mut room = test_room(RoomPuzzleChoice::Random);
        room.game_kind = RoomGameKind::Minesweeper;
        add_test_player(&mut room, "ada", None, false, 1);
        prepare_room_minesweeper_game(&mut room);
        room.race_player_ids.clear();
        room.phase = ServerRoomPhase::Countdown { starts_at_ms: 10 };

        assert!(room.race_player_ids.is_empty());
        assert!(room_minesweeper_player_can_point(&room, "ada"));
        assert!(!room_minesweeper_player_can_act(&room, "ada"));
    }

    #[test]
    fn personalized_room_snapshot_hides_other_minesweeper_flags() {
        let mut room = test_room(RoomPuzzleChoice::Random);
        room.game_kind = RoomGameKind::Minesweeper;
        add_test_player(&mut room, "ada", None, false, 1);
        add_test_player(&mut room, "bea", None, false, 2);
        room.players
            .get_mut("ada")
            .unwrap()
            .minesweeper_flags
            .extend([1, 2]);
        room.players
            .get_mut("bea")
            .unwrap()
            .minesweeper_flags
            .extend([3, 4]);

        let message = room_message_for_player(&room_snapshot_message(&room, &[]), "ada");
        let RoomServerMessage::Snapshot { snapshot } =
            serde_json::from_str(&message).expect("snapshot message")
        else {
            panic!("expected snapshot");
        };

        let ada = snapshot
            .players
            .iter()
            .find(|player| player.id == "ada")
            .expect("ada");
        let bea = snapshot
            .players
            .iter()
            .find(|player| player.id == "bea")
            .expect("bea");
        assert_eq!(ada.minesweeper_flags, vec![1, 2]);
        assert!(bea.minesweeper_flags.is_empty());
    }
}
