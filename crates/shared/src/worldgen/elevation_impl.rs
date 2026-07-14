use super::*;

// ---- elevation synthesis (SolveElevation) ----
//
// The final elevation field is CONSTRAINT-SOLVED from the finished tile map:
// every tile kind declares the elevation range its ground may occupy, every
// kind pair declares how fast elevation may change across one vertex edge
// (~70m), rivers must descend monotonically, and road corridors stay gentle.
// A deterministic Gauss-Seidel loop drives the proposed field into the
// constraint set. There are no later touchups — the mesh projects this field.

/// Elevation range (in [-1,1] units; 1.0 land unit = 500m) a tile's ground
/// may occupy.
/// How far below its neighbors' average a water bed vertex is pushed each
/// solver step — a concave basin/channel instead of a flat plate. `None` for
/// non-bed kinds. (Rivers cut sharper than lake basins.)
pub(super) fn water_concavity(t: Terrain) -> Option<f32> {
    match t {
        // Per-iteration push below the neighbour average — the deeper this, the
        // deeper the basin bowls (depth still grows with basin size). Lakes were
        // near-flat plates (0.005); deepen them so water pools with real depth.
        Terrain::Lake => Some(0.050),
        Terrain::River | Terrain::RiverSpring => Some(0.090),
        _ => None,
    }
}

/// The water kinds a shore/bank vertex must sit strictly above (its adjacent
/// body). Empty for non-bank kinds.
pub(super) fn bank_water(t: Terrain) -> &'static [Terrain] {
    match t {
        Terrain::RiverBank => &[Terrain::River, Terrain::RiverSpring],
        Terrain::LakeShore => &[Terrain::Lake],
        Terrain::Beach => &[Terrain::Ocean],
        _ => &[],
    }
}

/// A cover biome whose ELEVATION comes from its landform, not itself (a snowy
/// lowland stays low; snow doesn't imply a mountain). Water, shore bands and
/// rivers keep their own ranges — they aren't landforms.
pub(super) fn is_cover(t: Terrain) -> bool {
    use Terrain::*;
    matches!(
        t,
        Desert
            | Plains
            | Forest
            | Tundra
            | Savanna
            | Swamp
            | Jungle
            | Mountain
            | Snow
            | Volcanic
            | Glacier
    )
}

/// Elevation range a LANDFORM's ground may occupy — the base layer that drives
/// height (cover only colors it). Bands overlap so adjacent landforms
/// (ordered lowland→hills→mountains) meet without an impossible jump.
pub(super) fn landform_range(lf: u8) -> (f32, f32) {
    match lf {
        LANDFORM_VALLEY => (0.0, 0.16),
        LANDFORM_LOWLAND => (0.02, 0.20),
        LANDFORM_HILLS => (0.14, 0.45),
        LANDFORM_MOUNTAINS => (0.40, 1.0),
        LANDFORM_PLATEAU => (0.36, 0.74),
        _ => (0.02, 0.50),
    }
}

/// How steep a land edge may be, from the steeper of the two landforms:
/// lowlands are gentle, mountains steep, hills between.
pub(super) fn landform_edge_cap(lfa: u8, lfb: u8) -> f32 {
    let one = |lf: u8| -> f32 {
        match lf {
            LANDFORM_VALLEY | LANDFORM_LOWLAND => 0.04,
            LANDFORM_HILLS => 0.14,
            LANDFORM_PLATEAU => 0.20,
            LANDFORM_MOUNTAINS => 0.40,
            _ => 0.10,
        }
    };
    one(lfa).max(one(lfb))
}

