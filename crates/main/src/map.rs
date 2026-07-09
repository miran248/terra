use avian3d::prelude::*;
use bevy::prelude::*;
use bevy::render::mesh::VertexAttributeValues;
use crate::physics::RadialGravity;
use shared::level::LevelData;
use shared::sphere::PLANET_RADIUS;
use shared::terrain::TerrainGen;
use shared::theme;
use shared::planet::PlanetMesh;
use crate::minimap::MinimapCamera;
use crate::constants::*;

#[derive(Component)]
pub struct Ground;

#[derive(Component)]
pub struct Sun;

#[derive(Component)]
pub struct MainCamera;

#[derive(Component)]
pub struct Settlement {
    pub name: String,
}

#[derive(Component)]
pub struct Player {
    pub hp: f32,
    pub fire_timer: Timer,
    pub damage: f32,
    pub range: f32,
    pub heading: Vec3,
}

#[derive(Resource)]
pub struct PlayerHp(pub f32);

#[derive(Resource)]
pub struct GameAssets {
    pub zombie_mesh: Handle<Mesh>,
    pub zombie_mat: Handle<StandardMaterial>,
    pub projectile_mesh: Handle<Mesh>,
    pub projectile_mat: Handle<StandardMaterial>,
}

pub struct MapPlugin;

#[derive(Resource)]
pub struct LevelFeatures(pub Vec<u8>);

#[derive(Resource, Default)]
struct PlayerInput {
    fwd: i8,
    turning: i8,
    sprint: bool,
    jump: bool,
}

impl Plugin for MapPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PlayerInput>()
            .add_systems(OnEnter(AppState::Playing), setup_map)
            .add_systems(
                Update,
                read_player_input.run_if(in_state(AppState::Playing)),
            )
            .add_systems(
                FixedUpdate,
                move_player.run_if(in_state(AppState::Playing)),
            )
            .add_systems(
                Update,
                (orient_player, camera_follow)
                    .run_if(in_state(AppState::Playing)),
            );
    }
}

use shared::state::AppState;

fn setup_map(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let level_bytes = include_bytes!("../assets/level_1337.bin");
    let level: LevelData = postcard::from_bytes(level_bytes).expect("deserialize level");

    commands.insert_resource(PlayerHp(PLAYER_HP));
    let terrain = TerrainGen::new(PLANET_SEED);

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

    // Terrain layer
    let terrain_mesh = build_visual_mesh(&level.terrain_tris, &level.terrain_colors);
    commands.spawn((
        Mesh3d(meshes.add(terrain_mesh)),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 0.95,
            ..default()
        })),
        Transform::default(),
        Ground,
    ));
    commands.spawn((
        RigidBody::Static,
        build_collider(&level.terrain_tris),
        Transform::default(),
        Ground,
    ));

    // Features layer
    if !level.feature_tris.is_empty() {
        let feat_mesh = build_visual_mesh(&level.feature_tris, &level.feature_colors);
        commands.spawn((
            Mesh3d(meshes.add(feat_mesh)),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: Color::WHITE,
                perceptual_roughness: 0.4,
                cull_mode: None,
                ..default()
            })),
            Transform::default(),
            Ground,
        ));
        commands.spawn((
            RigidBody::Static,
            build_collider(&level.feature_tris),
            Transform::default(),
            Ground,
        ));
    }

    // Water
    commands.spawn((
        Mesh3d(meshes.add(Sphere::new(PLANET_RADIUS))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: theme::WATER_SURFACE,
            alpha_mode: AlphaMode::Blend,
            cull_mode: None,
            perceptual_roughness: 0.3,
            reflectance: 0.1,
            ..default()
        })),
        Transform::default(),
        Ground,
    ));

    // Sun
    commands.spawn((
        DirectionalLight { illuminance: 12_000.0, ..default() },
        Transform::from_xyz(1.0, 1.0, 1.0).looking_at(Vec3::ZERO, Vec3::Y),
        Sun,
        Ground,
    ));

    // Player
    let start = terrain.habitable_spawn();
    let up = start.0;
    let capsule_radius = PLAYER_SIZE * 0.4;
    let capsule_half = capsule_radius + PLAYER_SIZE * 0.5;
    let spawn_r = terrain.surface_radius(start) + capsule_half + 0.5;
    let spawn_pos = up * spawn_r;
    commands.spawn((
        Mesh3d(meshes.add(Capsule3d::new(capsule_radius, PLAYER_SIZE))),
        MeshMaterial3d(materials.add(StandardMaterial::from_color(PLAYER_COLOR))),
        RigidBody::Dynamic,
        ColliderConstructor::Sphere { radius: PLAYER_SIZE * 0.5 },
        RadialGravity,
        Mass(80.0),
        ColliderDensity(1000.0),
        LockedAxes::ROTATION_LOCKED,
        Transform::from_translation(spawn_pos),
        start,
        Player {
            hp: PLAYER_HP,
            fire_timer: Timer::from_seconds(ATTACK_INTERVAL, TimerMode::Repeating),
            damage: ATTACK_DAMAGE,
            range: ATTACK_RANGE,
            heading: start.tangent_basis().1,
        },
    ));

    // Settlement markers
    for s in &level.settlements {
        let pos = Vec3::from_array(s.pos) * PLANET_RADIUS;
        commands.spawn((
            Transform::from_translation(pos),
            Settlement { name: s.name.clone() },
        ));
    }
    let tris: Vec<[Vec3; 3]> = level.unit_tris.iter()
        .map(|t| [Vec3::from_array(t[0]), Vec3::from_array(t[1]), Vec3::from_array(t[2])])
        .collect();
    let planet_mesh = PlanetMesh::new(tris);
    let features = level.terrain_features.clone();
    commands.insert_resource(terrain);
    commands.insert_resource(planet_mesh);
    commands.insert_resource(LevelFeatures(features));
}

