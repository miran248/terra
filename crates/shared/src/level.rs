use serde::{Deserialize, Serialize};

use crate::terrain::Terrain;

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum FaceTag {
    Road,
    Town,
    Bridge,
    BridgeEntry,
}

impl FaceTag {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Road => "Road",
            Self::Town => "Town",
            Self::Bridge => "Bridge",
            Self::BridgeEntry => "Bridge Entry",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum BlendTarget {
    Terrain(Terrain),
    Road,
    Town,
    BridgeEntry,
}

impl BlendTarget {
    pub const fn name(self) -> Option<&'static str> {
        match self {
            Self::Terrain(_) => None,
            Self::Road => Some("Road"),
            Self::Town => Some("Town"),
            Self::BridgeEntry => Some("Bridge Entry"),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub struct FaceBlend {
    pub face: u32,
    pub base: Terrain,
    pub target: BlendTarget,
}

#[derive(Serialize, Deserialize)]
pub struct LevelData {
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
    pub face_types: Vec<Terrain>,
    /// Per-face water-surface radius (0.0 = dry), from the gen-time water
    /// clustering (`worldgen::water_surface_radii`). The runtime draws each face
    /// at this radius — no runtime clustering. Water-body IDENTITY/naming comes
    /// from the region layer (`face_region`), not a parallel id.
    pub face_water_r: Vec<f32>,
    /// Per-corner river-surface radii (all zero = dry), clustered and smoothed
    /// at generation time from connected River + RiverSpring + RiverBank face
    /// components. Springs anchor at the ground and outlets anchor to their
    /// neighboring lake/ocean waterline; runtime consumes these directly.
    pub face_river_r: Vec<[f32; 3]>,
    pub face_tags: Vec<Vec<FaceTag>>,
    /// Inland transition marks: faces on a biome/biome boundary or beside a
    /// built feature. The face keeps its own terrain type; renderers blend
    /// colors/textures toward the typed target.
    pub face_blend: Vec<FaceBlend>,
    pub settlements: Vec<SettlementData>,
    pub roads: Vec<RoadData>,
    /// Named contiguous feature clusters: oceans, lakes, rivers, beaches,
    /// forests, mountain ranges, towns, roads, …
    pub regions: Vec<RegionData>,
    /// Per-face zero-based region index.
    pub face_region: Vec<Option<u32>>,
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
    /// Per-face macro landform (LANDFORM_*).
    pub landform: Vec<u8>,
    /// Per-face road surface material (ROAD_MAT_*); 0 on non-road faces.
    pub road_material: Vec<u8>,
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

/// Macro LANDFORM (the terrain massif a cell belongs to), the base layer under
/// biome cover and slope-class detail: lowlands, hills, mountains, plateaus,
/// valleys — or water. Drives which biome COVER a cell gets and reads in the
/// HUD as the landform word.
pub const LANDFORM_WATER: u8 = 0;
pub const LANDFORM_LOWLAND: u8 = 1;
pub const LANDFORM_VALLEY: u8 = 2;
pub const LANDFORM_HILLS: u8 = 3;
pub const LANDFORM_MOUNTAINS: u8 = 4;
pub const LANDFORM_PLATEAU: u8 = 5;

/// Road SURFACE material, from the ground the road crosses (sand in deserts
/// and on beaches, rock in the mountains, dirt on soil, gravel otherwise).
pub const ROAD_MAT_GRAVEL: u8 = 0;
pub const ROAD_MAT_DIRT: u8 = 1;
pub const ROAD_MAT_SAND: u8 = 2;
pub const ROAD_MAT_ROCK: u8 = 3;

pub fn road_material_name(m: u8) -> &'static str {
    match m {
        ROAD_MAT_GRAVEL => "Gravel",
        ROAD_MAT_DIRT => "Dirt",
        ROAD_MAT_SAND => "Sand",
        ROAD_MAT_ROCK => "Rock",
        _ => "?",
    }
}

pub fn landform_name(l: u8) -> &'static str {
    match l {
        LANDFORM_WATER => "Water",
        LANDFORM_LOWLAND => "Lowland",
        LANDFORM_VALLEY => "Valley",
        LANDFORM_HILLS => "Hills",
        LANDFORM_MOUNTAINS => "Mountains",
        LANDFORM_PLATEAU => "Plateau",
        _ => "?",
    }
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

impl LevelData {
    /// Checks all cross-field invariants required by runtime indexing.
    pub fn validate(&self) -> Result<(), String> {
        let faces = self.unit_tris.len();
        for (name, len) in [
            ("terrain_tris", self.terrain_tris.len()),
            ("terrain_colors", self.terrain_colors.len()),
            ("face_types", self.face_types.len()),
            ("face_water_r", self.face_water_r.len()),
            ("face_river_r", self.face_river_r.len()),
            ("face_region", self.face_region.len()),
            ("slope_class", self.slope_class.len()),
            ("water_depth", self.water_depth.len()),
            ("landform", self.landform.len()),
            ("road_material", self.road_material.len()),
        ] {
            if len != faces {
                return Err(format!(
                    "{name} has {len} entries, expected one per face ({faces})"
                ));
            }
        }
        if self.face_tags.len() != faces {
            return Err(format!(
                "face_tags has {} entries, expected one per face ({faces})",
                self.face_tags.len()
            ));
        }
        for blend in &self.face_blend {
            if blend.face as usize >= faces {
                return Err(format!("blend references missing face {}", blend.face));
            }
        }
        for &reference in &self.face_region {
            if reference.is_some_and(|index| index as usize >= self.regions.len()) {
                return Err(format!("invalid region reference {reference:?}"));
            }
        }
        if self.settlements.is_empty() {
            return Err("level has no settlements".into());
        }
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::LevelData;

    fn tracked_level() -> LevelData {
        postcard::from_bytes(include_bytes!("../../main/assets/level_1337.bin"))
            .expect("tracked level must deserialize")
    }

    #[test]
    fn tracked_level_is_valid() {
        tracked_level().validate().unwrap();
    }

    #[test]
    fn validation_rejects_inconsistent_face_arrays() {
        let mut level = tracked_level();
        level.slope_class.pop();
        assert!(level.validate().unwrap_err().contains("slope_class"));
    }

    #[test]
    fn validation_rejects_inconsistent_face_tags() {
        let mut level = tracked_level();
        level.face_tags.pop();
        assert!(level.validate().unwrap_err().contains("face_tags"));
    }

    #[test]
    fn validation_rejects_invalid_region_references() {
        let mut level = tracked_level();
        level.face_region[0] = Some(level.regions.len() as u32);
        assert!(level.validate().unwrap_err().contains("region reference"));
    }
}
