//! Generation orchestrator: a decide/evolve/react command-event state machine.
//!
//! Each `Command` does exactly one thing. `decide` computes (pure over the
//! state), returning `Event`s that carry the results; `evolve` folds events
//! into the state; `react` maps events to follow-up commands on a FIFO queue.
//! The loop is deterministic (FIFO order, seeded RNG streams per pass), every
//! re-enqueue is bounded, and the event log is the audit trail when a seed
//! misbehaves. The public boundary returns only a completed runtime artifact
//! and its summary statistics.

use bevy::color::ColorToComponents;
use bevy::prelude::Vec3;
use std::collections::{BTreeMap, VecDeque};

use crate::level::{
    DEPTH_ABYSS, DEPTH_DEEP, DEPTH_SHALLOW, FLORA_BERRY, FLORA_BUSH, FLORA_CACTUS, FLORA_DEADTREE,
    FLORA_FLOWER, FLORA_GRASS, FLORA_LOG, FLORA_MUSHROOM, FLORA_REED, FLORA_ROCK, FLORA_TREE,
    FloraData, LANDFORM_HILLS, LANDFORM_LOWLAND, LANDFORM_MOUNTAINS, LANDFORM_PLATEAU,
    LANDFORM_VALLEY, LANDFORM_WATER, LEVEL_FORMAT_VERSION, LevelData, NO_REGION, ROAD_MAT_DIRT,
    ROAD_MAT_GRAVEL, ROAD_MAT_ROCK, ROAD_MAT_SAND, RegionData, RegionKind, RoadData, SLOPE_CLIFF,
    SLOPE_FLAT, SLOPE_GENTLE, SLOPE_STEEP, STRUCT_CAMPFIRE, STRUCT_DOCK, STRUCT_FARM, STRUCT_RUIN,
    STRUCT_WALL, STRUCT_WATCHTOWER, STRUCT_WELL, SettlementData, StructureData, TAG_BRIDGE,
    TAG_BRIDGE_ENTRY, TAG_ROAD, TAG_TOWN, slope_walkable,
};
use crate::planet::{PlanetMesh, unit_icosphere_tris};
use crate::sphere::SpherePos;
use crate::terrain::{Terrain, TerrainGen};
#[cfg(test)]
use crate::topology::FaceComponentId;
use crate::topology::{CellComponentId, CellId, ComponentLabels, FaceId, TerrainTopology};
use crate::wfc;
use crate::zones::FINE_SUB;

mod classification;
mod domain;
mod elevation;
mod features;
mod projection;
mod regions;
mod router;

use domain::{CellField, CellSet, FaceField};

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
struct Grid {
    pub seed: u32,
    pub unit_tris: Vec<[Vec3; 3]>,
    pub planet: PlanetMesh,
    /// Canonical cell positions and all connectivity.
    pub topology: TerrainTopology,
}

impl Grid {
    pub fn new(seed: u32) -> Self {
        let unit_tris = unit_icosphere_tris(FINE_SUB);
        let planet = PlanetMesh::new(unit_tris.clone());
        let topology_tris: Vec<[[f32; 3]; 3]> = unit_tris
            .iter()
            .map(|triangle| triangle.map(|position| position.to_array()))
            .collect();
        let topology = TerrainTopology::from_triangles(&topology_tris);
        debug_assert!(
            topology
                .cells()
                .all(|cell| (5..=6).contains(&topology.cell_neighbors(cell).len())),
            "hex adjacency broken"
        );
        debug_assert_eq!(topology.face_count(), unit_tris.len());
        Self {
            seed,
            unit_tris,
            planet,
            topology,
        }
    }

    pub fn centroid(&self, face: FaceId) -> SpherePos {
        let [a, b, c] = self.unit_tris[face.index()];
        SpherePos::new(((a + b + c) / 3.0).normalize())
    }

    pub fn cell_position(&self, cell: CellId) -> SpherePos {
        SpherePos::new(self.cell_direction(cell))
    }

    fn cell_direction(&self, cell: CellId) -> Vec3 {
        Vec3::from_array(self.topology.cell_position(cell)).normalize()
    }

    pub fn cell_count(&self) -> usize {
        self.topology.cell_count()
    }

    pub fn face_count(&self) -> usize {
        self.topology.face_count()
    }

    fn face_neighbors(&self, face: FaceId) -> [FaceId; 3] {
        self.topology.face_neighbors(face)
    }

    fn face_cells(&self, face: FaceId) -> [CellId; 3] {
        self.topology.face_cells(face)
    }

    fn cell_neighbors(&self, cell: CellId) -> &[CellId] {
        self.topology.cell_neighbors(cell)
    }
}

/// Face render type from its 3 corner cells: two agreeing corners win; a
/// junction face (3 distinct labels) goes to the transition kind if one is
/// present, else water, else the lowest discriminant — deterministic and
/// conservative at waterlines.
fn derive_tiles(grid: &Grid, cells: &[Terrain]) -> FaceField<Terrain> {
    grid.topology
        .faces()
        .map(|face| {
            let idx = grid.face_cells(face).map(CellId::index);
            classification::derive_face(cells[idx[0]], cells[idx[1]], cells[idx[2]])
        })
        .collect::<Vec<_>>()
        .into()
}

/// The neighbors of a cell in CYCLIC order (walking the face fan), starting
/// from the lowest-id neighbor — deterministic.
fn ring(grid: &Grid, v: usize) -> Vec<usize> {
    let mut out = Vec::with_capacity(grid.cell_neighbors(CellId::new(v)).len());
    let mut cur = grid.cell_neighbors(CellId::new(v))[0].index();
    out.push(cur);
    loop {
        let cell = grid
            .topology
            .cell(v)
            .expect("cell index from topology range");
        let next = grid.topology.cell_faces(cell).iter().find_map(|&face| {
            let idx = grid.face_cells(face).map(CellId::index);
            if !idx.contains(&cur) {
                return None;
            }
            let third = *idx.iter().find(|&&x| x != v && x != cur)?;
            (!out.contains(&third)).then_some(third)
        });
        match next {
            Some(t) => {
                out.push(t);
                cur = t;
            }
            None => break,
        }
    }
    debug_assert_eq!(out.len(), grid.cell_neighbors(CellId::new(v)).len());
    out
}

