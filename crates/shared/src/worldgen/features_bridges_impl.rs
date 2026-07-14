use super::*;

/// A band needs two parallel lattice lines. Widen the chain with each edge's
/// side partner so faces between the lines form a gap-free feature strip.
pub(super) fn widen_band(grid: &Grid, chain: &[usize]) -> Vec<usize> {
    features::widen_band(grid, chain, false)
}

/// Three lattice lines: the chain plus BOTH side partners (~105m) — wide
/// enough that the elevation solver owns distinct channel and bank verts and
/// can actually carve a cross-section (rivers).
pub(super) fn widen_band_sym(grid: &Grid, chain: &[usize]) -> Vec<usize> {
    features::widen_band(grid, chain, true)
}

/// Roads may not cross water: a planned path whose cell chain touches a water
/// cell is dropped entirely (crossing there needs a bridge, not a road).
pub(super) fn paint_features(
    grid: &Grid,
    terrain: &TerrainGen,
    cells: &[Terrain],
    slope_class: &[u8],
) -> (Painted, Vec<Vec<SpherePos>>) {
    let mut painted = Painted::empty(grid.cell_count());
    let mut kept: Vec<Vec<SpherePos>> = Vec::new();
    // Roads route on WALKABLE ground: they follow valleys and mountain passes
    // and refuse water and steep slopes (conform, don't carve). A leg that has
    // no gentle dry route is dropped — that gap wants a bridge.
    // Roads may not cross water or a cliff, and are steered strongly toward
    // gentle ground (steep cells cost extra), so they follow valleys and
    // passes but can still climb a slope when they must.
    let blocked = |cell: crate::topology::CellId| {
        cells[cell.index()].is_water() || slope_class[cell.index()] == SLOPE_CLIFF
    };
    let extra = |cell: crate::topology::CellId| match slope_class[cell.index()] {
        SLOPE_FLAT => 0,
        SLOPE_GENTLE => 400,
        _ => 4000, // steep
    };
    'paths: for path in &terrain.road_paths {
        let mut chain: Vec<usize> = Vec::new();
        let waypoints: Vec<usize> = path.iter().filter_map(|p| nearest_cell(grid, *p)).collect();
        for leg in waypoints.windows(2) {
            let from = grid.topology.cell(leg[0]).expect("waypoint cell id");
            let to = grid.topology.cell(leg[1]).expect("waypoint cell id");
            let seg: Vec<usize> = router::lattice_path(grid, blocked, extra, from, to)
                .into_iter()
                .map(|cell| cell.index())
                .collect();
            if seg.is_empty() {
                continue 'paths;
            }
            let skip = usize::from(chain.last() == seg.first());
            chain.extend(&seg[skip..]);
        }
        let band = widen_band(grid, &chain);
        if band.iter().any(|&cell_index| {
            cells[cell_index].is_water() || slope_class[cell_index] == SLOPE_CLIFF
        }) {
            continue;
        }
        for cell_index in band {
            painted.roads.insert(cell_index);
        }
        kept.push(path.clone());
    }
    // Towns sit on walkable ground within the settlement radius.
    for (cell_index, &slope) in slope_class.iter().enumerate().take(grid.cell_count()) {
        let pos = grid.cell_position(cell_index);
        if slope_walkable(slope)
            && terrain
                .settlement_anchors
                .iter()
                .any(|a| a.distance(pos) <= TOWN_RADIUS)
        {
            painted.towns.insert(cell_index);
        }
    }
    link_feature_pinches(grid, &mut painted.roads, |cell_index| {
        cells[cell_index].is_land()
    });
    link_feature_pinches(grid, &mut painted.towns, |_| true);
    (painted, kept)
}

