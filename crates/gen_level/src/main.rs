mod bitset;
mod pass;

use pass::GenCtx;

/// Each pass has exactly one responsibility, receives only its inputs, produces one output.
///   1. terrain       — TerrainGen from seed
///   2. settlements   — Vec<Settlement> from habitable anchors
///   3. classify      — base_types per face
///   4. lakes         — basin detection → face_types
///   5. rivers        — flow trace → face_types
///   6. water_cleanup — min sizes, names → face_types + water_bodies
///   7. shoreline     — Beach/Cliff/LakeShore/RiverBank → face_types
///   8. land_bodies   — continent IDs + settlement→body mapping
///   9. roads         — within same land body → Roads
///  10. bridges       — between bodies → Roads
///  11. terrain_setup — set roads on terrain (needed by mesh for road flattening)
///  12. paint         — BitSets for road/town/bridge faces
///  13. mesh          — vertex-first trimesh + colors + features
///  14. features      — bridge decks
///  15. serialize     — LevelData → postcard
fn main() {
    let ctx = GenCtx::from_env();
    let terrain = pass::gen_terrain(ctx.seed);
    let settlements = pass::place_settlements(&terrain);
    let base_types = pass::classify_base(&ctx, &terrain);
    let base_types_copy = base_types.clone();
    let face_types = pass::detect_lakes(&ctx, &terrain, &base_types);
    let face_types = pass::trace_rivers(&ctx, &terrain, face_types);
    let (face_types, water_bodies) = pass::cleanup_water(&ctx, face_types);
    let face_types = pass::paint_shoreline(&ctx, face_types, &base_types_copy);
    let (land_body, settle_land) = pass::build_land_bodies(&ctx, &face_types, &settlements);
    let roads = pass::build_roads(&ctx, &settlements, &settle_land, &face_types);
    let roads = pass::build_bridges(&ctx, &face_types, &land_body, roads);
    let mut terrain = pass::apply_roads(terrain, &roads);
    let (road_faces, town_faces, bridge_faces, bridge_entry_faces) = pass::paint_faces(&ctx, &roads);

    let (terrain_tris, terrain_colors, terrain_features) = pass::build_mesh(
        &ctx, &terrain, &face_types, &roads, &town_faces, &road_faces, &bridge_faces, &bridge_entry_faces,
    );
    let (feature_tris, feature_colors) = pass::build_features(&ctx, &terrain, &roads);
    pass::serialize(&ctx, &terrain, &roads, &face_types, &water_bodies,
        terrain_tris, terrain_colors, terrain_features, feature_tris, feature_colors);
}
