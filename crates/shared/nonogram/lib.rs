#![allow(clippy::missing_panics_doc)]

use serde::{Deserialize, Serialize};

pub const NONOGRAM_DEFAULT_WIDTH: usize = 10;
pub const NONOGRAM_DEFAULT_HEIGHT: usize = 10;
pub const NONOGRAM_MIN_AXIS: usize = 5;
pub const NONOGRAM_MAX_AXIS: usize = 15;
const NONOGRAM_GENERATION_ATTEMPTS: usize = 512;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct NonogramBootstrap {
    pub width: usize,
    pub height: usize,
}

impl Default for NonogramBootstrap {
    fn default() -> Self {
        Self {
            width: NONOGRAM_DEFAULT_WIDTH,
            height: NONOGRAM_DEFAULT_HEIGHT,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct NonogramPuzzle {
    pub width: usize,
    pub height: usize,
    pub seed: u64,
    pub row_clues: Vec<Vec<usize>>,
    pub col_clues: Vec<Vec<usize>>,
    pub solution: Vec<bool>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub enum NonogramCellState {
    Hidden,
    Filled,
    Crossed,
}

impl NonogramCellState {
    #[must_use]
    pub const fn is_filled(self) -> bool {
        matches!(self, Self::Filled)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct NonogramValidation {
    pub complete: bool,
    pub filled_count: usize,
    pub expected_filled: usize,
    pub incorrect_cells: Vec<usize>,
}

impl NonogramPuzzle {
    #[must_use]
    pub const fn cell_count(&self) -> usize {
        self.width * self.height
    }

    #[must_use]
    pub const fn index(&self, row: usize, col: usize) -> Option<usize> {
        if row < self.height && col < self.width {
            Some(row * self.width + col)
        } else {
            None
        }
    }
}

#[must_use]
pub fn clamp_nonogram_axis(axis: usize) -> usize {
    axis.clamp(NONOGRAM_MIN_AXIS, NONOGRAM_MAX_AXIS)
}

#[must_use]
pub fn generate_nonogram_puzzle(width: usize, height: usize, seed: u64) -> NonogramPuzzle {
    let width = clamp_nonogram_axis(width);
    let height = clamp_nonogram_axis(height);
    let mut candidate_seed = seed.max(1);
    let mut best_puzzle = None;
    let mut best_score = 0;
    let target_score = target_nonogram_interest_score(width, height);

    for _ in 0..NONOGRAM_GENERATION_ATTEMPTS {
        let solution = generated_solution(width, height, candidate_seed);
        if solution_is_usable(&solution, width, height)
            && solution_has_no_trivial_lines(width, height, &solution)
        {
            let puzzle = build_nonogram_puzzle(width, height, candidate_seed, solution);
            debug_assert!(puzzle_has_no_trivial_clues(&puzzle));
            if nonogram_has_human_solution(&puzzle) {
                let score = nonogram_interest_score(&puzzle);
                if score >= target_score {
                    return puzzle;
                }
                if best_puzzle.is_none() || score > best_score {
                    best_score = score;
                    best_puzzle = Some(puzzle);
                }
            }
        }
        candidate_seed = next_seed(candidate_seed);
    }

    if let Some(puzzle) = best_puzzle {
        return puzzle;
    }

    let puzzle = build_nonogram_puzzle(
        width,
        height,
        candidate_seed,
        fallback_solution(width, height, candidate_seed),
    );
    debug_assert!(puzzle_has_no_trivial_clues(&puzzle));
    puzzle
}

const fn target_nonogram_interest_score(width: usize, height: usize) -> usize {
    ((width + height) * 3) / 5
}

fn nonogram_interest_score(puzzle: &NonogramPuzzle) -> usize {
    puzzle
        .row_clues
        .iter()
        .chain(&puzzle.col_clues)
        .filter(|clues| clues_have_gap(clues))
        .count()
}

fn puzzle_has_no_trivial_clues(puzzle: &NonogramPuzzle) -> bool {
    puzzle
        .row_clues
        .iter()
        .all(|clues| line_clues_are_nontrivial(clues, puzzle.width))
        && puzzle
            .col_clues
            .iter()
            .all(|clues| line_clues_are_nontrivial(clues, puzzle.height))
}

fn line_clues_are_nontrivial(clues: &[usize], length: usize) -> bool {
    !(clues.is_empty() || clues.len() == 1 && clues[0] == length)
}

const fn clues_have_gap(clues: &[usize]) -> bool {
    clues.len() > 1
}

#[must_use]
pub fn build_nonogram_puzzle(
    width: usize,
    height: usize,
    seed: u64,
    solution: Vec<bool>,
) -> NonogramPuzzle {
    assert_eq!(solution.len(), width * height, "invalid solution size");
    let row_clues = (0..height)
        .map(|row| {
            line_clues((0..width).map(|col| {
                let index = row * width + col;
                solution[index]
            }))
        })
        .collect();
    let col_clues = (0..width)
        .map(|col| {
            line_clues((0..height).map(|row| {
                let index = row * width + col;
                solution[index]
            }))
        })
        .collect();

    NonogramPuzzle {
        width,
        height,
        seed,
        row_clues,
        col_clues,
        solution,
    }
}

#[must_use]
pub fn validate_nonogram_solution(
    puzzle: &NonogramPuzzle,
    cells: &[NonogramCellState],
) -> NonogramValidation {
    let expected_filled = puzzle.solution.iter().filter(|filled| **filled).count();
    let filled_count = cells.iter().filter(|cell| cell.is_filled()).count();
    let mut incorrect_cells = Vec::new();

    if cells.len() != puzzle.cell_count() {
        return NonogramValidation {
            complete: false,
            filled_count,
            expected_filled,
            incorrect_cells: (0..puzzle.cell_count()).collect(),
        };
    }

    for (index, (state, solution)) in cells.iter().zip(&puzzle.solution).enumerate() {
        let filled = state.is_filled();
        if filled != *solution {
            incorrect_cells.push(index);
        }
    }

    NonogramValidation {
        complete: incorrect_cells.is_empty(),
        filled_count,
        expected_filled,
        incorrect_cells,
    }
}

#[must_use]
pub fn nonogram_has_human_solution(puzzle: &NonogramPuzzle) -> bool {
    human_solve_nonogram(puzzle)
        .as_ref()
        .is_some_and(|solution| solution == &puzzle.solution)
}

#[must_use]
pub fn human_solve_nonogram(puzzle: &NonogramPuzzle) -> Option<Vec<bool>> {
    let mut known = vec![None; puzzle.cell_count()];

    loop {
        let mut progress = false;

        for row in 0..puzzle.height {
            let indexes = (0..puzzle.width)
                .map(|col| row * puzzle.width + col)
                .collect::<Vec<_>>();
            progress |= apply_line_deductions(&indexes, &puzzle.row_clues[row], &mut known)?;
        }

        for col in 0..puzzle.width {
            let indexes = (0..puzzle.height)
                .map(|row| row * puzzle.width + col)
                .collect::<Vec<_>>();
            progress |= apply_line_deductions(&indexes, &puzzle.col_clues[col], &mut known)?;
        }

        if known.iter().all(Option::is_some) {
            return known.into_iter().collect();
        }
        if !progress {
            return None;
        }
    }
}

fn apply_line_deductions(
    indexes: &[usize],
    clues: &[usize],
    known: &mut [Option<bool>],
) -> Option<bool> {
    let current = indexes
        .iter()
        .map(|index| known[*index])
        .collect::<Vec<_>>();
    let deductions = line_deductions(indexes.len(), clues, &current)?;
    let mut progress = false;
    for (line_index, deduction) in deductions.into_iter().enumerate() {
        let Some(value) = deduction else {
            continue;
        };
        let index = indexes[line_index];
        if known[index].is_none() {
            known[index] = Some(value);
            progress = true;
        } else if known[index] != Some(value) {
            return None;
        }
    }
    Some(progress)
}

fn line_deductions(
    length: usize,
    clues: &[usize],
    known: &[Option<bool>],
) -> Option<Vec<Option<bool>>> {
    let possibilities = line_possibilities(length, clues, known);
    let first = possibilities.first()?;
    let mut deductions = Vec::with_capacity(length);
    for index in 0..length {
        let value = first[index];
        deductions.push(
            possibilities
                .iter()
                .all(|possibility| possibility[index] == value)
                .then_some(value),
        );
    }
    Some(deductions)
}

fn line_possibilities(length: usize, clues: &[usize], known: &[Option<bool>]) -> Vec<Vec<bool>> {
    if clues.is_empty() {
        let line = vec![false; length];
        return line_is_compatible(&line, known)
            .then_some(line)
            .into_iter()
            .collect();
    }

    let mut possibilities = Vec::new();
    place_line_run(
        length,
        clues,
        known,
        0,
        0,
        &vec![false; length],
        &mut possibilities,
    );
    possibilities
}

fn place_line_run(
    length: usize,
    clues: &[usize],
    known: &[Option<bool>],
    clue_index: usize,
    min_start: usize,
    line: &[bool],
    possibilities: &mut Vec<Vec<bool>>,
) {
    let run = clues[clue_index];
    let remaining = minimum_remaining_width(&clues[(clue_index + 1)..]);
    if run + remaining > length {
        return;
    }
    let max_start = length - run - remaining;
    for start in min_start..=max_start {
        let end = start + run;
        if !false_span_is_compatible(start.saturating_sub(min_start), min_start, known) {
            continue;
        }
        if !true_span_is_compatible(run, start, known) {
            continue;
        }
        let mut next_line = line.to_vec();
        for cell in &mut next_line[start..end] {
            *cell = true;
        }

        if clue_index + 1 == clues.len() {
            if false_span_is_compatible(length - end, end, known)
                && line_is_compatible(&next_line, known)
            {
                possibilities.push(next_line);
            }
        } else if end < length
            && known[end] != Some(true)
            && false_span_is_compatible(start.saturating_sub(min_start), min_start, known)
        {
            place_line_run(
                length,
                clues,
                known,
                clue_index + 1,
                end + 1,
                &next_line,
                possibilities,
            );
        }
    }
}

fn minimum_remaining_width(clues: &[usize]) -> usize {
    if clues.is_empty() {
        0
    } else {
        clues.iter().sum::<usize>() + clues.len()
    }
}

fn false_span_is_compatible(length: usize, start: usize, known: &[Option<bool>]) -> bool {
    known
        .iter()
        .skip(start)
        .take(length)
        .all(|value| *value != Some(true))
}

fn true_span_is_compatible(length: usize, start: usize, known: &[Option<bool>]) -> bool {
    known
        .iter()
        .skip(start)
        .take(length)
        .all(|value| *value != Some(false))
}

fn line_is_compatible(line: &[bool], known: &[Option<bool>]) -> bool {
    line.iter()
        .zip(known)
        .all(|(value, known)| known.is_none_or(|known| known == *value))
}

fn line_clues(line: impl IntoIterator<Item = bool>) -> Vec<usize> {
    let mut clues = Vec::new();
    let mut run = 0usize;
    for filled in line {
        if filled {
            run += 1;
        } else if run > 0 {
            clues.push(run);
            run = 0;
        }
    }
    if run > 0 {
        clues.push(run);
    }
    clues
}

fn generated_solution(width: usize, height: usize, seed: u64) -> Vec<bool> {
    let mut rng = SeededRng::new(seed);
    let field = random_nonogram_field(width, height, &mut rng);
    let smoothed = smooth_nonogram_field(width, height, &field);
    threshold_nonogram_field(&smoothed, &mut rng)
}

fn random_nonogram_field(width: usize, height: usize, rng: &mut SeededRng) -> Vec<u32> {
    (0..(width * height)).map(|_| rng.next_u32()).collect()
}

fn smooth_nonogram_field(width: usize, height: usize, field: &[u32]) -> Vec<u32> {
    if width == 0 || height == 0 {
        return Vec::new();
    }

    let mut smoothed = Vec::with_capacity(field.len());
    for row in 0..height {
        for col in 0..width {
            let row_start = row.saturating_sub(1);
            let row_end = (row + 2).min(height);
            let col_start = col.saturating_sub(1);
            let col_end = (col + 2).min(width);
            let mut sum = 0_u64;
            let mut count = 0_u64;

            for neighbor_row in row_start..row_end {
                for neighbor_col in col_start..col_end {
                    let index = neighbor_row * width + neighbor_col;
                    sum += u64::from(field[index]);
                    count += 1;
                }
            }

            smoothed.push(u32::try_from(sum / count).unwrap_or(u32::MAX));
        }
    }
    smoothed
}

fn threshold_nonogram_field(field: &[u32], rng: &mut SeededRng) -> Vec<bool> {
    let mut sorted = field.to_vec();
    sorted.sort_unstable();

    let mut threshold_index = sorted.len() / 2;
    if sorted.len() % 2 == 1 && rng.next_bool() {
        threshold_index += 1;
    }

    let Some(threshold) = sorted.get(threshold_index).copied() else {
        return Vec::new();
    };

    field.iter().map(|value| *value >= threshold).collect()
}

fn solution_has_no_trivial_lines(width: usize, height: usize, cells: &[bool]) -> bool {
    (0..height).all(|row| {
        let filled = filled_in_row(width, cells, row);
        filled > 0 && filled < width
    }) && (0..width).all(|col| {
        let filled = filled_in_col(width, height, cells, col);
        filled > 0 && filled < height
    })
}

fn filled_in_row(width: usize, cells: &[bool], row: usize) -> usize {
    cells[(row * width)..((row + 1) * width)]
        .iter()
        .filter(|filled| **filled)
        .count()
}

fn filled_in_col(width: usize, height: usize, cells: &[bool], col: usize) -> usize {
    (0..height).filter(|row| cells[row * width + col]).count()
}

fn fallback_solution(width: usize, height: usize, _seed: u64) -> Vec<bool> {
    let mut cells = vec![false; width * height];
    for row in 0..height {
        if row == 1 {
            for col in 0..(width - 1) {
                cells[row * width + col] = true;
            }
        } else if row + 2 == height {
            for col in 1..width {
                cells[row * width + col] = true;
            }
        } else {
            cells[row * width] = true;
            cells[row * width + width - 1] = true;
        }
    }
    debug_assert!(solution_has_no_trivial_lines(width, height, &cells));
    cells
}

fn solution_is_usable(solution: &[bool], width: usize, height: usize) -> bool {
    let filled = solution.iter().filter(|filled| **filled).count();
    let cell_count = width * height;
    filled >= cell_count / 5 && filled <= (cell_count * 4) / 5
}

const fn next_seed(seed: u64) -> u64 {
    seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1)
}

struct SeededRng {
    state: u64,
}

impl SeededRng {
    const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    const fn next_u64(&mut self) -> u64 {
        self.state = next_seed(self.state);
        self.state ^ (self.state >> 33)
    }

    fn next_u32(&mut self) -> u32 {
        u32::try_from(self.next_u64() >> 32).unwrap_or(0)
    }

    const fn next_bool(&mut self) -> bool {
        self.next_u64() & 1 == 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clues_are_generated_for_rows_and_columns() {
        let puzzle = build_nonogram_puzzle(
            5,
            5,
            1,
            vec![
                true, true, false, false, true, false, false, false, false, false, true, false,
                true, true, false, false, true, true, false, false, false, false, false, true,
                true,
            ],
        );

        assert_eq!(
            puzzle.row_clues,
            vec![vec![2, 1], vec![], vec![1, 2], vec![2], vec![2]]
        );
        assert_eq!(
            puzzle.col_clues,
            vec![vec![1, 1], vec![1, 1], vec![2], vec![1, 1], vec![1, 1]]
        );
    }

    #[test]
    fn generated_puzzle_has_human_solution() {
        let puzzle = generate_nonogram_puzzle(10, 10, 42);

        assert_eq!(puzzle.width, 10);
        assert_eq!(puzzle.height, 10);
        assert!(nonogram_has_human_solution(&puzzle));
        assert!(puzzle_has_no_trivial_clues(&puzzle));
        assert!(nonogram_interest_score(&puzzle) >= target_nonogram_interest_score(10, 10));
    }

    #[test]
    fn generated_puzzles_avoid_empty_and_full_clues() {
        for seed in [1, 2, 3, 42, 777, 2024] {
            let puzzle = generate_nonogram_puzzle(10, 10, seed);

            assert!(
                puzzle_has_no_trivial_clues(&puzzle),
                "seed {seed} produced trivial clues: rows={:?} cols={:?}",
                puzzle.row_clues,
                puzzle.col_clues
            );
        }
    }

    #[test]
    fn raw_generated_solutions_are_balanced_by_threshold() {
        for seed in [1, 2, 3, 42, 777, 2024] {
            let solution = generated_solution(10, 10, seed);
            let filled = solution.iter().filter(|filled| **filled).count();

            assert!(
                (45..=55).contains(&filled),
                "seed {seed} produced {filled} filled cells"
            );
        }
    }

    #[test]
    fn fallback_puzzle_avoids_trivial_clues_and_has_gaps() {
        let puzzle = build_nonogram_puzzle(10, 10, 99, fallback_solution(10, 10, 99));

        assert!(puzzle_has_no_trivial_clues(&puzzle));
        assert!(nonogram_interest_score(&puzzle) >= target_nonogram_interest_score(10, 10));
        assert!(nonogram_has_human_solution(&puzzle));
    }

    #[test]
    fn validation_checks_exact_filled_cells() {
        let puzzle = build_nonogram_puzzle(2, 2, 7, vec![true, false, false, true]);
        let valid = vec![
            NonogramCellState::Filled,
            NonogramCellState::Crossed,
            NonogramCellState::Hidden,
            NonogramCellState::Filled,
        ];
        let invalid = vec![
            NonogramCellState::Filled,
            NonogramCellState::Filled,
            NonogramCellState::Hidden,
            NonogramCellState::Filled,
        ];

        assert!(validate_nonogram_solution(&puzzle, &valid).complete);
        assert!(!validate_nonogram_solution(&puzzle, &invalid).complete);
    }
}
