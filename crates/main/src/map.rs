use bevy::prelude::*;
use bevy::render::mesh::VertexAttributeValues;
use shared::planet::PlanetMesh;
use shared::sphere::{SpherePos, PLANET_RADIUS};
use shared::state::AppState;
use shared::terrain::TerrainGen;
use shared::theme;
use crate::constants::*;

#[derive(Component)]
pub struct Ground;

#[derive(Component)]
pub struct Sun;

/// Marks the main gameplay camera (distinct from the minimap render camera).
#[derive(Component)]
pub struct MainCamera;

/// A named settlement in the world (villages/towns). Carries its name for the minimap
/// and future labels/interaction.
#[derive(Component)]
pub struct Settlement {
    pub name: String,
}

/// Half-height of an actor's mesh, so it can be lifted to rest its base on the ground.
#[derive(Component, Clone, Copy)]
pub struct GroundOffset(pub f32);

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

    // Planet: displaced low-poly heightmap mesh; keep its triangles for actor grounding.
    let (planet_mesh, planet_tris) = build_planet_mesh(&terrain);
    commands.spawn((
        Mesh3d(meshes.add(planet_mesh)),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 0.95,
            ..default()
        })),
        Transform::default(),
        Ground,
    ));
    let planet = PlanetMesh::new(planet_tris);

    // Sun (follows the player, see camera_follow)
    commands.spawn((
        DirectionalLight { illuminance: 12_000.0, ..default() },
        Transform::from_xyz(1.0, 1.0, 1.0).looking_at(Vec3::ZERO, Vec3::Y),
        Sun,
        Ground,
    ));

    let start = terrain.habitable_spawn();
    let survivor_half = SURVIVOR_SIZE * 0.5 + SURVIVOR_SIZE * 0.4; // capsule length/2 + radius
    commands.spawn((
        Mesh3d(meshes.add(Capsule3d::new(SURVIVOR_SIZE * 0.4, SURVIVOR_SIZE))),
        MeshMaterial3d(materials.add(StandardMaterial::from_color(SURVIVOR_COLOR))),
        Transform::from_translation(grounded(&planet, start, survivor_half)),
        start,
        GroundOffset(survivor_half),
        Survivor {
            hp: SURVIVOR_HP,
            fire_timer: Timer::from_seconds(ATTACK_INTERVAL, TimerMode::Repeating),
            damage: ATTACK_DAMAGE,
            range: ATTACK_RANGE,
            heading: start.tangent_basis().1,
        },
    ));

    let roads = shared::roads::Roads::generate(&terrain);
    spawn_road_geometry(&mut commands, &mut meshes, &mut materials, &planet, &roads);
    commands.insert_resource(planet);
    commands.insert_resource(roads);
    commands.insert_resource(terrain);
}

/// Spawn the road network and settlements as flat ground patches lying on the *faceted*
/// terrain surface (matching the rendered mesh, so they aren't buried), facing up.
fn spawn_road_geometry(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    planet: &PlanetMesh,
    roads: &shared::roads::Roads,
) {
    let road_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.5, 0.42, 0.3),
        unlit: true,
        ..default()
    });
    // Patches overlap the ~40 m point spacing so the road reads as a continuous ribbon.
    let road_patch = meshes.add(Circle::new(26.0));
    for road in &roads.roads {
        for p in &road.points {
            commands.spawn((
                Mesh3d(road_patch.clone()),
                MeshMaterial3d(road_mat.clone()),
                ground_patch_transform(planet, *p),
                Ground,
            ));
        }
    }

    let town_mat = materials.add(StandardMaterial {
        base_color: theme::WARNING,
        unlit: true,
        ..default()
    });
    let town_patch = meshes.add(Circle::new(40.0)); // town footprint
    for s in &roads.settlements {
        // Visual ground patch (static transform; NOT a SpherePos so it isn't re-synced).
        commands.spawn((
            Mesh3d(town_patch.clone()),
            MeshMaterial3d(town_mat.clone()),
            ground_patch_transform(planet, s.pos),
            Ground,
        ));
        // Data marker the minimap reads (position + name), no mesh.
        commands.spawn((s.pos, Settlement { name: s.name.clone() }));
    }
}

/// Transform for a flat disc lying on the faceted terrain at `pos`, facing outward
/// (its local +Z aligned to the surface normal), lifted above the facet to avoid z-fighting.
fn ground_patch_transform(planet: &PlanetMesh, pos: SpherePos) -> Transform {
    let up = pos.0;
    let r = planet.facet_radius(up, PLANET_RADIUS).max(PLANET_RADIUS);
    Transform {
        translation: up * (r + 1.5),
        rotation: Quat::from_rotation_arc(Vec3::Z, up),
        ..default()
    }
}