pub(super) fn elev_range(t: Terrain) -> (f32, f32) {
    use Terrain::*;
    // Ranges of kinds that may sit next to each other must overlap (or lie
    // within one edge's gradient cap) or the constraint set is unsatisfiable.
    match t {
        // One ocean identity; the shelf constraint deepens the floor with
        // distance from land, so the range spans shore-shallows to abyss.
        Ocean => (-1.0, -0.01),
        // Lakes and rivers carry their OWN water level — a mountain lake may
        // sit high above the sea; only its shores must stay above it.
        Lake => (-0.25, 0.55),
        LakeShore => (0.0, 0.60),
        // Rivers descend from mountains to the sea; their range must span it.
        River => (-1.0, 0.60),
        RiverSpring => (0.0, 0.60),
        RiverBank => (0.0, 0.65),
        Beach => (0.0, 0.05),
        // Cliff tiles are RAMPS, not plateaus: the toe verts sit at shore
        // level and the crest verts track the hinterland (see the cliff
        // tracking step in the solver), so the whole drop happens across the
        // cliff face. The range here is just the envelope.
        Cliff => (-0.02, 0.60),
        Desert | Plains | Forest | Tundra | Savanna => (0.02, 0.50),
        // Swamp is low, wet, near-flat ground just above the water line.
        Swamp => (0.0, 0.15),
        // Jungle covers lowland to hills.
        Jungle => (0.02, 0.55),
        Mountain => (0.45, 1.0),
        Snow => (0.50, 1.0),
        // Volcanic peaks and glaciers ride the high ground like Mountain/Snow.
        Volcanic => (0.45, 1.0),
        Glacier => (0.45, 1.0),
    }
}

/// Max elevation change across one vertex edge (~70m) between two tile kinds.
/// Small at shores (continental shelf), large into mountains and at cliffs.
pub(super) fn max_gradient(a: Terrain, b: Terrain) -> f32 {
    use Terrain::*;
    let water = |t: Terrain| matches!(t, Ocean | Lake);
    let peak = |t: Terrain| matches!(t, Mountain | Snow | Volcanic | Glacier);
    // Rivers are canyons: their walls may be steep wherever they cut through.
    // Caps are per vertex edge (~35m at field sub=6).
    let base: f32 = if a == LakeShore && b == LakeShore {
        // The shore ring is the waterline: keep it nearly level so the flat lake
        // surface meets it evenly all the way round (an uneven rim floats the
        // surface at the low end). The land rising above a hillside lake is other
        // terrain, not the shore, so this doesn't flatten the surroundings.
        0.005
    } else if matches!(a, River | RiverBank) || matches!(b, River | RiverBank) {
        0.22
    } else if a == Cliff || b == Cliff {
        // The whole cliff drop can happen across one vertex edge (toe → crest).
        0.30
    } else if peak(a) || peak(b) {
        0.14
    } else if water(a) && water(b) {
        0.04
    } else if water(a) || water(b) || a == Beach || b == Beach {
        0.015
    } else {
        // Ordinary land: ~0.02 e per ~35m edge ≈ 15° — walkable country,
        // not ski slopes. Steepness is a property of mountains, cliffs and
        // canyons (their caps above), not of plains.
        0.02
    };
    // Feasibility: a cap can never be tighter than the jump the two kinds'
    // disjoint elevation ranges force — otherwise range clamp and gradient cap
    // fight forever (e.g. a mountain vertex right beside a lakeshore vertex).
    let (alo, ahi) = elev_range(a);
    let (blo, bhi) = elev_range(b);
    let forced_gap = (blo - ahi).max(alo - bhi).max(0.0);
    base.max(forced_gap + 0.01)
}

/// Tight cap along road corridors so roads stay walkable.
pub(super) const ROAD_EDGE_GRADIENT: f32 = 0.01;
pub(super) const SOLVER_MAX_ITERS: usize = 250;
pub(super) const SOLVER_EPS: f32 = 0.002;

