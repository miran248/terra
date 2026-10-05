//! Weather: randomized fronts drive a wind vector and a single precipitation
//! intensity. Whether that precipitation reads as rain or snow is decided
//! per-location by the local temperature, so one front snows on cold peaks and
//! rains in warm lowlands. Rendering is a camera-anchored pool of instanced
//! particles (streaks for rain, flakes for snow) that wrap around the camera.
//!
//! Wind-driven flora sway and snow accumulation on the terrain both need a
//! custom vertex/material shader (CPU can't sway 22k instances or repaint baked
//! vertex colours) and are staged as a follow-up; the `snow_accum` value is
//! already tracked here so that pass can consume it.

use bevy::prelude::*;
use shared::state::AppState;

use crate::map::{MainCamera, Player};

/// Number of precipitation particles in the pool.
const POOL: u32 = 1500;
/// Half-extent of the cube of particles kept around the camera.
const BOX_HALF: f32 = 28.0;

/// Below this local temperature (°C) precipitation falls as snow, else rain.
pub fn is_snow(temp_c: f32) -> bool {
    temp_c < 0.5
}

/// Global weather state. `precip` is overall precipitation intensity (0..1);
/// rain vs snow is resolved from local temperature at read time.
#[derive(Resource)]
pub struct Weather {
    pub wind: Vec3,
    pub precip: f32,
    pub snow_accum: f32,

    target_precip: f32,
    target_wind: Vec3,
    front_timer: f32,
    rng: u32,
}

impl Default for Weather {
    fn default() -> Self {
        Self {
            wind: Vec3::ZERO,
            precip: 0.0,
            snow_accum: 0.0,
            target_precip: 0.0,
            target_wind: Vec3::ZERO,
            front_timer: 8.0,
            rng: 0x9E3779B9,
        }
    }
}

impl Weather {
    /// Hold an authored weather front while keeping the normal transition and particles.
    #[cfg(feature = "asset-review")]
    pub(crate) fn showcase_front(&mut self, precip: f32, wind: Vec3) {
        self.target_precip = precip;
        self.target_wind = wind;
        self.front_timer = f32::INFINITY;
    }

    fn next_u32(&mut self) -> u32 {
        self.rng = self.rng.wrapping_mul(1664525).wrapping_add(1013904223);
        self.rng
    }
    fn next_f32(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 / (1u32 << 24) as f32
    }
}

/// A single precipitation particle; `idx` gives it a stable slot in the pool so
/// intensity can activate a deterministic fraction of the pool.
#[derive(Component)]
pub(crate) struct Precip {
    pub(crate) idx: u32,
}

pub struct WeatherPlugin;

impl Plugin for WeatherPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Weather>()
            .add_systems(OnEnter(AppState::Playing), setup_particles)
            .add_systems(
                Update,
                (advance_weather, apply_precip)
                    .chain()
                    .after(crate::exploration::ExplorationUpdate)
                    .run_if(in_state(AppState::Playing)),
            );
    }
}

fn setup_particles(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let mesh = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    let mat = materials.add(StandardMaterial {
        base_color: Color::srgba(0.85, 0.9, 1.0, 0.85),
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        ..default()
    });
    for idx in 0..POOL {
        commands.spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(mat.clone()),
            // Start collapsed (inactive) until weather activates it.
            Transform::from_scale(Vec3::ZERO),
            Visibility::Inherited,
            Precip { idx },
        ));
    }
}

/// Rolls new weather fronts on a random cadence and eases current weather toward
/// the front's targets.
fn advance_weather(time: Res<Time>, mut w: ResMut<Weather>) {
    let dt = time.delta_secs();

    w.front_timer -= dt;
    if w.front_timer <= 0.0 {
        // ~45% of fronts are clearing; the rest bring some precipitation.
        let r = w.next_f32();
        w.target_precip = if r < 0.45 {
            0.0
        } else {
            0.3 + w.next_f32() * 0.7
        };
        let ang = w.next_f32() * std::f32::consts::TAU;
        let strength = 2.0 + w.next_f32() * 10.0;
        w.target_wind = Vec3::new(ang.cos(), 0.0, ang.sin()) * strength;
        w.front_timer = 25.0 + w.next_f32() * 50.0;
    }

    // Smoothly ease toward the front's targets.
    let rate = (dt * 0.15).clamp(0.0, 1.0);
    let dp = w.target_precip - w.precip;
    w.precip += dp * rate;
    let dw = w.target_wind - w.wind;
    w.wind += dw * rate;

    // Snow accumulates while snowing and melts slowly otherwise. (Consumed by the
    // future accumulation shader; harmless to track now.)
    let snowing = w.precip > 0.2;
    let accum_target = if snowing { w.precip } else { 0.0 };
    let da = accum_target - w.snow_accum;
    w.snow_accum += da * (dt * 0.05).clamp(0.0, 1.0);
}

