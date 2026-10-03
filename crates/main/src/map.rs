use crate::constants::*;
use crate::physics::RadialGravity;
use avian3d::prelude::*;
use bevy::prelude::*;
use bevy::render::mesh::VertexAttributeValues;
use shared::level::{
    BlendTarget, FaceTag, Landform, LevelData, RoadKind, RoadMaterial, SceneryKind, SettlementKind,
    SlopeClass, WaterDepth, WaterPhase,
};
use shared::planet::PlanetMesh;
use shared::sphere::PLANET_RADIUS;
use shared::terrain::TerrainGen;

/// Small speculative skin; swept CCD protects fast motion without a half-meter visual offset.
const TERRAIN_MARGIN: f32 = 0.02;

pub(crate) fn player_half_height() -> f32 {
    shared::asset_contract::candidate_contract("actor.player")
        .unwrap()
        .dimensions[1]
        * 0.5
}

#[derive(Component)]
pub struct Ground;

/// The displaced triangles used for physics, distinct from the unit-sphere face index.
#[derive(Resource)]
pub(crate) struct CollisionTerrain(pub PlanetMesh);

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

/// Per-scenery-kind cull distance (metres) — the smaller the prop, the sooner it
/// stops being drawn in the distance.
pub fn scenery_cull(kind: SceneryKind) -> f32 {
    use shared::level::*;
    // ~1.6x the original ranges so props stay visible further out; the camera
    // fog visibility (main.rs) is set beyond the largest of these so props fade
    // into haze before this hard cull edge rather than popping.
    match kind {
        SceneryKind::Flora(
            FloraKind::Flower
            | FloraKind::Grass
            | FloraKind::Reed
            | FloraKind::Lilypad
            | FloraKind::Seaweed
            | FloraKind::Fern
            | FloraKind::Cattail
            | FloraKind::Vine,
        )
        | SceneryKind::Mushroom
        | SceneryKind::Coral
        | SceneryKind::Anemone
        | SceneryKind::Starfish
        | SceneryKind::Shell
        | SceneryKind::Icicle => 150.0,
        SceneryKind::Flora(
            FloraKind::Bush | FloraKind::Berry | FloraKind::Cactus | FloraKind::Kelp,
        )
        | SceneryKind::Rock
        | SceneryKind::Skull
        | SceneryKind::Flora(FloraKind::Tumbleweed) => 300.0,
        SceneryKind::Log | SceneryKind::Snowdrift | SceneryKind::Stump | SceneryKind::Snowman => {
            420.0
        }
        SceneryKind::Flora(FloraKind::Tree) | SceneryKind::DeadTree => 880.0,
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

/// Face-based region memberships and named bridge surfaces for the HUD.
#[derive(Resource)]
pub struct LevelRegions {
    pub regions: Vec<shared::level::RegionData>,
    pub face_regions: shared::level::RegionMemberships,
    pub settlements: Vec<(String, SettlementKind)>,
    /// Top surfaces from the same deck geometry used for rendering and collision.
    pub bridge_top_surfaces_by_name: std::collections::BTreeMap<String, Vec<[[f32; 3]; 3]>>,
}

impl LevelRegions {
    /// Named bridge decks under the player, when the player is standing close
    /// to the rendered deck top. Bridges are queried from their own geometry,
    /// separately from terrain-face region memberships.
    pub fn bridge_names_at_position(&self, player_position: Vec3) -> Vec<String> {
        const BELOW_DECK_TOLERANCE: f32 = 0.25;
        const ABOVE_DECK_TOLERANCE: f32 = 1.0;

        let Some(direction) = player_position.try_normalize() else {
            return Vec::new();
        };
        let player_radius = player_position.length();
        let collider_radius = player_half_height();
        let minimum_clearance = collider_radius - BELOW_DECK_TOLERANCE;
        let maximum_clearance = collider_radius + ABOVE_DECK_TOLERANCE;

        let mut names = self
            .bridge_top_surfaces_by_name
            .iter()
            .filter_map(|(name, surface)| {
                let deck_radius = surface
                    .iter()
                    .filter_map(|triangle| {
                        let triangle = triangle.map(Vec3::from_array);
                        shared::planet::ray_triangle_radius(direction, &triangle)
                    })
                    .max_by(f32::total_cmp)?;
                let clearance = player_radius - deck_radius;
                (minimum_clearance..=maximum_clearance)
                    .contains(&clearance)
                    .then(|| name.clone())
            })
            .collect::<Vec<_>>();
        names.sort_unstable();
        names.dedup();
        names
    }
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
            // Start around mid-morning so the player lands in daylight.
            angle: std::f32::consts::FRAC_PI_4,
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
            // TimeOfDay is inserted manually in setup_map for accurate local noon.
            .init_resource::<SunLock>()
            .add_systems(OnEnter(AppState::Playing), setup_map)
            .add_systems(
                Update,
                read_player_input.run_if(in_state(AppState::Playing)),
            )
            .add_systems(
                FixedUpdate,
                move_player
                    .run_if(in_state(AppState::Playing))
                    .run_if(crate::exploration::on_foot),
            )
            .add_systems(
                Update,
                (
                    orient_player,
                    diagnose_player_fall,
                    drive_daynight,
                    drive_fog,
                    toggle_sun_lock,
                    cull_props,
                    crate::chunks::update_chunk_lods,
                    crate::chunks::stream_scenery,
                )
                    .run_if(in_state(AppState::Playing)),
            );
    }
}

use shared::state::AppState;

pub(crate) fn setup_map(
    motion: Res<crate::shader_motion::ShaderMotionBuffer>,
    mut commands: Commands,
    catalog: Res<crate::asset_catalog::AssetCatalog>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut water_mats: ResMut<Assets<crate::water::WaterMaterial>>,
) {
    let level_bytes = include_bytes!("../assets/level_1337.bin");
    let level = LevelData::from_artifact_bytes(level_bytes)
        .expect("deserialize level artifact; regenerate it with `cargo run -p gen_level`");
    level.validate().expect("validate level binary");

    commands.insert_resource(PlayerHp(PLAYER_HP));
    // The level carries the SOLVED elevation field the mesh was baked from, so
    // height queries and the rendered surface agree exactly (and startup skips
    // all topology planning).
    let terrain = TerrainGen::from_field_with_settlement_config(
        level.seed,
        level.vert_elev.clone(),
        level.settlement_config,
    );

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

    // Terrain visuals are chunked (see crate::chunks) — only the whole-planet
    // collider is global. Physics never streams: no fall-through at chunk
    // borders, world-map teleports always land on solid ground.
    let terrain_mat = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        perceptual_roughness: 0.95,
        // The sun is re-aimed at the player's feet each frame, so its specular
        // hotspot rides with the player; on the flat-shaded terrain that broad
        // highlight aliases into moving grain. Near-zero reflectance removes
        // the specular lobe (matte terrain) and kills the sparkle.
        reflectance: 0.02,
        ..default()
    });
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

    // Structures and scenery spawn per-chunk with LOD (see crate::chunks).

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
    let mut bridge_top_surfaces_by_name = std::collections::BTreeMap::new();
    for road in level.roads.iter().filter(|r| r.kind == RoadKind::Bridge) {
        let span: Vec<shared::sphere::SpherePos> = road
            .points
            .iter()
            .map(|p| shared::sphere::SpherePos::new(Vec3::from_array(*p)))
            .collect();
        let deck = shared::roads::build_bridge_deck_geometry(&span, &ground, 4.0);
        if deck.triangles.is_empty() {
            continue;
        }
        bridge_top_surfaces_by_name.insert(road.name.clone(), deck.top_surface);
        let colors = vec![[bridge_color.to_f32_array(); 3]; deck.triangles.len()];
        commands.spawn((
            Mesh3d(meshes.add(build_smooth_mesh(&deck.triangles, &colors))),
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
            build_collider(&deck.triangles),
            Transform::default(),
            Ground,
        ));
    }

    // Water/river visuals spawn per-chunk with LOD (see crate::chunks); the
    // gen-time per-face waterline (`face_water_r`) clusters water into
    // connected bodies, so the sea can't flood an inland lake basin. Only the
    // ice COLLIDER is global here — walkable surfaces never stream.
    let water_mat = water_mats.add(crate::water::water_material(motion.handle.clone()));
    let river_mat = water_mats.add(crate::water::river_material(motion.handle.clone()));
    let ice_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.68, 0.86, 0.94),
        perceptual_roughness: 0.28,
        metallic: 0.05,
        ..default()
    });
    if let Some((_, ice_tris)) = crate::water::build_ice_surface(
        &level.terrain_tris,
        &level.face_water_r,
        &level.face_river_r,
        &level.water_phase,
    ) {
        commands.spawn((
            RigidBody::Static,
            build_collider(&ice_tris),
            CollisionMargin(TERRAIN_MARGIN),
            Transform::default(),
            Ground,
        ));
    }

    // Bin all chunked world data; chunks::update_chunk_lods spawns the visuals
    // (LOD 1 everywhere within the first frames, refined as the player moves).
    let border_mat = |color: Color, materials: &mut Assets<StandardMaterial>| {
        materials.add(StandardMaterial {
            base_color: color,
            unlit: true,
            ..default()
        })
    };
    let border_mats = [
        border_mat(Color::srgb(1.0, 0.2, 0.2), &mut materials), // LOD 1: red
        border_mat(Color::srgb(1.0, 0.9, 0.2), &mut materials), // LOD 2: yellow
        border_mat(Color::srgb(0.2, 1.0, 0.3), &mut materials), // LOD 3: green
    ];
    commands.insert_resource(crate::chunks::ChunkManager::new(
        level.terrain_tris.clone(),
        level.terrain_colors.clone(),
        level.face_water_r.clone(),
        level.face_river_r.clone(),
        level.water_phase.clone(),
        level.scenery.clone(),
        level.structures.clone(),
        terrain_mat,
        water_mat,
        river_mat,
        ice_mat,
        border_mats,
    ));

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

    // Start at local noon for the spawn settlement.
    // Sun orbits around Y: sun_dir ≈ (cos(angle), 0.35, sin(angle)).normalize().
    // Noon when sun_dir projected onto the XZ plane aligns with up projected onto XZ.
    // angle = atan2(up.z, up.x) — the settlement's XZ azimuth.
    let spawn_noon_angle = up.z.atan2(up.x);
    commands.insert_resource(TimeOfDay {
        angle: spawn_noon_angle,
        sun_dir: Vec3::new(spawn_noon_angle.cos(), 0.35, spawn_noon_angle.sin()).normalize(),
        ..Default::default()
    });
    let spawn_pos = player_spawn_position(up, spawn_surface_r);
    commands
        .spawn(player_physics_bundle(spawn_pos, start.tangent_basis().1))
        .with_child((
            PlayerVisual,
            WorldAssetRoot(catalog.scene("actor.player")),
            catalog.actor("actor.player", 0),
            Transform::from_translation(Vec3::NEG_Y * player_half_height()),
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
    commands.insert_resource(CollisionTerrain(ground));
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
        face_regions: level.face_regions.clone(),
        settlements: level
            .settlements
            .iter()
            .map(|settlement| (settlement.name.clone(), settlement.kind))
            .collect(),
        bridge_top_surfaces_by_name,
    });
    commands.insert_resource(LevelBlends(
        level
            .face_blend
            .iter()
            .map(|blend| (blend.face, (blend.base, blend.target)))
            .collect(),
    ));
}

