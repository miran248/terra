use avian3d::prelude::*;
use bevy::prelude::*;
use bevy::render::mesh::VertexAttributeValues;
use crate::physics::RadialGravity;
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

impl Plugin for MapPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(AppState::Playing), setup_map)
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

    // Planet visual: displaced trimesh with per-vertex colors (flat-shaded, ico(60)).
    // Planet physics: high-subdivision trimesh from shared icosahedron, no Bevy mesh limits.
    let (visual_mesh, _physics_mesh, _planet) = build_planet_mesh(&terrain, &roads);
    let terrain_collider = build_terrain_collider(&terrain);
    commands.spawn((
        Mesh3d(meshes.add(visual_mesh)),
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
        terrain_collider,
        Transform::default(),
        Ground,
    ));

    // Water: transparent sphere at sea level.
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

    // Player: Dynamic capsule at habitable spawn.
    let start = terrain.habitable_spawn();
    let up = start.0;
    let capsule_radius = PLAYER_SIZE * 0.4;
    let capsule_half = capsule_radius + PLAYER_SIZE * 0.5; // radius + half-height
    let spawn_r = terrain.surface_radius(start) + capsule_half + 0.5;
    let spawn_pos = up * spawn_r;
    let spawn_rot = Quat::from_rotation_arc(Vec3::Y, up);
    commands.spawn((
        Mesh3d(meshes.add(Capsule3d::new(capsule_radius, PLAYER_SIZE))),
        MeshMaterial3d(materials.add(StandardMaterial::from_color(PLAYER_COLOR))),
        RigidBody::Dynamic,
        ColliderConstructor::Sphere { radius: PLAYER_SIZE * 0.5 },
        RadialGravity,
        Mass(80.0),
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

    // Bridges: Static cuboid colliders.
    let bridge_color = Color::srgb(0.35, 0.25, 0.18);
    let plank_mat = materials.add(StandardMaterial::from_color(bridge_color));
    let plank_mesh = meshes.add(Cuboid::new(100.0, 1.0, 6.0));
    for road in &roads.roads {
        if road.kind != shared::roads::PathKind::Bridge {
            continue;
        }
        let bstart = road.points.first().copied().unwrap();
        let bend = road.points.last().copied().unwrap();
        let start_r = terrain.surface_radius(bstart) + 0.5; // plank half-thickness above terrain
        let end_r = terrain.surface_radius(bend) + 0.5;
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
                let base_r = start_r + (end_r - start_r) * arc_t;
                let arch_hump =
                    4.0 * arc_t * (1.0 - arc_t) * (PLANET_RADIUS + 15.0 - base_r).max(0.0);
                let r = base_r + arch_hump;
                let face_up = Quat::from_rotation_arc(Vec3::Y, dir);
                let z_dir = face_up * Vec3::Z;
                let angle = z_dir.dot(fwd).clamp(-1.0, 1.0).acos();
                let sign = dir.dot(z_dir.cross(fwd)).signum();
                let spin = Quat::from_axis_angle(dir, angle * sign);
                commands.spawn((
                    Mesh3d(plank_mesh.clone()),
                    MeshMaterial3d(plank_mat.clone()),
                    RigidBody::Static,
                    ColliderConstructor::Cuboid {
                        x_length: 100.0,
                        y_length: 1.0,
                        z_length: 6.0,
                    },
                    Transform::from_translation(dir * r).with_rotation(spin * face_up),
                    Ground,
                ));
            }
            dist_travelled += seg_len;
        }
    }

    for s in &roads.settlements {
        commands.spawn((s.pos, Settlement { name: s.name.clone() }));
    }
    commands.insert_resource(roads);
    commands.insert_resource(terrain);
}

// ---- mesh builders ----

