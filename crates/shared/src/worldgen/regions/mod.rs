use std::collections::VecDeque;

use super::Grid;
use crate::level::RegionKind;
use crate::topology::CellId;

pub(super) mod generation;

/// Typed cell partitioner for authoritative named-region extents.
pub(super) struct RegionPartitioner<'a> {
    grid: &'a Grid,
    class: &'a [Option<RegionKind>],
}

impl<'a> RegionPartitioner<'a> {
    pub(super) fn new(grid: &'a Grid, class: &'a [Option<RegionKind>]) -> Self {
        Self { grid, class }
    }

    /// Claims one edge-connected cluster of cells with the requested kind.
    pub(super) fn claim(
        &self,
        start: CellId,
        kind: RegionKind,
        region_index: u32,
        max_cells: usize,
        cell_region: &mut [Option<u32>],
    ) -> Vec<CellId> {
        let mut cells = vec![start];
        let mut queue = VecDeque::from([start]);
        cell_region[start.index()] = Some(region_index);
        while let Some(current) = queue.pop_front() {
            if cells.len() >= max_cells {
                break;
            }
            for &neighbor in self.grid.cell_neighbors(current) {
                let index = neighbor.index();
                if self.class[index] == Some(kind) && cell_region[index].is_none() {
                    cell_region[index] = Some(region_index);
                    cells.push(neighbor);
                    queue.push_back(neighbor);
                }
            }
        }
        cells
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claims_only_edge_connected_cells_of_the_same_region_kind() {
        let grid = Grid::new(1337);
        let start = grid.topology.cells().next().unwrap();
        let neighbor = grid.cell_neighbors(start)[0];
        let remote = grid
            .topology
            .cells()
            .find(|&cell| {
                cell != start
                    && !grid.cell_neighbors(start).contains(&cell)
                    && !grid.cell_neighbors(neighbor).contains(&cell)
            })
            .unwrap();
        let mut class = vec![None; grid.cell_count()];
        class[start.index()] = Some(RegionKind::Plains);
        class[neighbor.index()] = Some(RegionKind::Plains);
        class[remote.index()] = Some(RegionKind::Plains);
        let mut assignments = vec![None; grid.cell_count()];

        let cells = RegionPartitioner::new(&grid, &class).claim(
            start,
            RegionKind::Plains,
            1,
            usize::MAX,
            &mut assignments,
        );

        assert!(cells.contains(&start));
        assert!(cells.contains(&neighbor));
        assert!(!cells.contains(&remote));
        assert_eq!(assignments[remote.index()], None);
    }
}
