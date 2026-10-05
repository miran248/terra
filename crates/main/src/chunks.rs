//! Icosphere chunk streaming + LOD.
//!
//! The render icosphere is split into 320 chunks — the subdivision-2 faces —
//! using the deterministic 4-way child order of `unit_icosphere_tris` (fine
//! face `fi` → chunk `fi / faces_per_chunk`, a contiguous slice; the same
//! scheme `zones.rs` proves at subdiv 3→7). `faces_per_chunk` is derived from
//! the level data, so terrain density can grow without touching this module.
//!
//! Every chunk is ALWAYS resident at LOD 1 (terrain/water/river/ice visuals,
//! whole planet built on the first frames) so the gameplay and planet-view
//! cameras — which render the same world, there are no RenderLayers — never
//! see holes. Closer chunks add detail:
//!
//! - LOD 1: terrain slice, flat water, river, ice visual
//! - LOD 2 (≤ 960 m or the radial horizon to chunk edge): + structures, large scenery
//! - LOD 3 (≤ 300 m, shrinking with camera altitude): + small scenery, water swell subdivision
//!
//! Transitions are INCREMENTAL: static meshes (terrain/river/ice/border) are
//! built once and never respawned; the water mesh is rebuilt only when its
//! subdivision changes (LOD 3 boundary); structures and scenery are added or
//! removed by delta. Nothing already on screen is torn down and re-added, so
//! a LOD bounce never blinks the world (or the minimap).
//!
//! Initial chunks start coarse; nearest upgrades use 8 transitions per frame.
//! Downgrades use 15% hysteresis; regional scenery fills before local props,
//! and both spawning and visibility refreshes use fixed per-update budgets.
//! Colliders for terrain/ice/bridges stay whole-planet in `setup_map`
//! (physics never streams — no fall-through at chunk borders, teleports just
//! work); only scenery/structure colliders live in chunks.

use crate::asset_catalog::AssetCatalog;
use crate::map::{CullRange, Ground, MainCamera, scenery_cull};
#[cfg(test)]
use avian3d::prelude::*;
use bevy::prelude::*;
use shared::art::AssetName;
#[cfg(test)]
use shared::level::SceneryKind;
use shared::level::{SceneryData, StructureData, WaterPhase};
use shared::planet_detail::{self, SceneryTier};
use terra_geometry::sphere::PLANET_RADIUS;
use shared::terrain::TerrainGen;
use std::time::Instant;

/// Optional per-stage measurements aligned with the acceptance frame trace.
#[derive(Resource, Default)]
pub(crate) struct PlanetWorkTrace {
    measurement_start: Option<f64>,
    route: Option<&'static str>,
    repeat: u8,
    rows: Vec<PlanetWorkTraceRow>,
    vehicle_action_rows: Vec<PlanetVehicleActionRow>,
    next_vehicle_action_id: u32,
    pending_vehicle_followup: Option<PendingVehicleFollowup>,
}

#[derive(Clone, Copy)]
pub(crate) struct PlanetVehicleActionSample {
    pub(crate) kind: &'static str,
    pub(crate) existing_entity: bool,
    pub(crate) old_position: Option<Vec3>,
    pub(crate) new_position: Option<Vec3>,
    pub(crate) locate_ms: Option<f64>,
    pub(crate) dispatch_ms: f64,
    pub(crate) scene_child_spawned: bool,
    pub(crate) fixed_elapsed_s_before: f64,
    pub(crate) fixed_elapsed_s_after: f64,
    pub(crate) resident_obstacles_before: usize,
    pub(crate) world_ready_before: bool,
}

#[derive(Clone, Copy)]
struct PlanetVehicleActionRow {
    route: &'static str,
    repeat: u8,
    elapsed_s: f64,
    real_elapsed_s: f64,
    event: &'static str,
    action_id: u32,
    kind: &'static str,
    existing_entity: bool,
    old_position: Option<Vec3>,
    new_position: Option<Vec3>,
    locate_ms: Option<f64>,
    dispatch_ms: Option<f64>,
    scene_child_spawned: Option<bool>,
    fixed_elapsed_s_before: f64,
    fixed_elapsed_s_after: f64,
    resident_obstacles_before: usize,
    resident_obstacles_after: usize,
    fixed_delta_s: Option<f64>,
    resident_delta: Option<isize>,
    world_ready: bool,
    actions_pending: Option<bool>,
}

#[derive(Clone, Copy)]
struct PendingVehicleFollowup {
    route: &'static str,
    repeat: u8,
    measurement_start: f64,
    action_id: u32,
    kind: &'static str,
    existing_entity: bool,
    fixed_elapsed_s_before: f64,
    resident_obstacles_before: usize,
}

#[derive(Clone, Copy)]
struct PlanetWorkTraceRow {
    route: &'static str,
    repeat: u8,
    elapsed_s: f64,
    real_elapsed_s: f64,
    stage: &'static str,
    camera_radius_m: f32,
    duration_ms: f64,
    a: usize,
    b: usize,
    c: usize,
    d: usize,
}

impl PlanetWorkTrace {
    pub(crate) fn active(&self) -> bool {
        self.measurement_start.is_some()
    }

    pub(crate) fn start_repeat(&mut self, elapsed_s: f64, route: &'static str, repeat: u8) {
        self.measurement_start = Some(elapsed_s);
        self.route = Some(route);
        self.repeat = repeat;
        self.next_vehicle_action_id = 1;
    }

    pub(crate) fn stop_repeat(&mut self) {
        self.measurement_start = None;
        self.route = None;
    }

    pub(crate) fn record(
        &mut self,
        elapsed_s: f64,
        stage: &'static str,
        camera_radius_m: f32,
        duration_ms: f64,
        a: usize,
        b: usize,
        c: usize,
        d: usize,
    ) {
        let Some(measurement_start) = self.measurement_start else {
            return;
        };
        self.rows.push(PlanetWorkTraceRow {
            route: self.route.unwrap_or("unknown"),
            repeat: self.repeat,
            elapsed_s: elapsed_s - measurement_start,
            real_elapsed_s: elapsed_s,
            stage,
            camera_radius_m,
            duration_ms,
            a,
            b,
            c,
            d,
        });
    }

