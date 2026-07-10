use bevy_color::ColorToComponents;
use bevy_math::Vec3;
use shared::level::{LevelData, RegionData, RegionKind, RoadData, SettlementData, LEVEL_FORMAT_VERSION, NO_REGION, TAG_BRIDGE, TAG_BRIDGE_ENTRY, TAG_ROAD, TAG_TOWN};
use shared::planet::{build_face_adjacency, unit_icosphere_tris, PlanetMesh};
use shared::sphere::SpherePos;
use shared::terrain::{Terrain, TerrainGen};
use shared::wfc;
use shared::zones::FINE_SUB;
use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::path::PathBuf;

use crate::bitset::BitSet;

const TOWN_RADIUS: f32 = 55.0;
/// Vertical relief added to cliff-top vertices at mesh time, meters.
const CLIFF_LIFT: f32 = 20.0;

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
        let unit_tris = unit_icosphere_tris(FINE_SUB);
        let planet = PlanetMesh::new(unit_tris.clone());
        let adj = build_face_adjacency(&planet.tris()[..], unit_tris.len());
        let n = unit_tris.len();
        debug_assert!(adj.iter().all(|a| !a.contains(&u32::MAX)), "face adjacency incomplete");
        Self { seed, out, unit_tris, planet, adj, n }
    }

    fn centroid(&self, fi: usize) -> SpherePos {
        let [a, b, c] = self.unit_tris[fi];
        SpherePos::new(((a + b + c) / 3.0).normalize())
    }
}

// ---- L0–L3: zones, topology, elevation (all inside TerrainGen) ----

pub fn gen_terrain(seed: u32) -> TerrainGen {
    TerrainGen::new(seed)
}

// ---- L5a: zone-aware base classification per fine face ----

pub fn classify_base(ctx: &GenCtx, terrain: &TerrainGen) -> Vec<Terrain> {
    (0..ctx.n).map(|fi| terrain.base_classify_fine(fi, ctx.centroid(fi))).collect()
}

// ---- L4: snap topology polylines to fine faces ----

/// River polylines → River faces (land only; the mouth is already water).
pub fn paint_rivers(ctx: &GenCtx, terrain: &TerrainGen, face_types: &mut [Terrain]) {
    for path in &terrain.river_paths {
        for fi in face_chain(ctx, path) {
            if face_types[fi].is_land() {
                face_types[fi] = Terrain::River;
            }
        }
    }
}

/// Water-body identity comes from connectivity, not per-face depth: any face can
/// dip below sea level (blending, river carving), but a connected water body is
/// an Ocean only if it reaches ocean-zone faces — otherwise it's an enclosed
/// Lake, whatever its faces individually classified as. Rivers (painted channel
/// faces) are their own linear feature and never merge into either.
pub fn normalize_water_bodies(ctx: &GenCtx, terrain: &TerrainGen, face_types: &mut [Terrain]) {
    let mut visited = vec![false; ctx.n];
    for start in 0..ctx.n {
        if !face_types[start].is_water() || face_types[start] == Terrain::River || visited[start] {
            continue;
        }
        let mut body = vec![start];
        let mut q = VecDeque::from([start]);
        visited[start] = true;
        while let Some(cur) = q.pop_front() {
            for &nb in &ctx.adj[cur] {
                let nb = nb as usize;
                if face_types[nb].is_water() && face_types[nb] != Terrain::River && !visited[nb] {
                    visited[nb] = true;
                    body.push(nb);
                    q.push_back(nb);
                }
            }
        }
        let is_ocean = body.iter()
            .any(|&fi| terrain.zones().kind_at_fine(fi) == shared::zones::ZoneKind::Ocean);
        for &fi in &body {
            face_types[fi] = if !is_ocean {
                Terrain::Lake
            } else if face_types[fi] == Terrain::Lake {
                // A lake tile swallowed by the ocean body is just shallow ocean.
                Terrain::Ocean
            } else {
                face_types[fi]
            };
        }
    }
}

pub struct PaintedFaces {
    pub roads: BitSet,
    pub towns: BitSet,
    pub bridges: BitSet,
    pub bridge_entries: BitSet,
}

pub fn paint_faces(ctx: &GenCtx, terrain: &TerrainGen) -> PaintedFaces {
    let mut roads = BitSet::new(ctx.n);
    for path in &terrain.road_paths {
        for fi in face_chain(ctx, path) {
            roads.insert(fi);
        }
    }
    // Bridges are painted later, once regions exist to validate their endpoints.
    let bridges = BitSet::new(ctx.n);
    let mut towns = BitSet::new(ctx.n);
    for fi in 0..ctx.n {
        let cent = ctx.centroid(fi);
        if terrain.settlement_anchors.iter().any(|a| a.distance(cent) <= TOWN_RADIUS) {
            towns.insert(fi);
        }
    }
    PaintedFaces { roads, towns, bridges, bridge_entries: BitSet::new(ctx.n) }
}