fn player_spawn_position(up: Vec3, surface_radius: f32) -> Vec3 {
    up * (surface_radius + player_half_height() + TERRAIN_MARGIN + 0.1)
}

pub(crate) fn player_physics_bundle(spawn_pos: Vec3, heading: Vec3) -> impl Bundle {
    (
        RigidBody::Dynamic,
        crate::asset_collision::actor_body("actor.player").0,
        crate::physics::RadialUpright,
        // Swept CCD: thin trimesh colliders (terrain, bridge decks) must not be
        // tunneled through during fast falls.
        SweptCcd::default(),
        // Populated by Avian's narrow phase for fall-through diagnostics.
        CollidingEntities::default(),
        RadialGravity,
        Mass(80.0),
        ColliderDensity(1000.0),
        LockedAxes::ROTATION_LOCKED,
        Transform::from_translation(spawn_pos)
            .with_rotation(Quat::from_rotation_arc(Vec3::Y, spawn_pos.normalize())),
        Visibility::default(),
        Player {
            fire_timer: Timer::from_seconds(ATTACK_INTERVAL, TimerMode::Repeating),
            damage: ATTACK_DAMAGE,
            range: ATTACK_RANGE,
            heading,
        },
    )
}

#[cfg(test)]
mod region_identity_tests {
    use super::*;
    use shared::level::{RoadData, RoadKind};

