/// Roads may only occupy flat or gentle ground outside cliff terrain.
pub(in crate::worldgen) fn paint_features(
    grid: &Grid,
    terrain: &TerrainGen,
    cells: &[Terrain],
    slope_class: &[SlopeClass],
) -> (Painted, Vec<RoadPath>) {
    let mut painted = Painted::empty(grid.cell_count());
    let mut kept: Vec<RoadPath> = Vec::new();
    // Roads conform to terrain: steep slopes, cliff terrain, and water are not
    // roadable, so a route with no suitable land path is left disconnected.
    let blocked =
        |cell: CellId| !features::roadable(cells[cell.index()], slope_class[cell.index()]);
    let extra = |cell: terra_geometry::topology::CellId| match slope_class[cell.index()] {
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
        if band.iter().any(|&cell| blocked(cell)) {
            continue;
        }
        for cell in band {
            painted.roads.insert(cell);
        }
        let settlement_at = |endpoint: CellId| {
            terrain
                .settlement_anchors
                .iter()
                .position(|&anchor| nearest_cell(grid, anchor) == Some(endpoint))
                .map(|index| index as u32)
        };
        kept.push(RoadPath {
            from_settlement: chain.first().copied().and_then(settlement_at),
            to_settlement: chain.last().copied().and_then(settlement_at),
            cells: chain,
            purpose: crate::worldgen::RoadPathPurpose::ExternalRoute,
        });
    }
    // Each settlement occupies its configured radius on walkable ground.
    for (cell_index, &slope) in slope_class.iter().enumerate().take(grid.cell_count()) {
        let pos = grid.cell_position(CellId::new(cell_index));
        if slope.is_walkable()
            && terrain
                .settlement_anchors
                .iter()
                .enumerate()
                .any(|(index, &anchor)| {
                    let radius = terrain
                        .settlement_config()
                        .radius_m(terrain.settlement_kind(index));
                    anchor.distance(pos) <= radius
                })
        {
            painted.settlements.insert(CellId::new(cell_index));
        }
    }
    link_feature_pinches(grid, &mut painted.roads, |cell| {
        features::roadable(cells[cell.index()], slope_class[cell.index()])
    });
    link_feature_pinches(grid, &mut painted.settlements, |_| true);
    (painted, kept)
}
use crate::level::SlopeClass;
use crate::terrain::{Terrain, TerrainGen};
use terra_geometry::topology::CellId;
use crate::worldgen::{
    Grid, Painted, RoadPath, features, link_feature_pinches, nearest_cell, router,
};
