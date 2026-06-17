use queensgame_shared_queens::{CellState, CellView, ValidateResponse};
use std::collections::BTreeSet;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Queen,
    Mark,
    Clear,
}

#[must_use]
pub fn validation_status(validation: &ValidateResponse, size: usize) -> String {
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

#[must_use]
pub fn mode_button_class(current: Mode, mode: Mode) -> &'static str {
    if current == mode {
        "mode-button active"
    } else {
        "mode-button"
    }
}

#[must_use]
pub fn cell_class(
    cell: &CellView,
    state: CellState,
    conflict_cells: &BTreeSet<[usize; 2]>,
) -> String {
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

#[must_use]
pub fn replay_cell_class(cell: &CellView, state: CellState) -> String {
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

#[must_use]
pub fn cell_aria(cell: &CellView, state: CellState) -> String {
    let marker = match state {
        CellState::Queen => ", queen",
        CellState::Mark | CellState::AutoMark => ", marked",
        CellState::Empty => "",
    };
    let row = cell.row + 1;
    let col = cell.col + 1;
    format!("Row {row}, column {col}{marker}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell() -> CellView {
        CellView {
            row: 1,
            col: 2,
            region: 0,
            color: "#ffffff".to_string(),
            border_top: false,
            border_right: false,
            border_bottom: false,
            border_left: false,
        }
    }

    #[test]
    fn cell_class_adds_state_and_conflict_markers() {
        let class_name = cell_class(&cell(), CellState::Queen, &BTreeSet::from([[1, 2]]));

        assert!(class_name.contains("cell"));
        assert!(class_name.contains("queen"));
        assert!(class_name.contains("conflict"));
    }

    #[test]
    fn cell_aria_describes_markers() {
        assert_eq!(
            cell_aria(&cell(), CellState::Queen),
            "Row 2, column 3, queen"
        );
        assert_eq!(
            cell_aria(&cell(), CellState::Mark),
            "Row 2, column 3, marked"
        );
        assert_eq!(cell_aria(&cell(), CellState::Empty), "Row 2, column 3");
    }

    #[test]
    fn validation_status_includes_first_message() {
        let status = validation_status(
            &ValidateResponse {
                complete: false,
                queen_count: 0,
                expected_queens: 9,
                satisfied_rows: 1,
                satisfied_columns: 2,
                satisfied_regions: 3,
                conflict_cells: Vec::new(),
                messages: vec!["Place exactly 9 queens.".to_string()],
            },
            9,
        );

        assert_eq!(
            status,
            "1/9 rows · 2/9 columns · 3/9 regions - Place exactly 9 queens."
        );
    }
}
