use bevy_color::ColorToComponents;
use bevy_math::Vec3;
use shared::level::{LevelData, RoadData, SettlementData, WaterBodyData, WaterKind};
use shared::planet::{PlanetMesh, unit_icosphere_tris};
use shared::roads::{Roads, Settlement};
use shared::sphere::SpherePos;
use shared::terrain::{Terrain, TerrainGen};
use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::path::PathBuf;

use crate::bitset::{BitSet, FACE_FLAG_ROAD, FACE_FLAG_TOWN, FACE_FLAG_BRIDGE};

const TOWN_RADIUS: f32 = 55.0;

pub struct GenCtx {
    pub seed: u32,
    pub out: PathBuf,
    pub unit_tris: Vec<[Vec3; 3]>,
    pub planet: PlanetMesh,
    pub adj: Vec<[u32; 3]>,
    pub n: usize,
}

impl GenCtx {
    pub fn from_env() -> Self {
        let seed: u32 = std::env::var("PLANET_SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(1337);
        let out = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| {
            let dir = std::env!("CARGO_MANIFEST_DIR");
            format!("{dir}/../main/assets/level_{seed}.bin")
        }));
        let sub = 6;
        let unit_tris = unit_icosphere_tris(sub);
        let planet = PlanetMesh::new(unit_tris.clone());
        let adj = build_face_adjacency(&planet.tris()[..], unit_tris.len());
        let n = unit_tris.len();
        debug_assert!(adj.iter().all(|a| !a.contains(&u32::MAX)), "face adjacency incomplete");
        Self { seed, out, unit_tris, planet, adj, n }
    }
}

// ---- pass 1: terrain generation ----
pub fn gen_terrain(seed: u32) -> TerrainGen { TerrainGen::new(seed) }

// ---- pass 2: settlement placement ----
pub fn place_settlements(terrain: &TerrainGen) -> Vec<Settlement> {
    Roads::place_settlements(terrain)
}

pub fn classify_base(ctx: &GenCtx, terrain: &TerrainGen) -> Vec<Terrain> {
    let types: Vec<Terrain> = (0..ctx.n).map(|fi| {
        let [a, b, c] = ctx.unit_tris[fi];
        let cent = SpherePos::new(((a + b + c) / 3.0).normalize());
        terrain.base_classify(cent)
    }).collect();
    types
}

pub fn detect_lakes(ctx: &GenCtx, terrain: &TerrainGen, base_types: &[Terrain]) -> Vec<Terrain> {
    let mut face_types = base_types.to_vec();
    // Find basin faces.
    for fi in 0..ctx.n {
        if face_types[fi].is_water() { continue; }
        let [a, b, c] = ctx.unit_tris[fi];
        let cent = SpherePos::new(((a + b + c) / 3.0).normalize());
        let my_e = terrain.elevation_at(cent);
        if my_e >= 0.0 { continue; }
        if my_e <= -0.1 { continue; }
        let is_basin = ctx.adj[fi].iter().all(|&nb| {
            let nb = nb as usize;
            if nb == u32::MAX as usize { return true; }
            if base_types[nb].is_water() { return true; }
            let [na, nb2, nc] = ctx.unit_tris[nb];
            let nb_cent = SpherePos::new(((na + nb2 + nc) / 3.0).normalize());
            terrain.elevation_at(nb_cent) > my_e
        });
        if is_basin { face_types[fi] = Terrain::Lake; }
    }
    // Merge tiny lakes (<10 faces) into Ocean.
    let mut seen = vec![false; ctx.n];
    for fi in 0..ctx.n {
        if face_types[fi] != Terrain::Lake || seen[fi] { continue; }
        let mut comp = Vec::new();
        let mut q = VecDeque::from([fi]);
        seen[fi] = true;
        while let Some(cur) = q.pop_front() {
            comp.push(cur);
            for &nb in &ctx.adj[cur] {
                let nb = nb as usize;
                if nb == u32::MAX as usize || seen[nb] { continue; }
                if face_types[nb] == Terrain::Lake { seen[nb] = true; q.push_back(nb); }
            }
        }
        if comp.len() < 10 { for f in comp { face_types[f] = Terrain::Ocean; } }
    }
    face_types
}

