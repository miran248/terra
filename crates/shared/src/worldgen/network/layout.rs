use std::collections::BTreeSet;

use crate::level::SettlementKind;
use terra_geometry::sphere::{SpherePos, ring_point};
use terra_geometry::topology::CellId;

use super::{Grid, router, safe_road_band, unsafe_road_edges};

/// Plan the complete internal street skeleton before any road paint is
/// committed. A `None` result lets site selection try another anchor.
pub(super) fn plan_settlement_roads(
    grid: &Grid,
    kind: SettlementKind,
    anchor: SpherePos,
    radius: f32,
    entrances: &[CellId],
    blocked: impl Fn(CellId) -> bool + Copy,
    extra: impl Fn(CellId) -> u64 + Copy,
) -> Option<Vec<Vec<CellId>>> {
    if entrances.len() < 2 {
        return None;
    }

    match kind {
        SettlementKind::Village => {
            let (from, to) = most_separated_pair(grid, entrances)?;
            let main = find_safe_layout_path(grid, from, to, blocked, extra, &BTreeSet::new())?;
            let mut paths = vec![main.clone()];
            let mut existing = main.iter().copied().collect::<BTreeSet<_>>();
            let mut branch_count = 0;
            for &entrance in entrances {
                if entrance == from || entrance == to {
                    continue;
                }
                let mut targets = main
                    .iter()
                    .copied()
                    .skip(1)
                    .take(main.len().saturating_sub(2))
                    .collect::<Vec<_>>();
                targets.sort_by(|&a, &b| {
                    grid.cell_position(a)
                        .distance(grid.cell_position(entrance))
                        .total_cmp(&grid.cell_position(b).distance(grid.cell_position(entrance)))
                        .then_with(|| a.cmp(&b))
                });
                let branch = targets.into_iter().find_map(|target| {
                    find_safe_layout_path(grid, target, entrance, blocked, extra, &existing)
                })?;
                existing.extend(branch.iter().copied());
                paths.push(branch);
                branch_count += 1;
            }
            (branch_count > 0).then_some(paths)
        }
        SettlementKind::Outpost => {
            let (from, to) = most_separated_pair(grid, entrances)?;
            let spine = find_safe_layout_path(grid, from, to, blocked, extra, &BTreeSet::new())?;
            let mut paths = vec![spine.clone()];
            let mut existing = spine.iter().copied().collect::<BTreeSet<_>>();
            for &entrance in entrances {
                if entrance == from || entrance == to {
                    continue;
                }
                let mut targets = spine.clone();
                targets.sort_by(|&a, &b| {
                    grid.cell_position(a)
                        .distance(grid.cell_position(entrance))
                        .total_cmp(&grid.cell_position(b).distance(grid.cell_position(entrance)))
                        .then_with(|| a.cmp(&b))
                });
                let branch = targets.into_iter().find_map(|target| {
                    find_safe_layout_path(grid, target, entrance, blocked, extra, &existing)
                })?;
                existing.extend(branch.iter().copied());
                paths.push(branch);
            }
            Some(paths)
        }
        SettlementKind::Town => {
            find_town_grid_paths(grid, anchor, radius, entrances, blocked, extra)
        }
    }
}