/// THE LINKING RULE, generic over every type except the compact RiverSpring
/// point-feature patch: around any ordinary cell, the faces
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
            .map(|i| {
                classification::derive_face(cells[v], cells[ring[i]], cells[ring[(i + 1) % n]])
            })
            .collect();
        let mut types = ts.clone();
        types.sort_by_key(|t| *t as u8);
        types.dedup();
        types
            .iter()
            .map(|&t| {
                let runs = (0..n)
                    .filter(|&i| ts[i] == t && ts[(i + n - 1) % n] != t)
                    .count();
                runs.saturating_sub(1)
            })
            .sum()
    }
    // A retype of cell c only changes faces containing c — the fans of c and
    // its ring. Accept a candidate only if the potential over that
    // neighborhood strictly DROPS: the loop then converges (the global
    // potential is a non-negative integer that decreases with every change).
    let local = |cells: &[Terrain], c: usize| -> usize {
        potential_at(grid, cells, c)
            + ring(grid, c)
                .iter()
                .map(|&w| potential_at(grid, cells, w))
                .sum::<usize>()
    };
    for it in 0..64 {
        debug_assert!(it < 63, "link_tile_pinches did not converge");
        let mut changed = false;
        for v in 0..grid.cell_count() {
            if potential_at(grid, cells, v) == 0 {
                continue;
            }
            let ring_v = ring(grid, v);
            let ts: Vec<Terrain> = {
                let n = ring_v.len();
                (0..n)
                    .map(|i| {
                        classification::derive_face(
                            cells[v],
                            cells[ring_v[i]],
                            cells[ring_v[(i + 1) % n]],
                        )
                    })
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
                            || matches!(cells[c], Terrain::River | Terrain::RiverSpring)
                            || (cells[c].is_water() != t.is_water()) != cross
                        {
                            continue;
                        }
                        // Turning land into water must extend an existing
                        // body of that kind, never strand a puddle.
                        if cross
                            && t.is_water()
                            && !grid
                                .cell_neighbors(CellId::new(c))
                                .iter()
                                .any(|nb| cells[nb.index()] == t)
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
        for v in 0..grid.cell_count() {
            if !bits.contains(CellId::new(v)) {
                continue;
            }
            let ring = ring(grid, v);
            let n = ring.len();
            let runs = |bits: &BitSet| -> usize {
                let solid = |i: usize| {
                    bits.contains(CellId::new(ring[i]))
                        && bits.contains(CellId::new(ring[(i + 1) % n]))
                };
                (0..n)
                    .filter(|&i| solid(i) && !solid((i + n - 1) % n))
                    .count()
            };
            let r = runs(bits);
            if r < 2 {
                continue;
            }
            for &candidate in ring.iter().take(n) {
                if bits.contains(CellId::new(candidate)) || !passable(candidate) {
                    continue;
                }
                bits.insert(CellId::new(candidate));
                if runs(bits) < r {
                    changed = true;
                    break;
                }
                // BitSet has no remove; rebuild the bit by clearing the word bit.
                bits.remove(CellId::new(candidate));
            }
        }
        if !changed {
            break;
        }
    }
}

// ---- typed domain storage ----

type BitSet = CellSet;

/// Built features, painted per CELL (like terrain identity): a face is a
/// solid feature where ≥2 of its corner cells are painted, and fades out at
/// the edges through per-corner colors — the same construction as terrain,
/// so feature footprints can never pinch or zigzag either.
#[derive(Clone)]
struct Painted {
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
    features::painted_corners(grid, bits, fi)
}

/// A face is a solid feature surface only when the feature owns ALL its
/// corners: for a band painted as two parallel lattice lines that is exactly
/// the parallelogram strip between them (straight edges = the lines
/// themselves). Faces with 1–2 painted corners form one straight-edged strip
/// on each side — the blend band, rendered as a per-corner gradient.
fn face_solid(grid: &Grid, bits: &BitSet, fi: usize) -> bool {
    features::face_solid(grid, bits, fi)
}

// ---- state ----

struct GenState {
    pub grid: Grid,
    pub terrain: Option<TerrainGen>,
    /// Per-cell tile labels — the single source of truth for terrain identity.
    pub cells: CellField<Terrain>,
    /// Per-face render/physics type, DERIVED from `cells` on every cell change.
    pub tiles: FaceField<Terrain>,
    pub painted: Painted,
    pub bridges: Vec<Vec<SpherePos>>,
    /// Road polylines that survived water checks — the single source of truth
    /// for serialization (NOT terrain.road_paths, which is the L2 plan).
    pub roads: Vec<Vec<SpherePos>>,
    /// Inland biome-boundary faces and the kind pair they link.
    pub blends: Vec<(u32, u8, u8)>,
    pub regions: Vec<RegionData>,
    pub face_region: FaceField<u32>,
    /// Per-face water-surface radius (0.0 = dry), clustered once (sea + lakes).
    pub water_r: FaceField<f32>,
    /// Generation-baked per-corner river surface. Runtime and serializers do
    /// not derive river topology.
    pub river_r: FaceField<[f32; 3]>,
    pub mesh_tris: FaceField<[[f32; 3]; 3]>,
    pub mesh_colors: FaceField<[[f32; 4]; 3]>,
    pub tag_off: Vec<u32>,
    pub tag_data: Vec<u8>,
    pub flora: Vec<FloraData>,
    pub structures: Vec<StructureData>,
    /// Per-cell terrain steepness (0 Flat, 1 Gentle, 2 Steep, 3 Cliff) from the
    /// SOLVED field. Walkability and feature placement gate on this, so a
    /// mountain pass (Gentle inside Mountains) is traversable and a "flat"
    /// biome that solved steep is not.
    pub slope_class: CellField<u8>,
    /// Per-cell water depth class (DEPTH_*) for water cells.
    pub water_depth: CellField<u8>,
    /// Per-cell macro landform (LANDFORM_*), from the proposed field.
    pub landform: CellField<u8>,
    /// Final cell-to-face projections consumed by `LevelData`.
    pub face_slope_class: FaceField<u8>,
    pub face_water_depth: FaceField<u8>,
    pub face_landform: FaceField<u8>,
    pub face_road_material: FaceField<u8>,
}

pub struct CompletedWorld {
    level: LevelData,
    stats: GenerationStats,
}

impl CompletedWorld {
    pub fn level_data(&self) -> &LevelData {
        &self.level
    }
    pub fn into_level_data(self) -> LevelData {
        self.level
    }
    pub fn stats(&self) -> &GenerationStats {
        &self.stats
    }
}

pub struct GenerationStats {
    pub min_elevation: f32,
    pub max_elevation: f32,
    pub terrain_faces: BTreeMap<&'static str, usize>,
    pub water_faces: usize,
    pub face_count: usize,
    pub flora_count: usize,
    pub structure_count: usize,
    pub region_count: usize,
    pub bridge_count: usize,
}

impl GenState {
    fn new(seed: u32) -> Self {
        let grid = Grid::new(seed);
        let painted = Painted::empty(grid.cell_count());
        Self {
            grid,
            terrain: None,
            cells: CellField::default(),
            tiles: FaceField::default(),
            painted,
            bridges: Vec::new(),
            roads: Vec::new(),
            blends: Vec::new(),
            regions: Vec::new(),
            face_region: FaceField::default(),
            water_r: FaceField::default(),
            river_r: FaceField::default(),
            mesh_tris: FaceField::default(),
            mesh_colors: FaceField::default(),
            tag_off: Vec::new(),
            tag_data: Vec::new(),
            flora: Vec::new(),
            structures: Vec::new(),
            slope_class: CellField::default(),
            water_depth: CellField::default(),
            landform: CellField::default(),
            face_slope_class: FaceField::default(),
            face_water_depth: FaceField::default(),
            face_landform: FaceField::default(),
            face_road_material: FaceField::default(),
        }
    }

    fn terrain(&self) -> &TerrainGen {
        self.terrain.as_ref().expect("terrain not generated yet")
    }

    /// Package a completed pipeline into the runtime artifact. This is the
    /// single cell-to-runtime boundary; serializers only encode the result.
    fn to_level_data(&self) -> LevelData {
        let terrain = self.terrain.as_ref().expect("pipeline finished");
        let settlements = terrain
            .settlement_anchors
            .iter()
            .enumerate()
            .map(|(i, anchor)| SettlementData {
                name: crate::roads::settlement_name(i),
                pos: anchor.0.to_array(),
            })
            .collect();
        let mut roads: Vec<RoadData> = self
            .roads
            .iter()
            .map(|path| RoadData {
                points: path.iter().map(|point| point.0.to_array()).collect(),
                is_bridge: false,
            })
            .collect();
        roads.extend(self.bridges.iter().map(|path| RoadData {
            points: path.iter().map(|point| point.0.to_array()).collect(),
            is_bridge: true,
        }));

        LevelData {
            version: LEVEL_FORMAT_VERSION,
            seed: self.grid.seed,
            vert_elev: terrain.vert_elevations().to_vec(),
            terrain_tris: self.mesh_tris.dense().to_vec(),
            terrain_colors: self.mesh_colors.dense().to_vec(),
            unit_tris: self
                .grid
                .unit_tris
                .iter()
                .map(|triangle| triangle.map(|point| point.to_array()))
                .collect(),
            face_types: self.tiles.iter().map(|terrain| *terrain as u8).collect(),
            face_water_r: self.water_r.dense().to_vec(),
            face_river_r: self.river_r.dense().to_vec(),
            face_tag_off: self.tag_off.clone(),
            face_tag_data: self.tag_data.clone(),
            face_blend: self.blends.clone(),
            settlements,
            roads,
            regions: self.regions.clone(),
            face_region: self.face_region.to_vec(),
            flora: self.flora.clone(),
            structures: self.structures.clone(),
            slope_class: self.face_slope_class.to_vec(),
            water_depth: self.face_water_depth.to_vec(),
            landform: self.face_landform.to_vec(),
            road_material: self.face_road_material.to_vec(),
        }
    }

    fn stats(&self) -> GenerationStats {
        let terrain = self.terrain.as_ref().expect("pipeline finished");
        let elevations = terrain.vert_elevations();
        let mut terrain_faces = BTreeMap::new();
        for &kind in &self.tiles {
            let name = match kind {
                Terrain::Ocean => "Ocean",
                Terrain::Lake => "Lake",
                Terrain::LakeShore => "LakeShore",
                Terrain::River => "River",
                Terrain::RiverBank => "RiverBank",
                Terrain::RiverSpring => "RiverSpring",
                Terrain::Beach => "Beach",
                Terrain::Cliff => "Cliff",
                Terrain::Desert => "Desert",
                Terrain::Plains => "Plains",
                Terrain::Forest => "Forest",
                Terrain::Tundra => "Tundra",
                Terrain::Mountain => "Mountain",
                Terrain::Snow => "Snow",
                Terrain::Swamp => "Swamp",
                Terrain::Jungle => "Jungle",
                Terrain::Savanna => "Savanna",
                Terrain::Volcanic => "Volcanic",
                Terrain::Glacier => "Glacier",
            };
            *terrain_faces.entry(name).or_default() += 1;
        }
        GenerationStats {
            min_elevation: elevations.iter().copied().fold(f32::MAX, f32::min),
            max_elevation: elevations.iter().copied().fold(f32::MIN, f32::max),
            water_faces: self
                .tiles
                .iter()
                .filter(|terrain| terrain.is_water())
                .count(),
            face_count: self.grid.face_count(),
            terrain_faces,
            flora_count: self.flora.len(),
            structure_count: self.structures.len(),
            region_count: self.regions.len(),
            bridge_count: self.bridges.len(),
        }
    }
}

// ---- commands & events ----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Command {
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
    /// Cluster water into connected bodies (sea + lakes), one waterline each.
    ClusterWater,
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
    /// Bake every face projection and river geometry field after cell-owned
    /// state and physical mesh output are final.
    BakeOutputs,
}

