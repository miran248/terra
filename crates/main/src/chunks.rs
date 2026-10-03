//! Icosphere chunk streaming + LOD.
//!
//! The render icosphere is split into 320 chunks — the subdivision-2 faces —
//! using the deterministic 4-way child order of `unit_icosphere_tris` (fine
//! face `fi` → chunk `fi / faces_per_chunk`, a contiguous slice; the same
//! scheme `zones.rs` proves at subdiv 3→7). `faces_per_chunk` is derived from
//! the level data, so terrain density can grow without touching this module.
//!
//! Every chunk is ALWAYS resident at LOD 1 (terrain/water/river/ice visuals,
//! whole planet built on the first frames) so the minimap and fullscreen-map
//! cameras — which render the same world, there are no RenderLayers — never
//! see holes. Closer chunks add detail:
//!
//! - LOD 1: terrain slice, flat water, river, ice visual
//! - LOD 2 (≤ 960 m to chunk edge): + structures (GLB + colliders), large scenery
//! - LOD 3 (≤ 300 m to chunk edge): + small scenery, water swell subdivision
//!
//! Transitions are INCREMENTAL: static meshes (terrain/river/ice/border) are
//! built once and never respawned; the water mesh is rebuilt only when its
//! subdivision changes (LOD 3 boundary); structures and scenery are added or
//! removed by delta. Nothing already on screen is torn down and re-added, so
//! a LOD bounce never blinks the world (or the minimap).
//!
//! Downgrades use 15% hysteresis; scenery spawning is budgeted per frame.
//! Colliders for terrain/ice/bridges stay whole-planet in `setup_map`
//! (physics never streams — no fall-through at chunk borders, teleports just
//! work); only scenery/structure colliders live in chunks.

use crate::asset_catalog::AssetCatalog;
use crate::map::{CullRange, Ground, MainCamera, scenery_cull};
use avian3d::prelude::*;
use bevy::prelude::*;
use shared::art::AssetName;
use shared::level::{FloraKind, SceneryData, SceneryKind, StructureData, WaterPhase};
use shared::sphere::PLANET_RADIUS;

/// Chunk count: the subdivision-2 icosphere faces (20 × 4²).
pub const CHUNK_COUNT: usize = 320;

/// Debug: outline every chunk in its LOD color (red = 1, yellow = 2,
/// green = 3) so LOD rings are visually inspectable. Flip off to ship.
const DEBUG_CHUNK_BORDERS: bool = false;

/// Great-circle distances (m, camera → chunk edge) for LOD entry. Zombie ring
/// (120 m) sits well inside LOD 3, so actors always stand on loaded chunks.
const LOD3_DIST: f32 = 300.0;
const LOD2_DIST: f32 = 960.0;
/// Downgrade only past entry × this, so chunks don't thrash on the boundary.
const HYSTERESIS: f32 = 1.15;
/// LOD transitions applied per frame (water rebuilds + structure batches).
const TRANSITIONS_PER_FRAME: usize = 8;
/// Scenery entities spawned per frame across all chunks. A dense forest chunk
/// (~6.7k props) fills in a few frames — imperceptible next to the distance
/// fog — instead of one hitchy burst.
const SCENERY_PER_FRAME: usize = 1500;

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

