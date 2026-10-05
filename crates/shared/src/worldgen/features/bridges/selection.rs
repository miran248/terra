/// Longest permitted deck for every crossing kind.
pub(in crate::worldgen) const BRIDGE_MAX_SPAN: f32 = 500.0;
/// Shortest permitted deck for every crossing kind.
pub(in crate::worldgen) const BRIDGE_MIN_SPAN: f32 = 1.0;
/// Keep bridges apart: no two within this many meters.
pub(in crate::worldgen) const BRIDGE_MIN_SPACING: f32 = 1000.0;
/// A bridge must shorten the available walkable route by at least this much.
pub(in crate::worldgen) const BRIDGE_MIN_WALK_SAVING: f32 = 1000.0;
/// Deck centrelines closer than their combined half-width would overlap.
pub(in crate::worldgen) const BRIDGE_MIN_CLEARANCE: f32 = 8.0;
/// A land component smaller than this, ringed only by lake water, is an island
/// in the lake and earns a bridge to the mainland.
pub(in crate::worldgen) const LAKE_ISLAND_MAX_CELLS: usize = 1500;
/// An ocean island (small land ringed only by ocean) is bridged to the nearest
/// other landmass, but only across a SHORT gap (islands are seeded near land).
pub(in crate::worldgen) const OCEAN_ISLAND_MAX_CELLS: usize = 3000;
/// A bridge FOOTING must be this gentle (footing-scale slope, rise/run) — the
/// immediate spot the deck grounds on. Measured at footing scale (not cell
/// scale) because a bank cell is locally steep toward the channel yet has a
/// flat footing on top.
pub(in crate::worldgen) const BRIDGE_MAX_FOOTING_SLOPE: f32 = 0.25;

