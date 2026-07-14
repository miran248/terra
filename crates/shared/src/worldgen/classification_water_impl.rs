use super::*;

// ---- command implementations (single responsibility each) ----

/// River polylines → River cells, with a distinct ground-contact Spring at
/// each upstream source. The mouth remains an ordinary mixed transition into
/// its neighboring water body.
/// Painted as an edge PAIR (chain + parallel partner line), like roads: a
/// single chain's derived faces only touch at the chain vertices.
pub(super) fn paint_rivers(grid: &Grid, terrain: &TerrainGen, cells: &mut [Terrain]) {
    let sea = |t: Terrain| matches!(t, Terrain::Ocean | Terrain::Lake);
    for path in &terrain.river_paths {
        let mut chain = cell_chain(grid, path);
        // The planned endpoint sits on the PROPOSED waterline; normalization
        // may have moved the coast since. If the channel no longer meets open
        // water, extend it from its end along the shortest cell path to the
        // nearest sea/lake cell — a river always reaches a larger body.
        let reaches = chain.iter().any(|&cell_index| {
            sea(cells[cell_index])
                || grid
                    .cell_neighbors(CellId::new(cell_index))
                    .iter()
                    .any(|nb| sea(cells[nb.index()]))
        });
        if !reaches
            && let Some(end) = chain.last().and_then(|&index| grid.topology.cell(index))
            && let Some(extension) = grid
                .topology
                .cell_shortest_path_to(end, 60, |cell| sea(cells[cell.index()]))
        {
            let interior = extension.len().saturating_sub(2);
            chain.extend(
                extension
                    .into_iter()
                    .skip(1)
                    .take(interior)
                    .map(|cell| cell.index()),
            );
        }
        for cell_index in widen_band_sym(grid, &chain) {
            if cells[cell_index].is_land() {
                cells[cell_index] = Terrain::River;
            }
        }
        // A triangular three-cell source patch survives face derivation while
        // keeping each Spring face edge-connected (a thinner patch pinches).
        let mut spring_cells: Vec<usize> = chain.iter().take(2).copied().collect();
        if let [a, b, ..] = spring_cells.as_slice()
            && let Some(third) = grid
                .cell_neighbors(CellId::new(*a))
                .iter()
                .find(|candidate| {
                    candidate.index() != *a
                        && candidate.index() != *b
                        && grid.cell_neighbors(CellId::new(*b)).contains(candidate)
                })
        {
            spring_cells.push(third.index());
        }
        for source in spring_cells {
            if cells[source] == Terrain::River {
                cells[source] = Terrain::RiverSpring;
            }
        }
    }
}

/// The gap-free chain of cells a polyline passes over: nearest corner per
/// sample, gaps bridged along cell adjacency.
pub(super) fn cell_chain(grid: &Grid, points: &[SpherePos]) -> Vec<usize> {
    let mut c: Vec<usize> = Vec::new();
    for seg in points.windows(2) {
        let steps = (seg[0].distance(seg[1]) / 2.0).ceil().max(1.0) as usize;
        for k in 0..=steps {
            let p = seg[0].0.lerp(seg[1].0, k as f32 / steps as f32).normalize();
            let Some(face_index) = grid.planet.face_at(p) else {
                continue;
            };
            let cell_index = grid
                .face_cells(FaceId::new(face_index))
                .map(CellId::index)
                .into_iter()
                .max_by(|&a, &b| {
                    grid.cell_direction(CellId::new(a))
                        .dot(p)
                        .partial_cmp(&grid.cell_direction(CellId::new(b)).dot(p))
                        .unwrap()
                })
                .unwrap();
            if c.last() == Some(&cell_index) {
                continue;
            }
            if let Some(&prev) = c.last()
                && !grid
                    .cell_neighbors(CellId::new(prev))
                    .iter()
                    .any(|neighbor| neighbor.index() == cell_index)
            {
                c.extend(shortest_cell_path(grid, prev, cell_index));
            }
            if c.last() != Some(&cell_index) {
                c.push(cell_index);
            }
        }
    }
    c
}

