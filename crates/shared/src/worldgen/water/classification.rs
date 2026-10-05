use crate::level::{Landform, SlopeClass, WaterDepth};
use terra_geometry::sphere::SpherePos;
use crate::terrain::{Terrain, TerrainGen};
use terra_geometry::topology::CellId;

use super::super::{Grid, absorb_small_clusters, classification};
use super::normalization::cell_zone;

/// Macro landform per cell from the proposed field, with low ground boxed in
/// by higher ground classified as valleys and small clusters absorbed.
pub(in crate::worldgen) fn classify_landform(grid: &Grid, terrain: &TerrainGen) -> Vec<Landform> {
    let e: Vec<f32> = (0..grid.cell_count())
        .map(|cell_index| terrain.elevation_at(grid.cell_position(CellId::new(cell_index))))
        .collect();
    let mut lf = vec![Landform::Water; grid.cell_count()];
    for cell_index in 0..grid.cell_count() {
        if e[cell_index] < 0.0 {
            continue; // water
        }
        let relief = grid
            .cell_neighbors(CellId::new(cell_index))
            .iter()
            .map(|cell| cell.index())
            .map(|nb| (e[cell_index] - e[nb]).abs())
            .fold(0.0f32, f32::max);
        lf[cell_index] = if e[cell_index] >= 0.35 {
            if relief < 0.035 {
                Landform::Plateau
            } else {
                Landform::Mountains
            }
        } else if e[cell_index] >= 0.14 {
            Landform::Hills
        } else {
            Landform::Lowland
        };
    }
    // Valleys: low ground hemmed in by higher landform on most sides.
    let higher = |l: Landform| l.is_highland();
    let mut valleys = Vec::new();
    for cell_index in 0..grid.cell_count() {
        if lf[cell_index] == Landform::Lowland
            && grid
                .cell_neighbors(CellId::new(cell_index))
                .iter()
                .map(|cell| cell.index())
                .filter(|&nb| higher(lf[nb]))
                .count()
                >= 3
        {
            valleys.push(cell_index);
        }
    }
    for cell_index in valleys {
        lf[cell_index] = Landform::Valley;
    }
    // Absorb speckle: a landform cluster below the min joins its most common
    // land neighbor's landform, so each landform is a coherent region.
    absorb_small_landforms(grid, &mut lf);
    lf
}

pub(super) const MIN_LANDFORM_CELLS: usize = 25;

pub(super) fn absorb_small_landforms(grid: &Grid, lf: &mut [Landform]) {
    absorb_small_clusters(
        grid,
        lf,
        |l| l != Landform::Water,
        |_| MIN_LANDFORM_CELLS,
        |l| l != Landform::Water,
        Landform::rank,
    );
}

/// Cover per cell: the macro landform sets the base — high ground (mountains,
/// plateaus) gets rock/snow/ice/volcanic, everything lower gets a climate
/// biome. Water zones stay water. This is the landform → biome layering.
pub(in crate::worldgen) fn classify_cover(
    grid: &Grid,
    terrain: &TerrainGen,
    landform: &[Landform],
    cell_index: usize,
) -> Terrain {
    let pos = grid.cell_position(CellId::new(cell_index));
    let e = terrain.elevation_at(pos);
    match cell_zone(grid, terrain, cell_index) {
        crate::zones::ZoneKind::Ocean => Terrain::Ocean,
        crate::zones::ZoneKind::Lake => {
            if terrain.is_lake_bed(pos) {
                Terrain::Lake
            } else {
                land_cover(terrain, landform[cell_index], pos)
            }
        }
        _ => {
            if e < 0.0 {
                Terrain::Ocean
            } else {
                land_cover(terrain, landform[cell_index], pos)
            }
        }
    }
}

