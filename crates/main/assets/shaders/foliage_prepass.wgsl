// Prepass twin of foliage.wgsl. TAA on the main camera forces a depth +
// motion-vector prepass; without applying the same wind sway here, the main
// pass draws swayed geometry against un-swayed prepass depth, so fragments
// intermittently fail the depth test and blades flash black.
// Keep the sway/trample math in exact sync with foliage.wgsl.

#import bevy_pbr::{
    mesh_functions,
    prepass_io::{Vertex, VertexOutput},
    view_transformations::position_world_to_clip,
}
#import bevy_render::globals::Globals

// The prepass view layout differs from the main pass: globals live at
// @binding(1) here (mesh_view_bindings::globals is @binding(11), main pass
// only), so it must be declared manually.
@group(0) @binding(1) var<uniform> globals: Globals;

struct FoliageParams {
    wind: vec3<f32>,
    sway_speed: f32,
    sway_amplitude: f32,
    player_pos: vec3<f32>,
    trample_radius: f32,
    trample_strength: f32,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> foliage: FoliageParams;

fn foliage_offset(world_pos: vec3<f32>, height_factor: f32) -> vec3<f32> {
    let time = globals.time * foliage.sway_speed;
    let wave = sin(time + world_pos.x * 2.5 + world_pos.z * 2.5) * foliage.sway_amplitude * height_factor;
    let wind_speed = length(foliage.wind);
    let wind_dir = normalize(foliage.wind + vec3<f32>(0.001, 0.0, 0.0));
    let actual_sway = wave * (0.45 + wind_speed * 0.08);
    var offset = vec3<f32>(wind_dir.x, 0.0, wind_dir.z) * actual_sway;

    let swayed = world_pos + offset;
    let diff = swayed - foliage.player_pos;
    let dist = length(diff);
    if (dist < foliage.trample_radius && height_factor > 0.0) {
        let edge_fade = 1.0 - (dist / foliage.trample_radius);
        let push_dir = normalize(diff + vec3<f32>(0.0, 0.001, 0.0));
        let trample_bend = push_dir * edge_fade * foliage.trample_strength * height_factor;
        offset += vec3<f32>(trample_bend.x, 0.0, trample_bend.z);
    }
    return offset;
}

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;

    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    out.world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(vertex.position, 1.0));

    let height_factor = max(vertex.position.y, 0.0);
    let offset = foliage_offset(out.world_position.xyz, height_factor);
    out.world_position = vec4<f32>(out.world_position.xyz + offset, out.world_position.w);

    out.position = position_world_to_clip(out.world_position.xyz);

#ifdef VERTEX_UVS_A
    out.uv = vertex.uv;
#endif

#ifdef NORMAL_PREPASS_OR_DEFERRED_PREPASS
#ifdef VERTEX_NORMALS
    out.world_normal = mesh_functions::mesh_normal_local_to_world(vertex.normal, vertex.instance_index);
#endif
#endif

#ifdef VERTEX_COLORS
    out.color = vertex.color;
#endif

#ifdef MOTION_VECTOR_PREPASS
    // Apply the SAME current-frame offset to the previous position so motion
    // vectors only carry camera/instance motion; TAA then treats the sway as
    // static instead of smearing it.
    let prev_model = mesh_functions::get_previous_world_from_local(vertex.instance_index);
    let prev_world = mesh_functions::mesh_position_local_to_world(prev_model, vec4<f32>(vertex.position, 1.0));
    out.previous_world_position = vec4<f32>(prev_world.xyz + offset, prev_world.w);
#endif

#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = vertex.instance_index;
#endif

    return out;
}