    pub(crate) fn record_vehicle_summon(
        &mut self,
        elapsed_s: f64,
        real_elapsed_s: f64,
        sample: PlanetVehicleActionSample,
    ) {
        let (Some(measurement_start), Some(route)) = (self.measurement_start, self.route) else {
            return;
        };
        let action_id = self.next_vehicle_action_id;
        self.next_vehicle_action_id = self.next_vehicle_action_id.saturating_add(1);
        self.vehicle_action_rows.push(PlanetVehicleActionRow {
            route,
            repeat: self.repeat,
            elapsed_s: elapsed_s - measurement_start,
            real_elapsed_s,
            event: "summon-dispatch",
            action_id,
            kind: sample.kind,
            existing_entity: sample.existing_entity,
            old_position: sample.old_position,
            new_position: sample.new_position,
            locate_ms: sample.locate_ms,
            dispatch_ms: Some(sample.dispatch_ms),
            scene_child_spawned: Some(sample.scene_child_spawned),
            fixed_elapsed_s_before: sample.fixed_elapsed_s_before,
            fixed_elapsed_s_after: sample.fixed_elapsed_s_after,
            resident_obstacles_before: sample.resident_obstacles_before,
            resident_obstacles_after: sample.resident_obstacles_before,
            fixed_delta_s: None,
            resident_delta: Some(0),
            world_ready: sample.world_ready_before,
            actions_pending: None,
        });
        self.pending_vehicle_followup = Some(PendingVehicleFollowup {
            route,
            repeat: self.repeat,
            measurement_start,
            action_id,
            kind: sample.kind,
            existing_entity: sample.existing_entity,
            fixed_elapsed_s_before: sample.fixed_elapsed_s_before,
            resident_obstacles_before: sample.resident_obstacles_before,
        });
    }

    pub(crate) fn record_vehicle_followup(
        &mut self,
        elapsed_s: f64,
        real_elapsed_s: f64,
        fixed_elapsed_s: f64,
        resident_obstacles: usize,
        world_ready: bool,
        actions_pending: bool,
    ) {
        let Some(pending) = self.pending_vehicle_followup.take() else {
            return;
        };
        self.vehicle_action_rows.push(PlanetVehicleActionRow {
            route: pending.route,
            repeat: pending.repeat,
            elapsed_s: elapsed_s - pending.measurement_start,
            real_elapsed_s,
            event: "first-following-update",
            action_id: pending.action_id,
            kind: pending.kind,
            existing_entity: pending.existing_entity,
            old_position: None,
            new_position: None,
            locate_ms: None,
            dispatch_ms: None,
            scene_child_spawned: None,
            fixed_elapsed_s_before: pending.fixed_elapsed_s_before,
            fixed_elapsed_s_after: fixed_elapsed_s,
            resident_obstacles_before: pending.resident_obstacles_before,
            resident_obstacles_after: resident_obstacles,
            fixed_delta_s: Some(fixed_elapsed_s - pending.fixed_elapsed_s_before),
            resident_delta: Some(
                resident_obstacles as isize - pending.resident_obstacles_before as isize,
            ),
            world_ready,
            actions_pending: Some(actions_pending),
        });
    }

    pub(crate) fn to_csv(&self) -> String {
        let mut csv = String::from(
            "route,repeat,elapsed_s,real_elapsed_s,stage,camera_radius_m,duration_ms,operation_a,operation_b,operation_c,operation_d\n",
        );
        for row in &self.rows {
            csv.push_str(&format!(
                "{},{},{:.6},{:.6},{},{:.3},{:.6},{},{},{},{}\n",
                row.route,
                row.repeat,
                row.elapsed_s,
                row.real_elapsed_s,
                row.stage,
                row.camera_radius_m,
                row.duration_ms,
                row.a,
                row.b,
                row.c,
                row.d,
            ));
        }
        csv
    }

    pub(crate) fn vehicle_action_csv(&self, route: &str, repeat: u8) -> Option<String> {
        let rows = self
            .vehicle_action_rows
            .iter()
            .filter(|row| row.route == route && row.repeat == repeat)
            .collect::<Vec<_>>();
        if rows.is_empty() {
            return None;
        }
        let mut csv = String::from(
            "route,repeat,elapsed_s,real_elapsed_s,event,action_id,kind,existing_entity,old_x,old_y,old_z,new_x,new_y,new_z,locate_ms,dispatch_ms,scene_child_spawned,fixed_elapsed_s_before,fixed_elapsed_s_after,resident_obstacles_before,resident_obstacles_after,fixed_delta_s,resident_delta,world_ready,actions_pending\n",
        );
        for row in rows {
            let position = |value: Option<Vec3>| {
                value.map_or([String::new(), String::new(), String::new()], |position| {
                    position
                        .to_array()
                        .map(|component| format!("{component:.4}"))
                })
            };
            let optional = |value: Option<String>| value.unwrap_or_default();
            let old = position(row.old_position);
            let new = position(row.new_position);
            let fields = [
                row.route.to_owned(),
                row.repeat.to_string(),
                format!("{:.6}", row.elapsed_s),
                format!("{:.6}", row.real_elapsed_s),
                row.event.to_owned(),
                row.action_id.to_string(),
                row.kind.to_owned(),
                row.existing_entity.to_string(),
                old[0].clone(),
                old[1].clone(),
                old[2].clone(),
                new[0].clone(),
                new[1].clone(),
                new[2].clone(),
                optional(row.locate_ms.map(|value| format!("{value:.6}"))),
                optional(row.dispatch_ms.map(|value| format!("{value:.6}"))),
                optional(row.scene_child_spawned.map(|value| value.to_string())),
                format!("{:.6}", row.fixed_elapsed_s_before),
                format!("{:.6}", row.fixed_elapsed_s_after),
                row.resident_obstacles_before.to_string(),
                row.resident_obstacles_after.to_string(),
                optional(row.fixed_delta_s.map(|value| format!("{value:.6}"))),
                optional(row.resident_delta.map(|value| value.to_string())),
                row.world_ready.to_string(),
                optional(row.actions_pending.map(|value| value.to_string())),
            ];
            csv.push_str(&fields.join(","));
            csv.push('\n');
        }
        Some(csv)
    }

    pub(crate) fn repeat_csv(&self, route: &str, repeat: u8) -> String {
        let mut csv = String::from(
            "route,repeat,elapsed_s,real_elapsed_s,stage,camera_radius_m,duration_ms,operation_a,operation_b,operation_c,operation_d\n",
        );
        for row in self
            .rows
            .iter()
            .filter(|row| row.route == route && row.repeat == repeat)
        {
            csv.push_str(&format!(
                "{},{},{:.6},{:.6},{},{:.3},{:.6},{},{},{},{}\n",
                row.route,
                row.repeat,
                row.elapsed_s,
                row.real_elapsed_s,
                row.stage,
                row.camera_radius_m,
                row.duration_ms,
                row.a,
                row.b,
                row.c,
                row.d,
            ));
        }
        csv
    }
}

