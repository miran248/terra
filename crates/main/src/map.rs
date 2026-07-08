use bevy::prelude::*;
use bevy::platform::collections::HashMap;
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
    let roads = shared::roads::Roads::generate(&terrain);

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
    let (planet_mesh, planet_tris) = build_planet_mesh(&terrain, &roads);
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

    // Settlements are baked into the planet mesh color; spawn only the data markers the
    // minimap reads (position + name).
    for s in &roads.settlements {
        commands.spawn((s.pos, Settlement { name: s.name.clone() }));
    }
    commands.insert_resource(planet);
    commands.insert_resource(roads);
    commands.insert_resource(terrain);
}

/// Low-poly icosphere displaced by the terrain heightmap, flat-shaded with per-face
/// terrain colors — with roads and settlements painted directly into the mesh (so they
/// drape over the real surface instead of floating as separate discs). Returns the mesh
/// and its world-space triangles for actor grounding.
fn build_planet_mesh(terrain: &TerrainGen, roads: &shared::roads::Roads) -> (Mesh, Vec<[Vec3; 3]>) {
    let mut mesh = Sphere::new(PLANET_RADIUS).mesh().ico(40).unwrap();
    let paint = RoadPaint::new(roads);

    // Displace each vertex to its terrain radius.
    if let Some(VertexAttributeValues::Float32x3(positions)) =
        mesh.attribute_mut(Mesh::ATTRIBUTE_POSITION)
    {
        for p in positions.iter_mut() {
            let pos = SpherePos::new(Vec3::from_array(*p));
            *p = (pos.0 * terrain.render_radius(pos)).to_array();
        }
    }

    // Split shared vertices so every triangle is independent -> flat faceted shading.
    mesh.duplicate_vertices();
    mesh.compute_flat_normals();

    // Capture triangles and color each face: solid road/town color if a road actually
    // crosses the triangle (so road faces connect only along the edge the road passes
    // through — no blending, no vertex-fan spread), else the biome color.
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
            let centroid = SpherePos::new((a + b + c) / 3.0);
            let color = match paint.face_kind(a, b, c, centroid) {
                Some(FaceKind::Town) => theme::WARNING,
                Some(FaceKind::Road) => Color::srgb(0.5, 0.42, 0.3),
                None => terrain.color_at(centroid),
            }
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

/// Spatial hash of road *segments* and settlements, so per-face mesh coloring can ask
/// "does a road cross this triangle?" — giving crisp road faces connected only along the
/// edge a road passes through (no blending, no vertex-fan spread).
struct RoadPaint {
    roads: HashMap<(i32, i32), Vec<[Vec3; 2]>>,
    towns: HashMap<(i32, i32), Vec<Vec3>>,
}

const PAINT_CELL: f32 = 60.0; // grid cell size in meters (~ face scale)
const TOWN_RADIUS: f32 = 55.0;

enum FaceKind {
    Road,
    Town,
}

impl RoadPaint {
    fn new(roads: &shared::roads::Roads) -> Self {
        let mut r: HashMap<(i32, i32), Vec<[Vec3; 2]>> = HashMap::new();
        for road in &roads.roads {
            for seg in road.points.windows(2) {
                let (a, b) = (seg[0].0, seg[1].0);
                // Bucket the segment into every cell its endpoints/midpoint fall in.
                let mut cells = vec![paint_key(a), paint_key(b), paint_key((a + b).normalize())];
                cells.sort();
                cells.dedup();
                for c in cells {
                    r.entry(c).or_default().push([a, b]);
                }
            }
        }
        let mut t: HashMap<(i32, i32), Vec<Vec3>> = HashMap::new();
        for s in &roads.settlements {
            t.entry(paint_key(s.pos.0)).or_default().push(s.pos.0);
        }
        Self { roads: r, towns: t }
    }

    /// Classify a face: `Town` if its centroid is within a town, `Road` if a road segment
    /// passes through the triangle (so neighbouring road faces meet on the shared edge the
    /// road crosses), else `None`.
    fn face_kind(&self, a: Vec3, b: Vec3, c: Vec3, centroid: SpherePos) -> Option<FaceKind> {
        let (li, oi) = paint_key(centroid.0);
        // Town takes priority (centroid distance).
        for dl in -1..=1 {
            for doo in -1..=1 {
                if let Some(pts) = self.towns.get(&(li + dl, oi + doo)) {
                    if pts.iter().any(|q| arc(centroid.0, *q) <= TOWN_RADIUS) {
                        return Some(FaceKind::Town);
                    }
                }
            }
        }
        // Road if any segment (sampled) has a point inside this triangle's solid angle.
        let tri = [a, b, c];
        for dl in -1..=1 {
            for doo in -1..=1 {
                if let Some(segs) = self.roads.get(&(li + dl, oi + doo)) {
                    for s in segs {
                        if segment_crosses_triangle(s[0], s[1], &tri) {
                            return Some(FaceKind::Road);
                        }
                    }
                }
            }
        }
        None
    }
}

/// True if the great-circle segment `p0`→`p1` passes through the spherical triangle `tri`
/// (sampled finely; road points are ~40 m so a handful of samples covers a face).
fn segment_crosses_triangle(p0: Vec3, p1: Vec3, tri: &[Vec3; 3]) -> bool {
    const SAMPLES: usize = 12;
    for i in 0..=SAMPLES {
        let t = i as f32 / SAMPLES as f32;
        let p = p0.lerp(p1, t).normalize();
        if dir_in_triangle(p, tri) {
            return true;
        }
    }
    false
}

/// Whether unit direction `p` projects inside the spherical triangle `tri` (ray from the
/// planet centre through `p` hits the triangle). Reuses the shared ray-triangle test.
fn dir_in_triangle(p: Vec3, tri: &[Vec3; 3]) -> bool {
    shared::planet::ray_triangle_radius(p, tri).is_some()
}

fn paint_key(dir: Vec3) -> (i32, i32) {
    let cell = PAINT_CELL / PLANET_RADIUS;
    let lat = dir.y.clamp(-1.0, 1.0).acos();
    let lon = dir.z.atan2(dir.x) + std::f32::consts::PI;
    ((lat / cell).floor() as i32, (lon / cell).floor() as i32)
}

/// Arc-length (meters) between two unit directions.
fn arc(a: Vec3, b: Vec3) -> f32 {
    a.dot(b).clamp(-1.0, 1.0).acos() * PLANET_RADIUS
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
