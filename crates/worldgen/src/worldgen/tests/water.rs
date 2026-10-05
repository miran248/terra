use super::*;
use bevy_math::Vec3;
use terra_world::level::{SurfaceCondition, WaterPhase};

#[test]
fn freezing_is_local_across_lakes_and_rivers() {
    let state = run_state(1337, |_| {});
    let mut frozen_lake = 0;
    let mut frozen_river = 0;
    let mut liquid_river = 0;
    for face in state.grid.topology.faces() {
        let phase = state.face_water_phase[face];
        if state.water_r[face] > 0.0
            && matches!(
                state.tiles[face],
                Terrain::Lake | Terrain::SaltLake | Terrain::LakeShore
            )
            && phase == Some(WaterPhase::Frozen)
        {
            frozen_lake += 1;
        }
        if state.river_r[face].iter().any(|&radius| radius > 0.0) {
            match phase {
                Some(WaterPhase::Frozen) => frozen_river += 1,
                Some(WaterPhase::Liquid) => liquid_river += 1,
                None => {}
            }
        }
    }
    assert!(frozen_lake > 0, "seed must include frozen lake surface");
    assert!(frozen_river > 0, "seed must include frozen river surface");
    assert!(liquid_river > 0, "one river must retain flowing sections");

    for face in state.grid.topology.faces() {
        let rendered =
            state.water_r[face] > 0.0 || state.river_r[face].iter().any(|&radius| radius > 0.0);
        assert_eq!(
            state.face_water_phase[face].is_some(),
            rendered,
            "rendered water face {} must own exactly one phase",
            face.index()
        );
    }
    assert!(state.grid.topology.faces().any(|face| {
        !state.tiles[face].is_water()
            && state.face_surface_condition[face] == SurfaceCondition::Frozen
    }));
}

#[test]
fn polar_sea_ice_forms_coherent_partial_sheets() {
    let state = run_state(1337, |_| {});
    let frozen_ocean = state.grid.topology.face_components(|face| {
        state.tiles[face] == Terrain::Ocean
            && state.face_water_phase[face] == Some(WaterPhase::Frozen)
    });
    let mut sizes = vec![0usize; frozen_ocean.count()];
    let mut liquid_ocean = 0usize;
    for face in state.grid.topology.faces() {
        if let Some(component) = frozen_ocean.face(face) {
            sizes[component.index()] += 1;
        }
        if state.tiles[face] == Terrain::Ocean
            && state.face_water_phase[face] == Some(WaterPhase::Liquid)
        {
            liquid_ocean += 1;
        }
    }
    assert!(!sizes.is_empty(), "seed must include polar sea ice");
    assert!(
        sizes.iter().all(|&size| size >= 12),
        "orphan sea-ice patch: {sizes:?}"
    );
    assert!(liquid_ocean > 0, "ocean must remain partially liquid");
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
        if state.tiles.as_slice()[face_index] == Terrain::RiverSpring {
            for (corner, &radius) in face_river_r.iter().enumerate() {
                spring_corners += 1;
                let ground =
                    Vec3::from_array(state.mesh_tris.as_slice()[face_index][corner]).length();
                assert!(
                    (radius - (ground - RIVER_TERRAIN_CLIP)).abs() < 1e-3,
                    "spring water must start embedded in the terrain"
                );
            }
        }
        if !matches!(
            state.tiles.as_slice()[face_index],
            Terrain::River | Terrain::RiverSpring | Terrain::RiverBank
        ) {
            continue;
        }
        for neighbor in state
            .grid
            .face_neighbors(FaceId::new(face_index))
            .map(FaceId::index)
        {
            let waterline = state.water_r.as_slice()[neighbor];
            if waterline <= 0.0 {
                continue;
            }
            for (corner, &radius) in face_river_r.iter().enumerate() {
                if state.mesh_tris.as_slice()[neighbor]
                    .iter()
                    .any(|&other| key(other) == key(state.mesh_tris.as_slice()[face_index][corner]))
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
        state.tiles.as_slice()[face.index()].is_water()
            && !matches!(
                state.tiles.as_slice()[face.index()],
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
        if !has_ocean[c]
            && state
                .grid
                .topology
                .faces()
                .filter(|face| components.face(*face).is_some_and(|id| id.index() == c))
                .any(|face| state.tiles.as_slice()[face.index()].is_lake())
        {
            assert!(
                has_lake[c],
                "lake component {c} does not touch an authored lake zone"
            );
        }
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
            state.cells.as_slice()[start],
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
                        state.cells.as_slice()[cell.index()],
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
                        state.cells.as_slice()[nb.index()],
                        Terrain::Ocean | Terrain::Lake | Terrain::SaltLake
                    )
                })
        });
        assert!(
            touches_sea,
            "river component of {} cells cut off from any water body at {:?}",
            comp.len(),
            comp.iter()
                .map(|&cell| (
                    state.cells.as_slice()[cell],
                    state.grid.cell_position(CellId::new(cell)).0,
                ))
                .collect::<Vec<_>>()
        );
        // And the mouth is open at FACE level too: some River face is
        // edge-adjacent to an Ocean/Lake face.
        let mut open = false;
        'faces: for face_index in 0..state.grid.face_count() {
            if !matches!(
                state.tiles.as_slice()[face_index],
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
                if state.tiles.as_slice()[nb] == Terrain::Ocean
                    || state.tiles.as_slice()[nb].is_lake()
                {
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