/// Chunk count: the subdivision-2 icosphere faces (20 × 4²).
pub const CHUNK_COUNT: usize = 320;

/// Debug: outline every chunk in its LOD color (red = 1, yellow = 2,
/// green = 3) so LOD rings are visually inspectable. Flip off to ship.
const DEBUG_CHUNK_BORDERS: bool = false;

/// LOD transitions applied per frame (static/water geometry work).
const TRANSITIONS_PER_FRAME: usize = 8;
/// Total structure and scenery roots spawned or despawned per update.
const SCENE_ROOT_WORK_PER_UPDATE: usize = 256;
/// Reserve half the shared budget for removals so a continuing stream of
/// nearby promotions cannot keep distant detail resident indefinitely.
const SCENE_ROOT_REMOVALS_PER_UPDATE: usize = SCENE_ROOT_WORK_PER_UPDATE / 2;

/// Water mesh subdivision per LOD (see `water::build_water_surface`).
/// Render faces are ~19 m; LOD 3 subdivides once (~9 m spacing) so the 40 m
/// geometric swell resolves near the player; further out the bare faces
/// carry it coarsely.
fn water_subdiv(lod: u8) -> u32 {
    match lod {
        3 => crate::water::WATER_SUBDIV,
        _ => 0,
    }
}

/// Baked per-chunk world data, sliced/binned once at setup from `LevelData`.
pub struct ChunkData {
    pub terrain_tris: Vec<[[f32; 3]; 3]>,
    pub terrain_colors: Vec<[[f32; 4]; 3]>,
    pub water_r: Vec<f32>,
    pub river_r: Vec<[f32; 3]>,
    pub water_phase: Vec<Option<WaterPhase>>,
    /// Large scenery resident from LOD 2 (trees, rocks, logs) per chunk.
    pub scenery_large: Vec<Vec<SceneryData>>,
    /// Small scenery resident only at LOD 3 (ground cover) per chunk.
    pub scenery_small: Vec<Vec<SceneryData>>,
    pub structures: Vec<Vec<StructureData>>,
}

/// Live entity bookkeeping for one chunk. Split by lifetime so LOD
/// transitions only touch the group that actually changes.
#[derive(Default)]
pub struct ChunkState {
    pub lod: u8,
    /// Terrain + river + ice: built once, never respawned.
    pub static_ents: Vec<Entity>,
    /// Water mesh; rebuilt only when `water_subdiv` changes.
    pub water: Option<Entity>,
    pub border: Option<Entity>,
    pub structures: Vec<Entity>,
    pub scenery_large: Vec<Entity>,
    pub scenery_small: Vec<Entity>,
    /// Streaming cursors into ChunkData::scenery_*; entities below are spawned.
    pub large_cursor: usize,
    pub small_cursor: usize,
}

#[derive(Resource)]
pub struct ChunkManager {
    pub data: ChunkData,
    pub faces_per_chunk: usize,
    /// Unit direction of each chunk's centroid.
    pub centers: Vec<Vec3>,
    /// Surface distance (m) from each chunk's centroid to its farthest vertex.
    pub radii: Vec<f32>,
    /// Round-robin view-culling worklist for currently resident scenery roots.
    pub cull_order: Vec<Entity>,
    pub chunks: Vec<ChunkState>,
    pub terrain_mat: Handle<StandardMaterial>,
    pub water_mat: Handle<crate::water::WaterMaterial>,
    pub river_mat: Handle<crate::water::WaterMaterial>,
    pub ice_mat: Handle<StandardMaterial>,
    /// Debug border material per LOD (index = lod - 1); see DEBUG_CHUNK_BORDERS.
    pub border_mats: [Handle<StandardMaterial>; 3],
}

impl ChunkManager {
    /// Bin baked level data into the 320 chunks. Scenery/structures land in
    /// their chunk via the baked render-face index (`face / faces_per_chunk`).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        terrain_tris: Vec<[[f32; 3]; 3]>,
        terrain_colors: Vec<[[f32; 4]; 3]>,
        water_r: Vec<f32>,
        river_r: Vec<[f32; 3]>,
        water_phase: Vec<Option<WaterPhase>>,
        scenery: Vec<SceneryData>,
        structures: Vec<StructureData>,
        terrain_mat: Handle<StandardMaterial>,
        water_mat: Handle<crate::water::WaterMaterial>,
        river_mat: Handle<crate::water::WaterMaterial>,
        ice_mat: Handle<StandardMaterial>,
        border_mats: [Handle<StandardMaterial>; 3],
    ) -> Self {
        assert_eq!(
            terrain_tris.len() % CHUNK_COUNT,
            0,
            "render faces must divide into the subdiv-2 chunks"
        );
        let faces_per_chunk = terrain_tris.len() / CHUNK_COUNT;
        let centers: Vec<Vec3> = (0..CHUNK_COUNT)
            .map(|c| {
                let mut sum = Vec3::ZERO;
                for t in &terrain_tris[c * faces_per_chunk..(c + 1) * faces_per_chunk] {
                    for v in t {
                        sum += Vec3::from_array(*v).normalize();
                    }
                }
                sum.normalize()
            })
            .collect();
        // Angular radius (m along the surface) of each chunk: centroid → the
        // farthest of its vertices. LOD distance measures to the chunk's EDGE
        // (centroid distance minus this), not its centroid — otherwise standing
        // on a chunk corner reads ~350 m to all its neighbours and ground scenery
        // vanishes underfoot.
        let radii: Vec<f32> = (0..CHUNK_COUNT)
            .map(|c| {
                let center = centers[c];
                let mut min_cos = 1.0f32;
                for t in &terrain_tris[c * faces_per_chunk..(c + 1) * faces_per_chunk] {
                    for v in t {
                        min_cos = min_cos.min(Vec3::from_array(*v).normalize().dot(center));
                    }
                }
                min_cos.clamp(-1.0, 1.0).acos() * PLANET_RADIUS
            })
            .collect();
        let mut scenery_large = vec![Vec::new(); CHUNK_COUNT];
        let mut scenery_small = vec![Vec::new(); CHUNK_COUNT];
        for f in scenery {
            let c = f.face as usize / faces_per_chunk;
            if SceneryTier::for_kind(f.kind) == SceneryTier::Regional {
                scenery_large[c].push(f);
            } else {
                scenery_small[c].push(f);
            }
        }
        let mut chunk_structures = vec![Vec::new(); CHUNK_COUNT];
        for s in structures {
            chunk_structures[s.face as usize / faces_per_chunk].push(s);
        }
        Self {
            data: ChunkData {
                terrain_tris,
                terrain_colors,
                water_r,
                river_r,
                water_phase,
                scenery_large,
                scenery_small,
                structures: chunk_structures,
            },
            faces_per_chunk,
            centers,
            radii,
            cull_order: Vec::new(),
            chunks: (0..CHUNK_COUNT).map(|_| ChunkState::default()).collect(),
            terrain_mat,
            water_mat,
            river_mat,
            ice_mat,
            border_mats,
        }
    }

    fn face_range(&self, chunk: usize) -> std::ops::Range<usize> {
        chunk * self.faces_per_chunk..(chunk + 1) * self.faces_per_chunk
    }
}