/// Gaps up to this bridge freely.
/// A tiny overshoot past the gentle anchor cell so the deck grounds just
/// inside solid ground (bridges conform — no long inland ramp).
pub(super) const BRIDGE_ENTRY_OVERLAP: f32 = 3.0;
/// A river/lake crossing longer than this isn't a bridge — the water is too
/// wide (that would be a ferry, not a footbridge).
pub(super) const BRIDGE_MAX_SPAN: f32 = 200.0;
/// Keep bridges apart: no two within this many meters.
pub(super) const BRIDGE_MIN_SPACING: f32 = 400.0;
pub(super) const BRIDGE_MAX_COUNT: usize = 12;
/// A land component smaller than this, ringed only by lake water, is an island
/// in the lake and earns a bridge to the mainland.
pub(super) const LAKE_ISLAND_MAX_CELLS: usize = 1500;
/// An ocean island (small land ringed only by ocean) is bridged to the nearest
/// other landmass, but only across a SHORT gap (islands are seeded near land).
pub(super) const OCEAN_ISLAND_MAX_CELLS: usize = 3000;
pub(super) const OCEAN_BRIDGE_MAX_SPAN: f32 = 700.0;
/// A bridge FOOTING must be this gentle (footing-scale slope, rise/run) — the
/// immediate spot the deck grounds on. Measured at footing scale (not cell
/// scale) because a bank cell is locally steep toward the channel yet has a
/// flat footing on top.
pub(super) const BRIDGE_MAX_FOOTING_SLOPE: f32 = 0.25;

/// Terrain a bridge may land on: gentle, walkable ground — never a mountain,
/// cliff, snowfield, glacier or volcanic slope.
pub(super) fn bridge_walkable(t: Terrain) -> bool {
    features::bridge_walkable(t)
}

/// Walk straight across a water band from walkable ground `land`, entering the
/// band at `first`, following the initial heading cell-to-cell until walkable
/// ground is reached on the FAR side. `band` says which kinds are the crossing
/// (river+its banks, or lake+its shores). Returns the far-side walkable cell,
/// or None if the band doesn't end in walkable ground within `max` steps (e.g.
/// it runs into a mountain, or the band is too wide).
pub(super) fn cross_band(
    grid: &Grid,
    cells: &[Terrain],
    land: usize,
    first: usize,
    max: usize,
    band: impl Fn(Terrain) -> bool,
    is_end: impl Fn(Terrain) -> bool,
) -> Option<usize> {
    let tangent = |from: Vec3, step: Vec3| {
        let s = step - from * step.dot(from);
        s.normalize_or_zero()
    };
    let heading = tangent(
        grid.cell_direction(land),
        grid.cell_direction(first) - grid.cell_direction(land),
    );
    if heading == Vec3::ZERO {
        return None;
    }
    let (mut prev, mut cur) = (land, first);
    for _ in 0..max {
        if is_end(cells[cur]) {
            return Some(cur);
        }
        // Only cross the intended band; anything else (open ocean, a mountain
        // foot) aborts — no bridge there.
        if !band(cells[cur]) {
            return None;
        }
        let cpos = grid.cell_direction(cur);
        let mut best = None;
        let mut best_dot = -2.0;
        for nb in cell_neighbor_indices(grid, cur) {
            if nb == prev {
                continue;
            }
            let d = tangent(cpos, grid.cell_direction(nb) - cpos).dot(heading);
            if d > best_dot {
                best_dot = d;
                best = Some(nb);
            }
        }
        prev = cur;
        cur = best?;
    }
    None
}

