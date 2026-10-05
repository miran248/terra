use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::level::{
    RoadData, RoadEndpointData, RoadEndpointRole, RoadKind, SettlementConfig, SettlementKind,
    SlopeClass, StructureKind,
};
use terra_geometry::sphere::{PLANET_RADIUS, SpherePos, ring_point};
use crate::terrain::{Terrain, TerrainGen};
use terra_geometry::topology::CellId;

use super::{
    CellSet, Grid, Painted, RoadPath, RoadPathPurpose, StructureSite, features,
    link_feature_pinches, nearest_cell, router,
};

mod layout;

#[derive(Clone, Copy)]
pub(super) struct StrategicRoadTarget {
    pub(super) position: SpherePos,
    pub(super) elevation_m: f32,
    pub(super) crossing: bool,
}

pub(super) fn outpost_site_score(
    site: SpherePos,
    site_elevation_m: f32,
    targets: &[StrategicRoadTarget],
) -> f32 {
    let (road_view, crossing_view) = targets.iter().fold((0.0f32, 0.0f32), |scores, target| {
        let distance = site.distance(target.position);
        let curvature_drop = distance * distance / (2.0 * PLANET_RADIUS);
        let visible_height = (site_elevation_m - target.elevation_m - curvature_drop).max(0.0);
        (
            scores.0.max(visible_height),
            scores
                .1
                .max(if target.crossing { visible_height } else { 0.0 }),
        )
    });
    road_view + crossing_view
}

fn strategic_road_targets(grid: &Grid, terrain: &TerrainGen) -> Vec<StrategicRoadTarget> {
    let mut road_membership = BTreeMap::<CellId, BTreeSet<usize>>::new();
    for (road_index, path) in terrain.road_paths.iter().enumerate() {
        let cells = path
            .iter()
            .filter_map(|&position| nearest_cell(grid, position))
            .collect::<BTreeSet<_>>();
        for cell in cells {
            road_membership.entry(cell).or_default().insert(road_index);
        }
    }
    road_membership
        .into_iter()
        .map(|(cell, roads)| {
            let position = grid.cell_position(cell);
            StrategicRoadTarget {
                position,
                elevation_m: terrain.surface_radius(position) - PLANET_RADIUS,
                crossing: roads.len() > 1,
            }
        })
        .collect()
}

pub(super) struct RoadGraph {
    pub connections: Vec<RoadData>,
    pub endpoints: Vec<RoadEndpointData>,
    /// Centerline cells for each `Road` connection, in connection order.
    pub road_cells: Vec<(u32, Vec<CellId>)>,
}

/// Keep every settlement within its planned zone, but move anchors whose town
/// footprint cannot hold a safe local road to the nearest viable zone face.
pub(super) fn select_settlement_sites(
    grid: &Grid,
    terrain: &TerrainGen,
    cells: &[Terrain],
    slopes: &[SlopeClass],
) -> Result<Vec<SpherePos>, String> {
    const MAX_COARSE_CANDIDATES: usize = 32;
    const MAX_FINE_CANDIDATES: usize = 64;
    let blocked = |cell: CellId| !features::roadable(cells[cell.index()], slopes[cell.index()]);
    let unsafe_edges = unsafe_road_edges(grid, blocked);
    let road_targets = strategic_road_targets(grid, terrain);
    terrain
        .zones()
        .zones_of_kind(crate::zones::ZoneKind::Settlement)
        .zip(&terrain.settlement_anchors)
        .enumerate()
        .map(|(settlement_index, ((zone_id, zone), &anchor))| {
            let radius = terrain
                .settlement_config()
                .radius_m(terrain.settlement_kind(settlement_index));
            let kind = terrain.settlement_kind(settlement_index);
            let compare_candidates = |a: &SpherePos, b: &SpherePos| {
                let strategy_order = if kind == SettlementKind::Outpost {
                    let a_height = terrain.surface_radius(*a) - PLANET_RADIUS;
                    let b_height = terrain.surface_radius(*b) - PLANET_RADIUS;
                    outpost_site_score(*b, b_height, &road_targets)
                        .total_cmp(&outpost_site_score(*a, a_height, &road_targets))
                        .then_with(|| b_height.total_cmp(&a_height))
                } else {
                    Ordering::Equal
                };
                strategy_order
                    .then_with(|| anchor.distance(*a).total_cmp(&anchor.distance(*b)))
                    .then_with(|| a.0.x.total_cmp(&b.0.x))
                    .then_with(|| a.0.y.total_cmp(&b.0.y))
                    .then_with(|| a.0.z.total_cmp(&b.0.z))
            };
            let mut candidates = zone
                .faces
                .iter()
                .map(|&face| SpherePos::new(terrain.zones().centroids[face as usize]))
                .collect::<Vec<_>>();
            candidates.sort_by(compare_candidates);
            let (coarse_site, coarse_attempts) = select_feasible_settlement_site(
                grid,
                terrain,
                cells,
                slopes,
                &candidates,
                kind,
                radius,
                MAX_COARSE_CANDIDATES,
                &unsafe_edges,
            );
            if let Some(site) = coarse_site {
                return Ok(site);
            }

            let mut fine_candidates = grid
                .topology
                .cells()
                .filter(|&cell| {
                    !blocked(cell)
                        && terrain.zone_id_at(grid.cell_position(cell)) == zone_id
                })
                .map(|cell| grid.cell_position(cell))
                .collect::<Vec<_>>();
            fine_candidates.sort_by(compare_candidates);
            let (fine_site, fine_attempts) = select_feasible_settlement_site(
                grid,
                terrain,
                cells,
                slopes,
                &fine_candidates,
                kind,
                radius,
                MAX_FINE_CANDIDATES,
                &unsafe_edges,
            );
            if let Some(site) = fine_site {
                return Ok(site);
            }
            Err(format!(
                "settlement {settlement_index} in zone {zone_id} found no feasible complete layout within the deterministic search budget, including a safe multi-edge road corridor inside a {radius} m footprint (tried {coarse_attempts} coarse and {fine_attempts} fine candidates)"
            ))
        })
        .collect()
}

#[allow(
    clippy::too_many_arguments,
    reason = "the bounded retry planner keeps candidate and terrain inputs explicit"
)]
pub(super) fn select_feasible_settlement_site(
    grid: &Grid,
    terrain: &TerrainGen,
    cells: &[Terrain],
    slopes: &[SlopeClass],
    candidates: &[SpherePos],
    kind: SettlementKind,
    radius: f32,
    max_attempts: usize,
    unsafe_edges: &BTreeSet<(CellId, CellId)>,
) -> (Option<SpherePos>, usize) {
    let mut attempts = 0;
    for &candidate in candidates {
        if local_road_diameter(grid, cells, slopes, candidate, radius, unsafe_edges) < 2 {
            continue;
        }
        if attempts >= max_attempts {
            break;
        }
        attempts += 1;
        if settlement_candidate_has_full_layout(
            grid, terrain, cells, slopes, candidate, kind, radius,
        ) {
            return (Some(candidate), attempts);
        }
    }
    (None, attempts)
}

