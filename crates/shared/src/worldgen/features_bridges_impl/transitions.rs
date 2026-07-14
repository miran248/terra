use super::super::*;

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
            Terrain::Lake => Terrain::LakeShore,
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
    let mut wfc_cells: Vec<usize> = Vec::new();
    for cell_index in 0..grid.cell_count() {
        if base[cell_index].is_water() || in_band[cell_index] {
            continue;
        }
        if grid
            .cell_neighbors(CellId::new(cell_index))
            .iter()
            .any(|nb| base[nb.index()] != base[cell_index])
        {
            cell_of[cell_index] = wfc_cells.len();
            wfc_cells.push(cell_index);
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
        .map(|&cell_index| {
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
        .map(|&cell_index| {
            cell_neighbor_indices(grid, cell_index)
                .map(|nb| match cell_of[nb] {
                    usize::MAX => wfc::Neighbor::Fixed(base[nb]),
                    ci => wfc::Neighbor::Cell(ci),
                })
                .collect()
        })
        .collect();
    let fallback: Vec<Terrain> = wfc_cells
        .iter()
        .map(|&cell_index| base[cell_index])
        .collect();

    let solved = wfc::solve(
        &wfc::Compat::default(),
        &domains,
        &neighbors,
        &fallback,
        grid.seed as u64,
    );

    let mut resolved = base.clone();
    for (ci, &cell_index) in wfc_cells.iter().enumerate() {
        resolved[cell_index] = solved[ci];
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
    let lake = dist_to(&|t| t == Terrain::Lake);
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
        let mut counts: BTreeMap<u8, usize> = BTreeMap::new();
        for nb in cell_neighbor_indices(grid, cell_index) {
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
                *counts.entry(t as u8).or_default() += 1;
            }
        }
        cells[cell_index] = counts
            .iter()
            .max_by_key(|(_, c)| **c)
            .map(|(&k, _)| Terrain::ALL[k as usize])
            .unwrap_or(Terrain::Plains);
    }
}

/// Inland biome boundaries get a blend mark: the face keeps its derived
/// kind, but carries the pair it links so rendering/solving can transition
/// between the two. With cell-based tiles the boundary faces are simply the
/// faces whose corner cells disagree — edge-connected strips by construction.
/// Faces flanking a built feature blend toward it instead (feature codes).
pub(in crate::worldgen) fn mark_blends(
    grid: &Grid,
    cells: &[Terrain],
    tiles: &[Terrain],
    painted: &Painted,
) -> Vec<(u32, u8, u8)> {
    let plain = |t: Terrain| t.is_land();
    let overlay = |face_index: usize| {
        face_solid(grid, &painted.roads, face_index)
            || face_solid(grid, &painted.towns, face_index)
            || face_solid(grid, &painted.bridge_entries, face_index)
    };
    let mut out = Vec::new();
    for (face_index, &tile) in tiles.iter().enumerate().take(grid.face_count()) {
        if !plain(tile) || overlay(face_index) {
            continue;
        }
        // Feature flanks (a painted corner without ownership) blend toward the
        // feature; most specific wins (entry pad < town blob < road network).
        let feature = if painted_corners(grid, &painted.bridge_entries, face_index) > 0 {
            Some(crate::level::BLEND_BRIDGE_ENTRY)
        } else if painted_corners(grid, &painted.towns, face_index) > 0 {
            Some(crate::level::BLEND_TOWN)
        } else if painted_corners(grid, &painted.roads, face_index) > 0 {
            Some(crate::level::BLEND_ROAD)
        } else {
            None
        };
        if let Some(code) = feature {
            out.push((face_index as u32, tiles[face_index] as u8, code));
            continue;
        }
        // Corner cells that disagree with the face's derived kind: the face is
        // the linking tile between its kind and the most present other LAND
        // kind (water transitions are the shore band's job).
        let mut counts: BTreeMap<u8, usize> = BTreeMap::new();
        for cell_index in grid.face_cells(FaceId::new(face_index)).map(CellId::index) {
            let t = cells[cell_index];
            if plain(t) && t != tiles[face_index] {
                *counts.entry(t as u8).or_default() += 1;
            }
        }
        if let Some((&other, _)) = counts.iter().max_by_key(|(_, c)| **c) {
            out.push((face_index as u32, tiles[face_index] as u8, other));
        }
    }
    out
}

/// A biome patch smaller than this many cells is speckle, not a region —
/// threshold classifiers (snow by temperature, mountains by elevation)
/// salt-and-pepper at their contour lines without this.
/// Generic min-region-size for every plain land biome: undersized clusters
/// join their most common neighboring land kind. Transition bands (shore
/// kinds) are thin by design and exempt; water minimums live in
/// normalize_water_bodies.
/// Temporary index bridge for geometry-heavy algorithms whose state arrays are
/// still densely indexed by cell.
pub(in crate::worldgen) fn cell_neighbor_indices(
    grid: &Grid,
    v: usize,
) -> impl Iterator<Item = usize> + '_ {
    grid.cell_neighbors(CellId::new(v))
        .iter()
        .map(|cell| cell.index())
}

/// A contiguous cluster of equal values below its minimum size is speckle: it
/// is absorbed into its most common eligible neighbor value. Generic over the
/// value type (biome cover, landform, …). `eligible` selects which values
/// participate, `min_size` gives each value's floor, `absorbable` says which
/// neighbor values a speckle may merge into.
pub(in crate::worldgen) fn absorb_small_clusters<T: Copy + Ord>(
    grid: &Grid,
    out: &mut [T],
    eligible: impl Fn(T) -> bool,
    min_size: impl Fn(T) -> usize,
    absorbable: impl Fn(T) -> bool,
) {
    let mut visited = vec![false; grid.cell_count()];
    for start in 0..grid.cell_count() {
        if !eligible(out[start]) || visited[start] {
            continue;
        }
        let kind = out[start];
        let cluster: Vec<usize> = grid
            .topology
            .cell_component(
                grid.topology
                    .cell(start)
                    .expect("cell index from topology range"),
                |cell| out[cell.index()] == kind && !visited[cell.index()],
            )
            .into_iter()
            .map(|cell| cell.index())
            .collect();
        for &cell in &cluster {
            visited[cell] = true
        }
        if cluster.len() >= min_size(kind) {
            continue;
        }
        let mut counts: BTreeMap<T, usize> = BTreeMap::new();
        for &cell_index in &cluster {
            for nb in cell_neighbor_indices(grid, cell_index) {
                let t = out[nb];
                if t != kind && absorbable(t) {
                    *counts.entry(t).or_default() += 1;
                }
            }
        }
        if let Some((&k, _)) = counts.iter().max_by_key(|(_, c)| **c) {
            for cell_index in cluster {
                out[cell_index] = k;
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
    absorb_small_clusters(grid, out, plain, |t| size_range(t).0, |t| t.is_land());
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
            for nb in cell_neighbor_indices(grid, cell_index) {
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
