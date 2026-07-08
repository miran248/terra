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
pub struct Player {
    pub hp: f32,
    pub fire_timer: Timer,
    pub damage: f32,
    pub range: f32,
    /// Facing direction in the local tangent plane (heading), used for camera orientation.
    pub heading: Vec3,
    /// Altitude above the terrain surface (m). Zero when grounded; positive when jumping.
    pub altitude: f32,
    /// Vertical velocity for jump arcs (m/s, positive = away from planet center).
    pub vertical_vel: f32,
}

#[derive(Resource, Default)]
pub struct BridgeWalk {
    /// Each plank's direction (unit vector) and the surface radius it sits at.
    pub planks: Vec<(Vec3, f32)>,
}

#[derive(Resource)]
pub struct PlayerHp(pub f32);

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
                (move_player, sync_sphere_transforms, camera_follow)
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
    commands.insert_resource(PlayerHp(PLAYER_HP));

    let mut terrain = TerrainGen::new(PLANET_SEED);
    let roads = shared::roads::Roads::generate(&terrain);
    terrain.set_roads(&roads);

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

    // Planet: displaced low-poly heightmap mesh with roads/towns baked in; the returned
    // PlanetMesh is reused for actor grounding.
    let (planet_mesh, planet) = build_planet_mesh(&terrain, &roads);
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

    // Water: transparent sphere at sea level. Double-sided so it's visible both from
    // above (looking down onto the ocean) and from below (when the player is underwater).
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

    // Sun (follows the player, see camera_follow)
    commands.spawn((
        DirectionalLight { illuminance: 12_000.0, ..default() },
        Transform::from_xyz(1.0, 1.0, 1.0).looking_at(Vec3::ZERO, Vec3::Y),
        Sun,
        Ground,
    ));

    let start = terrain.habitable_spawn();
    let player_half = PLAYER_SIZE * 0.5 + PLAYER_SIZE * 0.4; // capsule length/2 + radius
    commands.spawn((
        Mesh3d(meshes.add(Capsule3d::new(PLAYER_SIZE * 0.4, PLAYER_SIZE))),
        MeshMaterial3d(materials.add(StandardMaterial::from_color(PLAYER_COLOR))),
        Transform::from_translation(grounded(&planet, start, terrain.surface_radius(start), player_half, 0.0)),
        start,
        GroundOffset(player_half),
        Player {
            hp: PLAYER_HP,
            fire_timer: Timer::from_seconds(ATTACK_INTERVAL, TimerMode::Repeating),
            damage: ATTACK_DAMAGE,
            range: ATTACK_RANGE,
            heading: start.tangent_basis().1,
            altitude: 0.0,
            vertical_vel: 0.0,
        },
    ));

    // Bridges: smooth arch from terrain-level start to terrain-level end. The curve
    // follows the great-circle arc between endpoints, rising above the water mid-span.
    let bridge_color = Color::srgb(0.35, 0.25, 0.18);
    let plank_mat = materials.add(StandardMaterial::from_color(bridge_color));
    let plank_mesh = meshes.add(Cuboid::new(100.0, 1.0, 6.0));
    let mut bridge_walk = BridgeWalk::default();
    for road in &roads.roads {
        if road.kind != shared::roads::PathKind::Bridge {
            continue;
        }
        let start = road.points.first().copied().unwrap();
        let end = road.points.last().copied().unwrap();
        let start_r = terrain.surface_radius(start) + 1.0;
        let end_r = terrain.surface_radius(end) + 1.0;
        // Total arc length along the bridge path.
        let total_len: f32 = road.points.windows(2).map(|s| s[0].distance(s[1])).sum();
        let mut dist_travelled = 0.0f32;
        for seg in road.points.windows(2) {
            let seg_len = seg[0].distance(seg[1]);
            let steps = (seg_len / 5.0).ceil().max(1.0) as usize;
            let fwd = (seg[1].0 - seg[0].0).normalize();
            for i in 0..=steps {
                let local_t = i as f32 / steps as f32;
                let dir = seg[0].0.lerp(seg[1].0, local_t).normalize();
                let arc_t = (dist_travelled + seg_len * local_t) / total_len;
                // Base radius: linear interpolation from start to end terrain.
                let base_r = start_r + (end_r - start_r) * arc_t;
                // Arch: parabolic hump in the middle, magnitude clamped to a minimum above water.
                let arch_hump =
                    4.0 * arc_t * (1.0 - arc_t) * (PLANET_RADIUS + 15.0 - base_r).max(0.0);
                let r = base_r + arch_hump;
                bridge_walk.planks.push((dir, r));
                let face_up = Quat::from_rotation_arc(Vec3::Y, dir);
                let z_dir = face_up * Vec3::Z;
                let angle = z_dir.dot(fwd).clamp(-1.0, 1.0).acos();
                let sign = dir.dot(z_dir.cross(fwd)).signum();
                let spin = Quat::from_axis_angle(dir, angle * sign);
                commands.spawn((
                    Mesh3d(plank_mesh.clone()),
                    MeshMaterial3d(plank_mat.clone()),
                    Transform::from_translation(dir * r).with_rotation(spin * face_up),
                    Ground,
                ));
            }
            dist_travelled += seg_len;
        }
    }
    commands.insert_resource(bridge_walk);

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
/// drape over the real surface instead of floating as separate discs). Returns the mesh,
/// its world triangles, and the `PlanetMesh` (built here and reused for grounding).
fn build_planet_mesh(
    terrain: &TerrainGen,
    roads: &shared::roads::Roads,
) -> (Mesh, PlanetMesh) {
    let mut mesh = Sphere::new(PLANET_RADIUS).mesh().ico(60).unwrap();

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

    let Some(VertexAttributeValues::Float32x3(positions)) =
        mesh.attribute(Mesh::ATTRIBUTE_POSITION)
    else {
        return (mesh, PlanetMesh::new(Vec::new()));
    };
    let positions = positions.clone();
    let faces: Vec<[Vec3; 3]> = positions
        .chunks_exact(3)
        .map(|t| [Vec3::from_array(t[0]), Vec3::from_array(t[1]), Vec3::from_array(t[2])])
        .collect();
    let planet = PlanetMesh::new(faces.clone());

    // Edge adjacency: faces sharing an edge (two identical vertex positions).
    let mut edge_faces: std::collections::HashMap<(u64, u64), Vec<usize>> =
        std::collections::HashMap::new();
    for (fi, f) in faces.iter().enumerate() {
        for (x, y) in [(0, 1), (1, 2), (2, 0)] {
            edge_faces.entry(edge_key(f[x], f[y])).or_default().push(fi);
        }
    }
    let adj: Vec<Vec<usize>> = faces
        .iter()
        .enumerate()
        .map(|(fi, f)| {
            let mut out = Vec::new();
            for (x, y) in [(0, 1), (1, 2), (2, 0)] {
                if let Some(a) = edge_faces.get(&edge_key(f[x], f[y])) {
                    out.extend(a.iter().copied().filter(|&nf| nf != fi));
                }
            }
            out.sort_unstable();
            out.dedup();
            out
        })
        .collect();
    let edge_adjacent = |u: usize, v: usize| adj[u].contains(&v);

    // Rasterize each road as an ordered chain of faces. Where two consecutive faces touch
    // only at a vertex (not edge-adjacent), insert a shared edge-neighbour so the painted
    // road stays edge-connected (no vertex-only diagonal gaps). Bridges are skipped here —
    // they're spawned as separate elevated geometry later.
    let mut road_faces: std::collections::HashSet<usize> = std::collections::HashSet::new();
    for road in &roads.roads {
        if road.kind == shared::roads::PathKind::Bridge {
            continue;
        }
        let mut chain: Vec<usize> = Vec::new();
        for seg in road.points.windows(2) {
            let (a, b) = (seg[0].0, seg[1].0);
            let steps = (arc(a, b) / 2.0).ceil().max(1.0) as usize;
            for i in 0..=steps {
                let p = a.lerp(b, i as f32 / steps as f32).normalize();
                if let Some(fi) = planet.face_at(p) {
                    if chain.last() != Some(&fi) {
                        chain.push(fi);
                    }
                }
            }
        }
        for w in chain.windows(2) {
            let (u, v) = (w[0], w[1]);
            road_faces.insert(u);
            road_faces.insert(v);
            if !edge_adjacent(u, v) {
                for f in shortest_face_path(&adj, u, v) {
                    road_faces.insert(f);
                }
            }
        }
        if let Some(&first) = chain.first() {
            road_faces.insert(first);
        }
    }

    // Town faces: any face whose centroid is within a settlement radius.
    let mut town_faces: std::collections::HashSet<usize> = std::collections::HashSet::new();
    for (fi, f) in faces.iter().enumerate() {
        let centroid = (f[0] + f[1] + f[2]) / 3.0;
        if roads
            .settlements
            .iter()
            .any(|s| arc(centroid.normalize(), s.pos.0) <= TOWN_RADIUS)
        {
            town_faces.insert(fi);
        }
    }

    let mut colors = Vec::with_capacity(positions.len());
    for (fi, f) in faces.iter().enumerate() {
        let centroid = SpherePos::new((f[0] + f[1] + f[2]) / 3.0);
        let color = if town_faces.contains(&fi) {
            theme::WARNING
        } else if road_faces.contains(&fi) {
            Color::srgb(0.5, 0.42, 0.3)
        } else {
            terrain.color_at(centroid)
        }
        .to_linear()
        .to_f32_array();
        colors.push(color);
        colors.push(color);
        colors.push(color);
    }
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);

    (mesh, planet)
}

