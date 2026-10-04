//! Opt-in production acceptance walkthrough; compiled only with `asset-review`.
use crate::map::{Player, SunLock, TimeOfDay};
use avian3d::prelude::*;
use bevy::{
    prelude::*,
    render::view::screenshot::{Screenshot, save_to_disk},
};
use shared::{level::LevelData, sphere::SpherePos, state::AppState, terrain::Terrain};
use std::path::PathBuf;

mod planet_acceptance;

pub struct AssetAcceptancePlugin;
impl Plugin for AssetAcceptancePlugin {
    fn build(&self, app: &mut App) {
        if let Some(directory) = std::env::var_os("TERRA_PLANET_ACCEPTANCE_CAPTURE") {
            assert!(
                [
                    "TERRA_PRODUCTION_CAPTURE",
                    "TERRA_LIGHTING_CAPTURE",
                    "TERRA_FLIGHT_CAPTURE",
                ]
                .into_iter()
                .all(|name| std::env::var_os(name).is_none()),
                "TERRA_PLANET_ACCEPTANCE_CAPTURE cannot run beside another capture mode"
            );
            planet_acceptance::register(app, PathBuf::from(directory));
            return;
        }
        if std::env::var_os("TERRA_PRODUCTION_CAPTURE").is_some() {
            app.add_systems(Update, capture.run_if(in_state(AppState::Playing)));
        }
    }
}
struct Stop {
    name: String,
    position: Vec3,
}
#[derive(Default)]
struct Review {
    stops: Vec<Stop>,
    index: usize,
    elapsed: f32,
    started: bool,
    frames: Vec<f32>,
    captured: bool,
    min_clearance: f32,
}

#[allow(
    clippy::too_many_arguments,
    reason = "isolated opt-in acceptance fixture"
)]
fn capture(
    mut commands: Commands,
    time: Res<Time<Real>>,
    mut state: Local<Review>,
    mut player: Query<(&mut Position, &mut LinearVelocity, &mut Player)>,
    meshes: Query<&ViewVisibility, With<Mesh3d>>,
    ground: Res<crate::map::CollisionTerrain>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut sun: ResMut<TimeOfDay>,
    mut sun_lock: ResMut<SunLock>,
    mut exit: MessageWriter<AppExit>,
) {
    let Ok((mut position, mut velocity, mut player)) = player.single_mut() else {
        return;
    };
    if state.stops.is_empty() {
        let level =
            LevelData::from_artifact_bytes(include_bytes!("../assets/level_1337.bin")).unwrap();
        state.stops.push(Stop {
            name: "settlement".into(),
            position: position.0,
        });
        for biome in [
            Terrain::Forest,
            Terrain::Jungle,
            Terrain::Desert,
            Terrain::Snow,
            Terrain::Swamp,
        ] {
            let mut density = std::collections::BTreeMap::<usize, usize>::new();
            for item in &level.scenery {
                if level.face_types[item.face as usize] == biome {
                    *density.entry(item.face as usize).or_default() += 1;
                }
            }
            if let Some((&face, _)) = density
                .iter()
                .max_by_key(|(face, count)| (**count, std::cmp::Reverse(**face)))
            {
                let tri = level.terrain_tris[face].map(Vec3::from_array);
                let surface = (tri[0] + tri[1] + tri[2]) / 3.;
                state.stops.push(Stop {
                    name: format!("{biome:?}").to_lowercase(),
                    position: surface + surface.normalize() * 0.7,
                });
            }
        }
        if std::env::var_os("TERRA_PRODUCTION_FIRST_ONLY").is_some() {
            state.stops.truncate(1);
        }
        std::fs::create_dir_all("/tmp/terra-production-review").unwrap();
    }
    if !state.started {
        position.0 = state.stops[state.index].position;
        velocity.0 = Vec3::ZERO;
        let up = position.0.normalize();
        player.heading = SpherePos::new(up).tangent_basis().1;
        sun.angle = up.z.atan2(up.x);
        sun_lock.0 = true;
        state.min_clearance = f32::INFINITY;
        state.started = true;
        info!(
            stop = state.stops[state.index].name,
            "production acceptance stop"
        );
    }
    let surface_radius = ground
        .0
        .facet_radius(position.0.normalize(), shared::sphere::PLANET_RADIUS);
    state.min_clearance = state
        .min_clearance
        .min(position.0.length() - surface_radius);
    state.elapsed += time.delta_secs();
    if state.elapsed > 2. && state.elapsed < 4. && std::env::var_os("TERRA_CAPTURE_STILL").is_none()
    {
        keys.press(KeyCode::KeyW);
    } else {
        keys.release(KeyCode::KeyW);
    }
    if state.elapsed > 8. {
        state.frames.push(time.delta_secs() * 1000.);
    }
    if state.elapsed > 12. && !state.captured {
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(format!(
                "/tmp/terra-production-review/{}.png",
                state.stops[state.index].name
            )));
        let mut frames = state.frames.clone();
        frames.sort_by(f32::total_cmp);
        let p50 = frames[frames.len() / 2];
        let p95 = frames[frames.len() * 95 / 100];
        info!(
            stop = state.stops[state.index].name,
            p50_ms = p50,
            p95_ms = p95,
            minimum_body_clearance = state.min_clearance,
            visible_meshes = meshes.iter().filter(|v| v.get()).count(),
            total_meshes = meshes.iter().count(),
            "PRODUCTION_ACCEPTANCE"
        );
        state.captured = true;
    }
    if state.elapsed > 13. {
        state.index += 1;
        if state.index == state.stops.len() {
            exit.write(AppExit::Success);
            return;
        }
        state.elapsed = 0.;
        state.frames.clear();
        state.started = false;
        state.captured = false;
    }
}