pub(super) fn land_cover(terrain: &TerrainGen, landform: Landform, pos: SpherePos) -> Terrain {
    let t = terrain.temperature_at(pos);
    let m = terrain.moisture_at(pos);
    let e = terrain.elevation_at(pos);
    // A mountain is not one thing: snow caps the high cells (above the snow
    // line), forest clothes the warm/wet lower flanks, bare rock fills the
    // rugged middle, ice covers the cold ranges, and hot high peaks are
    // volcanic. So a single range shows rock AND snow AND (sometimes) forest.
    if matches!(landform, Landform::Mountains | Landform::Plateau) {
        return if t < -12.0 {
            Terrain::Glacier
        } else if e > 0.70 {
            if t > 24.0 {
                Terrain::Volcanic
            } else {
                Terrain::Snow
            }
        } else if t < -2.0 {
            Terrain::Snow
        } else if m > 0.15 && e < 0.55 {
            Terrain::Forest
        } else {
            Terrain::Mountain
        };
    }
    // Low/rolling ground: climate biome cover.
    if t < -28.0 {
        return Terrain::Glacier;
    }
    if t < -15.0 {
        return Terrain::Snow;
    }
    if t < 0.0 {
        return Terrain::Tundra;
    }
    if e < 0.12 && m > 0.28 {
        return Terrain::Swamp;
    }
    if t > 24.0 && m > 0.25 {
        return Terrain::Jungle;
    }
    if t > 30.0 && m < -0.15 {
        return Terrain::Desert;
    }
    if t > 22.0 && m < 0.05 {
        return Terrain::Savanna;
    }
    if m > 0.10 {
        Terrain::Forest
    } else {
        Terrain::Plains
    }
}

/// Slope-class thresholds (rise/run ≈ tan angle) on the solved field.
pub(super) const SLOPE_GENTLE_MAX: f32 = 0.18; // ~10°: flat/gentle boundary
pub(super) const SLOPE_STEEP_MAX: f32 = 0.45; // ~24°: gentle/steep (walkable) boundary
pub(super) const SLOPE_CLIFF_MAX: f32 = 0.90; // ~42°: steep/cliff (impassable) boundary

/// Per-cell steepness of the SOLVED surface — the micro landform layer.
/// Measured at CELL scale (max rise/run to an edge-neighbor over the real
/// ground distance), not at a sub-metre probe, so it reflects terrain the
/// player traverses rather than interpolation noise. Passes (gentle cells in
/// mountains) and escarpments (cliff cells) fall out of it automatically.
pub(in crate::worldgen) fn classify_slope(grid: &Grid, terrain: &TerrainGen) -> Vec<SlopeClass> {
    let alt: Vec<f32> = (0..grid.cell_count())
        .map(|cell_index| terrain.altitude(grid.cell_position(CellId::new(cell_index))))
        .collect();
    (0..grid.cell_count())
        .map(|cell_index| {
            let a = grid.cell_direction(CellId::new(cell_index));
            let mut worst = 0.0f32;
            for nb in grid
                .cell_neighbors(CellId::new(cell_index))
                .iter()
                .map(|cell| cell.index())
            {
                let dist =
                    a.distance(grid.cell_direction(CellId::new(nb))) * terra_geometry::sphere::PLANET_RADIUS;
                if dist > 1.0 {
                    worst = worst.max((alt[cell_index] - alt[nb]).abs() / dist);
                }
            }

            match classification::bucket(
                worst,
                &[SLOPE_GENTLE_MAX, SLOPE_STEEP_MAX, SLOPE_CLIFF_MAX],
            ) {
                0 => SlopeClass::Flat,
                1 => SlopeClass::Gentle,
                2 => SlopeClass::Steep,
                _ => SlopeClass::Cliff,
            }
        })
        .collect()
}

/// The number of ascending `thresholds` a value reaches — turns a measurement
/// into an ordered class (flat/gentle/steep/cliff, shallow/deep/abyss).
/// Per-water-cell depth class from the solved surface: shore-shallows deepen
/// to abyss offshore (and lake/river beds shallow-to-deep by their concavity).
/// Land cells are WaterDepth::Shallow (unused). The depth analogue of slope class.
pub(in crate::worldgen) fn classify_water_depth(
    grid: &Grid,
    cells: &[Terrain],
    terrain: &TerrainGen,
) -> Vec<Option<WaterDepth>> {
    (0..grid.cell_count())
        .map(|cell_index| {
            if !cells[cell_index].is_water() {
                return None;
            }
            let e = terrain.elevation_at(grid.cell_position(CellId::new(cell_index)));
            Some(match classification::bucket(-e, &[0.20, 0.55]) {
                0 => WaterDepth::Shallow,
                1 => WaterDepth::Deep,
                _ => WaterDepth::Abyss,
            })
        })
        .collect()
}