// ---- pass 5: river tracing ----
pub fn trace_rivers(ctx: &GenCtx, terrain: &TerrainGen, mut face_types: Vec<Terrain>) -> Vec<Terrain> {
    let bake = terrain.bake_terrain();
    for vi in 0..bake.verts.len() {
        if bake.flow_accum.get(vi).copied().unwrap_or(0.0) < 5.0 { continue; }
        if bake.vert_elev.get(vi).copied().unwrap_or(0.0) <= 0.0 { continue; }
        let dir = Vec3::from_array(bake.verts[vi]);
        let Some(start_fi) = ctx.planet.face_at(dir) else { continue; };
        if face_types[start_fi].is_water() { continue; }
        let mut cur = vi;
        for _ in 0..200 {
            let Some(&fd) = bake.flow_dir.get(cur) else { break; };
            if fd == usize::MAX || fd >= bake.verts.len() { break; }
            let next_dir = Vec3::from_array(bake.verts[fd]);
            if let Some(fi) = ctx.planet.face_at(next_dir) {
                if face_types[fi].is_water() { break; }
                face_types[fi] = Terrain::River;
            }
            cur = fd;
        }
    }
    // Drop isolated river faces.
    for fi in 0..ctx.n {
        if face_types[fi] != Terrain::River { continue; }
        let has_nbr = ctx.adj[fi].iter().any(|&nb| {
            let nb = nb as usize;
            nb != u32::MAX as usize && face_types[nb] == Terrain::River
        });
        if !has_nbr { face_types[fi] = Terrain::Ocean; }
    }
    face_types
}

// ---- pass 6: water cleanup (min sizes, names) ----
pub fn cleanup_water(ctx: &GenCtx, mut face_types: Vec<Terrain>) -> (Vec<Terrain>, Vec<WaterBodyData>) {
    let n = ctx.n;
    let mut water_body = vec![usize::MAX; n];
    let mut body_count = 0usize;
    for fi in 0..n {
        if !face_types[fi].is_water() || water_body[fi] != usize::MAX { continue; }
        let mut q = VecDeque::from([fi]);
        water_body[fi] = body_count;
        while let Some(cur) = q.pop_front() {
            for &nb in &ctx.adj[cur] {
                let nb = nb as usize;
                if nb == u32::MAX as usize { continue; }
                if face_types[nb].is_water() && water_body[nb] == usize::MAX {
                    water_body[nb] = body_count;
                    q.push_back(nb);
                }
            }
        }
        body_count += 1;
    }

    let mut body_counts = vec![0usize; body_count];
    let mut body_centroids = vec![Vec3::ZERO; body_count];
    for fi in 0..n {
        let wb = water_body[fi];
        if wb == usize::MAX { continue; }
        body_counts[wb] += 1;
        body_centroids[wb] += (ctx.unit_tris[fi][0] + ctx.unit_tris[fi][1] + ctx.unit_tris[fi][2]) / 3.0;
    }
    for i in 0..body_count {
        if body_counts[i] > 0 { body_centroids[i] /= body_counts[i] as f32; }
    }

    // Merge tiny oceans.
    for fi in 0..n {
        if face_types[fi] != Terrain::Ocean || water_body[fi] == usize::MAX { continue; }
        if body_counts[water_body[fi]] >= 100 { continue; }
        if let Some(&nb) = ctx.adj[fi].iter().find(|&&nb| {
            let nb = nb as usize;
            nb != u32::MAX as usize && water_body[nb] == usize::MAX
        }) {
            face_types[fi] = face_types[nb as usize];
        }
    }

    let ocean_names = ["Azure", "Cobalt", "Cerulean", "Sapphire", "Indigo", "Teal",
        "Aquamarine", "Turquoise", "Navy", "Sky", "Marine", "Coral", "Lagoon", "Reef",
        "Abyss", "Trench", "Gulf", "Bay", "Strait", "Channel"];
    let lake_names = ["Mirror", "Crystal", "Emerald", "Silver", "Misty", "Clear",
        "Loch", "Mere", "Tarn", "Pond", "Basin", "Hollow"];
    let river_names = ["Serpent", "Winding", "Rushing", "Silver", "Mossy", "Deep",
        "Brook", "Stream", "Creek", "Fork", "Bend", "Rapids"];

    let mut water_bodies = Vec::with_capacity(body_count);
    for i in 0..body_count {
        let (lake_v, river_v): (usize, usize) = (0..n)
            .filter(|&fi| water_body[fi] == i)
            .fold((0, 0), |(l, r), fi| {
                (l + (face_types[fi] == Terrain::Lake) as usize,
                 r + (face_types[fi] == Terrain::River) as usize)
            });
        let total = body_counts[i];
        let kind = if lake_v > total / 3 { WaterKind::Lake }
        else if river_v > total / 3 { WaterKind::River }
        else { WaterKind::Ocean };
        let names: &[&str] = match kind { WaterKind::Lake => &lake_names, WaterKind::River => &river_names, WaterKind::Ocean => &ocean_names };
        let suffix = match kind { WaterKind::Ocean => "Ocean", WaterKind::Lake => "Lake", WaterKind::River => "River" };
        water_bodies.push(WaterBodyData {
            name: format!("{} {}", names[i % names.len()], suffix),
            pos: body_centroids[i].normalize().to_array(),
            kind,
        });
    }
    (face_types, water_bodies)
}

