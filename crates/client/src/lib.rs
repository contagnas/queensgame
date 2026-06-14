use dioxus::{html::input_data::MouseButton, prelude::*};
use gloo_timers::future::TimeoutFuture;
use queensgame_shared::{
    append_mouse_recording, append_recording_frame, build_cells, invalidated_by_queen,
    normalize_display_name, validate_solution, CellState, CellView, GameBootstrap,
    MinesweeperBoard, MinesweeperBootstrap, MinesweeperCell, MinesweeperCellState,
    MinesweeperStatus, Puzzle, PuzzleNav, RoomBootstrap, RoomClientMessage, RoomGameKind,
    RoomLivePointer, RoomMinesweeperCellSnapshot, RoomMinesweeperSnapshot, RoomMouseEvent,
    RoomMouseRecording, RoomMouseSample, RoomPhase, RoomPlayerSnapshot, RoomPuzzleChoice,
    RoomRecording, RoomRecordingFrame, RoomServerMessage, RoomSnapshot, ValidateResponse,
    DISPLAY_NAME_MAX_CHARS, ROOM_MINESWEEPER_MAX_SECONDS, ROOM_MINESWEEPER_MIN_SECONDS,
    ROOM_MOUSE_EVENT_ENTER, ROOM_MOUSE_EVENT_LEAVE, ROOM_MOUSE_EVENT_PRIMARY_DOWN,
    ROOM_MOUSE_EVENT_PRIMARY_UP, ROOM_MOUSE_EVENT_SECONDARY_DOWN, ROOM_MOUSE_EVENT_SECONDARY_UP,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, VecDeque},
    rc::Rc,
};
use wasm_bindgen::{prelude::*, JsCast};

#[wasm_bindgen(start)]
pub fn start() {
    dioxus::LaunchBuilder::web()
        .with_cfg(dioxus::web::Config::new().rootname("game-root"))
        .launch(app);
}

fn app() -> Element {
    let bootstrap = use_hook(read_app_bootstrap);

    match bootstrap {
        Ok(AppBootstrap::Game(bootstrap)) => rsx! {
            Game { bootstrap }
        },
        Ok(AppBootstrap::Room(bootstrap)) => rsx! {
            RoomApp { bootstrap }
        },
        Ok(AppBootstrap::Minesweeper(bootstrap)) => rsx! {
            MinesweeperApp { bootstrap }
        },
        Err(message) => rsx! {
            main { class: "game-page",
                section { class: "game-shell", role: "alert",
                    h1 { "Puzzle failed to load" }
                    p { class: "rule-panel", "{message}" }
                }
            }
        },
    }
}

#[derive(Clone)]
enum AppBootstrap {
    Game(GameBootstrap),
    Room(RoomBootstrap),
    Minesweeper(MinesweeperBootstrap),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Queen,
    Mark,
    Clear,
}

#[derive(Clone, PartialEq)]
struct GameState {
    puzzle: Puzzle,
    cells: Vec<CellView>,
    puzzle_nav: Vec<PuzzleNav>,
    total: usize,
    mode: Mode,
    states: Vec<CellState>,
    storage_key: Option<String>,
    room_started_at_ms: Option<u64>,
    recording: Option<RoomRecording>,
    mouse_recording: Option<RoomMouseRecording>,
    mouse_recording_sent: bool,
    last_mouse_sample_ms: Option<u32>,
    recording_frames_sent: usize,
    mouse_samples_sent: usize,
    mouse_events_sent: usize,
    history: Vec<Vec<CellState>>,
    started_at_ms: f64,
    completed: bool,
    completed_ms: u64,
    validation: ValidateResponse,
    finish_notified: bool,
    mark_drag: Option<MarkDrag>,
    win_visible: bool,
}

