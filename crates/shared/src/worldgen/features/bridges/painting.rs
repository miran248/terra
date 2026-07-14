/// Roads may not cross water: a planned path whose cell chain touches a water
/// cell is dropped entirely (crossing there needs a bridge, not a road).
pub(in crate::worldgen) fn paint_features(
    grid: &Grid,
    terrain: &TerrainGen,
    cells: &[Terrain],
    slope_class: &[SlopeClass],
) -> (Painted, Vec<Vec<SpherePos>>) {
    let mut painted = Painted::empty(grid.cell_count());
    let mut kept: Vec<Vec<SpherePos>> = Vec::new();
    // Roads route on WALKABLE ground: they follow valleys and mountain passes
    // and refuse water and steep slopes (conform, don't carve). A leg that has
    // no gentle dry route is dropped — that gap wants a bridge.
    // Roads may not cross water or a cliff, and are steered strongly toward
    // gentle ground (steep cells cost extra), so they follow valleys and
    // passes but can still climb a slope when they must.
    let blocked = |cell: crate::topology::CellId| {
        cells[cell.index()].is_water() || slope_class[cell.index()] == SlopeClass::Cliff
    };
    let extra = |cell: crate::topology::CellId| match slope_class[cell.index()] {
        SlopeClass::Flat => 0,
        SlopeClass::Gentle => 400,
        _ => 4000, // steep
    };
    'paths: for path in &terrain.road_paths {
        let mut chain: Vec<CellId> = Vec::new();
        let waypoints: Vec<CellId> = path.iter().filter_map(|p| nearest_cell(grid, *p)).collect();
        for leg in waypoints.windows(2) {
            let seg = router::lattice_path(grid, blocked, extra, leg[0], leg[1]);
            if seg.is_empty() {
                continue 'paths;
            }
            let skip = usize::from(chain.last() == seg.first());
            chain.extend(&seg[skip..]);
        }
        let band = features::widen_band(grid, &chain, false);
        if band.iter().any(|&cell| {
            cells[cell.index()].is_water() || slope_class[cell.index()] == SlopeClass::Cliff
        }) {
            continue;
        }
        for cell in band {
            painted.roads.insert(cell);
        }
        kept.push(path.clone());
    }
    // Towns sit on walkable ground within the settlement radius.
    for (cell_index, &slope) in slope_class.iter().enumerate().take(grid.cell_count()) {
        let pos = grid.cell_position(CellId::new(cell_index));
        if slope.is_walkable()
            && terrain
                .settlement_anchors
                .iter()
                .any(|a| a.distance(pos) <= TOWN_RADIUS)
        {
            painted.towns.insert(CellId::new(cell_index));
        }
    }
    link_feature_pinches(grid, &mut painted.roads, |cell| {
        cells[cell.index()].is_land()
    });
    link_feature_pinches(grid, &mut painted.towns, |_| true);
    (painted, kept)
}
use crate::level::SlopeClass;
use crate::sphere::SpherePos;
use crate::terrain::{Terrain, TerrainGen};
use crate::topology::CellId;
use crate::worldgen::{
    Grid, Painted, TOWN_RADIUS, features, link_feature_pinches, nearest_cell, router,
};
