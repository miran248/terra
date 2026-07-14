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
fn cluster_cell_types(
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
fn cluster_face_types(
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
fn water_surface_radii(grid: &Grid, terrain: &TerrainGen, cells: &[Terrain]) -> Vec<f32> {
    let sea_r = crate::sphere::PLANET_RADIUS - 2.0;

    let vert_r: Vec<f32> = grid
        .topology
        .cells()
        .map(|cell| terrain.render_radius(grid.cell_position(cell.index())))
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
            let idx = grid.face_cells(face_index);
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
            match idx
                .iter()
                .find_map(|&cell_index| lake_components.cell(grid.topology.cell(cell_index).unwrap()))
            {
                Some(component) => lake_r[component.index()],
                None => 0.0,
            }
        })
        .collect()
}

/// Extra clearance above the smooth River-only measurement field. This exceeds
/// water-wave displacement, so the channel terrain cannot pierce the skin.
const RIVER_SURFACE_CLEARANCE: f32 = 0.02;
/// Pull exterior river-bank edges into their adjacent ground. This hides the
/// open seam where a widened river surface meets uneven terrain.
const RIVER_TERRAIN_CLIP: f32 = 0.25;
/// Number of mesh-vertex rings over which a spring grows from its embedded
/// source into the normal channel surface.
const RIVER_SPRING_TAPER_RINGS: usize = 3;