enum Event {
    TerrainInitialized(Box<TerrainGen>),
    ElevationProposed(Vec<f32>),
    ClimateComputed(Vec<f32>, Vec<f32>),
    RiversPlanned(Vec<Vec<SpherePos>>),
    SettlementsPlaced(Vec<SpherePos>),
    RoadsPlanned(Vec<Vec<SpherePos>>),
    TilesClassified(CellField<Terrain>),
    RiversPainted(CellField<Terrain>),
    WaterNormalized(CellField<Terrain>),
    FeaturesPainted(Painted, Vec<Vec<SpherePos>>),
    TransitionsResolved(CellField<Terrain>),
    BlendsMarked(Vec<(u32, u8, u8)>),
    ElevationSolved {
        field: Vec<f32>,
        iters: usize,
        residual: f32,
    },
    WaterClustered(Vec<f32>),
    RegionsBuilt(Vec<RegionData>, Vec<u32>),
    BridgesSelected(Vec<Vec<SpherePos>>, Painted),
    MeshBuilt(Vec<[[f32; 3]; 3]>, Vec<[[f32; 4]; 3]>),
    TagsBuilt(Vec<u32>, Vec<u8>),
    FloraPlaced(Vec<FloraData>),
    StructuresPlaced(Vec<StructureData>),
    SlopeClassified(Vec<u8>, Vec<u8>),
    LandformClassified(Vec<u8>),
    OutputsBaked {
        river_r: Vec<[f32; 3]>,
        slope: Vec<u8>,
        depth: Vec<u8>,
        landform: Vec<u8>,
        road_material: Vec<u8>,
    },
}

impl Event {
    /// Short line for the audit log.
    pub fn label(&self) -> String {
        match self {
            Event::TerrainInitialized(t) => format!(
                "terrain initialized: {} zones on {} coarse faces",
                t.zones().zones.len(),
                t.zones().face_count()
            ),
            Event::ElevationProposed(e) => format!("elevation proposed: {} verts", e.len()),
            Event::ClimateComputed(m, _) => format!("climate computed: {} verts", m.len()),
            Event::RiversPlanned(r) => format!("rivers planned: {}", r.len()),
            Event::SettlementsPlaced(a) => format!("settlements placed: {}", a.len()),
            Event::RoadsPlanned(r) => format!("roads planned: {}", r.len()),
            Event::TilesClassified(c) => format!("cells classified: {}", c.len()),
            Event::RiversPainted(_) => "rivers painted".into(),
            Event::WaterNormalized(_) => "water bodies normalized".into(),
            Event::FeaturesPainted(_, roads) => {
                format!("roads + towns painted: {} roads kept", roads.len())
            }
            Event::TransitionsResolved(_) => "transitions resolved".into(),
            Event::BlendsMarked(b) => format!("blends marked: {}", b.len()),
            Event::ElevationSolved {
                iters, residual, ..
            } => {
                format!("elevation solved: {iters} iterations, residual {residual:.4}")
            }
            Event::WaterClustered(r) => {
                format!(
                    "water clustered: {} faces",
                    r.iter().filter(|&&x| x > 0.0).count()
                )
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
                let mtn = lf
                    .iter()
                    .filter(|&&l| l == LANDFORM_MOUNTAINS || l == LANDFORM_PLATEAU)
                    .count();
                format!("landform classified: {mtn} mountain/plateau cells")
            }
            Event::OutputsBaked { river_r, .. } => format!(
                "face outputs baked: {} river faces",
                river_r.iter().filter(|r| **r != [0.0; 3]).count()
            ),
        }
    }
}