/// Build broad planet geography first, then promote nearest chunks within a
/// fixed per-frame transition budget. Detail follows the camera's radial
/// position and attained altitude; physics residency remains body-centered.
pub fn update_chunk_lods(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    camera: Query<&Transform, With<MainCamera>>,
    terrain: Option<Res<TerrainGen>>,
    mut mgr: ResMut<ChunkManager>,
    mut probe: Option<ResMut<PlanetWorkTrace>>,
    time: Res<Time<Real>>,
) {
    let started = probe
        .as_ref()
        .is_some_and(|probe| probe.active())
        .then(Instant::now);
    let Some(cam) = camera.iter().next() else {
        return;
    };
    let camera_radius_m = cam.translation.length();
    let eye_dir = cam.translation.normalize_or(Vec3::Y);
    let camera_altitude = crate::map::altitude_above_surface(cam.translation, terrain.as_deref());

    // LOD 1 is the immediately visible globe. First-time chunks do not jump
    // directly to local detail; later promotions are nearest-first and budgeted.
    let initializing = mgr.chunks.iter().any(|state| state.lod == 0);
    let mut transitions = 0;
    let mut water_rebuilds = 0;
    for chunk in 0..CHUNK_COUNT {
        if mgr.chunks[chunk].lod == 0 {
            set_chunk_lod(&mut commands, &mut meshes, &mut mgr, chunk, 1);
            transitions += 1;
            water_rebuilds += 1;
        }
    }
    if initializing {
        if let (Some(probe), Some(started)) = (probe.as_mut(), started) {
            probe.record(
                time.elapsed_secs_f64(),
                "chunk_lod",
                camera_radius_m,
                started.elapsed().as_secs_f64() * 1_000.0,
                transitions,
                water_rebuilds,
                0,
                0,
            );
        }
        return;
    }

    let mut order: Vec<(usize, f32)> = (0..CHUNK_COUNT)
        .map(|chunk| {
            let center_dist =
                mgr.centers[chunk].dot(eye_dir).clamp(-1.0, 1.0).acos() * PLANET_RADIUS;
            (chunk, (center_dist - mgr.radii[chunk]).max(0.0))
        })
        .collect();
    order.sort_by(|(a, da), (b, db)| da.total_cmp(db).then_with(|| a.cmp(b)));
    let mut budget = TRANSITIONS_PER_FRAME;
    for (chunk, dist) in order {
        if budget == 0 {
            break;
        }
        let cur = mgr.chunks[chunk].lod;
        let want = planet_detail::desired_chunk_lod(dist, camera_altitude, cur);
        if want != cur {
            if water_subdiv(cur) != water_subdiv(want) {
                water_rebuilds += 1;
            }
            set_chunk_lod(&mut commands, &mut meshes, &mut mgr, chunk, want);
            transitions += 1;
            budget -= 1;
        }
    }
    if let (Some(probe), Some(started)) = (probe.as_mut(), started) {
        probe.record(
            time.elapsed_secs_f64(),
            "chunk_lod",
            camera_radius_m,
            started.elapsed().as_secs_f64() * 1_000.0,
            transitions,
            water_rebuilds,
            0,
            0,
        );
    }
}

/// Apply a LOD transition incrementally: only what differs between `cur` and
/// `lod` is spawned/despawned. Scenery spawning is deferred to `stream_scenery`.
fn set_chunk_lod(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    mgr: &mut ChunkManager,
    chunk: usize,
    lod: u8,
) {
    let cur = mgr.chunks[chunk].lod;
    let range = mgr.face_range(chunk);
    let tris = &mgr.data.terrain_tris[range.clone()];

    // Static geometry: first build only.
    if cur == 0 {
        let terrain = crate::map::build_visual_mesh(tris, &mgr.data.terrain_colors[range.clone()]);
        let mut static_ents = vec![
            commands
                .spawn((
                    Mesh3d(meshes.add(terrain)),
                    MeshMaterial3d(mgr.terrain_mat.clone()),
                    Transform::default(),
                    Ground,
                ))
                .id(),
        ];
        if let Some(river) = crate::water::build_river_surfaces(
            tris,
            &mgr.data.river_r[range.clone()],
            &mgr.data.water_phase[range.clone()],
        ) {
            static_ents.push(
                commands
                    .spawn((
                        Mesh3d(meshes.add(river)),
                        MeshMaterial3d(mgr.river_mat.clone()),
                        Transform::default(),
                        bevy::light::NotShadowCaster,
                        Ground,
                    ))
                    .id(),
            );
        }
        // Ice visual (the whole-planet ice collider lives in setup_map).
        if let Some((ice, _)) = crate::water::build_ice_surface(
            tris,
            &mgr.data.water_r[range.clone()],
            &mgr.data.river_r[range.clone()],
            &mgr.data.water_phase[range.clone()],
        ) {
            static_ents.push(
                commands
                    .spawn((
                        Mesh3d(meshes.add(ice)),
                        MeshMaterial3d(mgr.ice_mat.clone()),
                        Transform::default(),
                        bevy::light::NotShadowCaster,
                        Ground,
                    ))
                    .id(),
            );
        }
        mgr.chunks[chunk].static_ents = static_ents;
        if DEBUG_CHUNK_BORDERS {
            let border = commands
                .spawn((
                    Mesh3d(meshes.add(build_border_mesh(tris))),
                    MeshMaterial3d(mgr.border_mats[(lod - 1) as usize].clone()),
                    Transform::default(),
                    bevy::light::NotShadowCaster,
                    Ground,
                ))
                .id();
            mgr.chunks[chunk].border = Some(border);
        }
    } else if DEBUG_CHUNK_BORDERS && let Some(border) = mgr.chunks[chunk].border {
        // Border: material swap only, no respawn.
        commands
            .entity(border)
            .insert(MeshMaterial3d(mgr.border_mats[(lod - 1) as usize].clone()));
    }

    // Water: rebuild only when the subdivision level actually changes.
    if cur == 0 || water_subdiv(cur) != water_subdiv(lod) {
        if let Some(water) = mgr.chunks[chunk].water.take() {
            commands.entity(water).try_despawn();
        }
        if let Some(water) = crate::water::build_water_surface(
            tris,
            &mgr.data.water_r[range.clone()],
            &mgr.data.water_phase[range.clone()],
            water_subdiv(lod),
        ) {
            mgr.chunks[chunk].water = Some(
                commands
                    .spawn((
                        Mesh3d(meshes.add(water)),
                        MeshMaterial3d(mgr.water_mat.clone()),
                        Transform::default(),
                        bevy::light::NotShadowCaster,
                        Ground,
                    ))
                    .id(),
            );
        }
    }

    // Structures and scenery reconcile to this target in the bounded
    // stream_scenery system. Keeping only the newest target here means a
    // reversed camera request cancels any removals not already processed.
    mgr.chunks[chunk].lod = lod;
}

