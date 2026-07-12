// Water surface extension over StandardMaterial. Depth-based colour/opacity from
// the depth prepass (limited underwater visibility) + animated normals for swell.

#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing, alpha_discard},
    forward_io::{VertexOutput, FragmentOutput},
    pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT,
    mesh_view_bindings::globals,
    view_transformations::depth_ndc_to_view_z,
}

#ifdef DEPTH_PREPASS
#import bevy_pbr::prepass_utils::prepass_depth
#endif

struct WaterParams {
    shallow: vec4<f32>,
    deep: vec4<f32>,
    max_visibility: f32,
    wave_amp: f32,
    wave_scale: f32,
    wave_speed: f32,
    flow: f32,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> water: WaterParams;

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    // Standard PBR input (base colour, roughness, N, etc.) from StandardMaterial.
    var pbr_input = pbr_input_from_standard_material(in, is_front);

    // Animated ripples: perturb the surface normal with a couple of scrolling
    // sinusoids so the specular highlight moves and the surface reads as liquid.
    // For rivers (flow > 0) the sample point drifts along the per-vertex flow
    // direction (encoded in vertex colour), so the pattern moves downstream.
    let t = globals.time * water.wave_speed;
    var sample_pos = in.world_position.xyz;
#ifdef VERTEX_COLORS
    if water.flow > 0.0 {
        let flow_dir = in.color.rgb * 2.0 - vec3<f32>(1.0);
        sample_pos = sample_pos - flow_dir * globals.time * water.flow;
    }
#endif
    let p = sample_pos.xz * water.wave_scale;
    let bump = vec3<f32>(
        sin(p.x + t) + sin(p.y * 0.7 - t * 1.3),
        0.0,
        cos(p.y + t) + cos(p.x * 0.7 + t * 1.1),
    ) * water.wave_amp;
    pbr_input.N = normalize(pbr_input.N + bump);

    // Water column thickness from the opaque geometry behind the surface, so
    // shallows are clear and deep water is opaque. Falls back to fully deep when
    // the depth prepass isn't available.
    var thickness = water.max_visibility;
#ifdef DEPTH_PREPASS
    let scene_vz = depth_ndc_to_view_z(prepass_depth(in.position, 0u));
    let surface_vz = depth_ndc_to_view_z(in.position.z);
    thickness = max(surface_vz - scene_vz, 0.0);
#endif
    let fade = clamp(thickness / water.max_visibility, 0.0, 1.0);

    let rgb = mix(water.shallow.rgb, water.deep.rgb, fade);
    let alpha = mix(0.45, 0.96, fade);
    pbr_input.material.base_color = vec4<f32>(rgb, alpha);
    pbr_input.material.base_color =
        alpha_discard(pbr_input.material, pbr_input.material.base_color);

    var out: FragmentOutput;
    if (pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        out.color = apply_pbr_lighting(pbr_input);
    } else {
        out.color = pbr_input.material.base_color;
    }
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
