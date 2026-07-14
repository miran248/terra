use super::*;

#[test]
fn solved_field_invariants() {
    let state = run_state(1337, |_| {});
    let terrain = state.terrain.as_ref().unwrap();

    // Rivers descend monotonically on the SOLVED field.
    assert!(!terrain.river_paths.is_empty(), "no rivers generated");
    let spring_components =
        cluster_cell_types(&state.grid, state.cells.as_slice(), &[Terrain::RiverSpring]);
    assert_eq!(
        spring_components.count(),
        terrain.river_paths.len(),
        "every planned river has one connected ground-contact spring"
    );
    for (ri, path) in terrain.river_paths.iter().enumerate() {
        let mut prev = f32::MAX;
        for p in path {
            let e = terrain.elevation_at(*p);
            if prev < -0.05 {
                break; // reached the sea — below the surface it's ocean
            }
            assert!(
                e <= prev + 0.02,
                "river {ri} flows uphill: {e} after {prev}"
            );
            prev = prev.min(e);
        }
    }

    // Settlements stay dry.
    for a in &terrain.settlement_anchors {
        assert!(
            terrain.elevation_at(*a) > 0.0,
            "settlement anchor under water"
        );
    }

    // Tile ranges hold exactly on the solved field (canyon verts excepted —
    // rivers may dig through anything).
    let mut canyon = vec![false; terrain.vert_count()];
    for path in &terrain.river_paths {
        for s in path {
            for (solver_vertex, _) in terrain.kernel(*s) {
                canyon[solver_vertex] = true;
            }
        }
    }
    let e = terrain.vert_elevations();
    let blend_of: std::collections::BTreeMap<u32, FaceBlend> = state
        .blends
        .iter()
        .map(|blend| (blend.face, *blend))
        .collect();
    // Bridges CONFORM to the terrain (no entry pads), so every vertex must
    // satisfy its tile range — no pad exemption.
    // Ownership is per-CELL (each solver vert is a grid cell), exactly as
    // the solver assigns ranges — the face type may differ at boundaries.
    let owners = owner_cells(&state.grid, terrain, state.cells.as_slice());
    let owners_lf = owner_landform(&state.grid, terrain, state.landform.as_slice());
    for solver_vertex in 0..terrain.vert_count() {
        if canyon[solver_vertex] {
            continue;
        }
        let Some(face_index) = state.grid.planet.face_at(terrain.vert_dir(solver_vertex)) else {
            continue;
        };
        // Land cover takes its landform's range (mirror the solver);
        // water/shore/river keep their own range and blend hull.
        let (rlo, rhi) = if is_cover(owners[solver_vertex]) {
            landform_range(owners_lf[solver_vertex])
        } else {
            match blend_of.get(&(face_index as u32)) {
                Some(FaceBlend {
                    base,
                    target: BlendTarget::Terrain(target),
                    ..
                }) => {
                    let (alo, ahi) = elev_range(*base);
                    let (blo, bhi) = elev_range(*target);
                    (alo.min(blo), ahi.max(bhi))
                }
                Some(_) | None => elev_range(owners[solver_vertex]),
            }
        };
        assert!(
            e[solver_vertex] >= rlo - 1e-4 && e[solver_vertex] <= rhi + 1e-4,
            "vert {solver_vertex} ({:?}) out of range: {} not in [{rlo}, {rhi}]",
            owners[solver_vertex],
            e[solver_vertex]
        );
    }

    // No wall at the waterline: edges crossing sea level stay gentle, and
    // no edge anywhere jumps more than 0.5 (~250m over ~70m).
    let mut worst_cross = 0.0f32;
    let mut worst_any = 0.0f32;
    for a in 0..terrain.vert_count() {
        for &b in terrain.adj_of(a) {
            if b <= a {
                continue;
            }
            let d = (e[a] - e[b]).abs();
            worst_any = worst_any.max(d);
            // Cliff and mountain coasts are deliberately steep sea walls.
            let steep_coast = |solver_vertex: usize| {
                canyon[solver_vertex]
                    || state
                        .grid
                        .planet
                        .face_at(terrain.vert_dir(solver_vertex))
                        .is_some_and(|face_index| {
                            matches!(
                                state.tiles.dense()[face_index],
                                Terrain::Cliff | Terrain::Mountain | Terrain::Snow
                            )
                        })
            };
            if (e[a] >= 0.0) != (e[b] >= 0.0) && !steep_coast(a) && !steep_coast(b) {
                worst_cross = worst_cross.max(d);
            }
        }
    }
    assert!(worst_cross <= 0.30, "waterline wall: {worst_cross}");
    assert!(worst_any <= 0.60, "extreme edge: {worst_any}");

    // A lake never rises above its shore (solver step 3a) — otherwise the flat
    // water surface floats over ground where a lake tile pokes up past the rim.
    for solver_vertex in 0..terrain.vert_count() {
        if owners[solver_vertex] != Terrain::Lake {
            continue;
        }
        for &nb in terrain.adj_of(solver_vertex) {
            if owners[nb] == Terrain::LakeShore {
                assert!(
                    e[solver_vertex] <= e[nb] + 1e-3,
                    "lake vert {solver_vertex} ({}) above its shore {nb} ({})",
                    e[solver_vertex],
                    e[nb]
                );
            }
        }
    }

    // No enclosed water body smaller than the minimum (no 1-cell lakes).
    let components = state.grid.topology.cell_components(|cell| {
        state.cells.dense()[cell.index()].is_water()
            && !matches!(
                state.cells.dense()[cell.index()],
                Terrain::River | Terrain::RiverSpring
            )
    });
    let mut sizes = vec![0usize; components.count()];
    let mut is_ocean = vec![false; components.count()];
    for v in 0..state.grid.cell_count() {
        let cell = state.grid.topology.cell(v).unwrap();
        let Some(c) = components.cell(cell).map(|component| component.index()) else {
            continue;
        };
        sizes[c] += 1;
        if cell_zone(&state.grid, terrain, v) == crate::zones::ZoneKind::Ocean {
            is_ocean[c] = true;
        }
    }
    for c in 0..components.count() {
        assert!(
            is_ocean[c] || sizes[c] >= size_range(Terrain::Lake).0,
            "enclosed water body of only {} cells survived",
            sizes[c]
        );
    }

    // Deep water never surfaces: every abyss-depth cell solves well below
    // the waterline (the depth class replaces the old DeepOcean tile).
    // (Depth is per grid CELL — sample the solved field at the cell.)
    for cell_index in 0..state.grid.cell_count() {
        if state.water_depth.dense()[cell_index] == Some(WaterDepth::Abyss) {
            let d = terrain.elevation_at(state.grid.cell_position(CellId::new(cell_index)));
            assert!(d < -0.1, "abyss cell {cell_index} not deep: {d}");
        }
    }

    // Roads never sit on water: checked per cell (painting is per cell)
    // and per solid road face.
    for cell_index in 0..state.grid.cell_count() {
        if state.painted.roads.contains(CellId::new(cell_index)) {
            assert!(
                state.cells.dense()[cell_index].is_land(),
                "road painted on water cell {cell_index}"
            );
        }
    }
    for face_index in 0..state.grid.face_count() {
        let solid = state
            .grid
            .face_cells(FaceId::new(face_index))
            .map(CellId::index)
            .iter()
            .filter(|&&cell| state.painted.roads.contains(CellId::new(cell)))
            .count()
            == 3;
        if solid {
            assert!(
                state.tiles.dense()[face_index].is_land(),
                "solid road face on water tile {face_index}"
            );
        }
    }

    // Bridge entries land on walkable ground: the deck grounds at ground
    // height (no vertical gap, by the deck builder) and the player steps
    // onto it from gentle inland terrain. (An omnidirectional slope test is
    // meaningless here — every deck end sits at a water's edge, so the bank
    // drop toward the water is steep by nature; the approach is inland.)
    let bridge_walkable = |t: Terrain| {
        matches!(
            t,
            Terrain::Plains
                | Terrain::Forest
                | Terrain::Savanna
                | Terrain::Tundra
                | Terrain::Desert
                | Terrain::Jungle
                | Terrain::Swamp
        )
    };
    for span in &state.bridges {
        for end in [span.first(), span.last()].into_iter().flatten() {
            let face_index = state
                .grid
                .planet
                .face_at(end.0)
                .expect("deck end on a face");
            assert!(
                bridge_walkable(state.tiles.dense()[face_index]),
                "bridge entry on non-walkable tile {:?}",
                state.tiles.dense()[face_index]
            );
            // The anchor CELL (where placement gated on the solved slope)
            // is genuinely gentle — a bridge never lands on steep ground,
            // whatever the biome. (An omnidirectional slope at the exact
            // water's-edge end would just read the natural bank drop.)
            let cell = state
                .grid
                .face_cells(FaceId::new(face_index))
                .map(CellId::index)
                .into_iter()
                .max_by(|&a, &b| {
                    state
                        .grid
                        .cell_direction(CellId::new(a))
                        .dot(end.0)
                        .partial_cmp(&state.grid.cell_direction(CellId::new(b)).dot(end.0))
                        .unwrap()
                })
                .unwrap();
            let slope = terrain.slope(state.grid.cell_position(CellId::new(cell)));
            assert!(slope < 0.3, "bridge anchor on steep ground: slope {slope}");
        }
    }

    // Tile identity lives on hex cells (cells can't pinch), and the
    // LINKING RULE covers the derived faces: around any cell, same-type
    // faces form ONE edge-connected fan — never linked by a lone vertex.
    // Also checked: every face's type is one of its corner cells, and
    // solid feature faces obey the same fan rule.
    {
        for face_index in 0..state.grid.face_count() {
            let corners = state
                .grid
                .face_cells(FaceId::new(face_index))
                .map(CellId::index);
            assert!(
                corners
                    .iter()
                    .any(|&cell| state.cells.dense()[cell] == state.tiles.dense()[face_index]),
                "face {face_index} derived {:?} not among its corner cells",
                state.tiles.dense()[face_index]
            );
        }
        let mut tile_pinches = 0usize;
        let mut road_pinches = 0usize;
        for v in 0..state.grid.cell_count() {
            let ring = ring(&state.grid, CellId::new(v));
            let n = ring.len();
            let ts: Vec<Terrain> = (0..n)
                .map(|i| {
                    match classification::derive_face(
                        state.cells.dense()[v],
                        state.cells.dense()[ring[i].index()],
                        state.cells.dense()[ring[(i + 1) % n].index()],
                    ) {
                        // Spring is a river source marker, not a separate
                        // traversable band; validate it as part of the river
                        // water band for the no-pinch invariant.
                        Terrain::RiverSpring => Terrain::River,
                        t => t,
                    }
                })
                .collect();
            let mut types = ts.clone();
            types.sort_by_key(|terrain| classification::terrain_rank(*terrain));
            types.dedup();
            for t in types {
                let runs = (0..n)
                    .filter(|&i| ts[i] == t && ts[(i + n - 1) % n] != t)
                    .count();
                if runs >= 2 {
                    tile_pinches += 1;
                }
            }
            if state.painted.roads.contains(CellId::new(v)) {
                let solid = |i: usize| {
                    state.painted.roads.contains(ring[i])
                        && state.painted.roads.contains(ring[(i + 1) % n])
                };
                let runs = (0..n)
                    .filter(|&i| solid(i) && !solid((i + n - 1) % n))
                    .count();
                if runs >= 2 {
                    road_pinches += 1;
                }
            }
        }
        assert_eq!(tile_pinches, 0, "same-type faces linked by a lone vertex");
        assert_eq!(road_pinches, 0, "road strip pinched at a vertex");

        // Lake-to-ocean distance ≥ 10 edge steps.
        // Actual ocean TILES, not ocean-zone: an isolated ocean-zone
        // pocket is reclassified to a lake (see size_range(Ocean)) and
        // must not seed the distance field.
        let ocean_faces: Vec<_> = state
            .grid
            .topology
            .faces()
            .filter(|face| state.tiles.dense()[face.index()] == Terrain::Ocean)
            .collect();
        let ocean_distance = state.grid.topology.face_distances(&ocean_faces, 10);
        for face_index in 0..state.grid.face_count() {
            if state.tiles.dense()[face_index] == Terrain::Lake {
                let face = state
                    .grid
                    .topology
                    .face(face_index)
                    .expect("face index from topology range");
                assert!(
                    ocean_distance.face_steps(face).is_none(),
                    "lake face {face_index} is within 10 tiles of ocean water",
                );
            }
        }
    }

    // Cliff tiles are ramps: toe verts hug the shore (lower neighbors
    // unchanged), crest verts never rise above the hinterland behind them
    // (nothing sticks out) — the drop happens across the cliff face.
    let vkind: Vec<Terrain> = owner_cells(&state.grid, terrain, state.cells.as_slice());
    let shore_kind = |t: Terrain| t.is_water() || t == Terrain::Beach;
    for a in 0..terrain.vert_count() {
        if vkind[a] != Terrain::Cliff || canyon[a] {
            continue;
        }
        let shoreside = terrain.adj_of(a).iter().any(|&nb| shore_kind(vkind[nb]));
        // Seaward cliff verts are free — the drop happens at the water
        // edge; only the crest is constrained (carries the hinterland
        // inward, never sticks out above it).
        if !shoreside {
            let hinterland = terrain
                .adj_of(a)
                .iter()
                .filter(|&&nb| {
                    vkind[nb].is_land() && vkind[nb] != Terrain::Cliff && !shore_kind(vkind[nb])
                })
                .map(|&nb| e[nb])
                .fold(f32::MIN, f32::max);
            if hinterland > f32::MIN {
                assert!(
                    e[a] <= hinterland + 0.05,
                    "cliff crest sticks out: {} above hinterland {}",
                    e[a],
                    hinterland
                );
            }
        }
    }

    // Banks sit strictly above their water: bank verts exceed the
    // adjacent water surface (water renders clamped at 0).
    for solver_vertex in 0..terrain.vert_count() {
        if canyon[solver_vertex] && vkind[solver_vertex] != Terrain::RiverBank {
            continue;
        }
        let matching = bank_water(vkind[solver_vertex]);
        if matching.is_empty() {
            continue;
        }
        for &nb in terrain.adj_of(solver_vertex) {
            if matching.contains(&vkind[nb]) {
                assert!(
                    e[solver_vertex] > e[nb].max(0.0),
                    "{:?} vert at {} not above its {:?} water at {}",
                    vkind[solver_vertex],
                    e[solver_vertex],
                    vkind[nb],
                    e[nb].max(0.0)
                );
            }
        }
    }

    // Lake beds are concave: interior verts (all-lake neighborhoods) sit
    // below the bed's edge verts on average.
    {
        let mut interior = Vec::new();
        let mut edge = Vec::new();
        for solver_vertex in 0..terrain.vert_count() {
            if vkind[solver_vertex] != Terrain::Lake || canyon[solver_vertex] {
                continue;
            }
            if terrain
                .adj_of(solver_vertex)
                .iter()
                .all(|&nb| vkind[nb] == Terrain::Lake)
            {
                interior.push(e[solver_vertex]);
            } else {
                edge.push(e[solver_vertex]);
            }
        }
        if !interior.is_empty() && !edge.is_empty() {
            let avg = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
            assert!(
                avg(&interior) < avg(&edge),
                "lake beds not concave: interior {} vs edge {}",
                avg(&interior),
                avg(&edge)
            );
        }
    }

    // Blend marks link two differing plain kinds actually adjacent there.
    assert!(!state.blends.is_empty(), "no blends marked");
    for blend in &state.blends {
        if let BlendTarget::Terrain(target) = blend.target {
            assert_ne!(blend.base, target);
        }
        assert_eq!(
            state.tiles.dense()[blend.face as usize],
            blend.base,
            "blend face kind mismatch"
        );
    }
}