/// Reconcile structure and scenery roots to each chunk's current LOD target.
/// One shared root budget bounds both scene construction and recursive despawn;
/// additions prioritize nearest chunks, while removals start at the farthest.
pub fn stream_scenery(
    mut commands: Commands,
    catalog: Res<AssetCatalog>,
    camera: Query<&Transform, With<MainCamera>>,
    terrain: Option<Res<TerrainGen>>,
    mut mgr: ResMut<ChunkManager>,
    mut probe: Option<ResMut<PlanetWorkTrace>>,
    time: Res<Time<Real>>,
) {
    let Some(cam) = camera.iter().next() else {
        return;
    };
    let started = probe
        .as_ref()
        .is_some_and(|probe| probe.active())
        .then(Instant::now);
    let camera_radius_m = cam.translation.length();
    let eye_dir = cam.translation.normalize_or(Vec3::Y);
    let pending = |mgr: &ChunkManager, chunk: usize| {
        let state = &mgr.chunks[chunk];
        let structure_count = mgr.data.structures[chunk].len();
        let large_count = mgr.data.scenery_large[chunk].len();
        let small_count = mgr.data.scenery_small[chunk].len();
        (state.lod < 2 && (!state.structures.is_empty() || state.large_cursor > 0))
            || (state.lod >= 2
                && (state.structures.len() < structure_count || state.large_cursor < large_count))
            || (state.lod < 3 && state.small_cursor > 0)
            || (state.lod >= 3 && state.small_cursor < small_count)
    };
    let mut order: Vec<usize> = (0..CHUNK_COUNT)
        .filter(|&chunk| pending(&mgr, chunk))
        .collect();
    if order.is_empty() {
        return;
    }
    order.sort_by(|&a, &b| {
        mgr.centers[b]
            .dot(eye_dir)
            .total_cmp(&mgr.centers[a].dot(eye_dir))
    });

    let mut budget = SCENE_ROOT_WORK_PER_UPDATE;
    let mut removal_budget = SCENE_ROOT_REMOVALS_PER_UPDATE;
    let mut removed_scenery = std::collections::HashSet::with_capacity(removal_budget);
    let mut roots_removed = 0;
    let mut roots_spawned = 0;

    // Remove detail farthest from the camera first. Pop from each resident
    // prefix and move the cursors back so a later LOD reversal resumes at the
    // first missing baked item without duplicates.
    for &chunk in order.iter().rev() {
        while budget > 0 && removal_budget > 0 {
            let state = &mut mgr.chunks[chunk];
            let removed = if state.lod < 3 && state.small_cursor > 0 {
                state.small_cursor -= 1;
                state.scenery_small.pop().map(|entity| (entity, true))
            } else if state.lod < 2 && state.large_cursor > 0 {
                state.large_cursor -= 1;
                state.scenery_large.pop().map(|entity| (entity, true))
            } else if state.lod < 2 {
                state.structures.pop().map(|entity| (entity, false))
            } else {
                None
            };
            let Some((entity, is_scenery)) = removed else {
                break;
            };
            commands.entity(entity).try_despawn();
            if is_scenery {
                removed_scenery.insert(entity);
            }
            roots_removed += 1;
            budget -= 1;
            removal_budget -= 1;
        }
        if budget == 0 || removal_budget == 0 {
            break;
        }
    }
    // Compact once after the bounded batch instead of scanning the global
    // culling worklist once per demoted chunk.
    if !removed_scenery.is_empty() {
        mgr.cull_order
            .retain(|entity| !removed_scenery.contains(entity));
    }

    // Structures used to spawn synchronously inside each chunk transition.
    // Stream them through the same work budget, nearest chunk first.
    for &chunk in &order {
        while budget > 0 && mgr.chunks[chunk].lod >= 2 {
            let index = mgr.chunks[chunk].structures.len();
            let Some(structure) = mgr.data.structures[chunk].get(index).cloned() else {
                break;
            };
            let entity = spawn_structure(&mut commands, &catalog, &structure);
            mgr.chunks[chunk].structures.push(entity);
            roots_spawned += 1;
            budget -= 1;
        }
        if budget == 0 {
            break;
        }
    }

    // Regional scenery fills before local ground cover, retaining the prior
    // nearest-chunk policy while sharing the structure/root budget.
    for &chunk in &order {
        while budget > 0 && mgr.chunks[chunk].lod >= 2 {
            let index = mgr.chunks[chunk].large_cursor;
            let Some(scenery) = mgr.data.scenery_large[chunk].get(index).copied() else {
                break;
            };
            let entity = spawn_scenery(
                &mut commands,
                &catalog,
                &scenery,
                initial_scenery_visibility(scenery, cam.translation, terrain.as_deref()),
            );
            mgr.chunks[chunk].scenery_large.push(entity);
            mgr.cull_order.push(entity);
            mgr.chunks[chunk].large_cursor = index + 1;
            roots_spawned += 1;
            budget -= 1;
        }
        if budget == 0 {
            break;
        }
    }
    for chunk in order.iter().copied() {
        while budget > 0 && mgr.chunks[chunk].lod >= 3 {
            let index = mgr.chunks[chunk].small_cursor;
            let Some(scenery) = mgr.data.scenery_small[chunk].get(index).copied() else {
                break;
            };
            let entity = spawn_scenery(
                &mut commands,
                &catalog,
                &scenery,
                initial_scenery_visibility(scenery, cam.translation, terrain.as_deref()),
            );
            mgr.chunks[chunk].scenery_small.push(entity);
            mgr.cull_order.push(entity);
            mgr.chunks[chunk].small_cursor = index + 1;
            roots_spawned += 1;
            budget -= 1;
        }
        if budget == 0 {
            break;
        }
    }
    if let (Some(probe), Some(started)) = (probe.as_mut(), started) {
        probe.record(
            time.elapsed_secs_f64(),
            "scene_roots",
            camera_radius_m,
            started.elapsed().as_secs_f64() * 1_000.0,
            roots_removed,
            roots_spawned,
            order.len(),
            budget,
        );
    }
}

