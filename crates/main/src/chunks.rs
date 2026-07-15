//! Icosphere chunk streaming + LOD.
//!
//! The render icosphere is split into 320 chunks — the subdivision-2 faces —
//! using the deterministic 4-way child order of `unit_icosphere_tris` (fine
//! face `fi` → chunk `fi / faces_per_chunk`, a contiguous slice; the same
//! scheme `zones.rs` proves at subdiv 3→7). `faces_per_chunk` is derived from
//! the level data, so terrain density can grow without touching this module.
//!
//! Every chunk is ALWAYS resident at LOD 1 (coarse terrain/water/river/ice
//! visuals, spawned synchronously in `setup_map`) so the minimap and
//! fullscreen-map cameras — which render the same world, there are no
//! RenderLayers — never see holes. Closer chunks add detail:
//!
//! - LOD 1: terrain slice, flat water, river, ice visual
//! - LOD 2 (≤ 960 m): + structures (GLB + colliders), large flora, water subdiv 1
//! - LOD 3 (≤ 300 m): + all flora, water subdiv 2
//!
//! Downgrades use 15% hysteresis; rebuilds are budgeted per frame. Colliders
//! for terrain/ice/bridges stay whole-planet in `setup_map` (physics never
//! streams — no fall-through at chunk borders, teleports just work); only
//! flora/structure colliders live in chunks.

use crate::asset_catalog::AssetCatalog;
use crate::map::{CullRange, Ground, MainCamera, flora_cull};
use avian3d::prelude::*;
use bevy::prelude::*;
use shared::art::AssetName;
use shared::level::{FloraData, FloraKind, StructureData, StructureKind, WaterPhase};
use shared::sphere::PLANET_RADIUS;

/// Chunk count: the subdivision-2 icosphere faces (20 × 4²).
pub const CHUNK_COUNT: usize = 320;

/// Debug: outline every chunk in its LOD color (red = 1, yellow = 2,
/// green = 3) so LOD rings are visually inspectable. Flip off to ship.
const DEBUG_CHUNK_BORDERS: bool = true;

/// Great-circle distances (m) for LOD entry. Zombie ring (120 m) sits well
/// inside LOD 3, so actors always stand on fully-loaded chunks.
const LOD3_DIST: f32 = 300.0;
const LOD2_DIST: f32 = 960.0;
/// Downgrade only past entry × this, so chunks don't thrash on the boundary.
const HYSTERESIS: f32 = 1.15;
/// Chunk rebuilds allowed per frame. Meshes and structures spawn with the
/// rebuild; flora streams separately (below), so this only spreads mesh churn.
const REBUILDS_PER_FRAME: usize = 8;
/// Flora entities spawned per frame across all chunks. A dense forest chunk
/// (~6.7k props) fills in a few frames — imperceptible next to the distance
/// fog — instead of one hitchy burst.
const FLORA_PER_FRAME: usize = 1500;

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

/// Large flora visible from afar — resident from LOD 2; the rest joins at LOD 3.
fn flora_min_lod(kind: FloraKind) -> u8 {
    match kind {
        FloraKind::Tree | FloraKind::DeadTree | FloraKind::Rock | FloraKind::Log => 2,
        _ => 3,
    }
}

/// Baked per-chunk world data, sliced/binned once at setup from `LevelData`.
pub struct ChunkData {
    pub terrain_tris: Vec<[[f32; 3]; 3]>,
    pub terrain_colors: Vec<[[f32; 4]; 3]>,
    pub water_r: Vec<f32>,
    pub river_r: Vec<[f32; 3]>,
    pub water_phase: Vec<Option<WaterPhase>>,
    pub flora: Vec<Vec<FloraData>>,
    pub structures: Vec<Vec<StructureData>>,
}

#[derive(Resource)]
pub struct ChunkManager {
    pub data: ChunkData,
    pub faces_per_chunk: usize,
    /// Unit direction of each chunk's centroid.
    pub centers: Vec<Vec3>,
    /// Current LOD per chunk; 0 = not yet spawned.
    pub lods: Vec<u8>,
    /// Everything spawned for the chunk (meshes, flora, structures).
    pub entities: Vec<Vec<Entity>>,
    /// Per-chunk cursor into `data.flora`: entities below it are spawned.
    /// Reset on every rebuild; the streaming pass advances it.
    pub flora_cursor: Vec<usize>,
    pub terrain_mat: Handle<StandardMaterial>,
    pub water_mat: Handle<crate::water::WaterMaterial>,
    pub river_mat: Handle<crate::water::WaterMaterial>,
    pub ice_mat: Handle<StandardMaterial>,
    /// Debug border material per LOD (index = lod - 1); see DEBUG_CHUNK_BORDERS.
    pub border_mats: [Handle<StandardMaterial>; 3],
}