/// Gaps up to this bridge freely.
const BRIDGE_MAX_SPAN: f32 = 250.0;
/// Longer gaps (up to this) are bridged only to connect an otherwise
/// unreachable landmass — every island gets at least one way in.
const BRIDGE_CONNECT_SPAN: f32 = 500.0;
const BRIDGE_MAX_COUNT: usize = 6;

/// Pick bridges from the fine map, where true water separation is known: each
/// bridge runs from a shore-band face of one named region to a shore-band face
/// of a *different* region on a different landmass, crossing open water.
/// (Coarse-level candidates proved unreliable — shoreline blending can fill a
/// coarse "gap" with land.)
pub fn build_bridges(
    ctx: &GenCtx,
    face_types: &[Terrain],
    face_region: &[u32],
    painted: &mut PaintedFaces,
) -> Vec<Vec<SpherePos>> {
    // Landmasses: edge-connected components of land faces.
    let mut comp = vec![u32::MAX; ctx.n];
    let mut count = 0u32;
    for fi in 0..ctx.n {
        if face_types[fi].is_water() || comp[fi] != u32::MAX {
            continue;
        }
        let mut q = VecDeque::from([fi]);
        comp[fi] = count;
        while let Some(cur) = q.pop_front() {
            for &nb in &ctx.adj[cur] {
                let nb = nb as usize;
                if !face_types[nb].is_water() && comp[nb] == u32::MAX {
                    comp[nb] = count;
                    q.push_back(nb);
                }
            }
        }
        count += 1;
    }
    // Bridgeheads: shore-band faces that belong to a named region. Beaches only —
    // cliff tops get a mesh lift, and a deck ending on one becomes a wall.
    let mut heads: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
    for fi in 0..ctx.n {
        if face_types[fi] == Terrain::Beach
            && shared::level::region_index(face_region[fi]).is_some()
        {
            heads.entry(comp[fi]).or_default().push(fi);
        }
    }
    let comps: Vec<u32> = heads.keys().copied().collect();
    let mut candidates: Vec<(f32, usize, usize, usize, usize)> = Vec::new();
    for i in 0..comps.len() {
        for j in (i + 1)..comps.len() {
            let mut best: Option<(f32, usize, usize)> = None;
            for &fa in &heads[&comps[i]] {
                for &fb in &heads[&comps[j]] {
                    let d = ctx.centroid(fa).distance(ctx.centroid(fb));
                    if best.is_none_or(|(bd, _, _)| d < bd) {
                        best = Some((d, fa, fb));
                    }
                }
            }
            if let Some((d, fa, fb)) = best {
                if d <= BRIDGE_CONNECT_SPAN {
                    candidates.push((d, fa, fb, i, j));
                }
            }
        }
    }
    candidates.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    // Union-find over landmasses: long spans only earn a bridge by connecting.
    let mut parent: Vec<usize> = (0..comps.len()).collect();
    fn root(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]];
            x = parent[x];
        }
        x
    }
    let mut spans = Vec::new();
    for &(d, fa, fb, ci, cj) in &candidates {
        if spans.len() >= BRIDGE_MAX_COUNT {
            break;
        }
        let (ri, rj) = (root(&mut parent, ci), root(&mut parent, cj));
        if d > BRIDGE_MAX_SPAN && ri == rj {
            continue;
        }
        parent[ri] = rj;
        let (a, b) = (ctx.centroid(fa), ctx.centroid(fb));
        // Extend past both shore faces so the deck grounds on solid land instead
        // of ending exactly at the waterline face centroid.
        let ext = BRIDGE_ENTRY_OVERLAP / d;
        let steps = (d * (1.0 + 2.0 * ext) / 10.0).ceil().max(2.0) as usize;
        let span: Vec<SpherePos> = (0..=steps)
            .map(|k| shared::sphere::slerp(a, b, -ext + (1.0 + 2.0 * ext) * k as f32 / steps as f32))
            .collect();
        let crosses_water = span.iter().any(|p| {
            ctx.planet.face_at(p.0).is_some_and(|fi| face_types[fi].is_water())
        });
        if !crosses_water {
            continue;
        }
        for fi in face_chain(ctx, &span) {
            painted.bridges.insert(fi);
        }
        // Bridge entries: the land faces around each deck end.
        for end in [span.first(), span.last()] {
            let Some(fi) = end.and_then(|p| ctx.planet.face_at(p.0)) else { continue };
            if face_types[fi].is_land() {
                painted.bridge_entries.insert(fi);
            }
            for &nb in &ctx.adj[fi] {
                let nb = nb as usize;
                if face_types[nb].is_land() {
                    painted.bridge_entries.insert(nb);
                }
            }
        }
        spans.push(span);
    }
    spans
}

