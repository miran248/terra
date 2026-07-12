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

use crate::level::{FloraData, RegionData, RegionKind, StructureData, SLOPE_CLIFF, SLOPE_FLAT, SLOPE_GENTLE, SLOPE_STEEP, slope_walkable, DEPTH_SHALLOW, DEPTH_DEEP, DEPTH_ABYSS, LANDFORM_WATER, LANDFORM_LOWLAND, LANDFORM_VALLEY, LANDFORM_HILLS, LANDFORM_MOUNTAINS, LANDFORM_PLATEAU, ROAD_MAT_DIRT, ROAD_MAT_GRAVEL, ROAD_MAT_ROCK, ROAD_MAT_SAND, FLORA_BERRY, FLORA_BUSH, FLORA_CACTUS, FLORA_DEADTREE, FLORA_FLOWER, FLORA_GRASS, FLORA_LOG, FLORA_MUSHROOM, FLORA_REED, FLORA_ROCK, FLORA_TREE, NO_REGION, STRUCT_CAMPFIRE, STRUCT_DOCK, STRUCT_FARM, STRUCT_RUIN, STRUCT_WALL, STRUCT_WATCHTOWER, STRUCT_WELL, TAG_BRIDGE, TAG_BRIDGE_ENTRY, TAG_ROAD, TAG_TOWN};
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
    grid.face_verts.iter().map(|idx| {
        derive_one(cells[idx[0] as usize], cells[idx[1] as usize], cells[idx[2] as usize])
    }).collect()
}

fn derive_one(a: Terrain, b: Terrain, c: Terrain) -> Terrain {
    let pri = |t: Terrain| match t {
        Terrain::Beach | Terrain::Cliff | Terrain::LakeShore | Terrain::RiverBank => 0u8,
        t if t.is_water() => 1,
        _ => 2,
    };
    if a == b || a == c {
        a
    } else if b == c {
        b
    } else if [a, b, c].iter().filter(|t| t.is_water()).count() >= 2 {
        // Two water kinds meeting (a river mouth: River + Ocean + bank) are
        // never dammed by the third corner — the face stays water, keeping
        // the water surface continuous where bodies join.
        [a, b, c].into_iter().filter(|t| t.is_water()).min_by_key(|t| *t as u8).unwrap()
    } else {
        [a, b, c].into_iter().min_by_key(|t| (pri(*t), *t as u8)).unwrap()
    }
}

/// The neighbors of a cell in CYCLIC order (walking the face fan), starting
/// from the lowest-id neighbor — deterministic.
fn ring(grid: &Grid, v: usize) -> Vec<usize> {
    let mut out = Vec::with_capacity(grid.vert_adj[v].len());
    let mut cur = grid.vert_adj[v][0] as usize;
    out.push(cur);
    loop {
        let next = grid.vert_faces[v].iter().find_map(|&f| {
            let idx = grid.face_verts[f as usize];
            if !idx.contains(&(cur as u32)) {
                return None;
            }
            let third = *idx.iter().find(|&&x| x as usize != v && x as usize != cur)?;
            (!out.contains(&(third as usize))).then_some(third as usize)
        });
        match next {
            Some(t) => {
                out.push(t);
                cur = t;
            }
            None => break,
        }
    }
    debug_assert_eq!(out.len(), grid.vert_adj[v].len());
    out
}

