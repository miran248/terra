use bevy_color::ColorToComponents;
use bevy_math::Vec3;
use shared::level::{LevelData, RoadData, SettlementData};
use shared::planet::{PlanetMesh, unit_icosphere_tris};
use shared::roads::Roads;
use shared::sphere::SpherePos;
use shared::terrain::{Terrain, TerrainGen, transition};
use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::PathBuf;

// ---- bitset for face flags ----

const FACE_FLAG_ROAD: u8 = 1;
const FACE_FLAG_TOWN: u8 = 2;
const FACE_FLAG_BRIDGE: u8 = 4;

struct BitSet(Vec<u64>);

impl BitSet {
    fn new(n: usize) -> Self { Self(vec![0; n.div_ceil(64)]) }
    fn insert(&mut self, i: usize) { self.0[i >> 6] |= 1u64 << (i & 63); }
    fn contains(&self, i: usize) -> bool { self.0[i >> 6] & (1u64 << (i & 63)) != 0 }
    fn iter<'a>(&'a self) -> impl Iterator<Item = usize> + 'a {
        (0..self.0.len() * 64).filter(|i| self.contains(*i))
    }
}

fn main() {
    let seed: u32 = std::env::var("PLANET_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1337);

    let out = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| {
                // Default: write next to the source (canonical, not CWD-dependent).
                let dir = std::env!("CARGO_MANIFEST_DIR");
                format!("{dir}/../main/assets/level_{seed}.bin")
            }),
    );

    let mut terrain = TerrainGen::new(seed);
    let roads = Roads::generate(&terrain);
    terrain.set_roads(&roads);

    let baked = terrain.bake_terrain();

    let sub = 6;
    let unit_tris = unit_icosphere_tris(sub);
    let planet = PlanetMesh::new(unit_tris.clone());
    let adj = build_face_adjacency(&planet);

    let n = unit_tris.len();
    debug_assert!(adj.iter().all(|a| !a.contains(&u32::MAX)), "face adjacency incomplete");
    let road_faces = paint_road_faces(&planet, &adj, &roads, n);
    let town_faces = paint_town_faces(&planet, &roads, n);
    let bridge_faces = paint_bridge_faces(&planet, &adj, &roads, n);
    // Find bridge approach faces: terrain faces near bridge endpoints, flood BFS
    // from endpoint terrain faces outward a few rings for a smooth entry ramp.
    let bridge_entry_faces = bridge_approach_faces(&planet, &adj, &roads, &bridge_faces, n);

    // -- terrain layer: compute base types per face, then region-consolidate --
    let base_types: Vec<Terrain> = (0..n).map(|fi| {
        let centroid = SpherePos::new((unit_tris[fi][0] + unit_tris[fi][1] + unit_tris[fi][2]) / 3.0);
        terrain.base_classify(centroid)
    }).collect();
    // Assign each face a region ID — flood-fill same base_types that are edge-adjacent.
    let region_ids = build_regions(n, &base_types, &adj);
    // For each face, the final terrain type: check neighbor regions for transitions.
    let face_types: Vec<Terrain> = (0..n).map(|fi| {
        let base = base_types[fi];
        let ri = region_ids[fi];
        for &nb in &adj[fi] {
            let nb = nb as usize;
            if nb == u32::MAX as usize { continue; }
            if region_ids[nb] != ri {
                let t = transition(base, base_types[nb]);
                if t != base { return t; }
            }
        }
        base
    }).collect();

    let mut terrain_tris = Vec::with_capacity(n);
    let mut terrain_colors = Vec::with_capacity(n);
    let mut terrain_features = Vec::with_capacity(n);

    for (fi, [a, b, c]) in unit_tris.iter().enumerate() {
        let sa = SpherePos::new(*a);
        let sb = SpherePos::new(*b);
        let sc = SpherePos::new(*c);

        let mut ra = terrain.render_radius(sa);
        let mut rb = terrain.render_radius(sb);
        let mut rc = terrain.render_radius(sc);

        // Raise town + bridge entry faces to terrain surface so there are no gaps.
        if town_faces.contains(fi) || bridge_entry_faces.contains(fi) {
            let cent = (sa.0 + sb.0 + sc.0) / 3.0;
            let entry_r = terrain.surface_radius(SpherePos::new(cent)).max(shared::sphere::PLANET_RADIUS);
            ra = entry_r.max(ra);
            rb = entry_r.max(rb);
            rc = entry_r.max(rc);
        }

        let face_type = face_types[fi];
        let color = if town_faces.contains(fi) {
            shared::theme::WARNING.to_linear().to_f32_array()
        } else if road_faces.contains(fi) {
            bevy_color::Color::srgb(0.5, 0.42, 0.3).to_linear().to_f32_array()
        } else {
            face_type.color().to_linear().to_f32_array()
        };

        terrain_tris.push([
            (sa.0 * ra).to_array(),
            (sb.0 * rb).to_array(),
            (sc.0 * rc).to_array(),
        ]);
        terrain_colors.push(color);
        let mut feat = 0u8;
        if road_faces.contains(fi) { feat |= FACE_FLAG_ROAD; }
        if town_faces.contains(fi) { feat |= FACE_FLAG_TOWN; }
        if bridge_faces.contains(fi) { feat |= FACE_FLAG_BRIDGE; }
        terrain_features.push(feat);
    }

    // -- features layer: bridge decks + settlement flat discs --
    let (feature_tris, feature_colors) = build_features(&planet, &roads, &town_faces, &terrain);
    let feat_count = feature_tris.len();

    let unit_tris_arr: Vec<[[f32; 3]; 3]> = unit_tris.iter().map(|[a,b,c]| [a.to_array(), b.to_array(), c.to_array()]).collect();

    let data = LevelData {
        terrain_tris,
        terrain_colors,
        terrain_features,
        unit_tris: unit_tris_arr,
        feature_tris,
        feature_colors,
        settlements: roads
            .settlements
            .iter()
            .map(|s| SettlementData {
                name: s.name.clone(),
                pos: s.pos.0.to_array(),
            })
            .collect(),
        roads: roads
            .roads
            .iter()
            .map(|r| RoadData {
                points: r.points.iter().map(|p| p.0.to_array()).collect(),
                is_bridge: matches!(r.kind, shared::roads::PathKind::Bridge),
            })
            .collect(),
        baked_verts: baked,
    };

    let bytes = postcard::to_allocvec(&data).expect("serialize level");
    fs::create_dir_all(out.parent().unwrap()).ok();
    fs::write(&out, &bytes).expect("write level file");
    println!(
        "Wrote {} terrain tris, {} feature tris → {}",
        unit_tris.len(),
        feat_count,
        out.display()
    );
}

