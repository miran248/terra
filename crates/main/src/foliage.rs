use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;
use crate::weather::Weather;

pub type FoliageMaterial = ExtendedMaterial<StandardMaterial, FoliageExt>;

#[derive(Clone, Copy, ShaderType, Reflect, Debug)]
pub struct FoliageParams {
    pub wind: Vec3,
    pub sway_speed: f32,
    pub sway_amplitude: f32,
    // Interactive Player Trampling
    pub player_pos: Vec3,
    pub trample_radius: f32,
    pub trample_strength: f32,
}

#[derive(Asset, AsBindGroup, Clone, Reflect, Debug)]
pub struct FoliageExt {
    #[uniform(100)]
    pub params: FoliageParams,
}

impl MaterialExtension for FoliageExt {
    fn vertex_shader() -> ShaderRef {
        "shaders/foliage.wgsl".into()
    }
}

#[derive(Resource, Default)]
pub struct FoliageCache {
    pub map: std::collections::HashMap<Handle<StandardMaterial>, Handle<FoliageMaterial>>,
}

pub struct FoliagePlugin;

impl Plugin for FoliagePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<FoliageMaterial>::default())
            .init_resource::<FoliageCache>()
            .add_systems(Update, (apply_foliage_materials, update_foliage_wind));
    }
}

/// Automatically binds loaded catalog meshes to wind-swaying custom shaders.
/// Extremely high-performance: caches material creations and uses direct parent lookups.
#[allow(clippy::type_complexity)]
fn apply_foliage_materials(
    mut commands: Commands,
    mut cache: ResMut<FoliageCache>,
    mut foliage_mats: ResMut<Assets<FoliageMaterial>>,
    standard_mats: Res<Assets<StandardMaterial>>,
    mesh_q: Query<(Entity, &MeshMaterial3d<StandardMaterial>, &ChildOf), Added<MeshMaterial3d<StandardMaterial>>>,
    name_q: Query<&Name>,
    parent_q: Query<&ChildOf>,
) {
    for (entity, mat_handle, parent) in &mesh_q {
        // If already cached, apply immediately and skip any traversal!
        if let Some(cached_handle) = cache.map.get(&mat_handle.0) {
            commands.entity(entity).remove::<MeshMaterial3d<StandardMaterial>>().insert(
                MeshMaterial3d(cached_handle.clone())
            );
            continue;
        }

        // Fast parent lookup (only runs for new/unique catalog materials!)
        let mut current = parent.parent();
        let mut flora_name = None;
        for _ in 0..3 {
            if let Ok(name) = name_q.get(current) {
                let s = name.as_str();
                if s.starts_with("flora.") {
                    flora_name = Some(s);
                    break;
                }
            }
            if let Ok(next_parent) = parent_q.get(current) {
                current = next_parent.parent();
            } else {
                break;
            }
        }

        let Some(name) = flora_name else { continue };

        // Determine wind parameters based on foliage type
        let (speed, amplitude, trample_radius, trample_strength) = match name {
            "flora.grass" | "flora.reed" | "flora.seaweed" | "flora.lilypad" | "flora.flower" | "flora.mushroom" => (1.8, 0.12, 1.2, 0.8),
            "flora.tree" | "flora.dead_tree" => (0.8, 0.04, 2.0, 0.2), // trees bend slightly when stepped on
            "flora.bush" | "flora.berry" => (1.2, 0.07, 1.5, 0.5),
            _ => (0.0, 0.0, 0.0, 0.0),
        };

        if speed > 0.0 {
            if let Some(original_std) = standard_mats.get(&mat_handle.0) {
                let foliage_mat_handle = foliage_mats.add(ExtendedMaterial {
                    base: original_std.clone(),
                    extension: FoliageExt {
                        params: FoliageParams {
                            wind: Vec3::ZERO,
                            sway_speed: speed,
                            sway_amplitude: amplitude,
                            player_pos: Vec3::ZERO,
                            trample_radius,
                            trample_strength,
                        },
                    },
                });

                // Cache the mapping so that none of the other 120,000 instances
                // ever have to run this traversal or asset allocation again!
                cache.map.insert(mat_handle.0.clone(), foliage_mat_handle.clone());

                commands.entity(entity).remove::<MeshMaterial3d<StandardMaterial>>().insert(
                    MeshMaterial3d(foliage_mat_handle)
                );
            }
        }
    }
}

/// Push dynamic weather wind vectors and player positions into the foliage uniforms.
fn update_foliage_wind(
    weather: Res<Weather>,
    player_q: Query<&Transform, With<crate::map::Player>>,
    mut materials: ResMut<Assets<FoliageMaterial>>,
) {
    let wind = weather.wind;
    let player_pos = player_q.iter().next().map(|tf| tf.translation).unwrap_or(Vec3::ZERO);
    
    for (_, m) in materials.iter_mut() {
        m.extension.params.wind = wind;
        m.extension.params.player_pos = player_pos;
    }
}