#[test]
fn kernel_is_local() {
    // The interpolation kernel must return genuinely nearby vertices.
    // Guards the polar search bug: the fixed 3x3 grid window returned
    // verts up to 335m away near the poles (longitude cells shrink).
    let terrain = TerrainGen::init(1);
    for i in 0..2000 {
        let u = (i as f32 * 0.6180339) % 1.0;
        let v = (i as f32 * 0.7548776) % 1.0;
        let p = crate::sphere::random_point(u, v);
        for (solver_vertex, _) in terrain.kernel(p) {
            let d = p.distance(crate::sphere::SpherePos::new(
                terrain.vert_dir(solver_vertex),
            ));
            assert!(
                d < 200.0,
                "kernel vert {d:.0}m away at lat {:.0}",
                p.0.y.asin().to_degrees()
            );
        }
    }
}

#[test]
fn tile_type_sets_cluster_mixed_connected_groups() {
    let grid = Grid::new(1);

    let cell_neighbor = grid.cell_neighbors(CellId::new(0))[0].index();
    let cell_distant = (0..grid.cell_count())
        .find(|&v| {
            v != 0
                && v != cell_neighbor
                && !grid
                    .cell_neighbors(CellId::new(0))
                    .iter()
                    .any(|cell| cell.index() == v)
                && !grid
                    .cell_neighbors(CellId::new(cell_neighbor))
                    .iter()
                    .any(|cell| cell.index() == v)
        })
        .expect("grid needs a non-adjacent cell");
    let mut cells = vec![Terrain::Plains; grid.cell_count()];
    cells[0] = Terrain::Forest;
    cells[cell_neighbor] = Terrain::Mountain;
    cells[cell_distant] = Terrain::Snow;
    let cell_components = cluster_cell_types(
        &grid,
        &cells,
        &[
            Terrain::Forest,
            Terrain::Mountain,
            Terrain::Snow,
            Terrain::Forest,
        ],
    );
    assert_eq!(cell_components.count(), 2);
    let cell = |index| grid.topology.cell(index).unwrap();
    assert_eq!(
        cell_components.cell(cell(0)),
        cell_components.cell(cell(cell_neighbor))
    );
    assert_ne!(
        cell_components.cell(cell(0)),
        cell_components.cell(cell(cell_distant))
    );
    let cell_nonmember = cells.iter().position(|&t| t == Terrain::Plains).unwrap();
    assert_eq!(cell_components.cell(cell(cell_nonmember)), None);

    let face_neighbor = grid.face_neighbors(FaceId::new(0)).map(FaceId::index)[0];
    let face_distant = (0..grid.face_count())
        .find(|&f| {
            f != 0
                && f != face_neighbor
                && !grid
                    .face_neighbors(FaceId::new(0))
                    .map(FaceId::index)
                    .contains(&f)
                && !grid
                    .face_neighbors(FaceId::new(face_neighbor))
                    .map(FaceId::index)
                    .contains(&f)
        })
        .expect("grid needs a non-adjacent face");
    let mut face_types = vec![Terrain::Plains; grid.face_count()];
    face_types[0] = Terrain::Beach;
    face_types[face_neighbor] = Terrain::Cliff;
    face_types[face_distant] = Terrain::Ocean;
    let face_components = cluster_face_types(
        &grid,
        &face_types,
        &[Terrain::Beach, Terrain::Cliff, Terrain::Ocean],
    );
    assert_eq!(face_components.count(), 2);
    let face = |index| grid.topology.face(index).unwrap();
    assert_eq!(
        face_components.face(face(0)),
        face_components.face(face(face_neighbor))
    );
    assert_ne!(
        face_components.face(face(0)),
        face_components.face(face(face_distant))
    );
    let face_nonmember = face_types
        .iter()
        .position(|&t| t == Terrain::Plains)
        .unwrap();
    assert_eq!(face_components.face(face(face_nonmember)), None);

    let empty_components = cluster_cell_types(&grid, &cells, &[]);
    assert_eq!(empty_components.count(), 0);
    assert!(
        grid.topology
            .cells()
            .all(|cell| empty_components.cell(cell).is_none())
    );
}