/// Debug outline of a chunk: the boundary edges of its terrain slice (edges
/// used by exactly one triangle), rendered as a LineList lifted 2 m off the
/// surface so it clears the terrain and reads from the air.
fn build_border_mesh(tris: &[[[f32; 3]; 3]]) -> Mesh {
    use std::collections::HashMap;
    type Edge = ([f32; 3], [f32; 3], u32);
    let key = |v: &[f32; 3]| [v[0].to_bits(), v[1].to_bits(), v[2].to_bits()];
    let mut edges: HashMap<[[u32; 3]; 2], Edge> = HashMap::new();
    for t in tris {
        for (a, b) in [(0, 1), (1, 2), (2, 0)] {
            let (ka, kb) = (key(&t[a]), key(&t[b]));
            let ek = if ka <= kb { [ka, kb] } else { [kb, ka] };
            edges
                .entry(ek)
                .and_modify(|e| e.2 += 1)
                .or_insert((t[a], t[b], 1));
        }
    }
    let mut positions = Vec::new();
    for (a, b, count) in edges.into_values() {
        if count == 1 {
            for v in [a, b] {
                let p = Vec3::from_array(v);
                positions.push((p + p.normalize() * 2.0).to_array());
            }
        }
    }
    let mut mesh = Mesh::new(
        bevy::render::mesh::PrimitiveTopology::LineList,
        Default::default(),
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        bevy::render::mesh::VertexAttributeValues::Float32x3(positions),
    );
    mesh
}

/// Solid scenery and structures are obstacles, even though Ground also owns cleanup.
#[derive(Component)]
pub(crate) struct WorldObstacle;

fn spawn_structure(commands: &mut Commands, catalog: &AssetCatalog, s: &StructureData) -> Entity {
    let pos = Vec3::from_array(s.pos);
    let up = pos.normalize();
    let rotation = Quat::from_rotation_arc(Vec3::Y, up) * Quat::from_rotation_y(s.yaw);
    let mut root = commands.spawn((
        Transform::from_translation(pos).with_rotation(rotation),
        Visibility::default(),
        Ground,
    ));
    root.with_child((
        WorldAssetRoot(catalog.scene(s.kind.asset_name())),
        Transform::default(),
    ));
    root.id()
}

fn initial_scenery_visibility(
    f: SceneryData,
    camera_position: Vec3,
    terrain: Option<&TerrainGen>,
) -> Visibility {
    let point = Vec3::from_array(f.pos);
    let surface_distance = planet_detail::radial_surface_distance(camera_position, point);
    let visible = planet_detail::scenery_visible(
        SceneryTier::for_kind(f.kind),
        point.distance(camera_position),
        surface_distance,
        crate::map::altitude_above_surface(camera_position, terrain),
        scenery_cull(f.kind),
        false,
    );
    if visible {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    }
}

