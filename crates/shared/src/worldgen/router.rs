use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};

use bevy::prelude::Vec3;

use super::Grid;
use crate::topology::CellId;

/// Direction-aware A* over dense `(cell, incoming-edge)` states.
pub(super) fn lattice_path(
    grid: &Grid,
    blocked: impl Fn(CellId) -> bool,
    extra: impl Fn(CellId) -> u64,
    from: CellId,
    to: CellId,
) -> Vec<CellId> {
    if from == to {
        return vec![from];
    }
    const STEP: u64 = 1000;
    let turn_cost = |previous: Vec3, direction: Vec3| {
        ((1.0 - previous.dot(direction)).max(0.0) * 2500.0) as u64
    };
    let first_cell = grid.topology.cell(0).expect("grid has cells");
    let edge_angle = grid
        .cell_direction(first_cell)
        .angle_between(grid.cell_direction(grid.cell_neighbors(first_cell)[0]));
    let heuristic = |cell: CellId| {
        (grid
            .cell_direction(cell)
            .angle_between(grid.cell_direction(to))
            / edge_angle
            * 990.0) as u64
    };
    let direction =
        |a: CellId, b: CellId| (grid.cell_direction(b) - grid.cell_direction(a)).normalize();

    let mut best = vec![[u64::MAX; 7]; grid.cell_count()];
    let mut came: BTreeMap<(usize, usize), (usize, usize)> = BTreeMap::new();
    let mut heap: BinaryHeap<Reverse<(u64, u64, usize, usize)>> = BinaryHeap::new();
    best[from.index()][6] = 0;
    heap.push(Reverse((heuristic(from), 0, from.index(), 6)));
    while let Some(Reverse((_, cost, cell_index, slot))) = heap.pop() {
        if best[cell_index][slot] < cost {
            continue;
        }
        let cell = grid
            .topology
            .cell(cell_index)
            .expect("router state uses topology cell ids");
        if cell == to {
            let mut path = vec![cell];
            let mut current = (cell_index, slot);
            while let Some(&previous) = came.get(&current) {
                path.push(
                    grid.topology
                        .cell(previous.0)
                        .expect("router predecessor uses topology cell ids"),
                );
                current = previous;
            }
            path.reverse();
            return path;
        }
        let previous_direction =
            (slot < 6).then(|| direction(grid.cell_neighbors(cell)[slot], cell));
        for &neighbor in grid.cell_neighbors(cell) {
            if blocked(neighbor) && neighbor != to {
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
                came.insert((neighbor.index(), next_slot), (cell_index, slot));
                heap.push(Reverse((
                    next_cost + heuristic(neighbor),
                    next_cost,
                    neighbor.index(),
                    next_slot,
                )));
            }
        }
    }
    Vec::new()
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
