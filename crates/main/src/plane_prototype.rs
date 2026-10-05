//! Throwaway flight experiment on the actual planet, independent of vehicle lifecycle.
use crate::{
    map::{Ground, MainCamera, Player},
    ui::UiFont,
};
use avian3d::prelude::*;
use bevy::prelude::*;
use shared::{
    plane_prototype::{FlightInput, PlaneCamera, PlaneFlight, gentle_landing},
    state::AppState,
};
use terra_geometry::planet::PlanetMesh;

pub struct PlanePrototypePlugin;

#[derive(Component, Clone)]
struct PlanePrototype {
    flight: PlaneFlight,
    home: Vec3,
    home_heading: Vec3,
    previous_velocity: Vec3,
    air_time: f32,
    pending_ground: bool,
    crashed: bool,
    clearance: f32,
    event: &'static str,
}

#[derive(Resource)]
struct PlaneWater(PlanetMesh);

impl PlaneWater {
    fn touches(&self, position: Vec3) -> bool {
        position.length() - 0.5 <= self.0.facet_radius(position.normalize(), 0.0)
    }
}

#[derive(Component)]
struct PlaneReadout;

impl Plugin for PlanePrototypePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            OnEnter(AppState::Playing),
            setup.after(crate::map::setup_map),
        )
        .add_systems(
            FixedUpdate,
            (fly, align).chain().run_if(in_state(AppState::Playing)),
        )
        .add_systems(
            FixedPostUpdate,
            touchdown
                .after(PhysicsSystems::Last)
                .run_if(in_state(AppState::Playing)),
        )
        .add_systems(
            Update,
            (reset, measure_clearance, camera, readout)
                .chain()
                .run_if(in_state(AppState::Playing)),
        );
    }
}

fn collider() -> Collider {
    Collider::compound(vec![
        (Vec3::ZERO, Quat::IDENTITY, Collider::cuboid(1.2, 1.0, 5.0)),
        (
            Vec3::new(0.0, 0.3, 0.0),
            Quat::IDENTITY,
            Collider::cuboid(8.0, 0.15, 1.2),
        ),
        (
            Vec3::new(0.0, 0.4, 2.0),
            Quat::IDENTITY,
            Collider::cuboid(3.0, 0.15, 0.8),
        ),
    ])
}

fn setup(
    mut commands: Commands,
    players: Query<(Entity, &Transform, &Player, &Children)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    font: Res<UiFont>,
) {
    let Ok((entity, transform, player, children)) = players.single() else {
        return;
    };
    let level = terra_world::level::LevelData::from_artifact_bytes(include_bytes!(
        "../assets/level_1337.bin"
    ))
    .expect("embedded level");
    let water = level
        .terrain_tris
        .iter()
        .enumerate()
        .filter_map(|(face, triangle)| {
            if level.water_phase[face] == Some(terra_world::level::WaterPhase::Frozen) {
                return None;
            }
            let radii = level.face_river_r[face]
                .map(|r| if r > 0.0 { r } else { level.face_water_r[face] });
            radii.iter().all(|r| *r > 0.0).then(|| {
                std::array::from_fn(|i| Vec3::from_array(triangle[i]).normalize() * radii[i])
            })
        })
        .collect();
    commands.insert_resource(PlaneWater(PlanetMesh::new(water)));
    for child in children.iter() {
        commands.entity(child).insert(Visibility::Hidden);
    }
    let home = transform.translation;
    let up = home.normalize();
    let body = materials.add(StandardMaterial {
        base_color: shared::theme::PRIMARY,
        ..default()
    });
    let trim = materials.add(StandardMaterial {
        base_color: shared::theme::INFO,
        ..default()
    });
    commands
        .entity(entity)
        .remove::<(crate::physics::RadialGravity, crate::physics::RadialUpright)>()
        .insert((
            collider(),
            Mass(900.0),
            Friction::ZERO,
            Restitution::ZERO,
            Position(home + up * 60.0),
            LinearVelocity::ZERO,
            PlanePrototype {
                flight: PlaneFlight::new(player.heading),
                home,
                home_heading: player.heading,
                previous_velocity: Vec3::ZERO,
                air_time: 0.0,
                pending_ground: true,
                crashed: false,
                clearance: 60.0,
                event: "Finding a clear takeoff patch",
            },
        ))
        .with_children(|parent| {
            for (size, offset, material) in [
                (Vec3::new(1.2, 1.0, 5.0), Vec3::ZERO, body.clone()),
                (
                    Vec3::new(8.0, 0.15, 1.2),
                    Vec3::new(0.0, 0.3, 0.0),
                    body.clone(),
                ),
                (
                    Vec3::new(3.0, 0.15, 0.8),
                    Vec3::new(0.0, 0.4, 2.0),
                    body.clone(),
                ),
                (
                    Vec3::new(0.15, 1.1, 1.0),
                    Vec3::new(0.0, 0.8, 1.9),
                    trim.clone(),
                ),
                (
                    Vec3::new(0.9, 0.5, 1.1),
                    Vec3::new(0.0, 0.65, -0.5),
                    trim.clone(),
                ),
            ] {
                parent.spawn((
                    Mesh3d(meshes.add(Cuboid::from_size(size))),
                    MeshMaterial3d(material),
                    Transform::from_translation(offset),
                ));
            }
        });
    commands.spawn((
        PlaneReadout,
        Text::new("PLANE PROTOTYPE"),
        TextFont {
            font: font.0.clone().into(),
            font_size: 14.0.into(),
            ..default()
        },
        TextColor(shared::theme::INK),
        BackgroundColor(shared::theme::PANEL_BG),
        Node {
            position_type: PositionType::Absolute,
            bottom: px(12),
            left: px(240),
            padding: UiRect::all(px(8)),
            ..default()
        },
    ));
}

