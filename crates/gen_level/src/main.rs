use std::fs;
use std::path::PathBuf;
use terra_world::level::LevelData;
use terra_worldgen::terrain::{MAX_DEPTH, MAX_MOUNTAIN};
use terra_worldgen::worldgen::{CompletedWorld, run};

/// The pipeline itself lives in `terra_worldgen::worldgen` as a command/event state machine;
/// this binary just runs it for a seed, prints the event log + stats, and packs
/// the result into the level binary.
fn main() {
    let seed: u32 = std::env::var("PLANET_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1337);
    let out = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| {
        let dir = std::env!("CARGO_MANIFEST_DIR");
        format!("{dir}/../main/assets/level_{seed}.bin")
    }));

    let world = run(seed, |line| println!("• {line}"));
    print_stats(&world);
    serialize(world, &out);
}

fn serialize(world: CompletedWorld, out: &PathBuf) {
    let face_count = world.stats().face_count;
    let level: &LevelData = world.level_data();
    let bytes = level.to_artifact_bytes().expect("serialize level artifact");
    let _ = fs::create_dir_all(out.parent().unwrap());
    fs::write(out, &bytes).expect("write");
    println!("Wrote {face_count} faces → {}", out.display());
}

fn print_stats(world: &CompletedWorld) {
    let stats = world.stats();
    let (min_e, max_e) = (stats.min_elevation, stats.max_elevation);
    println!(
        "altitude: {:.0}m .. {:.0}m (e {min_e:.2} .. {max_e:.2})",
        -(-min_e).max(0.0) * MAX_DEPTH,
        max_e.max(0.0).powf(1.15) * MAX_MOUNTAIN,
    );
    println!(
        "water: {:.1}% ({} frozen faces)  breakdown: {:?}",
        stats.water_faces as f32 / stats.face_count as f32 * 100.0,
        stats.frozen_water_faces,
        stats.terrain_faces,
    );
    println!(
        "scenery: {}  structures: {}",
        stats.scenery_count, stats.structure_count
    );
    println!(
        "regions: {}  bridges: {}",
        stats.region_count, stats.bridge_count
    );
}
