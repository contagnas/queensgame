use dioxus::{html::input_data::MouseButton, prelude::*};
use gloo_timers::future::TimeoutFuture;
use queensgame_shared::{
    build_cells, invalidated_by_queen, validate_solution, CellState, CellView, GameBootstrap,
    Puzzle, PuzzleNav, ValidateResponse,
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
    let bootstrap = use_hook(read_bootstrap);

    match bootstrap {
        Ok(bootstrap) => rsx! {
            Game { bootstrap }
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Queen,
    Mark,
    Clear,
}

#[derive(Clone)]
struct GameState {
    puzzle: Puzzle,
    cells: Vec<CellView>,
    puzzle_nav: Vec<PuzzleNav>,
    total: usize,
    mode: Mode,
    states: Vec<CellState>,
    history: Vec<Vec<CellState>>,
    started_at_ms: f64,
    completed: bool,
    completed_seconds: u64,
    validation: ValidateResponse,
    mark_drag: Option<MarkDrag>,
    win_visible: bool,
}

#[derive(Clone, Copy)]
struct MarkDrag {
    start_index: usize,
    moved: bool,
    changed: bool,
    history_started: bool,
    needs_auto_refresh: bool,
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

impl GameState {
    fn new(bootstrap: GameBootstrap) -> Self {
        let validation = validate_solution(&bootstrap.puzzle, &[]);
        let mut game = Self {
            cells: build_cells(&bootstrap.puzzle),
            states: vec![CellState::Empty; bootstrap.puzzle.size * bootstrap.puzzle.size],
            puzzle: bootstrap.puzzle,
            puzzle_nav: bootstrap.puzzle_nav,
            total: bootstrap.total,
            mode: Mode::Queen,
            history: Vec::new(),
            started_at_ms: now_ms(),
            completed: false,
            completed_seconds: 0,
            validation,
            mark_drag: None,
            win_visible: false,
        };

        game.load();
        game.refresh_auto_marks();
        game.revalidate(false);
        game
    }

    fn storage_key(&self) -> String {
        format!("queensgame:9x9:{}", self.puzzle.id)
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
        let Some(storage) = local_storage() else {
            return;
        };
        let Ok(Some(raw)) = storage.get_item(&self.storage_key()) else {
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
            let _ = storage.set_item(&self.storage_key(), &raw);
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
        self.mark_drag = Some(MarkDrag {
            start_index: index,
            moved: false,
            changed: false,
            history_started: false,
            needs_auto_refresh: false,
        });
    }

    fn set_drag_mark(&mut self, index: usize) {
        let Some(mut drag) = self.mark_drag else {
            return;
        };
        if self.states[index] == CellState::Mark {
            return;
        }

        if !drag.history_started {
            self.push_history();
            drag.history_started = true;
        }

        if self.states[index] == CellState::Queen {
            drag.needs_auto_refresh = true;
        }

        self.states[index] = CellState::Mark;
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
            self.set_drag_mark(start_index);
        }

        self.set_drag_mark(index);
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

fn read_bootstrap() -> Result<GameBootstrap, String> {
    let document = web_sys::window()
        .and_then(|window| window.document())
        .ok_or_else(|| "Browser document is unavailable.".to_string())?;
    let raw = document
        .get_element_by_id("game-data")
        .and_then(|element| element.text_content())
        .ok_or_else(|| "Puzzle data is missing from this page.".to_string())?;
    serde_json::from_str(&raw).map_err(|error| format!("Puzzle data is invalid: {error}"))
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
