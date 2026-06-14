use dioxus::{html::input_data::MouseButton, prelude::*};
use gloo_timers::future::TimeoutFuture;
use queensgame_shared::{
    build_cells, invalidated_by_queen, validate_solution, CellState, CellView, GameBootstrap,
    Puzzle, PuzzleNav, RoomBootstrap, RoomClientMessage, RoomPhase, RoomPlayerSnapshot,
    RoomPuzzleChoice, RoomServerMessage, RoomSnapshot, ValidateResponse,
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
    history: Vec<Vec<CellState>>,
    started_at_ms: f64,
    completed: bool,
    completed_seconds: u64,
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

#[derive(Deserialize, Serialize)]
struct SavedGame {
    states: Vec<u8>,
    started_at_ms: f64,
    completed: bool,
    completed_seconds: u64,
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

    fn send(&self, message: &RoomClientMessage) {
        let Some(socket) = &self.socket else {
            return;
        };
        let Ok(raw) = serde_json::to_string(message) else {
            return;
        };
        let _ = socket.send_with_str(&raw);
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
            history: Vec::new(),
            started_at_ms: now_ms(),
            completed: false,
            completed_seconds: 0,
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

    fn new_room(puzzle: Puzzle) -> Self {
        let validation = validate_solution(&puzzle, &[]);
        let mut game = Self {
            cells: build_cells(&puzzle),
            states: vec![CellState::Empty; puzzle.size * puzzle.size],
            puzzle,
            puzzle_nav: Vec::new(),
            total: 0,
            mode: Mode::Queen,
            storage_key: None,
            history: Vec::new(),
            started_at_ms: now_ms(),
            completed: false,
            completed_seconds: 0,
            validation,
            finish_notified: false,
            mark_drag: None,
            win_visible: false,
        };
        game.revalidate(false);
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

    fn elapsed_seconds(&self) -> u64 {
        if self.completed {
            self.completed_seconds
        } else {
            ((now_ms() - self.started_at_ms).max(0.0) / 1000.0).floor() as u64
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
        self.completed_seconds = saved.completed_seconds;
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
            completed_seconds: self.completed_seconds,
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
            self.completed_seconds = self.elapsed_seconds();
            self.completed = true;
            self.win_visible = show_win;
        } else if !validation.complete {
            self.completed = false;
            self.completed_seconds = 0;
            self.finish_notified = false;
            self.win_visible = false;
        }

        self.validation = validation;
    }

    fn after_change(&mut self) {
        self.completed = false;
        self.completed_seconds = 0;
        self.win_visible = false;
        self.revalidate(true);
        self.save();
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
        self.completed_seconds = 0;
        self.finish_notified = false;
        self.win_visible = false;
        self.revalidate(false);
        self.save();
    }

    fn reset(&mut self) {
        self.push_history();
        self.states = vec![CellState::Empty; self.puzzle.size * self.puzzle.size];
        self.started_at_ms = now_ms();
        self.completed = false;
        self.completed_seconds = 0;
        self.finish_notified = false;
        self.win_visible = false;
        self.mark_drag = None;
        self.revalidate(false);
        self.save();
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
            TimeoutFuture::new(1000).await;
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
    let elapsed = format_time(snapshot.elapsed_seconds());
    let win_time = format!("Finished in {}.", format_time(snapshot.completed_seconds));

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
    let player_name = use_hook(|| load_or_create_player_name(&player_id));
    let room_snapshot = use_signal(|| bootstrap.snapshot.clone());
    let mut race_game = use_signal(|| None::<GameState>);
    let connection_status = use_signal(|| "Connecting".to_string());
    let mut tick = use_signal(|| 0u64);
    let _window_pointer_up = use_hook(move || RoomWindowPointerUpListener::new(race_game));
    let connection = use_hook({
        let slug = bootstrap.slug.clone();
        let player_id = player_id.clone();
        let player_name = player_name.clone();
        move || {
            RoomConnection::connect(
                &slug,
                &player_id,
                &player_name,
                room_snapshot,
                connection_status,
            )
        }
    });
    let _timer = use_future(move || async move {
        loop {
            TimeoutFuture::new(250).await;
            tick += 1;
        }
    });

    use_effect(move || {
        let puzzle = room_snapshot.read().puzzle.clone();
        let Some(puzzle) = puzzle else {
            race_game.set(None);
            return;
        };
        let needs_game = race_game
            .read()
            .as_ref()
            .map(|game| game.puzzle.id != puzzle.id)
            .unwrap_or(true);
        if needs_game {
            race_game.set(Some(GameState::new_room(puzzle)));
        }
    });

    use_effect({
        let connection = connection.clone();
        move || {
            let mut game = race_game.write();
            let Some(game) = game.as_mut() else {
                return;
            };
            if game.completed && !game.finish_notified {
                connection.send(&RoomClientMessage::Finish {
                    queens: game.queens(),
                });
                game.finish_notified = true;
            }
        }
    });

    let snapshot = room_snapshot.read().clone();
    let status = connection_status.read().clone();
    let _ = *tick.read();
    let me = snapshot
        .players
        .iter()
        .find(|player| player.id == player_id)
        .cloned();
    let my_ready = me.as_ref().map(|player| player.ready).unwrap_or(false);
    let my_finished = me.as_ref().and_then(|player| player.finish_ms).is_some();
    let ready_text = if my_ready { "Not Ready" } else { "Ready" };
    let room_url = current_url();
    let choice = puzzle_choice_label(&snapshot.puzzle_choice);
    let can_select = snapshot.phase.is_lobby();
    let countdown = countdown_label(&snapshot.phase);
    let race_started_at_ms = snapshot.phase.race_started_at_ms();
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
                                        let connection = connection.clone();
                                        move |_| connection.send(&RoomClientMessage::SelectRandom)
                                    },
                                    "Random Puzzle"
                                }
                                button {
                                    r#type: "button",
                                    class: "nav-button",
                                    onclick: {
                                        let connection = connection.clone();
                                        move |_| connection.send(&RoomClientMessage::SetReady { ready: !my_ready })
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
                                                let connection = connection.clone();
                                                move |_| connection.send(&RoomClientMessage::SelectPuzzle { puzzle_id })
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
                                        if let Some(game) = game_state.write().as_mut() {
                                            game.start_mark_drag(index);
                                        }
                                    }
                                },
                                onpointerenter: move |event| {
                                    let data = event.data();
                                    if data.pointer_type() == "mouse"
                                        && data.held_buttons().contains(MouseButton::Primary)
                                    {
                                        if let Some(game) = game_state.write().as_mut() {
                                            game.drag_mark_cell(index);
                                        }
                                    } else if let Some(game) = game_state.write().as_mut() {
                                        game.finish_mark_drag(None);
                                    }
                                },
                                onpointerup: move |event| {
                                    let data = event.data();
                                    if data.pointer_type() == "mouse" {
                                        if data.trigger_button() == Some(MouseButton::Primary) {
                                            event.prevent_default();
                                            if let Some(game) = game_state.write().as_mut() {
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

fn load_or_create_player_name(player_id: &str) -> String {
    const KEY: &str = "queensgame:player-name";
    if let Some(storage) = local_storage() {
        if let Ok(Some(name)) = storage.get_item(KEY) {
            if !name.trim().is_empty() {
                return name;
            }
        }
        let name = default_player_name(player_id);
        let _ = storage.set_item(KEY, &name);
        return name;
    }
    default_player_name(player_id)
}

fn generate_player_id() -> String {
    let random = (js_sys::Math::random() * u32::MAX as f64) as u32;
    format!("{:x}{random:08x}", now_ms() as u64)
}

fn default_player_name(player_id: &str) -> String {
    let suffix: String = player_id.chars().take(4).collect();
    format!("Player {}", suffix.to_uppercase())
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

fn current_url() -> String {
    web_sys::window()
        .and_then(|window| window.location().href().ok())
        .unwrap_or_else(|| String::from(""))
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
    let seconds = remaining_ms.div_ceil(1000).max(1);
    Some(format!("{seconds}"))
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
    if let Some(finish_ms) = player.finish_ms {
        return format_duration_ms(finish_ms);
    }
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
        RoomPhase::Racing { .. } | RoomPhase::Complete { .. } => {
            if let Some(started_at_ms) = started_at_ms {
                format!(
                    "In progress {}",
                    format_duration_ms((now_ms() as u64).saturating_sub(started_at_ms))
                )
            } else {
                "In progress".to_string()
            }
        }
    }
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

fn format_time(seconds: u64) -> String {
    let minutes = seconds / 60;
    let rest = seconds % 60;
    format!("{minutes:02}:{rest:02}")
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
