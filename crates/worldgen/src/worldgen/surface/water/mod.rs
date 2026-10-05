mod bodies;
mod components;
mod rivers;

pub(in crate::worldgen) use bodies::water_surface_radii;
#[cfg(test)]
pub(in crate::worldgen) use components::{cluster_cell_types, cluster_face_types};
#[cfg(test)]
pub(in crate::worldgen) use rivers::RIVER_TERRAIN_CLIP;
pub(in crate::worldgen) use rivers::river_surface_radii;
