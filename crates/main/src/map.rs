use avian3d::prelude::*;
use bevy::prelude::*;
use bevy::render::mesh::VertexAttributeValues;
use crate::physics::RadialGravity;
use shared::level::{FaceTags, LevelData, FLORA_BUSH, FLORA_FLOWER, FLORA_GRASS, FLORA_ROCK, FLORA_TREE, LEVEL_FORMAT_VERSION};
use shared::sphere::PLANET_RADIUS;
use shared::terrain::{TerrainGen, MAX_MOUNTAIN};
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
pub struct RegionMarker {
    pub name: String,
    pub kind: String,
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
pub struct LevelTags(pub FaceTags);

#[derive(Resource)]
pub struct LevelFaceTypes(pub Vec<shared::terrain::Terrain>);

/// Region names + per-face region ids for the HUD.
#[derive(Resource)]
pub struct LevelRegions {
    pub regions: Vec<shared::level::RegionData>,
    pub face_region: Vec<u32>,
}

/// Blend-marked boundary faces: the pair of terrain kinds each links.
#[derive(Resource)]
pub struct LevelBlends(pub std::collections::BTreeMap<u32, (u8, u8)>);

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
    assert_eq!(
        level.version, LEVEL_FORMAT_VERSION,
        "stale level binary (format {}, expected {LEVEL_FORMAT_VERSION}) — re-run gen_level",
        level.version,
    );

    commands.insert_resource(PlayerHp(PLAYER_HP));
    // The level carries the SOLVED elevation field the mesh was baked from, so
    // height queries and the rendered surface agree exactly (and startup skips
    // all topology planning).
    let terrain = TerrainGen::from_field(level.seed, level.vert_elev.clone());

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



    // Flora: instanced low-poly props on the baked positions (they sit exactly
    // on the displaced mesh). Shared mesh/material handles keep this cheap;
    // per-instance scale/yaw variety comes from hashing the position bits.
    {
        let trunk_mesh = meshes.add(Cylinder::new(0.25, 4.5));
        let canopy_mesh = meshes.add(Sphere::new(2.0));
        let bush_mesh = meshes.add(Sphere::new(0.7));
        let rock_mesh = meshes.add(Sphere::new(0.6).mesh().ico(1).unwrap());
        let grass_mesh = meshes.add(Cone { radius: 0.22, height: 0.55 });
        let flower_mesh = meshes.add(Sphere::new(0.16));
        let trunk_mat = materials.add(StandardMaterial::from_color(Color::srgb(0.42, 0.30, 0.18)));
        let canopy_mats = [
            materials.add(StandardMaterial::from_color(Color::srgb(0.13, 0.34, 0.16))),
            materials.add(StandardMaterial::from_color(Color::srgb(0.18, 0.42, 0.18))),
            materials.add(StandardMaterial::from_color(Color::srgb(0.24, 0.45, 0.14))),
        ];
        let bush_mat = materials.add(StandardMaterial::from_color(Color::srgb(0.22, 0.40, 0.20)));
        let rock_mats = [
            materials.add(StandardMaterial::from_color(Color::srgb(0.45, 0.44, 0.42))),
            materials.add(StandardMaterial::from_color(Color::srgb(0.55, 0.53, 0.50))),
        ];
        let grass_mat = materials.add(StandardMaterial::from_color(Color::srgb(0.38, 0.52, 0.22)));
        let flower_mats = [
            materials.add(StandardMaterial::from_color(Color::srgb(0.90, 0.25, 0.30))),
            materials.add(StandardMaterial::from_color(Color::srgb(0.95, 0.80, 0.25))),
            materials.add(StandardMaterial::from_color(Color::srgb(0.75, 0.45, 0.90))),
            materials.add(StandardMaterial::from_color(Color::srgb(0.95, 0.95, 0.95))),
        ];
        for f in &level.flora {
            let pos = Vec3::from_array(f.pos);
            let up = pos.normalize();
            let h = f.pos[0].to_bits() ^ f.pos[1].to_bits().rotate_left(13) ^ f.pos[2].to_bits().rotate_left(27);
            let scale = 0.7 + (h & 0xff) as f32 / 255.0 * 0.6;
            let yaw = (h >> 8 & 0xff) as f32 / 255.0 * std::f32::consts::TAU;
            let rot = Quat::from_rotation_arc(Vec3::Y, up) * Quat::from_rotation_y(yaw);
            match f.kind {
                FLORA_TREE => {
                    commands.spawn((
                        Mesh3d(trunk_mesh.clone()),
                        MeshMaterial3d(trunk_mat.clone()),
                        RigidBody::Static,
                        ColliderConstructor::Cylinder { radius: 0.3 * scale, height: 4.5 * scale },
                        Transform::from_translation(pos + up * 2.25 * scale)
                            .with_rotation(rot)
                            .with_scale(Vec3::splat(scale)),
                    ));
                    commands.spawn((
                        Mesh3d(canopy_mesh.clone()),
                        MeshMaterial3d(canopy_mats[(h >> 16) as usize % canopy_mats.len()].clone()),
                        Transform::from_translation(pos + up * 5.2 * scale)
                            .with_rotation(rot)
                            .with_scale(Vec3::new(scale, scale * 1.25, scale)),
                    ));
                }
                FLORA_BUSH => {
                    commands.spawn((
                        Mesh3d(bush_mesh.clone()),
                        MeshMaterial3d(bush_mat.clone()),
                        Transform::from_translation(pos + up * 0.45 * scale)
                            .with_rotation(rot)
                            .with_scale(Vec3::new(scale, scale * 0.75, scale)),
                    ));
                }
                FLORA_FLOWER => {
                    commands.spawn((
                        Mesh3d(flower_mesh.clone()),
                        MeshMaterial3d(flower_mats[(h >> 16) as usize % flower_mats.len()].clone()),
                        Transform::from_translation(pos + up * 0.22)
                            .with_rotation(rot)
                            .with_scale(Vec3::splat(scale)),
                    ));
                }
                FLORA_ROCK => {
                    // Irregular via non-uniform scale; boulders block movement.
                    let sx = 0.8 + (h >> 24 & 0x7) as f32 / 7.0 * 0.7;
                    let sz = 0.8 + (h >> 27 & 0x7) as f32 / 7.0 * 0.7;
                    let boulder = scale > 1.05;
                    let size = if boulder { scale * 1.8 } else { scale };
                    let mut e = commands.spawn((
                        Mesh3d(rock_mesh.clone()),
                        MeshMaterial3d(rock_mats[(h >> 16) as usize % rock_mats.len()].clone()),
                        Transform::from_translation(pos + up * 0.25 * size)
                            .with_rotation(rot)
                            .with_scale(Vec3::new(size * sx, size * 0.7, size * sz)),
                    ));
                    if boulder {
                        e.insert((
                            RigidBody::Static,
                            ColliderConstructor::Sphere { radius: 0.55 },
                        ));
                    }
                }
                FLORA_GRASS => {
                    commands.spawn((
                        Mesh3d(grass_mesh.clone()),
                        MeshMaterial3d(grass_mat.clone()),
                        Transform::from_translation(pos + up * 0.18 * scale)
                            .with_rotation(rot)
                            .with_scale(Vec3::splat(scale)),
                    ));
                }
                _ => {}
            }
        }
    }

