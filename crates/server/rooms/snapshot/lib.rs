#![allow(clippy::missing_panics_doc)]

use queensgame_server_rooms_minesweeper::minesweeper_ranked_player_ids;
use queensgame_server_rooms_model::{Room, ServerMinesweeperGame, ServerNonogramGame};
use queensgame_shared_minesweeper::MinesweeperCellState;
use queensgame_shared_queens::{Puzzle, find_puzzle_by_id};
use queensgame_shared_room::{
    RoomGameKind, RoomMinesweeperCellSnapshot, RoomMinesweeperSnapshot, RoomNonogramSnapshot,
    RoomPhase, RoomPlayerSnapshot, RoomServerMessage, RoomSnapshot,
};
use std::collections::BTreeSet;
use tokio::sync::broadcast;

pub struct PendingRoomMessage {
    tx: broadcast::Sender<String>,
    message: String,
}

impl PendingRoomMessage {
    pub fn send(self) {
        let _ = self.tx.send(self.message);
    }
}

#[must_use]
pub fn pending_room_snapshot(room: &Room, puzzles: &[Puzzle]) -> PendingRoomMessage {
    PendingRoomMessage {
        tx: room.tx.clone(),
        message: room_snapshot_message(room, puzzles),
    }
}

#[must_use]
pub fn pending_room_server_message(room: &Room, message: &RoomServerMessage) -> PendingRoomMessage {
    PendingRoomMessage {
        tx: room.tx.clone(),
        message: serde_json::to_string(message).expect("room message must be serializable"),
    }
}

pub fn send_pending_room_message(message: Option<PendingRoomMessage>) {
    if let Some(message) = message {
        message.send();
    }
}

#[must_use]
pub fn room_snapshot_message(room: &Room, puzzles: &[Puzzle]) -> String {
    serde_json::to_string(&RoomServerMessage::Snapshot {
        snapshot: snapshot_room(room, puzzles),
    })
    .expect("room snapshot must be serializable")
}

#[must_use]
pub fn room_message_for_player(message: &str, player_id: &str) -> String {
    let Ok(RoomServerMessage::Snapshot { mut snapshot }) =
        serde_json::from_str::<RoomServerMessage>(message)
    else {
        return message.to_string();
    };

    for player in &mut snapshot.players {
        if player.id != player_id {
            player.minesweeper_flags.clear();
        }
    }

    serde_json::to_string(&RoomServerMessage::Snapshot { snapshot })
        .expect("personalized room snapshot must be serializable")
}

pub fn send_room_error_locked(room: &Room, message: &str) {
    let _ = room.tx.send(
        serde_json::to_string(&RoomServerMessage::Error {
            message: message.to_string(),
        })
        .expect("room error must be serializable"),
    );
}

#[must_use]
pub fn snapshot_room(room: &Room, puzzles: &[Puzzle]) -> RoomSnapshot {
    let puzzle = match (room.game_kind, &room.phase) {
        (
            RoomGameKind::Queens,
            queensgame_server_rooms_model::ServerRoomPhase::Racing { .. }
            | queensgame_server_rooms_model::ServerRoomPhase::Complete { .. },
        ) => room
            .active_puzzle_id
            .and_then(|id| find_puzzle_by_id(puzzles, id).cloned()),
        _ => None,
    };
    let minesweeper = if room.game_kind == RoomGameKind::Minesweeper {
        room.minesweeper.as_ref().map(room_minesweeper_snapshot)
    } else {
        None
    };
    let nonogram = if room.game_kind == RoomGameKind::Nonogram {
        room.nonogram.as_ref().map(room_nonogram_snapshot)
    } else {
        None
    };
    let winner_id = if matches!(room.phase.as_snapshot_phase(), RoomPhase::Complete { .. }) {
        match room.game_kind {
            RoomGameKind::Queens | RoomGameKind::Nonogram => room
                .players
                .values()
                .filter_map(|player| player.finish_ms.map(|finish_ms| (player, finish_ms)))
                .min_by_key(|(player, finish_ms)| (*finish_ms, player.joined_order))
                .map(|(player, _)| player.id.clone()),
            RoomGameKind::Minesweeper => minesweeper_ranked_player_ids(room).into_iter().next(),
        }
    } else {
        None
    };

    RoomSnapshot {
        slug: room.slug.clone(),
        game_kind: room.game_kind,
        phase: room.phase.as_snapshot_phase(),
        puzzle_choice: room.puzzle_choice.clone(),
        minesweeper_time_limit_seconds: room.minesweeper_time_limit_seconds,
        minesweeper_tile_rows: room.minesweeper_tile_rows,
        minesweeper_tile_cols: room.minesweeper_tile_cols,
        played_puzzle_ids: room.played_puzzle_ids.iter().copied().collect(),
        players: room
            .players
            .values()
            .map(|player| RoomPlayerSnapshot {
                id: player.id.clone(),
                name: player.name.clone(),
                ready: player.ready,
                connected: player.connected,
                finish_ms: player.finish_ms,
                gave_up: player.gave_up,
                medals: player.medals,
                recording: player.recording.clone(),
                mouse_recording: player.mouse_recording.clone(),
                minesweeper_score: player.minesweeper_score,
                minesweeper_eliminated: player.minesweeper_eliminated,
                minesweeper_last_score_ms: player.minesweeper_last_score_ms,
                minesweeper_flags: player.minesweeper_flags.iter().copied().collect(),
                pointer: player.pointer,
            })
            .collect(),
        puzzle,
        minesweeper,
        nonogram,
        winner_id,
    }
}

