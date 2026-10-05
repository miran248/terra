use terra_geometry::sphere::SpherePos;
use crate::terrain::{Terrain, TerrainGen};
use terra_geometry::topology::{CellId, FaceId};

use super::super::{Grid, classification};

/// Majority coarse zone over a cell's face fan (deterministic tie-break).
pub(in crate::worldgen) fn cell_zone(
    grid: &Grid,
    terrain: &TerrainGen,
    cell_index: usize,
) -> crate::zones::ZoneKind {
    use crate::zones::ZoneKind;
    let kinds = [
        ZoneKind::Ocean,
        ZoneKind::Continent,
        ZoneKind::Island,
        ZoneKind::Lake,
        ZoneKind::MountainRange,
        ZoneKind::Settlement,
    ];
    let mut counts = [0usize; 6];
    let cell = grid
        .topology
        .cell(cell_index)
        .expect("cell index from topology range");
    for &face in grid.topology.cell_faces(cell) {
        let kind = terrain.zones().kind_at_fine(face.index());
        counts[kinds
            .iter()
            .position(|candidate| *candidate == kind)
            .unwrap()] += 1;
    }
    kinds[counts
        .iter()
        .enumerate()
        .max_by_key(|(_, count)| **count)
        .unwrap()
        .0]
}

fn most_common_land(
    grid: &Grid,
    cells: &[Terrain],
    members: impl IntoIterator<Item = CellId>,
) -> Terrain {
    let mut counts = [0usize; Terrain::ALL.len()];
    for cell in members {
        for &neighbor in grid.cell_neighbors(cell) {
            let terrain = cells[neighbor.index()];
            if terrain.is_land() {
                counts[classification::terrain_rank(terrain) as usize] += 1;
            }
        }
    }
    counts
        .iter()
        .enumerate()
        .max_by_key(|(_, count)| **count)
        .filter(|(_, count)| **count > 0)
        .map_or(Terrain::Plains, |(rank, _)| Terrain::ALL[rank])
}

/// Water-body identity comes from connectivity, not per-face depth: any face can
/// dip below sea level (blending, river carving), but a connected water body is
/// an Ocean only if it reaches ocean-zone faces — otherwise it's an enclosed
/// Lake, whatever its faces individually classified as. Rivers (painted channel
/// faces) are their own linear feature and never merge into either.
pub(in crate::worldgen) fn normalize_water_bodies(
    grid: &Grid,
    terrain: &TerrainGen,
    cells: &mut [Terrain],
) {
    // Dam every lake rim first: a land-zone cell that classified as water and
    // touches lake-zone water is the start of a drain channel to the sea (they
    // sneak along coarse-face edges past the vertex clamps). Turn it into
    // LakeShore — a one-cell dam that encloses the lake by construction; the
    // elevation solver then lifts it above sea level (LakeShore range).
    let lake_zone: Vec<bool> = (0..grid.cell_count())
        .map(|cell_index| cell_zone(grid, terrain, cell_index) == crate::zones::ZoneKind::Lake)
        .collect();
    let dams: Vec<CellId> = grid
        .topology
        .cells()
        .filter(|&cell| {
            cells[cell.index()].is_water()
                && !lake_zone[cell.index()]
                && grid.cell_neighbors(cell).iter().any(|nb| {
                    let nb = nb.index();
                    cells[nb].is_water() && lake_zone[nb]
                })
        })
        .collect();
    for cell in dams {
        cells[cell.index()] = Terrain::LakeShore;
    }

    enforce_water_shape(grid, cells);

    let mut visited = vec![false; grid.cell_count()];
    for start in grid.topology.cells() {
        if !cells[start.index()].is_water()
            || matches!(cells[start.index()], Terrain::River | Terrain::RiverSpring)
            || visited[start.index()]
        {
            continue;
        }
        let body: Vec<CellId> = grid
            .topology
            .cell_component(start, |cell| {
                cells[cell.index()].is_water()
                    && !matches!(cells[cell.index()], Terrain::River | Terrain::RiverSpring)
            })
            .into_iter()
            .collect();
        for &cell in &body {
            visited[cell.index()] = true
        }
        // A body is an OCEAN only if it reaches ocean-zone cells AND is large
        // enough to be one. Only authored lake-zone water may become a lake:
        // isolated ocean-zone pockets inherit coarse triangular boundaries and
        // are filled instead of creating rows of artificial triangular lakes.
        let is_ocean = body.len() >= classification::size_range(Terrain::Ocean).0
            && body.iter().any(|&cell| {
                cell_zone(grid, terrain, cell.index()) == crate::zones::ZoneKind::Ocean
            });
        let is_authored_lake = body.iter().any(|&cell| lake_zone[cell.index()]);
        if !is_ocean
            && (!is_authored_lake || body.len() < classification::size_range(Terrain::Lake).0)
        {
            // Unauthored pockets and undersized authored puddles become the
            // most common surrounding land kind.
            let fill = most_common_land(grid, cells, body.iter().copied());
            for &cell in &body {
                cells[cell.index()] = fill;
            }
            continue;
        }
        for &cell in &body {
            cells[cell.index()] = if !is_ocean {
                Terrain::Lake
            } else if cells[cell.index()] == Terrain::Lake {
                // A lake cell swallowed by the ocean body is just shallow ocean.
                Terrain::Ocean
            } else {
                cells[cell.index()]
            };
        }
    }

    // Lakes keep their distance from the sea: any lake cell within ~7 cell
    // steps (~250m ≈ the 10-tile rule) of ocean water becomes land, and a
    // lake trimmed under the minimum size drains entirely.
    let ocean_sources: Vec<_> = grid
        .topology
        .cells()
        .filter(|cell| cells[cell.index()] == Terrain::Ocean)
        .collect();
    let ocean_dist = grid.topology.cell_distances(&ocean_sources, 7);
    let fill_kind = |cells: &[Terrain], cell_index: usize| -> Terrain {
        most_common_land(grid, cells, [CellId::new(cell_index)])
    };
    for cell_index in 0..grid.cell_count() {
        let cell = grid
            .topology
            .cell(cell_index)
            .expect("cell index from topology range");
        if cells[cell_index] == Terrain::Lake && ocean_dist.cell_steps(cell).is_some() {
            cells[cell_index] = fill_kind(cells, cell_index);
        }
    }
    let mut visited = vec![false; grid.cell_count()];
    for start in grid.topology.cells() {
        if cells[start.index()] != Terrain::Lake || visited[start.index()] {
            continue;
        }
        let body: Vec<CellId> = grid
            .topology
            .cell_component(start, |cell| cells[cell.index()] == Terrain::Lake)
            .into_iter()
            .collect();
        for &cell in &body {
            visited[cell.index()] = true
        }
        if body.len() < classification::size_range(Terrain::Lake).0 {
            for &cell in &body {
                cells[cell.index()] = fill_kind(cells, cell.index());
            }
        }
    }

    // Give each surviving authored lake one body-wide salinity identity.
    // Freezing is an orthogonal, local water property classified later.
    let ocean_sources = grid
        .topology
        .cells()
        .filter(|cell| cells[cell.index()] == Terrain::Ocean)
        .collect::<Vec<_>>();
    let near_ocean = grid.topology.cell_distances(&ocean_sources, 15);
    let mut typed = vec![false; grid.cell_count()];
    for start in grid.topology.cells() {
        if cells[start.index()] != Terrain::Lake || typed[start.index()] {
            continue;
        }
        let body = grid
            .topology
            .cell_component(start, |cell| cells[cell.index()] == Terrain::Lake);
        let kind = if body
            .iter()
            .any(|&cell| near_ocean.cell_steps(cell).is_some())
        {
            Terrain::SaltLake
        } else {
            Terrain::Lake
        };
        for cell in body {
            typed[cell.index()] = true;
            cells[cell.index()] = kind;
        }
    }
}

