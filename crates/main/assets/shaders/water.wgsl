// Water surface extension over StandardMaterial. Depth-based colour/opacity from
// the depth prepass (limited underwater visibility) + animated normals for swell.

#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing, alpha_discard},
    forward_io::{VertexOutput, FragmentOutput},
    pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT,
    mesh_view_bindings::{globals, view},
    view_transformations::depth_ndc_to_view_z,
}

#ifdef DEPTH_PREPASS
#import bevy_pbr::prepass_utils::prepass_depth
#endif

struct WaterParams {
    shallow: vec4<f32>,
    deep: vec4<f32>,
    wind: vec3<f32>,
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

    // Animated ripples: perturb the surface normal with scrolling sinusoids so
    // the specular highlight moves and the surface reads as liquid.
    // Rivers (flow > 0) drift downstream along the per-vertex flow direction
    // (encoded in vertex colour); still water drifts downwind instead.
    let t = globals.time * water.wave_speed;
    let wind_speed = length(water.wind);
    var sample_pos = in.world_position.xyz;
    var drifted = false;
#ifdef VERTEX_COLORS
    if water.flow > 0.0 {
        let flow_dir = in.color.rgb * 2.0 - vec3<f32>(1.0);
        sample_pos = sample_pos - flow_dir * globals.time * water.flow;
        drifted = true;
    }
#endif
    if (!drifted) {
        // Ripples travel downwind at a fraction of wind speed (deep-water
        // waves lag the wind); calm air leaves them drifting on time alone.
        sample_pos = sample_pos - water.wind * globals.time * 0.35;
    }
    // Sample in all three world axes: on a spherical planet the surface plane
    // isn't world-XZ, so XZ-only ripples stretch into streaks near the poles.
    let base_n = pbr_input.N;
    let p = sample_pos * water.wave_scale;
    // Two octaves: broad swell + finer chop for close-up detail.
    let swell = vec3<f32>(
        sin(p.x + t) + sin(p.y * 0.7 - t * 1.3),
        sin(p.y + t * 0.8) + sin(p.z * 0.6 + t),
        cos(p.z + t) + cos(p.x * 0.7 + t * 1.1),
    );
    let q = p * 3.1;
    let chop = vec3<f32>(
        sin(q.x - t * 2.1) * cos(q.z + t * 1.7),
        sin(q.z + t * 1.9),
        cos(q.x + t * 2.3) * sin(q.y - t * 1.5),
    );
    // Wind roughens the surface: calm = glassy swell, storm (wind ~12) = chop
    // up to ~1.8x. Chop responds twice as strongly as the swell.
    let wind_boost = 1.0 + min(wind_speed * 0.07, 0.8);
    let bump = (swell + chop * 0.35 * wind_boost) * water.wave_amp * (0.6 + 0.4 * wind_boost);
    // Perturb in the surface's tangent plane only, so ripples tilt the normal
    // instead of shrinking it (keeps lighting stable at any planet latitude).
    let tangent_bump = bump - base_n * dot(bump, base_n);
    pbr_input.N = normalize(base_n + tangent_bump);

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

    var rgb = mix(water.shallow.rgb, water.deep.rgb, fade);
    var alpha = mix(0.45, 0.96, fade);

    // Fresnel: at grazing angles real water is a mirror — more opaque and a
    // touch brighter — while looking straight down it stays clear.
    let view_dir = normalize(view.world_position - in.world_position.xyz);
    let fresnel = pow(1.0 - clamp(dot(view_dir, pbr_input.N), 0.0, 1.0), 4.0);
    alpha = mix(alpha, 1.0, fresnel * 0.7);
    rgb = mix(rgb, water.shallow.rgb * 1.35, fresnel * 0.5);

    // Shoreline foam effect
    var foam = 0.0;
#ifdef DEPTH_PREPASS
    let foam_width = 0.55;
    if (thickness < foam_width) {
        let edge_fade = 1.0 - (thickness / foam_width);
        // Scrolling ripples towards the shoreline using wave frequencies
        let wave_pulse = sin(globals.time * 2.8 - thickness * 18.0) * 0.5 + 0.5;
        // Make the foam sparkly and noisy with high-frequency world position
        // variations (3D hash — a 2D xz hash bands into stripes on a sphere)
        let sparkles = fract(sin(dot(in.world_position.xyz, vec3<f32>(12.9898, 78.233, 37.719))) * 43758.5453) * 0.15;
        foam = edge_fade * (0.55 + 0.3 * wave_pulse + sparkles);
    }
#endif

    var final_color = vec4<f32>(rgb, alpha);
    if (foam > 0.02) {
        let foam_color = vec3<f32>(0.95, 0.97, 1.0);
        final_color = vec4<f32>(mix(final_color.rgb, foam_color, foam), max(final_color.a, foam * 0.9));
    }

    pbr_input.material.base_color = final_color;
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