const TOWN_RADIUS: f32 = 55.0;

/// Arc-length (meters) between two unit directions.
fn arc(a: Vec3, b: Vec3) -> f32 {
    a.dot(b).clamp(-1.0, 1.0).acos() * PLANET_RADIUS
}

/// Shortest chain of faces from `u` to `v` via edge adjacency (BFS, bounded), excluding
/// `u` and `v` themselves — used to fill any gap when consecutive rasterized road faces
/// aren't edge-adjacent, guaranteeing the painted road is connected.
fn shortest_face_path(adj: &[Vec<usize>], u: usize, v: usize) -> Vec<usize> {
    use std::collections::VecDeque;
    const MAX_HOPS: usize = 4;
    let mut prev: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    let mut q = VecDeque::from([(u, 0usize)]);
    prev.insert(u, u);
    while let Some((cur, depth)) = q.pop_front() {
        if cur == v {
            // Reconstruct, dropping the endpoints.
            let mut path = Vec::new();
            let mut c = v;
            while c != u {
                if c != v {
                    path.push(c);
                }
                c = prev[&c];
            }
            path.reverse();
            return path;
        }
        if depth >= MAX_HOPS {
            continue;
        }
        for &n in &adj[cur] {
            prev.entry(n).or_insert_with(|| {
                q.push_back((n, depth + 1));
                cur
            });
        }
    }
    Vec::new()
}

