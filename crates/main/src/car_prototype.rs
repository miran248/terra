//! Throwaway car handling on the real planet, independent of vehicle lifecycle.
use crate::{
    map::{MainCamera, Player},
    ui::UiFont,
};
use avian3d::prelude::*;
use bevy::prelude::*;
use shared::{
    car_prototype::{CarCamera, CarMotion},
    state::AppState,
};

pub struct CarPrototypePlugin;

#[cfg(test)]
mod tests {
    use super::*;

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
        #[cfg(feature = "on-foot-prototype")]
        app.init_resource::<shared::on_foot_prototype::OnFootPrototype>();
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
            Collider::cuboid(1.8, 0.8, 3.2),
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
    let support = spatial.cast_ray(
        position.0,
        Dir3::new(-up).unwrap(),
        0.75,
        false,
        &SpatialQueryFilter::from_excluded_entities([entity]),
    );
    car.support = support.map(|hit| hit.normal);
    let throttle = f32::from(keys.pressed(KeyCode::KeyW)) - f32::from(keys.pressed(KeyCode::KeyS));
    let steering = f32::from(keys.pressed(KeyCode::KeyA)) - f32::from(keys.pressed(KeyCode::KeyD));
    let motion = CarMotion {
        velocity: forces.linear_velocity(),
        heading: player.heading,
    }
    .step(time.delta_secs(), throttle, steering, up, car.support);
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
    let offset = origin.normalize() * 3.0 - player.heading * 8.0;
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
        "CAR PROTOTYPE — W/S accelerate · brake/reverse · A/D steer · R reset\nSpeed {speed:.1} m/s · {support} · nearly stopped: {stopped}\nForward 30 / reverse 8 m/s · acceleration 6 / braking 12 m/s²\nGrip 8/s · powered slope ≤45° · upright assist · camera 3/8/15 m\nAlready seated; entry/exit and summoning are separate decisions"
    );
}
