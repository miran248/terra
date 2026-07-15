use crate::terrain::{Terrain, TerrainGen};
use crate::topology::{CellId, FaceId};
use crate::worldgen::Grid;

use super::components::cluster_cell_types;

/// Per-face water-surface radius (`0.0` = dry). Lake components use their shore
/// rim as a waterline; ocean components share the sea radius.
pub(in crate::worldgen) fn water_surface_radii(
    grid: &Grid,
    terrain: &TerrainGen,
    cells: &[Terrain],
) -> Vec<f32> {
    let sea_r = crate::sphere::PLANET_RADIUS - 2.0;

    let vert_r: Vec<f32> = grid
        .topology
        .cells()
        .map(|cell| terrain.render_radius(grid.cell_position(cell)))
        .collect();
    let lake_components = cluster_cell_types(
        grid,
        cells,
        &[Terrain::Lake, Terrain::SaltLake, Terrain::FrozenLake],
    );
    let ocean_components = cluster_cell_types(
        grid,
        cells,
        &[Terrain::Ocean, Terrain::Beach, Terrain::Cliff],
    );

    // Per lake body: waterline from the LakeShore RIM adjacent to its Lake tiles
    // (low percentile ≈ spill point). Sea bodies: only real ones (hold Ocean).
    let mut rim: Vec<Vec<f32>> = vec![Vec::new(); lake_components.count()];
    let mut peak = vec![f32::MIN; lake_components.count()];
    let mut is_sea = vec![false; ocean_components.count()];
    for v in 0..grid.cell_count() {
        let cell = grid
            .topology
            .cell(v)
            .expect("cell index from topology range");
        if let Some(component) = lake_components.cell(cell) {
            let c = component.index();
            peak[c] = peak[c].max(vert_r[v]);
            for nb in grid
                .cell_neighbors(CellId::new(v))
                .iter()
                .map(|cell| cell.index())
            {
                if cells[nb] == Terrain::LakeShore {
                    rim[c].push(vert_r[nb]);
                }
            }
        }
        if let Some(component) = ocean_components.cell(cell)
            && cells[v] == Terrain::Ocean
        {
            is_sea[component.index()] = true;
        }
    }
    let lake_r: Vec<f32> = (0..lake_components.count())
        .map(|c| {
            if rim[c].is_empty() {
                peak[c]
            } else {
                rim[c].sort_by(f32::total_cmp);
                rim[c][rim[c].len() / 10]
            }
        })
        .collect();

    (0..grid.face_count())
        .map(|face_index| {
            let idx = grid.face_cells(FaceId::new(face_index)).map(CellId::index);
            // Sea: any corner in a real sea body (coast overdraw hidden by depth).
            let oc = idx.map(|cell| ocean_components.cell(grid.topology.cell(cell).unwrap()));
            if oc
                .iter()
                .flatten()
                .any(|component| is_sea[component.index()])
            {
                return sea_r;
            }
            // Lake: any corner on a lake body's Lake tiles, and no solid-land
            // corner (so it fills to the shore but never spills onto land).
            if idx.iter().any(|&cell| cells[cell].is_land_biome()) {
                return 0.0;
            }
            match idx.iter().find_map(|&cell_index| {
                lake_components.cell(grid.topology.cell(cell_index).unwrap())
            }) {
                Some(component) => lake_r[component.index()],
                None => 0.0,
            }
        })
        .collect()
}