impl ChunkManager {
    /// Bin baked level data into the 320 chunks. Flora/structures land in
    /// their chunk via the baked render-face index (`face / faces_per_chunk`).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        terrain_tris: Vec<[[f32; 3]; 3]>,
        terrain_colors: Vec<[[f32; 4]; 3]>,
        water_r: Vec<f32>,
        river_r: Vec<[f32; 3]>,
        water_phase: Vec<Option<WaterPhase>>,
        flora: Vec<FloraData>,
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
        let centers = (0..CHUNK_COUNT)
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
        let mut chunk_flora = vec![Vec::new(); CHUNK_COUNT];
        for f in flora {
            chunk_flora[f.face as usize / faces_per_chunk].push(f);
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
                flora: chunk_flora,
                structures: chunk_structures,
            },
            faces_per_chunk,
            centers,
            lods: vec![0; CHUNK_COUNT],
            entities: vec![Vec::new(); CHUNK_COUNT],
            flora_cursor: vec![0; CHUNK_COUNT],
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

    let mut budget = REBUILDS_PER_FRAME;
    for chunk in 0..CHUNK_COUNT {
        if budget == 0 {
            return;
        }
        let dist = mgr.centers[chunk].dot(eye_dir).clamp(-1.0, 1.0).acos() * PLANET_RADIUS;
        let cur = mgr.lods[chunk];
        let want = desired_lod(dist);
        let rebuild = if want > cur {
            true
        } else if want < cur {
            // Hysteresis: drop out of `cur` only past its entry distance + 15%.
            dist > lod_entry(cur) * HYSTERESIS
        } else {
            false
        };
        if rebuild {
            respawn_chunk(&mut commands, &mut meshes, &catalog, &mut mgr, chunk, want);
            // First-ever build (cur == 0) is free: the whole planet must appear
            // on frame one (setup_map used to build it synchronously), only
            // steady-state LOD churn is budgeted.
            if cur != 0 {
                budget -= 1;
            }
        }
    }
}

fn respawn_chunk(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    catalog: &AssetCatalog,
    mgr: &mut ChunkManager,
    chunk: usize,
    lod: u8,
) {
    for e in mgr.entities[chunk].drain(..) {
        commands.entity(e).try_despawn();
    }
    let range = mgr.face_range(chunk);
    let mut spawned = Vec::new();

    // Terrain slice (visual only — the whole-planet collider lives in setup_map).
    let terrain = crate::map::build_visual_mesh(
        &mgr.data.terrain_tris[range.clone()],
        &mgr.data.terrain_colors[range.clone()],
    );
    spawned.push(
        commands
            .spawn((
                Mesh3d(meshes.add(terrain)),
                MeshMaterial3d(mgr.terrain_mat.clone()),
                Transform::default(),
                Ground,
            ))
            .id(),
    );

    let tris = &mgr.data.terrain_tris[range.clone()];
    let water_r = &mgr.data.water_r[range.clone()];
    let river_r = &mgr.data.river_r[range.clone()];
    let phase = &mgr.data.water_phase[range.clone()];

    if let Some(water) = crate::water::build_water_surface(tris, water_r, phase, water_subdiv(lod))
    {
        spawned.push(
            commands
                .spawn((
                    Mesh3d(meshes.add(water)),
                    MeshMaterial3d(mgr.water_mat.clone()),
                    Transform::default(),
                    Ground,
                ))
                .id(),
        );
    }

    if let Some(river) = crate::water::build_river_surfaces(tris, river_r, phase) {
        spawned.push(
            commands
                .spawn((
                    Mesh3d(meshes.add(river)),
                    MeshMaterial3d(mgr.river_mat.clone()),
                    Transform::default(),
                    Ground,
                ))
                .id(),
        );
    }

    // Ice visual (the whole-planet ice collider lives in setup_map).
    if let Some((ice, _)) = crate::water::build_ice_surface(tris, water_r, river_r, phase) {
        spawned.push(
            commands
                .spawn((
                    Mesh3d(meshes.add(ice)),
                    MeshMaterial3d(mgr.ice_mat.clone()),
                    Transform::default(),
                    Ground,
                ))
                .id(),
        );
    }

    if lod >= 2 {
        for s in &mgr.data.structures[chunk] {
            spawned.push(spawn_structure(commands, catalog, s));
        }
    }

    if DEBUG_CHUNK_BORDERS {
        spawned.push(
            commands
                .spawn((
                    Mesh3d(meshes.add(build_border_mesh(tris))),
                    MeshMaterial3d(mgr.border_mats[(lod - 1) as usize].clone()),
                    Transform::default(),
                    bevy::light::NotShadowCaster,
                    Ground,
                ))
                .id(),
        );
    }

    mgr.entities[chunk] = spawned;
    mgr.lods[chunk] = lod;
    // Flora streams in over the following frames (stream_flora); despawned
    // flora is already gone via the entity drain above.
    mgr.flora_cursor[chunk] = 0;
}

