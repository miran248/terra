//! Water rendering (Phase 1): an `ExtendedMaterial<StandardMaterial, WaterExt>`
//! that gives the ocean depth-based colour + opacity (limited underwater
//! visibility) from the depth prepass and animated surface normals for swell.
//! Base PBR lighting/specular comes from `StandardMaterial`, so the sun glints
//! off the surface. Per-body meshes and river flow are later phases.

use bevy::asset::Asset;
use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;

pub type WaterMaterial = ExtendedMaterial<StandardMaterial, WaterExt>;

/// Extension uniforms (group 2, binding 100). Colours are linear.
#[derive(Clone, Copy, ShaderType, Reflect, Debug)]
pub struct WaterParams {
    pub shallow: LinearRgba,
    pub deep: LinearRgba,
    /// View distance (world units) at which water becomes fully deep/opaque.
    pub max_visibility: f32,
    pub wave_amp: f32,
    pub wave_scale: f32,
    pub wave_speed: f32,
}

#[derive(Asset, AsBindGroup, Clone, Reflect, Debug)]
pub struct WaterExt {
    #[uniform(100)]
    pub params: WaterParams,
}

impl MaterialExtension for WaterExt {
    fn fragment_shader() -> ShaderRef {
        "shaders/water.wgsl".into()
    }
}

/// Build the configured ocean material. Kept as a helper so the spawn site
/// (map::setup_map, which owns the `Ground` marker for level cleanup) stays simple.
pub fn water_material() -> WaterMaterial {
    ExtendedMaterial {
        base: StandardMaterial {
            // Alpha is driven by the shader (depth fade); this base colour mostly
            // feeds specular/roughness. Smooth + reflective so the sun glints.
            base_color: Color::srgba(0.2, 0.42, 0.6, 0.7),
            perceptual_roughness: 0.08,
            reflectance: 0.5,
            alpha_mode: AlphaMode::Blend,
            cull_mode: None,
            ..default()
        },
        extension: WaterExt {
            params: WaterParams {
                shallow: LinearRgba::rgb(0.10, 0.42, 0.48),
                deep: LinearRgba::rgb(0.01, 0.06, 0.18),
                max_visibility: 14.0,
                wave_amp: 0.05,
                wave_scale: 0.35,
                wave_speed: 0.7,
            },
        },
    }
}

pub struct WaterPlugin;

impl Plugin for WaterPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<WaterMaterial>::default());
    }
}
