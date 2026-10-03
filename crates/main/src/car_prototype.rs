//! Throwaway car handling on the real planet, independent of vehicle lifecycle.
use crate::{
    map::{MainCamera, Player},
    ui::UiFont,
};
use avian3d::prelude::*;
use bevy::prelude::*;
use shared::{
    car_prototype::{CHASSIS_SIZE, CarCamera, CarMotion, support_origins},
    state::AppState,
};

pub struct CarPrototypePlugin;

#[cfg(test)]
mod tests {
    use super::*;

    fn physics_app() -> App {
        use std::time::Duration;
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
            Duration::from_secs_f32(1.0 / 60.0),
        ))
        .add_plugins(CarPrototypePlugin);
        app
    }

    // Replay live failures against nearby triangles from the actual seed-1337 world.
    fn replay_car(position: Vec3, heading: Vec3, velocity: Vec3, on_ice: bool) -> (App, Entity) {
        let mut app = physics_app();
        let up = position.normalize();
        let rotation = Quat::from_mat3(&Mat3::from_cols(heading.cross(up), up, -heading));
        let level = shared::level::LevelData::from_artifact_bytes(include_bytes!(
            "../assets/level_1337.bin"
        ))
        .unwrap();
        let mut surfaces = vec![level.terrain_tris.clone()];
        if on_ice {
            let (_, ice) = crate::water::build_ice_surface(
                &level.terrain_tris,
                &level.face_water_r,
                &level.face_river_r,
                &level.water_phase,
            )
            .unwrap();
            surfaces.push(ice);
        }
        for surface in surfaces {
            let nearby: Vec<_> = surface
                .into_iter()
                .filter(|triangle| {
                    triangle
                        .iter()
                        .any(|v| Vec3::from_array(*v).distance(position) < 60.0)
                })
                .collect();
            app.world_mut().spawn((
                RigidBody::Static,
                crate::map::build_collider(&nearby),
                CollisionMargin(0.02),
                Transform::default(),
            ));
        }
        let car = app
            .world_mut()
            .spawn((
                RigidBody::Dynamic,
                Collider::cuboid(CHASSIS_SIZE.x, CHASSIS_SIZE.y, CHASSIS_SIZE.z),
                Mass(800.),
                Friction::ZERO,
                Restitution::ZERO,
                SweptCcd::default(),
                LockedAxes::ROTATION_LOCKED,
                Transform::from_translation(position).with_rotation(rotation),
                Player {
                    fire_timer: Timer::default(),
                    damage: 0.,
                    range: 0.,
                    heading,
                },
                CarPrototype {
                    spawn: position,
                    spawn_heading: heading,
                    support: None,
                },
            ))
            .id();
        app.finish();
        app.cleanup();
        for _ in 0..3 {
            app.update();
        }
        app.world_mut()
            .entity_mut(car)
            .insert((Position(position), LinearVelocity(velocity)));
        app.world_mut()
            .insert_resource(State::new(AppState::Playing));
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyW);
        (app, car)
    }

    #[test]
    fn captured_terrain_edge_does_not_stop_the_car() {
        let (mut app, car) = replay_car(
            Vec3::new(-1295.7529, -1537.0782, 83.25296),
            Vec3::new(-0.7583623, 0.62991464, -0.16761376),
            Vec3::new(-23.325546, 18.013994, -5.10847),
            false,
        );
        for _ in 0..12 {
            app.update();
            let speed = app.world().get::<LinearVelocity>(car).unwrap().0.length();
            assert!(
                speed > 25.0,
                "gentle captured terrain edge reduced speed to {speed:.3} m/s"
            );
        }
    }

    #[test]
    fn captured_crest_keeps_ground_contact_and_steering() {
        let (mut app, car) = replay_car(
            Vec3::new(-1676.6333, -1096.849, 271.30713),
            Vec3::new(-0.26067144, 0.5875229, 0.76607263),
            Vec3::new(-6.6882915, 19.858925, 21.473635),
            false,
        );
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyD);
        for _ in 0..24 {
            let before = app.world().get::<Player>(car).unwrap().heading;
            app.update();
            assert!(
                app.world()
                    .get::<CarPrototype>(car)
                    .unwrap()
                    .support
                    .is_some(),
                "car lost contact on the captured crest"
            );
            assert!(
                app.world().get::<LinearVelocity>(car).unwrap().0.length() > 25.0,
                "ground following must preserve useful driving speed"
            );
            let after = app.world().get::<Player>(car).unwrap().heading;
            let up = app.world().get::<Position>(car).unwrap().0.normalize();
            let turn = before.cross(after).dot(up);
            assert!(
                turn < -0.001,
                "held steering paused on the captured crest: turn={turn}"
            );
        }
    }

    #[test]
    fn captured_ice_bank_keeps_the_car_supported() {
        let (mut app, car) = replay_car(
            Vec3::new(-1327.5901, -1439.8948, 397.2706),
            Vec3::new(0.46975398, -0.6096097, -0.63851964),
            Vec3::new(14.008424, -18.294634, -19.087753),
            true,
        );
        for _ in 0..12 {
            app.update();
            assert!(
                app.world()
                    .get::<CarPrototype>(car)
                    .unwrap()
                    .support
                    .is_some(),
                "car launched off the captured ice bank"
            );
        }
    }

    #[test]
    fn chassis_supported_by_a_gentle_slope_is_not_reported_airborne() {
        let mut app = physics_app();

        let angle = 20.0_f32.to_radians();
        let rotation = Quat::from_rotation_x(angle);
        let normal = rotation * Vec3::Y;
        app.world_mut().spawn((
            RigidBody::Static,
            Collider::cuboid(20.0, 1.0, 20.0),
            Transform::from_translation(Vec3::Y * 2000.0 - normal * 0.5).with_rotation(rotation),
        ));
        // The upright box's uphill edge rests on this ordinary 20° ramp.
        let position = Vec3::Y * (2000.0 + 0.4 + 1.6 * angle.tan() + 0.01);
        let car = app
            .world_mut()
            .spawn((
                RigidBody::Dynamic,
                Collider::cuboid(1.8, 0.8, 3.2),
                Mass(800.0),
                LockedAxes::ROTATION_LOCKED,
                Transform::from_translation(position),
                Player {
                    fire_timer: Timer::default(),
                    damage: 0.0,
                    range: 0.0,
                    heading: Vec3::NEG_Z,
                },
                CarPrototype {
                    spawn: position,
                    spawn_heading: Vec3::NEG_Z,
                    support: None,
                },
            ))
            .id();
        app.finish();
        app.cleanup();
        // Populate Avian's real broad phase while driving is disabled.
        for _ in 0..3 {
            app.update();
        }
        app.world_mut()
            .insert_resource(State::new(AppState::Playing));
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyW);
        app.world_mut().run_schedule(FixedUpdate);
        assert!(
            app.world()
                .get::<CarPrototype>(car)
                .unwrap()
                .support
                .is_some(),
            "an upright chassis resting on a 20° ramp must retain ground support"
        );
        assert!(
            app.world()
                .get::<LinearVelocity>(car)
                .unwrap()
                .0
                .dot(Vec3::NEG_Z)
                > 0.1,
            "ground support must enable powered motion on this gentle slope"
        );
        app.world_mut().get_mut::<Position>(car).unwrap().0 += Vec3::Y;
        app.world_mut().run_schedule(FixedUpdate);
        assert!(
            app.world()
                .get::<CarPrototype>(car)
                .unwrap()
                .support
                .is_none(),
            "a car lifted a metre off the same ramp must still be airborne"
        );
    }

    #[test]
    fn driving_plugin_can_initialize_its_physics_schedule() {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            bevy::state::app::StatesPlugin,
            PhysicsPlugins::default(),
        ))
        .init_state::<AppState>()
        .add_plugins(CarPrototypePlugin);
        app.world_mut().run_schedule(FixedUpdate);
    }
}