// ---- feature generation ----

fn build_features(
    planet: &PlanetMesh,
    roads: &Roads,
    town_faces: &BitSet,
    terrain: &TerrainGen,
) -> (Vec<[[f32; 3]; 3]>, Vec<[f32; 4]>) {
    let mut tris = Vec::new();
    let mut colors = Vec::new();

    let bridge_color = bevy_color::Color::srgb(0.35, 0.25, 0.18).to_linear().to_f32_array();
    let deck_half_width = 20.0; // meters, ~2x road face width

    // Bridge decks: arched triangle strip from endpoint terrain heights.
    for road in &roads.roads {
        if !matches!(road.kind, shared::roads::PathKind::Bridge) {
            continue;
        }
        build_bridge_strip(
            &mut tris, &mut colors, &road.points,
            &terrain, deck_half_width, bridge_color,
        );
    }

    // Settlement flat discs: uniform radius per settlement.
    let town_color = shared::theme::WARNING.to_linear().to_f32_array();
    // Precompute disc radii per settlement.
    let settlement_radii: Vec<f32> = roads.settlements.iter()
        .map(|s| terrain.surface_radius(s.pos) + 0.5)
        .collect();

    for fi in town_faces.iter() {
        let [a, b, c] = planet.tris()[fi];
        let cent = SpherePos::new(((a + b + c) / 3.0).normalize());
        // Match face to nearest settlement for uniform disc radius.
        let r = roads.settlements.iter().zip(&settlement_radii)
            .min_by(|(a, _), (b, _)| a.pos.distance(cent).partial_cmp(&b.pos.distance(cent)).unwrap())
            .map(|(_, &r)| r)
            .unwrap_or(shared::sphere::PLANET_RADIUS + 0.5);
        tris.push([
            (a.normalize() * r).to_array(),
            (b.normalize() * r).to_array(),
            (c.normalize() * r).to_array(),
        ]);
        colors.push(town_color);
    }

    (tris, colors)
}

// ---- face painting helpers ----

// ---- bridge triangle strip ----