    #[test]
    fn production_player_has_a_one_meter_radial_capsule() {
        for up in [Vec3::Y, Vec3::X, Vec3::NEG_Z] {
            let mut world = World::new();
            let position = up * 2001.;
            let entity = world.spawn(player_physics_bundle(position, Vec3::Z)).id();
            let collider = world.get::<Collider>(entity).unwrap();
            let rotation = world.get::<Transform>(entity).unwrap().rotation;
            assert!(collider.contains_point(position, rotation, position + up * 0.49));
            assert!(collider.contains_point(position, rotation, position - up * 0.49));
            assert!(!collider.contains_point(position, rotation, position + up * 0.51));
            let tangent = up.any_orthonormal_vector();
            assert!(!collider.contains_point(position, rotation, position + tangent * 0.13));
        }
    }

    fn bridge(name: &str) -> RoadData {
        RoadData {
            name: name.into(),
            points: vec![
                Vec3::X.to_array(),
                Vec3::new(0.97, 0.24, 0.0).normalize().to_array(),
            ],
            kind: RoadKind::Bridge,
            from_endpoint: 0,
            to_endpoint: 1,
        }
    }

    #[test]
    fn bridge_hud_name_requires_the_player_to_be_on_the_deck_surface() {
        let ground = shared::planet::PlanetMesh::new(shared::planet::unit_icosphere_tris(3));
        let road = bridge("Test Bridge");
        let points = road
            .points
            .iter()
            .map(|point| shared::sphere::SpherePos::new(Vec3::from_array(*point)))
            .collect::<Vec<_>>();
        let deck = shared::roads::build_bridge_deck_geometry(&points, &ground, 4.0);
        let direction = shared::sphere::slerp(points[0], points[1], 0.5).0;
        let deck_radius = deck
            .top_surface
            .iter()
            .filter_map(|triangle| {
                let triangle = triangle.map(Vec3::from_array);
                shared::planet::ray_triangle_radius(direction, &triangle)
            })
            .max_by(f32::total_cmp)
            .expect("deck top should cross its centerline");
        let regions = LevelRegions {
            regions: vec![],
            face_regions: shared::level::RegionMemberships::from_memberships(vec![vec![]]),
            settlements: vec![],
            bridge_top_surfaces_by_name: std::collections::BTreeMap::from([(
                "Test Bridge".into(),
                deck.top_surface,
            )]),
        };
        let player_collider_radius = player_half_height();
        let on_deck = direction * (deck_radius + player_collider_radius);
        let above_deck = direction * (deck_radius + player_collider_radius + 3.0);
        let below_deck = direction * (deck_radius - 1.7);

        assert_eq!(regions.bridge_names_at_position(on_deck), ["Test Bridge"]);
        assert!(regions.bridge_names_at_position(above_deck).is_empty());
        assert!(regions.bridge_names_at_position(below_deck).is_empty());
    }

