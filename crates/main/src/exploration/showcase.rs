//! One connected expedition. Only initial placement authors poses; gameplay owns all travel.
use super::*;
use crate::{
    asset_catalog::AssetCatalog,
    map::{CollisionTerrain, SunLock, TimeOfDay},
    weather::Weather,
};
use bevy::{
    ecs::system::RunSystemOnce,
    input::InputSystems,
    render::view::screenshot::{Screenshot, save_to_disk},
    time::TimeUpdateStrategy,
    transform::TransformSystems,
    window::{PrimaryWindow, WindowResolution},
};
use shared::{
    flight_showcase::{pilot_at_speed, pilot_input},
    level::LevelData,
};
use terra_geometry::sphere::PLANET_RADIUS;
use std::{path::PathBuf, time::Duration};

#[derive(Resource, Default)]
pub(super) struct PilotControls {
    pub entity: Option<Entity>,
    pub flight: FlightInput,
    pub throttle: f32,
    pub steering: f32,
}
pub struct ShowcasePlugin;
impl Plugin for ShowcasePlugin {
    fn build(&self, app: &mut App) {
        let Some(directory) = std::env::var_os("TERRA_FLIGHT_CAPTURE") else {
            return;
        };
        app.insert_resource(bevy::winit::WinitSettings {
            focused_mode: bevy::winit::UpdateMode::Continuous,
            unfocused_mode: bevy::winit::UpdateMode::Continuous,
        })
        .insert_resource(Movie::new(directory.into()))
        .init_resource::<PilotControls>()
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
            1.0 / 30.0,
        )))
        .insert_resource(Time::<Fixed>::from_hz(60.0))
        .add_systems(
            PreUpdate,
            direct
                .after(InputSystems)
                .before(super::input)
                .run_if(in_state(AppState::Playing)),
        )
        .add_systems(
            FixedUpdate,
            control
                .before(super::drive)
                .run_if(in_state(AppState::Playing)),
        )
        .add_systems(
            PostUpdate,
            record
                .before(TransformSystems::Propagate)
                .run_if(in_state(AppState::Playing)),
        );
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum Beat {
    Opening,
    Walk,
    ApproachCar,
    Drive,
    ExitCar,
    ApproachPlane,
    Takeoff,
    Fly,
    Roll,
    Loop,
    ApproachRunway,
    Land,
    ExitPlane,
    ReturnToCar,
    DriveHome,
    ExitHome,
    WalkHome,
    Closing,
    Done,
}
#[derive(Resource)]
struct Movie {
    directory: PathBuf,
    preview: bool,
    target: Option<Handle<Image>>,
    tick: usize,
    frame: usize,
    beat: Beat,
    since: usize,
    explorer: Option<Entity>,
    car: Option<Entity>,
    plane: Option<Entity>,
    walk: Vec<Vec3>,
    drive: Vec<Vec3>,
    flight: Vec<Vec3>,
    waypoint: usize,
    leg: usize,
    home: Vec3,
    home_heading: Vec3,
    home_road: Vec<Vec3>,
    runway: Vec3,
    runway_heading: Vec3,
    camera: Option<Transform>,
    camera_heading: Vec3,
    opening_camera: Option<Transform>,
    opening_sun: f32,
    closing_sun: f32,
    roll_done: bool,
    loop_done: bool,
    turns: Vec2,
    previous_angles: Vec2,
    returning: bool,
}
impl Movie {
    fn new(directory: PathBuf) -> Self {
        std::fs::create_dir_all(&directory).unwrap();
        let level =
            LevelData::from_artifact_bytes(include_bytes!("../../assets/level_1337.bin")).unwrap();
        let road = |i: usize| {
            level.roads[i]
                .points
                .iter()
                .map(|p| Vec3::from_array(*p))
                .collect::<Vec<_>>()
        };
        // This Oakford house faces the rural road, giving the expedition a
        // visible doorstep without navigating the settlement's internal streets.
        let outward = road(20);
        let house = level.structures[6];
        assert!(matches!(house.kind, shared::level::StructureKind::House));
        let house_position = Vec3::from_array(house.pos);
        let front = Quat::from_rotation_arc(Vec3::Y, house_position.normalize())
            * Quat::from_rotation_y(house.yaw)
            * Vec3::NEG_Z;
        let home = (house_position + front * 2.2).normalize();
        let doorstep_path = (house_position + front * 8.0).normalize();
        let mut walk = vec![home, doorstep_path];
        walk.extend_from_slice(&outward[..=4]);
        let drive = outward[4..=38].to_vec();
        let runway = Vec3::new(-0.5646092, 0.614_004, -0.5515572).normalize();
        let runway_heading = tangent(Vec3::new(-0.48532894, 0.29353702, 0.82358474), runway);
        // The final route is a circuit; these are destinations, not animated poses.
        let flight = vec![
            Vec3::new(-0.291546, 0.569875, -0.768273), // Amber Plains: roll
            Vec3::new(-0.131894, 0.357123, -0.924699), // snowy Iron Range
            Vec3::new(-0.003770, 0.718326, -0.695697), // Mirror Lake
            Vec3::new(0.144652, 0.548893, -0.823281),  // Winding River
            Vec3::new(0.363133, 0.657794, -0.659880),  // Rolling Plains: loop
            Vec3::new(-0.313965, -0.045947, -0.948322), // Elder Woods
        ];
        Self {
            directory,
            preview: std::env::var_os("TERRA_FLIGHT_PREVIEW").is_some(),
            target: None,
            tick: 0,
            frame: 0,
            beat: Beat::Opening,
            since: 180,
            explorer: None,
            car: None,
            plane: None,
            walk,
            drive,
            flight,
            waypoint: 1,
            leg: 0,
            home,
            home_heading: tangent(-front, home),
            home_road: outward[..4].iter().rev().copied().collect(),
            runway,
            runway_heading,
            camera: None,
            camera_heading: Vec3::NEG_Z,
            opening_camera: None,
            opening_sun: 0.0,
            closing_sun: 0.0,
            roll_done: false,
            loop_done: false,
            turns: Vec2::ZERO,
            previous_angles: Vec2::ZERO,
            returning: false,
        }
    }
    fn prepare_homecoming(&mut self) {
        self.drive.reverse();
        self.drive.extend(self.home_road.iter().copied());
        let approach = (self.home * 2000.0 - self.home_heading * 6.0).normalize();
        self.walk = vec![*self.drive.last().unwrap(), approach, self.home];
    }

