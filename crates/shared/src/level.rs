use serde::{Deserialize, Serialize};
use std::{error::Error, fmt};

use crate::terrain::Terrain;
use terra_geometry::topology::FaceId;

/// Magic prefix for generated level artifacts.
pub const LEVEL_ARTIFACT_MAGIC: [u8; 4] = *b"TERA";
/// Wire format version accepted and written by this build.
pub const LEVEL_SCHEMA_VERSION: u16 = 1;
const LEVEL_ARTIFACT_HEADER_LEN: usize = LEVEL_ARTIFACT_MAGIC.len() + 2;

/// Failure to decode a generated level artifact or its version header.
#[derive(Debug)]
pub enum LevelArtifactError {
    MissingHeader,
    TruncatedHeader,
    UnsupportedSchemaVersion(u16),
    Deserialize(postcard::Error),
}

impl fmt::Display for LevelArtifactError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingHeader => write!(
                f,
                "level artifact has no TERA header; regenerate it with the current generator"
            ),
            Self::TruncatedHeader => write!(f, "level artifact has a truncated TERA header"),
            Self::UnsupportedSchemaVersion(version) => write!(
                f,
                "level artifact schema version {version} is unsupported; regenerate it with the current generator"
            ),
            Self::Deserialize(error) => write!(f, "invalid level artifact payload: {error}"),
        }
    }
}

impl Error for LevelArtifactError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Deserialize(error) => Some(error),
            Self::MissingHeader | Self::TruncatedHeader | Self::UnsupportedSchemaVersion(_) => None,
        }
    }
}

/// Compact membership lists for a dense set of locations.
///
/// Region IDs are indices into `LevelData::regions`. The offsets table stores
/// one half-open range per location plus a final sentinel offset.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct RegionMemberships {
    offsets: Vec<u32>,
    region_ids: Vec<u32>,
}

impl RegionMemberships {
    /// Packs per-location region IDs into deterministic, duplicate-free rows.
    pub fn from_memberships(mut memberships: Vec<Vec<u32>>) -> Self {
        for row in &mut memberships {
            row.sort_unstable();
            row.dedup();
        }
        let mut offsets = Vec::with_capacity(memberships.len() + 1);
        let mut region_ids = Vec::new();
        offsets.push(0);
        for mut row in memberships {
            region_ids.append(&mut row);
            offsets.push(u32::try_from(region_ids.len()).expect("region IDs exceed u32 offsets"));
        }
        Self {
            offsets,
            region_ids,
        }
    }

    pub(crate) fn from_pairs(location_count: usize, mut memberships: Vec<(usize, u32)>) -> Self {
        memberships.sort_unstable();
        memberships.dedup();
        let mut offsets = vec![0u32; location_count + 1];
        for &(location, _) in &memberships {
            assert!(
                location < location_count,
                "membership location out of bounds"
            );
            offsets[location + 1] += 1;
        }
        for location in 0..location_count {
            offsets[location + 1] += offsets[location];
        }
        let region_ids = memberships.into_iter().map(|(_, id)| id).collect();
        Self {
            offsets,
            region_ids,
        }
    }

    pub fn location_count(&self) -> usize {
        self.offsets.len().saturating_sub(1)
    }

    pub fn region_ids_at(&self, location: usize) -> &[u32] {
        let Some(&start) = self.offsets.get(location) else {
            return &[];
        };
        let Some(&end) = location
            .checked_add(1)
            .and_then(|next| self.offsets.get(next))
        else {
            return &[];
        };
        self.region_ids
            .get(start as usize..end as usize)
            .unwrap_or(&[])
    }

