use super::MinesweeperBoard;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

const MAX_COMPONENT_CELLS: usize = 32;
const MAX_COMPONENT_SOLUTIONS: usize = 100_000;

pub fn can_solve_without_guessing_from(board: &MinesweeperBoard, first_reveal: usize) -> bool {
    Solver::new(board).can_solve_from(first_reveal)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Constraint {
    cells: BTreeSet<usize>,
    mines: usize,
}

#[derive(Debug, Default)]
struct Deductions {
    safe: BTreeSet<usize>,
    mines: BTreeSet<usize>,
}

impl Deductions {
    fn is_empty(&self) -> bool {
        self.safe.is_empty() && self.mines.is_empty()
    }

    fn add_safe_cells(&mut self, cells: impl IntoIterator<Item = usize>) {
        self.safe.extend(cells);
    }

    fn add_mine_cells(&mut self, cells: impl IntoIterator<Item = usize>) {
        self.mines.extend(cells);
    }
}

struct Solver<'a> {
    board: &'a MinesweeperBoard,
    known_safe: Vec<bool>,
    known_mines: Vec<bool>,
}

impl<'a> Solver<'a> {
    fn new(board: &'a MinesweeperBoard) -> Self {
        Self {
            board,
            known_safe: vec![false; board.cells.len()],
            known_mines: vec![false; board.cells.len()],
        }
    }

    fn can_solve_from(mut self, first_reveal: usize) -> bool {
        if first_reveal >= self.board.cells.len() || self.board.cells[first_reveal].mine {
            return false;
        }
        if !self.reveal_safe_area(first_reveal) {
            return false;
        }

        loop {
            if self.all_safe_cells_known() {
                return true;
            }

            let Some(progress) = self.apply_pass() else {
                return false;
            };
            if !progress {
                return false;
            }
        }
    }

    fn all_safe_cells_known(&self) -> bool {
        self.board
            .cells
            .iter()
            .enumerate()
            .all(|(index, cell)| cell.mine || self.known_safe[index])
    }

    fn apply_pass(&mut self) -> Option<bool> {
        let local_constraints = self.local_constraints()?;
        let global_constraint = self.global_constraint()?;
        let mut constraints = local_constraints.clone();
        push_constraint(&mut constraints, global_constraint.clone());

        let mut deductions = simple_deductions(&constraints)?;
        if deductions.is_empty() {
            deductions = combination_deductions(&local_constraints, &global_constraint)?;
        }

        self.apply_deductions(deductions)
    }

    fn local_constraints(&self) -> Option<Vec<Constraint>> {
        let mut constraints = Vec::new();
        for index in 0..self.board.cells.len() {
            if !self.known_safe[index] || self.board.cells[index].adjacent_mines == 0 {
                continue;
            }

            let mut unknown_neighbors = BTreeSet::new();
            let mut known_neighbor_mines = 0usize;
            for neighbor in self.board.neighbors(index) {
                if self.known_mines[neighbor] {
                    known_neighbor_mines += 1;
                } else if !self.known_safe[neighbor] {
                    unknown_neighbors.insert(neighbor);
                }
            }

            let adjacent_mines = usize::from(self.board.cells[index].adjacent_mines);
            if known_neighbor_mines > adjacent_mines {
                return None;
            }
            let remaining_mines = adjacent_mines - known_neighbor_mines;
            if remaining_mines > unknown_neighbors.len() {
                return None;
            }
            push_constraint(
                &mut constraints,
                Constraint {
                    cells: unknown_neighbors,
                    mines: remaining_mines,
                },
            );
        }

        Some(constraints)
    }

    fn global_constraint(&self) -> Option<Constraint> {
        let known_mine_count = self.known_mines.iter().filter(|known| **known).count();
        if known_mine_count > self.board.mines {
            return None;
        }

        let remaining_mines = self.board.mines - known_mine_count;
        let unknown_cells = self
            .known_safe
            .iter()
            .enumerate()
            .filter_map(|(index, safe)| (!*safe && !self.known_mines[index]).then_some(index))
            .collect::<BTreeSet<_>>();
        if remaining_mines > unknown_cells.len() {
            return None;
        }

        Some(Constraint {
            cells: unknown_cells,
            mines: remaining_mines,
        })
    }

