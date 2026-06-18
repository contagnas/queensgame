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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MinesweeperBoard {
    size: MinesweeperBoardSize,
    placement: MinePlacement,
    cells: Vec<MinesweeperCellState>,
    phase: MinesweeperPhase,
    detonated_mines: BTreeSet<MinesweeperMineIndex>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MinesweeperBoardSize {
    width: usize,
    height: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MinePlacement {
    Pending {
        mines: usize,
        seed: u64,
        no_guess: bool,
    },
    Placed(MinesweeperLayout),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MinesweeperLayout {
    size: MinesweeperBoardSize,
    mines: BTreeSet<usize>,
    adjacent_mines: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MinesweeperPhase {
    Ready,
    Playing,
    Won,
    Lost {
        detonated_mine: MinesweeperMineIndex,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct MinesweeperMineIndex(usize);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomMinesweeperBoard {
    pub board: MinesweeperBoard,
    pub starting_cells: Vec<usize>,
    pub tile_rows: usize,
    pub tile_cols: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MinesweeperCell {
    Safe {
        adjacent_mines: u8,
        state: MinesweeperCellState,
    },
    Mine {
        state: MinesweeperMineCellState,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MinesweeperMineCellState {
    Hidden,
    Flagged,
    Revealed,
    Detonated,
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

impl MinesweeperBoardSize {
    #[must_use]
    pub const fn new(width: usize, height: usize) -> Option<Self> {
        if width == 0 || height == 0 || width > usize::MAX / height {
            None
        } else {
            Some(Self { width, height })
        }
    }

    #[must_use]
    pub const fn width(self) -> usize {
        self.width
    }

    #[must_use]
    pub const fn height(self) -> usize {
        self.height
    }

    #[must_use]
    pub const fn cell_count(self) -> usize {
        self.width * self.height
    }

    #[must_use]
    pub const fn contains_index(self, index: usize) -> bool {
        index < self.cell_count()
    }

    #[must_use]
    pub const fn index(self, row: usize, col: usize) -> Option<usize> {
        if row < self.height && col < self.width {
            Some(row * self.width + col)
        } else {
            None
        }
    }

    #[must_use]
    pub const fn row_col(self, index: usize) -> Option<(usize, usize)> {
        if self.contains_index(index) {
            Some((index / self.width, index % self.width))
        } else {
            None
        }
    }
}

impl MinesweeperLayout {
    #[must_use]
    pub fn from_mines(size: MinesweeperBoardSize, mines: BTreeSet<usize>) -> Self {
        let mines = mines
            .into_iter()
            .filter(|index| size.contains_index(*index))
            .collect::<BTreeSet<_>>();
        let mut layout = Self {
            size,
            mines,
            adjacent_mines: vec![0; size.cell_count()],
        };
        layout.recalculate_adjacent_counts();
        layout
    }

    #[must_use]
    pub fn mine_count(&self) -> usize {
        self.mines.len()
    }

    #[must_use]
    pub fn is_mine(&self, index: usize) -> bool {
        self.mines.contains(&index)
    }

    #[must_use]
    pub fn adjacent_mines(&self, index: usize) -> u8 {
        self.adjacent_mines.get(index).copied().unwrap_or(0)
    }

    pub fn mine_indices(&self) -> impl Iterator<Item = usize> + '_ {
        self.mines.iter().copied()
    }

    fn recalculate_adjacent_counts(&mut self) {
        for index in 0..self.size.cell_count() {
            self.adjacent_mines[index] = if self.is_mine(index) {
                0
            } else {
                let adjacent_mines = neighbors_for_size(self.size, index)
                    .into_iter()
                    .filter(|neighbor| self.is_mine(*neighbor))
                    .count();
                u8::try_from(adjacent_mines).unwrap_or(u8::MAX)
            };
        }
    }
}

impl MinesweeperCell {
    #[must_use]
    pub const fn safe(adjacent_mines: u8, state: MinesweeperCellState) -> Self {
        Self::Safe {
            adjacent_mines,
            state,
        }
    }

    #[must_use]
    pub const fn mine_cell(state: MinesweeperMineCellState) -> Self {
        Self::Mine { state }
    }

    #[must_use]
    pub const fn mine(self) -> bool {
        matches!(self, Self::Mine { .. })
    }

    #[must_use]
    pub const fn adjacent_mines(self) -> u8 {
        match self {
            Self::Safe { adjacent_mines, .. } => adjacent_mines,
            Self::Mine { .. } => 0,
        }
    }

    #[must_use]
    pub const fn state(self) -> MinesweeperCellState {
        match self {
            Self::Safe { state, .. } => state,
            Self::Mine { state } => state.as_cell_state(),
        }
    }

    #[must_use]
    pub const fn detonated(self) -> bool {
        matches!(
            self,
            Self::Mine {
                state: MinesweeperMineCellState::Detonated,
            }
        )
    }
}

impl MinesweeperMineCellState {
    const fn as_cell_state(self) -> MinesweeperCellState {
        match self {
            Self::Hidden => MinesweeperCellState::Hidden,
            Self::Flagged => MinesweeperCellState::Flagged,
            Self::Revealed | Self::Detonated => MinesweeperCellState::Revealed,
        }
    }
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
        Self::new_with_generation(width, height, mines, seed, false)
    }

    #[must_use]
    pub fn new_no_guess(width: usize, height: usize, mines: usize, seed: u64) -> Self {
        Self::new_with_generation(width, height, mines, seed, true)
    }

    fn new_with_generation(
        width: usize,
        height: usize,
        mines: usize,
        seed: u64,
        no_guess: bool,
    ) -> Self {
        let size = sanitized_minesweeper_size(width, height);
        let mines = mines.min(size.cell_count().saturating_sub(1));
        Self {
            size,
            placement: MinePlacement::Pending {
                mines,
                seed: seed.max(1),
                no_guess,
            },
            cells: vec![MinesweeperCellState::Hidden; size.cell_count()],
            phase: MinesweeperPhase::Ready,
            detonated_mines: BTreeSet::new(),
        }
    }

    #[must_use]
    pub fn from_mines(width: usize, height: usize, mines: BTreeSet<usize>, _seed: u64) -> Self {
        let size = sanitized_minesweeper_size(width, height);
        let layout = MinesweeperLayout::from_mines(size, mines);
        Self {
            size,
            cells: vec![MinesweeperCellState::Hidden; size.cell_count()],
            placement: MinePlacement::Placed(layout),
            phase: MinesweeperPhase::Ready,
            detonated_mines: BTreeSet::new(),
        }
    }

    #[must_use]
    pub const fn width(&self) -> usize {
        self.size.width()
    }

    #[must_use]
    pub const fn height(&self) -> usize {
        self.size.height()
    }

    #[must_use]
    pub fn mine_count(&self) -> usize {
        match &self.placement {
            MinePlacement::Pending { mines, .. } => *mines,
            MinePlacement::Placed(layout) => layout.mine_count(),
        }
    }

    #[must_use]
    pub const fn cell_count(&self) -> usize {
        self.size.cell_count()
    }

    #[must_use]
    pub const fn status(&self) -> MinesweeperStatus {
        match self.phase {
            MinesweeperPhase::Ready => MinesweeperStatus::Ready,
            MinesweeperPhase::Playing => MinesweeperStatus::Playing,
            MinesweeperPhase::Won => MinesweeperStatus::Won,
            MinesweeperPhase::Lost { .. } => MinesweeperStatus::Lost,
        }
    }

    #[must_use]
    pub const fn is_no_guess(&self) -> bool {
        matches!(
            self.placement,
            MinePlacement::Pending { no_guess: true, .. }
        )
    }

    #[must_use]
    pub const fn mines_placed(&self) -> bool {
        matches!(self.placement, MinePlacement::Placed(_))
    }

    pub fn set_playing(&mut self) {
        if self.phase == MinesweeperPhase::Ready {
            self.phase = MinesweeperPhase::Playing;
        }
    }

    #[must_use]
    pub fn cell(&self, index: usize) -> Option<MinesweeperCell> {
        let state = self.cells.get(index).copied()?;
        if self.is_mine(index) {
            Some(MinesweeperCell::mine_cell(
                self.mine_cell_state(index, state),
            ))
        } else {
            Some(MinesweeperCell::safe(self.adjacent_mines(index), state))
        }
    }

    pub fn cells(&self) -> impl Iterator<Item = MinesweeperCell> + '_ {
        (0..self.cell_count()).filter_map(|index| self.cell(index))
    }

    fn mine_cell_state(
        &self,
        index: usize,
        stored_state: MinesweeperCellState,
    ) -> MinesweeperMineCellState {
        let mine_index = MinesweeperMineIndex(index);
        if self.detonated_mines.contains(&mine_index)
            || matches!(
                self.phase,
                MinesweeperPhase::Lost { detonated_mine } if detonated_mine == mine_index
            )
        {
            return MinesweeperMineCellState::Detonated;
        }

        match stored_state {
            MinesweeperCellState::Hidden | MinesweeperCellState::Question => {
                MinesweeperMineCellState::Hidden
            }
            MinesweeperCellState::Flagged => MinesweeperMineCellState::Flagged,
            MinesweeperCellState::Revealed => MinesweeperMineCellState::Revealed,
        }
    }

    #[must_use]
    pub fn cell_state(&self, index: usize) -> Option<MinesweeperCellState> {
        self.cells.get(index).copied()
    }

    pub fn detonate_mine(&mut self, index: usize) -> bool {
        let Some(mine) = self.mine_index(index) else {
            return false;
        };
        self.cells[index] = MinesweeperCellState::Revealed;
        self.detonated_mines.insert(mine);
        true
    }

    fn mine_index(&self, index: usize) -> Option<MinesweeperMineIndex> {
        self.is_mine(index).then_some(MinesweeperMineIndex(index))
    }

    #[must_use]
    pub fn is_mine(&self, index: usize) -> bool {
        match &self.placement {
            MinePlacement::Pending { .. } => false,
            MinePlacement::Placed(layout) => layout.is_mine(index),
        }
    }

    #[must_use]
    pub fn adjacent_mines(&self, index: usize) -> u8 {
        match &self.placement {
            MinePlacement::Pending { .. } => 0,
            MinePlacement::Placed(layout) => layout.adjacent_mines(index),
        }
    }

    #[must_use]
    pub fn mine_indices(&self) -> Vec<usize> {
        match &self.placement {
            MinePlacement::Pending { .. } => Vec::new(),
            MinePlacement::Placed(layout) => layout.mine_indices().collect(),
        }
    }

    #[must_use]
    pub const fn index(&self, row: usize, col: usize) -> Option<usize> {
        self.size.index(row, col)
    }

    #[must_use]
    pub const fn row_col(&self, index: usize) -> Option<(usize, usize)> {
        self.size.row_col(index)
    }

    #[must_use]
    pub fn flagged_count(&self) -> usize {
        self.cells
            .iter()
            .filter(|state| **state == MinesweeperCellState::Flagged)
            .count()
    }

    #[must_use]
    pub fn remaining_mines(&self) -> i32 {
        let mines = i32::try_from(self.mine_count()).unwrap_or(i32::MAX);
        let flagged = i32::try_from(self.flagged_count()).unwrap_or(i32::MAX);
        mines.saturating_sub(flagged)
    }

    #[must_use]
    pub fn reveal(&mut self, index: usize) -> MinesweeperActionResult {
        if !self.accepts_action() || index >= self.cell_count() {
            return MinesweeperActionResult::default();
        }
        if !self.mines_placed() {
            self.place_mines(index);
        }
        if matches!(self.cells[index], MinesweeperCellState::Flagged) {
            return MinesweeperActionResult {
                changed: false,
                started: self.phase == MinesweeperPhase::Playing,
            };
        }

        let started = self.phase == MinesweeperPhase::Ready;
        self.phase = MinesweeperPhase::Playing;

        if let Some(mine) = self.mine_index(index) {
            self.cells[index] = MinesweeperCellState::Revealed;
            self.lose(mine);
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
        if !self.accepts_action() || index >= self.cell_count() {
            return false;
        }
        let cell = &mut self.cells[index];
        *cell = match *cell {
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
            || index >= self.cell_count()
            || self.cells[index] != MinesweeperCellState::Revealed
            || self.adjacent_mines(index) == 0
        {
            return MinesweeperActionResult::default();
        }
        let flagged_neighbors = self
            .neighbors(index)
            .into_iter()
            .filter(|neighbor| self.cells[*neighbor] == MinesweeperCellState::Flagged)
            .count();
        if flagged_neighbors != usize::from(self.adjacent_mines(index)) {
            return MinesweeperActionResult::default();
        }

        let mut changed = false;
        for neighbor in self.neighbors(index) {
            if matches!(self.cells[neighbor], MinesweeperCellState::Flagged) {
                continue;
            }
            if let Some(mine) = self.mine_index(neighbor) {
                self.cells[neighbor] = MinesweeperCellState::Revealed;
                self.lose(mine);
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
        neighbors_for_size(self.size, index)
    }

    const fn accepts_action(&self) -> bool {
        matches!(
            self.phase,
            MinesweeperPhase::Ready | MinesweeperPhase::Playing
        )
    }

    fn place_mines(&mut self, first_reveal: usize) {
        let MinePlacement::Pending {
            mines,
            mut seed,
            no_guess,
        } = self.placement.clone()
        else {
            return;
        };
        let layout = if no_guess {
            self.no_guess_layout(first_reveal, mines, &mut seed)
        } else {
            Self::layout_for_seed(self.size, mines, first_reveal, &mut seed)
        };
        self.placement = MinePlacement::Placed(layout);
    }

    fn no_guess_layout(
        &self,
        first_reveal: usize,
        mines: usize,
        seed: &mut u64,
    ) -> MinesweeperLayout {
        loop {
            let layout = Self::layout_for_seed(self.size, mines, first_reveal, seed);
            let mut candidate = self.clone();
            candidate.placement = MinePlacement::Placed(layout.clone());
            if candidate.can_solve_without_guessing_from(first_reveal) {
                return layout;
            }
        }
    }

    fn layout_for_seed(
        size: MinesweeperBoardSize,
        mines: usize,
        first_reveal: usize,
        seed: &mut u64,
    ) -> MinesweeperLayout {
        let mut excluded = BTreeSet::from([first_reveal]);
        excluded.extend(neighbors_for_size(size, first_reveal));

        let mut candidates = (0..size.cell_count())
            .filter(|index| !excluded.contains(index))
            .collect::<Vec<_>>();
        if candidates.len() < mines {
            candidates = (0..size.cell_count())
                .filter(|index| *index != first_reveal)
                .collect();
        }

        shuffle_indexes(&mut candidates, seed);
        let mine_indices = candidates.into_iter().take(mines).collect::<BTreeSet<_>>();
        MinesweeperLayout::from_mines(size, mine_indices)
    }

    #[must_use]
    pub fn reveal_safe_cells(&mut self, index: usize) -> Vec<usize> {
        if index >= self.cell_count() {
            return Vec::new();
        }
        if !self.mines_placed() {
            self.place_mines(index);
        }
        if self.is_mine(index)
            || self.cells[index] == MinesweeperCellState::Flagged
            || self.cells[index] == MinesweeperCellState::Revealed
        {
            return Vec::new();
        }

        let mut revealed = Vec::new();
        let mut queue = VecDeque::from([index]);
        while let Some(next) = queue.pop_front() {
            if self.is_mine(next)
                || self.cells[next] == MinesweeperCellState::Flagged
                || self.cells[next] == MinesweeperCellState::Revealed
            {
                continue;
            }

            self.cells[next] = MinesweeperCellState::Revealed;
            revealed.push(next);
            if self.adjacent_mines(next) == 0 {
                for neighbor in self.neighbors(next) {
                    if !self.is_mine(neighbor)
                        && !matches!(
                            self.cells[neighbor],
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
            .enumerate()
            .all(|(index, state)| self.is_mine(index) || *state == MinesweeperCellState::Revealed)
    }

    fn update_win_status(&mut self) {
        if matches!(self.phase, MinesweeperPhase::Lost { .. }) {
            return;
        }
        if self.all_safe_cells_revealed() {
            for index in self.mine_indices() {
                if self.cells[index] == MinesweeperCellState::Hidden {
                    self.cells[index] = MinesweeperCellState::Flagged;
                }
            }
            self.phase = MinesweeperPhase::Won;
        }
    }

    fn lose(&mut self, detonated_mine: MinesweeperMineIndex) {
        for index in self.mine_indices() {
            if self.cells[index] != MinesweeperCellState::Flagged {
                self.cells[index] = MinesweeperCellState::Revealed;
            }
        }
        self.phase = MinesweeperPhase::Lost { detonated_mine };
    }

    fn can_solve_without_guessing_from(&self, first_reveal: usize) -> bool {
        solver::can_solve_without_guessing_from(self, first_reveal)
    }
}

impl Default for MinesweeperCell {
    fn default() -> Self {
        Self::safe(0, MinesweeperCellState::Hidden)
    }
}

const fn sanitized_minesweeper_size(width: usize, height: usize) -> MinesweeperBoardSize {
    match MinesweeperBoardSize::new(width, height) {
        Some(size) => size,
        None => MinesweeperBoardSize {
            width: 1,
            height: 1,
        },
    }
}

fn neighbors_for_size(size: MinesweeperBoardSize, index: usize) -> Vec<usize> {
    let Some((row, col)) = size.row_col(index) else {
        return Vec::new();
    };
    let mut neighbors = Vec::with_capacity(8);
    let row_start = row.saturating_sub(1);
    let row_end = row.saturating_add(1).min(size.height().saturating_sub(1));
    let col_start = col.saturating_sub(1);
    let col_end = col.saturating_add(1).min(size.width().saturating_sub(1));
    for next_row in row_start..=row_end {
        for next_col in col_start..=col_end {
            if next_row == row && next_col == col {
                continue;
            }
            if let Some(index) = size.index(next_row, next_col) {
                neighbors.push(index);
            }
        }
    }
    neighbors
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

        for local_index in tile.mine_indices() {
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

        assert_eq!(board.width(), 30);
        assert_eq!(board.height(), 16);
        assert_eq!(board.mine_count(), 99);
        assert_eq!(board.cell_count(), 480);
        assert_eq!(board.remaining_mines(), 99);
    }

    #[test]
    fn minesweeper_no_guess_mode_is_opt_in_for_board_constructor() {
        assert!(!MinesweeperBoard::new_expert(42).is_no_guess());
        assert!(MinesweeperBoard::new_no_guess_expert(42).is_no_guess());
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
        assert_eq!(board.status(), MinesweeperStatus::Playing);
        assert_eq!(board.mine_indices().len(), 99);
        for index in opening {
            assert!(!board.is_mine(index));
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
        assert_eq!(board.status(), MinesweeperStatus::Playing);
        assert_eq!(board.mine_indices().len(), 99);
        for index in opening {
            assert!(!board.is_mine(index));
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
            assert_eq!(board.mine_indices().len(), 99);
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
        assert_eq!(board.board.width(), MINESWEEPER_EXPERT_WIDTH);
        assert_eq!(board.board.height(), MINESWEEPER_EXPERT_HEIGHT * 2);
        assert_eq!(board.board.mine_count(), MINESWEEPER_EXPERT_MINES * 2);
        assert_eq!(board.starting_cells.len(), 2);
        assert_eq!(
            board.board.mine_indices().len(),
            MINESWEEPER_EXPERT_MINES * 2
        );

        for start in board.starting_cells {
            let row = start / board.board.width();
            let col = start % board.board.width();
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
        assert_eq!(board.cell_state(0), Some(MinesweeperCellState::Flagged));
        assert_eq!(board.remaining_mines(), 0);
        assert!(board.toggle_mark(1));
        assert_eq!(board.remaining_mines(), -1);
        assert!(board.toggle_mark(0));
        assert_eq!(board.cell_state(0), Some(MinesweeperCellState::Hidden));
    }

    #[test]
    fn minesweeper_chording_reveals_neighbors_when_flags_match() {
        let mine = 0;
        let center = 4;
        let mut board = MinesweeperBoard::from_mines(3, 3, BTreeSet::from([mine]), 9);
        board.set_playing();
        assert!(board.toggle_mark(mine));
        assert_eq!(board.reveal_safe_cells(center), vec![center]);

        let result = board.chord(center);

        assert!(result.changed);
        assert_eq!(board.status(), MinesweeperStatus::Won);
        assert_eq!(board.cell_state(mine), Some(MinesweeperCellState::Flagged));
        for neighbor in board.neighbors(center) {
            if neighbor != mine {
                assert_eq!(
                    board.cell_state(neighbor),
                    Some(MinesweeperCellState::Revealed)
                );
            }
        }
    }

    #[test]
    fn minesweeper_chording_with_wrong_flag_loses() {
        let mine = 0;
        let wrong_flag = 1;
        let center = 4;
        let mut board = MinesweeperBoard::from_mines(3, 3, BTreeSet::from([mine]), 9);
        board.set_playing();
        assert!(board.toggle_mark(wrong_flag));
        assert_eq!(board.reveal_safe_cells(center), vec![center]);

        let result = board.chord(center);

        assert!(result.changed);
        assert_eq!(board.status(), MinesweeperStatus::Lost);
        assert!(board.cell(mine).is_some_and(MinesweeperCell::detonated));
    }

    #[test]
    fn minesweeper_revealing_all_safe_cells_wins() {
        let mut board = MinesweeperBoard::from_mines(2, 1, BTreeSet::from([1]), 11);

        let result = board.reveal(0);

        assert!(result.changed);
        assert_eq!(board.status(), MinesweeperStatus::Won);
        assert_eq!(board.cell_state(1), Some(MinesweeperCellState::Flagged));
    }
}
