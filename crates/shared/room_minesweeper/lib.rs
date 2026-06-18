use queensgame_shared_room::{
    RoomGameKind, RoomMinesweeperCellSnapshot, RoomMinesweeperSnapshot, RoomPhase,
    RoomPlayerSnapshot, RoomSnapshot,
};
use std::collections::{BTreeSet, VecDeque};

#[must_use]
pub fn room_minesweeper_can_act(
    phase: &RoomPhase,
    players: &[RoomPlayerSnapshot],
    player_id: &str,
) -> bool {
    matches!(phase, RoomPhase::Racing { .. })
        && players
            .iter()
            .find(|player| player.id == player_id)
            .is_some_and(|player| player.connected && !player.minesweeper_eliminated)
}

#[must_use]
pub fn room_minesweeper_can_point(
    phase: &RoomPhase,
    players: &[RoomPlayerSnapshot],
    player_id: &str,
) -> bool {
    if !matches!(
        phase,
        RoomPhase::Countdown { .. } | RoomPhase::Racing { .. }
    ) {
        return false;
    }
    players
        .iter()
        .find(|player| player.id == player_id)
        .is_some_and(|player| {
            player.connected
                && (!matches!(phase, RoomPhase::Racing { .. }) || !player.minesweeper_eliminated)
        })
}

#[must_use]
pub fn room_minesweeper_own_flags(
    players: &[RoomPlayerSnapshot],
    player_id: &str,
) -> BTreeSet<usize> {
    players
        .iter()
        .find(|player| player.id == player_id)
        .map(|player| player.minesweeper_flags.iter().copied().collect())
        .unwrap_or_default()
}

#[must_use]
pub fn room_minesweeper_chord_flags(
    cells: &[RoomMinesweeperCellSnapshot],
    own_flags: &BTreeSet<usize>,
    player_id: &str,
) -> BTreeSet<usize> {
    room_minesweeper_chord_flags_from_cells(
        own_flags,
        player_id,
        cells.iter().enumerate().map(|(index, cell)| {
            (
                index,
                cell.revealed,
                cell.mine,
                cell.detonated,
                cell.owner_id.as_deref(),
            )
        }),
    )
}

#[must_use]
pub fn room_minesweeper_chord_flags_from_cells<'a>(
    own_flags: &BTreeSet<usize>,
    player_id: &str,
    cells: impl IntoIterator<Item = (usize, bool, bool, bool, Option<&'a str>)>,
) -> BTreeSet<usize> {
    let mut flags = own_flags.clone();
    flags.extend(
        cells
            .into_iter()
            .filter_map(|(index, revealed, mine, detonated, owner_id)| {
                (revealed && mine && detonated && owner_id != Some(player_id)).then_some(index)
            }),
    );
    flags
}

#[must_use]
pub fn room_minesweeper_chord_target(
    board: &RoomMinesweeperSnapshot,
    index: usize,
) -> Option<usize> {
    let cell = board.cells.get(index)?;
    (cell.revealed && !cell.mine && cell.adjacent_mines.unwrap_or_default() > 0).then_some(index)
}

#[must_use]
pub fn room_minesweeper_pressed_neighbors(
    board: &RoomMinesweeperSnapshot,
    index: usize,
    own_flags: &BTreeSet<usize>,
) -> BTreeSet<usize> {
    if room_minesweeper_chord_target(board, index).is_none() {
        return BTreeSet::new();
    }
    room_minesweeper_neighbors(board, index)
        .into_iter()
        .filter(|neighbor| {
            board
                .cells
                .get(*neighbor)
                .is_some_and(|cell| !cell.revealed && !own_flags.contains(neighbor))
        })
        .collect()
}

#[must_use]
pub fn room_minesweeper_neighbors(board: &RoomMinesweeperSnapshot, index: usize) -> Vec<usize> {
    if board.width == 0 || board.height == 0 || index >= board.cells.len() {
        return Vec::new();
    }
    let row = index / board.width;
    let col = index % board.width;
    let row_start = row.saturating_sub(1);
    let row_end = (row + 1).min(board.height.saturating_sub(1));
    let col_start = col.saturating_sub(1);
    let col_end = (col + 1).min(board.width.saturating_sub(1));
    let mut neighbors = Vec::new();
    for next_row in row_start..=row_end {
        for next_col in col_start..=col_end {
            if next_row == row && next_col == col {
                continue;
            }
            neighbors.push(next_row * board.width + next_col);
        }
    }
    neighbors
}

pub fn optimistic_room_minesweeper_reveal_snapshot(
    snapshot: &mut RoomSnapshot,
    player_id: &str,
    index: usize,
    now_ms: impl FnOnce() -> u64,
) {
    if !room_minesweeper_can_act(&snapshot.phase, &snapshot.players, player_id) {
        return;
    }
    let own_flags = room_minesweeper_own_flags(&snapshot.players, player_id);
    if own_flags.contains(&index) {
        return;
    }
    let (score_delta, eliminated) = {
        let Some(board) = snapshot.minesweeper.as_mut() else {
            return;
        };
        optimistic_room_minesweeper_reveal_board(board, index, &own_flags, player_id)
    };
    optimistic_room_minesweeper_apply_result(snapshot, player_id, score_delta, eliminated, now_ms);
}

