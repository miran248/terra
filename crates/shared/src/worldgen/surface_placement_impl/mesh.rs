use super::super::*;

pub(in crate::worldgen) fn face_road_material(
    grid: &Grid,
    cells: &[Terrain],
    landform: &[u8],
    slope_class: &[u8],
    face_index: usize,
) -> u8 {
    let mut sand = false;
    let mut rock = false;
    let mut soil = false;
    for cell_index in grid.face_cells(FaceId::new(face_index)).map(CellId::index) {
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
pub(in crate::worldgen) type TerrainTriangles = Vec<[[f32; 3]; 3]>;
pub(in crate::worldgen) type TerrainColors = Vec<[[f32; 4]; 3]>;

pub(in crate::worldgen) fn build_mesh(
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
        .map(|cell| terrain.render_radius(grid.cell_position(cell)))
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
        let idx = grid.face_cells(FaceId::new(face_index)).map(CellId::index);
        // Features are built structures: a face the feature OWNS (≥2 painted
        // corners — the two-triangle quads along the painted cell chain)
        // renders solid with a hard edge. Faces with exactly one painted
        // corner are the flank band and fade via the corner gradient. Total:
        // blend band / solid strip / blend band, for every feature.
        let corner = |k: usize| {
            let cell_index = idx[k];
            if painted.bridge_entries.contains(CellId::new(cell_index)) {
                entry_color
            } else if painted.towns.contains(CellId::new(cell_index)) {
                town_color
            } else if painted.roads.contains(CellId::new(cell_index)) {
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
            [road_mat_color(face_road_material(
                grid,
                cells,
                landform,
                slope_class,
                face_index,
            )); 3]
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
            (grid.cell_direction(CellId::new(idx[0])) * vert_r[idx[0]]).to_array(),
            (grid.cell_direction(CellId::new(idx[1])) * vert_r[idx[1]]).to_array(),
            (grid.cell_direction(CellId::new(idx[2])) * vert_r[idx[2]]).to_array(),
        ]);
        cols.push(color);
    }
    (tris, cols)
}