    #[test]
    fn generated_bridge_deck_surface_reports_its_name_across_width_and_length() {
        let level = shared::level::LevelData::from_artifact_bytes(include_bytes!(
            "../assets/level_1337.bin"
        ))
        .expect("load generated level");
        let ground = shared::planet::PlanetMesh::new(
            level
                .terrain_tris
                .iter()
                .map(|triangle| triangle.map(Vec3::from_array))
                .collect(),
        );
        let bridge_roads = level
            .roads
            .iter()
            .filter(|road| road.kind == RoadKind::Bridge)
            .collect::<Vec<_>>();
        assert!(!bridge_roads.is_empty(), "generated level needs a bridge");

        let mut bridge_top_surfaces_by_name = std::collections::BTreeMap::new();
        for road in &bridge_roads {
            let points = road
                .points
                .iter()
                .map(|point| shared::sphere::SpherePos::new(Vec3::from_array(*point)))
                .collect::<Vec<_>>();
            let deck = shared::roads::build_bridge_deck_geometry(&points, &ground, 4.0);
            bridge_top_surfaces_by_name.insert(road.name.clone(), deck.top_surface);
        }
        let regions = LevelRegions {
            regions: vec![],
            face_regions: shared::level::RegionMemberships::from_memberships(vec![]),
            settlements: vec![],
            bridge_top_surfaces_by_name,
        };
        let mut sampled_faces = 0;
        let mut untagged_faces = 0;
        for road in bridge_roads {
            let surface = &regions.bridge_top_surfaces_by_name[&road.name];
            for triangle in surface {
                let triangle = triangle.map(Vec3::from_array);
                let direction = (triangle[0] + triangle[1] + triangle[2]).normalize();
                let deck_radius = shared::planet::ray_triangle_radius(direction, &triangle)
                    .expect("top triangle crosses its centroid ray");
                let face = ground
                    .face_at(direction)
                    .expect("deck sample projects onto a terrain face");
                if !level.face_tags[face].contains(&FaceTag::Bridge) {
                    untagged_faces += 1;
                }
                let player_position = direction * (deck_radius + player_half_height());

                assert_eq!(
                    regions.bridge_names_at_position(player_position),
                    [road.name.clone()],
                    "bridge deck sample on face {face} should name {}",
                    road.name,
                );
                sampled_faces += 1;
            }
        }
        assert!(sampled_faces > 0);
        assert!(untagged_faces > 0, "test should cover untagged deck faces");
    }
}

