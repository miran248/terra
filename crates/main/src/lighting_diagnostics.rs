//! Opt-in, frozen-scene evidence fixture for #31. Never changes production shaders.
use crate::map::{MainCamera, Player, Sun, SunLock, TimeOfDay};
use bevy::{
    camera::Exposure,
    prelude::*,
    render::view::screenshot::{Screenshot, save_to_disk},
    transform::TransformSystems,
    window::PrimaryWindow,
};
use shared::state::AppState;
use std::{io::Write, path::PathBuf};
use terra_geometry::sphere::SpherePos;
use terra_world::level::{LevelData, RoadKind, StructureKind, WaterPhase};

pub struct LightingDiagnosticsPlugin;
impl Plugin for LightingDiagnosticsPlugin {
    fn build(&self, app: &mut App) {
        let Some(directory) = std::env::var_os("TERRA_LIGHTING_CAPTURE") else {
            return;
        };
        if std::env::var("TERRA_LIGHTING_PROBE").as_deref() == Ok("no-atmosphere") {
            // Remove before the first render extraction; live teardown leaves dependent GPU resources.
            app.add_systems(Startup, without_atmosphere.after(crate::setup_camera));
        }
        app.insert_resource(Capture::new(directory.into()))
            .insert_resource(bevy::winit::WinitSettings {
                focused_mode: bevy::winit::UpdateMode::Continuous,
                unfocused_mode: bevy::winit::UpdateMode::Continuous,
            })
            .add_systems(PreUpdate, prepare.run_if(in_state(AppState::Playing)))
            .add_systems(
                PostUpdate,
                record
                    .before(TransformSystems::Propagate)
                    .run_if(in_state(AppState::Playing)),
            );
    }
}
fn without_atmosphere(world: &mut World) {
    let entities: Vec<_> = world
        .query_filtered::<Entity, With<bevy::light::Atmosphere>>()
        .iter(world)
        .collect();
    for entity in entities {
        world.despawn(entity);
    }
    let camera = world
        .query_filtered::<Entity, With<MainCamera>>()
        .single(world)
        .unwrap();
    world
        .entity_mut(camera)
        .remove::<bevy::pbr::AtmosphereSettings>();
}

