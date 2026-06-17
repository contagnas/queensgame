use queensgame_shared_room::{
    RoomMinesweeperCellSnapshot, RoomMinesweeperSnapshot, RoomPlayerSnapshot, RoomSnapshot,
};
use std::collections::BTreeSet;

#[must_use]
pub fn room_minesweeper_chord_flags(
    cells: &[RoomMinesweeperCellSnapshot],
    own_flags: &BTreeSet<usize>,
    player_id: &str,
) -> BTreeSet<usize> {
    let mut flags = own_flags.clone();
    flags.extend(cells.iter().enumerate().filter_map(|(index, cell)| {
        (cell.revealed
            && cell.mine
            && cell.detonated
            && cell.owner_id.as_deref() != Some(player_id))
        .then_some(index)
    }));
    flags
}

pub fn merge_optimistic_room_minesweeper_snapshot(
    current: &RoomSnapshot,
    next: &mut RoomSnapshot,
    player_id: &str,
) {
    if !same_active_minesweeper_race(current, next) {
        return;
    }
    merge_optimistic_room_minesweeper_board(
        current.minesweeper.as_ref(),
        next.minesweeper.as_mut(),
    );
    merge_optimistic_room_minesweeper_player(&current.players, &mut next.players, player_id);
}

fn same_active_minesweeper_race(current: &RoomSnapshot, next: &RoomSnapshot) -> bool {
    current.game_kind == next.game_kind
        && current.game_kind == queensgame_shared_room::RoomGameKind::Minesweeper
        && matches!(
            (&current.phase, &next.phase),
            (
                queensgame_shared_room::RoomPhase::Racing {
                    started_at_ms: current_started_at_ms,
                },
                queensgame_shared_room::RoomPhase::Racing {
                    started_at_ms: next_started_at_ms,
                }
            ) if current_started_at_ms == next_started_at_ms
        )
}

fn merge_optimistic_room_minesweeper_board(
    current: Option<&RoomMinesweeperSnapshot>,
    next: Option<&mut RoomMinesweeperSnapshot>,
) {
    let (Some(current), Some(next)) = (current, next) else {
        return;
    };
    if !same_minesweeper_board(current, next) {
        return;
    }

    for (index, current_cell) in current.cells.iter().enumerate() {
        let Some(next_cell) = next.cells.get_mut(index) else {
            continue;
        };
        if current_cell.revealed && !next_cell.revealed {
            next_cell.revealed = true;
            next_cell.detonated = current_cell.detonated;
            next_cell.owner_id.clone_from(&current_cell.owner_id);
        }
    }
}

fn same_minesweeper_board(
    current: &RoomMinesweeperSnapshot,
    next: &RoomMinesweeperSnapshot,
) -> bool {
    current.width == next.width
        && current.height == next.height
        && current.mines == next.mines
        && current.starting_cells == next.starting_cells
        && current.cells.len() == next.cells.len()
}

fn merge_optimistic_room_minesweeper_player(
    current: &[RoomPlayerSnapshot],
    next: &mut [RoomPlayerSnapshot],
    player_id: &str,
) {
    let Some(current_player) = current.iter().find(|player| player.id == player_id) else {
        return;
    };
    let Some(next_player) = next.iter_mut().find(|player| player.id == player_id) else {
        return;
    };

    next_player.minesweeper_flags = current_player.minesweeper_flags.clone();
    if current_player.minesweeper_score > next_player.minesweeper_score {
        next_player.minesweeper_score = current_player.minesweeper_score;
        next_player.minesweeper_last_score_ms = current_player.minesweeper_last_score_ms;
    }
    next_player.minesweeper_eliminated |= current_player.minesweeper_eliminated;
}

#[cfg(test)]
mod tests {
    use super::*;
    use queensgame_shared_room::{
        RoomGameKind, RoomMedalCounts, RoomPhase, RoomPuzzleChoice, RoomRecording,
        RoomRecordingFrame,
    };

    #[test]
    fn chord_flags_include_other_players_detonated_mines() {
        let cells = vec![
            RoomMinesweeperCellSnapshot {
                revealed: true,
                mine: true,
                detonated: true,
                start: false,
                adjacent_mines: Some(0),
                owner_id: Some("bea".to_string()),
            },
            RoomMinesweeperCellSnapshot {
                revealed: false,
                mine: false,
                detonated: false,
                start: false,
                adjacent_mines: Some(1),
                owner_id: None,
            },
        ];

        let ada_flags = room_minesweeper_chord_flags(&cells, &BTreeSet::from([1]), "ada");
        let bea_flags = room_minesweeper_chord_flags(&cells, &BTreeSet::new(), "bea");

        assert_eq!(ada_flags, BTreeSet::from([0, 1]));
        assert!(bea_flags.is_empty());
    }