pub fn optimistic_room_minesweeper_toggle_flag_snapshot(
    snapshot: &mut RoomSnapshot,
    player_id: &str,
    index: usize,
) {
    if !room_minesweeper_can_act(&snapshot.phase, &snapshot.players, player_id) {
        return;
    }
    let Some(board) = snapshot.minesweeper.as_ref() else {
        return;
    };
    if board.cells.get(index).is_none_or(|cell| cell.revealed) {
        return;
    }
    let Some(player) = snapshot
        .players
        .iter_mut()
        .find(|player| player.id == player_id)
    else {
        return;
    };
    if let Some(position) = player
        .minesweeper_flags
        .iter()
        .position(|flag| *flag == index)
    {
        player.minesweeper_flags.remove(position);
    } else {
        player.minesweeper_flags.push(index);
        player.minesweeper_flags.sort_unstable();
    }
}

pub fn optimistic_room_minesweeper_chord_snapshot(
    snapshot: &mut RoomSnapshot,
    player_id: &str,
    index: usize,
    now_ms: impl FnOnce() -> u64,
) {
    if !room_minesweeper_can_act(&snapshot.phase, &snapshot.players, player_id) {
        return;
    }
    let own_flags = room_minesweeper_own_flags(&snapshot.players, player_id);
    let (score_delta, eliminated) = {
        let Some(board) = snapshot.minesweeper.as_mut() else {
            return;
        };
        optimistic_room_minesweeper_chord_board(board, index, &own_flags, player_id)
    };
    optimistic_room_minesweeper_apply_result(snapshot, player_id, score_delta, eliminated, now_ms);
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

fn optimistic_room_minesweeper_reveal_board(
    board: &mut RoomMinesweeperSnapshot,
    index: usize,
    own_flags: &BTreeSet<usize>,
    player_id: &str,
) -> (u32, bool) {
    let Some(cell) = board.cells.get_mut(index) else {
        return (0, false);
    };
    if cell.revealed || own_flags.contains(&index) {
        return (0, false);
    }
    if cell.mine {
        cell.revealed = true;
        cell.detonated = true;
        cell.owner_id = Some(player_id.to_string());
        return (0, true);
    }

    let revealed = optimistic_room_minesweeper_reveal_safe_cells(board, index, own_flags);
    (
        optimistic_room_minesweeper_score_and_claim_revealed_cells(board, &revealed, player_id),
        false,
    )
}

fn optimistic_room_minesweeper_chord_board(
    board: &mut RoomMinesweeperSnapshot,
    index: usize,
    own_flags: &BTreeSet<usize>,
    player_id: &str,
) -> (u32, bool) {
    let Some(cell) = board.cells.get(index) else {
        return (0, false);
    };
    if !cell.revealed || cell.mine || cell.adjacent_mines.unwrap_or_default() == 0 {
        return (0, false);
    }
    let neighbors = room_minesweeper_neighbors(board, index);
    let chord_flags = room_minesweeper_chord_flags(&board.cells, own_flags, player_id);
    let flagged_neighbors = neighbors
        .iter()
        .filter(|neighbor| chord_flags.contains(neighbor))
        .count();
    if flagged_neighbors != usize::from(cell.adjacent_mines.unwrap_or_default()) {
        return (0, false);
    }

    let targets = neighbors
        .into_iter()
        .filter(|neighbor| !chord_flags.contains(neighbor))
        .filter(|neighbor| {
            board
                .cells
                .get(*neighbor)
                .is_some_and(|cell| !cell.revealed)
        })
        .collect::<Vec<_>>();
    if let Some(mine) = targets
        .iter()
        .copied()
        .find(|target| board.cells[*target].mine)
    {
        if let Some(cell) = board.cells.get_mut(mine) {
            cell.revealed = true;
            cell.detonated = true;
            cell.owner_id = Some(player_id.to_string());
        }
        return (0, true);
    }

    let mut score_delta = 0u32;
    for target in targets {
        let revealed = optimistic_room_minesweeper_reveal_safe_cells(board, target, own_flags);
        score_delta = score_delta.saturating_add(
            optimistic_room_minesweeper_score_and_claim_revealed_cells(board, &revealed, player_id),
        );
    }
    (score_delta, false)
}

fn optimistic_room_minesweeper_reveal_safe_cells(
    board: &mut RoomMinesweeperSnapshot,
    index: usize,
    own_flags: &BTreeSet<usize>,
) -> Vec<usize> {
    if index >= board.cells.len() {
        return Vec::new();
    }

    let mut revealed = Vec::new();
    let mut queued = BTreeSet::from([index]);
    let mut queue = VecDeque::from([index]);
    while let Some(next) = queue.pop_front() {
        if own_flags.contains(&next) {
            continue;
        }
        let adjacent_mines = {
            let Some(cell) = board.cells.get_mut(next) else {
                continue;
            };
            if cell.revealed || cell.mine {
                continue;
            }
            cell.revealed = true;
            cell.adjacent_mines.unwrap_or_default()
        };
        revealed.push(next);

        if adjacent_mines == 0 {
            for neighbor in room_minesweeper_neighbors(board, next) {
                if queued.contains(&neighbor) || own_flags.contains(&neighbor) {
                    continue;
                }
                let should_queue = board
                    .cells
                    .get(neighbor)
                    .is_some_and(|cell| !cell.revealed && !cell.mine);
                if should_queue {
                    queued.insert(neighbor);
                    queue.push_back(neighbor);
                }
            }
        }
    }
    revealed
}

fn optimistic_room_minesweeper_score_and_claim_revealed_cells(
    board: &mut RoomMinesweeperSnapshot,
    revealed: &[usize],
    player_id: &str,
) -> u32 {
    let mut score = 0;
    for index in revealed {
        let Some(cell) = board.cells.get_mut(*index) else {
            continue;
        };
        if !cell.mine && cell.adjacent_mines.unwrap_or_default() > 0 {
            if cell.owner_id.is_none() {
                cell.owner_id = Some(player_id.to_string());
            }
            score += 1;
        }
    }
    score
}

fn optimistic_room_minesweeper_apply_result(
    snapshot: &mut RoomSnapshot,
    player_id: &str,
    score_delta: u32,
    eliminated: bool,
    now_ms: impl FnOnce() -> u64,
) {
    if score_delta == 0 && !eliminated {
        return;
    }
    let score_elapsed_ms = room_minesweeper_elapsed_ms(&snapshot.phase, now_ms);
    let Some(player) = snapshot
        .players
        .iter_mut()
        .find(|player| player.id == player_id)
    else {
        return;
    };
    if score_delta > 0 {
        player.minesweeper_score = player.minesweeper_score.saturating_add(score_delta);
        player.minesweeper_last_score_ms = score_elapsed_ms;
    }
    if eliminated {
        player.minesweeper_eliminated = true;
    }
}

fn room_minesweeper_elapsed_ms(phase: &RoomPhase, now_ms: impl FnOnce() -> u64) -> Option<u64> {
    let RoomPhase::Racing { started_at_ms } = phase else {
        return None;
    };
    Some(now_ms().saturating_sub(*started_at_ms))
}

fn same_active_minesweeper_race(current: &RoomSnapshot, next: &RoomSnapshot) -> bool {
    current.game_kind == next.game_kind
        && current.game_kind == RoomGameKind::Minesweeper
        && matches!(
            (&current.phase, &next.phase),
            (
                RoomPhase::Racing {
                    started_at_ms: current_started_at_ms,
                },
                RoomPhase::Racing {
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

    next_player
        .minesweeper_flags
        .clone_from(&current_player.minesweeper_flags);
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
        RoomMedalCounts, RoomMouseRecording, RoomPuzzleChoice, RoomRecording, RoomRecordingFrame,
    };

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
            mouse_recording: None::<RoomMouseRecording>,
            minesweeper_score: 0,
            minesweeper_eliminated: false,
            minesweeper_last_score_ms: None,
            minesweeper_flags: Vec::new(),
            pointer: None,
        }
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
            nonogram: None,
            winner_id: None,
        }
    }

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
    fn optimistic_reveal_opens_safe_area_and_scores_numbers() {
        let mut snapshot = RoomSnapshot {
            minesweeper: Some(RoomMinesweeperSnapshot {
                width: 3,
                height: 3,
                mines: 1,
                starting_cells: Vec::new(),
                cells: vec![
                    test_cell(false, 0, false, None),
                    test_cell(false, 0, false, None),
                    test_cell(false, 0, false, None),
                    test_cell(false, 0, false, None),
                    test_cell(false, 1, false, None),
                    test_cell(false, 1, false, None),
                    test_cell(false, 0, false, None),
                    test_cell(false, 1, false, None),
                    test_cell(true, 0, false, None),
                ],
            }),
            ..test_snapshot(Vec::new())
        };

        optimistic_room_minesweeper_reveal_snapshot(&mut snapshot, "ada", 0, || 110);

        let board = snapshot.minesweeper.as_ref().expect("board");
        assert!(board.cells[..8].iter().all(|cell| cell.revealed));
        assert!(!board.cells[8].revealed);
        assert_eq!(snapshot.players[0].minesweeper_score, 3);
        assert_eq!(snapshot.players[0].minesweeper_last_score_ms, Some(100));
        assert_eq!(board.cells[4].owner_id.as_deref(), Some("ada"));
        assert_eq!(board.cells[0].owner_id, None);
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
}
