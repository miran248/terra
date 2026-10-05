mod mesh;
mod placement;
mod water;

pub(super) use mesh::{build_mesh, face_road_material};
pub(super) use placement::{
    build_face_tags, place_scenery, place_structures, structure_footprint_radius,
};
#[cfg(test)]
pub(super) use water::{RIVER_TERRAIN_CLIP, cluster_cell_types, cluster_face_types};
pub(super) use water::{river_surface_radii, water_surface_radii};
