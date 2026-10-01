use super::*;

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
        let is_land = state.tiles.as_slice()[face_index].is_land();
        let is_aquatic = matches!(
            f.kind,
            crate::level::FloraKind::Seaweed
                | crate::level::FloraKind::Lilypad
                | crate::level::FloraKind::Coral
                | crate::level::FloraKind::Anemone
                | crate::level::FloraKind::Starfish
                | crate::level::FloraKind::Kelp
        );
        // Shell appears on both beach and ocean — skip ambiguous domain check
        if matches!(f.kind, crate::level::FloraKind::Shell) {
            continue;
        }
        assert_eq!(
            is_land, !is_aquatic,
            "flora kind {:?} placed on land={} face {face_index}",
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
                "flora on a feature face {face_index}"
            );
        }
    }
}