fn ground_hit(
    spatial: &SpatialQuery,
    ground: &Query<(), (With<Ground>, Without<crate::chunks::WorldObstacle>)>,
    entity: Entity,
    position: Vec3,
    heading: Vec3,
) -> Option<RayHitData> {
    let up = position.normalize();
    let side = heading.cross(up);
    let filter = SpatialQueryFilter::from_excluded_entities([entity]);
    [
        Vec3::ZERO,
        side + heading * 1.8,
        -side + heading * 1.8,
        side - heading * 1.8,
        -side - heading * 1.8,
    ]
    .into_iter()
    .filter_map(|offset| {
        spatial.cast_ray(
            position + offset,
            Dir3::new(-up).unwrap(),
            0.9,
            false,
            &filter,
        )
    })
    .filter(|hit| ground.contains(hit.entity) && hit.normal.dot(up) > 0.7)
    .min_by(|a, b| a.distance.total_cmp(&b.distance))
}

fn fly(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    spatial: SpatialQuery,
    ground: Query<(), (With<Ground>, Without<crate::chunks::WorldObstacle>)>,
    mut planes: Query<(Entity, &Position, &mut Player, &mut PlanePrototype, Forces)>,
) {
    let Ok((entity, position, mut player, mut plane, mut forces)) = planes.single_mut() else {
        return;
    };
    if plane.pending_ground {
        return;
    }
    if plane.crashed {
        forces.apply_force(-position.0.normalize() * 18000.0);
        return;
    }
    let up = position.0.normalize();
    let hit = ground_hit(&spatial, &ground, entity, position.0, plane.flight.heading);
    let input = FlightInput {
        pitch: f32::from(keys.pressed(KeyCode::KeyS)) - f32::from(keys.pressed(KeyCode::KeyW)),
        bank: f32::from(keys.pressed(KeyCode::KeyA)) - f32::from(keys.pressed(KeyCode::KeyD)),
        throttle: f32::from(keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight))
            - f32::from(keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight)),
        brake: keys.pressed(KeyCode::Space),
    };
    let velocity = plane.flight.step(
        time.delta_secs(),
        input,
        forces.linear_velocity(),
        up,
        hit.map(|h| h.normal),
    );
    plane.previous_velocity = velocity;
    plane.air_time = if plane.flight.airborne {
        plane.air_time + time.delta_secs()
    } else {
        0.0
    };
    player.heading = plane.flight.heading;
    *forces.linear_velocity_mut() = velocity;
    if !plane.flight.airborne {
        forces.apply_force(-up * 18000.0);
    }
}

