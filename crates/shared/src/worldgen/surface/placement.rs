/// Sub-tile decoration scatter. Flora are points over the finished mesh.
/// Placement is deterministic in face order.
pub(in crate::worldgen) const FLORA_RNG_SALT: u64 = 0x466c_6f72;

/// How a flora kind's density responds to ground moisture.
#[derive(Clone, Copy)]
pub(in crate::worldgen) enum FloraScale {
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
pub(in crate::worldgen) fn flora_density(t: Terrain) -> Vec<(f32, FloraScale, FloraKind)> {
    use FloraScale::*;
    // (base, scale, kind). Kept sparse: only the kinds that grow on this tile.
    let v: &[(f32, FloraScale, FloraKind)] = match t {
        Terrain::Forest => &[
            (0.40, Wet, FloraKind::Tree),
            (0.10, Wet, FloraKind::Bush),
            (0.012, WetSq, FloraKind::Flower),
            (0.008, Dry, FloraKind::Rock),
            (0.075, Wet, FloraKind::Grass),
            (0.03, Flat, FloraKind::Log),
            (0.06, Wet, FloraKind::Mushroom),
            (0.04, Wet, FloraKind::Berry),
            (0.01, Flat, FloraKind::DeadTree),
            (0.02, Flat, FloraKind::Stump),
            (0.04, Wet, FloraKind::Fern),
        ],
        Terrain::Jungle => &[
            (0.55, Wet, FloraKind::Tree),
            (0.18, Wet, FloraKind::Bush),
            (0.02, WetSq, FloraKind::Flower),
            (0.004, Dry, FloraKind::Rock),
            (0.10, Wet, FloraKind::Grass),
            (0.05, Flat, FloraKind::Log),
            (0.09, Wet, FloraKind::Mushroom),
            (0.05, Wet, FloraKind::Berry),
            (0.02, Wet, FloraKind::Reed),
            (0.04, WetSq, FloraKind::Vine),
        ],
        Terrain::Swamp => &[
            (0.05, Wet, FloraKind::Tree),
            (0.12, Wet, FloraKind::Bush),
            (0.03, WetSq, FloraKind::Flower),
            (0.004, Dry, FloraKind::Rock),
            (0.10, Wet, FloraKind::Grass),
            (0.05, Flat, FloraKind::Log),
            (0.05, Wet, FloraKind::Mushroom),
            (0.06, Flat, FloraKind::DeadTree),
            (0.18, Wet, FloraKind::Reed),
            (0.06, Wet, FloraKind::Cattail),
            (0.03, WetSq, FloraKind::Vine),
        ],
        Terrain::Plains => &[
            (0.01, Wet, FloraKind::Tree),
            (0.025, Wet, FloraKind::Bush),
            (0.075, WetSq, FloraKind::Flower),
            (0.005, Dry, FloraKind::Rock),
            (0.088, Wet, FloraKind::Grass),
            (0.004, Flat, FloraKind::Log),
            (0.02, Wet, FloraKind::Berry),
            (0.015, Flat, FloraKind::Fern),
        ],
        Terrain::Savanna => &[
            (0.02, Wet, FloraKind::Tree),
            (0.04, Wet, FloraKind::Bush),
            (0.04, WetSq, FloraKind::Flower),
            (0.008, Dry, FloraKind::Rock),
            (0.11, Wet, FloraKind::Grass),
            (0.008, Flat, FloraKind::Log),
            (0.015, Dry, FloraKind::Cactus),
            (0.01, Wet, FloraKind::Berry),
            (0.02, Flat, FloraKind::DeadTree),
            (0.012, Flat, FloraKind::Tumbleweed),
        ],
        Terrain::Tundra => &[
            (0.003, Wet, FloraKind::Tree),
            (0.015, Wet, FloraKind::Bush),
            (0.005, WetSq, FloraKind::Flower),
            (0.03, Dry, FloraKind::Rock),
            (0.012, Wet, FloraKind::Grass),
            (0.01, Flat, FloraKind::Log),
            (0.008, Wet, FloraKind::Berry),
            (0.03, Flat, FloraKind::DeadTree),
            (0.02, Flat, FloraKind::Snowdrift),
            (0.015, Dry, FloraKind::Icicle),
            (0.002, Flat, FloraKind::Snowman),
        ],
        Terrain::Desert => &[
            (0.012, Wet, FloraKind::Bush),
            (0.025, Dry, FloraKind::Rock),
            (0.06, Dry, FloraKind::Cactus),
            (0.02, Flat, FloraKind::DeadTree),
            (0.008, Flat, FloraKind::Skull),
            (0.04, Flat, FloraKind::Tumbleweed),
        ],
        Terrain::Lake | Terrain::River => &[
            (0.15, Wet, FloraKind::Lilypad),
            (0.08, Wet, FloraKind::Seaweed),
            (0.04, Wet, FloraKind::Kelp),
        ],
        Terrain::Ocean => &[
            (0.05, Flat, FloraKind::Coral),
            (0.12, Wet, FloraKind::Seaweed),
            (0.06, Wet, FloraKind::Kelp),
            (0.04, Flat, FloraKind::Anemone),
            (0.03, Flat, FloraKind::Starfish),
            (0.015, Flat, FloraKind::Shell),
        ],
        Terrain::RiverBank | Terrain::LakeShore => &[
            (0.02, Wet, FloraKind::Tree),
            (0.05, Wet, FloraKind::Bush),
            (0.062, WetSq, FloraKind::Flower),
            (0.008, Dry, FloraKind::Rock),
            (0.075, Wet, FloraKind::Grass),
            (0.01, Flat, FloraKind::Log),
            (0.12, Wet, FloraKind::Reed),
            (0.04, Wet, FloraKind::Cattail),
        ],
        Terrain::Mountain => &[(0.005, Wet, FloraKind::Bush), (0.05, Dry, FloraKind::Rock)],
        Terrain::Cliff => &[(0.038, Dry, FloraKind::Rock)],
        Terrain::Snow => &[
            (0.004, Dry, FloraKind::DeadTree),
            (0.04, Dry, FloraKind::Rock),
            (0.06, Flat, FloraKind::Snowdrift),
            (0.012, Flat, FloraKind::Stump),
            (0.03, Dry, FloraKind::Icicle),
            (0.003, Flat, FloraKind::Snowman),
        ],
        Terrain::Beach => &[(0.01, Dry, FloraKind::Rock), (0.02, Flat, FloraKind::Shell)],
        Terrain::Volcanic => &[(0.06, Dry, FloraKind::Rock)],
        Terrain::Glacier => &[
            (0.01, Dry, FloraKind::Rock),
            (0.04, Flat, FloraKind::Snowdrift),
            (0.02, Dry, FloraKind::Icicle),
        ],
        _ => &[],
    };
    v.to_vec()
}

pub(in crate::worldgen) fn place_flora(
    grid: &Grid,
    terrain: &TerrainGen,
    tiles: &[Terrain],
    painted: &Painted,
    mesh_tris: &[[[f32; 3]; 3]],
) -> Vec<FloraData> {
    let mut rng = fastrand::Rng::with_seed(grid.seed as u64 ^ FLORA_RNG_SALT);
    let mut out = Vec::new();
    for face_index in 0..grid.face_count() {
        let face = FaceId::new(face_index);
        let clear = painted_corners(grid, &painted.roads, face) > 0
            || painted_corners(grid, &painted.settlements, face) > 0
            || painted_corners(grid, &painted.bridge_entries, face) > 0
            || painted_corners(grid, &painted.bridges, face) > 0;
        if clear {
            continue;
        }
        let mix = flora_density(tiles[face_index]);
        if mix.is_empty() {
            continue;
        }
        // Moisture in roughly [-1, 1]: scale greens up on wet ground, rocks
        // up on dry ground. One sample per face keeps it cheap.
        let m = terrain.moisture_at(grid.centroid(FaceId::new(face_index)));
        let wet = (1.0 + m).clamp(0.3, 1.8);
        let dry = (1.0 - m).clamp(0.5, 1.6);
        for &(base, scale, kind) in &mix {
            let multiplier = if matches!(kind, FloraKind::Grass) {
                300.0
            } else if matches!(kind, FloraKind::Rock) {
                1.0
            } else {
                3.0
            };
            let density =
                base * match scale {
                    FloraScale::Wet => wet,
                    FloraScale::WetSq => wet * wet,
                    FloraScale::Dry => dry,
                    FloraScale::Flat => 1.0,
                } * multiplier;
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
                let t = &mesh_tris[face_index];
                let (a, b, c) = (
                    Vec3::from_array(t[0]),
                    Vec3::from_array(t[1]),
                    Vec3::from_array(t[2]),
                );
                let pos = a + (b - a) * u + (c - a) * v;
                let hash = (pos.x.to_bits() as u64).wrapping_mul(0x9e37_79b9)
                    ^ (pos.z.to_bits() as u64).rotate_left(17);
                out.push(FloraData {
                    pos: pos.to_array(),
                    face: face_index as u32,
                    kind,
                    variant: crate::art::flora_variant_for(kind, tiles[face_index], hash),
                });
            }
        }
    }
    out
}