/// Bake a smooth, terrain-following river surface. River, RiverSpring, and
/// RiverBank faces form the core; one non-water, non-cliff face apron widens
/// that core beneath the terrain. Bank/apron-only clusters remain dry. River
/// faces measure channel height, Spring faces anchor beneath the ground,
/// adjoining lake/ocean waterlines anchor outlets, and bank/apron vertices clip
/// into terrain; RiverBank faces widen coverage and interpolate the nearby channel surface. A
/// component-wide clearance keeps the smooth field above every measured River
/// corner without copying noisy bank terrain into the water. Cliffs stay
/// excluded: a river mouth must not flood a coast.
fn river_surface_radii(
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
                        .face_neighbors(face_index)
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
            && matches!(face_types[face_index], Terrain::River | Terrain::RiverSpring)
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
        let Some(c) = component[face_index] else { continue };
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
        let Some(c) = component[face_index] else { continue };
        if !has_river[c.index()] {
            continue;
        }
        for neighbor in grid.face_neighbors(face_index) {
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
        let Some(c) = component[face_index] else { continue };
        if !has_river[c.index()] {
            continue;
        }
        for neighbor in grid.face_neighbors(face_index) {
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
        let Some(c) = component[face_index] else { continue };
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

fn face_max(grid: &Grid, per_cell: &[u8]) -> Vec<u8> {
    projection::face_max(grid, per_cell)
}

/// Reduce a per-cell u8 to per-face by majority corner (used for landform: the
/// massif a face sits in).
fn face_majority(grid: &Grid, per_cell: &[u8]) -> Vec<u8> {
    projection::face_majority(grid, per_cell)
}

fn face_road_material(
    grid: &Grid,
    cells: &[Terrain],
    landform: &[u8],
    slope_class: &[u8],
    face_index: usize,
) -> u8 {
    let mut sand = false;
    let mut rock = false;
    let mut soil = false;
    for cell_index in grid.face_cells(face_index) {
        match cells[cell_index] {
            Terrain::Desert | Terrain::Beach | Terrain::Savanna => sand = true,
            Terrain::Mountain | Terrain::Volcanic | Terrain::Cliff => rock = true,
            Terrain::Forest
            | Terrain::Plains
            | Terrain::Swamp
            | Terrain::Jungle
            | Terrain::Tundra => soil = true,
            _ => {}
        }
        if matches!(landform[cell_index], LANDFORM_MOUNTAINS | LANDFORM_PLATEAU)
            || slope_class[cell_index] >= SLOPE_STEEP
        {
            rock = true;
        }
    }
    // Rock wins on hard/steep ground, then sand, then dirt, else gravel.
    if rock {
        ROAD_MAT_ROCK
    } else if sand {
        ROAD_MAT_SAND
    } else if soil {
        ROAD_MAT_DIRT
    } else {
        ROAD_MAT_GRAVEL
    }
}

/// Pure projection of the solved field, with PER-CORNER colors: each corner
/// takes its cell's color, so a biome boundary renders as a smooth gradient
/// across its boundary faces — a hard color seam or single-vertex color pinch
/// cannot exist. Built features override per face (they are solid structures),
/// and feature flanks fade each corner halfway toward the feature color.
type TerrainTriangles = Vec<[[f32; 3]; 3]>;
type TerrainColors = Vec<[[f32; 4]; 3]>;

fn build_mesh(
    grid: &Grid,
    terrain: &TerrainGen,
    cells: &[Terrain],
    painted: &Painted,
    water_depth: &[u8],
    landform: &[u8],
    slope_class: &[u8],
) -> (TerrainTriangles, TerrainColors) {
    let vert_r: Vec<f32> = grid
        .topology
        .cells()
        .map(|cell| terrain.render_radius(grid.cell_position(cell.index())))
        .collect();

    let road_color = bevy::prelude::Color::srgb(0.5, 0.42, 0.3)
        .to_linear()
        .to_f32_array();
    let road_mat_color = |m: u8| -> [f32; 4] {
        match m {
            ROAD_MAT_DIRT => bevy::prelude::Color::srgb(0.45, 0.33, 0.22),
            ROAD_MAT_SAND => bevy::prelude::Color::srgb(0.78, 0.70, 0.50),
            ROAD_MAT_ROCK => bevy::prelude::Color::srgb(0.40, 0.38, 0.36),
            _ => bevy::prelude::Color::srgb(0.52, 0.50, 0.47), // gravel
        }
        .to_linear()
        .to_f32_array()
    };
    let town_color = crate::theme::WARNING.to_linear().to_f32_array();
    let entry_color = bevy::prelude::Color::srgb(0.42, 0.33, 0.24)
        .to_linear()
        .to_f32_array();
    let mut tris = Vec::with_capacity(grid.face_count());
    let mut cols = Vec::with_capacity(grid.face_count());
    for face_index in 0..grid.face_count() {
        let idx = grid.face_cells(face_index);
        // Features are built structures: a face the feature OWNS (≥2 painted
        // corners — the two-triangle quads along the painted cell chain)
        // renders solid with a hard edge. Faces with exactly one painted
        // corner are the flank band and fade via the corner gradient. Total:
        // blend band / solid strip / blend band, for every feature.
        let corner = |k: usize| {
            let cell_index = idx[k];
            if painted.bridge_entries.contains(cell_index) {
                entry_color
            } else if painted.towns.contains(cell_index) {
                town_color
            } else if painted.roads.contains(cell_index) {
                road_color
            } else {
                let mut c = cells[cell_index].color().to_linear().to_f32_array();
                if cells[cell_index].is_water() {
                    // Water darkens with depth (shallow shore → dark abyss).
                    let f = match water_depth[cell_index] {
                        DEPTH_SHALLOW => 1.0,
                        DEPTH_DEEP => 0.62,
                        _ => 0.35,
                    };
                    for ch in c.iter_mut().take(3) {
                        *ch *= f;
                    }
                } else {
                    // Land: the SHAPE reads through the cover. Higher landforms
                    // darken (ruggedness), and a steep/cliff cell bleeds toward
                    // bare rock — so a forested hill, a forested mountain and a
                    // cliff face all look distinct even under the same biome.
                    let shade = match landform[cell_index] {
                        LANDFORM_MOUNTAINS => 0.82,
                        LANDFORM_PLATEAU => 0.90,
                        LANDFORM_HILLS => 0.96,
                        _ => 1.0,
                    };
                    for ch in c.iter_mut().take(3) {
                        *ch *= shade;
                    }
                    if slope_class[cell_index] >= SLOPE_STEEP {
                        let rock = [0.24, 0.21, 0.19];
                        let k = if slope_class[cell_index] == SLOPE_CLIFF {
                            0.6
                        } else {
                            0.3
                        };
                        for i in 0..3 {
                            c[i] = c[i] * (1.0 - k) + rock[i] * k;
                        }
                    }
                }
                c
            }
        };
        let color: [[f32; 4]; 3] = if face_solid(grid, &painted.bridge_entries, face_index) {
            [entry_color; 3]
        } else if face_solid(grid, &painted.towns, face_index) {
            [town_color; 3]
        } else if face_solid(grid, &painted.roads, face_index) {
            [road_mat_color(face_road_material(grid, cells, landform, slope_class, face_index)); 3]
        } else {
            // Boundary faces render ONE flat color — the equal-weight average
            // of the distinct corner colors (50/50 for a pair) — so band
            // bounds stay crisp instead of smearing into a gradient.
            let (c0, c1, c2) = (corner(0), corner(1), corner(2));
            if c0 == c1 && c1 == c2 {
                [c0; 3]
            } else {
                let mut distinct = vec![c0];
                for c in [c1, c2] {
                    if !distinct.contains(&c) {
                        distinct.push(c);
                    }
                }
                let k = distinct.len() as f32;
                let mut avg = [0.0f32; 4];
                for c in &distinct {
                    for i in 0..4 {
                        avg[i] += c[i] / k;
                    }
                }
                [avg; 3]
            }
        };
        tris.push([
            (grid.cell_direction(idx[0]) * vert_r[idx[0]]).to_array(),
            (grid.cell_direction(idx[1]) * vert_r[idx[1]]).to_array(),
            (grid.cell_direction(idx[2]) * vert_r[idx[2]]).to_array(),
        ]);
        cols.push(color);
    }
    (tris, cols)
}

/// Sub-tile decoration scatter. Flora are POINTS, not tiles: a tree consumes
/// part of a face, so it lives in its own layer over the finished mesh.
/// Densities (expected instances per face) come from the tile kind; features
/// keep their ground clear including flanks; positions are uniform barycentric
/// samples of the DISPLACED triangle so every prop sits exactly on the ground.
/// Deterministic: one seeded stream in face order.
const FLORA_RNG_SALT: u64 = 0x466c_6f72;

/// How a flora kind's density responds to ground moisture.
#[derive(Clone, Copy)]
enum FloraScale {
    /// Denser on wet ground (greenery).
    Wet,
    /// Wet, squared — meadows bloom sharply with moisture (flowers).
    WetSq,
    /// Denser on dry ground (rocks, cacti).
    Dry,
    /// Density independent of moisture (fallen logs, dead trees).
    Flat,
}

/// Base per-face density (expected instances) for each flora kind on a tile,
/// with its moisture response — the scatter analogue of `elev_range`. Faces
/// are ~9m across at sub=7, so values are small. Empty ⇒ nothing grows here.
fn flora_density(t: Terrain) -> Vec<(f32, FloraScale, u8)> {
    use FloraScale::*;
    // (base, scale, kind). Kept sparse: only the kinds that grow on this tile.
    let v: &[(f32, FloraScale, u8)] = match t {
        Terrain::Forest => &[
            (0.40, Wet, FLORA_TREE),
            (0.10, Wet, FLORA_BUSH),
            (0.012, WetSq, FLORA_FLOWER),
            (0.008, Dry, FLORA_ROCK),
            (0.075, Wet, FLORA_GRASS),
            (0.03, Flat, FLORA_LOG),
            (0.06, Wet, FLORA_MUSHROOM),
            (0.04, Wet, FLORA_BERRY),
            (0.01, Flat, FLORA_DEADTREE),
        ],
        Terrain::Jungle => &[
            (0.55, Wet, FLORA_TREE),
            (0.18, Wet, FLORA_BUSH),
            (0.02, WetSq, FLORA_FLOWER),
            (0.004, Dry, FLORA_ROCK),
            (0.10, Wet, FLORA_GRASS),
            (0.05, Flat, FLORA_LOG),
            (0.09, Wet, FLORA_MUSHROOM),
            (0.05, Wet, FLORA_BERRY),
            (0.02, Wet, FLORA_REED),
        ],
        Terrain::Swamp => &[
            (0.05, Wet, FLORA_TREE),
            (0.12, Wet, FLORA_BUSH),
            (0.03, WetSq, FLORA_FLOWER),
            (0.004, Dry, FLORA_ROCK),
            (0.10, Wet, FLORA_GRASS),
            (0.05, Flat, FLORA_LOG),
            (0.05, Wet, FLORA_MUSHROOM),
            (0.06, Flat, FLORA_DEADTREE),
            (0.18, Wet, FLORA_REED),
        ],
        Terrain::Plains => &[
            (0.01, Wet, FLORA_TREE),
            (0.025, Wet, FLORA_BUSH),
            (0.075, WetSq, FLORA_FLOWER),
            (0.005, Dry, FLORA_ROCK),
            (0.088, Wet, FLORA_GRASS),
            (0.004, Flat, FLORA_LOG),
            (0.02, Wet, FLORA_BERRY),
        ],
        Terrain::Savanna => &[
            (0.02, Wet, FLORA_TREE),
            (0.04, Wet, FLORA_BUSH),
            (0.04, WetSq, FLORA_FLOWER),
            (0.008, Dry, FLORA_ROCK),
            (0.11, Wet, FLORA_GRASS),
            (0.008, Flat, FLORA_LOG),
            (0.015, Dry, FLORA_CACTUS),
            (0.01, Wet, FLORA_BERRY),
            (0.02, Flat, FLORA_DEADTREE),
        ],
        Terrain::Tundra => &[
            (0.003, Wet, FLORA_TREE),
            (0.015, Wet, FLORA_BUSH),
            (0.005, WetSq, FLORA_FLOWER),
            (0.03, Dry, FLORA_ROCK),
            (0.012, Wet, FLORA_GRASS),
            (0.01, Flat, FLORA_LOG),
            (0.008, Wet, FLORA_BERRY),
            (0.03, Flat, FLORA_DEADTREE),
        ],
        Terrain::Desert => &[
            (0.012, Wet, FLORA_BUSH),
            (0.025, Dry, FLORA_ROCK),
            (0.06, Dry, FLORA_CACTUS),
            (0.02, Flat, FLORA_DEADTREE),
        ],
        Terrain::RiverBank | Terrain::LakeShore => &[
            (0.02, Wet, FLORA_TREE),
            (0.05, Wet, FLORA_BUSH),
            (0.062, WetSq, FLORA_FLOWER),
            (0.008, Dry, FLORA_ROCK),
            (0.075, Wet, FLORA_GRASS),
            (0.01, Flat, FLORA_LOG),
            (0.12, Wet, FLORA_REED),
        ],
        Terrain::Mountain => &[(0.005, Wet, FLORA_BUSH), (0.05, Dry, FLORA_ROCK)],
        Terrain::Cliff => &[(0.038, Dry, FLORA_ROCK)],
        Terrain::Beach => &[(0.01, Dry, FLORA_ROCK)],
        Terrain::Volcanic => &[(0.06, Dry, FLORA_ROCK)],
        Terrain::Glacier => &[(0.01, Dry, FLORA_ROCK)],
        _ => &[],
    };
    v.to_vec()
}

fn place_flora(
    grid: &Grid,
    terrain: &TerrainGen,
    tiles: &[Terrain],
    painted: &Painted,
    mesh_tris: &[[[f32; 3]; 3]],
) -> Vec<FloraData> {
    let mut rng = fastrand::Rng::with_seed(grid.seed as u64 ^ FLORA_RNG_SALT);
    let mut out = Vec::new();
    for face_index in 0..grid.face_count() {
        let clear = painted_corners(grid, &painted.roads, face_index) > 0
            || painted_corners(grid, &painted.towns, face_index) > 0
            || painted_corners(grid, &painted.bridge_entries, face_index) > 0
            || painted_corners(grid, &painted.bridges, face_index) > 0;
        if clear {
            continue;
        }
        let mix = flora_density(tiles[face_index]);
        if mix.is_empty() {
            continue;
        }
        // Moisture in roughly [-1, 1]: scale greens up on wet ground, rocks
        // up on dry ground. One sample per face keeps it cheap.
        let m = terrain.moisture_at(grid.centroid(face_index));
        let wet = (1.0 + m).clamp(0.3, 1.8);
        let dry = (1.0 - m).clamp(0.5, 1.6);
        for &(base, scale, kind) in &mix {
            let density = base
                * match scale {
                    FloraScale::Wet => wet,
                    FloraScale::WetSq => wet * wet,
                    FloraScale::Dry => dry,
                    FloraScale::Flat => 1.0,
                };
            let mut n = density.trunc() as u32;
            if rng.f32() < density.fract() {
                n += 1;
            }
            for _ in 0..n {
                let (mut u, mut v) = (rng.f32(), rng.f32());
                if u + v > 1.0 {
                    u = 1.0 - u;
                    v = 1.0 - v;
                }
                let t = &mesh_tris[face_index];
                let (a, b, c) = (
                    Vec3::from_array(t[0]),
                    Vec3::from_array(t[1]),
                    Vec3::from_array(t[2]),
                );
                let pos = a + (b - a) * u + (c - a) * v;
                out.push(FloraData {
                    pos: pos.to_array(),
                    face: face_index as u32,
                    kind,
                });
            }
        }
    }
    out
}

const STRUCT_RNG_SALT: u64 = 0x0053_7475_6375_7265;

/// Contextual structures, placed like towns/bridges: wells, campfires and
/// farms cluster in and around towns; walls ring town edges; docks reach out
/// from coastal town shores; watchtowers crown high ground near roads; ruins
/// scatter through the wilderness. Positions sit on the displaced mesh (face
/// centroids). Deterministic: one seeded stream in face order.
fn place_structures(
    grid: &Grid,
    terrain: &TerrainGen,
    tiles: &[Terrain],
    painted: &Painted,
    slope_class: &[u8],
    mesh_tris: &[[[f32; 3]; 3]],
) -> Vec<StructureData> {
    let mut rng = fastrand::Rng::with_seed(grid.seed as u64 ^ STRUCT_RNG_SALT);
    let face_center = |face_index: usize| {
        let t = &mesh_tris[face_index];
        (Vec3::from_array(t[0]) + Vec3::from_array(t[1]) + Vec3::from_array(t[2])) / 3.0
    };
    let town = |face_index: usize| face_solid(grid, &painted.towns, face_index);
    let road = |face_index: usize| face_solid(grid, &painted.roads, face_index);
    let feature = |face_index: usize| {
        town(face_index)
            || road(face_index)
            || painted_corners(grid, &painted.bridges, face_index) > 0
            || painted_corners(grid, &painted.bridge_entries, face_index) > 0
    };
    // Face-step distance from any town (capped) — cheap context for the rest.
    let town_sources: Vec<_> = grid
        .topology
        .faces()
        .filter(|face| town(face.index()))
        .collect();
    let town_field = grid.topology.face_distances(&town_sources, 6);
    let town_dist: Vec<u16> = grid
        .topology
        .faces()
        .map(|face| {
            town_field
                .face_steps(face)
                .map_or(u16::MAX, |steps| steps as u16)
        })
        .collect();
    let road_near = |face_index: usize| {
        let face = grid
            .topology
            .face(face_index)
            .expect("face index from topology range");
        grid.topology
            .face_neighbors(face)
            .iter()
            .any(|neighbor| road(neighbor.index()))
            || road(face_index)
    };

    let mut out = Vec::new();
    let mut push = |rng: &mut fastrand::Rng, face_index: usize, kind: u8| {
        out.push(StructureData {
            pos: face_center(face_index).to_array(),
            face: face_index as u32,
            kind,
            yaw: rng.f32() * std::f32::consts::TAU,
        });
    };
    // A structure needs buildable ground: skip any face with a steep/cliff
    // corner (watchtowers on a ridge are the exception — handled below).
    let buildable = |face_index: usize| {
        grid.face_cells(face_index)
            .into_iter()
            .all(|cell| slope_walkable(slope_class[cell]))
    };
    for face_index in 0..grid.face_count() {
        if tiles[face_index].is_water() || !buildable(face_index) {
            continue;
        }
        // Town interior: a well or a campfire in a clearing.
        if town(face_index) {
            let r = rng.f32();
            if r < 0.010 {
                push(&mut rng, face_index, STRUCT_WELL);
            } else if r < 0.045 {
                push(&mut rng, face_index, STRUCT_CAMPFIRE);
            }
            continue;
        }
        // Town edge (non-town land beside a town): a wall segment or a farm.
        let touches_town = grid.face_neighbors(face_index).into_iter().any(town);
        if touches_town {
            let coastal = grid
                .face_neighbors(face_index)
                .into_iter()
                .any(|neighbor| tiles[neighbor].is_water());
            if coastal && rng.f32() < 0.5 {
                push(&mut rng, face_index, STRUCT_DOCK);
            } else if rng.f32() < 0.4 {
                push(&mut rng, face_index, STRUCT_WALL);
            }
            continue;
        }
        if feature(face_index) {
            continue;
        }
        // Farmland: fertile flat ground just outside town.
        if town_dist[face_index] <= 3
            && matches!(
                tiles[face_index],
                Terrain::Plains | Terrain::Savanna | Terrain::Forest
            )
            && rng.f32() < 0.10
        {
            push(&mut rng, face_index, STRUCT_FARM);
            continue;
        }
        // Watchtower: high ground overlooking a road.
        if road_near(face_index) && terrain.elevation_at(grid.centroid(face_index)) > 0.25 && rng.f32() < 0.03 {
            push(&mut rng, face_index, STRUCT_WATCHTOWER);
            continue;
        }
        // Ruins: rare, deep in the wilderness (far from any town).
        if town_dist[face_index] == u16::MAX
            && !matches!(
                tiles[face_index],
                Terrain::Beach | Terrain::Cliff | Terrain::LakeShore | Terrain::RiverBank
            )
            && rng.f32() < 0.0006
        {
            push(&mut rng, face_index, STRUCT_RUIN);
        }
    }
    out
}

fn build_face_tags(grid: &Grid, painted: &Painted) -> (Vec<u32>, Vec<u8>) {
    let mut off = Vec::with_capacity(grid.face_count() + 1);
    let mut data = Vec::new();
    off.push(0u32);
    for face_index in 0..grid.face_count() {
        if face_solid(grid, &painted.roads, face_index) {
            data.push(TAG_ROAD);
        }
        if face_solid(grid, &painted.towns, face_index) {
            data.push(TAG_TOWN);
        }
        if face_solid(grid, &painted.bridges, face_index) {
            data.push(TAG_BRIDGE);
        }
        if face_solid(grid, &painted.bridge_entries, face_index) {
            data.push(TAG_BRIDGE_ENTRY);
        }
        off.push(data.len() as u32);
    }
    (off, data)
}