    fn apply_deductions(&mut self, deductions: Deductions) -> Option<bool> {
        if deductions
            .safe
            .iter()
            .any(|index| deductions.mines.contains(index) || self.board.cells[*index].mine)
        {
            return None;
        }
        if deductions
            .mines
            .iter()
            .any(|index| self.known_safe[*index] || !self.board.cells[*index].mine)
        {
            return None;
        }

        let mut progress = false;
        for index in deductions.mines {
            if !self.known_mines[index] {
                self.known_mines[index] = true;
                progress = true;
            }
        }
        for index in deductions.safe {
            progress |= self.reveal_safe_area(index);
        }

        Some(progress)
    }

    fn reveal_safe_area(&mut self, index: usize) -> bool {
        if index >= self.board.cells.len() || self.board.cells[index].mine || self.known_safe[index]
        {
            return false;
        }

        let mut changed = false;
        let mut queue = VecDeque::from([index]);
        while let Some(next) = queue.pop_front() {
            if self.board.cells[next].mine || self.known_safe[next] {
                continue;
            }

            self.known_safe[next] = true;
            changed = true;
            if self.board.cells[next].adjacent_mines == 0 {
                for neighbor in self.board.neighbors(next) {
                    if !self.board.cells[neighbor].mine && !self.known_safe[neighbor] {
                        queue.push_back(neighbor);
                    }
                }
            }
        }

        changed
    }
}

fn simple_deductions(constraints: &[Constraint]) -> Option<Deductions> {
    let mut deductions = Deductions::default();

    for constraint in constraints {
        collect_direct_deductions(constraint, &mut deductions)?;
    }

    for left_index in 0..constraints.len() {
        for right_index in (left_index + 1)..constraints.len() {
            let left = &constraints[left_index];
            let right = &constraints[right_index];
            if left.cells == right.cells {
                continue;
            }

            if left.cells.is_subset(&right.cells) {
                collect_difference_deductions(left, right, &mut deductions)?;
            } else if right.cells.is_subset(&left.cells) {
                collect_difference_deductions(right, left, &mut deductions)?;
            }
        }
    }

    Some(deductions)
}

fn collect_difference_deductions(
    subset: &Constraint,
    superset: &Constraint,
    deductions: &mut Deductions,
) -> Option<()> {
    let mine_difference = superset.mines.checked_sub(subset.mines)?;
    let cell_difference = superset
        .cells
        .difference(&subset.cells)
        .copied()
        .collect::<BTreeSet<_>>();
    collect_direct_deductions(
        &Constraint {
            cells: cell_difference,
            mines: mine_difference,
        },
        deductions,
    )
}

fn collect_direct_deductions(constraint: &Constraint, deductions: &mut Deductions) -> Option<()> {
    if constraint.mines > constraint.cells.len() {
        return None;
    }
    if constraint.mines == 0 {
        deductions.add_safe_cells(constraint.cells.iter().copied());
    } else if constraint.mines == constraint.cells.len() {
        deductions.add_mine_cells(constraint.cells.iter().copied());
    }
    Some(())
}

fn combination_deductions(
    local_constraints: &[Constraint],
    global_constraint: &Constraint,
) -> Option<Deductions> {
    let components = constraint_components(local_constraints);
    let mut deductions = Deductions::default();
    let mut summaries = Vec::new();
    let mut all_components_enumerated = true;

    for component in components {
        match enumerate_component(&component)? {
            ComponentEnumeration::Enumerated(summary) => {
                collect_assignment_certainties(
                    &summary.cells,
                    summary.assignments.iter(),
                    &mut deductions,
                )?;
                summaries.push(summary);
            }
            ComponentEnumeration::Skipped => {
                all_components_enumerated = false;
            }
        }
    }

    if all_components_enumerated {
        collect_global_combination_deductions(&summaries, global_constraint, &mut deductions)?;
    }

    Some(deductions)
}

