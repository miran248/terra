// ---- mesh ----

/// Road surface material from the ground a road face crosses: sand on desert
/// and beach, rock in the mountains and on steep ground, dirt on soil, gravel
/// otherwise. Read from the underlying cover/landform/slope at the corners
/// (roads are an overlay — the cells still hold the terrain beneath).
/// Reduce a per-cell u8 to per-face by taking the max over the face's corner
/// cells (used for slope/depth: a face is as steep/deep as its worst corner).
/// Cluster authoritative terrain cells whose type occurs in `types`.
///
/// Every selected type belongs to the same membership class: adjacent selected
/// cells connect even when their terrain types differ. Cells outside the set
/// have no typed label. An empty set has no components and duplicate types do
/// not affect the result.
pub(in crate::worldgen) fn cluster_cell_types(
    grid: &Grid,
    cells: &[Terrain],
    types: &[Terrain],
) -> ComponentLabels<CellComponentId> {
    assert_eq!(
        cells.len(),
        grid.cell_count(),
        "cell type count must match the grid"
    );
    grid.topology
        .cell_components(|cell| types.contains(&cells[cell.index()]))
}

/// Cluster derived face tiles whose type occurs in `types`.
///
/// Every selected type belongs to the same membership class: selected faces
/// connect across shared edges even when their terrain types differ. The
/// returned labels are `-1` for faces outside the set; each connected selected
/// group has one non-negative id. An empty set has no components and duplicate
/// types do not affect the result.
#[cfg(test)]
pub(in crate::worldgen) fn cluster_face_types(
    grid: &Grid,
    face_types: &[Terrain],
    types: &[Terrain],
) -> ComponentLabels<FaceComponentId> {
    assert_eq!(
        face_types.len(),
        grid.face_count(),
        "face type count must match the grid"
    );
    grid.topology
        .face_components(|face| types.contains(&face_types[face.index()]))
}
use crate::terrain::Terrain;
#[cfg(test)]
use terra_geometry::topology::FaceComponentId;
use terra_geometry::topology::{CellComponentId, ComponentLabels};
use crate::worldgen::Grid;