fn build_bridge_strip(
    tris: &mut Vec<[[f32; 3]; 3]>,
    colors: &mut Vec<[f32; 4]>,
    points: &[SpherePos],
    terrain: &TerrainGen,
    half_width: f32,
    color: [f32; 4],
) {
    let start_r = terrain.surface_radius(points[0]);
    let end_r = terrain.surface_radius(points[points.len() - 1]);
    let sea = shared::sphere::PLANET_RADIUS;

    // Dense sample: every 2m along the path.
    let sample_spacing = 2.0;
    let mut dirs: Vec<Vec3> = Vec::new();
    let mut pos = 0.0f32;
    for seg in points.windows(2) {
        let seg_len = seg[0].distance(seg[1]);
        while pos <= seg_len + 1e-6 {
            let t = (pos / seg_len).min(1.0);
            dirs.push(shared::sphere::slerp(seg[0], seg[1], t).0);
            if pos >= seg_len { break; }
            pos = (pos + sample_spacing).min(seg_len);
        }
        pos -= seg_len;
    }
    // Ensure last point is included.
    let last_dir = points.last().unwrap().0;
    if (dirs.last().unwrap().dot(last_dir)).abs() < 0.999 {
        dirs.push(last_dir);
    }

    // Build vertex rings: each ring has center, left, right. Use averaged forward direction
    // between adjacent rings for smooth lateral offsets.
    let n = dirs.len();
    let mut rings: Vec<(Vec3, Vec3, Vec3)> = Vec::new(); // (mid, left, right)
    for i in 0..n {
        let dir = dirs[i];
        let arc_t = i as f32 / (n - 1).max(1) as f32;
        let arch = 2.0 * (4.0 * arc_t * (1.0 - arc_t));
        let r = (start_r + (end_r - start_r) * arc_t).max(sea) + arch;
        let up = dir.normalize();

        // Smooth forward: average of pair vectors to neighbors.
        let fwd = if i == 0 {
            (dirs[1] - dirs[0]).normalize()
        } else if i == n - 1 {
            (dirs[n - 1] - dirs[n - 2]).normalize()
        } else {
            let prev = (dirs[i] - dirs[i - 1]).normalize();
            let next = (dirs[i + 1] - dirs[i]).normalize();
            (prev + next).normalize_or((dirs[1] - dirs[0]).normalize())
        };

        let left = fwd.cross(up).normalize_or(Vec3::X) * (half_width / r);
        let right = -left;
        let left_pt = (dir + left).normalize() * r;
        let right_pt = (dir + right).normalize() * r;
        rings.push((dir * r, left_pt, right_pt));
    }

    // Triangle strip.
    for pair in rings.windows(2) {
        let (_, l0, r0) = pair[0];
        let (_, l1, r1) = pair[1];
        tris.push([l0.to_array(), r0.to_array(), l1.to_array()]);
        colors.push(color);
        tris.push([r0.to_array(), r1.to_array(), l1.to_array()]);
        colors.push(color);
    }
}

fn build_regions(n: usize, types: &[Terrain], adj: &[[u32; 3]]) -> Vec<usize> {
    let mut ids = vec![usize::MAX; n];
    let mut next_id = 0usize;
    for fi in 0..n {
        if ids[fi] != usize::MAX { continue; }
        let t = types[fi];
        let mut q = VecDeque::from([fi]);
        ids[fi] = next_id;
        while let Some(cur) = q.pop_front() {
            for &nb in &adj[cur] {
                let nb = nb as usize;
                if nb == u32::MAX as usize { continue; }
                if ids[nb] == usize::MAX && types[nb] == t {
                    ids[nb] = next_id;
                    q.push_back(nb);
                }
            }
        }
        next_id += 1;
    }
    ids
}

fn bridge_approach_faces(
    planet: &PlanetMesh,
    adj: &[[u32; 3]],
    roads: &Roads,
    bridge_faces: &BitSet,
    n: usize,
) -> BitSet {
    let mut approach = BitSet::new(n);
    for road in &roads.roads {
        if !matches!(road.kind, shared::roads::PathKind::Bridge) {
            continue;
        }
        // Start face at the first road point on land.
        let pt = road.points.first().unwrap().0;
        if let Some(fi) = planet.face_at(pt) {
            if !bridge_faces.contains(fi) {
                flood_bfs(&mut approach, adj, fi, bridge_faces, 6, n);
            }
        }
        let pt = road.points.last().unwrap().0;
        if let Some(fi) = planet.face_at(pt) {
            if !bridge_faces.contains(fi) {
                flood_bfs(&mut approach, adj, fi, bridge_faces, 6, n);
            }
        }
    }
    approach
}

fn flood_bfs(
    out: &mut BitSet,
    adj: &[[u32; 3]],
    start: usize,
    exclude: &BitSet,
    rings: usize,
    n: usize,
) {
    let mut q = VecDeque::from([(start, 0usize)]);
    let mut seen = BitSet::new(n);
    seen.insert(start);
    while let Some((cur, depth)) = q.pop_front() {
        if depth > rings { continue; }
        out.insert(cur);
        for &n in &adj[cur] {
            let n = n as usize;
            if n == u32::MAX as usize { continue; }
            if !seen.contains(n) && !exclude.contains(n) {
                seen.insert(n);
                q.push_back((n, depth + 1));
            }
        }
    }
}

