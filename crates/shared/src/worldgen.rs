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

/// The fine icosphere the pipeline works on (sub=6, ~82k faces, ~41k verts).
///
/// TILE IDENTITY LIVES ON VERTICES ("cells"), not faces. The dual of a
/// triangle grid is a hex grid: around any junction exactly 3 cells meet and
/// every pair of them shares a full edge, so two same-type cells can never
/// touch at a single point — the zigzag/pinch problem is impossible by
/// construction instead of being repaired after the fact. Faces derive their
/// render type from their 3 corner cells (all-same → solid, mixed → blend).
pub struct Grid {
    pub seed: u32,
    pub unit_tris: Vec<[Vec3; 3]>,
    pub planet: PlanetMesh,
    pub adj: Vec<[u32; 3]>,
    pub n: usize,
    /// Canonical unit direction per vertex (cell center).
    pub verts: Vec<Vec3>,
    /// The 3 cell ids at each face's corners.
    pub face_verts: Vec<[u32; 3]>,
    /// Hexagonal cell adjacency: 5–6 edge-linked neighbor cells.
    pub vert_adj: Vec<Vec<u32>>,
    /// The face fan around each cell (5–6 faces).
    pub vert_faces: Vec<Vec<u32>>,
    pub nv: usize,
}

impl Grid {
    pub fn new(seed: u32) -> Self {
        let unit_tris = unit_icosphere_tris(FINE_SUB);
        let planet = PlanetMesh::new(unit_tris.clone());
        let adj = build_face_adjacency(&planet.tris()[..], unit_tris.len());
        let n = unit_tris.len();
        debug_assert!(adj.iter().all(|a| !a.contains(&u32::MAX)), "face adjacency incomplete");

        // Canonical vertex ids (exact-bit key: subdivision emits identical floats).
        let mut vmap: BTreeMap<[u64; 3], u32> = BTreeMap::new();
        let mut verts: Vec<Vec3> = Vec::new();
        let mut face_verts: Vec<[u32; 3]> = Vec::with_capacity(n);
        for tri in &unit_tris {
            let mut idx = [0u32; 3];
            for (k, v) in tri.iter().enumerate() {
                let key = [v.x.to_bits() as u64, v.y.to_bits() as u64, v.z.to_bits() as u64];
                idx[k] = *vmap.entry(key).or_insert_with(|| {
                    verts.push(v.normalize());
                    (verts.len() - 1) as u32
                });
            }
            face_verts.push(idx);
        }
        let nv = verts.len();
        let mut vert_adj: Vec<Vec<u32>> = vec![Vec::new(); nv];
        let mut vert_faces: Vec<Vec<u32>> = vec![Vec::new(); nv];
        for (fi, idx) in face_verts.iter().enumerate() {
            for k in 0..3 {
                let (a, b) = (idx[k], idx[(k + 1) % 3]);
                if !vert_adj[a as usize].contains(&b) {
                    vert_adj[a as usize].push(b);
                    vert_adj[b as usize].push(a);
                }
                vert_faces[idx[k] as usize].push(fi as u32);
            }
        }
        for l in vert_adj.iter_mut().chain(vert_faces.iter_mut()) {
            l.sort_unstable();
        }
        debug_assert!(vert_adj.iter().all(|a| (5..=6).contains(&a.len())), "hex adjacency broken");

        Self { seed, unit_tris, planet, adj, n, verts, face_verts, vert_adj, vert_faces, nv }
    }

    pub fn centroid(&self, fi: usize) -> SpherePos {
        let [a, b, c] = self.unit_tris[fi];
        SpherePos::new(((a + b + c) / 3.0).normalize())
    }

    pub fn vert_pos(&self, vi: usize) -> SpherePos {
        SpherePos::new(self.verts[vi])
    }
}

/// Face render type from its 3 corner cells: two agreeing corners win; a
/// junction face (3 distinct labels) goes to the transition kind if one is
/// present, else water, else the lowest discriminant — deterministic and
/// conservative at waterlines.
pub fn derive_tiles(grid: &Grid, cells: &[Terrain]) -> Vec<Terrain> {
    let pri = |t: Terrain| match t {
        Terrain::Beach | Terrain::Cliff | Terrain::LakeShore | Terrain::RiverBank => 0u8,
        t if t.is_water() => 1,
        _ => 2,
    };
    grid.face_verts.iter().map(|idx| {
        let [a, b, c] = [cells[idx[0] as usize], cells[idx[1] as usize], cells[idx[2] as usize]];
        if a == b || a == c {
            a
        } else if b == c {
            b
        } else {
            [a, b, c].into_iter().min_by_key(|t| (pri(*t), *t as u8)).unwrap()
        }
    }).collect()
}

// ---- bitset ----

#[derive(Clone)]
pub struct BitSet(pub Vec<u64>);

impl BitSet {
    pub fn new(n: usize) -> Self { Self(vec![0; n.div_ceil(64)]) }
    pub fn insert(&mut self, i: usize) { self.0[i >> 6] |= 1u64 << (i & 63); }
    pub fn contains(&self, i: usize) -> bool {
        self.0.get(i >> 6).is_some_and(|w| w & (1u64 << (i & 63)) != 0)
    }
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
    /// Per-vertex tile labels — the single source of truth for terrain identity.
    pub cells: Vec<Terrain>,
    /// Per-face render/physics type, DERIVED from `cells` on every cell change.
    pub tiles: Vec<Terrain>,
    pub painted: Painted,
    pub bridges: Vec<Vec<SpherePos>>,
    /// Road polylines that survived water checks — the single source of truth
    /// for serialization (NOT terrain.road_paths, which is the L2 plan).
    pub roads: Vec<Vec<SpherePos>>,
    /// Inland biome-boundary faces and the kind pair they link.
    pub blends: Vec<(u32, u8, u8)>,
    pub regions: Vec<RegionData>,
    pub face_region: Vec<u32>,
    pub mesh_tris: Vec<[[f32; 3]; 3]>,
    pub mesh_colors: Vec<[[f32; 4]; 3]>,
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
            cells: Vec::new(),
            tiles: Vec::new(),
            painted: Painted {
                roads: BitSet::new(n),
                towns: BitSet::new(n),
                bridges: BitSet::new(n),
                bridge_entries: BitSet::new(n),
            },
            bridges: Vec::new(),
            roads: Vec::new(),
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
    /// L0/L1: grid, noise, coarse zones — the deterministic environment.
    InitTerrain,
    /// The proposed elevation field (classification hint, pre-solver).
    ProposeElevation,
    /// Moisture/temperature tables from noise + the current elevation field.
    /// Re-run after every field change (temperature lapses with altitude).
    ComputeClimate,
    /// L2: river waypoint paths (coarse routing).
    PlanRivers,
    /// L2: settlement anchors (flattest dry spot per settlement zone).
    PlaceSettlements,
    /// L2: road polylines between same-continent settlements.
    PlanRoads,
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
    TerrainInitialized(Box<TerrainGen>),
    ElevationProposed(Vec<f32>),
    ClimateComputed(Vec<f32>, Vec<f32>),
    RiversPlanned(Vec<Vec<SpherePos>>),
    SettlementsPlaced(Vec<SpherePos>),
    RoadsPlanned(Vec<Vec<SpherePos>>),
    TilesClassified(Vec<Terrain>),
    RiversPainted(Vec<Terrain>),
    WaterNormalized(Vec<Terrain>),
    FeaturesPainted(Painted, Vec<Vec<SpherePos>>),
    TransitionsResolved(Vec<Terrain>),
    BlendsMarked(Vec<(u32, u8, u8)>),
    ElevationSolved { field: Vec<f32>, iters: usize, residual: f32 },
    RegionsBuilt(Vec<RegionData>, Vec<u32>),
    BridgesSelected(Vec<Vec<SpherePos>>, Painted),
    MeshBuilt(Vec<[[f32; 3]; 3]>, Vec<[[f32; 4]; 3]>),
    TagsBuilt(Vec<u32>, Vec<u8>),
}

