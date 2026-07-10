//! Generation orchestrator: a decide/evolve/react command-event state machine.
//!
//! Each `Command` does exactly one thing. `decide` computes (pure over the
//! state), returning `Event`s that carry the results; `evolve` folds events
//! into the state; `react` maps events to follow-up commands on a FIFO queue.
//! The loop is deterministic (FIFO order, seeded RNG streams per pass), every
//! re-enqueue is bounded, and the event log is the audit trail when a seed
//! misbehaves. The runtime can later reuse `evolve` to replay events
//! (terraforming, dynamic world changes).

use bevy::color::ColorToComponents;
use bevy::prelude::Vec3;
use std::collections::{BTreeMap, VecDeque};

use crate::level::{RegionData, RegionKind, NO_REGION, TAG_BRIDGE, TAG_BRIDGE_ENTRY, TAG_ROAD, TAG_TOWN};
use crate::planet::{build_face_adjacency, unit_icosphere_tris, PlanetMesh};
use crate::sphere::SpherePos;
use crate::terrain::{Terrain, TerrainGen};
use crate::wfc;
use crate::zones::FINE_SUB;

const TOWN_RADIUS: f32 = 55.0;

// ---- fine grid ----

/// The fine icosphere the pipeline works on (sub=6, ~82k faces).
pub struct Grid {
    pub seed: u32,
    pub unit_tris: Vec<[Vec3; 3]>,
    pub planet: PlanetMesh,
    pub adj: Vec<[u32; 3]>,
    pub n: usize,
}

impl Grid {
    pub fn new(seed: u32) -> Self {
        let unit_tris = unit_icosphere_tris(FINE_SUB);
        let planet = PlanetMesh::new(unit_tris.clone());
        let adj = build_face_adjacency(&planet.tris()[..], unit_tris.len());
        let n = unit_tris.len();
        debug_assert!(adj.iter().all(|a| !a.contains(&u32::MAX)), "face adjacency incomplete");
        Self { seed, unit_tris, planet, adj, n }
    }

    pub fn centroid(&self, fi: usize) -> SpherePos {
        let [a, b, c] = self.unit_tris[fi];
        SpherePos::new(((a + b + c) / 3.0).normalize())
    }
}

// ---- bitset ----

#[derive(Clone)]
pub struct BitSet(pub Vec<u64>);

impl BitSet {
    pub fn new(n: usize) -> Self { Self(vec![0; n.div_ceil(64)]) }
    pub fn insert(&mut self, i: usize) { self.0[i >> 6] |= 1u64 << (i & 63); }
    pub fn contains(&self, i: usize) -> bool { self.0[i >> 6] & (1u64 << (i & 63)) != 0 }
}

#[derive(Clone)]
pub struct Painted {
    pub roads: BitSet,
    pub towns: BitSet,
    pub bridges: BitSet,
    pub bridge_entries: BitSet,
}

// ---- state ----

pub struct GenState {
    pub grid: Grid,
    pub terrain: Option<TerrainGen>,
    pub tiles: Vec<Terrain>,
    pub painted: Painted,
    pub bridges: Vec<Vec<SpherePos>>,
    /// Inland biome-boundary faces and the kind pair they link.
    pub blends: Vec<(u32, u8, u8)>,
    pub regions: Vec<RegionData>,
    pub face_region: Vec<u32>,
    pub mesh_tris: Vec<[[f32; 3]; 3]>,
    pub mesh_colors: Vec<[f32; 4]>,
    pub tag_off: Vec<u32>,
    pub tag_data: Vec<u8>,
}

impl GenState {
    pub fn new(seed: u32) -> Self {
        let grid = Grid::new(seed);
        let n = grid.n;
        Self {
            grid,
            terrain: None,
            tiles: Vec::new(),
            painted: Painted {
                roads: BitSet::new(n),
                towns: BitSet::new(n),
                bridges: BitSet::new(n),
                bridge_entries: BitSet::new(n),
            },
            bridges: Vec::new(),
            blends: Vec::new(),
            regions: Vec::new(),
            face_region: Vec::new(),
            mesh_tris: Vec::new(),
            mesh_colors: Vec::new(),
            tag_off: Vec::new(),
            tag_data: Vec::new(),
        }
    }

    fn terrain(&self) -> &TerrainGen {
        self.terrain.as_ref().expect("terrain not generated yet")
    }
}

// ---- commands & events ----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// L0–L3: zones, topology, elevation field (inside TerrainGen).
    GenTerrain,
    /// Zone-aware base classification per fine face.
    ClassifyTiles,
    /// Snap river polylines to fine faces.
    PaintRivers,
    /// Water identity from connectivity (ocean vs enclosed lake).
    NormalizeWater,
    /// Roads + towns face sets.
    PaintFeatures,
    /// Shore banding, micro WFC, coast segmentation, forest min-size.
    ResolveTransitions,
    /// Mark inland biome-boundary faces with the kind pair they link
    /// (the blend layer — texture/terrain transitions later).
    MarkBlends,
    /// Synthesize the final elevation field from the finished tile map:
    /// per-tile elevation ranges + per-pair gradient caps + river descent +
    /// road corridor caps, solved to a fixed point.
    SolveElevation,
    /// Named feature clusters.
    BuildRegions,
    /// Bridge spans between landmasses (needs regions).
    SelectBridges,
    /// Displaced trimesh + colors.
    BuildMesh,
    /// Per-face tag table.
    BuildTags,
}

pub enum Event {
    TerrainReady(Box<TerrainGen>),
    TilesClassified(Vec<Terrain>),
    RiversPainted(Vec<Terrain>),
    WaterNormalized(Vec<Terrain>),
    FeaturesPainted(Painted),
    TransitionsResolved(Vec<Terrain>),
    BlendsMarked(Vec<(u32, u8, u8)>),
    ElevationSolved { field: Vec<f32>, iters: usize, residual: f32 },
    RegionsBuilt(Vec<RegionData>, Vec<u32>),
    BridgesSelected(Vec<Vec<SpherePos>>, Painted),
    MeshBuilt(Vec<[[f32; 3]; 3]>, Vec<[f32; 4]>),
    TagsBuilt(Vec<u32>, Vec<u8>),
}

