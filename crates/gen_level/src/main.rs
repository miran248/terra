mod bitset;
mod pass;

use pass::GenCtx;

/// Strictly coarse → fine; each layer only reads what earlier layers wrote.
///   L0–L3  terrain      — zones, topology (settlements/rivers/roads/bridges),
///                         constrained vertex elevation (all inside TerrainGen)
///   L5a    classify     — zone-aware base type per fine face
///   L4     paint        — snap river/road/bridge polylines + towns to fine faces
///   L5b    transitions  — micro WFC resolves Beach/Cliff/LakeShore/RiverBank
///   —      water names  — flood-fill components, assign names
///   L6     mesh + pack  — displaced trimesh from the L3 verts, tags, postcard
fn main() {
    let ctx = GenCtx::from_env();
    let terrain = pass::gen_terrain(ctx.seed);
    let mut face_types = pass::classify_base(&ctx, &terrain);
    pass::paint_rivers(&ctx, &terrain, &mut face_types);
    pass::normalize_water_bodies(&ctx, &terrain, &mut face_types);
    let mut painted = pass::paint_faces(&ctx, &terrain);
    let vadj = pass::build_vertex_adjacency(&ctx);
    let face_types = pass::resolve_transitions(&ctx, &terrain, &vadj, &face_types);
    let (regions, face_region) = pass::build_regions(&ctx, &terrain, &face_types, &painted);
    let bridges = pass::build_bridges(&ctx, &face_types, &face_region, &mut painted);

    let (terrain_tris, terrain_colors) = pass::build_mesh(&ctx, &terrain, &face_types, &painted);
    let (face_tag_off, face_tag_data) = pass::build_face_tags(&ctx, &painted);
    pass::print_stats(&ctx, &terrain, &face_types);
    println!("regions: {}  bridges: {}", regions.len(), bridges.len());
    pass::serialize(&ctx, &terrain, &face_types, &bridges, regions, face_region,
        terrain_tris, terrain_colors, face_tag_off, face_tag_data);
}