    // Bridges: entities built at runtime from the recorded spans, like any building.
    // Deck heights come from the displaced terrain mesh (the surface that renders
    // and collides), so deck ends meet the actual ground — including lifted cliffs.
    let displaced: Vec<[Vec3; 3]> = level.terrain_tris.iter()
        .map(|t| [Vec3::from_array(t[0]), Vec3::from_array(t[1]), Vec3::from_array(t[2])])
        .collect();
    let ground = PlanetMesh::new(displaced);
    let bridge_color = Color::srgb(0.35, 0.25, 0.18).to_linear();
    for road in level.roads.iter().filter(|r| r.is_bridge) {
        let span: Vec<shared::sphere::SpherePos> = road.points.iter()
            .map(|p| shared::sphere::SpherePos::new(Vec3::from_array(*p)))
            .collect();
        let deck = shared::roads::build_bridge_deck(&span, &ground, 20.0);
        if deck.is_empty() {
            continue;
        }
        let colors = vec![[bridge_color.to_f32_array(); 3]; deck.len()];
        commands.spawn((
            Mesh3d(meshes.add(build_visual_mesh(&deck, &colors))),
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
            build_collider(&deck),
            Transform::default(),
            Ground,
        ));
    }

    // Water
    commands.spawn((
        Mesh3d(meshes.add(Sphere::new(PLANET_RADIUS - 0.5))),
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

    // Player: spawn at the first settlement, well above the mesh.
    let s = level.settlements.first().expect("no settlements");
    let start = shared::sphere::SpherePos::new(Vec3::from_array(s.pos));
    let up = start.0;
    let capsule_radius = PLAYER_SIZE * 0.4;
    let capsule_half = capsule_radius + PLAYER_SIZE * 0.5;
    // Spawn high above terrain — gravity settles onto the collider.
    let spawn_r = PLANET_RADIUS + MAX_MOUNTAIN + 50.0;
    let spawn_pos = up * spawn_r;
    commands.spawn((
        Mesh3d(meshes.add(Capsule3d::new(capsule_radius, PLAYER_SIZE))),
        MeshMaterial3d(materials.add(StandardMaterial::from_color(PLAYER_COLOR))),
        RigidBody::Dynamic,
        ColliderConstructor::Sphere { radius: PLAYER_SIZE * 0.5 },
        // Swept CCD: thin trimesh colliders (terrain, bridge decks) must not be
        // tunneled through during fast falls.
        SweptCcd::default(),
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

    // Region markers (oceans, beaches, forests, …)
    for region in &level.regions {
        let pos = Vec3::from_array(region.pos) * PLANET_RADIUS;
        commands.spawn((
            Transform::from_translation(pos),
            RegionMarker { name: region.name.clone(), kind: format!("{:?}", region.kind) },
        ));
    }
    let tris: Vec<[Vec3; 3]> = level.unit_tris.iter()
        .map(|t| [Vec3::from_array(t[0]), Vec3::from_array(t[1]), Vec3::from_array(t[2])])
        .collect();
    let planet_mesh = PlanetMesh::new(tris);
    let tags = FaceTags { off: level.face_tag_off.clone(), data: level.face_tag_data.clone() };
    let face_types: Vec<shared::terrain::Terrain> = level.face_types.iter()
        .map(|&b| unsafe { std::mem::transmute(b) })
        .collect();
    commands.insert_resource(terrain);
    commands.insert_resource(planet_mesh);
    commands.insert_resource(LevelTags(tags));
    commands.insert_resource(LevelFaceTypes(face_types));
    commands.insert_resource(LevelRegions {
        regions: level.regions.clone(),
        face_region: level.face_region.clone(),
    });
    commands.insert_resource(LevelBlends(
        level.face_blend.iter().map(|&(fi, a, b)| (fi, (a, b))).collect(),
    ));
}

fn build_visual_mesh(tris: &[[[f32; 3]; 3]], colors: &[[[f32; 4]; 3]]) -> Mesh {
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
        colors_out.push(color[0]);
        colors_out.push(color[1]);
        colors_out.push(color[2]);
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
