use crate::weather::Weather;
use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;

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

    // TAA (main camera) forces depth + motion-vector prepasses. The prepass
    // must sway identically to the main pass, otherwise main-pass fragments
    // fail the depth test against un-swayed prepass depth and flicker black.
    fn prepass_vertex_shader() -> ShaderRef {
        "shaders/foliage_prepass.wgsl".into()
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

/// Swap the standard material for the foliage-extended one at flush time.
/// Queued with `queue_silenced`: the target may legitimately despawn between
/// queue and flush (level reload, catalog re-instantiation), which must not
/// panic.
struct SwapFoliageMaterial {
    material: Handle<FoliageMaterial>,
}

impl bevy::ecs::system::EntityCommand for SwapFoliageMaterial {
    type Out = ();

    fn apply(self, mut entity: EntityWorldMut<'_>) {
        entity.remove::<MeshMaterial3d<StandardMaterial>>();
        entity.insert(MeshMaterial3d(self.material));
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
    mesh_q: Query<
        (Entity, &MeshMaterial3d<StandardMaterial>, &ChildOf),
        Added<MeshMaterial3d<StandardMaterial>>,
    >,
    name_q: Query<&Name>,
    parent_q: Query<&ChildOf>,
) {
    for (entity, mat_handle, parent) in &mesh_q {
        // If already cached, apply immediately and skip any traversal!
        if let Some(cached_handle) = cache.map.get(&mat_handle.0) {
            commands.entity(entity).queue_silenced(SwapFoliageMaterial {
                material: cached_handle.clone(),
            });
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

        // Strip variant suffix (e.g. "flora.tree.2" → "flora.tree") for matching
        let stripped = if let Some((prefix, suffix)) = name.rsplit_once('.') {
            if suffix.chars().all(|c| c.is_ascii_digit()) {
                prefix
            } else {
                name
            }
        } else {
            name
        };

        // Determine wind parameters based on foliage type
        let (speed, amplitude, trample_radius, trample_strength) = match stripped {
            "flora.grass" | "flora.reed" | "flora.seaweed" | "flora.lilypad" | "flora.flower"
            | "flora.mushroom" => (1.8, 0.12, 1.2, 0.8),
            "flora.tree" | "flora.dead_tree" => (0.8, 0.04, 2.0, 0.2), // trees bend slightly when stepped on
            "flora.bush" | "flora.berry" => (1.2, 0.07, 1.5, 0.5),
            _ => (0.0, 0.0, 0.0, 0.0),
        };

        if speed > 0.0
            && let Some(original_std) = standard_mats.get(&mat_handle.0)
        {
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
            cache
                .map
                .insert(mat_handle.0.clone(), foliage_mat_handle.clone());

            commands.entity(entity).queue_silenced(SwapFoliageMaterial {
                material: foliage_mat_handle,
            });
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
    let player_pos = player_q
        .iter()
        .next()
        .map(|tf| tf.translation)
        .unwrap_or(Vec3::ZERO);

    for (_, m) in materials.iter_mut() {
        m.extension.params.wind = wind;
        m.extension.params.player_pos = player_pos;
    }
}
