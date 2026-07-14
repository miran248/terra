//! Project-owned terrain connectivity.
//!
//! Cells are the authoritative terrain domain. Faces are triangles derived
//! from three cells and exist for projection, queries, and mesh output.

use std::collections::{BTreeMap, VecDeque};

macro_rules! id {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(u32);

        impl $name {
            pub fn index(self) -> usize {
                self.0 as usize
            }
            pub(crate) fn new(index: usize) -> Self {
                Self(u32::try_from(index).expect("terrain topology exceeds u32 ids"))
            }
        }
    };
}

id!(CellId);
id!(FaceId);
id!(CellComponentId);
id!(FaceComponentId);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComponentLabels<I> {
    labels: Vec<Option<I>>,
    count: usize,
}

impl<I> ComponentLabels<I> {
    pub fn count(&self) -> usize {
        self.count
    }
}

impl ComponentLabels<CellComponentId> {
    pub fn cell(&self, cell: CellId) -> Option<CellComponentId> {
        self.labels[cell.index()]
    }
}

impl ComponentLabels<FaceComponentId> {
    pub fn face(&self, face: FaceId) -> Option<FaceComponentId> {
        self.labels[face.index()]
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DistanceField<I> {
    steps: Vec<Option<u32>>,
    nearest_source: Vec<Option<I>>,
}

impl DistanceField<CellId> {
    pub fn cell_steps(&self, cell: CellId) -> Option<u32> {
        self.steps[cell.index()]
    }
    pub fn nearest_cell(&self, cell: CellId) -> Option<CellId> {
        self.nearest_source[cell.index()]
    }
}

impl DistanceField<FaceId> {
    pub fn face_steps(&self, face: FaceId) -> Option<u32> {
        self.steps[face.index()]
    }
    pub fn nearest_face(&self, face: FaceId) -> Option<FaceId> {
        self.nearest_source[face.index()]
    }
}

#[derive(Clone, Debug)]
pub struct TerrainTopology {
    cell_positions: Vec<[f32; 3]>,
    cell_neighbors: Vec<Vec<CellId>>,
    cell_faces: Vec<Vec<FaceId>>,
    face_cells: Vec<[CellId; 3]>,
    face_neighbors: Vec<[FaceId; 3]>,
    face_centroids: Vec<[f32; 3]>,
}

impl TerrainTopology {
    pub fn from_triangles(triangles: &[[[f32; 3]; 3]]) -> Self {
        let mut map = BTreeMap::<[u32; 3], CellId>::new();
        let mut cell_positions = Vec::new();
        let mut face_cells = Vec::with_capacity(triangles.len());
        for triangle in triangles {
            let cells = triangle.map(|position| {
                let key = position.map(f32::to_bits);
                *map.entry(key).or_insert_with(|| {
                    let id = CellId::new(cell_positions.len());
                    cell_positions.push(position);
                    id
                })
            });
            face_cells.push(cells);
        }
        let mut cell_neighbors = vec![Vec::new(); cell_positions.len()];
        let mut cell_faces = vec![Vec::new(); cell_positions.len()];
        let mut edges = BTreeMap::<(CellId, CellId), (FaceId, usize)>::new();
        let mut raw_face_neighbors = vec![[None; 3]; triangles.len()];
        for (index, cells) in face_cells.iter().copied().enumerate() {
            let face = FaceId::new(index);
            for cell in cells {
                cell_faces[cell.index()].push(face);
            }
            for edge in 0..3 {
                let (a, b) = (cells[edge], cells[(edge + 1) % 3]);
                if !cell_neighbors[a.index()].contains(&b) {
                    cell_neighbors[a.index()].push(b);
                }
                if !cell_neighbors[b.index()].contains(&a) {
                    cell_neighbors[b.index()].push(a);
                }
                let key = if a < b { (a, b) } else { (b, a) };
                if let Some((other, other_edge)) = edges.remove(&key) {
                    raw_face_neighbors[index][edge] = Some(other);
                    raw_face_neighbors[other.index()][other_edge] = Some(face);
                } else {
                    edges.insert(key, (face, edge));
                }
            }
        }
        assert!(
            edges.is_empty(),
            "terrain topology must be a closed triangle mesh"
        );
        for values in cell_neighbors.iter_mut() {
            values.sort_unstable();
        }
        for values in cell_faces.iter_mut() {
            values.sort_unstable();
        }
        let face_neighbors = raw_face_neighbors
            .into_iter()
            .map(|neighbors| neighbors.map(|neighbor| neighbor.expect("closed mesh face neighbor")))
            .collect();
        let face_centroids = triangles
            .iter()
            .map(|triangle| {
                let sum = [
                    triangle.iter().map(|p| p[0]).sum::<f32>(),
                    triangle.iter().map(|p| p[1]).sum::<f32>(),
                    triangle.iter().map(|p| p[2]).sum::<f32>(),
                ];
                let length = sum.iter().map(|v| v * v).sum::<f32>().sqrt();
                sum.map(|v| v / length)
            })
            .collect();
        Self {
            cell_positions,
            cell_neighbors,
            cell_faces,
            face_cells,
            face_neighbors,
            face_centroids,
        }
    }

    pub fn cell_count(&self) -> usize {
        self.cell_positions.len()
    }
    pub fn face_count(&self) -> usize {
        self.face_cells.len()
    }
    pub fn cells(&self) -> impl ExactSizeIterator<Item = CellId> {
        (0..self.cell_count()).map(CellId::new)
    }
    pub fn faces(&self) -> impl ExactSizeIterator<Item = FaceId> {
        (0..self.face_count()).map(FaceId::new)
    }
    pub fn cell(&self, index: usize) -> Option<CellId> {
        (index < self.cell_count()).then(|| CellId::new(index))
    }
    pub fn face(&self, index: usize) -> Option<FaceId> {
        (index < self.face_count()).then(|| FaceId::new(index))
    }
    pub fn cell_position(&self, cell: CellId) -> [f32; 3] {
        self.cell_positions[cell.index()]
    }
    pub fn cell_neighbors(&self, cell: CellId) -> &[CellId] {
        &self.cell_neighbors[cell.index()]
    }
    pub fn cell_faces(&self, cell: CellId) -> &[FaceId] {
        &self.cell_faces[cell.index()]
    }
    pub fn face_cells(&self, face: FaceId) -> [CellId; 3] {
        self.face_cells[face.index()]
    }
    pub fn face_neighbors(&self, face: FaceId) -> [FaceId; 3] {
        self.face_neighbors[face.index()]
    }
    pub fn face_centroid(&self, face: FaceId) -> [f32; 3] {
        self.face_centroids[face.index()]
    }

    pub fn cell_components(
        &self,
        member: impl Fn(CellId) -> bool,
    ) -> ComponentLabels<CellComponentId> {
        components(
            &self.cell_neighbors,
            CellId::new,
            member,
            CellComponentId::new,
        )
    }
    pub fn face_components(
        &self,
        member: impl Fn(FaceId) -> bool,
    ) -> ComponentLabels<FaceComponentId> {
        components(
            &self.face_neighbors,
            FaceId::new,
            member,
            FaceComponentId::new,
        )
    }
    pub fn cell_component(&self, start: CellId, member: impl Fn(CellId) -> bool) -> Vec<CellId> {
        component(&self.cell_neighbors, start, member)
    }
    pub fn face_component(&self, start: FaceId, member: impl Fn(FaceId) -> bool) -> Vec<FaceId> {
        component(&self.face_neighbors, start, member)
    }
    pub fn cell_distances(&self, sources: &[CellId], max_steps: u32) -> DistanceField<CellId> {
        distances(&self.cell_neighbors, sources, max_steps, |_| true)
    }
    pub fn face_distances(&self, sources: &[FaceId], max_steps: u32) -> DistanceField<FaceId> {
        distances(&self.face_neighbors, sources, max_steps, |_| true)
    }
    pub fn cell_distances_with(
        &self,
        sources: &[CellId],
        max_steps: u32,
        member: impl Fn(CellId) -> bool,
    ) -> DistanceField<CellId> {
        distances(&self.cell_neighbors, sources, max_steps, member)
    }
    pub fn face_distances_with(
        &self,
        sources: &[FaceId],
        max_steps: u32,
        member: impl Fn(FaceId) -> bool,
    ) -> DistanceField<FaceId> {
        distances(&self.face_neighbors, sources, max_steps, member)
    }
    pub fn cell_shortest_path(
        &self,
        start: CellId,
        goal: CellId,
        max_steps: u32,
    ) -> Option<Vec<CellId>> {
        shortest_path(&self.cell_neighbors, start, goal, max_steps)
    }
    pub fn face_shortest_path(
        &self,
        start: FaceId,
        goal: FaceId,
        max_steps: u32,
    ) -> Option<Vec<FaceId>> {
        shortest_path(&self.face_neighbors, start, goal, max_steps)
    }
    pub fn cell_shortest_path_to(
        &self,
        start: CellId,
        max_steps: u32,
        goal: impl Fn(CellId) -> bool,
    ) -> Option<Vec<CellId>> {
        shortest_path_to(&self.cell_neighbors, start, max_steps, goal)
    }
}

fn component<I: Copy + IdIndex>(
    adjacency: &[impl AsRef<[I]>],
    start: I,
    member: impl Fn(I) -> bool,
) -> Vec<I> {
    if !member(start) {
        return Vec::new();
    }
    let mut visited = vec![false; adjacency.len()];
    let mut result = vec![start];
    let mut queue = VecDeque::from([start]);
    visited[start.index()] = true;
    while let Some(current) = queue.pop_front() {
        for &next in adjacency[current.index()].as_ref() {
            if !visited[next.index()] && member(next) {
                visited[next.index()] = true;
                result.push(next);
                queue.push_back(next);
            }
        }
    }
    result
}

fn components<I, C: Copy>(
    adjacency: &[impl AsRef<[I]>],
    id: impl Fn(usize) -> I,
    member: impl Fn(I) -> bool,
    component: impl Fn(usize) -> C,
) -> ComponentLabels<C>
where
    I: Copy + IdIndex,
{
    let mut labels = vec![None; adjacency.len()];
    let mut count = 0;
    for index in 0..adjacency.len() {
        let start = id(index);
        if labels[index].is_some() || !member(start) {
            continue;
        }
        let label = component(count);
        count += 1;
        labels[index] = Some(label);
        let mut queue = VecDeque::from([start]);
        while let Some(current) = queue.pop_front() {
            for &next in adjacency[current.index()].as_ref() {
                if labels[next.index()].is_none() && member(next) {
                    labels[next.index()] = Some(label);
                    queue.push_back(next);
                }
            }
        }
    }
    ComponentLabels { labels, count }
}

trait IdIndex {
    fn index(self) -> usize;
}
impl IdIndex for CellId {
    fn index(self) -> usize {
        self.index()
    }
}
impl IdIndex for FaceId {
    fn index(self) -> usize {
        self.index()
    }
}

fn distances<I: Copy + IdIndex>(
    adjacency: &[impl AsRef<[I]>],
    sources: &[I],
    max_steps: u32,
    member: impl Fn(I) -> bool,
) -> DistanceField<I> {
    let mut steps = vec![None; adjacency.len()];
    let mut nearest_source = vec![None; adjacency.len()];
    let mut queue = VecDeque::new();
    for &source in sources {
        if member(source) && steps[source.index()].is_none() {
            steps[source.index()] = Some(0);
            nearest_source[source.index()] = Some(source);
            queue.push_back(source);
        }
    }
    while let Some(current) = queue.pop_front() {
        let step = steps[current.index()].unwrap();
        if step >= max_steps {
            continue;
        }
        for &next in adjacency[current.index()].as_ref() {
            if member(next) && steps[next.index()].is_none() {
                steps[next.index()] = Some(step + 1);
                nearest_source[next.index()] = nearest_source[current.index()];
                queue.push_back(next);
            }
        }
    }
    DistanceField {
        steps,
        nearest_source,
    }
}

fn shortest_path<I: Copy + Eq + IdIndex>(
    adjacency: &[impl AsRef<[I]>],
    start: I,
    goal: I,
    max_steps: u32,
) -> Option<Vec<I>> {
    shortest_path_to(adjacency, start, max_steps, |candidate| candidate == goal)
}

fn shortest_path_to<I: Copy + Eq + IdIndex>(
    adjacency: &[impl AsRef<[I]>],
    start: I,
    max_steps: u32,
    goal: impl Fn(I) -> bool,
) -> Option<Vec<I>> {
    let mut predecessor = vec![None; adjacency.len()];
    let mut steps = vec![u32::MAX; adjacency.len()];
    predecessor[start.index()] = Some(start);
    steps[start.index()] = 0;
    let mut queue = VecDeque::from([start]);
    let found = loop {
        let current = queue.pop_front()?;
        if goal(current) {
            break current;
        }
        if steps[current.index()] >= max_steps {
            continue;
        }
        for &next in adjacency[current.index()].as_ref() {
            if predecessor[next.index()].is_none() {
                predecessor[next.index()] = Some(current);
                steps[next.index()] = steps[current.index()] + 1;
                queue.push_back(next);
            }
        }
    };
    let mut path = vec![found];
    while *path.last().unwrap() != start {
        path.push(predecessor[path.last().unwrap().index()]?);
    }
    path.reverse();
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tetrahedron() -> TerrainTopology {
        let p = [[1., 1., 1.], [-1., -1., 1.], [-1., 1., -1.], [1., -1., -1.]];
        TerrainTopology::from_triangles(&[
            [p[0], p[2], p[1]],
            [p[0], p[1], p[3]],
            [p[0], p[3], p[2]],
            [p[1], p[2], p[3]],
        ])
    }

    #[test]
    fn typed_adjacency_and_incidence_are_consistent() {
        let topology = tetrahedron();
        assert_eq!((topology.cell_count(), topology.face_count()), (4, 4));
        for cell in topology.cells() {
            assert_eq!(topology.cell_neighbors(cell).len(), 3);
            assert_eq!(topology.cell_faces(cell).len(), 3);
        }
        for face in topology.faces() {
            assert_eq!(topology.face_neighbors(face).len(), 3);
            assert!(
                topology
                    .face_cells(face)
                    .iter()
                    .all(|cell| topology.cell_faces(*cell).contains(&face))
            );
        }
    }

    #[test]
    fn typed_traversals_cover_both_domains() {
        let topology = tetrahedron();
        let cells: Vec<_> = topology.cells().collect();
        let faces: Vec<_> = topology.faces().collect();
        let cc = topology.cell_components(|cell| cell != cells[3]);
        assert_eq!(cc.count(), 1);
        assert_eq!(cc.cell(cells[3]), None);
        let fc = topology.face_components(|face| face != faces[3]);
        assert_eq!(fc.count(), 1);
        assert_eq!(fc.face(faces[3]), None);
        assert_eq!(
            topology
                .cell_component(cells[0], |cell| cell != cells[3])
                .len(),
            3
        );
        assert_eq!(
            topology
                .face_component(faces[0], |face| face != faces[3])
                .len(),
            3
        );
        let cd = topology.cell_distances(&[cells[0]], 1);
        assert_eq!(cd.cell_steps(cells[1]), Some(1));
        assert_eq!(cd.nearest_cell(cells[1]), Some(cells[0]));
        let fd = topology.face_distances(&[faces[0]], 1);
        assert_eq!(fd.face_steps(faces[1]), Some(1));
        assert_eq!(fd.nearest_face(faces[1]), Some(faces[0]));
        let filtered_cd = topology.cell_distances_with(&[cells[0]], 1, |cell| cell != cells[1]);
        assert_eq!(filtered_cd.cell_steps(cells[1]), None);
        assert_eq!(filtered_cd.cell_steps(cells[2]), Some(1));
        let filtered_fd = topology.face_distances_with(&[faces[0]], 1, |face| face != faces[1]);
        assert_eq!(filtered_fd.face_steps(faces[1]), None);
        assert_eq!(filtered_fd.face_steps(faces[2]), Some(1));
        assert_eq!(
            topology.cell_shortest_path(cells[0], cells[1], 1),
            Some(vec![cells[0], cells[1]])
        );
        assert_eq!(
            topology.face_shortest_path(faces[0], faces[1], 1),
            Some(vec![faces[0], faces[1]])
        );
        assert_eq!(
            topology.cell_shortest_path_to(cells[0], 1, |cell| cell == cells[2]),
            Some(vec![cells[0], cells[2]])
        );
        assert_eq!(
            topology.cell_shortest_path_to(cells[0], 0, |cell| cell == cells[2]),
            None
        );
    }
}