fn settlement_candidate_has_full_layout(
    grid: &Grid,
    terrain: &TerrainGen,
    cells: &[Terrain],
    slopes: &[SlopeClass],
    anchor: SpherePos,
    kind: SettlementKind,
    radius: f32,
) -> bool {
    let mut config = terrain.settlement_config();
    config.towns = usize::from(kind == SettlementKind::Town);
    config.villages = usize::from(kind == SettlementKind::Village);
    config.outposts = usize::from(kind == SettlementKind::Outpost);
    let entrance_candidates =
        settlement_entrance_candidates(grid, cells, slopes, &[anchor], config);
    let Some(entrances) =
        select_layout_entrances(grid, &entrance_candidates[0], anchor, radius, kind)
    else {
        return false;
    };
    let blocked = |cell: CellId| {
        !features::roadable(cells[cell.index()], slopes[cell.index()])
            || anchor.distance(grid.cell_position(cell)) > radius + 6.0
    };
    let extra = |cell: CellId| match slopes[cell.index()] {
        SlopeClass::Flat => 0,
        SlopeClass::Gentle => 400,
        _ => 4000,
    };
    let Some(paths) =
        layout::plan_settlement_roads(grid, kind, anchor, radius, &entrances, blocked, extra)
    else {
        return false;
    };
    let mut road_cells = CellSet::new(grid.cell_count());
    let mut local_roads = Vec::with_capacity(paths.len());
    for path in paths {
        let Some(band) = safe_road_band(grid, &path, blocked) else {
            return false;
        };
        for cell in band {
            road_cells.insert(cell);
        }
        local_roads.push(RoadPath {
            cells: path,
            from_settlement: None,
            to_settlement: None,
            purpose: RoadPathPurpose::InternalLayout,
        });
    }
    plan_settlement_structures(
        grid,
        terrain,
        cells,
        slopes,
        &[anchor],
        config,
        &entrance_candidates,
        &road_cells,
        &local_roads,
    )
    .is_some()
}

/// Select a distinct perimeter entrance nearest the direction of an external
/// approach. Keeping endpoints on the settlement edge leaves its central
/// ground available for structures and internal streets.
pub(super) fn choose_settlement_entrance(
    grid: &Grid,
    candidates: &[CellId],
    anchor: SpherePos,
    target: SpherePos,
    radius_m: f32,
    used: &BTreeSet<CellId>,
) -> Option<CellId> {
    let (east, north) = anchor.tangent_basis();
    let toward_target = target.0 - anchor.0 * anchor.0.dot(target.0);
    let angle = if toward_target.length_squared() < 1e-8 {
        0.0
    } else {
        toward_target.dot(north).atan2(toward_target.dot(east))
    };
    let desired = ring_point(anchor, radius_m * 0.82, angle);
    let min_separation = (radius_m * 0.2).clamp(4.0, 9.0);
    let eligible = candidates
        .iter()
        .copied()
        .filter(|candidate| !used.contains(candidate))
        .filter(|&candidate| {
            used.iter().all(|&other| {
                grid.cell_position(candidate)
                    .distance(grid.cell_position(other))
                    >= min_separation
            })
        })
        .collect::<Vec<_>>();
    let perimeter = eligible
        .iter()
        .copied()
        .filter(|&candidate| anchor.distance(grid.cell_position(candidate)) >= radius_m * 0.65)
        .collect::<Vec<_>>();
    let pool = if perimeter.is_empty() {
        &eligible
    } else {
        &perimeter
    };

    pool.iter().copied().min_by(|&a, &b| {
        desired
            .distance(grid.cell_position(a))
            .total_cmp(&desired.distance(grid.cell_position(b)))
            .then_with(|| a.cmp(&b))
    })
}

fn local_road_diameter(
    grid: &Grid,
    cells: &[Terrain],
    slopes: &[SlopeClass],
    anchor: SpherePos,
    radius_m: f32,
    unsafe_edges: &BTreeSet<(CellId, CellId)>,
) -> usize {
    let candidates = grid
        .topology
        .cells()
        .filter(|&cell| anchor.distance(grid.cell_position(cell)) <= radius_m)
        .filter(|&cell| features::roadable(cells[cell.index()], slopes[cell.index()]))
        .collect::<BTreeSet<_>>();
    let mut assigned = BTreeSet::new();
    let mut diameter = 0;
    for &start in &candidates {
        if !assigned.insert(start) {
            continue;
        }
        let mut component = BTreeSet::from([start]);
        let mut queue = VecDeque::from([start]);
        while let Some(cell) = queue.pop_front() {
            for &neighbor in grid.cell_neighbors(cell) {
                if candidates.contains(&neighbor)
                    && !assigned.contains(&neighbor)
                    && !unsafe_edges.contains(&(cell, neighbor))
                {
                    assigned.insert(neighbor);
                    component.insert(neighbor);
                    queue.push_back(neighbor);
                }
            }
        }
        for &source in &component {
            let mut distances = BTreeMap::from([(source, 0usize)]);
            let mut queue = VecDeque::from([source]);
            while let Some(cell) = queue.pop_front() {
                for &neighbor in grid.cell_neighbors(cell) {
                    if component.contains(&neighbor) && !distances.contains_key(&neighbor) {
                        distances.insert(neighbor, distances[&cell] + 1);
                        queue.push_back(neighbor);
                    }
                }
            }
            diameter = diameter.max(distances.values().copied().max().unwrap_or(0));
            if diameter >= 2 {
                return diameter;
            }
        }
    }
    diameter
}