fn constraint_components(constraints: &[Constraint]) -> Vec<Vec<Constraint>> {
    let mut cell_to_constraints = BTreeMap::<usize, Vec<usize>>::new();
    for (index, constraint) in constraints.iter().enumerate() {
        for cell in &constraint.cells {
            cell_to_constraints.entry(*cell).or_default().push(index);
        }
    }

    let mut visited = vec![false; constraints.len()];
    let mut components = Vec::new();
    for start in 0..constraints.len() {
        if visited[start] {
            continue;
        }

        visited[start] = true;
        let mut queue = VecDeque::from([start]);
        let mut component = Vec::new();
        while let Some(index) = queue.pop_front() {
            let constraint = constraints[index].clone();
            for cell in &constraint.cells {
                if let Some(neighbor_constraints) = cell_to_constraints.get(cell) {
                    for neighbor in neighbor_constraints {
                        if !visited[*neighbor] {
                            visited[*neighbor] = true;
                            queue.push_back(*neighbor);
                        }
                    }
                }
            }
            component.push(constraint);
        }
        components.push(component);
    }

    components
}

#[derive(Debug, Clone)]
struct ComponentAssignment {
    mask: u128,
    mines: usize,
}

#[derive(Debug, Clone)]
struct ComponentSummary {
    cells: Vec<usize>,
    assignments: Vec<ComponentAssignment>,
}

enum ComponentEnumeration {
    Enumerated(ComponentSummary),
    Skipped,
}

#[derive(Debug, Clone)]
struct ComponentConstraint {
    cells: Vec<usize>,
    mines: usize,
}

#[derive(Debug, Clone)]
struct ConstraintProgress {
    mines: usize,
    unassigned: usize,
    required_mines: usize,
}

impl ConstraintProgress {
    const fn can_still_be_satisfied(&self) -> bool {
        self.mines <= self.required_mines && self.mines + self.unassigned >= self.required_mines
    }

    const fn is_satisfied(&self) -> bool {
        self.unassigned == 0 && self.mines == self.required_mines
    }
}

fn enumerate_component(constraints: &[Constraint]) -> Option<ComponentEnumeration> {
    let cells = constraints
        .iter()
        .flat_map(|constraint| constraint.cells.iter().copied())
        .collect::<BTreeSet<_>>();
    if cells.len() > MAX_COMPONENT_CELLS {
        return Some(ComponentEnumeration::Skipped);
    }

    let mut cells = cells.into_iter().collect::<Vec<_>>();
    cells.sort_by_key(|cell| {
        let touching_count = constraints
            .iter()
            .filter(|constraint| constraint.cells.contains(cell))
            .count();
        std::cmp::Reverse(touching_count)
    });

    let cell_positions = cells
        .iter()
        .enumerate()
        .map(|(position, cell)| (*cell, position))
        .collect::<BTreeMap<_, _>>();
    let component_constraints = constraints
        .iter()
        .map(|constraint| {
            let cells = constraint
                .cells
                .iter()
                .filter_map(|cell| cell_positions.get(cell).copied())
                .collect::<Vec<_>>();
            if constraint.mines > cells.len() {
                return None;
            }
            Some(ComponentConstraint {
                cells,
                mines: constraint.mines,
            })
        })
        .collect::<Option<Vec<_>>>()?;

    let mut touching_constraints = vec![Vec::new(); cells.len()];
    for (constraint_index, constraint) in component_constraints.iter().enumerate() {
        for cell_position in &constraint.cells {
            touching_constraints[*cell_position].push(constraint_index);
        }
    }

    let mut progress = component_constraints
        .iter()
        .map(|constraint| ConstraintProgress {
            mines: 0,
            unassigned: constraint.cells.len(),
            required_mines: constraint.mines,
        })
        .collect::<Vec<_>>();
    let mut assignments = Vec::new();
    let completed = collect_component_assignments(
        0,
        &touching_constraints,
        &mut progress,
        0,
        0,
        &mut assignments,
    );
    if !completed {
        return Some(ComponentEnumeration::Skipped);
    }
    if assignments.is_empty() {
        return None;
    }

    Some(ComponentEnumeration::Enumerated(ComponentSummary {
        cells,
        assignments,
    }))
}