// ---- pass 7: shoreline ----
pub fn paint_shoreline(ctx: &GenCtx, mut face_types: Vec<Terrain>, base_types: &[Terrain]) -> Vec<Terrain> {
    // Ocean-adjoining faces → Beach or Cliff.
    for fi in 0..ctx.n {
        if !base_types[fi].is_water() { continue; }
        for &nb in &ctx.adj[fi] {
            let nb = nb as usize;
            if nb == u32::MAX as usize || base_types[nb].is_water() { continue; }
            if face_types[nb] != base_types[nb] { continue; }
            face_types[nb] = match base_types[nb] {
                Terrain::Mountain | Terrain::Snow | Terrain::Tundra => Terrain::Cliff,
                _ => Terrain::Beach,
            };
        }
    }
    // Lake → LakeShore, River → RiverBank.
    for fi in 0..ctx.n {
        match face_types[fi] {
            Terrain::Lake => {
                for &nb in &ctx.adj[fi] {
                    let nb = nb as usize;
                    if nb == u32::MAX as usize || face_types[nb].is_water() { continue; }
                    face_types[nb] = Terrain::LakeShore;
                }
            }
            Terrain::River => {
                for &nb in &ctx.adj[fi] {
                    let nb = nb as usize;
                    if nb == u32::MAX as usize || face_types[nb].is_water() { continue; }
                    face_types[nb] = Terrain::RiverBank;
                }
            }
            _ => {}
        }
    }
    face_types
}

// ---- pass 8: land bodies ----
pub fn build_land_bodies(ctx: &GenCtx, face_types: &[Terrain], settlements: &[Settlement]) -> (Vec<usize>, Vec<usize>) {
    let n = ctx.n;
    let mut land_body = vec![usize::MAX; n];
    let mut land_count = 0usize;
    for fi in 0..n {
        if face_types[fi].is_water() || land_body[fi] != usize::MAX { continue; }
        let mut q = VecDeque::from([fi]);
        land_body[fi] = land_count;
        while let Some(cur) = q.pop_front() {
            for &nb in &ctx.adj[cur] {
                let nb = nb as usize;
                if nb == u32::MAX as usize { continue; }
                if !face_types[nb].is_water() && land_body[nb] == usize::MAX {
                    land_body[nb] = land_count;
                    q.push_back(nb);
                }
            }
        }
        land_count += 1;
    }
    let settle_land: Vec<usize> = settlements.iter().map(|s| {
        ctx.planet.face_at(s.pos.0).map(|fi| land_body[fi]).unwrap_or(usize::MAX)
    }).collect();
    (land_body, settle_land)
}