/// Water narrower than a few cells reads as a visual glitch: water within 2
/// steps of land must reach "core" water (≥3 steps from land) within 2 steps,
/// or it is a sliver/neck → land. Rivers are linear by nature and exempt.
/// (Vertex pinches no longer exist: cells are hexagonal, two same-type cells
/// can only meet along an edge.) Iterated because fills can expose new slivers.
pub(super) fn enforce_water_shape(grid: &Grid, cells: &mut [Terrain]) {
    let fill_kind = |cells: &[Terrain], cell_index: usize| -> Terrain {
        most_common_land(grid, cells, [CellId::new(cell_index)])
    };
    let wet = |t: Terrain| t.is_water() && !matches!(t, Terrain::River | Terrain::RiverSpring);

    for _ in 0..4 {
        let mut changed = false;
        let land_sources: Vec<_> = grid
            .topology
            .cells()
            .filter(|cell| cells[cell.index()].is_land())
            .collect();
        // Cell steps are ~17.5m: with resolution doubled to pack in content,
        // core water sits ≥3 steps from land (half the old physical width).
        let land_dist = grid.topology.cell_distances(&land_sources, 2);
        // Core water reaches outward 2 steps.
        let core_sources: Vec<_> = grid
            .topology
            .cells()
            .filter(|cell| wet(cells[cell.index()]) && land_dist.cell_steps(*cell).is_none())
            .collect();
        let core_reach = grid
            .topology
            .cell_distances_with(&core_sources, 2, |cell| wet(cells[cell.index()]));
        for cell_index in 0..grid.cell_count() {
            let cell = grid
                .topology
                .cell(cell_index)
                .expect("cell index from topology range");
            if wet(cells[cell_index]) && core_reach.cell_steps(cell).is_none() {
                cells[cell_index] = fill_kind(cells, cell_index);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
}

/// Enclosed water smaller than this many cells is filled to land — a couple
/// of tiles of "lake" beside the ocean makes no sense (one cell ≈ two fine
/// faces of area, so 15 cells ≈ the old 30-face minimum).
/// Size envelope, in CELLS, for every tile kind — the area analogue of
/// `elev_range`. `min`: a contiguous region smaller than this is not viable
/// and gets absorbed/filled/renamed away. `max`: a region larger than this is
/// split (only kinds with a splitter — the coastal bands — enforce it; all
/// others are `usize::MAX`, i.e. unbounded). Oceans and lakes are separated
/// here. Isolated ocean-zone pockets are filled; only authored lake-zone
/// components may receive Lake identity.
/// Nearest cell to a point: the closest corner of the face under it.
pub(in crate::worldgen) fn nearest_cell(grid: &Grid, p: SpherePos) -> Option<CellId> {
    let face_index = grid.planet.face_at(p.0)?;
    grid.face_cells(FaceId::new(face_index))
        .into_iter()
        .max_by(|&a, &b| {
            grid.cell_direction(a)
                .dot(p.0)
                .partial_cmp(&grid.cell_direction(b).dot(p.0))
                .unwrap()
        })
}