// ---- decide / evolve / react ----

fn decide(state: &GenState, cmd: &Command) -> Vec<Event> {
    match cmd {
        Command::InitTerrain => {
            vec![Event::TerrainInitialized(Box::new(TerrainGen::init(
                state.grid.seed,
            )))]
        }
        Command::ProposeElevation => {
            vec![Event::ElevationProposed(
                state.terrain().propose_elevation(),
            )]
        }
        Command::ComputeClimate => {
            let (moist, temp) = state.terrain().compute_climate();
            vec![Event::ClimateComputed(moist, temp)]
        }
        Command::PlanRivers => {
            vec![Event::RiversPlanned(state.terrain().plan_river_paths())]
        }
        Command::PlaceSettlements => {
            vec![Event::SettlementsPlaced(
                state.terrain().plan_settlement_anchors(),
            )]
        }
        Command::PlanRoads => {
            vec![Event::RoadsPlanned(state.terrain().plan_road_paths())]
        }
        Command::ClassifyLandform => {
            vec![Event::LandformClassified(classify_landform(
                &state.grid,
                state.terrain(),
            ))]
        }
        Command::ClassifySlope => {
            let slope = classify_slope(&state.grid, state.terrain());
            let depth = classify_water_depth(&state.grid, state.cells.dense(), state.terrain());
            vec![Event::SlopeClassified(slope, depth)]
        }
        Command::ClassifyTiles => {
            let terrain = state.terrain();
            let grid = &state.grid;
            // Cover per CELL: the macro landform (already classified) sets the
            // base — high ground gets rock/snow, low ground gets a climate
            // biome — so a "Forest" is genuinely a forested LOWLAND, not a
            // steep slope that merely isn't labelled Mountain.
            let cells = (0..grid.cell_count())
                .map(|vi| classify_cover(grid, terrain, &state.landform, vi))
                .collect::<Vec<_>>()
                .into();
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
                &state.grid,
                state.terrain(),
                &state.cells,
                &state.slope_class,
            );
            vec![Event::FeaturesPainted(painted, roads)]
        }
        Command::ResolveTransitions => {
            let cells = resolve_transitions(&state.grid, state.terrain(), &state.cells).into();
            vec![Event::TransitionsResolved(cells)]
        }
        Command::MarkBlends => {
            vec![Event::BlendsMarked(mark_blends(
                &state.grid,
                &state.cells,
                &state.tiles,
                &state.painted,
            ))]
        }
        Command::SolveElevation => {
            let (field, iters, residual) = solve_elevation(
                &state.grid,
                state.terrain(),
                &state.cells,
                &state.landform,
                &state.tiles,
                &state.painted,
                &state.blends,
            );
            vec![Event::ElevationSolved {
                field,
                iters,
                residual,
            }]
        }
        Command::ClusterWater => {
            vec![Event::WaterClustered(water_surface_radii(
                &state.grid,
                state.terrain(),
                &state.cells,
            ))]
        }
        Command::BuildRegions => {
            let (regions, face_region) =
                build_regions(&state.grid, state.terrain(), &state.tiles, &state.painted);
            vec![Event::RegionsBuilt(regions, face_region)]
        }
        Command::SelectBridges => {
            let mut painted = state.painted.clone();
            let bridges = build_bridges(
                &state.grid,
                state.terrain(),
                &state.cells,
                &state.slope_class,
                &state.tiles,
                &state.face_region,
                &mut painted,
            );
            vec![Event::BridgesSelected(bridges, painted)]
        }
        Command::BuildMesh => {
            let (tris, cols) = build_mesh(
                &state.grid,
                state.terrain(),
                &state.cells,
                &state.painted,
                &state.water_depth,
                &state.landform,
                &state.slope_class,
            );
            vec![Event::MeshBuilt(tris, cols)]
        }
        Command::BuildTags => {
            let (off, data) = build_face_tags(&state.grid, &state.painted);
            vec![Event::TagsBuilt(off, data)]
        }
        Command::PlaceFlora => {
            vec![Event::FloraPlaced(place_flora(
                &state.grid,
                state.terrain(),
                &state.tiles,
                &state.painted,
                &state.mesh_tris,
            ))]
        }
        Command::PlaceStructures => {
            vec![Event::StructuresPlaced(place_structures(
                &state.grid,
                state.terrain(),
                &state.tiles,
                &state.painted,
                &state.slope_class,
                &state.mesh_tris,
            ))]
        }
        Command::BakeOutputs => {
            let road_material = (0..state.grid.face_count())
                .map(|fi| {
                    let solid = state
                        .grid
                        .face_cells(FaceId::new(fi))
                        .map(CellId::index)
                        .iter()
                        .filter(|&&cell| state.painted.roads.contains(CellId::new(cell)))
                        .count()
                        == 3;
                    if solid {
                        face_road_material(
                            &state.grid,
                            &state.cells,
                            &state.landform,
                            &state.slope_class,
                            fi,
                        )
                    } else {
                        0
                    }
                })
                .collect();
            vec![Event::OutputsBaked {
                river_r: river_surface_radii(
                    &state.grid,
                    &state.mesh_tris,
                    &state.tiles,
                    &state.water_r,
                ),
                slope: face_max(&state.grid, &state.slope_class),
                depth: face_max(&state.grid, &state.water_depth),
                landform: face_majority(&state.grid, &state.landform),
                road_material,
            }]
        }
    }
}