/// High-resolution watertight mesh for physics (no vertex duplication, no colors).
/// Subdivides the icosahedron further than the visual mesh so triangles are small
/// enough (~20 m edge at 2 km radius) that a capsule body can't slip through.
fn build_physics_terrain_mesh(terrain: &TerrainGen) -> Mesh {
    let mut mesh = Sphere::new(PLANET_RADIUS).mesh().ico(100).unwrap();
    if let Some(VertexAttributeValues::Float32x3(positions)) =
        mesh.attribute_mut(Mesh::ATTRIBUTE_POSITION)
    {
        for p in positions.iter_mut() {
            let pos = SpherePos::new(Vec3::from_array(*p));
            *p = (pos.0 * terrain.render_radius(pos)).to_array();
        }
    }
    mesh
}

/// High-resolution watertight trimesh collider for physics. Uses the shared
/// icosahedron subdivision (no Bevy vertex limit), displaced by the terrain heightmap.
/// At ~6 subdivisions the triangles are ~20 m across — too small for a capsule to slip through.
fn build_terrain_collider(terrain: &TerrainGen) -> ColliderConstructor {
    let mut tris = shared::planet::unit_icosphere_tris(5);
    for tri in &mut tris {
        for v in tri.iter_mut() {
            let pos = SpherePos::new(v.normalize());
            let r = terrain.render_radius(pos);
            *v = pos.0 * r;
        }
    }
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for (i, tri) in tris.iter().enumerate() {
        let base = i as u32 * 3;
        vertices.push(tri[0]);
        vertices.push(tri[1]);
        vertices.push(tri[2]);
        indices.push([base, base + 1, base + 2]);
    }
    ColliderConstructor::Trimesh { vertices, indices }
}

fn build_planet_mesh(
    terrain: &TerrainGen,
    roads: &shared::roads::Roads,
) -> (Mesh, Mesh, PlanetMesh) {
    let mut base = Sphere::new(PLANET_RADIUS).mesh().ico(60).unwrap();

    if let Some(VertexAttributeValues::Float32x3(positions)) =
        base.attribute_mut(Mesh::ATTRIBUTE_POSITION)
    {
        for p in positions.iter_mut() {
            let pos = SpherePos::new(Vec3::from_array(*p));
            *p = (pos.0 * terrain.render_radius(pos)).to_array();
        }
    }

    // Physics mesh: watertight trimesh (shared vertices, no gaps).
    let physics_mesh = base.clone();

    // Visual mesh: split vertices for flat faceted shading + baked colors.
    base.duplicate_vertices();
    base.compute_flat_normals();

    let Some(VertexAttributeValues::Float32x3(positions)) =
        base.attribute(Mesh::ATTRIBUTE_POSITION)
    else {
        return (base, physics_mesh, PlanetMesh::new(Vec::new()));
    };
    let positions = positions.clone();

    let faces: Vec<[Vec3; 3]> = positions
        .chunks_exact(3)
        .map(|t| [Vec3::from_array(t[0]), Vec3::from_array(t[1]), Vec3::from_array(t[2])])
        .collect();
    let planet = PlanetMesh::new(faces.clone());

    // Edge adjacency for road painting.
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
    base.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);

    (base, physics_mesh, planet)
}

const TOWN_RADIUS: f32 = 55.0;

fn arc(a: Vec3, b: Vec3) -> f32 {
    a.dot(b).clamp(-1.0, 1.0).acos() * PLANET_RADIUS
}

fn shortest_face_path(adj: &[Vec<usize>], u: usize, v: usize) -> Vec<usize> {
    use std::collections::VecDeque;
    const MAX_HOPS: usize = 4;
    let mut prev: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    let mut q = VecDeque::from([(u, 0usize)]);
    prev.insert(u, u);
    while let Some((cur, depth)) = q.pop_front() {
        if cur == v {
            let mut path = Vec::new();
            let mut c = v;
            while c != u {
                if c != v { path.push(c); }
                c = prev[&c];
            }
            path.reverse();
            return path;
        }
        if depth >= MAX_HOPS { continue; }
        for &n in &adj[cur] {
            prev.entry(n).or_insert_with(|| {
                q.push_back((n, depth + 1));
                cur
            });
        }
    }
    Vec::new()
}

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