// ---- pass 9: roads (within same land body) ----
pub fn build_roads(ctx: &GenCtx, settlements: &[Settlement], settle_land: &[usize], face_types: &[Terrain]) -> Roads {
    let mut roads = Roads::default();
    roads.settlements = settlements.to_vec();
    let mut linked = BitSet::new(settlements.len() * settlements.len());

    for (ai, a) in settlements.iter().enumerate() {
        let la = settle_land[ai];
        if la == usize::MAX { continue; }
        let mut nbrs: Vec<(usize, f32)> = settlements.iter().enumerate()
            .filter(|(bi, _)| *bi != ai && settle_land[*bi] == la)
            .map(|(bi, b)| (bi, a.pos.distance(b.pos)))
            .collect();
        nbrs.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
        for (bi, _) in nbrs.iter().take(2) {
            let key = ai.min(*bi) * settlements.len() + ai.max(*bi);
            if linked.contains(key) { continue; }
            linked.insert(key);
            let b = &settlements[*bi];
            let dist = a.pos.distance(b.pos);
            let steps = (dist / shared::roads::SAMPLE_SPACING).ceil().max(1.0) as usize;
            let crosses = (0..=steps).any(|k| {
                let p = shared::sphere::slerp(a.pos, b.pos, k as f32 / steps as f32);
                ctx.planet.face_at(p.0).map(|fi| face_types[fi].is_water()).unwrap_or(false)
            });
            if crosses { continue; }
            let r = shared::roads::build_land_road_path(a.pos, b.pos, steps, (a.pos.0 + b.pos.0).normalize());
            roads.roads.push(shared::roads::Road { points: r, kind: shared::roads::PathKind::Road });
        }
    }
    roads
}

// ---- pass 10: bridges (between land bodies) ----
pub fn build_bridges(ctx: &GenCtx, face_types: &[Terrain], land_body: &[usize], mut roads: Roads) -> Roads {
    let land_count = land_body.iter().copied().filter(|&l| l != usize::MAX).max().map(|m| m + 1).unwrap_or(0);
    let mut shores: Vec<Vec<SpherePos>> = vec![Vec::new(); land_count];
    for fi in 0..ctx.n {
        let lb = land_body[fi];
        if lb == usize::MAX { continue; }
        if matches!(face_types[fi], Terrain::Beach | Terrain::RiverBank | Terrain::LakeShore) {
            let [a, b, c] = ctx.unit_tris[fi];
            let cent = SpherePos::new(((a + b + c) / 3.0).normalize());
            shores[lb].push(cent);
        }
    }
    let mut count = 0usize;
    for a_body in 0..land_count {
        for b_body in (a_body + 1)..land_count {
            if count >= 6 { break; }
            let mut best: Option<(SpherePos, SpherePos, f32)> = None;
            for sa in &shores[a_body] {
                for sb in &shores[b_body] {
                    let d = sa.distance(*sb);
                    if d <= 100.0 && best.is_none_or(|(_, _, bd)| d < bd) {
                        best = Some((*sa, *sb, d));
                    }
                }
            }
            if let Some((sa, sb, _)) = best {
                let steps = (sa.distance(sb) / 10.0_f32).ceil().max(1.0) as usize;
                let pts: Vec<SpherePos> = (0..=steps).map(|k| shared::sphere::slerp(sa, sb, k as f32 / steps as f32)).collect();
                roads.roads.push(shared::roads::Road { points: pts, kind: shared::roads::PathKind::Bridge });
                count += 1;
            }
        }
    }
    roads
}