/// Join every settlement in each roadable land group. Existing planned paths
/// seed the graph; shortest straight-line settlement pairs guide an MST-like
/// sequence of routable lattice paths between any remaining components.
#[allow(
    clippy::too_many_arguments,
    reason = "this generation stage combines terrain inputs with separate mutable outputs"
)]
pub(super) fn connect_settlements(
    grid: &Grid,
    terrain: &TerrainGen,
    cells: &[Terrain],
    slopes: &[SlopeClass],
    settlements: &[SpherePos],
    settlement_config: SettlementConfig,
    painted: &mut Painted,
    roads: &mut Vec<RoadPath>,
) -> (Vec<Option<CellId>>, Vec<Vec<CellId>>, Vec<StructureSite>) {
    let blocked = |cell: CellId| !features::roadable(cells[cell.index()], slopes[cell.index()]);
    let extra = |cell: CellId| match slopes[cell.index()] {
        SlopeClass::Flat => 0,
        SlopeClass::Gentle => 400,
        _ => 4000,
    };
    let entrance_candidates =
        settlement_entrance_candidates(grid, cells, slopes, settlements, settlement_config);
    roads.clear();
    painted.roads = CellSet::new(grid.cell_count());
    let mut components = CellComponents::new(grid.cell_count());
    let mut used_entrances = vec![BTreeSet::new(); settlements.len()];
    let mut entrances_by_settlement = vec![Vec::new(); settlements.len()];
    for settlement_index in 0..settlements.len() {
        let kind = settlement_config
            .kind_at(settlement_index)
            .expect("settlement index is configured");
        let radius = settlement_config.radius_m(kind);
        let anchor = settlements[settlement_index];
        entrances_by_settlement[settlement_index] = select_layout_entrances(
            grid,
            &entrance_candidates[settlement_index],
            anchor,
            radius,
            kind,
        )
        .unwrap_or_else(|| {
            panic!("settlement {settlement_index} has too few safe distinct entrances")
        });
        let local_blocked = |cell: CellId| {
            blocked(cell) || anchor.distance(grid.cell_position(cell)) > radius + 6.0
        };
        let layout_paths = layout::plan_settlement_roads(
            grid,
            kind,
            anchor,
            radius,
            &entrances_by_settlement[settlement_index],
            local_blocked,
            extra,
        )
        .unwrap_or_else(|| {
            panic!("settlement {settlement_index} has no safe internal road layout")
        });
        for path in layout_paths {
            commit_layout_path(grid, &path, local_blocked, &mut components, painted, roads);
        }
    }
    let settlement_structures = plan_settlement_structures(
        grid,
        terrain,
        cells,
        slopes,
        settlements,
        settlement_config,
        &entrance_candidates,
        &painted.roads,
        roads,
    )
    .unwrap_or_else(|| panic!("settlements have no collision-free required building layout"));
    let structure_obstacles = structure_obstacle_cells(grid, &settlement_structures);
    let route_blocked = |cell: CellId| blocked(cell) || structure_obstacles.contains(&cell);
    let route_unsafe_edges = unsafe_road_edges(grid, route_blocked);
    let mut route_land_groups = RoadableGroups::new(grid, route_blocked, &route_unsafe_edges);
    let eligible_entrances = entrances_by_settlement
        .iter()
        .enumerate()
        .map(|(settlement_index, candidates)| {
            let kind = settlement_config
                .kind_at(settlement_index)
                .expect("settlement index is configured");
            connected_entrance_candidates(
                grid,
                cells,
                slopes,
                settlements[settlement_index],
                settlement_config.radius_m(kind),
                candidates,
                &settlement_structures,
            )
        })
        .collect::<Vec<_>>();
    let centers = entrances_by_settlement
        .iter()
        .enumerate()
        .map(|(index, _)| eligible_entrances[index].first().copied())
        .collect::<Vec<_>>();
    let mut pairs = Vec::new();
    for (from_index, from) in centers.iter().enumerate() {
        let Some(from) = *from else { continue };
        let Some(group) = route_land_groups.cell(from) else {
            continue;
        };
        for (to_index, to) in centers.iter().enumerate().skip(from_index + 1) {
            let Some(to) = *to else { continue };
            if route_land_groups.cell(to) == Some(group) {
                pairs.push((
                    settlements[from_index].distance(settlements[to_index]),
                    from_index as u32,
                    to_index as u32,
                ));
            }
        }
    }
    pairs.sort_by(|a, b| {
        a.0.total_cmp(&b.0)
            .then_with(|| (a.1, a.2).cmp(&(b.1, b.2)))
    });

    // Rebuild surface routes from distinct perimeter entrances. The first
    // terrain pass routes between settlement anchors to guide elevation, but
    // those center-to-center paths would consume the very ground used by each
    // settlement's internal streets and structures.
    for (_, from_settlement, to_settlement) in pairs {
        let from_index = from_settlement as usize;
        let to_index = to_settlement as usize;
        let radius_from = settlement_config.radius_m(
            settlement_config
                .kind_at(from_index)
                .expect("settlement index must be configured"),
        );
        let radius_to = settlement_config.radius_m(
            settlement_config
                .kind_at(to_index)
                .expect("settlement index must be configured"),
        );
        let Some(from_entrance) = choose_settlement_entrance(
            grid,
            &eligible_entrances[from_index],
            settlements[from_index],
            settlements[to_index],
            radius_from,
            &used_entrances[from_index],
        ) else {
            continue;
        };
        let Some(to_entrance) = choose_settlement_entrance(
            grid,
            &eligible_entrances[to_index],
            settlements[to_index],
            settlements[from_index],
            radius_to,
            &used_entrances[to_index],
        ) else {
            continue;
        };
        if components.same(from_entrance, to_entrance) {
            continue;
        }
        let route_extra = |cell: CellId| {
            let base = match slopes[cell.index()] {
                SlopeClass::Flat => 0,
                SlopeClass::Gentle => 400,
                _ => 4000,
            };
            let pos = grid.cell_position(cell);
            let crosses_other_settlement = settlements
                .iter()
                .enumerate()
                .filter(|(index, _)| *index != from_index && *index != to_index)
                .any(|(index, &anchor)| {
                    anchor.distance(pos)
                        <= settlement_config.radius_m(
                            settlement_config
                                .kind_at(index)
                                .expect("settlement index must be configured"),
                        )
                });
            base + if crosses_other_settlement { 50_000 } else { 0 }
        };
        let mut path = router::lattice_path_with_edge(
            grid,
            route_blocked,
            route_extra,
            |from, to| route_unsafe_edges.contains(&(from, to)),
            from_entrance,
            to_entrance,
        );
        let mut reversed = false;
        if path.len() < 2 {
            path = router::lattice_path_with_edge(
                grid,
                route_blocked,
                route_extra,
                |from, to| route_unsafe_edges.contains(&(from, to)),
                to_entrance,
                from_entrance,
            );
            reversed = true;
        }
        if path.len() < 2 {
            continue;
        }
        let Some(band) = safe_road_band(grid, &path, route_blocked) else {
            continue;
        };
        components.connect_path(&path);
        for cell in band {
            painted.roads.insert(cell);
        }
        roads.push(RoadPath {
            cells: path,
            from_settlement: Some(if reversed {
                to_settlement
            } else {
                from_settlement
            }),
            to_settlement: Some(if reversed {
                from_settlement
            } else {
                to_settlement
            }),
            purpose: RoadPathPurpose::ExternalRoute,
        });
        used_entrances[from_index].insert(from_entrance);
        used_entrances[to_index].insert(to_entrance);
    }
    link_feature_pinches(grid, &mut painted.roads, |cell| !route_blocked(cell));
    let graph_entrances = entrances_by_settlement
        .iter()
        .map(|entrances| entrances.first().copied())
        .collect();
    (
        graph_entrances,
        entrances_by_settlement,
        settlement_structures,
    )
}

impl RoadGraph {
    pub(super) fn empty() -> Self {
        Self {
            connections: Vec::new(),
            endpoints: Vec::new(),
            road_cells: Vec::new(),
        }
    }
}