fn collect_component_assignments(
    position: usize,
    touching_constraints: &[Vec<usize>],
    progress: &mut [ConstraintProgress],
    mask: u128,
    mine_count: usize,
    assignments: &mut Vec<ComponentAssignment>,
) -> bool {
    if assignments.len() > MAX_COMPONENT_SOLUTIONS {
        return false;
    }

    if position == touching_constraints.len() {
        if progress.iter().all(ConstraintProgress::is_satisfied) {
            assignments.push(ComponentAssignment {
                mask,
                mines: mine_count,
            });
        }
        return true;
    }

    collect_component_assignments_with_cell(
        false,
        position,
        touching_constraints,
        progress,
        mask,
        mine_count,
        assignments,
    ) && collect_component_assignments_with_cell(
        true,
        position,
        touching_constraints,
        progress,
        mask,
        mine_count,
        assignments,
    )
}

fn collect_component_assignments_with_cell(
    is_mine: bool,
    position: usize,
    touching_constraints: &[Vec<usize>],
    progress: &mut [ConstraintProgress],
    mask: u128,
    mine_count: usize,
    assignments: &mut Vec<ComponentAssignment>,
) -> bool {
    for constraint_index in &touching_constraints[position] {
        progress[*constraint_index].unassigned -= 1;
        if is_mine {
            progress[*constraint_index].mines += 1;
        }
    }

    let valid = touching_constraints[position]
        .iter()
        .all(|constraint_index| progress[*constraint_index].can_still_be_satisfied());
    let completed = if valid {
        let next_mask = if is_mine {
            mask | (1u128 << position)
        } else {
            mask
        };
        collect_component_assignments(
            position + 1,
            touching_constraints,
            progress,
            next_mask,
            mine_count + usize::from(is_mine),
            assignments,
        )
    } else {
        true
    };

    for constraint_index in &touching_constraints[position] {
        if is_mine {
            progress[*constraint_index].mines -= 1;
        }
        progress[*constraint_index].unassigned += 1;
    }

    completed
}

fn collect_assignment_certainties<'a>(
    cells: &[usize],
    assignments: impl IntoIterator<Item = &'a ComponentAssignment>,
    deductions: &mut Deductions,
) -> Option<()> {
    let assignments = assignments.into_iter().collect::<Vec<_>>();
    if assignments.is_empty() {
        return None;
    }

    let all_cells_mask = (1u128 << cells.len()) - 1;
    let mut always_mines = all_cells_mask;
    let mut sometimes_mines = 0u128;
    for assignment in assignments {
        always_mines &= assignment.mask;
        sometimes_mines |= assignment.mask;
    }

    for (position, cell) in cells.iter().enumerate() {
        let cell_mask = 1u128 << position;
        if always_mines & cell_mask != 0 {
            deductions.mines.insert(*cell);
        } else if sometimes_mines & cell_mask == 0 {
            deductions.safe.insert(*cell);
        }
    }

    Some(())
}