struct Scene {
    name: &'static str,
    anchor: Vec3,
    camera: Transform,
}
#[derive(Resource)]
struct Capture {
    directory: PathBuf,
    scenes: Vec<Scene>,
    index: usize,
    elapsed: f64,
    samples: Vec<f64>,
    saved: bool,
    initialized: bool,
    probe: String,
    phases: Vec<usize>,
}
impl Capture {
    fn new(directory: PathBuf) -> Self {
        std::fs::create_dir_all(&directory).unwrap();
        let level =
            LevelData::from_artifact_bytes(include_bytes!("../assets/level_1337.bin")).unwrap();
        let center = |face: usize| {
            level.terrain_tris[face]
                .map(Vec3::from_array)
                .iter()
                .sum::<Vec3>()
                / 3.0
        };
        let mut counts = vec![0usize; level.terrain_tris.len()];
        for item in &level.scenery {
            counts[item.face as usize] += 1;
        }
        let boundary = level
            .face_corner_types
            .iter()
            .enumerate()
            .filter(|(i, t)| (t[0] != t[1] || t[1] != t[2]) && level.face_water_r[*i] == 0.0)
            .max_by_key(|(i, _)| (counts[*i], std::cmp::Reverse(*i)))
            .unwrap()
            .0;
        let shore = level
            .face_water_r
            .iter()
            .enumerate()
            .filter(|(i, r)| **r > 0.0 && level.water_phase[*i] == Some(WaterPhase::Liquid))
            .min_by(|(a, _), (b, _)| {
                center(*a)
                    .distance_squared(Vec3::from_array(level.settlements[0].pos) * 2020.0)
                    .total_cmp(
                        &center(*b)
                            .distance_squared(Vec3::from_array(level.settlements[0].pos) * 2020.0),
                    )
            })
            .unwrap()
            .0;
        let bridge = level
            .roads
            .iter()
            .find(|r| r.kind == RoadKind::Bridge)
            .unwrap();
        let bridge_start = Vec3::from_array(bridge.points[0]);
        let bridge_end = Vec3::from_array(*bridge.points.last().unwrap());
        let bridge_up = bridge_start.normalize();
        let bridge_heading = (bridge_end - bridge_start)
            .reject_from_normalized(bridge_up)
            .normalize();
        let home = Vec3::from_array(level.settlements[0].pos).normalize();
        let surface = |up: Vec3| {
            let face = level
                .unit_tris
                .iter()
                .enumerate()
                .max_by(|(_, a), (_, b)| {
                    let score = |t: &[[f32; 3]; 3]| {
                        t.iter()
                            .map(|v| Vec3::from_array(*v))
                            .sum::<Vec3>()
                            .normalize()
                            .dot(up)
                    };
                    score(a).total_cmp(&score(b))
                })
                .unwrap()
                .0;
            up * center(face).length()
        };
        let ground_scene = |name, anchor: Vec3, heading: Vec3| Scene {
            name,
            anchor,
            camera: Transform::from_translation(anchor + anchor.normalize() * 3.0 - heading * 8.0)
                .looking_at(anchor + heading * 20.0, anchor.normalize()),
        };
        let house = level
            .structures
            .iter()
            .filter(|s| s.kind == StructureKind::House)
            .min_by(|a, b| {
                Vec3::from_array(a.pos)
                    .distance_squared(home * 2020.0)
                    .total_cmp(&Vec3::from_array(b.pos).distance_squared(home * 2020.0))
            })
            .unwrap();
        let house_pos = Vec3::from_array(house.pos);
        let front = Quat::from_rotation_arc(Vec3::Y, house_pos.normalize())
            * Quat::from_rotation_y(house.yaw)
            * Vec3::NEG_Z;
        let shore_up = center(shore).normalize();
        let shore_anchor = shore_up * level.face_water_r[shore].max(center(shore).length());
        let shore_heading = (shore_anchor - surface(home))
            .reject_from_normalized(shore_up)
            .normalize();
        let mut scenes = vec![
            ground_scene(
                "dense-transition",
                center(boundary),
                SpherePos::new(center(boundary)).tangent_basis().1,
            ),
            ground_scene("bridge", surface(bridge_up), bridge_heading),
            ground_scene("shoreline", shore_anchor, shore_heading),
            ground_scene("settlement", house_pos + front * 15.0, -front),
            Scene {
                name: "overview",
                anchor: surface(home),
                camera: Transform::from_translation(home * 6500.0).looking_at(Vec3::ZERO, Vec3::Y),
            },
        ];
        let probe = std::env::var("TERRA_LIGHTING_PROBE").unwrap_or_else(|_| "baseline".into());
        assert!(
            [
                "baseline",
                "shadows",
                "no-fog",
                "no-atmosphere",
                "no-ambient",
                "rough-water",
                "flat-water",
                "exposure"
            ]
            .contains(&probe.as_str()),
            "unknown probe"
        );
        if let Ok(name) = std::env::var("TERRA_LIGHTING_SCENE") {
            scenes.retain(|s| s.name == name);
            assert!(!scenes.is_empty(), "unknown scene");
        }
        std::fs::write(directory.join("selection.txt"), format!("boundary_face={boundary} scenery_on_face={} shore_face={shore} bridge={} seed={} probe={probe}\n", counts[boundary], bridge.name, level.seed)).unwrap();
        let phases = match std::env::var("TERRA_LIGHTING_PHASE").as_deref() {
            Ok("noon") => vec![0],
            Ok("sunset") => vec![1],
            Ok("night") => vec![2],
            Err(_) => vec![0, 1, 2],
            _ => panic!("unknown phase"),
        };
        let mut min_dot = 1.0_f32;
        let mut inward = 0;
        for tri in &level.terrain_tris {
            let [a, b, c] = tri.map(Vec3::from_array);
            let dot = (b - a)
                .cross(c - a)
                .normalize()
                .dot((a + b + c).normalize());
            min_dot = min_dot.min(dot);
            if !dot.is_finite() || dot <= 0.0 {
                inward += 1;
            }
        }
        std::fs::write(
            directory.join("normals.txt"),
            format!(
                "terrain_faces={} inward_or_nonfinite={} min_normal_dot_radial={}\n",
                level.terrain_tris.len(),
                inward,
                min_dot
            ),
        )
        .unwrap();
        Self {
            directory,
            scenes,
            index: 0,
            elapsed: 0.0,
            samples: vec![],
            saved: false,
            initialized: false,
            probe,
            phases,
        }
    }
    fn scene(&self) -> &Scene {
        &self.scenes[self.index / self.phases.len()]
    }
    fn phase(&self) -> &'static str {
        ["noon", "sunset", "night"][self.phases[self.index % self.phases.len()]]
    }
}

fn prepare(world: &mut World) {
    world.resource_scope(|world, mut capture: Mut<Capture>| {
        if !capture.initialized {
            let mut time = Time::<Virtual>::default();
            time.pause();
            world.insert_resource(time);
            *world.resource_mut::<crate::weather::Weather>() = default();
            // Physics remains authored through Avian; this is a stationary rendering fixture.
            let entity = world
                .query_filtered::<Entity, With<Player>>()
                .single(world)
                .unwrap();
            world
                .entity_mut(entity)
                .insert(avian3d::prelude::RigidBody::Static);
            capture.initialized = true;
        }
        let scene = capture.scene();
        let up = scene.anchor.normalize();
        let phase = capture.phases[capture.index % capture.phases.len()];
        let longitude = up.z.atan2(up.x);
        // Solve dot(up, sun_dir)=0 for local sunset, retaining the game's 0.35 tilt.
        let sunset = (-0.35 * up.y / Vec2::new(up.x, up.z).length())
            .clamp(-1.0, 1.0)
            .acos();
        world.resource_mut::<SunLock>().0 = false;
        world.resource_mut::<TimeOfDay>().angle =
            longitude + [0.0, sunset, std::f32::consts::PI][phase];
        let entity = world
            .query_filtered::<Entity, With<Player>>()
            .single(world)
            .unwrap();
        // No fixed steps run while paused, so mirror the Avian teleport for render consumers.
        let position = scene.anchor + up * 0.7;
        world.entity_mut(entity).insert((
            avian3d::prelude::Position(position),
            avian3d::prelude::LinearVelocity::ZERO,
            Transform::from_translation(position)
                .with_rotation(Quat::from_rotation_arc(Vec3::Y, up)),
        ));
        *world
            .query_filtered::<&mut Transform, With<MainCamera>>()
            .single_mut(world)
            .unwrap() = scene.camera;
    });
}

