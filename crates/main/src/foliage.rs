use crate::shader_motion::ShaderMotionBuffer;
use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::render::storage::ShaderBuffer;
use bevy::shader::ShaderRef;

pub type FoliageMaterial = ExtendedMaterial<StandardMaterial, FoliageExt>;

#[derive(Clone, Copy, ShaderType, Reflect, Debug)]
pub struct FoliageParams {
    pub sway_speed: f32,
    pub sway_amplitude: f32,
    // Interactive Player Trampling
    pub trample_radius: f32,
    pub trample_strength: f32,
}

#[derive(Asset, AsBindGroup, Clone, Reflect, Debug)]
pub struct FoliageExt {
    #[uniform(100)]
    pub params: FoliageParams,
    #[storage(101, read_only)]
    pub motion: Handle<ShaderBuffer>,
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
            .add_systems(Update, apply_foliage_materials);
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
#[expect(
    clippy::type_complexity,
    clippy::too_many_arguments,
    reason = "Bevy ECS queries and resources are separate system parameters"
)]
fn apply_foliage_materials(
    motion: Res<ShaderMotionBuffer>,
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
    for (entity, mat_handle, _parent) in &mesh_q {
        // If already cached, apply immediately and skip any traversal!
        if let Some(cached_handle) = cache.map.get(&mat_handle.0) {
            commands.entity(entity).queue_silenced(SwapFoliageMaterial {
                material: cached_handle.clone(),
            });
            continue;
        }

        // Fast parent lookup (only runs for new/unique catalog materials!)
        let mut scenery_name = None;
        for current in parent_q.iter_ancestors(entity) {
            if let Ok(name) = name_q.get(current) {
                let s = name.as_str();
                // Multipart Blender nodes also start with scenery.; only a canonical
                // scene name (optionally followed by a numeric variant) identifies a kind.
                let base = s
                    .rsplit_once('.')
                    .filter(|(_, suffix)| suffix.chars().all(|c| c.is_ascii_digit()))
                    .map_or(s, |(base, _)| base);
                if shared::art::SCENERY_KINDS.iter().any(|kind| {
                    use shared::art::AssetName;
                    kind.asset_name() == base
                }) {
                    scenery_name = Some(s);
                    break;
                }
            }
        }

        let Some(name) = scenery_name else { continue };

        // Strip variant suffix (e.g. "scenery.tree.2" → "scenery.tree") for matching
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
            "scenery.grass" | "scenery.reed" | "scenery.seaweed" | "scenery.lilypad"
            | "scenery.flower" | "scenery.mushroom" => (1.8, 0.12, 1.2, 0.8),
            "scenery.tree" | "scenery.dead_tree" => (0.8, 0.04, 2.0, 0.2), // trees bend slightly when stepped on
            "scenery.bush" | "scenery.berry" => (1.2, 0.07, 1.5, 0.5),
            _ => (0.0, 0.0, 0.0, 0.0),
        };

        if speed > 0.0
            && let Some(original_std) = standard_mats.get(&mat_handle.0)
        {
            let foliage_mat_handle = foliage_mats.add(ExtendedMaterial {
                base: original_std.clone(),
                extension: FoliageExt {
                    params: FoliageParams {
                        sway_speed: speed,
                        sway_amplitude: amplitude,
                        trample_radius,
                        trample_strength,
                    },
                    motion: motion.handle.clone(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multipart_tree_resolves_its_scene_ancestor_for_wind_material() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<ShaderBuffer>()
            .init_asset::<StandardMaterial>()
            .init_asset::<FoliageMaterial>()
            .init_resource::<ShaderMotionBuffer>()
            .init_resource::<FoliageCache>()
            .add_systems(Update, apply_foliage_materials);
        let material = app
            .world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial::default());
        let root = app.world_mut().spawn(Name::new("scenery.tree.0")).id();
        let part = app
            .world_mut()
            .spawn((Name::new("scenery.tree.0.trunk"), ChildOf(root)))
            .id();
        let mesh = app
            .world_mut()
            .spawn((MeshMaterial3d(material), ChildOf(part)))
            .id();
        app.update();
        assert!(
            app.world()
                .get::<MeshMaterial3d<FoliageMaterial>>(mesh)
                .is_some()
        );
        assert!(
            app.world()
                .get::<MeshMaterial3d<StandardMaterial>>(mesh)
                .is_none()
        );
    }
}