// ---- pass 11: apply roads to terrain ----
pub fn apply_roads(mut terrain: TerrainGen, roads: &Roads) -> TerrainGen {
    terrain.set_roads(roads);
    terrain
}

// ---- pass 12: face painting ----
pub fn paint_faces(ctx: &GenCtx, roads: &Roads) -> (BitSet, BitSet, BitSet, BitSet) {
    let n = ctx.n;
    let mut road_fs = BitSet::new(n);
    let mut bridge_fs = BitSet::new(n);
    for road in &roads.roads {
        let chain = face_chain(&ctx.planet, &road.points);
        if matches!(road.kind, shared::roads::PathKind::Bridge) {
            paint_chain(&mut bridge_fs, &ctx.adj, &chain);
        } else {
            paint_chain(&mut road_fs, &ctx.adj, &chain);
        }
    }
    let town_fs = paint_town_faces(&ctx.planet, roads, n);
    let be_fs = bridge_approach_faces(&ctx.planet, &ctx.adj, roads, &bridge_fs, n);
    (road_fs, town_fs, bridge_fs, be_fs)
}

// ---- pass 12: vertex-first mesh construction ----
pub fn build_mesh(
    ctx: &GenCtx, terrain: &TerrainGen, face_types: &[Terrain],
    roads: &Roads, town_faces: &BitSet, road_faces: &BitSet,
    bridge_faces: &BitSet, bridge_entry_faces: &BitSet,
) -> (Vec<[[f32; 3]; 3]>, Vec<[f32; 4]>, Vec<u8>) {
    let n = ctx.n;
    let mut vmap: BTreeMap<[u64; 3], usize> = BTreeMap::new();
    let mut verts: Vec<Vec3> = Vec::new();
    let mut vert_r: Vec<f32> = Vec::new();
    let mut face_v: Vec<[usize; 3]> = Vec::with_capacity(n);
    for tri in &ctx.unit_tris {
        let mut idx = [0usize; 3];
        for (k, v) in tri.iter().enumerate() {
            let key = [v.x.to_bits() as u64, v.y.to_bits() as u64, v.z.to_bits() as u64];
            idx[k] = *vmap.entry(key).or_insert_with(|| {
                let i = verts.len();
                verts.push(v.normalize());
                vert_r.push(terrain.render_radius(SpherePos::new(v.normalize())));
                i
            });
        }
        face_v.push(idx);
    }
    let nv = verts.len();
    let mut va: Vec<Vec<usize>> = vec![Vec::new(); nv];
    for [a, b, c] in &face_v {
        va[*a].push(*b); va[*a].push(*c);
        va[*b].push(*a); va[*b].push(*c);
        va[*c].push(*a); va[*c].push(*b);
    }
    for l in &mut va { l.sort_unstable(); l.dedup(); }

    let settle_r: Vec<f32> = roads.settlements.iter()
        .map(|s| terrain.surface_radius(s.pos).max(shared::sphere::PLANET_RADIUS))
        .collect();
    let mut vt = vec![false; nv];
    for (fi, [a, b, c]) in face_v.iter().enumerate() {
        if town_faces.contains(fi) {
            let cent = SpherePos::new((verts[*a] + verts[*b] + verts[*c]) / 3.0);
            let r = roads.settlements.iter().zip(&settle_r)
                .min_by(|(a, _), (b, _)| a.pos.distance(cent).partial_cmp(&b.pos.distance(cent)).unwrap())
                .map(|(_, &r)| r).unwrap_or(shared::sphere::PLANET_RADIUS);
            for &vi in &[*a, *b, *c] { vt[vi] = true; vert_r[vi] = r; }
        }
    }
    for _ in 0..5 {
        let mut next = vert_r.clone();
        let mut changed = false;
        for vi in 0..nv {
            if vt[vi] { continue; }
            if !va[vi].iter().any(|&n| (vert_r[n] - next[vi]).abs() > 0.1) { continue; }
            let avg: f32 = va[vi].iter().map(|&n| vert_r[n]).sum::<f32>() / va[vi].len() as f32;
            next[vi] = next[vi] * 0.6 + avg * 0.4;
            changed = true;
        }
        vert_r = next;
        if !changed { break; }
    }
    for fi in bridge_entry_faces.iter() {
        let [a, b, c] = face_v[fi];
        let cent = ((verts[a] + verts[b] + verts[c]) / 3.0).normalize();
        let er = terrain.surface_radius(SpherePos::new(cent)).max(shared::sphere::PLANET_RADIUS);
        for &vi in &[a, b, c] { vert_r[vi] = er.max(vert_r[vi]); }
    }

    let mut tris = Vec::with_capacity(n);
    let mut cols = Vec::with_capacity(n);
    let mut feats = Vec::with_capacity(n);
    for (fi, [a, b, c]) in face_v.iter().enumerate() {
        let color = if town_faces.contains(fi) {
            shared::theme::WARNING.to_linear().to_f32_array()
        } else if road_faces.contains(fi) {
            bevy_color::Color::srgb(0.5, 0.42, 0.3).to_linear().to_f32_array()
        } else {
            face_types[fi].color().to_linear().to_f32_array()
        };
        tris.push([(verts[*a] * vert_r[*a]).to_array(), (verts[*b] * vert_r[*b]).to_array(), (verts[*c] * vert_r[*c]).to_array()]);
        cols.push(color);
        let mut f = 0u8;
        if road_faces.contains(fi) { f |= FACE_FLAG_ROAD; }
        if town_faces.contains(fi) { f |= FACE_FLAG_TOWN; }
        if bridge_faces.contains(fi) { f |= FACE_FLAG_BRIDGE; }
        feats.push(f);
    }
    (tris, cols, feats)
}