/// Spawn pending flora for loaded chunks, a bounded number per frame. Nearest
/// chunks first so the ground cover around the player fills before tree lines
/// on the horizon.
pub fn stream_flora(
    mut commands: Commands,
    catalog: Res<AssetCatalog>,
    camera: Query<&Transform, With<MainCamera>>,
    mut mgr: ResMut<ChunkManager>,
) {
    let Some(cam) = camera.iter().next() else {
        return;
    };
    let eye_dir = cam.translation.normalize_or(Vec3::Y);
    let mut order: Vec<usize> = (0..CHUNK_COUNT)
        .filter(|&c| mgr.flora_cursor[c] < mgr.data.flora[c].len())
        .collect();
    if order.is_empty() {
        return;
    }
    order.sort_by(|&a, &b| {
        mgr.centers[b]
            .dot(eye_dir)
            .total_cmp(&mgr.centers[a].dot(eye_dir))
    });

    let mut budget = FLORA_PER_FRAME;
    for chunk in order {
        let lod = mgr.lods[chunk];
        if lod < 2 {
            // No flora at LOD 1 — skip the whole list instead of walking it.
            mgr.flora_cursor[chunk] = mgr.data.flora[chunk].len();
            continue;
        }
        while budget > 0 {
            let i = mgr.flora_cursor[chunk];
            let Some(f) = mgr.data.flora[chunk].get(i) else {
                break;
            };
            if lod >= flora_min_lod(f.kind) {
                let f = *f;
                let e = spawn_flora(&mut commands, &catalog, &f);
                mgr.entities[chunk].push(e);
                budget -= 1;
            }
            mgr.flora_cursor[chunk] = i + 1;
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

fn spawn_structure(commands: &mut Commands, catalog: &AssetCatalog, s: &StructureData) -> Entity {
    let pos = Vec3::from_array(s.pos);
    let up = pos.normalize();
    let rotation = Quat::from_rotation_arc(Vec3::Y, up) * Quat::from_rotation_y(s.yaw);
    let scale = match s.kind {
        StructureKind::Ruin => Vec3::new(6.0, 3.0, 6.0),
        StructureKind::Watchtower => Vec3::new(4.5, 12.0, 4.5),
        StructureKind::Dock => Vec3::new(3.0, 0.8, 10.0),
        StructureKind::Farm => Vec3::new(9.0, 0.3, 9.0),
        StructureKind::Wall => Vec3::new(6.0, 3.0, 1.2),
        StructureKind::Well => Vec3::new(2.0, 1.2, 2.0),
        StructureKind::Campfire => Vec3::splat(1.6),
    };
    let mut root = commands.spawn((
        Transform::from_translation(pos).with_rotation(rotation),
        Visibility::default(),
        Ground,
    ));
    if !matches!(s.kind, StructureKind::Farm | StructureKind::Campfire) {
        root.insert((
            RigidBody::Static,
            Collider::cuboid(scale.x * 0.5, scale.y * 0.5, scale.z * 0.5),
        ));
    }
    root.with_child((
        WorldAssetRoot(catalog.scene(s.kind.asset_name())),
        Transform::from_scale(scale),
    ));
    root.id()
}

fn spawn_flora(commands: &mut Commands, catalog: &AssetCatalog, f: &FloraData) -> Entity {
    let pos = Vec3::from_array(f.pos);
    let up = pos.normalize();
    let hash = f.pos[0].to_bits()
        ^ f.pos[1].to_bits().rotate_left(13)
        ^ f.pos[2].to_bits().rotate_left(27);
    let scale = 0.7 + (hash & 0xff) as f32 / 255.0 * 0.6;
    let yaw = (hash >> 8 & 0xff) as f32 / 255.0 * std::f32::consts::TAU;
    let rotation = Quat::from_rotation_arc(Vec3::Y, up) * Quat::from_rotation_y(yaw);

    let mut root = commands.spawn((
        CullRange(flora_cull(f.kind)),
        Transform::from_translation(pos).with_rotation(rotation),
        Visibility::default(),
        bevy::light::NotShadowCaster,
        Ground,
    ));

    match shared::art::flora_collider(f.kind) {
        shared::art::ColliderSpec::None => {}
        shared::art::ColliderSpec::Box { half_extents } => {
            root.insert((
                RigidBody::Static,
                Collider::cuboid(
                    half_extents[0] * scale,
                    half_extents[1] * scale,
                    half_extents[2] * scale,
                ),
                Friction::ZERO,
                Restitution::ZERO,
            ));
        }
        shared::art::ColliderSpec::Capsule {
            radius,
            half_length,
        } => {
            root.insert((
                RigidBody::Static,
                Collider::capsule(radius * scale, half_length * 2.0 * scale),
            ));
        }
    }

    root.with_child((
        WorldAssetRoot(catalog.scene(f.kind.asset_name())),
        Transform::from_scale(Vec3::splat(scale)),
    ));
    root.id()
}

#[cfg(test)]
mod tests {
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
            sum += crate::water::build_water_surface(&tris[r.clone()], &water_r[r.clone()], &phase[r], 2)
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
