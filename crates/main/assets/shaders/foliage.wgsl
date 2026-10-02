#import "shaders/shader_motion.wgsl"::motion

#import bevy_pbr::{
    mesh_functions,
    forward_io::{Vertex, VertexOutput},
    mesh_view_bindings::{globals, view},
}

struct FoliageParams {
    sway_speed: f32,
    sway_amplitude: f32,
    // Interactive Player Trampling
    trample_radius: f32,
    trample_strength: f32,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> foliage: FoliageParams;


// Wind sway + player trample as a world-space offset. MUST stay in exact sync
// with foliage_prepass.wgsl: TAA's depth prepass renders this same geometry,
// and any divergence makes main-pass fragments fail the depth test (black
// flickering blades).
fn foliage_offset(world_pos: vec3<f32>, height_factor: f32) -> vec3<f32> {
    // Anchored vertices cannot sway or trample.
    if height_factor <= 0.0 {
        return vec3<f32>(0.0);
    }
    let time = globals.time * foliage.sway_speed;
    // Animated sinusoidal wave based on WORLD position (prevents unison swaying)
    let wave = sin(time + world_pos.x * 2.5 + world_pos.z * 2.5) * foliage.sway_amplitude * height_factor;
    let wind_dir = motion.wind_direction;
    // Scale sway amplitude smoothly by wind speed (subtle rustle to wind bend)
    let actual_sway = wave * motion.wind_sway;
    var offset = vec3<f32>(wind_dir.x, 0.0, wind_dir.z) * actual_sway;

    // Interactive trample bending (player pushing foliage away physically)
    let swayed = world_pos + offset;
    let diff = swayed - motion.player_pos;
    let dist_squared = dot(diff, diff);
    if (foliage.trample_radius > 0.0 && dist_squared < foliage.trample_radius * foliage.trample_radius) {
        let dist = sqrt(dist_squared);
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

    // First transform the vertex to WORLD space (fixes local rotation skewing!)
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    out.world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(vertex.position, 1.0));

    // High vertices sway more (Y > 0), base stays fixed (Y = 0)
    let height_factor = max(vertex.position.y, 0.0);
    let offset = foliage_offset(out.world_position.xyz, height_factor);
    out.world_position = vec4<f32>(out.world_position.xyz + offset, out.world_position.w);

    out.position = view.clip_from_world * out.world_position;

    // Use Bevy built-in normal transformer which correctly handles inverse transposes and scaling!
    out.world_normal = mesh_functions::mesh_normal_local_to_world(vertex.normal, vertex.instance_index);

#ifdef VERTEX_UVS_A
    out.uv = vertex.uv;
#endif

#ifdef VERTEX_COLORS
    out.color = vertex.color;
#endif

// Critical fix to prevent instanced shading from falling back to index 0 (black flickering!)
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = vertex.instance_index;
#endif

    return out;
}