/// Undirected edge key from two vertex positions (quantized so faces that share an edge
/// produce the same key — `duplicate_vertices` keeps shared edges bit-identical).
fn edge_key(a: Vec3, b: Vec3) -> (u64, u64) {
    let q = |v: Vec3| -> u64 {
        let x = (v.x * 4.0).round() as i64 as u64;
        let y = (v.y * 4.0).round() as i64 as u64;
        let z = (v.z * 4.0).round() as i64 as u64;
        x.wrapping_mul(73856093) ^ y.wrapping_mul(19349663) ^ z.wrapping_mul(83492791)
    };
    let (ka, kb) = (q(a), q(b));
    (ka.min(kb), ka.max(kb))
}

fn move_player(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    terrain: Res<TerrainGen>,
    mut player_q: Query<(&mut SpherePos, &mut Player)>,
) {
    let Ok((mut pos, mut player)) = player_q.single_mut() else { return };
    let dt = time.delta_secs();

    let up = pos.0;

    // A/D rotate the heading about the local up axis (turn in place).
    let mut turn = 0.0;
    if keys.pressed(KeyCode::KeyA) { turn += PLAYER_TURN * dt; }
    if keys.pressed(KeyCode::KeyD) { turn -= PLAYER_TURN * dt; }
    if turn != 0.0 {
        player.heading = Quat::from_axis_angle(up, turn) * player.heading;
    }

    // Keep heading tangent to the surface (re-project + renormalize).
    player.heading = (player.heading - up * player.heading.dot(up)).normalize();

    // Speed: slow in water, sprint on shift.
    let underwater = terrain.surface_radius(*pos) < PLANET_RADIUS;
    let sprint = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    let speed = PLAYER_SPEED
        * if underwater { 0.4 } else { 1.0 }
        * if sprint { 2.5 } else { 1.0 };

    // W/S move forward/back along the heading.
    let mut fwd = 0.0f32;
    if keys.pressed(KeyCode::KeyW) { fwd += 1.0; }
    if keys.pressed(KeyCode::KeyS) { fwd -= 1.0; }
    if fwd != 0.0 {
        let move_dir = player.heading * fwd.signum();
        pos.step_tangent(move_dir, speed * dt);
        // Parallel-transport the heading onto the new tangent plane so it never flips.
        let new_up = pos.0;
        player.heading = (player.heading - new_up * player.heading.dot(new_up)).normalize();
    }

    // Jump: space launches upward from the surface.
    let on_ground = player.altitude <= 0.0;
    if keys.just_pressed(KeyCode::Space) && on_ground {
        player.vertical_vel = if underwater { 40.0 } else { 25.0 };
        player.altitude = 0.1; // nudge above ground
    }

    // Fall/jump arc: apply gravity, integrate altitude.
    if player.altitude > 0.0 || player.vertical_vel != 0.0 {
        let grav = 25.0;
        player.vertical_vel -= grav * dt;
        player.altitude += player.vertical_vel * dt;
        if player.altitude <= 0.0 {
            player.altitude = 0.0;
            player.vertical_vel = 0.0;
        }
    }
}