/// Large scenery visible from afar — resident from LOD 2; the rest joins at LOD 3.
fn is_large_scenery(kind: SceneryKind) -> bool {
    matches!(
        kind,
        SceneryKind::Flora(FloraKind::Tree)
            | SceneryKind::DeadTree
            | SceneryKind::Rock
            | SceneryKind::Log
    )
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
            if is_large_scenery(f.kind) {
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

fn desired_lod(dist: f32) -> u8 {
    if dist <= LOD3_DIST {
        3
    } else if dist <= LOD2_DIST {
        2
    } else {
        1
    }
}

/// Entry distance of a LOD level (used with hysteresis on downgrades).
fn lod_entry(lod: u8) -> f32 {
    match lod {
        3 => LOD3_DIST,
        2 => LOD2_DIST,
        _ => f32::INFINITY,
    }
}

/// Promote/demote chunk LODs around the main camera, budgeted per frame.
/// All transitions are incremental — see the module docs.
pub fn update_chunk_lods(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    catalog: Res<AssetCatalog>,
    camera: Query<&Transform, With<MainCamera>>,
    mut mgr: ResMut<ChunkManager>,
) {
    let Some(cam) = camera.iter().next() else {
        return;
    };
    let eye_dir = cam.translation.normalize_or(Vec3::Y);

    let mut budget = TRANSITIONS_PER_FRAME;
    for chunk in 0..CHUNK_COUNT {
        if budget == 0 {
            return;
        }
        // Distance to the chunk's nearest EDGE: 0 when standing inside it.
        let center_dist = mgr.centers[chunk].dot(eye_dir).clamp(-1.0, 1.0).acos() * PLANET_RADIUS;
        let dist = (center_dist - mgr.radii[chunk]).max(0.0);
        let cur = mgr.chunks[chunk].lod;
        let want = desired_lod(dist);
        let transition = if want > cur {
            true
        } else if want < cur {
            // Hysteresis: drop out of `cur` only past its entry distance + 15%.
            dist > lod_entry(cur) * HYSTERESIS
        } else {
            false
        };
        if transition {
            set_chunk_lod(&mut commands, &mut meshes, &catalog, &mut mgr, chunk, want);
            // First-ever build (cur == 0) is free: the whole planet must appear
            // immediately; only steady-state LOD churn is budgeted.
            if cur != 0 {
                budget -= 1;
            }
        }
    }
}

/// Apply a LOD transition incrementally: only what differs between `cur` and
/// `lod` is spawned/despawned. Scenery spawning is deferred to `stream_scenery`.
fn set_chunk_lod(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    catalog: &AssetCatalog,
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

    // Structures: join at LOD 2, leave below it.
    if lod >= 2 && cur < 2 {
        let ents: Vec<Entity> = mgr.data.structures[chunk]
            .iter()
            .map(|s| spawn_structure(commands, catalog, s))
            .collect();
        mgr.chunks[chunk].structures = ents;
    } else if lod < 2 && cur >= 2 {
        for e in mgr.chunks[chunk].structures.drain(..) {
            commands.entity(e).try_despawn();
        }
    }

    // Scenery: only despawn here; spawning streams via `stream_scenery`.
    if lod < 2 && cur >= 2 {
        for e in mgr.chunks[chunk].scenery_large.drain(..) {
            commands.entity(e).try_despawn();
        }
        mgr.chunks[chunk].large_cursor = 0;
    }
    if lod < 3 && cur >= 3 {
        for e in mgr.chunks[chunk].scenery_small.drain(..) {
            commands.entity(e).try_despawn();
        }
        mgr.chunks[chunk].small_cursor = 0;
    }

    mgr.chunks[chunk].lod = lod;
}

/// Spawn pending scenery for loaded chunks, a bounded number per frame. Nearest
/// chunks first so the ground cover around the player fills before tree lines
/// on the horizon.
pub fn stream_scenery(
    mut commands: Commands,
    catalog: Res<AssetCatalog>,
    camera: Query<&Transform, With<MainCamera>>,
    mut mgr: ResMut<ChunkManager>,
) {
    let Some(cam) = camera.iter().next() else {
        return;
    };
    let eye_dir = cam.translation.normalize_or(Vec3::Y);
    let pending = |mgr: &ChunkManager, c: usize| {
        let st = &mgr.chunks[c];
        (st.lod >= 2 && st.large_cursor < mgr.data.scenery_large[c].len())
            || (st.lod >= 3 && st.small_cursor < mgr.data.scenery_small[c].len())
    };
    let mut order: Vec<usize> = (0..CHUNK_COUNT).filter(|&c| pending(&mgr, c)).collect();
    if order.is_empty() {
        return;
    }
    order.sort_by(|&a, &b| {
        mgr.centers[b]
            .dot(eye_dir)
            .total_cmp(&mgr.centers[a].dot(eye_dir))
    });

    let mut budget = SCENERY_PER_FRAME;
    for chunk in order {
        let lod = mgr.chunks[chunk].lod;
        // Large scenery (LOD 2+), then small (LOD 3).
        while budget > 0 && lod >= 2 {
            let i = mgr.chunks[chunk].large_cursor;
            let Some(f) = mgr.data.scenery_large[chunk].get(i).copied() else {
                break;
            };
            let e = spawn_scenery(&mut commands, &catalog, &f);
            mgr.chunks[chunk].scenery_large.push(e);
            mgr.chunks[chunk].large_cursor = i + 1;
            budget -= 1;
        }
        while budget > 0 && lod >= 3 {
            let i = mgr.chunks[chunk].small_cursor;
            let Some(f) = mgr.data.scenery_small[chunk].get(i).copied() else {
                break;
            };
            let e = spawn_scenery(&mut commands, &catalog, &f);
            mgr.chunks[chunk].scenery_small.push(e);
            mgr.chunks[chunk].small_cursor = i + 1;
            budget -= 1;
        }
        if budget == 0 {
            return;
        }
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
    if let Some(collider) =
        crate::asset_collision::candidate_collider(s.kind.asset_name(), Vec3::ONE)
    {
        root.insert((RigidBody::Static, collider, WorldObstacle));
    }
    root.with_child((
        WorldAssetRoot(catalog.scene(s.kind.asset_name())),
        Transform::default(),
    ));
    root.id()
}

fn spawn_scenery(commands: &mut Commands, catalog: &AssetCatalog, f: &SceneryData) -> Entity {
    let pos = Vec3::from_array(f.pos);
    let up = pos.normalize();
    let hash = f.pos[0].to_bits()
        ^ f.pos[1].to_bits().rotate_left(13)
        ^ f.pos[2].to_bits().rotate_left(27);
    let scale = 0.7 + (hash & 0xff) as f32 / 255.0 * 0.6;
    let yaw = (hash >> 8 & 0xff) as f32 / 255.0 * std::f32::consts::TAU;
    let rotation = Quat::from_rotation_arc(Vec3::Y, up) * Quat::from_rotation_y(yaw);

    let mut root = commands.spawn((
        CullRange(scenery_cull(f.kind)),
        Transform::from_translation(pos).with_rotation(rotation),
        Visibility::default(),
        bevy::light::NotShadowCaster,
        Ground,
    ));

    let name = shared::art::scenery_variant_name(f.kind, f.variant as u32);
    if let Some(collider) = crate::asset_collision::candidate_collider(&name, Vec3::splat(scale)) {
        root.insert((
            RigidBody::Static,
            WorldObstacle,
            collider,
            Friction::ZERO,
            Restitution::ZERO,
        ));
    }

    root.with_child((
        WorldAssetRoot(catalog.scene(&shared::art::scenery_variant_name(f.kind, f.variant as u32))),
        Transform::from_scale(Vec3::splat(scale)),
    ));
    root.id()
}

#[cfg(test)]
mod tests {
    use shared::level::StructureKind;
    #[test]
    fn production_spawns_use_meter_shapes_and_leave_tree_canopy_clear() {
        use super::*;
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
            );
            (house, tree)
        };
        queue.apply(&mut world);
        assert!(world.get::<WorldObstacle>(house).is_some());
        assert!(world.get::<WorldObstacle>(tree).is_some());
        let house_shape = world.get::<Collider>(house).unwrap();
        assert!(!house_shape.contains_point(Vec3::ZERO, Quat::IDENTITY, Vec3::new(0., 0.8, 0.)));
        let visual = world.get::<Children>(house).unwrap()[0];
        assert_eq!(world.get::<Transform>(visual).unwrap().scale, Vec3::ONE);
        let tree_shape = world.get::<Collider>(tree).unwrap();
        assert!(!tree_shape.contains_point(Vec3::ZERO, Quat::IDENTITY, Vec3::new(0.4, 0.5, 0.)));
    }
    use super::*;
    use shared::planet::{PlanetMesh, unit_icosphere_tris};

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
    fn lod_ladder_and_hysteresis() {
        assert_eq!(desired_lod(0.0), 3);
        assert_eq!(desired_lod(LOD3_DIST), 3);
        assert_eq!(desired_lod(LOD3_DIST + 1.0), 2);
        assert_eq!(desired_lod(LOD2_DIST), 2);
        assert_eq!(desired_lod(LOD2_DIST + 1.0), 1);
        // Inside the hysteresis band a LOD-3 chunk must not drop.
        let d = LOD3_DIST * 1.10;
        assert!(desired_lod(d) < 3 && d <= lod_entry(3) * HYSTERESIS);
    }
}
