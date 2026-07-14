use std::collections::BTreeMap;

use super::Grid;

pub(super) fn face_max(grid: &Grid, per_cell: &[u8]) -> Vec<u8> {
    (0..grid.face_count())
        .map(|face| {
            grid.face_cells(face)
                .into_iter()
                .map(|cell| per_cell[cell])
                .max()
                .unwrap_or(0)
        })
        .collect()
}

pub(super) fn face_majority(grid: &Grid, per_cell: &[u8]) -> Vec<u8> {
    (0..grid.face_count())
        .map(|face| {
            let mut counts: BTreeMap<u8, usize> = BTreeMap::new();
            for cell in grid.face_cells(face) {
                *counts.entry(per_cell[cell]).or_default() += 1;
            }
            counts
                .into_iter()
                .max_by_key(|(_, count)| *count)
                .map(|(kind, _)| kind)
                .unwrap_or(0)
        })
        .collect()
}
