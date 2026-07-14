use super::{CellSet, Grid};
use crate::terrain::Terrain;

pub(super) fn painted_corners(grid: &Grid, cells: &CellSet, face: usize) -> usize {
    grid.face_cells_index(face)
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

pub(super) fn face_solid(grid: &Grid, cells: &CellSet, face: usize) -> bool {
    painted_corners(grid, cells, face) == 3
}

/// Widens a lattice chain into one or two parallel feature bands.
pub(super) fn widen_band(grid: &Grid, chain: &[usize], both_sides: bool) -> Vec<usize> {
    let mut output = chain.to_vec();
    for segment in chain.windows(2) {
        let (start, end) = (segment[0], segment[1]);
        let left = grid
            .cell_direction_index(start)
            .cross(grid.cell_direction_index(end));
        let cell = grid
            .topology
            .cell(start)
            .expect("cell index from topology range");
        for &face in grid.topology.cell_faces(cell) {
            let corners = grid.face_cells_index(face.index());
            if !corners.contains(&end) {
                continue;
            }
            let partner = corners
                .iter()
                .find(|&&candidate| candidate != start && candidate != end)
                .unwrap();
            let side_matches = both_sides || grid.cell_direction_index(*partner).dot(left) > 0.0;
            if side_matches && !output.contains(partner) {
                output.push(*partner);
            }
        }
    }
    output
}
