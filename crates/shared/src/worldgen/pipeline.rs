use std::collections::VecDeque;

use super::network;
use super::{
    CellField, CellSet, FaceBlend, FaceId, FaceTag, FloraData, GenState, Landform, Painted,
    RegionData, RegionMemberships, RoadMaterial, RoadPath, SlopeClass, SpherePos, StructureData,
    SurfaceCondition, Terrain, TerrainGen, WaterDepth, WaterPhase, build_bridges, build_face_tags,
    build_mesh, build_regions, classify_cover, classify_landform, classify_slope,
    classify_water_depth, derive_tiles, face_majority, face_max, face_road_material, mark_blends,
    normalize_water_bodies, paint_features, paint_rivers, place_flora, place_structures,
    resolve_transitions, river_surface_radii, solve_elevation, water_surface_radii,
};

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
    /// Named geographic and built-feature clusters.
    BuildRegions,
    /// Bridge spans plus their road access connections.
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
    /// Choose settlement faces whose town footprints can carry local roads.
    SelectSettlementSites,
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
    FeaturesPainted(Painted, Vec<RoadPath>),
    TransitionsResolved(CellField<Terrain>),
    BlendsMarked(Vec<FaceBlend>),
    ElevationSolved {
        field: Vec<f32>,
        iters: usize,
        residual: f32,
    },
    WaterClustered(Vec<f32>),
    RegionsBuilt(Vec<RegionData>, RegionMemberships),
    BridgesSelected(
        Vec<Vec<SpherePos>>,
        Painted,
        Vec<RoadPath>,
        super::network::RoadGraph,
    ),
    MeshBuilt(Vec<[[f32; 3]; 3]>, Vec<[[f32; 4]; 3]>),
    TagsBuilt(Vec<Vec<FaceTag>>),
    FloraPlaced(Vec<FloraData>),
    StructuresPlaced(Vec<StructureData>),
    SlopeClassified(Vec<SlopeClass>, Vec<Option<WaterDepth>>),
    SettlementSitesSelected(Vec<SpherePos>),
    LandformClassified(Vec<Landform>),
    OutputsBaked {
        river_r: Vec<[f32; 3]>,
        slope: Vec<SlopeClass>,
        depth: Vec<Option<WaterDepth>>,
        phase: Vec<Option<WaterPhase>>,
        surface_condition: Vec<SurfaceCondition>,
        landform: Vec<Landform>,
        road_material: Vec<Option<RoadMaterial>>,
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
            Event::TilesClassified(c) => {
                format!("cells classified: {}", c.as_slice().len())
            }
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
            Event::BridgesSelected(b, ..) => format!("bridges selected: {}", b.len()),
            Event::MeshBuilt(t, _) => format!("mesh built: {} tris", t.len()),
            Event::TagsBuilt(tags) => format!(
                "tags built: {} entries",
                tags.iter().map(Vec::len).sum::<usize>()
            ),
            Event::FloraPlaced(f) => format!("flora placed: {}", f.len()),
            Event::StructuresPlaced(v) => format!("structures placed: {}", v.len()),
            Event::SlopeClassified(sc, wd) => {
                let cliffs = sc.iter().filter(|&&c| c == SlopeClass::Cliff).count();
                let abyss = wd.iter().filter(|&&d| d == Some(WaterDepth::Abyss)).count();
                format!("relief classified: {cliffs} cliff cells, {abyss} abyss cells")
            }
            Event::SettlementSitesSelected(anchors) => {
                format!("settlement sites selected: {}", anchors.len())
            }
            Event::LandformClassified(lf) => {
                let mtn = lf
                    .iter()
                    .filter(|&&l| l == Landform::Mountains || l == Landform::Plateau)
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

fn decide(state: &GenState, cmd: &Command) -> Event {
    match cmd {
        Command::InitTerrain => {
            Event::TerrainInitialized(Box::new(TerrainGen::init(state.grid.seed)))
        }
        Command::ProposeElevation => Event::ElevationProposed(state.terrain().propose_elevation()),
        Command::ComputeClimate => {
            let (moist, temp) = state.terrain().compute_climate();
            Event::ClimateComputed(moist, temp)
        }
        Command::PlanRivers => Event::RiversPlanned(state.terrain().plan_river_paths()),
        Command::PlaceSettlements => {
            Event::SettlementsPlaced(state.terrain().plan_settlement_anchors())
        }
        Command::PlanRoads => Event::RoadsPlanned(state.terrain().plan_road_paths()),
        Command::ClassifyLandform => {
            Event::LandformClassified(classify_landform(&state.grid, state.terrain()))
        }
        Command::ClassifySlope => {
            let slope = classify_slope(&state.grid, state.terrain());
            let depth = classify_water_depth(&state.grid, state.cells.as_slice(), state.terrain());
            Event::SlopeClassified(slope, depth)
        }
        Command::SelectSettlementSites => Event::SettlementSitesSelected(
            network::select_settlement_sites(
                &state.grid,
                state.terrain(),
                state.cells.as_slice(),
                state.slope_class.as_slice(),
            )
            .unwrap_or_else(|error| panic!("settlement placement failed: {error}")),
        ),
        Command::ClassifyTiles => {
            let terrain = state.terrain();
            let grid = &state.grid;
            // Cover per CELL: the macro landform (already classified) sets the
            // base — high ground gets rock/snow, low ground gets a climate
            // biome — so a "Forest" is genuinely a forested LOWLAND, not a
            // steep slope that merely isn't labelled Mountain.
            let cells = (0..grid.cell_count())
                .map(|cell_index| {
                    classify_cover(grid, terrain, state.landform.as_slice(), cell_index)
                })
                .collect::<Vec<_>>()
                .into();
            Event::TilesClassified(cells)
        }
        Command::PaintRivers => {
            let mut cells = state.cells.clone();
            paint_rivers(&state.grid, state.terrain(), cells.as_mut_slice());
            Event::RiversPainted(cells)
        }
        Command::NormalizeWater => {
            let mut cells = state.cells.clone();
            normalize_water_bodies(&state.grid, state.terrain(), cells.as_mut_slice());
            Event::WaterNormalized(cells)
        }
        Command::PaintFeatures => {
            let (painted, roads) = paint_features(
                &state.grid,
                state.terrain(),
                state.cells.as_slice(),
                state.slope_class.as_slice(),
            );
            Event::FeaturesPainted(painted, roads)
        }
        Command::ResolveTransitions => {
            let cells =
                resolve_transitions(&state.grid, state.terrain(), state.cells.as_slice()).into();
            Event::TransitionsResolved(cells)
        }
        Command::MarkBlends => Event::BlendsMarked(mark_blends(
            &state.grid,
            state.cells.as_slice(),
            state.tiles.as_slice(),
            &state.painted,
        )),
        Command::SolveElevation => {
            let (field, iters, residual) = solve_elevation(
                &state.grid,
                state.terrain(),
                state.cells.as_slice(),
                state.landform.as_slice(),
                state.tiles.as_slice(),
                &state.painted,
                &state.blends,
            );
            Event::ElevationSolved {
                field,
                iters,
                residual,
            }
        }
        Command::ClusterWater => Event::WaterClustered(water_surface_radii(
            &state.grid,
            state.terrain(),
            state.cells.as_slice(),
        )),
        Command::BuildRegions => {
            let (regions, face_regions) = build_regions(
                &state.grid,
                state.terrain(),
                state.cells.as_slice(),
                state.landform.as_slice(),
                &state.painted,
                &state.network,
            );
            Event::RegionsBuilt(regions, face_regions)
        }
        Command::SelectBridges => {
            let mut painted = state.painted.clone();
            let mut roads = state.roads.clone();
            let settlements = &state.terrain().settlement_anchors;
            let settlement_entrances = network::connect_settlements(
                &state.grid,
                state.cells.as_slice(),
                state.slope_class.as_slice(),
                settlements,
                &mut painted,
                &mut roads,
            );
            let mut candidates_painted = Painted::empty(state.grid.cell_count());
            let candidates = build_bridges(
                &state.grid,
                state.terrain(),
                state.cells.as_slice(),
                state.slope_class.as_slice(),
                state.tiles.as_slice(),
                &mut candidates_painted,
            );
            painted.bridges = CellSet::new(state.grid.cell_count());
            painted.bridge_entries = CellSet::new(state.grid.cell_count());
            let bridges = network::connect_bridges(
                &state.grid,
                state.cells.as_slice(),
                state.slope_class.as_slice(),
                &settlement_entrances,
                candidates,
                &mut painted,
                &mut roads,
            );
            let network =
                network::build_road_graph(&state.grid, &roads, &bridges, &settlement_entrances);
            Event::BridgesSelected(bridges, painted, roads, network)
        }
        Command::BuildMesh => {
            let (tris, cols) = build_mesh(
                &state.grid,
                state.terrain(),
                state.cells.as_slice(),
                &state.painted,
                state.water_depth.as_slice(),
                state.landform.as_slice(),
                state.slope_class.as_slice(),
            );
            Event::MeshBuilt(tris, cols)
        }
        Command::BuildTags => Event::TagsBuilt(build_face_tags(&state.grid, &state.painted)),
        Command::PlaceFlora => Event::FloraPlaced(place_flora(
            &state.grid,
            state.terrain(),
            state.tiles.as_slice(),
            &state.painted,
            state.mesh_tris.as_slice(),
        )),
        Command::PlaceStructures => Event::StructuresPlaced(place_structures(
            &state.grid,
            state.terrain(),
            state.tiles.as_slice(),
            &state.painted,
            state.slope_class.as_slice(),
            state.mesh_tris.as_slice(),
        )),
        Command::BakeOutputs => bake_outputs(state),
    }
}

fn bake_outputs(state: &GenState) -> Event {
    let depth: Vec<Option<WaterDepth>> =
        face_max(&state.grid, state.water_depth.as_slice(), |depth| {
            depth.map_or(0, |d| d.severity() + 1)
        })
        .into_iter()
        .zip(state.tiles.as_slice())
        .map(|(depth, terrain)| terrain.is_water().then_some(depth).flatten())
        .collect();
    let road_material = state
        .grid
        .topology
        .faces()
        .map(|face| {
            let solid = state
                .grid
                .face_cells(face)
                .iter()
                .filter(|&&cell| state.painted.roads.contains(cell))
                .count()
                == 3;
            solid.then(|| {
                face_road_material(
                    &state.grid,
                    state.cells.as_slice(),
                    state.landform.as_slice(),
                    state.slope_class.as_slice(),
                    face.index(),
                )
            })
        })
        .collect();
    let river_r = river_surface_radii(
        &state.grid,
        state.mesh_tris.as_slice(),
        state.tiles.as_slice(),
        state.water_r.as_slice(),
    );
    let has_surface = state
        .grid
        .topology
        .faces()
        .map(|face| {
            state.water_r[face] > 0.0 || river_r[face.index()].iter().any(|&radius| radius > 0.0)
        })
        .collect::<Vec<_>>();
    let mut phase = state
        .grid
        .topology
        .faces()
        .map(|face| {
            let has_ocean = state.water_r[face] > 0.0
                && matches!(
                    state.tiles[face],
                    Terrain::Ocean | Terrain::Beach | Terrain::Cliff
                );
            if !has_surface[face.index()] {
                return None;
            }
            let tri = state.grid.unit_tris[face.index()];
            let point = SpherePos::new((tri[0] + tri[1] + tri[2]).normalize());
            Some(
                if !has_ocean && state.terrain().temperature_at(point) <= 0.0 {
                    WaterPhase::Frozen
                } else {
                    WaterPhase::Liquid
                },
            )
        })
        .collect::<Vec<_>>();

    // Saltwater needs colder conditions than inland water. Form candidate sea
    // ice on shallow ocean below -2 C, or any ocean depth below -15 C, then
    // discard tiny disconnected patches before extending coherent sheets to
    // their beach edge.
    const MIN_SEA_ICE_FACES: usize = 12;
    let sea_ice_candidate = state
        .grid
        .topology
        .faces()
        .map(|face| {
            if state.tiles[face] != Terrain::Ocean || state.water_r[face] <= 0.0 {
                return false;
            }
            let tri = state.grid.unit_tris[face.index()];
            let point = SpherePos::new((tri[0] + tri[1] + tri[2]).normalize());
            let temperature = state.terrain().temperature_at(point);
            temperature <= -15.0
                || (temperature <= -2.0 && depth[face.index()] == Some(WaterDepth::Shallow))
        })
        .collect::<Vec<_>>();
    let sea_ice_components = state
        .grid
        .topology
        .face_components(|face| sea_ice_candidate[face.index()]);
    let mut sea_ice_sizes = vec![0usize; sea_ice_components.count()];
    for face in state.grid.topology.faces() {
        if let Some(component) = sea_ice_components.face(face) {
            sea_ice_sizes[component.index()] += 1;
        }
    }
    let coherent_sea_ice = state
        .grid
        .topology
        .faces()
        .map(|face| {
            sea_ice_components
                .face(face)
                .is_some_and(|component| sea_ice_sizes[component.index()] >= MIN_SEA_ICE_FACES)
        })
        .collect::<Vec<_>>();
    let sea_ice_sources = state
        .grid
        .topology
        .faces()
        .filter(|face| coherent_sea_ice[face.index()])
        .collect::<Vec<_>>();
    let ocean_footprint = |face: FaceId| {
        state.water_r[face] > 0.0
            && matches!(
                state.tiles[face],
                Terrain::Ocean | Terrain::Beach | Terrain::Cliff
            )
    };
    let sea_ice_edge =
        state
            .grid
            .topology
            .face_distances_with(&sea_ice_sources, 2, ocean_footprint);
    for face in state.grid.topology.faces() {
        if sea_ice_edge.face_steps(face).is_some() {
            phase[face.index()] = Some(WaterPhase::Frozen);
        }
    }

    // Smooth one-face liquid notches along the ice front even when they remain
    // technically connected to a large open-water component.
    for _ in 0..3 {
        let fill = state
            .grid
            .topology
            .faces()
            .filter(|face| {
                has_surface[face.index()]
                    && phase[face.index()] == Some(WaterPhase::Liquid)
                    && state
                        .grid
                        .face_neighbors(*face)
                        .into_iter()
                        .filter(|neighbor| phase[neighbor.index()] == Some(WaterPhase::Frozen))
                        .count()
                        >= 2
            })
            .collect::<Vec<_>>();
        if fill.is_empty() {
            break;
        }
        for face in fill {
            phase[face.index()] = Some(WaterPhase::Frozen);
        }
    }

    // Local face-centre climate can leave one- or two-triangle liquid holes
    // inside otherwise continuous ice. Close only small, fully surface-bound
    // liquid components that actually touch ice; large components remain the
    // intentional transition to open water.
    const MAX_LIQUID_HOLE_FACES: usize = 11;
    let liquid_components = state.grid.topology.face_components(|face| {
        has_surface[face.index()] && phase[face.index()] == Some(WaterPhase::Liquid)
    });
    let mut liquid_sizes = vec![0usize; liquid_components.count()];
    let mut liquid_touches_ice = vec![false; liquid_components.count()];
    for face in state.grid.topology.faces() {
        let Some(component) = liquid_components.face(face) else {
            continue;
        };
        liquid_sizes[component.index()] += 1;
        liquid_touches_ice[component.index()] |= state
            .grid
            .face_neighbors(face)
            .into_iter()
            .any(|neighbor| phase[neighbor.index()] == Some(WaterPhase::Frozen));
    }
    for face in state.grid.topology.faces() {
        if let Some(component) = liquid_components.face(face)
            && liquid_sizes[component.index()] <= MAX_LIQUID_HOLE_FACES
            && liquid_touches_ice[component.index()]
        {
            phase[face.index()] = Some(WaterPhase::Frozen);
        }
    }
    let surface_condition = state
        .grid
        .topology
        .faces()
        .map(|face| {
            let tri = state.grid.unit_tris[face.index()];
            let point = SpherePos::new((tri[0] + tri[1] + tri[2]).normalize());
            if phase[face.index()] == Some(WaterPhase::Frozen)
                || state.terrain().temperature_at(point) <= 0.0
            {
                SurfaceCondition::Frozen
            } else {
                SurfaceCondition::Normal
            }
        })
        .collect::<Vec<_>>();
    Event::OutputsBaked {
        river_r,
        slope: face_max(
            &state.grid,
            state.slope_class.as_slice(),
            SlopeClass::severity,
        ),
        depth,
        phase,
        surface_condition,
        landform: face_majority(&state.grid, state.landform.as_slice(), Landform::rank),
        road_material,
    }
}

fn evolve(state: &mut GenState, event: Event) {
    match event {
        Event::TerrainInitialized(t) => state.terrain = Some(*t),
        Event::ElevationProposed(e) => {
            state.terrain_mut().set_vert_elevations(e);
        }
        Event::ClimateComputed(moist, temp) => {
            state.terrain_mut().set_climate(moist, temp);
        }
        Event::RiversPlanned(r) => {
            state.terrain_mut().river_paths = r;
        }
        Event::SettlementsPlaced(a) => {
            state.terrain_mut().settlement_anchors = a;
        }
        Event::RoadsPlanned(r) => {
            state.terrain_mut().road_paths = r;
        }
        Event::TilesClassified(c)
        | Event::RiversPainted(c)
        | Event::WaterNormalized(c)
        | Event::TransitionsResolved(c) => {
            state.tiles = derive_tiles(&state.grid, c.as_slice());
            state.cells = c;
        }
        Event::FeaturesPainted(p, roads) => {
            state.painted = p;
            state.roads = roads;
        }
        Event::BlendsMarked(b) => state.blends = b,
        Event::ElevationSolved { field, .. } => {
            state.terrain_mut().set_vert_elevations(field);
        }
        Event::WaterClustered(r) => state.water_r = r.into(),
        Event::RegionsBuilt(r, fr) => {
            state.regions = r;
            state.face_regions = fr;
        }
        Event::BridgesSelected(b, p, roads, network) => {
            state.bridges = b;
            state.painted = p;
            state.roads = roads;
            state.network = network;
        }
        Event::MeshBuilt(t, c) => {
            state.mesh_tris = t.into();
            state.mesh_colors = c.into();
        }
        Event::TagsBuilt(tags) => state.face_tags = tags.into(),
        Event::FloraPlaced(f) => state.flora = f,
        Event::StructuresPlaced(v) => state.structures = v,
        Event::SlopeClassified(sc, wd) => {
            state.slope_class = sc.into();
            state.water_depth = wd.into();
        }
        Event::SettlementSitesSelected(anchors) => {
            let terrain = state.terrain_mut();
            terrain.settlement_anchors = anchors;
            terrain.road_paths = terrain.plan_road_paths();
        }
        Event::LandformClassified(lf) => state.landform = lf.into(),
        Event::OutputsBaked {
            river_r,
            slope,
            depth,
            phase,
            surface_condition,
            landform,
            road_material,
        } => {
            state.river_r = river_r.into();
            state.face_slope_class = slope.into();
            state.face_water_depth = depth.into();
            state.face_water_phase = phase.into();
            state.face_surface_condition = surface_condition.into();
            state.face_landform = landform.into();
            state.face_road_material = road_material.into();
        }
    }
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
        Event::SlopeClassified(..) => vec![Command::SelectSettlementSites],
        Event::SettlementSitesSelected(..) => vec![Command::PaintFeatures],
        Event::FeaturesPainted(..) => vec![Command::SelectBridges],
        Event::BridgesSelected(..) => vec![Command::BuildRegions],
        Event::RegionsBuilt(..) => vec![Command::MarkBlends],
        Event::BlendsMarked(_) => vec![Command::BuildMesh],
        Event::MeshBuilt(..) => vec![Command::BuildTags],
        Event::TagsBuilt(..) => vec![Command::PlaceFlora],
        Event::FloraPlaced(_) => vec![Command::PlaceStructures],
        Event::StructuresPlaced(_) => vec![Command::BakeOutputs],
        Event::OutputsBaked { .. } => vec![],
    }
}

/// Run the full pipeline for a seed. `log` receives one line per event.
pub(super) fn run_state(seed: u32, mut log: impl FnMut(&str)) -> GenState {
    let mut state = GenState::new(seed);
    let mut queue = VecDeque::from([Command::InitTerrain]);
    while let Some(cmd) = queue.pop_front() {
        let event = decide(&state, &cmd);
        log(&event.label());
        queue.extend(react(&event));
        evolve(&mut state, event);
    }
    state
}
