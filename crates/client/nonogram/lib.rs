use dioxus::{html::input_data::MouseButton, prelude::*};
use gloo_timers::future::TimeoutFuture;
use queensgame_shared_nonogram::{
    NonogramBootstrap, NonogramCellState, NonogramPuzzle, NonogramValidation,
    generate_nonogram_puzzle, validate_nonogram_solution,
};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NonogramPaintAction {
    Set(NonogramCellState),
    ClearMatching(NonogramCellState),
}

impl NonogramPaintAction {
    #[must_use]
    pub fn next_cell(self, current: NonogramCellState) -> Option<NonogramCellState> {
        match self {
            Self::Set(state) => {
                if current == state {
                    None
                } else {
                    Some(state)
                }
            }
            Self::ClearMatching(state) => {
                if current == state {
                    Some(NonogramCellState::Hidden)
                } else {
                    None
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NonogramInputShape {
    Free,
    Line,
    Rect,
}

#[derive(Clone, Default)]
pub struct NonogramDragState {
    action: Option<NonogramPaintAction>,
    start: Option<usize>,
    original_cells: Vec<NonogramCellState>,
    touched: BTreeSet<usize>,
    pointer_position: Option<(i32, i32)>,
    count: usize,
}

impl NonogramDragState {
    pub fn start(
        &mut self,
        index: usize,
        action: NonogramPaintAction,
        cells: &[NonogramCellState],
    ) {
        self.action = Some(action);
        self.start = Some(index);
        self.original_cells.clear();
        self.original_cells.extend_from_slice(cells);
        self.touched.clear();
        self.pointer_position = None;
        self.count = 0;
    }

    #[must_use]
    pub fn apply(
        &mut self,
        width: usize,
        height: usize,
        shape: NonogramInputShape,
        cells: &mut [NonogramCellState],
        index: usize,
    ) -> bool {
        if index >= cells.len() {
            return false;
        }
        let (Some(start), Some(action)) = (self.start, self.action) else {
            return false;
        };
        let indexes = nonogram_shape_indexes(width, height, start, index, shape);
        if indexes.is_empty() {
            return false;
        }
        match shape {
            NonogramInputShape::Free => self.apply_free(action, cells, &indexes),
            NonogramInputShape::Line | NonogramInputShape::Rect => {
                self.apply_shaped(action, cells, &indexes)
            }
        }
    }

    pub fn finish(&mut self) {
        self.action = None;
        self.start = None;
        self.original_cells.clear();
        self.touched.clear();
        self.pointer_position = None;
        self.count = 0;
    }

    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.action.is_some()
    }

    #[must_use]
    pub const fn count(&self) -> usize {
        self.count
    }

    pub fn set_pointer_position(&mut self, x: f64, y: f64) {
        if self.is_active() {
            self.pointer_position = Some((clamped_i32(x), clamped_i32(y)));
        }
    }

    #[must_use]
    pub const fn pointer_position(&self) -> Option<(i32, i32)> {
        self.pointer_position
    }

    fn apply_free(
        &mut self,
        action: NonogramPaintAction,
        cells: &mut [NonogramCellState],
        indexes: &[usize],
    ) -> bool {
        let mut changed = false;
        for &index in indexes {
            if index >= cells.len() {
                continue;
            }
            let original = self.original_cell(index, cells[index]);
            let next = action.next_cell(original).unwrap_or(original);
            if cells[index] != next {
                cells[index] = next;
                changed = true;
            }
            self.touched.insert(index);
        }
        self.count = self.touched.len();
        changed
    }

    fn apply_shaped(
        &mut self,
        action: NonogramPaintAction,
        cells: &mut [NonogramCellState],
        indexes: &[usize],
    ) -> bool {
        let active = indexes
            .iter()
            .copied()
            .filter(|index| *index < cells.len())
            .collect::<BTreeSet<_>>();
        let previous = std::mem::take(&mut self.touched);
        let mut changed = false;

        for &index in previous.difference(&active) {
            let original = self.original_cell(index, cells[index]);
            if cells[index] != original {
                cells[index] = original;
                changed = true;
            }
        }

        for &index in &active {
            let original = self.original_cell(index, cells[index]);
            let next = action.next_cell(original).unwrap_or(original);
            if cells[index] != next {
                cells[index] = next;
                changed = true;
            }
        }

        self.count = active.len();
        self.touched = active;
        changed
    }

    fn original_cell(&self, index: usize, fallback: NonogramCellState) -> NonogramCellState {
        self.original_cells.get(index).copied().unwrap_or(fallback)
    }
}

#[must_use]
pub const fn nonogram_primary_paint_action(
    selected: NonogramCellState,
    current: NonogramCellState,
) -> NonogramPaintAction {
    match (selected, current) {
        (NonogramCellState::Filled, NonogramCellState::Filled) => {
            NonogramPaintAction::ClearMatching(NonogramCellState::Filled)
        }
        (NonogramCellState::Crossed, NonogramCellState::Crossed) => {
            NonogramPaintAction::ClearMatching(NonogramCellState::Crossed)
        }
        _ => NonogramPaintAction::Set(selected),
    }
}

#[must_use]
pub const fn nonogram_fill_paint_action(current: NonogramCellState) -> NonogramPaintAction {
    nonogram_primary_paint_action(NonogramCellState::Filled, current)
}

#[must_use]
pub const fn nonogram_cross_paint_action(current: NonogramCellState) -> NonogramPaintAction {
    match current {
        NonogramCellState::Crossed => {
            NonogramPaintAction::ClearMatching(NonogramCellState::Crossed)
        }
        NonogramCellState::Hidden | NonogramCellState::Filled => {
            NonogramPaintAction::Set(NonogramCellState::Crossed)
        }
    }
}

#[must_use]
pub fn nonogram_shape_indexes(
    width: usize,
    height: usize,
    start: usize,
    end: usize,
    shape: NonogramInputShape,
) -> Vec<usize> {
    if width == 0 || height == 0 {
        return Vec::new();
    }
    let Some(cell_count) = width.checked_mul(height) else {
        return Vec::new();
    };
    if start >= cell_count || end >= cell_count {
        return Vec::new();
    }

    let start_row = start / width;
    let start_col = start % width;
    let end_row = end / width;
    let end_col = end % width;

    match shape {
        NonogramInputShape::Free => vec![end],
        NonogramInputShape::Line => {
            nonogram_line_indexes(width, start_row, start_col, end_row, end_col)
        }
        NonogramInputShape::Rect => {
            nonogram_rect_indexes(width, start_row, start_col, end_row, end_col)
        }
    }
}

fn nonogram_line_indexes(
    width: usize,
    start_row: usize,
    start_col: usize,
    end_row: usize,
    end_col: usize,
) -> Vec<usize> {
    let col_distance = start_col.abs_diff(end_col);
    let row_distance = start_row.abs_diff(end_row);
    let horizontal = start_row == end_row || (start_col != end_col && col_distance >= row_distance);
    if horizontal {
        let (first_col, last_col) = sorted_pair(start_col, end_col);
        (first_col..=last_col)
            .map(|col| start_row * width + col)
            .collect()
    } else {
        let (first_row, last_row) = sorted_pair(start_row, end_row);
        (first_row..=last_row)
            .map(|row| row * width + start_col)
            .collect()
    }
}

fn nonogram_rect_indexes(
    width: usize,
    start_row: usize,
    start_col: usize,
    end_row: usize,
    end_col: usize,
) -> Vec<usize> {
    let (first_row, last_row) = sorted_pair(start_row, end_row);
    let (first_col, last_col) = sorted_pair(start_col, end_col);
    let mut indexes = Vec::with_capacity((last_row - first_row + 1) * (last_col - first_col + 1));
    for row in first_row..=last_row {
        indexes.extend((first_col..=last_col).map(|col| row * width + col));
    }
    indexes
}

const fn sorted_pair(left: usize, right: usize) -> (usize, usize) {
    if left <= right {
        (left, right)
    } else {
        (right, left)
    }
}

#[must_use]
pub fn nonogram_drag_counter_label(
    drag: &NonogramDragState,
    width: usize,
    height: usize,
    shape: NonogramInputShape,
) -> Option<String> {
    if !drag.is_active() || drag.count() == 0 || drag.pointer_position().is_none() {
        return None;
    }
    let count = drag.count();
    let Some(total) = nonogram_extended_line_fill_count(drag, width, height, shape) else {
        return Some(count.to_string());
    };
    if total > count {
        Some(format!("{count} ({total})"))
    } else {
        Some(count.to_string())
    }
}

#[must_use]
pub fn nonogram_drag_counter_style(drag: &NonogramDragState) -> Option<String> {
    drag.pointer_position()
        .filter(|_| drag.is_active() && drag.count() > 0)
        .map(|(x, y)| format!("left: {x}px; top: {y}px"))
}

fn nonogram_extended_line_fill_count(
    drag: &NonogramDragState,
    width: usize,
    height: usize,
    shape: NonogramInputShape,
) -> Option<usize> {
    if shape != NonogramInputShape::Line
        || drag.action != Some(NonogramPaintAction::Set(NonogramCellState::Filled))
        || width == 0
        || height == 0
    {
        return None;
    }

    let first = drag.touched.iter().next().copied()?;
    let first_row = first / width;
    let first_col = first % width;
    let horizontal = drag.touched.iter().all(|index| index / width == first_row);
    let vertical = drag.touched.iter().all(|index| index % width == first_col);
    if horizontal {
        return Some(nonogram_extended_horizontal_fill_count(
            drag, width, first_row,
        ));
    }
    if vertical {
        return Some(nonogram_extended_vertical_fill_count(
            drag, width, height, first_col,
        ));
    }
    None
}

fn nonogram_extended_horizontal_fill_count(
    drag: &NonogramDragState,
    width: usize,
    row: usize,
) -> usize {
    let Some((first_col, last_col)) = drag
        .touched
        .iter()
        .map(|index| index % width)
        .min()
        .zip(drag.touched.iter().map(|index| index % width).max())
    else {
        return drag.count();
    };
    let mut total = drag.count();
    for col in (0..first_col).rev() {
        let index = row * width + col;
        if drag.original_cell(index, NonogramCellState::Hidden) != NonogramCellState::Filled {
            break;
        }
        total += 1;
    }
    for col in (last_col + 1)..width {
        let index = row * width + col;
        if drag.original_cell(index, NonogramCellState::Hidden) != NonogramCellState::Filled {
            break;
        }
        total += 1;
    }
    total
}

fn nonogram_extended_vertical_fill_count(
    drag: &NonogramDragState,
    width: usize,
    height: usize,
    col: usize,
) -> usize {
    let Some((first_row, last_row)) = drag
        .touched
        .iter()
        .map(|index| index / width)
        .min()
        .zip(drag.touched.iter().map(|index| index / width).max())
    else {
        return drag.count();
    };
    let mut total = drag.count();
    for row in (0..first_row).rev() {
        let index = row * width + col;
        if drag.original_cell(index, NonogramCellState::Hidden) != NonogramCellState::Filled {
            break;
        }
        total += 1;
    }
    for row in (last_row + 1)..height {
        let index = row * width + col;
        if drag.original_cell(index, NonogramCellState::Hidden) != NonogramCellState::Filled {
            break;
        }
        total += 1;
    }
    total
}

#[must_use]
pub fn nonogram_marked_error_cells(
    validation: &NonogramValidation,
    cells: &[NonogramCellState],
) -> BTreeSet<usize> {
    validation
        .incorrect_cells
        .iter()
        .copied()
        .filter(|index| {
            cells
                .get(*index)
                .is_some_and(|state| *state != NonogramCellState::Hidden)
        })
        .collect()
}

#[must_use]
pub fn nonogram_shape_button_class(
    active: NonogramInputShape,
    shape: NonogramInputShape,
) -> &'static str {
    if active == shape {
        "mode-button active"
    } else {
        "mode-button"
    }
}

#[allow(clippy::cast_possible_truncation)]
fn clamped_i32(value: f64) -> i32 {
    value
        .round()
        .clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
}

#[derive(Clone)]
struct NonogramGameState {
    puzzle: NonogramPuzzle,
    cells: Vec<NonogramCellState>,
    shape: NonogramInputShape,
    drag: NonogramDragState,
    drag_checkpoint: Option<Vec<NonogramCellState>>,
    undo_stack: Vec<Vec<NonogramCellState>>,
    redo_stack: Vec<Vec<NonogramCellState>>,
    started_at_ms: u64,
    elapsed_ms: u64,
    completed: bool,
    validation: NonogramValidation,
}

impl NonogramGameState {
    fn new(bootstrap: &NonogramBootstrap) -> Self {
        let puzzle = generate_nonogram_puzzle(bootstrap.width, bootstrap.height, seed());
        let cells = vec![NonogramCellState::Hidden; puzzle.cell_count()];
        let validation = validate_nonogram_solution(&puzzle, &cells);
        Self {
            puzzle,
            cells,
            shape: NonogramInputShape::Free,
            drag: NonogramDragState::default(),
            drag_checkpoint: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            started_at_ms: now_ms(),
            elapsed_ms: 0,
            completed: false,
            validation,
        }
    }

    fn reset(&mut self, bootstrap: &NonogramBootstrap) {
        *self = Self::new(bootstrap);
    }

    fn apply_fill(&mut self, index: usize) {
        let action = self.primary_action(index);
        self.apply_paint_action(index, action);
    }

    fn start_primary_drag(&mut self, index: usize) {
        if self.completed || index >= self.cells.len() {
            return;
        }
        let action = self.primary_action(index);
        self.drag_checkpoint = Some(self.cells.clone());
        self.drag.start(index, action, &self.cells);
        self.drag_cell(index);
    }

    fn start_cross_drag(&mut self, index: usize) {
        if self.completed || index >= self.cells.len() {
            return;
        }
        let action = self.cross_action(index);
        self.drag_checkpoint = Some(self.cells.clone());
        self.drag.start(index, action, &self.cells);
        self.drag_cell(index);
    }

    fn drag_cell(&mut self, index: usize) {
        if self.completed {
            return;
        }
        if self.drag.apply(
            self.puzzle.width,
            self.puzzle.height,
            self.shape,
            &mut self.cells,
            index,
        ) {
            self.after_cell_edit();
        }
    }

    fn finish_drag(&mut self) {
        let was_active = self.drag.is_active();
        let checkpoint = self.drag_checkpoint.take();
        self.drag.finish();
        if let Some(previous_cells) = checkpoint {
            self.commit_history(previous_cells);
        }
        if was_active {
            self.mark_complete_if_ready();
        }
    }

    fn apply_cross(&mut self, index: usize) {
        let action = self.cross_action(index);
        self.apply_paint_action(index, action);
    }

    fn primary_action(&self, index: usize) -> NonogramPaintAction {
        let current = self
            .cells
            .get(index)
            .copied()
            .unwrap_or(NonogramCellState::Hidden);
        nonogram_fill_paint_action(current)
    }

    fn cross_action(&self, index: usize) -> NonogramPaintAction {
        let current = self
            .cells
            .get(index)
            .copied()
            .unwrap_or(NonogramCellState::Hidden);
        nonogram_cross_paint_action(current)
    }

    fn apply_paint_action(&mut self, index: usize, action: NonogramPaintAction) {
        let Some(current) = self.cells.get(index).copied() else {
            return;
        };
        if let Some(next) = action.next_cell(current) {
            self.set_cell(index, next);
        }
    }

    fn set_cell(&mut self, index: usize, state: NonogramCellState) {
        if self.completed || index >= self.cells.len() {
            return;
        }
        let previous_cells = self.cells.clone();
        if self.set_cell_without_history(index, state) {
            self.commit_history(previous_cells);
            self.after_cell_edit();
        }
    }

    fn set_cell_without_history(&mut self, index: usize, state: NonogramCellState) -> bool {
        if index >= self.cells.len() {
            return false;
        }
        if self.cells[index] == state {
            return false;
        }
        self.cells[index] = state;
        true
    }

    fn after_cell_edit(&mut self) {
        self.tick();
        self.validation = validate_nonogram_solution(&self.puzzle, &self.cells);
        if !self.drag.is_active() {
            self.mark_complete_if_ready();
        }
    }

    const fn set_shape(&mut self, shape: NonogramInputShape) {
        self.shape = shape;
    }

    fn update_drag_pointer(&mut self, x: f64, y: f64) {
        self.drag.set_pointer_position(x, y);
    }

    fn mark_complete_if_ready(&mut self) {
        if self.validation.complete {
            self.completed = true;
            self.elapsed_ms = now_ms().saturating_sub(self.started_at_ms);
        }
    }

    fn tick(&mut self) {
        if !self.completed {
            self.elapsed_ms = now_ms().saturating_sub(self.started_at_ms);
        }
    }

    fn undo(&mut self) {
        let Some(previous_cells) = self.undo_stack.pop() else {
            return;
        };
        self.redo_stack.push(self.cells.clone());
        self.restore_cells(previous_cells);
    }

    fn redo(&mut self) {
        let Some(next_cells) = self.redo_stack.pop() else {
            return;
        };
        self.undo_stack.push(self.cells.clone());
        self.restore_cells(next_cells);
    }

    fn restore_cells(&mut self, cells: Vec<NonogramCellState>) {
        self.drag.finish();
        self.drag_checkpoint = None;
        self.cells = cells;
        self.validation = validate_nonogram_solution(&self.puzzle, &self.cells);
        self.completed = false;
        self.tick();
        self.mark_complete_if_ready();
    }

    fn commit_history(&mut self, previous_cells: Vec<NonogramCellState>) {
        if previous_cells != self.cells {
            self.undo_stack.push(previous_cells);
            self.redo_stack.clear();
        }
    }

    const fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    const fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }
}

#[component]
pub fn NonogramApp(bootstrap: NonogramBootstrap) -> Element {
    let mut game = use_signal(|| NonogramGameState::new(&bootstrap));
    let mut show_errors = use_signal(|| false);
    let mut show_counter = use_signal(|| true);
    let _timer = use_future(move || async move {
        loop {
            TimeoutFuture::new(100).await;
            game.write().tick();
        }
    });

    let snapshot = game.read().clone();
    let incorrect_cells = if *show_errors.read() {
        nonogram_marked_error_cells(&snapshot.validation, &snapshot.cells)
    } else {
        BTreeSet::new()
    };
    let timer = format_duration_ms(snapshot.elapsed_ms);
    let status = nonogram_status(&snapshot);
    let counter_enabled = *show_counter.read();
    let counter_class = if counter_enabled {
        "mode-button nonogram-counter-checkbox active"
    } else {
        "mode-button nonogram-counter-checkbox"
    };
    let (drag_counter, drag_counter_style) = if counter_enabled {
        (
            nonogram_drag_counter_label(
                &snapshot.drag,
                snapshot.puzzle.width,
                snapshot.puzzle.height,
                snapshot.shape,
            ),
            nonogram_drag_counter_style(&snapshot.drag),
        )
    } else {
        (None, None)
    };
    rsx! {
        main { class: "nonogram-page",
            section { class: "game-shell nonogram-shell", aria_labelledby: "nonogram-title",
                div { class: "game-toolbar",
                    div {
                        p { class: "eyebrow", "Generated puzzle" }
                        h1 { id: "nonogram-title", "Nonogram" }
                    }
                    div { class: "timer-box", aria_live: "polite",
                        span { class: "timer-label", "Time" }
                        span { id: "timer", "{timer}" }
                    }
                }

                div { class: "controls-row", aria_label: "Nonogram controls",
                    div { class: "segmented", role: "group", aria_label: "History",
                        button {
                            r#type: "button",
                            class: "mode-button",
                            title: "Undo",
                            disabled: !snapshot.can_undo(),
                            onclick: move |_| game.write().undo(),
                            "Undo"
                        }
                        button {
                            r#type: "button",
                            class: "mode-button",
                            title: "Redo",
                            disabled: !snapshot.can_redo(),
                            onclick: move |_| game.write().redo(),
                            "Redo"
                        }
                    }
                    div { class: "segmented", role: "group", aria_label: "Drag shape",
                        button {
                            r#type: "button",
                            class: nonogram_shape_button_class(snapshot.shape, NonogramInputShape::Free),
                            onclick: move |_| game.write().set_shape(NonogramInputShape::Free),
                            "Free"
                        }
                        button {
                            r#type: "button",
                            class: nonogram_shape_button_class(snapshot.shape, NonogramInputShape::Line),
                            onclick: move |_| game.write().set_shape(NonogramInputShape::Line),
                            "Line"
                        }
                        button {
                            r#type: "button",
                            class: nonogram_shape_button_class(snapshot.shape, NonogramInputShape::Rect),
                            onclick: move |_| game.write().set_shape(NonogramInputShape::Rect),
                            "Rect"
                        }
                        label { class: counter_class,
                            input {
                                r#type: "checkbox",
                                checked: "{counter_enabled}",
                                aria_label: "Show drag count",
                                onclick: move |_| {
                                    let enabled = *show_counter.read();
                                    show_counter.set(!enabled);
                                }
                            }
                            span { "Count" }
                        }
                    }
                    div { class: "tool-buttons",
                        button {
                            r#type: "button",
                            class: "tool-button",
                            title: "Highlight mistakes",
                            onclick: move |_| show_errors.set(true),
                            "Check"
                        }
                        button {
                            r#type: "button",
                            class: "tool-button",
                            title: "Generate a new puzzle",
                            onclick: move |_| {
                                show_errors.set(false);
                                game.write().reset(&bootstrap);
                            },
                            "New Puzzle"
                        }
                    }
                }

                div {
                    class: "nonogram-layout",
                    style: "--nonogram-cols: {snapshot.puzzle.width}; --nonogram-rows: {snapshot.puzzle.height}",
                    div { class: "nonogram-corner", aria_hidden: "true" }
                    div { class: "nonogram-col-clues", aria_hidden: "true",
                        for (col, clues) in snapshot.puzzle.col_clues.iter().enumerate() {
                            div {
                                class: "nonogram-clue nonogram-col-clue",
                                style: "grid-column: {col + 2}; grid-row: 1",
                                for clue in display_clues(clues) {
                                    span { "{clue}" }
                                }
                            }
                        }
                    }
                    div { class: "nonogram-row-clues", aria_hidden: "true",
                        for (row, clues) in snapshot.puzzle.row_clues.iter().enumerate() {
                            div {
                                class: "nonogram-clue nonogram-row-clue",
                                style: "grid-column: 1; grid-row: {row + 2}",
                                for clue in display_clues(clues) {
                                    span { "{clue}" }
                                }
                            }
                        }
                    }
                    div {
                        class: "nonogram-board",
                        role: "grid",
                        aria_label: "Nonogram board",
                        for (index, state) in snapshot.cells.iter().enumerate() {
                            {
                                let row = index / snapshot.puzzle.width;
                                let col = index % snapshot.puzzle.width;
                                let class_name = nonogram_cell_class_at(
                                    *state,
                                    incorrect_cells.contains(&index),
                                    row,
                                    col,
                                    snapshot.puzzle.width,
                                    snapshot.puzzle.height,
                                );
                                let aria = nonogram_cell_aria(row, col, *state);
                                rsx! {
                                    button {
                                        r#type: "button",
                                        class: "{class_name}",
                                        style: "grid-column: {col + 2}; grid-row: {row + 2}",
                                        role: "gridcell",
                                        aria_label: "{aria}",
                                        onpointerdown: move |event| {
                                            let data = event.data();
                                            if data.pointer_type() == "mouse" {
                                                let coordinates = data.client_coordinates();
                                                if data.trigger_button() == Some(MouseButton::Primary) {
                                                    event.prevent_default();
                                                    let mut game = game.write();
                                                    game.start_primary_drag(index);
                                                    game.update_drag_pointer(coordinates.x, coordinates.y);
                                                } else if data.trigger_button() == Some(MouseButton::Secondary) {
                                                    event.prevent_default();
                                                    let mut game = game.write();
                                                    game.start_cross_drag(index);
                                                    game.update_drag_pointer(coordinates.x, coordinates.y);
                                                }
                                            }
                                        },
                                        onpointermove: move |event| {
                                            let data = event.data();
                                            if data.pointer_type() != "mouse" {
                                                return;
                                            }
                                            let dragging = data.held_buttons().contains(MouseButton::Primary)
                                                || data.held_buttons().contains(MouseButton::Secondary);
                                            if dragging {
                                                let coordinates = data.client_coordinates();
                                                game.write().update_drag_pointer(coordinates.x, coordinates.y);
                                            }
                                        },
                                        onpointerenter: move |event| {
                                            let data = event.data();
                                            if data.pointer_type() != "mouse" {
                                                return;
                                            }
                                            let dragging = data.held_buttons().contains(MouseButton::Primary)
                                                || data.held_buttons().contains(MouseButton::Secondary);
                                            if dragging {
                                                event.prevent_default();
                                                let coordinates = data.client_coordinates();
                                                let mut game = game.write();
                                                game.drag_cell(index);
                                                game.update_drag_pointer(coordinates.x, coordinates.y);
                                            } else {
                                                game.write().finish_drag();
                                            }
                                        },
                                        onpointerup: move |event| {
                                            if event.data().pointer_type() == "mouse" {
                                                game.write().finish_drag();
                                            } else {
                                                game.write().apply_fill(index);
                                            }
                                        },
                                        oncontextmenu: move |event| {
                                            event.prevent_default();
                                        },
                                        onkeydown: move |event| {
                                            let code = event.data().code();
                                            match code {
                                                Code::Space | Code::Enter => {
                                                    event.prevent_default();
                                                    game.write().apply_fill(index);
                                                }
                                                Code::KeyX => {
                                                    event.prevent_default();
                                                    game.write().apply_cross(index);
                                                }
                                                Code::Backspace | Code::Delete => {
                                                    event.prevent_default();
                                                    game.write().set_cell(index, NonogramCellState::Hidden);
                                                }
                                                _ => {}
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                if let (Some(counter), Some(style)) = (drag_counter, drag_counter_style) {
                    div {
                        class: "nonogram-drag-counter",
                        style: "{style}",
                        aria_hidden: "true",
                        "{counter}"
                    }
                }

                div { class: "status-strip", aria_live: "polite",
                    span { "{snapshot.validation.filled_count} / {snapshot.validation.expected_filled} filled" }
                    span { "{status}" }
                }
                div { class: "rule-panel",
                    h2 { "Rules" }
                    p { "Use the row and column clues to fill each run of squares. Cross out cells that cannot be filled." }
                }
            }
        }
    }
}

#[must_use]
pub fn display_clues(clues: &[usize]) -> Vec<String> {
    if clues.is_empty() {
        vec!["0".to_string()]
    } else {
        clues.iter().map(ToString::to_string).collect()
    }
}

fn nonogram_status(game: &NonogramGameState) -> String {
    if game.completed {
        format!("Solved in {}.", format_duration_ms(game.elapsed_ms))
    } else {
        "Find every filled square.".to_string()
    }
}

#[must_use]
pub fn nonogram_cell_class(state: NonogramCellState, incorrect: bool) -> String {
    nonogram_cell_class_at(state, incorrect, 0, 0, 1, 1)
}

#[must_use]
pub fn nonogram_cell_class_at(
    state: NonogramCellState,
    incorrect: bool,
    row: usize,
    col: usize,
    width: usize,
    height: usize,
) -> String {
    let mut class_name = String::from("nonogram-cell");
    match state {
        NonogramCellState::Hidden => {}
        NonogramCellState::Filled => class_name.push_str(" filled"),
        NonogramCellState::Crossed => class_name.push_str(" crossed"),
    }
    if (col + 1).is_multiple_of(5) && col + 1 < width {
        class_name.push_str(" major-right");
    }
    if (row + 1).is_multiple_of(5) && row + 1 < height {
        class_name.push_str(" major-bottom");
    }
    if incorrect {
        class_name.push_str(" conflict");
    }
    class_name
}

#[must_use]
pub fn nonogram_cell_aria(row: usize, col: usize, state: NonogramCellState) -> String {
    let state = match state {
        NonogramCellState::Hidden => "empty",
        NonogramCellState::Filled => "filled",
        NonogramCellState::Crossed => "crossed",
    };
    format!("Row {}, column {}, {state}", row + 1, col + 1)
}

fn format_duration_ms(milliseconds: u64) -> String {
    let total_seconds = milliseconds / 1000;
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    let tenths = (milliseconds % 1000) / 100;
    format!("{minutes:02}:{seconds:02}.{tenths}")
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn now_ms() -> u64 {
    js_sys::Date::now().max(0.0).floor() as u64
}

fn seed() -> u64 {
    let random = random_u64();
    (now_ms() << 21) ^ random ^ 0xbf58_476d_1ce4_e5b9
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn random_u64() -> u64 {
    (js_sys::Math::random() * f64::from(u32::MAX)).floor() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonogram_cell_classes_include_state_and_conflict() {
        assert_eq!(
            nonogram_cell_class(NonogramCellState::Filled, true),
            "nonogram-cell filled conflict"
        );
        assert_eq!(
            nonogram_cell_class(NonogramCellState::Crossed, false),
            "nonogram-cell crossed"
        );
    }

    #[test]
    fn nonogram_cell_classes_include_five_cell_guides() {
        assert_eq!(
            nonogram_cell_class_at(NonogramCellState::Hidden, false, 4, 4, 10, 10),
            "nonogram-cell major-right major-bottom"
        );
        assert_eq!(
            nonogram_cell_class_at(NonogramCellState::Hidden, false, 9, 9, 10, 10),
            "nonogram-cell"
        );
    }

    #[test]
    fn nonogram_primary_paint_action_toggles_matching_marks() {
        let remove_fill =
            nonogram_primary_paint_action(NonogramCellState::Filled, NonogramCellState::Filled);
        assert_eq!(
            remove_fill.next_cell(NonogramCellState::Filled),
            Some(NonogramCellState::Hidden)
        );
        assert_eq!(remove_fill.next_cell(NonogramCellState::Crossed), None);

        let remove_cross =
            nonogram_primary_paint_action(NonogramCellState::Crossed, NonogramCellState::Crossed);
        assert_eq!(
            remove_cross.next_cell(NonogramCellState::Crossed),
            Some(NonogramCellState::Hidden)
        );
        assert_eq!(remove_cross.next_cell(NonogramCellState::Filled), None);
    }

    #[test]
    fn nonogram_cross_paint_action_toggles_crosses() {
        let add_cross = nonogram_cross_paint_action(NonogramCellState::Filled);
        assert_eq!(
            add_cross.next_cell(NonogramCellState::Filled),
            Some(NonogramCellState::Crossed)
        );

        let remove_cross = nonogram_cross_paint_action(NonogramCellState::Crossed);
        assert_eq!(
            remove_cross.next_cell(NonogramCellState::Crossed),
            Some(NonogramCellState::Hidden)
        );
        assert_eq!(remove_cross.next_cell(NonogramCellState::Filled), None);
    }

    #[test]
    fn nonogram_shape_indexes_build_lines() {
        assert_eq!(
            nonogram_shape_indexes(10, 10, 23, 26, NonogramInputShape::Line),
            vec![23, 24, 25, 26]
        );
        assert_eq!(
            nonogram_shape_indexes(10, 10, 23, 53, NonogramInputShape::Line),
            vec![23, 33, 43, 53]
        );
        assert_eq!(
            nonogram_shape_indexes(10, 10, 23, 46, NonogramInputShape::Line),
            vec![23, 24, 25, 26]
        );
    }

    #[test]
    fn nonogram_shape_indexes_build_rectangles() {
        assert_eq!(
            nonogram_shape_indexes(10, 10, 12, 34, NonogramInputShape::Rect),
            vec![12, 13, 14, 22, 23, 24, 32, 33, 34]
        );
    }

    #[test]
    fn nonogram_free_drag_keeps_touched_cells() {
        let mut cells = vec![NonogramCellState::Hidden; 9];
        let mut drag = NonogramDragState::default();
        drag.start(
            0,
            NonogramPaintAction::Set(NonogramCellState::Filled),
            &cells,
        );

        assert!(drag.apply(3, 3, NonogramInputShape::Free, &mut cells, 0));
        assert!(drag.apply(3, 3, NonogramInputShape::Free, &mut cells, 1));

        assert_eq!(drag.count(), 2);
        assert_eq!(cells[0], NonogramCellState::Filled);
        assert_eq!(cells[1], NonogramCellState::Filled);
    }

    #[test]
    fn nonogram_rect_drag_restores_cells_outside_current_rect() {
        let mut cells = vec![NonogramCellState::Hidden; 9];
        let mut drag = NonogramDragState::default();
        drag.start(
            0,
            NonogramPaintAction::Set(NonogramCellState::Filled),
            &cells,
        );

        assert!(drag.apply(3, 3, NonogramInputShape::Rect, &mut cells, 4));
        assert!(drag.apply(3, 3, NonogramInputShape::Rect, &mut cells, 1));

        assert_eq!(drag.count(), 2);
        assert_eq!(cells[0], NonogramCellState::Filled);
        assert_eq!(cells[1], NonogramCellState::Filled);
        assert_eq!(cells[3], NonogramCellState::Hidden);
        assert_eq!(cells[4], NonogramCellState::Hidden);
    }

    #[test]
    fn nonogram_line_counter_includes_attached_filled_segment() {
        let mut cells = vec![
            NonogramCellState::Filled,
            NonogramCellState::Hidden,
            NonogramCellState::Hidden,
            NonogramCellState::Hidden,
        ];
        let mut drag = NonogramDragState::default();
        drag.start(
            1,
            NonogramPaintAction::Set(NonogramCellState::Filled),
            &cells,
        );

        assert!(drag.apply(4, 1, NonogramInputShape::Line, &mut cells, 2));
        drag.set_pointer_position(100.0, 100.0);

        assert_eq!(
            nonogram_drag_counter_label(&drag, 4, 1, NonogramInputShape::Line),
            Some("2 (3)".to_string())
        );
    }

    #[test]
    fn nonogram_line_counter_uses_stroke_count_without_attachment() {
        let mut cells = vec![NonogramCellState::Hidden; 4];
        let mut drag = NonogramDragState::default();
        drag.start(
            1,
            NonogramPaintAction::Set(NonogramCellState::Filled),
            &cells,
        );

        assert!(drag.apply(4, 1, NonogramInputShape::Line, &mut cells, 2));
        drag.set_pointer_position(100.0, 100.0);

        assert_eq!(
            nonogram_drag_counter_label(&drag, 4, 1, NonogramInputShape::Line),
            Some("2".to_string())
        );
    }

    #[test]
    fn nonogram_marked_error_cells_ignores_hidden_misses() {
        let puzzle = NonogramPuzzle {
            width: 3,
            height: 1,
            seed: 1,
            row_clues: vec![vec![1, 1]],
            col_clues: vec![vec![1], vec![], vec![1]],
            solution: vec![true, false, true],
        };
        let cells = vec![
            NonogramCellState::Hidden,
            NonogramCellState::Filled,
            NonogramCellState::Crossed,
        ];
        let validation = validate_nonogram_solution(&puzzle, &cells);

        assert_eq!(
            nonogram_marked_error_cells(&validation, &cells),
            BTreeSet::from([1, 2])
        );
    }
}