impl Event {
    /// Short line for the audit log.
    pub fn label(&self) -> String {
        match self {
            Event::TerrainReady(t) => format!(
                "terrain ready: {} settlements, {} roads, {} rivers",
                t.settlement_anchors.len(), t.road_paths.len(), t.river_paths.len()
            ),
            Event::TilesClassified(t) => format!("tiles classified: {}", t.len()),
            Event::RiversPainted(_) => "rivers painted".into(),
            Event::WaterNormalized(_) => "water bodies normalized".into(),
            Event::FeaturesPainted(_) => "roads + towns painted".into(),
            Event::TransitionsResolved(_) => "transitions resolved".into(),
            Event::BlendsMarked(b) => format!("blends marked: {}", b.len()),
            Event::ElevationSolved { iters, residual, .. } => {
                format!("elevation solved: {iters} iterations, residual {residual:.4}")
            }
            Event::RegionsBuilt(r, _) => format!("regions built: {}", r.len()),
            Event::BridgesSelected(b, _) => format!("bridges selected: {}", b.len()),
            Event::MeshBuilt(t, _) => format!("mesh built: {} tris", t.len()),
            Event::TagsBuilt(_, d) => format!("tags built: {} entries", d.len()),
        }
    }
}

// ---- decide / evolve / react ----

pub fn decide(state: &GenState, cmd: &Command) -> Vec<Event> {
    match cmd {
        Command::GenTerrain => {
            vec![Event::TerrainReady(Box::new(TerrainGen::new(state.grid.seed)))]
        }
        Command::ClassifyTiles => {
            let terrain = state.terrain();
            let tiles = (0..state.grid.n)
                .map(|fi| terrain.base_classify_fine(fi, state.grid.centroid(fi)))
                .collect();
            vec![Event::TilesClassified(tiles)]
        }
        Command::PaintRivers => {
            let mut tiles = state.tiles.clone();
            paint_rivers(&state.grid, state.terrain(), &mut tiles);
            vec![Event::RiversPainted(tiles)]
        }
        Command::NormalizeWater => {
            let mut tiles = state.tiles.clone();
            normalize_water_bodies(&state.grid, state.terrain(), &mut tiles);
            vec![Event::WaterNormalized(tiles)]
        }
        Command::PaintFeatures => {
            vec![Event::FeaturesPainted(paint_features(&state.grid, state.terrain()))]
        }
        Command::ResolveTransitions => {
            let vadj = build_vertex_adjacency(&state.grid);
            let tiles = resolve_transitions(&state.grid, state.terrain(), &vadj, &state.tiles);
            vec![Event::TransitionsResolved(tiles)]
        }
        Command::MarkBlends => {
            vec![Event::BlendsMarked(mark_blends(&state.grid, &state.tiles))]
        }
        Command::SolveElevation => {
            let (field, iters, residual) =
                solve_elevation(&state.grid, state.terrain(), &state.tiles, &state.painted);
            vec![Event::ElevationSolved { field, iters, residual }]
        }
        Command::BuildRegions => {
            let (regions, face_region) =
                build_regions(&state.grid, state.terrain(), &state.tiles, &state.painted);
            vec![Event::RegionsBuilt(regions, face_region)]
        }
        Command::SelectBridges => {
            let mut painted = state.painted.clone();
            let bridges =
                build_bridges(&state.grid, &state.tiles, &state.face_region, &mut painted);
            vec![Event::BridgesSelected(bridges, painted)]
        }
        Command::BuildMesh => {
            let (tris, cols) = build_mesh(
                &state.grid, state.terrain(), &state.tiles, &state.painted, &state.blends,
            );
            vec![Event::MeshBuilt(tris, cols)]
        }
        Command::BuildTags => {
            let (off, data) = build_face_tags(&state.grid, &state.painted);
            vec![Event::TagsBuilt(off, data)]
        }
    }
}

pub fn evolve(mut state: GenState, event: Event) -> GenState {
    match event {
        Event::TerrainReady(t) => state.terrain = Some(*t),
        Event::TilesClassified(t)
        | Event::RiversPainted(t)
        | Event::WaterNormalized(t)
        | Event::TransitionsResolved(t) => state.tiles = t,
        Event::FeaturesPainted(p) => state.painted = p,
        Event::BlendsMarked(b) => state.blends = b,
        Event::ElevationSolved { field, .. } => {
            state.terrain.as_mut().expect("terrain exists").set_vert_elevations(field);
        }
        Event::RegionsBuilt(r, fr) => {
            state.regions = r;
            state.face_region = fr;
        }
        Event::BridgesSelected(b, p) => {
            state.bridges = b;
            state.painted = p;
        }
        Event::MeshBuilt(t, c) => {
            state.mesh_tris = t;
            state.mesh_colors = c;
        }
        Event::TagsBuilt(off, data) => {
            state.tag_off = off;
            state.tag_data = data;
        }
    }
    state
}

pub fn react(event: &Event) -> Vec<Command> {
    match event {
        Event::TerrainReady(_) => vec![Command::ClassifyTiles],
        Event::TilesClassified(_) => vec![Command::PaintRivers],
        Event::RiversPainted(_) => vec![Command::NormalizeWater],
        Event::WaterNormalized(_) => vec![Command::PaintFeatures],
        Event::FeaturesPainted(_) => vec![Command::ResolveTransitions],
        Event::TransitionsResolved(_) => vec![Command::MarkBlends],
        Event::BlendsMarked(_) => vec![Command::SolveElevation],
        Event::ElevationSolved { .. } => vec![Command::BuildRegions],
        Event::RegionsBuilt(..) => vec![Command::SelectBridges],
        Event::BridgesSelected(..) => vec![Command::BuildMesh],
        Event::MeshBuilt(..) => vec![Command::BuildTags],
        Event::TagsBuilt(..) => vec![],
    }
}

