use queensgame_server_rooms_model::{Room, ServerMinesweeperGame, elapsed_millis_u64, now_ms};
use queensgame_shared_minesweeper::{
    MinesweeperBoard, MinesweeperCellState, build_room_minesweeper_board,
    clamp_room_minesweeper_tile_axis,
};
use queensgame_shared_room::{RoomGameKind, RoomPhase};
use queensgame_shared_room_minesweeper::room_minesweeper_chord_flags_from_cells;
use std::collections::{BTreeMap, BTreeSet};

pub fn prepare_room_minesweeper_game(room: &mut Room) {
    let tile_rows = clamp_room_minesweeper_tile_axis(room.minesweeper_tile_rows);
    let tile_cols = clamp_room_minesweeper_tile_axis(room.minesweeper_tile_cols);
    room.minesweeper_tile_rows = tile_rows;
    room.minesweeper_tile_cols = tile_cols;
    let tile_rows = u64::try_from(tile_rows).unwrap_or(0);
    let tile_cols = u64::try_from(tile_cols).unwrap_or(0);
    let seed = now_ms()
        ^ tile_rows.wrapping_mul(0x9e37_79b9_7f4a_7c15)
        ^ tile_cols.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    let built =
        build_room_minesweeper_board(room.minesweeper_tile_rows, room.minesweeper_tile_cols, seed);
    room.minesweeper = Some(ServerMinesweeperGame {
        board: built.board,
        starting_cells: built.starting_cells,
        cell_owners: BTreeMap::new(),
    });
}

pub fn reveal_room_minesweeper_starts(room: &mut Room) {
    let Some(game) = room.minesweeper.as_mut() else {
        return;
    };
    game.board.set_playing();
    for start in game.starting_cells.clone() {
        let _ = game.board.reveal_safe_cells(start);
    }
}

#[must_use]
pub fn active_minesweeper_elapsed_ms(room: &Room) -> Option<(u64, u64)> {
    if room.game_kind != RoomGameKind::Minesweeper {
        return None;
    }
    match &room.phase {
        queensgame_server_rooms_model::ServerRoomPhase::Racing {
            started_at_ms,
            started_at,
        } => Some((*started_at_ms, elapsed_millis_u64(*started_at))),
        queensgame_server_rooms_model::ServerRoomPhase::Lobby
        | queensgame_server_rooms_model::ServerRoomPhase::Countdown { .. }
        | queensgame_server_rooms_model::ServerRoomPhase::Complete { .. } => None,
    }
}

#[must_use]
pub fn room_minesweeper_player_can_act(room: &Room, player_id: &str) -> bool {
    room.race_player_ids.iter().any(|id| id == player_id)
        && room
            .players
            .get(player_id)
            .is_some_and(|player| !player.minesweeper_eliminated)
}

#[must_use]
pub fn room_minesweeper_player_can_point(room: &Room, player_id: &str) -> bool {
    let Some(player) = room.players.get(player_id) else {
        return false;
    };
    if !player.connected {
        return false;
    }
    match room.phase.as_snapshot_phase() {
        RoomPhase::Countdown { .. } => room.minesweeper.is_some(),
        RoomPhase::Racing { .. } => !player.minesweeper_eliminated,
        RoomPhase::Lobby | RoomPhase::Complete { .. } => false,
    }
}

#[must_use]
pub fn minesweeper_score_for_revealed_cells(board: &MinesweeperBoard, revealed: &[usize]) -> u32 {
    let revealed_cells = revealed
        .iter()
        .filter(|index| {
            board.cell(**index).is_some_and(|cell| {
                cell.state() == MinesweeperCellState::Revealed
                    && !cell.mine()
                    && cell.adjacent_mines() > 0
            })
        })
        .count();
    u32::try_from(revealed_cells).unwrap_or(u32::MAX)
}

pub fn claim_minesweeper_revealed_cells(
    game: &mut ServerMinesweeperGame,
    player_id: &str,
    revealed: &[usize],
) -> u32 {
    for index in revealed {
        if game.board.cell(*index).is_some_and(|cell| {
            cell.state() == MinesweeperCellState::Revealed
                && !cell.mine()
                && cell.adjacent_mines() > 0
        }) {
            game.cell_owners
                .entry(*index)
                .or_insert_with(|| player_id.to_string());
        }
    }
    minesweeper_score_for_revealed_cells(&game.board, revealed)
}

pub fn detonate_room_minesweeper_mine(
    game: &mut ServerMinesweeperGame,
    player_id: &str,
    index: usize,
) {
    if game.board.detonate_mine(index) {
        game.cell_owners
            .entry(index)
            .or_insert_with(|| player_id.to_string());
    }
}

#[must_use]
pub fn room_minesweeper_chord_flags(
    game: &ServerMinesweeperGame,
    own_flags: &BTreeSet<usize>,
    player_id: &str,
) -> BTreeSet<usize> {
    room_minesweeper_chord_flags_from_cells(
        own_flags,
        player_id,
        game.board.cells().enumerate().map(|(index, cell)| {
            (
                index,
                cell.state() == MinesweeperCellState::Revealed,
                cell.mine(),
                cell.detonated(),
                game.cell_owners.get(&index).map(String::as_str),
            )
        }),
    )
}

pub fn apply_minesweeper_player_result(
    room: &mut Room,
    player_id: &str,
    score_delta: u32,
    elapsed_ms: u64,
    eliminated: bool,
) {
    let Some(player) = room.players.get_mut(player_id) else {
        return;
    };
    if score_delta > 0 {
        player.minesweeper_score = player.minesweeper_score.saturating_add(score_delta);
        player.minesweeper_last_score_ms = Some(elapsed_ms);
    }
    if eliminated {
        player.minesweeper_eliminated = true;
        player.pointer = None;
    }
}

