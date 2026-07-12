use serde::{Deserialize, Serialize};

/// Bump on any incompatible LevelData change so stale binaries fail loudly.
pub const LEVEL_FORMAT_VERSION: u32 = 13;

// Face tag ids (entries in face_tag_data).
pub const TAG_ROAD: u8 = 0;
pub const TAG_TOWN: u8 = 1;
pub const TAG_BRIDGE: u8 = 2;
pub const TAG_BRIDGE_ENTRY: u8 = 3;

/// Sentinels in `face_blend` pairs: the face blends toward a built feature
/// beside it (features are tags, not Terrain kinds). Codes count down from 255;
/// anything ≥ BLEND_FEATURE_MIN is a feature, below is a Terrain discriminant.
pub const BLEND_ROAD: u8 = 255;
pub const BLEND_TOWN: u8 = 254;
pub const BLEND_BRIDGE_ENTRY: u8 = 253;
pub const BLEND_FEATURE_MIN: u8 = 250;

/// Display name for a feature blend code, if it is one.
pub fn blend_feature_name(code: u8) -> Option<&'static str> {
    match code {
        BLEND_ROAD => Some("Road"),
        BLEND_TOWN => Some("Town"),
        BLEND_BRIDGE_ENTRY => Some("Bridge Entry"),
        _ => None,
    }
}

pub fn tag_name(tag: u8) -> &'static str {
    match tag {
        TAG_ROAD => "Road",
        TAG_TOWN => "Town",
        TAG_BRIDGE => "Bridge",
        TAG_BRIDGE_ENTRY => "Bridge Entry",
        _ => "?",
    }
}

#[derive(Serialize, Deserialize)]
pub struct LevelData {
    pub version: u32,
    /// TerrainGen seed — grid, zones, and climate are rebuilt from this.
    pub seed: u32,
    /// The SOLVED per-vertex elevation field (sub=5, ~10k). The tile map shapes
    /// this via the constraint solver at gen time; the runtime loads it directly
    /// (`TerrainGen::from_field`) so mesh, physics, and HUD agree exactly.
    pub vert_elev: Vec<f32>,
    pub terrain_tris: Vec<[[f32; 3]; 3]>,
    /// Per-corner colors: tile identity lives on mesh vertices (hex cells), so
    /// biome boundaries render as gradients across their boundary faces.
    pub terrain_colors: Vec<[[f32; 4]; 3]>,
    pub unit_tris: Vec<[[f32; 3]; 3]>,
    /// Per-face terrain type (precomputed, matches terrain_colors).
    pub face_types: Vec<u8>,
    /// Variable-length tag lists per face: face fi's tags are
    /// `face_tag_data[face_tag_off[fi] as usize..face_tag_off[fi + 1] as usize]`.
    pub face_tag_off: Vec<u32>,
    pub face_tag_data: Vec<u8>,
    /// Inland transition marks: faces on a biome/biome boundary, with the pair
    /// of Terrain kinds they link (as u8 discriminants). The face keeps its own
    /// terrain type; renderers blend colors/textures between the pair.
    pub face_blend: Vec<(u32, u8, u8)>,
    pub settlements: Vec<SettlementData>,
    pub roads: Vec<RoadData>,
    /// Named contiguous feature clusters: oceans, lakes, rivers, beaches,
    /// forests, mountain ranges, towns, roads, …
    pub regions: Vec<RegionData>,
    /// Per-face region reference: 0 = no region, otherwise region id + 1.
    /// Zero is the sentinel because postcard varint-encodes it in one byte.
    /// Decode with `region_index`.
    pub face_region: Vec<u32>,
    /// Sub-tile decoration scatter: trees, bushes, flowers. Points ON the
    /// displaced mesh (they sit exactly on the rendered ground), placed
    /// deterministically at gen time from the tile map.
    pub flora: Vec<FloraData>,
    /// Contextual built structures (ruins, docks, walls, …), placed at gen
    /// time and spawned as runtime entities like bridges.
    pub structures: Vec<StructureData>,
    /// Per-face slope class (SLOPE_*), from the solved field.
    pub slope_class: Vec<u8>,
    /// Per-face water depth class (DEPTH_*) for water faces; 0 on land.
    pub water_depth: Vec<u8>,
}

#[derive(Serialize, Deserialize, Clone, Copy)]
pub struct StructureData {
    pub pos: [f32; 3],
    pub face: u32,
    pub kind: u8,
    /// Facing yaw about the local up (radians).
    pub yaw: f32,
}