#[test]
fn river_surface_starts_on_springs_and_joins_body_water() {
    let state = run_state(1337, |_| {});
    let river_r = river_surface_radii(
        &state.grid,
        state.mesh_tris.as_slice(),
        state.tiles.as_slice(),
        state.water_r.as_slice(),
    );
    let key = |p: [f32; 3]| [p[0].to_bits(), p[1].to_bits(), p[2].to_bits()];

    let mut spring_corners = 0usize;
    let mut outlet_corners = 0usize;
    for (face_index, face_river_r) in river_r.iter().enumerate().take(state.grid.face_count()) {
        if state.tiles.dense()[face_index] == Terrain::RiverSpring {
            for (corner, &radius) in face_river_r.iter().enumerate() {
                spring_corners += 1;
                let ground = Vec3::from_array(state.mesh_tris.dense()[face_index][corner]).length();
                assert!(
                    (radius - (ground - RIVER_TERRAIN_CLIP)).abs() < 1e-3,
                    "spring water must start embedded in the terrain"
                );
            }
        }
        if !matches!(
            state.tiles.dense()[face_index],
            Terrain::River | Terrain::RiverSpring | Terrain::RiverBank
        ) {
            continue;
        }
        for neighbor in state
            .grid
            .face_neighbors(FaceId::new(face_index))
            .map(FaceId::index)
        {
            let waterline = state.water_r.dense()[neighbor];
            if waterline <= 0.0 {
                continue;
            }
            for (corner, &radius) in face_river_r.iter().enumerate() {
                if state.mesh_tris.dense()[neighbor]
                    .iter()
                    .any(|&other| key(other) == key(state.mesh_tris.dense()[face_index][corner]))
                {
                    outlet_corners += 1;
                    assert!(
                        (radius - waterline).abs() < 1e-3,
                        "river outlet must share its neighboring waterline"
                    );
                }
            }
        }
    }
    assert!(spring_corners > 0, "seed must include Spring faces");
    assert!(
        outlet_corners > 0,
        "seed must include water-connected river outlets"
    );
}