/// Connect each candidate bridge entrance to an existing local road network
/// before assigning road identities. A bridge is kept only when both banks
/// can reach roads or a settlement on roadable ground.
#[allow(
    clippy::too_many_arguments,
    reason = "bridge planning has distinct terrain, endpoint, and transactional paint inputs"
)]
pub(super) fn connect_bridges(
    grid: &Grid,
    cells: &[Terrain],
    slopes: &[SlopeClass],
    settlements: &[SpherePos],
    settlement_config: SettlementConfig,
    layout_entrances: &[Vec<CellId>],
    settlement_structures: &[StructureSite],
    candidates: Vec<Vec<SpherePos>>,
    painted: &mut Painted,
    roads: &mut Vec<RoadPath>,
) -> Vec<Vec<SpherePos>> {
    let structure_obstacles = structure_obstacle_cells(grid, settlement_structures);
    let blocked = |cell: CellId| {
        !features::roadable(cells[cell.index()], slopes[cell.index()])
            || structure_obstacles.contains(&cell)
    };
    let extra = |cell: CellId| match slopes[cell.index()] {
        SlopeClass::Flat => 0,
        SlopeClass::Gentle => 400,
        _ => 4000,
    };
    let settlement_at = |cell: CellId| {
        let position = grid.cell_position(cell);
        settlements.iter().enumerate().find_map(|(index, &anchor)| {
            let kind = settlement_config
                .kind_at(index)
                .expect("settlement index must be configured");
            (anchor.distance(position) <= settlement_config.radius_m(kind)).then_some(index as u32)
        })
    };
    let mut used_entrances = used_settlement_entrances(grid, roads, settlements, settlement_config);
    let is_settlement_endpoint = |road: &RoadPath, cell: CellId| {
        (road.cells.first() == Some(&cell) && road.from_settlement.is_some())
            || (road.cells.last() == Some(&cell) && road.to_settlement.is_some())
    };
    let unsafe_edges = unsafe_road_edges(grid, blocked);
    let mut roadable_groups = RoadableGroups::new(grid, blocked, &unsafe_edges);
    let mut bridges = Vec::new();

    for span in candidates {
        let Some(&from_pos) = span.first() else {
            continue;
        };
        let Some(&to_pos) = span.last() else {
            continue;
        };
        let (Some(from), Some(to)) = (nearest_cell(grid, from_pos), nearest_cell(grid, to_pos))
        else {
            continue;
        };
        if blocked(from) || blocked(to) {
            continue;
        }
        let mut entry_cells = BTreeSet::new();
        let mut valid_entries = true;
        for end in [span.first(), span.last()].into_iter().flatten() {
            let Some(face_index) = grid.planet.face_at(end.0) else {
                valid_entries = false;
                break;
            };
            for cell in grid.face_cells(terra_geometry::topology::FaceId::new(face_index)) {
                if cells[cell.index()].is_land() {
                    if structure_obstacles.contains(&cell) {
                        valid_entries = false;
                        break;
                    }
                    entry_cells.insert(cell);
                }
            }
            if !valid_entries {
                break;
            }
        }
        if !valid_entries {
            continue;
        }
        let bridge_cells = crate::worldgen::cell_chain(grid, &span);
        if bridge_cells
            .iter()
            .any(|&cell| structure_obstacles.contains(&cell))
        {
            continue;
        }
        let mut staged = Vec::new();
        let mut staged_bands = Vec::new();
        let mut staged_used_entrances = used_entrances.clone();
        let mut connectable = true;
        for entrance in [from, to] {
            let Some(entrance_component) = roadable_groups.cell(entrance) else {
                connectable = false;
                break;
            };
            let has_external_road_access = roads.iter().chain(&staged).any(|road: &RoadPath| {
                road.purpose != RoadPathPurpose::InternalLayout
                    && road.cells.contains(&entrance)
                    && (settlement_at(entrance).is_none() || is_settlement_endpoint(road, entrance))
            });
            let bank_settlement = settlement_at(entrance);
            if bank_settlement.is_none() && has_external_road_access {
                continue;
            }
            let mut targets = BTreeMap::new();
            if let Some(index) = bank_settlement {
                let index = index as usize;
                let kind = settlement_config
                    .kind_at(index)
                    .expect("settlement index must be configured");
                let anchor = settlements[index];
                let radius = settlement_config.radius_m(kind);
                let candidates = layout_entrances
                    .get(index)
                    .into_iter()
                    .flatten()
                    .copied()
                    .filter(|&cell| {
                        !blocked(cell) && roadable_groups.cell(cell) == Some(entrance_component)
                    })
                    .collect::<Vec<_>>();
                let mut unavailable = staged_used_entrances[index].clone();
                unavailable.insert(entrance);
                let Some(new_entrance) = choose_settlement_entrance(
                    grid,
                    &candidates,
                    anchor,
                    grid.cell_position(entrance),
                    radius,
                    &unavailable,
                ) else {
                    connectable = false;
                    break;
                };
                targets.insert(new_entrance, Some(index as u32));
            } else {
                for road in roads.iter().chain(&staged) {
                    if road.purpose == RoadPathPurpose::InternalLayout {
                        continue;
                    }
                    for &cell in &road.cells {
                        if settlement_at(cell).is_none() {
                            targets.entry(cell).or_insert(None);
                        }
                    }
                    if let (Some(&cell), Some(index)) = (road.cells.first(), road.from_settlement)
                        && !staged_used_entrances[index as usize].contains(&cell)
                    {
                        targets.insert(cell, Some(index));
                    }
                    if let (Some(&cell), Some(index)) = (road.cells.last(), road.to_settlement)
                        && !staged_used_entrances[index as usize].contains(&cell)
                    {
                        targets.insert(cell, Some(index));
                    }
                }
                for (index, candidates) in layout_entrances.iter().enumerate() {
                    let kind = settlement_config
                        .kind_at(index)
                        .expect("settlement index must be configured");
                    let anchor = settlements[index];
                    let radius = settlement_config.radius_m(kind);
                    let candidates = candidates
                        .iter()
                        .copied()
                        .filter(|&cell| {
                            !blocked(cell) && roadable_groups.cell(cell) == Some(entrance_component)
                        })
                        .collect::<Vec<_>>();
                    if let Some(new_entrance) = choose_settlement_entrance(
                        grid,
                        &candidates,
                        anchor,
                        grid.cell_position(entrance),
                        radius,
                        &staged_used_entrances[index],
                    ) {
                        targets.insert(new_entrance, Some(index as u32));
                    }
                }
            }
            let target_cells = targets
                .keys()
                .copied()
                .filter(|&cell| {
                    cell != entrance && roadable_groups.cell(cell) == Some(entrance_component)
                })
                .collect::<Vec<_>>();
            let connection = router::lattice_path_to_any_with_edge(
                grid,
                blocked,
                extra,
                |from, to| unsafe_edges.contains(&(from, to)),
                entrance,
                &target_cells,
            )
            .and_then(|(chain, target)| {
                if chain.len() < 2 {
                    return None;
                }
                let band = safe_road_band(grid, &chain, blocked)?;
                Some((chain, band, targets[&target]))
            });
            if let Some((chain, band, settlement_index)) = connection {
                if let Some(index) = settlement_index {
                    let entrance = *chain.last().expect("connection path is not empty");
                    staged_used_entrances[index as usize].insert(entrance);
                }
                staged.push(RoadPath {
                    cells: chain,
                    from_settlement: None,
                    to_settlement: settlement_index,
                    purpose: RoadPathPurpose::BridgeApproach,
                });
                staged_bands.push(band);
            } else {
                connectable = false;
                break;
            }
        }
        if connectable {
            roads.extend(staged);
            used_entrances = staged_used_entrances;
            for band in staged_bands {
                for cell in band {
                    painted.roads.insert(cell);
                }
            }
            for cell in bridge_cells {
                painted.bridges.insert(cell);
            }
            for cell in entry_cells {
                painted.bridge_entries.insert(cell);
            }
            bridges.push(span);
        }
    }
    link_feature_pinches(grid, &mut painted.roads, |cell| !blocked(cell));
    bridges
}

