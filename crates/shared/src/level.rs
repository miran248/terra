use serde::{Deserialize, Serialize};

/// Bump on any incompatible LevelData change so stale binaries fail loudly.
pub const LEVEL_FORMAT_VERSION: u32 = 6;

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
    pub terrain_colors: Vec<[f32; 4]>,
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
