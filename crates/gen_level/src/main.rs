use shared::worldgen::{CompletedWorld, run};
use std::fs;
use std::path::PathBuf;

/// The pipeline itself lives in `shared::worldgen` as a command/event state machine;
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
    let bytes = postcard::to_allocvec(world.level_data()).expect("serialize");
    let _ = fs::create_dir_all(out.parent().unwrap());
    fs::write(out, &bytes).expect("write");
    println!("Wrote {face_count} faces → {}", out.display());
}

fn print_stats(world: &CompletedWorld) {
    let stats = world.stats();
    let (min_e, max_e) = (stats.min_elevation, stats.max_elevation);
    println!(
        "altitude: {:.0}m .. {:.0}m (e {min_e:.2} .. {max_e:.2})",
        -(-min_e).max(0.0) * shared::terrain::MAX_DEPTH,
        max_e.max(0.0).powf(1.15) * shared::terrain::MAX_MOUNTAIN,
    );
    println!(
        "water: {:.1}%  breakdown: {:?}",
        stats.water_faces as f32 / stats.face_count as f32 * 100.0,
        stats.terrain_faces,
    );
    println!(
        "flora: {}  structures: {}",
        stats.flora_count, stats.structure_count
    );
    println!(
        "regions: {}  bridges: {}",
        stats.region_count, stats.bridge_count
    );
}
