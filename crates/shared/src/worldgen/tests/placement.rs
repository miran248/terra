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
        assert!(
            state.tiles.as_slice()[face_index].is_land(),
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
