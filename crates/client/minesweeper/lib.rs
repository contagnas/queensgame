use dioxus::{html::input_data::MouseButton, prelude::*};
use gloo_timers::future::TimeoutFuture;
use queensgame_client_components::{
    MinesweeperFaceState, MinesweeperLed, MinesweeperMousePress, format_minesweeper_counter,
    minesweeper_board_cell_display, minesweeper_cell_aria as shared_minesweeper_cell_aria,
    minesweeper_cell_class as shared_minesweeper_cell_class,
    minesweeper_cell_text as shared_minesweeper_cell_text, minesweeper_face_symbol,
};
use queensgame_shared_minesweeper::{
    MinesweeperBoard, MinesweeperBootstrap, MinesweeperCell, MinesweeperCellState,
    MinesweeperStatus,
};
use std::collections::BTreeSet;

#[derive(Clone, PartialEq)]
struct MinesweeperGameState {
    board: MinesweeperBoard,
    started_at_ms: Option<u64>,
    elapsed_ms: u64,
    face_down: bool,
}

impl MinesweeperGameState {
    fn new(bootstrap: &MinesweeperBootstrap) -> Self {
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
        let width = self.board.width();
        let height = self.board.height();
        let mines = self.board.mine_count();
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

    const fn set_face_down(&mut self, face_down: bool) {
        if matches!(
            self.board.status(),
            MinesweeperStatus::Ready | MinesweeperStatus::Playing
        ) {
            self.face_down = face_down;
        }
    }

    fn tick(&mut self) {
        if self.board.status() == MinesweeperStatus::Playing
            && let Some(started_at_ms) = self.started_at_ms
        {
            self.elapsed_ms = now_ms().saturating_sub(started_at_ms);
        }
    }

    fn timer_seconds(&self) -> u64 {
        (self.elapsed_ms / 1000).min(999)
    }
}

#[component]
pub fn MinesweeperApp(bootstrap: MinesweeperBootstrap) -> Element {
    let mut game = use_signal(|| MinesweeperGameState::new(&bootstrap));
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
    let timer =
        format_minesweeper_counter(i32::try_from(snapshot.timer_seconds()).unwrap_or(i32::MAX));
    let face = minesweeper_face(&snapshot);
    let board_width = snapshot.board.width();
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
                        style: "--mine-cols: {board_width}",
                        onpointerleave: move |_| {
                            game.write().set_face_down(false);
                            chord_target.set(None);
                            pressed_cells.set(BTreeSet::new());
                            left_mouse_down.set(false);
                            right_mouse_down.set(false);
                            suppress_next_secondary_up.set(false);
                        },
                        for (index, cell) in snapshot.board.cells().enumerate() {
                            {
                                let pressed = pressed_cell_set.contains(&index);
                                let class_name = minesweeper_cell_class(
                                    cell,
                                    snapshot.board.status(),
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
                                            let press = MinesweeperMousePress::new(
                                                primary,
                                                secondary,
                                                *left_mouse_down.read(),
                                                *right_mouse_down.read(),
                                            );
                                            if primary {
                                                event.prevent_default();
                                                left_mouse_down.set(true);
                                            }
                                            if secondary {
                                                event.prevent_default();
                                                right_mouse_down.set(true);
                                            }

                                            if press.should_toggle_flag() {
                                                chord_target.set(None);
                                                pressed_cells.set(BTreeSet::new());
                                                game.write().toggle_mark(index);
                                            }
                                            if press.should_press_cells() {
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

const fn minesweeper_face(snapshot: &MinesweeperGameState) -> &'static str {
    minesweeper_face_symbol(match snapshot.board.status() {
        MinesweeperStatus::Lost => MinesweeperFaceState::Lost,
        MinesweeperStatus::Won => MinesweeperFaceState::Won,
        MinesweeperStatus::Ready | MinesweeperStatus::Playing if snapshot.face_down => {
            MinesweeperFaceState::Pressed
        }
        MinesweeperStatus::Ready | MinesweeperStatus::Playing => MinesweeperFaceState::Ready,
    })
}

fn minesweeper_cell_class(
    cell: MinesweeperCell,
    status: MinesweeperStatus,
    pressed: bool,
) -> String {
    shared_minesweeper_cell_class(
        "ms-cell",
        minesweeper_board_cell_display(cell, status, pressed),
    )
}

fn minesweeper_cell_text(cell: MinesweeperCell, pressed: bool) -> String {
    shared_minesweeper_cell_text(minesweeper_board_cell_display(
        cell,
        MinesweeperStatus::Ready,
        pressed,
    ))
}

fn minesweeper_chord_target(board: &MinesweeperBoard, index: usize) -> Option<usize> {
    let cell = board.cell(index)?;
    (cell.state() == MinesweeperCellState::Revealed && cell.adjacent_mines() > 0).then_some(index)
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
                board.cell_state(*neighbor),
                Some(MinesweeperCellState::Hidden | MinesweeperCellState::Question)
            )
        })
        .collect()
}

fn minesweeper_cell_aria(index: usize, cell: MinesweeperCell, board: &MinesweeperBoard) -> String {
    let (row, col) = board.row_col(index).unwrap_or((0, 0));
    shared_minesweeper_cell_aria(
        row,
        col,
        minesweeper_board_cell_display(cell, board.status(), false),
    )
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn now_ms() -> u64 {
    js_sys::Date::now().max(0.0).floor() as u64
}

fn seed() -> u64 {
    let random = random_u64();
    (now_ms() << 21) ^ random ^ 0x9e37_79b9_7f4a_7c15
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn random_u64() -> u64 {
    (js_sys::Math::random() * f64::from(u32::MAX)).floor() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn minesweeper_pressed_neighbors_only_include_unopened_cells() {
        let hidden = 0;
        let flagged = 2;
        let revealed = 3;
        let center = 4;
        let mut board = MinesweeperBoard::from_mines(3, 3, BTreeSet::from([hidden, flagged]), 42);
        assert!(board.toggle_mark(flagged));
        assert_eq!(board.reveal_safe_cells(center), vec![center]);
        assert_eq!(board.reveal_safe_cells(revealed), vec![revealed]);

        let pressed = minesweeper_pressed_neighbors(&board, center);

        assert!(pressed.contains(&hidden));
        assert!(!pressed.contains(&flagged));
        assert!(!pressed.contains(&revealed));
    }
}