/// How far a bridge deck reaches inland past its shore face, meters.
const BRIDGE_ENTRY_OVERLAP: f32 = 25.0;

// ---- L5b: micro WFC resolves transition tiles at classification boundaries ----

/// Faces sharing at least one vertex with each face (up to ~12). Used only to
/// detect water contact (a corner-touching inlet still makes a face coastal) —
/// tile LINKING is always edge-based.
pub fn build_vertex_adjacency(ctx: &GenCtx) -> Vec<Vec<u32>> {
    let mut by_vert: BTreeMap<u64, Vec<u32>> = BTreeMap::new();
    for (fi, tri) in ctx.unit_tris.iter().enumerate() {
        for v in tri {
            by_vert.entry(vkey(*v)).or_default().push(fi as u32);
        }
    }
    let mut adj: Vec<Vec<u32>> = vec![Vec::new(); ctx.n];
    for faces in by_vert.values() {
        for &a in faces {
            for &b in faces {
                if a != b && !adj[a as usize].contains(&b) {
                    adj[a as usize].push(b);
                }
            }
        }
    }
    adj
}

fn vkey(v: Vec3) -> u64 {
    let a = v.to_array();
    a[0].to_bits() as u64
        ^ (a[1].to_bits() as u64).wrapping_mul(6364136223846793005)
        ^ (a[2].to_bits() as u64).wrapping_mul(1442695040888963407)
}

pub fn resolve_transitions(ctx: &GenCtx, terrain: &TerrainGen, vadj: &[Vec<u32>], base: &[Terrain]) -> Vec<Terrain> {
    // Shorelines are deterministic bands, not WFC cells: every land face touching
    // water gets its shore tile, so the waterline is never zigzagged by chance.
    // The band widens onto the second ring where the coast is flat, and a land
    // face wedged between two shore faces joins the band (no plains notches).
    let mut out = base.to_vec();
    let (water_dist, water_kind) = water_distance(ctx, base, 2);
    let shore = |fi: usize, kind: Terrain| -> Terrain {
        match kind {
            Terrain::Lake => Terrain::LakeShore,
            Terrain::River => Terrain::RiverBank,
            _ => {
                let steep = matches!(base[fi], Terrain::Mountain | Terrain::Snow)
                    || terrain.elevation_at(ctx.centroid(fi)) > 0.15;
                if steep { Terrain::Cliff } else { Terrain::Beach }
            }
        }
    };
    for fi in 0..ctx.n {
        if base[fi].is_water() {
            continue;
        }
        // Vertex adjacency, not edge adjacency: a one-tile inlet touches most of
        // its surrounding land only at corners — those faces are still coastline.
        let touching_water = vadj[fi].iter()
            .map(|&nb| base[nb as usize])
            .find(|t| t.is_water());
        if let Some(kind) = touching_water {
            out[fi] = shore(fi, kind);
        } else if water_dist[fi] <= 2 && terrain.elevation_at(ctx.centroid(fi)) < 0.08 {
            // Low, flat coast: the band is more than one tile wide.
            out[fi] = shore(fi, water_kind[fi].unwrap_or(Terrain::Ocean));
        }
    }
    // Fill notches: a land face with ≥2 edge neighbors in the shore band belongs
    // to the band too.
    let banded: Vec<bool> = (0..ctx.n)
        .map(|fi| matches!(out[fi], Terrain::Beach | Terrain::Cliff | Terrain::LakeShore | Terrain::RiverBank))
        .collect();
    for fi in 0..ctx.n {
        if base[fi].is_water() || banded[fi] {
            continue;
        }
        if ctx.adj[fi].iter().filter(|&&nb| banded[nb as usize]).count() >= 2 {
            out[fi] = shore(fi, water_kind[fi].unwrap_or(Terrain::Ocean));
        }
    }

    // Cells for WFC: remaining land faces bordering a different classification —
    // inland biome edges. Water and the shore band enter as fixed neighbors.
    let in_band: Vec<bool> = (0..ctx.n).map(|fi| out[fi] != base[fi]).collect();
    let base = &out;
    let mut cell_of = vec![usize::MAX; ctx.n];
    let mut cells: Vec<usize> = Vec::new();
    for fi in 0..ctx.n {
        if base[fi].is_water() || in_band[fi] {
            continue;
        }
        if ctx.adj[fi].iter().any(|&nb| base[nb as usize] != base[fi]) {
            cell_of[fi] = cells.len();
            cells.push(fi);
        }
    }

    let transition_tiles = [Terrain::Beach, Terrain::Cliff, Terrain::LakeShore, Terrain::RiverBank];
    let domains: Vec<Vec<(Terrain, f32)>> = cells.iter().map(|&fi| {
        let mut d = vec![(base[fi], 1.0)];
        for t in transition_tiles {
            if t != base[fi] {
                d.push((t, 0.3));
            }
        }
        d
    }).collect();
    let neighbors: Vec<Vec<wfc::Neighbor>> = cells.iter().map(|&fi| {
        ctx.adj[fi].iter().map(|&nb| {
            let nb = nb as usize;
            match cell_of[nb] {
                usize::MAX => wfc::Neighbor::Fixed(base[nb]),
                ci => wfc::Neighbor::Cell(ci),
            }
        }).collect()
    }).collect();
    let fallback: Vec<Terrain> = cells.iter().map(|&fi| base[fi]).collect();

    let solved = wfc::solve(&wfc::Compat::default(), &domains, &neighbors, &fallback, ctx.seed as u64);

    let mut resolved = base.clone();
    for (ci, &fi) in cells.iter().enumerate() {
        resolved[fi] = solved[ci];
    }
    segment_coastline(ctx, terrain, &mut resolved);
    absorb_small_forests(ctx, &mut resolved);
    resolved
}

