use super::{CellSet, Grid};
use crate::level::SlopeClass;
use crate::terrain::Terrain;
use crate::topology::{CellId, FaceId};

mod bridges;
mod transitions;

pub(super) use bridges::{build_bridges, paint_features};
pub(super) use transitions::{absorb_small_clusters, mark_blends, resolve_transitions};

pub(super) fn painted_corners(grid: &Grid, cells: &CellSet, face: FaceId) -> usize {
    grid.face_cells(face)
        .iter()
        .filter(|&&cell| cells.contains(cell))
        .count()
}

pub(super) fn bridge_walkable(terrain: Terrain) -> bool {
    matches!(
        terrain,
        Terrain::Plains
            | Terrain::Forest
            | Terrain::Savanna
            | Terrain::Tundra
            | Terrain::Desert
            | Terrain::Jungle
            | Terrain::Swamp
    )
}

pub(super) fn roadable(terrain: Terrain, slope: SlopeClass) -> bool {
    terrain != Terrain::Cliff
        && !terrain.is_water()
        && matches!(slope, SlopeClass::Flat | SlopeClass::Gentle)
}

pub(super) fn face_solid(grid: &Grid, cells: &CellSet, face: FaceId) -> bool {
    painted_corners(grid, cells, face) == 3
}

/// Widens a lattice chain into one or two parallel feature bands.
pub(super) fn widen_band(grid: &Grid, chain: &[CellId], both_sides: bool) -> Vec<CellId> {
    let mut output = chain.to_vec();
    for segment in chain.windows(2) {
        let (start, end) = (segment[0], segment[1]);
        let left = grid.cell_direction(start).cross(grid.cell_direction(end));
        for &face in grid.topology.cell_faces(start) {
            let corners = grid.face_cells(face);
            if !corners.contains(&end) {
                continue;
            }
            let partner = corners
                .iter()
                .find(|&&candidate| candidate != start && candidate != end)
                .unwrap();
            let side_matches = both_sides || grid.cell_direction(*partner).dot(left) > 0.0;
            if side_matches && !output.contains(partner) {
                output.push(*partner);
            }
        }
    }
    output
}