#[test]
fn lakes_stay_enclosed() {
    // No lake-zone water may connect to the ocean — the rim dam guarantees
    // every lake is its own body (guards the drain-channel bug where lakes
    // leaked to the sea along coarse-face edges and became ocean inlets).
    let state = run_state(1337, |_| {});
    let terrain = state.terrain.as_ref().unwrap();
    let components = state.grid.topology.face_components(|face| {
        state.tiles.dense()[face.index()].is_water()
            && !matches!(
                state.tiles.dense()[face.index()],
                Terrain::River | Terrain::RiverSpring
            )
    });
    let mut sizes = vec![0usize; components.count()];
    let mut has_lake = vec![false; components.count()];
    let mut has_ocean = vec![false; components.count()];
    for face_index in 0..state.grid.face_count() {
        let face = state.grid.topology.face(face_index).unwrap();
        let Some(c) = components.face(face).map(|component| component.index()) else {
            continue;
        };
        sizes[c] += 1;
        match terrain.zones().kind_at_fine(face_index) {
            crate::zones::ZoneKind::Lake => has_lake[c] = true,
            crate::zones::ZoneKind::Ocean => has_ocean[c] = true,
            _ => {}
        }
    }
    let mut lake_faces = 0;
    for c in 0..components.count() {
        if has_lake[c] {
            lake_faces += sizes[c];
            assert!(
                !has_ocean[c],
                "lake body of {} faces connects to the ocean",
                sizes[c]
            );
        }
    }
    assert!(
        lake_faces > 100,
        "lakes nearly vanished: {lake_faces} faces"
    );
}

