use bevy::prelude::*;
use bevy::render::mesh::VertexAttributeValues;
use shared::sphere::{SpherePos, PLANET_RADIUS};
use shared::state::AppState;
use shared::terrain::TerrainGen;
use crate::constants::*;

#[derive(Component)]
pub struct Ground;

#[derive(Component)]
pub struct Sun;

#[derive(Component)]
pub struct Survivor {
    pub hp: f32,
    pub fire_timer: Timer,
    pub damage: f32,
    pub range: f32,
    /// Facing direction in the local tangent plane (heading), used for camera orientation.
    pub heading: Vec3,
}

#[derive(Resource)]
pub struct SurvivorHp(pub f32);

/// Shared mesh/material handles so spawners don't rebuild assets per entity.
#[derive(Resource)]
pub struct GameAssets {
    pub zombie_mesh: Handle<Mesh>,
    pub zombie_mat: Handle<StandardMaterial>,
    pub projectile_mesh: Handle<Mesh>,
    pub projectile_mat: Handle<StandardMaterial>,
}

pub struct MapPlugin;

impl Plugin for MapPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(AppState::Playing), setup_map)
            .add_systems(
                Update,
                (move_survivor, sync_sphere_transforms, camera_follow)
                    .chain()
                    .run_if(in_state(AppState::Playing)),
            );
    }
}

fn setup_map(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(SurvivorHp(SURVIVOR_HP));

    commands.insert_resource(GameAssets {
        zombie_mesh: meshes.add(Sphere::new(ZOMBIE_SIZE * 0.5)),
        zombie_mat: materials.add(StandardMaterial::from_color(ZOMBIE_COLOR)),
        projectile_mesh: meshes.add(Sphere::new(PROJECTILE_SIZE)),
        projectile_mat: materials.add(StandardMaterial {
            base_color: PROJECTILE_COLOR,
            emissive: LinearRgba::rgb(0.9, 0.9, 0.2),
            ..default()
        }),
    });

    // Planet with per-vertex terrain colors.
    commands.spawn((
        Mesh3d(meshes.add(build_planet_mesh())),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 0.95,
            ..default()
        })),
        Transform::default(),
        Ground,
    ));

    // Sun (follows the player, see camera_follow)
    commands.spawn((
        DirectionalLight { illuminance: 12_000.0, ..default() },
        Transform::from_xyz(1.0, 1.0, 1.0).looking_at(Vec3::ZERO, Vec3::Y),
        Sun,
        Ground,
    ));

    let start = SpherePos::new(Vec3::Y);
    commands.spawn((
        Mesh3d(meshes.add(Capsule3d::new(SURVIVOR_SIZE * 0.4, SURVIVOR_SIZE))),
        MeshMaterial3d(materials.add(StandardMaterial::from_color(SURVIVOR_COLOR))),
        start.surface_transform(0.0),
        start,
        Survivor {
            hp: SURVIVOR_HP,
            fire_timer: Timer::from_seconds(ATTACK_INTERVAL, TimerMode::Repeating),
            damage: ATTACK_DAMAGE,
            range: ATTACK_RANGE,
            heading: start.tangent_basis().1,
        },
    ));
}

/// Icosphere with per-vertex terrain colors sampled from the terrain generator.
fn build_planet_mesh() -> Mesh {
    let mut mesh = Sphere::new(PLANET_RADIUS).mesh().ico(64).unwrap();
    let terrain = TerrainGen::new(PLANET_SEED);

    let Some(VertexAttributeValues::Float32x3(positions)) =
        mesh.attribute(Mesh::ATTRIBUTE_POSITION)
    else {
        return mesh;
    };

    let colors: Vec<[f32; 4]> = positions
        .iter()
        .map(|p| {
            let pos = SpherePos::new(Vec3::from_array(*p));
            terrain.color_at(pos).to_linear().to_f32_array()
        })
        .collect();

    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh
}

fn move_survivor(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut survivor_q: Query<(&mut SpherePos, &mut Survivor)>,
) {
    let Ok((mut pos, mut survivor)) = survivor_q.single_mut() else { return };
    const SPEED: f32 = 220.0;
    const TURN: f32 = 2.5; // radians/sec
    let dt = time.delta_secs();

    let up = pos.0;

    // A/D rotate the heading about the local up axis (turn in place).
    let mut turn = 0.0;
    if keys.pressed(KeyCode::KeyA) { turn += TURN * dt; }
    if keys.pressed(KeyCode::KeyD) { turn -= TURN * dt; }
    if turn != 0.0 {
        survivor.heading = Quat::from_axis_angle(up, turn) * survivor.heading;
    }

    // Keep heading tangent to the surface (re-project + renormalize).
    survivor.heading = (survivor.heading - up * survivor.heading.dot(up)).normalize();

    // W/S move forward/back along the heading.
    let mut fwd = 0.0f32;
    if keys.pressed(KeyCode::KeyW) { fwd += 1.0; }
    if keys.pressed(KeyCode::KeyS) { fwd -= 1.0; }
    if fwd != 0.0 {
        let move_dir = survivor.heading * fwd.signum();
        pos.step_tangent(move_dir, SPEED * dt);
        // Parallel-transport the heading onto the new tangent plane so it never flips.
        let new_up = pos.0;
        survivor.heading = (survivor.heading - new_up * survivor.heading.dot(new_up)).normalize();
    }
}

/// Every actor stores its position as `SpherePos`; derive the world `Transform` from it.
/// Survivor orientation is handled separately (it must face its heading).
fn sync_sphere_transforms(
    mut q: Query<(&SpherePos, &mut Transform), (Changed<SpherePos>, Without<Survivor>)>,
) {
    for (pos, mut tf) in &mut q {
        let up = pos.0;
        tf.translation = pos.world();
        tf.rotation = Quat::from_rotation_arc(Vec3::Y, up);
    }
}

fn camera_follow(
    mut survivor_q: Query<(&SpherePos, &Survivor, &mut Transform), Without<Camera3d>>,
    mut camera_q: Query<&mut Transform, With<Camera3d>>,
    mut light_q: Query<&mut Transform, (With<Sun>, Without<Camera3d>, Without<Survivor>)>,
) {
    let Ok((pos, survivor, mut survivor_tf)) = survivor_q.single_mut() else { return };
    let Ok(mut cam_tf) = camera_q.single_mut() else { return };

    let up = pos.0;
    let surface = pos.world();

    // Orient the survivor mesh: capsule +Y = up, face the heading.
    survivor_tf.translation = surface;
    survivor_tf.rotation =
        Quat::from_mat3(&Mat3::from_cols(survivor.heading.cross(up), up, -survivor.heading));

    // Camera sits above and slightly behind the heading, looking down at the player.
    let cam_pos = surface + up * CAMERA_HEIGHT - survivor.heading * CAMERA_BACK;
    cam_tf.translation = cam_pos;
    cam_tf.look_at(surface, up);

    // Sun follows: shine down onto the player from above.
    if let Ok(mut light_tf) = light_q.single_mut() {
        light_tf.translation = surface + up * 400.0;
        light_tf.look_at(surface, survivor.heading);
    }
}
