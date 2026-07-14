use bevy::prelude::Vec3;

use crate::planet::{PlanetMesh, unit_icosphere_tris};
use crate::sphere::SpherePos;
use crate::topology::{CellId, FaceId, TerrainTopology};
use crate::zones::FINE_SUB;

/// Fine generation lattice. Terrain identity lives on its vertices (`CellId`);
/// triangles (`FaceId`) are derived query and presentation output.
pub(super) struct Grid {
    pub(super) seed: u32,
    pub(super) unit_tris: Vec<[Vec3; 3]>,
    pub(super) planet: PlanetMesh,
    pub(super) topology: TerrainTopology,
}

impl Grid {
    pub(super) fn new(seed: u32) -> Self {
        let unit_tris = unit_icosphere_tris(FINE_SUB);
        let planet = PlanetMesh::new(unit_tris.clone());
        let topology_tris = unit_tris
            .iter()
            .map(|triangle| triangle.map(|position| position.to_array()))
            .collect::<Vec<_>>();
        let topology = TerrainTopology::from_triangles(&topology_tris);
        debug_assert!(
            topology
                .cells()
                .all(|cell| (5..=6).contains(&topology.cell_neighbors(cell).len())),
            "hex adjacency broken"
        );
        debug_assert_eq!(topology.face_count(), unit_tris.len());
        Self {
            seed,
            unit_tris,
            planet,
            topology,
        }
    }

    pub(super) fn centroid(&self, face: FaceId) -> SpherePos {
        let [a, b, c] = self.unit_tris[face.index()];
        SpherePos::new(((a + b + c) / 3.0).normalize())
    }

    pub(super) fn cell_position(&self, cell: CellId) -> SpherePos {
        SpherePos::new(self.cell_direction(cell))
    }

    pub(super) fn cell_direction(&self, cell: CellId) -> Vec3 {
        Vec3::from_array(self.topology.cell_position(cell)).normalize()
    }

    pub(super) fn cell_count(&self) -> usize {
        self.topology.cell_count()
    }

    pub(super) fn face_count(&self) -> usize {
        self.topology.face_count()
    }

    pub(super) fn face_neighbors(&self, face: FaceId) -> [FaceId; 3] {
        self.topology.face_neighbors(face)
    }

    pub(super) fn face_cells(&self, face: FaceId) -> [CellId; 3] {
        self.topology.face_cells(face)
    }

    pub(super) fn cell_neighbors(&self, cell: CellId) -> &[CellId] {
        self.topology.cell_neighbors(cell)
    }
}