// ---- pass 13: feature mesh (bridge decks) ----
pub fn build_features(ctx: &GenCtx, terrain: &TerrainGen, roads: &Roads) -> (Vec<[[f32; 3]; 3]>, Vec<[f32; 4]>) {
    let mut tris = Vec::new();
    let mut cols = Vec::new();
    let bc = bevy_color::Color::srgb(0.35, 0.25, 0.18).to_linear().to_f32_array();
    for road in &roads.roads {
        if !matches!(road.kind, shared::roads::PathKind::Bridge) { continue; }
        build_bridge_strip(&mut tris, &mut cols, &road.points, terrain, 20.0, bc);
    }
    (tris, cols)
}

// ---- pass 14: serialize ----
pub fn serialize(
    ctx: &GenCtx, terrain: &TerrainGen, roads: &Roads, face_types: &[Terrain], water_bodies: &[WaterBodyData],
    terrain_tris: Vec<[[f32; 3]; 3]>, terrain_colors: Vec<[f32; 4]>, terrain_features: Vec<u8>,
    feature_tris: Vec<[[f32; 3]; 3]>, feature_colors: Vec<[f32; 4]>,
) {
    let unit_tris_arr: Vec<[[f32; 3]; 3]> = ctx.unit_tris.iter().map(|[a,b,c]| [a.to_array(), b.to_array(), c.to_array()]).collect();
    let feat_count = feature_tris.len();
    let baked = terrain.bake_terrain();
    let data = LevelData {
        terrain_tris,
        terrain_colors,
        terrain_features,
        unit_tris: unit_tris_arr,
        face_types: face_types.iter().map(|t| *t as u8).collect(),
        feature_tris,
        feature_colors,
        settlements: roads.settlements.iter().map(|s| SettlementData { name: s.name.clone(), pos: s.pos.0.to_array() }).collect(),
        roads: roads.roads.iter().map(|r| RoadData { points: r.points.iter().map(|p| p.0.to_array()).collect(), is_bridge: matches!(r.kind, shared::roads::PathKind::Bridge) }).collect(),
        baked_verts: baked,
        water_bodies: water_bodies.to_vec(),
    };
    let bytes = postcard::to_allocvec(&data).expect("serialize");
    let _ = fs::create_dir_all(ctx.out.parent().unwrap());
    fs::write(&ctx.out, &bytes).expect("write");
    println!("Wrote {} terrain tris, {} feature tris → {}", ctx.n, feat_count, ctx.out.display());
}

