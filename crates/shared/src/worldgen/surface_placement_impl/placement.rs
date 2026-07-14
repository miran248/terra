use super::super::*;

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
pub(in crate::worldgen) fn flora_density(t: Terrain) -> Vec<(f32, FloraScale, u8)> {
    use FloraScale::*;
    // (base, scale, kind). Kept sparse: only the kinds that grow on this tile.
    let v: &[(f32, FloraScale, u8)] = match t {
        Terrain::Forest => &[
            (0.40, Wet, FLORA_TREE),
            (0.10, Wet, FLORA_BUSH),
            (0.012, WetSq, FLORA_FLOWER),
            (0.008, Dry, FLORA_ROCK),
            (0.075, Wet, FLORA_GRASS),
            (0.03, Flat, FLORA_LOG),
            (0.06, Wet, FLORA_MUSHROOM),
            (0.04, Wet, FLORA_BERRY),
            (0.01, Flat, FLORA_DEADTREE),
        ],
        Terrain::Jungle => &[
            (0.55, Wet, FLORA_TREE),
            (0.18, Wet, FLORA_BUSH),
            (0.02, WetSq, FLORA_FLOWER),
            (0.004, Dry, FLORA_ROCK),
            (0.10, Wet, FLORA_GRASS),
            (0.05, Flat, FLORA_LOG),
            (0.09, Wet, FLORA_MUSHROOM),
            (0.05, Wet, FLORA_BERRY),
            (0.02, Wet, FLORA_REED),
        ],
        Terrain::Swamp => &[
            (0.05, Wet, FLORA_TREE),
            (0.12, Wet, FLORA_BUSH),
            (0.03, WetSq, FLORA_FLOWER),
            (0.004, Dry, FLORA_ROCK),
            (0.10, Wet, FLORA_GRASS),
            (0.05, Flat, FLORA_LOG),
            (0.05, Wet, FLORA_MUSHROOM),
            (0.06, Flat, FLORA_DEADTREE),
            (0.18, Wet, FLORA_REED),
        ],
        Terrain::Plains => &[
            (0.01, Wet, FLORA_TREE),
            (0.025, Wet, FLORA_BUSH),
            (0.075, WetSq, FLORA_FLOWER),
            (0.005, Dry, FLORA_ROCK),
            (0.088, Wet, FLORA_GRASS),
            (0.004, Flat, FLORA_LOG),
            (0.02, Wet, FLORA_BERRY),
        ],
        Terrain::Savanna => &[
            (0.02, Wet, FLORA_TREE),
            (0.04, Wet, FLORA_BUSH),
            (0.04, WetSq, FLORA_FLOWER),
            (0.008, Dry, FLORA_ROCK),
            (0.11, Wet, FLORA_GRASS),
            (0.008, Flat, FLORA_LOG),
            (0.015, Dry, FLORA_CACTUS),
            (0.01, Wet, FLORA_BERRY),
            (0.02, Flat, FLORA_DEADTREE),
        ],
        Terrain::Tundra => &[
            (0.003, Wet, FLORA_TREE),
            (0.015, Wet, FLORA_BUSH),
            (0.005, WetSq, FLORA_FLOWER),
            (0.03, Dry, FLORA_ROCK),
            (0.012, Wet, FLORA_GRASS),
            (0.01, Flat, FLORA_LOG),
            (0.008, Wet, FLORA_BERRY),
            (0.03, Flat, FLORA_DEADTREE),
        ],
        Terrain::Desert => &[
            (0.012, Wet, FLORA_BUSH),
            (0.025, Dry, FLORA_ROCK),
            (0.06, Dry, FLORA_CACTUS),
            (0.02, Flat, FLORA_DEADTREE),
        ],
        Terrain::RiverBank | Terrain::LakeShore => &[
            (0.02, Wet, FLORA_TREE),
            (0.05, Wet, FLORA_BUSH),
            (0.062, WetSq, FLORA_FLOWER),
            (0.008, Dry, FLORA_ROCK),
            (0.075, Wet, FLORA_GRASS),
            (0.01, Flat, FLORA_LOG),
            (0.12, Wet, FLORA_REED),
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
        let clear = painted_corners(grid, &painted.roads, face_index) > 0
            || painted_corners(grid, &painted.towns, face_index) > 0
            || painted_corners(grid, &painted.bridge_entries, face_index) > 0
            || painted_corners(grid, &painted.bridges, face_index) > 0;
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
            let density = base
                * match scale {
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
                let t = &mesh_tris[face_index];
                let (a, b, c) = (
                    Vec3::from_array(t[0]),
                    Vec3::from_array(t[1]),
                    Vec3::from_array(t[2]),
                );
                let pos = a + (b - a) * u + (c - a) * v;
                out.push(FloraData {
                    pos: pos.to_array(),
                    face: face_index as u32,
                    kind,
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
    slope_class: &[u8],
    mesh_tris: &[[[f32; 3]; 3]],
) -> Vec<StructureData> {
    let mut rng = fastrand::Rng::with_seed(grid.seed as u64 ^ STRUCT_RNG_SALT);
    let face_center = |face_index: usize| {
        let t = &mesh_tris[face_index];
        (Vec3::from_array(t[0]) + Vec3::from_array(t[1]) + Vec3::from_array(t[2])) / 3.0
    };
    let town = |face_index: usize| face_solid(grid, &painted.towns, face_index);
    let road = |face_index: usize| face_solid(grid, &painted.roads, face_index);
    let feature = |face_index: usize| {
        town(face_index)
            || road(face_index)
            || painted_corners(grid, &painted.bridges, face_index) > 0
            || painted_corners(grid, &painted.bridge_entries, face_index) > 0
    };
    // Face-step distance from any town (capped) — cheap context for the rest.
    let town_sources: Vec<_> = grid
        .topology
        .faces()
        .filter(|face| town(face.index()))
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
    let mut push = |rng: &mut fastrand::Rng, face_index: usize, kind: u8| {
        out.push(StructureData {
            pos: face_center(face_index).to_array(),
            face: face_index as u32,
            kind,
            yaw: rng.f32() * std::f32::consts::TAU,
        });
    };
    // A structure needs buildable ground: skip any face with a steep/cliff
    // corner (watchtowers on a ridge are the exception — handled below).
    let buildable = |face_index: usize| {
        grid.face_cells(FaceId::new(face_index))
            .map(CellId::index)
            .into_iter()
            .all(|cell| slope_walkable(slope_class[cell]))
    };
    for face_index in 0..grid.face_count() {
        if tiles[face_index].is_water() || !buildable(face_index) {
            continue;
        }
        // Town interior: a well or a campfire in a clearing.
        if town(face_index) {
            let r = rng.f32();
            if r < 0.010 {
                push(&mut rng, face_index, STRUCT_WELL);
            } else if r < 0.045 {
                push(&mut rng, face_index, STRUCT_CAMPFIRE);
            }
            continue;
        }
        // Town edge (non-town land beside a town): a wall segment or a farm.
        let touches_town = grid
            .face_neighbors(FaceId::new(face_index))
            .map(FaceId::index)
            .into_iter()
            .any(town);
        if touches_town {
            let coastal = grid
                .face_neighbors(FaceId::new(face_index))
                .map(FaceId::index)
                .into_iter()
                .any(|neighbor| tiles[neighbor].is_water());
            if coastal && rng.f32() < 0.5 {
                push(&mut rng, face_index, STRUCT_DOCK);
            } else if rng.f32() < 0.4 {
                push(&mut rng, face_index, STRUCT_WALL);
            }
            continue;
        }
        if feature(face_index) {
            continue;
        }
        // Farmland: fertile flat ground just outside town.
        if town_dist[face_index] <= 3
            && matches!(
                tiles[face_index],
                Terrain::Plains | Terrain::Savanna | Terrain::Forest
            )
            && rng.f32() < 0.10
        {
            push(&mut rng, face_index, STRUCT_FARM);
            continue;
        }
        // Watchtower: high ground overlooking a road.
        if road_near(face_index)
            && terrain.elevation_at(grid.centroid(FaceId::new(face_index))) > 0.25
            && rng.f32() < 0.03
        {
            push(&mut rng, face_index, STRUCT_WATCHTOWER);
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
            push(&mut rng, face_index, STRUCT_RUIN);
        }
    }
    out
}

pub(in crate::worldgen) fn build_face_tags(grid: &Grid, painted: &Painted) -> (Vec<u32>, Vec<u8>) {
    let mut off = Vec::with_capacity(grid.face_count() + 1);
    let mut data = Vec::new();
    off.push(0u32);
    for face_index in 0..grid.face_count() {
        if face_solid(grid, &painted.roads, face_index) {
            data.push(TAG_ROAD);
        }
        if face_solid(grid, &painted.towns, face_index) {
            data.push(TAG_TOWN);
        }
        if face_solid(grid, &painted.bridges, face_index) {
            data.push(TAG_BRIDGE);
        }
        if face_solid(grid, &painted.bridge_entries, face_index) {
            data.push(TAG_BRIDGE_ENTRY);
        }
        off.push(data.len() as u32);
    }
    (off, data)
}