pub(in crate::worldgen) const STRUCT_RNG_SALT: u64 = 0x0053_7475_6375_7265;

/// Contextual structures, placed like towns/bridges: wells, campfires and
/// farms cluster in and around towns; walls ring town edges; docks reach out
/// from coastal town shores; watchtowers crown high ground near roads; ruins
/// scatter through the wilderness. Positions sit on the displaced mesh (face
/// centroids). Deterministic: one seeded stream in face order.
pub(in crate::worldgen) fn place_structures(
    grid: &Grid,
    terrain: &TerrainGen,
    tiles: &[Terrain],
    painted: &Painted,
    slope_class: &[SlopeClass],
    mesh_tris: &[[[f32; 3]; 3]],
    settlement_structures: &[StructureSite],
) -> Vec<StructureData> {
    let mut rng = fastrand::Rng::with_seed(grid.seed as u64 ^ STRUCT_RNG_SALT);
    let face_center = |face_index: usize| {
        let t = &mesh_tris[face_index];
        (Vec3::from_array(t[0]) + Vec3::from_array(t[1]) + Vec3::from_array(t[2])) / 3.0
    };
    let settlement =
        |face_index: usize| face_solid(grid, &painted.settlements, FaceId::new(face_index));
    let road = |face_index: usize| face_solid(grid, &painted.roads, FaceId::new(face_index));
    let feature = |face_index: usize| {
        settlement(face_index)
            || road(face_index)
            || painted_corners(grid, &painted.bridges, FaceId::new(face_index)) > 0
            || painted_corners(grid, &painted.bridge_entries, FaceId::new(face_index)) > 0
    };
    // Face-step distance from any town (capped) — cheap context for the rest.
    let town_sources: Vec<_> = grid
        .topology
        .faces()
        .filter(|face| settlement(face.index()))
        .collect();
    let town_field = grid.topology.face_distances(&town_sources, 6);
    let town_dist: Vec<u16> = grid
        .topology
        .faces()
        .map(|face| {
            town_field
                .face_steps(face)
                .map_or(u16::MAX, |steps| steps as u16)
        })
        .collect();
    let road_near = |face_index: usize| {
        let face = grid
            .topology
            .face(face_index)
            .expect("face index from topology range");
        grid.topology
            .face_neighbors(face)
            .iter()
            .any(|neighbor| road(neighbor.index()))
            || road(face_index)
    };

    let mut out = Vec::new();
    let mut required_faces = std::collections::BTreeSet::new();
    // A structure needs buildable ground: skip any face with a steep/cliff
    // corner.
    let buildable = |face_index: usize| {
        grid.face_cells(FaceId::new(face_index))
            .map(CellId::index)
            .into_iter()
            .all(|cell| slope_class[cell].is_walkable())
    };

    // The network stage planned these structures before internal streets.
    // Resolve each saved barycentric site onto the finished displaced mesh.
    for site in settlement_structures {
        let corners = mesh_tris[site.face_index].map(Vec3::from_array);
        let position = (corners[0] * site.barycentric[0]
            + corners[1] * site.barycentric[1]
            + corners[2] * site.barycentric[2])
            .normalize();
        required_faces.insert(site.face_index);
        push_structure(&mut rng, &mut out, site.face_index, site.kind, position);
    }

    for face_index in 0..grid.face_count() {
        if tiles[face_index].is_water() || !buildable(face_index) {
            continue;
        }
        if required_faces.contains(&face_index) {
            continue;
        }
        // Required settlement structures above define the initial layouts;
        // avoid adding random buildings that change their composition.
        if settlement(face_index) {
            continue;
        }
        // Road decorations: barricades, lamp posts, signposts, guardrails on road faces.
        if road(face_index) {
            let r = rng.f32();
            if r < 0.015 {
                let position = face_center(face_index);
                if clear_of_existing_structures(&out, position, StructureKind::LampPost) {
                    push_structure(
                        &mut rng,
                        &mut out,
                        face_index,
                        StructureKind::LampPost,
                        position,
                    );
                }
            } else if r < 0.025 {
                let position = face_center(face_index);
                if clear_of_existing_structures(&out, position, StructureKind::Signpost) {
                    push_structure(
                        &mut rng,
                        &mut out,
                        face_index,
                        StructureKind::Signpost,
                        position,
                    );
                }
            } else if r < 0.040 {
                let position = face_center(face_index);
                if clear_of_existing_structures(&out, position, StructureKind::Guardrail) {
                    push_structure(
                        &mut rng,
                        &mut out,
                        face_index,
                        StructureKind::Guardrail,
                        position,
                    );
                }
            }
            continue;
        }
        // Bridge decorations: railings and suspension cables.
        let is_bridge = painted_corners(grid, &painted.bridges, FaceId::new(face_index)) > 0
            || painted_corners(grid, &painted.bridge_entries, FaceId::new(face_index)) > 0;
        if is_bridge {
            let r = rng.f32();
            if r < 0.15 {
                let position = face_center(face_index);
                if clear_of_existing_structures(&out, position, StructureKind::Railing) {
                    push_structure(
                        &mut rng,
                        &mut out,
                        face_index,
                        StructureKind::Railing,
                        position,
                    );
                }
            } else if r < 0.17 {
                let position = face_center(face_index);
                if clear_of_existing_structures(&out, position, StructureKind::Suspension) {
                    push_structure(
                        &mut rng,
                        &mut out,
                        face_index,
                        StructureKind::Suspension,
                        position,
                    );
                }
            }
            continue;
        }
        if feature(face_index) {
            continue;
        }
        let inside_settlement =
            terrain
                .settlement_anchors
                .iter()
                .enumerate()
                .any(|(index, &anchor)| {
                    anchor.distance(SpherePos::new(face_center(face_index)))
                        <= terrain
                            .settlement_config()
                            .radius_m(terrain.settlement_kind(index))
                });
        if inside_settlement {
            continue;
        }
        // Farmland: fertile flat ground just outside settlements.
        if town_dist[face_index] <= 3
            && matches!(
                tiles[face_index],
                Terrain::Plains | Terrain::Savanna | Terrain::Forest
            )
            && rng.f32() < 0.10
        {
            let position = face_center(face_index);
            if clear_of_existing_structures(&out, position, StructureKind::Farm) {
                push_structure(
                    &mut rng,
                    &mut out,
                    face_index,
                    StructureKind::Farm,
                    position,
                );
            }
            continue;
        }
        // Watchtower: high ground overlooking a road.
        if road_near(face_index)
            && terrain.elevation_at(grid.centroid(FaceId::new(face_index))) > 0.25
            && rng.f32() < 0.03
        {
            let position = face_center(face_index);
            if clear_of_existing_structures(&out, position, StructureKind::Watchtower) {
                push_structure(
                    &mut rng,
                    &mut out,
                    face_index,
                    StructureKind::Watchtower,
                    position,
                );
            }
            continue;
        }
        // Ruins: rare, deep in the wilderness (far from any town).
        if town_dist[face_index] == u16::MAX
            && !matches!(
                tiles[face_index],
                Terrain::Beach | Terrain::Cliff | Terrain::LakeShore | Terrain::RiverBank
            )
            && rng.f32() < 0.0006
        {
            let position = face_center(face_index);
            if clear_of_existing_structures(&out, position, StructureKind::Ruin) {
                push_structure(
                    &mut rng,
                    &mut out,
                    face_index,
                    StructureKind::Ruin,
                    position,
                );
            }
        }
    }
    out
}