// ---- helpers (below passes, called by passes) ----

fn build_bridge_strip(tris: &mut Vec<[[f32; 3]; 3]>, colors: &mut Vec<[f32; 4]>, points: &[SpherePos], terrain: &TerrainGen, hw: f32, color: [f32; 4]) {
    let sr = terrain.surface_radius(points[0]);
    let er = terrain.surface_radius(points[points.len() - 1]);
    let sea = shared::sphere::PLANET_RADIUS;
    let spacing = 2.0;
    let mut dirs = Vec::new();
    let mut pos = 0.0f32;
    for seg in points.windows(2) {
        let sl = seg[0].distance(seg[1]);
        while pos <= sl + 1e-6 {
            dirs.push(shared::sphere::slerp(seg[0], seg[1], (pos / sl).min(1.0)).0);
            if pos >= sl { break; }
            pos = (pos + spacing).min(sl);
        }
        pos -= sl;
    }
    let last = points.last().unwrap().0;
    if (dirs.last().unwrap().dot(last)).abs() < 0.999 { dirs.push(last); }
    let n = dirs.len();
    let mut rings: Vec<(Vec3, Vec3, Vec3)> = Vec::new();
    for i in 0..n {
        let dir = dirs[i];
        let t = i as f32 / (n - 1).max(1) as f32;
        let arch = 2.0 * (4.0 * t * (1.0 - t));
        let base = sr + (er - sr) * t;
        let tr = terrain.surface_radius(SpherePos::new(dir)).max(sea);
        let r = base.max(tr) + arch;
        let up = dir.normalize();
        let fwd = if i == 0 { (dirs[1] - dirs[0]).normalize() }
        else if i == n - 1 { (dirs[n-1] - dirs[n-2]).normalize() }
        else { let p = (dirs[i] - dirs[i-1]).normalize(); let n = (dirs[i+1] - dirs[i]).normalize(); (p + n).normalize_or((dirs[1] - dirs[0]).normalize()) };
        let left = fwd.cross(up).normalize_or(Vec3::X) * (hw / r);
        rings.push((dir * r, (dir + left).normalize() * r, (dir - left).normalize() * r));
    }
    for p in rings.windows(2) {
        tris.push([p[0].1.to_array(), p[0].2.to_array(), p[1].1.to_array()]); colors.push(color);
        tris.push([p[0].2.to_array(), p[1].2.to_array(), p[1].1.to_array()]); colors.push(color);
    }
}

fn paint_town_faces(planet: &PlanetMesh, roads: &Roads, n: usize) -> BitSet {
    let mut bs = BitSet::new(n);
    for (fi, tri) in planet.tris().iter().enumerate() {
        let cent = (tri[0] + tri[1] + tri[2]) / 3.0;
        if roads.settlements.iter().any(|s| arc(cent.normalize(), s.pos.0) <= TOWN_RADIUS) {
            bs.insert(fi);
        }
    }
    bs
}

fn bridge_approach_faces(planet: &PlanetMesh, adj: &[[u32; 3]], roads: &Roads, bf: &BitSet, n: usize) -> BitSet {
    let mut ap = BitSet::new(n);
    for r in &roads.roads {
        if !matches!(r.kind, shared::roads::PathKind::Bridge) { continue; }
        for pt in [r.points.first(), r.points.last()] {
            if let Some(fi) = planet.face_at(pt.unwrap().0) {
                if !bf.contains(fi) { flood_bfs(&mut ap, adj, fi, bf, 6, n); }
            }
        }
    }
    ap
}