/// Low-poly icosphere displaced by the terrain heightmap, flat-shaded with per-face
/// terrain colors (chunky faceted planet, not a smooth blurry ball). Returns the mesh
/// and its world-space triangles for actor grounding.
fn build_planet_mesh(terrain: &TerrainGen) -> (Mesh, Vec<[Vec3; 3]>) {
    let mut mesh = Sphere::new(PLANET_RADIUS).mesh().ico(40).unwrap();

    // Displace each vertex to its terrain radius.
    if let Some(VertexAttributeValues::Float32x3(positions)) =
        mesh.attribute_mut(Mesh::ATTRIBUTE_POSITION)
    {
        for p in positions.iter_mut() {
            let pos = SpherePos::new(Vec3::from_array(*p));
            *p = (pos.0 * terrain.surface_radius(pos)).to_array();
        }
    }

    // Split shared vertices so every triangle is independent -> flat faceted shading.
    mesh.duplicate_vertices();
    mesh.compute_flat_normals();

    // Capture triangles and color each face a single terrain color (at its centroid).
    let mut tris = Vec::new();
    if let Some(VertexAttributeValues::Float32x3(positions)) =
        mesh.attribute(Mesh::ATTRIBUTE_POSITION)
    {
        let positions = positions.clone();
        let mut colors = Vec::with_capacity(positions.len());
        for tri in positions.chunks_exact(3) {
            let a = Vec3::from_array(tri[0]);
            let b = Vec3::from_array(tri[1]);
            let c = Vec3::from_array(tri[2]);
            tris.push([a, b, c]);
            let color = terrain
                .color_at(SpherePos::new((a + b + c) / 3.0))
                .to_linear()
                .to_f32_array();
            colors.push(color);
            colors.push(color);
            colors.push(color);
        }
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    }

    (mesh, tris)
}

fn move_survivor(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut survivor_q: Query<(&mut SpherePos, &mut Survivor)>,
) {
    let Ok((mut pos, mut survivor)) = survivor_q.single_mut() else { return };
    let dt = time.delta_secs();

    let up = pos.0;

    // A/D rotate the heading about the local up axis (turn in place).
    let mut turn = 0.0;
    if keys.pressed(KeyCode::KeyA) { turn += SURVIVOR_TURN * dt; }
    if keys.pressed(KeyCode::KeyD) { turn -= SURVIVOR_TURN * dt; }
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
        pos.step_tangent(move_dir, SURVIVOR_SPEED * dt);
        // Parallel-transport the heading onto the new tangent plane so it never flips.
        let new_up = pos.0;
        survivor.heading = (survivor.heading - new_up * survivor.heading.dot(new_up)).normalize();
    }
}

/// Every actor stores its position as `SpherePos`; derive the world `Transform` from it,
/// lifted so the mesh rests on the terrain (never dipping below ground). Survivor
/// orientation is handled separately.
fn sync_sphere_transforms(
    planet: Res<PlanetMesh>,
    mut q: Query<(&SpherePos, Option<&GroundOffset>, &mut Transform), (Changed<SpherePos>, Without<Survivor>)>,
) {
    for (pos, offset, mut tf) in &mut q {
        let up = pos.0;
        let half = offset.map(|o| o.0).unwrap_or(0.0);
        tf.translation = grounded(&planet, *pos, half);
        tf.rotation = Quat::from_rotation_arc(Vec3::Y, up);
    }
}

/// World position resting an actor on the *rendered facet* at `pos`, clamped to sea level,
/// lifted by `half_height`. Uses the mesh triangles so actors never clip the low-poly ground.
fn grounded(planet: &PlanetMesh, pos: SpherePos, half_height: f32) -> Vec3 {
    let r = planet.facet_radius(pos.0, PLANET_RADIUS).max(PLANET_RADIUS);
    pos.0 * (r + half_height)
}

fn camera_follow(
    planet: Res<PlanetMesh>,
    mut survivor_q: Query<(&SpherePos, &Survivor, &GroundOffset, &mut Transform), Without<MainCamera>>,
    mut camera_q: Query<&mut Transform, With<MainCamera>>,
    mut light_q: Query<&mut Transform, (With<Sun>, Without<MainCamera>, Without<Survivor>)>,
) {
    let Ok((pos, survivor, offset, mut survivor_tf)) = survivor_q.single_mut() else { return };
    let Ok(mut cam_tf) = camera_q.single_mut() else { return };

    let up = pos.0;
    let ground = grounded(&planet, *pos, offset.0);
    // Feet point on the facet surface for camera framing and lighting.
    let feet = up * planet.facet_radius(up, PLANET_RADIUS).max(PLANET_RADIUS);

    // Orient the survivor mesh: capsule +Y = up, face the heading.
    survivor_tf.translation = ground;
    survivor_tf.rotation =
        Quat::from_mat3(&Mat3::from_cols(survivor.heading.cross(up), up, -survivor.heading));

    // Camera sits above and slightly behind the heading, looking down at the player.
    let cam_pos = feet + up * CAMERA_HEIGHT - survivor.heading * CAMERA_BACK;
    cam_tf.translation = cam_pos;
    cam_tf.look_at(feet, up);

    // Sun follows: shine down onto the player from above (well above peak relief).
    if let Ok(mut light_tf) = light_q.single_mut() {
        light_tf.translation = feet + up * 800.0;
        light_tf.look_at(feet, survivor.heading);
    }
}