fn evolve(mut state: GenState, event: Event) -> GenState {
    match event {
        Event::TerrainInitialized(t) => state.terrain = Some(*t),
        Event::ElevationProposed(e) => {
            state
                .terrain
                .as_mut()
                .expect("terrain exists")
                .set_vert_elevations(e);
        }
        Event::ClimateComputed(moist, temp) => {
            state
                .terrain
                .as_mut()
                .expect("terrain exists")
                .set_climate(moist, temp);
        }
        Event::RiversPlanned(r) => {
            state.terrain.as_mut().expect("terrain exists").river_paths = r;
        }
        Event::SettlementsPlaced(a) => {
            state
                .terrain
                .as_mut()
                .expect("terrain exists")
                .settlement_anchors = a;
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
            state
                .terrain
                .as_mut()
                .expect("terrain exists")
                .set_vert_elevations(field);
        }
        Event::WaterClustered(r) => state.water_r = r.into(),
        Event::RegionsBuilt(r, fr) => {
            state.regions = r;
            state.face_region = fr.into();
        }
        Event::BridgesSelected(b, p) => {
            state.bridges = b;
            state.painted = p;
        }
        Event::MeshBuilt(t, c) => {
            state.mesh_tris = t.into();
            state.mesh_colors = c.into();
        }
        Event::TagsBuilt(off, data) => {
            state.tag_off = off;
            state.tag_data = data;
        }
        Event::FloraPlaced(f) => state.flora = f,
        Event::StructuresPlaced(v) => state.structures = v,
        Event::SlopeClassified(sc, wd) => {
            state.slope_class = sc.into();
            state.water_depth = wd.into();
        }
        Event::LandformClassified(lf) => state.landform = lf.into(),
        Event::OutputsBaked {
            river_r,
            slope,
            depth,
            landform,
            road_material,
        } => {
            state.river_r = river_r.into();
            state.face_slope_class = slope.into();
            state.face_water_depth = depth.into();
            state.face_landform = landform.into();
            state.face_road_material = road_material.into();
        }
    }
    state
}

fn react(event: &Event) -> Vec<Command> {
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
        Event::ElevationSolved { .. } => {
            vec![
                Command::ComputeClimate,
                Command::ClassifySlope,
                Command::ClusterWater,
            ]
        }
        Event::WaterClustered(_) => vec![],
        Event::SlopeClassified(..) => vec![Command::PaintFeatures],
        Event::FeaturesPainted(..) => vec![Command::BuildRegions],
        Event::RegionsBuilt(..) => vec![Command::SelectBridges],
        Event::BridgesSelected(..) => vec![Command::MarkBlends],
        Event::BlendsMarked(_) => vec![Command::BuildMesh],
        Event::MeshBuilt(..) => vec![Command::BuildTags],
        Event::TagsBuilt(..) => vec![Command::PlaceFlora],
        Event::FloraPlaced(_) => vec![Command::PlaceStructures],
        Event::StructuresPlaced(_) => vec![Command::BakeOutputs],
        Event::OutputsBaked { .. } => vec![],
    }
}

