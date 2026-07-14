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
use std::collections::BTreeMap;

use crate::level::{
    BlendTarget, FaceBlend, FaceTag, FloraData, FloraKind, Landform, LevelData, RegionData,
    RegionKind, RoadData, RoadKind, RoadMaterial, SettlementData, SlopeClass, StructureData,
    StructureKind, WaterDepth,
};
use crate::sphere::SpherePos;
use crate::terrain::{Terrain, TerrainGen};
#[cfg(test)]
use crate::topology::FaceComponentId;
use crate::topology::{CellComponentId, CellId, ComponentLabels, FaceId};
use crate::wfc;

mod classification;
mod domain;
mod elevation;
mod features;
mod grid;
mod pipeline;
mod projection;
mod regions;
mod router;

use domain::{CellField, CellSet, FaceField};
use features::{
    absorb_small_clusters, build_bridges, mark_blends, paint_features, resolve_transitions,
};
use grid::Grid;
use pipeline::run_state;
use projection::{face_majority, face_max};

const TOWN_RADIUS: f32 = 55.0;

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
fn ring(grid: &Grid, cell: CellId) -> Vec<CellId> {
    let mut out = Vec::with_capacity(grid.cell_neighbors(cell).len());
    let mut cur = grid.cell_neighbors(cell)[0];
    out.push(cur);
    loop {
        let next = grid.topology.cell_faces(cell).iter().find_map(|&face| {
            let cells = grid.face_cells(face);
            if !cells.contains(&cur) {
                return None;
            }
            let third = *cells
                .iter()
                .find(|&&candidate| candidate != cell && candidate != cur)?;
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
    debug_assert_eq!(out.len(), grid.cell_neighbors(cell).len());
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
    fn potential_at(grid: &Grid, cells: &[Terrain], cell: CellId) -> usize {
        let ring = ring(grid, cell);
        let n = ring.len();
        let ts: Vec<Terrain> = (0..n)
            .map(|i| {
                classification::derive_face(
                    cells[cell.index()],
                    cells[ring[i].index()],
                    cells[ring[(i + 1) % n].index()],
                )
            })
            .collect();
        let mut types = ts.clone();
        types.sort_by_key(|terrain| classification::terrain_rank(*terrain));
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
    let local = |cells: &[Terrain], cell: CellId| -> usize {
        potential_at(grid, cells, cell)
            + ring(grid, cell)
                .iter()
                .map(|&neighbor| potential_at(grid, cells, neighbor))
                .sum::<usize>()
    };
    for it in 0..64 {
        debug_assert!(it < 63, "link_tile_pinches did not converge");
        let mut changed = false;
        for cell in grid.topology.cells() {
            if potential_at(grid, cells, cell) == 0 {
                continue;
            }
            let ring_v = ring(grid, cell);
            let ts: Vec<Terrain> = {
                let n = ring_v.len();
                (0..n)
                    .map(|i| {
                        classification::derive_face(
                            cells[cell.index()],
                            cells[ring_v[i].index()],
                            cells[ring_v[(i + 1) % n].index()],
                        )
                    })
                    .collect()
            };
            let mut types = ts;
            types.sort_by_key(|terrain| classification::terrain_rank(*terrain));
            types.dedup();
            // Prefer a class-preserving retype; fall back to crossing the
            // waterline only when nothing else merges the runs.
            'candidates: for cross in [false, true] {
                for t in &types {
                    let t = *t;
                    for &candidate in &ring_v {
                        // Rivers are planned linear features two cells wide —
                        // consuming a cell can sever the channel.
                        if cells[candidate.index()] == t
                            || matches!(
                                cells[candidate.index()],
                                Terrain::River | Terrain::RiverSpring
                            )
                            || (cells[candidate.index()].is_water() != t.is_water()) != cross
                        {
                            continue;
                        }
                        // Turning land into water must extend an existing
                        // body of that kind, never strand a puddle.
                        if cross
                            && t.is_water()
                            && !grid
                                .cell_neighbors(candidate)
                                .iter()
                                .any(|nb| cells[nb.index()] == t)
                        {
                            continue;
                        }
                        let before = local(cells, candidate);
                        let old = cells[candidate.index()];
                        cells[candidate.index()] = t;
                        if local(cells, candidate) < before {
                            changed = true;
                            break 'candidates;
                        }
                        cells[candidate.index()] = old;
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
fn link_feature_pinches(grid: &Grid, bits: &mut CellSet, passable: impl Fn(CellId) -> bool) {
    for _ in 0..8 {
        let mut changed = false;
        for cell in grid.topology.cells() {
            if !bits.contains(cell) {
                continue;
            }
            let ring = ring(grid, cell);
            let n = ring.len();
            let runs = |bits: &CellSet| -> usize {
                let solid = |i: usize| bits.contains(ring[i]) && bits.contains(ring[(i + 1) % n]);
                (0..n)
                    .filter(|&i| solid(i) && !solid((i + n - 1) % n))
                    .count()
            };
            let r = runs(bits);
            if r < 2 {
                continue;
            }
            for &candidate in ring.iter().take(n) {
                if bits.contains(candidate) || !passable(candidate) {
                    continue;
                }
                bits.insert(candidate);
                if runs(bits) < r {
                    changed = true;
                    break;
                }
                bits.remove(candidate);
            }
        }
        if !changed {
            break;
        }
    }
}

// ---- typed domain storage ----

/// Built features, painted per CELL (like terrain identity): a face is a
/// solid feature where ≥2 of its corner cells are painted, and fades out at
/// the edges through per-corner colors — the same construction as terrain,
/// so feature footprints can never pinch or zigzag either.
#[derive(Clone)]
struct Painted {
    pub roads: CellSet,
    pub towns: CellSet,
    pub bridges: CellSet,
    pub bridge_entries: CellSet,
}

impl Painted {
    fn empty(nv: usize) -> Self {
        Self {
            roads: CellSet::new(nv),
            towns: CellSet::new(nv),
            bridges: CellSet::new(nv),
            bridge_entries: CellSet::new(nv),
        }
    }
}

/// How many of a face's corner cells are in the set.
fn painted_corners(grid: &Grid, bits: &CellSet, face: FaceId) -> usize {
    features::painted_corners(grid, bits, face)
}

/// A face is a solid feature surface only when the feature owns ALL its
/// corners: for a band painted as two parallel lattice lines that is exactly
/// the parallelogram strip between them (straight edges = the lines
/// themselves). Faces with 1–2 painted corners form one straight-edged strip
/// on each side — the blend band, rendered as a per-corner gradient.
fn face_solid(grid: &Grid, bits: &CellSet, face: FaceId) -> bool {
    features::face_solid(grid, bits, face)
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
    pub blends: Vec<FaceBlend>,
    pub regions: Vec<RegionData>,
    pub face_region: FaceField<Option<u32>>,
    /// Per-face water-surface radius (0.0 = dry), clustered once (sea + lakes).
    pub water_r: FaceField<f32>,
    /// Generation-baked per-corner river surface. Runtime and serializers do
    /// not derive river topology.
    pub river_r: FaceField<[f32; 3]>,
    pub mesh_tris: FaceField<[[f32; 3]; 3]>,
    pub mesh_colors: FaceField<[[f32; 4]; 3]>,
    pub face_tags: FaceField<Vec<FaceTag>>,
    pub flora: Vec<FloraData>,
    pub structures: Vec<StructureData>,
    /// Per-cell terrain steepness (0 Flat, 1 Gentle, 2 Steep, 3 Cliff) from the
    /// SOLVED field. Walkability and feature placement gate on this, so a
    /// mountain pass (Gentle inside Mountains) is traversable and a "flat"
    /// biome that solved steep is not.
    pub slope_class: CellField<SlopeClass>,
    /// Per-cell water depth class; `None` on dry cells.
    pub water_depth: CellField<Option<WaterDepth>>,
    /// Per-cell macro landform from the proposed field.
    pub landform: CellField<Landform>,
    /// Final cell-to-face projections consumed by `LevelData`.
    pub face_slope_class: FaceField<SlopeClass>,
    pub face_water_depth: FaceField<Option<WaterDepth>>,
    pub face_landform: FaceField<Landform>,
    pub face_road_material: FaceField<Option<RoadMaterial>>,
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
            face_tags: FaceField::default(),
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
                kind: RoadKind::Road,
            })
            .collect();
        roads.extend(self.bridges.iter().map(|path| RoadData {
            points: path.iter().map(|point| point.0.to_array()).collect(),
            kind: RoadKind::Bridge,
        }));

        LevelData {
            seed: self.grid.seed,
            vert_elev: terrain.vert_elevations().to_vec(),
            terrain_tris: self.mesh_tris.to_vec(),
            terrain_colors: self.mesh_colors.to_vec(),
            unit_tris: self
                .grid
                .unit_tris
                .iter()
                .map(|triangle| triangle.map(|point| point.to_array()))
                .collect(),
            face_types: self.tiles.to_vec(),
            face_water_r: self.water_r.to_vec(),
            face_river_r: self.river_r.to_vec(),
            face_tags: self.face_tags.to_vec(),
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
        for &kind in self.tiles.iter() {
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

pub fn run(seed: u32, log: impl FnMut(&str)) -> CompletedWorld {
    let state = run_state(seed, log);
    CompletedWorld {
        level: state.to_level_data(),
        stats: state.stats(),
    }
}

mod water;
use water::*;

mod surface;
use surface::*;

use elevation::generation::*;
use regions::generation::*;

#[cfg(test)]
mod tests;
