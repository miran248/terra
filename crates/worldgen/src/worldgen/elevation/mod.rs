pub(super) mod generation;

/// Hard per-solver-vertex elevation intervals constructed from terrain,
/// landform, shelf, river, and feature constraints.
pub(super) struct ElevationConstraints {
    pub(super) lower: Vec<f32>,
    pub(super) upper: Vec<f32>,
}

impl ElevationConstraints {
    pub(super) fn unconstrained(vertex_count: usize) -> Self {
        Self {
            lower: vec![-1.0; vertex_count],
            upper: vec![1.0; vertex_count],
        }
    }
}

/// Runs relaxation in its declared order and stops only after a complete pass.
pub(super) fn ordered_relaxation(
    mut field: Vec<f32>,
    max_iterations: usize,
    epsilon: f32,
    mut relax_once: impl FnMut(&mut [f32]) -> f32,
) -> (Vec<f32>, usize, f32) {
    let mut iterations = 0;
    let mut residual = f32::MAX;
    for iteration in 0..max_iterations {
        residual = relax_once(&mut field);
        iterations = iteration + 1;
        if residual < epsilon {
            break;
        }
    }
    (field, iterations, residual)
}

/// Final serialized elevation classification range.
pub(super) fn classify_result(field: &mut [f32]) {
    for value in field {
        *value = value.clamp(-1.0, 1.0);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SolverVertexId(usize);

impl SolverVertexId {
    pub(super) fn new(index: usize) -> Self {
        Self(index)
    }

    pub(super) fn index(self) -> usize {
        self.0
    }
}

/// Typed view over the solver mesh; these vertices are not terrain cells.
pub(super) struct SolverVertexGraph<'a> {
    adjacency: &'a [Vec<usize>],
}

/// Typed solver-vertex view over the elevation mesh owned by `TerrainGen`.
pub(super) struct TerrainSolverVertexGraph<'a> {
    terrain: &'a crate::terrain::TerrainGen,
}

impl<'a> TerrainSolverVertexGraph<'a> {
    pub(super) fn new(terrain: &'a crate::terrain::TerrainGen) -> Self {
        Self { terrain }
    }

    pub(super) fn vertices(&self) -> impl Iterator<Item = SolverVertexId> {
        (0..self.terrain.vert_count()).map(SolverVertexId::new)
    }

    pub(super) fn neighbors(
        &self,
        vertex: SolverVertexId,
    ) -> impl Iterator<Item = SolverVertexId> + '_ {
        self.terrain
            .adj_of(vertex.index())
            .iter()
            .copied()
            .map(SolverVertexId::new)
    }
}

impl<'a> SolverVertexGraph<'a> {
    pub(super) fn new(adjacency: &'a [Vec<usize>]) -> Self {
        Self { adjacency }
    }

    pub(super) fn vertices(&self) -> impl Iterator<Item = SolverVertexId> {
        (0..self.adjacency.len()).map(SolverVertexId::new)
    }

    pub(super) fn neighbors(
        &self,
        vertex: SolverVertexId,
    ) -> impl Iterator<Item = SolverVertexId> + '_ {
        self.adjacency[vertex.index()]
            .iter()
            .copied()
            .map(SolverVertexId::new)
    }
}
