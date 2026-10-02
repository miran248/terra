//! Shared, frequently changing shader inputs. Updating this fixed-size buffer
//! preserves material bind groups and avoids per-instance pipeline specialization.
use bevy::prelude::*;
use bevy::render::{render_resource::ShaderType, storage::ShaderBuffer};

#[derive(Clone, Copy, PartialEq, ShaderType)]
struct MotionUniform {
    wind: Vec3,
    wind_swell: f32,
    wind_direction: Vec3,
    wind_sway: f32,
    player_pos: Vec3,
    wind_boost: f32,
}

impl MotionUniform {
    fn new(wind: Vec3, player_pos: Vec3) -> Self {
        let speed = wind.length();
        Self {
            wind,
            wind_swell: 0.4 + (speed * 0.075).min(0.9),
            wind_direction: (wind + Vec3::new(0.001, 0.0, 0.0)).normalize(),
            wind_sway: 0.45 + speed * 0.08,
            player_pos,
            wind_boost: 1.0 + (speed * 0.07).min(0.8),
        }
    }
}

#[derive(Resource)]
pub struct ShaderMotionBuffer {
    pub handle: Handle<ShaderBuffer>,
    current: MotionUniform,
}

impl FromWorld for ShaderMotionBuffer {
    fn from_world(world: &mut World) -> Self {
        let current = MotionUniform::new(Vec3::ZERO, Vec3::ZERO);
        let handle = world
            .resource_mut::<Assets<ShaderBuffer>>()
            .add(ShaderBuffer::from(current));
        Self { handle, current }
    }
}

pub struct ShaderMotionPlugin;

impl Plugin for ShaderMotionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ShaderMotionBuffer>()
            .add_systems(Update, update_shader_motion);
    }
}

fn update_shader_motion(
    weather: Res<crate::weather::Weather>,
    player: Query<&Transform, With<crate::map::Player>>,
    mut motion: ResMut<ShaderMotionBuffer>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
) {
    let player_pos = player.iter().next().map_or(Vec3::ZERO, |tf| tf.translation);
    let current = MotionUniform::new(weather.wind, player_pos);
    if current != motion.current {
        // ShaderBuffer reuses its GPU allocation when size/usage remain fixed.
        // Material handles never change, including during wind or player motion.
        buffers
            .get_mut(&motion.handle)
            .expect("shared shader motion buffer")
            .set_data(current);
        motion.current = current;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foliage::{FoliageExt, FoliageMaterial, FoliageParams};
    use crate::water::{WaterMaterial, water_material};

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<ShaderBuffer>()
            .init_asset::<WaterMaterial>()
            .init_asset::<FoliageMaterial>()
            .init_resource::<crate::weather::Weather>()
            .add_plugins(ShaderMotionPlugin);
        app
    }

    fn scalar(data: &[u8], offset: usize) -> f32 {
        f32::from_le_bytes(data[offset..offset + 4].try_into().unwrap())
    }

    #[test]
    fn changing_wind_and_player_uploads_data_without_invalidating_materials() {
        let mut app = app();
        let motion = app.world().resource::<ShaderMotionBuffer>().handle.clone();
        let water = app
            .world_mut()
            .resource_mut::<Assets<WaterMaterial>>()
            .add(water_material(motion.clone()));
        let foliage = app
            .world_mut()
            .resource_mut::<Assets<FoliageMaterial>>()
            .add(FoliageMaterial {
                base: StandardMaterial::default(),
                extension: FoliageExt {
                    params: FoliageParams {
                        sway_speed: 1.8,
                        sway_amplitude: 0.12,
                        trample_radius: 1.2,
                        trample_strength: 0.8,
                    },
                    motion: motion.clone(),
                },
            });
        app.update();
        app.world_mut()
            .resource_mut::<Messages<AssetEvent<WaterMaterial>>>()
            .clear();
        app.world_mut()
            .resource_mut::<Messages<AssetEvent<FoliageMaterial>>>()
            .clear();
        app.world_mut()
            .resource_mut::<Messages<AssetEvent<ShaderBuffer>>>()
            .clear();
        app.world_mut()
            .resource_mut::<crate::weather::Weather>()
            .wind = Vec3::new(3.0, 0.0, 4.0);
        let player = app
            .world_mut()
            .spawn((
                crate::map::Player {
                    fire_timer: Timer::default(),
                    damage: 0.0,
                    range: 0.0,
                    heading: Vec3::Z,
                },
                Transform::from_xyz(2.0, 3.0, 4.0),
            ))
            .id();
        app.update();
        assert!(
            !app.world_mut()
                .resource_mut::<Messages<AssetEvent<WaterMaterial>>>()
                .drain()
                .any(|event| matches!(event, AssetEvent::Modified { .. }))
        );
        assert!(
            !app.world_mut()
                .resource_mut::<Messages<AssetEvent<FoliageMaterial>>>()
                .drain()
                .any(|event| matches!(event, AssetEvent::Modified { .. }))
        );
        assert_eq!(
            app.world_mut()
                .resource_mut::<Messages<AssetEvent<ShaderBuffer>>>()
                .drain()
                .filter(|event| matches!(event, AssetEvent::Modified { id } if *id == motion.id()))
                .count(),
            1
        );
        let buffers = app.world().resource::<Assets<ShaderBuffer>>();
        let data = buffers.get(&motion).unwrap().data.as_ref().unwrap();
        // Public GPU layout: three vec3/scalar pairs, shared by both materials.
        assert_eq!(data.len(), 48);
        for (offset, expected) in [
            (0, 3.0),
            (4, 0.0),
            (8, 4.0),
            (12, 0.775),
            (16, 0.600128),
            (20, 0.0),
            (24, 0.799904),
            (28, 0.85),
            (32, 2.0),
            (36, 3.0),
            (40, 4.0),
            (44, 1.35),
        ] {
            assert!((scalar(data, offset) - expected).abs() < 1e-5);
        }
        assert_eq!(
            app.world()
                .resource::<Assets<WaterMaterial>>()
                .get(&water)
                .unwrap()
                .extension
                .motion,
            motion
        );
        assert_eq!(
            app.world()
                .resource::<Assets<FoliageMaterial>>()
                .get(&foliage)
                .unwrap()
                .extension
                .motion,
            motion
        );

        // Subsequent player motion changes the shared data, not material handles.
        app.world_mut()
            .entity_mut(player)
            .get_mut::<Transform>()
            .unwrap()
            .translation = Vec3::ONE;
        app.update();
        let data = app
            .world()
            .resource::<Assets<ShaderBuffer>>()
            .get(&motion)
            .unwrap()
            .data
            .as_ref()
            .unwrap();
        assert_eq!(scalar(data, 32), 1.0);
        assert_eq!(scalar(data, 36), 1.0);
        assert_eq!(scalar(data, 40), 1.0);
        let late = water_material(motion.clone());
        assert_eq!(late.extension.motion, motion);
    }

    #[test]
    fn unchanged_inputs_do_not_upload_the_shared_buffer_again() {
        let mut app = app();
        app.update();
        app.world_mut()
            .resource_mut::<Messages<AssetEvent<ShaderBuffer>>>()
            .clear();
        app.update();
        assert!(
            !app.world_mut()
                .resource_mut::<Messages<AssetEvent<ShaderBuffer>>>()
                .drain()
                .any(|event| matches!(event, AssetEvent::Modified { .. }))
        );
    }
}