#[derive(Clone, PartialEq)]
struct MinesweeperGameState {
    board: MinesweeperBoard,
    started_at_ms: Option<f64>,
    elapsed_ms: u64,
    face_down: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct MarkDrag {
    start_index: usize,
    action: MarkDragAction,
    moved: bool,
    changed: bool,
    history_started: bool,
    needs_auto_refresh: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MarkDragAction {
    Add,
    Remove,
}

struct ReplayMousePointer {
    x_percent: String,
    y_percent: String,
    active_click: bool,
}

const ROOM_POINTER_SEND_INTERVAL_MS: f64 = 33.0;

#[derive(Deserialize, Serialize)]
struct SavedGame {
    states: Vec<u8>,
    started_at_ms: f64,
    completed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    completed_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    completed_seconds: Option<u64>,
}

#[derive(Clone)]
struct WindowPointerUpListener {
    _closure: Rc<Closure<dyn FnMut(web_sys::PointerEvent)>>,
}

#[derive(Clone)]
struct RoomWindowPointerUpListener {
    _closure: Rc<Closure<dyn FnMut(web_sys::PointerEvent)>>,
}

type EventClosure<T> = Rc<Closure<dyn FnMut(T)>>;

const REPLAY_SCRUBBER_ID: &str = "replay-scrubber";
const ROOM_BOARD_ID: &str = "room-board";
const MOUSE_SAMPLE_INTERVAL_MS: u32 = 33;

#[derive(Clone)]
struct RoomEntry {
    name: String,
    auto_join: bool,
}

#[derive(Clone)]
struct RoomConnection {
    socket: Option<Rc<web_sys::WebSocket>>,
    _on_message: Option<EventClosure<web_sys::MessageEvent>>,
    _on_open: Option<EventClosure<web_sys::Event>>,
    _on_error: Option<EventClosure<web_sys::ErrorEvent>>,
    _on_close: Option<EventClosure<web_sys::CloseEvent>>,
}

impl RoomConnection {
    fn connect(
        slug: &str,
        player_id: &str,
        player_name: &str,
        mut snapshot: Signal<RoomSnapshot>,
        mut status: Signal<String>,
    ) -> Self {
        let Some(url) = room_ws_url(slug, player_id, player_name) else {
            status.set("Could not build room socket URL.".to_string());
            return Self::disconnected();
        };
        let Ok(socket) = web_sys::WebSocket::new(&url) else {
            status.set("Could not connect to this room.".to_string());
            return Self::disconnected();
        };

        let on_message = Closure::wrap(Box::new(move |event: web_sys::MessageEvent| {
            let Some(raw) = event.data().as_string() else {
                return;
            };
            match serde_json::from_str::<RoomServerMessage>(&raw) {
                Ok(RoomServerMessage::Snapshot { snapshot: next }) => {
                    snapshot.set(next);
                    status.set("Connected".to_string());
                }
                Ok(RoomServerMessage::PointerUpdate { player_id, pointer }) => {
                    update_snapshot_pointer(&mut snapshot.write(), &player_id, pointer);
                }
                Ok(RoomServerMessage::RecordingFrame { player_id, frame }) => {
                    append_snapshot_recording_frame(&mut snapshot.write(), &player_id, frame);
                }
                Ok(RoomServerMessage::MouseRecordingChunk {
                    player_id,
                    recording,
                }) => {
                    append_snapshot_mouse_recording(&mut snapshot.write(), &player_id, recording);
                }
                Ok(RoomServerMessage::Error { message }) => {
                    status.set(message);
                }
                Err(error) => {
                    status.set(format!("Room update failed: {error}"));
                }
            }
        }) as Box<dyn FnMut(_)>);
        socket.set_onmessage(Some(on_message.as_ref().unchecked_ref()));

        let mut open_status = status;
        let on_open = Closure::wrap(Box::new(move |_event: web_sys::Event| {
            open_status.set("Connected".to_string());
        }) as Box<dyn FnMut(_)>);
        socket.set_onopen(Some(on_open.as_ref().unchecked_ref()));

        let mut error_status = status;
        let on_error = Closure::wrap(Box::new(move |_event: web_sys::ErrorEvent| {
            error_status.set("Room connection error.".to_string());
        }) as Box<dyn FnMut(_)>);
        socket.set_onerror(Some(on_error.as_ref().unchecked_ref()));

        let mut close_status = status;
        let on_close = Closure::wrap(Box::new(move |_event: web_sys::CloseEvent| {
            close_status.set("Disconnected".to_string());
        }) as Box<dyn FnMut(_)>);
        socket.set_onclose(Some(on_close.as_ref().unchecked_ref()));

        Self {
            socket: Some(Rc::new(socket)),
            _on_message: Some(Rc::new(on_message)),
            _on_open: Some(Rc::new(on_open)),
            _on_error: Some(Rc::new(on_error)),
            _on_close: Some(Rc::new(on_close)),
        }
    }

    fn disconnected() -> Self {
        Self {
            socket: None,
            _on_message: None,
            _on_open: None,
            _on_error: None,
            _on_close: None,
        }
    }

    fn send(&self, message: &RoomClientMessage) -> bool {
        let Some(socket) = &self.socket else {
            return false;
        };
        let Ok(raw) = serde_json::to_string(message) else {
            return false;
        };
        socket.send_with_str(&raw).is_ok()
    }
}

impl WindowPointerUpListener {
    fn new(mut game: Signal<GameState>) -> Self {
        let closure = Closure::wrap(Box::new(move |event: web_sys::PointerEvent| {
            if event.pointer_type() == "mouse" && event.button() == 0 {
                game.write().finish_mark_drag(None);
            }
        }) as Box<dyn FnMut(_)>);

        if let Some(window) = web_sys::window() {
            let _ = window
                .add_event_listener_with_callback("pointerup", closure.as_ref().unchecked_ref());
        }

        Self {
            _closure: Rc::new(closure),
        }
    }
}

impl RoomWindowPointerUpListener {
    fn new(mut game: Signal<Option<GameState>>) -> Self {
        let closure = Closure::wrap(Box::new(move |event: web_sys::PointerEvent| {
            if event.pointer_type() == "mouse" && event.button() == 0 {
                if let Some(game) = game.write().as_mut() {
                    game.finish_mark_drag(None);
                }
            }
        }) as Box<dyn FnMut(_)>);

        if let Some(window) = web_sys::window() {
            let _ = window
                .add_event_listener_with_callback("pointerup", closure.as_ref().unchecked_ref());
        }

        Self {
            _closure: Rc::new(closure),
        }
    }
}

impl GameState {
    fn new(bootstrap: GameBootstrap) -> Self {
        let validation = validate_solution(&bootstrap.puzzle, &[]);
        let storage_key = format!("queensgame:9x9:{}", bootstrap.puzzle.id);
        let mut game = Self {
            cells: build_cells(&bootstrap.puzzle),
            states: vec![CellState::Empty; bootstrap.puzzle.size * bootstrap.puzzle.size],
            puzzle: bootstrap.puzzle,
            puzzle_nav: bootstrap.puzzle_nav,
            total: bootstrap.total,
            mode: Mode::Queen,
            storage_key: Some(storage_key),
            room_started_at_ms: None,
            recording: None,
            mouse_recording: None,
            mouse_recording_sent: false,
            last_mouse_sample_ms: None,
            recording_frames_sent: 0,
            mouse_samples_sent: 0,
            mouse_events_sent: 0,
            history: Vec::new(),
            started_at_ms: now_ms(),
            completed: false,
            completed_ms: 0,
            validation,
            finish_notified: false,
            mark_drag: None,
            win_visible: false,
        };

        game.load();
        game.refresh_auto_marks();
        game.revalidate(false);
        game
    }

    fn new_room(puzzle: Puzzle, room_started_at_ms: Option<u64>) -> Self {
        let validation = validate_solution(&puzzle, &[]);
        let mut game = Self {
            cells: build_cells(&puzzle),
            states: vec![CellState::Empty; puzzle.size * puzzle.size],
            puzzle,
            puzzle_nav: Vec::new(),
            total: 0,
            mode: Mode::Queen,
            storage_key: None,
            room_started_at_ms,
            recording: Some(RoomRecording { frames: Vec::new() }),
            mouse_recording: Some(RoomMouseRecording {
                samples: Vec::new(),
                events: Vec::new(),
            }),
            mouse_recording_sent: false,
            last_mouse_sample_ms: None,
            recording_frames_sent: 0,
            mouse_samples_sent: 0,
            mouse_events_sent: 0,
            history: Vec::new(),
            started_at_ms: now_ms(),
            completed: false,
            completed_ms: 0,
            validation,
            finish_notified: false,
            mark_drag: None,
            win_visible: false,
        };
        game.revalidate(false);
        game.record_current_frame();
        game
    }

    fn index_for(&self, row: usize, col: usize) -> usize {
        row * self.puzzle.size + col
    }

    fn queens(&self) -> Vec<[usize; 2]> {
        self.states
            .iter()
            .enumerate()
            .filter_map(|(index, state)| {
                if *state == CellState::Queen {
                    Some([index / self.puzzle.size, index % self.puzzle.size])
                } else {
                    None
                }
            })
            .collect()
    }

    fn recording(&self) -> RoomRecording {
        self.recording.clone().unwrap_or_else(|| RoomRecording {
            frames: vec![self.recording_frame()],
        })
    }

    fn mouse_recording(&self) -> RoomMouseRecording {
        self.mouse_recording
            .clone()
            .unwrap_or_else(|| RoomMouseRecording {
                samples: Vec::new(),
                events: Vec::new(),
            })
    }

    fn recording_frame(&self) -> RoomRecordingFrame {
        RoomRecordingFrame {
            elapsed_ms: ((now_ms() - self.started_at_ms).max(0.0)).floor() as u64,
            states: self
                .states
                .iter()
                .map(|state| state.storage_code())
                .collect(),
        }
    }

    fn record_current_frame(&mut self) {
        let frame = self.recording_frame();
        if let Some(recording) = &mut self.recording {
            recording.frames.push(frame);
        }
    }

    fn unsent_recording_frames(&self) -> Vec<RoomRecordingFrame> {
        if self.completed {
            return Vec::new();
        }

        self.recording
            .as_ref()
            .map(|recording| {
                recording
                    .frames
                    .iter()
                    .skip(self.recording_frames_sent)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    fn mark_recording_frames_sent(&mut self, sent_count: usize) {
        self.recording_frames_sent = self.recording_frames_sent.saturating_add(sent_count);
    }

    fn unsent_mouse_recording(&self) -> Option<RoomMouseRecording> {
        if self.completed {
            return None;
        }

        let recording = self.mouse_recording.as_ref()?;
        let samples = recording
            .samples
            .iter()
            .skip(self.mouse_samples_sent)
            .copied()
            .collect::<Vec<_>>();
        let events = recording
            .events
            .iter()
            .skip(self.mouse_events_sent)
            .copied()
            .collect::<Vec<_>>();
        if samples.is_empty() && events.is_empty() {
            return None;
        }

        Some(RoomMouseRecording { samples, events })
    }

    fn mark_mouse_recording_sent(&mut self, sent_samples: usize, sent_events: usize) {
        self.mouse_samples_sent = self.mouse_samples_sent.saturating_add(sent_samples);
        self.mouse_events_sent = self.mouse_events_sent.saturating_add(sent_events);
    }

    fn recording_elapsed_ms(&self) -> u32 {
        ((now_ms() - self.started_at_ms).max(0.0)).floor() as u32
    }

    fn record_mouse_sample(&mut self, x: u16, y: u16, force: bool) {
        if self.completed {
            return;
        }
        let elapsed_ms = self.recording_elapsed_ms();
        if !force
            && self
                .last_mouse_sample_ms
                .map(|last_ms| elapsed_ms.saturating_sub(last_ms) < MOUSE_SAMPLE_INTERVAL_MS)
                .unwrap_or(false)
        {
            return;
        }
        if let Some(recording) = &mut self.mouse_recording {
            recording.samples.push(RoomMouseSample(elapsed_ms, x, y));
            self.last_mouse_sample_ms = Some(elapsed_ms);
        }
    }

    fn record_mouse_event(&mut self, kind: u8, x: u16, y: u16, cell_index: Option<usize>) {
        if self.completed {
            return;
        }
        self.record_mouse_sample(x, y, true);
        let elapsed_ms = self.recording_elapsed_ms();
        let cell_index = cell_index.and_then(|index| u16::try_from(index).ok());
        if let Some(recording) = &mut self.mouse_recording {
            recording
                .events
                .push(RoomMouseEvent(elapsed_ms, kind, x, y, cell_index));
        }
    }

    fn elapsed_ms(&self) -> u64 {
        if self.completed {
            self.completed_ms
        } else {
            ((now_ms() - self.started_at_ms).max(0.0)).floor() as u64
        }
    }

    fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
    }

    fn load(&mut self) {
        let Some(storage_key) = self.storage_key.as_deref() else {
            return;
        };
        let Some(storage) = local_storage() else {
            return;
        };
        let Ok(Some(raw)) = storage.get_item(storage_key) else {
            return;
        };
        let Ok(saved) = serde_json::from_str::<SavedGame>(&raw) else {
            return;
        };
        if saved.states.len() != self.states.len() {
            return;
        }

        self.states = saved
            .states
            .into_iter()
            .map(CellState::from_storage_code)
            .collect();
        self.started_at_ms = if saved.started_at_ms.is_finite() && saved.started_at_ms > 0.0 {
            saved.started_at_ms
        } else {
            now_ms()
        };
        self.completed = saved.completed;
        self.completed_ms = saved
            .completed_ms
            .or_else(|| {
                saved
                    .completed_seconds
                    .map(|seconds| seconds.saturating_mul(1000))
            })
            .unwrap_or(0);
    }

    fn save(&self) {
        let Some(storage_key) = self.storage_key.as_deref() else {
            return;
        };
        let Some(storage) = local_storage() else {
            return;
        };
        let saved = SavedGame {
            states: self
                .states
                .iter()
                .map(|state| state.storage_code())
                .collect(),
            started_at_ms: self.started_at_ms,
            completed: self.completed,
            completed_ms: Some(self.completed_ms),
            completed_seconds: None,
        };
        if let Ok(raw) = serde_json::to_string(&saved) {
            let _ = storage.set_item(storage_key, &raw);
        }
    }

    fn push_history(&mut self) {
        self.history.push(self.states.clone());
        if self.history.len() > 100 {
            self.history.remove(0);
        }
    }

    fn refresh_auto_marks(&mut self) {
        for state in &mut self.states {
            if *state == CellState::AutoMark {
                *state = CellState::Empty;
            }
        }

        for [queen_row, queen_col] in self.queens() {
            for row in 0..self.puzzle.size {
                for col in 0..self.puzzle.size {
                    let index = self.index_for(row, col);
                    if self.states[index] == CellState::Empty
                        && invalidated_by_queen(&self.puzzle, queen_row, queen_col, row, col)
                    {
                        self.states[index] = CellState::AutoMark;
                    }
                }
            }
        }
    }

    fn revalidate(&mut self, show_win: bool) {
        let validation = validate_solution(&self.puzzle, &self.queens());

        if validation.complete && !self.completed {
            self.completed_ms = self.elapsed_ms();
            self.completed = true;
            self.win_visible = show_win;
        } else if !validation.complete {
            self.completed = false;
            self.completed_ms = 0;
            self.finish_notified = false;
            self.win_visible = false;
        }

        self.validation = validation;
    }

    fn after_change(&mut self) {
        self.completed = false;
        self.completed_ms = 0;
        self.win_visible = false;
        self.revalidate(true);
        self.save();
        self.record_current_frame();
    }

    fn commit_state(&mut self, index: usize, next_state: CellState, refresh_auto_marks: bool) {
        if self.states[index] == next_state {
            return;
        }

        self.push_history();
        self.states[index] = next_state;
        if refresh_auto_marks {
            self.refresh_auto_marks();
        }
        self.after_change();
    }

    fn toggle_mark(&mut self, index: usize) {
        let current_state = self.states[index];
        let next_state = if current_state.is_marked() {
            CellState::Empty
        } else {
            CellState::Mark
        };
        self.commit_state(index, next_state, current_state == CellState::Queen);
    }

    fn toggle_queen(&mut self, index: usize) {
        let next_state = if self.states[index] == CellState::Queen {
            CellState::Empty
        } else {
            CellState::Queen
        };
        self.commit_state(index, next_state, true);
    }

    fn apply_mode_action(&mut self, index: usize) {
        let current_state = self.states[index];
        let next_state = match self.mode {
            Mode::Clear => CellState::Empty,
            Mode::Mark => {
                if current_state.is_marked() {
                    CellState::Empty
                } else {
                    CellState::Mark
                }
            }
            Mode::Queen => {
                if current_state == CellState::Queen {
                    CellState::Empty
                } else {
                    CellState::Queen
                }
            }
        };
        self.commit_state(
            index,
            next_state,
            current_state == CellState::Queen || next_state == CellState::Queen,
        );
    }

    fn start_mark_drag(&mut self, index: usize) {
        let action = if self.states[index].is_marked() {
            MarkDragAction::Remove
        } else {
            MarkDragAction::Add
        };

        self.mark_drag = Some(MarkDrag {
            start_index: index,
            action,
            moved: false,
            changed: false,
            history_started: false,
            needs_auto_refresh: false,
        });
    }

    fn apply_drag_mark(&mut self, index: usize) {
        let Some(mut drag) = self.mark_drag else {
            return;
        };
        let current_state = self.states[index];
        let next_state = match drag.action {
            MarkDragAction::Add => {
                if current_state == CellState::Mark {
                    return;
                }
                CellState::Mark
            }
            MarkDragAction::Remove => {
                if !current_state.is_marked() {
                    return;
                }
                CellState::Empty
            }
        };

        if current_state == next_state {
            return;
        }

        if !drag.history_started {
            self.push_history();
            drag.history_started = true;
        }

        if current_state == CellState::Queen {
            drag.needs_auto_refresh = true;
        }

        self.states[index] = next_state;
        drag.changed = true;
        self.mark_drag = Some(drag);
        self.record_current_frame();
    }

    fn drag_mark_cell(&mut self, index: usize) {
        let Some(mut drag) = self.mark_drag else {
            return;
        };

        if !drag.moved {
            drag.moved = true;
            let start_index = drag.start_index;
            self.mark_drag = Some(drag);
            self.apply_drag_mark(start_index);
        }

        self.apply_drag_mark(index);
    }

    fn finish_mark_drag(&mut self, index: Option<usize>) {
        let Some(drag) = self.mark_drag.take() else {
            return;
        };

        if !drag.moved {
            self.toggle_mark(index.unwrap_or(drag.start_index));
            return;
        }

        if !drag.changed {
            return;
        }

        if drag.needs_auto_refresh {
            self.refresh_auto_marks();
        }

        self.after_change();
    }

    fn undo(&mut self) {
        let Some(previous) = self.history.pop() else {
            return;
        };
        self.states = previous;
        self.completed = false;
        self.completed_ms = 0;
        self.finish_notified = false;
        self.mouse_recording_sent = false;
        self.win_visible = false;
        self.revalidate(false);
        self.save();
        self.record_current_frame();
    }

    fn reset(&mut self) {
        self.push_history();
        self.states = vec![CellState::Empty; self.puzzle.size * self.puzzle.size];
        if self.recording.is_none() {
            self.started_at_ms = now_ms();
        }
        self.completed = false;
        self.completed_ms = 0;
        self.finish_notified = false;
        self.mouse_recording_sent = false;
        self.win_visible = false;
        self.mark_drag = None;
        self.revalidate(false);
        self.save();
        self.record_current_frame();
    }

    fn close_win(&mut self) {
        self.win_visible = false;
    }
}

impl MinesweeperGameState {
    fn new(bootstrap: MinesweeperBootstrap) -> Self {
        Self {
            board: MinesweeperBoard::new_no_guess(
                bootstrap.width,
                bootstrap.height,
                bootstrap.mines,
                seed(),
            ),
            started_at_ms: None,
            elapsed_ms: 0,
            face_down: false,
        }
    }

    fn reset(&mut self) {
        let width = self.board.width;
        let height = self.board.height;
        let mines = self.board.mines;
        *self = Self {
            board: MinesweeperBoard::new_no_guess(width, height, mines, seed()),
            started_at_ms: None,
            elapsed_ms: 0,
            face_down: false,
        };
    }

    fn reveal(&mut self, index: usize) {
        let result = self.board.reveal(index);
        if result.started {
            self.started_at_ms = Some(now_ms());
        }
        if result.changed {
            self.tick();
        }
    }

    fn toggle_mark(&mut self, index: usize) {
        let _ = self.board.toggle_mark(index);
    }

    fn chord(&mut self, index: usize) {
        let result = self.board.chord(index);
        if result.changed {
            self.tick();
        }
    }

    fn set_face_down(&mut self, face_down: bool) {
        if matches!(
            self.board.status,
            MinesweeperStatus::Ready | MinesweeperStatus::Playing
        ) {
            self.face_down = face_down;
        }
    }

    fn tick(&mut self) {
        if self.board.status == MinesweeperStatus::Playing {
            if let Some(started_at_ms) = self.started_at_ms {
                self.elapsed_ms = (now_ms() - started_at_ms).max(0.0).floor() as u64;
            }
        }
    }

    fn timer_seconds(&self) -> u64 {
        (self.elapsed_ms / 1000).min(999)
    }
}

#[component]
fn Game(bootstrap: GameBootstrap) -> Element {
    let mut tick = use_signal(|| 0u64);
    let mut game = use_signal(|| GameState::new(bootstrap));
    let _window_pointer_up = use_hook(move || WindowPointerUpListener::new(game));
    let _timer = use_future(move || async move {
        loop {
            TimeoutFuture::new(100).await;
            tick += 1;
        }
    });

    let snapshot = game.read().clone();
    let _ = *tick.read();

    let size = snapshot.puzzle.size;
    let has_prev = snapshot.puzzle.id > 1;
    let prev_id = snapshot.puzzle.id.saturating_sub(1);
    let has_next = snapshot.puzzle.id < snapshot.total;
    let next_id = snapshot.puzzle.id + 1;
    let conflict_cells = snapshot
        .validation
        .conflict_cells
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let status = validation_status(&snapshot.validation, size);
    let elapsed = format_duration_ms(snapshot.elapsed_ms());
    let win_time = format!("Finished in {}.", format_duration_ms(snapshot.completed_ms));

    rsx! {
        main { class: "game-page",
            section { class: "game-shell", aria_labelledby: "game-title",
                div { class: "game-toolbar",
                    div {
                        p { class: "eyebrow", "9x9 puzzle {snapshot.puzzle.id} of {snapshot.total}" }
                        h1 { id: "game-title", "Queens Puzzle #{snapshot.puzzle.id}" }
                    }
                    div { class: "timer-box", aria_live: "polite",
                        span { class: "timer-label", "Time" }
                        span { id: "timer", "{elapsed}" }
                    }
                }
                div { class: "controls-row", aria_label: "Game controls",
                    div { class: "segmented", role: "group", aria_label: "Cell mode",
                        button {
                            r#type: "button",
                            class: mode_button_class(snapshot.mode, Mode::Queen),
                            onclick: move |_| game.write().set_mode(Mode::Queen),
                            "Queen"
                        }
                        button {
                            r#type: "button",
                            class: mode_button_class(snapshot.mode, Mode::Mark),
                            onclick: move |_| game.write().set_mode(Mode::Mark),
                            "Mark"
                        }
                        button {
                            r#type: "button",
                            class: mode_button_class(snapshot.mode, Mode::Clear),
                            onclick: move |_| game.write().set_mode(Mode::Clear),
                            "Clear"
                        }
                    }
                    div { class: "tool-buttons",
                        button {
                            r#type: "button",
                            class: "tool-button",
                            title: "Undo last move",
                            onclick: move |_| game.write().undo(),
                            "Undo"
                        }
                        button {
                            r#type: "button",
                            class: "tool-button",
                            title: "Highlight conflicts",
                            onclick: move |_| game.write().revalidate(false),
                            "Check"
                        }
                        button {
                            r#type: "button",
                            class: "tool-button",
                            title: "Reset this puzzle",
                            onclick: move |_| game.write().reset(),
                            "Reset"
                        }
                    }
                }
                div { class: "board-wrap",
                    div {
                        class: "board",
                        role: "grid",
                        aria_label: "Queens board",
                        style: "--board-size: {size}",
                        for cell in snapshot.cells.iter() {
                            {
                                let index = snapshot.index_for(cell.row, cell.col);
                                let state = snapshot.states[index];
                                let class_name = cell_class(cell, state, &conflict_cells);
                                let aria = cell_aria(cell, state);

                                rsx! {
                                    button {
                                        r#type: "button",
                                        class: "{class_name}",
                                        style: "--cell-color: {cell.color}",
                                        "data-row": "{cell.row}",
                                        "data-col": "{cell.col}",
                                        "data-region": "{cell.region}",
                                        aria_label: "{aria}",
                                        role: "gridcell",
                                        onpointerdown: move |event| {
                                            let data = event.data();
                                            if data.pointer_type() == "mouse"
                                                && data.trigger_button() == Some(MouseButton::Primary)
                                            {
                                                event.prevent_default();
                                                game.write().start_mark_drag(index);
                                            }
                                        },
                                        onpointerenter: move |event| {
                                            let data = event.data();
                                            if data.pointer_type() == "mouse"
                                                && data.held_buttons().contains(MouseButton::Primary)
                                            {
                                                game.write().drag_mark_cell(index);
                                            } else {
                                                game.write().finish_mark_drag(None);
                                            }
                                        },
                                        onpointerup: move |event| {
                                            let data = event.data();
                                            if data.pointer_type() == "mouse" {
                                                if data.trigger_button() == Some(MouseButton::Primary) {
                                                    event.prevent_default();
                                                    game.write().finish_mark_drag(Some(index));
                                                }
                                            } else {
                                                game.write().apply_mode_action(index);
                                            }
                                        },
                                        oncontextmenu: move |event| {
                                            event.prevent_default();
                                            game.write().toggle_queen(index);
                                        },
                                        onkeydown: move |event| {
                                            let code = event.data().code();
                                            match code {
                                                Code::Space | Code::Enter => {
                                                    event.prevent_default();
                                                    game.write().apply_mode_action(index);
                                                }
                                                Code::KeyQ => {
                                                    event.prevent_default();
                                                    game.write().toggle_queen(index);
                                                }
                                                Code::KeyX => {
                                                    event.prevent_default();
                                                    game.write().toggle_mark(index);
                                                }
                                                Code::Backspace | Code::Delete => {
                                                    event.prevent_default();
                                                    let refresh = game.read().states[index] == CellState::Queen;
                                                    game.write().commit_state(index, CellState::Empty, refresh);
                                                }
                                                _ => {}
                                            }
                                        },
                                        span { class: "cell-symbol", aria_hidden: "true" }
                                    }
                                }
                            }
                        }
                    }
                }
                div { class: "status-strip", aria_live: "polite",
                    span { id: "queen-count", "{snapshot.validation.queen_count} / {snapshot.validation.expected_queens} queens" }
                    span { id: "rule-status", "{status}" }
                }
                div { class: "rule-panel",
                    h2 { "Rules" }
                    p { "Place exactly one queen in each row, column, and colored region. Queens may not touch diagonally." }
                }
                div { class: "puzzle-actions",
                    if has_prev {
                        a { class: "nav-button", href: "/puzzles/9x9/{prev_id}", "Previous" }
                    }
                    if has_next {
                        a { class: "nav-button primary", href: "/puzzles/9x9/{next_id}", "Next Puzzle" }
                    }
                }
            }
            aside { class: "side-panel", aria_label: "Puzzle selector",
                div { class: "selector-header",
                    p { class: "eyebrow", "Archive" }
                    h2 { "9x9 puzzles" }
                }
                div { class: "puzzle-grid",
                    for nav in snapshot.puzzle_nav.iter() {
                        a {
                            class: if nav.active { "active" } else { "" },
                            href: "/puzzles/9x9/{nav.id}",
                            "{nav.id}"
                        }
                    }
                }
            }
        }
        div {
            class: "win-dialog",
            hidden: !snapshot.win_visible,
            onclick: move |_| game.write().close_win(),
            div {
                class: "win-panel",
                role: "dialog",
                aria_modal: "true",
                aria_labelledby: "win-title",
                onclick: move |event| event.stop_propagation(),
                p { class: "eyebrow", "Solved" }
                h2 { id: "win-title", "Puzzle complete" }
                p { "{win_time}" }
                div { class: "dialog-actions",
                    button {
                        r#type: "button",
                        class: "tool-button",
                        onclick: move |_| game.write().close_win(),
                        "Keep Playing"
                    }
                    if has_next {
                        a { class: "nav-button primary", href: "/puzzles/9x9/{next_id}", "Next Puzzle" }
                    }
                }
            }
        }
    }
}

#[component]
fn MinesweeperApp(bootstrap: MinesweeperBootstrap) -> Element {
    let mut game = use_signal(|| MinesweeperGameState::new(bootstrap));
    let mut chord_target = use_signal(|| None::<usize>);
    let mut pressed_cells = use_signal(BTreeSet::<usize>::new);
    let mut left_mouse_down = use_signal(|| false);
    let mut right_mouse_down = use_signal(|| false);
    let mut suppress_next_secondary_up = use_signal(|| false);
    let _timer = use_future(move || async move {
        loop {
            TimeoutFuture::new(100).await;
            game.write().tick();
        }
    });

    let snapshot = game.read().clone();
    let mine_counter = format_minesweeper_counter(snapshot.board.remaining_mines());
    let timer = format_minesweeper_counter(snapshot.timer_seconds() as i32);
    let face = minesweeper_face(&snapshot);
    let pressed_cell_set = pressed_cells.read().clone();

    rsx! {
        main { class: "minesweeper-page",
            section { class: "minesweeper-window", aria_labelledby: "minesweeper-title",
                div { class: "minesweeper-titlebar",
                    div {
                        p { class: "eyebrow", "Expert" }
                        h1 { id: "minesweeper-title", "Minesweeper" }
                    }
                    a { class: "nav-button", href: "/puzzles/9x9/1", "Queens" }
                }
                div { class: "ms-shell",
                    div { class: "ms-panel ms-header", aria_label: "Minesweeper status",
                        MinesweeperLed { label: "Mines remaining", value: mine_counter }
                        button {
                            r#type: "button",
                            class: "ms-face",
                            title: "New game",
                            aria_label: "New game",
                            onclick: move |_| game.write().reset(),
                            "{face}"
                        }
                        MinesweeperLed { label: "Elapsed time", value: timer }
                    }
                    div {
                        class: "ms-board",
                        role: "grid",
                        aria_label: "Expert Minesweeper board",
                        style: "--mine-cols: {snapshot.board.width}",
                        onpointerleave: move |_| {
                            game.write().set_face_down(false);
                            chord_target.set(None);
                            pressed_cells.set(BTreeSet::new());
                            left_mouse_down.set(false);
                            right_mouse_down.set(false);
                            suppress_next_secondary_up.set(false);
                        },
                        for (index, cell) in snapshot.board.cells.iter().enumerate() {
                            {
                                let pressed = pressed_cell_set.contains(&index);
                                let class_name = minesweeper_cell_class(
                                    cell,
                                    snapshot.board.status,
                                    pressed,
                                );
                                let text = minesweeper_cell_text(cell, pressed);
                                let aria = minesweeper_cell_aria(index, cell, &snapshot.board);

                                rsx! {
                                    button {
                                        r#type: "button",
                                        class: "{class_name}",
                                        role: "gridcell",
                                        aria_label: "{aria}",
                                        onpointerdown: move |event| {
                                            let data = event.data();
                                            if data.pointer_type() != "mouse" {
                                                return;
                                            }
                                            let primary = data.trigger_button() == Some(MouseButton::Primary);
                                            let secondary = data.trigger_button() == Some(MouseButton::Secondary);
                                            if primary {
                                                event.prevent_default();
                                                left_mouse_down.set(true);
                                            }
                                            if secondary {
                                                event.prevent_default();
                                                right_mouse_down.set(true);
                                            }

                                            let both_down = (primary || *left_mouse_down.read())
                                                && (secondary || *right_mouse_down.read());
                                            if both_down || primary {
                                                let chord_press = {
                                                    let state = game.read();
                                                    minesweeper_chord_target(&state.board, index).map(|target| {
                                                        (
                                                            target,
                                                            minesweeper_pressed_neighbors(&state.board, target),
                                                        )
                                                    })
                                                };
                                                if let Some((target, pressed)) = chord_press {
                                                    chord_target.set(Some(target));
                                                    pressed_cells.set(pressed);
                                                    game.write().set_face_down(true);
                                                } else if primary {
                                                    chord_target.set(None);
                                                    pressed_cells.set(BTreeSet::new());
                                                    game.write().set_face_down(true);
                                                }
                                            }
                                        },
                                        onpointerenter: move |event| {
                                            let data = event.data();
                                            if data.pointer_type() != "mouse" || !*left_mouse_down.read() {
                                                return;
                                            }
                                            let chord_press = {
                                                let state = game.read();
                                                minesweeper_chord_target(&state.board, index).map(|target| {
                                                    (
                                                        target,
                                                        minesweeper_pressed_neighbors(&state.board, target),
                                                    )
                                                })
                                            };
                                            if let Some((target, pressed)) = chord_press {
                                                chord_target.set(Some(target));
                                                pressed_cells.set(pressed);
                                                game.write().set_face_down(true);
                                            } else {
                                                chord_target.set(None);
                                                pressed_cells.set(BTreeSet::new());
                                            }
                                        },
                                        onpointerup: move |event| {
                                            let data = event.data();
                                            if data.pointer_type() == "mouse" {
                                                if data.trigger_button() == Some(MouseButton::Primary) {
                                                    event.prevent_default();
                                                    left_mouse_down.set(false);
                                                    game.write().set_face_down(false);
                                                    pressed_cells.set(BTreeSet::new());
                                                    if *chord_target.read() == Some(index) {
                                                        if *right_mouse_down.read() {
                                                            suppress_next_secondary_up.set(true);
                                                        }
                                                        game.write().chord(index);
                                                        chord_target.set(None);
                                                    } else if minesweeper_chord_target(&game.read().board, index).is_some() {
                                                        game.write().chord(index);
                                                    } else {
                                                        game.write().reveal(index);
                                                    }
                                                } else if data.trigger_button() == Some(MouseButton::Secondary) {
                                                    event.prevent_default();
                                                    right_mouse_down.set(false);
                                                    game.write().set_face_down(false);
                                                    pressed_cells.set(BTreeSet::new());
                                                    if *suppress_next_secondary_up.read() {
                                                        suppress_next_secondary_up.set(false);
                                                        chord_target.set(None);
                                                    } else if *chord_target.read() == Some(index) {
                                                        game.write().chord(index);
                                                        chord_target.set(None);
                                                    } else if !*left_mouse_down.read() {
                                                        game.write().toggle_mark(index);
                                                    }
                                                }
                                            } else {
                                                game.write().reveal(index);
                                            }
                                        },
                                        ondoubleclick: move |event| {
                                            event.prevent_default();
                                            game.write().chord(index);
                                        },
                                        oncontextmenu: move |event| {
                                            event.prevent_default();
                                        },
                                        onkeydown: move |event| {
                                            let code = event.data().code();
                                            match code {
                                                Code::Space | Code::Enter => {
                                                    event.prevent_default();
                                                    game.write().reveal(index);
                                                }
                                                Code::KeyF => {
                                                    event.prevent_default();
                                                    game.write().toggle_mark(index);
                                                }
                                                Code::KeyC => {
                                                    event.prevent_default();
                                                    game.write().chord(index);
                                                }
                                                _ => {}
                                            }
                                        },
                                        span { class: "ms-cell-symbol", aria_hidden: "true", "{text}" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn MinesweeperLed(label: String, value: String) -> Element {
    rsx! {
        div { class: "ms-led", aria_label: "{label}",
            "{value}"
        }
    }
}

#[component]
fn RoomApp(bootstrap: RoomBootstrap) -> Element {
    let player_id = use_hook(load_or_create_player_id);
    let room_entry = use_hook(initial_room_entry);
    let mut pending_name = use_signal(|| room_entry.name.clone());
    let mut name_error = use_signal(String::new);
    let room_snapshot = use_signal(|| bootstrap.snapshot.clone());
    let mut race_game = use_signal(|| None::<GameState>);
    let mut connection_status = use_signal(|| {
        if room_entry.auto_join {
            "Connecting".to_string()
        } else {
            "Not joined".to_string()
        }
    });
    let mut connection = use_signal({
        let slug = bootstrap.slug.clone();
        let player_id = player_id.clone();
        let room_entry = room_entry.clone();
        move || {
            if room_entry.auto_join {
                save_player_name(&room_entry.name);
                Some(RoomConnection::connect(
                    &slug,
                    &player_id,
                    &room_entry.name,
                    room_snapshot,
                    connection_status,
                ))
            } else {
                None
            }
        }
    });
    let mut tick = use_signal(|| 0u64);
    let mut replay_scrub_ms = use_signal(|| None::<u64>);
    let mut replay_started_at_ms = use_signal(|| None::<(u64, f64)>);
    let mut replay_manual_player_ids = use_signal(Vec::<String>::new);
    let _window_pointer_up = use_hook(move || RoomWindowPointerUpListener::new(race_game));
    let _timer = use_future(move || async move {
        loop {
            TimeoutFuture::new(100).await;
            tick += 1;
        }
    });

    use_effect(move || {
        let snapshot = room_snapshot.read().clone();
        let puzzle = snapshot.puzzle.clone();
        let room_started_at_ms = snapshot.phase.race_started_at_ms();
        let Some(puzzle) = puzzle else {
            race_game.set(None);
            return;
        };
        let needs_game = race_game
            .read()
            .as_ref()
            .map(|game| {
                game.puzzle.id != puzzle.id || game.room_started_at_ms != room_started_at_ms
            })
            .unwrap_or(true);
        if needs_game {
            race_game.set(Some(GameState::new_room(puzzle, room_started_at_ms)));
        }
    });

    use_effect({
        move || {
            let _ = *tick.read();
            let pending_frames = race_game
                .read()
                .as_ref()
                .map(GameState::unsent_recording_frames)
                .unwrap_or_default();
            let pending_mouse = race_game
                .read()
                .as_ref()
                .and_then(GameState::unsent_mouse_recording);
            if pending_frames.is_empty() && pending_mouse.is_none() {
                return;
            }

            let (sent_frame_count, sent_mouse_counts) = connection
                .read()
                .as_ref()
                .map(|connection| {
                    let sent_frame_count = pending_frames
                        .into_iter()
                        .map(|frame| RoomClientMessage::RecordingFrame { frame })
                        .take_while(|message| connection.send(message))
                        .count();
                    let sent_mouse_counts = pending_mouse.and_then(|recording| {
                        let sample_count = recording.samples.len();
                        let event_count = recording.events.len();
                        connection
                            .send(&RoomClientMessage::MouseRecordingChunk { recording })
                            .then_some((sample_count, event_count))
                    });
                    (sent_frame_count, sent_mouse_counts)
                })
                .unwrap_or((0, None));
            if sent_frame_count > 0 || sent_mouse_counts.is_some() {
                if let Some(game) = race_game.write().as_mut() {
                    game.mark_recording_frames_sent(sent_frame_count);
                    if let Some((sample_count, event_count)) = sent_mouse_counts {
                        game.mark_mouse_recording_sent(sample_count, event_count);
                    }
                }
            }
        }
    });

    use_effect({
        move || {
            let finish_submission = race_game.read().as_ref().and_then(|game| {
                if game.completed && !game.finish_notified {
                    Some((game.queens(), game.recording()))
                } else {
                    None
                }
            });
            let Some((queens, recording)) = finish_submission else {
                return;
            };
            let sent = connection
                .read()
                .as_ref()
                .map(|connection| connection.send(&RoomClientMessage::Finish { queens, recording }))
                .unwrap_or(false);
            if sent {
                if let Some(game) = race_game.write().as_mut() {
                    game.finish_notified = true;
                }
            }
        }
    });

    use_effect({
        move || {
            let mouse_submission = race_game.read().as_ref().and_then(|game| {
                if game.completed && game.finish_notified && !game.mouse_recording_sent {
                    Some(game.mouse_recording())
                } else {
                    None
                }
            });
            let Some(recording) = mouse_submission else {
                return;
            };
            let sent = connection
                .read()
                .as_ref()
                .map(|connection| connection.send(&RoomClientMessage::MouseRecording { recording }))
                .unwrap_or(false);
            if sent {
                if let Some(game) = race_game.write().as_mut() {
                    game.mouse_recording_sent = true;
                }
            }
        }
    });

    let replay_player_id = player_id.clone();
    use_effect(move || {
        let snapshot = room_snapshot.read().clone();
        let my_done = snapshot
            .players
            .iter()
            .find(|player| player.id == replay_player_id)
            .map(room_player_done)
            .unwrap_or(false);
        match snapshot.phase {
            RoomPhase::Complete { started_at_ms } => {
                let replay_start = *replay_started_at_ms.read();
                let current_key = replay_start.map(|(key, _)| key);
                let is_live_clock = replay_start
                    .map(|(_, start_ms)| (start_ms - started_at_ms as f64).abs() < 1.0)
                    .unwrap_or(false);
                if current_key != Some(started_at_ms) || is_live_clock {
                    replay_started_at_ms.set(Some((started_at_ms, now_ms())));
                    replay_scrub_ms.set(None);
                    replay_manual_player_ids.set(Vec::new());
                }
            }
            RoomPhase::Racing { started_at_ms } if my_done => {
                let replay_start = *replay_started_at_ms.read();
                let current_key = replay_start.map(|(key, _)| key);
                if current_key != Some(started_at_ms) {
                    replay_started_at_ms.set(Some((started_at_ms, started_at_ms as f64)));
                    replay_scrub_ms.set(None);
                    replay_manual_player_ids.set(Vec::new());
                }
            }
            RoomPhase::Lobby | RoomPhase::Countdown { .. } | RoomPhase::Racing { .. } => {
                if (*replay_started_at_ms.read()).is_some() {
                    replay_started_at_ms.set(None);
                }
                if (*replay_scrub_ms.read()).is_some() {
                    replay_scrub_ms.set(None);
                }
                let has_manual_selection = !replay_manual_player_ids.read().is_empty();
                if has_manual_selection {
                    replay_manual_player_ids.set(Vec::new());
                }
            }
        }
    });

    let snapshot = room_snapshot.read().clone();
    let status = connection_status.read().clone();
    let is_joined = connection.read().is_some();
    let pending_name_value = pending_name.read().clone();
    let name_error_text = name_error.read().clone();
    let room_tick = *tick.read();
    let me = snapshot
        .players
        .iter()
        .find(|player| player.id == player_id)
        .cloned();
    let my_ready = me.as_ref().map(|player| player.ready).unwrap_or(false);
    let my_finished = me.as_ref().and_then(|player| player.finish_ms).is_some();
    let my_gave_up = me.as_ref().map(|player| player.gave_up).unwrap_or(false);
    let my_done = my_finished || my_gave_up;
    let ready_text = if my_ready { "Not Ready" } else { "Ready" };
    let room_url = current_room_url(&snapshot.slug);
    let choice = puzzle_choice_label(&snapshot.puzzle_choice);
    let can_select = matches!(
        snapshot.phase,
        RoomPhase::Lobby | RoomPhase::Complete { .. }
    );
    let countdown = countdown_label(&snapshot.phase);
    let race_started_at_ms = snapshot.phase.race_started_at_ms();
    let played_puzzle_ids = snapshot
        .played_puzzle_ids
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let leaderboard_players = sorted_leaderboard_players(&snapshot.players);
    let leaderboard_max_total = leaderboard_players
        .iter()
        .map(|player| player.medals.total())
        .max()
        .unwrap_or(0)
        .max(1);
    let live_replay = matches!(snapshot.phase, RoomPhase::Racing { .. }) && my_done;
    let replay_players = if live_replay {
        snapshot
            .players
            .iter()
            .filter(|player| player.id != player_id && !room_player_done(player))
            .cloned()
            .collect::<Vec<_>>()
    } else {
        snapshot.players.clone()
    };
    let live_replay_has_recordings = replay_players
        .iter()
        .any(|player| player.recording.is_some());
    let recorded_replay_duration_ms = replay_duration_ms(&replay_players);
    let live_elapsed_ms = if live_replay {
        race_started_at_ms
            .map(|started_at_ms| (now_ms() as u64).saturating_sub(started_at_ms))
            .unwrap_or(0)
    } else {
        0
    };
    let replay_duration_ms = if live_replay {
        recorded_replay_duration_ms.max(live_elapsed_ms).max(1)
    } else {
        recorded_replay_duration_ms
    };
    let replay_scrubbed_time_ms = *replay_scrub_ms.read();
    let replay_time_ms = replay_scrubbed_time_ms.map_or_else(
        || {
            if live_replay {
                live_elapsed_ms.min(replay_duration_ms)
            } else {
                current_replay_time_ms(*replay_started_at_ms.read(), replay_duration_ms)
            }
        },
        |time| time.min(replay_duration_ms),
    );
    let winner_name = snapshot
        .winner_id
        .as_ref()
        .and_then(|winner_id| {
            snapshot
                .players
                .iter()
                .find(|player| &player.id == winner_id)
        })
        .map(|player| player.name.clone());

    if !is_joined {
        let join_slug = bootstrap.slug.clone();
        let join_player_id = player_id.clone();

        return rsx! {
            main { class: "game-page room-page",
                section { class: "game-shell room-entry-shell", aria_labelledby: "room-title",
                    div { class: "game-toolbar",
                        div {
                            p { class: "eyebrow", "Room {snapshot.slug}" }
                            h1 { id: "room-title", "Enter Room" }
                        }
                    }
                    form {
                        class: "display-name-form room-entry-form",
                        onsubmit: move |event| {
                            event.prevent_default();
                            let raw_name = pending_name.read().clone();
                            let Some(display_name) = normalize_display_name(&raw_name) else {
                                name_error.set("Enter a display name.".to_string());
                                return;
                            };
                            save_player_name(&display_name);
                            pending_name.set(display_name.clone());
                            name_error.set(String::new());
                            connection_status.set("Connecting".to_string());
                            connection.set(Some(RoomConnection::connect(
                                &join_slug,
                                &join_player_id,
                                &display_name,
                                room_snapshot,
                                connection_status,
                            )));
                        },
                        label { r#for: "room-display-name", "Display name" }
                        input {
                            id: "room-display-name",
                            r#type: "text",
                            autocomplete: "nickname",
                            maxlength: "{DISPLAY_NAME_MAX_CHARS}",
                            required: true,
                            placeholder: "Your name",
                            value: "{pending_name_value}",
                            oninput: move |event| pending_name.set(event.value())
                        }
                        if !name_error_text.is_empty() {
                            p { class: "form-error", "{name_error_text}" }
                        }
                        button { r#type: "submit", class: "nav-button primary", "Join Room" }
                    }
                }
            }
        };
    }

    if snapshot.game_kind == RoomGameKind::Minesweeper {
        return rsx! {
            RoomMinesweeperRoom {
                snapshot,
                player_id: player_id.clone(),
                room_url,
                room_snapshot,
                connection,
                status,
                tick: room_tick
            }
        };
    }

    rsx! {
        main { class: "game-page room-page",
            section { class: "game-shell", aria_labelledby: "room-title",
                div { class: "game-toolbar",
                    div {
                        p { class: "eyebrow", "Room {snapshot.slug}" }
                        h1 { id: "room-title", "Multiplayer Race" }
                    }
                }

                div { class: "room-share",
                    label { "Invite link" }
                    input { readonly: true, value: "{room_url}" }
                }

                match &snapshot.phase {
                    RoomPhase::Lobby | RoomPhase::Countdown { .. } => rsx! {
                        div { class: "room-lobby",
                            RoomGameSelector {
                                current: snapshot.game_kind,
                                can_select,
                                connection
                            }
                            div { class: "selector-header",
                                p { class: "eyebrow", "Puzzle" }
                                h2 { "{choice}" }
                            }
                            div { class: "controls-row",
                                button {
                                    r#type: "button",
                                    class: "nav-button primary",
                                    disabled: !can_select,
                                    onclick: {
                                        let connection = connection;
                                        move |_| send_room_message(connection, RoomClientMessage::SelectRandom)
                                    },
                                    "Random Puzzle"
                                }
                                button {
                                    r#type: "button",
                                    class: "nav-button",
                                    onclick: {
                                        let connection = connection;
                                        move |_| send_room_message(connection, RoomClientMessage::SetReady { ready: !my_ready })
                                    },
                                    "{ready_text}"
                                }
                            }
                            if let Some(countdown) = countdown {
                                div { class: "countdown-panel", aria_live: "polite",
                                    p { class: "eyebrow", "Starting" }
                                    h2 { "{countdown}" }
                                }
                            }
                            div { class: "room-puzzle-picker",
                                p { class: "eyebrow", "Or choose a puzzle" }
                                div { class: "puzzle-grid wide compact",
                                    for puzzle_id in 1..=bootstrap.total_puzzles {
                                        button {
                                            r#type: "button",
                                            class: puzzle_choice_button_class(
                                                &snapshot.puzzle_choice,
                                                puzzle_id,
                                                played_puzzle_ids.contains(&puzzle_id),
                                            ),
                                            disabled: !can_select,
                                            onclick: {
                                                let connection = connection;
                                                move |_| send_room_message(connection, RoomClientMessage::SelectPuzzle { puzzle_id })
                                            },
                                            "{puzzle_id}"
                                        }
                                    }
                                }
                            }
                        }
                    },
                    RoomPhase::Racing { .. } | RoomPhase::Complete { .. } => rsx! {
                        if let Some(winner_name) = winner_name.clone() {
                            div { class: "countdown-panel", aria_live: "polite",
                                p { class: "eyebrow", "Winner" }
                                h2 { "{winner_name}" }
                            }
                        }
                        if matches!(snapshot.phase, RoomPhase::Racing { .. }) && my_done {
                            div { class: "status-strip",
                                span { if my_gave_up { "Gave up" } else { "Finished" } }
                                span { "Waiting for the remaining racers." }
                            }
                        }
                        if matches!(snapshot.phase, RoomPhase::Complete { .. }) {
                            div { class: "room-lobby next-race-panel",
                                RoomGameSelector {
                                    current: snapshot.game_kind,
                                    can_select,
                                    connection
                                }
                                div { class: "selector-header",
                                    p { class: "eyebrow", "Next race" }
                                    h2 { "{choice}" }
                                }
                                div { class: "controls-row",
                                    button {
                                        r#type: "button",
                                        class: "nav-button primary",
                                        disabled: !can_select,
                                        onclick: {
                                            let connection = connection;
                                            move |_| send_room_message(connection, RoomClientMessage::SelectRandom)
                                        },
                                        "Random Puzzle"
                                    }
                                    button {
                                        r#type: "button",
                                        class: "nav-button",
                                        onclick: {
                                            let connection = connection;
                                            move |_| send_room_message(connection, RoomClientMessage::SetReady { ready: !my_ready })
                                        },
                                        "{ready_text}"
                                    }
                                }
                                div { class: "room-puzzle-picker",
                                    p { class: "eyebrow", "Or choose a puzzle" }
                                    div { class: "puzzle-grid wide compact",
                                        for puzzle_id in 1..=bootstrap.total_puzzles {
                                            button {
                                                r#type: "button",
                                                class: puzzle_choice_button_class(
                                                    &snapshot.puzzle_choice,
                                                    puzzle_id,
                                                    played_puzzle_ids.contains(&puzzle_id),
                                                ),
                                                disabled: !can_select,
                                                onclick: {
                                                    let connection = connection;
                                                    move |_| send_room_message(connection, RoomClientMessage::SelectPuzzle { puzzle_id })
                                                },
                                                "{puzzle_id}"
                                            }
                                        }
                                    }
                                }
                            }
                            if let Some(puzzle) = snapshot.puzzle.clone() {
                                RoomReplayPanel {
                                    puzzle,
                                    players: replay_players.clone(),
                                    replay_time_ms,
                                    replay_duration_ms,
                                    live: false,
                                    replay_scrub_ms,
                                    replay_started_at_ms,
                                    replay_manual_player_ids
                                }
                            }
                        }
                        if live_replay {
                            if let Some(puzzle) = snapshot.puzzle.clone() {
                                RoomReplayPanel {
                                    puzzle,
                                    players: replay_players.clone(),
                                    replay_time_ms,
                                    replay_duration_ms,
                                    live: true,
                                    replay_scrub_ms,
                                    replay_started_at_ms,
                                    replay_manual_player_ids
                                }
                            }
                        }
                        if live_replay {
                            if !live_replay_has_recordings {
                                div { class: "rule-panel", "Waiting for replay updates..." }
                            }
                        } else if let Some(game) = race_game.read().as_ref().cloned() {
                            RoomBoard { game_state: race_game, snapshot: game, connection }
                        } else {
                            div { class: "rule-panel", "Waiting for the puzzle..." }
                        }
                    },
                }
            }

            div { class: "room-side-column",
                aside { class: "side-panel", aria_label: "Players",
                    div { class: "selector-header",
                        p { class: "eyebrow", "Players" }
                        h2 { "{snapshot.players.len()} in room" }
                    }
                    div { class: "player-list",
                        for player in snapshot.players.iter() {
                            {
                                let place = race_place(&snapshot.players, &player.id);
                                let status = player_status(
                                    player,
                                    &snapshot.phase,
                                    race_started_at_ms,
                                    place,
                                );

                                rsx! {
                                    div {
                                        class: player_row_class(player, snapshot.winner_id.as_deref()),
                                        span { class: "player-name", "{player.name}" }
                                        span { class: "player-status", "{status}" }
                                    }
                                }
                            }
                        }
                    }
                }
                aside { class: "side-panel leaderboard-panel", aria_label: "Leaderboard",
                    div { class: "selector-header",
                        p { class: "eyebrow", "Leaderboard" }
                        h2 { "Medals" }
                    }
                    div { class: "leaderboard-list",
                        for player in leaderboard_players.iter() {
                            {
                                let total = player.medals.total();
                                let gold_width = medal_width(player.medals.gold, leaderboard_max_total);
                                let silver_width = medal_width(player.medals.silver, leaderboard_max_total);
                                let bronze_width = medal_width(player.medals.bronze, leaderboard_max_total);

                                rsx! {
                                    div { class: "leaderboard-row",
                                        div { class: "leaderboard-row-header",
                                            span { class: "player-name", "{player.name}" }
                                            span { class: "leaderboard-total", "{total}" }
                                        }
                                        div { class: "medal-counts", aria_label: "Medal counts",
                                            span { class: "medal-count gold", "G {player.medals.gold}" }
                                            span { class: "medal-count silver", "S {player.medals.silver}" }
                                            span { class: "medal-count bronze", "B {player.medals.bronze}" }
                                        }
                                        div { class: "medal-bars", aria_hidden: "true",
                                            span { class: "medal-bar gold", style: "width: {gold_width}" }
                                            span { class: "medal-bar silver", style: "width: {silver_width}" }
                                            span { class: "medal-bar bronze", style: "width: {bronze_width}" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                div { class: "timer-box room-status-box", aria_live: "polite",
                    span { class: "timer-label", "Status" }
                    span { "{status}" }
                }
            }
        }
    }
}

#[component]
fn RoomMinesweeperRoom(
    snapshot: RoomSnapshot,
    player_id: String,
    room_url: String,
    room_snapshot: Signal<RoomSnapshot>,
    connection: Signal<Option<RoomConnection>>,
    status: String,
    tick: u64,
) -> Element {
    let _ = tick;
    let me = snapshot
        .players
        .iter()
        .find(|player| player.id == player_id)
        .cloned();
    let my_ready = me.as_ref().map(|player| player.ready).unwrap_or(false);
    let ready_text = if my_ready { "Not Ready" } else { "Ready" };
    let can_select = matches!(
        snapshot.phase,
        RoomPhase::Lobby | RoomPhase::Complete { .. }
    );
    let winner_name = snapshot
        .winner_id
        .as_ref()
        .and_then(|winner_id| {
            snapshot
                .players
                .iter()
                .find(|player| &player.id == winner_id)
        })
        .map(|player| player.name.clone());
    let scoreboard_players = sorted_minesweeper_score_players(&snapshot.players);
    let leaderboard_players = sorted_leaderboard_players(&snapshot.players);
    let leaderboard_max_total = leaderboard_players
        .iter()
        .map(|player| player.medals.total())
        .max()
        .unwrap_or(0)
        .max(1);
    let time_label =
        room_minesweeper_time_label(&snapshot.phase, snapshot.minesweeper_time_limit_seconds);
    let my_score = me
        .as_ref()
        .map(|player| player.minesweeper_score)
        .unwrap_or(0);
    let my_eliminated = me
        .as_ref()
        .map(|player| player.minesweeper_eliminated)
        .unwrap_or(false);
    let play_status = if my_eliminated {
        "Eliminated"
    } else {
        match snapshot.phase {
            RoomPhase::Countdown { .. } => "Starting",
            RoomPhase::Racing { .. } => "Playing",
            RoomPhase::Complete { .. } => "Complete",
            RoomPhase::Lobby => "Setup",
        }
    };

    rsx! {
        main { class: "game-page room-page room-minesweeper-page",
            section { class: "game-shell", aria_labelledby: "room-title",
                div { class: "game-toolbar",
                    div {
                        p { class: "eyebrow", "Room {snapshot.slug}" }
                        h1 { id: "room-title", "Multiplayer Minesweeper" }
                    }
                    div { class: "timer-box",
                        span { class: "timer-label", "Time" }
                        span { id: "timer", "{time_label}" }
                    }
                }

                div { class: "room-share",
                    label { "Invite link" }
                    input { readonly: true, value: "{room_url}" }
                }

                match &snapshot.phase {
                    RoomPhase::Lobby => rsx! {
                        div { class: "room-lobby",
                            RoomGameSelector {
                                current: snapshot.game_kind,
                                can_select,
                                connection
                            }
                            RoomMinesweeperSetup {
                                seconds: snapshot.minesweeper_time_limit_seconds,
                                can_select,
                                connection
                            }
                        }
                    },
                    RoomPhase::Countdown { .. } | RoomPhase::Racing { .. } | RoomPhase::Complete { .. } => rsx! {
                        if let Some(winner_name) = winner_name.clone() {
                            div { class: "countdown-panel", aria_live: "polite",
                                p { class: "eyebrow", "Winner" }
                                h2 { "{winner_name}" }
                            }
                        }
                        div { class: "status-strip room-ms-status-strip", aria_live: "polite",
                            span { "Score {my_score}" }
                            span { "{play_status}" }
                        }
                        if matches!(snapshot.phase, RoomPhase::Complete { .. }) {
                            div { class: "room-lobby next-race-panel",
                                div { class: "selector-header",
                                    p { class: "eyebrow", "Next game" }
                                    h2 { "Minesweeper" }
                                }
                                RoomGameSelector {
                                    current: snapshot.game_kind,
                                    can_select,
                                    connection
                                }
                                RoomMinesweeperSetup {
                                    seconds: snapshot.minesweeper_time_limit_seconds,
                                    can_select,
                                    connection
                                }
                            }
                            RoomMinesweeperScores { players: scoreboard_players.clone() }
                        }
                        if let Some(board) = snapshot.minesweeper.clone() {
                            RoomMinesweeperBoard {
                                board,
                                players: snapshot.players.clone(),
                                player_id: player_id.clone(),
                                phase: snapshot.phase.clone(),
                                time_limit_seconds: snapshot.minesweeper_time_limit_seconds,
                                tick,
                                room_snapshot,
                                connection
                            }
                        } else {
                            div { class: "rule-panel", "Waiting for the board..." }
                        }
                    },
                }
            }

            div { class: "room-side-column",
                aside { class: "side-panel", aria_label: "Players",
                    div { class: "selector-header",
                        p { class: "eyebrow", "Players" }
                        h2 { "{snapshot.players.len()} in room" }
                    }
                    div { class: "player-list",
                        for player in snapshot.players.iter() {
                            {
                                let status = minesweeper_player_status(
                                    player,
                                    &snapshot.phase,
                                    snapshot.minesweeper_time_limit_seconds,
                                );

                                rsx! {
                                    div {
                                        class: player_row_class(player, snapshot.winner_id.as_deref()),
                                        span { class: "player-name", "{player.name}" }
                                        span { class: "player-status", "{status}" }
                                    }
                                }
                            }
                        }
                    }
                }
                if matches!(snapshot.phase, RoomPhase::Lobby | RoomPhase::Complete { .. }) {
                    aside { class: "side-panel ready-panel", aria_label: "Ready",
                        div { class: "selector-header",
                            p { class: "eyebrow", "Ready" }
                            h2 { "{ready_text}" }
                        }
                        button {
                            r#type: "button",
                            class: "nav-button primary",
                            onclick: {
                                let connection = connection;
                                move |_| send_room_message(connection, RoomClientMessage::SetReady { ready: !my_ready })
                            },
                            "{ready_text}"
                        }
                    }
                }
                aside { class: "side-panel leaderboard-panel", aria_label: "Leaderboard",
                    div { class: "selector-header",
                        p { class: "eyebrow", "Leaderboard" }
                        h2 { "Medals" }
                    }
                    div { class: "leaderboard-list",
                        for player in leaderboard_players.iter() {
                            {
                                let total = player.medals.total();
                                let gold_width = medal_width(player.medals.gold, leaderboard_max_total);
                                let silver_width = medal_width(player.medals.silver, leaderboard_max_total);
                                let bronze_width = medal_width(player.medals.bronze, leaderboard_max_total);

                                rsx! {
                                    div { class: "leaderboard-row",
                                        div { class: "leaderboard-row-header",
                                            span { class: "player-name", "{player.name}" }
                                            span { class: "leaderboard-total", "{total}" }
                                        }
                                        div { class: "medal-counts", aria_label: "Medal counts",
                                            span { class: "medal-count gold", "G {player.medals.gold}" }
                                            span { class: "medal-count silver", "S {player.medals.silver}" }
                                            span { class: "medal-count bronze", "B {player.medals.bronze}" }
                                        }
                                        div { class: "medal-bars", aria_hidden: "true",
                                            span { class: "medal-bar gold", style: "width: {gold_width}" }
                                            span { class: "medal-bar silver", style: "width: {silver_width}" }
                                            span { class: "medal-bar bronze", style: "width: {bronze_width}" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                div { class: "timer-box room-status-box", aria_live: "polite",
                    span { class: "timer-label", "Status" }
                    span { "{status}" }
                }
            }
        }
    }
}

#[component]
fn RoomGameSelector(
    current: RoomGameKind,
    can_select: bool,
    connection: Signal<Option<RoomConnection>>,
) -> Element {
    rsx! {
        div { class: "room-game-selector",
            p { class: "eyebrow", "Game" }
            div { class: "segmented", role: "group", aria_label: "Room game",
                button {
                    r#type: "button",
                    class: room_game_button_class(current, RoomGameKind::Queens),
                    disabled: !can_select,
                    onclick: {
                        let connection = connection;
                        move |_| send_room_message(
                            connection,
                            RoomClientMessage::SelectGame { game_kind: RoomGameKind::Queens },
                        )
                    },
                    "Queens"
                }
                button {
                    r#type: "button",
                    class: room_game_button_class(current, RoomGameKind::Minesweeper),
                    disabled: !can_select,
                    onclick: {
                        let connection = connection;
                        move |_| send_room_message(
                            connection,
                            RoomClientMessage::SelectGame { game_kind: RoomGameKind::Minesweeper },
                        )
                    },
                    "Minesweeper"
                }
            }
        }
    }
}

#[component]
fn RoomMinesweeperSetup(
    seconds: u32,
    can_select: bool,
    connection: Signal<Option<RoomConnection>>,
) -> Element {
    rsx! {
        div { class: "room-ms-setup",
            label { r#for: "room-minesweeper-seconds", "Time limit" }
            div { class: "room-ms-time-input",
                input {
                    id: "room-minesweeper-seconds",
                    r#type: "number",
                    min: "{ROOM_MINESWEEPER_MIN_SECONDS}",
                    max: "{ROOM_MINESWEEPER_MAX_SECONDS}",
                    step: "1",
                    disabled: !can_select,
                    value: "{seconds}",
                    oninput: move |event| {
                        if let Ok(seconds) = event.value().parse::<u32>() {
                            send_room_message(
                                connection,
                                RoomClientMessage::SetMinesweeperTimeLimit { seconds },
                            );
                        }
                    }
                }
                span { "seconds" }
            }
        }
    }
}

#[component]
fn RoomMinesweeperScores(players: Vec<RoomPlayerSnapshot>) -> Element {
    rsx! {
        section { class: "room-ms-scores", aria_label: "Minesweeper scores",
            div { class: "selector-header",
                p { class: "eyebrow", "Scores" }
                h2 { "Final standings" }
            }
            div { class: "leaderboard-list",
                for (index, player) in players.iter().enumerate() {
                    {
                        let place = ordinal_place(index + 1);
                        let place_label = if place.is_empty() {
                            format!("#{}", index + 1)
                        } else {
                            place.to_string()
                        };

                        rsx! {
                            div { class: "leaderboard-row room-ms-score-row",
                                div { class: "leaderboard-row-header",
                                    span { class: "player-name", "{place_label} {player.name}" }
                                    span { class: "leaderboard-total", "{player.minesweeper_score}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn RoomMinesweeperBoard(
    board: RoomMinesweeperSnapshot,
    players: Vec<RoomPlayerSnapshot>,
    player_id: String,
    phase: RoomPhase,
    time_limit_seconds: u32,
    tick: u64,
    room_snapshot: Signal<RoomSnapshot>,
    connection: Signal<Option<RoomConnection>>,
) -> Element {
    let _ = tick;
    let board = Rc::new(board);
    let mut chord_target = use_signal(|| None::<usize>);
    let mut pressed_cells = use_signal(BTreeSet::<usize>::new);
    let mut left_mouse_down = use_signal(|| false);
    let mut right_mouse_down = use_signal(|| false);
    let mut suppress_next_secondary_up = use_signal(|| false);
    let last_pointer_sent_ms = use_signal(|| 0.0);
    let own_flags = Rc::new(room_minesweeper_own_flags(&players, &player_id));
    let can_act = room_minesweeper_can_act(&phase, &players, &player_id);
    let pressed_cell_set = pressed_cells.read().clone();
    let board_width = board.width;
    let board_height = board.height;
    let cell_size = room_minesweeper_cell_size(board_width);
    let board_class = if cell_size < 24 {
        "ms-board room-ms-board compact"
    } else {
        "ms-board room-ms-board"
    };
    let mine_counter = format_minesweeper_counter(board.mines as i32 - own_flags.len() as i32);
    let timer = format_minesweeper_counter(room_minesweeper_timer_seconds(
        &phase,
        time_limit_seconds,
    ) as i32);
    let face = room_minesweeper_face(&phase, &players, &player_id);
    let countdown_cell_value = room_minesweeper_countdown_cell_value(&phase);

    rsx! {
        div { class: "room-ms-board-wrap",
            div { class: "ms-shell room-ms-shell",
                div { class: "ms-panel ms-header room-ms-header", aria_label: "Minesweeper status",
                    MinesweeperLed { label: "Mines remaining", value: mine_counter }
                    button {
                        r#type: "button",
                        class: "ms-face",
                        title: "Minesweeper",
                        aria_label: "Minesweeper",
                        disabled: true,
                        "{face}"
                    }
                    MinesweeperLed { label: "Time remaining", value: timer }
                }
                div {
                    id: ROOM_BOARD_ID,
                    class: "{board_class}",
                    role: "grid",
                    aria_label: "Multiplayer Minesweeper board",
                    style: "--mine-cols: {board_width}; --room-ms-cell-size: {cell_size}px",
                    onpointermove: move |event| {
                        let data = event.data();
                        if data.pointer_type() != "mouse" {
                            return;
                        }
                        let coordinates = data.client_coordinates();
                        let active = *left_mouse_down.read() || *right_mouse_down.read();
                        send_room_pointer_from_coordinates(
                            connection,
                            board_width,
                            board_height,
                            coordinates.x,
                            coordinates.y,
                            active,
                            false,
                            last_pointer_sent_ms,
                        );
                    },
                    onpointerleave: move |_| {
                        chord_target.set(None);
                        pressed_cells.set(BTreeSet::new());
                        left_mouse_down.set(false);
                        right_mouse_down.set(false);
                        suppress_next_secondary_up.set(false);
                        send_room_message(connection, RoomClientMessage::PointerUpdate { pointer: None });
                    },
                    for (index, cell) in board.cells.iter().enumerate() {
                        {
                            let pressed = pressed_cell_set.contains(&index);
                            let own_flag = own_flags.contains(&index);
                            let other_flag_count = room_minesweeper_visible_other_flag_count(
                                &players, &player_id, own_flag, index,
                            );
                            let start_countdown =
                                room_minesweeper_start_countdown_value(cell, countdown_cell_value);
                            let owner_color_index =
                                room_minesweeper_cell_owner_color_index(cell, &players);
                            let class_name = room_minesweeper_cell_class(
                                cell,
                                own_flag,
                                other_flag_count,
                                pressed,
                                start_countdown,
                                owner_color_index,
                            );
                            let text =
                                room_minesweeper_cell_text(cell, own_flag, pressed, start_countdown);
                            let aria = room_minesweeper_cell_aria(
                                index,
                                cell,
                                &board,
                                own_flag,
                                start_countdown,
                            );
                            let board_for_down = Rc::clone(&board);
                            let board_for_enter = Rc::clone(&board);
                            let board_for_up = Rc::clone(&board);
                            let board_for_key = Rc::clone(&board);
                            let own_flags_for_down = Rc::clone(&own_flags);
                            let own_flags_for_enter = Rc::clone(&own_flags);
                            let own_flags_for_up = Rc::clone(&own_flags);
                            let own_flags_for_key = Rc::clone(&own_flags);
                            let player_id_for_up = player_id.clone();
                            let player_id_for_double_click = player_id.clone();
                            let player_id_for_key = player_id.clone();
                            let room_snapshot_for_up = room_snapshot;
                            let room_snapshot_for_double_click = room_snapshot;
                            let room_snapshot_for_key = room_snapshot;

                            rsx! {
                                button {
                                    r#type: "button",
                                    class: "{class_name}",
                                    role: "gridcell",
                                    aria_label: "{aria}",
                                    disabled: !can_act,
                                    onpointerdown: move |event| {
                                        let data = event.data();
                                        if data.pointer_type() != "mouse" {
                                            return;
                                        }
                                        let coordinates = data.client_coordinates();
                                        let primary = data.trigger_button() == Some(MouseButton::Primary);
                                        let secondary = data.trigger_button() == Some(MouseButton::Secondary);
                                        if primary {
                                            event.prevent_default();
                                            left_mouse_down.set(true);
                                        }
                                        if secondary {
                                            event.prevent_default();
                                            right_mouse_down.set(true);
                                        }
                                        send_room_pointer_from_coordinates(
                                            connection,
                                            board_width,
                                            board_height,
                                            coordinates.x,
                                            coordinates.y,
                                            true,
                                            true,
                                            last_pointer_sent_ms,
                                        );
                                        if !can_act {
                                            return;
                                        }

                                        let both_down = (primary || *left_mouse_down.read())
                                            && (secondary || *right_mouse_down.read());
                                        if both_down || primary {
                                            let chord_press =
                                                room_minesweeper_chord_target(&board_for_down, index).map(|target| {
                                                    (
                                                        target,
                                                        room_minesweeper_pressed_neighbors(
                                                            &board_for_down,
                                                            target,
                                                            &own_flags_for_down,
                                                        ),
                                                    )
                                                });
                                            if let Some((target, pressed)) = chord_press {
                                                chord_target.set(Some(target));
                                                pressed_cells.set(pressed);
                                            } else if primary {
                                                chord_target.set(None);
                                                pressed_cells.set(BTreeSet::new());
                                            }
                                        }
                                    },
                                    onpointerenter: move |event| {
                                        let data = event.data();
                                        if data.pointer_type() != "mouse" || !*left_mouse_down.read() || !can_act {
                                            return;
                                        }
                                        let chord_press =
                                            room_minesweeper_chord_target(&board_for_enter, index).map(|target| {
                                                (
                                                    target,
                                                    room_minesweeper_pressed_neighbors(
                                                        &board_for_enter,
                                                        target,
                                                        &own_flags_for_enter,
                                                    ),
                                                )
                                            });
                                        if let Some((target, pressed)) = chord_press {
                                            chord_target.set(Some(target));
                                            pressed_cells.set(pressed);
                                        } else {
                                            chord_target.set(None);
                                            pressed_cells.set(BTreeSet::new());
                                        }
                                    },
                                    onpointerup: move |event| {
                                        let data = event.data();
                                        if data.pointer_type() == "mouse" {
                                            let coordinates = data.client_coordinates();
                                            let primary = data.trigger_button() == Some(MouseButton::Primary);
                                            let secondary = data.trigger_button() == Some(MouseButton::Secondary);
                                            if primary {
                                                event.prevent_default();
                                                left_mouse_down.set(false);
                                            }
                                            if secondary {
                                                event.prevent_default();
                                                right_mouse_down.set(false);
                                            }
                                            let active = (primary && *right_mouse_down.read())
                                                || (secondary && *left_mouse_down.read());
                                            send_room_pointer_from_coordinates(
                                                connection,
                                                board_width,
                                                board_height,
                                                coordinates.x,
                                                coordinates.y,
                                                active,
                                                true,
                                                last_pointer_sent_ms,
                                            );
                                            if !can_act {
                                                pressed_cells.set(BTreeSet::new());
                                                chord_target.set(None);
                                                return;
                                            }
                                            pressed_cells.set(BTreeSet::new());

                                            if primary {
                                                if *chord_target.read() == Some(index) {
                                                    if *right_mouse_down.read() {
                                                        suppress_next_secondary_up.set(true);
                                                    }
                                                    optimistic_room_minesweeper_chord(
                                                        room_snapshot_for_up,
                                                        &player_id_for_up,
                                                        index,
                                                    );
                                                    send_room_message(connection, RoomClientMessage::MinesweeperChord { index });
                                                    chord_target.set(None);
                                                } else if room_minesweeper_chord_target(&board_for_up, index).is_some() {
                                                    optimistic_room_minesweeper_chord(
                                                        room_snapshot_for_up,
                                                        &player_id_for_up,
                                                        index,
                                                    );
                                                    send_room_message(connection, RoomClientMessage::MinesweeperChord { index });
                                                } else if !own_flags_for_up.contains(&index) {
                                                    optimistic_room_minesweeper_reveal(
                                                        room_snapshot_for_up,
                                                        &player_id_for_up,
                                                        index,
                                                    );
                                                    send_room_message(connection, RoomClientMessage::MinesweeperReveal { index });
                                                }
                                            } else if secondary {
                                                if *suppress_next_secondary_up.read() {
                                                    suppress_next_secondary_up.set(false);
                                                    chord_target.set(None);
                                                } else if *chord_target.read() == Some(index) {
                                                    optimistic_room_minesweeper_chord(
                                                        room_snapshot_for_up,
                                                        &player_id_for_up,
                                                        index,
                                                    );
                                                    send_room_message(connection, RoomClientMessage::MinesweeperChord { index });
                                                    chord_target.set(None);
                                                } else if !*left_mouse_down.read() {
                                                    optimistic_room_minesweeper_toggle_flag(
                                                        room_snapshot_for_up,
                                                        &player_id_for_up,
                                                        index,
                                                    );
                                                    send_room_message(connection, RoomClientMessage::MinesweeperToggleFlag { index });
                                                }
                                            }
                                        } else if can_act && !own_flags_for_up.contains(&index) {
                                            optimistic_room_minesweeper_reveal(
                                                room_snapshot_for_up,
                                                &player_id_for_up,
                                                index,
                                            );
                                            send_room_message(connection, RoomClientMessage::MinesweeperReveal { index });
                                        }
                                    },
                                    ondoubleclick: move |event| {
                                        event.prevent_default();
                                        if can_act {
                                            optimistic_room_minesweeper_chord(
                                                room_snapshot_for_double_click,
                                                &player_id_for_double_click,
                                                index,
                                            );
                                            send_room_message(connection, RoomClientMessage::MinesweeperChord { index });
                                        }
                                    },
                                    oncontextmenu: move |event| {
                                        event.prevent_default();
                                    },
                                    onkeydown: move |event| {
                                        if !can_act {
                                            return;
                                        }
                                        let code = event.data().code();
                                        match code {
                                            Code::Space | Code::Enter => {
                                                event.prevent_default();
                                                if room_minesweeper_chord_target(&board_for_key, index).is_some() {
                                                    optimistic_room_minesweeper_chord(
                                                        room_snapshot_for_key,
                                                        &player_id_for_key,
                                                        index,
                                                    );
                                                    send_room_message(connection, RoomClientMessage::MinesweeperChord { index });
                                                } else if !own_flags_for_key.contains(&index) {
                                                    optimistic_room_minesweeper_reveal(
                                                        room_snapshot_for_key,
                                                        &player_id_for_key,
                                                        index,
                                                    );
                                                    send_room_message(connection, RoomClientMessage::MinesweeperReveal { index });
                                                }
                                            }
                                            Code::KeyF => {
                                                event.prevent_default();
                                                optimistic_room_minesweeper_toggle_flag(
                                                    room_snapshot_for_key,
                                                    &player_id_for_key,
                                                    index,
                                                );
                                                send_room_message(connection, RoomClientMessage::MinesweeperToggleFlag { index });
                                            }
                                            Code::KeyC => {
                                                event.prevent_default();
                                                optimistic_room_minesweeper_chord(
                                                    room_snapshot_for_key,
                                                    &player_id_for_key,
                                                    index,
                                                );
                                                send_room_message(connection, RoomClientMessage::MinesweeperChord { index });
                                            }
                                            _ => {}
                                        }
                                    },
                                    span { class: "ms-cell-symbol", aria_hidden: "true", "{text}" }
                                    if other_flag_count > 0 && !cell.revealed {
                                        span { class: "ms-other-flag", aria_hidden: "true",
                                            if other_flag_count > 1 {
                                                span { class: "ms-other-flag-count", "{other_flag_count}" }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    for player in players.iter() {
                        if player.id != player_id {
                            if let Some(pointer) = player.pointer {
                                if room_live_pointer_is_fresh(pointer) {
                                    div {
                                        class: room_live_pointer_class(pointer.active_click),
                                        style: "{room_live_pointer_style(pointer)}",
                                        aria_hidden: "true",
                                        span { class: "room-live-pointer-label", "{player.name}" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn RoomBoard(
    game_state: Signal<Option<GameState>>,
    snapshot: GameState,
    connection: Signal<Option<RoomConnection>>,
) -> Element {
    let size = snapshot.puzzle.size;
    let conflict_cells = snapshot
        .validation
        .conflict_cells
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let status = validation_status(&snapshot.validation, size);

    rsx! {
        div { class: "controls-row", aria_label: "Game controls",
            div { class: "segmented", role: "group", aria_label: "Cell mode",
                button {
                    r#type: "button",
                    class: mode_button_class(snapshot.mode, Mode::Queen),
                    onclick: move |_| {
                        if let Some(game) = game_state.write().as_mut() {
                            game.set_mode(Mode::Queen);
                        }
                    },
                    "Queen"
                }
                button {
                    r#type: "button",
                    class: mode_button_class(snapshot.mode, Mode::Mark),
                    onclick: move |_| {
                        if let Some(game) = game_state.write().as_mut() {
                            game.set_mode(Mode::Mark);
                        }
                    },
                    "Mark"
                }
                button {
                    r#type: "button",
                    class: mode_button_class(snapshot.mode, Mode::Clear),
                    onclick: move |_| {
                        if let Some(game) = game_state.write().as_mut() {
                            game.set_mode(Mode::Clear);
                        }
                    },
                    "Clear"
                }
            }
            div { class: "tool-buttons",
                button {
                    r#type: "button",
                    class: "tool-button",
                    title: "Undo last move",
                    onclick: move |_| {
                        if let Some(game) = game_state.write().as_mut() {
                            game.undo();
                        }
                    },
                    "Undo"
                }
                button {
                    r#type: "button",
                    class: "tool-button",
                    title: "Highlight conflicts",
                    onclick: move |_| {
                        if let Some(game) = game_state.write().as_mut() {
                            game.revalidate(false);
                        }
                    },
                    "Check"
                }
                button {
                    r#type: "button",
                    class: "tool-button",
                    title: "Reset this puzzle",
                    onclick: move |_| {
                        if let Some(game) = game_state.write().as_mut() {
                            game.reset();
                        }
                    },
                    "Reset"
                }
                button {
                    r#type: "button",
                    class: "tool-button danger",
                    title: "Give up this race",
                    onclick: {
                        let connection = connection;
                        move |_| send_room_message(connection, RoomClientMessage::GiveUp)
                    },
                    "Give Up"
                }
            }
        }
        div { class: "board-wrap",
            div {
                id: ROOM_BOARD_ID,
                class: "board",
                role: "grid",
                aria_label: "Queens board",
                style: "--board-size: {size}",
                onpointermove: move |event| {
                    let data = event.data();
                    if data.pointer_type() == "mouse" {
                        let coordinates = data.client_coordinates();
                        if let Some((x, y)) = normalized_board_pointer(coordinates.x, coordinates.y) {
                            if let Some(game) = game_state.write().as_mut() {
                                game.record_mouse_sample(x, y, false);
                            }
                        }
                    }
                },
                onpointerleave: move |event| {
                    let data = event.data();
                    if data.pointer_type() == "mouse" {
                        let coordinates = data.client_coordinates();
                        if let Some((x, y)) = normalized_board_pointer(coordinates.x, coordinates.y) {
                            if let Some(game) = game_state.write().as_mut() {
                                game.record_mouse_event(ROOM_MOUSE_EVENT_LEAVE, x, y, None);
                            }
                        }
                    }
                },
                for cell in snapshot.cells.iter() {
                    {
                        let index = snapshot.index_for(cell.row, cell.col);
                        let state = snapshot.states[index];
                        let class_name = cell_class(cell, state, &conflict_cells);
                        let aria = cell_aria(cell, state);

                        rsx! {
                            button {
                                r#type: "button",
                                class: "{class_name}",
                                style: "--cell-color: {cell.color}",
                                "data-row": "{cell.row}",
                                "data-col": "{cell.col}",
                                "data-region": "{cell.region}",
                                aria_label: "{aria}",
                                role: "gridcell",
                                onpointerdown: move |event| {
                                    let data = event.data();
                                    if data.pointer_type() == "mouse" {
                                        let coordinates = data.client_coordinates();
                                        let pointer = normalized_board_pointer(coordinates.x, coordinates.y);
                                        if let Some(game) = game_state.write().as_mut() {
                                            if let Some((x, y)) = pointer {
                                                let kind = if data.trigger_button() == Some(MouseButton::Primary) {
                                                    Some(ROOM_MOUSE_EVENT_PRIMARY_DOWN)
                                                } else if data.trigger_button() == Some(MouseButton::Secondary) {
                                                    Some(ROOM_MOUSE_EVENT_SECONDARY_DOWN)
                                                } else {
                                                    None
                                                };
                                                if let Some(kind) = kind {
                                                    game.record_mouse_event(kind, x, y, Some(index));
                                                }
                                            }
                                            if data.trigger_button() == Some(MouseButton::Primary) {
                                                event.prevent_default();
                                                game.start_mark_drag(index);
                                            }
                                        }
                                    }
                                },
                                onpointerenter: move |event| {
                                    let data = event.data();
                                    if data.pointer_type() == "mouse" {
                                        let coordinates = data.client_coordinates();
                                        let pointer = normalized_board_pointer(coordinates.x, coordinates.y);
                                        if let Some(game) = game_state.write().as_mut() {
                                            if let Some((x, y)) = pointer {
                                                game.record_mouse_event(ROOM_MOUSE_EVENT_ENTER, x, y, Some(index));
                                            }
                                            if data.held_buttons().contains(MouseButton::Primary) {
                                                game.drag_mark_cell(index);
                                            } else {
                                                game.finish_mark_drag(None);
                                            }
                                        }
                                    }
                                },
                                onpointerup: move |event| {
                                    let data = event.data();
                                    if data.pointer_type() == "mouse" {
                                        let coordinates = data.client_coordinates();
                                        let pointer = normalized_board_pointer(coordinates.x, coordinates.y);
                                        if let Some(game) = game_state.write().as_mut() {
                                            if let Some((x, y)) = pointer {
                                                let kind = if data.trigger_button() == Some(MouseButton::Primary) {
                                                    Some(ROOM_MOUSE_EVENT_PRIMARY_UP)
                                                } else if data.trigger_button() == Some(MouseButton::Secondary) {
                                                    Some(ROOM_MOUSE_EVENT_SECONDARY_UP)
                                                } else {
                                                    None
                                                };
                                                if let Some(kind) = kind {
                                                    game.record_mouse_event(kind, x, y, Some(index));
                                                }
                                            }
                                            if data.trigger_button() == Some(MouseButton::Primary) {
                                                event.prevent_default();
                                                game.finish_mark_drag(Some(index));
                                            }
                                        }
                                    } else if let Some(game) = game_state.write().as_mut() {
                                        game.apply_mode_action(index);
                                    }
                                },
                                oncontextmenu: move |event| {
                                    event.prevent_default();
                                    if let Some(game) = game_state.write().as_mut() {
                                        game.toggle_queen(index);
                                    }
                                },
                                onkeydown: move |event| {
                                    let code = event.data().code();
                                    match code {
                                        Code::Space | Code::Enter => {
                                            event.prevent_default();
                                            if let Some(game) = game_state.write().as_mut() {
                                                game.apply_mode_action(index);
                                            }
                                        }
                                        Code::KeyQ => {
                                            event.prevent_default();
                                            if let Some(game) = game_state.write().as_mut() {
                                                game.toggle_queen(index);
                                            }
                                        }
                                        Code::KeyX => {
                                            event.prevent_default();
                                            if let Some(game) = game_state.write().as_mut() {
                                                game.toggle_mark(index);
                                            }
                                        }
                                        Code::Backspace | Code::Delete => {
                                            event.prevent_default();
                                            if let Some(game) = game_state.write().as_mut() {
                                                let refresh = game.states[index] == CellState::Queen;
                                                game.commit_state(index, CellState::Empty, refresh);
                                            }
                                        }
                                        _ => {}
                                    }
                                },
                                span { class: "cell-symbol", aria_hidden: "true" }
                            }
                        }
                    }
                }
            }
        }
        div { class: "status-strip", aria_live: "polite",
            span { id: "queen-count", "{snapshot.validation.queen_count} / {snapshot.validation.expected_queens} queens" }
            span { id: "rule-status", "{status}" }
        }
    }
}

#[component]
fn RoomReplayPanel(
    puzzle: Puzzle,
    mut players: Vec<RoomPlayerSnapshot>,
    replay_time_ms: u64,
    replay_duration_ms: u64,
    live: bool,
    mut replay_scrub_ms: Signal<Option<u64>>,
    mut replay_started_at_ms: Signal<Option<(u64, f64)>>,
    mut replay_manual_player_ids: Signal<Vec<String>>,
) -> Element {
    let _smooth_scrubber = use_future(move || async move {
        loop {
            TimeoutFuture::new(16).await;
            if !live && (*replay_scrub_ms.read()).is_none() {
                let replay_time_ms =
                    current_replay_time_ms(*replay_started_at_ms.read(), replay_duration_ms)
                        .min(replay_duration_ms);
                set_replay_scrubber_value(replay_time_ms);
            }
        }
    });

    players.retain(|player| player.recording.is_some());
    players.sort_by(|left, right| {
        replay_player_sort_key(left)
            .cmp(&replay_player_sort_key(right))
            .then_with(|| left.name.cmp(&right.name))
    });

    if players.is_empty() {
        return rsx! {};
    }

    let manual_player_ids = replay_manual_player_ids.read().clone();
    let displayed_player_ids =
        selected_replay_player_ids(&players, replay_time_ms, &manual_player_ids);
    let displayed_players = replay_players_by_id(&players, &displayed_player_ids);
    let displayed_player_id_set = displayed_player_ids
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let manual_player_id_set = manual_player_ids.iter().cloned().collect::<BTreeSet<_>>();
    let is_manual_replay_selection = !manual_player_ids.is_empty();
    let auto_button_class = if is_manual_replay_selection {
        "replay-racer-button"
    } else {
        "replay-racer-button active"
    };

    let cells = build_cells(&puzzle);
    let size = puzzle.size;
    let replay_time_label = format_duration_ms(replay_time_ms.min(replay_duration_ms));
    let replay_duration_label = format_duration_ms(replay_duration_ms);
    let scrubbed_time_ms = *replay_scrub_ms.read();
    let is_paused = !live && scrubbed_time_ms.is_some();
    let playback_button_class = if is_paused {
        "tool-button replay-playback-button is-paused"
    } else {
        "tool-button replay-playback-button is-playing"
    };
    let playback_button_label = if is_paused {
        "Play replay"
    } else {
        "Pause replay"
    };

    rsx! {
        section { class: "replay-section", aria_labelledby: "replay-title",
            div { class: "selector-header",
                p { class: "eyebrow", "Replay" }
                h2 { id: "replay-title", "Race playback" }
            }
            div { class: "replay-racer-bar", aria_label: "Replay racers",
                button {
                    r#type: "button",
                    class: "{auto_button_class}",
                    onclick: move |_| replay_manual_player_ids.set(Vec::new()),
                    "Auto"
                }
                for player in players.iter() {
                    {
                        let player_id = player.id.clone();
                        let is_displayed = displayed_player_id_set.contains(&player.id);
                        let is_manual = manual_player_id_set.contains(&player.id);
                        let class_name = replay_racer_button_class(is_displayed, is_manual);

                        rsx! {
                            button {
                                r#type: "button",
                                class: "{class_name}",
                                onclick: move |_| toggle_replay_player_selection(replay_manual_player_ids, player_id.clone()),
                                "{player.name}"
                            }
                        }
                    }
                }
            }
            if live {
                div { class: "replay-live-feed",
                    span { "Live" }
                    span { class: "replay-time", "{replay_time_label}" }
                }
            } else {
                div { class: "replay-controls",
                    button {
                        r#type: "button",
                        class: "{playback_button_class}",
                        aria_label: "{playback_button_label}",
                        title: "{playback_button_label}",
                        onclick: move |_| {
                            let scrubbed_time_ms = *replay_scrub_ms.read();
                            if let Some(scrubbed_time_ms) = scrubbed_time_ms {
                                let replay_start = *replay_started_at_ms.read();
                                let replay_key = replay_start.map(|(key, _)| key).unwrap_or_default();
                                let resume_time_ms = scrubbed_time_ms.min(replay_duration_ms);
                                replay_started_at_ms.set(Some((replay_key, now_ms() - resume_time_ms as f64)));
                                replay_scrub_ms.set(None);
                            } else {
                                replay_scrub_ms.set(Some(replay_time_ms.min(replay_duration_ms)));
                            }
                        },
                        if is_paused {
                            span { class: "playback-icon play", aria_hidden: "true" }
                        } else {
                            span { class: "playback-icon pause", aria_hidden: "true" }
                        }
                    }
                    input {
                        id: "{REPLAY_SCRUBBER_ID}",
                        class: "replay-scrubber",
                        r#type: "range",
                        min: "0",
                        max: "{replay_duration_ms}",
                        step: "1",
                        value: "{replay_time_ms.min(replay_duration_ms)}",
                        aria_label: "Replay position",
                        oninput: move |event| {
                            if let Ok(value) = event.value().parse::<u64>() {
                                replay_scrub_ms.set(Some(value.min(replay_duration_ms)));
                            }
                        }
                    }
                    span { class: "replay-time", "{replay_time_label} / {replay_duration_label}" }
                }
            }
            div { class: "replay-grid",
                for player in displayed_players.iter() {
                    {
                        let states = player
                            .recording
                            .as_ref()
                            .map(|recording| replay_states(recording, replay_time_ms, size * size))
                            .unwrap_or_else(|| vec![CellState::Empty; size * size]);
                        let finish_time = player
                            .finish_ms
                            .map(format_duration_ms)
                            .unwrap_or_else(|| {
                                if player.gave_up {
                                    "Gave up".to_string()
                                } else {
                                    "In progress".to_string()
                                }
                            });
                        let mouse_pointer = player
                            .mouse_recording
                            .as_ref()
                            .and_then(|recording| replay_mouse_pointer(recording, replay_time_ms));

                        rsx! {
                            article { class: "replay-card",
                                div { class: "replay-card-header",
                                    h3 { "{player.name}" }
                                    span { "{finish_time}" }
                                }
                                div {
                                    class: "replay-board",
                                    style: "--board-size: {size}",
                                    aria_label: "Replay board for {player.name}",
                                    for cell in cells.iter() {
                                        {
                                            let index = cell.row * size + cell.col;
                                            let state = states
                                                .get(index)
                                                .copied()
                                                .unwrap_or(CellState::Empty);
                                            let class_name = replay_cell_class(cell, state);

                                            rsx! {
                                                div {
                                                    class: "{class_name}",
                                                    style: "--cell-color: {cell.color}",
                                                    span { class: "cell-symbol", aria_hidden: "true" }
                                                }
                                            }
                                        }
                                    }
                                    if let Some(pointer) = mouse_pointer {
                                        div {
                                            class: replay_mouse_class(pointer.active_click, !is_paused),
                                            style: "--mouse-x: {pointer.x_percent}; --mouse-y: {pointer.y_percent}",
                                            aria_hidden: "true"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn read_app_bootstrap() -> Result<AppBootstrap, String> {
    let document = web_sys::window()
        .and_then(|window| window.document())
        .ok_or_else(|| "Browser document is unavailable.".to_string())?;
    if let Some(raw) = document
        .get_element_by_id("game-data")
        .and_then(|element| element.text_content())
    {
        return serde_json::from_str(&raw)
            .map(AppBootstrap::Game)
            .map_err(|error| format!("Puzzle data is invalid: {error}"));
    }
    if let Some(raw) = document
        .get_element_by_id("room-data")
        .and_then(|element| element.text_content())
    {
        return serde_json::from_str(&raw)
            .map(AppBootstrap::Room)
            .map_err(|error| format!("Room data is invalid: {error}"));
    }
    if let Some(raw) = document
        .get_element_by_id("minesweeper-data")
        .and_then(|element| element.text_content())
    {
        return serde_json::from_str(&raw)
            .map(AppBootstrap::Minesweeper)
            .map_err(|error| format!("Minesweeper data is invalid: {error}"));
    }
    Err("No app data is available on this page.".to_string())
}

fn load_or_create_player_id() -> String {
    const KEY: &str = "queensgame:player-id";
    if let Some(storage) = local_storage() {
        if let Ok(Some(player_id)) = storage.get_item(KEY) {
            if !player_id.trim().is_empty() {
                return player_id;
            }
        }
        let player_id = generate_player_id();
        let _ = storage.set_item(KEY, &player_id);
        return player_id;
    }
    generate_player_id()
}

fn initial_room_entry() -> RoomEntry {
    if let Some(name) = room_name_from_url() {
        return RoomEntry {
            name,
            auto_join: true,
        };
    }

    if let Some(name) = load_saved_player_name() {
        return RoomEntry {
            name,
            auto_join: true,
        };
    }

    RoomEntry {
        name: String::new(),
        auto_join: false,
    }
}

fn room_name_from_url() -> Option<String> {
    let search = web_sys::window()?.location().search().ok()?;
    let params = web_sys::UrlSearchParams::new_with_str(&search).ok()?;
    params
        .get("name")
        .and_then(|name| normalize_display_name(&name))
}

fn load_saved_player_name() -> Option<String> {
    const KEY: &str = "queensgame:player-name";
    local_storage()
        .and_then(|storage| storage.get_item(KEY).ok().flatten())
        .and_then(|name| normalize_display_name(&name))
}

fn save_player_name(name: &str) {
    const KEY: &str = "queensgame:player-name";
    let Some(display_name) = normalize_display_name(name) else {
        return;
    };
    if let Some(storage) = local_storage() {
        let _ = storage.set_item(KEY, &display_name);
    }
}

fn generate_player_id() -> String {
    let random = (js_sys::Math::random() * u32::MAX as f64) as u32;
    format!("{:x}{random:08x}", now_ms() as u64)
}

fn send_room_message(connection: Signal<Option<RoomConnection>>, message: RoomClientMessage) {
    if let Some(connection) = connection.read().as_ref() {
        let _ = connection.send(&message);
    }
}

fn optimistic_room_minesweeper_reveal(
    mut room_snapshot: Signal<RoomSnapshot>,
    player_id: &str,
    index: usize,
) {
    optimistic_room_minesweeper_reveal_snapshot(&mut room_snapshot.write(), player_id, index);
}

fn optimistic_room_minesweeper_toggle_flag(
    mut room_snapshot: Signal<RoomSnapshot>,
    player_id: &str,
    index: usize,
) {
    optimistic_room_minesweeper_toggle_flag_snapshot(&mut room_snapshot.write(), player_id, index);
}

fn optimistic_room_minesweeper_chord(
    mut room_snapshot: Signal<RoomSnapshot>,
    player_id: &str,
    index: usize,
) {
    optimistic_room_minesweeper_chord_snapshot(&mut room_snapshot.write(), player_id, index);
}

fn optimistic_room_minesweeper_reveal_snapshot(
    snapshot: &mut RoomSnapshot,
    player_id: &str,
    index: usize,
) {
    if !room_minesweeper_can_act(&snapshot.phase, &snapshot.players, player_id) {
        return;
    }
    let own_flags = room_minesweeper_own_flags(&snapshot.players, player_id);
    if own_flags.contains(&index) {
        return;
    }
    let (score_delta, eliminated) = {
        let Some(board) = snapshot.minesweeper.as_mut() else {
            return;
        };
        optimistic_room_minesweeper_reveal_board(board, index, &own_flags, player_id)
    };
    optimistic_room_minesweeper_apply_result(snapshot, player_id, score_delta, eliminated);
}

fn optimistic_room_minesweeper_toggle_flag_snapshot(
    snapshot: &mut RoomSnapshot,
    player_id: &str,
    index: usize,
) {
    if !room_minesweeper_can_act(&snapshot.phase, &snapshot.players, player_id) {
        return;
    }
    let Some(board) = snapshot.minesweeper.as_ref() else {
        return;
    };
    if board
        .cells
        .get(index)
        .map(|cell| cell.revealed)
        .unwrap_or(true)
    {
        return;
    }
    let Some(player) = snapshot
        .players
        .iter_mut()
        .find(|player| player.id == player_id)
    else {
        return;
    };
    if let Some(position) = player
        .minesweeper_flags
        .iter()
        .position(|flag| *flag == index)
    {
        player.minesweeper_flags.remove(position);
    } else {
        player.minesweeper_flags.push(index);
        player.minesweeper_flags.sort_unstable();
    }
}

fn optimistic_room_minesweeper_chord_snapshot(
    snapshot: &mut RoomSnapshot,
    player_id: &str,
    index: usize,
) {
    if !room_minesweeper_can_act(&snapshot.phase, &snapshot.players, player_id) {
        return;
    }
    let own_flags = room_minesweeper_own_flags(&snapshot.players, player_id);
    let (score_delta, eliminated) = {
        let Some(board) = snapshot.minesweeper.as_mut() else {
            return;
        };
        optimistic_room_minesweeper_chord_board(board, index, &own_flags, player_id)
    };
    optimistic_room_minesweeper_apply_result(snapshot, player_id, score_delta, eliminated);
}

fn optimistic_room_minesweeper_reveal_board(
    board: &mut RoomMinesweeperSnapshot,
    index: usize,
    own_flags: &BTreeSet<usize>,
    player_id: &str,
) -> (u32, bool) {
    let Some(cell) = board.cells.get_mut(index) else {
        return (0, false);
    };
    if cell.revealed || own_flags.contains(&index) {
        return (0, false);
    }
    if cell.mine {
        cell.revealed = true;
        cell.detonated = true;
        return (0, true);
    }

    let revealed = optimistic_room_minesweeper_reveal_safe_cells(board, index, own_flags);
    (
        optimistic_room_minesweeper_score_and_claim_revealed_cells(board, &revealed, player_id),
        false,
    )
}

fn optimistic_room_minesweeper_chord_board(
    board: &mut RoomMinesweeperSnapshot,
    index: usize,
    own_flags: &BTreeSet<usize>,
    player_id: &str,
) -> (u32, bool) {
    let Some(cell) = board.cells.get(index) else {
        return (0, false);
    };
    if !cell.revealed || cell.mine || cell.adjacent_mines.unwrap_or_default() == 0 {
        return (0, false);
    }
    let neighbors = room_minesweeper_neighbors(board, index);
    let flagged_neighbors = neighbors
        .iter()
        .filter(|neighbor| own_flags.contains(neighbor))
        .count();
    if flagged_neighbors != usize::from(cell.adjacent_mines.unwrap_or_default()) {
        return (0, false);
    }

    let targets = neighbors
        .into_iter()
        .filter(|neighbor| !own_flags.contains(neighbor))
        .filter(|neighbor| {
            board
                .cells
                .get(*neighbor)
                .map(|cell| !cell.revealed)
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    if let Some(mine) = targets
        .iter()
        .copied()
        .find(|target| board.cells[*target].mine)
    {
        if let Some(cell) = board.cells.get_mut(mine) {
            cell.revealed = true;
            cell.detonated = true;
        }
        return (0, true);
    }

    let mut score_delta = 0u32;
    for target in targets {
        let revealed = optimistic_room_minesweeper_reveal_safe_cells(board, target, own_flags);
        score_delta = score_delta.saturating_add(
            optimistic_room_minesweeper_score_and_claim_revealed_cells(board, &revealed, player_id),
        );
    }
    (score_delta, false)
}

fn optimistic_room_minesweeper_reveal_safe_cells(
    board: &mut RoomMinesweeperSnapshot,
    index: usize,
    own_flags: &BTreeSet<usize>,
) -> Vec<usize> {
    if index >= board.cells.len() {
        return Vec::new();
    }

    let mut revealed = Vec::new();
    let mut queued = BTreeSet::from([index]);
    let mut queue = VecDeque::from([index]);
    while let Some(next) = queue.pop_front() {
        if own_flags.contains(&next) {
            continue;
        }
        let adjacent_mines = {
            let Some(cell) = board.cells.get_mut(next) else {
                continue;
            };
            if cell.revealed || cell.mine {
                continue;
            }
            cell.revealed = true;
            cell.adjacent_mines.unwrap_or_default()
        };
        revealed.push(next);

        if adjacent_mines == 0 {
            for neighbor in room_minesweeper_neighbors(board, next) {
                if queued.contains(&neighbor) || own_flags.contains(&neighbor) {
                    continue;
                }
                let should_queue = board
                    .cells
                    .get(neighbor)
                    .map(|cell| !cell.revealed && !cell.mine)
                    .unwrap_or(false);
                if should_queue {
                    queued.insert(neighbor);
                    queue.push_back(neighbor);
                }
            }
        }
    }
    revealed
}

fn optimistic_room_minesweeper_score_and_claim_revealed_cells(
    board: &mut RoomMinesweeperSnapshot,
    revealed: &[usize],
    player_id: &str,
) -> u32 {
    let mut score = 0;
    for index in revealed {
        let Some(cell) = board.cells.get_mut(*index) else {
            continue;
        };
        if !cell.mine && cell.adjacent_mines.unwrap_or_default() > 0 {
            if cell.owner_id.is_none() {
                cell.owner_id = Some(player_id.to_string());
            }
            score += 1;
        }
    }
    score
}

fn optimistic_room_minesweeper_apply_result(
    snapshot: &mut RoomSnapshot,
    player_id: &str,
    score_delta: u32,
    eliminated: bool,
) {
    if score_delta == 0 && !eliminated {
        return;
    }
    let score_elapsed_ms = room_minesweeper_elapsed_ms(&snapshot.phase);
    let Some(player) = snapshot
        .players
        .iter_mut()
        .find(|player| player.id == player_id)
    else {
        return;
    };
    if score_delta > 0 {
        player.minesweeper_score = player.minesweeper_score.saturating_add(score_delta);
        player.minesweeper_last_score_ms = score_elapsed_ms;
    }
    if eliminated {
        player.minesweeper_eliminated = true;
    }
}

fn room_minesweeper_elapsed_ms(phase: &RoomPhase) -> Option<u64> {
    let RoomPhase::Racing { started_at_ms } = phase else {
        return None;
    };
    Some(current_time_ms().saturating_sub(*started_at_ms))
}

fn current_time_ms() -> u64 {
    #[cfg(target_arch = "wasm32")]
    {
        now_ms().max(0.0).floor() as u64
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or(0)
    }
}

fn append_snapshot_recording_frame(
    snapshot: &mut RoomSnapshot,
    player_id: &str,
    frame: RoomRecordingFrame,
) {
    let Some(player) = snapshot
        .players
        .iter_mut()
        .find(|player| player.id == player_id)
    else {
        return;
    };
    if player.finish_ms.is_some() {
        return;
    }
    let recording = player
        .recording
        .get_or_insert_with(|| RoomRecording { frames: Vec::new() });
    let _ = append_recording_frame(recording, frame);
}

fn update_snapshot_pointer(
    snapshot: &mut RoomSnapshot,
    player_id: &str,
    pointer: Option<RoomLivePointer>,
) {
    let Some(player) = snapshot
        .players
        .iter_mut()
        .find(|player| player.id == player_id)
    else {
        return;
    };
    player.pointer = pointer;
}

fn append_snapshot_mouse_recording(
    snapshot: &mut RoomSnapshot,
    player_id: &str,
    recording: RoomMouseRecording,
) {
    let Some(player) = snapshot
        .players
        .iter_mut()
        .find(|player| player.id == player_id)
    else {
        return;
    };
    if player.finish_ms.is_some() {
        return;
    }
    let existing = player
        .mouse_recording
        .get_or_insert_with(|| RoomMouseRecording {
            samples: Vec::new(),
            events: Vec::new(),
        });
    let _ = append_mouse_recording(existing, recording);
}

fn room_ws_url(slug: &str, player_id: &str, player_name: &str) -> Option<String> {
    let location = web_sys::window()?.location();
    let protocol = if location.protocol().ok()?.as_str() == "https:" {
        "wss:"
    } else {
        "ws:"
    };
    let host = location.host().ok()?;
    Some(format!(
        "{protocol}//{host}/ws/rooms/{}?player_id={}&name={}",
        encode_component(slug),
        encode_component(player_id),
        encode_component(player_name)
    ))
}

fn encode_component(value: &str) -> String {
    js_sys::encode_uri_component(value)
        .as_string()
        .unwrap_or_else(|| value.to_string())
}

fn current_room_url(slug: &str) -> String {
    let Some(location) = web_sys::window().map(|window| window.location()) else {
        return String::from("");
    };
    let Ok(origin) = location.origin() else {
        return String::from("");
    };
    format!("{origin}/rooms/{}", encode_component(slug))
}

fn puzzle_choice_label(choice: &RoomPuzzleChoice) -> String {
    match choice {
        RoomPuzzleChoice::Puzzle { id } => format!("Puzzle #{id}"),
        RoomPuzzleChoice::Random => "Random puzzle".to_string(),
    }
}

fn puzzle_choice_button_class(choice: &RoomPuzzleChoice, puzzle_id: usize, played: bool) -> String {
    let mut class_name = String::new();
    if matches!(choice, RoomPuzzleChoice::Puzzle { id } if *id == puzzle_id) {
        class_name.push_str("active");
    }
    if played {
        if !class_name.is_empty() {
            class_name.push(' ');
        }
        class_name.push_str("played");
    }
    class_name
}

fn countdown_label(phase: &RoomPhase) -> Option<String> {
    let RoomPhase::Countdown { starts_at_ms } = phase else {
        return None;
    };
    let remaining_ms = starts_at_ms.saturating_sub(now_ms() as u64);
    let tenths = remaining_ms.div_ceil(100);
    Some(format!("{}.{:01}", tenths / 10, tenths % 10))
}

fn room_minesweeper_countdown_cell_value(phase: &RoomPhase) -> Option<u8> {
    let RoomPhase::Countdown { starts_at_ms } = phase else {
        return None;
    };
    let remaining_ms = starts_at_ms.saturating_sub(current_time_ms());
    Some(remaining_ms.div_ceil(1000).clamp(1, 5) as u8)
}

fn player_row_class(player: &RoomPlayerSnapshot, winner_id: Option<&str>) -> &'static str {
    if winner_id == Some(player.id.as_str()) {
        "player-row winner"
    } else if !player.connected {
        "player-row disconnected"
    } else {
        "player-row"
    }
}

fn player_status(
    player: &RoomPlayerSnapshot,
    phase: &RoomPhase,
    started_at_ms: Option<u64>,
    place: Option<usize>,
) -> String {
    if !player.connected {
        return "Disconnected".to_string();
    }
    match phase {
        RoomPhase::Lobby => {
            if player.ready {
                "Ready".to_string()
            } else {
                "Not ready".to_string()
            }
        }
        RoomPhase::Countdown { .. } => "Ready".to_string(),
        RoomPhase::Racing { .. } => {
            if player.gave_up {
                return "Gave up".to_string();
            }
            if let Some(finish_ms) = player.finish_ms {
                return format_place_time(place, finish_ms);
            }
            if let Some(started_at_ms) = started_at_ms {
                format!(
                    "In progress {}",
                    format_duration_ms((now_ms() as u64).saturating_sub(started_at_ms))
                )
            } else {
                "In progress".to_string()
            }
        }
        RoomPhase::Complete { .. } => {
            if player.ready {
                "Ready".to_string()
            } else if player.gave_up {
                "Gave up".to_string()
            } else if let Some(finish_ms) = player.finish_ms {
                format_place_time(place, finish_ms)
            } else {
                "Not ready".to_string()
            }
        }
    }
}

fn minesweeper_player_status(
    player: &RoomPlayerSnapshot,
    phase: &RoomPhase,
    time_limit_seconds: u32,
) -> String {
    if !player.connected {
        return "Disconnected".to_string();
    }

    match phase {
        RoomPhase::Lobby => {
            if player.ready {
                "Ready".to_string()
            } else {
                "Not ready".to_string()
            }
        }
        RoomPhase::Countdown { .. } => "Ready".to_string(),
        RoomPhase::Racing { started_at_ms } => {
            if player.minesweeper_eliminated {
                format!("Out - {}", player.minesweeper_score)
            } else {
                let remaining = room_minesweeper_remaining_ms(*started_at_ms, time_limit_seconds);
                format!(
                    "{} pts - {}",
                    player.minesweeper_score,
                    format_duration_ms(remaining)
                )
            }
        }
        RoomPhase::Complete { .. } => {
            if player.ready {
                format!("Ready - {}", player.minesweeper_score)
            } else if player.minesweeper_eliminated {
                format!("Out - {}", player.minesweeper_score)
            } else {
                format!("{} pts", player.minesweeper_score)
            }
        }
    }
}

fn room_minesweeper_time_label(phase: &RoomPhase, time_limit_seconds: u32) -> String {
    match phase {
        RoomPhase::Lobby => format!("{}s", time_limit_seconds),
        RoomPhase::Countdown { starts_at_ms } => {
            let remaining_ms = starts_at_ms.saturating_sub(now_ms() as u64);
            format_duration_ms(remaining_ms)
        }
        RoomPhase::Racing { started_at_ms } => format_duration_ms(room_minesweeper_remaining_ms(
            *started_at_ms,
            time_limit_seconds,
        )),
        RoomPhase::Complete { .. } => "Done".to_string(),
    }
}

fn room_minesweeper_timer_seconds(phase: &RoomPhase, time_limit_seconds: u32) -> u64 {
    match phase {
        RoomPhase::Lobby => u64::from(time_limit_seconds),
        RoomPhase::Countdown { starts_at_ms } => {
            starts_at_ms.saturating_sub(now_ms() as u64).div_ceil(1000)
        }
        RoomPhase::Racing { started_at_ms } => {
            room_minesweeper_remaining_ms(*started_at_ms, time_limit_seconds).div_ceil(1000)
        }
        RoomPhase::Complete { .. } => 0,
    }
}

fn room_minesweeper_remaining_ms(started_at_ms: u64, time_limit_seconds: u32) -> u64 {
    started_at_ms
        .saturating_add(u64::from(time_limit_seconds) * 1000)
        .saturating_sub(now_ms() as u64)
}

fn sorted_minesweeper_score_players(players: &[RoomPlayerSnapshot]) -> Vec<RoomPlayerSnapshot> {
    let mut players = players.to_vec();
    players.sort_by(|left, right| {
        right
            .minesweeper_score
            .cmp(&left.minesweeper_score)
            .then_with(|| {
                left.minesweeper_last_score_ms
                    .unwrap_or(u64::MAX)
                    .cmp(&right.minesweeper_last_score_ms.unwrap_or(u64::MAX))
            })
            .then_with(|| left.name.cmp(&right.name))
    });
    players
}

fn room_game_button_class(current: RoomGameKind, button: RoomGameKind) -> &'static str {
    if current == button {
        "mode-button active"
    } else {
        "mode-button"
    }
}

fn room_minesweeper_can_act(
    phase: &RoomPhase,
    players: &[RoomPlayerSnapshot],
    player_id: &str,
) -> bool {
    matches!(phase, RoomPhase::Racing { .. })
        && players
            .iter()
            .find(|player| player.id == player_id)
            .map(|player| player.connected && !player.minesweeper_eliminated)
            .unwrap_or(false)
}

fn room_minesweeper_face(
    phase: &RoomPhase,
    players: &[RoomPlayerSnapshot],
    player_id: &str,
) -> &'static str {
    let eliminated = players
        .iter()
        .find(|player| player.id == player_id)
        .map(|player| player.minesweeper_eliminated)
        .unwrap_or(false);
    if eliminated {
        return ":(";
    }
    match phase {
        RoomPhase::Countdown { .. } => ":O",
        RoomPhase::Complete { .. } => "B)",
        RoomPhase::Lobby | RoomPhase::Racing { .. } => ":)",
    }
}

fn room_minesweeper_own_flags(players: &[RoomPlayerSnapshot], player_id: &str) -> BTreeSet<usize> {
    players
        .iter()
        .find(|player| player.id == player_id)
        .map(|player| player.minesweeper_flags.iter().copied().collect())
        .unwrap_or_default()
}

fn room_minesweeper_other_flag_count(
    players: &[RoomPlayerSnapshot],
    player_id: &str,
    index: usize,
) -> usize {
    players
        .iter()
        .filter(|player| player.id != player_id)
        .filter(|player| player.minesweeper_flags.contains(&index))
        .count()
}

fn room_minesweeper_visible_other_flag_count(
    players: &[RoomPlayerSnapshot],
    player_id: &str,
    own_flag: bool,
    index: usize,
) -> usize {
    if own_flag {
        0
    } else {
        room_minesweeper_other_flag_count(players, player_id, index)
    }
}

fn room_minesweeper_cell_size(width: usize) -> usize {
    match width {
        0..=30 => 24,
        31..=60 => 18,
        _ => 14,
    }
}

fn room_minesweeper_cell_class(
    cell: &RoomMinesweeperCellSnapshot,
    own_flag: bool,
    other_flag_count: usize,
    pressed: bool,
    start_countdown: Option<u8>,
    owner_color_index: Option<u8>,
) -> String {
    let mut class_name = String::from("ms-cell room-ms-cell");
    if cell.revealed || pressed || start_countdown.is_some() {
        class_name.push_str(" revealed");
    } else {
        class_name.push_str(" raised");
    }
    if own_flag && !cell.revealed {
        class_name.push_str(" flagged");
    }
    if other_flag_count > 0 && !cell.revealed {
        class_name.push_str(" has-other-flag");
    }
    if cell.revealed && cell.mine {
        class_name.push_str(" mine");
    }
    if cell.detonated {
        class_name.push_str(" detonated");
    }
    if cell.revealed && !cell.mine {
        if let Some(adjacent) = cell.adjacent_mines {
            if adjacent > 0 {
                class_name.push_str(&format!(" n{adjacent}"));
            }
        }
    }
    if let Some(countdown) = start_countdown {
        class_name.push_str(&format!(" n{countdown}"));
    }
    if let Some(owner_color_index) = owner_color_index {
        class_name.push_str(&format!(" owner-color-{owner_color_index}"));
    }
    class_name
}

fn room_minesweeper_cell_text(
    cell: &RoomMinesweeperCellSnapshot,
    own_flag: bool,
    pressed: bool,
    start_countdown: Option<u8>,
) -> String {
    if let Some(countdown) = start_countdown {
        return countdown.to_string();
    }
    if pressed || (own_flag && !cell.revealed) {
        return String::new();
    }
    if cell.revealed && !cell.mine {
        return cell
            .adjacent_mines
            .filter(|adjacent| *adjacent > 0)
            .map(|adjacent| adjacent.to_string())
            .unwrap_or_default();
    }
    String::new()
}

fn room_minesweeper_cell_aria(
    index: usize,
    cell: &RoomMinesweeperCellSnapshot,
    board: &RoomMinesweeperSnapshot,
    own_flag: bool,
    start_countdown: Option<u8>,
) -> String {
    let row = index / board.width.max(1);
    let col = index % board.width.max(1);
    let state = if let Some(countdown) = start_countdown {
        format!("starting cell {countdown}")
    } else if own_flag && !cell.revealed {
        "flagged".to_string()
    } else if !cell.revealed {
        "hidden".to_string()
    } else if cell.mine {
        "mine".to_string()
    } else if cell.adjacent_mines.unwrap_or_default() == 0 {
        "clear".to_string()
    } else {
        format!("{} adjacent mines", cell.adjacent_mines.unwrap_or_default())
    };
    format!("Row {}, column {}, {}", row + 1, col + 1, state)
}

fn room_minesweeper_start_countdown_value(
    cell: &RoomMinesweeperCellSnapshot,
    countdown: Option<u8>,
) -> Option<u8> {
    (cell.start && !cell.revealed)
        .then_some(countdown)
        .flatten()
}

fn room_minesweeper_cell_owner_color_index(
    cell: &RoomMinesweeperCellSnapshot,
    players: &[RoomPlayerSnapshot],
) -> Option<u8> {
    if !cell.revealed || cell.mine || cell.adjacent_mines.unwrap_or_default() == 0 {
        return None;
    }
    let owner_id = cell.owner_id.as_deref()?;
    players
        .iter()
        .position(|player| player.id == owner_id)
        .map(|index| (index % 8 + 1) as u8)
}

fn room_minesweeper_chord_target(board: &RoomMinesweeperSnapshot, index: usize) -> Option<usize> {
    let cell = board.cells.get(index)?;
    (cell.revealed && !cell.mine && cell.adjacent_mines.unwrap_or_default() > 0).then_some(index)
}

fn room_minesweeper_pressed_neighbors(
    board: &RoomMinesweeperSnapshot,
    index: usize,
    own_flags: &BTreeSet<usize>,
) -> BTreeSet<usize> {
    if room_minesweeper_chord_target(board, index).is_none() {
        return BTreeSet::new();
    }
    room_minesweeper_neighbors(board, index)
        .into_iter()
        .filter(|neighbor| {
            board
                .cells
                .get(*neighbor)
                .map(|cell| !cell.revealed && !own_flags.contains(neighbor))
                .unwrap_or(false)
        })
        .collect()
}

fn room_minesweeper_neighbors(board: &RoomMinesweeperSnapshot, index: usize) -> Vec<usize> {
    if board.width == 0 || board.height == 0 || index >= board.cells.len() {
        return Vec::new();
    }
    let row = index / board.width;
    let col = index % board.width;
    let row_start = row.saturating_sub(1);
    let row_end = (row + 1).min(board.height.saturating_sub(1));
    let col_start = col.saturating_sub(1);
    let col_end = (col + 1).min(board.width.saturating_sub(1));
    let mut neighbors = Vec::new();
    for next_row in row_start..=row_end {
        for next_col in col_start..=col_end {
            if next_row == row && next_col == col {
                continue;
            }
            neighbors.push(next_row * board.width + next_col);
        }
    }
    neighbors
}

fn send_room_pointer_from_coordinates(
    connection: Signal<Option<RoomConnection>>,
    board_width: usize,
    board_height: usize,
    client_x: f64,
    client_y: f64,
    active_click: bool,
    force: bool,
    mut last_sent_ms: Signal<f64>,
) {
    let now = now_ms();
    if !force && now - *last_sent_ms.read() < ROOM_POINTER_SEND_INTERVAL_MS {
        return;
    }
    let Some((x, y)) = normalized_board_pointer(client_x, client_y) else {
        return;
    };
    last_sent_ms.set(now);
    send_room_message(
        connection,
        RoomClientMessage::PointerUpdate {
            pointer: Some(RoomLivePointer {
                x,
                y,
                cell_index: normalized_room_cell_index(x, y, board_width, board_height),
                active_click,
                updated_at_ms: 0,
            }),
        },
    );
}

fn normalized_room_cell_index(
    x: u16,
    y: u16,
    board_width: usize,
    board_height: usize,
) -> Option<u16> {
    if board_width == 0 || board_height == 0 {
        return None;
    }
    let col = ((usize::from(x) * board_width) / (usize::from(u16::MAX) + 1)).min(board_width - 1);
    let row = ((usize::from(y) * board_height) / (usize::from(u16::MAX) + 1)).min(board_height - 1);
    u16::try_from(row * board_width + col).ok()
}

fn room_live_pointer_is_fresh(pointer: RoomLivePointer) -> bool {
    (now_ms() as u64).saturating_sub(pointer.updated_at_ms) <= 5_000
}

fn room_live_pointer_class(active_click: bool) -> &'static str {
    if active_click {
        "replay-mouse room-live-pointer active playing"
    } else {
        "replay-mouse room-live-pointer playing"
    }
}

fn room_live_pointer_style(pointer: RoomLivePointer) -> String {
    format!(
        "--mouse-x: {:.3}%; --mouse-y: {:.3}%",
        f64::from(pointer.x) / f64::from(u16::MAX) * 100.0,
        f64::from(pointer.y) / f64::from(u16::MAX) * 100.0
    )
}

fn room_player_done(player: &RoomPlayerSnapshot) -> bool {
    player.finish_ms.is_some() || player.gave_up
}

fn race_place(players: &[RoomPlayerSnapshot], player_id: &str) -> Option<usize> {
    let mut finishers = players
        .iter()
        .filter_map(|player| {
            player
                .finish_ms
                .map(|finish_ms| (player.id.as_str(), finish_ms, player.name.as_str()))
        })
        .collect::<Vec<_>>();
    finishers.sort_by(|left, right| {
        left.1
            .cmp(&right.1)
            .then_with(|| left.2.cmp(right.2))
            .then_with(|| left.0.cmp(right.0))
    });
    finishers
        .iter()
        .position(|(id, _, _)| *id == player_id)
        .map(|index| index + 1)
}

fn format_place_time(place: Option<usize>, finish_ms: u64) -> String {
    match place {
        Some(place) if place <= 3 => {
            format!("{} {}", ordinal_place(place), format_duration_ms(finish_ms))
        }
        _ => format_duration_ms(finish_ms),
    }
}

fn ordinal_place(place: usize) -> &'static str {
    match place {
        1 => "1st",
        2 => "2nd",
        3 => "3rd",
        _ => "",
    }
}

fn sorted_leaderboard_players(players: &[RoomPlayerSnapshot]) -> Vec<RoomPlayerSnapshot> {
    let mut players = players.to_vec();
    players.sort_by(|left, right| {
        right
            .medals
            .gold
            .cmp(&left.medals.gold)
            .then_with(|| right.medals.silver.cmp(&left.medals.silver))
            .then_with(|| right.medals.bronze.cmp(&left.medals.bronze))
            .then_with(|| left.name.cmp(&right.name))
    });
    players
}

fn medal_width(count: u32, max_total: u32) -> String {
    if count == 0 {
        "0%".to_string()
    } else {
        format!("{:.2}%", count as f64 / max_total.max(1) as f64 * 100.0)
    }
}

fn current_replay_time_ms(replay_start: Option<(u64, f64)>, replay_duration_ms: u64) -> u64 {
    let Some((_, started_at_ms)) = replay_start else {
        return 0;
    };
    let cycle_ms = replay_duration_ms.saturating_add(1_500).max(2_500);
    ((now_ms() - started_at_ms).max(0.0).floor() as u64) % cycle_ms
}

fn selected_replay_player_ids(
    players: &[RoomPlayerSnapshot],
    replay_time_ms: u64,
    manual_player_ids: &[String],
) -> Vec<String> {
    let mut selected_ids = manual_player_ids
        .iter()
        .filter(|player_id| players.iter().any(|player| &player.id == *player_id))
        .fold(Vec::<String>::new(), |mut selected, player_id| {
            if !selected.contains(player_id) {
                selected.push(player_id.clone());
            }
            selected
        });

    if selected_ids.is_empty() {
        selected_ids = automatic_replay_player_ids(players, replay_time_ms);
    } else if selected_ids.len() == 1 && players.len() > 1 {
        let selected_index = replay_player_sort_index(players, &selected_ids[0]);
        let partner_index = if selected_index == 0 {
            1
        } else {
            selected_index.saturating_sub(1)
        };
        if let Some(partner) = players.get(partner_index) {
            selected_ids.push(partner.id.clone());
        }
    }

    for player in players {
        if selected_ids.len() >= 2 {
            break;
        }
        if !selected_ids.contains(&player.id) {
            selected_ids.push(player.id.clone());
        }
    }

    selected_ids.truncate(2);
    selected_ids.sort_by(|left, right| {
        replay_player_sort_index(players, left).cmp(&replay_player_sort_index(players, right))
    });
    selected_ids
}

fn replay_player_sort_key(player: &RoomPlayerSnapshot) -> (u8, u64) {
    player
        .finish_ms
        .map(|finish_ms| (0, finish_ms))
        .unwrap_or((1, u64::MAX))
}

fn automatic_replay_player_ids(players: &[RoomPlayerSnapshot], replay_time_ms: u64) -> Vec<String> {
    match players.len() {
        0 => Vec::new(),
        1 => vec![players[0].id.clone()],
        player_count => {
            let first_unfinished = players
                .iter()
                .position(|player| {
                    player
                        .finish_ms
                        .map(|finish_ms| finish_ms > replay_time_ms)
                        .unwrap_or(true)
                })
                .unwrap_or(player_count - 1);
            let first_index = first_unfinished.min(player_count - 2);
            players[first_index..=first_index + 1]
                .iter()
                .map(|player| player.id.clone())
                .collect()
        }
    }
}

fn replay_player_sort_index(players: &[RoomPlayerSnapshot], player_id: &str) -> usize {
    players
        .iter()
        .position(|player| player.id == player_id)
        .unwrap_or(usize::MAX)
}

fn replay_players_by_id(
    players: &[RoomPlayerSnapshot],
    player_ids: &[String],
) -> Vec<RoomPlayerSnapshot> {
    player_ids
        .iter()
        .filter_map(|player_id| {
            players
                .iter()
                .find(|player| &player.id == player_id)
                .cloned()
        })
        .collect()
}

fn toggle_replay_player_selection(
    mut replay_manual_player_ids: Signal<Vec<String>>,
    player_id: String,
) {
    let mut selected = replay_manual_player_ids.read().clone();
    if let Some(index) = selected
        .iter()
        .position(|selected_id| selected_id == &player_id)
    {
        selected.remove(index);
    } else {
        if selected.len() >= 2 {
            selected.remove(0);
        }
        selected.push(player_id);
    }
    replay_manual_player_ids.set(selected);
}

fn replay_racer_button_class(is_displayed: bool, is_manual: bool) -> &'static str {
    match (is_displayed, is_manual) {
        (true, true) => "replay-racer-button active manual",
        (true, false) => "replay-racer-button active",
        (false, true) => "replay-racer-button manual",
        (false, false) => "replay-racer-button",
    }
}

fn replay_duration_ms(players: &[RoomPlayerSnapshot]) -> u64 {
    players
        .iter()
        .filter_map(|player| player.recording.as_ref())
        .filter_map(|recording| recording.frames.last())
        .map(|frame| frame.elapsed_ms)
        .max()
        .unwrap_or(0)
        .max(1)
}

fn set_replay_scrubber_value(replay_time_ms: u64) {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    let Some(input) = document
        .get_element_by_id(REPLAY_SCRUBBER_ID)
        .and_then(|element| element.dyn_into::<web_sys::HtmlInputElement>().ok())
    else {
        return;
    };
    input.set_value(&replay_time_ms.to_string());
}

fn replay_states(
    recording: &RoomRecording,
    replay_time_ms: u64,
    expected_cells: usize,
) -> Vec<CellState> {
    let frame = recording
        .frames
        .iter()
        .take_while(|frame| frame.elapsed_ms <= replay_time_ms)
        .last()
        .or_else(|| recording.frames.first());
    let Some(frame) = frame else {
        return vec![CellState::Empty; expected_cells];
    };
    if frame.states.len() != expected_cells {
        return vec![CellState::Empty; expected_cells];
    }
    frame
        .states
        .iter()
        .map(|state| CellState::from_storage_code(*state))
        .collect()
}

fn replay_mouse_pointer(
    recording: &RoomMouseRecording,
    replay_time_ms: u64,
) -> Option<ReplayMousePointer> {
    let replay_time_ms = u32::try_from(replay_time_ms).unwrap_or(u32::MAX);
    let (x, y) = interpolated_mouse_position(recording, replay_time_ms)?;
    let active_click = recording.events.iter().rev().any(|event| {
        event.0 <= replay_time_ms
            && replay_time_ms.saturating_sub(event.0) <= 180
            && matches!(
                event.1,
                ROOM_MOUSE_EVENT_PRIMARY_DOWN
                    | ROOM_MOUSE_EVENT_PRIMARY_UP
                    | ROOM_MOUSE_EVENT_SECONDARY_DOWN
                    | ROOM_MOUSE_EVENT_SECONDARY_UP
            )
    });

    Some(ReplayMousePointer {
        x_percent: format!("{:.3}%", x / f64::from(u16::MAX) * 100.0),
        y_percent: format!("{:.3}%", y / f64::from(u16::MAX) * 100.0),
        active_click,
    })
}

fn interpolated_mouse_position(
    recording: &RoomMouseRecording,
    replay_time_ms: u32,
) -> Option<(f64, f64)> {
    let mut previous = None;
    let mut next = None;
    for sample in &recording.samples {
        if sample.0 <= replay_time_ms {
            previous = Some(*sample);
        } else {
            next = Some(*sample);
            break;
        }
    }

    match (previous, next) {
        (Some(previous), Some(next)) if next.0 > previous.0 => {
            let progress = f64::from(replay_time_ms.saturating_sub(previous.0))
                / f64::from(next.0 - previous.0);
            Some((
                lerp_u16(previous.1, next.1, progress),
                lerp_u16(previous.2, next.2, progress),
            ))
        }
        (Some(sample), _) | (None, Some(sample)) => {
            Some((f64::from(sample.1), f64::from(sample.2)))
        }
        (None, None) => recording
            .events
            .iter()
            .take_while(|event| event.0 <= replay_time_ms)
            .last()
            .map(|event| (f64::from(event.2), f64::from(event.3))),
    }
}

fn lerp_u16(start: u16, end: u16, progress: f64) -> f64 {
    f64::from(start) + (f64::from(end) - f64::from(start)) * progress.clamp(0.0, 1.0)
}

fn replay_mouse_class(active_click: bool, is_playing: bool) -> &'static str {
    match (active_click, is_playing) {
        (true, true) => "replay-mouse active playing",
        (true, false) => "replay-mouse active",
        (false, true) => "replay-mouse playing",
        (false, false) => "replay-mouse",
    }
}

fn replay_cell_class(cell: &CellView, state: CellState) -> String {
    let mut class_name = cell.class_name();
    class_name.push_str(" replay-cell");
    if state.is_marked() {
        class_name.push_str(" marked");
    }
    if state == CellState::AutoMark {
        class_name.push_str(" auto-marked");
    }
    if state == CellState::Queen {
        class_name.push_str(" queen");
    }
    class_name
}

fn format_duration_ms(milliseconds: u64) -> String {
    let total_seconds = milliseconds / 1000;
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    let tenths = (milliseconds % 1000) / 100;
    format!("{minutes:02}:{seconds:02}.{tenths}")
}

fn format_minesweeper_counter(value: i32) -> String {
    let value = value.clamp(-99, 999);
    if value < 0 {
        format!("-{:02}", value.abs())
    } else {
        format!("{value:03}")
    }
}

fn minesweeper_face(snapshot: &MinesweeperGameState) -> &'static str {
    match snapshot.board.status {
        MinesweeperStatus::Lost => ":(",
        MinesweeperStatus::Won => "B)",
        MinesweeperStatus::Ready | MinesweeperStatus::Playing if snapshot.face_down => ":O",
        MinesweeperStatus::Ready | MinesweeperStatus::Playing => ":)",
    }
}

fn minesweeper_cell_class(
    cell: &MinesweeperCell,
    status: MinesweeperStatus,
    pressed: bool,
) -> String {
    let mut class_name = String::from("ms-cell");
    match cell.state {
        MinesweeperCellState::Hidden => class_name.push_str(" raised"),
        MinesweeperCellState::Flagged => class_name.push_str(" raised flagged"),
        MinesweeperCellState::Question => class_name.push_str(" raised question"),
        MinesweeperCellState::Revealed => class_name.push_str(" revealed"),
    }
    if pressed {
        class_name.push_str(" pressed");
    }
    if cell.state == MinesweeperCellState::Revealed && cell.mine {
        class_name.push_str(" mine");
    }
    if cell.detonated {
        class_name.push_str(" detonated");
    }
    if status == MinesweeperStatus::Lost
        && cell.state == MinesweeperCellState::Flagged
        && !cell.mine
    {
        class_name.push_str(" wrong-flag");
    }
    if cell.state == MinesweeperCellState::Revealed && cell.adjacent_mines > 0 {
        class_name.push_str(&format!(" n{}", cell.adjacent_mines));
    }
    class_name
}

fn minesweeper_cell_text(cell: &MinesweeperCell, pressed: bool) -> String {
    if pressed {
        return String::new();
    }
    match cell.state {
        MinesweeperCellState::Question => "?".to_string(),
        MinesweeperCellState::Revealed if !cell.mine && cell.adjacent_mines > 0 => {
            cell.adjacent_mines.to_string()
        }
        MinesweeperCellState::Hidden
        | MinesweeperCellState::Flagged
        | MinesweeperCellState::Revealed => String::new(),
    }
}

fn minesweeper_chord_target(board: &MinesweeperBoard, index: usize) -> Option<usize> {
    let cell = board.cells.get(index)?;
    (cell.state == MinesweeperCellState::Revealed && cell.adjacent_mines > 0).then_some(index)
}

fn minesweeper_pressed_neighbors(board: &MinesweeperBoard, index: usize) -> BTreeSet<usize> {
    if minesweeper_chord_target(board, index).is_none() {
        return BTreeSet::new();
    }
    board
        .neighbors(index)
        .into_iter()
        .filter(|neighbor| {
            matches!(
                board.cells[*neighbor].state,
                MinesweeperCellState::Hidden | MinesweeperCellState::Question
            )
        })
        .collect()
}

fn minesweeper_cell_aria(index: usize, cell: &MinesweeperCell, board: &MinesweeperBoard) -> String {
    let (row, col) = board.row_col(index).unwrap_or((0, 0));
    let state = match cell.state {
        MinesweeperCellState::Hidden => "hidden".to_string(),
        MinesweeperCellState::Flagged => "flagged".to_string(),
        MinesweeperCellState::Question => "question marked".to_string(),
        MinesweeperCellState::Revealed if cell.mine => "mine".to_string(),
        MinesweeperCellState::Revealed if cell.adjacent_mines == 0 => "clear".to_string(),
        MinesweeperCellState::Revealed => {
            format!("{} adjacent mines", cell.adjacent_mines)
        }
    };
    format!("Row {}, column {}, {}", row + 1, col + 1, state)
}

fn local_storage() -> Option<web_sys::Storage> {
    web_sys::window().and_then(|window| window.local_storage().ok().flatten())
}

fn now_ms() -> f64 {
    js_sys::Date::now()
}

fn seed() -> u64 {
    let random = (js_sys::Math::random() * u32::MAX as f64) as u64;
    ((now_ms() as u64) << 21) ^ random ^ 0x9e37_79b9_7f4a_7c15
}

fn normalized_board_pointer(client_x: f64, client_y: f64) -> Option<(u16, u16)> {
    let document = web_sys::window()?.document()?;
    let board = document.get_element_by_id(ROOM_BOARD_ID)?;
    let rect = board.get_bounding_client_rect();
    let width = rect.width();
    let height = rect.height();
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    let x = ((client_x - rect.left()) / width).clamp(0.0, 1.0);
    let y = ((client_y - rect.top()) / height).clamp(0.0, 1.0);
    Some((normalized_pointer_axis(x), normalized_pointer_axis(y)))
}

fn normalized_pointer_axis(value: f64) -> u16 {
    (value * u16::MAX as f64)
        .round()
        .clamp(0.0, u16::MAX as f64) as u16
}

fn validation_status(validation: &ValidateResponse, size: usize) -> String {
    let base = format!(
        "{}/{} rows · {}/{} columns · {}/{} regions",
        validation.satisfied_rows,
        size,
        validation.satisfied_columns,
        size,
        validation.satisfied_regions,
        size
    );
    if let Some(message) = validation.messages.first() {
        format!("{base} - {message}")
    } else {
        base
    }
}

fn mode_button_class(current: Mode, mode: Mode) -> &'static str {
    if current == mode {
        "mode-button active"
    } else {
        "mode-button"
    }
}

fn cell_class(cell: &CellView, state: CellState, conflict_cells: &BTreeSet<[usize; 2]>) -> String {
    let mut class_name = cell.class_name();
    if state.is_marked() {
        class_name.push_str(" marked");
    }
    if state == CellState::AutoMark {
        class_name.push_str(" auto-marked");
    }
    if state == CellState::Queen {
        class_name.push_str(" queen");
    }
    if conflict_cells.contains(&[cell.row, cell.col]) {
        class_name.push_str(" conflict");
    }
    class_name
}

fn cell_aria(cell: &CellView, state: CellState) -> String {
    let marker = match state {
        CellState::Queen => ", queen",
        CellState::Mark | CellState::AutoMark => ", marked",
        CellState::Empty => "",
    };
    format!("Row {}, column {}{}", cell.row + 1, cell.col + 1, marker)
}

#[cfg(test)]
mod tests {
    use super::*;
    use queensgame_shared::RoomMedalCounts;

    fn replay_player(id: &str, name: &str, finish_ms: u64) -> RoomPlayerSnapshot {
        RoomPlayerSnapshot {
            id: id.to_string(),
            name: name.to_string(),
            ready: false,
            connected: true,
            finish_ms: Some(finish_ms),
            gave_up: false,
            medals: RoomMedalCounts::default(),
            recording: Some(RoomRecording {
                frames: vec![RoomRecordingFrame {
                    elapsed_ms: finish_ms,
                    states: Vec::new(),
                }],
            }),
            mouse_recording: None,
            minesweeper_score: 0,
            minesweeper_eliminated: false,
            minesweeper_last_score_ms: None,
            minesweeper_flags: Vec::new(),
            pointer: None,
        }
    }

    fn live_replay_player(id: &str, name: &str) -> RoomPlayerSnapshot {
        RoomPlayerSnapshot {
            id: id.to_string(),
            name: name.to_string(),
            ready: false,
            connected: true,
            finish_ms: None,
            gave_up: false,
            medals: RoomMedalCounts::default(),
            recording: Some(RoomRecording {
                frames: vec![RoomRecordingFrame {
                    elapsed_ms: 1_500,
                    states: Vec::new(),
                }],
            }),
            mouse_recording: None,
            minesweeper_score: 0,
            minesweeper_eliminated: false,
            minesweeper_last_score_ms: None,
            minesweeper_flags: Vec::new(),
            pointer: None,
        }
    }

    fn test_room_minesweeper_cell(
        mine: bool,
        adjacent_mines: u8,
        revealed: bool,
    ) -> RoomMinesweeperCellSnapshot {
        RoomMinesweeperCellSnapshot {
            revealed,
            mine,
            detonated: false,
            start: false,
            adjacent_mines: Some(adjacent_mines),
            owner_id: None,
        }
    }

    fn test_room_minesweeper_snapshot(
        width: usize,
        height: usize,
        cells: Vec<RoomMinesweeperCellSnapshot>,
    ) -> RoomSnapshot {
        RoomSnapshot {
            slug: "ROOMTEST".to_string(),
            game_kind: RoomGameKind::Minesweeper,
            phase: RoomPhase::Racing { started_at_ms: 0 },
            puzzle_choice: RoomPuzzleChoice::Random,
            minesweeper_time_limit_seconds: 99,
            played_puzzle_ids: Vec::new(),
            players: vec![live_replay_player("ada", "Ada")],
            puzzle: None,
            minesweeper: Some(RoomMinesweeperSnapshot {
                width,
                height,
                mines: cells.iter().filter(|cell| cell.mine).count(),
                starting_cells: Vec::new(),
                cells,
            }),
            winner_id: None,
        }
    }

    fn replay_players() -> Vec<RoomPlayerSnapshot> {
        vec![
            replay_player("p1", "Ada", 1_000),
            replay_player("p2", "Bea", 2_000),
            replay_player("p3", "Cam", 3_000),
            replay_player("p4", "Dee", 4_000),
        ]
    }

    #[test]
    fn automatic_replay_pair_follows_finish_line() {
        let players = replay_players();

        assert_eq!(
            automatic_replay_player_ids(&players, 0),
            vec!["p1".to_string(), "p2".to_string()]
        );
        assert_eq!(
            automatic_replay_player_ids(&players, 999),
            vec!["p1".to_string(), "p2".to_string()]
        );
        assert_eq!(
            automatic_replay_player_ids(&players, 1_000),
            vec!["p2".to_string(), "p3".to_string()]
        );
        assert_eq!(
            automatic_replay_player_ids(&players, 2_500),
            vec!["p3".to_string(), "p4".to_string()]
        );
        assert_eq!(
            automatic_replay_player_ids(&players, 5_000),
            vec!["p3".to_string(), "p4".to_string()]
        );
    }

    #[test]
    fn automatic_replay_pair_treats_missing_finish_as_in_progress() {
        let players = vec![
            replay_player("p1", "Ada", 1_000),
            live_replay_player("p2", "Bea"),
            live_replay_player("p3", "Cam"),
        ];

        assert_eq!(
            automatic_replay_player_ids(&players, 1_500),
            vec!["p2".to_string(), "p3".to_string()]
        );
    }

    #[test]
    fn manual_replay_pair_is_sorted_by_finish_time() {
        let players = replay_players();

        assert_eq!(
            selected_replay_player_ids(&players, 0, &["p3".to_string(), "p1".to_string()]),
            vec!["p1".to_string(), "p3".to_string()]
        );
        assert_eq!(
            selected_replay_player_ids(&players, 0, &["p3".to_string()]),
            vec!["p2".to_string(), "p3".to_string()]
        );
    }

    #[test]
    fn mouse_replay_uses_latest_sample_and_click_window() {
        let recording = RoomMouseRecording {
            samples: vec![RoomMouseSample(0, 0, 0), RoomMouseSample(100, 0, u16::MAX)],
            events: vec![RoomMouseEvent(
                120,
                ROOM_MOUSE_EVENT_PRIMARY_DOWN,
                0,
                u16::MAX,
                Some(10),
            )],
        };

        let pointer = replay_mouse_pointer(&recording, 50).expect("mouse pointer");
        assert_eq!(pointer.x_percent, "0.000%");
        assert_eq!(pointer.y_percent, "50.000%");

        let pointer = replay_mouse_pointer(&recording, 200).expect("mouse pointer");
        assert_eq!(pointer.x_percent, "0.000%");
        assert_eq!(pointer.y_percent, "100.000%");
        assert!(pointer.active_click);

        let pointer = replay_mouse_pointer(&recording, 400).expect("mouse pointer");
        assert!(!pointer.active_click);
    }

    #[test]
    fn minesweeper_pressed_neighbors_only_include_unopened_cells() {
        let mut board = MinesweeperBoard::new(3, 3, 1, 42);
        let center = board.index(1, 1).unwrap();
        let hidden = board.index(0, 0).unwrap();
        let question = board.index(0, 1).unwrap();
        let flagged = board.index(0, 2).unwrap();
        let revealed = board.index(1, 0).unwrap();
        board.cells[center].state = MinesweeperCellState::Revealed;
        board.cells[center].adjacent_mines = 2;
        board.cells[question].state = MinesweeperCellState::Question;
        board.cells[flagged].state = MinesweeperCellState::Flagged;
        board.cells[revealed].state = MinesweeperCellState::Revealed;

        let pressed = minesweeper_pressed_neighbors(&board, center);

        assert!(pressed.contains(&hidden));
        assert!(pressed.contains(&question));
        assert!(!pressed.contains(&flagged));
        assert!(!pressed.contains(&revealed));
        assert_eq!(minesweeper_cell_text(&board.cells[question], true), "");
    }

    #[test]
    fn minesweeper_counter_formats_to_three_digits() {
        assert_eq!(format_minesweeper_counter(99), "099");
        assert_eq!(format_minesweeper_counter(-4), "-04");
    }

    #[test]
    fn minesweeper_scoreboard_sorts_by_score_then_last_score_time() {
        let mut players = vec![
            live_replay_player("ada", "Ada"),
            live_replay_player("bea", "Bea"),
            live_replay_player("cam", "Cam"),
        ];
        players[0].minesweeper_score = 9;
        players[0].minesweeper_last_score_ms = Some(900);
        players[1].minesweeper_score = 12;
        players[1].minesweeper_last_score_ms = Some(950);
        players[2].minesweeper_score = 9;
        players[2].minesweeper_last_score_ms = Some(700);

        let sorted = sorted_minesweeper_score_players(&players);

        assert_eq!(
            sorted
                .iter()
                .map(|player| player.id.as_str())
                .collect::<Vec<_>>(),
            vec!["bea", "cam", "ada"]
        );
    }

    #[test]
    fn own_minesweeper_flag_hides_other_flag_overlay() {
        let mut players = vec![
            live_replay_player("ada", "Ada"),
            live_replay_player("bea", "Bea"),
            live_replay_player("cam", "Cam"),
        ];
        players[0].minesweeper_flags = vec![7];
        players[1].minesweeper_flags = vec![7, 8];
        players[2].minesweeper_flags = vec![7];

        assert_eq!(
            room_minesweeper_visible_other_flag_count(&players, "ada", true, 7),
            0
        );
        assert_eq!(
            room_minesweeper_visible_other_flag_count(&players, "ada", false, 8),
            1
        );
        assert_eq!(
            room_minesweeper_visible_other_flag_count(&players, "guest", false, 7),
            3
        );
    }

    #[test]
    fn optimistic_minesweeper_reveal_opens_safe_area_and_scores_numbers() {
        let mut snapshot = test_room_minesweeper_snapshot(
            3,
            3,
            vec![
                test_room_minesweeper_cell(false, 0, false),
                test_room_minesweeper_cell(false, 0, false),
                test_room_minesweeper_cell(false, 0, false),
                test_room_minesweeper_cell(false, 0, false),
                test_room_minesweeper_cell(false, 1, false),
                test_room_minesweeper_cell(false, 1, false),
                test_room_minesweeper_cell(false, 0, false),
                test_room_minesweeper_cell(false, 1, false),
                test_room_minesweeper_cell(true, 0, false),
            ],
        );

        optimistic_room_minesweeper_reveal_snapshot(&mut snapshot, "ada", 0);

        let board = snapshot.minesweeper.as_ref().expect("board");
        assert!(board.cells[..8].iter().all(|cell| cell.revealed));
        assert!(!board.cells[8].revealed);
        assert_eq!(snapshot.players[0].minesweeper_score, 3);
        assert_eq!(board.cells[4].owner_id.as_deref(), Some("ada"));
        assert_eq!(board.cells[5].owner_id.as_deref(), Some("ada"));
        assert_eq!(board.cells[7].owner_id.as_deref(), Some("ada"));
        assert_eq!(board.cells[0].owner_id, None);
    }

    #[test]
    fn optimistic_minesweeper_flag_blocks_local_reveal_until_removed() {
        let mut snapshot = test_room_minesweeper_snapshot(
            2,
            2,
            vec![
                test_room_minesweeper_cell(false, 1, false),
                test_room_minesweeper_cell(true, 0, false),
                test_room_minesweeper_cell(false, 1, false),
                test_room_minesweeper_cell(false, 1, false),
            ],
        );

        optimistic_room_minesweeper_toggle_flag_snapshot(&mut snapshot, "ada", 0);
        optimistic_room_minesweeper_reveal_snapshot(&mut snapshot, "ada", 0);
        assert_eq!(snapshot.players[0].minesweeper_flags, vec![0]);
        assert!(!snapshot.minesweeper.as_ref().unwrap().cells[0].revealed);

        optimistic_room_minesweeper_toggle_flag_snapshot(&mut snapshot, "ada", 0);
        optimistic_room_minesweeper_reveal_snapshot(&mut snapshot, "ada", 0);
        assert!(snapshot.players[0].minesweeper_flags.is_empty());
        assert!(snapshot.minesweeper.as_ref().unwrap().cells[0].revealed);
    }

    #[test]
    fn optimistic_minesweeper_chord_uses_own_flags() {
        let mut snapshot = test_room_minesweeper_snapshot(
            2,
            2,
            vec![
                test_room_minesweeper_cell(false, 1, true),
                test_room_minesweeper_cell(true, 0, false),
                test_room_minesweeper_cell(false, 1, false),
                test_room_minesweeper_cell(false, 1, false),
            ],
        );
        snapshot.players[0].minesweeper_flags = vec![1];

        optimistic_room_minesweeper_chord_snapshot(&mut snapshot, "ada", 0);

        let board = snapshot.minesweeper.as_ref().expect("board");
        assert!(board.cells[2].revealed);
        assert!(board.cells[3].revealed);
        assert!(!board.cells[1].revealed);
        assert_eq!(snapshot.players[0].minesweeper_score, 2);
        assert_eq!(board.cells[2].owner_id.as_deref(), Some("ada"));
        assert_eq!(board.cells[3].owner_id.as_deref(), Some("ada"));
    }

    #[test]
    fn countdown_start_cells_render_as_revealed_digits() {
        let mut cell = test_room_minesweeper_cell(false, 2, false);
        cell.start = true;
        let countdown = room_minesweeper_start_countdown_value(&cell, Some(5));

        let class_name = room_minesweeper_cell_class(&cell, false, 0, false, countdown, None);

        assert_eq!(
            room_minesweeper_cell_text(&cell, false, false, countdown),
            "5"
        );
        assert!(class_name.contains("revealed"));
        assert!(class_name.contains("n5"));
        assert!(!class_name.contains("raised"));
        assert!(!class_name.contains("start"));
    }

    #[test]
    fn claimed_number_cells_get_player_color_class() {
        let mut cell = test_room_minesweeper_cell(false, 3, true);
        cell.owner_id = Some("bea".to_string());
        let players = vec![
            live_replay_player("ada", "Ada"),
            live_replay_player("bea", "Bea"),
        ];
        let owner_color_index = room_minesweeper_cell_owner_color_index(&cell, &players);

        let class_name =
            room_minesweeper_cell_class(&cell, false, 0, false, None, owner_color_index);

        assert_eq!(owner_color_index, Some(2));
        assert!(class_name.contains("owner-color-2"));
    }
}