pub(super) fn shortest_cell_path(grid: &Grid, from: usize, to: usize) -> Vec<usize> {
    let Some(from) = grid.topology.cell(from) else {
        return Vec::new();
    };
    let Some(to) = grid.topology.cell(to) else {
        return Vec::new();
    };
    let Some(path) = grid.topology.cell_shortest_path(from, to, 4) else {
        return Vec::new();
    };
    let interior = path.len().saturating_sub(2);
    path.into_iter()
        .skip(1)
        .take(interior)
        .map(|cell| cell.index())
        .collect()
}

/// Majority coarse zone over a cell's face fan (deterministic tie-break).
pub(super) fn cell_zone(
    grid: &Grid,
    terrain: &TerrainGen,
    cell_index: usize,
) -> crate::zones::ZoneKind {
    let mut counts: BTreeMap<u8, usize> = BTreeMap::new();
    let cell = grid
        .topology
        .cell(cell_index)
        .expect("cell index from topology range");
    for &face in grid.topology.cell_faces(cell) {
        *counts
            .entry(terrain.zones().kind_at_fine(face.index()) as u8)
            .or_default() += 1;
    }
    let (&k, _) = counts.iter().max_by_key(|(_, c)| **c).unwrap();
    use crate::zones::ZoneKind::*;
    [Ocean, Continent, Island, Lake, MountainRange, Settlement][k as usize]
}