    fn elapsed(&self) -> f32 {
        self.tick.saturating_sub(self.since) as f32 / 30.0
    }
    fn change(&mut self, beat: Beat) {
        info!("MOVIE frame {} {:?} -> {:?}", self.frame, self.beat, beat);
        use std::io::Write;
        writeln!(
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.directory.join("beats.tsv"))
                .unwrap(),
            "{}\t{:?}",
            self.frame,
            beat
        )
        .unwrap();
        self.beat = beat;
        self.since = self.tick;
        self.waypoint = 1;
    }
}
fn radius(world: &World, p: Vec3) -> f32 {
    world
        .resource::<CollisionTerrain>()
        .0
        .facet_radius(p.normalize(), PLANET_RADIUS)
        .max(PLANET_RADIUS)
}
fn placed(world: &World, p: Vec3, height: f32) -> Vec3 {
    p.normalize() * (radius(world, p) + height)
}
fn spawn_vehicle(world: &mut World, kind: Kind, p: Vec3, h: Vec3) -> Entity {
    let rotation = facing(h, p.normalize());
    let e = world
        .spawn((
            Vehicle::new(kind, h),
            RigidBody::Static,
            kind.collider(),
            Mass(if kind == Kind::Car { 800.0 } else { 900.0 }),
            Position(p),
            Rotation(rotation),
            Transform::from_translation(p).with_rotation(rotation),
            Visibility::default(),
            SweptCcd::default(),
            CollidingEntities::default(),
            physics_reset(),
        ))
        .id();
    let scene = world.resource::<AssetCatalog>().scene(kind.asset());
    let child = world
        .spawn((
            WorldAssetRoot(scene),
            Transform::from_translation(Vec3::NEG_Y * kind.height()),
            view::VehicleVisual,
        ))
        .id();
    world.entity_mut(e).add_child(child);
    e
}
fn initialize(world: &mut World, m: &mut Movie) {
    let e = world
        .query_filtered::<Entity, With<Player>>()
        .single(world)
        .unwrap();
    let origin = placed(world, m.home, 100.0);
    let h = m.home_heading;
    let p = world
        .run_system_once(move |placement: Placement| {
            placement
                .locate(origin, h, None, &[e], 12.0)
                .map(|(p, _)| p)
        })
        .unwrap()
        .expect("validated starting ground");
    m.home = p.normalize();
    m.home_heading = h;
    m.walk[0] = m.home;
    world.entity_mut(e).insert((
        Position(p),
        LinearVelocity::ZERO,
        Transform::from_translation(p)
            .with_rotation(Quat::from_rotation_arc(Vec3::Y, p.normalize())),
    ));
    world.get_mut::<Player>(e).unwrap().heading = h;
    let car_dir = *m.walk.last().unwrap();
    let car_h = tangent(m.drive[1] - car_dir, car_dir);
    let car_origin = placed(world, car_dir * 2000.0 + car_h * 5.0, 100.0);
    let car_p = world
        .run_system_once(move |placement: Placement| {
            placement
                .locate(car_origin, car_h, Some(Kind::Car), &[e], 12.0)
                .map(|(p, _)| p)
        })
        .unwrap()
        .expect("validated parked car ground");
    let car = spawn_vehicle(world, Kind::Car, car_p, car_h);
    let plane = spawn_vehicle(
        world,
        Kind::Plane,
        placed(world, m.runway, 0.58),
        m.runway_heading,
    );
    let mut exploration = world.resource_mut::<Exploration>();
    exploration.vehicles = [Some(car), Some(plane)];
    exploration.snap_camera = true;
    m.explorer = Some(e);
    m.car = Some(car);
    m.plane = Some(plane);
    m.camera_heading = h;
    world
        .query_filtered::<&mut Window, With<PrimaryWindow>>()
        .single_mut(world)
        .unwrap()
        .resolution = WindowResolution::new(1280, 720).with_scale_factor_override(1.0);
    if !m.preview {
        let image = Image::new_target_texture(
            1280,
            720,
            bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
            None,
        );
        let target = world.resource_mut::<Assets<Image>>().add(image);
        let camera = world
            .query_filtered::<Entity, With<MainCamera>>()
            .single(world)
            .unwrap();
        world.entity_mut(camera).insert((
            bevy::camera::RenderTarget::from(target.clone()),
            IsDefaultUiCamera,
        ));
        world
            .query_filtered::<&mut Window, With<PrimaryWindow>>()
            .single_mut(world)
            .unwrap()
            .visible = false;
        m.target = Some(target);
    }
    world.resource_mut::<Time<Virtual>>().pause();
    crate::minimap::showcase_map(world, true, m.home);
}
fn interact(world: &mut World, expected: Entity) {
    let state = world.resource::<Exploration>();
    if state.occupied.is_none() && state.target == Some(expected) && state.actions.is_empty() {
        world
            .resource_mut::<Exploration>()
            .request(Action::Interact);
    }
}
fn exit_vehicle(world: &mut World, e: Entity) {
    let state = world.resource::<Exploration>();
    if state.occupied == Some(e)
        && state.actions.is_empty()
        && world.get::<Vehicle>(e).unwrap().stable >= 0.3
    {
        world
            .resource_mut::<Exploration>()
            .request(Action::Interact);
    }
}
fn near(world: &World, e: Entity, target: Vec3, meters: f32) -> bool {
    world
        .get::<Position>(e)
        .unwrap()
        .0
        .normalize()
        .distance(target.normalize())
        * 2000.0
        < meters
}