fn collect_global_combination_deductions(
    summaries: &[ComponentSummary],
    global_constraint: &Constraint,
    deductions: &mut Deductions,
) -> Option<()> {
    let frontier_cells = summaries
        .iter()
        .flat_map(|summary| summary.cells.iter().copied())
        .collect::<BTreeSet<_>>();
    let unconstrained_cells = global_constraint
        .cells
        .difference(&frontier_cells)
        .copied()
        .collect::<BTreeSet<_>>();
    let component_counts = summaries
        .iter()
        .map(ComponentSummary::mine_counts)
        .collect::<Vec<_>>();
    let prefix_counts = prefix_mine_counts(&component_counts, global_constraint.mines);
    let suffix_counts = suffix_mine_counts(&component_counts, global_constraint.mines);
    let total_component_counts = prefix_counts
        .last()
        .cloned()
        .unwrap_or_else(|| BTreeSet::from([0]));

    let feasible_unconstrained_counts = total_component_counts
        .iter()
        .filter_map(|component_mines| {
            let unconstrained_mines = global_constraint.mines.checked_sub(*component_mines)?;
            (unconstrained_mines <= unconstrained_cells.len()).then_some(unconstrained_mines)
        })
        .collect::<BTreeSet<_>>();
    if feasible_unconstrained_counts.is_empty() {
        return None;
    }
    if feasible_unconstrained_counts.len() == 1 {
        let unconstrained_mines = *feasible_unconstrained_counts
            .iter()
            .next()
            .expect("set has one value");
        if unconstrained_mines == 0 {
            deductions.add_safe_cells(unconstrained_cells.iter().copied());
        } else if unconstrained_mines == unconstrained_cells.len() {
            deductions.add_mine_cells(unconstrained_cells.iter().copied());
        }
    }

    for (index, summary) in summaries.iter().enumerate() {
        collect_assignment_certainties(
            &summary.cells,
            summary.assignments.iter().filter(|assignment| {
                assignment_has_global_completion(
                    assignment.mines,
                    &prefix_counts[index],
                    &suffix_counts[index + 1],
                    global_constraint.mines,
                    unconstrained_cells.len(),
                )
            }),
            deductions,
        )?;
    }

    Some(())
}

impl ComponentSummary {
    fn mine_counts(&self) -> BTreeSet<usize> {
        self.assignments
            .iter()
            .map(|assignment| assignment.mines)
            .collect()
    }
}

fn prefix_mine_counts(
    component_counts: &[BTreeSet<usize>],
    max_mines: usize,
) -> Vec<BTreeSet<usize>> {
    let mut prefix = vec![BTreeSet::from([0])];
    for counts in component_counts {
        let previous = prefix.last().expect("prefix starts with zero");
        prefix.push(combine_mine_counts(previous, counts, max_mines));
    }
    prefix
}

fn suffix_mine_counts(
    component_counts: &[BTreeSet<usize>],
    max_mines: usize,
) -> Vec<BTreeSet<usize>> {
    let mut suffix = vec![BTreeSet::new(); component_counts.len() + 1];
    suffix[component_counts.len()].insert(0);
    for index in (0..component_counts.len()).rev() {
        suffix[index] =
            combine_mine_counts(&component_counts[index], &suffix[index + 1], max_mines);
    }
    suffix
}

fn combine_mine_counts(
    left: &BTreeSet<usize>,
    right: &BTreeSet<usize>,
    max_mines: usize,
) -> BTreeSet<usize> {
    left.iter()
        .flat_map(|left_count| {
            right.iter().filter_map(move |right_count| {
                let total = left_count + right_count;
                (total <= max_mines).then_some(total)
            })
        })
        .collect()
}

fn assignment_has_global_completion(
    assignment_mines: usize,
    prefix_counts: &BTreeSet<usize>,
    suffix_counts: &BTreeSet<usize>,
    remaining_mines: usize,
    unconstrained_cells: usize,
) -> bool {
    prefix_counts.iter().any(|prefix_mines| {
        suffix_counts.iter().any(|suffix_mines| {
            let used_mines = prefix_mines + assignment_mines + suffix_mines;
            used_mines <= remaining_mines && remaining_mines - used_mines <= unconstrained_cells
        })
    })
}

