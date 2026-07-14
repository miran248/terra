use crate::constants::*;
use crate::minimap::MinimapCamera;
use crate::physics::RadialGravity;
use avian3d::prelude::*;
use bevy::prelude::*;
use bevy::render::mesh::VertexAttributeValues;
use shared::level::{
    FLORA_BERRY, FLORA_BUSH, FLORA_CACTUS, FLORA_DEADTREE, FLORA_FLOWER, FLORA_GRASS, FLORA_LOG,
    FLORA_MUSHROOM, FLORA_REED, FLORA_ROCK, FLORA_TREE, FaceTags, LEVEL_FORMAT_VERSION, LevelData,
    STRUCT_CAMPFIRE, STRUCT_DOCK, STRUCT_FARM, STRUCT_RUIN, STRUCT_WALL, STRUCT_WATCHTOWER,
    STRUCT_WELL,
};
use shared::planet::PlanetMesh;
use shared::sphere::PLANET_RADIUS;
use shared::terrain::TerrainGen;

/// Speculative collision skin added to the thin terrain trimesh so fast movement
/// doesn't tunnel through it. The player mesh is dropped by this much to hide the
/// float it would otherwise cause.
const TERRAIN_MARGIN: f32 = 0.5;

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

#[derive(Resource)]
pub struct LevelSlope(pub Vec<u8>);

#[derive(Resource)]
pub struct LevelWaterDepth(pub Vec<u8>);

#[derive(Resource)]
pub struct LevelLandform(pub Vec<u8>);

#[derive(Resource)]
pub struct LevelRoadMaterial(pub Vec<u8>);

/// Distance-based render culling for scatter props: beyond `0` metres the
/// entity is hidden. Tiny ground cover culls close, trees stay visible far.
#[derive(Component, Copy, Clone)]
pub struct CullRange(pub f32);

/// Per-flora-kind cull distance (metres) — the smaller the prop, the sooner it
/// stops being drawn in the distance.
fn flora_cull(kind: u8) -> f32 {
    use shared::level::*;
    // ~1.6x the original ranges so props stay visible further out; the camera
    // fog visibility (main.rs) is set beyond the largest of these so props fade
    // into haze before this hard cull edge rather than popping.
    match kind {
        FLORA_FLOWER | FLORA_GRASS | FLORA_MUSHROOM | FLORA_REED => 150.0,
        FLORA_BUSH | FLORA_BERRY | FLORA_CACTUS | FLORA_ROCK => 300.0,
        FLORA_LOG => 420.0,
        FLORA_TREE | FLORA_DEADTREE => 880.0,
        _ => 480.0,
    }
}

