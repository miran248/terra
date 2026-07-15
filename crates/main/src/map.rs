use crate::constants::*;
use crate::minimap::MinimapCamera;
use crate::physics::RadialGravity;
use avian3d::prelude::*;
use bevy::prelude::*;
use bevy::render::mesh::VertexAttributeValues;
use shared::art::AssetName;
use shared::level::{
    BlendTarget, FaceTag, FloraKind, Landform, LevelData, RoadKind, RoadMaterial, SlopeClass,
    StructureKind, WaterDepth, WaterPhase,
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

/// Render-only child whose orientation follows the spherical surface without
/// writing to the dynamic body's physics-synchronized transform.
#[derive(Component)]
struct PlayerVisual;

#[derive(Resource)]
pub struct PlayerHp(pub f32);

#[derive(Resource)]
pub struct GameAssets {
    pub projectile_mesh: Handle<Mesh>,
    pub projectile_mat: Handle<StandardMaterial>,
}

pub struct MapPlugin;

#[derive(Resource)]
pub struct LevelTags(pub Vec<Vec<FaceTag>>);

#[derive(Resource)]
pub struct LevelFaceTypes(pub Vec<shared::terrain::Terrain>);

#[derive(Resource)]
pub struct LevelFaceCornerTypes(pub Vec<[shared::terrain::Terrain; 3]>);

#[derive(Resource)]
pub struct LevelSlope(pub Vec<SlopeClass>);

#[derive(Resource)]
pub struct LevelWaterDepth(pub Vec<Option<WaterDepth>>);

#[derive(Resource)]
pub struct LevelWaterPhase(pub Vec<Option<WaterPhase>>);

#[derive(Resource)]
pub struct LevelIceSurface(pub Vec<bool>);

#[derive(Resource)]
pub struct LevelLandform(pub Vec<Landform>);

#[derive(Resource)]
pub struct LevelRoadMaterial(pub Vec<Option<RoadMaterial>>);

/// Distance-based render culling for scatter props: beyond `0` metres the
/// entity is hidden. Tiny ground cover culls close, trees stay visible far.
#[derive(Component, Copy, Clone)]
pub struct CullRange(pub f32);

/// Per-flora-kind cull distance (metres) — the smaller the prop, the sooner it
/// stops being drawn in the distance.
fn flora_cull(kind: FloraKind) -> f32 {
    use shared::level::*;
    // ~1.6x the original ranges so props stay visible further out; the camera
    // fog visibility (main.rs) is set beyond the largest of these so props fade
    // into haze before this hard cull edge rather than popping.
    match kind {
        FloraKind::Flower | FloraKind::Grass | FloraKind::Mushroom | FloraKind::Reed | FloraKind::Lilypad | FloraKind::Seaweed | FloraKind::Coral => 150.0,
        FloraKind::Bush | FloraKind::Berry | FloraKind::Cactus | FloraKind::Rock => 300.0,
        FloraKind::Log => 420.0,
        FloraKind::Tree | FloraKind::DeadTree => 880.0,
    }
}

const FLORA_CHUNK_SIZE: f32 = 120.0;
type ChunkId = [i32; 3];

#[derive(Resource, Default)]
pub struct FloraManager {
    pub chunks: std::collections::HashMap<ChunkId, Vec<shared::level::FloraData>>,
    pub loaded_chunks: std::collections::HashMap<ChunkId, Vec<Entity>>,
}

fn pos_to_chunk(pos: Vec3) -> ChunkId {
    [
        (pos.x / FLORA_CHUNK_SIZE).floor() as i32,
        (pos.y / FLORA_CHUNK_SIZE).floor() as i32,
        (pos.z / FLORA_CHUNK_SIZE).floor() as i32,
    ]
}

#[allow(clippy::type_complexity)]
fn manage_flora_chunks(
    mut commands: Commands,
    catalog: Res<crate::asset_catalog::AssetCatalog>,
    camera: Query<&Transform, With<MainCamera>>,
    mut manager: ResMut<FloraManager>,
) {
    let Some(cam) = camera.iter().next() else { return };
    let eye = cam.translation;
    let center_chunk = pos_to_chunk(eye);
    
    // Radius of chunks (each chunk is 120m, flora cull reaches up to 880m -> ~8 chunks)
    let radius = 8;
    
    let mut visible_chunks = std::collections::HashSet::new();
    for x in -radius..=radius {
        for y in -radius..=radius {
            for z in -radius..=radius {
                if x*x + y*y + z*z <= radius*radius {
                    visible_chunks.insert([center_chunk[0] + x, center_chunk[1] + y, center_chunk[2] + z]);
                }
            }
        }
    }

    // Unload far chunks
    manager.loaded_chunks.retain(|chunk_id, entities| {
        if visible_chunks.contains(chunk_id) {
            true // keep
        } else {
            for &e in entities.iter() {
                commands.entity(e).despawn();
            }
            false // remove
        }
    });

    // Load new chunks
    for chunk_id in visible_chunks {
        if !manager.loaded_chunks.contains_key(&chunk_id) {
            if let Some(flora_list) = manager.chunks.get(&chunk_id) {
                let mut spawned_entities = Vec::with_capacity(flora_list.len());
                for f in flora_list {
                    let pos = Vec3::from_array(f.pos);
                    let up = pos.normalize();
                    let hash = f.pos[0].to_bits()
                        ^ f.pos[1].to_bits().rotate_left(13)
                        ^ f.pos[2].to_bits().rotate_left(27);
                    let scale = 0.7 + (hash & 0xff) as f32 / 255.0 * 0.6;
                    let yaw = (hash >> 8 & 0xff) as f32 / 255.0 * std::f32::consts::TAU;
                    let rotation = Quat::from_rotation_arc(Vec3::Y, up) * Quat::from_rotation_y(yaw);
                    
                    let mut root = commands.spawn((
                        CullRange(flora_cull(f.kind)),
                        Transform::from_translation(pos).with_rotation(rotation),
                        Visibility::default(),
                        bevy::light::NotShadowCaster,
                    ));

                    match shared::art::flora_collider(f.kind) {
                        shared::art::ColliderSpec::None => {}
                        shared::art::ColliderSpec::Box { half_extents } => {
                            root.insert((
                                RigidBody::Static,
                                Collider::cuboid(
                                    half_extents[0] * scale,
                                    half_extents[1] * scale,
                                    half_extents[2] * scale,
                                ),
                                Friction::ZERO,
                                Restitution::ZERO,
                            ));
                        }
                        shared::art::ColliderSpec::Capsule { radius, half_length } => {
                            root.insert((
                                RigidBody::Static,
                                Collider::capsule(radius * scale, half_length * 2.0 * scale),
                            ));
                        }
                    }

                    root.with_child((
                        WorldAssetRoot(catalog.scene(f.kind.asset_name())),
                        Transform::from_scale(Vec3::splat(scale)),
                    ));
                    
                    spawned_entities.push(root.id());
                }
                manager.loaded_chunks.insert(chunk_id, spawned_entities);
            }
        }
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
    pub face_region: Vec<Option<u32>>,
}

/// Blend-marked boundary faces: the pair of terrain kinds each links.
#[derive(Resource)]
pub struct LevelBlends(
    pub std::collections::BTreeMap<u32, (shared::terrain::Terrain, BlendTarget)>,
);

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
                    diagnose_player_fall,
                    drive_daynight,
                    drive_fog,
                    toggle_sun_lock,
                    cull_props,
                    manage_flora_chunks,
                )
                    .run_if(in_state(AppState::Playing)),
            );
    }
}

