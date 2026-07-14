use shared::worldgen::{run, GenState};
use shared::level::{LevelData, RoadData, SettlementData, LEVEL_FORMAT_VERSION};
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
    let terrain = state.terrain.as_ref().expect("pipeline finished");
    let unit_tris_arr: Vec<[[f32; 3]; 3]> = state.grid.unit_tris.iter()
        .map(|[a, b, c]| [a.to_array(), b.to_array(), c.to_array()])
        .collect();
    let settlements = terrain.settlement_anchors.iter().enumerate()
        .map(|(i, a)| SettlementData { name: shared::roads::settlement_name(i), pos: a.0.to_array() })
        .collect();
    let mut roads: Vec<RoadData> = state.roads.iter()
        .map(|p| RoadData { points: p.iter().map(|s| s.0.to_array()).collect(), is_bridge: false })
        .collect();
    roads.extend(state.bridges.iter()
        .map(|p| RoadData { points: p.iter().map(|s| s.0.to_array()).collect(), is_bridge: true }));

    let data = LevelData {
        version: LEVEL_FORMAT_VERSION,
        seed: state.grid.seed,
        vert_elev: terrain.vert_elevations().to_vec(),
        terrain_tris: state.mesh_tris.clone(),
        terrain_colors: state.mesh_colors.clone(),
        unit_tris: unit_tris_arr,
        face_types: state.tiles.iter().map(|t| *t as u8).collect(),
        face_water_r: state.water_r.clone(),
        face_river_r: state.river_r.clone(),
        face_blend: state.blends.clone(),
        face_tag_off: state.tag_off.clone(),
        face_tag_data: state.tag_data.clone(),
        settlements,
        roads,
        regions: state.regions.clone(),
        face_region: state.face_region.clone(),
        flora: state.flora.clone(),
        structures: state.structures.clone(),
        slope_class: state.face_slope_class.clone(),
        water_depth: state.face_water_depth.clone(),
        landform: state.face_landform.clone(),
        road_material: state.face_road_material.clone(),
    };
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