    #[test]
    fn merge_preserves_optimistic_reveals_from_older_server_snapshot() {
        let mut current = test_snapshot(vec![
            test_cell(false, 1, true, Some("ada")),
            test_cell(true, 0, false, None),
            test_cell(false, 1, false, None),
            test_cell(false, 1, false, None),
        ]);
        current.players[0].minesweeper_score = 1;
        current.players[0].minesweeper_last_score_ms = Some(500);
        current.players[0].minesweeper_flags = vec![1];
        let mut next = test_snapshot(vec![
            test_cell(false, 1, false, None),
            test_cell(true, 0, false, None),
            test_cell(false, 1, false, None),
            test_cell(false, 1, false, None),
        ]);

        merge_optimistic_room_minesweeper_snapshot(&current, &mut next, "ada");

        let board = next.minesweeper.as_ref().expect("board");
        assert!(board.cells[0].revealed);
        assert_eq!(board.cells[0].owner_id.as_deref(), Some("ada"));
        assert_eq!(next.players[0].minesweeper_score, 1);
        assert_eq!(next.players[0].minesweeper_last_score_ms, Some(500));
        assert_eq!(next.players[0].minesweeper_flags, vec![1]);
    }

    #[test]
    fn merge_keeps_server_revealed_cells_authoritative() {
        let current = test_snapshot(vec![
            test_cell(false, 1, true, Some("ada")),
            test_cell(true, 0, false, None),
            test_cell(false, 1, false, None),
            test_cell(false, 1, false, None),
        ]);
        let mut next = test_snapshot(vec![
            test_cell(false, 1, true, Some("bea")),
            test_cell(true, 0, false, None),
            test_cell(false, 1, false, None),
            test_cell(false, 1, false, None),
        ]);
        next.players[0].minesweeper_score = 0;

        merge_optimistic_room_minesweeper_snapshot(&current, &mut next, "ada");

        let board = next.minesweeper.as_ref().expect("board");
        assert_eq!(board.cells[0].owner_id.as_deref(), Some("bea"));
        assert_eq!(next.players[0].minesweeper_score, 0);
    }

    #[test]
    fn merge_does_not_carry_reveals_across_races() {
        let current = test_snapshot(vec![
            test_cell(false, 1, true, Some("ada")),
            test_cell(true, 0, false, None),
            test_cell(false, 1, false, None),
            test_cell(false, 1, false, None),
        ]);
        let mut next = test_snapshot(vec![
            test_cell(false, 1, false, None),
            test_cell(true, 0, false, None),
            test_cell(false, 1, false, None),
            test_cell(false, 1, false, None),
        ]);
        next.phase = RoomPhase::Racing { started_at_ms: 20 };

        merge_optimistic_room_minesweeper_snapshot(&current, &mut next, "ada");

        let board = next.minesweeper.as_ref().expect("board");
        assert!(!board.cells[0].revealed);
    }

    fn test_snapshot(cells: Vec<RoomMinesweeperCellSnapshot>) -> RoomSnapshot {
        RoomSnapshot {
            slug: "ROOMTEST".to_string(),
            game_kind: RoomGameKind::Minesweeper,
            phase: RoomPhase::Racing { started_at_ms: 10 },
            puzzle_choice: RoomPuzzleChoice::Random,
            minesweeper_time_limit_seconds: 99,
            minesweeper_tile_rows: 1,
            minesweeper_tile_cols: 1,
            played_puzzle_ids: Vec::new(),
            players: vec![test_player("ada")],
            puzzle: None,
            minesweeper: Some(RoomMinesweeperSnapshot {
                width: 2,
                height: 2,
                mines: cells.iter().filter(|cell| cell.mine).count(),
                starting_cells: Vec::new(),
                cells,
            }),
            winner_id: None,
        }
    }

    fn test_player(id: &str) -> RoomPlayerSnapshot {
        RoomPlayerSnapshot {
            id: id.to_string(),
            name: id.to_string(),
            ready: false,
            connected: true,
            finish_ms: None,
            gave_up: false,
            medals: RoomMedalCounts::default(),
            recording: Some(RoomRecording {
                frames: vec![RoomRecordingFrame {
                    elapsed_ms: 1_500,
                    states: Vec::new(),
                }],
            }),
            mouse_recording: None,
            minesweeper_score: 0,
            minesweeper_eliminated: false,
            minesweeper_last_score_ms: None,
            minesweeper_flags: Vec::new(),
            pointer: None,
        }
    }

    fn test_cell(
        mine: bool,
        adjacent_mines: u8,
        revealed: bool,
        owner_id: Option<&str>,
    ) -> RoomMinesweeperCellSnapshot {
        RoomMinesweeperCellSnapshot {
            revealed,
            mine,
            detonated: false,
            start: false,
            adjacent_mines: Some(adjacent_mines),
            owner_id: owner_id.map(str::to_string),
        }
    }
}
