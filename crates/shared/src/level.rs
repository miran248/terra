use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct LevelData {
    /// Pure terrain trimesh (visual + physics). No feature raises.
    pub terrain_tris: Vec<[[f32; 3]; 3]>,
    /// Per-terrain-triangle color (linear RGB, includes road/town paint).
    pub terrain_colors: Vec<[f32; 4]>,
    /// Per-terrain-triangle feature flags: bit 0=road, bit 1=town, bit 2=bridge.
    pub terrain_features: Vec<u8>,
    /// Undisplaced unit icosphere tris for face-at-position lookup.
    pub unit_tris: Vec<[[f32; 3]; 3]>,
    /// Features trimesh: bridge decks, settlement flat discs. Separate collider layer.
    pub feature_tris: Vec<[[f32; 3]; 3]>,
    /// Per-feature-triangle color.
    pub feature_colors: Vec<[f32; 4]>,
    /// Settlement names and unit-vector positions.
    pub settlements: Vec<SettlementData>,
    /// Road/bridge paths: sampled unit-vector points + kind.
    pub roads: Vec<RoadData>,
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