pub(in crate::worldgen) fn structure_footprint_radius(kind: StructureKind) -> f32 {
    let (width, depth): (f32, f32) = match kind {
        StructureKind::Ruin => (6.0, 6.0),
        StructureKind::Watchtower => (4.5, 4.5),
        StructureKind::Dock => (3.0, 10.0),
        StructureKind::Farm => (9.0, 9.0),
        StructureKind::Wall => (6.0, 1.2),
        StructureKind::Well => (2.0, 2.0),
        StructureKind::Campfire => (1.6, 1.6),
        StructureKind::Tent => (3.0, 3.0),
        StructureKind::Crate => (1.5, 1.5),
        StructureKind::Fence => (6.0, 0.6),
        StructureKind::Barricade => (4.0, 0.5),
        StructureKind::LampPost => (1.2, 1.2),
        StructureKind::Signpost => (1.5, 0.3),
        StructureKind::Guardrail => (3.0, 0.4),
        StructureKind::Railing => (3.0, 0.2),
        StructureKind::Suspension => (2.0, 0.6),
        StructureKind::House => (8.0, 6.0),
    };
    (width * width + depth * depth).sqrt() * 0.5
}

fn push_structure(
    rng: &mut fastrand::Rng,
    structures: &mut Vec<StructureData>,
    face_index: usize,
    kind: StructureKind,
    position: Vec3,
) {
    structures.push(StructureData {
        pos: position.to_array(),
        face: face_index as u32,
        kind,
        yaw: rng.f32() * std::f32::consts::TAU,
    });
}