fn spawn_scenery(
    commands: &mut Commands,
    catalog: &AssetCatalog,
    f: &SceneryData,
    visibility: Visibility,
) -> Entity {
    let pos = Vec3::from_array(f.pos);
    let up = pos.normalize();
    let hash = f.pos[0].to_bits()
        ^ f.pos[1].to_bits().rotate_left(13)
        ^ f.pos[2].to_bits().rotate_left(27);
    let scale = 0.7 + (hash & 0xff) as f32 / 255.0 * 0.6;
    let yaw = (hash >> 8 & 0xff) as f32 / 255.0 * std::f32::consts::TAU;
    let rotation = Quat::from_rotation_arc(Vec3::Y, up) * Quat::from_rotation_y(yaw);

    let mut root = commands.spawn((
        CullRange {
            meters: scenery_cull(f.kind),
            tier: SceneryTier::for_kind(f.kind),
        },
        Transform::from_translation(pos).with_rotation(rotation),
        visibility,
        bevy::light::NotShadowCaster,
        Ground,
    ));

    root.with_child((
        WorldAssetRoot(catalog.scene(&shared::art::scenery_variant_name(f.kind, f.variant as u32))),
        Transform::from_scale(Vec3::splat(scale)),
    ));
    root.id()
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::level::{FloraKind, StructureKind};
    use terra_geometry::planet::{PlanetMesh, unit_icosphere_tris};

    const DETAIL_BUDGET_TEST_ROOTS: usize = 700;

    fn scene_root_ids(app: &mut App) -> std::collections::HashSet<Entity> {
        let mut roots = app.world_mut().query_filtered::<Entity, With<Ground>>();
        roots.iter(app.world()).collect()
    }

    fn scene_root_change_count(
        before: &std::collections::HashSet<Entity>,
        after: &std::collections::HashSet<Entity>,
    ) -> usize {
        before.symmetric_difference(after).count()
    }

    fn detail_app() -> (App, Entity, Entity, Entity) {
        let triangles = unit_icosphere_tris(2)
            .into_iter()
            .map(|triangle| triangle.map(|vertex| vertex.to_array()))
            .collect::<Vec<_>>();
        let face_zero_position =
            Vec3::from_array(triangles[0][0]).normalize() * (PLANET_RADIUS + 1.0);
        let structures = (0..DETAIL_BUDGET_TEST_ROOTS)
            .map(|_| StructureData {
                pos: face_zero_position.to_array(),
                face: 0,
                kind: StructureKind::House,
                yaw: 0.0,
            })
            .collect();
        let scenery = (0..DETAIL_BUDGET_TEST_ROOTS)
            .map(|_| SceneryData {
                pos: face_zero_position.to_array(),
                face: 0,
                kind: SceneryKind::Flora(FloraKind::Tree),
                variant: 0,
            })
            .chain((0..DETAIL_BUDGET_TEST_ROOTS).map(|_| SceneryData {
                pos: face_zero_position.to_array(),
                face: 0,
                kind: SceneryKind::Flora(FloraKind::Grass),
                variant: 0,
            }))
            .collect();

        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Mesh>();
        app.insert_resource(AssetCatalog::fixture(&[
            "structure.house",
            "scenery.tree.0",
            "scenery.grass",
        ]));

        let mut manager = ChunkManager::new(
            triangles,
            vec![[[0.0, 0.0, 0.0, 1.0]; 3]; CHUNK_COUNT],
            vec![0.0; CHUNK_COUNT],
            vec![[0.0; 3]; CHUNK_COUNT],
            vec![None; CHUNK_COUNT],
            scenery,
            structures,
            Handle::default(),
            Handle::default(),
            Handle::default(),
            Handle::default(),
            [Handle::default(), Handle::default(), Handle::default()],
        );

        let high_direction = -manager.centers[0];
        let high_camera = high_direction * (PLANET_RADIUS + 1_300.0);
        for (chunk, state) in manager.chunks.iter_mut().enumerate() {
            let distance = (manager.centers[chunk]
                .dot(high_direction)
                .clamp(-1.0, 1.0)
                .acos()
                * PLANET_RADIUS
                - manager.radii[chunk])
                .max(0.0);
            state.lod = planet_detail::desired_chunk_lod(distance, 1_300.0, 1);
        }
        manager.chunks[0].lod = 3;

        let camera = app
            .world_mut()
            .spawn((MainCamera, Transform::from_translation(high_camera)))
            .id();
        let static_terrain = app.world_mut().spawn((Ground, Transform::default())).id();
        let support = app
            .world_mut()
            .spawn((
                RigidBody::Static,
                Collider::cuboid(10.0, 1.0, 10.0),
                Transform::from_translation(face_zero_position),
                Ground,
            ))
            .id();
        manager.chunks[0].static_ents.push(static_terrain);

        let mut scenery_roots = Vec::with_capacity(DETAIL_BUDGET_TEST_ROOTS * 2);
        for tier in [SceneryTier::Regional, SceneryTier::Local] {
            for _ in 0..DETAIL_BUDGET_TEST_ROOTS {
                let entity = app
                    .world_mut()
                    .spawn((
                        CullRange {
                            meters: 100.0,
                            tier,
                        },
                        Transform::from_translation(face_zero_position),
                        Visibility::Inherited,
                        Ground,
                    ))
                    .id();
                scenery_roots.push(entity);
            }
        }
        let mut structure_roots = Vec::with_capacity(DETAIL_BUDGET_TEST_ROOTS);
        for _ in 0..DETAIL_BUDGET_TEST_ROOTS {
            structure_roots.push(
                app.world_mut()
                    .spawn((Transform::from_translation(face_zero_position), Ground))
                    .id(),
            );
        }
        manager.chunks[0].structures = structure_roots;
        manager.chunks[0].scenery_large = scenery_roots[..DETAIL_BUDGET_TEST_ROOTS].to_vec();
        manager.chunks[0].scenery_small = scenery_roots[DETAIL_BUDGET_TEST_ROOTS..].to_vec();
        manager.chunks[0].large_cursor = DETAIL_BUDGET_TEST_ROOTS;
        manager.chunks[0].small_cursor = DETAIL_BUDGET_TEST_ROOTS;
        manager.cull_order = scenery_roots;
        app.insert_resource(manager);
        app.add_systems(Update, (update_chunk_lods, stream_scenery).chain());

        (app, camera, static_terrain, support)
    }

    #[test]
    fn scene_detail_work_is_bounded_and_reverses_without_losing_support() {
        const MAX_ROOT_CHANGES_PER_UPDATE: usize = 256;
        let (mut app, camera, static_terrain, support) = detail_app();
        let initial_roots = scene_root_ids(&mut app);
        for _ in 0..2 {
            let before = scene_root_ids(&mut app);
            app.update();
            let after = scene_root_ids(&mut app);
            let changed = scene_root_change_count(&before, &after);
            assert!(
                changed <= MAX_ROOT_CHANGES_PER_UPDATE,
                "LOD demotion changed {changed} scene roots in one update"
            );
        }
        let roots_after_first_demotion = scene_root_ids(&mut app).len();
        assert!(roots_after_first_demotion < initial_roots.len());
        let dense_roots_after_first_demotion = {
            let chunk = &app.world().resource::<ChunkManager>().chunks[0];
            chunk.structures.len() + chunk.scenery_large.len() + chunk.scenery_small.len()
        };
        assert!(
            dense_roots_after_first_demotion > 0
                && dense_roots_after_first_demotion < DETAIL_BUDGET_TEST_ROOTS * 3,
            "the dense chunk should be partially demoted before reversal"
        );

        let close_direction = app.world().resource::<ChunkManager>().centers[0];
        app.world_mut()
            .get_mut::<Transform>(camera)
            .unwrap()
            .translation = close_direction * (PLANET_RADIUS + 10.0);

        let mut restored = false;
        for _ in 0..32 {
            let before = scene_root_ids(&mut app);
            app.update();
            let after = scene_root_ids(&mut app);
            let changed = scene_root_change_count(&before, &after);
            assert!(
                changed <= MAX_ROOT_CHANGES_PER_UPDATE,
                "LOD promotion changed {changed} scene roots in one update"
            );
            let manager = app.world().resource::<ChunkManager>();
            let chunk = &manager.chunks[0];
            if chunk.lod == 3
                && chunk.structures.len() == DETAIL_BUDGET_TEST_ROOTS
                && chunk.scenery_large.len() == DETAIL_BUDGET_TEST_ROOTS
                && chunk.scenery_small.len() == DETAIL_BUDGET_TEST_ROOTS
            {
                restored = true;
                break;
            }
        }

        assert!(restored, "dense chunk did not restore its detailed LOD");
        let manager = app.world().resource::<ChunkManager>();
        let chunk = &manager.chunks[0];
        let resident = chunk
            .structures
            .iter()
            .chain(&chunk.scenery_large)
            .chain(&chunk.scenery_small)
            .copied()
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(resident.len(), DETAIL_BUDGET_TEST_ROOTS * 3);
        assert_eq!(manager.cull_order.len(), DETAIL_BUDGET_TEST_ROOTS * 2);
        assert!(app.world().get::<Transform>(static_terrain).is_some());
        assert!(app.world().get::<Collider>(support).is_some());

        let high_direction = -close_direction;
        app.world_mut()
            .get_mut::<Transform>(camera)
            .unwrap()
            .translation = high_direction * (PLANET_RADIUS + 1_300.0);

        let mut cleared = false;
        for _ in 0..32 {
            let before = scene_root_ids(&mut app);
            app.update();
            let after = scene_root_ids(&mut app);
            let changed = scene_root_change_count(&before, &after);
            assert!(
                changed <= MAX_ROOT_CHANGES_PER_UPDATE,
                "LOD demotion changed {changed} scene roots in one update"
            );
            let manager = app.world().resource::<ChunkManager>();
            let chunk = &manager.chunks[0];
            if chunk.lod == 1
                && chunk.structures.is_empty()
                && chunk.scenery_large.is_empty()
                && chunk.scenery_small.is_empty()
            {
                cleared = true;
                break;
            }
        }

        assert!(
            cleared,
            "dense chunk did not converge to its requested coarse LOD"
        );
        assert_eq!(
            scene_root_ids(&mut app).len(),
            initial_roots.len() - DETAIL_BUDGET_TEST_ROOTS * 3
        );
        assert!(app.world().resource::<ChunkManager>().cull_order.is_empty());
        assert!(app.world().get::<Transform>(static_terrain).is_some());
        assert!(app.world().get::<Collider>(support).is_some());
    }

    #[test]
    fn render_spawns_do_not_own_collision_residency() {
        use bevy::ecs::world::CommandQueue;
        let mut world = World::new();
        let mut queue = CommandQueue::default();
        let catalog = AssetCatalog::fixture(&["structure.house", "scenery.tree.0"]);
        let (house, tree) = {
            let mut commands = Commands::new(&mut queue, &world);
            let house = spawn_structure(
                &mut commands,
                &catalog,
                &StructureData {
                    pos: [0., 2000., 0.],
                    face: 0,
                    kind: StructureKind::House,
                    yaw: 0.,
                },
            );
            let tree = spawn_scenery(
                &mut commands,
                &catalog,
                &SceneryData {
                    pos: [0., 2000., 0.],
                    face: 0,
                    kind: SceneryKind::Flora(FloraKind::Tree),
                    variant: 0,
                },
                Visibility::Inherited,
            );
            (house, tree)
        };
        queue.apply(&mut world);
        assert!(world.get::<Collider>(house).is_none());
        assert!(world.get::<Collider>(tree).is_none());
        let visual = world.get::<Children>(house).unwrap()[0];
        assert_eq!(world.get::<Transform>(visual).unwrap().scale, Vec3::ONE);
    }
    /// Fine faces (any subdiv ≥ 2) must map to their subdiv-2 chunk by
    /// fi / faces_per_chunk — the contiguous-slice assumption everything here
    /// rests on. Mirrors the zones.rs subdiv 3→7 proof at the chunk scale.
    /// Tested at subdiv 4; the mapping is per-level recursive, so it holds for
    /// the subdiv-7 render mesh too (zones.rs proves 3→7 directly).
    #[test]
    fn fine_faces_map_to_chunk_by_index() {
        let coarse = PlanetMesh::new(unit_icosphere_tris(2));
        let fine = unit_icosphere_tris(4);
        let faces_per_chunk = fine.len() / CHUNK_COUNT;
        for (fi, t) in fine.iter().enumerate() {
            let cent = ((t[0] + t[1] + t[2]) / 3.0).normalize();
            let hit = coarse.face_at(cent).expect("centroid hits coarse mesh");
            assert_eq!(hit, fi / faces_per_chunk, "fine face {fi}");
        }
    }

    /// Chunked water slices must add up to exactly the whole-planet build —
    /// no face lost or doubled at chunk borders.
    #[test]
    fn chunked_water_covers_planet() {
        use bevy::render::mesh::VertexAttributeValues;
        let fine = unit_icosphere_tris(4);
        let tris: Vec<[[f32; 3]; 3]> = fine
            .iter()
            .map(|t| [t[0].to_array(), t[1].to_array(), t[2].to_array()])
            .collect();
        let faces_per_chunk = tris.len() / CHUNK_COUNT;
        // Water on an arbitrary deterministic subset of faces.
        let water_r: Vec<f32> = (0..tris.len())
            .map(|fi| if fi % 7 == 0 { 100.0 } else { 0.0 })
            .collect();
        let phase = vec![Some(WaterPhase::Liquid); tris.len()];
        let verts = |m: bevy::prelude::Mesh| -> usize {
            match m.attribute(bevy::prelude::Mesh::ATTRIBUTE_POSITION) {
                Some(VertexAttributeValues::Float32x3(p)) => p.len(),
                _ => 0,
            }
        };
        let whole = crate::water::build_water_surface(&tris, &water_r, &phase, 2)
            .map(verts)
            .unwrap_or(0);
        let mut sum = 0;
        for c in 0..CHUNK_COUNT {
            let r = c * faces_per_chunk..(c + 1) * faces_per_chunk;
            sum += crate::water::build_water_surface(
                &tris[r.clone()],
                &water_r[r.clone()],
                &phase[r],
                2,
            )
            .map(verts)
            .unwrap_or(0);
        }
        assert_eq!(sum, whole, "chunk water slices must tile the planet build");
    }

    #[test]
    fn planet_scale_keeps_regional_scenery_and_hides_small_props() {
        let camera = Vec3::X * terra_geometry::sphere::PLANET_RADIUS * 3.0;
        let tree = SceneryData {
            pos: (Vec3::X * terra_geometry::sphere::PLANET_RADIUS).to_array(),
            face: 0,
            kind: SceneryKind::Flora(FloraKind::Tree),
            variant: 0,
        };
        let grass = SceneryData {
            kind: SceneryKind::Flora(FloraKind::Grass),
            ..tree
        };

        assert_eq!(
            initial_scenery_visibility(tree, camera, None),
            Visibility::Inherited
        );
        assert_eq!(
            initial_scenery_visibility(grass, camera, None),
            Visibility::Hidden
        );
    }
}