/// Per-cell/-face terrain steepness (slope class) from the solved field.
pub const SLOPE_FLAT: u8 = 0;
pub const SLOPE_GENTLE: u8 = 1;
pub const SLOPE_STEEP: u8 = 2;
pub const SLOPE_CLIFF: u8 = 3;

pub fn slope_name(c: u8) -> &'static str {
    match c {
        SLOPE_FLAT => "Flat",
        SLOPE_GENTLE => "Gentle",
        SLOPE_STEEP => "Steep",
        SLOPE_CLIFF => "Cliff",
        _ => "?",
    }
}

/// A cell is walkable/buildable when its slope class is flat or gentle.
pub fn slope_walkable(c: u8) -> bool {
    c <= SLOPE_GENTLE
}

/// Per-cell/-face WATER DEPTH class (from the solved field). The depth analogue
/// of the slope class: identity (ocean/lake/river) is one thing, depth another.
pub const DEPTH_SHALLOW: u8 = 0;
pub const DEPTH_DEEP: u8 = 1;
pub const DEPTH_ABYSS: u8 = 2;

pub fn depth_name(d: u8) -> &'static str {
    match d {
        DEPTH_SHALLOW => "Shallow",
        DEPTH_DEEP => "Deep",
        DEPTH_ABYSS => "Abyss",
        _ => "?",
    }
}

pub const FLORA_TREE: u8 = 0;
pub const FLORA_BUSH: u8 = 1;
pub const FLORA_FLOWER: u8 = 2;
pub const FLORA_ROCK: u8 = 3;
pub const FLORA_GRASS: u8 = 4;
pub const FLORA_LOG: u8 = 5;
pub const FLORA_MUSHROOM: u8 = 6;
pub const FLORA_CACTUS: u8 = 7;
pub const FLORA_BERRY: u8 = 8;
pub const FLORA_DEADTREE: u8 = 9;
pub const FLORA_REED: u8 = 10;

// Structures: built props placed contextually (like towns and bridges).
pub const STRUCT_RUIN: u8 = 0;
pub const STRUCT_WATCHTOWER: u8 = 1;
pub const STRUCT_DOCK: u8 = 2;
pub const STRUCT_FARM: u8 = 3;
pub const STRUCT_WALL: u8 = 4;
pub const STRUCT_WELL: u8 = 5;
pub const STRUCT_CAMPFIRE: u8 = 6;

pub fn structure_name(kind: u8) -> &'static str {
    match kind {
        STRUCT_RUIN => "Ruins",
        STRUCT_WATCHTOWER => "Watchtower",
        STRUCT_DOCK => "Dock",
        STRUCT_FARM => "Farm",
        STRUCT_WALL => "Wall",
        STRUCT_WELL => "Well",
        STRUCT_CAMPFIRE => "Campfire",
        _ => "?",
    }
}

#[derive(Serialize, Deserialize, Clone, Copy)]
pub struct FloraData {
    /// World position on the displaced terrain mesh.
    pub pos: [f32; 3],
    /// The face it sits on (for gameplay queries).
    pub face: u32,
    pub kind: u8,
}

pub const NO_REGION: u32 = 0;

/// The `regions` index a `face_region` value points at, if any.
pub fn region_index(face_region_value: u32) -> Option<usize> {
    (face_region_value != NO_REGION).then(|| face_region_value as usize - 1)
}

impl LevelData {
    pub fn face_tags(&self, fi: usize) -> &[u8] {
        let start = self.face_tag_off[fi] as usize;
        let end = self.face_tag_off[fi + 1] as usize;
        &self.face_tag_data[start..end]
    }
}

/// Runtime lookup of per-face tags without keeping the whole LevelData around.
#[derive(Clone)]
pub struct FaceTags {
    pub off: Vec<u32>,
    pub data: Vec<u8>,
}

impl FaceTags {
    pub fn of(&self, fi: usize) -> &[u8] {
        let start = self.off[fi] as usize;
        let end = self.off[fi + 1] as usize;
        &self.data[start..end]
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct RegionData {
    pub name: String,
    /// Approximate center (unit vector on sphere).
    pub pos: [f32; 3],
    pub kind: RegionKind,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum RegionKind {
    Ocean,
    Lake,
    River,
    Beach,
    Cliff,
    Forest,
    Desert,
    Mountain,
    Plains,
    Tundra,
    Swamp,
    Jungle,
    Savanna,
    Volcano,
    Glacier,
    Town,
    Road,
}

#[derive(Serialize, Deserialize)]
pub struct SettlementData {
    pub name: String,
    pub pos: [f32; 3],
}

#[derive(Serialize, Deserialize)]
pub struct RoadData {
    pub points: Vec<[f32; 3]>,
    pub is_bridge: bool,
}
