use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct LevelData {
    pub terrain_tris: Vec<[[f32; 3]; 3]>,
    pub terrain_colors: Vec<[f32; 4]>,
    pub terrain_features: Vec<u8>,
    pub unit_tris: Vec<[[f32; 3]; 3]>,
    pub feature_tris: Vec<[[f32; 3]; 3]>,
    pub feature_colors: Vec<[f32; 4]>,
    pub settlements: Vec<SettlementData>,
    pub roads: Vec<RoadData>,
    /// Baked TerrainGen vertex data: avoids reconstructing noise at startup.
    pub baked_verts: BakedTerrain,
}

/// Precomputed vertex-level terrain data — eliminates runtime noise generation.
#[derive(Serialize, Deserialize)]
pub struct BakedTerrain {
    /// Icosphere vertices on unit sphere (sub=5, ~10k).
    pub verts: Vec<[f32; 3]>,
    /// Columnar adjacency: adj_off[i]..adj_off[i+1] slices into adj_data.
    pub vert_adj_off: Vec<usize>,
    pub vert_adj_data: Vec<usize>,
    /// Spatial grid for nearest-vert lookup: 32×64 lat/lon buckets of vertex indices.
    pub vert_grid: Vec<Vec<usize>>,
    /// Per-vertex precomputed values.
    pub vert_elev: Vec<f32>,
    pub vert_moist: Vec<f32>,
    pub vert_temp: Vec<f32>,
    /// Flow + erosion data.
    pub flow_accum: Vec<f32>,
    pub river_depth: Vec<f32>,
    /// Road proximity spatial hash: (lat_cell, lon_cell) → list of road/settlement points.
    pub road_cells: Vec<(i32, i32, Vec<[f32; 3]>)>,
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