fn build_visual_mesh(tris: &[[[f32; 3]; 3]], colors: &[[f32; 4]]) -> Mesh {
    let mut positions = Vec::with_capacity(tris.len() * 3);
    let mut normals_out = Vec::with_capacity(tris.len() * 3);
    let mut colors_out = Vec::with_capacity(tris.len() * 3);

    for (tri, color) in tris.iter().zip(colors.iter()) {
        let a = Vec3::from_array(tri[0]);
        let b = Vec3::from_array(tri[1]);
        let c = Vec3::from_array(tri[2]);
        let n = (b - a).cross(c - a).normalize();

        positions.push(tri[0]);
        positions.push(tri[1]);
        positions.push(tri[2]);
        normals_out.push(n.to_array());
        normals_out.push(n.to_array());
        normals_out.push(n.to_array());
        colors_out.push(*color);
        colors_out.push(*color);
        colors_out.push(*color);
    }

    let mut mesh = Mesh::new(
        bevy::render::mesh::PrimitiveTopology::TriangleList,
        Default::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, VertexAttributeValues::Float32x3(positions));
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, VertexAttributeValues::Float32x3(normals_out));
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, VertexAttributeValues::Float32x4(colors_out));
    mesh
}

fn build_collider(tris: &[[[f32; 3]; 3]]) -> ColliderConstructor {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for (i, tri) in tris.iter().enumerate() {
        let base = i as u32 * 3;
        vertices.push(Vec3::from_array(tri[0]));
        vertices.push(Vec3::from_array(tri[1]));
        vertices.push(Vec3::from_array(tri[2]));
        indices.push([base, base + 1, base + 2]);
    }
    ColliderConstructor::Trimesh { vertices, indices }
}

// ---- player movement ----

fn read_player_input(keys: Res<ButtonInput<KeyCode>>, mut input: ResMut<PlayerInput>) {
    input.fwd = 0;
    if keys.pressed(KeyCode::KeyW) { input.fwd += 1; }
    if keys.pressed(KeyCode::KeyS) { input.fwd -= 1; }
    input.turning = 0;
    if keys.pressed(KeyCode::KeyA) { input.turning += 1; }
    if keys.pressed(KeyCode::KeyD) { input.turning -= 1; }
    input.sprint = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    input.jump = keys.just_pressed(KeyCode::Space);
}

fn move_player(
    time: Res<Time>,
    input: Res<PlayerInput>,
    mut player_q: Query<(&mut Player, &Position, Forces)>,
) {
    let Ok((mut player, pos, mut forces)) = player_q.single_mut() else { return };
    let dt = time.delta_secs();
    let up = pos.0.normalize();
    let world_r = pos.0.length();

    if input.turning != 0 {
        let angle = PLAYER_TURN * dt * input.turning as f32;
        player.heading = Quat::from_axis_angle(up, angle) * player.heading;
    }
    player.heading = (player.heading - up * player.heading.dot(up)).normalize();

    let underwater = world_r < PLANET_RADIUS;
    let speed = PLAYER_SPEED
        * if underwater { 0.4 } else { 1.0 }
        * if input.sprint { 2.5 } else { 1.0 };

    if input.fwd != 0 {
        let dir = player.heading * input.fwd as f32;
        let radial = up * forces.linear_velocity().dot(up);
        *forces.linear_velocity_mut() = dir * speed + radial;
    } else {
        let v = forces.linear_velocity();
        *forces.linear_velocity_mut() = up * v.dot(up);
    }

    if input.jump {
        let v = forces.linear_velocity();
        *forces.linear_velocity_mut() = v + up * 8.0;
    }
}

fn orient_player(mut q: Query<(&Player, &mut Transform), With<RigidBody>>) {
    for (player, mut tf) in &mut q {
        let up = tf.translation.normalize();
        tf.rotation = Quat::from_mat3(&Mat3::from_cols(
            player.heading.cross(up), up, -player.heading,
        ));
    }
}

fn camera_follow(
    time: Res<Time>,
    mut player_q: Query<(&Player, &Transform), Without<MainCamera>>,
    mut camera_q: Query<&mut Transform, (With<MainCamera>, Without<MinimapCamera>)>,
    mut light_q: Query<&mut Transform, (With<Sun>, Without<MainCamera>, Without<Player>, Without<MinimapCamera>)>,
) {
    let Ok((player, tf)) = player_q.single() else { return };
    let Ok(mut cam_tf) = camera_q.single_mut() else { return };

    let up = tf.translation.normalize();
    let feet = tf.translation;

    cam_tf.translation = feet + up * CAMERA_HEIGHT - player.heading * CAMERA_BACK;
    let look_target = feet + player.heading * CAMERA_LOOK_AHEAD;
    let t = 1.0 - (-6.0 * time.delta_secs()).exp();
    let current_look = cam_tf.rotation * -Vec3::Z + cam_tf.translation;
    cam_tf.look_at(current_look.lerp(look_target, t), up);

    if let Ok(mut light_tf) = light_q.single_mut() {
        light_tf.translation = feet + up * 800.0;
        light_tf.look_at(feet, player.heading);
    }
}