impl Event {
    /// Short line for the audit log.
    pub fn label(&self) -> String {
        match self {
            Event::TerrainInitialized(t) => format!(
                "terrain initialized: {} zones on {} coarse faces",
                t.zones().zones.len(), t.zones().face_count()
            ),
            Event::ElevationProposed(e) => format!("elevation proposed: {} verts", e.len()),
            Event::ClimateComputed(m, _) => format!("climate computed: {} verts", m.len()),
            Event::RiversPlanned(r) => format!("rivers planned: {}", r.len()),
            Event::SettlementsPlaced(a) => format!("settlements placed: {}", a.len()),
            Event::RoadsPlanned(r) => format!("roads planned: {}", r.len()),
            Event::TilesClassified(c) => format!("cells classified: {}", c.len()),
            Event::RiversPainted(_) => "rivers painted".into(),
            Event::WaterNormalized(_) => "water bodies normalized".into(),
            Event::FeaturesPainted(_, roads) => format!("roads + towns painted: {} roads kept", roads.len()),
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
        Command::InitTerrain => {
            vec![Event::TerrainInitialized(Box::new(TerrainGen::init(state.grid.seed)))]
        }
        Command::ProposeElevation => {
            vec![Event::ElevationProposed(state.terrain().propose_elevation())]
        }
        Command::ComputeClimate => {
            let (moist, temp) = state.terrain().compute_climate();
            vec![Event::ClimateComputed(moist, temp)]
        }
        Command::PlanRivers => {
            vec![Event::RiversPlanned(state.terrain().plan_river_paths())]
        }
        Command::PlaceSettlements => {
            vec![Event::SettlementsPlaced(state.terrain().plan_settlement_anchors())]
        }
        Command::PlanRoads => {
            vec![Event::RoadsPlanned(state.terrain().plan_road_paths())]
        }
        Command::ClassifyTiles => {
            let terrain = state.terrain();
            let grid = &state.grid;
            // Classify each CELL: sample at the vertex, majority over the fan's
            // zones (fans can straddle a coarse zone boundary).
            let cells = (0..grid.nv)
                .map(|vi| {
                    let pos = grid.vert_pos(vi);
                    let mut counts: BTreeMap<u8, usize> = BTreeMap::new();
                    for &fi in &grid.vert_faces[vi] {
                        let t = terrain.base_classify_fine(fi as usize, pos);
                        *counts.entry(t as u8).or_default() += 1;
                    }
                    let (&k, _) = counts.iter().max_by_key(|(_, c)| **c).unwrap();
                    Terrain::ALL[k as usize]
                })
                .collect();
            vec![Event::TilesClassified(cells)]
        }
        Command::PaintRivers => {
            let mut cells = state.cells.clone();
            paint_rivers(&state.grid, state.terrain(), &mut cells);
            vec![Event::RiversPainted(cells)]
        }
        Command::NormalizeWater => {
            let mut cells = state.cells.clone();
            normalize_water_bodies(&state.grid, state.terrain(), &mut cells);
            vec![Event::WaterNormalized(cells)]
        }
        Command::PaintFeatures => {
            let (painted, roads) = paint_features(&state.grid, state.terrain(), &state.tiles);
            vec![Event::FeaturesPainted(painted, roads)]
        }
        Command::ResolveTransitions => {
            let cells = resolve_transitions(&state.grid, state.terrain(), &state.cells);
            vec![Event::TransitionsResolved(cells)]
        }
        Command::MarkBlends => {
            vec![Event::BlendsMarked(mark_blends(
                &state.grid, &state.cells, &state.tiles, &state.painted,
            ))]
        }
        Command::SolveElevation => {
            let (field, iters, residual) = solve_elevation(
                &state.grid, state.terrain(), &state.tiles, &state.painted,
                &state.blends, &state.bridges,
            );
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
                &state.grid, state.terrain(), &state.cells, &state.painted, &state.blends,
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
        Event::TerrainInitialized(t) => state.terrain = Some(*t),
        Event::ElevationProposed(e) => {
            state.terrain.as_mut().expect("terrain exists").set_vert_elevations(e);
        }
        Event::ClimateComputed(moist, temp) => {
            state.terrain.as_mut().expect("terrain exists").set_climate(moist, temp);
        }
        Event::RiversPlanned(r) => {
            state.terrain.as_mut().expect("terrain exists").river_paths = r;
        }
        Event::SettlementsPlaced(a) => {
            state.terrain.as_mut().expect("terrain exists").settlement_anchors = a;
        }
        Event::RoadsPlanned(r) => {
            state.terrain.as_mut().expect("terrain exists").road_paths = r;
        }
        Event::TilesClassified(c)
        | Event::RiversPainted(c)
        | Event::WaterNormalized(c)
        | Event::TransitionsResolved(c) => {
            state.tiles = derive_tiles(&state.grid, &c);
            state.cells = c;
        }
        Event::FeaturesPainted(p, roads) => {
            state.painted = p;
            state.roads = roads;
        }
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
        Event::TerrainInitialized(_) => vec![Command::ProposeElevation],
        // Climate follows every field change; FIFO runs it before the next step.
        Event::ElevationProposed(_) => vec![Command::ComputeClimate, Command::PlanRivers],
        Event::ClimateComputed(..) => vec![],
        Event::RiversPlanned(_) => vec![Command::PlaceSettlements],
        Event::SettlementsPlaced(_) => vec![Command::PlanRoads],
        Event::RoadsPlanned(_) => vec![Command::ClassifyTiles],
        Event::TilesClassified(_) => vec![Command::PaintRivers],
        Event::RiversPainted(_) => vec![Command::NormalizeWater],
        Event::WaterNormalized(_) => vec![Command::PaintFeatures],
        Event::FeaturesPainted(..) => vec![Command::ResolveTransitions],
        // Regions and bridge selection read only tiles/geometry, so they run
        // BEFORE the solver — which then knows the bridge-entry pads to flatten.
        // Blends come after bridges so entry flanks can blend toward the pads.
        Event::TransitionsResolved(_) => vec![Command::BuildRegions],
        Event::RegionsBuilt(..) => vec![Command::SelectBridges],
        Event::BridgesSelected(..) => vec![Command::MarkBlends],
        Event::BlendsMarked(_) => vec![Command::SolveElevation],
        Event::ElevationSolved { .. } => vec![Command::ComputeClimate, Command::BuildMesh],
        Event::MeshBuilt(..) => vec![Command::BuildTags],
        Event::TagsBuilt(..) => vec![],
    }
}

/// Run the full pipeline for a seed. `log` receives one line per event.
pub fn run(seed: u32, mut log: impl FnMut(&str)) -> GenState {
    let mut state = GenState::new(seed);
    let mut queue = VecDeque::from([Command::InitTerrain]);
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

/// River polylines → River cells (land only; the mouth is already water).
fn paint_rivers(grid: &Grid, terrain: &TerrainGen, cells: &mut [Terrain]) {
    for path in &terrain.river_paths {
        for vi in vert_chain(grid, path) {
            if cells[vi].is_land() {
                cells[vi] = Terrain::River;
            }
        }
    }
}

/// The gap-free chain of cells a polyline passes over: nearest corner per
/// sample, gaps bridged along cell adjacency.
fn vert_chain(grid: &Grid, points: &[SpherePos]) -> Vec<usize> {
    let mut c: Vec<usize> = Vec::new();
    for seg in points.windows(2) {
        let steps = (seg[0].distance(seg[1]) / 2.0).ceil().max(1.0) as usize;
        for k in 0..=steps {
            let p = seg[0].0.lerp(seg[1].0, k as f32 / steps as f32).normalize();
            let Some(fi) = grid.planet.face_at(p) else { continue };
            let vi = grid.face_verts[fi].iter().copied()
                .max_by(|&a, &b| {
                    grid.verts[a as usize].dot(p)
                        .partial_cmp(&grid.verts[b as usize].dot(p)).unwrap()
                })
                .unwrap() as usize;
            if c.last() == Some(&vi) {
                continue;
            }
            if let Some(&prev) = c.last() {
                if !grid.vert_adj[prev].contains(&(vi as u32)) {
                    c.extend(shortest_vert_path(grid, prev, vi));
                }
            }
            if c.last() != Some(&vi) {
                c.push(vi);
            }
        }
    }
    c
}

fn shortest_vert_path(grid: &Grid, u: usize, v: usize) -> Vec<usize> {
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
        for &n in &grid.vert_adj[cur] {
            let n = n as usize;
            prev.entry(n).or_insert_with(|| {
                q.push_back((n, d + 1));
                cur
            });
        }
    }
    Vec::new()
}

/// Majority coarse zone over a cell's face fan (deterministic tie-break).
fn cell_zone(grid: &Grid, terrain: &TerrainGen, vi: usize) -> crate::zones::ZoneKind {
    let mut counts: BTreeMap<u8, usize> = BTreeMap::new();
    for &fi in &grid.vert_faces[vi] {
        *counts.entry(terrain.zones().kind_at_fine(fi as usize) as u8).or_default() += 1;
    }
    let (&k, _) = counts.iter().max_by_key(|(_, c)| **c).unwrap();
    use crate::zones::ZoneKind::*;
    [Ocean, Continent, Island, Lake, MountainRange, Settlement][k as usize]
}

/// Water-body identity comes from connectivity, not per-face depth: any face can
/// dip below sea level (blending, river carving), but a connected water body is
/// an Ocean only if it reaches ocean-zone faces — otherwise it's an enclosed
/// Lake, whatever its faces individually classified as. Rivers (painted channel
/// faces) are their own linear feature and never merge into either.
fn normalize_water_bodies(grid: &Grid, terrain: &TerrainGen, cells: &mut [Terrain]) {
    // Dam every lake rim first: a land-zone cell that classified as water and
    // touches lake-zone water is the start of a drain channel to the sea (they
    // sneak along coarse-face edges past the vertex clamps). Turn it into
    // LakeShore — a one-cell dam that encloses the lake by construction; the
    // elevation solver then lifts it above sea level (LakeShore range).
    let lake_zone: Vec<bool> = (0..grid.nv)
        .map(|vi| cell_zone(grid, terrain, vi) == crate::zones::ZoneKind::Lake)
        .collect();
    let dams: Vec<usize> = (0..grid.nv)
        .filter(|&vi| {
            cells[vi].is_water()
                && !lake_zone[vi]
                && grid.vert_adj[vi].iter().any(|&nb| {
                    let nb = nb as usize;
                    cells[nb].is_water() && lake_zone[nb]
                })
        })
        .collect();
    for vi in dams {
        cells[vi] = Terrain::LakeShore;
    }

    enforce_water_shape(grid, cells);

    let mut visited = vec![false; grid.nv];
    for start in 0..grid.nv {
        if !cells[start].is_water() || cells[start] == Terrain::River || visited[start] {
            continue;
        }
        let mut body = vec![start];
        let mut q = VecDeque::from([start]);
        visited[start] = true;
        while let Some(cur) = q.pop_front() {
            for &nb in &grid.vert_adj[cur] {
                let nb = nb as usize;
                if cells[nb].is_water() && cells[nb] != Terrain::River && !visited[nb] {
                    visited[nb] = true;
                    body.push(nb);
                    q.push_back(nb);
                }
            }
        }
        let is_ocean = body.iter()
            .any(|&vi| cell_zone(grid, terrain, vi) == crate::zones::ZoneKind::Ocean);
        if !is_ocean && body.len() < MIN_WATER_BODY_CELLS {
            // A puddle isn't a lake: fill it with the most common surrounding
            // land kind so no 1-cell water ever survives.
            let mut counts: BTreeMap<u8, usize> = BTreeMap::new();
            for &vi in &body {
                for &nb in &grid.vert_adj[vi] {
                    let t = cells[nb as usize];
                    if t.is_land() {
                        *counts.entry(t as u8).or_default() += 1;
                    }
                }
            }
            let fill = counts.iter().max_by_key(|(_, c)| **c)
                .map(|(&k, _)| Terrain::ALL[k as usize])
                .unwrap_or(Terrain::Plains);
            for &vi in &body {
                cells[vi] = fill;
            }
            continue;
        }
        for &vi in &body {
            cells[vi] = if !is_ocean {
                Terrain::Lake
            } else if cells[vi] == Terrain::Lake {
                // A lake cell swallowed by the ocean body is just shallow ocean.
                Terrain::Ocean
            } else {
                cells[vi]
            };
        }
    }

    // Lakes keep their distance from the sea: any lake cell within ~7 cell
    // steps (~250m ≈ the 10-tile rule) of ocean water becomes land, and a
    // lake trimmed under the minimum size drains entirely.
    let mut ocean_dist = vec![u8::MAX; grid.nv];
    let mut q: VecDeque<usize> = VecDeque::new();
    for vi in 0..grid.nv {
        if matches!(cells[vi], Terrain::Ocean | Terrain::DeepOcean) {
            ocean_dist[vi] = 0;
            q.push_back(vi);
        }
    }
    while let Some(cur) = q.pop_front() {
        if ocean_dist[cur] >= 7 {
            continue;
        }
        for &nb in &grid.vert_adj[cur] {
            let nb = nb as usize;
            if ocean_dist[nb] == u8::MAX {
                ocean_dist[nb] = ocean_dist[cur] + 1;
                q.push_back(nb);
            }
        }
    }
    let fill_kind = |cells: &[Terrain], vi: usize| -> Terrain {
        let mut counts: BTreeMap<u8, usize> = BTreeMap::new();
        for &nb in &grid.vert_adj[vi] {
            let t = cells[nb as usize];
            if t.is_land() {
                *counts.entry(t as u8).or_default() += 1;
            }
        }
        counts.iter().max_by_key(|(_, c)| **c)
            .map(|(&k, _)| Terrain::ALL[k as usize])
            .unwrap_or(Terrain::Plains)
    };
    for vi in 0..grid.nv {
        if cells[vi] == Terrain::Lake && ocean_dist[vi] <= 7 {
            cells[vi] = fill_kind(cells, vi);
        }
    }
    let mut visited = vec![false; grid.nv];
    for start in 0..grid.nv {
        if cells[start] != Terrain::Lake || visited[start] {
            continue;
        }
        let mut body = vec![start];
        let mut q = VecDeque::from([start]);
        visited[start] = true;
        while let Some(cur) = q.pop_front() {
            for &nb in &grid.vert_adj[cur] {
                let nb = nb as usize;
                if cells[nb] == Terrain::Lake && !visited[nb] {
                    visited[nb] = true;
                    body.push(nb);
                    q.push_back(nb);
                }
            }
        }
        if body.len() < MIN_WATER_BODY_CELLS {
            for &vi in &body {
                cells[vi] = fill_kind(cells, vi);
            }
        }
    }
}

/// Water narrower than a few cells reads as a visual glitch: water within 2
/// steps of land must reach "core" water (≥3 steps from land) within 2 steps,
/// or it is a sliver/neck → land. Rivers are linear by nature and exempt.
/// (Vertex pinches no longer exist: cells are hexagonal, two same-type cells
/// can only meet along an edge.) Iterated because fills can expose new slivers.
fn enforce_water_shape(grid: &Grid, cells: &mut [Terrain]) {
    let fill_kind = |cells: &[Terrain], vi: usize| -> Terrain {
        let mut counts: BTreeMap<u8, usize> = BTreeMap::new();
        for &nb in &grid.vert_adj[vi] {
            let t = cells[nb as usize];
            if t.is_land() {
                *counts.entry(t as u8).or_default() += 1;
            }
        }
        counts.iter().max_by_key(|(_, c)| **c)
            .map(|(&k, _)| Terrain::ALL[k as usize])
            .unwrap_or(Terrain::Plains)
    };
    let wet = |t: Terrain| t.is_water() && t != Terrain::River;

    for _ in 0..4 {
        let mut changed = false;
        let mut dist = vec![u8::MAX; grid.nv];
        let mut q: VecDeque<usize> = VecDeque::new();
        for vi in 0..grid.nv {
            if cells[vi].is_land() {
                dist[vi] = 0;
                q.push_back(vi);
            }
        }
        // Cell steps are ~35m (vs ~20m face steps): core water sits ≥3 cells
        // (~100m) from land, matching the old face-based width in meters.
        while let Some(cur) = q.pop_front() {
            if dist[cur] >= 2 {
                continue;
            }
            for &nb in &grid.vert_adj[cur] {
                let nb = nb as usize;
                if dist[nb] == u8::MAX {
                    dist[nb] = dist[cur] + 1;
                    q.push_back(nb);
                }
            }
        }
        // Core water reaches outward 2 steps.
        let mut core_reach = vec![false; grid.nv];
        let mut q: VecDeque<(usize, u8)> = VecDeque::new();
        for vi in 0..grid.nv {
            if wet(cells[vi]) && dist[vi] == u8::MAX {
                core_reach[vi] = true;
                q.push_back((vi, 0));
            }
        }
        while let Some((cur, d)) = q.pop_front() {
            if d >= 2 {
                continue;
            }
            for &nb in &grid.vert_adj[cur] {
                let nb = nb as usize;
                if wet(cells[nb]) && !core_reach[nb] {
                    core_reach[nb] = true;
                    q.push_back((nb, d + 1));
                }
            }
        }
        for vi in 0..grid.nv {
            if wet(cells[vi]) && !core_reach[vi] {
                cells[vi] = fill_kind(cells, vi);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
}

/// Enclosed water smaller than this many cells is filled to land — a couple
/// of tiles of "lake" beside the ocean makes no sense (one cell ≈ two fine
/// faces of area, so 15 cells ≈ the old 30-face minimum).
const MIN_WATER_BODY_CELLS: usize = 15;

/// Roads may not cross water: a planned path whose face chain touches a water
/// tile is dropped entirely (crossing there needs a bridge, not a road).
fn paint_features(
    grid: &Grid,
    terrain: &TerrainGen,
    tiles: &[Terrain],
) -> (Painted, Vec<Vec<SpherePos>>) {
    let mut roads = BitSet::new(grid.n);
    let mut kept: Vec<Vec<SpherePos>> = Vec::new();
    for path in &terrain.road_paths {
        let chain = face_chain(grid, path);
        if chain.iter().any(|&fi| tiles[fi].is_water()) {
            continue;
        }
        for fi in chain {
            roads.insert(fi);
        }
        kept.push(path.clone());
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
    (Painted { roads, towns, bridges, bridge_entries: BitSet::new(grid.n) }, kept)
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

fn resolve_transitions(grid: &Grid, terrain: &TerrainGen, base: &[Terrain]) -> Vec<Terrain> {
    // Shorelines are deterministic bands, not WFC cells: every land cell
    // touching water gets its shore tile, so the waterline is never zigzagged
    // by chance. The band widens onto the second ring where the coast is flat,
    // and a land cell wedged between two shore cells joins the band.
    let mut out = base.to_vec();
    // DeepOcean may never surface: not even a corner of a deep face may rise
    // above the waterline. The interpolated surface reaches ~150m past any
    // land vertex and cells are ~35m apart — deep water keeps a shallow
    // (Ocean) margin so no land vertex can leak into a deep corner.
    {
        let mut land_dist = vec![u8::MAX; grid.nv];
        let mut q: VecDeque<usize> = VecDeque::new();
        for vi in 0..grid.nv {
            if out[vi].is_land() {
                land_dist[vi] = 0;
                q.push_back(vi);
            }
        }
        while let Some(cur) = q.pop_front() {
            if land_dist[cur] >= 6 {
                continue;
            }
            for &nb in &grid.vert_adj[cur] {
                let nb = nb as usize;
                if land_dist[nb] == u8::MAX {
                    land_dist[nb] = land_dist[cur] + 1;
                    q.push_back(nb);
                }
            }
        }
        for vi in 0..grid.nv {
            if out[vi] == Terrain::DeepOcean && land_dist[vi] <= 6 {
                out[vi] = Terrain::Ocean;
            }
        }
    }
    let (water_dist, water_kind) = water_distance(grid, base, 2);
    let shore = |vi: usize, kind: Terrain| -> Terrain {
        match kind {
            Terrain::Lake => Terrain::LakeShore,
            Terrain::River => Terrain::RiverBank,
            _ => {
                let steep = matches!(base[vi], Terrain::Mountain | Terrain::Snow)
                    || terrain.elevation_at(grid.vert_pos(vi)) > 0.15;
                if steep { Terrain::Cliff } else { Terrain::Beach }
            }
        }
    };
    for vi in 0..grid.nv {
        if base[vi].is_water() {
            continue;
        }
        let touching_water = grid.vert_adj[vi].iter()
            .map(|&nb| base[nb as usize])
            .find(|t| t.is_water());
        if let Some(kind) = touching_water {
            out[vi] = shore(vi, kind);
        } else if water_dist[vi] <= 2 && terrain.elevation_at(grid.vert_pos(vi)) < 0.08 {
            // Low, flat coast: the band is more than one cell wide.
            out[vi] = shore(vi, water_kind[vi].unwrap_or(Terrain::Ocean));
        }
    }
    // Fill notches: a land cell with ≥2 neighbors in the shore band belongs
    // to the band too.
    let banded: Vec<bool> = (0..grid.nv)
        .map(|vi| matches!(out[vi], Terrain::Beach | Terrain::Cliff | Terrain::LakeShore | Terrain::RiverBank))
        .collect();
    for vi in 0..grid.nv {
        if base[vi].is_water() || banded[vi] {
            continue;
        }
        if grid.vert_adj[vi].iter().filter(|&&nb| banded[nb as usize]).count() >= 2 {
            out[vi] = shore(vi, water_kind[vi].unwrap_or(Terrain::Ocean));
        }
    }

    // WFC over inland biome edges: land cells bordering a different
    // classification. Water and the shore band enter as fixed neighbors.
    let in_band: Vec<bool> = (0..grid.nv).map(|vi| out[vi] != base[vi]).collect();
    let base = &out;
    let mut cell_of = vec![usize::MAX; grid.nv];
    let mut wfc_cells: Vec<usize> = Vec::new();
    for vi in 0..grid.nv {
        if base[vi].is_water() || in_band[vi] {
            continue;
        }
        if grid.vert_adj[vi].iter().any(|&nb| base[nb as usize] != base[vi]) {
            cell_of[vi] = wfc_cells.len();
            wfc_cells.push(vi);
        }
    }

    let transition_tiles = [Terrain::Beach, Terrain::Cliff, Terrain::LakeShore, Terrain::RiverBank];
    let domains: Vec<Vec<(Terrain, f32)>> = wfc_cells.iter().map(|&vi| {
        let mut d = vec![(base[vi], 1.0)];
        for t in transition_tiles {
            if t != base[vi] {
                d.push((t, 0.3));
            }
        }
        d
    }).collect();
    let neighbors: Vec<Vec<wfc::Neighbor>> = wfc_cells.iter().map(|&vi| {
        grid.vert_adj[vi].iter().map(|&nb| {
            let nb = nb as usize;
            match cell_of[nb] {
                usize::MAX => wfc::Neighbor::Fixed(base[nb]),
                ci => wfc::Neighbor::Cell(ci),
            }
        }).collect()
    }).collect();
    let fallback: Vec<Terrain> = wfc_cells.iter().map(|&vi| base[vi]).collect();

    let solved = wfc::solve(&wfc::Compat::default(), &domains, &neighbors, &fallback, grid.seed as u64);

    let mut resolved = base.clone();
    for (ci, &vi) in wfc_cells.iter().enumerate() {
        resolved[vi] = solved[ci];
    }
    segment_coastline(grid, terrain, &mut resolved);
    absorb_small_forests(grid, &mut resolved);
    resolved
}

/// Inland biome boundaries get a blend mark: the face keeps its derived
/// kind, but carries the pair it links so rendering/solving can transition
/// between the two. With cell-based tiles the boundary faces are simply the
/// faces whose corner cells disagree — edge-connected strips by construction.
/// Faces flanking a built feature blend toward it instead (feature codes).
fn mark_blends(grid: &Grid, cells: &[Terrain], tiles: &[Terrain], painted: &Painted) -> Vec<(u32, u8, u8)> {
    let plain = |t: Terrain| t.is_land();
    let overlay = |fi: usize| {
        painted.roads.contains(fi) || painted.towns.contains(fi) || painted.bridge_entries.contains(fi)
    };
    let mut out = Vec::new();
    for fi in 0..grid.n {
        if !plain(tiles[fi]) || overlay(fi) {
            continue;
        }
        // Feature flanks blend toward the feature; most specific wins
        // (entry pad < town blob < road network by footprint).
        let feature = if grid.adj[fi].iter().any(|&nb| painted.bridge_entries.contains(nb as usize)) {
            Some(crate::level::BLEND_BRIDGE_ENTRY)
        } else if grid.adj[fi].iter().any(|&nb| painted.towns.contains(nb as usize)) {
            Some(crate::level::BLEND_TOWN)
        } else if grid.adj[fi].iter().any(|&nb| painted.roads.contains(nb as usize)) {
            Some(crate::level::BLEND_ROAD)
        } else {
            None
        };
        if let Some(code) = feature {
            out.push((fi as u32, tiles[fi] as u8, code));
            continue;
        }
        // Corner cells that disagree with the face's derived kind: the face is
        // the linking tile between its kind and the most present other LAND
        // kind (water transitions are the shore band's job).
        let mut counts: BTreeMap<u8, usize> = BTreeMap::new();
        for &vi in &grid.face_verts[fi] {
            let t = cells[vi as usize];
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

/// A forest smaller than this many cells is just some trees in a field.
const MIN_FOREST_CELLS: usize = 10;

fn absorb_small_forests(grid: &Grid, out: &mut [Terrain]) {
    let mut visited = vec![false; grid.nv];
    for start in 0..grid.nv {
        if out[start] != Terrain::Forest || visited[start] {
            continue;
        }
        let mut cluster = vec![start];
        let mut q = VecDeque::from([start]);
        visited[start] = true;
        while let Some(cur) = q.pop_front() {
            for &nb in &grid.vert_adj[cur] {
                let nb = nb as usize;
                if out[nb] == Terrain::Forest && !visited[nb] {
                    visited[nb] = true;
                    cluster.push(nb);
                    q.push_back(nb);
                }
            }
        }
        if cluster.len() < MIN_FOREST_CELLS {
            for vi in cluster {
                out[vi] = Terrain::Plains;
            }
        }
    }
}

/// Max coastal segment sizes in cells (the band is 1-2 cells wide, cells are
/// ~35m apart, so 30 cells is on the order of a kilometre of beach).
const MAX_BEACH_CELLS: usize = 30;
const MAX_CLIFF_CELLS: usize = 12;
const MIN_BEACH_CELLS: usize = 10;

/// Break each continuous ocean shore band into alternating Beach and Cliff
/// segments with bounded lengths — beaches can't wrap a whole continent as one
/// region, and each cliff range between them becomes a named region too.
/// Steep cells still force Cliff regardless of alternation.
fn segment_coastline(grid: &Grid, terrain: &TerrainGen, out: &mut [Terrain]) {
    let in_band: Vec<bool> = (0..grid.nv)
        .map(|vi| matches!(out[vi], Terrain::Beach | Terrain::Cliff))
        .collect();
    let mut visited = vec![false; grid.nv];
    for start in 0..grid.nv {
        if !in_band[start] || visited[start] {
            continue;
        }
        // BFS order approximates walking along the thin band.
        let mut order = vec![start];
        let mut q = VecDeque::from([start]);
        visited[start] = true;
        while let Some(cur) = q.pop_front() {
            for &nb in &grid.vert_adj[cur] {
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
        for vi in order {
            let steep = terrain.elevation_at(grid.vert_pos(vi)) > 0.15;
            if steep && kind == Terrain::Beach {
                kind = Terrain::Cliff;
                count = 0;
            }
            let max = if kind == Terrain::Beach { MAX_BEACH_CELLS } else { MAX_CLIFF_CELLS };
            if count >= max {
                kind = if kind == Terrain::Beach { Terrain::Cliff } else { Terrain::Beach };
                count = 0;
            }
            out[vi] = kind;
            count += 1;
        }
    }
    // Remove single-cell islands: a band cell with no same-kind neighbor in
    // the band joins its neighbors' kind (a lone beach cell between two cliffs
    // becomes cliff, and vice versa).
    for _ in 0..8 {
        let mut changed = false;
        for vi in 0..grid.nv {
            if !matches!(out[vi], Terrain::Beach | Terrain::Cliff) {
                continue;
            }
            let mut same = 0;
            let mut other = 0;
            for &nb in &grid.vert_adj[vi] {
                match out[nb as usize] {
                    t if t == out[vi] => same += 1,
                    Terrain::Beach | Terrain::Cliff => other += 1,
                    _ => {}
                }
            }
            if same == 0 && other >= 2 {
                out[vi] = if out[vi] == Terrain::Beach { Terrain::Cliff } else { Terrain::Beach };
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    // A cove under MIN_BEACH_CELLS isn't a beach — fold it into the cliffs.
    let mut visited = vec![false; grid.nv];
    for start in 0..grid.nv {
        if out[start] != Terrain::Beach || visited[start] {
            continue;
        }
        let mut cluster = vec![start];
        let mut q = VecDeque::from([start]);
        visited[start] = true;
        while let Some(cur) = q.pop_front() {
            for &nb in &grid.vert_adj[cur] {
                let nb = nb as usize;
                if out[nb] == Terrain::Beach && !visited[nb] {
                    visited[nb] = true;
                    cluster.push(nb);
                    q.push_back(nb);
                }
            }
        }
        if cluster.len() < MIN_BEACH_CELLS {
            for vi in cluster {
                out[vi] = Terrain::Cliff;
            }
        }
    }
}

/// BFS distance (in cell steps, capped at `max_dist`) from each land cell to the
/// nearest water cell, plus which water terrain is nearest (for shore tile choice).
fn water_distance(grid: &Grid, base: &[Terrain], max_dist: u8) -> (Vec<u8>, Vec<Option<Terrain>>) {
    let mut dist = vec![u8::MAX; grid.nv];
    let mut kind: Vec<Option<Terrain>> = vec![None; grid.nv];
    let mut q = VecDeque::new();
    for vi in 0..grid.nv {
        if base[vi].is_water() {
            dist[vi] = 0;
            kind[vi] = Some(base[vi]);
            q.push_back(vi);
        }
    }
    while let Some(cur) = q.pop_front() {
        if dist[cur] >= max_dist {
            continue;
        }
        for &nb in &grid.vert_adj[cur] {
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
    // Terrain-derived class ignoring the road/town overlay: a road slicing
    // through a desert must not split it into two regions, so terrain clusters
    // may flow THROUGH overlay faces whose underlying terrain matches (without
    // claiming them — those faces belong to their Road/Town region).
    let terrain_class: Vec<Option<RegionKind>> = (0..grid.n)
        .map(|fi| {
            region_class(
                face_types,
                &Painted {
                    roads: BitSet::new(0),
                    towns: BitSet::new(0),
                    bridges: BitSet::new(0),
                    bridge_entries: BitSet::new(0),
                },
                fi,
            )
        })
        .collect();

    let mut face_region = vec![NO_REGION; grid.n];
    let mut regions: Vec<RegionData> = Vec::new();
    let mut kind_counts: BTreeMap<u8, usize> = BTreeMap::new();

    for start in 0..grid.n {
        let Some(kind) = class[start] else { continue };
        if face_region[start] != NO_REGION {
            continue;
        }
        let overlay_kind = matches!(kind, RegionKind::Road | RegionKind::Town);
        // Collect the edge-connected cluster. Stored refs are region id + 1
        // (0 = no region, see level::region_index).
        let re = regions.len() as u32 + 1;
        let mut faces = vec![start];
        let mut q = VecDeque::from([start]);
        let mut visited_connector = BitSet::new(grid.n);
        face_region[start] = re;
        while let Some(cur) = q.pop_front() {
            for &nb in &grid.adj[cur] {
                let nb = nb as usize;
                if class[nb] == Some(kind) && face_region[nb] == NO_REGION {
                    face_region[nb] = re;
                    faces.push(nb);
                    q.push_back(nb);
                } else if !overlay_kind
                    && class[nb] != Some(kind)
                    && terrain_class[nb] == Some(kind)
                    && !visited_connector.contains(nb)
                {
                    // Overlay face with matching ground: pass through it.
                    visited_connector.insert(nb);
                    q.push_back(nb);
                }
            }
        }
        // Tiny scraps stay unnamed (towns and roads always name).
        let min_faces = match kind {
            RegionKind::Town | RegionKind::Road | RegionKind::River => 1,
            // Cell minimums expressed in faces (one cell ≈ two faces of area).
            RegionKind::Forest => MIN_FOREST_CELLS * 2,
            RegionKind::Beach => MIN_BEACH_CELLS * 2,
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

/// Pure projection of the solved field, with PER-CORNER colors: each corner
/// takes its cell's color, so a biome boundary renders as a smooth gradient
/// across its boundary faces — a hard color seam or single-vertex color pinch
/// cannot exist. Built features override per face (they are solid structures),
/// and feature flanks fade each corner halfway toward the feature color.
fn build_mesh(
    grid: &Grid,
    terrain: &TerrainGen,
    cells: &[Terrain],
    painted: &Painted,
    blends: &[(u32, u8, u8)],
) -> (Vec<[[f32; 3]; 3]>, Vec<[[f32; 4]; 3]>) {
    let mut blend_of: BTreeMap<u32, (u8, u8)> = BTreeMap::new();
    for &(fi, a, b) in blends {
        blend_of.insert(fi, (a, b));
    }
    let vert_r: Vec<f32> = grid.verts.iter()
        .map(|dir| terrain.render_radius(SpherePos::new(*dir)))
        .collect();

    let road_color = bevy::prelude::Color::srgb(0.5, 0.42, 0.3).to_linear().to_f32_array();
    let town_color = crate::theme::WARNING.to_linear().to_f32_array();
    let entry_color = bevy::prelude::Color::srgb(0.42, 0.33, 0.24).to_linear().to_f32_array();
    let mix = |a: [f32; 4], b: [f32; 4]| {
        [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0, (a[2] + b[2]) / 2.0, (a[3] + b[3]) / 2.0]
    };
    let mut tris = Vec::with_capacity(grid.n);
    let mut cols = Vec::with_capacity(grid.n);
    for (fi, idx) in grid.face_verts.iter().enumerate() {
        let corner = |k: usize| cells[idx[k] as usize].color().to_linear().to_f32_array();
        let color: [[f32; 4]; 3] = if painted.towns.contains(fi) {
            [town_color; 3]
        } else if painted.bridge_entries.contains(fi) {
            [entry_color; 3]
        } else if painted.roads.contains(fi) {
            [road_color; 3]
        } else if let Some(&(_, kb)) = blend_of.get(&(fi as u32)).filter(|&&(_, b)| b >= crate::level::BLEND_FEATURE_MIN) {
            // Feature flank: fade each corner toward the feature color.
            let cb = match kb {
                crate::level::BLEND_ROAD => road_color,
                crate::level::BLEND_TOWN => town_color,
                _ => entry_color,
            };
            [mix(corner(0), cb), mix(corner(1), cb), mix(corner(2), cb)]
        } else {
            // Cell colors per corner: solid inside a region, gradient at
            // boundaries (terrain-pair blends need no special casing).
            [corner(0), corner(1), corner(2)]
        };
        tris.push([
            (grid.verts[idx[0] as usize] * vert_r[idx[0] as usize]).to_array(),
            (grid.verts[idx[1] as usize] * vert_r[idx[1] as usize]).to_array(),
            (grid.verts[idx[2] as usize] * vert_r[idx[2] as usize]).to_array(),
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
        // Cliff tiles are RAMPS, not plateaus: the toe verts sit at shore
        // level and the crest verts track the hinterland (see the cliff
        // tracking step in the solver), so the whole drop happens across the
        // cliff face. The range here is just the envelope.
        Cliff => (-0.02, 0.60),
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
        // The whole cliff drop can happen across one vertex edge (toe → crest).
        0.60
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
    blends: &[(u32, u8, u8)],
    bridges: &[Vec<SpherePos>],
) -> (Vec<f32>, usize, f32) {
    let nv = terrain.vert_count();
    let mut blend_of: BTreeMap<u32, (u8, u8)> = BTreeMap::new();
    for &(fi, a, b) in blends {
        blend_of.insert(fi, (a, b));
    }

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
    let owner_face: Vec<Option<usize>> = (0..nv)
        .map(|vi| grid.planet.face_at(terrain.vert_dir(vi)))
        .collect();
    let mut lo = vec![-1.0f32; nv];
    let mut hi = vec![1.0f32; nv];
    let mut is_road_vert = vec![false; nv];
    for vi in 0..nv {
        // Blend faces are the altitude ramp between two kinds: their vertices
        // get the HULL of both ranges so the solver can transition through.
        let (rlo, rhi) = match owner_face[vi].and_then(|fi| blend_of.get(&(fi as u32))) {
            // Feature blends are visual only — features follow the ground.
            Some(&(_, b)) if b >= crate::level::BLEND_FEATURE_MIN => elev_range(owner[vi]),
            Some(&(a, b)) => {
                let (alo, ahi) = elev_range(Terrain::ALL[a as usize]);
                let (blo, bhi) = elev_range(Terrain::ALL[b as usize]);
                (alo.min(blo), ahi.max(bhi))
            }
            None => elev_range(owner[vi]),
        };
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

    // Cliff ramp vertices: a cliff-owned vert beside water/beach is the TOE
    // (pinned to shore level); every other cliff vert is the CREST (tracks the
    // highest adjacent hinterland vert exactly, so nothing sticks out above
    // the terrain behind and lower neighbors stay untouched).
    let shore_kind = |t: Terrain| t.is_water() || t == Terrain::Beach;
    let mut cliff_toe: Vec<bool> = vec![false; nv];
    let mut cliff_crest: Vec<bool> = vec![false; nv];
    for vi in 0..nv {
        if owner[vi] != Terrain::Cliff {
            continue;
        }
        let shoreside = terrain.adj_of(vi).iter().any(|&nb| shore_kind(owner[nb]));
        if shoreside {
            cliff_toe[vi] = true;
        } else {
            cliff_crest[vi] = true;
        }
    }

    // Bridge-entry pads: the vertices under each deck end are driven to one
    // common height (slope 0) so the deck meets the ground seamlessly.
    let pad_verts_at = |end: &SpherePos| -> Vec<usize> {
        // The pad covers the end plus a ~30m ring around it, so height probes
        // anywhere near the footing read only pad vertices (slope ≈ 0).
        let (east, north) = end.tangent_basis();
        let step = 30.0 / crate::sphere::PLANET_RADIUS;
        let mut verts: Vec<usize> = Vec::new();
        for dir in [
            end.0,
            (end.0 + east * step).normalize(),
            (end.0 - east * step).normalize(),
            (end.0 + north * step).normalize(),
            (end.0 - north * step).normalize(),
        ] {
            for (vi, _) in terrain.kernel(SpherePos::new(dir)) {
                if !verts.contains(&vi) {
                    verts.push(vi);
                }
            }
        }
        verts
    };
    let pads: Vec<Vec<usize>> = bridges.iter()
        .flat_map(|span| [span.first(), span.last()])
        .flatten()
        .map(pad_verts_at)
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
        // 3) cliff ramp tracking: toe hugs the shore, crest equals the
        // hinterland edge.
        for vi in 0..nv {
            if cliff_toe[vi] {
                let mut shore = f32::MAX;
                for &nb in terrain.adj_of(vi) {
                    if shore_kind(owner[nb]) {
                        shore = shore.min(e[nb]);
                    }
                }
                if shore < f32::MAX {
                    let target = (shore.max(-0.02) + 0.02).min(0.06);
                    residual += (e[vi] - target).abs();
                    e[vi] = target;
                }
            } else if cliff_crest[vi] {
                let mut hinterland = f32::MIN;
                for &nb in terrain.adj_of(vi) {
                    if owner[nb].is_land() && owner[nb] != Terrain::Cliff && !shore_kind(owner[nb]) {
                        hinterland = hinterland.max(e[nb]);
                    }
                }
                if hinterland > f32::MIN {
                    residual += (e[vi] - hinterland).abs();
                    e[vi] = hinterland;
                }
            }
        }
        // 4) tile ranges — hard constraints and always get the
        // final word each iteration, so the finished field satisfies every
        // tile's elevation range exactly (caps are best-effort where the tile
        // map demands steeper chains than they allow).
        for vi in 0..nv {
            let c = e[vi].clamp(lo[vi], hi[vi]);
            residual += (c - e[vi]).abs();
            e[vi] = c;
        }
        // 4) bridge-entry pads LAST: the deck is a built structure — its
        // footing is dead flat even where tile ranges disagree slightly.
        for pad in &pads {
            let mean: f32 = pad.iter().map(|&vi| e[vi]).sum::<f32>() / pad.len() as f32;
            // A footing sits at shore level regardless of what it averaged.
            let footing = mean.clamp(0.01, 0.08);
            for &vi in pad {
                residual += (e[vi] - footing).abs();
                e[vi] = footing;
            }
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
        let blend_of: std::collections::BTreeMap<u32, (u8, u8)> =
            state.blends.iter().map(|&(fi, a, b)| (fi, (a, b))).collect();
        // Bridge-entry pad verts are a built-structure exception (like canyons):
        // the footing is flattened to shore level regardless of tile ranges.
        let mut pad_vert = vec![false; terrain.vert_count()];
        for span in &state.bridges {
            for end in [span.first(), span.last()].into_iter().flatten() {
                let (east, north) = end.tangent_basis();
                let step = 30.0 / crate::sphere::PLANET_RADIUS;
                for dir in [
                    end.0,
                    (end.0 + east * step).normalize(),
                    (end.0 - east * step).normalize(),
                    (end.0 + north * step).normalize(),
                    (end.0 - north * step).normalize(),
                ] {
                    for (vi, _) in terrain.kernel(crate::sphere::SpherePos::new(dir)) {
                        pad_vert[vi] = true;
                    }
                }
            }
        }
        for vi in 0..terrain.vert_count() {
            if canyon[vi] || pad_vert[vi] {
                continue;
            }
            let Some(fi) = state.grid.planet.face_at(terrain.vert_dir(vi)) else { continue };
            // Blend faces ramp between both kinds' ranges (mirror the solver).
            let (rlo, rhi) = match blend_of.get(&(fi as u32)) {
                Some(&(_, b)) if b >= crate::level::BLEND_FEATURE_MIN => elev_range(state.tiles[fi]),
                Some(&(a, b)) => {
                    let (alo, ahi) = elev_range(Terrain::ALL[a as usize]);
                    let (blo, bhi) = elev_range(Terrain::ALL[b as usize]);
                    (alo.min(blo), ahi.max(bhi))
                }
                None => elev_range(state.tiles[fi]),
            };
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

        // No enclosed water body smaller than the minimum (no 1-cell lakes).
        let mut visited = vec![false; state.grid.nv];
        for start in 0..state.grid.nv {
            if !state.cells[start].is_water() || state.cells[start] == Terrain::River || visited[start] {
                continue;
            }
            let mut body = vec![start];
            let mut q = VecDeque::from([start]);
            visited[start] = true;
            while let Some(cur) = q.pop_front() {
                for &nb in &state.grid.vert_adj[cur] {
                    let nb = nb as usize;
                    if state.cells[nb].is_water() && state.cells[nb] != Terrain::River && !visited[nb] {
                        visited[nb] = true;
                        body.push(nb);
                        q.push_back(nb);
                    }
                }
            }
            let is_ocean = body.iter().any(|&vi| {
                cell_zone(&state.grid, terrain, vi) == crate::zones::ZoneKind::Ocean
            });
            assert!(
                is_ocean || body.len() >= MIN_WATER_BODY_CELLS,
                "enclosed water body of only {} cells survived",
                body.len()
            );
        }

        // Every corner of every DeepOcean face renders below the waterline
        // (the mesh displaces corners by the interpolated field).
        for fi in 0..state.grid.n {
            if state.tiles[fi] != Terrain::DeepOcean {
                continue;
            }
            for v in &state.grid.unit_tris[fi] {
                let corner = crate::sphere::SpherePos::new(v.normalize());
                let elev = terrain.elevation_at(corner);
                if elev >= 0.0 {
                    let detail: Vec<String> = terrain.kernel(corner).iter().map(|&(vi, _)| {
                        let owner = state.grid.planet.face_at(terrain.vert_dir(vi))
                            .map(|of| format!("{:?}", state.tiles[of]))
                            .unwrap_or("?".into());
                        let d = corner.distance(crate::sphere::SpherePos::new(terrain.vert_dir(vi)));
                        format!("v{vi}={:.2}({owner},{d:.0}m)", e[vi])
                    }).collect();
                    panic!("DeepOcean face {fi} corner renders above water: {elev} kernel: {}", detail.join(" "));
                }
            }
        }

        // Roads never sit on water tiles.
        for fi in 0..state.grid.n {
            if state.painted.roads.contains(fi) {
                assert!(state.tiles[fi].is_land(), "road painted on water tile {fi}");
            }
        }

        // Bridge-entry pads are flat: the deck meets the ground seamlessly.
        for span in &state.bridges {
            for end in [span.first(), span.last()].into_iter().flatten() {
                let slope = terrain.slope(*end);
                assert!(slope < 0.15, "bridge entry not flat: slope {slope}");
            }
        }

        // Tile identity lives on hex cells: two same-type cells can only meet
        // along an edge, so vertex pinches are impossible by construction.
        // Check the derivation instead: every face's type is one of its
        // corner cells, and blend faces genuinely have mixed corners.
        {
            for fi in 0..state.grid.n {
                let corners = state.grid.face_verts[fi];
                assert!(
                    corners.iter().any(|&vi| state.cells[vi as usize] == state.tiles[fi]),
                    "face {fi} derived {:?} not among its corner cells",
                    state.tiles[fi]
                );
            }

            // Lake-to-ocean distance ≥ 10 edge steps.
            let mut dist = vec![u16::MAX; state.grid.n];
            let mut q: VecDeque<usize> = VecDeque::new();
            for fi in 0..state.grid.n {
                if state.tiles[fi].is_water()
                    && terrain.zones().kind_at_fine(fi) == crate::zones::ZoneKind::Ocean
                {
                    dist[fi] = 0;
                    q.push_back(fi);
                }
            }
            while let Some(cur) = q.pop_front() {
                if dist[cur] >= 10 {
                    continue;
                }
                for &nb in &state.grid.adj[cur] {
                    let nb = nb as usize;
                    if dist[nb] == u16::MAX {
                        dist[nb] = dist[cur] + 1;
                        q.push_back(nb);
                    }
                }
            }
            for fi in 0..state.grid.n {
                if state.tiles[fi] == Terrain::Lake {
                    assert!(
                        dist[fi] > 10,
                        "lake face {fi} only {} tiles from ocean water",
                        dist[fi]
                    );
                }
            }
        }

        // Cliff tiles are ramps: toe verts hug the shore (lower neighbors
        // unchanged), crest verts never rise above the hinterland behind them
        // (nothing sticks out) — the drop happens across the cliff face.
        let vkind: Vec<Terrain> = (0..terrain.vert_count())
            .map(|vi| {
                state.grid.planet.face_at(terrain.vert_dir(vi))
                    .map(|fi| state.tiles[fi])
                    .unwrap_or(Terrain::Plains)
            })
            .collect();
        let shore_kind = |t: Terrain| t.is_water() || t == Terrain::Beach;
        for a in 0..terrain.vert_count() {
            if vkind[a] != Terrain::Cliff || pad_vert[a] || canyon[a] {
                continue;
            }
            let shoreside = terrain.adj_of(a).iter().any(|&nb| shore_kind(vkind[nb]));
            if shoreside {
                assert!(e[a] <= 0.08, "cliff toe floats above the shore: {}", e[a]);
            } else {
                let hinterland = terrain.adj_of(a).iter()
                    .filter(|&&nb| vkind[nb].is_land() && vkind[nb] != Terrain::Cliff && !shore_kind(vkind[nb]))
                    .map(|&nb| e[nb])
                    .fold(f32::MIN, f32::max);
                if hinterland > f32::MIN {
                    assert!(
                        e[a] <= hinterland + 0.05,
                        "cliff crest sticks out: {} above hinterland {}",
                        e[a], hinterland
                    );
                }
            }
        }

        // Blend marks link two differing plain kinds actually adjacent there.
        assert!(!state.blends.is_empty(), "no blends marked");
        for &(fi, a, b) in &state.blends {
            assert_ne!(a, b);
            assert_eq!(state.tiles[fi as usize] as u8, a, "blend face kind mismatch");
        }
    }

    #[test]
    fn kernel_is_local() {
        // The interpolation kernel must return genuinely nearby vertices.
        // Guards the polar search bug: the fixed 3x3 grid window returned
        // verts up to 335m away near the poles (longitude cells shrink).
        let terrain = TerrainGen::init(1);
        for i in 0..2000 {
            let u = (i as f32 * 0.6180339) % 1.0;
            let v = (i as f32 * 0.7548776) % 1.0;
            let p = crate::sphere::random_point(u, v);
            for (vi, _) in terrain.kernel(p) {
                let d = p.distance(crate::sphere::SpherePos::new(terrain.vert_dir(vi)));
                assert!(d < 200.0, "kernel vert {d:.0}m away at lat {:.0}", p.0.y.asin().to_degrees());
            }
        }
    }

    #[test]
    fn lakes_stay_enclosed() {
        // No lake-zone water may connect to the ocean — the rim dam guarantees
        // every lake is its own body (guards the drain-channel bug where lakes
        // leaked to the sea along coarse-face edges and became ocean inlets).
        let state = run(1337, |_| {});
        let terrain = state.terrain.as_ref().unwrap();
        let mut lake_faces = 0;
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
            let has_lake_zone = body.iter().any(|&fi| {
                terrain.zones().kind_at_fine(fi) == crate::zones::ZoneKind::Lake
            });
            let has_ocean_zone = body.iter().any(|&fi| {
                terrain.zones().kind_at_fine(fi) == crate::zones::ZoneKind::Ocean
            });
            if has_lake_zone {
                lake_faces += body.len();
                assert!(!has_ocean_zone, "lake body of {} faces connects to the ocean", body.len());
            }
        }
        assert!(lake_faces > 100, "lakes nearly vanished: {lake_faces} faces");
    }

    #[test]
    fn deterministic_pipeline() {
        let a = run(42, |_| {});
        let b = run(42, |_| {});
        assert_eq!(a.terrain.unwrap().vert_elevations(), b.terrain.unwrap().vert_elevations());
        assert_eq!(a.cells, b.cells);
        assert_eq!(a.tiles, b.tiles);
        assert_eq!(a.regions.len(), b.regions.len());
    }
}