// ---- player movement: uses Avian's Forces API ----

fn move_player(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut player_q: Query<(&mut SpherePos, &mut Player, &Transform, Forces)>,
) {
    let Ok((mut pos, mut player, tf, mut forces)) = player_q.single_mut() else { return };
    let dt = time.delta_secs();

    let up = pos.0;

    let mut turn = 0.0;
    if keys.pressed(KeyCode::KeyA) { turn += PLAYER_TURN * dt; }
    if keys.pressed(KeyCode::KeyD) { turn -= PLAYER_TURN * dt; }
    if turn != 0.0 {
        player.heading = Quat::from_axis_angle(up, turn) * player.heading;
    }
    player.heading = (player.heading - up * player.heading.dot(up)).normalize();

    let world_r = tf.translation.length();
    let underwater = world_r < PLANET_RADIUS;
    let sprint = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    let speed = PLAYER_SPEED
        * if underwater { 0.4 } else { 1.0 }
        * if sprint { 2.5 } else { 1.0 };

    let mut fwd = 0.0f32;
    if keys.pressed(KeyCode::KeyW) { fwd += 1.0; }
    if keys.pressed(KeyCode::KeyS) { fwd -= 1.0; }
    if fwd != 0.0 {
        let move_dir = player.heading * fwd.signum();
        pos.step_tangent(move_dir, speed * dt);
        let new_up = pos.0;
        player.heading = (player.heading - new_up * player.heading.dot(new_up)).normalize();
    }

    // Drive toward target SpherePos via Forces acceleration.
    // When not pressing W/S, target velocity is zero (stop on a dime).
    let target_on_sphere = pos.0 * world_r;
    let delta = target_on_sphere - tf.translation;
    let radial = delta.dot(up) * up;
    let tangent_delta = delta - radial;
    let target_vel = if fwd != 0.0 {
        tangent_delta / 0.2
    } else {
        Vec3::ZERO
    };
    let max_vel = speed * 1.5;
    let target_vel = if target_vel.length() > max_vel {
        target_vel.normalize_or_zero() * max_vel
    } else {
        target_vel
    };
    let current_vel = forces.linear_velocity();
    forces.apply_linear_acceleration((target_vel - current_vel) / dt.clamp(0.001, 1.0));
    // Damp radial drift: push back toward surface if floating above ground, but only
    // when above sea level — allows diving underwater.
    let surface_spawn = PLANET_RADIUS + PLAYER_SIZE * 0.4 + PLAYER_SIZE * 0.5;
    if world_r > surface_spawn && world_r > PLANET_RADIUS {
        forces.apply_linear_acceleration(-up * (world_r - surface_spawn) * 2.0);
    }

    if keys.just_pressed(KeyCode::Space) {
        let on_ground = forces.linear_velocity().length() < 3.0;
        if on_ground {
            forces.apply_linear_impulse(up * 30.0);
        }
    }
}

/// Set player mesh rotation after physics (rotation is locked, so Transform is safe).
fn orient_player(mut q: Query<(&SpherePos, &Player, &mut Transform), With<RigidBody>>) {
    for (pos, player, mut tf) in &mut q {
        let up = pos.0;
        tf.rotation = Quat::from_mat3(&Mat3::from_cols(
            player.heading.cross(up), up, -player.heading,
        ));
    }
}

// ---- camera ----

fn camera_follow(
    time: Res<Time>,
    mut player_q: Query<(&SpherePos, &Player, &Transform), Without<MainCamera>>,
    mut camera_q: Query<&mut Transform, With<MainCamera>>,
    mut light_q: Query<&mut Transform, (With<Sun>, Without<MainCamera>, Without<Player>)>,
) {
    let Ok((pos, player, tf)) = player_q.single() else { return };
    let Ok(mut cam_tf) = camera_q.single_mut() else { return };

    let up = pos.0;
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
