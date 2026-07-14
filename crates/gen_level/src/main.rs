use shared::worldgen::{run, GenState};
use shared::terrain::Terrain;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

/// The pipeline itself lives in `shared::gen` as a command/event state machine;
/// this binary just runs it for a seed, prints the event log + stats, and packs
/// the result into the level binary.
fn main() {
    let seed: u32 = std::env::var("PLANET_SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(1337);
    let out = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| {
        let dir = std::env!("CARGO_MANIFEST_DIR");
        format!("{dir}/../main/assets/level_{seed}.bin")
    }));

    let state = run(seed, |line| println!("• {line}"));
    print_stats(&state);
    serialize(&state, &out);
}

fn serialize(state: &GenState, out: &PathBuf) {
    let data = state.to_level_data();
    let bytes = postcard::to_allocvec(&data).expect("serialize");
    let _ = fs::create_dir_all(out.parent().unwrap());
    fs::write(out, &bytes).expect("write");
    println!("Wrote {} faces → {}", state.grid.face_count(), out.display());
}

fn print_stats(state: &GenState) {
    let terrain = state.terrain.as_ref().expect("pipeline finished");
    let e = terrain.vert_elevations();
    let max_e = e.iter().copied().fold(f32::MIN, f32::max);
    let min_e = e.iter().copied().fold(f32::MAX, f32::min);
    println!(
        "altitude: {:.0}m .. {:.0}m (e {min_e:.2} .. {max_e:.2})",
        -(-min_e).max(0.0) * shared::terrain::MAX_DEPTH,
        max_e.max(0.0).powf(1.15) * shared::terrain::MAX_MOUNTAIN,
    );
    let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
    for t in &state.tiles {
        *counts.entry(match t {
            Terrain::Ocean => "Ocean",
            Terrain::Lake => "Lake", Terrain::LakeShore => "LakeShore",
            Terrain::River => "River", Terrain::RiverBank => "RiverBank",
            Terrain::RiverSpring => "RiverSpring",
            Terrain::Beach => "Beach", Terrain::Cliff => "Cliff",
            Terrain::Desert => "Desert", Terrain::Plains => "Plains",
            Terrain::Forest => "Forest", Terrain::Tundra => "Tundra",
            Terrain::Mountain => "Mountain", Terrain::Snow => "Snow",
            Terrain::Swamp => "Swamp", Terrain::Jungle => "Jungle",
            Terrain::Savanna => "Savanna", Terrain::Volcanic => "Volcanic",
            Terrain::Glacier => "Glacier",
        }).or_default() += 1;
    }
    let water: usize = state.tiles.iter().filter(|t| t.is_water()).count();
    println!("water: {:.1}%  breakdown: {:?}", water as f32 / state.grid.face_count() as f32 * 100.0, counts);
    println!("flora: {}  structures: {}", state.flora.len(), state.structures.len());
    println!("regions: {}  bridges: {}", state.regions.len(), state.bridges.len());
}