fn record(world: &mut World) {
    world.resource_scope(|world, mut capture: Mut<Capture>| {
        let scene = capture.scene();
        let camera = scene.camera;
        let anchor = scene.anchor;
        let name = scene.name;
        let camera_entity = world.query_filtered::<Entity, With<MainCamera>>().single(world).unwrap();
        *world.get_mut::<Transform>(camera_entity).unwrap() = camera;
        match capture.probe.as_str() {
            "shadows" => {
                for mut light in world.query_filtered::<&mut DirectionalLight, With<Sun>>().iter_mut(world) { light.shadow_maps_enabled = true; }
            }
            "no-fog" => { world.get_mut::<DistanceFog>(camera_entity).unwrap().falloff = FogFalloff::Linear { start: 1.0e8, end: 2.0e8 }; }
            "no-ambient" => { world.resource_mut::<GlobalAmbientLight>().brightness = 0.0; }
            "exposure" => { world.entity_mut(camera_entity).insert(Exposure { ev100: 12.0 }); }
            "rough-water" | "flat-water" if capture.elapsed == 0.0 => {
                for (_, material) in world.resource_mut::<Assets<crate::water::WaterMaterial>>().iter_mut() {
                    if capture.probe == "rough-water" { material.base.perceptual_roughness = 0.5; }
                    else { material.extension.params.wave_amp = 0.0; material.extension.params.swell_amp = 0.0; }
                }
            }
            _ => {}
        }
        let delta = world.resource::<Time<Real>>().delta_secs_f64();
        capture.elapsed += delta;
        if capture.elapsed > 8.0 && !capture.saved { capture.samples.push(delta*1000.0); }
        if capture.elapsed > 13.0 && !capture.saved {
            let window = world.query_filtered::<&Window, With<PrimaryWindow>>().single(world).unwrap();
            let resolution = format!("{}x{} logical={}x{} scale={} present={:?}", window.physical_width(), window.physical_height(), window.width(), window.height(), window.scale_factor(), window.present_mode);
            let (light, tf) = world.query_filtered::<(&DirectionalLight,&Transform), With<Sun>>().single(world).unwrap();
            let sun = world.resource::<TimeOfDay>().sun_dir;
            let shadow = light.shadow_maps_enabled;
            let light_info = format!("sun={sun:?} forward={:?} forward_dot_sun={:.6} local_sun_elevation_deg={:.3} lux={} shadows={shadow}", tf.forward(), tf.forward().dot(sun), anchor.normalize().dot(sun).asin().to_degrees(), light.illuminance);
            let ambient = world.resource::<GlobalAmbientLight>().brightness;
            let ev = world.get::<Exposure>(camera_entity).copied().unwrap_or_default().ev100;
            let visible = world.query_filtered::<&ViewVisibility, With<Mesh3d>>().iter(world).filter(|v| v.get()).count();
            let points = world.query::<&PointLight>().iter(world).count();
            let spots = world.query::<&SpotLight>().iter(world).count();
            let mut frames = capture.samples.clone(); frames.sort_by(f64::total_cmp);
            let p = |q: f64| frames[((frames.len()-1) as f64*q).round() as usize];
            let row = format!("{name}/{} probe={} {resolution} camera={camera:?} anchor={anchor:?} {light_info} ambient={ambient} ev100={ev} visible={visible} point_lights={points} spot_lights={spots} n={} p50_ms={:.3} p95_ms={:.3} p99_ms={:.3} max_ms={:.3} over33ms={}\n", capture.phase(), capture.probe, frames.len(), p(0.5), p(0.95), p(0.99), p(1.0), frames.iter().filter(|v| **v > 1000.0/30.0).count());
            let mut file = std::fs::OpenOptions::new().create(true).append(true).open(capture.directory.join("measurements.txt")).unwrap();
            file.write_all(row.as_bytes()).unwrap();
            let stem = format!("{name}-{}", capture.phase());
            std::fs::write(capture.directory.join(format!("{stem}-frames.csv")), capture.samples.iter().map(|v| format!("{v:.6}\n")).collect::<String>()).unwrap();
            world.spawn(Screenshot::primary_window()).observe(save_to_disk(capture.directory.join(format!("{stem}.png"))));
            info!("LIGHTING_BASELINE {}", row.trim());
            capture.saved = true;
        }
        if capture.elapsed > 14.0 {
            capture.index += 1;
            if capture.index == capture.scenes.len()*capture.phases.len() { world.write_message(AppExit::Success); }
            else { capture.elapsed = 0.0; capture.samples.clear(); capture.saved = false; }
        }
    });
}
