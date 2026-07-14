use super::super::*;

/// A band needs two parallel lattice lines. Widen the chain with each edge's
/// side partner so faces between the lines form a gap-free feature strip.
pub(in crate::worldgen) fn widen_band(grid: &Grid, chain: &[usize]) -> Vec<usize> {
    features::widen_band(grid, chain, false)
}

/// Three lattice lines: the chain plus BOTH side partners (~105m) — wide
/// enough that the elevation solver owns distinct channel and bank verts and
/// can actually carve a cross-section (rivers).
pub(in crate::worldgen) fn widen_band_sym(grid: &Grid, chain: &[usize]) -> Vec<usize> {
    features::widen_band(grid, chain, true)
}

/// Roads may not cross water: a planned path whose cell chain touches a water
/// cell is dropped entirely (crossing there needs a bridge, not a road).
pub(in crate::worldgen) fn paint_features(
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
            painted.roads.insert(CellId::new(cell_index));
        }
        kept.push(path.clone());
    }
    // Towns sit on walkable ground within the settlement radius.
    for (cell_index, &slope) in slope_class.iter().enumerate().take(grid.cell_count()) {
        let pos = grid.cell_position(CellId::new(cell_index));
        if slope_walkable(slope)
            && terrain
                .settlement_anchors
                .iter()
                .any(|a| a.distance(pos) <= TOWN_RADIUS)
        {
            painted.towns.insert(CellId::new(cell_index));
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
pub(in crate::worldgen) const BRIDGE_ENTRY_OVERLAP: f32 = 3.0;
/// A river/lake crossing longer than this isn't a bridge — the water is too
/// wide (that would be a ferry, not a footbridge).
pub(in crate::worldgen) const BRIDGE_MAX_SPAN: f32 = 200.0;
/// Keep bridges apart: no two within this many meters.
pub(in crate::worldgen) const BRIDGE_MIN_SPACING: f32 = 400.0;
pub(in crate::worldgen) const BRIDGE_MAX_COUNT: usize = 12;
/// A land component smaller than this, ringed only by lake water, is an island
/// in the lake and earns a bridge to the mainland.
pub(in crate::worldgen) const LAKE_ISLAND_MAX_CELLS: usize = 1500;
/// An ocean island (small land ringed only by ocean) is bridged to the nearest
/// other landmass, but only across a SHORT gap (islands are seeded near land).
pub(in crate::worldgen) const OCEAN_ISLAND_MAX_CELLS: usize = 3000;
pub(in crate::worldgen) const OCEAN_BRIDGE_MAX_SPAN: f32 = 700.0;
/// A bridge FOOTING must be this gentle (footing-scale slope, rise/run) — the
/// immediate spot the deck grounds on. Measured at footing scale (not cell
/// scale) because a bank cell is locally steep toward the channel yet has a
/// flat footing on top.
pub(in crate::worldgen) const BRIDGE_MAX_FOOTING_SLOPE: f32 = 0.25;

/// Terrain a bridge may land on: gentle, walkable ground — never a mountain,
/// cliff, snowfield, glacier or volcanic slope.
pub(in crate::worldgen) fn bridge_walkable(t: Terrain) -> bool {
    features::bridge_walkable(t)
}

/// Walk straight across a water band from walkable ground `land`, entering the
/// band at `first`, following the initial heading cell-to-cell until walkable
/// ground is reached on the FAR side. `band` says which kinds are the crossing
/// (river+its banks, or lake+its shores). Returns the far-side walkable cell,
/// or None if the band doesn't end in walkable ground within `max` steps (e.g.
/// it runs into a mountain, or the band is too wide).
pub(in crate::worldgen) fn cross_band(
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
        grid.cell_direction(CellId::new(land)),
        grid.cell_direction(CellId::new(first)) - grid.cell_direction(CellId::new(land)),
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
        let cpos = grid.cell_direction(CellId::new(cur));
        let mut best = None;
        let mut best_dot = -2.0;
        for nb in cell_neighbor_indices(grid, cur) {
            if nb == prev {
                continue;
            }
            let d = tangent(cpos, grid.cell_direction(CellId::new(nb)) - cpos).dot(heading);
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
pub(in crate::worldgen) fn build_bridges(
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
                grid.cell_direction(CellId::new(a))
                    .distance(away)
                    .partial_cmp(&grid.cell_direction(CellId::new(b)).distance(away))
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
            && terrain.slope(grid.cell_position(CellId::new(cell_index))) < BRIDGE_MAX_FOOTING_SLOPE
            && grid
                .cell_neighbors(CellId::new(cell_index))
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
        let (pa, pb) = (
            grid.cell_position(CellId::new(a)),
            grid.cell_position(CellId::new(b)),
        );
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
            painted.bridges.insert(CellId::new(cell_index));
        }
        for end in [span.first(), span.last()] {
            let Some(face_index) = end.and_then(|p| grid.planet.face_at(p.0)) else {
                continue;
            };
            for cell_index in grid.face_cells(FaceId::new(face_index)).map(CellId::index) {
                if cells[cell_index].is_land() {
                    painted.bridge_entries.insert(CellId::new(cell_index));
                    for nb in cell_neighbor_indices(grid, cell_index) {
                        if cells[nb].is_land() {
                            painted.bridge_entries.insert(CellId::new(nb));
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
            .cell_neighbors(CellId::new(l1))
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
            let mid =
                (grid.cell_direction(CellId::new(l1)) + grid.cell_direction(CellId::new(l2))) * 0.5;
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
            .cell_neighbors(CellId::new(l1))
            .iter()
            .find(|nb| cells[nb.index()] == Terrain::Swamp)
        else {
            continue;
        };
        if let Some(l2) = cross_band(grid, cells, l1, entry.index(), 9, swamp_band, dry_end) {
            if l2 == l1 {
                continue;
            }
            let mid =
                (grid.cell_direction(CellId::new(l1)) + grid.cell_direction(CellId::new(l2))) * 0.5;
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
                        .cell_neighbors(CellId::new(cell_index))
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
                let pa = grid.cell_position(CellId::new(a));
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
                    let dd = pa.distance(grid.cell_position(CellId::new(cell_index)));
                    if best.is_none_or(|(bd, _, _)| dd < bd) {
                        best = Some((dd, a, cell_index));
                    }
                }
            }
            if let Some((_, a, b)) = best {
                let mid = (grid.cell_direction(CellId::new(a))
                    + grid.cell_direction(CellId::new(b)))
                    * 0.5;
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