fn used_settlement_entrances(
    grid: &Grid,
    roads: &[RoadPath],
    settlements: &[SpherePos],
    settlement_config: SettlementConfig,
) -> Vec<BTreeSet<CellId>> {
    let mut used = vec![BTreeSet::new(); settlements.len()];
    for road in roads
        .iter()
        .filter(|road| road.purpose != RoadPathPurpose::InternalLayout)
    {
        for (settlement_index, &settlement) in settlements.iter().enumerate() {
            let radius = settlement_config.radius_m(
                settlement_config
                    .kind_at(settlement_index)
                    .expect("settlement index must be configured"),
            );
            let mut run_start = None;
            for (path_index, &cell) in road.cells.iter().enumerate() {
                let inside = settlement.distance(grid.cell_position(cell)) <= radius;
                match (run_start, inside) {
                    (None, true) => run_start = Some(path_index),
                    (Some(start), false) => {
                        used[settlement_index].insert(road.cells[start]);
                        used[settlement_index].insert(road.cells[path_index - 1]);
                        run_start = None;
                    }
                    _ => {}
                }
            }
            if let Some(start) = run_start {
                used[settlement_index].insert(road.cells[start]);
                if let Some(&last) = road.cells.last() {
                    used[settlement_index].insert(last);
                }
            }
        }
        if let (Some(&cell), Some(index)) = (road.cells.first(), road.from_settlement) {
            used[index as usize].insert(cell);
        }
        if let (Some(&cell), Some(index)) = (road.cells.last(), road.to_settlement) {
            used[index as usize].insert(cell);
        }
    }
    used
}

/// Pick a roadable cell in each painted settlement footprint for its entrance.
fn settlement_entrance_candidates(
    grid: &Grid,
    cells: &[Terrain],
    slopes: &[SlopeClass],
    settlements: &[SpherePos],
    settlement_config: SettlementConfig,
) -> Vec<Vec<CellId>> {
    settlements
        .iter()
        .enumerate()
        .map(|(index, &settlement)| {
            let radius = settlement_config.radius_m(
                settlement_config
                    .kind_at(index)
                    .expect("settlement index must be configured"),
            );
            let mut candidates = grid
                .topology
                .cells()
                .filter(|&cell| {
                    settlement.distance(grid.cell_position(cell)) <= radius
                        && features::roadable(cells[cell.index()], slopes[cell.index()])
                })
                .collect::<Vec<_>>();
            candidates.sort_by(|&a, &b| {
                settlement
                    .distance(grid.cell_position(a))
                    .total_cmp(&settlement.distance(grid.cell_position(b)))
                    .then_with(|| a.cmp(&b))
            });
            candidates
        })
        .collect()
}

const STRUCTURE_SITE_SAMPLES: [[f32; 3]; 7] = [
    [1.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0],
    [0.65, 0.175, 0.175],
    [0.175, 0.65, 0.175],
    [0.175, 0.175, 0.65],
    [0.1, 0.45, 0.45],
    [0.45, 0.1, 0.45],
    [0.45, 0.45, 0.1],
];

#[derive(Clone, Copy)]
struct StructureCandidate {
    site: StructureSite,
    priority: f32,
}

fn required_structures(kind: SettlementKind) -> &'static [StructureKind] {
    match kind {
        SettlementKind::Town => &[
            StructureKind::Well,
            StructureKind::House,
            StructureKind::House,
            StructureKind::House,
            StructureKind::House,
        ],
        SettlementKind::Village => &[
            StructureKind::Farm,
            StructureKind::House,
            StructureKind::House,
        ],
        SettlementKind::Outpost => &[
            StructureKind::Watchtower,
            StructureKind::Tent,
            StructureKind::Barricade,
            StructureKind::Barricade,
        ],
    }
}

fn required_entrances(kind: SettlementKind) -> usize {
    match kind {
        SettlementKind::Town => 4,
        SettlementKind::Village => 3,
        SettlementKind::Outpost => 2,
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "the packing planner keeps reserved roads and terrain constraints explicit"
)]
fn plan_settlement_structures(
    grid: &Grid,
    terrain: &TerrainGen,
    cells: &[Terrain],
    slopes: &[SlopeClass],
    settlements: &[SpherePos],
    settlement_config: SettlementConfig,
    entrance_candidates: &[Vec<CellId>],
    layout_road_cells: &CellSet,
    local_roads: &[RoadPath],
) -> Option<Vec<StructureSite>> {
    let mut sites = Vec::new();
    let mut occupied = Vec::<(SpherePos, StructureKind)>::new();
    for (settlement_index, &anchor) in settlements.iter().enumerate() {
        let kind = settlement_config
            .kind_at(settlement_index)
            .expect("settlement index is configured");
        let radius = settlement_config.radius_m(kind);
        let required = required_structures(kind);
        let mut local_road_graph = BTreeMap::<CellId, BTreeSet<CellId>>::new();
        for road in local_roads {
            if road.from_settlement.is_some() || road.to_settlement.is_some() {
                continue;
            }
            for pair in road.cells.windows(2) {
                if anchor.distance(grid.cell_position(pair[0])) <= radius + 6.0
                    && anchor.distance(grid.cell_position(pair[1])) <= radius + 6.0
                {
                    local_road_graph.entry(pair[0]).or_default().insert(pair[1]);
                    local_road_graph.entry(pair[1]).or_default().insert(pair[0]);
                }
            }
        }
        let mut candidate_sets = Vec::with_capacity(required.len());

        for &structure_kind in required {
            let footprint = super::surface::structure_footprint_radius(structure_kind);
            let max_distance = radius - footprint - 1.0;
            let mut candidates = Vec::new();
            for face_index in 0..grid.face_count() {
                let face = terra_geometry::topology::FaceId::new(face_index);
                if !grid
                    .face_cells(face)
                    .into_iter()
                    .all(|cell| features::roadable(cells[cell.index()], slopes[cell.index()]))
                    || anchor.distance(grid.centroid(face)) > max_distance + 6.0
                {
                    continue;
                }
                let corners = grid
                    .face_cells(face)
                    .into_iter()
                    .map(|cell| {
                        grid.cell_direction(cell) * terrain.render_radius(grid.cell_position(cell))
                    })
                    .collect::<Vec<_>>();
                for (sample_index, barycentric) in
                    STRUCTURE_SITE_SAMPLES.iter().copied().enumerate()
                {
                    let position = SpherePos::new(
                        corners[0] * barycentric[0]
                            + corners[1] * barycentric[1]
                            + corners[2] * barycentric[2],
                    );
                    let anchor_distance = anchor.distance(position);
                    if anchor_distance > max_distance
                        || !footprint_is_roadable(
                            grid,
                            cells,
                            slopes,
                            layout_road_cells,
                            face,
                            position,
                            footprint,
                        )
                    {
                        continue;
                    }
                    let target_ring = radius
                        * if kind == SettlementKind::Town {
                            0.68
                        } else {
                            0.52
                        };
                    let radial_fit = -(anchor_distance - target_ring).abs();
                    let watchtower_view = if kind == SettlementKind::Outpost
                        && structure_kind == StructureKind::Watchtower
                    {
                        let tower_elevation = terrain.elevation_at(position);
                        let road_view = local_road_graph
                            .iter()
                            .map(|(&cell, neighbors)| {
                                let road_position = grid.cell_position(cell);
                                let distance = position.distance(road_position);
                                let relative_elevation = (tower_elevation
                                    - terrain.elevation_at(road_position))
                                .max(0.0);
                                let horizon =
                                    (relative_elevation + 0.01) * 300.0 / (distance + 8.0);
                                let crossing = if neighbors.len() >= 3 { 1.0 } else { 0.0 };
                                horizon + crossing
                                    - (distance - (radius * 0.6).clamp(8.0, 16.0)).abs() * 0.02
                            })
                            .fold(0.0f32, f32::max);
                        tower_elevation * 10.0 + road_view
                    } else {
                        0.0
                    };
                    candidates.push(StructureCandidate {
                        site: StructureSite {
                            face_index,
                            barycentric,
                            position: position.0,
                            kind: structure_kind,
                        },
                        priority: watchtower_view + radial_fit + sample_index as f32 * 1e-5,
                    });
                }
            }
            candidates.sort_by(|a, b| {
                b.priority
                    .total_cmp(&a.priority)
                    .then_with(|| a.site.face_index.cmp(&b.site.face_index))
                    .then_with(|| a.site.barycentric[0].total_cmp(&b.site.barycentric[0]))
                    .then_with(|| a.site.barycentric[1].total_cmp(&b.site.barycentric[1]))
                    .then_with(|| a.site.barycentric[2].total_cmp(&b.site.barycentric[2]))
            });
            candidates.truncate(192);
            if candidates.is_empty() {
                return None;
            }
            candidate_sets.push(candidates);
        }

        let mut planned = Vec::with_capacity(required.len());
        let mut search_budget = 50_000usize;
        if !assign_structure_plan(
            0,
            required,
            &candidate_sets,
            grid,
            cells,
            slopes,
            anchor,
            radius,
            &entrance_candidates[settlement_index],
            required_entrances(kind),
            &mut occupied,
            &mut planned,
            &mut search_budget,
        ) {
            return None;
        }
        sites.extend(planned);
    }
    Some(sites)
}