fn measure_clearance(
    spatial: SpatialQuery,
    water: Option<Res<PlaneWater>>,
    mut planes: Query<(Entity, &Position, &mut PlanePrototype)>,
) {
    let Ok((entity, position, mut plane)) = planes.single_mut() else {
        return;
    };
    let up = position.0.normalize();
    plane.clearance = spatial
        .cast_ray(
            position.0,
            Dir3::new(-up).unwrap(),
            5000.0,
            false,
            &SpatialQueryFilter::from_excluded_entities([entity]),
        )
        .map_or(5000.0, |h| h.distance);
    if let Some(water) = water {
        let radius = water.0.facet_radius(up, 0.0);
        if radius > 0.0 {
            plane.clearance = plane.clearance.min((position.0.length() - radius).max(0.0));
        }
    }
}

fn align(mut planes: Query<(&Position, &PlanePrototype, &mut Rotation)>) {
    for (position, plane, mut rotation) in &mut planes {
        if !plane.crashed {
            rotation.0 = plane.flight.rotation(position.0.normalize());
        }
    }
}

fn airborne_reset(plane: &mut PlanePrototype, reason: &'static str) -> (Position, LinearVelocity) {
    plane.flight = PlaneFlight::new(plane.home_heading);
    plane.flight.airborne = true;
    plane.flight.throttle = 0.0;
    plane.air_time = 0.0;
    plane.pending_ground = false;
    plane.crashed = false;
    plane.event = reason;
    plane.previous_velocity = plane.home_heading * 60.0;
    (
        Position(plane.home + plane.home.normalize() * 60.0),
        LinearVelocity(plane.previous_velocity),
    )
}

fn wreck(commands: &mut Commands, entity: Entity, plane: &mut PlanePrototype, event: &'static str) {
    plane.crashed = true;
    plane.flight.throttle = 0.0;
    plane.event = event;
    // Preserve collision-resolved momentum; release controller rotation so the
    // body can tumble, skid, and settle under gravity rather than pinning its nose.
    commands.entity(entity).remove::<LockedAxes>().insert((
        Friction::new(0.6),
        Restitution::new(0.1),
        LinearDamping(0.1),
        AngularDamping(1.5),
    ));
}

fn reset_physics() -> impl Bundle {
    (
        LockedAxes::ROTATION_LOCKED,
        AngularVelocity(Vec3::ZERO),
        Friction::ZERO,
        Restitution::ZERO,
        LinearDamping(0.0),
        AngularDamping(0.0),
    )
}

fn touchdown(
    mut commands: Commands,
    spatial: SpatialQuery,
    water: Option<Res<PlaneWater>>,
    ground: Query<(), (With<Ground>, Without<crate::chunks::WorldObstacle>)>,
    mut planes: Query<(Entity, &Position, &CollidingEntities, &mut PlanePrototype)>,
) {
    for (entity, position, contacts, mut plane) in &mut planes {
        if plane.pending_ground || plane.crashed {
            continue;
        }
        let up = position.0.normalize();
        let wet = water
            .as_ref()
            .is_some_and(|water| water.touches(position.0));
        let obstacle_impact = contacts.0.iter().any(|e| !ground.contains(*e));
        if wet {
            wreck(
                &mut commands,
                entity,
                &mut plane,
                "Water contact — R flight reset / T ground reset",
            );
        } else if obstacle_impact
            || (!contacts.0.is_empty() && plane.flight.airborne && plane.air_time > 0.3)
        {
            let hit = ground_hit(&spatial, &ground, entity, position.0, plane.flight.heading);
            let gentle = !obstacle_impact
                && hit.is_some_and(|hit| {
                    ground.contains(hit.entity)
                        && contacts.0.iter().all(|e| ground.contains(*e))
                        && gentle_landing(
                            plane.previous_velocity,
                            hit.normal,
                            up,
                            plane.flight.pitch,
                            plane.flight.bank,
                        )
                });
            if gentle {
                plane.flight.airborne = false;
                plane.flight.stalled = false;
                plane.flight.pitch = 0.0;
                plane.flight.bank = 0.0;
                plane.flight.throttle = 0.0;
                plane.event = "Gentle landing — Space brakes; Shift then S to take off";
            } else {
                wreck(
                    &mut commands,
                    entity,
                    &mut plane,
                    "Hard impact — R flight reset / T ground reset",
                );
            }
        }
    }
}

