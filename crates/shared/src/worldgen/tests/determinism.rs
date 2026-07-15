use super::*;

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
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../main/assets/level_1337.bin"
        ))
    );
    assert_eq!(serialized_fingerprint(&seed_1337), 0x2224_7d53_ebbd_baa8);

    let seed_42 = postcard::to_allocvec(run(42, |_| {}).level_data()).unwrap();
    assert_eq!(serialized_fingerprint(&seed_42), 0x6d39_4319_6d09_febe);
}