/// Water-body identity comes from connectivity, not per-face depth: any face can
/// dip below sea level (blending, river carving), but a connected water body is
/// an Ocean only if it reaches ocean-zone faces — otherwise it's an enclosed
/// Lake, whatever its faces individually classified as. Rivers (painted channel
/// faces) are their own linear feature and never merge into either.
pub(super) fn normalize_water_bodies(grid: &Grid, terrain: &TerrainGen, cells: &mut [Terrain]) {
    // Dam every lake rim first: a land-zone cell that classified as water and
    // touches lake-zone water is the start of a drain channel to the sea (they
    // sneak along coarse-face edges past the vertex clamps). Turn it into
    // LakeShore — a one-cell dam that encloses the lake by construction; the
    // elevation solver then lifts it above sea level (LakeShore range).
    let lake_zone: Vec<bool> = (0..grid.cell_count())
        .map(|cell_index| cell_zone(grid, terrain, cell_index) == crate::zones::ZoneKind::Lake)
        .collect();
    let dams: Vec<usize> = (0..grid.cell_count())
        .filter(|&cell_index| {
            cells[cell_index].is_water()
                && !lake_zone[cell_index]
                && grid
                    .cell_neighbors(CellId::new(cell_index))
                    .iter()
                    .any(|nb| {
                        let nb = nb.index();
                        cells[nb].is_water() && lake_zone[nb]
                    })
        })
        .collect();
    for cell_index in dams {
        cells[cell_index] = Terrain::LakeShore;
    }

    enforce_water_shape(grid, cells);

    let mut visited = vec![false; grid.cell_count()];
    for start in 0..grid.cell_count() {
        if !cells[start].is_water()
            || matches!(cells[start], Terrain::River | Terrain::RiverSpring)
            || visited[start]
        {
            continue;
        }
        let body: Vec<usize> = grid
            .topology
            .cell_component(
                grid.topology
                    .cell(start)
                    .expect("cell index from topology range"),
                |cell| {
                    cells[cell.index()].is_water()
                        && !matches!(cells[cell.index()], Terrain::River | Terrain::RiverSpring)
                },
            )
            .into_iter()
            .map(|cell| cell.index())
            .collect();
        for &cell in &body {
            visited[cell] = true
        }
        // A body is an OCEAN only if it reaches ocean-zone cells AND is large
        // enough to be one: an isolated pocket of ocean-zone faces walled off
        // by land (a single coarse face — hence the tell-tale triangle shape)
        // is a lake, not a sea. Min ocean size sits well above any lake.
        let is_ocean = body.len() >= size_range(Terrain::Ocean).0
            && body.iter().any(|&cell_index| {
                cell_zone(grid, terrain, cell_index) == crate::zones::ZoneKind::Ocean
            });
        if !is_ocean && body.len() < size_range(Terrain::Lake).0 {
            // A puddle isn't a lake: fill it with the most common surrounding
            // land kind so no 1-cell water ever survives.
            let mut counts: BTreeMap<u8, usize> = BTreeMap::new();
            for &cell_index in &body {
                for nb in cell_neighbor_indices(grid, cell_index) {
                    let t = cells[nb];
                    if t.is_land() {
                        *counts.entry(t as u8).or_default() += 1;
                    }
                }
            }
            let fill = counts
                .iter()
                .max_by_key(|(_, c)| **c)
                .map(|(&k, _)| Terrain::ALL[k as usize])
                .unwrap_or(Terrain::Plains);
            for &cell_index in &body {
                cells[cell_index] = fill;
            }
            continue;
        }
        for &cell_index in &body {
            cells[cell_index] = if !is_ocean {
                Terrain::Lake
            } else if cells[cell_index] == Terrain::Lake {
                // A lake cell swallowed by the ocean body is just shallow ocean.
                Terrain::Ocean
            } else {
                cells[cell_index]
            };
        }
    }

    // Lakes keep their distance from the sea: any lake cell within ~7 cell
    // steps (~250m ≈ the 10-tile rule) of ocean water becomes land, and a
    // lake trimmed under the minimum size drains entirely.
    let ocean_sources: Vec<_> = grid
        .topology
        .cells()
        .filter(|cell| cells[cell.index()] == Terrain::Ocean)
        .collect();
    let ocean_dist = grid.topology.cell_distances(&ocean_sources, 7);
    let fill_kind = |cells: &[Terrain], cell_index: usize| -> Terrain {
        let mut counts: BTreeMap<u8, usize> = BTreeMap::new();
        for nb in cell_neighbor_indices(grid, cell_index) {
            let t = cells[nb];
            if t.is_land() {
                *counts.entry(t as u8).or_default() += 1;
            }
        }
        counts
            .iter()
            .max_by_key(|(_, c)| **c)
            .map(|(&k, _)| Terrain::ALL[k as usize])
            .unwrap_or(Terrain::Plains)
    };
    for cell_index in 0..grid.cell_count() {
        let cell = grid
            .topology
            .cell(cell_index)
            .expect("cell index from topology range");
        if cells[cell_index] == Terrain::Lake && ocean_dist.cell_steps(cell).is_some() {
            cells[cell_index] = fill_kind(cells, cell_index);
        }
    }
    let mut visited = vec![false; grid.cell_count()];
    for start in 0..grid.cell_count() {
        if cells[start] != Terrain::Lake || visited[start] {
            continue;
        }
        let body: Vec<usize> = grid
            .topology
            .cell_component(
                grid.topology
                    .cell(start)
                    .expect("cell index from topology range"),
                |cell| cells[cell.index()] == Terrain::Lake,
            )
            .into_iter()
            .map(|cell| cell.index())
            .collect();
        for &cell in &body {
            visited[cell] = true
        }
        if body.len() < size_range(Terrain::Lake).0 {
            for &cell_index in &body {
                cells[cell_index] = fill_kind(cells, cell_index);
            }
        }
    }
}

