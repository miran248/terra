#import bevy_pbr::{
    mesh_functions,
    forward_io::{Vertex, VertexOutput},
    mesh_view_bindings::{globals, view},
}

struct FoliageParams {
    wind: vec3<f32>,
    sway_speed: f32,
    sway_amplitude: f32,
    // Interactive Player Trampling
    player_pos: vec3<f32>,
    trample_radius: f32,
    trample_strength: f32,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> foliage: FoliageParams;

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;

    // First transform the vertex to WORLD space (fixes local rotation skewing!)
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    out.world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(vertex.position, 1.0));

    // High vertices sway more (Y > 0), base stays fixed (Y = 0)
    let height_factor = max(vertex.position.y, 0.0);
    let time = globals.time * foliage.sway_speed;
    
    // Create animated sinusoidal wave based on WORLD position (prevents unison swaying!)
    let wave = sin(time + out.world_position.x * 2.5 + out.world_position.z * 2.5) * foliage.sway_amplitude * height_factor;
    
    // Calculate wind speed and safe normalized direction vector
    let wind_speed = length(foliage.wind);
    let wind_dir = normalize(foliage.wind + vec3<f32>(0.001, 0.0, 0.0));
    
    // Scale sway amplitude smoothly by wind speed (subtle rustle to wind bend)
    let actual_sway = wave * (0.45 + wind_speed * 0.08);
    let wind_sway = wind_dir * actual_sway;

    // Apply wind sway directly to WORLD space position
    out.world_position.x += wind_sway.x;
    out.world_position.z += wind_sway.z;

    // Interactive Trample Bending (Player pushing foliage away physically)
    let diff = out.world_position.xyz - foliage.player_pos;
    let dist = length(diff);
    if (dist < foliage.trample_radius && height_factor > 0.0) {
        let edge_fade = 1.0 - (dist / foliage.trample_radius);
        
        // Push outward from the player center, flat on the ground plane
        let push_dir = normalize(diff + vec3<f32>(0.0, 0.001, 0.0));
        let trample_bend = push_dir * edge_fade * foliage.trample_strength * height_factor;
        
        // Offset world position
        out.world_position.x += trample_bend.x;
        out.world_position.z += trample_bend.z;
    }

    // Version-safe, mathematical world-to-clip mapping!
    out.position = view.clip_from_world * out.world_position;

    // Use Bevy built-in normal transformer which correctly handles inverse transposes and scaling!
    out.world_normal = mesh_functions::mesh_normal_local_to_world(vertex.normal, vertex.instance_index);
    
#ifdef VERTEX_UVS
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
