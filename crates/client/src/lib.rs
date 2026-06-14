use dioxus::{html::input_data::MouseButton, prelude::*};
use gloo_timers::future::TimeoutFuture;
use queensgame_shared::{
    build_cells, invalidated_by_queen, normalize_display_name, validate_solution, CellState,
    CellView, GameBootstrap, Puzzle, PuzzleNav, RoomBootstrap, RoomClientMessage, RoomMouseEvent,
    RoomMouseRecording, RoomMouseSample, RoomPhase, RoomPlayerSnapshot, RoomPuzzleChoice,
    RoomRecording, RoomRecordingFrame, RoomServerMessage, RoomSnapshot, ValidateResponse,
    DISPLAY_NAME_MAX_CHARS, ROOM_MOUSE_EVENT_ENTER, ROOM_MOUSE_EVENT_LEAVE,
    ROOM_MOUSE_EVENT_PRIMARY_DOWN, ROOM_MOUSE_EVENT_PRIMARY_UP, ROOM_MOUSE_EVENT_SECONDARY_DOWN,
    ROOM_MOUSE_EVENT_SECONDARY_UP,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, rc::Rc};
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
    history: Vec<Vec<CellState>>,
    started_at_ms: f64,
    completed: bool,
    completed_ms: u64,
    validation: ValidateResponse,
    finish_notified: bool,
    mark_drag: Option<MarkDrag>,
    win_visible: bool,
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

    use_effect(move || match room_snapshot.read().phase {
        RoomPhase::Complete { started_at_ms } => {
            let replay_start = *replay_started_at_ms.read();
            let current_key = replay_start.map(|(key, _)| key);
            if current_key != Some(started_at_ms) {
                replay_started_at_ms.set(Some((started_at_ms, now_ms())));
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
    });

    let snapshot = room_snapshot.read().clone();
    let status = connection_status.read().clone();
    let is_joined = connection.read().is_some();
    let pending_name_value = pending_name.read().clone();
    let name_error_text = name_error.read().clone();
    let _ = *tick.read();
    let me = snapshot
        .players
        .iter()
        .find(|player| player.id == player_id)
        .cloned();
    let my_ready = me.as_ref().map(|player| player.ready).unwrap_or(false);
    let my_finished = me.as_ref().and_then(|player| player.finish_ms).is_some();
    let ready_text = if my_ready { "Not Ready" } else { "Ready" };
    let room_url = current_room_url(&snapshot.slug);
    let choice = puzzle_choice_label(&snapshot.puzzle_choice);
    let can_select = matches!(
        snapshot.phase,
        RoomPhase::Lobby | RoomPhase::Complete { .. }
    );
    let countdown = countdown_label(&snapshot.phase);
    let race_started_at_ms = snapshot.phase.race_started_at_ms();
    let replay_duration_ms = replay_duration_ms(&snapshot.players);
    let replay_scrubbed_time_ms = *replay_scrub_ms.read();
    let replay_time_ms = replay_scrubbed_time_ms
        .map(|time| time.min(replay_duration_ms))
        .unwrap_or_else(|| {
            current_replay_time_ms(*replay_started_at_ms.read(), replay_duration_ms)
        });
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

    rsx! {
        main { class: "game-page room-page",
            section { class: "game-shell", aria_labelledby: "room-title",
                div { class: "game-toolbar",
                    div {
                        p { class: "eyebrow", "Room {snapshot.slug}" }
                        h1 { id: "room-title", "Multiplayer Race" }
                    }
                    div { class: "timer-box", aria_live: "polite",
                        span { class: "timer-label", "Status" }
                        span { "{status}" }
                    }
                }

                div { class: "room-share",
                    label { "Invite link" }
                    input { readonly: true, value: "{room_url}" }
                }

                match &snapshot.phase {
                    RoomPhase::Lobby | RoomPhase::Countdown { .. } => rsx! {
                        div { class: "room-lobby",
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
                                            class: puzzle_choice_button_class(&snapshot.puzzle_choice, puzzle_id),
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
                        if my_finished {
                            div { class: "status-strip",
                                span { "Finished" }
                                span { "Waiting for the remaining racers." }
                            }
                        }
                        if matches!(snapshot.phase, RoomPhase::Complete { .. }) {
                            div { class: "room-lobby next-race-panel",
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
                                                class: puzzle_choice_button_class(&snapshot.puzzle_choice, puzzle_id),
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
                                    players: snapshot.players.clone(),
                                    replay_time_ms,
                                    replay_duration_ms,
                                    replay_scrub_ms,
                                    replay_started_at_ms,
                                    replay_manual_player_ids
                                }
                            }
                        }
                        if let Some(game) = race_game.read().as_ref().cloned() {
                            RoomBoard { game_state: race_game, snapshot: game }
                        } else {
                            div { class: "rule-panel", "Waiting for the puzzle..." }
                        }
                    },
                }
            }

            aside { class: "side-panel", aria_label: "Players",
                div { class: "selector-header",
                    p { class: "eyebrow", "Players" }
                    h2 { "{snapshot.players.len()} in room" }
                }
                div { class: "player-list",
                    for player in snapshot.players.iter() {
                        div {
                            class: player_row_class(player, snapshot.winner_id.as_deref()),
                            span { class: "player-name", "{player.name}" }
                            span { class: "player-status", "{player_status(player, &snapshot.phase, race_started_at_ms)}" }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn RoomBoard(game_state: Signal<Option<GameState>>, snapshot: GameState) -> Element {
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
    mut replay_scrub_ms: Signal<Option<u64>>,
    mut replay_started_at_ms: Signal<Option<(u64, f64)>>,
    mut replay_manual_player_ids: Signal<Vec<String>>,
) -> Element {
    let _smooth_scrubber = use_future(move || async move {
        loop {
            TimeoutFuture::new(16).await;
            if (*replay_scrub_ms.read()).is_none() {
                let replay_time_ms =
                    current_replay_time_ms(*replay_started_at_ms.read(), replay_duration_ms)
                        .min(replay_duration_ms);
                set_replay_scrubber_value(replay_time_ms);
            }
        }
    });

    players.retain(|player| player.recording.is_some() && player.finish_ms.is_some());
    players.sort_by(|left, right| {
        left.finish_ms
            .cmp(&right.finish_ms)
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
    let is_paused = scrubbed_time_ms.is_some();
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
                            .unwrap_or_else(|| "In progress".to_string());
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

fn puzzle_choice_button_class(choice: &RoomPuzzleChoice, puzzle_id: usize) -> &'static str {
    if matches!(choice, RoomPuzzleChoice::Puzzle { id } if *id == puzzle_id) {
        "active"
    } else {
        ""
    }
}

fn countdown_label(phase: &RoomPhase) -> Option<String> {
    let RoomPhase::Countdown { starts_at_ms } = phase else {
        return None;
    };
    let remaining_ms = starts_at_ms.saturating_sub(now_ms() as u64);
    let tenths = remaining_ms.div_ceil(100);
    Some(format!("{}.{:01}", tenths / 10, tenths % 10))
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
            if let Some(finish_ms) = player.finish_ms {
                return format_duration_ms(finish_ms);
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
            } else if let Some(finish_ms) = player.finish_ms {
                format_duration_ms(finish_ms)
            } else {
                "Not ready".to_string()
            }
        }
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

fn automatic_replay_player_ids(players: &[RoomPlayerSnapshot], replay_time_ms: u64) -> Vec<String> {
    match players.len() {
        0 => Vec::new(),
        1 => vec![players[0].id.clone()],
        player_count => {
            let first_unfinished = players
                .iter()
                .position(|player| player.finish_ms.unwrap_or(0) > replay_time_ms)
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

fn local_storage() -> Option<web_sys::Storage> {
    web_sys::window().and_then(|window| window.local_storage().ok().flatten())
}

fn now_ms() -> f64 {
    js_sys::Date::now()
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

    fn replay_player(id: &str, name: &str, finish_ms: u64) -> RoomPlayerSnapshot {
        RoomPlayerSnapshot {
            id: id.to_string(),
            name: name.to_string(),
            ready: false,
            connected: true,
            finish_ms: Some(finish_ms),
            recording: Some(RoomRecording {
                frames: vec![RoomRecordingFrame {
                    elapsed_ms: finish_ms,
                    states: Vec::new(),
                }],
            }),
            mouse_recording: None,
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
}