#[cfg(test)]
mod startup_physics_tests {
    use super::*;
    use bevy::mesh::MeshPlugin;
    use bevy::state::app::StatesPlugin;
    use bevy::time::TimeUpdateStrategy;
    use std::time::Duration;

    #[derive(Resource)]
    struct StartupPhysicsData {
        terrain_tris: Vec<[[f32; 3]; 3]>,
        spawn_pos: Vec3,
        heading: Vec3,
    }

    fn spawn_seed_1337_world(mut commands: Commands, data: Res<StartupPhysicsData>) {
        commands.spawn((
            RigidBody::Static,
            build_collider(&data.terrain_tris),
            CollisionMargin(TERRAIN_MARGIN),
            Transform::default(),
            Ground,
        ));
        commands.spawn(player_physics_bundle(data.spawn_pos, data.heading));
    }

    #[test]
    fn seed_1337_player_remains_grounded_through_a_long_state_entry_frame() {
        let level = LevelData::from_artifact_bytes(include_bytes!("../assets/level_1337.bin"))
            .expect("load generated level");
        let terrain = TerrainGen::from_field_with_settlement_config(
            level.seed,
            level.vert_elev.clone(),
            level.settlement_config,
        );
        let settlement = level
            .settlements
            .first()
            .expect("generated spawn settlement");
        let start = shared::sphere::SpherePos::new(Vec3::from_array(settlement.pos));
        let spawn_surface_r = terrain.surface_radius(start);
        let spawn_pos = player_spawn_position(start.0, spawn_surface_r);

        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            TransformPlugin,
            StatesPlugin,
            PhysicsPlugins::default(),
            AssetPlugin::default(),
            MeshPlugin,
        ))
        .add_plugins(crate::physics::PhysicsPlugin)
        .init_state::<AppState>()
        .insert_resource(StartupPhysicsData {
            terrain_tris: level.terrain_tris.clone(),
            spawn_pos,
            heading: start.tangent_basis().1,
        })
        .insert_resource(SubstepCount(12))
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
            155,
        )))
        .add_systems(OnEnter(AppState::Playing), spawn_seed_1337_world);

        app.finish();
        app.update();
        app.world_mut()
            .resource_mut::<NextState<AppState>>()
            .set(AppState::Playing);
        app.update();

        let mut query = app
            .world_mut()
            .query_filtered::<(&Position, &CollidingEntities), With<Player>>();
        let (position, contacts) = query.single(app.world()).unwrap();
        let altitude = position.0.length() - spawn_surface_r;
        assert!(
            altitude >= -0.05,
            "player crossed beneath the seed 1337 spawn terrain after its first physics frame: altitude={altitude:.3}m"
        );
        assert!(
            !contacts.is_empty(),
            "player should have contacted terrain during the first seed 1337 physics frame"
        );

        for _ in 0..4 {
            app.update();
        }
        let (position, contacts) = query.single(app.world()).unwrap();
        let altitude = position.length() - spawn_surface_r;
        assert!(
            altitude >= -0.05,
            "player crossed beneath the seed 1337 spawn terrain after sustained physics: altitude={altitude:.3}m"
        );
        assert!(
            !contacts.is_empty(),
            "player should remain in terrain contact after several physics frames"
        );
    }
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

