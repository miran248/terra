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
    /// Per-face slope class from the solved field.
    pub slope_class: Vec<SlopeClass>,
    /// Per-face water depth class; `None` on dry faces.
    pub water_depth: Vec<Option<WaterDepth>>,
    /// Per-face macro landform.
    pub landform: Vec<Landform>,
    /// Per-face road surface material; `None` on non-road faces.
    pub road_material: Vec<Option<RoadMaterial>>,
}

#[derive(Serialize, Deserialize, Clone, Copy)]
pub struct StructureData {
    pub pos: [f32; 3],
    pub face: u32,
    pub kind: StructureKind,
    /// Facing yaw about the local up (radians).
    pub yaw: f32,
}

/// Per-cell/-face terrain steepness (slope class) from the solved field.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum SlopeClass {
    Flat,
    Gentle,
    Steep,
    Cliff,
}

impl SlopeClass {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Flat => "Flat",
            Self::Gentle => "Gentle",
            Self::Steep => "Steep",
            Self::Cliff => "Cliff",
        }
    }
    pub const fn is_walkable(self) -> bool {
        matches!(self, Self::Flat | Self::Gentle)
    }
    pub const fn severity(self) -> u8 {
        match self {
            Self::Flat => 0,
            Self::Gentle => 1,
            Self::Steep => 2,
            Self::Cliff => 3,
        }
    }
}

/// Macro LANDFORM (the terrain massif a cell belongs to), the base layer under
/// biome cover and slope-class detail: lowlands, hills, mountains, plateaus,
/// valleys — or water. Drives which biome COVER a cell gets and reads in the
/// HUD as the landform word.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Landform {
    Water,
    Lowland,
    Valley,
    Hills,
    Mountains,
    Plateau,
}

impl Landform {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Water => "Water",
            Self::Lowland => "Lowland",
            Self::Valley => "Valley",
            Self::Hills => "Hills",
            Self::Mountains => "Mountains",
            Self::Plateau => "Plateau",
        }
    }
    pub const fn is_highland(self) -> bool {
        matches!(self, Self::Hills | Self::Mountains | Self::Plateau)
    }
    pub const fn rank(self) -> u8 {
        match self {
            Self::Water => 0,
            Self::Lowland => 1,
            Self::Valley => 2,
            Self::Hills => 3,
            Self::Mountains => 4,
            Self::Plateau => 5,
        }
    }
}

/// Road SURFACE material, from the ground the road crosses (sand in deserts
/// and on beaches, rock in the mountains, dirt on soil, gravel otherwise).
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum RoadMaterial {
    Gravel,
    Dirt,
    Sand,
    Rock,
}
impl RoadMaterial {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Gravel => "Gravel",
            Self::Dirt => "Dirt",
            Self::Sand => "Sand",
            Self::Rock => "Rock",
        }
    }
}

/// Per-cell/-face WATER DEPTH class (from the solved field). The depth analogue
/// of the slope class: identity (ocean/lake/river) is one thing, depth another.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum WaterDepth {
    Shallow,
    Deep,
    Abyss,
}
impl WaterDepth {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Shallow => "Shallow",
            Self::Deep => "Deep",
            Self::Abyss => "Abyss",
        }
    }
    pub const fn severity(self) -> u8 {
        match self {
            Self::Shallow => 0,
            Self::Deep => 1,
            Self::Abyss => 2,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum FloraKind {
    Tree,
    Bush,
    Flower,
    Rock,
    Grass,
    Log,
    Mushroom,
    Cactus,
    Berry,
    DeadTree,
    Reed,
}

// Structures: built props placed contextually (like towns and bridges).
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum StructureKind {
    Ruin,
    Watchtower,
    Dock,
    Farm,
    Wall,
    Well,
    Campfire,
}
impl StructureKind {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Ruin => "Ruins",
            Self::Watchtower => "Watchtower",
            Self::Dock => "Dock",
            Self::Farm => "Farm",
            Self::Wall => "Wall",
            Self::Well => "Well",
            Self::Campfire => "Campfire",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy)]
pub struct FloraData {
    /// World position on the displaced terrain mesh.
    pub pos: [f32; 3],
    /// The face it sits on (for gameplay queries).
    pub face: u32,
    pub kind: FloraKind,
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
    SaltLake,
    FrozenLake,
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

impl RegionKind {
    pub const fn rank(self) -> usize {
        match self {
            Self::Ocean => 0,
            Self::Lake => 1,
            Self::SaltLake => 2,
            Self::FrozenLake => 3,
            Self::River => 4,
            Self::Beach => 5,
            Self::Cliff => 6,
            Self::Forest => 7,
            Self::Desert => 8,
            Self::Mountain => 9,
            Self::Plains => 10,
            Self::Tundra => 11,
            Self::Swamp => 12,
            Self::Jungle => 13,
            Self::Savanna => 14,
            Self::Volcano => 15,
            Self::Glacier => 16,
            Self::Town => 17,
            Self::Road => 18,
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct SettlementData {
    pub name: String,
    pub pos: [f32; 3],
}

#[derive(Serialize, Deserialize)]
pub struct RoadData {
    pub points: Vec<[f32; 3]>,
    pub kind: RoadKind,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum RoadKind {
    Road,
    Bridge,
}

#[cfg(test)]
mod tests {
    use super::{
        FloraKind, Landform, LevelData, RoadKind, RoadMaterial, SlopeClass, StructureKind,
        WaterDepth,
    };
    use serde::{Serialize, de::DeserializeOwned};

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

    fn round_trip<T>(value: T)
    where
        T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug,
    {
        let bytes = postcard::to_allocvec(&value).unwrap();
        assert_eq!(postcard::from_bytes::<T>(&bytes).unwrap(), value);
        assert!(postcard::from_bytes::<T>(&[u8::MAX]).is_err());
    }

    #[test]
    fn typed_schema_enums_round_trip_and_reject_invalid_discriminants() {
        round_trip(SlopeClass::Cliff);
        round_trip(WaterDepth::Abyss);
        round_trip(Landform::Plateau);
        round_trip(RoadMaterial::Rock);
        round_trip(FloraKind::Reed);
        round_trip(StructureKind::Campfire);
        round_trip(RoadKind::Bridge);
    }

    #[test]
    fn tracked_level_uses_none_for_dry_and_non_road_faces() {
        let level = tracked_level();
        for face in 0..level.face_types.len() {
            assert_eq!(
                level.water_depth[face].is_some(),
                level.face_types[face].is_water()
            );
            assert_eq!(
                level.road_material[face].is_some(),
                level.face_tags[face].contains(&super::FaceTag::Road)
            );
        }
    }
}