/// Water narrower than a few cells reads as a visual glitch: water within 2
/// steps of land must reach "core" water (≥3 steps from land) within 2 steps,
/// or it is a sliver/neck → land. Rivers are linear by nature and exempt.
/// (Vertex pinches no longer exist: cells are hexagonal, two same-type cells
/// can only meet along an edge.) Iterated because fills can expose new slivers.
pub(super) fn enforce_water_shape(grid: &Grid, cells: &mut [Terrain]) {
    let fill_kind = |cells: &[Terrain], cell_index: usize| -> Terrain {
        let mut counts: BTreeMap<u8, usize> = BTreeMap::new();
        for nb in cell_neighbor_indices(grid, cell_index) {
            let t = cells[nb];
            if t.is_land() {
                *counts.entry(t as u8).or_default() += 1;
            }
        }
        counts
            .iter()
            .max_by_key(|(_, c)| **c)
            .map(|(&k, _)| Terrain::ALL[k as usize])
            .unwrap_or(Terrain::Plains)
    };
    let wet = |t: Terrain| t.is_water() && !matches!(t, Terrain::River | Terrain::RiverSpring);

    for _ in 0..4 {
        let mut changed = false;
        let land_sources: Vec<_> = grid
            .topology
            .cells()
            .filter(|cell| cells[cell.index()].is_land())
            .collect();
        // Cell steps are ~17.5m: with resolution doubled to pack in content,
        // core water sits ≥3 steps from land (half the old physical width).
        let land_dist = grid.topology.cell_distances(&land_sources, 2);
        // Core water reaches outward 2 steps.
        let core_sources: Vec<_> = grid
            .topology
            .cells()
            .filter(|cell| wet(cells[cell.index()]) && land_dist.cell_steps(*cell).is_none())
            .collect();
        let core_reach = grid
            .topology
            .cell_distances_with(&core_sources, 2, |cell| wet(cells[cell.index()]));
        for cell_index in 0..grid.cell_count() {
            let cell = grid
                .topology
                .cell(cell_index)
                .expect("cell index from topology range");
            if wet(cells[cell_index]) && core_reach.cell_steps(cell).is_none() {
                cells[cell_index] = fill_kind(cells, cell_index);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
}

/// Enclosed water smaller than this many cells is filled to land — a couple
/// of tiles of "lake" beside the ocean makes no sense (one cell ≈ two fine
/// faces of area, so 15 cells ≈ the old 30-face minimum).
/// Size envelope, in CELLS, for every tile kind — the area analogue of
/// `elev_range`. `min`: a contiguous region smaller than this is not viable
/// and gets absorbed/filled/renamed away. `max`: a region larger than this is
/// split (only kinds with a splitter — the coastal bands — enforce it; all
/// others are `usize::MAX`, i.e. unbounded). Oceans and lakes are separated
/// here: min-ocean sits one above max-lake, so a small isolated ocean-zone
/// pocket falls through to Lake.
pub(super) fn size_range(t: Terrain) -> (usize, usize) {
    classification::size_range(t)
}

/// Nearest cell to a point: the closest corner of the face under it.
pub(super) fn nearest_cell(grid: &Grid, p: SpherePos) -> Option<usize> {
    let face_index = grid.planet.face_at(p.0)?;
    grid.face_cells(FaceId::new(face_index))
        .map(CellId::index)
        .into_iter()
        .max_by(|&a, &b| {
            grid.cell_direction(CellId::new(a))
                .dot(p.0)
                .partial_cmp(&grid.cell_direction(CellId::new(b)).dot(p.0))
                .unwrap()
        })
}

/// Lattice-aligned routing: A* over cells where changing direction costs
/// extra, so paths run straight along one of the 3 lattice axes and turn in
/// discrete 60° steps spaced apart (the lattice dictates the turning radius).
/// Water cells are impassable — roads route around lakes on the lattice.
/// Returns the chain including both endpoints, or empty if unreachable.
/// Macro landform per cell from the PROPOSED field: elevation bands
/// (lowland/hills/mountains) split by local relief (a flat high patch is a
/// plateau), with low ground boxed in by higher ground marked as valley.
/// Because it reads a smooth continuous field the bands are naturally ordered
/// (no lowland directly against a peak). Speckle is absorbed into the
/// surrounding landform so each is a coherent cluster.
pub(super) fn classify_landform(grid: &Grid, terrain: &TerrainGen) -> Vec<u8> {
    let e: Vec<f32> = (0..grid.cell_count())
        .map(|cell_index| terrain.elevation_at(grid.cell_position(CellId::new(cell_index))))
        .collect();
    let mut lf = vec![LANDFORM_WATER; grid.cell_count()];
    for cell_index in 0..grid.cell_count() {
        if e[cell_index] < 0.0 {
            continue; // water
        }
        let relief = cell_neighbor_indices(grid, cell_index)
            .map(|nb| (e[cell_index] - e[nb]).abs())
            .fold(0.0f32, f32::max);
        lf[cell_index] = if e[cell_index] >= 0.35 {
            if relief < 0.035 {
                LANDFORM_PLATEAU
            } else {
                LANDFORM_MOUNTAINS
            }
        } else if e[cell_index] >= 0.14 {
            LANDFORM_HILLS
        } else {
            LANDFORM_LOWLAND
        };
    }
    // Valleys: low ground hemmed in by higher landform on most sides.
    let higher = |l: u8| matches!(l, LANDFORM_HILLS | LANDFORM_MOUNTAINS | LANDFORM_PLATEAU);
    let mut valleys = Vec::new();
    for cell_index in 0..grid.cell_count() {
        if lf[cell_index] == LANDFORM_LOWLAND
            && cell_neighbor_indices(grid, cell_index)
                .filter(|&nb| higher(lf[nb]))
                .count()
                >= 3
        {
            valleys.push(cell_index);
        }
    }
    for cell_index in valleys {
        lf[cell_index] = LANDFORM_VALLEY;
    }
    // Absorb speckle: a landform cluster below the min joins its most common
    // land neighbor's landform, so each landform is a coherent region.
    absorb_small_landforms(grid, &mut lf);
    lf
}

pub(super) const MIN_LANDFORM_CELLS: usize = 25;

pub(super) fn absorb_small_landforms(grid: &Grid, lf: &mut [u8]) {
    absorb_small_clusters(
        grid,
        lf,
        |l| l != LANDFORM_WATER,
        |_| MIN_LANDFORM_CELLS,
        |l| l != LANDFORM_WATER,
    );
}

/// Cover per cell: the macro landform sets the base — high ground (mountains,
/// plateaus) gets rock/snow/ice/volcanic, everything lower gets a climate
/// biome. Water zones stay water. This is the landform → biome layering.
pub(super) fn classify_cover(
    grid: &Grid,
    terrain: &TerrainGen,
    landform: &[u8],
    cell_index: usize,
) -> Terrain {
    let pos = grid.cell_position(CellId::new(cell_index));
    let e = terrain.elevation_at(pos);
    match cell_zone(grid, terrain, cell_index) {
        crate::zones::ZoneKind::Ocean => Terrain::Ocean,
        crate::zones::ZoneKind::Lake => {
            if e < 0.0 {
                Terrain::Lake
            } else {
                land_cover(terrain, landform[cell_index], pos)
            }
        }
        _ => {
            if e < 0.0 {
                Terrain::Ocean
            } else {
                land_cover(terrain, landform[cell_index], pos)
            }
        }
    }
}

pub(super) fn land_cover(terrain: &TerrainGen, landform: u8, pos: SpherePos) -> Terrain {
    let t = terrain.temperature_at(pos);
    let m = terrain.moisture_at(pos);
    let e = terrain.elevation_at(pos);
    // A mountain is not one thing: snow caps the high cells (above the snow
    // line), forest clothes the warm/wet lower flanks, bare rock fills the
    // rugged middle, ice covers the cold ranges, and hot high peaks are
    // volcanic. So a single range shows rock AND snow AND (sometimes) forest.
    if matches!(landform, LANDFORM_MOUNTAINS | LANDFORM_PLATEAU) {
        return if t < -12.0 {
            Terrain::Glacier
        } else if e > 0.70 {
            if t > 24.0 {
                Terrain::Volcanic
            } else {
                Terrain::Snow
            }
        } else if t < -2.0 {
            Terrain::Snow
        } else if m > 0.15 && e < 0.55 {
            Terrain::Forest
        } else {
            Terrain::Mountain
        };
    }
    // Low/rolling ground: climate biome cover.
    if t < -28.0 {
        return Terrain::Glacier;
    }
    if t < -15.0 {
        return Terrain::Snow;
    }
    if t < 0.0 {
        return Terrain::Tundra;
    }
    if e < 0.12 && m > 0.28 {
        return Terrain::Swamp;
    }
    if t > 24.0 && m > 0.25 {
        return Terrain::Jungle;
    }
    if t > 30.0 && m < -0.15 {
        return Terrain::Desert;
    }
    if t > 22.0 && m < 0.05 {
        return Terrain::Savanna;
    }
    if m > 0.10 {
        Terrain::Forest
    } else {
        Terrain::Plains
    }
}

/// Slope-class thresholds (rise/run ≈ tan angle) on the solved field.
pub(super) const SLOPE_GENTLE_MAX: f32 = 0.18; // ~10°: flat/gentle boundary
pub(super) const SLOPE_STEEP_MAX: f32 = 0.45; // ~24°: gentle/steep (walkable) boundary
pub(super) const SLOPE_CLIFF_MAX: f32 = 0.90; // ~42°: steep/cliff (impassable) boundary

/// Per-cell steepness of the SOLVED surface — the micro landform layer.
/// Measured at CELL scale (max rise/run to an edge-neighbor over the real
/// ground distance), not at a sub-metre probe, so it reflects terrain the
/// player traverses rather than interpolation noise. Passes (gentle cells in
/// mountains) and escarpments (cliff cells) fall out of it automatically.
pub(super) fn classify_slope(grid: &Grid, terrain: &TerrainGen) -> Vec<u8> {
    let alt: Vec<f32> = (0..grid.cell_count())
        .map(|cell_index| terrain.altitude(grid.cell_position(CellId::new(cell_index))))
        .collect();
    (0..grid.cell_count())
        .map(|cell_index| {
            let a = grid.cell_direction(CellId::new(cell_index));
            let mut worst = 0.0f32;
            for nb in cell_neighbor_indices(grid, cell_index) {
                let dist =
                    a.distance(grid.cell_direction(CellId::new(nb))) * crate::sphere::PLANET_RADIUS;
                if dist > 1.0 {
                    worst = worst.max((alt[cell_index] - alt[nb]).abs() / dist);
                }
            }

            bucket(worst, &[SLOPE_GENTLE_MAX, SLOPE_STEEP_MAX, SLOPE_CLIFF_MAX])
        })
        .collect()
}

/// The number of ascending `thresholds` a value reaches — turns a measurement
/// into an ordered class (flat/gentle/steep/cliff, shallow/deep/abyss).
pub(super) fn bucket(value: f32, thresholds: &[f32]) -> u8 {
    classification::bucket(value, thresholds)
}

/// Per-water-cell depth class from the solved surface: shore-shallows deepen
/// to abyss offshore (and lake/river beds shallow-to-deep by their concavity).
/// Land cells are DEPTH_SHALLOW (unused). The depth analogue of slope class.
pub(super) fn classify_water_depth(
    grid: &Grid,
    cells: &[Terrain],
    terrain: &TerrainGen,
) -> Vec<u8> {
    (0..grid.cell_count())
        .map(|cell_index| {
            if !cells[cell_index].is_water() {
                return DEPTH_SHALLOW;
            }
            let e = terrain.elevation_at(grid.cell_position(CellId::new(cell_index)));
            bucket(-e, &[0.20, 0.55])
        })
        .collect()
}
