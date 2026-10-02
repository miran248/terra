// Must match shader_motion::MotionUniform; shared by water and both foliage passes.
struct MotionUniform {
    wind: vec3<f32>,
    wind_swell: f32,
    wind_direction: vec3<f32>,
    wind_sway: f32,
    player_pos: vec3<f32>,
    wind_boost: f32,
};
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var<storage, read> motion: MotionUniform;