/// Run the full pipeline for a seed. `log` receives one line per event.
fn run_state(seed: u32, mut log: impl FnMut(&str)) -> GenState {
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

pub fn run(seed: u32, log: impl FnMut(&str)) -> CompletedWorld {
    let state = run_state(seed, log);
    CompletedWorld {
        level: state.to_level_data(),
        stats: state.stats(),
    }
}

mod classification_water_impl;
use classification_water_impl::*;

mod features_bridges_impl;
use features_bridges_impl::*;

mod regions_impl;
use regions_impl::*;

mod surface_placement_impl;
use surface_placement_impl::*;

mod elevation_impl;
use elevation_impl::*;

// ---- helpers ----

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solved_field_invariants() {
        let state = run_state(1337, |_| {});
        let terrain = state.terrain.as_ref().unwrap();

        // Rivers descend monotonically on the SOLVED field.
        assert!(!terrain.river_paths.is_empty(), "no rivers generated");
        let spring_components =
            cluster_cell_types(&state.grid, &state.cells, &[Terrain::RiverSpring]);
        assert_eq!(
            spring_components.count(),
            terrain.river_paths.len(),
            "every planned river has one connected ground-contact spring"
        );
        for (ri, path) in terrain.river_paths.iter().enumerate() {
            let mut prev = f32::MAX;
            for p in path {
                let e = terrain.elevation_at(*p);
                if prev < -0.05 {
                    break; // reached the sea — below the surface it's ocean
                }
                assert!(
                    e <= prev + 0.02,
                    "river {ri} flows uphill: {e} after {prev}"
                );
                prev = prev.min(e);
            }
        }

        // Settlements stay dry.
        for a in &terrain.settlement_anchors {
            assert!(
                terrain.elevation_at(*a) > 0.0,
                "settlement anchor under water"
            );
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
        let blend_of: std::collections::BTreeMap<u32, (u8, u8)> = state
            .blends
            .iter()
            .map(|&(fi, a, b)| (fi, (a, b)))
            .collect();
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
            let Some(fi) = state.grid.planet.face_at(terrain.vert_dir(vi)) else {
                continue;
            };
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
                owners[vi],
                e[vi]
            );
        }

        // No wall at the waterline: edges crossing sea level stay gentle, and
        // no edge anywhere jumps more than 0.5 (~250m over ~70m).
        let mut worst_cross = 0.0f32;
        let mut worst_any = 0.0f32;
        for a in 0..terrain.vert_count() {
            for &b in terrain.adj_of(a) {
                if b <= a {
                    continue;
                }
                let d = (e[a] - e[b]).abs();
                worst_any = worst_any.max(d);
                // Cliff and mountain coasts are deliberately steep sea walls.
                let steep_coast = |vi: usize| {
                    canyon[vi]
                        || state
                            .grid
                            .planet
                            .face_at(terrain.vert_dir(vi))
                            .is_some_and(|fi| {
                                matches!(
                                    state.tiles.dense()[fi],
                                    Terrain::Cliff | Terrain::Mountain | Terrain::Snow
                                )
                            })
                };
                if (e[a] >= 0.0) != (e[b] >= 0.0) && !steep_coast(a) && !steep_coast(b) {
                    worst_cross = worst_cross.max(d);
                }
            }
        }
        assert!(worst_cross <= 0.30, "waterline wall: {worst_cross}");
        assert!(worst_any <= 0.60, "extreme edge: {worst_any}");

        // A lake never rises above its shore (solver step 3a) — otherwise the flat
        // water surface floats over ground where a lake tile pokes up past the rim.
        for vi in 0..terrain.vert_count() {
            if owners[vi] != Terrain::Lake {
                continue;
            }
            for &nb in terrain.adj_of(vi) {
                if owners[nb] == Terrain::LakeShore {
                    assert!(
                        e[vi] <= e[nb] + 1e-3,
                        "lake vert {vi} ({}) above its shore {nb} ({})",
                        e[vi],
                        e[nb]
                    );
                }
            }
        }

        // No enclosed water body smaller than the minimum (no 1-cell lakes).
        let components = state.grid.topology.cell_components(|cell| {
            state.cells.dense()[cell.index()].is_water()
                && !matches!(
                    state.cells.dense()[cell.index()],
                    Terrain::River | Terrain::RiverSpring
                )
        });
        let mut sizes = vec![0usize; components.count()];
        let mut is_ocean = vec![false; components.count()];
        for v in 0..state.grid.cell_count() {
            let cell = state.grid.topology.cell(v).unwrap();
            let Some(c) = components.cell(cell).map(|component| component.index()) else {
                continue;
            };
            sizes[c] += 1;
            if cell_zone(&state.grid, terrain, v) == crate::zones::ZoneKind::Ocean {
                is_ocean[c] = true;
            }
        }
        for c in 0..components.count() {
            assert!(
                is_ocean[c] || sizes[c] >= size_range(Terrain::Lake).0,
                "enclosed water body of only {} cells survived",
                sizes[c]
            );
        }

        // Deep water never surfaces: every abyss-depth cell solves well below
        // the waterline (the depth class replaces the old DeepOcean tile).
        // (Depth is per grid CELL — sample the solved field at the cell.)
        for vi in 0..state.grid.cell_count() {
            if state.water_depth.dense()[vi] == DEPTH_ABYSS {
                let d = terrain.elevation_at(state.grid.cell_position(CellId::new(vi)));
                assert!(d < -0.1, "abyss cell {vi} not deep: {d}");
            }
        }

        // Roads never sit on water: checked per cell (painting is per cell)
        // and per solid road face.
        for vi in 0..state.grid.cell_count() {
            if state.painted.roads.contains(CellId::new(vi)) {
                assert!(
                    state.cells.dense()[vi].is_land(),
                    "road painted on water cell {vi}"
                );
            }
        }
        for fi in 0..state.grid.face_count() {
            let solid = state
                .grid
                .face_cells(FaceId::new(fi))
                .map(CellId::index)
                .iter()
                .filter(|&&cell| state.painted.roads.contains(CellId::new(cell)))
                .count()
                == 3;
            if solid {
                assert!(
                    state.tiles.dense()[fi].is_land(),
                    "solid road face on water tile {fi}"
                );
            }
        }

        // Bridge entries land on walkable ground: the deck grounds at ground
        // height (no vertical gap, by the deck builder) and the player steps
        // onto it from gentle inland terrain. (An omnidirectional slope test is
        // meaningless here — every deck end sits at a water's edge, so the bank
        // drop toward the water is steep by nature; the approach is inland.)
        let bridge_walkable = |t: Terrain| {
            matches!(
                t,
                Terrain::Plains
                    | Terrain::Forest
                    | Terrain::Savanna
                    | Terrain::Tundra
                    | Terrain::Desert
                    | Terrain::Jungle
                    | Terrain::Swamp
            )
        };
        for span in &state.bridges {
            for end in [span.first(), span.last()].into_iter().flatten() {
                let fi = state
                    .grid
                    .planet
                    .face_at(end.0)
                    .expect("deck end on a face");
                assert!(
                    bridge_walkable(state.tiles.dense()[fi]),
                    "bridge entry on non-walkable tile {:?}",
                    state.tiles.dense()[fi]
                );
                // The anchor CELL (where placement gated on the solved slope)
                // is genuinely gentle — a bridge never lands on steep ground,
                // whatever the biome. (An omnidirectional slope at the exact
                // water's-edge end would just read the natural bank drop.)
                let cell = state
                    .grid
                    .face_cells(FaceId::new(fi))
                    .map(CellId::index)
                    .into_iter()
                    .max_by(|&a, &b| {
                        state
                            .grid
                            .cell_direction(CellId::new(a))
                            .dot(end.0)
                            .partial_cmp(&state.grid.cell_direction(CellId::new(b)).dot(end.0))
                            .unwrap()
                    })
                    .unwrap();
                let slope = terrain.slope(state.grid.cell_position(CellId::new(cell)));
                assert!(slope < 0.3, "bridge anchor on steep ground: slope {slope}");
            }
        }

        // Tile identity lives on hex cells (cells can't pinch), and the
        // LINKING RULE covers the derived faces: around any cell, same-type
        // faces form ONE edge-connected fan — never linked by a lone vertex.
        // Also checked: every face's type is one of its corner cells, and
        // solid feature faces obey the same fan rule.
        {
            for fi in 0..state.grid.face_count() {
                let corners = state.grid.face_cells(FaceId::new(fi)).map(CellId::index);
                assert!(
                    corners
                        .iter()
                        .any(|&cell| state.cells.dense()[cell] == state.tiles.dense()[fi]),
                    "face {fi} derived {:?} not among its corner cells",
                    state.tiles.dense()[fi]
                );
            }
            let mut tile_pinches = 0usize;
            let mut road_pinches = 0usize;
            for v in 0..state.grid.cell_count() {
                let ring = ring(&state.grid, v);
                let n = ring.len();
                let ts: Vec<Terrain> = (0..n)
                    .map(|i| {
                        match classification::derive_face(
                            state.cells.dense()[v],
                            state.cells.dense()[ring[i]],
                            state.cells.dense()[ring[(i + 1) % n]],
                        ) {
                            // Spring is a river source marker, not a separate
                            // traversable band; validate it as part of the river
                            // water band for the no-pinch invariant.
                            Terrain::RiverSpring => Terrain::River,
                            t => t,
                        }
                    })
                    .collect();
                let mut types = ts.clone();
                types.sort_by_key(|t| *t as u8);
                types.dedup();
                for t in types {
                    let runs = (0..n)
                        .filter(|&i| ts[i] == t && ts[(i + n - 1) % n] != t)
                        .count();
                    if runs >= 2 {
                        tile_pinches += 1;
                    }
                }
                if state.painted.roads.contains(CellId::new(v)) {
                    let solid = |i: usize| {
                        state.painted.roads.contains(CellId::new(ring[i]))
                            && state.painted.roads.contains(CellId::new(ring[(i + 1) % n]))
                    };
                    let runs = (0..n)
                        .filter(|&i| solid(i) && !solid((i + n - 1) % n))
                        .count();
                    if runs >= 2 {
                        road_pinches += 1;
                    }
                }
            }
            assert_eq!(tile_pinches, 0, "same-type faces linked by a lone vertex");
            assert_eq!(road_pinches, 0, "road strip pinched at a vertex");

            // Lake-to-ocean distance ≥ 10 edge steps.
            // Actual ocean TILES, not ocean-zone: an isolated ocean-zone
            // pocket is reclassified to a lake (see size_range(Ocean)) and
            // must not seed the distance field.
            let ocean_faces: Vec<_> = state
                .grid
                .topology
                .faces()
                .filter(|face| state.tiles.dense()[face.index()] == Terrain::Ocean)
                .collect();
            let ocean_distance = state.grid.topology.face_distances(&ocean_faces, 10);
            for fi in 0..state.grid.face_count() {
                if state.tiles.dense()[fi] == Terrain::Lake {
                    let face = state
                        .grid
                        .topology
                        .face(fi)
                        .expect("face index from topology range");
                    assert!(
                        ocean_distance.face_steps(face).is_none(),
                        "lake face {fi} is within 10 tiles of ocean water",
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
                let hinterland = terrain
                    .adj_of(a)
                    .iter()
                    .filter(|&&nb| {
                        vkind[nb].is_land() && vkind[nb] != Terrain::Cliff && !shore_kind(vkind[nb])
                    })
                    .map(|&nb| e[nb])
                    .fold(f32::MIN, f32::max);
                if hinterland > f32::MIN {
                    assert!(
                        e[a] <= hinterland + 0.05,
                        "cliff crest sticks out: {} above hinterland {}",
                        e[a],
                        hinterland
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
                        vkind[vi],
                        e[vi],
                        vkind[nb],
                        e[nb].max(0.0)
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
                if terrain
                    .adj_of(vi)
                    .iter()
                    .all(|&nb| vkind[nb] == Terrain::Lake)
                {
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
                    avg(&interior),
                    avg(&edge)
                );
            }
        }

        // Blend marks link two differing plain kinds actually adjacent there.
        assert!(!state.blends.is_empty(), "no blends marked");
        for &(fi, a, b) in &state.blends {
            assert_ne!(a, b);
            assert_eq!(
                state.tiles.dense()[fi as usize] as u8,
                a,
                "blend face kind mismatch"
            );
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
                assert!(
                    d < 200.0,
                    "kernel vert {d:.0}m away at lat {:.0}",
                    p.0.y.asin().to_degrees()
                );
            }
        }
    }

    #[test]
    fn tile_type_sets_cluster_mixed_connected_groups() {
        let grid = Grid::new(1);

        let cell_neighbor = grid.cell_neighbors(CellId::new(0))[0].index();
        let cell_distant = (0..grid.cell_count())
            .find(|&v| {
                v != 0
                    && v != cell_neighbor
                    && !grid
                        .cell_neighbors(CellId::new(0))
                        .iter()
                        .any(|cell| cell.index() == v)
                    && !grid
                        .cell_neighbors(CellId::new(cell_neighbor))
                        .iter()
                        .any(|cell| cell.index() == v)
            })
            .expect("grid needs a non-adjacent cell");
        let mut cells = vec![Terrain::Plains; grid.cell_count()];
        cells[0] = Terrain::Forest;
        cells[cell_neighbor] = Terrain::Mountain;
        cells[cell_distant] = Terrain::Snow;
        let cell_components = cluster_cell_types(
            &grid,
            &cells,
            &[
                Terrain::Forest,
                Terrain::Mountain,
                Terrain::Snow,
                Terrain::Forest,
            ],
        );
        assert_eq!(cell_components.count(), 2);
        let cell = |index| grid.topology.cell(index).unwrap();
        assert_eq!(
            cell_components.cell(cell(0)),
            cell_components.cell(cell(cell_neighbor))
        );
        assert_ne!(
            cell_components.cell(cell(0)),
            cell_components.cell(cell(cell_distant))
        );
        let cell_nonmember = cells.iter().position(|&t| t == Terrain::Plains).unwrap();
        assert_eq!(cell_components.cell(cell(cell_nonmember)), None);

        let face_neighbor = grid.face_neighbors(FaceId::new(0)).map(FaceId::index)[0];
        let face_distant = (0..grid.face_count())
            .find(|&f| {
                f != 0
                    && f != face_neighbor
                    && !grid
                        .face_neighbors(FaceId::new(0))
                        .map(FaceId::index)
                        .contains(&f)
                    && !grid
                        .face_neighbors(FaceId::new(face_neighbor))
                        .map(FaceId::index)
                        .contains(&f)
            })
            .expect("grid needs a non-adjacent face");
        let mut face_types = vec![Terrain::Plains; grid.face_count()];
        face_types[0] = Terrain::Beach;
        face_types[face_neighbor] = Terrain::Cliff;
        face_types[face_distant] = Terrain::Ocean;
        let face_components = cluster_face_types(
            &grid,
            &face_types,
            &[Terrain::Beach, Terrain::Cliff, Terrain::Ocean],
        );
        assert_eq!(face_components.count(), 2);
        let face = |index| grid.topology.face(index).unwrap();
        assert_eq!(
            face_components.face(face(0)),
            face_components.face(face(face_neighbor))
        );
        assert_ne!(
            face_components.face(face(0)),
            face_components.face(face(face_distant))
        );
        let face_nonmember = face_types
            .iter()
            .position(|&t| t == Terrain::Plains)
            .unwrap();
        assert_eq!(face_components.face(face(face_nonmember)), None);

        let empty_components = cluster_cell_types(&grid, &cells, &[]);
        assert_eq!(empty_components.count(), 0);
        assert!(
            grid.topology
                .cells()
                .all(|cell| empty_components.cell(cell).is_none())
        );
    }

    #[test]
    fn river_surface_starts_on_springs_and_joins_body_water() {
        let state = run_state(1337, |_| {});
        let river_r =
            river_surface_radii(&state.grid, &state.mesh_tris, &state.tiles, &state.water_r);
        let key = |p: [f32; 3]| [p[0].to_bits(), p[1].to_bits(), p[2].to_bits()];

        let mut spring_corners = 0usize;
        let mut outlet_corners = 0usize;
        for (fi, face_river_r) in river_r.iter().enumerate().take(state.grid.face_count()) {
            if state.tiles.dense()[fi] == Terrain::RiverSpring {
                for (corner, &radius) in face_river_r.iter().enumerate() {
                    spring_corners += 1;
                    let ground = Vec3::from_array(state.mesh_tris.dense()[fi][corner]).length();
                    assert!(
                        (radius - (ground - RIVER_TERRAIN_CLIP)).abs() < 1e-3,
                        "spring water must start embedded in the terrain"
                    );
                }
            }
            if !matches!(
                state.tiles.dense()[fi],
                Terrain::River | Terrain::RiverSpring | Terrain::RiverBank
            ) {
                continue;
            }
            for neighbor in state
                .grid
                .face_neighbors(FaceId::new(fi))
                .map(FaceId::index)
            {
                let waterline = state.water_r.dense()[neighbor];
                if waterline <= 0.0 {
                    continue;
                }
                for (corner, &radius) in face_river_r.iter().enumerate() {
                    if state.mesh_tris.dense()[neighbor]
                        .iter()
                        .any(|&other| key(other) == key(state.mesh_tris.dense()[fi][corner]))
                    {
                        outlet_corners += 1;
                        assert!(
                            (radius - waterline).abs() < 1e-3,
                            "river outlet must share its neighboring waterline"
                        );
                    }
                }
            }
        }
        assert!(spring_corners > 0, "seed must include Spring faces");
        assert!(
            outlet_corners > 0,
            "seed must include water-connected river outlets"
        );
    }

    #[test]
    fn lakes_stay_enclosed() {
        // No lake-zone water may connect to the ocean — the rim dam guarantees
        // every lake is its own body (guards the drain-channel bug where lakes
        // leaked to the sea along coarse-face edges and became ocean inlets).
        let state = run_state(1337, |_| {});
        let terrain = state.terrain.as_ref().unwrap();
        let components = state.grid.topology.face_components(|face| {
            state.tiles.dense()[face.index()].is_water()
                && !matches!(
                    state.tiles.dense()[face.index()],
                    Terrain::River | Terrain::RiverSpring
                )
        });
        let mut sizes = vec![0usize; components.count()];
        let mut has_lake = vec![false; components.count()];
        let mut has_ocean = vec![false; components.count()];
        for fi in 0..state.grid.face_count() {
            let face = state.grid.topology.face(fi).unwrap();
            let Some(c) = components.face(face).map(|component| component.index()) else {
                continue;
            };
            sizes[c] += 1;
            match terrain.zones().kind_at_fine(fi) {
                crate::zones::ZoneKind::Lake => has_lake[c] = true,
                crate::zones::ZoneKind::Ocean => has_ocean[c] = true,
                _ => {}
            }
        }
        let mut lake_faces = 0;
        for c in 0..components.count() {
            if has_lake[c] {
                lake_faces += sizes[c];
                assert!(
                    !has_ocean[c],
                    "lake body of {} faces connects to the ocean",
                    sizes[c]
                );
            }
        }
        assert!(
            lake_faces > 100,
            "lakes nearly vanished: {lake_faces} faces"
        );
    }

    #[test]
    fn rivers_reach_the_sea() {
        // Every river must join a larger water body — no thin terrain band
        // may cut a mouth off (guards the junction-face damming bug).
        let state = run_state(1337, |_| {});
        let mut visited = vec![false; state.grid.cell_count()];
        for start in 0..state.grid.cell_count() {
            if !matches!(
                state.cells.dense()[start],
                Terrain::River | Terrain::RiverSpring
            ) || visited[start]
            {
                continue;
            }
            let comp: Vec<_> = state
                .grid
                .topology
                .cell_component(
                    state
                        .grid
                        .topology
                        .cell(start)
                        .expect("cell index from topology range"),
                    |cell| {
                        matches!(
                            state.cells.dense()[cell.index()],
                            Terrain::River | Terrain::RiverSpring
                        )
                    },
                )
                .into_iter()
                .map(|cell| cell.index())
                .collect();
            for &cell in &comp {
                visited[cell] = true;
            }
            let touches_sea = comp.iter().any(|&vi| {
                state.grid.cell_neighbors(CellId::new(vi)).iter().any(|nb| {
                    matches!(
                        state.cells.dense()[nb.index()],
                        Terrain::Ocean | Terrain::Lake
                    )
                })
            });
            assert!(
                touches_sea,
                "river component of {} cells cut off from any water body",
                comp.len()
            );
            // And the mouth is open at FACE level too: some River face is
            // edge-adjacent to an Ocean/Lake face.
            let mut open = false;
            'faces: for fi in 0..state.grid.face_count() {
                if !matches!(
                    state.tiles.dense()[fi],
                    Terrain::River | Terrain::RiverSpring
                ) {
                    continue;
                }
                if !state
                    .grid
                    .face_cells(FaceId::new(fi))
                    .map(CellId::index)
                    .iter()
                    .any(|cell| comp.contains(cell))
                {
                    continue;
                }
                for nb in state
                    .grid
                    .face_neighbors(FaceId::new(fi))
                    .map(FaceId::index)
                {
                    if matches!(state.tiles.dense()[nb], Terrain::Ocean | Terrain::Lake) {
                        open = true;
                        break 'faces;
                    }
                }
            }
            assert!(
                open,
                "river mouth dammed at face level ({} cells)",
                comp.len()
            );
        }
    }

    #[test]
    fn mesh_is_watertight() {
        // Every fall-through bug is a crack: an edge used by only one triangle
        // is a hole the player can drop through. On a closed surface each edge
        // is shared by EXACTLY two faces, and every vertex fan is a full ring.
        // Assert both on the baked mesh (the collider is built from it).
        let state = run_state(1337, |_| {});
        let grid = &state.grid;

        // (a) each undirected edge belongs to exactly two faces.
        let mut edge_faces: BTreeMap<(usize, usize), u32> = BTreeMap::new();
        for fi in 0..grid.face_count() {
            let idx = grid.face_cells(FaceId::new(fi)).map(CellId::index);
            for k in 0..3 {
                let (a, b) = (idx[k], idx[(k + 1) % 3]);
                let key = if a < b { (a, b) } else { (b, a) };
                *edge_faces.entry(key).or_default() += 1;
            }
        }
        for (&(a, b), &n) in &edge_faces {
            assert_eq!(n, 2, "edge ({a},{b}) shared by {n} faces (not 2) — a crack");
        }

        // (b) every cell's faces form ONE closed fan: walking face→face
        // across shared edges visits all of them and returns. A cell whose
        // fan splits is a pinhole even if each edge is shared twice.
        for v in 0..grid.cell_count() {
            let cell = grid
                .topology
                .cell(v)
                .expect("cell index from topology range");
            let faces = grid.topology.cell_faces(cell);
            let n = faces.len();
            assert!((5..=6).contains(&n), "cell {v} has {n} faces");
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
                    let fi = grid.face_cells(faces[i]).map(CellId::index);
                    let fj = grid.face_cells(faces[j]).map(CellId::index);
                    let shared = fi.iter().filter(|x| fj.contains(x)).count();
                    if shared == 2 {
                        seen[j] = true;
                        count += 1;
                        stack.push(j);
                    }
                }
            }
            assert_eq!(count, n, "cell {v} fan is not one closed ring — a pinhole");
        }
    }

    #[test]
    fn deterministic_pipeline() {
        let a = run_state(42, |_| {});
        let b = run_state(42, |_| {});
        assert_eq!(
            a.terrain.unwrap().vert_elevations(),
            b.terrain.unwrap().vert_elevations()
        );
        assert_eq!(a.cells, b.cells);
        assert_eq!(a.tiles, b.tiles);
        assert_eq!(a.regions.len(), b.regions.len());
        assert_eq!(a.flora.len(), b.flora.len());
        assert!(
            a.flora
                .iter()
                .zip(&b.flora)
                .all(|(x, y)| x.pos == y.pos && x.kind == y.kind)
        );
        assert_eq!(a.structures.len(), b.structures.len());
        assert!(
            a.structures
                .iter()
                .zip(&b.structures)
                .all(|(x, y)| x.pos == y.pos && x.kind == y.kind)
        );
        assert_eq!(a.slope_class, b.slope_class);
        assert_eq!(a.water_depth, b.water_depth);
        assert_eq!(a.landform, b.landform);
    }

    fn serialized_fingerprint(bytes: &[u8]) -> u64 {
        bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
        })
    }

    #[test]
    fn locked_serialized_worlds() {
        let seed_1337 = postcard::to_allocvec(run(1337, |_| {}).level_data()).unwrap();
        assert_eq!(
            seed_1337.as_slice(),
            include_bytes!("../../main/assets/level_1337.bin")
        );
        assert_eq!(serialized_fingerprint(&seed_1337), 0xefb3_6ef5_28ab_ac4e);

        let seed_42 = postcard::to_allocvec(run(42, |_| {}).level_data()).unwrap();
        assert_eq!(serialized_fingerprint(&seed_42), 0x713a_71fe_65e0_c310);
    }

    #[test]
    fn flora_stays_off_water_and_features() {
        let state = run_state(1337, |_| {});
        assert!(
            state.flora.len() > 1000,
            "flora nearly absent: {}",
            state.flora.len()
        );
        for f in &state.flora {
            let fi = f.face as usize;
            assert!(
                state.tiles.dense()[fi].is_land(),
                "flora on water face {fi}"
            );
            for bits in [
                &state.painted.roads,
                &state.painted.towns,
                &state.painted.bridge_entries,
            ] {
                assert_eq!(
                    painted_corners(&state.grid, bits, fi),
                    0,
                    "flora on a feature face {fi}"
                );
            }
        }
    }
}