use shared::state::AppState;

fn setup_map(
    mut commands: Commands,
    catalog: Res<crate::asset_catalog::AssetCatalog>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut water_mats: ResMut<Assets<crate::water::WaterMaterial>>,
) {
    let level_bytes = include_bytes!("../assets/level_1337.bin");
    let level: LevelData = postcard::from_bytes(level_bytes).expect("deserialize level");
    level.validate().expect("validate level binary");

    commands.insert_resource(PlayerHp(PLAYER_HP));
    // The level carries the SOLVED elevation field the mesh was baked from, so
    // height queries and the rendered surface agree exactly (and startup skips
    // all topology planning).
    let terrain = TerrainGen::from_field(level.seed, level.vert_elev.clone());

    commands.insert_resource(GameAssets {
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
    if false {
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
                FloraKind::Tree => {
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
                FloraKind::Bush => {
                    commands.spawn((
                        CullRange(cull),
                        Mesh3d(bush_mesh.clone()),
                        MeshMaterial3d(bush_mat.clone()),
                        Transform::from_translation(pos + up * 0.45 * scale)
                            .with_rotation(rot)
                            .with_scale(Vec3::new(scale, scale * 0.75, scale)),
                    ));
                }
                FloraKind::Flower => {
                    commands.spawn((
                        CullRange(cull),
                        Mesh3d(flower_mesh.clone()),
                        MeshMaterial3d(flower_mats[(h >> 16) as usize % flower_mats.len()].clone()),
                        Transform::from_translation(pos + up * 0.22)
                            .with_rotation(rot)
                            .with_scale(Vec3::splat(scale)),
                    ));
                }
                FloraKind::Rock => {
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
                FloraKind::Grass => {
                    commands.spawn((
                        CullRange(cull),
                        Mesh3d(grass_mesh.clone()),
                        MeshMaterial3d(grass_mat.clone()),
                        Transform::from_translation(pos + up * 0.18 * scale)
                            .with_rotation(rot)
                            .with_scale(Vec3::splat(scale)),
                    ));
                }
                FloraKind::Log => {
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
                FloraKind::Mushroom => {
                    commands.spawn((
                        CullRange(cull),
                        Mesh3d(mush_mesh.clone()),
                        MeshMaterial3d(mush_mat.clone()),
                        Transform::from_translation(pos + up * 0.16 * scale)
                            .with_rotation(rot)
                            .with_scale(Vec3::new(scale, scale * 0.7, scale)),
                    ));
                }
                FloraKind::Cactus => {
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
                FloraKind::Berry => {
                    commands.spawn((
                        CullRange(cull),
                        Mesh3d(berry_mesh.clone()),
                        MeshMaterial3d(berry_mat.clone()),
                        Transform::from_translation(pos + up * 0.4 * scale)
                            .with_rotation(rot)
                            .with_scale(Vec3::new(scale, scale * 0.85, scale)),
                    ));
                }
                FloraKind::DeadTree => {
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
                FloraKind::Seaweed | FloraKind::Lilypad | FloraKind::Coral => {}, FloraKind::Reed => {
                    commands.spawn((
                        CullRange(cull),
                        Mesh3d(reed_mesh.clone()),
                        MeshMaterial3d(reed_mat.clone()),
                        Transform::from_translation(pos + up * 0.65 * scale)
                            .with_rotation(rot)
                            .with_scale(Vec3::splat(scale)),
                    ));
                }
            }
        }
    }

    let mut flora_manager = FloraManager::default();
    for f in &level.flora {
        let chunk_id = pos_to_chunk(Vec3::from_array(f.pos));
        flora_manager.chunks.entry(chunk_id).or_default().push(f.clone());
    }
    commands.insert_resource(flora_manager);

    // Structures: contextual built props (wells, docks, walls, watchtowers,
    // ruins, farms, campfires), spawned from baked positions like bridges.
    if false {
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
                StructureKind::Well => {
                    at(
                        &mut commands,
                        cyl_mesh.clone(),
                        stone.clone(),
                        0.6,
                        Vec3::new(2.0, 1.2, 2.0),
                        true,
                    );
                }
                StructureKind::Campfire => {
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
                StructureKind::Wall => {
                    at(
                        &mut commands,
                        box_mesh.clone(),
                        dark_stone.clone(),
                        1.5,
                        Vec3::new(6.0, 3.0, 1.2),
                        true,
                    );
                }
                StructureKind::Dock => {
                    at(
                        &mut commands,
                        box_mesh.clone(),
                        plank.clone(),
                        0.4,
                        Vec3::new(3.0, 0.4, 10.0),
                        true,
                    );
                }
                StructureKind::Farm => {
                    at(
                        &mut commands,
                        box_mesh.clone(),
                        soil.clone(),
                        0.1,
                        Vec3::new(9.0, 0.2, 9.0),
                        false,
                    );
                }
                StructureKind::Watchtower => {
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
                StructureKind::Ruin => {
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
            }
        }
    }

    for structure in &level.structures {
        let pos = Vec3::from_array(structure.pos);
        let up = pos.normalize();
        let rotation = Quat::from_rotation_arc(Vec3::Y, up) * Quat::from_rotation_y(structure.yaw);
        let scale = match structure.kind {
            StructureKind::Ruin => Vec3::new(6.0, 3.0, 6.0),
            StructureKind::Watchtower => Vec3::new(4.5, 12.0, 4.5),
            StructureKind::Dock => Vec3::new(3.0, 0.8, 10.0),
            StructureKind::Farm => Vec3::new(9.0, 0.3, 9.0),
            StructureKind::Wall => Vec3::new(6.0, 3.0, 1.2),
            StructureKind::Well => Vec3::new(2.0, 1.2, 2.0),
            StructureKind::Campfire => Vec3::splat(1.6),
        };
        let mut root = commands.spawn((
            Transform::from_translation(pos).with_rotation(rotation),
            Visibility::default(),
        ));
        if !matches!(
            structure.kind,
            StructureKind::Farm | StructureKind::Campfire
        ) {
            root.insert((
                RigidBody::Static,
                Collider::cuboid(scale.x * 0.5, scale.y * 0.5, scale.z * 0.5),
            ));
        }
        root.with_child((
            WorldAssetRoot(catalog.scene(structure.kind.asset_name())),
            Transform::from_scale(scale),
        ));
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

    // Roads are visual ribbons over the authoritative terrain collider. Their
    // narrow width reads as a path without changing traversal or level data.
    let (road_tris, road_colors) = build_road_ribbons(&level, &ground, 4.0);
    if !road_tris.is_empty() {
        commands.spawn((
            Mesh3d(meshes.add(build_smooth_mesh(&road_tris, &road_colors))),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: Color::WHITE,
                perceptual_roughness: 0.95,
                ..default()
            })),
            Transform::default(),
            Ground,
        ));
    }

    let bridge_color = Color::srgb(0.35, 0.25, 0.18).to_linear();
    for road in level.roads.iter().filter(|r| r.kind == RoadKind::Bridge) {
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
    if let Some(water) = crate::water::build_water_surface(
        &level.terrain_tris,
        &level.face_water_r,
        &level.water_phase,
    ) {
        commands.spawn((
            Mesh3d(meshes.add(water)),
            MeshMaterial3d(water_mat.clone()),
            Transform::default(),
            Ground,
        ));
    }

    // River surfaces use the generator-baked, smoothed corner radii so runtime
    // rendering has no topology/clustering work and cannot introduce seams.
    if let Some(river) = crate::water::build_river_surfaces(
        &level.terrain_tris,
        &level.face_river_r,
        &level.water_phase,
    ) {
        commands.spawn((
            Mesh3d(meshes.add(river)),
            MeshMaterial3d(water_mats.add(crate::water::river_material())),
            Transform::default(),
            Ground,
        ));
    }

    if let Some((ice, ice_tris)) = crate::water::build_ice_surface(
        &level.terrain_tris,
        &level.face_water_r,
        &level.face_river_r,
        &level.water_phase,
    ) {
        commands.spawn((
            Mesh3d(meshes.add(ice)),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: Color::srgb(0.68, 0.86, 0.94),
                perceptual_roughness: 0.28,
                metallic: 0.05,
                ..default()
            })),
            RigidBody::Static,
            build_collider(&ice_tris),
            CollisionMargin(TERRAIN_MARGIN),
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
            // Populated by Avian's narrow phase for fall-through diagnostics.
            CollidingEntities::default(),
            RadialGravity,
            Mass(80.0),
            ColliderDensity(1000.0),
            LockedAxes::ROTATION_LOCKED,
            Transform::from_translation(spawn_pos),
            Visibility::default(),
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
            PlayerVisual,
            WorldAssetRoot(catalog.scene("actor.player")),
            // Rest the base on the ground contact point, then drop by the terrain
            // collision margin so the visual doesn't float above the terrain.
            Transform::from_xyz(0.0, -capsule_radius + TERRAIN_MARGIN, 0.0)
                .with_scale(Vec3::splat(PLAYER_SIZE)),
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
    let tags = level.face_tags.clone();
    let face_types = level.face_types.clone();
    commands.insert_resource(terrain);
    commands.insert_resource(planet_mesh);
    commands.insert_resource(LevelTags(tags));
    commands.insert_resource(LevelFaceTypes(face_types));
    commands.insert_resource(LevelFaceCornerTypes(level.face_corner_types.clone()));
    commands.insert_resource(LevelSlope(level.slope_class.clone()));
    commands.insert_resource(LevelWaterDepth(level.water_depth.clone()));
    commands.insert_resource(LevelWaterPhase(level.water_phase.clone()));
    commands.insert_resource(LevelIceSurface(
        level
            .water_phase
            .iter()
            .enumerate()
            .map(|(face, phase)| {
                *phase == Some(WaterPhase::Frozen)
                    && (level.face_water_r[face] > 0.0
                        || level.face_river_r[face].iter().any(|&radius| radius > 0.0))
            })
            .collect(),
    ));
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
            .map(|blend| (blend.face, (blend.base, blend.target)))
            .collect(),
    ));
}

fn road_color(material: RoadMaterial) -> [f32; 4] {
    let color = match material {
        RoadMaterial::Gravel => Color::srgb(0.40, 0.39, 0.36),
        RoadMaterial::Dirt => Color::srgb(0.45, 0.33, 0.22),
        RoadMaterial::Sand => Color::srgb(0.78, 0.70, 0.50),
        RoadMaterial::Rock => Color::srgb(0.32, 0.31, 0.30),
    };
    color.to_linear().to_f32_array()
}

type RenderTriangle = [[f32; 3]; 3];
type TriangleColors = [[f32; 4]; 3];

fn build_road_ribbons(
    level: &LevelData,
    ground: &PlanetMesh,
    width: f32,
) -> (Vec<RenderTriangle>, Vec<TriangleColors>) {
    let mut triangles = Vec::new();
    let mut colors = Vec::new();
    for road in level
        .roads
        .iter()
        .filter(|road| road.kind == RoadKind::Road)
    {
        let mut directions = Vec::new();
        for pair in road.points.windows(2) {
            let from = Vec3::from_array(pair[0]).normalize();
            let to = Vec3::from_array(pair[1]).normalize();
            let angle = from.dot(to).clamp(-1.0, 1.0).acos();
            let steps = ((angle * PLANET_RADIUS / 4.0).ceil() as usize).max(1);
            for step in 0..steps {
                directions.push(from.slerp(to, step as f32 / steps as f32).normalize());
            }
        }
        if let Some(last) = road.points.last() {
            directions.push(Vec3::from_array(*last).normalize());
        }
        if directions.len() < 2 {
            continue;
        }
        let mut edges = Vec::with_capacity(directions.len());
        let mut edge_materials = Vec::with_capacity(directions.len());
        for index in 0..directions.len() {
            let up = directions[index];
            let previous = directions[index.saturating_sub(1)];
            let next = directions[(index + 1).min(directions.len() - 1)];
            let forward = (next - previous).reject_from(up).normalize_or(Vec3::X);
            let side = up.cross(forward).normalize_or(Vec3::Z) * (width * 0.5);
            let edge = |offset: Vec3| {
                let direction = (up * PLANET_RADIUS + offset).normalize();
                direction * (ground.facet_radius(direction, PLANET_RADIUS) + 0.08)
            };
            edges.push([edge(side), edge(-side)]);
            let material = ground
                .face_at(up)
                .and_then(|face| level.road_material[face])
                .unwrap_or(RoadMaterial::Gravel);
            edge_materials.push(material);
        }
        for index in 0..edges.len() - 1 {
            let [left, right] = edges[index];
            let [next_left, next_right] = edges[index + 1];
            triangles.push([left.to_array(), right.to_array(), next_left.to_array()]);
            triangles.push([
                right.to_array(),
                next_right.to_array(),
                next_left.to_array(),
            ]);
            let first = road_color(edge_materials[index]);
            let second = road_color(edge_materials[index + 1]);
            colors.push([first, first, second]);
            colors.push([first, second, second]);
        }
    }
    (triangles, colors)
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
    planet: Res<PlanetMesh>,
    ice_surface: Res<LevelIceSurface>,
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
    let on_frozen_water = planet
        .face_at(up)
        .and_then(|face| ice_surface.0.get(face))
        .copied()
        .unwrap_or(false);
    let slowed_by_water = underwater || on_frozen_water;
    let speed = PLAYER_SPEED
        * if slowed_by_water { 0.4 } else { 1.0 }
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

/// Emit one high-signal report when the player's center crosses beneath the
/// terrain. Logging only the transition avoids flooding the console while the
/// body continues falling and preserves the values from the failure frame.
fn diagnose_player_fall(
    real_time: Res<Time<Real>>,
    fixed_time: Res<Time<Fixed>>,
    terrain: Option<Res<TerrainGen>>,
    player_q: Query<(&Position, &LinearVelocity, &CollidingEntities), With<Player>>,
    mut was_below_surface: Local<bool>,
) {
    let Some(terrain) = terrain else { return };
    let Ok((position, velocity, contacts)) = player_q.single() else {
        return;
    };
    let radius = position.0.length();
    let up = position.0.normalize_or_zero();
    if up == Vec3::ZERO {
        return;
    }
    let surface_radius = terrain.surface_radius(shared::sphere::SpherePos::new(up));
    let altitude = radius - surface_radius;
    let below_surface = altitude < 0.0;

    if below_surface && !*was_below_surface {
        let radial_velocity = velocity.0.dot(up);
        warn!(
            altitude,
            radius,
            surface_radius,
            radial_velocity,
            speed = velocity.0.length(),
            frame_ms = real_time.delta_secs() * 1000.0,
            fixed_ms = fixed_time.delta_secs() * 1000.0,
            contact_count = contacts.len(),
            ?contacts,
            "player crossed beneath terrain surface"
        );
    }
    *was_below_surface = below_surface;
}

fn orient_player(
    player_q: Query<(&Player, &Position)>,
    mut visual_q: Query<&mut Transform, With<PlayerVisual>>,
) {
    let Ok((player, position)) = player_q.single() else {
        return;
    };
    let Ok(mut visual_tf) = visual_q.single_mut() else {
        return;
    };
    let up = position.0.normalize();
    // Child translation is expressed in the unrotated physics parent's space;
    // rotating this child does not rotate its own offset. Keep the capsule lift
    // radial explicitly so its base remains on the collider contact point.
    visual_tf.translation = up * (PLAYER_SIZE * 0.4 - TERRAIN_MARGIN);
    visual_tf.rotation = Quat::from_mat3(&Mat3::from_cols(
        player.heading.cross(up),
        up,
        -player.heading,
    ));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn road_ribbons_are_finite_complete_and_four_metres_wide() {
        let level: LevelData = postcard::from_bytes(include_bytes!("../assets/level_1337.bin"))
            .expect("tracked level should deserialize");
        let displaced = level
            .terrain_tris
            .iter()
            .map(|triangle| triangle.map(Vec3::from_array))
            .collect();
        let ground = PlanetMesh::new(displaced);
        let (triangles, colors) = build_road_ribbons(&level, &ground, 4.0);
        assert!(!triangles.is_empty());
        assert_eq!(triangles.len(), colors.len());
        assert!(
            triangles
                .iter()
                .flatten()
                .flatten()
                .all(|value| value.is_finite())
        );
        let first = triangles[0].map(Vec3::from_array);
        assert!((first[0].distance(first[1]) - 4.0).abs() < 0.1);
    }
}
