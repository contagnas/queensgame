use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, VecDeque};

mod solver;

pub const MINESWEEPER_EXPERT_WIDTH: usize = 30;
pub const MINESWEEPER_EXPERT_HEIGHT: usize = 16;
pub const MINESWEEPER_EXPERT_MINES: usize = 99;
pub const ROOM_MINESWEEPER_DEFAULT_SECONDS: u32 = 60;
pub const ROOM_MINESWEEPER_MIN_SECONDS: u32 = 30;
pub const ROOM_MINESWEEPER_MAX_SECONDS: u32 = 3_600;
pub const ROOM_MINESWEEPER_DEFAULT_TILE_ROWS: usize = 1;
pub const ROOM_MINESWEEPER_DEFAULT_TILE_COLS: usize = 1;
pub const ROOM_MINESWEEPER_MIN_TILE_AXIS: usize = 1;
pub const ROOM_MINESWEEPER_MAX_TILE_AXIS: usize = 3;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct MinesweeperBootstrap {
    pub width: usize,
    pub height: usize,
    pub mines: usize,
}

impl Default for MinesweeperBootstrap {
    fn default() -> Self {
        Self {
            width: MINESWEEPER_EXPERT_WIDTH,
            height: MINESWEEPER_EXPERT_HEIGHT,
            mines: MINESWEEPER_EXPERT_MINES,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct MinesweeperBoard {
    pub width: usize,
    pub height: usize,
    pub mines: usize,
    pub cells: Vec<MinesweeperCell>,
    pub status: MinesweeperStatus,
    seed: u64,
    mines_placed: bool,
    #[serde(default)]
    no_guess: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct RoomMinesweeperBoard {
    pub board: MinesweeperBoard,
    pub starting_cells: Vec<usize>,
    pub tile_rows: usize,
    pub tile_cols: usize,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub struct MinesweeperCell {
    pub mine: bool,
    pub adjacent_mines: u8,
    pub state: MinesweeperCellState,
    pub detonated: bool,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub enum MinesweeperCellState {
    Hidden,
    Flagged,
    Question,
    Revealed,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub enum MinesweeperStatus {
    Ready,
    Playing,
    Won,
    Lost,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MinesweeperActionResult {
    pub changed: bool,
    pub started: bool,
}

impl MinesweeperBoard {
    #[must_use]
    pub fn new_expert(seed: u64) -> Self {
        Self::new(
            MINESWEEPER_EXPERT_WIDTH,
            MINESWEEPER_EXPERT_HEIGHT,
            MINESWEEPER_EXPERT_MINES,
            seed,
        )
    }

    #[must_use]
    pub fn new_no_guess_expert(seed: u64) -> Self {
        Self::new_no_guess(
            MINESWEEPER_EXPERT_WIDTH,
            MINESWEEPER_EXPERT_HEIGHT,
            MINESWEEPER_EXPERT_MINES,
            seed,
        )
    }

    #[must_use]
    pub fn new(width: usize, height: usize, mines: usize, seed: u64) -> Self {
        let cell_count = width.saturating_mul(height);
        let mines = mines.min(cell_count.saturating_sub(1));
        Self {
            width,
            height,
            mines,
            cells: vec![MinesweeperCell::default(); cell_count],
            status: MinesweeperStatus::Ready,
            seed: seed.max(1),
            mines_placed: false,
            no_guess: false,
        }
    }

    #[must_use]
    pub fn new_no_guess(width: usize, height: usize, mines: usize, seed: u64) -> Self {
        Self {
            no_guess: true,
            ..Self::new(width, height, mines, seed)
        }
    }

    #[must_use]
    pub fn from_mines(width: usize, height: usize, mines: BTreeSet<usize>, seed: u64) -> Self {
        let mut board = Self::new(width, height, mines.len(), seed);
        for index in mines {
            if let Some(cell) = board.cells.get_mut(index) {
                cell.mine = true;
            }
        }
        board.recalculate_adjacent_counts();
        board.mines_placed = true;
        board
    }

    #[must_use]
    pub fn index(&self, row: usize, col: usize) -> Option<usize> {
        (row < self.height && col < self.width).then_some(row * self.width + col)
    }

    #[must_use]
    pub fn row_col(&self, index: usize) -> Option<(usize, usize)> {
        (index < self.cells.len()).then_some((index / self.width, index % self.width))
    }

    #[must_use]
    pub fn flagged_count(&self) -> usize {
        self.cells
            .iter()
            .filter(|cell| cell.state == MinesweeperCellState::Flagged)
            .count()
    }

    #[must_use]
    pub fn remaining_mines(&self) -> i32 {
        let mines = i32::try_from(self.mines).unwrap_or(i32::MAX);
        let flagged = i32::try_from(self.flagged_count()).unwrap_or(i32::MAX);
        mines.saturating_sub(flagged)
    }

    #[must_use]
    pub fn reveal(&mut self, index: usize) -> MinesweeperActionResult {
        if !self.accepts_action() || index >= self.cells.len() {
            return MinesweeperActionResult::default();
        }
        if !self.mines_placed {
            self.place_mines(index);
        }
        if matches!(self.cells[index].state, MinesweeperCellState::Flagged) {
            return MinesweeperActionResult {
                changed: false,
                started: self.status == MinesweeperStatus::Playing,
            };
        }

        let started = self.status == MinesweeperStatus::Ready;
        self.status = MinesweeperStatus::Playing;

        if self.cells[index].mine {
            self.cells[index].state = MinesweeperCellState::Revealed;
            self.cells[index].detonated = true;
            self.lose();
            return MinesweeperActionResult {
                changed: true,
                started,
            };
        }

        let changed = self.reveal_safe_area(index);
        self.update_win_status();
        MinesweeperActionResult { changed, started }
    }

    #[must_use]
    pub fn toggle_mark(&mut self, index: usize) -> bool {
        if !self.accepts_action() || index >= self.cells.len() {
            return false;
        }
        let cell = &mut self.cells[index];
        cell.state = match cell.state {
            MinesweeperCellState::Hidden => MinesweeperCellState::Flagged,
            MinesweeperCellState::Flagged | MinesweeperCellState::Question => {
                MinesweeperCellState::Hidden
            }
            MinesweeperCellState::Revealed => return false,
        };
        true
    }

    #[must_use]
    pub fn chord(&mut self, index: usize) -> MinesweeperActionResult {
        if !self.accepts_action()
            || index >= self.cells.len()
            || self.cells[index].state != MinesweeperCellState::Revealed
            || self.cells[index].adjacent_mines == 0
        {
            return MinesweeperActionResult::default();
        }
        let flagged_neighbors = self
            .neighbors(index)
            .into_iter()
            .filter(|neighbor| self.cells[*neighbor].state == MinesweeperCellState::Flagged)
            .count();
        if flagged_neighbors != usize::from(self.cells[index].adjacent_mines) {
            return MinesweeperActionResult::default();
        }

        let mut changed = false;
        for neighbor in self.neighbors(index) {
            if matches!(self.cells[neighbor].state, MinesweeperCellState::Flagged) {
                continue;
            }
            if self.cells[neighbor].mine {
                self.cells[neighbor].state = MinesweeperCellState::Revealed;
                self.cells[neighbor].detonated = true;
                self.lose();
                return MinesweeperActionResult {
                    changed: true,
                    started: false,
                };
            }
            changed |= self.reveal_safe_area(neighbor);
        }

        self.update_win_status();
        MinesweeperActionResult {
            changed,
            started: false,
        }
    }

    #[must_use]
    pub fn neighbors(&self, index: usize) -> Vec<usize> {
        let Some((row, col)) = self.row_col(index) else {
            return Vec::new();
        };
        let mut neighbors = Vec::with_capacity(8);
        let row_start = row.saturating_sub(1);
        let row_end = row.saturating_add(1).min(self.height.saturating_sub(1));
        let col_start = col.saturating_sub(1);
        let col_end = col.saturating_add(1).min(self.width.saturating_sub(1));
        for next_row in row_start..=row_end {
            for next_col in col_start..=col_end {
                if next_row == row && next_col == col {
                    continue;
                }
                if let Some(index) = self.index(next_row, next_col) {
                    neighbors.push(index);
                }
            }
        }
        neighbors
    }

    const fn accepts_action(&self) -> bool {
        matches!(
            self.status,
            MinesweeperStatus::Ready | MinesweeperStatus::Playing
        )
    }

    fn place_mines(&mut self, first_reveal: usize) {
        if self.no_guess {
            self.place_no_guess_mines(first_reveal);
            return;
        }

        let mut seed = self.seed;
        self.place_mines_for_seed(first_reveal, &mut seed);
        self.seed = seed;
        self.mines_placed = true;
    }

    fn place_no_guess_mines(&mut self, first_reveal: usize) {
        let mut seed = self.seed;
        loop {
            self.place_mines_for_seed(first_reveal, &mut seed);
            if self.can_solve_without_guessing_from(first_reveal) {
                self.seed = seed;
                self.mines_placed = true;
                return;
            }
        }
    }

    fn place_mines_for_seed(&mut self, first_reveal: usize, seed: &mut u64) {
        for cell in &mut self.cells {
            cell.mine = false;
            cell.adjacent_mines = 0;
            cell.detonated = false;
        }

        let mut excluded = BTreeSet::from([first_reveal]);
        excluded.extend(self.neighbors(first_reveal));

        let mut candidates = (0..self.cells.len())
            .filter(|index| !excluded.contains(index))
            .collect::<Vec<_>>();
        if candidates.len() < self.mines {
            candidates = (0..self.cells.len())
                .filter(|index| *index != first_reveal)
                .collect();
        }

        shuffle_indexes(&mut candidates, seed);
        for index in candidates.into_iter().take(self.mines) {
            self.cells[index].mine = true;
        }
        self.recalculate_adjacent_counts();
    }

    pub fn recalculate_adjacent_counts(&mut self) {
        for index in 0..self.cells.len() {
            self.cells[index].adjacent_mines = if self.cells[index].mine {
                0
            } else {
                let adjacent_mines = self
                    .neighbors(index)
                    .into_iter()
                    .filter(|neighbor| self.cells[*neighbor].mine)
                    .count();
                u8::try_from(adjacent_mines).unwrap_or(u8::MAX)
            };
        }
    }

    #[must_use]
    pub fn reveal_safe_cells(&mut self, index: usize) -> Vec<usize> {
        if index >= self.cells.len()
            || self.cells[index].mine
            || self.cells[index].state == MinesweeperCellState::Flagged
            || self.cells[index].state == MinesweeperCellState::Revealed
        {
            return Vec::new();
        }

        let mut revealed = Vec::new();
        let mut queue = VecDeque::from([index]);
        while let Some(next) = queue.pop_front() {
            if self.cells[next].mine
                || self.cells[next].state == MinesweeperCellState::Flagged
                || self.cells[next].state == MinesweeperCellState::Revealed
            {
                continue;
            }

            self.cells[next].state = MinesweeperCellState::Revealed;
            revealed.push(next);
            if self.cells[next].adjacent_mines == 0 {
                for neighbor in self.neighbors(next) {
                    if !self.cells[neighbor].mine
                        && !matches!(
                            self.cells[neighbor].state,
                            MinesweeperCellState::Flagged | MinesweeperCellState::Revealed
                        )
                    {
                        queue.push_back(neighbor);
                    }
                }
            }
        }
        revealed
    }

    fn reveal_safe_area(&mut self, index: usize) -> bool {
        !self.reveal_safe_cells(index).is_empty()
    }

    #[must_use]
    pub fn all_safe_cells_revealed(&self) -> bool {
        self.cells
            .iter()
            .all(|cell| cell.mine || cell.state == MinesweeperCellState::Revealed)
    }

    fn update_win_status(&mut self) {
        if self.status == MinesweeperStatus::Lost {
            return;
        }
        let won = self
            .cells
            .iter()
            .all(|cell| cell.mine || cell.state == MinesweeperCellState::Revealed);
        if won {
            for cell in &mut self.cells {
                if cell.mine && cell.state == MinesweeperCellState::Hidden {
                    cell.state = MinesweeperCellState::Flagged;
                }
            }
            self.status = MinesweeperStatus::Won;
        }
    }

    fn lose(&mut self) {
        for cell in &mut self.cells {
            if cell.mine && cell.state != MinesweeperCellState::Flagged {
                cell.state = MinesweeperCellState::Revealed;
            }
        }
        self.status = MinesweeperStatus::Lost;
    }

    fn can_solve_without_guessing_from(&self, first_reveal: usize) -> bool {
        solver::can_solve_without_guessing_from(self, first_reveal)
    }
}

impl Default for MinesweeperCell {
    fn default() -> Self {
        Self {
            mine: false,
            adjacent_mines: 0,
            state: MinesweeperCellState::Hidden,
            detonated: false,
        }
    }
}

fn shuffle_indexes(values: &mut [usize], seed: &mut u64) {
    for index in (1..values.len()).rev() {
        let swap_index = random_index(seed, index + 1);
        values.swap(index, swap_index);
    }
}

fn next_seed(seed: &mut u64) -> u64 {
    let mut value = *seed;
    value ^= value << 13;
    value ^= value >> 7;
    value ^= value << 17;
    *seed = value.max(1);
    *seed
}

#[must_use]
pub fn build_room_minesweeper_board(
    tile_rows: usize,
    tile_cols: usize,
    seed: u64,
) -> RoomMinesweeperBoard {
    let tile_rows = clamp_room_minesweeper_tile_axis(tile_rows);
    let tile_cols = clamp_room_minesweeper_tile_axis(tile_cols);
    let tile_count = tile_rows * tile_cols;
    let width = tile_cols * MINESWEEPER_EXPERT_WIDTH;
    let height = tile_rows * MINESWEEPER_EXPERT_HEIGHT;
    let mut seed = seed.max(1);
    let mut mines = BTreeSet::new();
    let mut starting_cells = Vec::with_capacity(tile_count);

    for tile_index in 0..tile_count {
        let tile_row = tile_index / tile_cols;
        let tile_col = tile_index % tile_cols;
        let start_row = random_tile_start_axis(&mut seed, MINESWEEPER_EXPERT_HEIGHT);
        let start_col = random_tile_start_axis(&mut seed, MINESWEEPER_EXPERT_WIDTH);
        let tile_start = start_row * MINESWEEPER_EXPERT_WIDTH + start_col;
        let tile_seed = next_seed(&mut seed) ^ u64::try_from(tile_index).unwrap_or(0);
        let mut tile = MinesweeperBoard::new_no_guess_expert(tile_seed);
        let _ = tile.reveal(tile_start);

        let global_start_row = tile_row * MINESWEEPER_EXPERT_HEIGHT + start_row;
        let global_start_col = tile_col * MINESWEEPER_EXPERT_WIDTH + start_col;
        starting_cells.push(global_start_row * width + global_start_col);

        for (local_index, cell) in tile.cells.iter().enumerate() {
            if !cell.mine {
                continue;
            }
            let local_row = local_index / MINESWEEPER_EXPERT_WIDTH;
            let local_col = local_index % MINESWEEPER_EXPERT_WIDTH;
            let global_row = tile_row * MINESWEEPER_EXPERT_HEIGHT + local_row;
            let global_col = tile_col * MINESWEEPER_EXPERT_WIDTH + local_col;
            mines.insert(global_row * width + global_col);
        }
    }

    RoomMinesweeperBoard {
        board: MinesweeperBoard::from_mines(width, height, mines, seed),
        starting_cells,
        tile_rows,
        tile_cols,
    }
}

fn random_tile_start_axis(seed: &mut u64, size: usize) -> usize {
    let min = 4usize;
    let max_exclusive = size.saturating_sub(4);
    if max_exclusive <= min {
        return size / 2;
    }
    min + random_index(seed, max_exclusive - min)
}

fn random_index(seed: &mut u64, upper_exclusive: usize) -> usize {
    if upper_exclusive == 0 {
        return 0;
    }
    let bound = u64::try_from(upper_exclusive).expect("usize must fit in u64");
    usize::try_from(next_seed(seed) % bound).expect("random value is less than upper bound")
}

#[must_use]
pub const fn default_room_minesweeper_time_limit_seconds() -> u32 {
    ROOM_MINESWEEPER_DEFAULT_SECONDS
}

#[must_use]
pub const fn clamp_room_minesweeper_time_limit_seconds(seconds: u32) -> u32 {
    if seconds < ROOM_MINESWEEPER_MIN_SECONDS {
        ROOM_MINESWEEPER_MIN_SECONDS
    } else if seconds > ROOM_MINESWEEPER_MAX_SECONDS {
        ROOM_MINESWEEPER_MAX_SECONDS
    } else {
        seconds
    }
}

#[must_use]
pub const fn default_room_minesweeper_tile_rows() -> usize {
    ROOM_MINESWEEPER_DEFAULT_TILE_ROWS
}

#[must_use]
pub const fn default_room_minesweeper_tile_cols() -> usize {
    ROOM_MINESWEEPER_DEFAULT_TILE_COLS
}

#[must_use]
pub const fn clamp_room_minesweeper_tile_axis(value: usize) -> usize {
    if value < ROOM_MINESWEEPER_MIN_TILE_AXIS {
        ROOM_MINESWEEPER_MIN_TILE_AXIS
    } else if value > ROOM_MINESWEEPER_MAX_TILE_AXIS {
        ROOM_MINESWEEPER_MAX_TILE_AXIS
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minesweeper_expert_starts_with_classic_dimensions() {
        let board = MinesweeperBoard::new_expert(42);

        assert_eq!(board.width, 30);
        assert_eq!(board.height, 16);
        assert_eq!(board.mines, 99);
        assert_eq!(board.cells.len(), 480);
        assert_eq!(board.remaining_mines(), 99);
    }

    #[test]
    fn minesweeper_no_guess_mode_is_opt_in_for_board_constructor() {
        assert!(!MinesweeperBoard::new_expert(42).no_guess);
        assert!(MinesweeperBoard::new_no_guess_expert(42).no_guess);
    }

    #[test]
    fn minesweeper_first_reveal_keeps_opening_safe() {
        let mut board = MinesweeperBoard::new_expert(42);
        let first = board.index(8, 15).expect("valid cell");
        let opening = std::iter::once(first)
            .chain(board.neighbors(first))
            .collect::<BTreeSet<_>>();

        let result = board.reveal(first);

        assert!(result.changed);
        assert!(result.started);
        assert_eq!(board.status, MinesweeperStatus::Playing);
        assert_eq!(board.cells.iter().filter(|cell| cell.mine).count(), 99);
        for index in opening {
            assert!(!board.cells[index].mine);
        }
    }

    #[test]
    fn minesweeper_no_guess_first_reveal_is_solver_proven() {
        let mut board = MinesweeperBoard::new_no_guess_expert(42);
        let first = board.index(8, 15).expect("valid cell");
        let opening = std::iter::once(first)
            .chain(board.neighbors(first))
            .collect::<BTreeSet<_>>();

        let result = board.reveal(first);

        assert!(result.changed);
        assert!(result.started);
        assert_eq!(board.status, MinesweeperStatus::Playing);
        assert_eq!(board.cells.iter().filter(|cell| cell.mine).count(), 99);
        for index in opening {
            assert!(!board.cells[index].mine);
        }
        assert!(board.can_solve_without_guessing_from(first));
    }

    #[test]
    fn minesweeper_no_guess_generation_handles_fixed_expert_starts() {
        for (seed, row, col) in [(1, 0, 0), (7, 15, 29), (12345, 4, 17)] {
            let mut board = MinesweeperBoard::new_no_guess_expert(seed);
            let first = board.index(row, col).expect("valid cell");

            let result = board.reveal(first);

            assert!(result.changed);
            assert_eq!(board.cells.iter().filter(|cell| cell.mine).count(), 99);
            assert!(board.can_solve_without_guessing_from(first));
        }
    }

    #[test]
    fn room_minesweeper_tile_axes_are_clamped() {
        assert_eq!(clamp_room_minesweeper_tile_axis(0), 1);
        assert_eq!(clamp_room_minesweeper_tile_axis(2), 2);
        assert_eq!(clamp_room_minesweeper_tile_axis(99), 3);
    }

    #[test]
    fn room_minesweeper_stitches_no_guess_tiles_without_whole_board_verification() {
        let board = build_room_minesweeper_board(2, 1, 123);

        assert_eq!(board.tile_rows, 2);
        assert_eq!(board.tile_cols, 1);
        assert_eq!(board.board.width, MINESWEEPER_EXPERT_WIDTH);
        assert_eq!(board.board.height, MINESWEEPER_EXPERT_HEIGHT * 2);
        assert_eq!(board.board.mines, MINESWEEPER_EXPERT_MINES * 2);
        assert_eq!(board.starting_cells.len(), 2);
        assert_eq!(
            board.board.cells.iter().filter(|cell| cell.mine).count(),
            MINESWEEPER_EXPERT_MINES * 2
        );

        for start in board.starting_cells {
            let row = start / board.board.width;
            let col = start % board.board.width;
            let tile_row = row % MINESWEEPER_EXPERT_HEIGHT;
            let tile_col = col % MINESWEEPER_EXPERT_WIDTH;
            assert!(tile_row >= 4);
            assert!(tile_col >= 4);
            assert!(MINESWEEPER_EXPERT_HEIGHT - 1 - tile_row >= 4);
            assert!(MINESWEEPER_EXPERT_WIDTH - 1 - tile_col >= 4);
        }
    }

    #[test]
    fn minesweeper_marks_toggle_and_counter_can_go_negative() {
        let mut board = MinesweeperBoard::new(2, 2, 1, 7);

        assert!(board.toggle_mark(0));
        assert_eq!(board.cells[0].state, MinesweeperCellState::Flagged);
        assert_eq!(board.remaining_mines(), 0);
        assert!(board.toggle_mark(1));
        assert_eq!(board.remaining_mines(), -1);
        assert!(board.toggle_mark(0));
        assert_eq!(board.cells[0].state, MinesweeperCellState::Hidden);
    }

    #[test]
    fn minesweeper_chording_reveals_neighbors_when_flags_match() {
        let mut board = MinesweeperBoard::new(3, 3, 1, 9);
        let mine = board.index(0, 0).unwrap();
        let center = board.index(1, 1).unwrap();
        board.cells[mine].mine = true;
        board.recalculate_adjacent_counts();
        board.mines_placed = true;
        board.status = MinesweeperStatus::Playing;
        board.cells[mine].state = MinesweeperCellState::Flagged;
        board.cells[center].state = MinesweeperCellState::Revealed;

        let result = board.chord(center);

        assert!(result.changed);
        assert_eq!(board.status, MinesweeperStatus::Won);
        assert_eq!(board.cells[mine].state, MinesweeperCellState::Flagged);
        for neighbor in board.neighbors(center) {
            if neighbor != mine {
                assert_eq!(board.cells[neighbor].state, MinesweeperCellState::Revealed);
            }
        }
    }

    #[test]
    fn minesweeper_chording_with_wrong_flag_loses() {
        let mut board = MinesweeperBoard::new(3, 3, 1, 9);
        let mine = board.index(0, 0).unwrap();
        let wrong_flag = board.index(0, 1).unwrap();
        let center = board.index(1, 1).unwrap();
        board.cells[mine].mine = true;
        board.recalculate_adjacent_counts();
        board.mines_placed = true;
        board.status = MinesweeperStatus::Playing;
        board.cells[wrong_flag].state = MinesweeperCellState::Flagged;
        board.cells[center].state = MinesweeperCellState::Revealed;

        let result = board.chord(center);

        assert!(result.changed);
        assert_eq!(board.status, MinesweeperStatus::Lost);
        assert!(board.cells[mine].detonated);
    }

    #[test]
    fn minesweeper_revealing_all_safe_cells_wins() {
        let mut board = MinesweeperBoard::new(2, 1, 1, 11);
        board.cells[1].mine = true;
        board.recalculate_adjacent_counts();
        board.mines_placed = true;

        let result = board.reveal(0);

        assert!(result.changed);
        assert_eq!(board.status, MinesweeperStatus::Won);
        assert_eq!(board.cells[1].state, MinesweeperCellState::Flagged);
    }
}