/// Walk straight across a water band from walkable ground `land`, entering the
/// band at `first`, following the initial heading cell-to-cell until walkable
/// ground is reached on the FAR side. `band` says which kinds are the crossing
/// (river+its banks, or lake+its shores). Returns the far-side walkable cell,
/// or None if the band doesn't end in walkable ground within `max` steps (e.g.
/// it runs into a mountain, or the band is too wide).
pub(in crate::worldgen) fn cross_band(
    grid: &Grid,
    cells: &[Terrain],
    land: CellId,
    first: CellId,
    max: usize,
    band: impl Fn(Terrain) -> bool,
    is_end: impl Fn(Terrain) -> bool,
) -> Option<CellId> {
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
        if is_end(cells[cur.index()]) {
            return Some(cur);
        }
        // Only cross the intended band; anything else (open ocean, a mountain
        // foot) aborts — no bridge there.
        if !band(cells[cur.index()]) {
            return None;
        }
        let cpos = grid.cell_direction(cur);
        let mut best = None;
        let mut best_dot = -2.0;
        for &nb in grid.cell_neighbors(cur) {
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

struct BridgeOutput {
    spans: Vec<Vec<SpherePos>>,
    mids: Vec<SpherePos>,
}

struct BridgeContext<'a> {
    grid: &'a Grid,
    terrain: &'a TerrainGen,
    cells: &'a [Terrain],
    face_types: &'a [Terrain],
}

fn bridge_span(
    context: &BridgeContext<'_>,
    a: CellId,
    b: CellId,
    max_span: f32,
) -> Option<(Vec<SpherePos>, SpherePos)> {
    let BridgeContext {
        grid,
        terrain,
        cells,
        face_types,
    } = context;
    let (pa, pb) = (grid.cell_position(a), grid.cell_position(b));
    let anchor_distance = pa.distance(pb);
    if anchor_distance < BRIDGE_MIN_SPAN {
        return None;
    }
    // Walk from each validated inland anchor toward the water and retain the
    // last buildable sample. The deck is then embedded only a few meters into
    // solid ground instead of spanning the full distance between inland pads.
    let samples = (anchor_distance / 2.0).ceil().max(2.0) as usize;
    let buildable = |step: usize| {
        let point = terra_geometry::sphere::slerp(pa, pb, step as f32 / samples as f32);
        crate::worldgen::nearest_cell(grid, point).is_some_and(|cell| {
            features::bridge_walkable(cells[cell.index()])
                && terrain.slope(grid.cell_position(cell)) < BRIDGE_MAX_FOOTING_SLOPE
        }) && grid
            .planet
            .face_at(point.0)
            .is_some_and(|face| features::bridge_walkable(face_types[face]))
    };
    let left = (0..=samples).take_while(|&step| buildable(step)).last()?;
    let right = (0..=samples)
        .rev()
        .take_while(|&step| buildable(step))
        .last()?;
    if left >= right {
        return None;
    }
    let from = left as f32 / samples as f32;
    let to = right as f32 / samples as f32;
    let start = terra_geometry::sphere::slerp(pa, pb, from);
    let end = terra_geometry::sphere::slerp(pa, pb, to);
    let distance = start.distance(end);
    if !(BRIDGE_MIN_SPAN..=max_span).contains(&distance) {
        return None;
    }
    let steps = (distance / 6.0).ceil().max(2.0) as usize;
    let span = (0..=steps)
        .map(|step| terra_geometry::sphere::slerp(start, end, step as f32 / steps as f32))
        .collect::<Vec<_>>();
    let mid = terra_geometry::sphere::slerp(start, end, 0.5);
    Some((span, mid))
}

fn commit_bridge(
    context: &BridgeContext<'_>,
    a: CellId,
    b: CellId,
    max_span: f32,
    output: &mut BridgeOutput,
    painted: &mut Painted,
) -> bool {
    let BridgeContext { grid, cells, .. } = context;
    let Some((span, mid)) = bridge_span(context, a, b, max_span) else {
        return false;
    };
    // A disconnected landmass has no finite walking route and therefore
    // always benefits. On one landmass, reject cheap bay cuts and tiny local
    // crossings whose dry detour is less than one kilometre longer.
    if let Some(path) = grid.topology.cell_shortest_path_with(a, b, 256, |cell| {
        features::bridge_walkable(cells[cell.index()])
    }) {
        let walking = path
            .windows(2)
            .map(|edge| {
                grid.cell_position(edge[0])
                    .distance(grid.cell_position(edge[1]))
            })
            .sum::<f32>();
        let deck = span.first().unwrap().distance(*span.last().unwrap());
        if walking - deck < BRIDGE_MIN_WALK_SAVING {
            return false;
        }
    }
    if output
        .mids
        .iter()
        .any(|existing| existing.distance(mid) < BRIDGE_MIN_SPACING)
    {
        return false;
    }
    if output.spans.iter().any(|existing| {
        span.iter().any(|point| {
            existing
                .iter()
                .any(|other| point.distance(*other) < BRIDGE_MIN_CLEARANCE)
        })
    }) {
        return false;
    }
    for cell in cell_chain(grid, &span) {
        painted.bridges.insert(cell);
    }
    for end in [span.first(), span.last()] {
        let Some(face_index) = end.and_then(|point| grid.planet.face_at(point.0)) else {
            continue;
        };
        for cell in grid.face_cells(FaceId::new(face_index)) {
            if cells[cell.index()].is_land() {
                painted.bridge_entries.insert(cell);
            }
        }
    }
    output.mids.push(mid);
    output.spans.push(span);
    true
}

fn crosses_only_ocean_between(
    grid: &Grid,
    cells: &[Terrain],
    components: &terra_geometry::topology::ComponentLabels<
        terra_geometry::topology::CellComponentId,
    >,
    a_component: usize,
    b_component: usize,
    a: CellId,
    b: CellId,
) -> bool {
    let (pa, pb) = (grid.cell_position(a), grid.cell_position(b));
    let steps = (pa.distance(pb) / 6.0).ceil().max(2.0) as usize;
    let path = (0..=steps)
        .map(|step| terra_geometry::sphere::slerp(pa, pb, step as f32 / steps as f32))
        .collect::<Vec<_>>();
    let mut crossed_ocean = false;
    for cell in cell_chain(grid, &path) {
        if cells[cell.index()] == Terrain::Ocean {
            crossed_ocean = true;
            continue;
        }
        if let Some(component) = components.cell(cell)
            && (component.index() == a_component || component.index() == b_component)
        {
            continue;
        }
        return false;
    }
    crossed_ocean
}

/// Bridges cross water locally: short river, swamp and lake bands, plus ocean
/// gaps between islands or separate continents. Every deck lands on gentle
/// walkable ground and ocean decks may not pass over a third landmass.
pub(in crate::worldgen) fn build_bridges(
    grid: &Grid,
    terrain: &TerrainGen,
    cells: &[Terrain],
    slope_class: &[SlopeClass],
    face_types: &[Terrain],
    painted: &mut Painted,
) -> Vec<Vec<SpherePos>> {
    let mut output = BridgeOutput {
        spans: Vec::new(),
        mids: Vec::new(),
    };
    let bridge_context = BridgeContext {
        grid,
        terrain,
        cells,
        face_types,
    };
    // A deck grounds cleanly only on gentle ground: reject an endpoint whose
    // proposed elevation differs sharply from its land neighbors (a steep bank
    // shoulder the entry pad couldn't flatten). Proposed field is set by now.
    // One walkable cell further from `away`, so the deck grounds inland of the
    // steep bank edge (the pad there sits on flat ground, clear of the carved
    // channel). Falls back to the cell itself if no inland walkable neighbor.
    let inland1 = |cell: CellId, away: Vec3| {
        grid.cell_neighbors(cell)
            .iter()
            .copied()
            .filter(|&neighbor| features::bridge_walkable(cells[neighbor.index()]))
            .max_by(|&a, &b| {
                grid.cell_direction(a)
                    .distance(away)
                    .partial_cmp(&grid.cell_direction(b).distance(away))
                    .unwrap()
            })
            .unwrap_or(cell)
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
    let good_anchor = |cell: CellId| {
        features::bridge_walkable(cells[cell.index()])
            && terrain.slope(grid.cell_position(cell)) < BRIDGE_MAX_FOOTING_SLOPE
            && grid
                .cell_neighbors(cell)
                .iter()
                .all(|nb| !forbidden(cells[nb.index()]))
    };

    // (1) River crossings: a walkable cell beside a river bank, straight
    // across the band (bank → channel → bank) to walkable ground on the far
    // side. The endpoints are the walkable ground next to the banks.
    let river_band = |t: Terrain| matches!(t, Terrain::River | Terrain::RiverBank);
    let spring_sources = grid
        .topology
        .cells()
        .filter(|cell| cells[cell.index()] == Terrain::RiverSpring)
        .collect::<Vec<_>>();
    let spring_distance = grid.topology.cell_distances(&spring_sources, 4);
    for l1 in grid.topology.cells() {
        if !features::bridge_walkable(cells[l1.index()]) {
            continue;
        }
        if spring_distance.cell_steps(l1).is_some() {
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
            *entry,
            9,
            river_band,
            features::bridge_walkable,
        ) {
            if l2 == l1 {
                continue;
            }
            let mid = (grid.cell_direction(l1) + grid.cell_direction(l2)) * 0.5;
            let (g1, g2) = (inland(l1, mid), inland(l2, mid));
            if good_anchor(g1) && good_anchor(g2) {
                commit_bridge(
                    &bridge_context,
                    g1,
                    g2,
                    BRIDGE_MAX_SPAN,
                    &mut output,
                    painted,
                );
            }
        }
    }

    // (1b) Swamp crossings (boardwalks): a walkable cell beside a swamp, across
    // the swamp band to dry walkable ground on the far side — same rules, with
    // the swamp itself as the crossable band.
    let swamp_band = |t: Terrain| t == Terrain::Swamp;
    let dry_end = |t: Terrain| features::bridge_walkable(t) && t != Terrain::Swamp;
    for l1 in grid.topology.cells() {
        if !dry_end(cells[l1.index()]) {
            continue;
        }
        let Some(entry) = grid
            .cell_neighbors(l1)
            .iter()
            .find(|nb| cells[nb.index()] == Terrain::Swamp)
        else {
            continue;
        };
        if let Some(l2) = cross_band(grid, cells, l1, *entry, 9, swamp_band, dry_end) {
            if l2 == l1 {
                continue;
            }
            let mid = (grid.cell_direction(l1) + grid.cell_direction(l2)) * 0.5;
            let (g1, g2) = (inland(l1, mid), inland(l2, mid));
            if good_anchor(g1) && good_anchor(g2) {
                commit_bridge(
                    &bridge_context,
                    g1,
                    g2,
                    BRIDGE_MAX_SPAN,
                    &mut output,
                    painted,
                );
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
    for cell in grid.topology.cells() {
        let Some(id) = components.cell(cell).map(|component| component.index()) else {
            continue;
        };
        for &neighbor in grid.cell_neighbors(cell) {
            match cells[neighbor.index()] {
                t if t.is_lake() => {
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
        (Terrain::Beach, OCEAN_ISLAND_MAX_CELLS, BRIDGE_MAX_SPAN),
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
            if !(ringed(id) && size <= max_cells) {
                continue;
            }
            let shore = |cell: CellId| {
                features::bridge_walkable(cells[cell.index()])
                    && grid
                        .cell_neighbors(cell)
                        .iter()
                        .any(|nb| cells[nb.index()] == shore_kind)
            };
            let island_shore: Vec<CellId> = grid
                .topology
                .cells()
                .filter(|&cell| {
                    components
                        .cell(cell)
                        .is_some_and(|component| component.index() == id)
                        && shore(cell)
                })
                .collect();
            let mut best: Option<(f32, CellId, CellId)> = None;
            for &a in &island_shore {
                let pa = grid.cell_position(a);
                for cell in grid.topology.cells() {
                    let Some(other) = components.cell(cell) else {
                        continue;
                    };
                    if other.index() == id || !shore(cell) {
                        continue;
                    }
                    let dd = pa.distance(grid.cell_position(cell));
                    if best.is_none_or(|(bd, _, _)| dd < bd) {
                        best = Some((dd, a, cell));
                    }
                }
            }
            if let Some((_, a, b)) = best {
                let b_id = components.cell(b).unwrap().index();
                let mid = (grid.cell_direction(a) + grid.cell_direction(b)) * 0.5;
                let (g1, g2) = (inland(a, mid), inland(b, mid));
                if good_anchor(g1)
                    && good_anchor(g2)
                    && (which == 0
                        || crosses_only_ocean_between(grid, cells, &components, id, b_id, g1, g2))
                {
                    commit_bridge(&bridge_context, g1, g2, max_span, &mut output, painted);
                }
            }
        }
    }

    // (3) Useful ocean straits: apply the ocean-island rules between large
    // landmasses and between two coasts of the SAME landmass. Short,
    // near-perpendicular crossings win, approximating construction cost at a
    // narrow pass while avoiding long diagonal cuts across a bay.
    let ocean_shore = |cell: CellId| {
        features::bridge_walkable(cells[cell.index()])
            && grid
                .cell_neighbors(cell)
                .iter()
                .any(|nb| cells[nb.index()] == Terrain::Beach)
    };
    let large_coast_ids = sizes
        .iter()
        .enumerate()
        .filter_map(|(id, &size)| {
            // Continents may contain authored lakes, so unlike a small ocean
            // island they need only touch the ocean rather than be ringed by
            // ocean exclusively.
            (size > OCEAN_ISLAND_MAX_CELLS && touch_ocean[id]).then_some(id)
        })
        .collect::<Vec<_>>();
    let large_coasts = large_coast_ids
        .iter()
        .map(|&id| {
            grid.topology
                .cells()
                .filter(|&cell| {
                    components
                        .cell(cell)
                        .is_some_and(|component| component.index() == id)
                        && ocean_shore(cell)
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    for left in 0..large_coast_ids.len() {
        for right in left..large_coast_ids.len() {
            for (a_index, &a) in large_coasts[left].iter().enumerate() {
                let pa = grid.cell_position(a);
                let first_b = if left == right { a_index + 1 } else { 0 };
                let mut candidates = large_coasts[right]
                    .iter()
                    .copied()
                    .skip(first_b)
                    .filter_map(|b| {
                        let distance = pa.distance(grid.cell_position(b));
                        let heading = |from: CellId, to: CellId| {
                            let p = grid.cell_direction(from);
                            let q = grid.cell_direction(to);
                            (q - p * q.dot(p)).normalize_or_zero()
                        };
                        let outward = |land: CellId| {
                            let p = grid.cell_direction(land);
                            grid.cell_neighbors(land)
                                .iter()
                                .filter(|neighbor| cells[neighbor.index()] == Terrain::Beach)
                                .map(|neighbor| {
                                    let q = grid.cell_direction(*neighbor);
                                    (q - p * q.dot(p)).normalize_or_zero()
                                })
                                .sum::<Vec3>()
                                .normalize_or_zero()
                        };
                        let align_a = heading(a, b).dot(outward(a)).max(0.0);
                        let align_b = heading(b, a).dot(outward(b)).max(0.0);
                        ((BRIDGE_MIN_SPAN..=BRIDGE_MAX_SPAN).contains(&distance)
                            && align_a >= 0.5
                            && align_b >= 0.5)
                            .then_some((distance / (align_a * align_b), b))
                    })
                    .collect::<Vec<_>>();
                candidates.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
                for (_, b) in candidates {
                    let mid = (grid.cell_direction(a) + grid.cell_direction(b)) * 0.5;
                    let (g1, g2) = (inland(a, mid), inland(b, mid));
                    let anchor_midpoint = SpherePos::new(
                        (grid.cell_direction(g1) + grid.cell_direction(g2)).normalize(),
                    );
                    if output
                        .mids
                        .iter()
                        .any(|existing| existing.distance(anchor_midpoint) < BRIDGE_MIN_SPACING)
                    {
                        continue;
                    }
                    if !good_anchor(g1)
                        || !good_anchor(g2)
                        || !crosses_only_ocean_between(
                            grid,
                            cells,
                            &components,
                            large_coast_ids[left],
                            large_coast_ids[right],
                            g1,
                            g2,
                        )
                    {
                        continue;
                    }
                    let Some((_candidate_span, _)) =
                        bridge_span(&bridge_context, g1, g2, BRIDGE_MAX_SPAN)
                    else {
                        continue;
                    };
                    if commit_bridge(
                        &bridge_context,
                        g1,
                        g2,
                        BRIDGE_MAX_SPAN,
                        &mut output,
                        painted,
                    ) {
                        break;
                    }
                }
            }
        }
    }

    output.spans
}
use bevy::prelude::Vec3;

use crate::terrain::TerrainGen;
use crate::worldgen::{Grid, Painted, cell_chain, features};
use terra_geometry::sphere::SpherePos;
use terra_geometry::topology::{CellId, FaceId};
use terra_world::level::SlopeClass;
use terra_world::terrain::Terrain;