/// Hide props past their cull range from the camera (throttled — the set only
/// changes as the player moves). Frustum culling is on top of this, in Bevy.
fn cull_props(
    time: Res<Time>,
    mut timer: Local<f32>,
    camera: Query<&Transform, With<MainCamera>>,
    mut props: Query<(&CullRange, &GlobalTransform, &mut Visibility)>,
) {
    *timer += time.delta_secs();
    if *timer < 0.2 {
        return;
    }
    *timer = 0.0;
    let Ok(cam) = camera.single() else { return };
    let eye = cam.translation;
    for (range, tf, mut vis) in &mut props {
        let far = tf.translation().distance_squared(eye) > range.0 * range.0;
        let want = if far {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
        if *vis != want {
            *vis = want;
        }
    }
}

/// Region names + per-face region ids for the HUD.
#[derive(Resource)]
pub struct LevelRegions {
    pub regions: Vec<shared::level::RegionData>,
    pub face_region: Vec<u32>,
}

/// Blend-marked boundary faces: the pair of terrain kinds each links.
#[derive(Resource)]
pub struct LevelBlends(pub std::collections::BTreeMap<u32, (u8, u8)>);

/// Drives the world sun (see `drive_daynight`). The sun orbits the planet's Y
/// axis, so day and night are real hemispheres: `angle` is the sun's longitude
/// (advances over `day_length` seconds), and `sun_dir` is the resulting
/// world-space direction toward the sun, cached for solar-time queries.
#[derive(Resource)]
pub struct TimeOfDay {
    pub angle: f32,
    pub day_length: f32,
    pub sun_dir: Vec3,
    pub day: u32,
}

impl Default for TimeOfDay {
    fn default() -> Self {
        Self {
            angle: 0.0,
            day_length: 240.0,
            sun_dir: Vec3::X,
            day: 1,
        }
    }
}

impl TimeOfDay {
    /// Local solar time (hours, 0..24) at a point with the given surface `up`.
    /// Noon is when the sun is highest over that point; midnight the opposite.
    /// Advances forward as the sun orbits.
    pub fn local_hours(&self, up: Vec3) -> f32 {
        let sun_long = self.sun_dir.z.atan2(self.sun_dir.x);
        let point_long = up.z.atan2(up.x);
        let hour_angle = (sun_long - point_long).rem_euclid(std::f32::consts::TAU);
        (12.0 + hour_angle * (24.0 / std::f32::consts::TAU)).rem_euclid(24.0)
    }
}

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
            .init_resource::<TimeOfDay>()
            .init_resource::<SunLock>()
            .add_systems(OnEnter(AppState::Playing), setup_map)
            .add_systems(
                Update,
                read_player_input.run_if(in_state(AppState::Playing)),
            )
            .add_systems(FixedUpdate, move_player.run_if(in_state(AppState::Playing)))
            .add_systems(
                Update,
                (
                    orient_player,
                    camera_follow,
                    drive_daynight,
                    drive_fog,
                    toggle_sun_lock,
                    cull_props,
                )
                    .run_if(in_state(AppState::Playing)),
            );
    }
}

use shared::state::AppState;

