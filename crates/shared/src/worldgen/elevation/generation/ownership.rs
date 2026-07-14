pub(in crate::worldgen) fn kernel_interp(kernel: &[(usize, f32); 6], values: &[f32]) -> f32 {
    let mut sum = 0.0f32;
    let mut weighted = 0.0f32;
    for &(solver_vertex, dot) in kernel {
        let w = 1.0 / (1.01 - dot).max(0.01);
        weighted += values[solver_vertex] * w;
        sum += w;
    }
    if sum > 0.0 {
        weighted / sum
    } else {
        values[kernel[0].0]
    }
}

/// Solver verts are a bit-exact subset of the grid's cell vertices (each
/// subdivision keeps its parents), so tile ownership comes straight from the
/// cell labels — no geometric face lookup, no fallback kind.
/// Per-solver-vertex value looked up from the per-CELL array: solver verts are
/// a bit-exact subset of grid cells (each subdivision keeps its parents), so
/// the map is exact, with a nearest-corner fallback for the (unexpected) miss.
/// Backs both tile-kind ownership and landform ownership.
pub(in crate::worldgen) fn owner_of<T: Copy>(
    grid: &Grid,
    terrain: &TerrainGen,
    per_cell: &[T],
    default: T,
) -> Vec<T> {
    let index: BTreeMap<[u32; 3], u32> = grid
        .topology
        .cells()
        .map(|cell| {
            let direction = grid.cell_direction(cell);
            (
                [
                    direction.x.to_bits(),
                    direction.y.to_bits(),
                    direction.z.to_bits(),
                ],
                cell.index() as u32,
            )
        })
        .collect();
    (0..terrain.vert_count())
        .map(|solver_vertex| {
            let d = terrain.vert_dir(solver_vertex);
            let key = [d.x.to_bits(), d.y.to_bits(), d.z.to_bits()];
            match index.get(&key) {
                Some(&cell_index) => per_cell[cell_index as usize],
                None => grid
                    .planet
                    .face_at(d)
                    .map(|face_index| {
                        let best = grid
                            .face_cells(FaceId::new(face_index))
                            .map(CellId::index)
                            .into_iter()
                            .max_by(|&a, &b| {
                                grid.cell_direction(CellId::new(a))
                                    .dot(d)
                                    .partial_cmp(&grid.cell_direction(CellId::new(b)).dot(d))
                                    .unwrap()
                            })
                            .unwrap();
                        per_cell[best]
                    })
                    .unwrap_or(default),
            }
        })
        .collect()
}

pub(in crate::worldgen) fn owner_cells(
    grid: &Grid,
    terrain: &TerrainGen,
    cells: &[Terrain],
) -> Vec<Terrain> {
    owner_of(grid, terrain, cells, Terrain::Plains)
}

pub(in crate::worldgen) fn owner_landform(
    grid: &Grid,
    terrain: &TerrainGen,
    landform: &[Landform],
) -> Vec<Landform> {
    owner_of(grid, terrain, landform, Landform::Lowland)
}
use std::collections::BTreeMap;

use crate::level::Landform;
use crate::terrain::{Terrain, TerrainGen};
use crate::topology::{CellId, FaceId};
use crate::worldgen::Grid;