    fn validate(&self, location_count: usize, region_count: usize) -> Result<(), String> {
        if self.offsets.len() != location_count + 1 {
            return Err(format!(
                "face_regions has {} locations, expected one per face ({location_count})",
                self.location_count()
            ));
        }
        if self.offsets.first() != Some(&0) {
            return Err("face_regions offsets must start at zero".into());
        }
        if self.offsets.last().copied() != Some(self.region_ids.len() as u32) {
            return Err("face_regions final offset does not match region IDs".into());
        }
        if self.offsets.windows(2).any(|pair| pair[0] > pair[1]) {
            return Err("face_regions offsets are not monotonic".into());
        }
        for face in 0..location_count {
            let region_ids = self.region_ids_at(face);
            if region_ids
                .iter()
                .any(|&region| region as usize >= region_count)
            {
                return Err(format!("invalid region reference on face {face}"));
            }
            if region_ids.windows(2).any(|pair| pair[0] >= pair[1]) {
                return Err(format!(
                    "face {face} region references must be sorted and unique"
                ));
            }
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum FaceTag {
    Road,
    Settlement,
    Bridge,
    BridgeEntry,
}

impl FaceTag {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Road => "Road",
            Self::Settlement => "Settlement",
            Self::Bridge => "Bridge",
            Self::BridgeEntry => "Bridge Entry",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum BlendTarget {
    Terrain(Terrain),
    Road,
    Settlement,
    BridgeEntry,
}

impl BlendTarget {
    pub const fn name(self) -> Option<&'static str> {
        match self {
            Self::Terrain(_) => None,
            Self::Road => Some("Road"),
            Self::Settlement => Some("Settlement"),
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
    /// Settlement kind targets and footprint sizes used to generate this level.
    /// Runtime uses the same configuration when rebuilding terrain queries.
    pub settlement_config: SettlementConfig,
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
    /// Authoritative terrain identity at each face corner, in the same order
    /// as `unit_tris` and `terrain_colors`.
    pub face_corner_types: Vec<[Terrain; 3]>,
    /// Per-face water-surface radius (0.0 = dry), from the gen-time water
    /// clustering (`worldgen::water_surface_radii`). The runtime draws each face
    /// at this radius — no runtime clustering. Water-body IDENTITY/naming comes
    /// from the region layer (`face_regions`), not a parallel id.
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
    /// Named road and bridge connections. `from_endpoint` and `to_endpoint`
    /// index `road_endpoints`; connection order is deterministic for a seed.
    pub roads: Vec<RoadData>,
    /// Shared network locations. A location can carry several roles, such as
    /// a settlement entrance that also meets a bridge.
    pub road_endpoints: Vec<RoadEndpointData>,
    /// Named contiguous feature clusters: oceans, lakes, rivers, beaches,
    /// forests, mountain ranges, settlements, roads, …
    pub regions: Vec<RegionData>,
    /// Named region memberships for each derived face, projected from the
    /// authoritative cell memberships.
    pub face_regions: RegionMemberships,
    /// Sub-tile environmental scenery, including plant life and nonliving
    /// objects. Points sit on the displaced mesh and are placed deterministically.
    pub scenery: Vec<SceneryData>,
    /// Contextual built structures (ruins, docks, walls, …), placed at gen
    /// time and spawned as runtime entities like bridges.
    pub structures: Vec<StructureData>,
    /// Per-face slope class from the solved field.
    pub slope_class: Vec<SlopeClass>,
    /// Per-face water depth class; `None` on dry faces.
    pub water_depth: Vec<Option<WaterDepth>>,
    /// Local rendered-water phase. It may vary along one body.
    pub water_phase: Vec<Option<WaterPhase>>,
    /// General ground/surface condition for terrain and water alike.
    pub surface_condition: Vec<SurfaceCondition>,
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

/// Orthogonal physical state of water. Terrain retains geographic identity
/// while phase can change locally along the same lake or river.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum WaterPhase {
    Liquid,
    Frozen,
}

impl WaterPhase {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Liquid => "Liquid",
            Self::Frozen => "Frozen",
        }
    }
}

/// Climate/material condition of any world surface, independent of terrain
/// identity and whether the face carries water.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum SurfaceCondition {
    Normal,
    Frozen,
}

/// Plant-life category within `SceneryKind`.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum FloraKind {
    Tree,
    Bush,
    Flower,
    Grass,
    Cactus,
    Berry,
    Reed,
    Seaweed,
    Lilypad,
    Kelp,
    Cattail,
    Vine,
    Tumbleweed,
    Fern,
}

/// Environmental objects distinct from built structures; flora is one variant.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum SceneryKind {
    Flora(FloraKind),
    Rock,
    Log,
    Mushroom,
    DeadTree,
    Coral,
    Anemone,
    Starfish,
    Shell,
    Skull,
    Snowdrift,
    Stump,
    Icicle,
    Snowman,
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
    Tent,
    Crate,
    Fence,
    Barricade,
    LampPost,
    Signpost,
    Guardrail,
    Railing,
    Suspension,
    House,
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
            Self::Tent => "Tent",
            Self::Crate => "Crate",
            Self::Fence => "Fence",
            Self::Barricade => "Barricade",
            Self::LampPost => "Lamp Post",
            Self::Signpost => "Signpost",
            Self::Guardrail => "Guardrail",
            Self::Railing => "Railing",
            Self::Suspension => "Suspension Cable",
            Self::House => "House",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy)]
