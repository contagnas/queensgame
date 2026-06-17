use queensgame_shared_queens::{GameBootstrap, Puzzle, PuzzleArchiveBootstrap, PuzzleNav};
use rand::Rng;
use std::collections::BTreeSet;

#[must_use]
pub fn find_puzzle_by_id(puzzles: &[Puzzle], id: usize) -> Option<&Puzzle> {
    puzzles.iter().find(|puzzle| puzzle.id == id)
}

#[must_use]
pub fn puzzle_bootstrap(puzzles: &[Puzzle], id: usize) -> Option<GameBootstrap> {
    let puzzle = find_puzzle_by_id(puzzles, id)?.clone();
    Some(GameBootstrap {
        puzzle,
        puzzle_nav: puzzle_nav(puzzles, id),
        total: puzzles.len(),
    })
}

#[must_use]
pub fn puzzle_archive_bootstrap(puzzles: &[Puzzle]) -> PuzzleArchiveBootstrap {
    PuzzleArchiveBootstrap {
        puzzle_nav: puzzle_nav(puzzles, 0),
        total: puzzles.len(),
    }
}

#[must_use]
pub fn puzzle_nav(puzzles: &[Puzzle], active_id: usize) -> Vec<PuzzleNav> {
    puzzles
        .iter()
        .map(|puzzle| PuzzleNav {
            id: puzzle.id,
            active: puzzle.id == active_id,
        })
        .collect()
}

#[must_use]
pub fn random_room_puzzle_id(
    puzzles: &[Puzzle],
    played_puzzle_ids: &BTreeSet<usize>,
) -> Option<usize> {
    let mut candidates = puzzles
        .iter()
        .filter(|puzzle| !played_puzzle_ids.contains(&puzzle.id))
        .map(|puzzle| puzzle.id)
        .collect::<Vec<_>>();

    if candidates.is_empty() {
        candidates = puzzles.iter().map(|puzzle| puzzle.id).collect();
    }
    if candidates.is_empty() {
        return None;
    }

    let index = rand::rng().random_range(0..candidates.len());
    candidates.get(index).copied()
}

#[must_use]
pub fn next_puzzle_id(puzzles: &[Puzzle], current_id: usize) -> Option<usize> {
    puzzles
        .iter()
        .map(|puzzle| puzzle.id)
        .filter(|id| *id > current_id)
        .min()
        .or_else(|| puzzles.iter().map(|puzzle| puzzle.id).min())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_puzzle(id: usize) -> Puzzle {
        Puzzle {
            id,
            size: 1,
            colors: vec!["#ffffff".to_string()],
            regions: vec![vec![0]],
        }
    }

    #[test]
    fn random_room_puzzle_id_uses_unplayed_puzzles_first() {
        let puzzles = vec![test_puzzle(1), test_puzzle(2), test_puzzle(3)];
        let played = BTreeSet::from([1, 2]);

        assert_eq!(random_room_puzzle_id(&puzzles, &played), Some(3));

        let played = BTreeSet::from([1, 2, 3]);
        let next = random_room_puzzle_id(&puzzles, &played);
        assert!(matches!(next, Some(1..=3)));
    }

    #[test]
    fn next_puzzle_id_wraps_to_first_puzzle() {
        let puzzles = vec![test_puzzle(10), test_puzzle(20)];

        assert_eq!(next_puzzle_id(&puzzles, 10), Some(20));
        assert_eq!(next_puzzle_id(&puzzles, 20), Some(10));
        assert_eq!(next_puzzle_id(&[], 20), None);
    }

    #[test]
    fn puzzle_bootstrap_marks_active_nav() {
        let puzzles = vec![test_puzzle(1), test_puzzle(2)];
        let bootstrap = puzzle_bootstrap(&puzzles, 2).expect("puzzle");

        assert_eq!(bootstrap.puzzle.id, 2);
        assert_eq!(bootstrap.total, 2);
        assert_eq!(
            bootstrap
                .puzzle_nav
                .iter()
                .map(|nav| (nav.id, nav.active))
                .collect::<Vec<_>>(),
            vec![(1, false), (2, true)]
        );
    }
}
