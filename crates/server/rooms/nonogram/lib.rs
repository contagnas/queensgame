use queensgame_server_rooms_model::{Room, ServerNonogramGame, now_ms};
use queensgame_shared_nonogram::{
    NONOGRAM_DEFAULT_HEIGHT, NONOGRAM_DEFAULT_WIDTH, NonogramCellState, NonogramPuzzle,
    generate_nonogram_puzzle, validate_nonogram_solution,
};

pub fn prepare_room_nonogram_game(room: &mut Room) {
    let seed = now_ms() ^ 0xd6e8_feb8_6659_fd93;
    room.nonogram = Some(ServerNonogramGame {
        puzzle: generate_nonogram_puzzle(NONOGRAM_DEFAULT_WIDTH, NONOGRAM_DEFAULT_HEIGHT, seed),
    });
}

#[must_use]
pub fn nonogram_filled_cells_are_complete(puzzle: &NonogramPuzzle, filled: &[usize]) -> bool {
    let mut cells = vec![NonogramCellState::Hidden; puzzle.cell_count()];
    for index in filled {
        let Some(cell) = cells.get_mut(*index) else {
            return false;
        };
        if *cell == NonogramCellState::Filled {
            return false;
        }
        *cell = NonogramCellState::Filled;
    }
    validate_nonogram_solution(puzzle, &cells).complete
}

pub fn award_room_nonogram_medals(room: &mut Room) {
    let mut placements = room
        .race_player_ids
        .iter()
        .filter_map(|player_id| {
            let player = room.players.get(player_id)?;
            player
                .finish_ms
                .map(|finish_ms| (player.id.clone(), finish_ms, player.joined_order))
        })
        .collect::<Vec<_>>();
    placements.sort_by_key(|(_, finish_ms, joined_order)| (*finish_ms, *joined_order));

    for (place, (player_id, _, _)) in placements.into_iter().take(3).enumerate() {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonogram_filled_cells_must_match_solution_exactly() {
        let puzzle = queensgame_shared_nonogram::build_nonogram_puzzle(
            2,
            2,
            1,
            vec![true, false, false, true],
        );

        assert!(nonogram_filled_cells_are_complete(&puzzle, &[0, 3]));
        assert!(!nonogram_filled_cells_are_complete(&puzzle, &[0, 1, 3]));
        assert!(!nonogram_filled_cells_are_complete(&puzzle, &[0]));
    }
}