fn find_town_grid_paths(
    grid: &Grid,
    anchor: SpherePos,
    radius: f32,
    entrances: &[CellId],
    blocked: impl Fn(CellId) -> bool + Copy,
    extra: impl Fn(CellId) -> u64 + Copy,
) -> Option<Vec<Vec<CellId>>> {
    let roadable = grid
        .topology
        .cells()
        .filter(|&cell| !blocked(cell) && anchor.distance(grid.cell_position(cell)) <= radius + 6.0)
        .collect::<Vec<_>>();
    let quarter_turn = std::f32::consts::FRAC_PI_2;

    // Try a bounded deterministic set of compact grids. Reserve a four-sided
    // cycle and cross street before adding the external entrance spurs.
    for fraction in [0.32, 0.40, 0.48] {
        for rotation in 0..8 {
            let first_angle = rotation as f32 * std::f32::consts::FRAC_PI_4;
            let corners = (0..4)
                .map(|index| {
                    let angle = first_angle + index as f32 * quarter_turn;
                    let target = ring_point(anchor, radius * fraction, angle);
                    roadable.iter().copied().min_by(|&a, &b| {
                        grid.cell_position(a)
                            .distance(target)
                            .total_cmp(&grid.cell_position(b).distance(target))
                            .then_with(|| a.cmp(&b))
                    })
                })
                .collect::<Option<Vec<_>>>()?;
            if corners.iter().copied().collect::<BTreeSet<_>>().len() != 4 {
                continue;
            }

            let mut paths = Vec::new();
            let mut existing = BTreeSet::new();
            let mut viable = true;
            for index in 0..4 {
                let from = corners[index];
                let to = corners[(index + 1) % 4];
                let Some(path) = find_safe_layout_path(grid, from, to, blocked, extra, &existing)
                else {
                    viable = false;
                    break;
                };
                existing.extend(path.iter().copied());
                paths.push(path);
            }
            if !viable {
                continue;
            }

            let Some(cross_street) =
                find_safe_layout_path(grid, corners[0], corners[2], blocked, extra, &existing)
            else {
                continue;
            };
            if cross_street.len() < 3 {
                continue;
            }
            existing.extend(cross_street.iter().copied());
            paths.push(cross_street);

            for &entrance in entrances {
                let Some(target) = existing.iter().copied().min_by(|&a, &b| {
                    grid.cell_position(a)
                        .distance(grid.cell_position(entrance))
                        .total_cmp(&grid.cell_position(b).distance(grid.cell_position(entrance)))
                        .then_with(|| a.cmp(&b))
                }) else {
                    viable = false;
                    break;
                };
                let Some(spur) =
                    find_safe_layout_path(grid, target, entrance, blocked, extra, &existing)
                else {
                    viable = false;
                    break;
                };
                if spur.len() < 2 {
                    viable = false;
                    break;
                }
                existing.extend(spur.iter().copied());
                paths.push(spur);
            }
            if viable {
                return Some(paths);
            }
        }
    }
    None
}

fn find_safe_layout_path(
    grid: &Grid,
    from: CellId,
    to: CellId,
    blocked: impl Fn(CellId) -> bool + Copy,
    extra: impl Fn(CellId) -> u64 + Copy,
    existing: &BTreeSet<CellId>,
) -> Option<Vec<CellId>> {
    if from == to || blocked(from) || blocked(to) {
        return None;
    }
    let route_blocked =
        |cell| blocked(cell) || (existing.contains(&cell) && cell != from && cell != to);
    let unsafe_edges = unsafe_road_edges(grid, route_blocked);
    let mut path = router::lattice_path_with_edge(
        grid,
        route_blocked,
        extra,
        |a, b| unsafe_edges.contains(&(a, b)),
        from,
        to,
    );
    if path.len() < 2 {
        path = router::lattice_path_with_edge(
            grid,
            route_blocked,
            extra,
            |a, b| unsafe_edges.contains(&(a, b)),
            to,
            from,
        );
        path.reverse();
    }
    (path.len() >= 2 && safe_road_band(grid, &path, blocked).is_some()).then_some(path)
}

fn most_separated_pair(grid: &Grid, entrances: &[CellId]) -> Option<(CellId, CellId)> {
    entrances
        .iter()
        .copied()
        .flat_map(|from| {
            entrances
                .iter()
                .copied()
                .filter(move |&to| to > from)
                .map(move |to| (from, to))
        })
        .max_by(|(a_from, a_to), (b_from, b_to)| {
            grid.cell_position(*a_from)
                .distance(grid.cell_position(*a_to))
                .total_cmp(
                    &grid
                        .cell_position(*b_from)
                        .distance(grid.cell_position(*b_to)),
                )
                .then_with(|| (b_from, b_to).cmp(&(a_from, a_to)))
        })
}