#[test]
fn rivers_reach_the_sea() {
    // Every river must join a larger water body — no thin terrain band
    // may cut a mouth off (guards the junction-face damming bug).
    let state = run_state(1337, |_| {});
    let mut visited = vec![false; state.grid.cell_count()];
    for start in 0..state.grid.cell_count() {
        if !matches!(
            state.cells.dense()[start],
            Terrain::River | Terrain::RiverSpring
        ) || visited[start]
        {
            continue;
        }
        let comp: Vec<_> = state
            .grid
            .topology
            .cell_component(
                state
                    .grid
                    .topology
                    .cell(start)
                    .expect("cell index from topology range"),
                |cell| {
                    matches!(
                        state.cells.dense()[cell.index()],
                        Terrain::River | Terrain::RiverSpring
                    )
                },
            )
            .into_iter()
            .map(|cell| cell.index())
            .collect();
        for &cell in &comp {
            visited[cell] = true;
        }
        let touches_sea = comp.iter().any(|&cell_index| {
            state
                .grid
                .cell_neighbors(CellId::new(cell_index))
                .iter()
                .any(|nb| {
                    matches!(
                        state.cells.dense()[nb.index()],
                        Terrain::Ocean | Terrain::Lake
                    )
                })
        });
        assert!(
            touches_sea,
            "river component of {} cells cut off from any water body",
            comp.len()
        );
        // And the mouth is open at FACE level too: some River face is
        // edge-adjacent to an Ocean/Lake face.
        let mut open = false;
        'faces: for face_index in 0..state.grid.face_count() {
            if !matches!(
                state.tiles.dense()[face_index],
                Terrain::River | Terrain::RiverSpring
            ) {
                continue;
            }
            if !state
                .grid
                .face_cells(FaceId::new(face_index))
                .map(CellId::index)
                .iter()
                .any(|cell| comp.contains(cell))
            {
                continue;
            }
            for nb in state
                .grid
                .face_neighbors(FaceId::new(face_index))
                .map(FaceId::index)
            {
                if matches!(state.tiles.dense()[nb], Terrain::Ocean | Terrain::Lake) {
                    open = true;
                    break 'faces;
                }
            }
        }
        assert!(
            open,
            "river mouth dammed at face level ({} cells)",
            comp.len()
        );
    }
}

