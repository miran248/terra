use avian3d::prelude::*;
use bevy::prelude::*;
use bevy::render::mesh::VertexAttributeValues;
use crate::physics::RadialGravity;
use shared::level::{
    FaceTags, LevelData, LEVEL_FORMAT_VERSION,
    FLORA_BUSH, FLORA_FLOWER, FLORA_GRASS, FLORA_ROCK, FLORA_TREE,
    FLORA_BERRY, FLORA_CACTUS, FLORA_DEADTREE, FLORA_LOG, FLORA_MUSHROOM, FLORA_REED,
    STRUCT_CAMPFIRE, STRUCT_DOCK, STRUCT_FARM, STRUCT_RUIN, STRUCT_WALL,
    STRUCT_WATCHTOWER, STRUCT_WELL,
};
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

/// Debug teleport ring: named landmarks (bridges, towns, one per named
/// region) the player can jump between. `[` / `]` cycle, the current name is
/// logged. Each target is the WORLD position to drop the player at (already
/// lifted above the ground or the bridge deck).
#[derive(Resource, Default)]
pub struct Teleports {
    pub targets: Vec<(String, Vec3)>,
    pub idx: usize,
}

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
                (orient_player, camera_follow, teleport_player)
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

    let spawn_surface_r = terrain
        .surface_radius(shared::sphere::SpherePos::new(Vec3::from_array(
            level.settlements.first().expect("no settlements").pos,
        )));

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
        let log_mesh = meshes.add(Cylinder::new(0.35, 3.0));
        let log_mat = materials.add(StandardMaterial::from_color(Color::srgb(0.36, 0.26, 0.16)));
        let mush_mesh = meshes.add(Sphere::new(0.18));
        let mush_mat = materials.add(StandardMaterial::from_color(Color::srgb(0.80, 0.35, 0.30)));
        let cactus_mesh = meshes.add(Capsule3d::new(0.30, 1.6));
        let cactus_mat = materials.add(StandardMaterial::from_color(Color::srgb(0.28, 0.45, 0.24)));
        let berry_mesh = meshes.add(Sphere::new(0.6));
        let berry_mat = materials.add(StandardMaterial::from_color(Color::srgb(0.30, 0.20, 0.35)));
        let dead_mesh = meshes.add(Cylinder::new(0.22, 4.0));
        let dead_mat = materials.add(StandardMaterial::from_color(Color::srgb(0.34, 0.30, 0.24)));
        let reed_mesh = meshes.add(Cone { radius: 0.10, height: 1.3 });
        let reed_mat = materials.add(StandardMaterial::from_color(Color::srgb(0.55, 0.58, 0.30)));
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
                FLORA_LOG => {
                    // Fallen: lie along the ground (trunk axis tangent to up).
                    let lie = rot * Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
                    commands.spawn((
                        Mesh3d(log_mesh.clone()),
                        MeshMaterial3d(log_mat.clone()),
                        RigidBody::Static,
                        ColliderConstructor::Cylinder { radius: 0.35 * scale, height: 3.0 * scale },
                        Transform::from_translation(pos + up * 0.35 * scale)
                            .with_rotation(lie)
                            .with_scale(Vec3::splat(scale)),
                    ));
                }
                FLORA_MUSHROOM => {
                    commands.spawn((
                        Mesh3d(mush_mesh.clone()),
                        MeshMaterial3d(mush_mat.clone()),
                        Transform::from_translation(pos + up * 0.16 * scale)
                            .with_rotation(rot)
                            .with_scale(Vec3::new(scale, scale * 0.7, scale)),
                    ));
                }
                FLORA_CACTUS => {
                    commands.spawn((
                        Mesh3d(cactus_mesh.clone()),
                        MeshMaterial3d(cactus_mat.clone()),
                        RigidBody::Static,
                        ColliderConstructor::Capsule { radius: 0.30 * scale, height: 1.6 * scale },
                        Transform::from_translation(pos + up * 0.9 * scale)
                            .with_rotation(rot)
                            .with_scale(Vec3::splat(scale)),
                    ));
                }
                FLORA_BERRY => {
                    commands.spawn((
                        Mesh3d(berry_mesh.clone()),
                        MeshMaterial3d(berry_mat.clone()),
                        Transform::from_translation(pos + up * 0.4 * scale)
                            .with_rotation(rot)
                            .with_scale(Vec3::new(scale, scale * 0.85, scale)),
                    ));
                }
                FLORA_DEADTREE => {
                    commands.spawn((
                        Mesh3d(dead_mesh.clone()),
                        MeshMaterial3d(dead_mat.clone()),
                        RigidBody::Static,
                        ColliderConstructor::Cylinder { radius: 0.25 * scale, height: 4.0 * scale },
                        Transform::from_translation(pos + up * 2.0 * scale)
                            .with_rotation(rot)
                            .with_scale(Vec3::splat(scale)),
                    ));
                }
                FLORA_REED => {
                    commands.spawn((
                        Mesh3d(reed_mesh.clone()),
                        MeshMaterial3d(reed_mat.clone()),
                        Transform::from_translation(pos + up * 0.65 * scale)
                            .with_rotation(rot)
                            .with_scale(Vec3::splat(scale)),
                    ));
                }
                _ => {}
            }
        }
    }

    // Structures: contextual built props (wells, docks, walls, watchtowers,
    // ruins, farms, campfires), spawned from baked positions like bridges.
    {
        let stone = materials.add(StandardMaterial::from_color(Color::srgb(0.55, 0.53, 0.50)));
        let dark_stone = materials.add(StandardMaterial::from_color(Color::srgb(0.40, 0.38, 0.36)));
        let wood = materials.add(StandardMaterial::from_color(Color::srgb(0.45, 0.32, 0.20)));
        let plank = materials.add(StandardMaterial::from_color(Color::srgb(0.52, 0.40, 0.26)));
        let soil = materials.add(StandardMaterial::from_color(Color::srgb(0.34, 0.24, 0.15)));
        let ember = materials.add(StandardMaterial {
            base_color: Color::srgb(0.9, 0.4, 0.1),
            emissive: LinearRgba::rgb(0.9, 0.35, 0.05),
            ..default()
        });
        let box_mesh = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
        let cyl_mesh = meshes.add(Cylinder::new(0.5, 1.0));
        for st in &level.structures {
            let pos = Vec3::from_array(st.pos);
            let up = pos.normalize();
            let base = Quat::from_rotation_arc(Vec3::Y, up) * Quat::from_rotation_y(st.yaw);
            let mut at = |cmd: &mut Commands, mesh: Handle<Mesh>, mat: Handle<StandardMaterial>,
                          lift: f32, scale: Vec3, collide: bool| {
                let mut e = cmd.spawn((
                    Mesh3d(mesh),
                    MeshMaterial3d(mat),
                    Transform::from_translation(pos + up * lift)
                        .with_rotation(base)
                        .with_scale(scale),
                ));
                if collide {
                    e.insert((RigidBody::Static, ColliderConstructor::ConvexHullFromMesh));
                }
            };
            match st.kind {
                STRUCT_WELL => {
                    at(&mut commands, cyl_mesh.clone(), stone.clone(), 0.6, Vec3::new(2.0, 1.2, 2.0), true);
                }
                STRUCT_CAMPFIRE => {
                    at(&mut commands, cyl_mesh.clone(), dark_stone.clone(), 0.2, Vec3::new(1.6, 0.4, 1.6), false);
                    at(&mut commands, box_mesh.clone(), ember.clone(), 0.5, Vec3::splat(0.7), false);
                }
                STRUCT_WALL => {
                    at(&mut commands, box_mesh.clone(), dark_stone.clone(), 1.5, Vec3::new(6.0, 3.0, 1.2), true);
                }
                STRUCT_DOCK => {
                    at(&mut commands, box_mesh.clone(), plank.clone(), 0.4, Vec3::new(3.0, 0.4, 10.0), true);
                }
                STRUCT_FARM => {
                    at(&mut commands, box_mesh.clone(), soil.clone(), 0.1, Vec3::new(9.0, 0.2, 9.0), false);
                }
                STRUCT_WATCHTOWER => {
                    at(&mut commands, box_mesh.clone(), wood.clone(), 6.0, Vec3::new(3.0, 12.0, 3.0), true);
                    at(&mut commands, box_mesh.clone(), plank.clone(), 12.5, Vec3::new(4.5, 1.0, 4.5), true);
                }
                STRUCT_RUIN => {
                    // A broken ring of stub columns.
                    for k in 0..5 {
                        let a = k as f32 / 5.0 * std::f32::consts::TAU;
                        let (east, north) = shared::sphere::SpherePos::new(up).tangent_basis();
                        let off = (east * a.cos() + north * a.sin()) * 3.0;
                        let cpos = pos + off;
                        let cup = cpos.normalize();
                        commands.spawn((
                            Mesh3d(box_mesh.clone()),
                            MeshMaterial3d(stone.clone()),
                            RigidBody::Static,
                            ColliderConstructor::ConvexHullFromMesh,
                            Transform::from_translation(cpos + cup * (1.0 + k as f32 % 2.0))
                                .with_rotation(Quat::from_rotation_arc(Vec3::Y, cup))
                                .with_scale(Vec3::new(1.0, 2.0 + (k % 3) as f32, 1.0)),
                        ));
                    }
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

    // Player: spawn ON the ground at the first settlement.
    let s = level.settlements.first().expect("no settlements");
    let start = shared::sphere::SpherePos::new(Vec3::from_array(s.pos));
    let up = start.0;
    let capsule_radius = PLAYER_SIZE * 0.4;
    let capsule_half = capsule_radius + PLAYER_SIZE * 0.5;
    let spawn_pos = up * (spawn_surface_r + capsule_half + 0.5);
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
    // Teleport ring: bridges first (hard to find on foot), then towns, then
    // one landmark per named region.
    let mut targets: Vec<(String, Vec3)> = Vec::new();
    // Drop just above the SOLVED surface (works at any altitude — inland
    // river bridges sit high, so a fixed sea-level height would be
    // underground).
    let surface = |p: [f32; 3]| {
        let dir = Vec3::from_array(p).normalize();
        let pos = shared::sphere::SpherePos::new(dir);
        dir * (terrain.surface_radius(pos) + 3.0)
    };
    // Bridges: spawn at the on-land entry (the deck's grounded end) so the
    // player arrives beside the bridge on solid ground, not over the water.
    for (i, road) in level.roads.iter().filter(|r| r.is_bridge).enumerate() {
        if let Some(end) = road.points.first() {
            targets.push((format!("Bridge {}", i + 1), surface(*end)));
        }
    }
    for s in &level.settlements {
        targets.push((s.name.clone(), surface(s.pos)));
    }
    for r in &level.regions {
        targets.push((r.name.clone(), surface(r.pos)));
    }
    commands.insert_resource(Teleports { targets, idx: 0 });
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
    // Shared corners get one vertex — the subdivided mesh would otherwise
    // triple the collider's vertex count.
    let mut map: std::collections::HashMap<[u32; 3], u32> = std::collections::HashMap::new();
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for tri in tris {
        let mut idx = [0u32; 3];
        for (k, v) in tri.iter().enumerate() {
            let key = [v[0].to_bits(), v[1].to_bits(), v[2].to_bits()];
            idx[k] = *map.entry(key).or_insert_with(|| {
                vertices.push(Vec3::from_array(*v));
                vertices.len() as u32 - 1
            });
        }
        indices.push(idx);
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

/// `[` / `]` step through the teleport ring; `T` jumps to the next bridge.
/// The player is placed just above the ground at the target and its velocity
/// zeroed so it settles cleanly instead of launching.
fn teleport_player(
    keys: Res<ButtonInput<KeyCode>>,
    mut tp: ResMut<Teleports>,
    mut q: Query<(&mut Transform, &mut LinearVelocity, &mut shared::sphere::SpherePos), With<Player>>,
) {
    if tp.targets.is_empty() {
        return;
    }
    let n = tp.targets.len();
    let step = if keys.just_pressed(KeyCode::BracketRight) {
        1
    } else if keys.just_pressed(KeyCode::BracketLeft) {
        n - 1
    } else if keys.just_pressed(KeyCode::KeyT) {
        // Jump to the next bridge in the ring (bridges are named "Bridge N").
        let mut k = 1;
        while k <= n && !tp.targets[(tp.idx + k) % n].0.starts_with("Bridge") {
            k += 1;
        }
        k % n
    } else {
        return;
    };
    tp.idx = (tp.idx + step) % n;
    let (name, world) = tp.targets[tp.idx].clone();
    let Ok((mut tf, mut vel, mut sp)) = q.single_mut() else { return };
    tf.translation = world;
    *sp = shared::sphere::SpherePos::new(world.normalize());
    vel.0 = Vec3::ZERO;
    info!("Teleported to {name} ({}/{n})", tp.idx + 1);
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