#[derive(Component)]
struct CarPrototype {
    spawn: Vec3,
    spawn_heading: Vec3,
    support: Option<Vec3>,
}

#[derive(Component)]
struct CarReadout;

impl Plugin for CarPrototypePlugin {
    fn build(&self, app: &mut App) {
        // Both feature flags can be enabled for CI; car owns input/camera then.
        app.add_systems(
            OnEnter(AppState::Playing),
            setup.after(crate::map::setup_map),
        )
        .add_systems(
            FixedUpdate,
            (drive, align).chain().run_if(in_state(AppState::Playing)),
        )
        .add_systems(
            Update,
            (reset, camera, readout)
                .chain()
                .run_if(in_state(AppState::Playing)),
        );
    }
}

fn setup(
    mut commands: Commands,
    player: Query<(Entity, &Transform, &Player, &Children)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    font: Res<UiFont>,
) {
    let Ok((entity, transform, player, children)) = player.single() else {
        return;
    };
    let spawn = transform.translation + transform.translation.normalize();
    for child in children.iter() {
        commands.entity(child).insert(Visibility::Hidden);
    }
    let body = materials.add(StandardMaterial {
        base_color: shared::theme::PRIMARY,
        ..default()
    });
    let glass = materials.add(StandardMaterial {
        base_color: shared::theme::INFO,
        ..default()
    });
    let rubber = materials.add(StandardMaterial {
        base_color: shared::theme::NEUTRAL,
        ..default()
    });
    commands
        .entity(entity)
        .remove::<(crate::physics::RadialGravity, crate::physics::RadialUpright)>()
        .insert((
            CarPrototype {
                spawn,
                spawn_heading: player.heading,
                support: None,
            },
            Collider::cuboid(CHASSIS_SIZE.x, CHASSIS_SIZE.y, CHASSIS_SIZE.z),
            Mass(800.0),
            Friction::ZERO,
            Restitution::ZERO,
            Position(spawn),
            LinearVelocity::ZERO,
        ))
        .with_children(|parent| {
            parent.spawn((
                Mesh3d(meshes.add(Cuboid::new(1.8, 0.6, 3.2))),
                MeshMaterial3d(body),
                Transform::from_xyz(0.0, 0.1, 0.0),
            ));
            parent.spawn((
                Mesh3d(meshes.add(Cuboid::new(1.4, 0.6, 1.5))),
                MeshMaterial3d(glass),
                Transform::from_xyz(0.0, 0.7, 0.2),
            ));
            for x in [-0.95, 0.95] {
                for z in [-1.05, 1.05] {
                    parent.spawn((
                        Mesh3d(meshes.add(Cylinder::new(0.4, 0.25))),
                        MeshMaterial3d(rubber.clone()),
                        Transform::from_xyz(x, 0.0, z)
                            .with_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)),
                    ));
                }
            }
        });
    commands.spawn((
        CarReadout,
        Text::new("CAR PROTOTYPE"),
        TextFont {
            font: font.0.clone().into(),
            font_size: 16.0.into(),
            ..default()
        },
        TextColor(shared::theme::INK),
        BackgroundColor(shared::theme::PANEL_BG),
        Node {
            position_type: PositionType::Absolute,
            bottom: px(12),
            left: px(240),
            padding: UiRect::all(px(10)),
            ..default()
        },
    ));
}

