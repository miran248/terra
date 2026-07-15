use std::collections::BTreeMap;

use crate::terrain::{Terrain, TerrainGen};
use crate::topology::CellId;
use crate::wfc;
use crate::worldgen::{Grid, link_tile_pinches};

use super::clusters::{absorb_small_patches, smooth_coast_band, water_distance};

pub(in crate::worldgen) fn resolve_transitions(
    grid: &Grid,
    terrain: &TerrainGen,
    base: &[Terrain],
) -> Vec<Terrain> {
    // Shorelines are deterministic bands, not WFC cells: every land cell
    // touching water gets its shore tile, so the waterline is never zigzagged
    // by chance. The band widens onto the second ring where the coast is flat,
    // and a land cell wedged between two shore cells joins the band.
    let mut out = base.to_vec();
    // Ocean is one identity now; DEPTH is a per-cell class derived from the
    // solved field afterwards (shallow near shore → abyss offshore, via the
    // shelf constraint). Deep water never surfaces because Ocean's range floor
    // deepens with distance from land — no separate tile, no margin pass.
    let (water_dist, water_kind) = water_distance(grid, base, 2);
    let shore = |cell_index: usize, kind: Terrain| -> Terrain {
        match kind {
            Terrain::Lake | Terrain::SaltLake => Terrain::LakeShore,
            Terrain::River | Terrain::RiverSpring => Terrain::RiverBank,
            _ => {
                let steep = matches!(base[cell_index], Terrain::Mountain | Terrain::Snow)
                    || terrain.elevation_at(grid.cell_position(CellId::new(cell_index))) > 0.15;
                if steep {
                    Terrain::Cliff
                } else {
                    Terrain::Beach
                }
            }
        }
    };
    // Every transition band is TWO chains of cells one edge apart (never a
    // single chain — its faces would only touch at vertices): the waterline
    // chain plus the chain right behind it, for beaches, cliffs, lake shores
    // and river banks alike. Wide types (oceans, lakes, rivers, towns) are
    // free-width; bands are not.
    for cell_index in 0..grid.cell_count() {
        if base[cell_index].is_water() {
            continue;
        }
        if water_dist[cell_index] <= 2 {
            out[cell_index] = shore(cell_index, water_kind[cell_index].unwrap_or(Terrain::Ocean));
        }
    }
    // Fill notches: a land cell with ≥2 neighbors in the shore band belongs
    // to the band too.
    let banded: Vec<bool> = (0..grid.cell_count())
        .map(|cell_index| {
            matches!(
                out[cell_index],
                Terrain::Beach | Terrain::Cliff | Terrain::LakeShore | Terrain::RiverBank
            )
        })
        .collect();
    for cell_index in 0..grid.cell_count() {
        if base[cell_index].is_water() || banded[cell_index] {
            continue;
        }
        if grid
            .cell_neighbors(CellId::new(cell_index))
            .iter()
            .filter(|nb| banded[nb.index()])
            .count()
            >= 2
        {
            out[cell_index] = shore(cell_index, water_kind[cell_index].unwrap_or(Terrain::Ocean));
        }
    }

    // WFC over inland biome edges: land cells bordering a different
    // classification. Water and the shore band enter as fixed neighbors.
    let in_band: Vec<bool> = (0..grid.cell_count())
        .map(|cell_index| out[cell_index] != base[cell_index])
        .collect();
    let base = &out;
    let mut cell_of = vec![usize::MAX; grid.cell_count()];
    let mut wfc_cells: Vec<CellId> = Vec::new();
    for cell in grid.topology.cells() {
        let cell_index = cell.index();
        if base[cell_index].is_water() || in_band[cell_index] {
            continue;
        }
        if grid
            .cell_neighbors(CellId::new(cell_index))
            .iter()
            .any(|nb| base[nb.index()] != base[cell_index])
        {
            cell_of[cell_index] = wfc_cells.len();
            wfc_cells.push(cell);
        }
    }

    let transition_tiles = [
        Terrain::Beach,
        Terrain::Cliff,
        Terrain::LakeShore,
        Terrain::RiverBank,
    ];
    let domains: Vec<Vec<(Terrain, f32)>> = wfc_cells
        .iter()
        .map(|&cell| {
            let cell_index = cell.index();
            let mut d = vec![(base[cell_index], 1.0)];
            for t in transition_tiles {
                if t != base[cell_index] {
                    d.push((t, 0.3));
                }
            }
            d
        })
        .collect();
    let neighbors: Vec<Vec<wfc::Neighbor>> = wfc_cells
        .iter()
        .map(|&cell| {
            grid.cell_neighbors(cell)
                .iter()
                .map(|cell| cell.index())
                .map(|nb| match cell_of[nb] {
                    usize::MAX => wfc::Neighbor::Fixed(base[nb]),
                    ci => wfc::Neighbor::Cell(ci),
                })
                .collect()
        })
        .collect();
    let fallback: Vec<Terrain> = wfc_cells.iter().map(|&cell| base[cell.index()]).collect();

    let solved = wfc::solve(
        &wfc::Compat::default(),
        &domains,
        &neighbors,
        &fallback,
        grid.seed as u64,
    );

    let mut resolved = base.clone();
    for (ci, &cell) in wfc_cells.iter().enumerate() {
        resolved[cell.index()] = solved[ci];
    }
    smooth_coast_band(grid, &mut resolved);
    absorb_small_patches(grid, &mut resolved);
    prune_orphan_bands(grid, &mut resolved);
    link_tile_pinches(grid, &mut resolved);
    resolved
}

/// A transition band without its water is not a transition: a river bank
/// needs a River within band reach (2 cells), a lake shore a Lake, a beach
/// or cliff the sea. Orphans (left behind when fills/trims move the water)
/// join their most common plain land neighbor.
pub(in crate::worldgen) fn prune_orphan_bands(grid: &Grid, cells: &mut [Terrain]) {
    let dist_to = |pred: &dyn Fn(Terrain) -> bool| -> Vec<u8> {
        let sources: Vec<_> = grid
            .topology
            .cells()
            .filter(|cell| pred(cells[cell.index()]))
            .collect();
        let field = grid.topology.cell_distances(&sources, 2);
        grid.topology
            .cells()
            .map(|cell| field.cell_steps(cell).map_or(u8::MAX, |steps| steps as u8))
            .collect()
    };
    let river = dist_to(&|t| t == Terrain::River);
    let lake = dist_to(&|terrain| terrain.is_lake());
    let sea = dist_to(&|t| t == Terrain::Ocean);
    for cell_index in 0..grid.cell_count() {
        let orphan = match cells[cell_index] {
            Terrain::RiverBank => river[cell_index] > 2,
            Terrain::LakeShore => lake[cell_index] > 2,
            Terrain::Beach | Terrain::Cliff => {
                sea[cell_index] > 2 && lake[cell_index] > 2 && river[cell_index] > 2
            }
            _ => false,
        };
        if !orphan {
            continue;
        }
        let mut counts: BTreeMap<Terrain, usize> = BTreeMap::new();
        for nb in grid
            .cell_neighbors(CellId::new(cell_index))
            .iter()
            .map(|cell| cell.index())
        {
            let t = cells[nb];
            if matches!(
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
            ) {
                *counts.entry(t).or_default() += 1;
            }
        }
        cells[cell_index] = counts
            .iter()
            .max_by_key(|(_, c)| **c)
            .map(|(&k, _)| Terrain::ALL[k as usize])
            .unwrap_or(Terrain::Plains);
    }
}