fn clear_ground(
    spatial: &SpatialQuery,
    entity: Entity,
    origin: Vec3,
    heading: Vec3,
    water: Option<&PlaneWater>,
    ground: &Query<(), (With<Ground>, Without<crate::chunks::WorldObstacle>)>,
) -> Option<(Vec3, Vec3)> {
    let up = origin.normalize();
    let side = heading.cross(up);
    for radius in [0.0, 80.0, 160.0, 300.0, 500.0] {
        for sector in 0..12 {
            let angle = sector as f32 * std::f32::consts::TAU / 12.0;
            let candidate = origin + (heading * angle.cos() + side * angle.sin()) * radius;
            let local_up = candidate.normalize();
            let Some(hit) = spatial.cast_ray(
                candidate + local_up * 100.0,
                Dir3::new(-local_up).unwrap(),
                candidate.length() + 100.0,
                false,
                &SpatialQueryFilter::from_excluded_entities([entity]),
            ) else {
                continue;
            };
            if !ground.contains(hit.entity) || hit.normal.dot(local_up) < 8.0_f32.to_radians().cos()
            {
                continue;
            }
            let position = candidate + local_up * (100.0 - hit.distance + 0.65);
            if water.is_some_and(|water| water.touches(position)) {
                continue;
            }
            let forward = (heading - local_up * heading.dot(local_up)).normalize();
            let rotation = PlaneFlight::new(forward).rotation(local_up);
            let mut clear = true;
            for distance in [0.0, 10.0, 20.0, 30.0, 40.0, 50.0] {
                let sample = position + forward * distance;
                let sample_up = sample.normalize();
                let Some(floor) = spatial.cast_ray(
                    sample + sample_up * 10.0,
                    Dir3::new(-sample_up).unwrap(),
                    30.0,
                    false,
                    &SpatialQueryFilter::from_excluded_entities([entity]),
                ) else {
                    clear = false;
                    break;
                };
                if !ground.contains(floor.entity)
                    || floor.normal.dot(sample_up) < 8.0_f32.to_radians().cos()
                    || (floor.distance - 10.65).abs() > 2.0
                {
                    clear = false;
                    break;
                }
                let center = sample + sample_up * (10.0 - floor.distance + 0.65);
                if water.is_some_and(|water| water.touches(center))
                    || !spatial
                        .shape_intersections(
                            &collider(),
                            center,
                            rotation,
                            &SpatialQueryFilter::from_excluded_entities([entity, floor.entity]),
                        )
                        .is_empty()
                {
                    clear = false;
                    break;
                }
            }
            if clear {
                return Some((position, forward));
            }
        }
    }
    None
}

fn reset(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    spatial: SpatialQuery,
    water: Option<Res<PlaneWater>>,
    ground: Query<(), (With<Ground>, Without<crate::chunks::WorldObstacle>)>,
    planes: Query<(Entity, &Position, &PlanePrototype)>,
) {
    let Ok((entity, position, old)) = planes.single() else {
        return;
    };
    if keys.just_pressed(KeyCode::KeyR) {
        let mut plane = old.clone();
        let reset = airborne_reset(
            &mut plane,
            "Flight reset — hold Shift for thrust; release to coast",
        );
        commands
            .entity(entity)
            .insert((plane, reset, reset_physics()));
    } else if old.pending_ground || keys.just_pressed(KeyCode::KeyT) {
        let mut plane = old.clone();
        let origin = if old.pending_ground {
            plane.home
        } else {
            position.0
        };
        if let Some((position, heading)) = clear_ground(
            &spatial,
            entity,
            origin,
            plane.flight.heading,
            water.as_deref(),
            &ground,
        ) {
            plane.flight = PlaneFlight::new(heading);
            plane.pending_ground = false;
            plane.crashed = false;
            plane.home = position;
            plane.home_heading = heading;
            plane.air_time = 0.0;
            plane.previous_velocity = Vec3::ZERO;
            plane.event = "Ground reset — hold Shift, then S at 24 m/s";
            commands.entity(entity).insert((
                plane,
                Position(position),
                LinearVelocity::ZERO,
                reset_physics(),
            ));
        } else {
            let reset = airborne_reset(
                &mut plane,
                "No clear ground patch found; started in flight. T retries nearby",
            );
            commands
                .entity(entity)
                .insert((plane, reset, reset_physics()));
        }
    }
}