fn footprint_is_roadable(
    grid: &Grid,
    cells: &[Terrain],
    slopes: &[SlopeClass],
    layout_road_cells: &CellSet,
    face: terra_geometry::topology::FaceId,
    center: SpherePos,
    footprint_radius: f32,
) -> bool {
    const CELL_MARGIN_M: f32 = 5.0;
    let search_radius = footprint_radius + CELL_MARGIN_M + 14.0;
    let mut visited = BTreeSet::new();
    let mut queue = VecDeque::new();
    for cell in grid.face_cells(face) {
        visited.insert(cell);
        queue.push_back(cell);
    }
    while let Some(cell) = queue.pop_front() {
        let distance = center.distance(grid.cell_position(cell));
        if distance <= footprint_radius + CELL_MARGIN_M
            && !features::roadable(cells[cell.index()], slopes[cell.index()])
        {
            return false;
        }
        if distance <= footprint_radius + 6.0 && layout_road_cells.contains(cell) {
            return false;
        }
        if distance <= search_radius {
            for &neighbor in grid.cell_neighbors(cell) {
                if visited.insert(neighbor) {
                    queue.push_back(neighbor);
                }
            }
        }
    }
    true
}

#[allow(
    clippy::too_many_arguments,
    reason = "recursive packing passes reservations explicitly so failed branches unwind"
)]
fn assign_structure_plan(
    index: usize,
    required: &[StructureKind],
    candidates: &[Vec<StructureCandidate>],
    grid: &Grid,
    cells: &[Terrain],
    slopes: &[SlopeClass],
    anchor: SpherePos,
    radius: f32,
    entrance_candidates: &[CellId],
    minimum_entrances: usize,
    occupied: &mut Vec<(SpherePos, StructureKind)>,
    planned: &mut Vec<StructureSite>,
    search_budget: &mut usize,
) -> bool {
    if index == required.len() {
        let obstacles = structure_obstacle_cells(grid, planned);
        let blocked = |cell: CellId| {
            !features::roadable(cells[cell.index()], slopes[cell.index()])
                || obstacles.contains(&cell)
                || anchor.distance(grid.cell_position(cell)) > radius + 6.0
        };
        let unsafe_edges = unsafe_road_edges(grid, blocked);
        let mut groups = RoadableGroups::new(grid, blocked, &unsafe_edges);
        let mut counts = BTreeMap::<usize, usize>::new();
        for &cell in entrance_candidates {
            if !blocked(cell)
                && let Some(group) = groups.cell(cell)
            {
                *counts.entry(group).or_default() += 1;
            }
        }
        return counts.values().copied().max().unwrap_or(0) >= minimum_entrances;
    }

    let kind = required[index];
    for candidate in &candidates[index] {
        if *search_budget == 0 {
            return false;
        }
        *search_budget -= 1;
        let position = SpherePos::new(candidate.site.position);
        let separated = occupied.iter().all(|(other_position, other_kind)| {
            position.distance(*other_position)
                >= super::surface::structure_footprint_radius(kind)
                    + 1.0
                    + super::surface::structure_footprint_radius(*other_kind)
        });
        if !separated {
            continue;
        }
        if index > 0
            && required[index - 1] == kind
            && let Some(previous) = planned.last()
            && (candidate.site.face_index, candidate.site.barycentric)
                <= (previous.face_index, previous.barycentric)
        {
            continue;
        }

        occupied.push((position, kind));
        planned.push(candidate.site);
        if assign_structure_plan(
            index + 1,
            required,
            candidates,
            grid,
            cells,
            slopes,
            anchor,
            radius,
            entrance_candidates,
            minimum_entrances,
            occupied,
            planned,
            search_budget,
        ) {
            return true;
        }
        planned.pop();
        occupied.pop();
    }
    false
}

fn connected_entrance_candidates(
    grid: &Grid,
    cells: &[Terrain],
    slopes: &[SlopeClass],
    anchor: SpherePos,
    radius: f32,
    candidates: &[CellId],
    sites: &[StructureSite],
) -> Vec<CellId> {
    let obstacles = structure_obstacle_cells(grid, sites);
    let blocked = |cell: CellId| {
        !features::roadable(cells[cell.index()], slopes[cell.index()])
            || obstacles.contains(&cell)
            || anchor.distance(grid.cell_position(cell)) > radius + 6.0
    };
    let unsafe_edges = unsafe_road_edges(grid, blocked);
    let mut groups = RoadableGroups::new(grid, blocked, &unsafe_edges);
    let mut group_counts = BTreeMap::<usize, (usize, CellId)>::new();
    for &cell in candidates {
        if blocked(cell) {
            continue;
        }
        let Some(group) = groups.cell(cell) else {
            continue;
        };
        group_counts
            .entry(group)
            .and_modify(|(count, first)| {
                *count += 1;
                *first = (*first).min(cell);
            })
            .or_insert((1, cell));
    }
    let Some((&selected_group, _)) = group_counts.iter().max_by(|(a_group, a), (b_group, b)| {
        a.0.cmp(&b.0)
            .then_with(|| b.1.cmp(&a.1))
            .then_with(|| b_group.cmp(a_group))
    }) else {
        return Vec::new();
    };
    candidates
        .iter()
        .copied()
        .filter(|&cell| !blocked(cell) && groups.cell(cell) == Some(selected_group))
        .collect()
}