#[test]
fn mesh_is_watertight() {
    // Every fall-through bug is a crack: an edge used by only one triangle
    // is a hole the player can drop through. On a closed surface each edge
    // is shared by EXACTLY two faces, and every vertex fan is a full ring.
    // Assert both on the baked mesh (the collider is built from it).
    let state = run_state(1337, |_| {});
    let grid = &state.grid;

    // (a) each undirected edge belongs to exactly two faces.
    let mut edge_faces: BTreeMap<(usize, usize), u32> = BTreeMap::new();
    for face_index in 0..grid.face_count() {
        let idx = grid.face_cells(FaceId::new(face_index)).map(CellId::index);
        for k in 0..3 {
            let (a, b) = (idx[k], idx[(k + 1) % 3]);
            let key = if a < b { (a, b) } else { (b, a) };
            *edge_faces.entry(key).or_default() += 1;
        }
    }
    for (&(a, b), &n) in &edge_faces {
        assert_eq!(n, 2, "edge ({a},{b}) shared by {n} faces (not 2) — a crack");
    }

    // (b) every cell's faces form ONE closed fan: walking face→face
    // across shared edges visits all of them and returns. A cell whose
    // fan splits is a pinhole even if each edge is shared twice.
    for v in 0..grid.cell_count() {
        let cell = grid
            .topology
            .cell(v)
            .expect("cell index from topology range");
        let faces = grid.topology.cell_faces(cell);
        let n = faces.len();
        assert!((5..=6).contains(&n), "cell {v} has {n} faces");
        let mut seen = vec![false; n];
        let mut stack = vec![0usize];
        seen[0] = true;
        let mut count = 1;
        while let Some(i) = stack.pop() {
            for j in 0..n {
                if seen[j] {
                    continue;
                }
                // Adjacent in the fan iff they share an edge through v
                // (two common vertices).
                let face_index = grid.face_cells(faces[i]).map(CellId::index);
                let fj = grid.face_cells(faces[j]).map(CellId::index);
                let shared = face_index.iter().filter(|x| fj.contains(x)).count();
                if shared == 2 {
                    seen[j] = true;
                    count += 1;
                    stack.push(j);
                }
            }
        }
        assert_eq!(count, n, "cell {v} fan is not one closed ring — a pinhole");
    }
}