fn camera(
    time: Res<Time>,
    spatial: SpatialQuery,
    planes: Query<(Entity, &Transform, &PlanePrototype), Without<MainCamera>>,
    mut cameras: Query<&mut Transform, (With<MainCamera>, Without<PlanePrototype>)>,
    mut camera: Local<PlaneCamera>,
) {
    let Ok((entity, transform, plane)) = planes.single() else {
        return;
    };
    let Ok(mut view) = cameras.single_mut() else {
        return;
    };
    let position = transform.translation;
    let offset = PlaneCamera::offset(position, plane.flight.heading);
    let hit = spatial.cast_shape(
        &Collider::sphere(0.3),
        position,
        Quat::IDENTITY,
        Dir3::new(offset).unwrap(),
        &ShapeCastConfig::from_max_distance(offset.length()),
        &SpatialQueryFilter::from_excluded_entities([entity]),
    );
    *view = camera.follow(
        position,
        plane.flight.heading,
        time.delta_secs(),
        hit.map(|h| h.distance),
    );
}

fn readout(
    planes: Query<(&PlanePrototype, &LinearVelocity, &Position)>,
    mut texts: Query<&mut Text, With<PlaneReadout>>,
) {
    let Ok((plane, velocity, position)) = planes.single() else {
        return;
    };
    let Ok(mut text) = texts.single_mut() else {
        return;
    };
    text.0 = format!(
        "PLANE PROTOTYPE · {} · {:.0} m/s · thrust {:.0}% · altitude ASL {:.0} m · clearance {:.0} m\nW/S pitch down/up · A/D bank/turn (steer on ground) · Hold Shift thrust · Ctrl airbrake · Space ground brake\nR flight reset · T nearby ground reset · {}",
        if plane.crashed {
            "CRASHED"
        } else if plane.flight.stalled {
            "STALL — lower nose + Shift"
        } else if plane.flight.airborne {
            "AIR"
        } else {
            "GROUND"
        },
        velocity.length(),
        plane.flight.throttle * 100.0,
        position.0.length() - terra_geometry::sphere::PLANET_RADIUS,
        plane.clearance,
        plane.event
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn physics_app(position: Vec3, velocity: Vec3, airborne: bool) -> (App, Entity) {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            TransformPlugin,
            AssetPlugin::default(),
            bevy::state::app::StatesPlugin,
            PhysicsPlugins::default(),
        ))
        .init_state::<AppState>()
        .init_asset::<Mesh>()
        .init_resource::<ButtonInput<KeyCode>>()
        .insert_resource(Gravity::ZERO)
        .insert_resource(SubstepCount(12))
        .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f32(1.0 / 60.0),
        ))
        .add_plugins(PlanePrototypePlugin);
        app.world_mut().spawn((
            RigidBody::Static,
            Collider::cuboid(1000.0, 1.0, 1000.0),
            Transform::from_xyz(0.0, 1999.5, 0.0),
            Ground,
        ));
        let mut flight = PlaneFlight::new(Vec3::NEG_Z);
        flight.airborne = airborne;
        flight.throttle = if airborne { 0.0 } else { 1.0 };
        flight.pitch = if airborne { -0.1 } else { 0.0 };
        let entity = app
            .world_mut()
            .spawn((
                RigidBody::Dynamic,
                collider(),
                Mass(900.0),
                Friction::ZERO,
                Restitution::ZERO,
                LockedAxes::ROTATION_LOCKED,
                SweptCcd::default(),
                CollidingEntities::default(),
                Transform::from_translation(position),
                Player {
                    heading: Vec3::NEG_Z,
                    fire_timer: Timer::default(),
                    damage: 0.0,
                    range: 0.0,
                },
                PlanePrototype {
                    flight,
                    home: Vec3::new(0.0, 2000.65, 0.0),
                    home_heading: Vec3::NEG_Z,
                    previous_velocity: velocity,
                    air_time: 1.0,
                    pending_ground: false,
                    crashed: false,
                    clearance: 1.0,
                    event: "Test",
                },
            ))
            .id();
        app.finish();
        app.cleanup();
        for _ in 0..3 {
            app.update();
        }
        app.world_mut()
            .entity_mut(entity)
            .insert((Position(position), LinearVelocity(velocity)));
        app.world_mut()
            .insert_resource(State::new(AppState::Playing));
        (app, entity)
    }

    #[test]
    fn real_body_takes_off_after_a_ground_run_and_pull_up() {
        let (mut app, entity) = physics_app(Vec3::new(0.0, 2000.65, 0.0), Vec3::ZERO, false);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::ShiftLeft);
        for _ in 0..300 {
            app.update();
            if app.world().get::<LinearVelocity>(entity).unwrap().length() >= 26.0 {
                break;
            }
        }
        assert!(app.world().get::<LinearVelocity>(entity).unwrap().length() >= 26.0);
        assert!(
            !app.world()
                .get::<PlanePrototype>(entity)
                .unwrap()
                .flight
                .airborne
        );
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyS);
        for _ in 0..60 {
            app.update();
        }
        assert!(
            app.world()
                .get::<PlanePrototype>(entity)
                .unwrap()
                .flight
                .airborne
        );
        assert!(
            app.world().get::<Position>(entity).unwrap().y > 2003.0,
            "position={:?} velocity={:?} event={} pitch={}",
            app.world().get::<Position>(entity).unwrap(),
            app.world().get::<LinearVelocity>(entity).unwrap(),
            app.world().get::<PlanePrototype>(entity).unwrap().event,
            app.world()
                .get::<PlanePrototype>(entity)
                .unwrap()
                .flight
                .pitch
        );
    }

    #[test]
    fn gentle_touchdown_returns_to_ground_controls() {
        let (mut app, entity) = physics_app(
            Vec3::new(0.0, 2000.8, 0.0),
            Vec3::new(0.0, -3.0, -30.0),
            true,
        );
        for _ in 0..100 {
            app.update();
        }
        let plane = app.world().get::<PlanePrototype>(entity).unwrap();
        assert!(!plane.flight.airborne, "{}", plane.event);
        assert!(plane.event.starts_with("Gentle landing"));
    }

    #[test]
    fn wall_impact_keeps_the_wreck_on_site_until_manual_reset() {
        let (mut app, entity) = physics_app(Vec3::new(0.0, 2005.0, 0.0), Vec3::NEG_Z * 80.0, true);
        app.world_mut().spawn((
            RigidBody::Static,
            Collider::cuboid(30.0, 20.0, 0.5),
            Transform::from_xyz(0.0, 2005.0, -5.0),
        ));
        for _ in 0..30 {
            app.update();
        }
        assert!(
            app.world()
                .get::<PlanePrototype>(entity)
                .unwrap()
                .event
                .starts_with("Hard impact")
        );
        assert!(app.world().get::<Position>(entity).unwrap().y < 2010.0);
        assert_eq!(
            app.world()
                .get::<PlanePrototype>(entity)
                .unwrap()
                .flight
                .throttle,
            0.0
        );
        assert!(app.world().get::<Position>(entity).unwrap().y < 2004.0);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyR);
        app.update();
        assert!(app.world().get::<Position>(entity).unwrap().y > 2050.0);
        assert!(!app.world().get::<PlanePrototype>(entity).unwrap().crashed);
    }

    #[test]
    fn obstacle_top_does_not_provide_runway_support() {
        let (mut app, entity) = physics_app(Vec3::new(0.0, 2010.8, 0.0), Vec3::ZERO, false);
        app.world_mut().spawn((
            RigidBody::Static,
            Collider::cuboid(20.0, 1.0, 20.0),
            Ground,
            crate::chunks::WorldObstacle,
            Transform::from_xyz(0.0, 2009.5, 0.0),
        ));
        app.update();
        assert!(
            app.world()
                .get::<PlanePrototype>(entity)
                .unwrap()
                .flight
                .airborne
        );
    }

    #[test]
    fn obstacle_contact_crashes_at_low_speed_and_during_takeoff_grace() {
        for airborne in [false, true] {
            let (mut app, entity) =
                physics_app(Vec3::new(0.0, 2000.65, 0.0), Vec3::NEG_Z, airborne);
            app.world_mut()
                .get_mut::<PlanePrototype>(entity)
                .unwrap()
                .air_time = 0.0;
            // A narrow trunk touches the outer wing, away from the fuselage.
            app.world_mut().spawn((
                RigidBody::Static,
                Collider::cylinder(0.25, 6.0),
                Ground,
                crate::chunks::WorldObstacle,
                Transform::from_xyz(3.5, 2003.0, 0.0),
            ));
            for _ in 0..5 {
                app.update();
            }
            assert!(
                app.world().get::<PlanePrototype>(entity).unwrap().crashed,
                "obstacle contact was ignored with airborne={airborne}"
            );
        }
    }

    #[test]
    fn fast_ground_impact_stops_without_teleporting() {
        let (mut app, entity) =
            physics_app(Vec3::new(0.0, 2000.65, 0.0), Vec3::NEG_Z * 30.0, false);
        app.world_mut().spawn((
            RigidBody::Static,
            Collider::cuboid(30.0, 10.0, 0.5),
            Transform::from_xyz(0.0, 2005.0, -5.0),
        ));
        for _ in 0..30 {
            app.update();
        }
        assert!(
            app.world()
                .get::<PlanePrototype>(entity)
                .unwrap()
                .event
                .starts_with("Hard impact")
        );
        assert!(app.world().get::<Position>(entity).unwrap().y < 2010.0);
    }

    #[test]
    fn an_airborne_stall_loses_height_and_can_recover_with_a_nose_down_thrust_run() {
        let (mut app, entity) = physics_app(Vec3::new(0.0, 2200.0, 0.0), Vec3::NEG_Z * 18.0, true);
        for _ in 0..30 {
            app.update();
        }
        assert!(
            app.world()
                .get::<PlanePrototype>(entity)
                .unwrap()
                .flight
                .stalled
        );
        assert!(app.world().get::<Position>(entity).unwrap().y < 2199.0);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::ShiftLeft);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyW);
        for _ in 0..90 {
            app.update();
        }
        let plane = app.world().get::<PlanePrototype>(entity).unwrap();
        assert!(!plane.flight.stalled && !plane.crashed);
    }

    #[test]
    fn hard_nose_impact_tumbles_instead_of_pinning_the_plane() {
        let (mut app, entity) = physics_app(
            Vec3::new(0.0, 2003.0, 0.0),
            Vec3::new(0.0, -30.0, -25.0),
            true,
        );
        app.world_mut()
            .get_mut::<PlanePrototype>(entity)
            .unwrap()
            .flight
            .pitch = -0.9;
        for _ in 0..120 {
            app.update();
            if app.world().get::<PlanePrototype>(entity).unwrap().crashed {
                break;
            }
        }
        assert!(app.world().get::<PlanePrototype>(entity).unwrap().crashed);
        let impact_rotation = app.world().get::<Rotation>(entity).unwrap().0;
        for _ in 0..120 {
            app.update();
        }
        let rotation = app.world().get::<Rotation>(entity).unwrap().0;
        assert!(
            impact_rotation.angle_between(rotation) > 0.15,
            "wreck stayed pinned at its impact angle"
        );
        let position = app.world().get::<Position>(entity).unwrap().0;
        let clearance = app.world().get::<PlanePrototype>(entity).unwrap().clearance;
        assert!(
            (clearance - (position.y - 2000.0)).abs() < 0.1,
            "wreck clearance stayed stale: {clearance} at {position:?}"
        );
        assert!(position.y < 2004.0, "crash must not teleport the plane");
    }

    #[test]
    fn ground_reset_finds_a_clear_patch_below_a_high_flight() {
        let (mut app, entity) = physics_app(Vec3::new(0.0, 2500.0, 0.0), Vec3::NEG_Z * 60.0, true);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyT);
        app.update();
        let plane = app.world().get::<PlanePrototype>(entity).unwrap();
        assert!(!plane.flight.airborne, "{}", plane.event);
        assert!((app.world().get::<Position>(entity).unwrap().y - 2000.65).abs() < 0.1);
    }

    #[test]
    fn liquid_surface_contact_stops_without_teleporting() {
        let (mut app, entity) = physics_app(Vec3::new(0.0, 2001.0, 0.0), Vec3::NEG_Z * 30.0, true);
        app.world_mut()
            .insert_resource(PlaneWater(PlanetMesh::new(vec![[
                Vec3::new(-50.0, 2001.0, -50.0),
                Vec3::new(50.0, 2001.0, -50.0),
                Vec3::new(0.0, 2001.0, 50.0),
            ]])));
        app.update();
        assert!(
            app.world()
                .get::<PlanePrototype>(entity)
                .unwrap()
                .event
                .starts_with("Water contact")
        );
        assert!(app.world().get::<Position>(entity).unwrap().y < 2010.0);
        for _ in 0..30 {
            app.update();
        }
        assert!(app.world().get::<Position>(entity).unwrap().y < 2000.8);
    }
}
