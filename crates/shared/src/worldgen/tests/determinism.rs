use super::*;
use crate::level::{Landform, RegionKind};
use crate::terrain::Terrain;
use crate::topology::FaceId;

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
    assert_eq!(a.scenery.len(), b.scenery.len());
    assert!(
        a.scenery
            .iter()
            .zip(&b.scenery)
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

#[test]
fn forested_mountain_faces_belong_to_both_regions() {
    let world = run(1337, |_| {});
    let level = world.level_data();
    let face = (0..level.landform.len())
        .find(|&face| {
            if level.landform[face] != Landform::Mountains
                || level.face_types[face] != Terrain::Forest
            {
                return false;
            }
            let kinds = level
                .region_ids_at_face(FaceId::new(face))
                .iter()
                .map(|&region| level.regions[region as usize].kind)
                .collect::<Vec<_>>();
            kinds.contains(&RegionKind::Forest) && kinds.contains(&RegionKind::MountainRange)
        })
        .expect("seed 1337 should include a forested mountain face in both regions");

    assert!(
        level
            .region_ids_at_face(FaceId::new(face))
            .iter()
            .all(|&region| (region as usize) < level.regions.len())
    );
}

#[test]
fn built_feature_regions_overlap_the_underlying_landscape() {
    let world = run(1337, |_| {});
    let level = world.level_data();
    let overlaps = |wanted| {
        (0..level.face_regions.location_count()).any(|face| {
            let kinds = level
                .region_ids_at_face(FaceId::new(face))
                .iter()
                .map(|&region| level.regions[region as usize].kind)
                .collect::<Vec<_>>();
            kinds.contains(&wanted)
                && kinds
                    .iter()
                    .any(|&kind| !matches!(kind, RegionKind::Settlement | RegionKind::Road))
        })
    };

    assert!(overlaps(RegionKind::Settlement));
    assert!(overlaps(RegionKind::Road));
}

fn serialized_fingerprint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

#[test]
fn locked_serialized_worlds() {
    let seed_1337 = run(1337, |_| {}).level_data().to_artifact_bytes().unwrap();
    assert_eq!(
        serialized_fingerprint(&seed_1337),
        12088493585452139082,
        "fingerprint changed — regenerate level_1337.bin and update this value"
    );
    let expected_bytes = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../main/assets/level_1337.bin"
    ));
    if seed_1337.as_slice() != expected_bytes {
        panic!(
            "level_1337.bin mismatch! expected {} bytes, got {} bytes. Run 'cargo run -p gen_level -- crates/main/assets/level_1337.bin' to update.",
            expected_bytes.len(),
            seed_1337.len()
        );
    }

    let seed_42 = run(42, |_| {}).level_data().to_artifact_bytes().unwrap();
    assert_eq!(serialized_fingerprint(&seed_42), 99101208779465957);
    let repeated_seed_42 = run(42, |_| {}).level_data().to_artifact_bytes().unwrap();
    assert_eq!(seed_42, repeated_seed_42, "seed-42 artifact bytes changed");
}