/// Run the full pipeline for a seed. `log` receives one line per event.
pub fn run(seed: u32, mut log: impl FnMut(&str)) -> GenState {
    let mut state = GenState::new(seed);
    let mut queue = VecDeque::from([Command::GenTerrain]);
    while let Some(cmd) = queue.pop_front() {
        for event in decide(&state, &cmd) {
            log(&event.label());
            queue.extend(react(&event));
            state = evolve(state, event);
        }
    }
    state
}

// ---- command implementations (single responsibility each) ----

/// River polylines → River faces (land only; the mouth is already water).
fn paint_rivers(grid: &Grid, terrain: &TerrainGen, face_types: &mut [Terrain]) {
    for path in &terrain.river_paths {
        for fi in face_chain(grid, path) {
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
fn normalize_water_bodies(grid: &Grid, terrain: &TerrainGen, face_types: &mut [Terrain]) {
    let mut visited = vec![false; grid.n];
    for start in 0..grid.n {
        if !face_types[start].is_water() || face_types[start] == Terrain::River || visited[start] {
            continue;
        }
        let mut body = vec![start];
        let mut q = VecDeque::from([start]);
        visited[start] = true;
        while let Some(cur) = q.pop_front() {
            for &nb in &grid.adj[cur] {
                let nb = nb as usize;
                if face_types[nb].is_water() && face_types[nb] != Terrain::River && !visited[nb] {
                    visited[nb] = true;
                    body.push(nb);
                    q.push_back(nb);
                }
            }
        }
        let is_ocean = body.iter()
            .any(|&fi| terrain.zones().kind_at_fine(fi) == crate::zones::ZoneKind::Ocean);
        if !is_ocean && body.len() < MIN_WATER_BODY_FACES {
            // A puddle isn't a lake: fill it with the most common surrounding
            // land kind so no 1-tile water ever survives.
            let mut counts: BTreeMap<u8, usize> = BTreeMap::new();
            for &fi in &body {
                for &nb in &grid.adj[fi] {
                    let t = face_types[nb as usize];
                    if t.is_land() {
                        *counts.entry(t as u8).or_default() += 1;
                    }
                }
            }
            let fill = counts.iter().max_by_key(|(_, c)| **c)
                .map(|(&k, _)| Terrain::ALL[k as usize])
                .unwrap_or(Terrain::Plains);
            for &fi in &body {
                face_types[fi] = fill;
            }
            continue;
        }
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

/// Enclosed water smaller than this is filled to land — a couple of tiles of
/// "lake" beside the ocean makes no sense.
const MIN_WATER_BODY_FACES: usize = 30;

fn paint_features(grid: &Grid, terrain: &TerrainGen) -> Painted {
    let mut roads = BitSet::new(grid.n);
    for path in &terrain.road_paths {
        for fi in face_chain(grid, path) {
            roads.insert(fi);
        }
    }
    // Bridges are painted later, once regions exist to validate their endpoints.
    let bridges = BitSet::new(grid.n);
    let mut towns = BitSet::new(grid.n);
    for fi in 0..grid.n {
        let cent = grid.centroid(fi);
        if terrain.settlement_anchors.iter().any(|a| a.distance(cent) <= TOWN_RADIUS) {
            towns.insert(fi);
        }
    }
    Painted { roads, towns, bridges, bridge_entries: BitSet::new(grid.n) }
}

/// Gaps up to this bridge freely.
const BRIDGE_MAX_SPAN: f32 = 250.0;
/// Longer gaps (up to this) are bridged only to connect an otherwise
/// unreachable landmass — every island gets at least one way in.
const BRIDGE_CONNECT_SPAN: f32 = 500.0;
const BRIDGE_MAX_COUNT: usize = 6;
/// How far a bridge deck reaches inland past its shore face, meters.
const BRIDGE_ENTRY_OVERLAP: f32 = 25.0;

/// Pick bridges from the fine map, where true water separation is known: each
/// bridge runs from a shore-band face of one named region to a shore-band face
/// of a *different* region on a different landmass, crossing open water.
fn build_bridges(
    grid: &Grid,
    face_types: &[Terrain],
    face_region: &[u32],
    painted: &mut Painted,
) -> Vec<Vec<SpherePos>> {
    // Landmasses: edge-connected components of land faces.
    let mut comp = vec![u32::MAX; grid.n];
    let mut count = 0u32;
    for fi in 0..grid.n {
        if face_types[fi].is_water() || comp[fi] != u32::MAX {
            continue;
        }
        let mut q = VecDeque::from([fi]);
        comp[fi] = count;
        while let Some(cur) = q.pop_front() {
            for &nb in &grid.adj[cur] {
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
    // cliff tops are high by construction, and a deck ending on one becomes a wall.
    let mut heads: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
    for fi in 0..grid.n {
        if face_types[fi] == Terrain::Beach
            && crate::level::region_index(face_region[fi]).is_some()
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
                    let d = grid.centroid(fa).distance(grid.centroid(fb));
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
        let (a, b) = (grid.centroid(fa), grid.centroid(fb));
        // Extend past both shore faces so the deck grounds on solid land instead
        // of ending exactly at the waterline face centroid.
        let ext = BRIDGE_ENTRY_OVERLAP / d;
        let steps = (d * (1.0 + 2.0 * ext) / 10.0).ceil().max(2.0) as usize;
        let span: Vec<SpherePos> = (0..=steps)
            .map(|k| crate::sphere::slerp(a, b, -ext + (1.0 + 2.0 * ext) * k as f32 / steps as f32))
            .collect();
        let crosses_water = span.iter().any(|p| {
            grid.planet.face_at(p.0).is_some_and(|fi| face_types[fi].is_water())
        });
        if !crosses_water {
            continue;
        }
        for fi in face_chain(grid, &span) {
            painted.bridges.insert(fi);
        }
        // Bridge entries: the land faces around each deck end.
        for end in [span.first(), span.last()] {
            let Some(fi) = end.and_then(|p| grid.planet.face_at(p.0)) else { continue };
            if face_types[fi].is_land() {
                painted.bridge_entries.insert(fi);
            }
            for &nb in &grid.adj[fi] {
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

/// Faces sharing at least one vertex with each face (up to ~12). Used only to
/// detect water contact (a corner-touching inlet still makes a face coastal) —
/// tile LINKING is always edge-based.
fn build_vertex_adjacency(grid: &Grid) -> Vec<Vec<u32>> {
    let mut by_vert: BTreeMap<u64, Vec<u32>> = BTreeMap::new();
    for (fi, tri) in grid.unit_tris.iter().enumerate() {
        for v in tri {
            by_vert.entry(vkey(*v)).or_default().push(fi as u32);
        }
    }
    let mut adj: Vec<Vec<u32>> = vec![Vec::new(); grid.n];
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

fn resolve_transitions(grid: &Grid, terrain: &TerrainGen, vadj: &[Vec<u32>], base: &[Terrain]) -> Vec<Terrain> {
    // Shorelines are deterministic bands, not WFC cells: every land face touching
    // water gets its shore tile, so the waterline is never zigzagged by chance.
    // The band widens onto the second ring where the coast is flat, and a land
    // face wedged between two shore faces joins the band (no plains notches).
    let mut out = base.to_vec();
    let (water_dist, water_kind) = water_distance(grid, base, 2);
    let shore = |fi: usize, kind: Terrain| -> Terrain {
        match kind {
            Terrain::Lake => Terrain::LakeShore,
            Terrain::River => Terrain::RiverBank,
            _ => {
                let steep = matches!(base[fi], Terrain::Mountain | Terrain::Snow)
                    || terrain.elevation_at(grid.centroid(fi)) > 0.15;
                if steep { Terrain::Cliff } else { Terrain::Beach }
            }
        }
    };
    for fi in 0..grid.n {
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
        } else if water_dist[fi] <= 2 && terrain.elevation_at(grid.centroid(fi)) < 0.08 {
            // Low, flat coast: the band is more than one tile wide.
            out[fi] = shore(fi, water_kind[fi].unwrap_or(Terrain::Ocean));
        }
    }
    // Fill notches: a land face with ≥2 edge neighbors in the shore band belongs
    // to the band too.
    let banded: Vec<bool> = (0..grid.n)
        .map(|fi| matches!(out[fi], Terrain::Beach | Terrain::Cliff | Terrain::LakeShore | Terrain::RiverBank))
        .collect();
    for fi in 0..grid.n {
        if base[fi].is_water() || banded[fi] {
            continue;
        }
        if grid.adj[fi].iter().filter(|&&nb| banded[nb as usize]).count() >= 2 {
            out[fi] = shore(fi, water_kind[fi].unwrap_or(Terrain::Ocean));
        }
    }

    // Cells for WFC: remaining land faces bordering a different classification —
    // inland biome edges. Water and the shore band enter as fixed neighbors.
    let in_band: Vec<bool> = (0..grid.n).map(|fi| out[fi] != base[fi]).collect();
    let base = &out;
    let mut cell_of = vec![usize::MAX; grid.n];
    let mut cells: Vec<usize> = Vec::new();
    for fi in 0..grid.n {
        if base[fi].is_water() || in_band[fi] {
            continue;
        }
        if grid.adj[fi].iter().any(|&nb| base[nb as usize] != base[fi]) {
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
        grid.adj[fi].iter().map(|&nb| {
            let nb = nb as usize;
            match cell_of[nb] {
                usize::MAX => wfc::Neighbor::Fixed(base[nb]),
                ci => wfc::Neighbor::Cell(ci),
            }
        }).collect()
    }).collect();
    let fallback: Vec<Terrain> = cells.iter().map(|&fi| base[fi]).collect();

    let solved = wfc::solve(&wfc::Compat::default(), &domains, &neighbors, &fallback, grid.seed as u64);

    let mut resolved = base.clone();
    for (ci, &fi) in cells.iter().enumerate() {
        resolved[fi] = solved[ci];
    }
    segment_coastline(grid, terrain, &mut resolved);
    absorb_small_forests(grid, &mut resolved);
    enforce_counting_rule(grid, &mut resolved);
    resolved
}

/// Inland biome boundaries get a blend mark: the face keeps its kind, but
/// carries the pair it links so rendering can transition between the two.
/// Shore tiles already ARE transitions and water never blends.
fn mark_blends(grid: &Grid, tiles: &[Terrain]) -> Vec<(u32, u8, u8)> {
    let plain = |t: Terrain| {
        t.is_land()
            && !matches!(t, Terrain::Beach | Terrain::Cliff | Terrain::LakeShore | Terrain::RiverBank)
    };
    let mut out = Vec::new();
    for fi in 0..grid.n {
        if !plain(tiles[fi]) {
            continue;
        }
        // Most common differing plain neighbor kind (deterministic tie-break).
        let mut counts: BTreeMap<u8, usize> = BTreeMap::new();
        for &nb in &grid.adj[fi] {
            let t = tiles[nb as usize];
            if plain(t) && t != tiles[fi] {
                *counts.entry(t as u8).or_default() += 1;
            }
        }
        if let Some((&other, _)) = counts.iter().max_by_key(|(_, c)| **c) {
            out.push((fi as u32, tiles[fi] as u8, other));
        }
    }
    out
}

/// The tile-linking rule: a non-transition land tile must share an edge with at
/// least 2 tiles of its own kind — no orphans, no one-tile necks. Transition
/// tiles (shore band) and Rivers (linear by nature) are the sanctioned linking
/// tiles and are exempt. Violators join their most common neighbor kind;
/// iterate to a fixed point (each pass only removes violators).
fn enforce_counting_rule(grid: &Grid, out: &mut [Terrain]) {
    let exempt = |t: Terrain| {
        t.is_water()
            || matches!(t, Terrain::Beach | Terrain::Cliff | Terrain::LakeShore | Terrain::RiverBank)
    };
    for _ in 0..64 {
        let mut changed = false;
        for fi in 0..grid.n {
            if exempt(out[fi]) {
                continue;
            }
            let same = grid.adj[fi].iter().filter(|&&nb| out[nb as usize] == out[fi]).count();
            if same >= 2 {
                continue;
            }
            // Flip only to a kind that actually stabilizes the tile (≥2 edges
            // of that kind). A tile with three different neighbors CAN'T be
            // stabilized — it is a genuine boundary tile and the blend layer
            // marks it as the linking tile instead.
            let mut counts: BTreeMap<u8, usize> = BTreeMap::new();
            for &nb in &grid.adj[fi] {
                let t = out[nb as usize];
                if t.is_land() && t != out[fi] {
                    *counts.entry(t as u8).or_default() += 1;
                }
            }
            if let Some((&k, _)) = counts.iter().filter(|(_, c)| **c >= 2).next() {
                out[fi] = Terrain::ALL[k as usize];
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
}

/// A forest smaller than this many faces is just some trees in a field.
const MIN_FOREST_FACES: usize = 10;

fn absorb_small_forests(grid: &Grid, out: &mut [Terrain]) {
    let mut visited = vec![false; grid.n];
    for start in 0..grid.n {
        if out[start] != Terrain::Forest || visited[start] {
            continue;
        }
        let mut cluster = vec![start];
        let mut q = VecDeque::from([start]);
        visited[start] = true;
        while let Some(cur) = q.pop_front() {
            for &nb in &grid.adj[cur] {
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
const MIN_BEACH_FACES: usize = 10;

/// Break each continuous ocean shore band into alternating Beach and Cliff
/// segments with bounded lengths — beaches can't wrap a whole continent as one
/// region, and each cliff range between them becomes a named region too.
/// Steep faces still force Cliff regardless of alternation.
fn segment_coastline(grid: &Grid, terrain: &TerrainGen, out: &mut [Terrain]) {
    let in_band: Vec<bool> = (0..grid.n)
        .map(|fi| matches!(out[fi], Terrain::Beach | Terrain::Cliff))
        .collect();
    let mut visited = vec![false; grid.n];
    for start in 0..grid.n {
        if !in_band[start] || visited[start] {
            continue;
        }
        // BFS order approximates walking along the thin band.
        let mut order = vec![start];
        let mut q = VecDeque::from([start]);
        visited[start] = true;
        while let Some(cur) = q.pop_front() {
            for &nb in &grid.adj[cur] {
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
            let steep = terrain.elevation_at(grid.centroid(fi)) > 0.15;
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
        for fi in 0..grid.n {
            if !matches!(out[fi], Terrain::Beach | Terrain::Cliff) {
                continue;
            }
            let mut same = 0;
            let mut other = 0;
            for &nb in &grid.adj[fi] {
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
    let mut visited = vec![false; grid.n];
    for start in 0..grid.n {
        if out[start] != Terrain::Beach || visited[start] {
            continue;
        }
        let mut cluster = vec![start];
        let mut q = VecDeque::from([start]);
        visited[start] = true;
        while let Some(cur) = q.pop_front() {
            for &nb in &grid.adj[cur] {
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

/// BFS distance (in face steps, capped at `max_dist`) from each land face to the
/// nearest water face, plus which water terrain is nearest (for shore tile choice).
fn water_distance(grid: &Grid, base: &[Terrain], max_dist: u8) -> (Vec<u8>, Vec<Option<Terrain>>) {
    let mut dist = vec![u8::MAX; grid.n];
    let mut kind: Vec<Option<Terrain>> = vec![None; grid.n];
    let mut q = VecDeque::new();
    for fi in 0..grid.n {
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
        for &nb in &grid.adj[cur] {
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

// ---- named regions: contiguous feature clusters (edge-connected) ----

/// Which nameable feature a face belongs to. Tags win over terrain so towns and
/// roads cluster as themselves; LakeShore/RiverBank separate regions and stay
/// unnamed.
fn region_class(face_types: &[Terrain], painted: &Painted, fi: usize) -> Option<RegionKind> {
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
fn build_regions(
    grid: &Grid,
    terrain: &TerrainGen,
    face_types: &[Terrain],
    painted: &Painted,
) -> (Vec<RegionData>, Vec<u32>) {
    let class: Vec<Option<RegionKind>> =
        (0..grid.n).map(|fi| region_class(face_types, painted, fi)).collect();

    let mut face_region = vec![NO_REGION; grid.n];
    let mut regions: Vec<RegionData> = Vec::new();
    let mut kind_counts: BTreeMap<u8, usize> = BTreeMap::new();

    for start in 0..grid.n {
        let Some(kind) = class[start] else { continue };
        if face_region[start] != NO_REGION {
            continue;
        }
        // Collect the edge-connected cluster. Stored refs are region id + 1
        // (0 = no region, see level::region_index).
        let re = regions.len() as u32 + 1;
        let mut faces = vec![start];
        let mut q = VecDeque::from([start]);
        face_region[start] = re;
        while let Some(cur) = q.pop_front() {
            for &nb in &grid.adj[cur] {
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
        let cent = faces.iter().map(|&fi| grid.centroid(fi).0).sum::<Vec3>().normalize_or(Vec3::Y);
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
                .map(|(i, _)| crate::roads::settlement_name(i))
                .unwrap_or_else(|| format!("Town {idx}"))
        }
    }
}

// ---- mesh ----

fn build_mesh(
    grid: &Grid,
    terrain: &TerrainGen,
    face_types: &[Terrain],
    painted: &Painted,
    blends: &[(u32, u8, u8)],
) -> (Vec<[[f32; 3]; 3]>, Vec<[f32; 4]>) {
    let mut blend_of: BTreeMap<u32, (u8, u8)> = BTreeMap::new();
    for &(fi, a, b) in blends {
        blend_of.insert(fi, (a, b));
    }
    // Deduplicate vertices so shared corners get one radius — a watertight surface.
    let mut vmap: BTreeMap<[u64; 3], usize> = BTreeMap::new();
    let mut verts: Vec<Vec3> = Vec::new();
    let mut vert_r: Vec<f32> = Vec::new();
    let mut face_v: Vec<[usize; 3]> = Vec::with_capacity(grid.n);
    for tri in &grid.unit_tris {
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

    let road_color = bevy::prelude::Color::srgb(0.5, 0.42, 0.3).to_linear().to_f32_array();
    let town_color = crate::theme::WARNING.to_linear().to_f32_array();
    let entry_color = bevy::prelude::Color::srgb(0.42, 0.33, 0.24).to_linear().to_f32_array();
    let mut tris = Vec::with_capacity(grid.n);
    let mut cols = Vec::with_capacity(grid.n);
    for (fi, [a, b, c]) in face_v.iter().enumerate() {
        let color = if painted.towns.contains(fi) {
            town_color
        } else if painted.bridge_entries.contains(fi) {
            entry_color
        } else if painted.roads.contains(fi) {
            road_color
        } else if let Some(&(ka, kb)) = blend_of.get(&(fi as u32)) {
            // Boundary face: blend the two linked kinds 50/50.
            let ca = Terrain::ALL[ka as usize].color().to_linear().to_f32_array();
            let cb = Terrain::ALL[kb as usize].color().to_linear().to_f32_array();
            [
                (ca[0] + cb[0]) / 2.0,
                (ca[1] + cb[1]) / 2.0,
                (ca[2] + cb[2]) / 2.0,
                (ca[3] + cb[3]) / 2.0,
            ]
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

fn build_face_tags(grid: &Grid, painted: &Painted) -> (Vec<u32>, Vec<u8>) {
    let mut off = Vec::with_capacity(grid.n + 1);
    let mut data = Vec::new();
    off.push(0u32);
    for fi in 0..grid.n {
        if painted.roads.contains(fi) { data.push(TAG_ROAD); }
        if painted.towns.contains(fi) { data.push(TAG_TOWN); }
        if painted.bridges.contains(fi) { data.push(TAG_BRIDGE); }
        if painted.bridge_entries.contains(fi) { data.push(TAG_BRIDGE_ENTRY); }
        off.push(data.len() as u32);
    }
    (off, data)
}


// ---- elevation synthesis (SolveElevation) ----
//
// The final elevation field is CONSTRAINT-SOLVED from the finished tile map:
// every tile kind declares the elevation range its ground may occupy, every
// kind pair declares how fast elevation may change across one vertex edge
// (~70m), rivers must descend monotonically, and road corridors stay gentle.
// A deterministic Gauss-Seidel loop drives the proposed field into the
// constraint set. There are no later touchups — the mesh projects this field.

/// Elevation range (in [-1,1] units; 1.0 land unit = 500m) a tile's ground
/// may occupy.
fn elev_range(t: Terrain) -> (f32, f32) {
    use Terrain::*;
    // Ranges of kinds that may sit next to each other must overlap (or lie
    // within one edge's gradient cap) or the constraint set is unsatisfiable.
    match t {
        DeepOcean => (-1.0, -0.15),
        Ocean => (-0.35, -0.01),
        Lake => (-0.25, -0.03),
        LakeShore => (0.0, 0.08),
        // Rivers descend from mountains to the sea; their range must span it.
        River => (-1.0, 0.60),
        RiverBank => (0.0, 0.65),
        Beach => (0.0, 0.05),
        // Cliff relief lives in the field itself (no mesh lift).
        Cliff => (0.12, 0.40),
        Desert | Plains | Forest | Tundra => (0.02, 0.50),
        Mountain => (0.45, 1.0),
        Snow => (0.50, 1.0),
    }
}

/// Max elevation change across one vertex edge (~70m) between two tile kinds.
/// Small at shores (continental shelf), large into mountains and at cliffs.
fn max_gradient(a: Terrain, b: Terrain) -> f32 {
    use Terrain::*;
    let water = |t: Terrain| matches!(t, Ocean | DeepOcean | Lake);
    let peak = |t: Terrain| matches!(t, Mountain | Snow);
    // Rivers are canyons: their walls may be steep wherever they cut through.
    let base: f32 = if matches!(a, River | RiverBank) || matches!(b, River | RiverBank) {
        0.45
    } else if a == Cliff || b == Cliff {
        0.20
    } else if peak(a) || peak(b) {
        0.22
    } else if water(a) && water(b) {
        0.08
    } else if water(a) || water(b) || a == Beach || b == Beach {
        0.03
    } else {
        0.10
    };
    // Feasibility: a cap can never be tighter than the jump the two kinds'
    // disjoint elevation ranges force — otherwise range clamp and gradient cap
    // fight forever (e.g. a mountain vertex right beside a lakeshore vertex).
    let (alo, ahi) = elev_range(a);
    let (blo, bhi) = elev_range(b);
    let forced_gap = (blo - ahi).max(alo - bhi).max(0.0);
    base.max(forced_gap + 0.01)
}

/// Tight cap along road corridors so roads stay walkable.
const ROAD_EDGE_GRADIENT: f32 = 0.02;
const SOLVER_MAX_ITERS: usize = 150;
const SOLVER_EPS: f32 = 0.002;

fn kernel_interp(kernel: &[(usize, f32); 6], values: &[f32]) -> f32 {
    let mut sum = 0.0f32;
    let mut weighted = 0.0f32;
    for &(vi, dot) in kernel {
        let w = 1.0 / (1.01 - dot).max(0.01);
        weighted += values[vi] * w;
        sum += w;
    }
    if sum > 0.0 { weighted / sum } else { values[kernel[0].0] }
}

fn solve_elevation(
    grid: &Grid,
    terrain: &TerrainGen,
    tiles: &[Terrain],
    painted: &Painted,
) -> (Vec<f32>, usize, f32) {
    let nv = terrain.vert_count();

    // Per-vertex interval and kind: each vertex takes the range of the tile
    // directly under it — the same mapping the gradient caps use, so the
    // constraint set is self-consistent by construction. Transitions between
    // kinds are shaped by the edge caps, not by range intersections.
    let owner: Vec<Terrain> = (0..nv)
        .map(|vi| {
            grid.planet.face_at(terrain.vert_dir(vi))
                .map(|fi| tiles[fi])
                .unwrap_or(Terrain::Plains)
        })
        .collect();
    let mut lo = vec![-1.0f32; nv];
    let mut hi = vec![1.0f32; nv];
    let mut is_road_vert = vec![false; nv];
    for vi in 0..nv {
        let (rlo, rhi) = elev_range(owner[vi]);
        lo[vi] = rlo;
        hi[vi] = rhi;
    }
    for fi in 0..grid.n {
        if painted.roads.contains(fi) {
            for &(vi, _) in terrain.kernel(grid.centroid(fi))[..3].iter() {
                is_road_vert[vi] = true;
            }
        }
    }
    // Continental shelf as a HARD range: water depth may only grow with
    // distance from land (per vertex step ~70m), so the sea floor can never
    // wall off right at the shore no matter what chains the tile map builds.
    {
        let mut dist = vec![u8::MAX; nv];
        let mut q: VecDeque<usize> = VecDeque::new();
        for vi in 0..nv {
            if !owner[vi].is_water() {
                dist[vi] = 0;
                q.push_back(vi);
            }
        }
        while let Some(cur) = q.pop_front() {
            if dist[cur] >= 3 {
                continue;
            }
            for &nb in terrain.adj_of(cur) {
                if dist[nb] == u8::MAX {
                    dist[nb] = dist[cur] + 1;
                    q.push_back(nb);
                }
            }
        }
        for vi in 0..nv {
            if owner[vi].is_water() {
                let shelf = match dist[vi] {
                    1 => -0.20,
                    2 => -0.45,
                    3 => -0.70,
                    _ => -1.0,
                };
                lo[vi] = lo[vi].max(shelf);
                // Keep the interval non-empty against the owner's ceiling.
                hi[vi] = hi[vi].max(lo[vi]);
            }
        }
    }

    // River sample kernels for the monotone-descent constraint. Every vertex a
    // river sample touches is canyon ground: river range + canyon gradient caps.
    let river_kernels: Vec<Vec<[(usize, f32); 6]>> = terrain.river_paths.iter()
        .map(|path| path.iter().map(|s| terrain.kernel(*s)).collect())
        .collect();
    let mut is_canyon_vert = vec![false; nv];
    for kernels in &river_kernels {
        for kernel in kernels {
            for &(vi, _) in kernel {
                is_canyon_vert[vi] = true;
                let (rlo, rhi) = elev_range(Terrain::River);
                lo[vi] = rlo;
                hi[vi] = rhi;
            }
        }
    }

    // Per-vertex kind for gradient caps (canyon verts count as River).
    let vkind: Vec<Terrain> = (0..nv)
        .map(|vi| if is_canyon_vert[vi] { Terrain::River } else { owner[vi] })
        .collect();

    let mut e: Vec<f32> = terrain.vert_elevations().to_vec();
    let mut iters = 0;
    let mut residual = f32::MAX;
    for it in 0..SOLVER_MAX_ITERS {
        residual = 0.0;
        // 1) gradient caps per edge (best-effort smoothing).
        for a in 0..nv {
            for &b in terrain.adj_of(a) {
                if b <= a {
                    continue;
                }
                let mut cap = max_gradient(vkind[a], vkind[b]);
                if is_road_vert[a] && is_road_vert[b] {
                    cap = cap.min(ROAD_EDGE_GRADIENT);
                }
                let d = e[a] - e[b];
                if d.abs() > cap {
                    let excess = (d.abs() - cap) / 2.0;
                    let dir = d.signum();
                    e[a] -= dir * excess;
                    e[b] += dir * excess;
                    residual += excess;
                }
            }
        }
        // 2) rivers descend monotonically (lower-only kernel clamp) — but only
        // until they reach the sea; below the surface the river IS the ocean
        // and fighting its ranges would oscillate forever.
        for kernels in &river_kernels {
            let mut floor = f32::MAX;
            for kernel in kernels {
                let v = kernel_interp(kernel, &e);
                if floor < -0.05 {
                    break;
                }
                if v > floor + 0.001 {
                    let delta = v - floor;
                    for &(vi, _) in kernel {
                        e[vi] -= delta;
                    }
                    residual += delta;
                } else {
                    floor = floor.min(v);
                }
            }
        }
        // 3) tile ranges LAST — they are hard constraints and always get the
        // final word each iteration, so the finished field satisfies every
        // tile's elevation range exactly (caps are best-effort where the tile
        // map demands steeper chains than they allow).
        for vi in 0..nv {
            let c = e[vi].clamp(lo[vi], hi[vi]);
            residual += (c - e[vi]).abs();
            e[vi] = c;
        }
        iters = it + 1;
        if residual < SOLVER_EPS {
            break;
        }
    }
    for v in &mut e {
        *v = v.clamp(-1.0, 1.0);
    }
    (e, iters, residual)
}

// ---- helpers ----

/// The gap-free chain of fine faces a polyline passes over.
fn face_chain(grid: &Grid, points: &[SpherePos]) -> Vec<usize> {
    let mut c = Vec::new();
    for seg in points.windows(2) {
        let steps = (seg[0].distance(seg[1]) / 2.0).ceil().max(1.0) as usize;
        for k in 0..=steps {
            let p = seg[0].0.lerp(seg[1].0, k as f32 / steps as f32).normalize();
            if let Some(fi) = grid.planet.face_at(p) {
                if c.last() != Some(&fi) {
                    // Bridge non-adjacent jumps so the chain has no holes.
                    if let Some(&prev) = c.last() {
                        if !grid.adj[prev].contains(&(fi as u32)) {
                            c.extend(shortest_face_path(&grid.adj, prev, fi));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solved_field_invariants() {
        let state = run(1337, |_| {});
        let terrain = state.terrain.as_ref().unwrap();

        // Rivers descend monotonically on the SOLVED field.
        assert!(!terrain.river_paths.is_empty(), "no rivers generated");
        for (ri, path) in terrain.river_paths.iter().enumerate() {
            let mut prev = f32::MAX;
            for p in path {
                let e = terrain.elevation_at(*p);
                if prev < -0.05 {
                    break; // reached the sea — below the surface it's ocean
                }
                assert!(e <= prev + 0.02, "river {ri} flows uphill: {e} after {prev}");
                prev = prev.min(e);
            }
        }

        // Settlements stay dry.
        for a in &terrain.settlement_anchors {
            assert!(terrain.elevation_at(*a) > 0.0, "settlement anchor under water");
        }

        // Tile ranges hold exactly on the solved field (canyon verts excepted —
        // rivers may dig through anything).
        let mut canyon = vec![false; terrain.vert_count()];
        for path in &terrain.river_paths {
            for s in path {
                for (vi, _) in terrain.kernel(*s) {
                    canyon[vi] = true;
                }
            }
        }
        let e = terrain.vert_elevations();
        for vi in 0..terrain.vert_count() {
            if canyon[vi] {
                continue;
            }
            let Some(fi) = state.grid.planet.face_at(terrain.vert_dir(vi)) else { continue };
            let (rlo, rhi) = elev_range(state.tiles[fi]);
            assert!(
                e[vi] >= rlo - 1e-4 && e[vi] <= rhi + 1e-4,
                "vert {vi} ({:?}) out of range: {} not in [{rlo}, {rhi}]",
                state.tiles[fi], e[vi]
            );
        }

        // No wall at the waterline: edges crossing sea level stay gentle, and
        // no edge anywhere jumps more than 0.5 (~250m over ~70m).
        let mut worst_cross = 0.0f32;
        let mut worst_any = 0.0f32;
        for a in 0..terrain.vert_count() {
            for &b in terrain.adj_of(a) {
                if b <= a { continue; }
                let d = (e[a] - e[b]).abs();
                worst_any = worst_any.max(d);
                // Cliff and mountain coasts are deliberately steep sea walls.
                let steep_coast = |vi: usize| {
                    canyon[vi]
                        || state.grid.planet.face_at(terrain.vert_dir(vi))
                            .is_some_and(|fi| matches!(
                                state.tiles[fi],
                                Terrain::Cliff | Terrain::Mountain | Terrain::Snow
                            ))
                };
                if (e[a] >= 0.0) != (e[b] >= 0.0) && !steep_coast(a) && !steep_coast(b) {
                    worst_cross = worst_cross.max(d);
                }
            }
        }
        assert!(worst_cross <= 0.30, "waterline wall: {worst_cross}");
        assert!(worst_any <= 0.60, "extreme edge: {worst_any}");

        // No enclosed water body smaller than the minimum (no 1-tile lakes).
        let mut visited = vec![false; state.grid.n];
        for start in 0..state.grid.n {
            if !state.tiles[start].is_water() || state.tiles[start] == Terrain::River || visited[start] {
                continue;
            }
            let mut body = vec![start];
            let mut q = VecDeque::from([start]);
            visited[start] = true;
            while let Some(cur) = q.pop_front() {
                for &nb in &state.grid.adj[cur] {
                    let nb = nb as usize;
                    if state.tiles[nb].is_water() && state.tiles[nb] != Terrain::River && !visited[nb] {
                        visited[nb] = true;
                        body.push(nb);
                        q.push_back(nb);
                    }
                }
            }
            let is_ocean = body.iter().any(|&fi| {
                terrain.zones().kind_at_fine(fi) == crate::zones::ZoneKind::Ocean
            });
            assert!(
                is_ocean || body.len() >= MIN_WATER_BODY_FACES,
                "enclosed water body of only {} faces survived",
                body.len()
            );
        }

        // Counting rule: every plain land tile has ≥2 same-kind edge neighbors.
        let exempt = |t: Terrain| {
            t.is_water()
                || matches!(t, Terrain::Beach | Terrain::Cliff | Terrain::LakeShore | Terrain::RiverBank)
        };
        let blend_faces: std::collections::BTreeSet<u32> =
            state.blends.iter().map(|&(fi, _, _)| fi).collect();
        for fi in 0..state.grid.n {
            if exempt(state.tiles[fi]) || blend_faces.contains(&(fi as u32)) {
                continue;
            }
            let land_nbs = state.grid.adj[fi].iter()
                .filter(|&&nb| state.tiles[nb as usize].is_land())
                .count();
            if land_nbs < 2 {
                continue; // nothing to link to — mostly surrounded by water
            }
            let same = state.grid.adj[fi].iter()
                .filter(|&&nb| state.tiles[nb as usize] == state.tiles[fi])
                .count();
            assert!(same >= 2, "orphan tile {fi} ({:?}) with {same} same-kind edges", state.tiles[fi]);
        }

        // Blend marks link two differing plain kinds actually adjacent there.
        assert!(!state.blends.is_empty(), "no blends marked");
        for &(fi, a, b) in &state.blends {
            assert_ne!(a, b);
            assert_eq!(state.tiles[fi as usize] as u8, a, "blend face kind mismatch");
        }
    }

    #[test]
    fn deterministic_pipeline() {
        let a = run(42, |_| {});
        let b = run(42, |_| {});
        assert_eq!(a.terrain.unwrap().vert_elevations(), b.terrain.unwrap().vert_elevations());
        assert_eq!(a.tiles, b.tiles);
        assert_eq!(a.regions.len(), b.regions.len());
    }
}
