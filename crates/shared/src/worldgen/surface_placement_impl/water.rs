use super::super::*;

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

/// Per-face water-surface radius (0.0 = dry) — the GEOMETRY of the water sheet.
/// Body IDENTITY/naming lives in the region clustering (`build_regions`: a lake
/// is a `RegionKind::Lake` cluster, an ocean a `RegionKind::Ocean` one); this
/// only decides where the sheet sits.
///
/// Two cell-component passes:
///   • lakes cluster on `Lake` tiles ONLY — transition tiles (LakeShore/Cliff)
///     are NOT members, so a shore/cliff chain between two lakes can't fuse them
///     into one body (that fusion put three lakes on one waterline);
///   • ocean clusters `Ocean` + `Beach` + `Cliff` — the sea is one body anyway,
///     and the coast tiles let the sheet cover the shoreline.
/// A lake body's waterline is its `LakeShore` RIM (a low percentile of the shore
/// radii adjacent to its Lake tiles) — the shore, not the deep bed. A face joins
/// the sheet only if it has no solid-land corner, so water never spills onto
/// land; lake faces are any face touching the body's Lake tiles (fills to shore
/// without drawing the outer shore band).
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
    let lake_components = cluster_cell_types(grid, cells, &[Terrain::Lake]);
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
            for nb in cell_neighbor_indices(grid, v) {
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

/// Extra clearance above the smooth River-only measurement field. This exceeds
/// water-wave displacement, so the channel terrain cannot pierce the skin.
pub(in crate::worldgen) const RIVER_SURFACE_CLEARANCE: f32 = 0.02;
/// Pull exterior river-bank edges into their adjacent ground. This hides the
/// open seam where a widened river surface meets uneven terrain.
pub(in crate::worldgen) const RIVER_TERRAIN_CLIP: f32 = 0.25;
/// Number of mesh-vertex rings over which a spring grows from its embedded
/// source into the normal channel surface.
pub(in crate::worldgen) const RIVER_SPRING_TAPER_RINGS: usize = 3;

/// Bake a smooth, terrain-following river surface. River, RiverSpring, and
/// RiverBank faces form the core; one non-water, non-cliff face apron widens
/// that core beneath the terrain. Bank/apron-only clusters remain dry. River
/// faces measure channel height, Spring faces anchor beneath the ground,
/// adjoining lake/ocean waterlines anchor outlets, and bank/apron vertices clip
/// into terrain; RiverBank faces widen coverage and interpolate the nearby channel surface. A
/// component-wide clearance keeps the smooth field above every measured River
/// corner without copying noisy bank terrain into the water. Cliffs stay
/// excluded: a river mouth must not flood a coast.
pub(in crate::worldgen) fn river_surface_radii(
    grid: &Grid,
    mesh_tris: &[[[f32; 3]; 3]],
    face_types: &[Terrain],
    face_water_r: &[f32],
) -> Vec<[f32; 3]> {
    assert_eq!(
        mesh_tris.len(),
        grid.face_count(),
        "river mesh must match the grid"
    );
    assert_eq!(
        face_types.len(),
        grid.face_count(),
        "river face types must match the grid"
    );
    assert_eq!(
        face_water_r.len(),
        grid.face_count(),
        "river waterlines must match the grid"
    );
    let core: Vec<bool> = face_types
        .iter()
        .map(|&t| {
            matches!(
                t,
                Terrain::River | Terrain::RiverSpring | Terrain::RiverBank
            )
        })
        .collect();
    // The rendering apron is deliberately buried in its neighboring terrain:
    // it adds a full face ring beyond irregular RiverBank tiles, so the water
    // skin cannot end short of the visible bank. Do not spread into cliffs or
    // another water body; outlet edges have their own exact waterline anchor.
    let footprint: Vec<bool> = (0..grid.face_count())
        .map(|face_index| {
            core[face_index]
                || (!face_types[face_index].is_water()
                    && face_types[face_index] != Terrain::Cliff
                    && grid
                        .face_neighbors(FaceId::new(face_index))
                        .map(FaceId::index)
                        .into_iter()
                        .any(|neighbor| core[neighbor]))
        })
        .collect();
    let components = grid
        .topology
        .face_components(|face| footprint[face.index()]);
    let component: Vec<_> = grid
        .topology
        .faces()
        .map(|face| components.face(face))
        .collect();
    let mut has_river = vec![false; components.count()];
    for face_index in 0..grid.face_count() {
        if let Some(component) = component[face_index]
            && matches!(
                face_types[face_index],
                Terrain::River | Terrain::RiverSpring
            )
        {
            has_river[component.index()] = true;
        }
    }

    let key = |p: [f32; 3]| [p[0].to_bits(), p[1].to_bits(), p[2].to_bits()];
    let mut node_of: BTreeMap<[u32; 3], usize> = BTreeMap::new();
    let mut nodes = vec![[usize::MAX; 3]; grid.face_count()];
    let mut neighbors: Vec<Vec<usize>> = Vec::new();
    let mut measured_sum: Vec<f32> = Vec::new();
    let mut measured_count: Vec<u32> = Vec::new();
    let mut river_corner: Vec<f32> = Vec::new();
    let mut ground_radius: Vec<f32> = Vec::new();
    let mut spring_anchor: Vec<f32> = Vec::new();
    let mut outlet_anchor: Vec<f32> = Vec::new();
    let mut bank_edge_anchor: Vec<f32> = Vec::new();
    for face_index in 0..grid.face_count() {
        let Some(c) = component[face_index] else {
            continue;
        };
        if !has_river[c.index()] {
            continue;
        }
        for (k, &corner) in mesh_tris[face_index].iter().enumerate() {
            let next = node_of.len();
            let node = *node_of.entry(key(corner)).or_insert_with(|| {
                neighbors.push(Vec::new());
                measured_sum.push(0.0);
                measured_count.push(0);
                river_corner.push(f32::MIN);
                ground_radius.push(Vec3::from_array(corner).length());
                spring_anchor.push(f32::MIN);
                outlet_anchor.push(f32::MIN);
                bank_edge_anchor.push(f32::MIN);
                next
            });
            nodes[face_index][k] = node;
        }
        for edge in 0..3 {
            let (a, b) = (nodes[face_index][edge], nodes[face_index][(edge + 1) % 3]);
            if !neighbors[a].contains(&b) {
                neighbors[a].push(b);
                neighbors[b].push(a);
            }
        }
        if face_types[face_index] == Terrain::River {
            let radius = mesh_tris[face_index]
                .iter()
                .map(|&p| Vec3::from_array(p).length())
                .sum::<f32>()
                / 3.0;
            for (k, &corner) in mesh_tris[face_index].iter().enumerate() {
                let node = nodes[face_index][k];
                measured_sum[node] += radius;
                measured_count[node] += 1;
                river_corner[node] = river_corner[node].max(Vec3::from_array(corner).length());
            }
        }
        if face_types[face_index] == Terrain::RiverSpring {
            for (k, &corner) in mesh_tris[face_index].iter().enumerate() {
                let node = nodes[face_index][k];
                let radius = Vec3::from_array(corner).length();
                measured_sum[node] += radius;
                measured_count[node] += 1;
                river_corner[node] = river_corner[node].max(radius);
                spring_anchor[node] = spring_anchor[node].max(radius);
            }
        }
    }

    // The outside edge of a widened RiverBank component must meet the ground,
    // not a channel-height interpolation that can float beside a deep or wide
    // bank. Sink it just below the ground to avoid a visible crack from tiny
    // precision differences between the independently drawn meshes.
    for face_index in 0..grid.face_count() {
        let Some(c) = component[face_index] else {
            continue;
        };
        if !has_river[c.index()] {
            continue;
        }
        for neighbor in grid
            .face_neighbors(FaceId::new(face_index))
            .map(FaceId::index)
        {
            if component[neighbor] == Some(c) {
                continue;
            }
            for (k, &corner) in mesh_tris[face_index].iter().enumerate() {
                if mesh_tris[neighbor]
                    .iter()
                    .any(|&other| key(other) == key(corner))
                {
                    let node = nodes[face_index][k];
                    bank_edge_anchor[node] =
                        bank_edge_anchor[node].max(ground_radius[node] - RIVER_TERRAIN_CLIP);
                }
            }
        }
    }

    // An outlet shares its final edge with the already-baked lake/ocean mesh.
    // Feed that exact waterline into the river interpolation and retain it as
    // a hard final anchor, so the two independently drawn meshes join without
    // a vertical seam or a dry gap.
    for face_index in 0..grid.face_count() {
        let Some(c) = component[face_index] else {
            continue;
        };
        if !has_river[c.index()] {
            continue;
        }
        for neighbor in grid
            .face_neighbors(FaceId::new(face_index))
            .map(FaceId::index)
        {
            let waterline = face_water_r[neighbor];
            if waterline <= 0.0 {
                continue;
            }
            for (k, &corner) in mesh_tris[face_index].iter().enumerate() {
                if mesh_tris[neighbor]
                    .iter()
                    .any(|&other| key(other) == key(corner))
                {
                    let node = nodes[face_index][k];
                    measured_sum[node] += waterline;
                    measured_count[node] += 1;
                    outlet_anchor[node] = outlet_anchor[node].max(waterline);
                }
            }
        }
    }

    // Seed at River-only measurements, then extend them across RiverBank faces.
    // The fixed River samples retain the smooth channel profile; bank-only
    // vertices solve a discrete harmonic extension of that profile.
    let mut surface: Vec<Option<f32>> = measured_sum
        .iter()
        .zip(&measured_count)
        .map(|(&sum, &count)| (count > 0).then(|| sum / count as f32))
        .collect();
    for _ in 0..surface.len() {
        let mut changed = false;
        for node in 0..surface.len() {
            if surface[node].is_some() {
                continue;
            }
            let mut sum = 0.0;
            let mut count = 0usize;
            for &neighbor in &neighbors[node] {
                if let Some(radius) = surface[neighbor] {
                    sum += radius;
                    count += 1;
                }
            }
            if count > 0 {
                surface[node] = Some(sum / count as f32);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    // Shortest mesh-vertex distance from each Spring vertex. A linear blend
    // through the first few rings lets water emerge from the ground naturally
    // instead of ending as an abrupt, hovering cap at the source patch.
    let mut spring_distance = vec![usize::MAX; surface.len()];
    let solver_graph = elevation::SolverVertexGraph::new(&neighbors);
    let mut frontier: VecDeque<elevation::SolverVertexId> = VecDeque::new();
    for solver_vertex in solver_graph.vertices() {
        if spring_anchor[solver_vertex.index()] > f32::MIN {
            spring_distance[solver_vertex.index()] = 0;
            frontier.push_back(solver_vertex);
        }
    }
    while let Some(solver_vertex) = frontier.pop_front() {
        if spring_distance[solver_vertex.index()] >= RIVER_SPRING_TAPER_RINGS {
            continue;
        }
        for neighbor in solver_graph.neighbors(solver_vertex) {
            if spring_distance[neighbor.index()] == usize::MAX {
                spring_distance[neighbor.index()] = spring_distance[solver_vertex.index()] + 1;
                frontier.push_back(neighbor);
            }
        }
    }
    let surface: Vec<f32> = surface
        .into_iter()
        .map(|radius| radius.unwrap_or(0.0))
        .collect();
    // Keep one clearance per connected river. This is the intentionally smooth
    // water field: a local terrain spike cannot add a visible crease or seam
    // across the channel. The buried apron only widens its footprint.
    let mut clearance = vec![RIVER_SURFACE_CLEARANCE; components.count()];
    for face_index in 0..grid.face_count() {
        let Some(c) = component[face_index] else {
            continue;
        };
        if face_types[face_index] != Terrain::River {
            continue;
        }
        let c = c.index();
        for &node in &nodes[face_index] {
            clearance[c] =
                clearance[c].max(river_corner[node] - surface[node] + RIVER_SURFACE_CLEARANCE);
        }
    }
    (0..grid.face_count())
        .map(|face_index| {
            let Some(c) = component[face_index] else {
                return [0.0; 3];
            };
            if !has_river[c.index()] {
                return [0.0; 3];
            }
            nodes[face_index].map(|node| {
                if outlet_anchor[node] > f32::MIN {
                    outlet_anchor[node]
                } else if spring_anchor[node] > f32::MIN {
                    // Start slightly inside the source terrain. This prevents
                    // coplanar z-fighting while the following tapered rings let
                    // the water emerge naturally from the carved bed.
                    spring_anchor[node] - RIVER_TERRAIN_CLIP
                } else if bank_edge_anchor[node] > f32::MIN {
                    bank_edge_anchor[node]
                } else {
                    let channel = surface[node] + clearance[c.index()];
                    let rings = RIVER_SPRING_TAPER_RINGS as f32;
                    let taper = (spring_distance[node] as f32 / rings).min(1.0);
                    ground_radius[node] + (channel - ground_radius[node]) * taper
                }
            })
        })
        .collect()
}

pub(in crate::worldgen) fn face_max(grid: &Grid, per_cell: &[u8]) -> Vec<u8> {
    projection::face_max(grid, per_cell)
}

/// Reduce a per-cell u8 to per-face by majority corner (used for landform: the
/// massif a face sits in).
pub(in crate::worldgen) fn face_majority(grid: &Grid, per_cell: &[u8]) -> Vec<u8> {
    projection::face_majority(grid, per_cell)
}