fn clear_of_existing_structures(
    existing: &[StructureData],
    position: Vec3,
    kind: StructureKind,
) -> bool {
    let position = SpherePos::new(position.normalize());
    existing.iter().all(|structure| {
        let other = SpherePos::new(Vec3::from_array(structure.pos).normalize());
        position.distance(other)
            >= structure_footprint_radius(kind) + 1.0 + structure_footprint_radius(structure.kind)
    })
}

pub(in crate::worldgen) fn build_face_tags(grid: &Grid, painted: &Painted) -> Vec<Vec<FaceTag>> {
    let mut tags = Vec::with_capacity(grid.face_count());
    for face_index in 0..grid.face_count() {
        let mut face_tags = Vec::new();
        let face = FaceId::new(face_index);
        if face_solid(grid, &painted.roads, face) {
            face_tags.push(FaceTag::Road);
        }
        if face_solid(grid, &painted.settlements, face) {
            face_tags.push(FaceTag::Settlement);
        }
        if face_solid(grid, &painted.bridges, face) {
            face_tags.push(FaceTag::Bridge);
        }
        if face_solid(grid, &painted.bridge_entries, face) {
            face_tags.push(FaceTag::BridgeEntry);
        }
        tags.push(face_tags);
    }
    tags
}
use bevy::prelude::Vec3;

use crate::level::{FaceTag, FloraData, FloraKind, SlopeClass, StructureData, StructureKind};
use crate::sphere::SpherePos;
use crate::terrain::{Terrain, TerrainGen};
use crate::topology::{CellId, FaceId};
use crate::worldgen::{Grid, Painted, StructureSite, face_solid, painted_corners};