pub struct SceneryData {
    /// World position on the displaced terrain mesh.
    pub pos: [f32; 3],
    /// The face it sits on (for gameplay queries).
    pub face: u32,
    pub kind: SceneryKind,
    pub variant: u8,
}

impl LevelData {
    /// Encodes this level as a TERA versioned header followed by a Postcard payload.
    pub fn to_artifact_bytes(&self) -> Result<Vec<u8>, postcard::Error> {
        let payload = postcard::to_allocvec(self)?;
        let mut bytes = Vec::with_capacity(LEVEL_ARTIFACT_HEADER_LEN + payload.len());
        bytes.extend_from_slice(&LEVEL_ARTIFACT_MAGIC);
        bytes.extend_from_slice(&LEVEL_SCHEMA_VERSION.to_le_bytes());
        bytes.extend_from_slice(&payload);
        Ok(bytes)
    }

    /// Decodes a current-version artifact, rejecting headerless or unsupported data.
    pub fn from_artifact_bytes(bytes: &[u8]) -> Result<Self, LevelArtifactError> {
        if bytes.get(..LEVEL_ARTIFACT_MAGIC.len()) != Some(LEVEL_ARTIFACT_MAGIC.as_slice()) {
            return Err(LevelArtifactError::MissingHeader);
        }
        if bytes.len() < LEVEL_ARTIFACT_HEADER_LEN {
            return Err(LevelArtifactError::TruncatedHeader);
        }
        let version = u16::from_le_bytes([
            bytes[LEVEL_ARTIFACT_MAGIC.len()],
            bytes[LEVEL_ARTIFACT_MAGIC.len() + 1],
        ]);
        if version != LEVEL_SCHEMA_VERSION {
            return Err(LevelArtifactError::UnsupportedSchemaVersion(version));
        }
        postcard::from_bytes(&bytes[LEVEL_ARTIFACT_HEADER_LEN..])
            .map_err(LevelArtifactError::Deserialize)
    }

    /// Returns all region IDs assigned to the derived face at this index.
    /// Position queries first resolve a single face; adjacent faces are not
    /// combined at their shared boundary.
    pub fn region_ids_at_face(&self, face: FaceId) -> &[u32] {
        self.face_regions.region_ids_at(face.index())
    }