fn wrap(x: f32, half: f32) -> f32 {
    (x + half).rem_euclid(2.0 * half) - half
}

fn set_precip_visibility(visibility: &mut Visibility, hidden: bool) {
    let target = if hidden {
        Visibility::Hidden
    } else {
        Visibility::Inherited
    };
    if *visibility != target {
        *visibility = target;
    }
}

/// Moves the particle pool: each active particle falls (plus wind), wrapping
/// inside a cube centred on the camera. Rain streaks / snow flakes are chosen
/// from the local temperature under the player.
#[allow(
    clippy::type_complexity,
    reason = "Bevy ECS query filters encode access rules"
)]
fn apply_precip(
    time: Res<Time>,
    mut w: ResMut<Weather>,
    exploration: Option<Res<crate::exploration::Exploration>>,
    terrain: Option<Res<shared::terrain::TerrainGen>>,
    cam_q: Query<&Transform, With<MainCamera>>,
    player_q: Query<&Transform, With<Player>>,
    mut precip_q: Query<
        (&Precip, &mut Transform, &mut Visibility),
        (Without<MainCamera>, Without<Player>),
    >,
) {
    let Ok(cam) = cam_q.single() else { return };

    let hide_precipitation = exploration
        .as_deref()
        .is_some_and(crate::exploration::Exploration::is_planet_view_active);
    let intensity = w.precip;
    if intensity < 0.02 {
        for (_, mut tf, mut visibility) in &mut precip_q {
            if tf.scale != Vec3::ZERO {
                tf.scale = Vec3::ZERO;
            }
            set_precip_visibility(&mut visibility, hide_precipitation);
        }
        return;
    }

    // Local temperature under the player decides rain vs snow.
    let snow = match (player_q.single(), terrain.as_ref()) {
        (Ok(p), Some(t)) => {
            is_snow(t.temperature_at(terra_geometry::sphere::SpherePos::new(p.translation)))
        }
        _ => false,
    };

    let up = cam.translation.normalize();
    let e1 = up.cross(Vec3::Y).try_normalize().unwrap_or(Vec3::X);
    let e2 = up.cross(e1).normalize();
    // Bias the box slightly ahead of and above the camera so particles fill the view.
    let center = cam.translation + cam.forward() * 6.0 + up * 4.0;

    let (speed, scale) = if snow {
        (3.5, Vec3::new(0.06, 0.06, 0.06))
    } else {
        (30.0, Vec3::new(0.015, 0.5, 0.015))
    };
    let wind = w.wind * if snow { 1.4 } else { 0.5 };
    let vel = -up * speed + wind;
    let fall = vel.try_normalize().unwrap_or(-up);
    let rot = Quat::from_rotation_arc(Vec3::Y, fall);
    let dt = time.delta_secs();
    let step = vel * dt;

    for (p, mut tf, mut visibility) in &mut precip_q {
        set_precip_visibility(&mut visibility, hide_precipitation);

        // Activate a deterministic fraction of the pool by index.
        let active = (p.idx as f32) < intensity * POOL as f32;
        if !active {
            if tf.scale != Vec3::ZERO {
                tf.scale = Vec3::ZERO;
            }
            continue;
        }

        let mut pos = tf.translation;
        // Seed a fresh position on first activation (particle was collapsed).
        if tf.scale == Vec3::ZERO {
            let a = (w.next_f32() * 2.0 - 1.0) * BOX_HALF;
            let b = (w.next_f32() * 2.0 - 1.0) * BOX_HALF;
            let c = (w.next_f32() * 2.0 - 1.0) * BOX_HALF;
            pos = center + e1 * a + e2 * b + up * c;
        }

        pos += step;
        let rel = pos - center;
        let mut a = rel.dot(e1);
        let mut b = rel.dot(e2);
        let mut c = rel.dot(up);
        a = wrap(a, BOX_HALF);
        b = wrap(b, BOX_HALF);
        if c < -BOX_HALF {
            // Fell out the bottom — respawn somewhere across the top of the column
            // (not a crisp sheet at the very top) with a new horizontal spot, so
            // re-entries stay staggered and the field reads as continuous.
            c = BOX_HALF * (0.5 + 0.5 * w.next_f32());
            a = (w.next_f32() * 2.0 - 1.0) * BOX_HALF;
            b = (w.next_f32() * 2.0 - 1.0) * BOX_HALF;
        } else {
            c = wrap(c, BOX_HALF);
        }
        tf.translation = center + e1 * a + e2 * b + up * c;
        tf.rotation = rot;
        tf.scale = scale;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{exploration::ExplorationPlugin, map::Ground};
    use avian3d::prelude::*;

    fn fixture() -> (App, Entity) {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            TransformPlugin,
            AssetPlugin::default(),
            bevy::state::app::StatesPlugin,
            PhysicsPlugins::default(),
        ))
        .init_state::<AppState>()
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<ButtonInput<MouseButton>>()
        .add_message::<bevy::input::mouse::MouseWheel>()
        .init_asset::<Mesh>()
        .init_asset::<StandardMaterial>()
        .insert_resource(Gravity::ZERO)
        .insert_resource(crate::map::WorldEpoch::new(1))
        .insert_resource(SubstepCount(12))
        .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f32(1.0 / 60.0),
        ))
        .add_plugins((ExplorationPlugin, WeatherPlugin));
        app.world_mut().spawn((
            RigidBody::Static,
            Collider::cuboid(1000.0, 1.0, 1000.0),
            Transform::from_xyz(0.0, 1999.5, 0.0),
            Ground,
        ));
        let explorer = app
            .world_mut()
            .spawn(crate::map::player_physics_bundle(
                Vec3::new(0.0, 2000.6, 0.0),
                Vec3::NEG_Z,
            ))
            .id();
        app.finish();
        app.cleanup();
        for _ in 0..3 {
            app.update();
        }
        app.world_mut()
            .insert_resource(State::new(AppState::Playing));
        (app, explorer)
    }

    #[test]
    fn weather_plugin_runs_without_exploration() {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            TransformPlugin,
            AssetPlugin::default(),
            bevy::state::app::StatesPlugin,
        ))
        .init_state::<AppState>()
        .init_asset::<Mesh>()
        .init_asset::<StandardMaterial>()
        .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f32(1.0 / 60.0),
        ))
        .add_plugins(WeatherPlugin);
        app.world_mut().spawn((
            MainCamera,
            Transform::from_xyz(0.0, 2005.0, -5.0).looking_at(Vec3::new(0.0, 2000.6, 0.0), Vec3::Y),
        ));
        let particle = app
            .world_mut()
            .spawn((
                Precip { idx: 0 },
                Transform::from_xyz(1.0, 2003.0, 2.0),
                Visibility::Inherited,
            ))
            .id();
        app.finish();
        app.cleanup();
        app.world_mut()
            .insert_resource(State::new(AppState::Playing));
        {
            let mut weather = app.world_mut().resource_mut::<Weather>();
            weather.precip = 0.8;
            weather.front_timer = 1000.0;
        }

        app.update();

        assert_eq!(
            *app.world().get::<Visibility>(particle).unwrap(),
            Visibility::Inherited
        );
        assert_ne!(
            app.world().get::<Transform>(particle).unwrap().scale,
            Vec3::ZERO
        );
    }

    #[test]
    fn planet_view_hides_rain_and_snow_through_reversals_without_pausing_weather() {
        let (mut app, player) = fixture();
        let camera = app
            .world_mut()
            .spawn((
                MainCamera,
                Transform::from_xyz(0.0, 2005.0, -5.0)
                    .looking_at(Vec3::new(0.0, 2000.6, 0.0), Vec3::Y),
            ))
            .id();
        let particle = app
            .world_mut()
            .spawn((
                Precip { idx: 0 },
                Transform::from_xyz(1.0, 2003.0, 2.0),
                Visibility::Inherited,
            ))
            .id();
        {
            let mut weather = app.world_mut().resource_mut::<Weather>();
            weather.precip = 0.8;
            weather.target_precip = 0.0;
            weather.front_timer = 1000.0;
        }
        app.update();
        assert_eq!(
            *app.world().get::<Visibility>(particle).unwrap(),
            Visibility::Inherited
        );
        assert_ne!(
            app.world().get::<Transform>(particle).unwrap().scale,
            Vec3::ZERO
        );

        let initial_precip = app.world().resource::<Weather>().precip;
        let initial_particle_position = app.world().get::<Transform>(particle).unwrap().translation;
        app.world_mut()
            .resource_mut::<crate::exploration::Exploration>()
            .set_planet_view_open(true);

        for _ in 0..8 {
            app.update();
            assert!(
                app.world()
                    .resource::<crate::exploration::Exploration>()
                    .is_planet_view_active()
            );
            assert_eq!(
                *app.world().get::<Visibility>(particle).unwrap(),
                Visibility::Hidden,
                "precipitation should be hidden while Planet view opens"
            );
        }
        let hidden_position = app.world().get::<Transform>(particle).unwrap().translation;
        assert_ne!(hidden_position, initial_particle_position);
        assert!(app.world().resource::<Weather>().precip < initial_precip);

        // Start without terrain so this is rain, then add a cold pole beneath
        // the player while the view is open. The same pool now renders snow,
        // which must remain hidden without pausing its movement.
        assert_eq!(
            app.world().get::<Transform>(particle).unwrap().scale,
            Vec3::new(0.015, 0.5, 0.015)
        );
        let terrain = shared::terrain::TerrainGen::init(1337);
        let player_position = app.world().get::<Transform>(player).unwrap().translation;
        assert!(is_snow(terrain.temperature_at(
            terra_geometry::sphere::SpherePos::new(player_position)
        )));
        app.insert_resource(terrain);
        let before_snow_updates = app.world().get::<Transform>(particle).unwrap().translation;
        for _ in 0..4 {
            app.update();
            assert_eq!(
                *app.world().get::<Visibility>(particle).unwrap(),
                Visibility::Hidden,
                "snow should be hidden while Planet view remains open"
            );
            assert_eq!(
                app.world().get::<Transform>(particle).unwrap().scale,
                Vec3::splat(0.06),
                "the precipitation pool should continue using its snow path"
            );
        }
        assert_ne!(
            app.world().get::<Transform>(particle).unwrap().translation,
            before_snow_updates,
            "snow particles should keep moving while hidden"
        );

        app.world_mut()
            .resource_mut::<crate::exploration::Exploration>()
            .set_planet_view_open(false);
        for _ in 0..8 {
            app.update();
            assert!(
                app.world()
                    .resource::<crate::exploration::Exploration>()
                    .is_planet_view_active()
            );
            assert_eq!(
                *app.world().get::<Visibility>(particle).unwrap(),
                Visibility::Hidden
            );
        }
        app.world_mut()
            .resource_mut::<crate::exploration::Exploration>()
            .set_planet_view_open(true);
        for _ in 0..8 {
            app.update();
            assert_eq!(
                *app.world().get::<Visibility>(particle).unwrap(),
                Visibility::Hidden
            );
        }
        app.world_mut()
            .resource_mut::<crate::exploration::Exploration>()
            .set_planet_view_open(false);

        let mut returned = false;
        for _ in 0..160 {
            app.update();
            let active = app
                .world()
                .resource::<crate::exploration::Exploration>()
                .is_planet_view_active();
            if active {
                assert_eq!(
                    *app.world().get::<Visibility>(particle).unwrap(),
                    Visibility::Hidden
                );
            } else {
                assert_eq!(
                    *app.world().get::<Visibility>(particle).unwrap(),
                    Visibility::Inherited,
                    "precipitation should render again after camera return"
                );
                assert_eq!(
                    app.world().get::<Transform>(particle).unwrap().scale,
                    Vec3::splat(0.06),
                    "snow should render again after camera return"
                );
                returned = true;
                break;
            }
        }

        assert!(returned, "the camera return should eventually finish");
        assert!(app.world().get::<Transform>(camera).is_some());
    }
}