/// A forest smaller than this many faces is just some trees in a field.
const MIN_FOREST_FACES: usize = 10;

fn absorb_small_forests(ctx: &GenCtx, out: &mut [Terrain]) {
    let mut visited = vec![false; ctx.n];
    for start in 0..ctx.n {
        if out[start] != Terrain::Forest || visited[start] {
            continue;
        }
        let mut cluster = vec![start];
        let mut q = VecDeque::from([start]);
        visited[start] = true;
        while let Some(cur) = q.pop_front() {
            for &nb in &ctx.adj[cur] {
                let nb = nb as usize;
                if out[nb] == Terrain::Forest && !visited[nb] {
                    visited[nb] = true;
                    cluster.push(nb);
                    q.push_back(nb);
                }
            }
        }
        if cluster.len() < MIN_FOREST_FACES {
            for fi in cluster {
                out[fi] = Terrain::Plains;
            }
        }
    }
}

/// Max coastal segment sizes in fine faces (the band is 1–2 faces wide, faces are
/// ~35m across, so 60 faces ≈ a kilometre of beach).
const MAX_BEACH_FACES: usize = 60;
const MAX_CLIFF_FACES: usize = 24;

/// Break each continuous ocean shore band into alternating Beach and Cliff
/// segments with bounded lengths — beaches can't wrap a whole continent as one
/// region, and each cliff range between them becomes a named region too.
/// Steep faces still force Cliff regardless of alternation.
fn segment_coastline(ctx: &GenCtx, terrain: &TerrainGen, out: &mut [Terrain]) {
    let in_band: Vec<bool> = (0..ctx.n)
        .map(|fi| matches!(out[fi], Terrain::Beach | Terrain::Cliff))
        .collect();
    let mut visited = vec![false; ctx.n];
    for start in 0..ctx.n {
        if !in_band[start] || visited[start] {
            continue;
        }
        // BFS order approximates walking along the thin band.
        let mut order = vec![start];
        let mut q = VecDeque::from([start]);
        visited[start] = true;
        while let Some(cur) = q.pop_front() {
            for &nb in &ctx.adj[cur] {
                let nb = nb as usize;
                if in_band[nb] && !visited[nb] {
                    visited[nb] = true;
                    order.push(nb);
                    q.push_back(nb);
                }
            }
        }
        let mut kind = Terrain::Beach;
        let mut count = 0usize;
        for fi in order {
            let steep = terrain.elevation_at(ctx.centroid(fi)) > 0.15;
            if steep && kind == Terrain::Beach {
                kind = Terrain::Cliff;
                count = 0;
            }
            let max = if kind == Terrain::Beach { MAX_BEACH_FACES } else { MAX_CLIFF_FACES };
            if count >= max {
                kind = if kind == Terrain::Beach { Terrain::Cliff } else { Terrain::Beach };
                count = 0;
            }
            out[fi] = kind;
            count += 1;
        }
    }
    // Remove single-tile islands: a band face with no same-kind edge neighbor in
    // the band joins its neighbors' kind (a lone beach tile between two cliffs
    // becomes cliff, and vice versa).
    for _ in 0..8 {
        let mut changed = false;
        for fi in 0..ctx.n {
            if !matches!(out[fi], Terrain::Beach | Terrain::Cliff) {
                continue;
            }
            let mut same = 0;
            let mut other = 0;
            for &nb in &ctx.adj[fi] {
                match out[nb as usize] {
                    t if t == out[fi] => same += 1,
                    Terrain::Beach | Terrain::Cliff => other += 1,
                    _ => {}
                }
            }
            if same == 0 && other >= 2 {
                out[fi] = if out[fi] == Terrain::Beach { Terrain::Cliff } else { Terrain::Beach };
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    // A cove under MIN_BEACH_FACES isn't a beach — fold it into the cliffs.
    let mut visited = vec![false; ctx.n];
    for start in 0..ctx.n {
        if out[start] != Terrain::Beach || visited[start] {
            continue;
        }
        let mut cluster = vec![start];
        let mut q = VecDeque::from([start]);
        visited[start] = true;
        while let Some(cur) = q.pop_front() {
            for &nb in &ctx.adj[cur] {
                let nb = nb as usize;
                if out[nb] == Terrain::Beach && !visited[nb] {
                    visited[nb] = true;
                    cluster.push(nb);
                    q.push_back(nb);
                }
            }
        }
        if cluster.len() < MIN_BEACH_FACES {
            for fi in cluster {
                out[fi] = Terrain::Cliff;
            }
        }
    }
}

const MIN_BEACH_FACES: usize = 10;

/// BFS distance (in face steps, capped at `max_dist`) from each land face to the
/// nearest water face, plus which water terrain is nearest (for shore tile choice).
fn water_distance(ctx: &GenCtx, base: &[Terrain], max_dist: u8) -> (Vec<u8>, Vec<Option<Terrain>>) {
    let mut dist = vec![u8::MAX; ctx.n];
    let mut kind: Vec<Option<Terrain>> = vec![None; ctx.n];
    let mut q = VecDeque::new();
    for fi in 0..ctx.n {
        if base[fi].is_water() {
            dist[fi] = 0;
            kind[fi] = Some(base[fi]);
            q.push_back(fi);
        }
    }
    while let Some(cur) = q.pop_front() {
        if dist[cur] >= max_dist {
            continue;
        }
        for &nb in &ctx.adj[cur] {
            let nb = nb as usize;
            if dist[nb] == u8::MAX {
                dist[nb] = dist[cur] + 1;
                kind[nb] = kind[cur];
                q.push_back(nb);
            }
        }
    }
    (dist, kind)
}

// ---- named regions: contiguous feature clusters (vertex-connected) ----

/// Which nameable feature a face belongs to. Tags win over terrain so towns and
/// roads cluster as themselves; transition tiles (Cliff/LakeShore/RiverBank)
/// separate regions and stay unnamed.
fn region_class(face_types: &[Terrain], painted: &PaintedFaces, fi: usize) -> Option<RegionKind> {
    if painted.towns.contains(fi) {
        return Some(RegionKind::Town);
    }
    if painted.roads.contains(fi) {
        return Some(RegionKind::Road);
    }
    match face_types[fi] {
        Terrain::DeepOcean | Terrain::Ocean => Some(RegionKind::Ocean),
        Terrain::Lake => Some(RegionKind::Lake),
        Terrain::River => Some(RegionKind::River),
        Terrain::Beach => Some(RegionKind::Beach),
        Terrain::Cliff => Some(RegionKind::Cliff),
        Terrain::Forest => Some(RegionKind::Forest),
        Terrain::Desert => Some(RegionKind::Desert),
        Terrain::Mountain | Terrain::Snow => Some(RegionKind::Mountain),
        Terrain::Plains => Some(RegionKind::Plains),
        Terrain::Tundra => Some(RegionKind::Tundra),
        Terrain::LakeShore | Terrain::RiverBank => None,
    }
}

/// Flood-fill same-class faces into clusters via edge adjacency (tiles sharing
/// only a single vertex are NOT linked), name each cluster, and record the
/// per-face region id for HUD lookup.
pub fn build_regions(
    ctx: &GenCtx,
    terrain: &TerrainGen,
    face_types: &[Terrain],
    painted: &PaintedFaces,
) -> (Vec<RegionData>, Vec<u32>) {
    let class: Vec<Option<RegionKind>> =
        (0..ctx.n).map(|fi| region_class(face_types, painted, fi)).collect();

    let mut face_region = vec![NO_REGION; ctx.n];
    let mut regions: Vec<RegionData> = Vec::new();
    let mut kind_counts: BTreeMap<u8, usize> = BTreeMap::new();

    for start in 0..ctx.n {
        let Some(kind) = class[start] else { continue };
        if face_region[start] != NO_REGION {
            continue;
        }
        // Collect the vertex-connected cluster. Stored refs are region id + 1
        // (0 = no region, see level::region_index).
        let re = regions.len() as u32 + 1;
        let mut faces = vec![start];
        let mut q = VecDeque::from([start]);
        face_region[start] = re;
        while let Some(cur) = q.pop_front() {
            for &nb in &ctx.adj[cur] {
                let nb = nb as usize;
                if class[nb] == Some(kind) && face_region[nb] == NO_REGION {
                    face_region[nb] = re;
                    faces.push(nb);
                    q.push_back(nb);
                }
            }
        }
        // Tiny scraps stay unnamed (towns and roads always name).
        let min_faces = match kind {
            RegionKind::Town | RegionKind::Road | RegionKind::River => 1,
            RegionKind::Forest => MIN_FOREST_FACES,
            RegionKind::Beach => MIN_BEACH_FACES,
            _ => 8,
        };
        if faces.len() < min_faces {
            for fi in faces {
                face_region[fi] = NO_REGION;
            }
            continue;
        }
        let cent = faces.iter().map(|&fi| ctx.centroid(fi).0).sum::<Vec3>().normalize_or(Vec3::Y);
        let idx = *kind_counts.entry(kind as u8).and_modify(|c| *c += 1).or_insert(0);
        let name = region_name(kind, idx, cent, terrain);
        regions.push(RegionData { name, pos: cent.to_array(), kind });
    }
    (regions, face_region)
}

fn region_name(kind: RegionKind, idx: usize, cent: Vec3, terrain: &TerrainGen) -> String {
    const OCEAN: [&str; 20] = ["Azure", "Cobalt", "Cerulean", "Sapphire", "Indigo", "Teal",
        "Aquamarine", "Turquoise", "Navy", "Sky", "Marine", "Coral", "Lagoon", "Reef",
        "Abyss", "Trench", "Gulf", "Bay", "Strait", "Channel"];
    const LAKE: [&str; 12] = ["Mirror", "Crystal", "Emerald", "Silver", "Misty", "Clear",
        "Loch", "Mere", "Tarn", "Pond", "Basin", "Hollow"];
    const RIVER: [&str; 12] = ["Serpent", "Winding", "Rushing", "Silver", "Mossy", "Deep",
        "Brook", "Stream", "Creek", "Fork", "Bend", "Rapids"];
    const BEACH: [&str; 10] = ["Silver", "Golden", "Pebble", "Shell", "Driftwood", "Coral",
        "Windswept", "Quiet", "Gull", "Smuggler's"];
    const CLIFF: [&str; 8] = ["Raven", "Grey", "Storm", "White", "Shear", "Widow's", "Falcon", "Chalk"];
    const FOREST: [&str; 10] = ["Elder", "Whisper", "Thorn", "Mossy", "Shadow", "Bright",
        "Tangle", "Hollow", "Fern", "Wolf"];
    const DESERT: [&str; 6] = ["Amber", "Bone", "Shimmer", "Red", "Glass", "Silent"];
    const MOUNTAIN: [&str; 8] = ["Iron", "Grey", "Storm", "Frost", "Raven", "Broken", "Cloud", "Thunder"];
    const PLAINS: [&str; 8] = ["Green", "Wide", "Amber", "Rolling", "Sunlit", "Long", "Low", "Open"];
    const TUNDRA: [&str; 6] = ["Pale", "Frozen", "White", "Bitter", "Still", "North"];
    const ROAD: [&str; 8] = ["Old", "King's", "Salt", "Trade", "Pilgrim's", "Coastal", "High", "Low"];

    let pick = |pool: &[&str], suffixes: &[&str]| {
        format!("{} {}", pool[idx % pool.len()], suffixes[(idx / pool.len()) % suffixes.len()])
    };
    match kind {
        RegionKind::Ocean => pick(&OCEAN, &["Ocean", "Sea"]),
        RegionKind::Lake => pick(&LAKE, &["Lake"]),
        RegionKind::River => pick(&RIVER, &["River"]),
        RegionKind::Beach => pick(&BEACH, &["Beach", "Coast", "Sands"]),
        RegionKind::Cliff => pick(&CLIFF, &["Cliffs", "Bluffs"]),
        RegionKind::Forest => pick(&FOREST, &["Forest", "Woods"]),
        RegionKind::Desert => pick(&DESERT, &["Desert", "Dunes"]),
        RegionKind::Mountain => pick(&MOUNTAIN, &["Peaks", "Range"]),
        RegionKind::Plains => pick(&PLAINS, &["Plains", "Fields"]),
        RegionKind::Tundra => pick(&TUNDRA, &["Tundra", "Wastes"]),
        RegionKind::Road => pick(&ROAD, &["Road"]),
        // Towns take the name of the settlement they surround.
        RegionKind::Town => {
            terrain.settlement_anchors.iter().enumerate()
                .min_by(|(_, a), (_, b)| {
                    a.0.dot(cent).partial_cmp(&b.0.dot(cent)).unwrap().reverse()
                })
                .map(|(i, _)| shared::roads::settlement_name(i))
                .unwrap_or_else(|| format!("Town {idx}"))
        }
    }
}

// ---- L6: mesh straight from the L3 vertex field (no mesh-time terrain edits) ----

pub fn build_mesh(
    ctx: &GenCtx,
    terrain: &TerrainGen,
    face_types: &[Terrain],
    painted: &PaintedFaces,
) -> (Vec<[[f32; 3]; 3]>, Vec<[f32; 4]>) {
    // Deduplicate vertices so shared corners get one radius — a watertight surface.
    let mut vmap: BTreeMap<[u64; 3], usize> = BTreeMap::new();
    let mut verts: Vec<Vec3> = Vec::new();
    let mut vert_r: Vec<f32> = Vec::new();
    let mut face_v: Vec<[usize; 3]> = Vec::with_capacity(ctx.n);
    for tri in &ctx.unit_tris {
        let mut idx = [0usize; 3];
        for (k, v) in tri.iter().enumerate() {
            let key = [v.x.to_bits() as u64, v.y.to_bits() as u64, v.z.to_bits() as u64];
            idx[k] = *vmap.entry(key).or_insert_with(|| {
                let i = verts.len();
                let dir = v.normalize();
                verts.push(dir);
                vert_r.push(terrain.render_radius(SpherePos::new(dir)));
                i
            });
        }
        face_v.push(idx);
    }

    // Cliff relief: raise cliff-top vertices while leaving the waterline edge
    // down, so cliffs read as an escarpment. Physics and actors follow the baked
    // mesh (collider + facet_radius), so this stays consistent with gameplay;
    // only the smooth-field HUD altitude reads slightly low on cliff tops.
    let mut touches_cliff = vec![false; verts.len()];
    let mut touches_water = vec![false; verts.len()];
    for (fi, idx) in face_v.iter().enumerate() {
        for &vi in idx {
            if face_types[fi] == Terrain::Cliff {
                touches_cliff[vi] = true;
            }
            if face_types[fi].is_water() {
                touches_water[vi] = true;
            }
        }
    }
    for vi in 0..verts.len() {
        if touches_cliff[vi] && !touches_water[vi] {
            vert_r[vi] += CLIFF_LIFT;
        }
    }

    let road_color = bevy_color::Color::srgb(0.5, 0.42, 0.3).to_linear().to_f32_array();
    let town_color = shared::theme::WARNING.to_linear().to_f32_array();
    let entry_color = bevy_color::Color::srgb(0.42, 0.33, 0.24).to_linear().to_f32_array();
    let mut tris = Vec::with_capacity(ctx.n);
    let mut cols = Vec::with_capacity(ctx.n);
    for (fi, [a, b, c]) in face_v.iter().enumerate() {
        let color = if painted.towns.contains(fi) {
            town_color
        } else if painted.bridge_entries.contains(fi) {
            entry_color
        } else if painted.roads.contains(fi) {
            road_color
        } else {
            face_types[fi].color().to_linear().to_f32_array()
        };
        tris.push([
            (verts[*a] * vert_r[*a]).to_array(),
            (verts[*b] * vert_r[*b]).to_array(),
            (verts[*c] * vert_r[*c]).to_array(),
        ]);
        cols.push(color);
    }
    (tris, cols)
}

pub fn build_face_tags(ctx: &GenCtx, painted: &PaintedFaces) -> (Vec<u32>, Vec<u8>) {
    let mut off = Vec::with_capacity(ctx.n + 1);
    let mut data = Vec::new();
    off.push(0u32);
    for fi in 0..ctx.n {
        if painted.roads.contains(fi) { data.push(TAG_ROAD); }
        if painted.towns.contains(fi) { data.push(TAG_TOWN); }
        if painted.bridges.contains(fi) { data.push(TAG_BRIDGE); }
        if painted.bridge_entries.contains(fi) { data.push(TAG_BRIDGE_ENTRY); }
        off.push(data.len() as u32);
    }
    (off, data)
}

// ---- serialize ----

pub fn serialize(
    ctx: &GenCtx,
    terrain: &TerrainGen,
    face_types: &[Terrain],
    bridges: &[Vec<SpherePos>],
    regions: Vec<RegionData>,
    face_region: Vec<u32>,
    terrain_tris: Vec<[[f32; 3]; 3]>,
    terrain_colors: Vec<[f32; 4]>,
    face_tag_off: Vec<u32>,
    face_tag_data: Vec<u8>,
) {
    let unit_tris_arr: Vec<[[f32; 3]; 3]> =
        ctx.unit_tris.iter().map(|[a, b, c]| [a.to_array(), b.to_array(), c.to_array()]).collect();
    let settlements = terrain.settlement_anchors.iter().enumerate()
        .map(|(i, a)| SettlementData { name: shared::roads::settlement_name(i), pos: a.0.to_array() })
        .collect();
    let mut roads: Vec<RoadData> = terrain.road_paths.iter()
        .map(|p| RoadData { points: p.iter().map(|s| s.0.to_array()).collect(), is_bridge: false })
        .collect();
    roads.extend(bridges.iter()
        .map(|p| RoadData { points: p.iter().map(|s| s.0.to_array()).collect(), is_bridge: true }));

    let data = LevelData {
        version: LEVEL_FORMAT_VERSION,
        seed: ctx.seed,
        terrain_tris,
        terrain_colors,
        unit_tris: unit_tris_arr,
        face_types: face_types.iter().map(|t| *t as u8).collect(),
        face_tag_off,
        face_tag_data,
        settlements,
        roads,
        regions,
        face_region,
    };
    let bytes = postcard::to_allocvec(&data).expect("serialize");
    let _ = fs::create_dir_all(ctx.out.parent().unwrap());
    fs::write(&ctx.out, &bytes).expect("write");
    println!("Wrote {} faces → {}", ctx.n, ctx.out.display());
}

/// Print generation stats for visual sanity checking without a renderer.
pub fn print_stats(ctx: &GenCtx, terrain: &TerrainGen, face_types: &[Terrain]) {
    let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
    for t in face_types {
        *counts.entry(match t {
            Terrain::DeepOcean => "DeepOcean", Terrain::Ocean => "Ocean",
            Terrain::Lake => "Lake", Terrain::LakeShore => "LakeShore",
            Terrain::River => "River", Terrain::RiverBank => "RiverBank",
            Terrain::Beach => "Beach", Terrain::Cliff => "Cliff",
            Terrain::Desert => "Desert", Terrain::Plains => "Plains",
            Terrain::Forest => "Forest", Terrain::Tundra => "Tundra",
            Terrain::Mountain => "Mountain", Terrain::Snow => "Snow",
        }).or_default() += 1;
    }
    let water: usize = face_types.iter().filter(|t| t.is_water()).count();
    println!("water: {:.1}%  breakdown: {:?}", water as f32 / ctx.n as f32 * 100.0, counts);
    println!(
        "settlements: {}  roads: {}  rivers: {}",
        terrain.settlement_anchors.len(),
        terrain.road_paths.len(),
        terrain.river_paths.len(),
    );
}

// ---- helpers ----

/// The gap-free chain of fine faces a polyline passes over.
fn face_chain(ctx: &GenCtx, points: &[SpherePos]) -> Vec<usize> {
    let mut c = Vec::new();
    for seg in points.windows(2) {
        let steps = (seg[0].distance(seg[1]) / 2.0).ceil().max(1.0) as usize;
        for k in 0..=steps {
            let p = seg[0].0.lerp(seg[1].0, k as f32 / steps as f32).normalize();
            if let Some(fi) = ctx.planet.face_at(p) {
                if c.last() != Some(&fi) {
                    // Bridge non-adjacent jumps so the chain has no holes.
                    if let Some(&prev) = c.last() {
                        if !ctx.adj[prev].contains(&(fi as u32)) {
                            c.extend(shortest_face_path(&ctx.adj, prev, fi));
                        }
                    }
                    c.push(fi);
                }
            }
        }
    }
    c
}

fn shortest_face_path(adj: &[[u32; 3]], u: usize, v: usize) -> Vec<usize> {
    let mut prev: BTreeMap<usize, usize> = BTreeMap::new();
    let mut q = VecDeque::from([(u, 0usize)]);
    prev.insert(u, u);
    while let Some((cur, d)) = q.pop_front() {
        if cur == v {
            let mut p = Vec::new();
            let mut c = v;
            while c != u {
                if c != v {
                    p.push(c);
                }
                c = prev[&c];
            }
            p.reverse();
            return p;
        }
        if d >= 4 {
            continue;
        }
        for &n in &adj[cur] {
            let n = n as usize;
            if n == u32::MAX as usize {
                continue;
            }
            prev.entry(n).or_insert_with(|| {
                q.push_back((n, d + 1));
                cur
            });
        }
    }
    Vec::new()
}