    /// Checks all cross-field invariants required by runtime indexing.
    pub fn validate(&self) -> Result<(), String> {
        let faces = self.unit_tris.len();
        for (name, len) in [
            ("terrain_tris", self.terrain_tris.len()),
            ("terrain_colors", self.terrain_colors.len()),
            ("face_types", self.face_types.len()),
            ("face_corner_types", self.face_corner_types.len()),
            ("face_water_r", self.face_water_r.len()),
            ("face_river_r", self.face_river_r.len()),
            ("face_regions", self.face_regions.location_count()),
            ("slope_class", self.slope_class.len()),
            ("water_depth", self.water_depth.len()),
            ("water_phase", self.water_phase.len()),
            ("surface_condition", self.surface_condition.len()),
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
        self.face_regions.validate(faces, self.regions.len())?;
        if self.settlements.is_empty() {
            return Err("level has no settlements".into());
        }
        self.settlement_config.validate()?;
        if self.settlement_config.total() != self.settlements.len() {
            return Err(format!(
                "settlement_config targets {} settlements, level has {}",
                self.settlement_config.total(),
                self.settlements.len()
            ));
        }
        for (kind, expected) in [
            (SettlementKind::Town, self.settlement_config.towns),
            (SettlementKind::Village, self.settlement_config.villages),
            (SettlementKind::Outpost, self.settlement_config.outposts),
        ] {
            let actual = self
                .settlements
                .iter()
                .filter(|settlement| settlement.kind == kind)
                .count();
            if actual != expected {
                return Err(format!(
                    "level has {actual} {} settlements, expected {expected}",
                    kind.name()
                ));
            }
        }
        for (road_index, road) in self.roads.iter().enumerate() {
            if road.from_endpoint as usize >= self.road_endpoints.len()
                || road.to_endpoint as usize >= self.road_endpoints.len()
            {
                return Err(format!(
                    "road {road_index} references a missing road endpoint"
                ));
            }
            if road.from_endpoint == road.to_endpoint {
                return Err(format!(
                    "road {road_index} has the same endpoint at both ends"
                ));
            }
            if road.points.len() < 2 {
                return Err(format!("road {road_index} has fewer than two points"));
            }
        }
        for (endpoint_index, endpoint) in self.road_endpoints.iter().enumerate() {
            for role in &endpoint.roles {
                if let RoadEndpointRole::SettlementEntrance { settlement_index } = role
                    && *settlement_index as usize >= self.settlements.len()
                {
                    return Err(format!(
                        "road endpoint {endpoint_index} references missing settlement {settlement_index}"
                    ));
                }
            }
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
    River,
    Beach,
    Cliff,
    Forest,
    Desert,
    MountainRange,
    Plains,
    Tundra,
    Swamp,
    Jungle,
    Savanna,
    Volcano,
    Glacier,
    Settlement,
    Road,
}

impl RegionKind {
    pub const fn rank(self) -> usize {
        match self {
            Self::Ocean => 0,
            Self::Lake => 1,
            Self::SaltLake => 2,
            Self::River => 3,
            Self::Beach => 4,
            Self::Cliff => 5,
            Self::Forest => 6,
            Self::Desert => 7,
            Self::MountainRange => 8,
            Self::Plains => 9,
            Self::Tundra => 10,
            Self::Swamp => 11,
            Self::Jungle => 12,
            Self::Savanna => 13,
            Self::Volcano => 14,
            Self::Glacier => 15,
            Self::Settlement => 16,
            Self::Road => 17,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum SettlementKind {
    Town,
    Village,
    Outpost,
}

impl SettlementKind {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Town => "Town",
            Self::Village => "Village",
            Self::Outpost => "Outpost",
        }
    }
}

/// Configurable size and world-wide target counts for generated settlements.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
pub struct SettlementConfig {
    pub towns: usize,
    pub villages: usize,
    pub outposts: usize,
    pub town_radius_m: f32,
    pub village_radius_m: f32,
    pub outpost_radius_m: f32,
}

impl SettlementConfig {
    pub const fn total(self) -> usize {
        self.towns
            .saturating_add(self.villages)
            .saturating_add(self.outposts)
    }

    pub fn kind_at(self, index: usize) -> Option<SettlementKind> {
        if index < self.towns {
            Some(SettlementKind::Town)
        } else if index < self.towns.saturating_add(self.villages) {
            Some(SettlementKind::Village)
        } else if index < self.total() {
            Some(SettlementKind::Outpost)
        } else {
            None
        }
    }

    pub const fn radius_m(self, kind: SettlementKind) -> f32 {
        match kind {
            SettlementKind::Town => self.town_radius_m,
            SettlementKind::Village => self.village_radius_m,
            SettlementKind::Outpost => self.outpost_radius_m,
        }
    }

    pub fn validate(self) -> Result<(), String> {
        if self.towns == 0 || self.villages == 0 || self.outposts == 0 {
            return Err("settlement config must include towns, villages, and outposts".into());
        }
        if self
            .towns
            .checked_add(self.villages)
            .and_then(|count| count.checked_add(self.outposts))
            .is_none()
        {
            return Err("settlement target counts overflow usize".into());
        }
        if [
            self.town_radius_m,
            self.village_radius_m,
            self.outpost_radius_m,
        ]
        .into_iter()
        .any(|radius| !radius.is_finite() || radius <= 0.0)
        {
            return Err("settlement radii must be positive and finite".into());
        }
        Ok(())
    }
}

impl Default for SettlementConfig {
    fn default() -> Self {
        Self {
            towns: 3,
            villages: 6,
            outposts: 3,
            town_radius_m: 55.0,
            village_radius_m: 35.0,
            outpost_radius_m: 20.0,
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct SettlementData {
    pub name: String,
    pub pos: [f32; 3],
    pub kind: SettlementKind,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct RoadData {
    pub name: String,
    pub points: Vec<[f32; 3]>,
    pub kind: RoadKind,
    pub from_endpoint: u32,
    pub to_endpoint: u32,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct RoadEndpointData {
    /// Unit-sphere position shared by every connection meeting here.
    pub pos: [f32; 3],
    pub roles: Vec<RoadEndpointRole>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub enum RoadEndpointRole {
    SettlementEntrance { settlement_index: u32 },
    Junction,
    BridgeEntrance,
    RoadEnd,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum RoadKind {
    Road,
    Bridge,
}

#[cfg(test)]
mod tests {
    use terra_geometry::topology::FaceId;

    use super::{
        FloraKind, Landform, LevelArtifactError, LevelData, RegionMemberships, RoadEndpointRole,
        RoadKind, RoadMaterial, SlopeClass, StructureKind, SurfaceCondition, WaterDepth,
        WaterPhase,
    };
    use serde::{Serialize, de::DeserializeOwned};

    fn tracked_level() -> LevelData {
        LevelData::from_artifact_bytes(include_bytes!("../../main/assets/level_1337.bin"))
            .expect("generated level asset must deserialize")
    }

    #[test]
    fn level_artifact_rejects_unversioned_postcard_payload() {
        let old_payload = postcard::to_allocvec(&1337_u32).unwrap();

        assert!(matches!(
            LevelData::from_artifact_bytes(&old_payload),
            Err(LevelArtifactError::MissingHeader)
        ));
    }

    #[test]
    fn level_artifact_rejects_wrong_magic() {
        assert!(matches!(
            LevelData::from_artifact_bytes(b"NOPE\x01\x00"),
            Err(LevelArtifactError::MissingHeader)
        ));
    }

    #[test]
    fn level_artifact_rejects_unsupported_schema_version() {
        let mut bytes = b"TERA".to_vec();
        bytes.extend_from_slice(&u16::MAX.to_le_bytes());

        assert!(matches!(
            LevelData::from_artifact_bytes(&bytes),
            Err(LevelArtifactError::UnsupportedSchemaVersion(u16::MAX))
        ));
    }

    #[test]
    fn level_artifact_rejects_truncated_header() {
        assert!(matches!(
            LevelData::from_artifact_bytes(b"TERA\x01"),
            Err(LevelArtifactError::TruncatedHeader)
        ));
    }

    #[test]
    fn level_artifact_rejects_invalid_payload() {
        assert!(matches!(
            LevelData::from_artifact_bytes(b"TERA\x01\x00\xff"),
            Err(LevelArtifactError::Deserialize(_))
        ));
    }

    #[test]
    fn level_artifact_round_trip_preserves_bytes() {
        let level = tracked_level();
        let encoded = level.to_artifact_bytes().unwrap();
        let decoded = LevelData::from_artifact_bytes(&encoded).unwrap();

        assert_eq!(&encoded[..4], b"TERA");
        assert_eq!(u16::from_le_bytes([encoded[4], encoded[5]]), 1);
        assert_eq!(decoded.to_artifact_bytes().unwrap(), encoded);
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
        let mut memberships = (0..level.face_regions.location_count())
            .map(|face| level.region_ids_at_face(FaceId::new(face)).to_vec())
            .collect::<Vec<_>>();
        memberships[0] = vec![level.regions.len() as u32];
        level.face_regions = RegionMemberships::from_memberships(memberships);
        assert!(level.validate().unwrap_err().contains("region reference"));
    }

    #[test]
    fn validation_rejects_invalid_road_endpoint_references() {
        let mut level = tracked_level();
        level.roads[0].from_endpoint = level.road_endpoints.len() as u32;
        assert!(level.validate().unwrap_err().contains("road endpoint"));

        let mut level = tracked_level();
        level.roads[0].to_endpoint = level.road_endpoints.len() as u32;
        assert!(level.validate().unwrap_err().contains("road endpoint"));
    }

    #[test]
    fn validation_rejects_invalid_settlement_entrance_references() {
        let mut level = tracked_level();
        let endpoint = level
            .road_endpoints
            .iter_mut()
            .find(|endpoint| {
                endpoint
                    .roles
                    .iter()
                    .any(|role| matches!(role, RoadEndpointRole::SettlementEntrance { .. }))
            })
            .expect("generated road network has settlement entrances");
        endpoint.roles.push(RoadEndpointRole::SettlementEntrance {
            settlement_index: level.settlements.len() as u32,
        });
        assert!(level.validate().unwrap_err().contains("missing settlement"));
    }

    #[test]
    fn face_region_memberships_keep_all_sorted_region_ids_for_a_face() {
        let memberships = RegionMemberships::from_memberships(vec![vec![2, 1, 2], vec![], vec![0]]);

        assert_eq!(memberships.location_count(), 3);
        assert_eq!(memberships.region_ids_at(0), &[1, 2]);
        assert!(memberships.region_ids_at(1).is_empty());
        assert_eq!(memberships.region_ids_at(2), &[0]);
        assert!(memberships.region_ids_at(3).is_empty());
    }

    #[test]
    fn level_validation_requires_one_membership_list_per_face() {
        let mut level = tracked_level();
        level.face_regions = RegionMemberships::from_memberships(vec![]);

        assert!(level.validate().unwrap_err().contains("face_regions"));
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
        round_trip(WaterPhase::Frozen);
        round_trip(SurfaceCondition::Frozen);
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