fn direct(world: &mut World) {
    let mut m = world.remove_resource::<Movie>().unwrap();
    if m.explorer.is_none() {
        initialize(world, &mut m);
    }
    let Some(explorer) = m.explorer else {
        world.insert_resource(m);
        return;
    };
    let car = m.car.unwrap();
    let plane = m.plane.unwrap();
    if m.tick == 180 {
        world.resource_mut::<Time<Virtual>>().unpause();
    }
    if m.tick >= 180 {
        let occupied = world.resource::<Exploration>().occupied;
        match m.beat {
            Beat::Opening if m.elapsed() > 4.0 => {
                crate::minimap::showcase_map(world, false, m.home);
                m.change(Beat::Walk);
            }
            Beat::Walk
                if m.waypoint == m.walk.len() - 1
                    && near(world, explorer, *m.walk.last().unwrap(), 3.0) =>
            {
                m.change(Beat::ApproachCar)
            }
            Beat::ApproachCar => {
                interact(world, car);
                if occupied == Some(car) {
                    m.change(Beat::Drive);
                }
            }
            Beat::Drive
                if m.waypoint == m.drive.len() - 1
                    && near(world, car, *m.drive.last().unwrap(), 3.5) =>
            {
                m.change(Beat::ExitCar)
            }
            Beat::ExitCar => {
                exit_vehicle(world, car);
                if occupied.is_none() {
                    m.change(Beat::ApproachPlane);
                }
            }
            Beat::ApproachPlane => {
                interact(world, plane);
                if occupied == Some(plane) {
                    m.change(Beat::Takeoff);
                }
            }
            Beat::Takeoff if world.get::<Vehicle>(plane).unwrap().air_time > 10.0 => {
                m.change(Beat::Fly)
            }
            Beat::Fly => {
                if near(world, plane, m.flight[m.leg], 230.0) {
                    info!(
                        "MOVIE reached flight destination {} at frame {}",
                        m.leg, m.frame
                    );
                    m.leg += 1;
                    if m.leg == m.flight.len() {
                        m.change(Beat::ApproachRunway);
                    }
                }
                if m.beat == Beat::Fly && m.leg == 1 && !m.roll_done {
                    m.roll_done = true;
                    m.turns = Vec2::ZERO;
                    m.previous_angles = Vec2::ZERO;
                    m.change(Beat::Roll);
                }
                if m.beat == Beat::Fly && m.leg == 5 && !m.loop_done {
                    m.loop_done = true;
                    m.turns = Vec2::ZERO;
                    m.previous_angles = Vec2::ZERO;
                    m.change(Beat::Loop);
                }
            }
            Beat::Roll if m.elapsed() > 10.0 => {
                assert!(m.turns.y > 6.0, "roll incomplete");
                m.change(Beat::Fly);
            }
            Beat::Loop if m.elapsed() > 22.0 => {
                assert!(m.turns.x > 6.0, "loop incomplete {:?}", m.turns);
                m.change(Beat::Fly);
            }
            Beat::ApproachRunway => {
                let approach = approach_target(&m);
                if m.waypoint < 3 && near(world, plane, approach, 120.0) {
                    m.waypoint += 1;
                }
                let p = world.get::<Position>(plane).unwrap().0;
                let flight = &world.get::<Vehicle>(plane).unwrap().flight;
                let distance = (m.runway * radius(world, m.runway) - p).dot(m.runway_heading);
                if m.waypoint == 3
                    && distance < 750.0
                    && flight.heading.dot(tangent(m.runway_heading, p.normalize())) > 0.985
                    && flight.bank.abs() < 0.2
                {
                    m.change(Beat::Land);
                }
            }
            Beat::Land if !world.get::<Vehicle>(plane).unwrap().flight.airborne => {
                m.change(Beat::ExitPlane)
            }
            Beat::ExitPlane => {
                exit_vehicle(world, plane);
                if occupied.is_none() {
                    m.change(Beat::ReturnToCar);
                }
            }
            Beat::ReturnToCar => {
                interact(world, car);
                if occupied == Some(car) {
                    m.prepare_homecoming();
                    m.returning = true;
                    m.change(Beat::DriveHome);
                }
            }
            Beat::DriveHome
                if m.waypoint == m.drive.len() - 1
                    && near(world, car, *m.drive.last().unwrap(), 3.5) =>
            {
                m.change(Beat::ExitHome)
            }
            Beat::ExitHome => {
                exit_vehicle(world, car);
                if occupied.is_none() {
                    m.change(Beat::WalkHome);
                }
            }
            Beat::WalkHome
                if m.waypoint == m.walk.len() - 1 && near(world, explorer, m.home, 0.8) =>
            {
                let heading = world.get::<Player>(explorer).unwrap().heading;
                assert!(
                    heading.dot(m.home_heading) > 0.99,
                    "homecoming must arrive facing the opening direction"
                );
                m.closing_sun = world.resource::<TimeOfDay>().angle;
                crate::minimap::showcase_map(world, true, m.home);
                m.change(Beat::Closing);
            }
            Beat::Closing if m.elapsed() > 14.0 => m.change(Beat::Done),
            _ => {}
        }
        assert!(m.elapsed() < 180.0, "movie stalled at {:?}", m.beat);
    }
    // Weather changes over the journey rather than restarting at every shot.
    let precipitation = match m.beat {
        Beat::Drive | Beat::ExitCar | Beat::ApproachPlane => 0.65,
        // Keep the front through the mountain crossing and the following valley.
        // Production temperature selects snow on cold terrain and rain below.
        Beat::Fly | Beat::Roll if (1..=2).contains(&m.leg) => 0.8,
        _ => 0.0,
    };
    world
        .resource_mut::<Weather>()
        .showcase_front(precipitation, Vec3::new(2.0, 0.0, 1.0));
    let p = world.get::<Position>(explorer).unwrap().0.normalize();
    // A morning departure develops into an evening arrival. The stationary
    // closing map then advances through night to the next morning.
    let home_distance = m.car.map_or(f32::INFINITY, |car| {
        world
            .get::<Position>(car)
            .unwrap()
            .0
            .normalize()
            .distance(*m.drive.last().unwrap())
            * 2000.0
    });
    let phase = match m.beat {
        Beat::Opening | Beat::Walk | Beat::ApproachCar => -1.0,
        Beat::DriveHome if home_distance < 100.0 => 1.4,
        Beat::ExitHome | Beat::WalkHome | Beat::Closing => 1.4,
        Beat::ApproachRunway
        | Beat::Land
        | Beat::ExitPlane
        | Beat::ReturnToCar
        | Beat::DriveHome => 0.5,
        _ => 0.0,
    };
    let target = p.z.atan2(p.x) + phase;
    let mut tod = world.resource_mut::<TimeOfDay>();
    let delta = (target - tod.angle + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
        - std::f32::consts::PI;
    if m.beat == Beat::Closing {
        tod.angle = timelapse_angle(m.closing_sun, m.opening_sun, m.elapsed());
    } else {
        tod.angle += delta * 0.035;
    }
    tod.day_length = 1.0e9;
    tod.day = 1;
    world.resource_mut::<SunLock>().0 = false;
    world.insert_resource(m);
}

fn driving_speed(turn: f32, final_distance: f32) -> f32 {
    // Small waypoint corrections do not warrant braking; reserve it for
    // actual corners and a roughly 20 m parking approach at cruise speed.
    (12.0 / (1.0 + (turn.abs() - 0.2).max(0.0) * 2.0))
        .min((final_distance * 7.0).sqrt())
        .max(2.0)
}

fn shot_heading(beat: Beat, tracked: Vec3, home: Vec3, up: Vec3) -> Vec3 {
    if matches!(
        beat,
        Beat::DriveHome | Beat::ExitHome | Beat::WalkHome | Beat::Closing
    ) {
        tangent(home, up)
    } else {
        tracked
    }
}

fn timelapse_angle(start: f32, opening: f32, seconds: f32) -> f32 {
    let progress = ((seconds - 2.0) / 10.0).clamp(0.0, 1.0);
    start + (opening - start).rem_euclid(std::f32::consts::TAU) * progress
}

fn approach_target(m: &Movie) -> Vec3 {
    let side = m.runway_heading.cross(m.runway);
    let offset = match m.waypoint {
        1 => -m.runway_heading * 1400.0 + side * 500.0,
        2 => -m.runway_heading * 1100.0,
        _ => Vec3::ZERO,
    };
    (m.runway * 2000.0 + offset).normalize()
}

fn angle(heading: Vec3, target: Vec3, up: Vec3) -> f32 {
    up.dot(heading.cross(target)).atan2(heading.dot(target))
}
fn control(world: &mut World) {
    let mut m = world.remove_resource::<Movie>().unwrap();
    let mut controls = PilotControls::default();
    let mut keys = Vec::new();
    let Some(explorer) = m.explorer else {
        world.insert_resource(m);
        return;
    };
    let occupied = world.resource::<Exploration>().occupied;
    match m.beat {
        Beat::Drive | Beat::DriveHome => {
            assert_eq!(occupied, m.car, "car travel must remain occupied")
        }
        Beat::Takeoff | Beat::Fly | Beat::Roll | Beat::Loop | Beat::ApproachRunway | Beat::Land => {
            assert_eq!(occupied, m.plane, "flight must remain occupied")
        }
        _ => {}
    }
    let entity = occupied.unwrap_or(explorer);
    let p = world.get::<Position>(entity).unwrap().0;
    let up = p.normalize();
    let velocity = world.get::<LinearVelocity>(entity).unwrap().0;
    if m.tick < 180
        || matches!(m.beat, Beat::Opening | Beat::Closing | Beat::Done)
        || (m.beat == Beat::Walk && m.elapsed() < 2.0)
    {
        world.resource_mut::<ButtonInput<KeyCode>>().reset_all();
        world.insert_resource(controls);
        world.insert_resource(m);
        return;
    }
    if let Some(e) = occupied {
        let v = world.get::<Vehicle>(e).unwrap();
        assert!(
            !v.crashed,
            "movie crash at {:?}, {:.2}s",
            m.beat,
            m.elapsed()
        );
        controls.entity = Some(e);
        if v.kind == Kind::Car {
            let stopping = matches!(m.beat, Beat::ExitCar | Beat::ExitHome);
            while m.waypoint + 1 < m.drive.len() && near(world, e, m.drive[m.waypoint], 5.0) {
                m.waypoint += 1;
            }
            let dir = tangent(m.drive[m.waypoint] - up, up);
            let turn = angle(v.flight.heading, dir, up);
            let final_distance = p.normalize().distance(*m.drive.last().unwrap()) * 2000.0;
            let speed = if stopping {
                0.0
            } else {
                driving_speed(turn, final_distance)
            };
            let parking_turn = stopping && m.elapsed() < 0.65;
            controls.steering = if parking_turn {
                turn.signum() * 0.22
            } else if stopping {
                0.0
            } else {
                (turn * 1.5).clamp(-1.0, 1.0)
            };
            let forward_speed = velocity.dot(v.flight.heading);
            controls.throttle = if parking_turn {
                0.0
            } else if stopping {
                if forward_speed.abs() > 0.2 {
                    -forward_speed.signum()
                } else {
                    0.0
                }
            } else if velocity.length() < speed - 0.2 {
                1.0
            } else if velocity.length() > speed + 0.4 {
                -1.0
            } else {
                0.0
            };
        } else {
            let vflight = v.flight;
            let target = match m.beat {
                Beat::Takeoff => (m.runway * 2000.0 + m.runway_heading * 1200.0).normalize(),
                Beat::ApproachRunway => approach_target(&m),
                Beat::Land | Beat::ExitPlane => {
                    (m.runway * 2000.0 + m.runway_heading * 180.0).normalize()
                }
                _ => m.flight[m.leg.min(m.flight.len() - 1)],
            };
            let direction = tangent(target - up, up);
            let highest = (0..=20)
                .map(|i| radius(world, p + direction * i as f32 * 30.0))
                .fold(radius(world, p), f32::max);
            let altitude = if matches!(m.beat, Beat::Roll | Beat::Loop) {
                220.0
            } else {
                80.0
            };
            controls.flight = pilot_input(&vflight, p, velocity, target * (highest + altitude));
            match m.beat {
                Beat::Takeoff if m.elapsed() < 1.0 => controls.flight = FlightInput::default(),
                Beat::Takeoff if !vflight.airborne => {
                    controls.flight = FlightInput {
                        pitch: 1.0,
                        throttle: 1.0,
                        ..default()
                    }
                }
                Beat::ExitPlane => {
                    controls.flight = FlightInput {
                        brake: true,
                        ..default()
                    }
                }
                Beat::Land => {
                    let distance = (m.runway * radius(world, m.runway) - p).dot(m.runway_heading);
                    let margin = (distance * 0.075 - 13.5).clamp(-10.0, 80.0);
                    controls.flight = pilot_at_speed(
                        &vflight,
                        p,
                        velocity,
                        target * (radius(world, m.runway) + margin),
                        35.0,
                    );
                }
                Beat::ApproachRunway => {
                    controls.flight =
                        pilot_at_speed(&vflight, p, velocity, target * (highest + 80.0), 50.0)
                }
                Beat::Roll if (2.0..8.3).contains(&m.elapsed()) => {
                    controls.flight = FlightInput {
                        bank: 1.0,
                        throttle: 1.0,
                        ..default()
                    }
                }
                Beat::Loop if (2.0..5.0).contains(&m.elapsed()) => {
                    controls.flight = FlightInput {
                        pitch: -0.12,
                        throttle: 1.0,
                        ..default()
                    }
                }
                Beat::Loop if (5.0..16.65).contains(&m.elapsed()) => {
                    controls.flight = FlightInput {
                        pitch: 1.0,
                        throttle: 1.0,
                        ..default()
                    }
                }
                _ => {}
            }
        }
    } else {
        let target = match m.beat {
            Beat::ApproachCar | Beat::ReturnToCar => {
                world.get::<Position>(m.car.unwrap()).unwrap().0.normalize()
            }
            Beat::ApproachPlane => world
                .get::<Position>(m.plane.unwrap())
                .unwrap()
                .0
                .normalize(),
            _ => {
                while m.waypoint + 1 < m.walk.len()
                    && near(world, explorer, m.walk[m.waypoint], 1.5)
                {
                    m.waypoint += 1;
                }
                m.walk[m.waypoint]
            }
        };
        let heading = world.get::<Player>(explorer).unwrap().heading;
        let dir = tangent(target - up, up);
        let turn = angle(heading, dir, up);
        if turn.abs() > 0.065 {
            keys.push(if turn > 0.0 {
                KeyCode::KeyA
            } else {
                KeyCode::KeyD
            });
        }
        if turn.abs() < 0.12 {
            keys.push(KeyCode::KeyW);
        }
    }
    let mut input = world.resource_mut::<ButtonInput<KeyCode>>();
    input.reset_all();
    for key in keys {
        input.press(key);
    }
    world.insert_resource(controls);
    world.insert_resource(m);
}

fn record(world: &mut World) {
    let mut m = world.remove_resource::<Movie>().unwrap();
    let Some(explorer) = m.explorer else {
        world.insert_resource(m);
        return;
    };
    let entity = world.resource::<Exploration>().occupied.unwrap_or(explorer);
    let p = world.get::<Position>(entity).unwrap().0;
    let up = p.normalize();
    let vehicle = world.get::<Vehicle>(entity);
    let heading = vehicle.map_or_else(
        || world.get::<Player>(explorer).unwrap().heading,
        |v| v.flight.heading,
    );
    let kind = vehicle.map(|v| v.kind);
    if let Some(v) = vehicle {
        assert!(!v.crashed, "movie crash at {:?}", m.beat);
        let angles = Vec2::new(v.flight.pitch, v.flight.bank);
        let delta = (angles - m.previous_angles).map(|a| {
            (a + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
        });
        // Count the commanded positive rotation separately from recovery corrections.
        m.turns += delta.max(Vec2::ZERO);
        m.previous_angles = angles;
    }
    m.camera_heading = tangent(
        m.camera_heading.lerp(
            heading,
            if kind == Some(Kind::Plane) {
                0.07
            } else {
                0.045
            },
        ),
        up,
    );
    let returning_ground = matches!(
        m.beat,
        Beat::DriveHome | Beat::ExitHome | Beat::WalkHome | Beat::Closing
    );
    let (back, height, side, ahead) = match kind {
        _ if returning_ground => (5.0, 2.3, 1.2, 3.0),
        None => (5.0, 2.3, 1.2, 3.0),
        Some(Kind::Car) => (10.0, 3.5, 2.0, 6.0),
        Some(Kind::Plane) => (24.0, 18.0, 7.0, 8.0),
    };
    let h = shot_heading(m.beat, m.camera_heading, m.home_heading, up);
    let mut desired = p - h * back + up * height + h.cross(up) * side;
    let floor = radius(world, desired) + 1.0;
    if desired.length() < floor {
        desired = desired.normalize() * floor;
    }
    let desired = Transform::from_translation(desired).looking_at(p + h * ahead, up);
    let mut previous = m.camera.unwrap_or(desired);
    if kind == Some(Kind::Plane) {
        previous.translation += world.get::<LinearVelocity>(entity).unwrap().0 / 30.0;
    }
    let blend = 0.10;
    let camera = Transform::from_translation(previous.translation.lerp(desired.translation, blend))
        .with_rotation(previous.rotation.slerp(desired.rotation, blend));
    if m.tick >= 180 && m.opening_camera.is_none() {
        m.opening_camera = Some(camera);
        m.opening_sun = world.resource::<TimeOfDay>().angle;
    }
    let camera = if m.beat == Beat::WalkHome {
        let opening = m.opening_camera.unwrap();
        let t = (m.elapsed() / 2.0).clamp(0.0, 1.0);
        Transform::from_translation(camera.translation)
            .with_rotation(camera.rotation.slerp(opening.rotation, t))
    } else if m.beat == Beat::Closing {
        let opening = m.opening_camera.unwrap();
        let t = (m.elapsed() / 2.0).clamp(0.0, 1.0);
        Transform::from_translation(camera.translation.lerp(opening.translation, t))
            .with_rotation(opening.rotation)
    } else {
        camera
    };
    m.camera = Some(camera);
    *world
        .query_filtered::<&mut Transform, With<MainCamera>>()
        .single_mut(world)
        .unwrap() = camera;
    if m.tick >= 180 && m.beat != Beat::Done {
        if !m.preview || m.frame % 90 == 45 {
            world
                .spawn(
                    m.target
                        .clone()
                        .map_or_else(Screenshot::primary_window, Screenshot::image),
                )
                .observe(save_to_disk(
                    m.directory.join(format!("00-{:05}.png", m.frame)),
                ));
        }
        m.frame += 1;
    }
    if m.beat == Beat::Done {
        assert!(
            near(world, explorer, m.home, 1.0),
            "the explorer must physically return home"
        );
        std::fs::write(
            m.directory.join("scenes.tsv"),
            format!("0\t{}\tTerra expedition", m.frame),
        )
        .unwrap();
        if m.elapsed() > 1.0 {
            world.write_message(AppExit::Success);
        }
    }
    m.tick += 1;
    world.insert_resource(m);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expedition_starts_and_returns_at_the_house_entrance() {
        let directory = std::env::temp_dir().join("terra-doorstep-route-test");
        let mut movie = Movie::new(directory.clone());
        let house = Vec3::new(-709.2458, 888.3811, -1690.7773);
        assert!(
            movie.home.distance(house.normalize()) * 2000.0 < 3.0,
            "the opening must place the explorer at the chosen house, not on the road"
        );
        let toward_door = tangent(house.normalize() - movie.home, movie.home);
        assert!(movie.home_heading.dot(toward_door) > 0.995);
        let start = movie.home;
        movie.prepare_homecoming();
        assert_eq!(*movie.walk.last().unwrap(), start);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn driver_keeps_cruising_through_gentle_bends_and_brakes_near_the_stop() {
        // Captured waypoint correction: six degrees should not trigger a 12→9.85 m/s brake.
        assert!(driving_speed(-0.109, 100.0) >= 11.5);
        assert!(
            driving_speed(0.0, 25.0) >= 11.5,
            "the car must not start crawling immediately after the downhill crest"
        );
        assert!(driving_speed(0.0, 4.0) < 6.0, "still brake before parking");
    }

    #[test]
    fn arrival_camera_keeps_its_side_when_the_explorer_exits() {
        let up = Vec3::Y;
        let departure = Vec3::NEG_Z;
        let arrival = Vec3::Z;
        let before = shot_heading(Beat::ExitHome, arrival, departure, up);
        let after = shot_heading(Beat::WalkHome, arrival, departure, up);
        assert!(
            before.dot(after) > 0.999,
            "exiting the car must not reverse the camera's viewing direction"
        );
    }

    #[test]
    fn map_timelapse_passes_through_night_and_returns_to_opening_light() {
        let start = 1.65;
        let opening = -1.35;
        assert_eq!(timelapse_angle(start, opening, 0.0), start);
        assert!(
            timelapse_angle(start, opening, 7.0).cos() < -0.9,
            "the middle of the closing timelapse must show the night hemisphere"
        );
        let end = timelapse_angle(start, opening, 14.0);
        assert!((end.cos() - opening.cos()).abs() < 0.00001);
        assert!((end.sin() - opening.sin()).abs() < 0.00001);
        assert!(
            end > start,
            "time must advance through night, not rewind sunset"
        );
    }

    #[test]
    fn homecoming_approaches_in_the_departure_direction() {
        let directory = std::env::temp_dir().join("terra-homecoming-route-test");
        let mut movie = Movie::new(directory.clone());
        let departure = movie.home_heading;
        movie.prepare_homecoming();
        let approach = tangent(movie.home - movie.walk[movie.walk.len() - 2], movie.home);
        assert!(
            approach.dot(departure) > 0.995,
            "the final walk must face the opening direction instead of requiring a camera turn"
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn interaction_waits_for_completion_without_toggling_the_vehicle_again() {
        let mut world = World::new();
        let mut vehicle = Vehicle::new(Kind::Car, Vec3::NEG_Z);
        vehicle.stable = 1.0;
        let car = world.spawn(vehicle).id();
        world.insert_resource(Exploration {
            target: Some(car),
            ..default()
        });
        interact(&mut world, car);
        interact(&mut world, car); // Residency may need another frame to prepare the action.
        assert_eq!(world.resource::<Exploration>().actions.len(), 1);
        {
            let mut state = world.resource_mut::<Exploration>();
            state.clear_actions();
            state.occupied = Some(car);
        }
        interact(&mut world, car); // The director has not changed its beat yet.
        assert!(world.resource::<Exploration>().actions.is_empty());
        exit_vehicle(&mut world, car);
        exit_vehicle(&mut world, car);
        assert_eq!(world.resource::<Exploration>().actions.len(), 1);
        {
            let mut state = world.resource_mut::<Exploration>();
            state.clear_actions();
            state.occupied = None;
        }
        exit_vehicle(&mut world, car);
        assert!(world.resource::<Exploration>().actions.is_empty());
    }

    fn landing_input(
        mut controls: ResMut<PilotControls>,
        plane: Query<(Entity, &Position, &LinearVelocity, &Vehicle)>,
    ) {
        let (e, p, v, body) = plane.single().unwrap();
        controls.entity = Some(e);
        controls.flight = if body.flight.airborne {
            let target = (p.0 + body.flight.heading * 1000.0).normalize() * 1990.0;
            pilot_at_speed(&body.flight, p.0, v.0, target, 35.0)
        } else {
            FlightInput {
                brake: true,
                ..default()
            }
        };
    }

    #[test]
    fn scripted_pilot_lands_and_stops_using_gameplay_physics() {
        let (mut app, explorer) = super::super::tests::fixture();
        let floors = app
            .world_mut()
            .query_filtered::<Entity, With<Ground>>()
            .iter(app.world())
            .collect::<Vec<_>>();
        for e in floors {
            app.world_mut().despawn(e);
        }
        app.world_mut().spawn((
            RigidBody::Static,
            Collider::sphere(2000.0),
            Transform::default(),
            Ground,
        ));
        app.world_mut()
            .entity_mut(explorer)
            .insert((ColliderDisabled, RigidBody::Kinematic));
        let mut body = Vehicle::new(Kind::Plane, Vec3::NEG_Z);
        body.parked = false;
        body.flight.airborne = true;
        let start = Vec3::Y * 2045.0;
        let e = app
            .world_mut()
            .spawn((
                body,
                RigidBody::Dynamic,
                Kind::Plane.collider(),
                Mass(900.0),
                Position(start),
                Rotation::default(),
                Transform::from_translation(start),
                SweptCcd::default(),
                CollidingEntities::default(),
                physics_reset(),
            ))
            .id();
        app.world_mut()
            .entity_mut(e)
            .insert(LinearVelocity(Vec3::NEG_Z * 40.0));
        app.world_mut().resource_mut::<Exploration>().occupied = Some(e);
        app.init_resource::<PilotControls>()
            .add_systems(FixedUpdate, landing_input.before(super::super::drive));
        for frame in 0..2400 {
            app.update();
            let body = app.world().get::<Vehicle>(e).unwrap();
            assert!(
                !body.crashed,
                "frame {frame} position {:?} speed {:?} pitch {} clearance {} contacts {:?}",
                app.world().get::<Position>(e).unwrap().0,
                app.world().get::<LinearVelocity>(e).unwrap().0,
                body.flight.pitch,
                body.clearance,
                app.world().get::<CollidingEntities>(e).unwrap()
            );
        }
        let body = app.world().get::<Vehicle>(e).unwrap();
        assert!(
            !body.flight.airborne,
            "plane must make actual ground contact"
        );
        assert!(app.world().get::<LinearVelocity>(e).unwrap().length() < 0.5);
        assert!(app.world().get::<Position>(e).unwrap().0.distance(start) > 300.0);
    }
}
