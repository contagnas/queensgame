use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, VecDeque};

pub const DISPLAY_NAME_MAX_CHARS: usize = 32;

pub fn normalize_display_name(name: &str) -> Option<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.chars().take(DISPLAY_NAME_MAX_CHARS).collect())
    }
}

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

pub const MINESWEEPER_EXPERT_WIDTH: usize = 30;
pub const MINESWEEPER_EXPERT_HEIGHT: usize = 16;
pub const MINESWEEPER_EXPERT_MINES: usize = 99;

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
    pub fn new_expert(seed: u64) -> Self {
        Self::new(
            MINESWEEPER_EXPERT_WIDTH,
            MINESWEEPER_EXPERT_HEIGHT,
            MINESWEEPER_EXPERT_MINES,
            seed,
        )
    }

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
        }
    }

    pub fn index(&self, row: usize, col: usize) -> Option<usize> {
        (row < self.height && col < self.width).then_some(row * self.width + col)
    }

    pub fn row_col(&self, index: usize) -> Option<(usize, usize)> {
        (index < self.cells.len()).then_some((index / self.width, index % self.width))
    }

    pub fn flagged_count(&self) -> usize {
        self.cells
            .iter()
            .filter(|cell| cell.state == MinesweeperCellState::Flagged)
            .count()
    }

    pub fn remaining_mines(&self) -> i32 {
        self.mines as i32 - self.flagged_count() as i32
    }

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

    pub fn toggle_mark(&mut self, index: usize) -> bool {
        if !self.accepts_action() || index >= self.cells.len() {
            return false;
        }
        let cell = &mut self.cells[index];
        cell.state = match cell.state {
            MinesweeperCellState::Hidden => MinesweeperCellState::Flagged,
            MinesweeperCellState::Flagged => MinesweeperCellState::Question,
            MinesweeperCellState::Question => MinesweeperCellState::Hidden,
            MinesweeperCellState::Revealed => return false,
        };
        true
    }

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

    pub fn neighbors(&self, index: usize) -> Vec<usize> {
        let Some((row, col)) = self.row_col(index) else {
            return Vec::new();
        };
        let mut neighbors = Vec::with_capacity(8);
        for row_offset in -1isize..=1 {
            for col_offset in -1isize..=1 {
                if row_offset == 0 && col_offset == 0 {
                    continue;
                }
                let next_row = row as isize + row_offset;
                let next_col = col as isize + col_offset;
                if next_row < 0 || next_col < 0 {
                    continue;
                }
                if let Some(index) = self.index(next_row as usize, next_col as usize) {
                    neighbors.push(index);
                }
            }
        }
        neighbors
    }

    fn accepts_action(&self) -> bool {
        matches!(
            self.status,
            MinesweeperStatus::Ready | MinesweeperStatus::Playing
        )
    }

    fn place_mines(&mut self, first_reveal: usize) {
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

        shuffle_indexes(&mut candidates, &mut self.seed);
        for index in candidates.into_iter().take(self.mines) {
            self.cells[index].mine = true;
        }
        self.recalculate_adjacent_counts();
        self.mines_placed = true;
    }

    fn recalculate_adjacent_counts(&mut self) {
        for index in 0..self.cells.len() {
            self.cells[index].adjacent_mines = if self.cells[index].mine {
                0
            } else {
                self.neighbors(index)
                    .into_iter()
                    .filter(|neighbor| self.cells[*neighbor].mine)
                    .count() as u8
            };
        }
    }

    fn reveal_safe_area(&mut self, index: usize) -> bool {
        if index >= self.cells.len()
            || self.cells[index].mine
            || self.cells[index].state == MinesweeperCellState::Flagged
            || self.cells[index].state == MinesweeperCellState::Revealed
        {
            return false;
        }

        let mut changed = false;
        let mut queue = VecDeque::from([index]);
        while let Some(next) = queue.pop_front() {
            if self.cells[next].mine
                || self.cells[next].state == MinesweeperCellState::Flagged
                || self.cells[next].state == MinesweeperCellState::Revealed
            {
                continue;
            }

            self.cells[next].state = MinesweeperCellState::Revealed;
            changed = true;
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
        changed
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
        let swap_index = (next_seed(seed) as usize) % (index + 1);
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

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct RoomBootstrap {
    pub slug: String,
    pub total_puzzles: usize,
    pub snapshot: RoomSnapshot,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct CreateRoomResponse {
    pub slug: String,
    pub path: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct RoomSnapshot {
    pub slug: String,
    pub phase: RoomPhase,
    pub puzzle_choice: RoomPuzzleChoice,
    #[serde(default)]
    pub played_puzzle_ids: Vec<usize>,
    pub players: Vec<RoomPlayerSnapshot>,
    pub puzzle: Option<Puzzle>,
    pub winner_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RoomPhase {
    Lobby,
    Countdown { starts_at_ms: u64 },
    Racing { started_at_ms: u64 },
    Complete { started_at_ms: u64 },
}

impl RoomPhase {
    pub fn is_lobby(&self) -> bool {
        matches!(self, Self::Lobby)
    }

    pub fn race_started_at_ms(&self) -> Option<u64> {
        match self {
            Self::Racing { started_at_ms } | Self::Complete { started_at_ms } => {
                Some(*started_at_ms)
            }
            Self::Lobby | Self::Countdown { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RoomPuzzleChoice {
    Puzzle { id: usize },
    Random,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct RoomPlayerSnapshot {
    pub id: String,
    pub name: String,
    pub ready: bool,
    pub connected: bool,
    pub finish_ms: Option<u64>,
    #[serde(default)]
    pub gave_up: bool,
    #[serde(default)]
    pub medals: RoomMedalCounts,
    pub recording: Option<RoomRecording>,
    pub mouse_recording: Option<RoomMouseRecording>,
}

#[derive(Debug, Default, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub struct RoomMedalCounts {
    pub gold: u32,
    pub silver: u32,
    pub bronze: u32,
}

impl RoomMedalCounts {
    pub fn total(self) -> u32 {
        self.gold + self.silver + self.bronze
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct RoomRecording {
    pub frames: Vec<RoomRecordingFrame>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct RoomRecordingFrame {
    pub elapsed_ms: u64,
    pub states: Vec<u8>,
}

pub const ROOM_MOUSE_EVENT_ENTER: u8 = 0;
pub const ROOM_MOUSE_EVENT_LEAVE: u8 = 1;
pub const ROOM_MOUSE_EVENT_PRIMARY_DOWN: u8 = 2;
pub const ROOM_MOUSE_EVENT_PRIMARY_UP: u8 = 3;
pub const ROOM_MOUSE_EVENT_SECONDARY_DOWN: u8 = 4;
pub const ROOM_MOUSE_EVENT_SECONDARY_UP: u8 = 5;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct RoomMouseRecording {
    pub samples: Vec<RoomMouseSample>,
    pub events: Vec<RoomMouseEvent>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub struct RoomMouseSample(pub u32, pub u16, pub u16);

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub struct RoomMouseEvent(pub u32, pub u8, pub u16, pub u16, pub Option<u16>);

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RoomClientMessage {
    SelectPuzzle {
        puzzle_id: usize,
    },
    SelectRandom,
    SetReady {
        ready: bool,
    },
    Finish {
        queens: Vec<[usize; 2]>,
        recording: RoomRecording,
    },
    GiveUp,
    RecordingFrame {
        frame: RoomRecordingFrame,
    },
    MouseRecordingChunk {
        recording: RoomMouseRecording,
    },
    MouseRecording {
        recording: RoomMouseRecording,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RoomServerMessage {
    Snapshot {
        snapshot: RoomSnapshot,
    },
    RecordingFrame {
        player_id: String,
        frame: RoomRecordingFrame,
    },
    MouseRecordingChunk {
        player_id: String,
        recording: RoomMouseRecording,
    },
    Error {
        message: String,
    },
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

pub fn recording_frame_is_valid(frame: &RoomRecordingFrame, expected_cells: usize) -> bool {
    frame.states.len() == expected_cells
        && frame
            .states
            .iter()
            .all(|state| CellState::from_storage_code(*state).storage_code() == *state)
}

pub fn append_recording_frame(recording: &mut RoomRecording, frame: RoomRecordingFrame) -> bool {
    if let Some(last_frame) = recording.frames.last_mut() {
        if frame.elapsed_ms < last_frame.elapsed_ms {
            return false;
        }
        if frame.elapsed_ms == last_frame.elapsed_ms {
            *last_frame = frame;
            return true;
        }
    }

    recording.frames.push(frame);
    true
}

pub fn mouse_recording_times_are_sorted(recording: &RoomMouseRecording) -> bool {
    recording
        .samples
        .windows(2)
        .all(|samples| samples[0].0 <= samples[1].0)
        && recording
            .events
            .windows(2)
            .all(|events| events[0].0 <= events[1].0)
}

pub fn append_mouse_recording(
    recording: &mut RoomMouseRecording,
    mut chunk: RoomMouseRecording,
) -> bool {
    if !mouse_recording_times_are_sorted(&chunk) {
        return false;
    }
    if let (Some(last), Some(first)) = (recording.samples.last(), chunk.samples.first()) {
        if first.0 < last.0 {
            return false;
        }
    }
    if let (Some(last), Some(first)) = (recording.events.last(), chunk.events.first()) {
        if first.0 < last.0 {
            return false;
        }
    }

    recording.samples.append(&mut chunk.samples);
    recording.events.append(&mut chunk.events);
    true
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
    fn display_names_are_trimmed_and_required() {
        assert_eq!(normalize_display_name("  Ada  "), Some("Ada".to_string()));
        assert_eq!(normalize_display_name("   "), None);

        let long_name = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJK";
        let normalized = normalize_display_name(long_name).expect("name is not empty");
        assert_eq!(normalized.chars().count(), DISPLAY_NAME_MAX_CHARS);
    }

    #[test]
    fn recording_frames_validate_and_append_in_order() {
        let mut recording = RoomRecording { frames: Vec::new() };
        let first = RoomRecordingFrame {
            elapsed_ms: 10,
            states: vec![0, 1, 2, 3],
        };
        let replacement = RoomRecordingFrame {
            elapsed_ms: 10,
            states: vec![3, 2, 1, 0],
        };
        let older = RoomRecordingFrame {
            elapsed_ms: 9,
            states: vec![0, 0, 0, 0],
        };

        assert!(recording_frame_is_valid(&first, 4));
        assert!(!recording_frame_is_valid(
            &RoomRecordingFrame {
                elapsed_ms: 11,
                states: vec![4],
            },
            1,
        ));
        assert!(append_recording_frame(&mut recording, first));
        assert!(append_recording_frame(&mut recording, replacement));
        assert!(!append_recording_frame(&mut recording, older));
        assert_eq!(recording.frames.len(), 1);
        assert_eq!(recording.frames[0].states, vec![3, 2, 1, 0]);
    }

    #[test]
    fn mouse_recording_chunks_append_in_order() {
        let mut recording = RoomMouseRecording {
            samples: vec![RoomMouseSample(10, 1, 1)],
            events: vec![RoomMouseEvent(12, ROOM_MOUSE_EVENT_ENTER, 1, 1, Some(0))],
        };
        let next = RoomMouseRecording {
            samples: vec![RoomMouseSample(20, 2, 2)],
            events: vec![RoomMouseEvent(
                22,
                ROOM_MOUSE_EVENT_PRIMARY_DOWN,
                2,
                2,
                Some(1),
            )],
        };
        let older = RoomMouseRecording {
            samples: vec![RoomMouseSample(19, 3, 3)],
            events: Vec::new(),
        };

        assert!(append_mouse_recording(&mut recording, next));
        assert!(!append_mouse_recording(&mut recording, older));
        assert_eq!(recording.samples.len(), 2);
        assert_eq!(recording.events.len(), 2);
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
    fn minesweeper_first_reveal_keeps_opening_safe() {
        let mut board = MinesweeperBoard::new_expert(42);
        let first = board.index(8, 15).expect("valid cell");
        let opening = BTreeSet::from_iter(std::iter::once(first).chain(board.neighbors(first)));

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
    fn minesweeper_marks_cycle_and_counter_can_go_negative() {
        let mut board = MinesweeperBoard::new(2, 2, 1, 7);

        assert!(board.toggle_mark(0));
        assert_eq!(board.cells[0].state, MinesweeperCellState::Flagged);
        assert_eq!(board.remaining_mines(), 0);
        assert!(board.toggle_mark(1));
        assert_eq!(board.remaining_mines(), -1);
        assert!(board.toggle_mark(0));
        assert_eq!(board.cells[0].state, MinesweeperCellState::Question);
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