fn setup_map(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut water_mats: ResMut<Assets<crate::water::WaterMaterial>>,
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

    let spawn_surface_r = terrain.surface_radius(shared::sphere::SpherePos::new(Vec3::from_array(
        level.settlements.first().expect("no settlements").pos,
    )));

    // Terrain layer
    let terrain_mesh = build_visual_mesh(&level.terrain_tris, &level.terrain_colors);
    commands.spawn((
        Mesh3d(meshes.add(terrain_mesh)),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 0.95,
            // The sun is re-aimed at the player's feet each frame, so its specular
            // hotspot rides with the player; on the flat-shaded terrain that broad
            // highlight aliases into moving grain. Near-zero reflectance removes
            // the specular lobe (matte terrain) and kills the sparkle.
            reflectance: 0.02,
            ..default()
        })),
        Transform::default(),
        Ground,
    ));
    commands.spawn((
        RigidBody::Static,
        build_collider(&level.terrain_tris),
        // The terrain is an infinitely-thin trimesh, so fast/unlucky movement can
        // tunnel through it (random falls through the map). A collision margin
        // gives the surface thickness — a speculative skin that catches movers
        // before they pass through. The player mesh is lowered by the same amount
        // (see the player spawn) to hide the resulting float.
        CollisionMargin(TERRAIN_MARGIN),
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
        let grass_mesh = meshes.add(Cone {
            radius: 0.22,
            height: 0.55,
        });
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
        let reed_mesh = meshes.add(Cone {
            radius: 0.10,
            height: 1.3,
        });
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
            let h = f.pos[0].to_bits()
                ^ f.pos[1].to_bits().rotate_left(13)
                ^ f.pos[2].to_bits().rotate_left(27);
            let scale = 0.7 + (h & 0xff) as f32 / 255.0 * 0.6;
            let yaw = (h >> 8 & 0xff) as f32 / 255.0 * std::f32::consts::TAU;
            let rot = Quat::from_rotation_arc(Vec3::Y, up) * Quat::from_rotation_y(yaw);
            let cull = flora_cull(f.kind);
            match f.kind {
                FLORA_TREE => {
                    commands.spawn((
                        CullRange(cull),
                        Mesh3d(trunk_mesh.clone()),
                        MeshMaterial3d(trunk_mat.clone()),
                        RigidBody::Static,
                        ColliderConstructor::Cylinder {
                            radius: 0.3 * scale,
                            height: 4.5 * scale,
                        },
                        Transform::from_translation(pos + up * 2.25 * scale)
                            .with_rotation(rot)
                            .with_scale(Vec3::splat(scale)),
                    ));
                    commands.spawn((
                        CullRange(cull),
                        Mesh3d(canopy_mesh.clone()),
                        MeshMaterial3d(canopy_mats[(h >> 16) as usize % canopy_mats.len()].clone()),
                        Transform::from_translation(pos + up * 5.2 * scale)
                            .with_rotation(rot)
                            .with_scale(Vec3::new(scale, scale * 1.25, scale)),
                    ));
                }
                FLORA_BUSH => {
                    commands.spawn((
                        CullRange(cull),
                        Mesh3d(bush_mesh.clone()),
                        MeshMaterial3d(bush_mat.clone()),
                        Transform::from_translation(pos + up * 0.45 * scale)
                            .with_rotation(rot)
                            .with_scale(Vec3::new(scale, scale * 0.75, scale)),
                    ));
                }
                FLORA_FLOWER => {
                    commands.spawn((
                        CullRange(cull),
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
                        CullRange(cull),
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
                        CullRange(cull),
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
                        CullRange(cull),
                        Mesh3d(log_mesh.clone()),
                        MeshMaterial3d(log_mat.clone()),
                        RigidBody::Static,
                        ColliderConstructor::Cylinder {
                            radius: 0.35 * scale,
                            height: 3.0 * scale,
                        },
                        Transform::from_translation(pos + up * 0.35 * scale)
                            .with_rotation(lie)
                            .with_scale(Vec3::splat(scale)),
                    ));
                }
                FLORA_MUSHROOM => {
                    commands.spawn((
                        CullRange(cull),
                        Mesh3d(mush_mesh.clone()),
                        MeshMaterial3d(mush_mat.clone()),
                        Transform::from_translation(pos + up * 0.16 * scale)
                            .with_rotation(rot)
                            .with_scale(Vec3::new(scale, scale * 0.7, scale)),
                    ));
                }
                FLORA_CACTUS => {
                    commands.spawn((
                        CullRange(cull),
                        Mesh3d(cactus_mesh.clone()),
                        MeshMaterial3d(cactus_mat.clone()),
                        RigidBody::Static,
                        ColliderConstructor::Capsule {
                            radius: 0.30 * scale,
                            height: 1.6 * scale,
                        },
                        Transform::from_translation(pos + up * 0.9 * scale)
                            .with_rotation(rot)
                            .with_scale(Vec3::splat(scale)),
                    ));
                }
                FLORA_BERRY => {
                    commands.spawn((
                        CullRange(cull),
                        Mesh3d(berry_mesh.clone()),
                        MeshMaterial3d(berry_mat.clone()),
                        Transform::from_translation(pos + up * 0.4 * scale)
                            .with_rotation(rot)
                            .with_scale(Vec3::new(scale, scale * 0.85, scale)),
                    ));
                }
                FLORA_DEADTREE => {
                    commands.spawn((
                        CullRange(cull),
                        Mesh3d(dead_mesh.clone()),
                        MeshMaterial3d(dead_mat.clone()),
                        RigidBody::Static,
                        ColliderConstructor::Cylinder {
                            radius: 0.25 * scale,
                            height: 4.0 * scale,
                        },
                        Transform::from_translation(pos + up * 2.0 * scale)
                            .with_rotation(rot)
                            .with_scale(Vec3::splat(scale)),
                    ));
                }
                FLORA_REED => {
                    commands.spawn((
                        CullRange(cull),
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
            let at = |cmd: &mut Commands,
                      mesh: Handle<Mesh>,
                      mat: Handle<StandardMaterial>,
                      lift: f32,
                      scale: Vec3,
                      collide: bool| {
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
                    at(
                        &mut commands,
                        cyl_mesh.clone(),
                        stone.clone(),
                        0.6,
                        Vec3::new(2.0, 1.2, 2.0),
                        true,
                    );
                }
                STRUCT_CAMPFIRE => {
                    at(
                        &mut commands,
                        cyl_mesh.clone(),
                        dark_stone.clone(),
                        0.2,
                        Vec3::new(1.6, 0.4, 1.6),
                        false,
                    );
                    at(
                        &mut commands,
                        box_mesh.clone(),
                        ember.clone(),
                        0.5,
                        Vec3::splat(0.7),
                        false,
                    );
                }
                STRUCT_WALL => {
                    at(
                        &mut commands,
                        box_mesh.clone(),
                        dark_stone.clone(),
                        1.5,
                        Vec3::new(6.0, 3.0, 1.2),
                        true,
                    );
                }
                STRUCT_DOCK => {
                    at(
                        &mut commands,
                        box_mesh.clone(),
                        plank.clone(),
                        0.4,
                        Vec3::new(3.0, 0.4, 10.0),
                        true,
                    );
                }
                STRUCT_FARM => {
                    at(
                        &mut commands,
                        box_mesh.clone(),
                        soil.clone(),
                        0.1,
                        Vec3::new(9.0, 0.2, 9.0),
                        false,
                    );
                }
                STRUCT_WATCHTOWER => {
                    at(
                        &mut commands,
                        box_mesh.clone(),
                        wood.clone(),
                        6.0,
                        Vec3::new(3.0, 12.0, 3.0),
                        true,
                    );
                    at(
                        &mut commands,
                        box_mesh.clone(),
                        plank.clone(),
                        12.5,
                        Vec3::new(4.5, 1.0, 4.5),
                        true,
                    );
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
    let displaced: Vec<[Vec3; 3]> = level
        .terrain_tris
        .iter()
        .map(|t| {
            [
                Vec3::from_array(t[0]),
                Vec3::from_array(t[1]),
                Vec3::from_array(t[2]),
            ]
        })
        .collect();
    let ground = PlanetMesh::new(displaced);
    let bridge_color = Color::srgb(0.35, 0.25, 0.18).to_linear();
    for road in level.roads.iter().filter(|r| r.is_bridge) {
        let span: Vec<shared::sphere::SpherePos> = road
            .points
            .iter()
            .map(|p| shared::sphere::SpherePos::new(Vec3::from_array(*p)))
            .collect();
        let deck = shared::roads::build_bridge_deck(&span, &ground, 4.0);
        if deck.is_empty() {
            continue;
        }
        let colors = vec![[bridge_color.to_f32_array(); 3]; deck.len()];
        commands.spawn((
            Mesh3d(meshes.add(build_smooth_mesh(&deck, &colors))),
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

    // Water (see crate::water): sea + lakes are one mesh, built from the
    // gen-time per-face waterline (`face_water_r`) which clusters water into
    // connected bodies — so the sea can't flood an inland lake basin and lakes
    // can't spill onto land. Rivers are their own flowing mesh.
    let water_mat = water_mats.add(crate::water::water_material());
    if let Some(water) = crate::water::build_water_surface(&level.terrain_tris, &level.face_water_r)
    {
        commands.spawn((
            Mesh3d(meshes.add(water)),
            MeshMaterial3d(water_mat.clone()),
            Transform::default(),
            Ground,
        ));
    }

    // River surfaces use the generator-baked, smoothed corner radii so runtime
    // rendering has no topology/clustering work and cannot introduce seams.
    if let Some(river) =
        crate::water::build_river_surfaces(&level.terrain_tris, &level.face_river_r)
    {
        commands.spawn((
            Mesh3d(meshes.add(river)),
            MeshMaterial3d(water_mats.add(crate::water::river_material())),
            Transform::default(),
            Ground,
        ));
    }

    // Sun
    commands.spawn((
        DirectionalLight {
            illuminance: 8_000.0,
            ..default()
        },
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
    commands
        .spawn((
            RigidBody::Dynamic,
            ColliderConstructor::Sphere {
                radius: PLAYER_SIZE * 0.5,
            },
            // Swept CCD: thin trimesh colliders (terrain, bridge decks) must not be
            // tunneled through during fast falls.
            SweptCcd::default(),
            RadialGravity,
            Mass(80.0),
            ColliderDensity(1000.0),
            LockedAxes::ROTATION_LOCKED,
            Transform::from_translation(spawn_pos),
            Visibility::default(),
            start,
            Player {
                fire_timer: Timer::from_seconds(ATTACK_INTERVAL, TimerMode::Repeating),
                damage: ATTACK_DAMAGE,
                range: ATTACK_RANGE,
                heading: start.tangent_basis().1,
            },
        ))
        // The visual capsule is taller than the sphere collider, so on the entity
        // origin its base sank ~capsule_radius into the terrain — a z-fighting
        // ring around the feet that flickered as the body micro-jittered. Lift the
        // mesh onto a child so its base rests exactly on the collider's ground
        // contact point.
        .with_child((
            Mesh3d(meshes.add(Capsule3d::new(capsule_radius, PLAYER_SIZE))),
            MeshMaterial3d(materials.add(StandardMaterial::from_color(PLAYER_COLOR))),
            // Rest the base on the ground contact point, then drop by the terrain
            // collision margin so the visual doesn't float above the terrain.
            Transform::from_xyz(0.0, capsule_radius - TERRAIN_MARGIN, 0.0),
        ));

    // Settlement markers
    for s in &level.settlements {
        let pos = Vec3::from_array(s.pos) * PLANET_RADIUS;
        commands.spawn((
            Transform::from_translation(pos),
            Settlement {
                name: s.name.clone(),
            },
        ));
    }

    let tris: Vec<[Vec3; 3]> = level
        .unit_tris
        .iter()
        .map(|t| {
            [
                Vec3::from_array(t[0]),
                Vec3::from_array(t[1]),
                Vec3::from_array(t[2]),
            ]
        })
        .collect();
    let planet_mesh = PlanetMesh::new(tris);
    let tags = FaceTags {
        off: level.face_tag_off.clone(),
        data: level.face_tag_data.clone(),
    };
    let face_types: Vec<shared::terrain::Terrain> = level
        .face_types
        .iter()
        .map(|&b| unsafe { std::mem::transmute(b) })
        .collect();
    commands.insert_resource(terrain);
    commands.insert_resource(planet_mesh);
    commands.insert_resource(LevelTags(tags));
    commands.insert_resource(LevelFaceTypes(face_types));
    commands.insert_resource(LevelSlope(level.slope_class.clone()));
    commands.insert_resource(LevelWaterDepth(level.water_depth.clone()));
    commands.insert_resource(LevelLandform(level.landform.clone()));
    commands.insert_resource(LevelRoadMaterial(level.road_material.clone()));
    commands.insert_resource(LevelRegions {
        regions: level.regions.clone(),
        face_region: level.face_region.clone(),
    });
    commands.insert_resource(LevelBlends(
        level
            .face_blend
            .iter()
            .map(|&(fi, a, b)| (fi, (a, b)))
            .collect(),
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
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        VertexAttributeValues::Float32x3(positions),
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_NORMAL,
        VertexAttributeValues::Float32x3(normals_out),
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_COLOR,
        VertexAttributeValues::Float32x4(colors_out),
    );
    mesh
}

/// Like build_visual_mesh but with ANGLE-THRESHOLDED smooth normals: a vertex
/// averages the normals of the faces sharing its position that lie within
/// ~40° of each other. The gently curving arch top smooths away its planks,
/// while the slab's hard edges (top↔side, ends) stay sharp.
fn build_smooth_mesh(tris: &[[[f32; 3]; 3]], colors: &[[[f32; 4]; 3]]) -> Mesh {
    let key = |v: [f32; 3]| [v[0].to_bits(), v[1].to_bits(), v[2].to_bits()];
    let face_n: Vec<Vec3> = tris
        .iter()
        .map(|t| {
            let (a, b, c) = (
                Vec3::from_array(t[0]),
                Vec3::from_array(t[1]),
                Vec3::from_array(t[2]),
            );
            (b - a).cross(c - a).normalize_or(Vec3::Y)
        })
        .collect();
    let mut at: std::collections::HashMap<[u32; 3], Vec<usize>> = std::collections::HashMap::new();
    for (fi, t) in tris.iter().enumerate() {
        for v in t {
            at.entry(key(*v)).or_default().push(fi);
        }
    }
    let cos_thresh = 40.0f32.to_radians().cos();
    let mut positions = Vec::with_capacity(tris.len() * 3);
    let mut normals = Vec::with_capacity(tris.len() * 3);
    let mut cols = Vec::with_capacity(tris.len() * 3);
    for (fi, (t, color)) in tris.iter().zip(colors.iter()).enumerate() {
        let fn_ = face_n[fi];
        for (k, v) in t.iter().enumerate() {
            let mut acc = Vec3::ZERO;
            for &nb in &at[&key(*v)] {
                if face_n[nb].dot(fn_) >= cos_thresh {
                    acc += face_n[nb];
                }
            }
            positions.push(*v);
            normals.push(acc.normalize_or(fn_).to_array());
            cols.push(color[k]);
        }
    }
    let mut mesh = Mesh::new(
        bevy::render::mesh::PrimitiveTopology::TriangleList,
        Default::default(),
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        VertexAttributeValues::Float32x3(positions),
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_NORMAL,
        VertexAttributeValues::Float32x3(normals),
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_COLOR,
        VertexAttributeValues::Float32x4(cols),
    );
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
    if keys.pressed(KeyCode::KeyW) {
        input.fwd += 1;
    }
    if keys.pressed(KeyCode::KeyS) {
        input.fwd -= 1;
    }
    input.turning = 0;
    if keys.pressed(KeyCode::KeyA) {
        input.turning += 1;
    }
    if keys.pressed(KeyCode::KeyD) {
        input.turning -= 1;
    }
    input.sprint = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    input.jump = keys.just_pressed(KeyCode::Space);
}

fn move_player(
    time: Res<Time>,
    input: Res<PlayerInput>,
    mut player_q: Query<(&mut Player, &Position, Forces)>,
) {
    let Ok((mut player, pos, mut forces)) = player_q.single_mut() else {
        return;
    };
    let dt = time.delta_secs();
    let up = pos.0.normalize();
    let world_r = pos.0.length();

    if input.turning != 0 {
        let angle = PLAYER_TURN * dt * input.turning as f32;
        player.heading = Quat::from_axis_angle(up, angle) * player.heading;
    }
    player.heading = (player.heading - up * player.heading.dot(up)).normalize();

    let underwater = world_r < PLANET_RADIUS;
    let speed =
        PLAYER_SPEED * if underwater { 0.4 } else { 1.0 } * if input.sprint { 2.5 } else { 1.0 };

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
            player.heading.cross(up),
            up,
            -player.heading,
        ));
    }
}

fn camera_follow(
    time: Res<Time>,
    player_q: Query<(&Player, &Transform), Without<MainCamera>>,
    mut camera_q: Query<&mut Transform, (With<MainCamera>, Without<MinimapCamera>)>,
) {
    let Ok((player, tf)) = player_q.single() else {
        return;
    };
    let Ok(mut cam_tf) = camera_q.single_mut() else {
        return;
    };

    let up = tf.translation.normalize();
    let feet = tf.translation;

    cam_tf.translation = feet + up * CAMERA_HEIGHT - player.heading * CAMERA_BACK;
    let look_target = feet + player.heading * CAMERA_LOOK_AHEAD;
    let t = 1.0 - (-6.0 * time.delta_secs()).exp();
    let current_look = cam_tf.rotation * -Vec3::Z + cam_tf.translation;
    cam_tf.look_at(current_look.lerp(look_target, t), up);
}

/// Advances the day and orbits the world sun around the planet's Y axis.
///
/// Because the sun is a single world-space direction (not player-relative), the
/// hemisphere facing it is lit (day) and the far side is dark (night) purely
/// from geometry — a directional light only lights surfaces whose normals face
/// it, and the atmosphere darkens wherever the sun is below the local horizon.
/// So illuminance stays constant; day/night comes from where you stand and where
/// the sun is. Ambient is a low constant floor (see `main`) so the night side
/// isn't pitch black.
/// When on, the sun stays overhead the player (permanent day). Toggled with `1`.
#[derive(Resource)]
pub struct SunLock(pub bool);

impl Default for SunLock {
    fn default() -> Self {
        Self(true)
    }
}

fn toggle_sun_lock(keys: Res<ButtonInput<KeyCode>>, mut lock: ResMut<SunLock>) {
    if keys.just_pressed(KeyCode::Digit1) {
        lock.0 = !lock.0;
    }
}

fn drive_daynight(
    time: Res<Time>,
    mut tod: ResMut<TimeOfDay>,
    lock: Res<SunLock>,
    player_q: Query<&Transform, (With<Player>, Without<Sun>)>,
    mut sun_q: Query<(&mut Transform, &mut DirectionalLight), With<Sun>>,
) {
    let dir = if lock.0 {
        // Locked: sun overhead the player — permanent day (time is frozen).
        player_q
            .single()
            .map(|p| p.translation.normalize())
            .unwrap_or(Vec3::Y)
    } else {
        // Sun orbits the polar (Y) axis, tilted above the equatorial plane.
        let advanced = tod.angle + time.delta_secs() * std::f32::consts::TAU / tod.day_length;
        if advanced >= std::f32::consts::TAU {
            tod.day += 1;
        }
        tod.angle = advanced.rem_euclid(std::f32::consts::TAU);
        Vec3::new(tod.angle.cos(), 0.35, tod.angle.sin()).normalize()
    };
    tod.sun_dir = dir;

    let Ok((mut sun_tf, mut light)) = sun_q.single_mut() else {
        return;
    };
    // Directional light shines from the sun toward the planet centre.
    sun_tf.translation = dir * 800.0;
    let up_ref = if dir.dot(Vec3::Y).abs() > 0.99 {
        Vec3::X
    } else {
        Vec3::Y
    };
    sun_tf.look_at(Vec3::ZERO, up_ref);
    light.illuminance = 13_000.0;
    light.color = Color::WHITE;
}

/// Tint the distance fog by how sunlit the camera's location is. With a fixed
/// bright fog colour, at night the distant terrain faded to bright blue while the
/// near (unlit) terrain went dark — distant looked brighter than near. Scaling
/// the fog colour with the local day factor keeps distance haze consistent with
/// the sky's day/night state.
fn drive_fog(
    tod: Res<TimeOfDay>,
    mut ambient: ResMut<GlobalAmbientLight>,
    mut fog_q: Query<(&Transform, &mut DistanceFog), With<MainCamera>>,
) {
    let Ok((tf, mut fog)) = fog_q.single_mut() else {
        return;
    };
    let day = tf.translation.normalize().dot(tod.sun_dir).clamp(0.0, 1.0);
    let c = Color::srgb(0.7 * day + 0.02, 0.8 * day + 0.03, 0.92 * day + 0.07);
    fog.color = c;
    fog.falloff = FogFalloff::from_visibility_colors(1700.0, c, c);

    // Ambient is global (can't track the day/night hemispheres on its own), so
    // drive it by the camera's day factor: a dim moonlit floor at night rising to
    // full fill by day. Fixed-bright ambient made objects glow at night.
    ambient.brightness = 35.0 + 130.0 * day;
}