#[test]
fn deterministic_pipeline() {
    let a = run_state(42, |_| {});
    let b = run_state(42, |_| {});
    assert_eq!(
        a.terrain.unwrap().vert_elevations(),
        b.terrain.unwrap().vert_elevations()
    );
    assert_eq!(a.cells, b.cells);
    assert_eq!(a.tiles, b.tiles);
    assert_eq!(a.regions.len(), b.regions.len());
    assert_eq!(a.flora.len(), b.flora.len());
    assert!(
        a.flora
            .iter()
            .zip(&b.flora)
            .all(|(x, y)| x.pos == y.pos && x.kind == y.kind)
    );
    assert_eq!(a.structures.len(), b.structures.len());
    assert!(
        a.structures
            .iter()
            .zip(&b.structures)
            .all(|(x, y)| x.pos == y.pos && x.kind == y.kind)
    );
    assert_eq!(a.slope_class, b.slope_class);
    assert_eq!(a.water_depth, b.water_depth);
    assert_eq!(a.landform, b.landform);
}

fn serialized_fingerprint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

#[test]
fn locked_serialized_worlds() {
    let seed_1337 = postcard::to_allocvec(run(1337, |_| {}).level_data()).unwrap();
    assert_eq!(
        seed_1337.as_slice(),
        include_bytes!("../../../main/assets/level_1337.bin")
    );
    assert_eq!(serialized_fingerprint(&seed_1337), 0x6e14_b46d_517e_d0e2);

    let seed_42 = postcard::to_allocvec(run(42, |_| {}).level_data()).unwrap();
    assert_eq!(serialized_fingerprint(&seed_42), 0x49fa_fb72_6e14_c803);
}

#[test]
fn flora_stays_off_water_and_features() {
    let state = run_state(1337, |_| {});
    assert!(
        state.flora.len() > 1000,
        "flora nearly absent: {}",
        state.flora.len()
    );
    for f in &state.flora {
        let face_index = f.face as usize;
        assert!(
            state.tiles.dense()[face_index].is_land(),
            "flora on water face {face_index}"
        );
        for bits in [
            &state.painted.roads,
            &state.painted.towns,
            &state.painted.bridge_entries,
        ] {
            assert_eq!(
                painted_corners(&state.grid, bits, FaceId::new(face_index)),
                0,
                "flora on a feature face {face_index}"
            );
        }
    }
}
