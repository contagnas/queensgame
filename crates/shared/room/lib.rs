use queensgame_shared_minesweeper::{
    default_room_minesweeper_tile_cols, default_room_minesweeper_tile_rows,
    default_room_minesweeper_time_limit_seconds,
};
use queensgame_shared_nonogram::NonogramPuzzle;
use queensgame_shared_queens::{CellState, Puzzle};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct RoomBootstrap {
    pub slug: String,
    pub total_puzzles: usize,
    pub snapshot: RoomSnapshot,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct RoomSnapshot {
    pub slug: String,
    #[serde(default)]
    pub game_kind: RoomGameKind,
    pub phase: RoomPhase,
    pub puzzle_choice: RoomPuzzleChoice,
    #[serde(default = "default_room_minesweeper_time_limit_seconds")]
    pub minesweeper_time_limit_seconds: u32,
    #[serde(default = "default_room_minesweeper_tile_rows")]
    pub minesweeper_tile_rows: usize,
    #[serde(default = "default_room_minesweeper_tile_cols")]
    pub minesweeper_tile_cols: usize,
    #[serde(default)]
    pub played_puzzle_ids: Vec<usize>,
    pub players: Vec<RoomPlayerSnapshot>,
    pub puzzle: Option<Puzzle>,
    #[serde(default)]
    pub minesweeper: Option<RoomMinesweeperSnapshot>,
    #[serde(default)]
    pub nonogram: Option<RoomNonogramSnapshot>,
    pub winner_id: Option<String>,
}

#[derive(Debug, Default, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RoomGameKind {
    #[default]
    Queens,
    Minesweeper,
    Nonogram,
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
    #[must_use]
    pub const fn is_lobby(&self) -> bool {
        matches!(self, Self::Lobby)
    }

    #[must_use]
    pub const fn race_started_at_ms(&self) -> Option<u64> {
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

#[allow(clippy::struct_excessive_bools)]
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
    #[serde(default)]
    pub minesweeper_score: u32,
    #[serde(default)]
    pub minesweeper_eliminated: bool,
    #[serde(default)]
    pub minesweeper_last_score_ms: Option<u64>,
    #[serde(default)]
    pub minesweeper_flags: Vec<usize>,
    #[serde(default)]
    pub pointer: Option<RoomLivePointer>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct RoomMinesweeperSnapshot {
    pub width: usize,
    pub height: usize,
    pub mines: usize,
    pub starting_cells: Vec<usize>,
    pub cells: Vec<RoomMinesweeperCellSnapshot>,
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct RoomMinesweeperCellSnapshot {
    pub revealed: bool,
    pub mine: bool,
    pub detonated: bool,
    pub start: bool,
    pub adjacent_mines: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct RoomNonogramSnapshot {
    pub puzzle: NonogramPuzzle,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub struct RoomLivePointer {
    pub x: u16,
    pub y: u16,
    pub cell_index: Option<u16>,
    pub active_click: bool,
    pub updated_at_ms: u64,
}

#[derive(Debug, Default, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub struct RoomMedalCounts {
    pub gold: u32,
    pub silver: u32,
    pub bronze: u32,
}

impl RoomMedalCounts {
    #[must_use]
    pub const fn total(self) -> u32 {
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
    SelectGame {
        game_kind: RoomGameKind,
    },
    SelectPuzzle {
        puzzle_id: usize,
    },
    SelectRandom,
    SetMinesweeperTimeLimit {
        seconds: u32,
    },
    SetMinesweeperTiles {
        rows: usize,
        cols: usize,
    },
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
    MinesweeperReveal {
        index: usize,
    },
    MinesweeperToggleFlag {
        index: usize,
    },
    MinesweeperChord {
        index: usize,
    },
    NonogramFinish {
        filled: Vec<usize>,
    },
    PointerUpdate {
        pointer: Option<RoomLivePointer>,
    },
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RoomServerMessage {
    Snapshot {
        snapshot: RoomSnapshot,
    },
    PointerUpdate {
        player_id: String,
        pointer: Option<RoomLivePointer>,
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

#[must_use]
pub fn recording_frame_is_valid(frame: &RoomRecordingFrame, expected_cells: usize) -> bool {
    frame.states.len() == expected_cells
        && frame
            .states
            .iter()
            .all(|state| CellState::from_storage_code(*state).storage_code() == *state)
}

#[must_use]
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

#[must_use]
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

#[must_use]
pub fn append_mouse_recording(
    recording: &mut RoomMouseRecording,
    mut chunk: RoomMouseRecording,
) -> bool {
    if !mouse_recording_times_are_sorted(&chunk) {
        return false;
    }
    if let (Some(last), Some(first)) = (recording.samples.last(), chunk.samples.first())
        && first.0 < last.0
    {
        return false;
    }
    if let (Some(last), Some(first)) = (recording.events.last(), chunk.events.first())
        && first.0 < last.0
    {
        return false;
    }

    recording.samples.append(&mut chunk.samples);
    recording.events.append(&mut chunk.events);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