/// THE LINKING RULE, generic over every type: around any cell, the faces
/// deriving to a given type must form ONE edge-connected fan — same-type
/// triangles are never linked by a lone vertex. Where a fan splits into two
/// runs, retype one gap cell (same land/water class, deterministic order,
/// only if it provably merges the runs) so the band thickens through the
/// pinch. Iterated to a fixed point.
fn link_tile_pinches(grid: &Grid, cells: &mut [Terrain]) {
    // Pinch potential at a cell: how many extra runs its face fan carries,
    // summed over types. Zero everywhere ⇔ the linking rule holds.
    fn potential_at(grid: &Grid, cells: &[Terrain], v: usize) -> usize {
        let ring = ring(grid, v);
        let n = ring.len();
        let ts: Vec<Terrain> = (0..n)
            .map(|i| derive_one(cells[v], cells[ring[i]], cells[ring[(i + 1) % n]]))
            .collect();
        let mut types = ts.clone();
        types.sort_by_key(|t| *t as u8);
        types.dedup();
        types.iter().map(|&t| {
            let runs = (0..n).filter(|&i| ts[i] == t && ts[(i + n - 1) % n] != t).count();
            runs.saturating_sub(1)
        }).sum()
    }
    // A retype of cell c only changes faces containing c — the fans of c and
    // its ring. Accept a candidate only if the potential over that
    // neighborhood strictly DROPS: the loop then converges (the global
    // potential is a non-negative integer that decreases with every change).
    let local = |cells: &[Terrain], c: usize| -> usize {
        potential_at(grid, cells, c)
            + ring(grid, c).iter().map(|&w| potential_at(grid, cells, w)).sum::<usize>()
    };
    for it in 0..64 {
        debug_assert!(it < 63, "link_tile_pinches did not converge");
        let mut changed = false;
        for v in 0..grid.nv {
            if potential_at(grid, cells, v) == 0 {
                continue;
            }
            let ring_v = ring(grid, v);
            let ts: Vec<Terrain> = {
                let n = ring_v.len();
                (0..n)
                    .map(|i| derive_one(cells[v], cells[ring_v[i]], cells[ring_v[(i + 1) % n]]))
                    .collect()
            };
            let mut types = ts;
            types.sort_by_key(|t| *t as u8);
            types.dedup();
            // Prefer a class-preserving retype; fall back to crossing the
            // waterline only when nothing else merges the runs.
            'candidates: for cross in [false, true] {
                for t in &types {
                    let t = *t;
                    for &c in &ring_v {
                        // Rivers are planned linear features two cells wide —
                        // consuming a cell can sever the channel.
                        if cells[c] == t
                            || cells[c] == Terrain::River
                            || (cells[c].is_water() != t.is_water()) != cross
                        {
                            continue;
                        }
                        // Turning land into water must extend an existing
                        // body of that kind, never strand a puddle.
                        if cross
                            && t.is_water()
                            && !grid.vert_adj[c].iter().any(|&nb| cells[nb as usize] == t)
                        {
                            continue;
                        }
                        let before = local(cells, c);
                        let old = cells[c];
                        cells[c] = t;
                        if local(cells, c) < before {
                            changed = true;
                            break 'candidates;
                        }
                        cells[c] = old;
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
}

/// Same rule for painted features: the solid faces of a feature around any
/// cell must form one edge-connected fan. Solid needs all 3 corners painted,
/// so a pinch is two painted runs around a painted cell — paint a passable
/// gap cell to merge them (e.g. the miter cell at a road's 60° turn).
fn link_feature_pinches(grid: &Grid, bits: &mut BitSet, passable: impl Fn(usize) -> bool) {
    for _ in 0..8 {
        let mut changed = false;
        for v in 0..grid.nv {
            if !bits.contains(v) {
                continue;
            }
            let ring = ring(grid, v);
            let n = ring.len();
            let runs = |bits: &BitSet| -> usize {
                let solid =
                    |i: usize| bits.contains(ring[i]) && bits.contains(ring[(i + 1) % n]);
                (0..n).filter(|&i| solid(i) && !solid((i + n - 1) % n)).count()
            };
            let r = runs(bits);
            if r < 2 {
                continue;
            }
            for i in 0..n {
                if bits.contains(ring[i]) || !passable(ring[i]) {
                    continue;
                }
                bits.insert(ring[i]);
                if runs(bits) < r {
                    changed = true;
                    break;
                }
                // BitSet has no remove; rebuild the bit by clearing the word bit.
                bits.0[ring[i] >> 6] &= !(1u64 << (ring[i] & 63));
            }
        }
        if !changed {
            break;
        }
    }
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

/// Built features, painted per CELL (like terrain identity): a face is a
/// solid feature where ≥2 of its corner cells are painted, and fades out at
/// the edges through per-corner colors — the same construction as terrain,
/// so feature footprints can never pinch or zigzag either.
#[derive(Clone)]
pub struct Painted {
    pub roads: BitSet,
    pub towns: BitSet,
    pub bridges: BitSet,
    pub bridge_entries: BitSet,
}

impl Painted {
    fn empty(nv: usize) -> Self {
        Self {
            roads: BitSet::new(nv),
            towns: BitSet::new(nv),
            bridges: BitSet::new(nv),
            bridge_entries: BitSet::new(nv),
        }
    }
}

/// How many of a face's corner cells are in the set.
fn painted_corners(grid: &Grid, bits: &BitSet, fi: usize) -> usize {
    grid.face_verts[fi].iter().filter(|&&vi| bits.contains(vi as usize)).count()
}

/// A face is a solid feature surface only when the feature owns ALL its
/// corners: for a band painted as two parallel lattice lines that is exactly
/// the parallelogram strip between them (straight edges = the lines
/// themselves). Faces with 1–2 painted corners form one straight-edged strip
/// on each side — the blend band, rendered as a per-corner gradient.
fn face_solid(grid: &Grid, bits: &BitSet, fi: usize) -> bool {
    painted_corners(grid, bits, fi) == 3
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
    pub flora: Vec<FloraData>,
    pub structures: Vec<StructureData>,
    /// Per-cell terrain steepness (0 Flat, 1 Gentle, 2 Steep, 3 Cliff) from the
    /// SOLVED field. Walkability and feature placement gate on this, so a
    /// mountain pass (Gentle inside Mountains) is traversable and a "flat"
    /// biome that solved steep is not.
    pub slope_class: Vec<u8>,
    /// Per-cell water depth class (DEPTH_*) for water cells.
    pub water_depth: Vec<u8>,
    /// Per-cell macro landform (LANDFORM_*), from the proposed field.
    pub landform: Vec<u8>,
}

impl GenState {
    pub fn new(seed: u32) -> Self {
        let grid = Grid::new(seed);
        let painted = Painted::empty(grid.nv);
        Self {
            grid,
            terrain: None,
            cells: Vec::new(),
            tiles: Vec::new(),
            painted,
            bridges: Vec::new(),
            roads: Vec::new(),
            blends: Vec::new(),
            regions: Vec::new(),
            face_region: Vec::new(),
            mesh_tris: Vec::new(),
            mesh_colors: Vec::new(),
            tag_off: Vec::new(),
            tag_data: Vec::new(),
            flora: Vec::new(),
            structures: Vec::new(),
            slope_class: Vec::new(),
            water_depth: Vec::new(),
            landform: Vec::new(),
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
    /// Sub-tile decoration scatter: trees, bushes, flowers on the finished
    /// mesh, densities from the tile map.
    PlaceFlora,
    /// Contextual built structures (ruins, docks, walls, wells, …).
    PlaceStructures,
    /// Per-cell steepness from the solved field (slope class).
    ClassifySlope,
    /// Per-cell macro landform from the proposed field (base layer).
    ClassifyLandform,
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
    FloraPlaced(Vec<FloraData>),
    StructuresPlaced(Vec<StructureData>),
    SlopeClassified(Vec<u8>, Vec<u8>),
    LandformClassified(Vec<u8>),
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
            Event::FloraPlaced(f) => format!("flora placed: {}", f.len()),
            Event::StructuresPlaced(v) => format!("structures placed: {}", v.len()),
            Event::SlopeClassified(sc, wd) => {
                let cliffs = sc.iter().filter(|&&c| c == SLOPE_CLIFF).count();
                let abyss = wd.iter().filter(|&&d| d == DEPTH_ABYSS).count();
                format!("relief classified: {cliffs} cliff cells, {abyss} abyss cells")
            }
            Event::LandformClassified(lf) => {
                let mtn = lf.iter().filter(|&&l| l == LANDFORM_MOUNTAINS || l == LANDFORM_PLATEAU).count();
                format!("landform classified: {mtn} mountain/plateau cells")
            }
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
        Command::ClassifyLandform => {
            vec![Event::LandformClassified(classify_landform(&state.grid, state.terrain()))]
        }
        Command::ClassifySlope => {
            let slope = classify_slope(&state.grid, state.terrain());
            let depth = classify_water_depth(&state.grid, &state.cells, state.terrain());
            vec![Event::SlopeClassified(slope, depth)]
        }
        Command::ClassifyTiles => {
            let terrain = state.terrain();
            let grid = &state.grid;
            // Cover per CELL: the macro landform (already classified) sets the
            // base — high ground gets rock/snow, low ground gets a climate
            // biome — so a "Forest" is genuinely a forested LOWLAND, not a
            // steep slope that merely isn't labelled Mountain.
            let cells = (0..grid.nv)
                .map(|vi| classify_cover(grid, terrain, &state.landform, vi))
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
            let (painted, roads) = paint_features(
                &state.grid, state.terrain(), &state.cells, &state.slope_class,
            );
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
                &state.grid, state.terrain(), &state.cells, &state.landform, &state.tiles,
                &state.painted, &state.blends,
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
                build_bridges(
                &state.grid, state.terrain(), &state.cells, &state.slope_class,
                &state.tiles, &state.face_region, &mut painted,
            );
            vec![Event::BridgesSelected(bridges, painted)]
        }
        Command::BuildMesh => {
            let (tris, cols) =
                build_mesh(
                &state.grid, state.terrain(), &state.cells, &state.painted,
                &state.water_depth, &state.landform, &state.slope_class,
            );
            vec![Event::MeshBuilt(tris, cols)]
        }
        Command::BuildTags => {
            let (off, data) = build_face_tags(&state.grid, &state.painted);
            vec![Event::TagsBuilt(off, data)]
        }
        Command::PlaceFlora => {
            vec![Event::FloraPlaced(place_flora(
                &state.grid, state.terrain(), &state.tiles, &state.painted, &state.mesh_tris,
            ))]
        }
        Command::PlaceStructures => {
            vec![Event::StructuresPlaced(place_structures(
                &state.grid, state.terrain(), &state.tiles, &state.painted,
                &state.slope_class, &state.mesh_tris,
            ))]
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
        Event::FloraPlaced(f) => state.flora = f,
        Event::StructuresPlaced(v) => state.structures = v,
        Event::SlopeClassified(sc, wd) => {
            state.slope_class = sc;
            state.water_depth = wd;
        }
        Event::LandformClassified(lf) => state.landform = lf,
    }
    state
}

pub fn react(event: &Event) -> Vec<Command> {
    match event {
        Event::TerrainInitialized(_) => vec![Command::ProposeElevation],
        // Climate follows every field change; FIFO runs it before the next step.
        Event::ElevationProposed(_) => vec![Command::ComputeClimate, Command::ClassifyLandform],
        Event::LandformClassified(_) => vec![Command::PlanRivers],
        Event::ClimateComputed(..) => vec![],
        Event::RiversPlanned(_) => vec![Command::PlaceSettlements],
        Event::SettlementsPlaced(_) => vec![Command::PlanRoads],
        Event::RoadsPlanned(_) => vec![Command::ClassifyTiles],
        // Water is normalized BEFORE rivers paint: min-size fills and dams
        // must not punch holes into an already-painted channel.
        Event::TilesClassified(_) => vec![Command::NormalizeWater],
        Event::WaterNormalized(_) => vec![Command::PaintRivers],
        Event::RiversPainted(_) => vec![Command::ResolveTransitions],
        // Elevation is solved FIRST, then EVERY built feature CONFORMS to the
        // finished terrain: slope is classified from the solved field, then
        // roads route around steep ground and bridges/structures gate on it —
        // a road or bridge never lands on a slope, whatever the tile is
        // labelled. Regions/blends follow so they see the final footprints.
        Event::TransitionsResolved(_) => vec![Command::SolveElevation],
        Event::ElevationSolved { .. } => vec![Command::ComputeClimate, Command::ClassifySlope],
        Event::SlopeClassified(..) => vec![Command::PaintFeatures],
        Event::FeaturesPainted(..) => vec![Command::BuildRegions],
        Event::RegionsBuilt(..) => vec![Command::SelectBridges],
        Event::BridgesSelected(..) => vec![Command::MarkBlends],
        Event::BlendsMarked(_) => vec![Command::BuildMesh],
        Event::MeshBuilt(..) => vec![Command::BuildTags],
        Event::TagsBuilt(..) => vec![Command::PlaceFlora],
        Event::FloraPlaced(_) => vec![Command::PlaceStructures],
        Event::StructuresPlaced(_) => vec![],
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
/// Painted as an edge PAIR (chain + parallel partner line), like roads: a
/// single chain's derived faces only touch at the chain vertices.
fn paint_rivers(grid: &Grid, terrain: &TerrainGen, cells: &mut [Terrain]) {
    let sea = |t: Terrain| matches!(t, Terrain::Ocean | Terrain::Lake);
    for path in &terrain.river_paths {
        let mut chain = vert_chain(grid, path);
        // The planned endpoint sits on the PROPOSED waterline; normalization
        // may have moved the coast since. If the channel no longer meets open
        // water, extend it from its end along the shortest cell path to the
        // nearest sea/lake cell — a river always reaches a larger body.
        let reaches = chain.iter().any(|&vi| {
            sea(cells[vi]) || grid.vert_adj[vi].iter().any(|&nb| sea(cells[nb as usize]))
        });
        if !reaches {
            if let Some(&end) = chain.last() {
                let mut prev: BTreeMap<usize, usize> = BTreeMap::new();
                let mut q = VecDeque::from([(end, 0usize)]);
                prev.insert(end, end);
                'bfs: while let Some((cur, d)) = q.pop_front() {
                    if d >= 60 {
                        continue;
                    }
                    for &nb in &grid.vert_adj[cur] {
                        let nb = nb as usize;
                        if prev.contains_key(&nb) {
                            continue;
                        }
                        prev.insert(nb, cur);
                        if sea(cells[nb]) {
                            let mut ext = Vec::new();
                            let mut c = cur;
                            while c != end {
                                ext.push(c);
                                c = prev[&c];
                            }
                            ext.reverse();
                            chain.extend(ext);
                            break 'bfs;
                        }
                        q.push_back((nb, d + 1));
                    }
                }
            }
        }
        for vi in widen_band_sym(grid, &chain) {
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
        let body = flood_cells(grid, start, &mut visited,
            |nb| cells[nb].is_water() && cells[nb] != Terrain::River);
        // A body is an OCEAN only if it reaches ocean-zone cells AND is large
        // enough to be one: an isolated pocket of ocean-zone faces walled off
        // by land (a single coarse face — hence the tell-tale triangle shape)
        // is a lake, not a sea. Min ocean size sits well above any lake.
        let is_ocean = body.len() >= size_range(Terrain::Ocean).0
            && body.iter()
                .any(|&vi| cell_zone(grid, terrain, vi) == crate::zones::ZoneKind::Ocean);
        if !is_ocean && body.len() < size_range(Terrain::Lake).0 {
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
        if cells[vi] == Terrain::Ocean {
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
        let body = flood_cells(grid, start, &mut visited, |nb| cells[nb] == Terrain::Lake);
        if body.len() < size_range(Terrain::Lake).0 {
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
        // Cell steps are ~17.5m: with resolution doubled to pack in content,
        // core water sits ≥3 steps from land (half the old physical width).
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
/// Size envelope, in CELLS, for every tile kind — the area analogue of
/// `elev_range`. `min`: a contiguous region smaller than this is not viable
/// and gets absorbed/filled/renamed away. `max`: a region larger than this is
/// split (only kinds with a splitter — the coastal bands — enforce it; all
/// others are `usize::MAX`, i.e. unbounded). Oceans and lakes are separated
/// here: min-ocean sits one above max-lake, so a small isolated ocean-zone
/// pocket falls through to Lake.
fn size_range(t: Terrain) -> (usize, usize) {
    use Terrain::*;
    const INF: usize = usize::MAX;
    match t {
        // A real sea; anything smaller on ocean zone is reclassified a lake.
        Ocean => (4000, INF),
        // Enclosed water: a puddle below min is filled; capped just under the
        // ocean minimum so "min ocean > max lake" holds by construction.
        Lake => (60, 3999),
        // Linear water/bands — area minimums don't apply.
        River | LakeShore | RiverBank => (1, INF),
        // Coastal bands: thin, with a min length and a max named-segment length.
        Beach => (20, 60),
        Cliff => (1, 24),
        // Land biomes: a patch below min is speckle and joins its surroundings.
        Desert | Plains | Forest | Tundra | Mountain | Snow
            | Swamp | Jungle | Savanna | Volcanic | Glacier => (40, INF),
    }
}

/// Nearest cell to a point: the closest corner of the face under it.
fn nearest_cell(grid: &Grid, p: SpherePos) -> Option<usize> {
    let fi = grid.planet.face_at(p.0)?;
    grid.face_verts[fi].iter().copied()
        .max_by(|&a, &b| {
            grid.verts[a as usize].dot(p.0)
                .partial_cmp(&grid.verts[b as usize].dot(p.0)).unwrap()
        })
        .map(|vi| vi as usize)
}

/// Lattice-aligned routing: A* over cells where changing direction costs
/// extra, so paths run straight along one of the 3 lattice axes and turn in
/// discrete 60° steps spaced apart (the lattice dictates the turning radius).
/// Water cells are impassable — roads route around lakes on the lattice.
/// Returns the chain including both endpoints, or empty if unreachable.
/// Macro landform per cell from the PROPOSED field: elevation bands
/// (lowland/hills/mountains) split by local relief (a flat high patch is a
/// plateau), with low ground boxed in by higher ground marked as valley.
/// Because it reads a smooth continuous field the bands are naturally ordered
/// (no lowland directly against a peak). Speckle is absorbed into the
/// surrounding landform so each is a coherent cluster.
fn classify_landform(grid: &Grid, terrain: &TerrainGen) -> Vec<u8> {
    let e: Vec<f32> = (0..grid.nv).map(|vi| terrain.elevation_at(grid.vert_pos(vi))).collect();
    let mut lf = vec![LANDFORM_WATER; grid.nv];
    for vi in 0..grid.nv {
        if e[vi] < 0.0 {
            continue; // water
        }
        let relief = grid.vert_adj[vi].iter()
            .map(|&nb| (e[vi] - e[nb as usize]).abs())
            .fold(0.0f32, f32::max);
        lf[vi] = if e[vi] >= 0.35 {
            if relief < 0.035 { LANDFORM_PLATEAU } else { LANDFORM_MOUNTAINS }
        } else if e[vi] >= 0.14 {
            LANDFORM_HILLS
        } else {
            LANDFORM_LOWLAND
        };
    }
    // Valleys: low ground hemmed in by higher landform on most sides.
    let higher = |l: u8| matches!(l, LANDFORM_HILLS | LANDFORM_MOUNTAINS | LANDFORM_PLATEAU);
    let mut valleys = Vec::new();
    for vi in 0..grid.nv {
        if lf[vi] == LANDFORM_LOWLAND
            && grid.vert_adj[vi].iter().filter(|&&nb| higher(lf[nb as usize])).count() >= 3
        {
            valleys.push(vi);
        }
    }
    for vi in valleys {
        lf[vi] = LANDFORM_VALLEY;
    }
    // Absorb speckle: a landform cluster below the min joins its most common
    // land neighbor's landform, so each landform is a coherent region.
    absorb_small_landforms(grid, &mut lf);
    lf
}

const MIN_LANDFORM_CELLS: usize = 25;

fn absorb_small_landforms(grid: &Grid, lf: &mut [u8]) {
    absorb_small_clusters(
        grid, lf,
        |l| l != LANDFORM_WATER,
        |_| MIN_LANDFORM_CELLS,
        |l| l != LANDFORM_WATER,
    );
}

/// Cover per cell: the macro landform sets the base — high ground (mountains,
/// plateaus) gets rock/snow/ice/volcanic, everything lower gets a climate
/// biome. Water zones stay water. This is the landform → biome layering.
fn classify_cover(grid: &Grid, terrain: &TerrainGen, landform: &[u8], vi: usize) -> Terrain {
    let pos = grid.vert_pos(vi);
    let e = terrain.elevation_at(pos);
    match cell_zone(grid, terrain, vi) {
        crate::zones::ZoneKind::Ocean => Terrain::Ocean,
        crate::zones::ZoneKind::Lake => {
            if e < 0.0 { Terrain::Lake } else { land_cover(terrain, landform[vi], pos) }
        }
        _ => {
            if e < 0.0 { Terrain::Ocean } else { land_cover(terrain, landform[vi], pos) }
        }
    }
}

fn land_cover(terrain: &TerrainGen, landform: u8, pos: SpherePos) -> Terrain {
    let t = terrain.temperature_at(pos);
    let m = terrain.moisture_at(pos);
    let e = terrain.elevation_at(pos);
    // A mountain is not one thing: snow caps the high cells (above the snow
    // line), forest clothes the warm/wet lower flanks, bare rock fills the
    // rugged middle, ice covers the cold ranges, and hot high peaks are
    // volcanic. So a single range shows rock AND snow AND (sometimes) forest.
    if matches!(landform, LANDFORM_MOUNTAINS | LANDFORM_PLATEAU) {
        return if t < -12.0 {
            Terrain::Glacier
        } else if e > 0.70 {
            if t > 24.0 { Terrain::Volcanic } else { Terrain::Snow }
        } else if t < -2.0 {
            Terrain::Snow
        } else if m > 0.15 && e < 0.55 {
            Terrain::Forest
        } else {
            Terrain::Mountain
        };
    }
    // Low/rolling ground: climate biome cover.
    if t < -28.0 { return Terrain::Glacier; }
    if t < -15.0 { return Terrain::Snow; }
    if t < 0.0 { return Terrain::Tundra; }
    if e < 0.12 && m > 0.28 { return Terrain::Swamp; }
    if t > 24.0 && m > 0.25 { return Terrain::Jungle; }
    if t > 30.0 && m < -0.15 { return Terrain::Desert; }
    if t > 22.0 && m < 0.05 { return Terrain::Savanna; }
    if m > 0.10 { Terrain::Forest } else { Terrain::Plains }
}

/// Slope-class thresholds (rise/run ≈ tan angle) on the solved field.
const SLOPE_GENTLE_MAX: f32 = 0.18; // ~10°: flat/gentle boundary
const SLOPE_STEEP_MAX: f32 = 0.45;  // ~24°: gentle/steep (walkable) boundary
const SLOPE_CLIFF_MAX: f32 = 0.90;  // ~42°: steep/cliff (impassable) boundary

/// Per-cell steepness of the SOLVED surface — the micro landform layer.
/// Measured at CELL scale (max rise/run to an edge-neighbor over the real
/// ground distance), not at a sub-metre probe, so it reflects terrain the
/// player traverses rather than interpolation noise. Passes (gentle cells in
/// mountains) and escarpments (cliff cells) fall out of it automatically.
fn classify_slope(grid: &Grid, terrain: &TerrainGen) -> Vec<u8> {
    let alt: Vec<f32> = (0..grid.nv)
        .map(|vi| terrain.altitude(grid.vert_pos(vi)))
        .collect();
    (0..grid.nv)
        .map(|vi| {
            let a = grid.verts[vi];
            let mut worst = 0.0f32;
            for &nb in &grid.vert_adj[vi] {
                let nb = nb as usize;
                let dist = a.distance(grid.verts[nb]) * crate::sphere::PLANET_RADIUS;
                if dist > 1.0 {
                    worst = worst.max((alt[vi] - alt[nb]).abs() / dist);
                }
            }

            bucket(worst, &[SLOPE_GENTLE_MAX, SLOPE_STEEP_MAX, SLOPE_CLIFF_MAX])
        })
        .collect()
}

/// The number of ascending `thresholds` a value reaches — turns a measurement
/// into an ordered class (flat/gentle/steep/cliff, shallow/deep/abyss).
fn bucket(value: f32, thresholds: &[f32]) -> u8 {
    thresholds.iter().filter(|&&t| value >= t).count() as u8
}

/// Per-water-cell depth class from the solved surface: shore-shallows deepen
/// to abyss offshore (and lake/river beds shallow-to-deep by their concavity).
/// Land cells are DEPTH_SHALLOW (unused). The depth analogue of slope class.
fn classify_water_depth(grid: &Grid, cells: &[Terrain], terrain: &TerrainGen) -> Vec<u8> {
    (0..grid.nv)
        .map(|vi| {
            if !cells[vi].is_water() {
                return DEPTH_SHALLOW;
            }
            let e = terrain.elevation_at(grid.vert_pos(vi));
            bucket(-e, &[0.20, 0.55])
        })
        .collect()
}

/// Route on the cell lattice, refusing `blocked` cells and paying `extra` per
/// cell entered (so roads can be steered toward gentle ground without being
/// walled off it).
fn lattice_path(
    grid: &Grid,
    blocked: impl Fn(usize) -> bool,
    extra: impl Fn(usize) -> u64,
    from: usize,
    to: usize,
) -> Vec<usize> {
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;
    if from == to {
        return vec![from];
    }
    // Costs in milli-steps. One 60° turn ≈ 1.25 extra steps keeps runs long.
    const STEP: u64 = 1000;
    let turn_cost = |pd: Vec3, d: Vec3| ((1.0 - pd.dot(d)).max(0.0) * 2500.0) as u64;
    let edge_angle = grid.verts[0].angle_between(grid.verts[grid.vert_adj[0][0] as usize]);
    let h = |vi: usize| (grid.verts[vi].angle_between(grid.verts[to]) / edge_angle * 990.0) as u64;
    let dir = |a: usize, b: usize| (grid.verts[b] - grid.verts[a]).normalize();

    // State: (cell, slot of the edge we arrived through; 6 = start).
    let mut best: BTreeMap<(usize, usize), u64> = BTreeMap::new();
    let mut came: BTreeMap<(usize, usize), (usize, usize)> = BTreeMap::new();
    let mut heap: BinaryHeap<Reverse<(u64, u64, usize, usize)>> = BinaryHeap::new();
    best.insert((from, 6), 0);
    heap.push(Reverse((h(from), 0, from, 6)));
    while let Some(Reverse((_, g, vi, slot))) = heap.pop() {
        if best.get(&(vi, slot)).is_some_and(|&b| b < g) {
            continue;
        }
        if vi == to {
            let mut path = vec![vi];
            let mut cur = (vi, slot);
            while let Some(&prev) = came.get(&cur) {
                path.push(prev.0);
                cur = prev;
            }
            path.reverse();
            return path;
        }
        let pd = (slot < 6).then(|| dir(grid.vert_adj[vi][slot] as usize, vi));
        for &nb in &grid.vert_adj[vi] {
            let nb = nb as usize;
            if blocked(nb) && nb != to {
                continue;
            }
            let d = dir(vi, nb);
            let ng = g + STEP + extra(nb) + pd.map_or(0, |pd| turn_cost(pd, d));
            let nslot = grid.vert_adj[nb].iter().position(|&x| x as usize == vi).unwrap();
            if best.get(&(nb, nslot)).is_none_or(|&b| ng < b) {
                best.insert((nb, nslot), ng);
                came.insert((nb, nslot), (vi, slot));
                heap.push(Reverse((ng + h(nb), ng, nb, nslot)));
            }
        }
    }
    Vec::new()
}

/// A band needs TWO parallel lattice lines: a single chain's quads only touch
/// at the chain vertices. Widen the chain with each edge's left partner so
/// every face between the two lines has ≥2 painted corners — a gap-free strip
/// of stacked parallelograms with straight edges.
fn widen_band(grid: &Grid, chain: &[usize]) -> Vec<usize> {
    widen(grid, chain, false)
}

/// Three lattice lines: the chain plus BOTH side partners (~105m) — wide
/// enough that the elevation solver owns distinct channel and bank verts and
/// can actually carve a cross-section (rivers).
fn widen_band_sym(grid: &Grid, chain: &[usize]) -> Vec<usize> {
    widen(grid, chain, true)
}

fn widen(grid: &Grid, chain: &[usize], both_sides: bool) -> Vec<usize> {
    let mut out = chain.to_vec();
    for seg in chain.windows(2) {
        let (a, b) = (seg[0], seg[1]);
        let left = grid.verts[a].cross(grid.verts[b]);
        for &f in &grid.vert_faces[a] {
            let idx = grid.face_verts[f as usize];
            if !idx.contains(&(b as u32)) {
                continue;
            }
            let third = idx.iter().find(|&&v| v as usize != a && v as usize != b).unwrap();
            let side_ok = both_sides || grid.verts[*third as usize].dot(left) > 0.0;
            if side_ok && !out.contains(&(*third as usize)) {
                out.push(*third as usize);
            }
        }
    }
    out
}

/// Roads may not cross water: a planned path whose cell chain touches a water
/// cell is dropped entirely (crossing there needs a bridge, not a road).
fn paint_features(
    grid: &Grid,
    terrain: &TerrainGen,
    cells: &[Terrain],
    slope_class: &[u8],
) -> (Painted, Vec<Vec<SpherePos>>) {
    let mut painted = Painted::empty(grid.nv);
    let mut kept: Vec<Vec<SpherePos>> = Vec::new();
    // Roads route on WALKABLE ground: they follow valleys and mountain passes
    // and refuse water and steep slopes (conform, don't carve). A leg that has
    // no gentle dry route is dropped — that gap wants a bridge.
    // Roads may not cross water or a cliff, and are steered strongly toward
    // gentle ground (steep cells cost extra), so they follow valleys and
    // passes but can still climb a slope when they must.
    let blocked = |vi: usize| cells[vi].is_water() || slope_class[vi] == SLOPE_CLIFF;
    let extra = |vi: usize| match slope_class[vi] {
        SLOPE_FLAT => 0,
        SLOPE_GENTLE => 400,
        _ => 4000, // steep
    };
    'paths: for path in &terrain.road_paths {
        let mut chain: Vec<usize> = Vec::new();
        let waypoints: Vec<usize> = path.iter().filter_map(|p| nearest_cell(grid, *p)).collect();
        for leg in waypoints.windows(2) {
            let seg = lattice_path(grid, blocked, extra, leg[0], leg[1]);
            if seg.is_empty() {
                continue 'paths;
            }
            let skip = usize::from(chain.last() == seg.first());
            chain.extend(&seg[skip..]);
        }
        let band = widen_band(grid, &chain);
        if band.iter().any(|&vi| cells[vi].is_water() || slope_class[vi] == SLOPE_CLIFF) {
            continue;
        }
        for vi in band {
            painted.roads.insert(vi);
        }
        kept.push(path.clone());
    }
    // Towns sit on walkable ground within the settlement radius.
    for vi in 0..grid.nv {
        let pos = grid.vert_pos(vi);
        if slope_walkable(slope_class[vi])
            && terrain.settlement_anchors.iter().any(|a| a.distance(pos) <= TOWN_RADIUS)
        {
            painted.towns.insert(vi);
        }
    }
    link_feature_pinches(grid, &mut painted.roads, |vi| cells[vi].is_land());
    link_feature_pinches(grid, &mut painted.towns, |_| true);
    (painted, kept)
}

/// Gaps up to this bridge freely.
/// A tiny overshoot past the gentle anchor cell so the deck grounds just
/// inside solid ground (bridges conform — no long inland ramp).
const BRIDGE_ENTRY_OVERLAP: f32 = 3.0;
/// A river/lake crossing longer than this isn't a bridge — the water is too
/// wide (that would be a ferry, not a footbridge).
const BRIDGE_MAX_SPAN: f32 = 200.0;
/// Keep bridges apart: no two within this many meters.
const BRIDGE_MIN_SPACING: f32 = 400.0;
const BRIDGE_MAX_COUNT: usize = 12;
/// A land component smaller than this, ringed only by lake water, is an island
/// in the lake and earns a bridge to the mainland.
const LAKE_ISLAND_MAX_CELLS: usize = 1500;
/// An ocean island (small land ringed only by ocean) is bridged to the nearest
/// other landmass, but only across a SHORT gap (islands are seeded near land).
const OCEAN_ISLAND_MAX_CELLS: usize = 3000;
const OCEAN_BRIDGE_MAX_SPAN: f32 = 700.0;
/// A bridge FOOTING must be this gentle (footing-scale slope, rise/run) — the
/// immediate spot the deck grounds on. Measured at footing scale (not cell
/// scale) because a bank cell is locally steep toward the channel yet has a
/// flat footing on top.
const BRIDGE_MAX_FOOTING_SLOPE: f32 = 0.25;

/// Terrain a bridge may land on: gentle, walkable ground — never a mountain,
/// cliff, snowfield, glacier or volcanic slope.
fn bridge_walkable(t: Terrain) -> bool {
    matches!(
        t,
        Terrain::Plains | Terrain::Forest | Terrain::Savanna | Terrain::Tundra
            | Terrain::Desert | Terrain::Jungle | Terrain::Swamp
    )
}

/// Walk straight across a water band from walkable ground `land`, entering the
/// band at `first`, following the initial heading cell-to-cell until walkable
/// ground is reached on the FAR side. `band` says which kinds are the crossing
/// (river+its banks, or lake+its shores). Returns the far-side walkable cell,
/// or None if the band doesn't end in walkable ground within `max` steps (e.g.
/// it runs into a mountain, or the band is too wide).
fn cross_band(
    grid: &Grid,
    cells: &[Terrain],
    land: usize,
    first: usize,
    max: usize,
    band: impl Fn(Terrain) -> bool,
    is_end: impl Fn(Terrain) -> bool,
) -> Option<usize> {
    let tangent = |from: Vec3, step: Vec3| {
        let s = step - from * step.dot(from);
        s.normalize_or_zero()
    };
    let heading = tangent(grid.verts[land], grid.verts[first] - grid.verts[land]);
    if heading == Vec3::ZERO {
        return None;
    }
    let (mut prev, mut cur) = (land, first);
    for _ in 0..max {
        if is_end(cells[cur]) {
            return Some(cur);
        }
        // Only cross the intended band; anything else (open ocean, a mountain
        // foot) aborts — no bridge there.
        if !band(cells[cur]) {
            return None;
        }
        let cpos = grid.verts[cur];
        let mut best = None;
        let mut best_dot = -2.0;
        for &nb in &grid.vert_adj[cur] {
            let nb = nb as usize;
            if nb == prev {
                continue;
            }
            let d = tangent(cpos, grid.verts[nb] - cpos).dot(heading);
            if d > best_dot {
                best_dot = d;
                best = Some(nb);
            }
        }
        prev = cur;
        cur = best?;
    }
    None
}

/// Bridges cross WATER LOCALLY:/// Bridges cross WATER LOCALLY: over a river from one walkable bank to the
/// other, and over a lake to reach an island within it. Never oceans, never
/// mountains — a short footbridge on gentle ground.
fn build_bridges(
    grid: &Grid,
    terrain: &TerrainGen,
    cells: &[Terrain],
    slope_class: &[u8],
    _face_types: &[Terrain],
    _face_region: &[u32],
    painted: &mut Painted,
) -> Vec<Vec<SpherePos>> {
    let mut spans: Vec<Vec<SpherePos>> = Vec::new();
    let mut mids: Vec<SpherePos> = Vec::new();
    // A deck grounds cleanly only on gentle ground: reject an endpoint whose
    // proposed elevation differs sharply from its land neighbors (a steep bank
    // shoulder the entry pad couldn't flatten). Proposed field is set by now.
    // One walkable cell further from `away`, so the deck grounds inland of the
    // steep bank edge (the pad there sits on flat ground, clear of the carved
    // channel). Falls back to the cell itself if no inland walkable neighbor.
    let inland1 = |vi: usize, away: Vec3| {
        grid.vert_adj[vi].iter()
            .map(|&nb| nb as usize)
            .filter(|&nb| bridge_walkable(cells[nb]))
            .max_by(|&a, &b| {
                grid.verts[a].distance(away).partial_cmp(&grid.verts[b].distance(away)).unwrap()
            })
            .unwrap_or(vi)
    };
    let inland = inland1;
    // A deck grounds cleanly only on an open, gentle patch: the anchor cell
    // and all its neighbors must be walkable land (no sea, shore band, cliff,
    // mountain, snow, glacier or volcanic rock anywhere in the ring) with a
    // small proposed-elevation spread. This keeps bridges inland on flat
    // ground and away from mouths, coasts and steep shoulders.
    // Forbidden around an anchor: steep ground and the open sea/coast. The
    // crossing band itself (river/lake and their shores) is fine — that is what
    // the bridge spans.
    // Forbidden around an anchor: mountains and the open sea/beach. Cliffs are
    // now ALLOWED — a bridge may start on a clifftop (canyon rim), as long as
    // the footing itself is gentle enough (checked below).
    let forbidden = |t: Terrain| matches!(
        t,
        Terrain::Mountain | Terrain::Snow | Terrain::Glacier
            | Terrain::Volcanic | Terrain::Ocean | Terrain::Beach
    );
    // The field is SOLVED by now, so gate on the REAL slope: a deck anchor
    // must be gentle ground (a mountain "pass" qualifies, a steep forested
    // slope does not — whatever the tile is labelled), with no marine/steep
    // tile in its ring.
    // Footing gentle (the spot the deck grounds on — a bank top can be flat
    // even though the cell is steep toward the channel) and no marine/steep
    // tile in the ring. slope_class is unused here (bridges use footing scale).
    let _ = slope_class;
    let good_anchor = |vi: usize| {
        bridge_walkable(cells[vi])
            && terrain.slope(grid.vert_pos(vi)) < BRIDGE_MAX_FOOTING_SLOPE
            && grid.vert_adj[vi].iter().all(|&nb| !forbidden(cells[nb as usize]))
    };

    // Commit a bridge between two bank cells if it clears the spacing rule.
    let mut commit = |a: usize, b: usize, max_span: f32, spans: &mut Vec<Vec<SpherePos>>,
                      mids: &mut Vec<SpherePos>, painted: &mut Painted| -> bool {
        let (pa, pb) = (grid.vert_pos(a), grid.vert_pos(b));
        let d = pa.distance(pb);
        if d < 1.0 || d > max_span {
            return false;
        }
        let mid = SpherePos::new((pa.0 + pb.0).normalize());
        if mids.iter().any(|m| m.distance(mid) < BRIDGE_MIN_SPACING) {
            return false;
        }
        let ext = BRIDGE_ENTRY_OVERLAP / d;
        let steps = (d * (1.0 + 2.0 * ext) / 6.0).ceil().max(2.0) as usize;
        let span: Vec<SpherePos> = (0..=steps)
            .map(|k| crate::sphere::slerp(pa, pb, -ext + (1.0 + 2.0 * ext) * k as f32 / steps as f32))
            .collect();
        for vi in vert_chain(grid, &span) {
            painted.bridges.insert(vi);
        }
        for end in [span.first(), span.last()] {
            let Some(fi) = end.and_then(|p| grid.planet.face_at(p.0)) else { continue };
            for &vi in &grid.face_verts[fi] {
                let vi = vi as usize;
                if cells[vi].is_land() {
                    painted.bridge_entries.insert(vi);
                    for &nb in &grid.vert_adj[vi] {
                        if cells[nb as usize].is_land() {
                            painted.bridge_entries.insert(nb as usize);
                        }
                    }
                }
            }
        }
        mids.push(mid);
        spans.push(span);
        true
    };

    // (1) River crossings: a walkable cell beside a river bank, straight
    // across the band (bank → channel → bank) to walkable ground on the far
    // side. The endpoints are the walkable ground next to the banks.
    let river_band = |t: Terrain| matches!(t, Terrain::River | Terrain::RiverBank);
    for l1 in 0..grid.nv {
        if spans.len() >= BRIDGE_MAX_COUNT {
            break;
        }
        if !bridge_walkable(cells[l1]) {
            continue;
        }
        let Some(&entry) = grid.vert_adj[l1].iter()
            .find(|&&nb| cells[nb as usize] == Terrain::RiverBank)
        else {
            continue;
        };
        if let Some(l2) = cross_band(grid, cells, l1, entry as usize, 9, river_band, bridge_walkable) {
            if l2 == l1 {
                continue;
            }
            let mid = (grid.verts[l1] + grid.verts[l2]) * 0.5;
            let (g1, g2) = (inland(l1, mid), inland(l2, mid));
            if good_anchor(g1) && good_anchor(g2) {
                commit(g1, g2, BRIDGE_MAX_SPAN, &mut spans, &mut mids, painted);
            }
        }
    }

    // (1b) Swamp crossings (boardwalks): a walkable cell beside a swamp, across
    // the swamp band to dry walkable ground on the far side — same rules, with
    // the swamp itself as the crossable band.
    let swamp_band = |t: Terrain| t == Terrain::Swamp;
    let dry_end = |t: Terrain| bridge_walkable(t) && t != Terrain::Swamp;
    for l1 in 0..grid.nv {
        if spans.len() >= BRIDGE_MAX_COUNT {
            break;
        }
        if !dry_end(cells[l1]) {
            continue;
        }
        let Some(&entry) = grid.vert_adj[l1].iter()
            .find(|&&nb| cells[nb as usize] == Terrain::Swamp)
        else {
            continue;
        };
        if let Some(l2) = cross_band(grid, cells, l1, entry as usize, 9, swamp_band, dry_end) {
            if l2 == l1 {
                continue;
            }
            let mid = (grid.verts[l1] + grid.verts[l2]) * 0.5;
            let (g1, g2) = (inland(l1, mid), inland(l2, mid));
            if good_anchor(g1) && good_anchor(g2) {
                commit(g1, g2, BRIDGE_MAX_SPAN, &mut spans, &mut mids, painted);
            }
        }
    }


    // (2) Lake islands: a land component ringed only by lake water, small
    // enough to be an island, bridged to the nearest mainland lake shore.
    let mut comp = vec![u32::MAX; grid.nv];
    let mut sizes: Vec<usize> = Vec::new();
    for start in 0..grid.nv {
        if !cells[start].is_land() || comp[start] != u32::MAX {
            continue;
        }
        let id = sizes.len() as u32;
        let mut n = 0usize;
        let mut q = VecDeque::from([start]);
        comp[start] = id;
        while let Some(cur) = q.pop_front() {
            n += 1;
            for &nb in &grid.vert_adj[cur] {
                let nb = nb as usize;
                if cells[nb].is_land() && comp[nb] == u32::MAX {
                    comp[nb] = id;
                    q.push_back(nb);
                }
            }
        }
        sizes.push(n);
    }
    // Per-component water adjacency: is every water cell it touches Lake?
    // Ocean? (an island ringed by exactly one body is bridgeable to the
    // mainland). Rivers touching don't disqualify — they cross separately.
    let n_comp = sizes.len();
    let mut touch_lake = vec![false; n_comp];
    let mut touch_ocean = vec![false; n_comp];
    let mut only_lake = vec![true; n_comp];
    let mut only_ocean = vec![true; n_comp];
    for vi in 0..grid.nv {
        let Some(&id) = (cells[vi].is_land()).then(|| &comp[vi]) else { continue };
        let id = id as usize;
        for &nb in &grid.vert_adj[vi] {
            match cells[nb as usize] {
                Terrain::Lake => { touch_lake[id] = true; only_ocean[id] = false; }
                Terrain::Ocean => { touch_ocean[id] = true; only_lake[id] = false; }
                Terrain::River => {}
                t if t.is_water() => { only_lake[id] = false; only_ocean[id] = false; }
                _ => {}
            }
        }
    }
    // (2) Islands: a small land component ringed by a single body is bridged to
    // the nearest OTHER landmass across it — lakes (short), then ocean islands
    // to the nearest continent (longer gap, islands are seeded near land).
    let island_kinds: [(Terrain, usize, f32); 2] = [
        (Terrain::LakeShore, LAKE_ISLAND_MAX_CELLS, BRIDGE_MAX_SPAN),
        (Terrain::Beach, OCEAN_ISLAND_MAX_CELLS, OCEAN_BRIDGE_MAX_SPAN),
    ];
    for (which, (shore_kind, max_cells, max_span)) in island_kinds.into_iter().enumerate() {
        let ringed = |id: usize| if which == 0 {
            touch_lake[id] && only_lake[id]
        } else {
            touch_ocean[id] && only_ocean[id]
        };
        for id in 0..n_comp {
            if spans.len() >= BRIDGE_MAX_COUNT {
                break;
            }
            if !(ringed(id) && sizes[id] <= max_cells) {
                continue;
            }
            let shore = |vi: usize| bridge_walkable(cells[vi])
                && grid.vert_adj[vi].iter().any(|&nb| cells[nb as usize] == shore_kind);
            let island_shore: Vec<usize> = (0..grid.nv)
                .filter(|&vi| comp[vi] == id as u32 && shore(vi))
                .collect();
            let mut best: Option<(f32, usize, usize)> = None;
            for &a in &island_shore {
                let pa = grid.vert_pos(a);
                for vi in 0..grid.nv {
                    if comp[vi] == id as u32 || comp[vi] == u32::MAX || !shore(vi) {
                        continue;
                    }
                    let dd = pa.distance(grid.vert_pos(vi));
                    if best.is_none_or(|(bd, _, _)| dd < bd) {
                        best = Some((dd, a, vi));
                    }
                }
            }
            if let Some((_, a, b)) = best {
                let mid = (grid.verts[a] + grid.verts[b]) * 0.5;
                let (g1, g2) = (inland(a, mid), inland(b, mid));
                if good_anchor(g1) && good_anchor(g2) {
                    commit(g1, g2, max_span, &mut spans, &mut mids, painted);
                }
            }
        }
    }

    link_feature_pinches(grid, &mut painted.bridge_entries, |vi| cells[vi].is_land());
    spans
}

fn resolve_transitions(grid: &Grid, terrain: &TerrainGen, base: &[Terrain]) -> Vec<Terrain> {
    // Shorelines are deterministic bands, not WFC cells: every land cell
    // touching water gets its shore tile, so the waterline is never zigzagged
    // by chance. The band widens onto the second ring where the coast is flat,
    // and a land cell wedged between two shore cells joins the band.
    let mut out = base.to_vec();
    // Ocean is one identity now; DEPTH is a per-cell class derived from the
    // solved field afterwards (shallow near shore → abyss offshore, via the
    // shelf constraint). Deep water never surfaces because Ocean's range floor
    // deepens with distance from land — no separate tile, no margin pass.
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
    // Every transition band is TWO chains of cells one edge apart (never a
    // single chain — its faces would only touch at vertices): the waterline
    // chain plus the chain right behind it, for beaches, cliffs, lake shores
    // and river banks alike. Wide types (oceans, lakes, rivers, towns) are
    // free-width; bands are not.
    for vi in 0..grid.nv {
        if base[vi].is_water() {
            continue;
        }
        if water_dist[vi] <= 2 {
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
    smooth_coast_band(grid, &mut resolved);
    absorb_small_patches(grid, &mut resolved);
    prune_orphan_bands(grid, &mut resolved);
    link_tile_pinches(grid, &mut resolved);
    resolved
}

/// A transition band without its water is not a transition: a river bank
/// needs a River within band reach (2 cells), a lake shore a Lake, a beach
/// or cliff the sea. Orphans (left behind when fills/trims move the water)
/// join their most common plain land neighbor.
fn prune_orphan_bands(grid: &Grid, cells: &mut [Terrain]) {
    let dist_to = |pred: &dyn Fn(Terrain) -> bool| -> Vec<u8> {
        let mut dist = vec![u8::MAX; grid.nv];
        let mut q: VecDeque<usize> = VecDeque::new();
        for vi in 0..grid.nv {
            if pred(cells[vi]) {
                dist[vi] = 0;
                q.push_back(vi);
            }
        }
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
        dist
    };
    let river = dist_to(&|t| t == Terrain::River);
    let lake = dist_to(&|t| t == Terrain::Lake);
    let sea = dist_to(&|t| t == Terrain::Ocean);
    for vi in 0..grid.nv {
        let orphan = match cells[vi] {
            Terrain::RiverBank => river[vi] > 2,
            Terrain::LakeShore => lake[vi] > 2,
            Terrain::Beach | Terrain::Cliff => sea[vi] > 2 && lake[vi] > 2 && river[vi] > 2,
            _ => false,
        };
        if !orphan {
            continue;
        }
        let mut counts: BTreeMap<u8, usize> = BTreeMap::new();
        for &nb in &grid.vert_adj[vi] {
            let t = cells[nb as usize];
            if matches!(
                t,
                Terrain::Desert | Terrain::Plains | Terrain::Forest
                    | Terrain::Tundra | Terrain::Mountain | Terrain::Snow
                    | Terrain::Swamp | Terrain::Jungle | Terrain::Savanna
                    | Terrain::Volcanic | Terrain::Glacier
            ) {
                *counts.entry(t as u8).or_default() += 1;
            }
        }
        cells[vi] = counts.iter().max_by_key(|(_, c)| **c)
            .map(|(&k, _)| Terrain::ALL[k as usize])
            .unwrap_or(Terrain::Plains);
    }
}

/// Inland biome boundaries get a blend mark: the face keeps its derived
/// kind, but carries the pair it links so rendering/solving can transition
/// between the two. With cell-based tiles the boundary faces are simply the
/// faces whose corner cells disagree — edge-connected strips by construction.
/// Faces flanking a built feature blend toward it instead (feature codes).
fn mark_blends(grid: &Grid, cells: &[Terrain], tiles: &[Terrain], painted: &Painted) -> Vec<(u32, u8, u8)> {
    let plain = |t: Terrain| t.is_land();
    let overlay = |fi: usize| {
        face_solid(grid, &painted.roads, fi)
            || face_solid(grid, &painted.towns, fi)
            || face_solid(grid, &painted.bridge_entries, fi)
    };
    let mut out = Vec::new();
    for fi in 0..grid.n {
        if !plain(tiles[fi]) || overlay(fi) {
            continue;
        }
        // Feature flanks (a painted corner without ownership) blend toward the
        // feature; most specific wins (entry pad < town blob < road network).
        let feature = if painted_corners(grid, &painted.bridge_entries, fi) > 0 {
            Some(crate::level::BLEND_BRIDGE_ENTRY)
        } else if painted_corners(grid, &painted.towns, fi) > 0 {
            Some(crate::level::BLEND_TOWN)
        } else if painted_corners(grid, &painted.roads, fi) > 0 {
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

/// A biome patch smaller than this many cells is speckle, not a region —
/// threshold classifiers (snow by temperature, mountains by elevation)
/// salt-and-pepper at their contour lines without this.
/// Generic min-region-size for every plain land biome: undersized clusters
/// join their most common neighboring land kind. Transition bands (shore
/// kinds) are thin by design and exempt; water minimums live in
/// normalize_water_bodies.
/// Collect the edge-connected component of cells reachable from `start` for
/// which `member` holds, marking `visited`. The one BFS behind water bodies,
/// lakes and speckle clusters.
fn flood_cells(grid: &Grid, start: usize, visited: &mut [bool], member: impl Fn(usize) -> bool) -> Vec<usize> {
    let mut out = vec![start];
    let mut q = VecDeque::from([start]);
    visited[start] = true;
    while let Some(cur) = q.pop_front() {
        for &nb in &grid.vert_adj[cur] {
            let nb = nb as usize;
            if !visited[nb] && member(nb) {
                visited[nb] = true;
                out.push(nb);
                q.push_back(nb);
            }
        }
    }
    out
}

/// A contiguous cluster of equal values below its minimum size is speckle: it
/// is absorbed into its most common eligible neighbor value. Generic over the
/// value type (biome cover, landform, …). `eligible` selects which values
/// participate, `min_size` gives each value's floor, `absorbable` says which
/// neighbor values a speckle may merge into.
fn absorb_small_clusters<T: Copy + Ord>(
    grid: &Grid,
    out: &mut [T],
    eligible: impl Fn(T) -> bool,
    min_size: impl Fn(T) -> usize,
    absorbable: impl Fn(T) -> bool,
) {
    let mut visited = vec![false; grid.nv];
    for start in 0..grid.nv {
        if !eligible(out[start]) || visited[start] {
            continue;
        }
        let kind = out[start];
        let cluster = flood_cells(grid, start, &mut visited, |nb| out[nb] == kind);
        if cluster.len() >= min_size(kind) {
            continue;
        }
        let mut counts: BTreeMap<T, usize> = BTreeMap::new();
        for &vi in &cluster {
            for &nb in &grid.vert_adj[vi] {
                let t = out[nb as usize];
                if t != kind && absorbable(t) {
                    *counts.entry(t).or_default() += 1;
                }
            }
        }
        if let Some((&k, _)) = counts.iter().max_by_key(|(_, c)| **c) {
            for vi in cluster {
                out[vi] = k;
            }
        }
    }
}

fn absorb_small_patches(grid: &Grid, out: &mut [Terrain]) {
    let plain = |t: Terrain| matches!(
        t,
        Terrain::Desert | Terrain::Plains | Terrain::Forest
            | Terrain::Tundra | Terrain::Mountain | Terrain::Snow
            | Terrain::Swamp | Terrain::Jungle | Terrain::Savanna
            | Terrain::Volcanic | Terrain::Glacier
    );
    absorb_small_clusters(grid, out, plain, |t| size_range(t).0, |t| t.is_land());
}

/// The coast band is PROACTIVE: Beach vs Cliff was already decided by the
/// proposed elevation field (high ground meeting water is a cliff, low ground
/// a beach — the ground is raised first, the label follows). This pass only
/// smooths single-cell islands in the band: a lone beach cell between two
/// cliffs joins them, and vice versa.
fn smooth_coast_band(grid: &Grid, out: &mut [Terrain]) {
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
fn region_class(grid: &Grid, face_types: &[Terrain], painted: Option<&Painted>, fi: usize) -> Option<RegionKind> {
    if let Some(p) = painted {
        if face_solid(grid, &p.towns, fi) {
            return Some(RegionKind::Town);
        }
        if face_solid(grid, &p.roads, fi) {
            return Some(RegionKind::Road);
        }
    }
    match face_types[fi] {
        Terrain::Ocean => Some(RegionKind::Ocean),
        Terrain::Lake => Some(RegionKind::Lake),
        Terrain::River => Some(RegionKind::River),
        Terrain::Beach => Some(RegionKind::Beach),
        Terrain::Cliff => Some(RegionKind::Cliff),
        Terrain::Forest => Some(RegionKind::Forest),
        Terrain::Desert => Some(RegionKind::Desert),
        Terrain::Mountain | Terrain::Snow => Some(RegionKind::Mountain),
        Terrain::Plains => Some(RegionKind::Plains),
        Terrain::Tundra => Some(RegionKind::Tundra),
        Terrain::Swamp => Some(RegionKind::Swamp),
        Terrain::Jungle => Some(RegionKind::Jungle),
        Terrain::Savanna => Some(RegionKind::Savanna),
        Terrain::Volcanic => Some(RegionKind::Volcano),
        Terrain::Glacier => Some(RegionKind::Glacier),
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
        (0..grid.n).map(|fi| region_class(grid, face_types, Some(painted), fi)).collect();
    // Terrain-derived class ignoring the road/town overlay: a road slicing
    // through a desert must not split it into two regions, so terrain clusters
    // may flow THROUGH overlay faces whose underlying terrain matches (without
    // claiming them — those faces belong to their Road/Town region).
    let terrain_class: Vec<Option<RegionKind>> = (0..grid.n)
        .map(|fi| region_class(grid, face_types, None, fi))
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
        // Long coastlines split into multiple named regions while naming —
        // the tiles themselves are never retyped for naming's sake.
        let max_faces = match kind {
            RegionKind::Beach => size_range(Terrain::Beach).1 * 2,
            RegionKind::Cliff => size_range(Terrain::Cliff).1 * 2,
            _ => usize::MAX,
        };
        let mut faces = vec![start];
        let mut q = VecDeque::from([start]);
        let mut visited_connector = BitSet::new(grid.n);
        face_region[start] = re;
        while let Some(cur) = q.pop_front() {
            if faces.len() >= max_faces {
                break;
            }
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
            RegionKind::Forest => size_range(Terrain::Forest).0 * 2,
            RegionKind::Beach => size_range(Terrain::Beach).0 * 2,
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
    const SWAMP: [&str; 6] = ["Murk", "Fen", "Bog", "Mire", "Black", "Sunken"];
    const JUNGLE: [&str; 6] = ["Verdant", "Emerald", "Tangle", "Vine", "Fever", "Green"];
    const SAVANNA: [&str; 6] = ["Amber", "Sun", "Dust", "Lion", "Wide", "Gold"];
    const VOLCANO: [&str; 6] = ["Ash", "Ember", "Cinder", "Smoke", "Molten", "Black"];
    const GLACIER: [&str; 6] = ["Frost", "White", "Blue", "Silent", "Everice", "North"];

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
        RegionKind::Swamp => pick(&SWAMP, &["Swamp", "Marsh", "Fen"]),
        RegionKind::Jungle => pick(&JUNGLE, &["Jungle", "Rainforest"]),
        RegionKind::Savanna => pick(&SAVANNA, &["Savanna", "Plains"]),
        RegionKind::Volcano => pick(&VOLCANO, &["Peaks", "Fields", "Wastes"]),
        RegionKind::Glacier => pick(&GLACIER, &["Glacier", "Ice", "Wastes"]),
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

/// Road surface material from the ground a road face crosses: sand on desert
/// and beach, rock in the mountains and on steep ground, dirt on soil, gravel
/// otherwise. Read from the underlying cover/landform/slope at the corners
/// (roads are an overlay — the cells still hold the terrain beneath).
/// Reduce a per-cell u8 to per-face by taking the max over the face's corner
/// cells (used for slope/depth: a face is as steep/deep as its worst corner).
pub fn face_max(grid: &Grid, per_cell: &[u8]) -> Vec<u8> {
    (0..grid.n)
        .map(|fi| grid.face_verts[fi].iter().map(|&vi| per_cell[vi as usize]).max().unwrap_or(0))
        .collect()
}

/// Reduce a per-cell u8 to per-face by majority corner (used for landform: the
/// massif a face sits in).
pub fn face_majority(grid: &Grid, per_cell: &[u8]) -> Vec<u8> {
    (0..grid.n)
        .map(|fi| {
            let mut c: BTreeMap<u8, usize> = BTreeMap::new();
            for &vi in &grid.face_verts[fi] {
                *c.entry(per_cell[vi as usize]).or_default() += 1;
            }
            c.into_iter().max_by_key(|(_, n)| *n).map(|(k, _)| k).unwrap_or(0)
        })
        .collect()
}

pub fn face_road_material(
    grid: &Grid,
    cells: &[Terrain],
    landform: &[u8],
    slope_class: &[u8],
    fi: usize,
) -> u8 {
    let mut sand = false;
    let mut rock = false;
    let mut soil = false;
    for &vi in &grid.face_verts[fi] {
        let vi = vi as usize;
        match cells[vi] {
            Terrain::Desert | Terrain::Beach | Terrain::Savanna => sand = true,
            Terrain::Mountain | Terrain::Volcanic | Terrain::Cliff => rock = true,
            Terrain::Forest | Terrain::Plains | Terrain::Swamp
                | Terrain::Jungle | Terrain::Tundra => soil = true,
            _ => {}
        }
        if matches!(landform[vi], LANDFORM_MOUNTAINS | LANDFORM_PLATEAU)
            || slope_class[vi] >= SLOPE_STEEP
        {
            rock = true;
        }
    }
    // Rock wins on hard/steep ground, then sand, then dirt, else gravel.
    if rock { ROAD_MAT_ROCK }
    else if sand { ROAD_MAT_SAND }
    else if soil { ROAD_MAT_DIRT }
    else { ROAD_MAT_GRAVEL }
}

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
    water_depth: &[u8],
    landform: &[u8],
    slope_class: &[u8],
) -> (Vec<[[f32; 3]; 3]>, Vec<[[f32; 4]; 3]>) {
    let vert_r: Vec<f32> = grid.verts.iter()
        .map(|dir| terrain.render_radius(SpherePos::new(*dir)))
        .collect();

    let road_color = bevy::prelude::Color::srgb(0.5, 0.42, 0.3).to_linear().to_f32_array();
    let road_mat_color = |m: u8| -> [f32; 4] {
        match m {
            ROAD_MAT_DIRT => bevy::prelude::Color::srgb(0.45, 0.33, 0.22),
            ROAD_MAT_SAND => bevy::prelude::Color::srgb(0.78, 0.70, 0.50),
            ROAD_MAT_ROCK => bevy::prelude::Color::srgb(0.40, 0.38, 0.36),
            _ => bevy::prelude::Color::srgb(0.52, 0.50, 0.47), // gravel
        }.to_linear().to_f32_array()
    };
    let town_color = crate::theme::WARNING.to_linear().to_f32_array();
    let entry_color = bevy::prelude::Color::srgb(0.42, 0.33, 0.24).to_linear().to_f32_array();
    let mut tris = Vec::with_capacity(grid.n);
    let mut cols = Vec::with_capacity(grid.n);
    for (fi, idx) in grid.face_verts.iter().enumerate() {
        // Features are built structures: a face the feature OWNS (≥2 painted
        // corners — the two-triangle quads along the painted cell chain)
        // renders solid with a hard edge. Faces with exactly one painted
        // corner are the flank band and fade via the corner gradient. Total:
        // blend band / solid strip / blend band, for every feature.
        let corner = |k: usize| {
            let vi = idx[k] as usize;
            if painted.bridge_entries.contains(vi) {
                entry_color
            } else if painted.towns.contains(vi) {
                town_color
            } else if painted.roads.contains(vi) {
                road_color
            } else {
                let mut c = cells[vi].color().to_linear().to_f32_array();
                if cells[vi].is_water() {
                    // Water darkens with depth (shallow shore → dark abyss).
                    let f = match water_depth[vi] {
                        DEPTH_SHALLOW => 1.0,
                        DEPTH_DEEP => 0.62,
                        _ => 0.35,
                    };
                    for ch in c.iter_mut().take(3) {
                        *ch *= f;
                    }
                } else {
                    // Land: the SHAPE reads through the cover. Higher landforms
                    // darken (ruggedness), and a steep/cliff cell bleeds toward
                    // bare rock — so a forested hill, a forested mountain and a
                    // cliff face all look distinct even under the same biome.
                    let shade = match landform[vi] {
                        LANDFORM_MOUNTAINS => 0.82,
                        LANDFORM_PLATEAU => 0.90,
                        LANDFORM_HILLS => 0.96,
                        _ => 1.0,
                    };
                    for ch in c.iter_mut().take(3) {
                        *ch *= shade;
                    }
                    if slope_class[vi] >= SLOPE_STEEP {
                        let rock = [0.24, 0.21, 0.19];
                        let k = if slope_class[vi] == SLOPE_CLIFF { 0.6 } else { 0.3 };
                        for i in 0..3 {
                            c[i] = c[i] * (1.0 - k) + rock[i] * k;
                        }
                    }
                }
                c
            }
        };
        let color: [[f32; 4]; 3] = if face_solid(grid, &painted.bridge_entries, fi) {
            [entry_color; 3]
        } else if face_solid(grid, &painted.towns, fi) {
            [town_color; 3]
        } else if face_solid(grid, &painted.roads, fi) {
            [road_mat_color(face_road_material(grid, cells, landform, slope_class, fi)); 3]
        } else {
            // Boundary faces render ONE flat color — the equal-weight average
            // of the distinct corner colors (50/50 for a pair) — so band
            // bounds stay crisp instead of smearing into a gradient.
            let (c0, c1, c2) = (corner(0), corner(1), corner(2));
            if c0 == c1 && c1 == c2 {
                [c0; 3]
            } else {
                let mut distinct = vec![c0];
                for c in [c1, c2] {
                    if !distinct.contains(&c) {
                        distinct.push(c);
                    }
                }
                let k = distinct.len() as f32;
                let mut avg = [0.0f32; 4];
                for c in &distinct {
                    for i in 0..4 {
                        avg[i] += c[i] / k;
                    }
                }
                [avg; 3]
            }
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

/// Sub-tile decoration scatter. Flora are POINTS, not tiles: a tree consumes
/// part of a face, so it lives in its own layer over the finished mesh.
/// Densities (expected instances per face) come from the tile kind; features
/// keep their ground clear including flanks; positions are uniform barycentric
/// samples of the DISPLACED triangle so every prop sits exactly on the ground.
/// Deterministic: one seeded stream in face order.
const FLORA_RNG_SALT: u64 = 0x466c_6f72;

/// How a flora kind's density responds to ground moisture.
#[derive(Clone, Copy)]
enum FloraScale {
    /// Denser on wet ground (greenery).
    Wet,
    /// Wet, squared — meadows bloom sharply with moisture (flowers).
    WetSq,
    /// Denser on dry ground (rocks, cacti).
    Dry,
    /// Density independent of moisture (fallen logs, dead trees).
    Flat,
}

/// Base per-face density (expected instances) for each flora kind on a tile,
/// with its moisture response — the scatter analogue of `elev_range`. Faces
/// are ~9m across at sub=7, so values are small. Empty ⇒ nothing grows here.
fn flora_density(t: Terrain) -> Vec<(f32, FloraScale, u8)> {
    use FloraScale::*;
    // (base, scale, kind). Kept sparse: only the kinds that grow on this tile.
    let v: &[(f32, FloraScale, u8)] = match t {
        Terrain::Forest => &[
            (0.40, Wet, FLORA_TREE), (0.10, Wet, FLORA_BUSH), (0.012, WetSq, FLORA_FLOWER),
            (0.008, Dry, FLORA_ROCK), (0.075, Wet, FLORA_GRASS),
            (0.03, Flat, FLORA_LOG), (0.06, Wet, FLORA_MUSHROOM), (0.04, Wet, FLORA_BERRY),
            (0.01, Flat, FLORA_DEADTREE),
        ],
        Terrain::Jungle => &[
            (0.55, Wet, FLORA_TREE), (0.18, Wet, FLORA_BUSH), (0.02, WetSq, FLORA_FLOWER),
            (0.004, Dry, FLORA_ROCK), (0.10, Wet, FLORA_GRASS),
            (0.05, Flat, FLORA_LOG), (0.09, Wet, FLORA_MUSHROOM), (0.05, Wet, FLORA_BERRY),
            (0.02, Wet, FLORA_REED),
        ],
        Terrain::Swamp => &[
            (0.05, Wet, FLORA_TREE), (0.12, Wet, FLORA_BUSH), (0.03, WetSq, FLORA_FLOWER),
            (0.004, Dry, FLORA_ROCK), (0.10, Wet, FLORA_GRASS),
            (0.05, Flat, FLORA_LOG), (0.05, Wet, FLORA_MUSHROOM), (0.06, Flat, FLORA_DEADTREE),
            (0.18, Wet, FLORA_REED),
        ],
        Terrain::Plains => &[
            (0.01, Wet, FLORA_TREE), (0.025, Wet, FLORA_BUSH), (0.075, WetSq, FLORA_FLOWER),
            (0.005, Dry, FLORA_ROCK), (0.088, Wet, FLORA_GRASS),
            (0.004, Flat, FLORA_LOG), (0.02, Wet, FLORA_BERRY),
        ],
        Terrain::Savanna => &[
            (0.02, Wet, FLORA_TREE), (0.04, Wet, FLORA_BUSH), (0.04, WetSq, FLORA_FLOWER),
            (0.008, Dry, FLORA_ROCK), (0.11, Wet, FLORA_GRASS),
            (0.008, Flat, FLORA_LOG), (0.015, Dry, FLORA_CACTUS), (0.01, Wet, FLORA_BERRY),
            (0.02, Flat, FLORA_DEADTREE),
        ],
        Terrain::Tundra => &[
            (0.003, Wet, FLORA_TREE), (0.015, Wet, FLORA_BUSH), (0.005, WetSq, FLORA_FLOWER),
            (0.03, Dry, FLORA_ROCK), (0.012, Wet, FLORA_GRASS),
            (0.01, Flat, FLORA_LOG), (0.008, Wet, FLORA_BERRY), (0.03, Flat, FLORA_DEADTREE),
        ],
        Terrain::Desert => &[
            (0.012, Wet, FLORA_BUSH), (0.025, Dry, FLORA_ROCK),
            (0.06, Dry, FLORA_CACTUS), (0.02, Flat, FLORA_DEADTREE),
        ],
        Terrain::RiverBank | Terrain::LakeShore => &[
            (0.02, Wet, FLORA_TREE), (0.05, Wet, FLORA_BUSH), (0.062, WetSq, FLORA_FLOWER),
            (0.008, Dry, FLORA_ROCK), (0.075, Wet, FLORA_GRASS),
            (0.01, Flat, FLORA_LOG), (0.12, Wet, FLORA_REED),
        ],
        Terrain::Mountain => &[(0.005, Wet, FLORA_BUSH), (0.05, Dry, FLORA_ROCK)],
        Terrain::Cliff => &[(0.038, Dry, FLORA_ROCK)],
        Terrain::Beach => &[(0.01, Dry, FLORA_ROCK)],
        Terrain::Volcanic => &[(0.06, Dry, FLORA_ROCK)],
        Terrain::Glacier => &[(0.01, Dry, FLORA_ROCK)],
        _ => &[],
    };
    v.to_vec()
}

fn place_flora(
    grid: &Grid,
    terrain: &TerrainGen,
    tiles: &[Terrain],
    painted: &Painted,
    mesh_tris: &[[[f32; 3]; 3]],
) -> Vec<FloraData> {
    let mut rng = fastrand::Rng::with_seed(grid.seed as u64 ^ FLORA_RNG_SALT);
    let mut out = Vec::new();
    for fi in 0..grid.n {
        let clear = painted_corners(grid, &painted.roads, fi) > 0
            || painted_corners(grid, &painted.towns, fi) > 0
            || painted_corners(grid, &painted.bridge_entries, fi) > 0
            || painted_corners(grid, &painted.bridges, fi) > 0;
        if clear {
            continue;
        }
        let mix = flora_density(tiles[fi]);
        if mix.is_empty() {
            continue;
        }
        // Moisture in roughly [-1, 1]: scale greens up on wet ground, rocks
        // up on dry ground. One sample per face keeps it cheap.
        let m = terrain.moisture_at(grid.centroid(fi));
        let wet = (1.0 + m).clamp(0.3, 1.8);
        let dry = (1.0 - m).clamp(0.5, 1.6);
        for &(base, scale, kind) in &mix {
            let density = base * match scale {
                FloraScale::Wet => wet,
                FloraScale::WetSq => wet * wet,
                FloraScale::Dry => dry,
                FloraScale::Flat => 1.0,
            };
            let mut n = density.trunc() as u32;
            if rng.f32() < density.fract() {
                n += 1;
            }
            for _ in 0..n {
                let (mut u, mut v) = (rng.f32(), rng.f32());
                if u + v > 1.0 {
                    u = 1.0 - u;
                    v = 1.0 - v;
                }
                let t = &mesh_tris[fi];
                let (a, b, c) = (
                    Vec3::from_array(t[0]),
                    Vec3::from_array(t[1]),
                    Vec3::from_array(t[2]),
                );
                let pos = a + (b - a) * u + (c - a) * v;
                out.push(FloraData { pos: pos.to_array(), face: fi as u32, kind });
            }
        }
    }
    out
}

const STRUCT_RNG_SALT: u64 = 0x5374_7563_7572_65;

/// Contextual structures, placed like towns/bridges: wells, campfires and
/// farms cluster in and around towns; walls ring town edges; docks reach out
/// from coastal town shores; watchtowers crown high ground near roads; ruins
/// scatter through the wilderness. Positions sit on the displaced mesh (face
/// centroids). Deterministic: one seeded stream in face order.
fn place_structures(
    grid: &Grid,
    terrain: &TerrainGen,
    tiles: &[Terrain],
    painted: &Painted,
    slope_class: &[u8],
    mesh_tris: &[[[f32; 3]; 3]],
) -> Vec<StructureData> {
    let mut rng = fastrand::Rng::with_seed(grid.seed as u64 ^ STRUCT_RNG_SALT);
    let face_center = |fi: usize| {
        let t = &mesh_tris[fi];
        (Vec3::from_array(t[0]) + Vec3::from_array(t[1]) + Vec3::from_array(t[2])) / 3.0
    };
    let town = |fi: usize| face_solid(grid, &painted.towns, fi);
    let road = |fi: usize| face_solid(grid, &painted.roads, fi);
    let feature = |fi: usize| {
        town(fi) || road(fi)
            || painted_corners(grid, &painted.bridges, fi) > 0
            || painted_corners(grid, &painted.bridge_entries, fi) > 0
    };
    // Face-step distance from any town (capped) — cheap context for the rest.
    let mut town_dist = vec![u16::MAX; grid.n];
    let mut q: VecDeque<usize> = VecDeque::new();
    for fi in 0..grid.n {
        if town(fi) {
            town_dist[fi] = 0;
            q.push_back(fi);
        }
    }
    while let Some(cur) = q.pop_front() {
        if town_dist[cur] >= 6 {
            continue;
        }
        for &nb in &grid.adj[cur] {
            let nb = nb as usize;
            if town_dist[nb] == u16::MAX {
                town_dist[nb] = town_dist[cur] + 1;
                q.push_back(nb);
            }
        }
    }
    let road_near = |fi: usize| grid.adj[fi].iter().any(|&nb| road(nb as usize)) || road(fi);

    let mut out = Vec::new();
    let mut push = |rng: &mut fastrand::Rng, fi: usize, kind: u8| {
        out.push(StructureData {
            pos: face_center(fi).to_array(),
            face: fi as u32,
            kind,
            yaw: rng.f32() * std::f32::consts::TAU,
        });
    };
    // A structure needs buildable ground: skip any face with a steep/cliff
    // corner (watchtowers on a ridge are the exception — handled below).
    let buildable = |fi: usize| grid.face_verts[fi].iter()
        .all(|&vi| slope_walkable(slope_class[vi as usize]));
    for fi in 0..grid.n {
        if tiles[fi].is_water() || !buildable(fi) {
            continue;
        }
        // Town interior: a well or a campfire in a clearing.
        if town(fi) {
            let r = rng.f32();
            if r < 0.010 {
                push(&mut rng, fi, STRUCT_WELL);
            } else if r < 0.045 {
                push(&mut rng, fi, STRUCT_CAMPFIRE);
            }
            continue;
        }
        // Town edge (non-town land beside a town): a wall segment or a farm.
        let touches_town = grid.adj[fi].iter().any(|&nb| town(nb as usize));
        if touches_town {
            let coastal = grid.adj[fi].iter().any(|&nb| tiles[nb as usize].is_water());
            if coastal && rng.f32() < 0.5 {
                push(&mut rng, fi, STRUCT_DOCK);
            } else if rng.f32() < 0.4 {
                push(&mut rng, fi, STRUCT_WALL);
            }
            continue;
        }
        if feature(fi) {
            continue;
        }
        // Farmland: fertile flat ground just outside town.
        if town_dist[fi] <= 3
            && matches!(tiles[fi], Terrain::Plains | Terrain::Savanna | Terrain::Forest)
            && rng.f32() < 0.10
        {
            push(&mut rng, fi, STRUCT_FARM);
            continue;
        }
        // Watchtower: high ground overlooking a road.
        if road_near(fi)
            && terrain.elevation_at(grid.centroid(fi)) > 0.25
            && rng.f32() < 0.03
        {
            push(&mut rng, fi, STRUCT_WATCHTOWER);
            continue;
        }
        // Ruins: rare, deep in the wilderness (far from any town).
        if town_dist[fi] == u16::MAX
            && !matches!(tiles[fi], Terrain::Beach | Terrain::Cliff | Terrain::LakeShore | Terrain::RiverBank)
            && rng.f32() < 0.0006
        {
            push(&mut rng, fi, STRUCT_RUIN);
        }
    }
    out
}

fn build_face_tags(grid: &Grid, painted: &Painted) -> (Vec<u32>, Vec<u8>) {
    let mut off = Vec::with_capacity(grid.n + 1);
    let mut data = Vec::new();
    off.push(0u32);
    for fi in 0..grid.n {
        if face_solid(grid, &painted.roads, fi) { data.push(TAG_ROAD); }
        if face_solid(grid, &painted.towns, fi) { data.push(TAG_TOWN); }
        if face_solid(grid, &painted.bridges, fi) { data.push(TAG_BRIDGE); }
        if face_solid(grid, &painted.bridge_entries, fi) { data.push(TAG_BRIDGE_ENTRY); }
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
/// How far below its neighbors' average a water bed vertex is pushed each
/// solver step — a concave basin/channel instead of a flat plate. `None` for
/// non-bed kinds. (Rivers cut sharper than lake basins.)
fn water_concavity(t: Terrain) -> Option<f32> {
    match t {
        Terrain::Lake => Some(0.005),
        Terrain::River => Some(0.020),
        _ => None,
    }
}

/// The water kinds a shore/bank vertex must sit strictly above (its adjacent
/// body). Empty for non-bank kinds.
fn bank_water(t: Terrain) -> &'static [Terrain] {
    match t {
        Terrain::RiverBank => &[Terrain::River],
        Terrain::LakeShore => &[Terrain::Lake],
        Terrain::Beach => &[Terrain::Ocean],
        _ => &[],
    }
}

/// A cover biome whose ELEVATION comes from its landform, not itself (a snowy
/// lowland stays low; snow doesn't imply a mountain). Water, shore bands and
/// rivers keep their own ranges — they aren't landforms.
fn is_cover(t: Terrain) -> bool {
    use Terrain::*;
    matches!(
        t,
        Desert | Plains | Forest | Tundra | Savanna | Swamp | Jungle
            | Mountain | Snow | Volcanic | Glacier
    )
}

/// Elevation range a LANDFORM's ground may occupy — the base layer that drives
/// height (cover only colors it). Bands overlap so adjacent landforms
/// (ordered lowland→hills→mountains) meet without an impossible jump.
fn landform_range(lf: u8) -> (f32, f32) {
    match lf {
        LANDFORM_VALLEY => (0.0, 0.16),
        LANDFORM_LOWLAND => (0.02, 0.20),
        LANDFORM_HILLS => (0.14, 0.45),
        LANDFORM_MOUNTAINS => (0.40, 1.0),
        LANDFORM_PLATEAU => (0.36, 0.74),
        _ => (0.02, 0.50),
    }
}

/// How steep a land edge may be, from the steeper of the two landforms:
/// lowlands are gentle, mountains steep, hills between.
fn landform_edge_cap(lfa: u8, lfb: u8) -> f32 {
    let one = |lf: u8| -> f32 { match lf {
        LANDFORM_VALLEY | LANDFORM_LOWLAND => 0.04,
        LANDFORM_HILLS => 0.14,
        LANDFORM_PLATEAU => 0.20,
        LANDFORM_MOUNTAINS => 0.40,
        _ => 0.10,
    } };
    one(lfa).max(one(lfb))
}

fn elev_range(t: Terrain) -> (f32, f32) {
    use Terrain::*;
    // Ranges of kinds that may sit next to each other must overlap (or lie
    // within one edge's gradient cap) or the constraint set is unsatisfiable.
    match t {
        // One ocean identity; the shelf constraint deepens the floor with
        // distance from land, so the range spans shore-shallows to abyss.
        Ocean => (-1.0, -0.01),
        // Lakes and rivers carry their OWN water level — a mountain lake may
        // sit high above the sea; only its shores must stay above it.
        Lake => (-0.25, 0.55),
        LakeShore => (0.0, 0.60),
        // Rivers descend from mountains to the sea; their range must span it.
        River => (-1.0, 0.60),
        RiverBank => (0.0, 0.65),
        Beach => (0.0, 0.05),
        // Cliff tiles are RAMPS, not plateaus: the toe verts sit at shore
        // level and the crest verts track the hinterland (see the cliff
        // tracking step in the solver), so the whole drop happens across the
        // cliff face. The range here is just the envelope.
        Cliff => (-0.02, 0.60),
        Desert | Plains | Forest | Tundra | Savanna => (0.02, 0.50),
        // Swamp is low, wet, near-flat ground just above the water line.
        Swamp => (0.0, 0.15),
        // Jungle covers lowland to hills.
        Jungle => (0.02, 0.55),
        Mountain => (0.45, 1.0),
        Snow => (0.50, 1.0),
        // Volcanic peaks and glaciers ride the high ground like Mountain/Snow.
        Volcanic => (0.45, 1.0),
        Glacier => (0.45, 1.0),
    }
}

/// Max elevation change across one vertex edge (~70m) between two tile kinds.
/// Small at shores (continental shelf), large into mountains and at cliffs.
fn max_gradient(a: Terrain, b: Terrain) -> f32 {
    use Terrain::*;
    let water = |t: Terrain| matches!(t, Ocean | Lake);
    let peak = |t: Terrain| matches!(t, Mountain | Snow | Volcanic | Glacier);
    // Rivers are canyons: their walls may be steep wherever they cut through.
    // Caps are per vertex edge (~35m at field sub=6).
    let base: f32 = if matches!(a, River | RiverBank) || matches!(b, River | RiverBank) {
        0.22
    } else if a == Cliff || b == Cliff {
        // The whole cliff drop can happen across one vertex edge (toe → crest).
        0.30
    } else if peak(a) || peak(b) {
        0.14
    } else if water(a) && water(b) {
        0.04
    } else if water(a) || water(b) || a == Beach || b == Beach {
        0.015
    } else {
        // Ordinary land: ~0.02 e per ~35m edge ≈ 15° — walkable country,
        // not ski slopes. Steepness is a property of mountains, cliffs and
        // canyons (their caps above), not of plains.
        0.02
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
const ROAD_EDGE_GRADIENT: f32 = 0.01;
const SOLVER_MAX_ITERS: usize = 250;
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

/// Solver verts are a bit-exact subset of the grid's cell vertices (each
/// subdivision keeps its parents), so tile ownership comes straight from the
/// cell labels — no geometric face lookup, no fallback kind.
/// Per-solver-vertex value looked up from the per-CELL array: solver verts are
/// a bit-exact subset of grid cells (each subdivision keeps its parents), so
/// the map is exact, with a nearest-corner fallback for the (unexpected) miss.
/// Backs both tile-kind ownership and landform ownership.
fn owner_of<T: Copy>(grid: &Grid, terrain: &TerrainGen, per_cell: &[T], default: T) -> Vec<T> {
    let index: BTreeMap<[u32; 3], u32> = grid.verts.iter().enumerate()
        .map(|(i, v)| ([v.x.to_bits(), v.y.to_bits(), v.z.to_bits()], i as u32))
        .collect();
    (0..terrain.vert_count())
        .map(|vi| {
            let d = terrain.vert_dir(vi);
            let key = [d.x.to_bits(), d.y.to_bits(), d.z.to_bits()];
            match index.get(&key) {
                Some(&ci) => per_cell[ci as usize],
                None => grid.planet.face_at(d)
                    .map(|fi| {
                        let best = grid.face_verts[fi].iter().copied()
                            .max_by(|&a, &b| grid.verts[a as usize].dot(d)
                                .partial_cmp(&grid.verts[b as usize].dot(d)).unwrap())
                            .unwrap();
                        per_cell[best as usize]
                    })
                    .unwrap_or(default),
            }
        })
        .collect()
}

fn owner_cells(grid: &Grid, terrain: &TerrainGen, cells: &[Terrain]) -> Vec<Terrain> {
    owner_of(grid, terrain, cells, Terrain::Plains)
}

fn owner_landform(grid: &Grid, terrain: &TerrainGen, landform: &[u8]) -> Vec<u8> {
    owner_of(grid, terrain, landform, LANDFORM_LOWLAND)
}

fn solve_elevation(
    grid: &Grid,
    terrain: &TerrainGen,
    cells: &[Terrain],
    landform: &[u8],
    tiles: &[Terrain],
    painted: &Painted,
    blends: &[(u32, u8, u8)],
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
    let owner: Vec<Terrain> = owner_cells(grid, terrain, cells);
    let owner_lf: Vec<u8> = owner_landform(grid, terrain, landform);
    let _ = tiles;
    let owner_face: Vec<Option<usize>> = (0..nv)
        .map(|vi| grid.planet.face_at(terrain.vert_dir(vi)))
        .collect();
    let mut lo = vec![-1.0f32; nv];
    let mut hi = vec![1.0f32; nv];
    let mut is_road_vert = vec![false; nv];
    for vi in 0..nv {
        // Blend faces are the altitude ramp between two kinds: their vertices
        // get the HULL of both ranges so the solver can transition through.
        // Land COVER takes its landform's range (height from the massif, not
        // the biome); water/shore/river keep their own range and blend hull.
        let (rlo, rhi) = if is_cover(owner[vi]) {
            landform_range(owner_lf[vi])
        } else {
            match owner_face[vi].and_then(|fi| blend_of.get(&(fi as u32))) {
                Some(&(_, b)) if b >= crate::level::BLEND_FEATURE_MIN => elev_range(owner[vi]),
                Some(&(a, b)) => {
                    let (alo, ahi) = elev_range(Terrain::ALL[a as usize]);
                    let (blo, bhi) = elev_range(Terrain::ALL[b as usize]);
                    (alo.min(blo), ahi.max(bhi))
                }
                None => elev_range(owner[vi]),
            }
        };
        lo[vi] = rlo;
        hi[vi] = rhi;
    }
    for ci in 0..grid.nv {
        if painted.roads.contains(ci) {
            for &(vi, _) in terrain.kernel(grid.vert_pos(ci))[..3].iter() {
                is_road_vert[vi] = true;
            }
        }
    }
    // Continental shelf as a HARD range: since Ocean is one identity now, the
    // shelf both keeps water shallow near shore (no wall at the coast) AND
    // forces it deep offshore (the ceiling drops with distance from land), so
    // the deep basins/abyss come from geometry, not a separate tile. Distance
    // to land in vertex steps (~35m).
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
            if dist[cur] >= 8 {
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
            if !owner[vi].is_water() || owner[vi] == Terrain::River {
                continue;
            }
            // Lakes keep their own (concave) profile; only the open sea shelves.
            if owner[vi] == Terrain::Lake {
                continue;
            }
            // A narrow depth WINDOW per distance band, both bounds deepening
            // with distance: shore-shallows (no wall at the coast) grading to
            // abyss offshore. floor ≤ ceil, and the step between adjacent
            // bands stays within the extreme-edge limit.
            let (floor, ceil) = match dist[vi] {
                1 => (-0.10, -0.03),
                2 => (-0.22, -0.08),
                3 => (-0.38, -0.18),
                4 => (-0.52, -0.32),
                5 => (-0.64, -0.46),
                6 => (-0.74, -0.56),
                7 => (-0.82, -0.62),
                _ => (-0.95, -0.70),
            };
            hi[vi] = hi[vi].min(ceil);
            lo[vi] = lo[vi].max(floor).min(hi[vi]);
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

    // Cliff crest vertices track the hinterland: the high ground CARRIES
    // INWARD (the cliff is the edge of a raised coast, not a wall in front of
    // low ground), and nothing sticks out above the terrain behind. The
    // seaward drop needs no pinning — the wide Cliff range and the steep
    // cliff/water gradient allowance let the whole drop happen at the edge.
    let shore_kind = |t: Terrain| t.is_water() || t == Terrain::Beach;
    let mut cliff_crest: Vec<bool> = vec![false; nv];
    for vi in 0..nv {
        if owner[vi] == Terrain::Cliff
            && !terrain.adj_of(vi).iter().any(|&nb| shore_kind(owner[nb]))
        {
            cliff_crest[vi] = true;
        }
    }

    // Bridges CONFORM to the finished terrain (selected after this solve on
    // gentle ground), so the solver no longer flattens entry pads — features
    // no longer reshape the field here.
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
                let mut cap = if is_cover(vkind[a]) && is_cover(vkind[b]) {
                    landform_edge_cap(owner_lf[a], owner_lf[b])
                } else {
                    max_gradient(vkind[a], vkind[b])
                };
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
        // 1b) road smoothing: pull every road vertex toward the average of
        // its road-corridor neighbors, so the ROAD SURFACE has no local bumps —
        // the gradient cap bounds the slope, this bounds the change in slope
        // (curvature), giving a road that eases over the ground.
        {
            let mut delta = vec![0.0f32; nv];
            for a in 0..nv {
                if !is_road_vert[a] {
                    continue;
                }
                let mut sum = 0.0;
                let mut cnt = 0;
                for &b in terrain.adj_of(a) {
                    if is_road_vert[b] {
                        sum += e[b];
                        cnt += 1;
                    }
                }
                if cnt > 0 {
                    // Half-strength Laplacian: smooths bumps without erasing
                    // the road's overall descent.
                    delta[a] = 0.5 * (sum / cnt as f32 - e[a]);
                }
            }
            for a in 0..nv {
                if delta[a] != 0.0 {
                    e[a] += delta[a];
                    residual += delta[a].abs();
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
        // 3) lake and river beds are CONCAVE: every water vert is pushed
        // below the average of its neighbors, so basins bowl toward the
        // middle and channels dip below their banks — depth grows naturally
        // with basin size instead of being a flat plate.
        for vi in 0..nv {
            let Some(c) = water_concavity(owner[vi]) else { continue };
            let nbs = terrain.adj_of(vi);
            let avg: f32 = nbs.iter().map(|&nb| e[nb]).sum::<f32>() / nbs.len() as f32;
            let cap = avg - c;
            if e[vi] > cap {
                residual += e[vi] - cap;
                e[vi] = cap;
            }
        }
        // 3b) cliff crest tracking: the crest equals the hinterland edge.
        for vi in 0..nv {
            if cliff_crest[vi] {
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
        // 4) banks sit ABOVE their water: a river bank is strictly higher
        // than the adjacent river, a lake shore than its lake, a beach than
        // the sea — the water's edge is always a step up onto land. Water
        // surfaces render clamped at 0, so the floor is vs max(water e, 0).
        for vi in 0..nv {
            // A bank vert caught in a river's descent kernel still IS a bank:
            // on a hillside the kernel would drag the downhill bank below the
            // water and the river would spill. The floor runs after the
            // descent step, so both banks end above the channel everywhere.
            if is_canyon_vert[vi] && owner[vi] != Terrain::RiverBank {
                continue;
            }
            let matching_water = bank_water(owner[vi]);
            if matching_water.is_empty() {
                continue;
            }
            let mut water_surface = f32::MIN;
            for &nb in terrain.adj_of(vi) {
                if matching_water.contains(&owner[nb]) {
                    water_surface = water_surface.max(e[nb].max(0.0));
                }
            }
            if water_surface > f32::MIN {
                let floor = water_surface + 0.01;
                if e[vi] < floor {
                    residual += floor - e[vi];
                    e[vi] = floor;
                }
            }
        }
        // 5) tile ranges — hard constraints and always get the
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
    // Final guarantee (the solver's per-iteration bank floor can lose a race
    // to river descent at a high source): every bank vert ends strictly above
    // its adjacent water surface. Highest banks first so a bank that borders
    // another bank still clears the shared water.
    let mut order: Vec<usize> = (0..nv)
        .filter(|&vi| matches!(owner[vi], Terrain::RiverBank | Terrain::LakeShore | Terrain::Beach))
        .collect();
    order.sort_by(|&a, &b| e[b].partial_cmp(&e[a]).unwrap());
    for &vi in &order {
        let matching = bank_water(owner[vi]);
        if matching.is_empty() {
            continue;
        }
        let mut surface = f32::MIN;
        for &nb in terrain.adj_of(vi) {
            if matching.contains(&owner[nb]) {
                surface = surface.max(e[nb].max(0.0));
            }
        }
        if surface > f32::MIN {
            e[vi] = e[vi].max(surface + 0.01);
        }
    }
    for v in &mut e {
        *v = v.clamp(-1.0, 1.0);
    }
    (e, iters, residual)
}

// ---- helpers ----

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
        // Bridges CONFORM to the terrain (no entry pads), so every vertex must
        // satisfy its tile range — no pad exemption.
        // Ownership is per-CELL (each solver vert is a grid cell), exactly as
        // the solver assigns ranges — the face type may differ at boundaries.
        let owners = owner_cells(&state.grid, terrain, &state.cells);
        let owners_lf = owner_landform(&state.grid, terrain, &state.landform);
        for vi in 0..terrain.vert_count() {
            if canyon[vi] {
                continue;
            }
            let Some(fi) = state.grid.planet.face_at(terrain.vert_dir(vi)) else { continue };
            // Land cover takes its landform's range (mirror the solver);
            // water/shore/river keep their own range and blend hull.
            let (rlo, rhi) = if is_cover(owners[vi]) {
                landform_range(owners_lf[vi])
            } else {
                match blend_of.get(&(fi as u32)) {
                    Some(&(_, b)) if b >= crate::level::BLEND_FEATURE_MIN => elev_range(owners[vi]),
                    Some(&(a, b)) => {
                        let (alo, ahi) = elev_range(Terrain::ALL[a as usize]);
                        let (blo, bhi) = elev_range(Terrain::ALL[b as usize]);
                        (alo.min(blo), ahi.max(bhi))
                    }
                    None => elev_range(owners[vi]),
                }
            };
            assert!(
                e[vi] >= rlo - 1e-4 && e[vi] <= rhi + 1e-4,
                "vert {vi} ({:?}) out of range: {} not in [{rlo}, {rhi}]",
                owners[vi], e[vi]
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
                is_ocean || body.len() >= size_range(Terrain::Lake).0,
                "enclosed water body of only {} cells survived",
                body.len()
            );
        }

        // Deep water never surfaces: every abyss-depth cell solves well below
        // the waterline (the depth class replaces the old DeepOcean tile).
        // (Depth is per grid CELL — sample the solved field at the cell.)
        for vi in 0..state.grid.nv {
            if state.water_depth[vi] == DEPTH_ABYSS {
                let d = terrain.elevation_at(state.grid.vert_pos(vi));
                assert!(d < -0.1, "abyss cell {vi} not deep: {d}");
            }
        }

        // Roads never sit on water: checked per cell (painting is per cell)
        // and per solid road face.
        for vi in 0..state.grid.nv {
            if state.painted.roads.contains(vi) {
                assert!(state.cells[vi].is_land(), "road painted on water cell {vi}");
            }
        }
        for fi in 0..state.grid.n {
            let solid = state.grid.face_verts[fi].iter()
                .filter(|&&vi| state.painted.roads.contains(vi as usize))
                .count() == 3;
            if solid {
                assert!(state.tiles[fi].is_land(), "solid road face on water tile {fi}");
            }
        }

        // Bridge entries land on walkable ground: the deck grounds at ground
        // height (no vertical gap, by the deck builder) and the player steps
        // onto it from gentle inland terrain. (An omnidirectional slope test is
        // meaningless here — every deck end sits at a water's edge, so the bank
        // drop toward the water is steep by nature; the approach is inland.)
        let bridge_walkable = |t: Terrain| matches!(
            t,
            Terrain::Plains | Terrain::Forest | Terrain::Savanna | Terrain::Tundra
                | Terrain::Desert | Terrain::Jungle | Terrain::Swamp
        );
        for span in &state.bridges {
            for end in [span.first(), span.last()].into_iter().flatten() {
                let fi = state.grid.planet.face_at(end.0).expect("deck end on a face");
                assert!(
                    bridge_walkable(state.tiles[fi]),
                    "bridge entry on non-walkable tile {:?}", state.tiles[fi]
                );
                // The anchor CELL (where placement gated on the solved slope)
                // is genuinely gentle — a bridge never lands on steep ground,
                // whatever the biome. (An omnidirectional slope at the exact
                // water's-edge end would just read the natural bank drop.)
                let cell = state.grid.face_verts[fi].iter().copied()
                    .max_by(|&a, &b| state.grid.verts[a as usize].dot(end.0)
                        .partial_cmp(&state.grid.verts[b as usize].dot(end.0)).unwrap())
                    .unwrap();
                let slope = terrain.slope(state.grid.vert_pos(cell as usize));
                assert!(slope < 0.3, "bridge anchor on steep ground: slope {slope}");
            }
        }

        // Tile identity lives on hex cells (cells can't pinch), and the
        // LINKING RULE covers the derived faces: around any cell, same-type
        // faces form ONE edge-connected fan — never linked by a lone vertex.
        // Also checked: every face's type is one of its corner cells, and
        // solid feature faces obey the same fan rule.
        {
            for fi in 0..state.grid.n {
                let corners = state.grid.face_verts[fi];
                assert!(
                    corners.iter().any(|&vi| state.cells[vi as usize] == state.tiles[fi]),
                    "face {fi} derived {:?} not among its corner cells",
                    state.tiles[fi]
                );
            }
            let mut tile_pinches = 0usize;
            let mut road_pinches = 0usize;
            for v in 0..state.grid.nv {
                let ring = ring(&state.grid, v);
                let n = ring.len();
                let ts: Vec<Terrain> = (0..n)
                    .map(|i| derive_one(
                        state.cells[v], state.cells[ring[i]], state.cells[ring[(i + 1) % n]],
                    ))
                    .collect();
                let mut types = ts.clone();
                types.sort_by_key(|t| *t as u8);
                types.dedup();
                for t in types {
                    let runs = (0..n).filter(|&i| ts[i] == t && ts[(i + n - 1) % n] != t).count();
                    if runs >= 2 {
                        tile_pinches += 1;
                    }
                }
                if state.painted.roads.contains(v) {
                    let solid = |i: usize| {
                        state.painted.roads.contains(ring[i])
                            && state.painted.roads.contains(ring[(i + 1) % n])
                    };
                    let runs = (0..n).filter(|&i| solid(i) && !solid((i + n - 1) % n)).count();
                    if runs >= 2 {
                        road_pinches += 1;
                    }
                }
            }
            assert_eq!(tile_pinches, 0, "same-type faces linked by a lone vertex");
            assert_eq!(road_pinches, 0, "road strip pinched at a vertex");

            // Lake-to-ocean distance ≥ 10 edge steps.
            let mut dist = vec![u16::MAX; state.grid.n];
            let mut q: VecDeque<usize> = VecDeque::new();
            for fi in 0..state.grid.n {
                // Actual ocean TILES, not ocean-zone: an isolated ocean-zone
                // pocket is reclassified to a lake (see size_range(Ocean)) and
                // must not seed the distance field.
                if state.tiles[fi] == Terrain::Ocean {
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
        let vkind: Vec<Terrain> = owner_cells(&state.grid, terrain, &state.cells);
        let shore_kind = |t: Terrain| t.is_water() || t == Terrain::Beach;
        for a in 0..terrain.vert_count() {
            if vkind[a] != Terrain::Cliff || canyon[a] {
                continue;
            }
            let shoreside = terrain.adj_of(a).iter().any(|&nb| shore_kind(vkind[nb]));
            // Seaward cliff verts are free — the drop happens at the water
            // edge; only the crest is constrained (carries the hinterland
            // inward, never sticks out above it).
            if !shoreside {
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

        // Banks sit strictly above their water: bank verts exceed the
        // adjacent water surface (water renders clamped at 0).
        for vi in 0..terrain.vert_count() {
            if canyon[vi] && vkind[vi] != Terrain::RiverBank {
                continue;
            }
            let matching = bank_water(vkind[vi]);
            if matching.is_empty() {
                continue;
            }
            for &nb in terrain.adj_of(vi) {
                if matching.contains(&vkind[nb]) {
                    assert!(
                        e[vi] > e[nb].max(0.0),
                        "{:?} vert at {} not above its {:?} water at {}",
                        vkind[vi], e[vi], vkind[nb], e[nb].max(0.0)
                    );
                }
            }
        }

        // Lake beds are concave: interior verts (all-lake neighborhoods) sit
        // below the bed's edge verts on average.
        {
            let mut interior = Vec::new();
            let mut edge = Vec::new();
            for vi in 0..terrain.vert_count() {
                if vkind[vi] != Terrain::Lake || canyon[vi] {
                    continue;
                }
                if terrain.adj_of(vi).iter().all(|&nb| vkind[nb] == Terrain::Lake) {
                    interior.push(e[vi]);
                } else {
                    edge.push(e[vi]);
                }
            }
            if !interior.is_empty() && !edge.is_empty() {
                let avg = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
                assert!(
                    avg(&interior) < avg(&edge),
                    "lake beds not concave: interior {} vs edge {}",
                    avg(&interior), avg(&edge)
                );
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
    fn rivers_reach_the_sea() {
        // Every river must join a larger water body — no thin terrain band
        // may cut a mouth off (guards the junction-face damming bug).
        let state = run(1337, |_| {});
        let mut visited = vec![false; state.grid.nv];
        for start in 0..state.grid.nv {
            if state.cells[start] != Terrain::River || visited[start] {
                continue;
            }
            let mut comp = vec![start];
            let mut q = VecDeque::from([start]);
            visited[start] = true;
            while let Some(cur) = q.pop_front() {
                for &nb in &state.grid.vert_adj[cur] {
                    let nb = nb as usize;
                    if state.cells[nb] == Terrain::River && !visited[nb] {
                        visited[nb] = true;
                        comp.push(nb);
                        q.push_back(nb);
                    }
                }
            }
            let touches_sea = comp.iter().any(|&vi| {
                state.grid.vert_adj[vi].iter().any(|&nb| matches!(
                    state.cells[nb as usize], Terrain::Ocean | Terrain::Lake
                ))
            });
            assert!(touches_sea, "river component of {} cells cut off from any water body", comp.len());
            // And the mouth is open at FACE level too: some River face is
            // edge-adjacent to an Ocean/Lake face.
            let mut open = false;
            'faces: for fi in 0..state.grid.n {
                if state.tiles[fi] != Terrain::River {
                    continue;
                }
                if !state.grid.face_verts[fi].iter().any(|&vi| comp.contains(&(vi as usize))) {
                    continue;
                }
                for &nb in &state.grid.adj[fi] {
                    if matches!(
                        state.tiles[nb as usize],
                        Terrain::Ocean | Terrain::Lake
                    ) {
                        open = true;
                        break 'faces;
                    }
                }
            }
            assert!(open, "river mouth dammed at face level ({} cells)", comp.len());
        }
    }

    #[test]
    fn mesh_is_watertight() {
        // Every fall-through bug is a crack: an edge used by only one triangle
        // is a hole the player can drop through. On a closed surface each edge
        // is shared by EXACTLY two faces, and every vertex fan is a full ring.
        // Assert both on the baked mesh (the collider is built from it).
        let state = run(1337, |_| {});
        let grid = &state.grid;

        // (a) each undirected edge belongs to exactly two faces.
        let mut edge_faces: BTreeMap<(u32, u32), u32> = BTreeMap::new();
        for idx in &grid.face_verts {
            for k in 0..3 {
                let (a, b) = (idx[k], idx[(k + 1) % 3]);
                let key = if a < b { (a, b) } else { (b, a) };
                *edge_faces.entry(key).or_default() += 1;
            }
        }
        for (&(a, b), &n) in &edge_faces {
            assert_eq!(n, 2, "edge ({a},{b}) shared by {n} faces (not 2) — a crack");
        }

        // (b) every vertex's faces form ONE closed fan: walking face→face
        // across shared edges visits all of them and returns. A vertex whose
        // fan splits is a pinhole even if each edge is shared twice.
        for v in 0..grid.nv {
            let faces = &grid.vert_faces[v];
            let n = faces.len();
            assert!((5..=6).contains(&n), "vertex {v} has {n} faces");
            let mut seen = vec![false; n];
            let mut stack = vec![0usize];
            seen[0] = true;
            let mut count = 1;
            while let Some(i) = stack.pop() {
                for j in 0..n {
                    if seen[j] {
                        continue;
                    }
                    // Adjacent in the fan iff they share an edge through v
                    // (two common vertices).
                    let fi = grid.face_verts[faces[i] as usize];
                    let fj = grid.face_verts[faces[j] as usize];
                    let shared = fi.iter().filter(|x| fj.contains(x)).count();
                    if shared == 2 {
                        seen[j] = true;
                        count += 1;
                        stack.push(j);
                    }
                }
            }
            assert_eq!(count, n, "vertex {v} fan is not one closed ring — a pinhole");
        }
    }

    #[test]
    fn deterministic_pipeline() {
        let a = run(42, |_| {});
        let b = run(42, |_| {});
        assert_eq!(a.terrain.unwrap().vert_elevations(), b.terrain.unwrap().vert_elevations());
        assert_eq!(a.cells, b.cells);
        assert_eq!(a.tiles, b.tiles);
        assert_eq!(a.regions.len(), b.regions.len());
        assert_eq!(a.flora.len(), b.flora.len());
        assert!(a.flora.iter().zip(&b.flora).all(|(x, y)| x.pos == y.pos && x.kind == y.kind));
        assert_eq!(a.structures.len(), b.structures.len());
        assert!(a.structures.iter().zip(&b.structures).all(|(x, y)| x.pos == y.pos && x.kind == y.kind));
        assert_eq!(a.slope_class, b.slope_class);
        assert_eq!(a.water_depth, b.water_depth);
        assert_eq!(a.landform, b.landform);
    }

    #[test]
    fn flora_stays_off_water_and_features() {
        let state = run(1337, |_| {});
        assert!(state.flora.len() > 1000, "flora nearly absent: {}", state.flora.len());
        for f in &state.flora {
            let fi = f.face as usize;
            assert!(state.tiles[fi].is_land(), "flora on water face {fi}");
            for bits in [&state.painted.roads, &state.painted.towns, &state.painted.bridge_entries] {
                assert_eq!(
                    painted_corners(&state.grid, bits, fi), 0,
                    "flora on a feature face {fi}"
                );
            }
        }
    }
}
