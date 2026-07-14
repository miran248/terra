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
                                state.tiles.as_slice()[face_index],
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
        state.cells.as_slice()[cell.index()].is_water()
            && !matches!(
                state.cells.as_slice()[cell.index()],
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
        if state.water_depth.as_slice()[cell_index] == Some(WaterDepth::Abyss) {
            let d = terrain.elevation_at(state.grid.cell_position(CellId::new(cell_index)));
            assert!(d < -0.1, "abyss cell {cell_index} not deep: {d}");
        }
    }

    // Roads never sit on water: checked per cell (painting is per cell)
    // and per solid road face.
    for cell_index in 0..state.grid.cell_count() {
        if state.painted.roads.contains(CellId::new(cell_index)) {
            assert!(
                state.cells.as_slice()[cell_index].is_land(),
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
                state.tiles.as_slice()[face_index].is_land(),
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
                bridge_walkable(state.tiles.as_slice()[face_index]),
                "bridge entry on non-walkable tile {:?}",
                state.tiles.as_slice()[face_index]
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
                    .any(|&cell| state.cells.as_slice()[cell] == state.tiles.as_slice()[face_index]),
                "face {face_index} derived {:?} not among its corner cells",
                state.tiles.as_slice()[face_index]
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
                        state.cells.as_slice()[v],
                        state.cells.as_slice()[ring[i].index()],
                        state.cells.as_slice()[ring[(i + 1) % n].index()],
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
            .filter(|face| state.tiles.as_slice()[face.index()] == Terrain::Ocean)
            .collect();
        let ocean_distance = state.grid.topology.face_distances(&ocean_faces, 10);
        for face_index in 0..state.grid.face_count() {
            if state.tiles.as_slice()[face_index] == Terrain::Lake {
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
            state.tiles.as_slice()[blend.face as usize],
            blend.base,
            "blend face kind mismatch"
        );
    }
}