fn structure_obstacle_cells(grid: &Grid, sites: &[StructureSite]) -> BTreeSet<CellId> {
    let mut obstacles = BTreeSet::new();
    for site in sites {
        let position = SpherePos::new(site.position);
        let radius = super::surface::structure_footprint_radius(site.kind) + 6.0;
        for cell in grid.topology.cells() {
            if position.distance(grid.cell_position(cell)) < radius {
                obstacles.insert(cell);
            }
        }
    }
    obstacles
}

fn select_layout_entrances(
    grid: &Grid,
    candidates: &[CellId],
    anchor: SpherePos,
    radius: f32,
    kind: SettlementKind,
) -> Option<Vec<CellId>> {
    let target_count = required_entrances(kind);
    let mut used = BTreeSet::new();
    let mut entrances = Vec::with_capacity(target_count);
    let compass = [
        0.0,
        std::f32::consts::FRAC_PI_2,
        std::f32::consts::PI,
        std::f32::consts::PI * 1.5,
        std::f32::consts::FRAC_PI_4,
        std::f32::consts::PI * 0.75,
        std::f32::consts::PI * 1.25,
        std::f32::consts::PI * 1.75,
    ];
    while entrances.len() < target_count {
        let angle = compass
            .iter()
            .copied()
            .max_by(|a, b| {
                let a_pos = ring_point(anchor, radius * 0.82, *a);
                let b_pos = ring_point(anchor, radius * 0.82, *b);
                let a_clearance = used
                    .iter()
                    .map(|&cell| a_pos.distance(grid.cell_position(cell)))
                    .fold(f32::INFINITY, f32::min);
                let b_clearance = used
                    .iter()
                    .map(|&cell| b_pos.distance(grid.cell_position(cell)))
                    .fold(f32::INFINITY, f32::min);
                a_clearance
                    .total_cmp(&b_clearance)
                    .then_with(|| b.total_cmp(a))
            })
            .unwrap_or(0.0);
        let desired = ring_point(anchor, radius * 0.82, angle);
        let entrance =
            choose_settlement_entrance(grid, candidates, anchor, desired, radius, &used)?;
        used.insert(entrance);
        entrances.push(entrance);
    }
    Some(entrances)
}

fn commit_layout_path(
    grid: &Grid,
    path: &[CellId],
    blocked: impl Fn(CellId) -> bool,
    components: &mut CellComponents,
    painted: &mut Painted,
    roads: &mut Vec<RoadPath>,
) {
    let band = safe_road_band(grid, path, blocked).expect("planned layout road remains safe");
    components.connect_path(path);
    for cell in band {
        painted.roads.insert(cell);
    }
    roads.push(RoadPath {
        cells: path.to_vec(),
        from_settlement: None,
        to_settlement: None,
        purpose: RoadPathPurpose::InternalLayout,
    });
}

pub(super) fn build_road_graph(
    grid: &Grid,
    roads: &[RoadPath],
    bridges: &[Vec<SpherePos>],
    settlement_entrances: &[Option<CellId>],
    settlements: &[SpherePos],
    settlement_config: SettlementConfig,
) -> RoadGraph {
    let mut adjacency = BTreeMap::<CellId, BTreeSet<CellId>>::new();
    let mut roles = BTreeMap::<CellId, Vec<RoadEndpointRole>>::new();
    let mut bridge_positions = BTreeMap::<CellId, SpherePos>::new();

    for path in roads {
        for pair in path.cells.windows(2) {
            adjacency.entry(pair[0]).or_default().insert(pair[1]);
            adjacency.entry(pair[1]).or_default().insert(pair[0]);
        }
        if let Some(&cell) = path.cells.first()
            && let Some(settlement_index) = path.from_settlement
        {
            push_role(
                &mut roles,
                cell,
                RoadEndpointRole::SettlementEntrance { settlement_index },
            );
        }
        if let Some(&cell) = path.cells.last()
            && let Some(settlement_index) = path.to_settlement
        {
            push_role(
                &mut roles,
                cell,
                RoadEndpointRole::SettlementEntrance { settlement_index },
            );
        }
    }

    for (settlement_index, entrance) in settlement_entrances.iter().enumerate() {
        if let Some(cell) = entrance
            && adjacency.contains_key(cell)
        {
            push_role(
                &mut roles,
                *cell,
                RoadEndpointRole::SettlementEntrance {
                    settlement_index: settlement_index as u32,
                },
            );
        }
    }

    // A route may pass through a settlement that was not one of its planned
    // endpoints. Split that road at the footprint boundary so travel switches
    // between named road connections at distinct, walkable entrances.
    for (settlement_index, &settlement) in settlements.iter().enumerate() {
        let radius = settlement_config.radius_m(
            settlement_config
                .kind_at(settlement_index)
                .expect("settlement index must be configured"),
        );
        for road in roads {
            // Internal street geometry is represented by graph junctions and
            // road ends. Only routes planned between settlements can create
            // exterior entrances as they pass through a third settlement.
            // A route's planned origin or destination already has an explicit
            // entrance cell above; scanning its in-footprint run would create
            // a second, deeper entrance for that same approach.
            if road.purpose == RoadPathPurpose::InternalLayout
                || road.from_settlement == Some(settlement_index as u32)
                || road.to_settlement == Some(settlement_index as u32)
            {
                continue;
            }
            let mut run_start = None;
            for (index, &cell) in road.cells.iter().enumerate() {
                let inside = settlement.distance(grid.cell_position(cell)) <= radius;
                match (run_start, inside) {
                    (None, true) => run_start = Some(index),
                    (Some(start), false) => {
                        push_role(
                            &mut roles,
                            road.cells[start],
                            RoadEndpointRole::SettlementEntrance {
                                settlement_index: settlement_index as u32,
                            },
                        );
                        push_role(
                            &mut roles,
                            road.cells[index - 1],
                            RoadEndpointRole::SettlementEntrance {
                                settlement_index: settlement_index as u32,
                            },
                        );
                        run_start = None;
                    }
                    _ => {}
                }
            }
            if let Some(start) = run_start {
                push_role(
                    &mut roles,
                    road.cells[start],
                    RoadEndpointRole::SettlementEntrance {
                        settlement_index: settlement_index as u32,
                    },
                );
                push_role(
                    &mut roles,
                    *road.cells.last().expect("road path is not empty"),
                    RoadEndpointRole::SettlementEntrance {
                        settlement_index: settlement_index as u32,
                    },
                );
            }
        }
    }

    let bridge_cells = bridges
        .iter()
        .filter_map(|points| {
            Some((
                nearest_cell(grid, *points.first()?)?,
                nearest_cell(grid, *points.last()?)?,
                points,
            ))
        })
        .collect::<Vec<_>>();
    for (from, to, points) in &bridge_cells {
        push_role(&mut roles, *from, RoadEndpointRole::BridgeEntrance);
        push_role(&mut roles, *to, RoadEndpointRole::BridgeEntrance);
        bridge_positions.entry(*from).or_insert(points[0]);
        bridge_positions
            .entry(*to)
            .or_insert(*points.last().unwrap());
    }

    for (&cell, neighbors) in &adjacency {
        if neighbors.len() >= 3 {
            push_role(&mut roles, cell, RoadEndpointRole::Junction);
        } else if neighbors.len() == 1 {
            push_role(&mut roles, cell, RoadEndpointRole::RoadEnd);
        }
    }

    let endpoint_ids = roles
        .keys()
        .enumerate()
        .map(|(index, &cell)| (cell, index as u32))
        .collect::<BTreeMap<_, _>>();
    let endpoints = endpoint_ids
        .iter()
        .map(|(&cell, _)| RoadEndpointData {
            pos: bridge_positions
                .get(&cell)
                .copied()
                .unwrap_or_else(|| grid.cell_position(cell))
                .0
                .to_array(),
            roles: roles[&cell].clone(),
        })
        .collect::<Vec<_>>();

    let mut road_connections = Vec::<(RoadData, Vec<CellId>)>::new();
    let nodes = endpoint_ids.keys().copied().collect::<BTreeSet<_>>();
    let mut visited = BTreeSet::<(CellId, CellId)>::new();
    for &start in &nodes {
        let Some(neighbors) = adjacency.get(&start) else {
            continue;
        };
        for &next in neighbors {
            let first_edge = ordered_edge(start, next);
            if !visited.insert(first_edge) {
                continue;
            }
            let mut cells = vec![start, next];
            let (mut previous, mut current) = (start, next);
            while !nodes.contains(&current) {
                let Some(&next) = adjacency[&current].iter().find(|&&cell| cell != previous) else {
                    break;
                };
                visited.insert(ordered_edge(current, next));
                cells.push(next);
                previous = current;
                current = next;
            }
            if nodes.contains(&current) {
                let connection_index = road_connections.len();
                let from_endpoint = endpoint_ids[&start];
                let to_endpoint = endpoint_ids[&current];
                let points = cells
                    .iter()
                    .map(|cell| {
                        bridge_positions
                            .get(cell)
                            .copied()
                            .unwrap_or_else(|| grid.cell_position(*cell))
                            .0
                            .to_array()
                    })
                    .collect();
                road_connections.push((
                    RoadData {
                        name: format!("Road {}", connection_index + 1),
                        points,
                        kind: RoadKind::Road,
                        from_endpoint,
                        to_endpoint,
                    },
                    cells,
                ));
            }
        }
    }

    let mut connections = Vec::new();
    let mut road_cells = Vec::new();
    for (connection, cells) in road_connections {
        road_cells.push((connections.len() as u32, cells));
        connections.push(connection);
    }
    for (index, (from, to, points)) in bridge_cells.into_iter().enumerate() {
        connections.push(RoadData {
            name: format!("Bridge {}", index + 1),
            points: points.iter().map(|point| point.0.to_array()).collect(),
            kind: RoadKind::Bridge,
            from_endpoint: endpoint_ids[&from],
            to_endpoint: endpoint_ids[&to],
        });
    }

    RoadGraph {
        connections,
        endpoints,
        road_cells,
    }
}

