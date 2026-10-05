/// A contiguous cluster below its minimum size is absorbed into its most
/// common eligible neighbor. The explicit rank preserves deterministic ties.
pub(in crate::worldgen) fn absorb_small_clusters<T: Copy + Eq>(
    grid: &Grid,
    out: &mut [T],
    eligible: impl Fn(T) -> bool,
    min_size: impl Fn(T) -> usize,
    absorbable: impl Fn(T) -> bool,
    rank: impl Fn(T) -> u8,
) {
    let mut visited = vec![false; grid.cell_count()];
    for start in grid.topology.cells() {
        if !eligible(out[start.index()]) || visited[start.index()] {
            continue;
        }
        let kind = out[start.index()];
        let cluster: Vec<CellId> = grid
            .topology
            .cell_component(start, |cell| {
                out[cell.index()] == kind && !visited[cell.index()]
            })
            .into_iter()
            .collect();
        for &cell in &cluster {
            visited[cell.index()] = true
        }
        if cluster.len() >= min_size(kind) {
            continue;
        }
        let mut counts: BTreeMap<u8, (T, usize)> = BTreeMap::new();
        for &cell in &cluster {
            for &neighbor in grid.cell_neighbors(cell) {
                let t = out[neighbor.index()];
                if t != kind && absorbable(t) {
                    counts
                        .entry(rank(t))
                        .and_modify(|(_, count)| *count += 1)
                        .or_insert((t, 1));
                }
            }
        }
        if let Some((_, &(kind, _))) = counts.iter().max_by_key(|(_, (_, count))| *count) {
            for cell in cluster {
                out[cell.index()] = kind;
            }
        }
    }
}

pub(in crate::worldgen) fn absorb_small_patches(grid: &Grid, out: &mut [Terrain]) {
    let plain = |t: Terrain| {
        matches!(
            t,
            Terrain::Desert
                | Terrain::Plains
                | Terrain::Forest
                | Terrain::Tundra
                | Terrain::Mountain
                | Terrain::Snow
                | Terrain::Swamp
                | Terrain::Jungle
                | Terrain::Savanna
                | Terrain::Volcanic
                | Terrain::Glacier
        )
    };
    absorb_small_clusters(
        grid,
        out,
        plain,
        |terrain| size_range(terrain).0,
        |terrain| terrain.is_land(),
        classification::terrain_rank,
    );
}

/// The coast band is PROACTIVE: Beach vs Cliff was already decided by the
/// proposed elevation field (high ground meeting water is a cliff, low ground
/// a beach — the ground is raised first, the label follows). This pass only
/// smooths single-cell islands in the band: a lone beach cell between two
/// cliffs joins them, and vice versa.
pub(in crate::worldgen) fn smooth_coast_band(grid: &Grid, out: &mut [Terrain]) {
    for _ in 0..8 {
        let mut changed = false;
        for cell_index in 0..grid.cell_count() {
            if !matches!(out[cell_index], Terrain::Beach | Terrain::Cliff) {
                continue;
            }
            let mut same = 0;
            let mut other = 0;
            for nb in grid
                .cell_neighbors(CellId::new(cell_index))
                .iter()
                .map(|cell| cell.index())
            {
                match out[nb] {
                    t if t == out[cell_index] => same += 1,
                    Terrain::Beach | Terrain::Cliff => other += 1,
                    _ => {}
                }
            }
            if same == 0 && other >= 2 {
                out[cell_index] = if out[cell_index] == Terrain::Beach {
                    Terrain::Cliff
                } else {
                    Terrain::Beach
                };
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
}

/// BFS distance (in cell steps, capped at `max_dist`) from each land cell to the
/// nearest water cell, plus which water terrain is nearest (for shore tile choice).
pub(in crate::worldgen) fn water_distance(
    grid: &Grid,
    base: &[Terrain],
    max_dist: u8,
) -> (Vec<u8>, Vec<Option<Terrain>>) {
    let sources: Vec<_> = grid
        .topology
        .cells()
        .filter(|cell| base[cell.index()].is_water())
        .collect();
    let field = grid.topology.cell_distances(&sources, u32::from(max_dist));
    let dist = grid
        .topology
        .cells()
        .map(|cell| field.cell_steps(cell).map_or(u8::MAX, |steps| steps as u8))
        .collect();
    let kind = grid
        .topology
        .cells()
        .map(|cell| field.nearest_cell(cell).map(|source| base[source.index()]))
        .collect();
    (dist, kind)
}
use std::collections::BTreeMap;

use crate::worldgen::{Grid, classification, size_range};
use terra_geometry::topology::CellId;
use terra_world::terrain::Terrain;