fn push_constraint(constraints: &mut Vec<Constraint>, constraint: Constraint) {
    if constraint.cells.is_empty() {
        return;
    }
    if constraints.iter().any(|existing| existing == &constraint) {
        return;
    }
    constraints.push(constraint);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn component_solver_finds_corner_mine_from_group_counts() {
        let constraints = vec![
            constraint([0, 1], 1),
            constraint([2, 3], 1),
            constraint([0, 1, 2, 3, 4], 3),
        ];
        let global = constraint([0, 1, 2, 3, 4], 3);

        let deductions = combination_deductions(&constraints, &global).expect("deductions");

        assert_eq!(deductions.mines, BTreeSet::from([4]));
        assert!(deductions.safe.is_empty());
    }

    #[test]
    fn component_solver_finds_corner_safe_from_group_counts() {
        let constraints = vec![
            constraint([0, 1], 1),
            constraint([2, 3], 1),
            constraint([0, 1, 2, 3, 4], 2),
        ];
        let global = constraint([0, 1, 2, 3, 4], 2);

        let deductions = combination_deductions(&constraints, &global).expect("deductions");

        assert_eq!(deductions.safe, BTreeSet::from([4]));
        assert!(deductions.mines.is_empty());
    }

    #[test]
    fn global_mine_count_marks_unconstrained_cells_safe() {
        let constraints = vec![constraint([0, 1], 1), constraint([2, 3], 1)];
        let global = constraint([0, 1, 2, 3, 4], 2);

        let deductions = combination_deductions(&constraints, &global).expect("deductions");

        assert_eq!(deductions.safe, BTreeSet::from([4]));
        assert!(deductions.mines.is_empty());
    }

    #[test]
    fn solver_solves_minesweeper_online_challenge_from_top_right_opening() {
        let board = board_from_minesweeper_online_rows(&[
            "2*31111101*11*11*2*11111*10000",
            "2*3*11*10122211223111*22110000",
            "22311111001*2102*21123*2012210",
            "1*2211000123*102*33*22*201**32",
            "23*2*10012*2110124**21110245**",
            "*21333101*3210002**3101111**42",
            "2322**10112*10113*42101*224*20",
            "1**332100122212*213*20223*2110",
            "123*100002*32*32313*201*333100",
            "0022200002*5*32*2*2222333**210",
            "124*2011112**3234212*4**324*20",
            "1***311*1024*33**102**4*213*20",
            "14*5*111223*3*3*4212332234*321",
            "24*422112**343312*11*101***21*",
            "**23*32*3343**1022211101232111",
            "2212*3*22*2*32101*100000000000",
        ]);
        let first_reveal = board.index(0, 29).expect("top-right cell");

        assert!(can_solve_without_guessing_from(&board, first_reveal));
    }

    fn constraint<const N: usize>(cells: [usize; N], mines: usize) -> Constraint {
        Constraint {
            cells: BTreeSet::from(cells),
            mines,
        }
    }

    fn board_from_minesweeper_online_rows<const HEIGHT: usize>(
        rows: &[&str; HEIGHT],
    ) -> MinesweeperBoard {
        let width = rows[0].len();
        let mut mines = BTreeSet::new();
        for (row, cells) in rows.iter().enumerate() {
            assert_eq!(cells.len(), width);
            for (col, cell) in cells.bytes().enumerate() {
                if cell == b'*' {
                    mines.insert(row * width + col);
                }
            }
        }

        let board = MinesweeperBoard::from_mines(width, HEIGHT, mines, 1);
        assert_eq!(board.width, 30);
        assert_eq!(board.height, 16);
        assert_eq!(board.mines, 99);

        for (row, cells) in rows.iter().enumerate() {
            for (col, cell) in cells.bytes().enumerate() {
                let index = board.index(row, col).expect("fixture cell");
                if cell == b'*' {
                    assert!(board.cells[index].mine);
                } else {
                    assert_eq!(board.cells[index].adjacent_mines, cell - b'0');
                }
            }
        }

        board
    }
}
