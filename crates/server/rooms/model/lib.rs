#![allow(clippy::missing_errors_doc)]

use queensgame_shared_minesweeper::{
    MinesweeperBoard, default_room_minesweeper_tile_cols, default_room_minesweeper_tile_rows,
    default_room_minesweeper_time_limit_seconds,
};
use queensgame_shared_queens::Puzzle;
use queensgame_shared_room::{
    RoomGameKind, RoomLivePointer, RoomMedalCounts, RoomMouseRecording, RoomPhase,
    RoomPuzzleChoice, RoomRecording,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{Mutex, broadcast};

#[derive(Clone)]
pub struct AppState {
    pub puzzles: Arc<Vec<Puzzle>>,
    pub rooms: Arc<Mutex<BTreeMap<String, Room>>>,
}

impl AppState {
    #[must_use]
    pub fn new(puzzles: Vec<Puzzle>) -> Self {
        Self {
            puzzles: Arc::new(puzzles),
            rooms: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }
}

pub struct Room {
    pub slug: String,
    pub game_kind: RoomGameKind,
    pub puzzle_choice: RoomPuzzleChoice,
    pub minesweeper_time_limit_seconds: u32,
    pub minesweeper_tile_rows: usize,
    pub minesweeper_tile_cols: usize,
    pub minesweeper: Option<ServerMinesweeperGame>,
    pub active_puzzle_id: Option<usize>,
    pub played_puzzle_ids: BTreeSet<usize>,
    pub players: BTreeMap<String, RoomPlayer>,
    pub race_player_ids: Vec<String>,
    pub phase: ServerRoomPhase,
    pub tx: broadcast::Sender<String>,
}

impl Room {
    #[must_use]
    pub const fn new(slug: String, tx: broadcast::Sender<String>) -> Self {
        Self {
            slug,
            game_kind: RoomGameKind::Queens,
            puzzle_choice: RoomPuzzleChoice::Random,
            minesweeper_time_limit_seconds: default_room_minesweeper_time_limit_seconds(),
            minesweeper_tile_rows: default_room_minesweeper_tile_rows(),
            minesweeper_tile_cols: default_room_minesweeper_tile_cols(),
            minesweeper: None,
            active_puzzle_id: None,
            played_puzzle_ids: BTreeSet::new(),
            players: BTreeMap::new(),
            race_player_ids: Vec::new(),
            phase: ServerRoomPhase::Lobby,
            tx,
        }
    }
}

#[allow(clippy::struct_excessive_bools)]
pub struct RoomPlayer {
    pub id: String,
    pub name: String,
    pub ready: bool,
    pub connected: bool,
    pub finish_ms: Option<u64>,
    pub gave_up: bool,
    pub medals: RoomMedalCounts,
    pub recording: Option<RoomRecording>,
    pub mouse_recording: Option<RoomMouseRecording>,
    pub minesweeper_score: u32,
    pub minesweeper_eliminated: bool,
    pub minesweeper_last_score_ms: Option<u64>,
    pub minesweeper_flags: BTreeSet<usize>,
    pub pointer: Option<RoomLivePointer>,
    pub joined_order: u64,
}

impl RoomPlayer {
    #[must_use]
    pub fn new(id: String, name: String, joined_order: u64) -> Self {
        Self {
            id,
            name,
            ready: false,
            connected: true,
            finish_ms: None,
            gave_up: false,
            medals: RoomMedalCounts::default(),
            recording: None,
            mouse_recording: None,
            minesweeper_score: 0,
            minesweeper_eliminated: false,
            minesweeper_last_score_ms: None,
            minesweeper_flags: BTreeSet::new(),
            pointer: None,
            joined_order,
        }
    }
}

pub struct ServerMinesweeperGame {
    pub board: MinesweeperBoard,
    pub starting_cells: Vec<usize>,
    pub cell_owners: BTreeMap<usize, String>,
}

pub enum ServerRoomPhase {
    Lobby,
    Countdown {
        starts_at_ms: u64,
    },
    Racing {
        started_at_ms: u64,
        started_at: Instant,
    },
    Complete {
        started_at_ms: u64,
    },
}

impl ServerRoomPhase {
    #[must_use]
    pub const fn as_snapshot_phase(&self) -> RoomPhase {
        match self {
            Self::Lobby => RoomPhase::Lobby,
            Self::Countdown { starts_at_ms } => RoomPhase::Countdown {
                starts_at_ms: *starts_at_ms,
            },
            Self::Racing { started_at_ms, .. } => RoomPhase::Racing {
                started_at_ms: *started_at_ms,
            },
            Self::Complete { started_at_ms } => RoomPhase::Complete {
                started_at_ms: *started_at_ms,
            },
        }
    }
}

#[must_use]
pub fn now_ms() -> u64 {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(u64::MAX)
}

#[must_use]
pub fn elapsed_millis_u64(started_at: Instant) -> u64 {
    u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[must_use]
pub const fn room_accepts_next_race_setup(room: &Room) -> bool {
    matches!(
        room.phase,
        ServerRoomPhase::Lobby | ServerRoomPhase::Complete { .. }
    )
}

pub fn reset_room_setup_for_selection(room: &mut Room) {
    reset_room_ready_flags(room);
    if matches!(room.phase, ServerRoomPhase::Lobby) {
        clear_room_race_results(room);
    }
}

pub fn reset_room_ready_flags(room: &mut Room) {
    for player in room.players.values_mut() {
        player.ready = false;
    }
}

pub fn clear_room_race_results(room: &mut Room) {
    for player in room.players.values_mut() {
        player.finish_ms = None;
        player.gave_up = false;
        player.recording = None;
        player.mouse_recording = None;
        player.minesweeper_score = 0;
        player.minesweeper_eliminated = false;
        player.minesweeper_last_score_ms = None;
        player.minesweeper_flags.clear();
        player.pointer = None;
    }
    room.race_player_ids.clear();
    room.active_puzzle_id = None;
    room.minesweeper = None;
}

pub fn begin_room_race_for_connected_players(room: &mut Room) {
    room.race_player_ids = room
        .players
        .values()
        .filter(|player| player.connected)
        .map(|player| player.id.clone())
        .collect();
    for player in room.players.values_mut() {
        player.ready = false;
        player.finish_ms = None;
        player.gave_up = false;
        player.recording = None;
        player.mouse_recording = None;
        player.minesweeper_score = 0;
        player.minesweeper_eliminated = false;
        player.minesweeper_last_score_ms = None;
        player.minesweeper_flags.clear();
        player.pointer = None;
    }
}

#[must_use]
pub fn room_all_connected_players_ready(room: &Room) -> bool {
    let mut connected_players = room
        .players
        .values()
        .filter(|player| player.connected)
        .peekable();
    if connected_players.peek().is_none() {
        return false;
    }
    connected_players.all(|player| player.ready)
}

#[must_use]
pub fn room_all_racers_done(room: &Room) -> bool {
    !room.race_player_ids.is_empty()
        && room.race_player_ids.iter().all(|player_id| {
            room.players
                .get(player_id)
                .is_some_and(|player| player.finish_ms.is_some() || player.gave_up)
        })
}

pub fn with_room_mut<T>(
    rooms: &mut BTreeMap<String, Room>,
    slug: &str,
    update: impl FnOnce(&mut Room) -> Option<T>,
) -> Option<T> {
    update(rooms.get_mut(slug)?)
}

pub fn with_room<T>(
    rooms: &BTreeMap<String, Room>,
    slug: &str,
    read: impl FnOnce(&Room) -> Option<T>,
) -> Option<T> {
    read(rooms.get(slug)?)
}

#[must_use]
pub fn require(condition: bool) -> Option<()> {
    condition.then_some(())
}

pub mod test_support {
    use super::{Room, RoomPlayer};
    use queensgame_shared_queens::Puzzle;
    use queensgame_shared_room::RoomPuzzleChoice;
    use tokio::sync::broadcast;

    #[must_use]
    pub fn test_puzzle(id: usize) -> Puzzle {
        Puzzle {
            id,
            size: 1,
            colors: vec!["#ffffff".to_string()],
            regions: vec![vec![0]],
        }
    }

    #[must_use]
    pub fn test_room(puzzle_choice: RoomPuzzleChoice) -> Room {
        let (tx, _) = broadcast::channel(4);
        Room::new("ROOMTEST".to_string(), tx).tap_mut(|room| {
            room.puzzle_choice = puzzle_choice;
        })
    }

    pub fn add_test_player(
        room: &mut Room,
        id: &str,
        finish_ms: Option<u64>,
        gave_up: bool,
        joined_order: u64,
    ) {
        room.players.insert(
            id.to_string(),
            RoomPlayer {
                finish_ms,
                gave_up,
                ..RoomPlayer::new(id.to_string(), id.to_string(), joined_order)
            },
        );
        room.race_player_ids.push(id.to_string());
    }

    trait TapMut: Sized {
        fn tap_mut(self, update: impl FnOnce(&mut Self)) -> Self;
    }

    impl<T> TapMut for T {
        fn tap_mut(mut self, update: impl FnOnce(&mut Self)) -> Self {
            update(&mut self);
            self
        }
    }
}
