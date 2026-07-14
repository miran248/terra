use super::*;

#[test]
fn kernel_is_local() {
    // The interpolation kernel must return genuinely nearby vertices.
    // Guards the polar search bug: the fixed 3x3 grid window returned
    // verts up to 335m away near the poles (longitude cells shrink).
    let terrain = TerrainGen::init(1);
    for i in 0..2000 {
        let u = (i as f32 * 0.6180339) % 1.0;
        let v = (i as f32 * 0.7548776) % 1.0;
        let p = crate::sphere::random_point(u, v);
        for (solver_vertex, _) in terrain.kernel(p) {
            let d = p.distance(crate::sphere::SpherePos::new(
                terrain.vert_dir(solver_vertex),
            ));
            assert!(
                d < 200.0,
                "kernel vert {d:.0}m away at lat {:.0}",
                p.0.y.asin().to_degrees()
            );
        }
    }
}

#[test]
fn tile_type_sets_cluster_mixed_connected_groups() {
    let grid = Grid::new(1);

    let cell_neighbor = grid.cell_neighbors(CellId::new(0))[0].index();
    let cell_distant = (0..grid.cell_count())
        .find(|&v| {
            v != 0
                && v != cell_neighbor
                && !grid
                    .cell_neighbors(CellId::new(0))
                    .iter()
                    .any(|cell| cell.index() == v)
                && !grid
                    .cell_neighbors(CellId::new(cell_neighbor))
                    .iter()
                    .any(|cell| cell.index() == v)
        })
        .expect("grid needs a non-adjacent cell");
    let mut cells = vec![Terrain::Plains; grid.cell_count()];
    cells[0] = Terrain::Forest;
    cells[cell_neighbor] = Terrain::Mountain;
    cells[cell_distant] = Terrain::Snow;
    let cell_components = cluster_cell_types(
        &grid,
        &cells,
        &[
            Terrain::Forest,
            Terrain::Mountain,
            Terrain::Snow,
            Terrain::Forest,
        ],
    );
    assert_eq!(cell_components.count(), 2);
    let cell = |index| grid.topology.cell(index).unwrap();
    assert_eq!(
        cell_components.cell(cell(0)),
        cell_components.cell(cell(cell_neighbor))
    );
    assert_ne!(
        cell_components.cell(cell(0)),
        cell_components.cell(cell(cell_distant))
    );
    let cell_nonmember = cells.iter().position(|&t| t == Terrain::Plains).unwrap();
    assert_eq!(cell_components.cell(cell(cell_nonmember)), None);

    let face_neighbor = grid.face_neighbors(FaceId::new(0)).map(FaceId::index)[0];
    let face_distant = (0..grid.face_count())
        .find(|&f| {
            f != 0
                && f != face_neighbor
                && !grid
                    .face_neighbors(FaceId::new(0))
                    .map(FaceId::index)
                    .contains(&f)
                && !grid
                    .face_neighbors(FaceId::new(face_neighbor))
                    .map(FaceId::index)
                    .contains(&f)
        })
        .expect("grid needs a non-adjacent face");
    let mut face_types = vec![Terrain::Plains; grid.face_count()];
    face_types[0] = Terrain::Beach;
    face_types[face_neighbor] = Terrain::Cliff;
    face_types[face_distant] = Terrain::Ocean;
    let face_components = cluster_face_types(
        &grid,
        &face_types,
        &[Terrain::Beach, Terrain::Cliff, Terrain::Ocean],
    );
    assert_eq!(face_components.count(), 2);
    let face = |index| grid.topology.face(index).unwrap();
    assert_eq!(
        face_components.face(face(0)),
        face_components.face(face(face_neighbor))
    );
    assert_ne!(
        face_components.face(face(0)),
        face_components.face(face(face_distant))
    );
    let face_nonmember = face_types
        .iter()
        .position(|&t| t == Terrain::Plains)
        .unwrap();
    assert_eq!(face_components.face(face(face_nonmember)), None);

    let empty_components = cluster_cell_types(&grid, &cells, &[]);
    assert_eq!(empty_components.count(), 0);
    assert!(
        grid.topology
            .cells()
            .all(|cell| empty_components.cell(cell).is_none())
    );
}
