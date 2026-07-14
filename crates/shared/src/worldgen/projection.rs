use super::Grid;
use crate::topology::{CellId, FaceId};

pub(super) fn face_max<T: Copy>(grid: &Grid, per_cell: &[T], rank: impl Fn(T) -> u8) -> Vec<T> {
    (0..grid.face_count())
        .map(|face| {
            grid.face_cells(FaceId::new(face))
                .map(CellId::index)
                .into_iter()
                .map(|cell| per_cell[cell])
                .max_by_key(|value| rank(*value))
                .expect("every face has three cells")
        })
        .collect()
}

pub(super) fn face_majority<T: Copy + Eq>(
    grid: &Grid,
    per_cell: &[T],
    rank: impl Fn(T) -> u8,
) -> Vec<T> {
    (0..grid.face_count())
        .map(|face| {
            let cells = grid.face_cells(FaceId::new(face)).map(CellId::index);
            let values = cells.map(|cell| per_cell[cell]);
            values
                .into_iter()
                .max_by_key(|candidate| {
                    let count = values.iter().filter(|value| *value == candidate).count();
                    (count, u8::MAX - rank(*candidate))
                })
                .expect("every face has three cells")
        })
        .collect()
}