pub(super) fn kernel_interp(kernel: &[(usize, f32); 6], values: &[f32]) -> f32 {
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
pub(super) fn owner_of<T: Copy>(
    grid: &Grid,
    terrain: &TerrainGen,
    per_cell: &[T],
    default: T,
) -> Vec<T> {
    let index: BTreeMap<[u32; 3], u32> = grid
        .topology
        .cells()
        .map(|cell| {
            let direction = grid.cell_direction_index(cell.index());
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
                            .face_cells_index(face_index)
                            .into_iter()
                            .max_by(|&a, &b| {
                                grid.cell_direction_index(a)
                                    .dot(d)
                                    .partial_cmp(&grid.cell_direction_index(b).dot(d))
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

pub(super) fn owner_cells(grid: &Grid, terrain: &TerrainGen, cells: &[Terrain]) -> Vec<Terrain> {
    owner_of(grid, terrain, cells, Terrain::Plains)
}

pub(super) fn owner_landform(grid: &Grid, terrain: &TerrainGen, landform: &[u8]) -> Vec<u8> {
    owner_of(grid, terrain, landform, LANDFORM_LOWLAND)
}

pub(super) fn solve_elevation(
    grid: &Grid,
    terrain: &TerrainGen,
    cells: &[Terrain],
    landform: &[u8],
    tiles: &[Terrain],
    painted: &Painted,
    blends: &[(u32, u8, u8)],
) -> (Vec<f32>, usize, f32) {
    let solver_vertex_count = terrain.vert_count();
    let mut blend_of: BTreeMap<u32, (u8, u8)> = BTreeMap::new();
    for &(face_index, a, b) in blends {
        blend_of.insert(face_index, (a, b));
    }

    // Per-vertex interval and kind: each vertex takes the range of the tile
    // directly under it — the same mapping the gradient caps use, so the
    // constraint set is self-consistent by construction. Transitions between
    // kinds are shaped by the edge caps, not by range intersections.
    let owner: Vec<Terrain> = owner_cells(grid, terrain, cells);
    let owner_lf: Vec<u8> = owner_landform(grid, terrain, landform);
    let _ = tiles;
    let owner_face: Vec<Option<usize>> = (0..solver_vertex_count)
        .map(|solver_vertex| grid.planet.face_at(terrain.vert_dir(solver_vertex)))
        .collect();
    let elevation::ElevationConstraints {
        lower: mut lo,
        upper: mut hi,
    } = elevation::ElevationConstraints::unconstrained(solver_vertex_count);
    let mut is_road_vert = vec![false; solver_vertex_count];
    for solver_vertex in 0..solver_vertex_count {
        // Blend faces are the altitude ramp between two kinds: their vertices
        // get the HULL of both ranges so the solver can transition through.
        // Land COVER takes its landform's range (height from the massif, not
        // the biome); water/shore/river keep their own range and blend hull.
        let (rlo, rhi) = if is_cover(owner[solver_vertex]) {
            landform_range(owner_lf[solver_vertex])
        } else {
            match owner_face[solver_vertex]
                .and_then(|face_index| blend_of.get(&(face_index as u32)))
            {
                Some(&(_, b)) if b >= crate::level::BLEND_FEATURE_MIN => {
                    elev_range(owner[solver_vertex])
                }
                Some(&(a, b)) => {
                    let (alo, ahi) = elev_range(Terrain::ALL[a as usize]);
                    let (blo, bhi) = elev_range(Terrain::ALL[b as usize]);
                    (alo.min(blo), ahi.max(bhi))
                }
                None => elev_range(owner[solver_vertex]),
            }
        };
        lo[solver_vertex] = rlo;
        hi[solver_vertex] = rhi;
    }
    for cell_index in 0..grid.cell_count() {
        if painted.roads.contains(cell_index) {
            for &(solver_vertex, _) in
                terrain.kernel(grid.cell_position_index(cell_index))[..3].iter()
            {
                is_road_vert[solver_vertex] = true;
            }
        }
    }
    // Continental shelf as a HARD range: since Ocean is one identity now, the
    // shelf both keeps water shallow near shore (no wall at the coast) AND
    // forces it deep offshore (the ceiling drops with distance from land), so
    // the deep basins/abyss come from geometry, not a separate tile. Distance
    // to land in vertex steps (~35m).
    {
        let mut dist = vec![u8::MAX; solver_vertex_count];
        let solver_graph = elevation::TerrainSolverVertexGraph::new(terrain);
        let mut q: VecDeque<elevation::SolverVertexId> = VecDeque::new();
        for solver_vertex in solver_graph.vertices() {
            if !owner[solver_vertex.index()].is_water() {
                dist[solver_vertex.index()] = 0;
                q.push_back(solver_vertex);
            }
        }
        while let Some(solver_vertex) = q.pop_front() {
            if dist[solver_vertex.index()] >= 8 {
                continue;
            }
            for neighbor in solver_graph.neighbors(solver_vertex) {
                if dist[neighbor.index()] == u8::MAX {
                    dist[neighbor.index()] = dist[solver_vertex.index()] + 1;
                    q.push_back(neighbor);
                }
            }
        }
        for solver_vertex in 0..solver_vertex_count {
            if !owner[solver_vertex].is_water() || owner[solver_vertex] == Terrain::River {
                continue;
            }
            // Lakes keep their own (concave) profile; only the open sea shelves.
            if owner[solver_vertex] == Terrain::Lake {
                continue;
            }
            // A narrow depth WINDOW per distance band, both bounds deepening
            // with distance: shore-shallows (no wall at the coast) grading to
            // abyss offshore. floor ≤ ceil, and the step between adjacent
            // bands stays within the extreme-edge limit.
            let (floor, ceil) = match dist[solver_vertex] {
                1 => (-0.10, -0.03),
                2 => (-0.22, -0.08),
                3 => (-0.38, -0.18),
                4 => (-0.52, -0.32),
                5 => (-0.64, -0.46),
                6 => (-0.74, -0.56),
                7 => (-0.82, -0.62),
                _ => (-0.95, -0.70),
            };
            hi[solver_vertex] = hi[solver_vertex].min(ceil);
            lo[solver_vertex] = lo[solver_vertex].max(floor).min(hi[solver_vertex]);
        }
    }

    // River sample kernels for the monotone-descent constraint. Every vertex a
    // river sample touches is canyon ground: river range + canyon gradient caps.
    let river_kernels: Vec<Vec<[(usize, f32); 6]>> = terrain
        .river_paths
        .iter()
        .map(|path| path.iter().map(|s| terrain.kernel(*s)).collect())
        .collect();
    let mut is_canyon_vert = vec![false; solver_vertex_count];
    for kernels in &river_kernels {
        for kernel in kernels {
            for &(solver_vertex, _) in kernel {
                is_canyon_vert[solver_vertex] = true;
                let (rlo, rhi) = elev_range(Terrain::River);
                lo[solver_vertex] = rlo;
                hi[solver_vertex] = rhi;
            }
        }
    }

    // Per-vertex kind for gradient caps (canyon verts count as River).
    let vkind: Vec<Terrain> = (0..solver_vertex_count)
        .map(|solver_vertex| {
            if is_canyon_vert[solver_vertex] {
                Terrain::River
            } else {
                owner[solver_vertex]
            }
        })
        .collect();

    // Cliff crest vertices track the hinterland: the high ground CARRIES
    // INWARD (the cliff is the edge of a raised coast, not a wall in front of
    // low ground), and nothing sticks out above the terrain behind. The
    // seaward drop needs no pinning — the wide Cliff range and the steep
    // cliff/water gradient allowance let the whole drop happen at the edge.
    let shore_kind = |t: Terrain| t.is_water() || t == Terrain::Beach;
    let mut cliff_crest: Vec<bool> = vec![false; solver_vertex_count];
    for solver_vertex in 0..solver_vertex_count {
        if owner[solver_vertex] == Terrain::Cliff
            && !terrain
                .adj_of(solver_vertex)
                .iter()
                .any(|&nb| shore_kind(owner[nb]))
        {
            cliff_crest[solver_vertex] = true;
        }
    }

    // Bridges CONFORM to the finished terrain (selected after this solve on
    // gentle ground), so the solver no longer flattens entry pads — features
    // no longer reshape the field here.
    let (mut e, iters, residual) = elevation::ordered_relaxation(
        terrain.vert_elevations().to_vec(),
        SOLVER_MAX_ITERS,
        SOLVER_EPS,
        |e| {
            let mut residual = 0.0;
            // 1) gradient caps per edge (best-effort smoothing).
            for a in 0..solver_vertex_count {
                for &b in terrain.adj_of(a) {
                    if b <= a {
                        continue;
                    }
                    let mut cap = if is_cover(vkind[a]) && is_cover(vkind[b]) {
                        landform_edge_cap(owner_lf[a], owner_lf[b])
                    } else {
                        max_gradient(vkind[a], vkind[b])
                    };
                    if is_road_vert[a] && is_road_vert[b] {
                        cap = cap.min(ROAD_EDGE_GRADIENT);
                    }
                    let d = e[a] - e[b];
                    if d.abs() > cap {
                        let excess = (d.abs() - cap) / 2.0;
                        let dir = d.signum();
                        e[a] -= dir * excess;
                        e[b] += dir * excess;
                        residual += excess;
                    }
                }
            }
            // 1b) road smoothing: pull every road vertex toward the average of
            // its road-corridor neighbors, so the ROAD SURFACE has no local bumps —
            // the gradient cap bounds the slope, this bounds the change in slope
            // (curvature), giving a road that eases over the ground.
            {
                let mut delta = vec![0.0f32; solver_vertex_count];
                for a in 0..solver_vertex_count {
                    if !is_road_vert[a] {
                        continue;
                    }
                    let mut sum = 0.0;
                    let mut cnt = 0;
                    for &b in terrain.adj_of(a) {
                        if is_road_vert[b] {
                            sum += e[b];
                            cnt += 1;
                        }
                    }
                    if cnt > 0 {
                        // Half-strength Laplacian: smooths bumps without erasing
                        // the road's overall descent.
                        delta[a] = 0.5 * (sum / cnt as f32 - e[a]);
                    }
                }
                for a in 0..solver_vertex_count {
                    if delta[a] != 0.0 {
                        e[a] += delta[a];
                        residual += delta[a].abs();
                    }
                }
            }
            // 1c) river-bed smoothing: a light Laplacian over the carved channel
            // removes cell-to-cell chatter before the downstream constraint runs.
            // It only samples other channel vertices, preserving the banks as the
            // raised rim; the following monotone pass keeps flow downhill.
            {
                let mut delta = vec![0.0f32; solver_vertex_count];
                for solver_vertex in 0..solver_vertex_count {
                    if owner[solver_vertex] != Terrain::River {
                        continue;
                    }
                    let mut sum = 0.0;
                    let mut count = 0usize;
                    for &nb in terrain.adj_of(solver_vertex) {
                        if owner[nb] == Terrain::River {
                            sum += e[nb];
                            count += 1;
                        }
                    }
                    if count > 0 {
                        delta[solver_vertex] = 0.15 * (sum / count as f32 - e[solver_vertex]);
                    }
                }
                for solver_vertex in 0..solver_vertex_count {
                    if delta[solver_vertex] != 0.0 {
                        e[solver_vertex] += delta[solver_vertex];
                        residual += delta[solver_vertex].abs();
                    }
                }
            }
            // 2) rivers descend monotonically (lower-only kernel clamp) — but only
            // until they reach the sea; below the surface the river IS the ocean
            // and fighting its ranges would oscillate forever.
            for kernels in &river_kernels {
                let mut floor = f32::MAX;
                for kernel in kernels {
                    let v = kernel_interp(kernel, e);
                    if floor < -0.05 {
                        break;
                    }
                    if v > floor + 0.001 {
                        let delta = v - floor;
                        for &(solver_vertex, _) in kernel {
                            e[solver_vertex] -= delta;
                        }
                        residual += delta;
                    } else {
                        floor = floor.min(v);
                    }
                }
            }
            // 3) lake and river beds are CONCAVE: every water vert is pushed
            // below the average of its neighbors, so basins bowl toward the
            // middle and channels dip below their banks — depth grows naturally
            // with basin size instead of being a flat plate.
            for solver_vertex in 0..solver_vertex_count {
                let Some(c) = water_concavity(owner[solver_vertex]) else {
                    continue;
                };
                let nbs = terrain.adj_of(solver_vertex);
                let avg: f32 = nbs.iter().map(|&nb| e[nb]).sum::<f32>() / nbs.len() as f32;
                let cap = avg - c;
                if e[solver_vertex] > cap {
                    residual += e[solver_vertex] - cap;
                    e[solver_vertex] = cap;
                }
            }
            // Concavity can lower a downstream channel vertex more than its next
            // neighbour, so reapply the directional constraint after carving. This
            // permits a deeper bed without ever creating an uphill segment.
            for kernels in &river_kernels {
                let mut floor = f32::MAX;
                for kernel in kernels {
                    let v = kernel_interp(kernel, e);
                    if floor < -0.05 {
                        break;
                    }
                    if v > floor + 0.001 {
                        let delta = v - floor;
                        for &(solver_vertex, _) in kernel {
                            e[solver_vertex] -= delta;
                        }
                        residual += delta;
                    } else {
                        floor = floor.min(v);
                    }
                }
            }
            // 3a) a lake never rises above its shore: clamp every lake vertex that
            // touches the shore to just below its lowest shore neighbour. The
            // concavity above then keeps the interior below the edge, so the whole
            // basin stays under its rim (otherwise the flat water surface floats over
            // ground where a lake tile pokes up past the shore).
            for solver_vertex in 0..solver_vertex_count {
                if owner[solver_vertex] != Terrain::Lake {
                    continue;
                }
                let mut min_shore = f32::MAX;
                for &nb in terrain.adj_of(solver_vertex) {
                    if owner[nb] == Terrain::LakeShore {
                        min_shore = min_shore.min(e[nb]);
                    }
                }
                if min_shore != f32::MAX {
                    let cap = min_shore - 0.01;
                    if e[solver_vertex] > cap {
                        residual += e[solver_vertex] - cap;
                        e[solver_vertex] = cap;
                    }
                }
            }
            // 3b) cliff crest tracking: the crest equals the hinterland edge.
            for solver_vertex in 0..solver_vertex_count {
                if cliff_crest[solver_vertex] {
                    let mut hinterland = f32::MIN;
                    for &nb in terrain.adj_of(solver_vertex) {
                        if owner[nb].is_land()
                            && owner[nb] != Terrain::Cliff
                            && !shore_kind(owner[nb])
                        {
                            hinterland = hinterland.max(e[nb]);
                        }
                    }
                    if hinterland > f32::MIN {
                        residual += (e[solver_vertex] - hinterland).abs();
                        e[solver_vertex] = hinterland;
                    }
                }
            }
            // 4) banks sit ABOVE their water: a river bank is strictly higher
            // than the adjacent river, a lake shore than its lake, a beach than
            // the sea — the water's edge is always a step up onto land. Water
            // surfaces render clamped at 0, so the floor is vs max(water e, 0).
            for solver_vertex in 0..solver_vertex_count {
                // A bank vert caught in a river's descent kernel still IS a bank:
                // on a hillside the kernel would drag the downhill bank below the
                // water and the river would spill. The floor runs after the
                // descent step, so both banks end above the channel everywhere.
                if is_canyon_vert[solver_vertex] && owner[solver_vertex] != Terrain::RiverBank {
                    continue;
                }
                let matching_water = bank_water(owner[solver_vertex]);
                if matching_water.is_empty() {
                    continue;
                }
                let mut water_surface = f32::MIN;
                for &nb in terrain.adj_of(solver_vertex) {
                    if matching_water.contains(&owner[nb]) {
                        water_surface = water_surface.max(e[nb].max(0.0));
                    }
                }
                if water_surface > f32::MIN {
                    let floor = water_surface + 0.01;
                    if e[solver_vertex] < floor {
                        residual += floor - e[solver_vertex];
                        e[solver_vertex] = floor;
                    }
                }
            }
            // 5) tile ranges — hard constraints and always get the
            // final word each iteration, so the finished field satisfies every
            // tile's elevation range exactly (caps are best-effort where the tile
            // map demands steeper chains than they allow).
            for solver_vertex in 0..solver_vertex_count {
                let c = e[solver_vertex].clamp(lo[solver_vertex], hi[solver_vertex]);
                residual += (c - e[solver_vertex]).abs();
                e[solver_vertex] = c;
            }
            residual
        },
    );
    // Final guarantee (the solver's per-iteration bank floor can lose a race
    // to river descent at a high source): every bank vert ends strictly above
    // its adjacent water surface. Highest banks first so a bank that borders
    // another bank still clears the shared water.
    let mut order: Vec<usize> = (0..solver_vertex_count)
        .filter(|&solver_vertex| {
            matches!(
                owner[solver_vertex],
                Terrain::RiverBank | Terrain::LakeShore | Terrain::Beach
            )
        })
        .collect();
    order.sort_by(|&a, &b| e[b].partial_cmp(&e[a]).unwrap());
    for &solver_vertex in &order {
        let matching = bank_water(owner[solver_vertex]);
        if matching.is_empty() {
            continue;
        }
        let mut surface = f32::MIN;
        for &nb in terrain.adj_of(solver_vertex) {
            if matching.contains(&owner[nb]) {
                surface = surface.max(e[nb].max(0.0));
            }
        }
        if surface > f32::MIN {
            e[solver_vertex] = e[solver_vertex].max(surface + 0.01);
        }
    }
    elevation::classify_result(&mut e);
    (e, iters, residual)
}
