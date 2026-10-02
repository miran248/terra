use std::cmp::Reverse;
use std::collections::{BTreeSet, BinaryHeap};

use bevy::prelude::Vec3;

use super::Grid;
use crate::topology::CellId;

const ROUTE_STATE_SLOTS: usize = 7;
const START_SLOT: usize = 6;
const NO_PREDECESSOR: u32 = u32::MAX;

/// Direction-aware A* over dense `(cell, incoming-edge)` states.
pub(super) fn lattice_path(
    grid: &Grid,
    blocked: impl Fn(CellId) -> bool,
    extra: impl Fn(CellId) -> u64,
    from: CellId,
    to: CellId,
) -> Vec<CellId> {
    lattice_path_with_edge(grid, blocked, extra, |_, _| false, from, to)
}

/// Direction-aware A* over dense `(cell, incoming-edge)` states, with an
/// optional directed-edge constraint for features whose footprint depends on
/// travel direction.
pub(super) fn lattice_path_with_edge(
    grid: &Grid,
    blocked: impl Fn(CellId) -> bool,
    extra: impl Fn(CellId) -> u64,
    edge_blocked: impl Fn(CellId, CellId) -> bool,
    from: CellId,
    to: CellId,
) -> Vec<CellId> {
    lattice_path_to_any_with_edge(grid, blocked, extra, edge_blocked, from, &[to])
        .map_or_else(Vec::new, |(path, _)| path)
}

/// Route to the first reachable cell in a target set, using the closest
/// target as the A* heuristic. This avoids restarting a full route search for
/// every road cell around a bridge entrance.
pub(super) fn lattice_path_to_any_with_edge(
    grid: &Grid,
    blocked: impl Fn(CellId) -> bool,
    extra: impl Fn(CellId) -> u64,
    edge_blocked: impl Fn(CellId, CellId) -> bool,
    from: CellId,
    targets: &[CellId],
) -> Option<(Vec<CellId>, CellId)> {
    let target_set = targets.iter().copied().collect::<BTreeSet<_>>();
    if target_set.is_empty() {
        return None;
    }
    if target_set.contains(&from) {
        return Some((vec![from], from));
    }
    let target_directions = target_set
        .iter()
        .map(|&cell| grid.cell_direction(cell))
        .collect::<Vec<_>>();
    const STEP: u64 = 1000;
    let turn_cost = |previous: Vec3, direction: Vec3| {
        ((1.0 - previous.dot(direction)).max(0.0) * 2500.0) as u64
    };
    let first_cell = grid.topology.cell(0).expect("grid has cells");
    let edge_angle = grid
        .cell_direction(first_cell)
        .angle_between(grid.cell_direction(grid.cell_neighbors(first_cell)[0]));
    let heuristic = |cell: CellId| {
        let direction = grid.cell_direction(cell);
        let nearest_dot = target_directions
            .iter()
            .map(|target| direction.dot(*target))
            .fold(-1.0f32, f32::max)
            .clamp(-1.0, 1.0);
        (nearest_dot.acos() / edge_angle * 990.0) as u64
    };
    let direction =
        |a: CellId, b: CellId| (grid.cell_direction(b) - grid.cell_direction(a)).normalize();

    let state_count = grid
        .cell_count()
        .checked_mul(ROUTE_STATE_SLOTS)
        .expect("route state count overflow");
    let state_count_u32 = u32::try_from(state_count)
        .expect("grid has too many route states for predecessor encoding");
    let mut best = vec![[u64::MAX; ROUTE_STATE_SLOTS]; grid.cell_count()];
    let mut came = vec![NO_PREDECESSOR; state_count];
    let mut heap: BinaryHeap<Reverse<(u64, u64, usize, usize)>> = BinaryHeap::new();
    best[from.index()][START_SLOT] = 0;
    heap.push(Reverse((heuristic(from), 0, from.index(), START_SLOT)));
    while let Some(Reverse((_, cost, cell_index, slot))) = heap.pop() {
        if best[cell_index][slot] < cost {
            continue;
        }
        let cell = grid
            .topology
            .cell(cell_index)
            .expect("router state uses topology cell ids");
        if target_set.contains(&cell) {
            let mut path = vec![cell];
            let mut current = cell_index * ROUTE_STATE_SLOTS + slot;
            while came[current] != NO_PREDECESSOR {
                let previous = came[current] as usize;
                path.push(
                    grid.topology
                        .cell(previous / ROUTE_STATE_SLOTS)
                        .expect("router predecessor uses topology cell ids"),
                );
                current = previous;
            }
            path.reverse();
            return Some((path, cell));
        }
        let previous_direction =
            (slot < 6).then(|| direction(grid.cell_neighbors(cell)[slot], cell));
        for &neighbor in grid.cell_neighbors(cell) {
            if (blocked(neighbor) && !target_set.contains(&neighbor))
                || edge_blocked(cell, neighbor)
            {
                continue;
            }
            let next_direction = direction(cell, neighbor);
            let next_cost = cost
                + STEP
                + extra(neighbor)
                + previous_direction.map_or(0, |previous| turn_cost(previous, next_direction));
            let next_slot = grid
                .cell_neighbors(neighbor)
                .iter()
                .position(|candidate| *candidate == cell)
                .expect("adjacent cells have reciprocal edges");
            if next_cost < best[neighbor.index()][next_slot] {
                best[neighbor.index()][next_slot] = next_cost;
                let state = neighbor.index() * ROUTE_STATE_SLOTS + next_slot;
                let predecessor = cell_index * ROUTE_STATE_SLOTS + slot;
                debug_assert!(state < state_count);
                debug_assert!(predecessor < state_count);
                debug_assert!(state <= state_count_u32 as usize);
                came[state] = predecessor as u32;
                heap.push(Reverse((
                    next_cost + heuristic(neighbor),
                    next_cost,
                    neighbor.index(),
                    next_slot,
                )));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locked_directional_path() {
        let grid = Grid::new(1337);
        let from = grid.topology.cell(0).unwrap();
        let to = grid.topology.cell(100).unwrap();
        let path: Vec<_> = lattice_path(&grid, |_| false, |_| 0, from, to)
            .into_iter()
            .map(CellId::index)
            .collect();
        assert_eq!(
            path,
            vec![
                0, 2, 5, 12, 11, 36, 32, 30, 29, 105, 108, 114, 95, 96, 98, 102, 100
            ]
        );
    }
}