fn flood_bfs(out: &mut BitSet, adj: &[[u32; 3]], start: usize, exclude: &BitSet, rings: usize, n: usize) {
    let mut q = VecDeque::from([(start, 0usize)]);
    let mut seen = BitSet::new(n);
    seen.insert(start);
    while let Some((cur, d)) = q.pop_front() {
        if d > rings { continue; }
        out.insert(cur);
        for &n in &adj[cur] {
            let n = n as usize;
            if n == u32::MAX as usize || seen.contains(n) || exclude.contains(n) { continue; }
            seen.insert(n);
            q.push_back((n, d + 1));
        }
    }
}

fn face_chain(planet: &PlanetMesh, points: &[SpherePos]) -> Vec<usize> {
    let mut c = Vec::new();
    for seg in points.windows(2) {
        let steps = (arc(seg[0].0, seg[1].0) / 2.0).ceil().max(1.0) as usize;
        for k in 0..=steps {
            let p = seg[0].0.lerp(seg[1].0, k as f32 / steps as f32).normalize();
            if let Some(fi) = planet.face_at(p) {
                if c.last() != Some(&fi) { c.push(fi); }
            }
        }
    }
    c
}

fn paint_chain(faces: &mut BitSet, adj: &[[u32; 3]], chain: &[usize]) {
    if let Some(&f) = chain.first() { faces.insert(f); }
    for w in chain.windows(2) {
        faces.insert(w[0]); faces.insert(w[1]);
        if !adj[w[0]].contains(&(w[1] as u32)) {
            for f in shortest_face_path(adj, w[0], w[1]) { faces.insert(f); }
        }
    }
}

fn shortest_face_path(adj: &[[u32; 3]], u: usize, v: usize) -> Vec<usize> {
    let mut prev: BTreeMap<usize, usize> = BTreeMap::new();
    let mut q = VecDeque::from([(u, 0usize)]);
    prev.insert(u, u);
    while let Some((cur, d)) = q.pop_front() {
        if cur == v {
            let mut p = Vec::new();
            let mut c = v;
            while c != u { if c != v { p.push(c); } c = prev[&c]; }
            p.reverse();
            return p;
        }
        if d >= 4 { continue; }
        for &n in &adj[cur] {
            let n = n as usize;
            if n == u32::MAX as usize { continue; }
            prev.entry(n).or_insert_with(|| { q.push_back((n, d + 1)); cur });
        }
    }
    Vec::new()
}

fn build_face_adjacency(tris: &[[Vec3; 3]], n: usize) -> Vec<[u32; 3]> {
    let mut em: BTreeMap<[u64; 2], Vec<u32>> = BTreeMap::new();
    for (fi, tri) in tris.iter().enumerate() {
        let fi = fi as u32;
        for (x, y) in [(0, 1), (1, 2), (2, 0)] {
            let (ka, kb) = (hv(tri[x]), hv(tri[y]));
            em.entry(if ka <= kb { [ka, kb] } else { [kb, ka] }).or_default().push(fi);
        }
    }
    let mut adj = vec![[u32::MAX; 3]; n];
    for (fi, tri) in tris.iter().enumerate() {
        let mut k = 0;
        for (x, y) in [(0, 1), (1, 2), (2, 0)] {
            let (ka, kb) = (hv(tri[x]), hv(tri[y]));
            if let Some(ns) = em.get(&if ka <= kb { [ka, kb] } else { [kb, ka] }) {
                for &n in ns { if n as usize != fi && k < 3 { adj[fi][k] = n; k += 1; break; } }
            }
        }
    }
    adj
}

fn hv(v: Vec3) -> u64 {
    let a = v.to_array();
    a[0].to_bits() as u64 ^ (a[1].to_bits() as u64).wrapping_mul(6364136223846793005) ^ (a[2].to_bits() as u64).wrapping_mul(1442695040888963407)
}

fn arc(a: Vec3, b: Vec3) -> f32 { a.dot(b).clamp(-1.0, 1.0).acos() * shared::sphere::PLANET_RADIUS }