#[must_use]
pub fn room_minesweeper_should_complete(room: &Room) -> bool {
    let solved = room
        .minesweeper
        .as_ref()
        .is_some_and(|game| game.board.all_safe_cells_revealed());
    solved || room_all_minesweeper_players_eliminated(room)
}

#[must_use]
pub fn room_all_minesweeper_players_eliminated(room: &Room) -> bool {
    !room.race_player_ids.is_empty()
        && room.race_player_ids.iter().all(|player_id| {
            room.players
                .get(player_id)
                .is_none_or(|player| player.minesweeper_eliminated)
        })
}

pub fn award_room_minesweeper_medals(room: &mut Room) {
    for (place, player_id) in minesweeper_ranked_player_ids(room)
        .into_iter()
        .take(3)
        .enumerate()
    {
        let Some(player) = room.players.get_mut(&player_id) else {
            continue;
        };
        match place {
            0 => player.medals.gold += 1,
            1 => player.medals.silver += 1,
            2 => player.medals.bronze += 1,
            _ => {}
        }
    }
}

#[must_use]
pub fn minesweeper_ranked_player_ids(room: &Room) -> Vec<String> {
    let mut players = room
        .race_player_ids
        .iter()
        .filter_map(|player_id| room.players.get(player_id))
        .collect::<Vec<_>>();
    players.sort_by(|left, right| {
        right
            .minesweeper_score
            .cmp(&left.minesweeper_score)
            .then_with(|| {
                left.minesweeper_last_score_ms
                    .unwrap_or(u64::MAX)
                    .cmp(&right.minesweeper_last_score_ms.unwrap_or(u64::MAX))
            })
            .then_with(|| left.joined_order.cmp(&right.joined_order))
    });
    players
        .into_iter()
        .map(|player| player.id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use queensgame_server_rooms_model::test_support::{add_test_player, test_room};
    use queensgame_shared::{
        MINESWEEPER_EXPERT_HEIGHT, MINESWEEPER_EXPERT_MINES, MINESWEEPER_EXPERT_WIDTH,
    };
    use queensgame_shared_room::RoomPuzzleChoice;

    #[test]
    fn minesweeper_room_defaults_to_one_tile_even_with_many_players() {
        let mut room = test_room(RoomPuzzleChoice::Random);
        room.game_kind = RoomGameKind::Minesweeper;
        for index in 0..5 {
            add_test_player(&mut room, &format!("p{index}"), None, false, index);
        }

        prepare_room_minesweeper_game(&mut room);
        let game = room.minesweeper.as_ref().expect("minesweeper game");

        assert_eq!(room.minesweeper_tile_rows, 1);
        assert_eq!(room.minesweeper_tile_cols, 1);
        assert_eq!(game.board.width(), MINESWEEPER_EXPERT_WIDTH);
        assert_eq!(game.board.height(), MINESWEEPER_EXPERT_HEIGHT);
        assert_eq!(game.starting_cells.len(), 1);
    }

    #[test]
    fn minesweeper_room_uses_configured_tile_grid() {
        let mut room = test_room(RoomPuzzleChoice::Random);
        room.game_kind = RoomGameKind::Minesweeper;
        room.minesweeper_tile_rows = 3;
        room.minesweeper_tile_cols = 2;
        add_test_player(&mut room, "ada", None, false, 1);

        prepare_room_minesweeper_game(&mut room);
        let game = room.minesweeper.as_ref().expect("minesweeper game");

        assert_eq!(game.board.width(), MINESWEEPER_EXPERT_WIDTH * 2);
        assert_eq!(game.board.height(), MINESWEEPER_EXPERT_HEIGHT * 3);
        assert_eq!(game.board.mine_count(), MINESWEEPER_EXPERT_MINES * 6);
        assert_eq!(game.starting_cells.len(), 6);
    }

    #[test]
    fn minesweeper_chord_flags_include_other_players_detonated_mines() {
        let mut board = MinesweeperBoard::from_mines(2, 2, BTreeSet::from([1]), 7);
        assert_eq!(board.reveal_safe_cells(0), vec![0]);
        assert!(board.detonate_mine(1));
        let game = ServerMinesweeperGame {
            board,
            starting_cells: Vec::new(),
            cell_owners: BTreeMap::from([(1, "bea".to_string())]),
        };

        let ada_flags = room_minesweeper_chord_flags(&game, &BTreeSet::new(), "ada");
        let bea_flags = room_minesweeper_chord_flags(&game, &BTreeSet::new(), "bea");

        assert!(ada_flags.contains(&1));
        assert!(!bea_flags.contains(&1));
    }

    #[test]
    fn minesweeper_players_can_point_during_countdown_before_race_ids_exist() {
        let mut room = test_room(RoomPuzzleChoice::Random);
        room.game_kind = RoomGameKind::Minesweeper;
        add_test_player(&mut room, "ada", None, false, 1);
        prepare_room_minesweeper_game(&mut room);
        room.race_player_ids.clear();
        room.phase = queensgame_server_rooms_model::ServerRoomPhase::Countdown { starts_at_ms: 10 };

        assert!(room.race_player_ids.is_empty());
        assert!(room_minesweeper_player_can_point(&room, "ada"));
        assert!(!room_minesweeper_player_can_act(&room, "ada"));
    }
}