/// Bridges cross WATER LOCALLY:/// Bridges cross WATER LOCALLY: over a river from one walkable bank to the
/// other, and over a lake to reach an island within it. Never oceans, never
/// mountains — a short footbridge on gentle ground.
pub(super) fn build_bridges(
    grid: &Grid,
    terrain: &TerrainGen,
    cells: &[Terrain],
    slope_class: &[u8],
    _face_types: &[Terrain],
    _face_region: &[u32],
    painted: &mut Painted,
) -> Vec<Vec<SpherePos>> {
    let mut spans: Vec<Vec<SpherePos>> = Vec::new();
    let mut mids: Vec<SpherePos> = Vec::new();
    // A deck grounds cleanly only on gentle ground: reject an endpoint whose
    // proposed elevation differs sharply from its land neighbors (a steep bank
    // shoulder the entry pad couldn't flatten). Proposed field is set by now.
    // One walkable cell further from `away`, so the deck grounds inland of the
    // steep bank edge (the pad there sits on flat ground, clear of the carved
    // channel). Falls back to the cell itself if no inland walkable neighbor.
    let inland1 = |cell_index: usize, away: Vec3| {
        cell_neighbor_indices(grid, cell_index)
            .filter(|&nb| bridge_walkable(cells[nb]))
            .max_by(|&a, &b| {
                grid.cell_direction(a)
                    .distance(away)
                    .partial_cmp(&grid.cell_direction(b).distance(away))
                    .unwrap()
            })
            .unwrap_or(cell_index)
    };
    let inland = inland1;
    // A deck grounds cleanly only on an open, gentle patch: the anchor cell
    // and all its neighbors must be walkable land (no sea, shore band, cliff,
    // mountain, snow, glacier or volcanic rock anywhere in the ring) with a
    // small proposed-elevation spread. This keeps bridges inland on flat
    // ground and away from mouths, coasts and steep shoulders.
    // Forbidden around an anchor: steep ground and the open sea/coast. The
    // crossing band itself (river/lake and their shores) is fine — that is what
    // the bridge spans.
    // Forbidden around an anchor: mountains and the open sea/beach. Cliffs are
    // now ALLOWED — a bridge may start on a clifftop (canyon rim), as long as
    // the footing itself is gentle enough (checked below).
    let forbidden = |t: Terrain| {
        matches!(
            t,
            Terrain::Mountain
                | Terrain::Snow
                | Terrain::Glacier
                | Terrain::Volcanic
                | Terrain::Ocean
                | Terrain::Beach
        )
    };
    // The field is SOLVED by now, so gate on the REAL slope: a deck anchor
    // must be gentle ground (a mountain "pass" qualifies, a steep forested
    // slope does not — whatever the tile is labelled), with no marine/steep
    // tile in its ring.
    // Footing gentle (the spot the deck grounds on — a bank top can be flat
    // even though the cell is steep toward the channel) and no marine/steep
    // tile in the ring. slope_class is unused here (bridges use footing scale).
    let _ = slope_class;
    let good_anchor = |cell_index: usize| {
        bridge_walkable(cells[cell_index])
            && terrain.slope(grid.cell_position(cell_index)) < BRIDGE_MAX_FOOTING_SLOPE
            && grid
                .cell_neighbors(cell_index)
                .iter()
                .all(|nb| !forbidden(cells[nb.index()]))
    };

    // Commit a bridge between two bank cells if it clears the spacing rule.
    let commit = |a: usize,
                  b: usize,
                  max_span: f32,
                  spans: &mut Vec<Vec<SpherePos>>,
                  mids: &mut Vec<SpherePos>,
                  painted: &mut Painted|
     -> bool {
        let (pa, pb) = (grid.cell_position(a), grid.cell_position(b));
        let d = pa.distance(pb);
        if d < 1.0 || d > max_span {
            return false;
        }
        let mid = SpherePos::new((pa.0 + pb.0).normalize());
        if mids.iter().any(|m| m.distance(mid) < BRIDGE_MIN_SPACING) {
            return false;
        }
        let ext = BRIDGE_ENTRY_OVERLAP / d;
        let steps = (d * (1.0 + 2.0 * ext) / 6.0).ceil().max(2.0) as usize;
        let span: Vec<SpherePos> = (0..=steps)
            .map(|k| {
                crate::sphere::slerp(pa, pb, -ext + (1.0 + 2.0 * ext) * k as f32 / steps as f32)
            })
            .collect();
        for cell_index in cell_chain(grid, &span) {
            painted.bridges.insert(cell_index);
        }
        for end in [span.first(), span.last()] {
            let Some(face_index) = end.and_then(|p| grid.planet.face_at(p.0)) else {
                continue;
            };
            for cell_index in grid.face_cells(face_index) {
                if cells[cell_index].is_land() {
                    painted.bridge_entries.insert(cell_index);
                    for nb in cell_neighbor_indices(grid, cell_index) {
                        if cells[nb].is_land() {
                            painted.bridge_entries.insert(nb);
                        }
                    }
                }
            }
        }
        mids.push(mid);
        spans.push(span);
        true
    };

    // (1) River crossings: a walkable cell beside a river bank, straight
    // across the band (bank → channel → bank) to walkable ground on the far
    // side. The endpoints are the walkable ground next to the banks.
    let river_band = |t: Terrain| matches!(t, Terrain::River | Terrain::RiverBank);
    for l1 in 0..grid.cell_count() {
        if spans.len() >= BRIDGE_MAX_COUNT {
            break;
        }
        if !bridge_walkable(cells[l1]) {
            continue;
        }
        let Some(entry) = grid
            .cell_neighbors(l1)
            .iter()
            .find(|nb| cells[nb.index()] == Terrain::RiverBank)
        else {
            continue;
        };
        if let Some(l2) = cross_band(
            grid,
            cells,
            l1,
            entry.index(),
            9,
            river_band,
            bridge_walkable,
        ) {
            if l2 == l1 {
                continue;
            }
            let mid = (grid.cell_direction(l1) + grid.cell_direction(l2)) * 0.5;
            let (g1, g2) = (inland(l1, mid), inland(l2, mid));
            if good_anchor(g1) && good_anchor(g2) {
                commit(g1, g2, BRIDGE_MAX_SPAN, &mut spans, &mut mids, painted);
            }
        }
    }

    // (1b) Swamp crossings (boardwalks): a walkable cell beside a swamp, across
    // the swamp band to dry walkable ground on the far side — same rules, with
    // the swamp itself as the crossable band.
    let swamp_band = |t: Terrain| t == Terrain::Swamp;
    let dry_end = |t: Terrain| bridge_walkable(t) && t != Terrain::Swamp;
    for l1 in 0..grid.cell_count() {
        if spans.len() >= BRIDGE_MAX_COUNT {
            break;
        }
        if !dry_end(cells[l1]) {
            continue;
        }
        let Some(entry) = grid
            .cell_neighbors(l1)
            .iter()
            .find(|nb| cells[nb.index()] == Terrain::Swamp)
        else {
            continue;
        };
        if let Some(l2) = cross_band(grid, cells, l1, entry.index(), 9, swamp_band, dry_end) {
            if l2 == l1 {
                continue;
            }
            let mid = (grid.cell_direction(l1) + grid.cell_direction(l2)) * 0.5;
            let (g1, g2) = (inland(l1, mid), inland(l2, mid));
            if good_anchor(g1) && good_anchor(g2) {
                commit(g1, g2, BRIDGE_MAX_SPAN, &mut spans, &mut mids, painted);
            }
        }
    }

    // (2) Lake islands: a land component ringed only by lake water, small
    // enough to be an island, bridged to the nearest mainland lake shore.
    let components = grid
        .topology
        .cell_components(|cell| cells[cell.index()].is_land());
    let mut sizes = vec![0usize; components.count()];
    for cell in grid.topology.cells() {
        if let Some(component) = components.cell(cell) {
            sizes[component.index()] += 1;
        }
    }
    // Per-component water adjacency: is every water cell it touches Lake?
    // Ocean? (an island ringed by exactly one body is bridgeable to the
    // mainland). Rivers touching don't disqualify — they cross separately.
    let mut touch_lake = vec![false; components.count()];
    let mut touch_ocean = vec![false; components.count()];
    let mut only_lake = vec![true; components.count()];
    let mut only_ocean = vec![true; components.count()];
    for cell_index in 0..grid.cell_count() {
        let cell = grid
            .topology
            .cell(cell_index)
            .expect("cell index from topology range");
        let Some(id) = components.cell(cell).map(|component| component.index()) else {
            continue;
        };
        for nb in cell_neighbor_indices(grid, cell_index) {
            match cells[nb] {
                Terrain::Lake => {
                    touch_lake[id] = true;
                    only_ocean[id] = false;
                }
                Terrain::Ocean => {
                    touch_ocean[id] = true;
                    only_lake[id] = false;
                }
                Terrain::River => {}
                t if t.is_water() => {
                    only_lake[id] = false;
                    only_ocean[id] = false;
                }
                _ => {}
            }
        }
    }
    // (2) Islands: a small land component ringed by a single body is bridged to
    // the nearest OTHER landmass across it — lakes (short), then ocean islands
    // to the nearest continent (longer gap, islands are seeded near land).
    let island_kinds: [(Terrain, usize, f32); 2] = [
        (Terrain::LakeShore, LAKE_ISLAND_MAX_CELLS, BRIDGE_MAX_SPAN),
        (
            Terrain::Beach,
            OCEAN_ISLAND_MAX_CELLS,
            OCEAN_BRIDGE_MAX_SPAN,
        ),
    ];
    for (which, (shore_kind, max_cells, max_span)) in island_kinds.into_iter().enumerate() {
        let ringed = |id: usize| {
            if which == 0 {
                touch_lake[id] && only_lake[id]
            } else {
                touch_ocean[id] && only_ocean[id]
            }
        };
        for (id, &size) in sizes.iter().enumerate().take(components.count()) {
            if spans.len() >= BRIDGE_MAX_COUNT {
                break;
            }
            if !(ringed(id) && size <= max_cells) {
                continue;
            }
            let shore = |cell_index: usize| {
                bridge_walkable(cells[cell_index])
                    && grid
                        .cell_neighbors(cell_index)
                        .iter()
                        .any(|nb| cells[nb.index()] == shore_kind)
            };
            let island_shore: Vec<usize> = (0..grid.cell_count())
                .filter(|&cell_index| {
                    let cell = grid
                        .topology
                        .cell(cell_index)
                        .expect("cell index from topology range");
                    components
                        .cell(cell)
                        .is_some_and(|component| component.index() == id)
                        && shore(cell_index)
                })
                .collect();
            let mut best: Option<(f32, usize, usize)> = None;
            for &a in &island_shore {
                let pa = grid.cell_position(a);
                for cell_index in 0..grid.cell_count() {
                    let cell = grid
                        .topology
                        .cell(cell_index)
                        .expect("cell index from topology range");
                    let Some(other) = components.cell(cell) else {
                        continue;
                    };
                    if other.index() == id || !shore(cell_index) {
                        continue;
                    }
                    let dd = pa.distance(grid.cell_position(cell_index));
                    if best.is_none_or(|(bd, _, _)| dd < bd) {
                        best = Some((dd, a, cell_index));
                    }
                }
            }
            if let Some((_, a, b)) = best {
                let mid = (grid.cell_direction(a) + grid.cell_direction(b)) * 0.5;
                let (g1, g2) = (inland(a, mid), inland(b, mid));
                if good_anchor(g1) && good_anchor(g2) {
                    commit(g1, g2, max_span, &mut spans, &mut mids, painted);
                }
            }
        }
    }

    link_feature_pinches(grid, &mut painted.bridge_entries, |cell_index| {
        cells[cell_index].is_land()
    });
    spans
}

pub(super) fn resolve_transitions(
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
                    || terrain.elevation_at(grid.cell_position(cell_index)) > 0.15;
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
            .cell_neighbors(cell_index)
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
            .cell_neighbors(cell_index)
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
pub(super) fn prune_orphan_bands(grid: &Grid, cells: &mut [Terrain]) {
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
pub(super) fn mark_blends(
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
        for cell_index in grid.face_cells(face_index) {
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
pub(super) fn cell_neighbor_indices(grid: &Grid, v: usize) -> impl Iterator<Item = usize> + '_ {
    grid.cell_neighbors(v).iter().map(|cell| cell.index())
}

/// A contiguous cluster of equal values below its minimum size is speckle: it
/// is absorbed into its most common eligible neighbor value. Generic over the
/// value type (biome cover, landform, …). `eligible` selects which values
/// participate, `min_size` gives each value's floor, `absorbable` says which
/// neighbor values a speckle may merge into.
pub(super) fn absorb_small_clusters<T: Copy + Ord>(
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

pub(super) fn absorb_small_patches(grid: &Grid, out: &mut [Terrain]) {
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
pub(super) fn smooth_coast_band(grid: &Grid, out: &mut [Terrain]) {
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
pub(super) fn water_distance(
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
