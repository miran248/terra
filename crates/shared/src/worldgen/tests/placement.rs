use super::*;

#[test]
fn scenery_stays_off_water_and_features() {
    let state = run_state(1337, |_| {});
    assert!(
        state.scenery.len() > 1000,
        "scenery nearly absent: {}",
        state.scenery.len()
    );
    for f in &state.scenery {
        let face_index = f.face as usize;
        let is_land = state.tiles.as_slice()[face_index].is_land();
        let is_aquatic = matches!(
            f.kind,
            terra_world::level::SceneryKind::Flora(terra_world::level::FloraKind::Seaweed)
                | terra_world::level::SceneryKind::Flora(terra_world::level::FloraKind::Lilypad)
                | terra_world::level::SceneryKind::Coral
                | terra_world::level::SceneryKind::Anemone
                | terra_world::level::SceneryKind::Starfish
                | terra_world::level::SceneryKind::Flora(terra_world::level::FloraKind::Kelp)
        );
        // Shell appears on both beach and ocean — skip ambiguous domain check
        if matches!(f.kind, terra_world::level::SceneryKind::Shell) {
            continue;
        }
        assert_eq!(
            is_land, !is_aquatic,
            "scenery kind {:?} placed on land={} face {face_index}",
            f.kind, is_land
        );
        for bits in [
            &state.painted.roads,
            &state.painted.settlements,
            &state.painted.bridge_entries,
        ] {
            assert_eq!(
                painted_corners(&state.grid, bits, FaceId::new(face_index)),
                0,
                "scenery on a feature face {face_index}"
            );
        }
    }
}