/// Every actor stores its position as `SpherePos`; derive the world `Transform` from it,
/// lifted so the mesh rests on the terrain (never dipping below ground). Player
/// orientation is handled separately.
fn sync_sphere_transforms(
    planet: Res<PlanetMesh>,
    terrain: Res<TerrainGen>,
    mut q: Query<(&SpherePos, Option<&GroundOffset>, &mut Transform), (Changed<SpherePos>, Without<Player>)>,
) {
    for (pos, offset, mut tf) in &mut q {
        let up = pos.0;
        let half = offset.map(|o| o.0).unwrap_or(0.0);
        tf.translation = grounded(&planet, *pos, terrain.surface_radius(*pos), half, 0.0);
        tf.rotation = Quat::from_rotation_arc(Vec3::Y, up);
    }
}

/// World position resting an actor on the rendered facet surface, plus any altitude offset.
fn grounded(planet: &PlanetMesh, pos: SpherePos, _true_surface_r: f32, half_height: f32, altitude: f32) -> Vec3 {
    let r = planet.facet_radius(pos.0, PLANET_RADIUS);
    pos.0 * (r + half_height + altitude)
}

/// If `pos` is near a bridge plank and the terrain below is lower than the bridge,
/// return the top surface radius of the bridge. Otherwise return `None`.
fn bridge_radius(pos: SpherePos, bridge: &BridgeWalk, facet_r: f32) -> Option<f32> {
    const WALK_RANGE: f32 = 50.0; // m, half the plank width
    let threshold = (WALK_RANGE / PLANET_RADIUS).cos();
    for (dir, r) in &bridge.planks {
        if dir.dot(pos.0) >= threshold {
            // Only walk on bridge if the terrain is below it (not on land above the bridge).
            let bridge_top = r + 0.5; // plank half-thickness
            if facet_r < bridge_top {
                return Some(bridge_top);
            }
        }
    }
    None
}

fn camera_follow(
    planet: Res<PlanetMesh>,
    bridge: Res<BridgeWalk>,
    time: Res<Time>,
    mut player_q: Query<(&SpherePos, &Player, &GroundOffset, &mut Transform), Without<MainCamera>>,
    mut camera_q: Query<&mut Transform, With<MainCamera>>,
    mut light_q: Query<&mut Transform, (With<Sun>, Without<MainCamera>, Without<Player>)>,
) {
    let Ok((pos, player, offset, mut player_tf)) = player_q.single_mut() else { return };
    let Ok(mut cam_tf) = camera_q.single_mut() else { return };

    let up = pos.0;
    // Walk on bridge surface if nearby AND the terrain is below the bridge.
    let facet_r = planet.facet_radius(up, PLANET_RADIUS);
    let surface_r = bridge_radius(*pos, &bridge, facet_r).unwrap_or(facet_r);
    let ground = up * (surface_r + offset.0 + player.altitude);
    let feet = up * (surface_r + player.altitude);

    // Orient the player mesh: capsule +Y = up, face the heading.
    player_tf.translation = ground;
    player_tf.rotation =
        Quat::from_mat3(&Mat3::from_cols(player.heading.cross(up), up, -player.heading));

    // Camera sits above and behind the heading, aimed ahead. Snap position so the
    // player stays centered during rotation; smooth the look target for fluid feel.
    cam_tf.translation = feet + up * CAMERA_HEIGHT - player.heading * CAMERA_BACK;
    let look_target = feet + player.heading * CAMERA_LOOK_AHEAD;
    let t = 1.0 - (-6.0 * time.delta_secs()).exp();
    let current_look = cam_tf.rotation * -Vec3::Z + cam_tf.translation;
    cam_tf.look_at(current_look.lerp(look_target, t), up);

    // Sun follows: shine down onto the player from above (well above peak relief).
    if let Ok(mut light_tf) = light_q.single_mut() {
        light_tf.translation = feet + up * 800.0;
        light_tf.look_at(feet, player.heading);
    }
}