fn drive(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    spatial: SpatialQuery,
    mut car: Query<(Entity, &mut Player, &Position, Forces, &mut CarPrototype)>,
) {
    let Ok((entity, mut player, position, mut forces, mut car)) = car.single_mut() else {
        return;
    };
    let up = position.0.normalize();
    let filter = SpatialQueryFilter::from_excluded_entities([entity]);
    let support = support_origins(position.0, player.heading)
        .into_iter()
        .filter_map(|origin| {
            spatial.cast_ray(origin, Dir3::new(-up).unwrap(), 0.75, false, &filter)
        })
        .filter(|hit| hit.normal.dot(up) > 0.0)
        .min_by(|a, b| a.distance.total_cmp(&b.distance));
    let next_support = support.map(|hit| hit.normal);
    let throttle = f32::from(keys.pressed(KeyCode::KeyW)) - f32::from(keys.pressed(KeyCode::KeyS));
    let steering = f32::from(keys.pressed(KeyCode::KeyA)) - f32::from(keys.pressed(KeyCode::KeyD));
    let motion = CarMotion {
        velocity: forces.linear_velocity(),
        heading: player.heading,
    }
    .step(time.delta_secs(), throttle, steering, up, next_support);
    car.support = next_support;
    player.heading = motion.heading;
    *forces.linear_velocity_mut() = motion.velocity;
    forces.apply_force(-up * 16000.0); // 20 m/s² at the prototype's 800 kg mass.
}