fn paint_road_faces(
    planet: &PlanetMesh,
    adj: &[[u32; 3]],
    roads: &Roads,
    n: usize,
) -> BitSet {
    let mut road_faces = BitSet::new(n);
    for road in &roads.roads {
        if matches!(road.kind, shared::roads::PathKind::Bridge) {
            continue;
        }
        let chain = face_chain(planet, &road.points);
        paint_chain(&mut road_faces, adj, &chain);
    }
    road_faces
}

fn face_chain(planet: &PlanetMesh, points: &[SpherePos]) -> Vec<usize> {
    let mut chain = Vec::new();
    for seg in points.windows(2) {
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
    chain
}

fn paint_chain(faces: &mut BitSet, adj: &[[u32; 3]], chain: &[usize]) {
    if let Some(&first) = chain.first() {
        faces.insert(first);
    }
    for w in chain.windows(2) {
        let (u, v) = (w[0], w[1]);
        faces.insert(u);
        faces.insert(v);
        if !adj[u].contains(&(v as u32)) {
            for f in shortest_face_path(adj, u, v) {
                faces.insert(f);
            }
        }
    }
}

fn paint_bridge_faces(
    planet: &PlanetMesh,
    adj: &[[u32; 3]],
    roads: &Roads,
    n: usize,
) -> BitSet {
    let mut bridge_faces = BitSet::new(n);
    for road in &roads.roads {
        if !matches!(road.kind, shared::roads::PathKind::Bridge) {
            continue;
        }
        let chain = face_chain(planet, &road.points);
        paint_chain(&mut bridge_faces, adj, &chain);
    }
    bridge_faces
}

fn paint_town_faces(planet: &PlanetMesh, roads: &Roads, n: usize) -> BitSet {
    let mut town_faces = BitSet::new(n);
    for (fi, tri) in planet.tris().iter().enumerate() {
        let centroid = (tri[0] + tri[1] + tri[2]) / 3.0;
        for s in &roads.settlements {
            if arc(centroid.normalize(), s.pos.0) <= TOWN_RADIUS {
                town_faces.insert(fi);
                break;
            }
        }
    }
    town_faces
}

const TOWN_RADIUS: f32 = 55.0;

fn arc(a: Vec3, b: Vec3) -> f32 {
    a.dot(b).clamp(-1.0, 1.0).acos() * shared::sphere::PLANET_RADIUS
}

// ---- face adjacency ----

/// Build adjacency from icosphere subdivision indexing via edge-map on raw vertex bits.
/// Each face has exactly 3 edge-neighbors. Hash collisions are impossible with f32::to_bits
/// on the exact same vec3 (unit icosphere coords are bit-identical after normalize).
fn build_face_adjacency(planet: &PlanetMesh) -> Vec<[u32; 3]> {
    let tris = planet.tris();
    let n = tris.len();

    let mut edge_map: HashMap<[u64; 2], Vec<u32>> = HashMap::new();
    for (fi, tri) in tris.iter().enumerate() {
        let fi = fi as u32;
        for (x, y) in [(0, 1), (1, 2), (2, 0)] {
            let ka = hash_vert(tri[x]);
            let kb = hash_vert(tri[y]);
            let key = if ka <= kb { [ka, kb] } else { [kb, ka] };
            edge_map.entry(key).or_default().push(fi);
        }
    }

    let mut adj = vec![[u32::MAX; 3]; n];
    for (fi, tri) in tris.iter().enumerate() {
        let mut k = 0;
        for (x, y) in [(0, 1), (1, 2), (2, 0)] {
            let ka = hash_vert(tri[x]);
            let kb = hash_vert(tri[y]);
            let key = if ka <= kb { [ka, kb] } else { [kb, ka] };
            if let Some(neighbors) = edge_map.get(&key) {
                for &n in neighbors {
                    if n as usize != fi { adj[fi][k] = n; k += 1; break; }
                }
            }
        }
    }
    adj
}

fn hash_vert(v: Vec3) -> u64 {
    let arr = v.to_array();
    arr[0].to_bits() as u64
        ^ (arr[1].to_bits() as u64).wrapping_mul(6364136223846793005)
        ^ (arr[2].to_bits() as u64).wrapping_mul(1442695040888963407)
}

fn shortest_face_path(adj: &[[u32; 3]], u: usize, v: usize) -> Vec<usize> {
    const MAX_HOPS: usize = 4;
    let mut prev: HashMap<usize, usize> = HashMap::new();
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
            let n = n as usize;
            if n == u32::MAX as usize { continue; }
            prev.entry(n).or_insert_with(|| {
                q.push_back((n, depth + 1));
                cur
            });
        }
    }
    Vec::new()
}
