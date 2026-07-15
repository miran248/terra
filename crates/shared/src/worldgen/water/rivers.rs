use crate::sphere::SpherePos;
use crate::terrain::{Terrain, TerrainGen};
use crate::topology::{CellId, FaceId};

use super::super::{Grid, features};

// ---- command implementations (single responsibility each) ----

/// River polylines → River cells, with a distinct ground-contact Spring at
/// each upstream source. The mouth remains an ordinary mixed transition into
/// its neighboring water body.
/// Painted as an edge PAIR (chain + parallel partner line), like roads: a
/// single chain's derived faces only touch at the chain vertices.
pub(in crate::worldgen) fn paint_rivers(grid: &Grid, terrain: &TerrainGen, cells: &mut [Terrain]) {
    let sea = |t: Terrain| t == Terrain::Ocean || t.is_lake();
    for path in &terrain.river_paths {
        let mut chain = cell_chain(grid, path);
        // The planned endpoint sits on the PROPOSED waterline; normalization
        // may have moved the coast since. If the channel no longer meets open
        // water, extend it from its end along the shortest cell path to the
        // nearest sea/lake cell — a river always reaches a larger body.
        let reaches = chain.iter().any(|&cell| {
            sea(cells[cell.index()])
                || grid
                    .cell_neighbors(cell)
                    .iter()
                    .any(|nb| sea(cells[nb.index()]))
        });
        if !reaches
            && let Some(&end) = chain.last()
            && let Some(extension) = grid
                .topology
                .cell_shortest_path_to(end, 60, |cell| sea(cells[cell.index()]))
        {
            let interior = extension.len().saturating_sub(2);
            chain.extend(extension.into_iter().skip(1).take(interior));
        }
        for cell in features::widen_band(grid, &chain, true) {
            if cells[cell.index()].is_land() {
                cells[cell.index()] = Terrain::River;
            }
        }
        // A triangular three-cell source patch survives face derivation while
        // keeping each Spring face edge-connected (a thinner patch pinches).
        let mut spring_cells: Vec<CellId> = chain.iter().take(2).copied().collect();
        if let [a, b, ..] = spring_cells.as_slice()
            && let Some(third) = grid.cell_neighbors(*a).iter().find(|candidate| {
                **candidate != *a
                    && **candidate != *b
                    && grid.cell_neighbors(*b).contains(candidate)
            })
        {
            spring_cells.push(*third);
        }
        for source in spring_cells {
            if cells[source.index()] == Terrain::River {
                cells[source.index()] = Terrain::RiverSpring;
            }
        }
    }
}

/// The gap-free chain of cells a polyline passes over: nearest corner per
/// sample, gaps bridged along cell adjacency.
pub(in crate::worldgen) fn cell_chain(grid: &Grid, points: &[SpherePos]) -> Vec<CellId> {
    let mut c: Vec<CellId> = Vec::new();
    for seg in points.windows(2) {
        let steps = (seg[0].distance(seg[1]) / 2.0).ceil().max(1.0) as usize;
        for k in 0..=steps {
            let p = seg[0].0.lerp(seg[1].0, k as f32 / steps as f32).normalize();
            let Some(face_index) = grid.planet.face_at(p) else {
                continue;
            };
            let cell = grid
                .face_cells(FaceId::new(face_index))
                .into_iter()
                .max_by(|a, b| {
                    grid.cell_direction(*a)
                        .dot(p)
                        .partial_cmp(&grid.cell_direction(*b).dot(p))
                        .unwrap()
                })
                .unwrap();
            if c.last() == Some(&cell) {
                continue;
            }
            if let Some(&prev) = c.last()
                && !grid.cell_neighbors(prev).contains(&cell)
            {
                c.extend(shortest_cell_path(grid, prev, cell));
            }
            if c.last() != Some(&cell) {
                c.push(cell);
            }
        }
    }
    c
}

pub(super) fn shortest_cell_path(grid: &Grid, from: CellId, to: CellId) -> Vec<CellId> {
    let Some(path) = grid.topology.cell_shortest_path(from, to, 4) else {
        return Vec::new();
    };
    let interior = path.len().saturating_sub(2);
    path.into_iter().skip(1).take(interior).collect()
}