fn align(mut car: Query<(&Player, &Position, &mut Rotation), With<CarPrototype>>) {
    let Ok((player, position, mut rotation)) = car.single_mut() else {
        return;
    };
    let up = position.0.normalize();
    // Forgiving rollover policy: keep the chassis upright. No wheel simulation.
    rotation.0 = Quat::from_mat3(&Mat3::from_cols(
        player.heading.cross(up),
        up,
        -player.heading,
    ));
}

fn reset(
    keys: Res<ButtonInput<KeyCode>>,
    mut car: Query<(
        &CarPrototype,
        &mut Player,
        &mut Position,
        &mut LinearVelocity,
    )>,
) {
    if !keys.just_pressed(KeyCode::KeyR) {
        return;
    }
    let Ok((car, mut player, mut position, mut velocity)) = car.single_mut() else {
        return;
    };
    position.0 = car.spawn;
    velocity.0 = Vec3::ZERO;
    player.heading = car.spawn_heading;
}

#[expect(
    clippy::type_complexity,
    reason = "Bevy ECS query filters encode access rules"
)]
fn camera(
    time: Res<Time>,
    spatial: SpatialQuery,
    car: Query<(Entity, &Player, &Transform), (With<CarPrototype>, Without<MainCamera>)>,
    mut cameras: Query<&mut Transform, (With<MainCamera>, Without<CarPrototype>)>,
    mut state: Local<CarCamera>,
) {
    let Ok((entity, player, transform)) = car.single() else {
        return;
    };
    let Ok(mut camera) = cameras.single_mut() else {
        return;
    };
    let origin = transform.translation;
    let offset = CarCamera::offset(origin, player.heading);
    let hit = spatial.cast_shape(
        &Collider::sphere(0.2),
        origin,
        Quat::IDENTITY,
        Dir3::new(offset).unwrap(),
        &ShapeCastConfig::from_max_distance(offset.length()),
        &SpatialQueryFilter::from_excluded_entities([entity]),
    );
    *camera = state.follow(
        origin,
        player.heading,
        time.delta_secs(),
        hit.map(|hit| hit.distance),
    );
}

fn readout(
    car: Query<(&CarPrototype, &Player, &Transform, &LinearVelocity)>,
    mut texts: Query<&mut Text, With<CarReadout>>,
) {
    let Ok((car, player, transform, velocity)) = car.single() else {
        return;
    };
    let Ok(mut text) = texts.single_mut() else {
        return;
    };
    let speed = velocity.0.dot(player.heading);
    let slope = car.support.map(|normal| {
        normal
            .dot(transform.translation.normalize())
            .clamp(-1.0, 1.0)
            .acos()
            .to_degrees()
    });
    let support = slope.map_or("airborne".to_owned(), |slope| format!("slope {slope:.0}°"));
    let stopped = car.support.is_some() && velocity.0.length() < 0.5;
    **text = format!(
        "CAR PROTOTYPE — W/S accelerate · brake/reverse · A/D steer · R reset\nSpeed {speed:.1} m/s · {support} · nearly stopped: {stopped}\nForward 30 / reverse 8 m/s · acceleration 18 / braking 12 m/s²\nGrip 8/s · powered slope ≤45° · upright assist · camera 3/8/15 m\nAlready seated; entry/exit and summoning are separate decisions"
    );
}