pub fn build_visual_mesh(tris: &[[[f32; 3]; 3]], colors: &[[[f32; 4]; 3]]) -> Mesh {
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

pub(crate) fn build_collider(tris: &[[[f32; 3]; 3]]) -> Collider {
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
    // Adjacent triangles form one surface. Treating their shared edges as
    // independent features can produce sideways contacts that stop a car.
    Collider::trimesh_with_config(vertices, indices, TrimeshFlags::FIX_INTERNAL_EDGES)
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
    let speed =
        shared::on_foot_prototype::OnFootPrototype::default().speed(input.sprint, slowed_by_water);

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
        *forces.linear_velocity_mut() = v + up * 14.0;
    }
}

/// Emit one high-signal report when the player's center crosses beneath the
/// terrain. Logging only the transition avoids flooding the console while the
/// body continues falling and preserves the values from the failure frame.
fn diagnose_player_fall(
    real_time: Res<Time<Real>>,
    fixed_time: Res<Time<Fixed>>,
    terrain: Option<Res<CollisionTerrain>>,
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
    let surface_radius = terrain.0.facet_radius(up, PLANET_RADIUS);
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
    input: Res<PlayerInput>,
    player_q: Query<(&Player, &Position, &Rotation)>,
    mut visual_q: Query<
        (&mut Transform, &mut shared::actor_animation::ActorPlayback),
        With<PlayerVisual>,
    >,
) {
    let Ok((player, position, body_rotation)) = player_q.single() else {
        return;
    };
    let Ok((mut visual, mut playback)) = visual_q.single_mut() else {
        return;
    };
    let up = position.0.normalize();
    let facing = Quat::from_mat3(&Mat3::from_cols(
        player.heading.cross(up),
        up,
        -player.heading,
    ));
    visual.translation = Vec3::NEG_Y * player_half_height();
    visual.rotation = body_rotation.0.inverse() * facing;
    playback.action = usize::from(input.fwd != 0);
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
        Self(false)
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
        let level = LevelData::from_artifact_bytes(include_bytes!("../assets/level_1337.bin"))
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
