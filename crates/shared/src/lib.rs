use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct PuzzleFile {
    pub puzzles: Vec<Puzzle>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Puzzle {
    pub id: usize,
    pub size: usize,
    pub colors: Vec<String>,
    pub regions: Vec<Vec<usize>>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct PuzzleNav {
    pub id: usize,
    pub active: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct GameBootstrap {
    pub puzzle: Puzzle,
    pub puzzle_nav: Vec<PuzzleNav>,
    pub total: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CellView {
    pub row: usize,
    pub col: usize,
    pub region: usize,
    pub color: String,
    pub border_top: bool,
    pub border_right: bool,
    pub border_bottom: bool,
    pub border_left: bool,
}

impl CellView {
    pub fn class_name(&self) -> String {
        let mut class_name = String::from("cell");
        if self.border_top {
            class_name.push_str(" border-top");
        }
        if self.border_right {
            class_name.push_str(" border-right");
        }
        if self.border_bottom {
            class_name.push_str(" border-bottom");
        }
        if self.border_left {
            class_name.push_str(" border-left");
        }
        class_name
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellState {
    Empty,
    Mark,
    Queen,
    AutoMark,
}

impl CellState {
    pub fn is_marked(self) -> bool {
        matches!(self, Self::Mark | Self::AutoMark)
    }

    pub fn from_storage_code(value: u8) -> Self {
        match value {
            1 => Self::Mark,
            2 => Self::Queen,
            3 => Self::AutoMark,
            _ => Self::Empty,
        }
    }

    pub fn storage_code(self) -> u8 {
        match self {
            Self::Empty => 0,
            Self::Mark => 1,
            Self::Queen => 2,
            Self::AutoMark => 3,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ValidateRequest {
    pub id: usize,
    pub queens: Vec<[usize; 2]>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ValidateResponse {
    pub complete: bool,
    pub queen_count: usize,
    pub expected_queens: usize,
    pub satisfied_rows: usize,
    pub satisfied_columns: usize,
    pub satisfied_regions: usize,
    pub conflict_cells: Vec<[usize; 2]>,
    pub messages: Vec<String>,
}

pub fn build_cells(puzzle: &Puzzle) -> Vec<CellView> {
    let size = puzzle.size;
    let mut cells = Vec::with_capacity(size * size);

    for row in 0..size {
        for col in 0..size {
            let region = puzzle.regions[row][col];
            cells.push(CellView {
                row,
                col,
                region,
                color: puzzle.colors[region].clone(),
                border_top: row == 0 || puzzle.regions[row - 1][col] != region,
                border_right: col + 1 == size,
                border_bottom: row + 1 == size,
                border_left: col == 0 || puzzle.regions[row][col - 1] != region,
            });
        }
    }

    cells
}

pub fn invalidated_by_queen(
    puzzle: &Puzzle,
    queen_row: usize,
    queen_col: usize,
    row: usize,
    col: usize,
) -> bool {
    if queen_row == row && queen_col == col {
        return false;
    }

    queen_row == row
        || queen_col == col
        || puzzle.regions[queen_row][queen_col] == puzzle.regions[row][col]
        || (queen_row.abs_diff(row) == 1 && queen_col.abs_diff(col) == 1)
}

pub fn validate_solution(puzzle: &Puzzle, queens: &[[usize; 2]]) -> ValidateResponse {
    let size = puzzle.size;
    let mut row_counts = vec![0usize; size];
    let mut col_counts = vec![0usize; size];
    let mut region_counts = vec![0usize; size];
    let mut conflict_cells = BTreeSet::new();
    let mut messages = Vec::new();
    let mut valid_queens = Vec::new();

    for &[row, col] in queens {
        if row >= size || col >= size {
            messages.push(format!(
                "Ignored out-of-bounds queen at {},{}.",
                row + 1,
                col + 1
            ));
            continue;
        }

        valid_queens.push([row, col]);
        row_counts[row] += 1;
        col_counts[col] += 1;
        region_counts[puzzle.regions[row][col]] += 1;
    }

    for queen in &valid_queens {
        let [row, col] = *queen;
        let region = puzzle.regions[row][col];

        if row_counts[row] > 1 || col_counts[col] > 1 || region_counts[region] > 1 {
            conflict_cells.insert([row, col]);
        }
    }

    for i in 0..valid_queens.len() {
        for j in (i + 1)..valid_queens.len() {
            let [row_a, col_a] = valid_queens[i];
            let [row_b, col_b] = valid_queens[j];
            let row_delta = row_a.abs_diff(row_b);
            let col_delta = col_a.abs_diff(col_b);
            if row_delta == 1 && col_delta == 1 {
                conflict_cells.insert([row_a, col_a]);
                conflict_cells.insert([row_b, col_b]);
            }
        }
    }

    let satisfied_rows = row_counts.iter().filter(|&&count| count == 1).count();
    let satisfied_columns = col_counts.iter().filter(|&&count| count == 1).count();
    let satisfied_regions = region_counts.iter().filter(|&&count| count == 1).count();

    if valid_queens.len() != size {
        messages.push(format!("Place exactly {size} queens."));
    }
    if satisfied_rows != size {
        messages.push("Each row needs one queen.".to_string());
    }
    if satisfied_columns != size {
        messages.push("Each column needs one queen.".to_string());
    }
    if satisfied_regions != size {
        messages.push("Each colored region needs one queen.".to_string());
    }
    if !conflict_cells.is_empty() {
        messages
            .push("Queens cannot share a row, column, region, or touch diagonally.".to_string());
    }

    let complete = valid_queens.len() == size
        && satisfied_rows == size
        && satisfied_columns == size
        && satisfied_regions == size
        && conflict_cells.is_empty();

    ValidateResponse {
        complete,
        queen_count: valid_queens.len(),
        expected_queens: size,
        satisfied_rows,
        satisfied_columns,
        satisfied_regions,
        conflict_cells: conflict_cells.into_iter().collect(),
        messages,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_rejects_diagonal_touching() {
        let puzzle = Puzzle {
            id: 1,
            size: 2,
            colors: vec!["#000000".into(), "#FFFFFF".into()],
            regions: vec![vec![0, 0], vec![1, 1]],
        };

        let response = validate_solution(&puzzle, &[[0, 0], [1, 1]]);
        assert!(!response.complete);
        assert_eq!(response.conflict_cells.len(), 2);
    }

    #[test]
    fn queen_invalidates_rows_columns_regions_and_touching_diagonals() {
        let puzzle = Puzzle {
            id: 1,
            size: 4,
            colors: vec![
                "#000000".into(),
                "#FFFFFF".into(),
                "#FF0000".into(),
                "#00FF00".into(),
            ],
            regions: vec![
                vec![0, 1, 1, 3],
                vec![0, 1, 2, 3],
                vec![0, 2, 2, 3],
                vec![0, 0, 2, 3],
            ],
        };

        assert!(invalidated_by_queen(&puzzle, 1, 1, 1, 0));
        assert!(invalidated_by_queen(&puzzle, 1, 1, 0, 1));
        assert!(invalidated_by_queen(&puzzle, 1, 1, 0, 2));
        assert!(invalidated_by_queen(&puzzle, 1, 1, 0, 0));
        assert!(!invalidated_by_queen(&puzzle, 1, 1, 3, 3));
    }
}