#[must_use]
pub fn room_minesweeper_snapshot(game: &ServerMinesweeperGame) -> RoomMinesweeperSnapshot {
    let starting_cells = game.starting_cells.iter().copied().collect::<BTreeSet<_>>();
    let cells = game
        .board
        .cells()
        .enumerate()
        .map(|(index, cell)| {
            let revealed = cell.state() == MinesweeperCellState::Revealed;
            let start = starting_cells.contains(&index);
            RoomMinesweeperCellSnapshot {
                revealed,
                mine: cell.mine(),
                detonated: cell.detonated(),
                start,
                adjacent_mines: Some(cell.adjacent_mines()),
                owner_id: game.cell_owners.get(&index).cloned(),
            }
        })
        .collect();

    RoomMinesweeperSnapshot {
        width: game.board.width(),
        height: game.board.height(),
        mines: game.board.mine_count(),
        starting_cells: game.starting_cells.clone(),
        cells,
    }
}

#[must_use]
pub fn room_nonogram_snapshot(game: &ServerNonogramGame) -> RoomNonogramSnapshot {
    RoomNonogramSnapshot {
        puzzle: game.puzzle.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use queensgame_server_rooms_minesweeper::{
        claim_minesweeper_revealed_cells, detonate_room_minesweeper_mine,
        prepare_room_minesweeper_game,
    };
    use queensgame_server_rooms_model::test_support::{add_test_player, test_room};
    use queensgame_shared_minesweeper::MinesweeperCell;
    use queensgame_shared_room::{RoomGameKind, RoomPuzzleChoice};

    #[test]
    fn minesweeper_snapshot_includes_hidden_board_data_for_optimistic_play() {
        let mut room = test_room(RoomPuzzleChoice::Random);
        room.game_kind = RoomGameKind::Minesweeper;
        add_test_player(&mut room, "ada", None, false, 1);
        prepare_room_minesweeper_game(&mut room);

        let snapshot =
            room_minesweeper_snapshot(room.minesweeper.as_ref().expect("minesweeper game"));

        assert!(
            snapshot
                .cells
                .iter()
                .any(|cell| !cell.revealed && cell.mine)
        );
        assert!(
            snapshot
                .cells
                .iter()
                .all(|cell| cell.adjacent_mines.is_some())
        );
    }

    #[test]
    fn minesweeper_snapshot_marks_number_owner_after_reveal() {
        let mut room = test_room(RoomPuzzleChoice::Random);
        room.game_kind = RoomGameKind::Minesweeper;
        add_test_player(&mut room, "ada", None, false, 1);
        prepare_room_minesweeper_game(&mut room);
        let game = room.minesweeper.as_mut().expect("minesweeper game");
        let index = game
            .board
            .cells()
            .position(|cell| !cell.mine() && cell.adjacent_mines() > 0)
            .expect("numbered safe cell");

        let revealed = game.board.reveal_safe_cells(index);
        assert_eq!(claim_minesweeper_revealed_cells(game, "ada", &revealed), 1);
        let snapshot = room_minesweeper_snapshot(game);

        assert_eq!(snapshot.cells[index].owner_id.as_deref(), Some("ada"));
    }

    #[test]
    fn minesweeper_snapshot_marks_detonated_mine_owner() {
        let mut room = test_room(RoomPuzzleChoice::Random);
        room.game_kind = RoomGameKind::Minesweeper;
        add_test_player(&mut room, "ada", None, false, 1);
        prepare_room_minesweeper_game(&mut room);
        let game = room.minesweeper.as_mut().expect("minesweeper game");
        let index = game
            .board
            .cells()
            .position(MinesweeperCell::mine)
            .expect("mine cell");

        detonate_room_minesweeper_mine(game, "ada", index);
        let snapshot = room_minesweeper_snapshot(game);

        assert!(snapshot.cells[index].revealed);
        assert!(snapshot.cells[index].detonated);
        assert_eq!(snapshot.cells[index].owner_id.as_deref(), Some("ada"));
    }

    #[test]
    fn personalized_room_snapshot_hides_other_minesweeper_flags() {
        let mut room = test_room(RoomPuzzleChoice::Random);
        room.game_kind = RoomGameKind::Minesweeper;
        add_test_player(&mut room, "ada", None, false, 1);
        add_test_player(&mut room, "bea", None, false, 2);
        room.players
            .get_mut("ada")
            .unwrap()
            .minesweeper_flags
            .extend([1, 2]);
        room.players
            .get_mut("bea")
            .unwrap()
            .minesweeper_flags
            .extend([3, 4]);

        let message = room_message_for_player(&room_snapshot_message(&room, &[]), "ada");
        let RoomServerMessage::Snapshot { snapshot } =
            serde_json::from_str(&message).expect("snapshot message")
        else {
            panic!("expected snapshot");
        };

        let ada = snapshot
            .players
            .iter()
            .find(|player| player.id == "ada")
            .expect("ada");
        let bea = snapshot
            .players
            .iter()
            .find(|player| player.id == "bea")
            .expect("bea");
        assert_eq!(ada.minesweeper_flags, vec![1, 2]);
        assert!(bea.minesweeper_flags.is_empty());
    }
}
