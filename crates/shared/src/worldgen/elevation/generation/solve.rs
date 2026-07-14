use std::collections::VecDeque;

use super::super::super::*;

pub(in crate::worldgen) fn solve_elevation(
    grid: &Grid,
    terrain: &TerrainGen,
    cells: &[Terrain],
    landform: &[Landform],
    tiles: &[Terrain],
    painted: &Painted,
    blends: &[FaceBlend],
) -> (Vec<f32>, usize, f32) {
    let solver_vertex_count = terrain.vert_count();
    let blend_of: BTreeMap<u32, FaceBlend> =
        blends.iter().map(|blend| (blend.face, *blend)).collect();

    // Per-vertex interval and kind: each vertex takes the range of the tile
    // directly under it — the same mapping the gradient caps use, so the
    // constraint set is self-consistent by construction. Transitions between
    // kinds are shaped by the edge caps, not by range intersections.
    let owner: Vec<Terrain> = owner_cells(grid, terrain, cells);
    let owner_lf: Vec<Landform> = owner_landform(grid, terrain, landform);
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
                Some(FaceBlend {
                    base,
                    target: BlendTarget::Terrain(target),
                    ..
                }) => {
                    let (alo, ahi) = elev_range(*base);
                    let (blo, bhi) = elev_range(*target);
                    (alo.min(blo), ahi.max(bhi))
                }
                Some(_) | None => elev_range(owner[solver_vertex]),
            }
        };
        lo[solver_vertex] = rlo;
        hi[solver_vertex] = rhi;
    }
    for cell_index in 0..grid.cell_count() {
        if painted.roads.contains(CellId::new(cell_index)) {
            for &(solver_vertex, _) in
                terrain.kernel(grid.cell_position(CellId::new(cell_index)))[..3].iter()
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