fn push_role(
    roles: &mut BTreeMap<CellId, Vec<RoadEndpointRole>>,
    cell: CellId,
    role: RoadEndpointRole,
) {
    let roles = roles.entry(cell).or_default();
    if !roles.contains(&role) {
        roles.push(role);
    }
}

fn ordered_edge(a: CellId, b: CellId) -> (CellId, CellId) {
    if a < b { (a, b) } else { (b, a) }
}

fn unsafe_road_edges(grid: &Grid, blocked: impl Fn(CellId) -> bool) -> BTreeSet<(CellId, CellId)> {
    let mut unsafe_edges = BTreeSet::new();
    for from in grid.topology.cells() {
        for &to in grid.cell_neighbors(from) {
            if safe_road_band(grid, &[from, to], &blocked).is_none() {
                unsafe_edges.insert((from, to));
            }
        }
    }
    unsafe_edges
}

/// Widen each road segment onto a passable neighboring face. Prefer the
/// normal left side of travel, but use the other side where water or cliffs
/// would otherwise make the whole connection impossible.
pub(super) fn safe_road_band(
    grid: &Grid,
    chain: &[CellId],
    blocked: impl Fn(CellId) -> bool,
) -> Option<Vec<CellId>> {
    if chain.iter().copied().any(&blocked) {
        return None;
    }
    let mut band = chain.to_vec();
    for pair in chain.windows(2) {
        let (start, end) = (pair[0], pair[1]);
        let left = grid.cell_direction(start).cross(grid.cell_direction(end));
        let mut left_partner = None;
        let mut right_partner = None;
        for &face in grid.topology.cell_faces(start) {
            let corners = grid.face_cells(face);
            if !corners.contains(&end) {
                continue;
            }
            let partner = *corners
                .iter()
                .find(|&&candidate| candidate != start && candidate != end)?;
            if grid.cell_direction(partner).dot(left) > 0.0 {
                left_partner = Some(partner);
            } else {
                right_partner = Some(partner);
            }
        }
        let partner = left_partner
            .filter(|&cell| !blocked(cell))
            .or_else(|| right_partner.filter(|&cell| !blocked(cell)))?;
        if !band.contains(&partner) {
            band.push(partner);
        }
    }
    Some(band)
}

struct CellComponents(Vec<usize>);

impl CellComponents {
    fn new(cell_count: usize) -> Self {
        Self((0..cell_count).collect())
    }

    fn root(&mut self, cell: CellId) -> usize {
        let index = cell.index();
        if self.0[index] != index {
            self.0[index] = self.root(CellId::new(self.0[index]));
        }
        self.0[index]
    }

    fn same(&mut self, a: CellId, b: CellId) -> bool {
        self.root(a) == self.root(b)
    }

    fn connect_path(&mut self, path: &[CellId]) {
        for pair in path.windows(2) {
            self.connect(pair[0], pair[1]);
        }
    }

    fn connect(&mut self, a: CellId, b: CellId) {
        let (from, to) = (self.root(a), self.root(b));
        self.0[to] = from;
    }
}

struct RoadableGroups {
    components: CellComponents,
    roadable: Vec<bool>,
}

impl RoadableGroups {
    fn new(
        grid: &Grid,
        blocked: impl Fn(CellId) -> bool,
        unsafe_edges: &BTreeSet<(CellId, CellId)>,
    ) -> Self {
        let roadable = grid
            .topology
            .cells()
            .map(|cell| !blocked(cell))
            .collect::<Vec<_>>();
        let mut components = CellComponents::new(grid.cell_count());
        for cell in grid.topology.cells() {
            if !roadable[cell.index()] {
                continue;
            }
            for &neighbor in grid.cell_neighbors(cell) {
                if roadable[neighbor.index()] && !unsafe_edges.contains(&(cell, neighbor)) {
                    components.connect(cell, neighbor);
                }
            }
        }
        Self {
            components,
            roadable,
        }
    }

    fn cell(&mut self, cell: CellId) -> Option<usize> {
        self.roadable[cell.index()].then(|| self.components.root(cell))
    }
}
