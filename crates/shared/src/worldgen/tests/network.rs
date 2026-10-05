use std::collections::{BTreeMap, BTreeSet, VecDeque};

use bevy::prelude::Vec3;

use super::*;
use crate::level::{RoadEndpointRole, RoadKind, SlopeClass};
use terra_geometry::sphere::{SpherePos, ring_point};
use crate::terrain::Terrain;

fn distance_to_segment(point: [f32; 3], start: [f32; 3], end: [f32; 3]) -> f32 {
    let point = Vec3::from_array(point);
    let start = Vec3::from_array(start);
    let segment = Vec3::from_array(end) - start;
    let t = ((point - start).dot(segment) / segment.length_squared()).clamp(0.0, 1.0);
    point.distance(start + segment * t) * terra_geometry::sphere::PLANET_RADIUS
}

#[test]
fn settlement_site_selection_reports_when_a_zone_has_no_roadable_site() {
    let terrain = TerrainGen::new(42);
    let grid = Grid::new(42);
    let cells = vec![Terrain::Cliff; grid.cell_count()];
    let slopes = vec![SlopeClass::Cliff; grid.cell_count()];

    let error = super::super::network::select_settlement_sites(&grid, &terrain, &cells, &slopes)
        .unwrap_err();
    assert!(error.contains("settlement 0 in zone"));
    assert!(error.contains("safe multi-edge road corridor"));
}

#[test]
fn settlement_site_selection_relocates_an_infeasible_anchor_within_its_zone() {
    let config = crate::level::SettlementConfig {
        towns: 1,
        villages: 1,
        outposts: 1,
        ..crate::level::SettlementConfig::default()
    };
    let terrain = TerrainGen::new_with_settlement_config(1337, config);
    let grid = Grid::new(1337);
    let original = terrain.settlement_anchors[0];
    let (zone_id, _) = terrain
        .zones()
        .zones_of_kind(crate::zones::ZoneKind::Settlement)
        .next()
        .expect("configured town has a settlement zone");
    let relocated = grid
        .topology
        .cells()
        .map(|cell| grid.cell_position(cell))
        .filter(|&position| terrain.zone_id_at(position) == zone_id)
        .max_by(|a, b| original.distance(*a).total_cmp(&original.distance(*b)))
        .expect("settlement zone has fine-grid candidate sites");
    let blocked_radius = config.town_radius_m * 0.7;
    let mut cells = vec![Terrain::Plains; grid.cell_count()];
    let mut slopes = vec![SlopeClass::Flat; grid.cell_count()];
    for cell in grid.topology.cells() {
        if original.distance(grid.cell_position(cell)) <= blocked_radius {
            cells[cell.index()] = Terrain::Cliff;
            slopes[cell.index()] = SlopeClass::Cliff;
        }
    }

    let (exhausted, exhausted_attempts) = super::super::network::select_feasible_settlement_site(
        &grid,
        &terrain,
        &cells,
        &slopes,
        &[original, relocated],
        crate::level::SettlementKind::Town,
        config.town_radius_m,
        1,
        &BTreeSet::new(),
    );
    assert!(exhausted.is_none());
    assert_eq!(exhausted_attempts, 1, "retry budget is respected");

    let (selected, attempts) = super::super::network::select_feasible_settlement_site(
        &grid,
        &terrain,
        &cells,
        &slopes,
        &[original, relocated],
        crate::level::SettlementKind::Town,
        config.town_radius_m,
        2,
        &BTreeSet::new(),
    );

    let selected = selected.expect("selector finds the viable replacement");
    assert!(selected.distance(relocated) < 1e-3);
    assert_eq!(attempts, 2, "failed full-layout preflight is retried");
    assert!(original.distance(relocated) > config.town_radius_m);
    assert_eq!(terrain.zone_id_at(relocated), zone_id);
}

#[test]
fn settlement_entrances_follow_approach_direction_and_are_distinct() {
    let grid = Grid::new(42);
    let anchor = SpherePos(Vec3::Y);
    let radius = 55.0;
    let candidates = grid
        .topology
        .cells()
        .filter(|&cell| {
            let distance = anchor.distance(grid.cell_position(cell));
            distance >= radius * 0.65 && distance <= radius
        })
        .collect::<Vec<_>>();
    let east_neighbor = ring_point(anchor, 300.0, 0.0);
    let north_neighbor = ring_point(anchor, 300.0, std::f32::consts::FRAC_PI_2);
    let mut used = BTreeSet::new();

    let east = super::super::network::choose_settlement_entrance(
        &grid,
        &candidates,
        anchor,
        east_neighbor,
        radius,
        &used,
    )
    .expect("the footprint has a roadable perimeter candidate");
    used.insert(east);
    let north = super::super::network::choose_settlement_entrance(
        &grid,
        &candidates,
        anchor,
        north_neighbor,
        radius,
        &used,
    )
    .expect("a second approach gets its own perimeter candidate");

    assert_ne!(east, north);
    assert!(anchor.distance(grid.cell_position(east)) >= radius * 0.65);
    assert!(anchor.distance(grid.cell_position(north)) >= radius * 0.65);
    assert!(
        grid.cell_position(east).distance(east_neighbor)
            < grid.cell_position(north).distance(east_neighbor)
    );
    assert!(
        grid.cell_position(north).distance(north_neighbor)
            < grid.cell_position(east).distance(north_neighbor)
    );
}

#[test]
fn internal_street_endpoints_are_not_external_settlement_entrances() {
    let grid = super::super::grid::Grid::new(42);
    let settlement = SpherePos(Vec3::Y);
    let start = grid
        .topology
        .cells()
        .min_by(|&a, &b| {
            settlement
                .distance(grid.cell_position(a))
                .total_cmp(&settlement.distance(grid.cell_position(b)))
        })
        .expect("grid has cells");
    let end = grid.cell_neighbors(start)[0];
    let internal_street = crate::worldgen::RoadPath {
        cells: vec![start, end],
        from_settlement: None,
        to_settlement: None,
        purpose: crate::worldgen::RoadPathPurpose::InternalLayout,
    };

    let graph = super::super::network::build_road_graph(
        &grid,
        &[internal_street],
        &[],
        &[None],
        &[settlement],
        crate::level::SettlementConfig::default(),
    );

    assert!(graph.endpoints.iter().all(|endpoint| {
        !endpoint
            .roles
            .contains(&RoadEndpointRole::SettlementEntrance {
                settlement_index: 0,
            })
    }));
    assert!(
        graph
            .endpoints
            .iter()
            .any(|endpoint| endpoint.roles.contains(&RoadEndpointRole::RoadEnd))
    );
}

#[test]
fn bridge_approach_paths_create_settlement_entrances() {
    let grid = super::super::grid::Grid::new(42);
    let settlement = SpherePos(Vec3::Y);
    let radius = crate::level::SettlementConfig::default().town_radius_m;
    let (inside, outside) = grid
        .topology
        .cells()
        .find_map(|cell| {
            if settlement.distance(grid.cell_position(cell)) > radius {
                return None;
            }
            grid.cell_neighbors(cell)
                .iter()
                .copied()
                .find(|&neighbor| settlement.distance(grid.cell_position(neighbor)) > radius)
                .map(|neighbor| (cell, neighbor))
        })
        .expect("settlement footprint has a boundary edge");
    let approach = crate::worldgen::RoadPath {
        cells: vec![inside, outside],
        from_settlement: None,
        to_settlement: None,
        purpose: crate::worldgen::RoadPathPurpose::BridgeApproach,
    };

    let graph = super::super::network::build_road_graph(
        &grid,
        &[approach],
        &[],
        &[None],
        &[settlement],
        crate::level::SettlementConfig::default(),
    );

    let entrance = graph
        .endpoints
        .iter()
        .find(|endpoint| {
            endpoint
                .roles
                .contains(&RoadEndpointRole::SettlementEntrance {
                    settlement_index: 0,
                })
        })
        .expect("bridge approach must connect through a named settlement entrance");
    assert_eq!(entrance.pos, grid.cell_position(inside).0.to_array());
}

#[test]
fn bridge_banks_do_not_connect_through_internal_streets_alone() {
    let grid = super::super::grid::Grid::new(42);
    let anchor = SpherePos(Vec3::Y);
    let first = grid
        .topology
        .cells()
        .min_by(|&a, &b| {
            anchor
                .distance(grid.cell_position(a))
                .total_cmp(&anchor.distance(grid.cell_position(b)))
        })
        .expect("grid has cells");
    let second = grid.cell_neighbors(first)[0];
    let mut roads = vec![crate::worldgen::RoadPath {
        cells: vec![first, second],
        from_settlement: None,
        to_settlement: None,
        purpose: crate::worldgen::RoadPathPurpose::InternalLayout,
    }];
    let cells = vec![Terrain::Plains; grid.cell_count()];
    let slopes = vec![SlopeClass::Flat; grid.cell_count()];
    let mut painted = crate::worldgen::Painted::empty(grid.cell_count());
    let span = vec![grid.cell_position(first), grid.cell_position(second)];

    let bridges = super::super::network::connect_bridges(
        &grid,
        &cells,
        &slopes,
        &[],
        crate::level::SettlementConfig::default(),
        &[],
        &[],
        vec![span],
        &mut painted,
        &mut roads,
    );

    assert!(bridges.is_empty());
    assert!(
        grid.topology
            .cells()
            .all(|cell| !painted.bridges.contains(cell))
    );
    assert_eq!(roads.len(), 1, "no bridge approach road should be staged");
}

#[test]
fn bridge_approach_uses_a_distinct_entrance_when_its_bank_meets_a_used_one() {
    let grid = super::super::grid::Grid::new(42);
    let settlement = SpherePos(Vec3::Y);
    let config = crate::level::SettlementConfig::default();
    let radius = config.town_radius_m;
    let (used_entrance, route_outside, bridge_bank) = grid
        .topology
        .cells()
        .find_map(|entrance| {
            let distance = settlement.distance(grid.cell_position(entrance));
            if !(radius * 0.65..=radius).contains(&distance) {
                return None;
            }
            let outside = grid
                .cell_neighbors(entrance)
                .iter()
                .copied()
                .find(|&cell| settlement.distance(grid.cell_position(cell)) > radius)?;
            let bridge_bank = grid.cell_neighbors(outside).iter().copied().find(|&cell| {
                cell != entrance
                    && cell != outside
                    && settlement.distance(grid.cell_position(cell)) > radius
            })?;
            Some((entrance, outside, bridge_bank))
        })
        .expect("fixture has a settlement edge with two exterior roadable cells");
    let mut previous = vec![None; grid.cell_count()];
    let mut reachable = vec![false; grid.cell_count()];
    let mut queue = std::collections::VecDeque::from([used_entrance]);
    reachable[used_entrance.index()] = true;
    while let Some(cell) = queue.pop_front() {
        for &neighbor in grid.cell_neighbors(cell) {
            if !reachable[neighbor.index()]
                && settlement.distance(grid.cell_position(neighbor)) <= radius + 6.0
            {
                reachable[neighbor.index()] = true;
                previous[neighbor.index()] = Some(cell);
                queue.push_back(neighbor);
            }
        }
    }
    let available_entrance = grid
        .topology
        .cells()
        .filter(|&cell| {
            cell != used_entrance
                && reachable[cell.index()]
                && (radius * 0.65..=radius).contains(&settlement.distance(grid.cell_position(cell)))
                && grid
                    .cell_position(cell)
                    .distance(grid.cell_position(used_entrance))
                    >= radius * 0.2
        })
        .max_by(|&a, &b| {
            grid.cell_position(a)
                .distance(grid.cell_position(used_entrance))
                .total_cmp(
                    &grid
                        .cell_position(b)
                        .distance(grid.cell_position(used_entrance)),
                )
        })
        .expect("fixture has another connected layout entrance");
    let mut internal_path = vec![available_entrance];
    let mut cursor = available_entrance;
    while cursor != used_entrance {
        cursor = previous[cursor.index()].expect("reachable entrance has a predecessor");
        internal_path.push(cursor);
    }
    internal_path.reverse();
    let mut roads = vec![
        crate::worldgen::RoadPath {
            cells: internal_path,
            from_settlement: None,
            to_settlement: None,
            purpose: crate::worldgen::RoadPathPurpose::InternalLayout,
        },
        crate::worldgen::RoadPath {
            cells: vec![route_outside, used_entrance],
            from_settlement: None,
            to_settlement: Some(0),
            purpose: crate::worldgen::RoadPathPurpose::ExternalRoute,
        },
    ];
    let cells = vec![Terrain::Plains; grid.cell_count()];
    let slopes = vec![SlopeClass::Flat; grid.cell_count()];
    let mut painted = crate::worldgen::Painted::empty(grid.cell_count());
    let span = vec![
        grid.cell_position(used_entrance),
        grid.cell_position(bridge_bank),
    ];

    let bridges = super::super::network::connect_bridges(
        &grid,
        &cells,
        &slopes,
        &[settlement],
        config,
        &[vec![used_entrance, available_entrance]],
        &[],
        vec![span],
        &mut painted,
        &mut roads,
    );
    assert_eq!(bridges.len(), 1, "fixture has a safe distinct entrance");
    assert!(roads.iter().any(|path| {
        path.purpose == crate::worldgen::RoadPathPurpose::BridgeApproach
            && path.to_settlement == Some(0)
            && path.cells.last() == Some(&available_entrance)
    }));
    assert!(roads.iter().any(|path| {
        path.purpose == crate::worldgen::RoadPathPurpose::InternalLayout
            && path.cells.contains(&available_entrance)
    }));
    let graph = super::super::network::build_road_graph(
        &grid,
        &roads,
        &bridges,
        &[Some(used_entrance)],
        &[settlement],
        config,
    );

    let entrances = graph
        .endpoints
        .iter()
        .filter(|endpoint| {
            endpoint
                .roles
                .contains(&RoadEndpointRole::SettlementEntrance {
                    settlement_index: 0,
                })
        })
        .map(|endpoint| endpoint.pos)
        .collect::<Vec<_>>();
    let has_distinct_entrances = entrances.iter().any(|&first| {
        entrances
            .iter()
            .any(|&second| Vec3::from_array(first).distance(Vec3::from_array(second)) > 1e-4)
    });
    assert!(
        has_distinct_entrances,
        "a bridge bank at an occupied entrance must allocate a distinct approach"
    );
    assert!(
        entrances.contains(&grid.cell_position(used_entrance).0.to_array()),
        "the pre-existing exterior entrance remains present"
    );
}

#[test]
fn outpost_site_ranking_prefers_a_visible_road_crossing() {
    let crossing = SpherePos(Vec3::Y);
    let nearby_site = ring_point(crossing, 100.0, 0.0);
    let distant_site = ring_point(crossing, 500.0, std::f32::consts::FRAC_PI_2);
    let targets = [super::super::network::StrategicRoadTarget {
        position: crossing,
        elevation_m: 0.0,
        crossing: true,
    }];

    let nearby_score = super::super::network::outpost_site_score(nearby_site, 100.0, &targets);
    let distant_score = super::super::network::outpost_site_score(distant_site, 100.0, &targets);

    assert!(nearby_score > distant_score);
}

#[test]
fn generated_network_contract_holds_for_locked_seeds() {
    for seed in [42, 1337] {
        let world = run_state(seed, |_| {});
        let level = world.to_level_data();
        for road in &world.roads {
            for &cell in &road.cells {
                let terrain = world.cells.as_slice()[cell.index()];
                let slope = world.slope_class.as_slice()[cell.index()];
                assert!(
                    terrain != Terrain::Cliff
                        && !terrain.is_water()
                        && matches!(slope, SlopeClass::Flat | SlopeClass::Gentle),
                    "seed {seed}: road path crosses unsuitable cell {} ({terrain:?}, {slope:?})",
                    cell.index(),
                );
            }
        }
        let mut endpoint_degree = vec![0usize; level.road_endpoints.len()];
        let mut endpoint_road_degree = vec![0usize; level.road_endpoints.len()];
        let mut connected_settlements = vec![false; level.settlements.len()];
        let mut names = BTreeSet::new();

        assert!(!level.road_endpoints.is_empty(), "seed {seed}");
        assert!(!level.roads.is_empty(), "seed {seed}");

        for connection in &level.roads {
            assert!(!connection.name.trim().is_empty(), "seed {seed}");
            assert!(names.insert(connection.name.as_str()), "seed {seed}");
            assert_ne!(connection.from_endpoint, connection.to_endpoint);
            assert!(connection.points.len() >= 2);
            let from = connection.from_endpoint as usize;
            let to = connection.to_endpoint as usize;
            assert!(from < endpoint_degree.len());
            assert!(to < endpoint_degree.len());
            endpoint_degree[from] += 1;
            endpoint_degree[to] += 1;
            if connection.kind == RoadKind::Road {
                endpoint_road_degree[from] += 1;
                endpoint_road_degree[to] += 1;
            }
            assert!(
                Vec3::from_array(connection.points[0])
                    .distance(Vec3::from_array(level.road_endpoints[from].pos))
                    < 1e-5,
                "seed {seed}: road start must match its endpoint"
            );
            assert!(
                Vec3::from_array(*connection.points.last().unwrap())
                    .distance(Vec3::from_array(level.road_endpoints[to].pos))
                    < 1e-5,
                "seed {seed}: road end must match its endpoint"
            );
        }

        let mut endpoint_components = (0..level.road_endpoints.len()).collect::<Vec<_>>();
        for connection in level
            .roads
            .iter()
            .filter(|road| road.kind == RoadKind::Road)
        {
            join(
                &mut endpoint_components,
                connection.from_endpoint as usize,
                connection.to_endpoint as usize,
            );
        }

        let (mut roadable_groups, roadable_cells) = safe_roadable_component_map(&world);
        let zones = world.terrain().zones();
        for (index, &anchor) in world.terrain().settlement_anchors.iter().enumerate() {
            let (zone_id, _) = zones
                .zones_of_kind(crate::zones::ZoneKind::Settlement)
                .nth(index)
                .expect("each settlement anchor retains its planned zone");
            assert!(
                world.terrain().zone_id_at(anchor) == zone_id,
                "seed {seed}: settlement {index} should remain in its planned zone"
            );
            let candidates = roadable_settlement_cells(&world, index);
            let diameter = safe_settlement_road_components(&world, &candidates)
                .iter()
                .map(|component| roadable_settlement_component_diameter(&world, component))
                .max()
                .unwrap_or(0);
            assert!(
                diameter >= 2,
                "seed {seed}: settlement {index} has no safe multi-edge road corridor inside its footprint ({} roadable cells, diameter {diameter})",
                candidates.len(),
            );
        }
        let mut road_networks_by_land = BTreeMap::<_, Vec<(usize, usize, usize)>>::new();
        for (endpoint_index, endpoint) in level.road_endpoints.iter().enumerate() {
            assert!(endpoint_degree[endpoint_index] > 0, "seed {seed}");
            assert!(!endpoint.roles.is_empty(), "seed {seed}");
            for role in &endpoint.roles {
                match role {
                    RoadEndpointRole::SettlementEntrance { settlement_index } => {
                        let settlement_index = *settlement_index as usize;
                        connected_settlements[settlement_index] = true;
                        let anchor = world.terrain().settlement_anchors[settlement_index];
                        let cell = road_endpoint_cell(&world, &level, endpoint_index)
                            .expect("settlement entrance should meet a road connection");
                        assert!(world.painted.settlements.contains(cell));
                        assert!(
                            anchor.distance(world.grid.cell_position(cell))
                                <= settlement_radius(&world, settlement_index),
                            "seed {seed}: settlement entrance must stay inside its footprint"
                        );
                        assert!(
                            roadable_cells[cell.index()],
                            "settlement entrance should stand on roadable land"
                        );
                        let land = root(&mut roadable_groups, cell.index());
                        road_networks_by_land.entry(land).or_default().push((
                            settlement_index,
                            endpoint_index,
                            root(&mut endpoint_components, endpoint_index),
                        ));
                    }
                    RoadEndpointRole::Junction => {
                        assert!(endpoint_road_degree[endpoint_index] >= 3, "seed {seed}");
                    }
                    RoadEndpointRole::BridgeEntrance => {
                        assert!(endpoint_road_degree[endpoint_index] > 0, "seed {seed}");
                    }
                    RoadEndpointRole::RoadEnd => {
                        assert_eq!(endpoint_road_degree[endpoint_index], 1, "seed {seed}");
                    }
                }
            }
        }
        let unconnected = connected_settlements
            .iter()
            .enumerate()
            .filter_map(|(index, &connected)| {
                if connected {
                    return None;
                }
                let candidates = roadable_settlement_cells(&world, index);
                let Some(entrance) = candidates.first().copied() else {
                    return Some((
                        index,
                        0usize,
                        None,
                        0usize,
                        0usize,
                        0usize,
                        0usize,
                        0.0f32,
                        (0, 0),
                    ));
                };
                let (component_size, diameter_edges, diameter_m, start, end) =
                    roadable_settlement_diameter(&world, &candidates, entrance);
                let land = roadable_cells[entrance.index()]
                    .then(|| root(&mut roadable_groups, entrance.index()));
                let road_count = world
                    .roads
                    .iter()
                    .filter(|road| road.cells.contains(&entrance))
                    .count();
                Some((
                    index,
                    entrance.index(),
                    land,
                    road_count,
                    candidates.len(),
                    component_size,
                    diameter_edges,
                    diameter_m,
                    (start.index(), end.index()),
                ))
            })
            .collect::<Vec<_>>();
        assert!(
            unconnected.is_empty(),
            "seed {seed}: each settlement needs a road entrance; unconnected={unconnected:?}"
        );
        for (land, networks) in road_networks_by_land {
            let unique_networks = networks
                .iter()
                .map(|(_, _, network)| *network)
                .collect::<BTreeSet<_>>();
            assert_eq!(
                unique_networks.len(),
                1,
                "seed {seed}: settlements in roadable network group {land} need one local road network; settlement/endpoints/roots={networks:?}",
            );
        }

        let bridge_roads = level
            .roads
            .iter()
            .filter(|road| road.kind == RoadKind::Bridge)
            .collect::<Vec<_>>();
        assert!(
            !bridge_roads.is_empty(),
            "seed {seed} should contain bridges"
        );
        let road_centerlines = level
            .roads
            .iter()
            .filter(|road| road.kind == RoadKind::Road)
            .flat_map(|road| road.points.windows(2))
            .collect::<Vec<_>>();
        for bridge in bridge_roads {
            for entrance in [
                bridge.points.first().unwrap(),
                bridge.points.last().unwrap(),
            ] {
                let nearest = road_centerlines
                    .iter()
                    .map(|pair| distance_to_segment(*entrance, pair[0], pair[1]))
                    .fold(f32::INFINITY, f32::min);
                assert!(
                    nearest <= 8.0,
                    "seed {seed}: bridge entrance misses the road centerline by {nearest:.1} m"
                );
            }
            assert!(
                level.road_endpoints[bridge.from_endpoint as usize]
                    .roles
                    .contains(&RoadEndpointRole::BridgeEntrance)
            );
            assert!(
                level.road_endpoints[bridge.to_endpoint as usize]
                    .roles
                    .contains(&RoadEndpointRole::BridgeEntrance)
            );
        }

        let road_names = level
            .roads
            .iter()
            .filter(|road| road.kind == RoadKind::Road)
            .map(|road| road.name.as_str())
            .collect::<BTreeSet<_>>();
        let region_names = level
            .regions
            .iter()
            .filter(|region| region.kind == crate::level::RegionKind::Road)
            .map(|region| region.name.as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(road_names, region_names, "seed {seed}");
        level
            .validate()
            .unwrap_or_else(|error| panic!("seed {seed}: {error}"));
    }
}

fn road_endpoint_cell(
    world: &GenState,
    level: &crate::level::LevelData,
    endpoint: usize,
) -> Option<CellId> {
    world
        .network
        .road_cells
        .iter()
        .find_map(|(road_index, cells)| {
            let road = &level.roads[*road_index as usize];
            if road.from_endpoint as usize == endpoint {
                cells.first().copied()
            } else if road.to_endpoint as usize == endpoint {
                cells.last().copied()
            } else {
                None
            }
        })
}

fn settlement_radius(world: &GenState, index: usize) -> f32 {
    let terrain = world.terrain();
    terrain
        .settlement_config()
        .radius_m(terrain.settlement_kind(index))
}

fn roadable_settlement_cells(world: &GenState, index: usize) -> Vec<CellId> {
    let anchor = world.terrain().settlement_anchors[index];
    let radius = settlement_radius(world, index);
    let mut candidates = world
        .grid
        .topology
        .cells()
        .filter(|&cell| {
            world.painted.settlements.contains(cell)
                && anchor.distance(world.grid.cell_position(cell)) <= radius
        })
        .filter(|&cell| {
            let terrain = world.cells.as_slice()[cell.index()];
            terrain != Terrain::Cliff
                && !terrain.is_water()
                && matches!(
                    world.slope_class.as_slice()[cell.index()],
                    SlopeClass::Flat | SlopeClass::Gentle
                )
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|&a, &b| {
        anchor
            .distance(world.grid.cell_position(a))
            .total_cmp(&anchor.distance(world.grid.cell_position(b)))
            .then_with(|| a.cmp(&b))
    });
    candidates
}

fn safe_roadable_component_map(world: &GenState) -> (Vec<usize>, Vec<bool>) {
    let blocked = |cell: CellId| {
        !features::roadable(
            world.cells.as_slice()[cell.index()],
            world.slope_class.as_slice()[cell.index()],
        )
    };
    let roadable = world
        .grid
        .topology
        .cells()
        .map(|cell| !blocked(cell))
        .collect::<Vec<_>>();
    let mut components = (0..world.grid.cell_count()).collect::<Vec<_>>();
    for cell in world.grid.topology.cells() {
        if blocked(cell) {
            continue;
        }
        for &neighbor in world.grid.cell_neighbors(cell) {
            if !blocked(neighbor)
                && super::super::network::safe_road_band(&world.grid, &[cell, neighbor], blocked)
                    .is_some()
            {
                join(&mut components, cell.index(), neighbor.index());
            }
        }
    }
    (components, roadable)
}

fn safe_settlement_road_components(
    world: &GenState,
    candidates: &[CellId],
) -> Vec<BTreeSet<CellId>> {
    let candidate_set = candidates.iter().copied().collect::<BTreeSet<_>>();
    let blocked = |cell: CellId| {
        !features::roadable(
            world.cells.as_slice()[cell.index()],
            world.slope_class.as_slice()[cell.index()],
        )
    };
    let mut assigned = BTreeSet::new();
    let mut components = Vec::new();
    for &start in candidates {
        if !assigned.insert(start) {
            continue;
        }
        let mut component = BTreeSet::from([start]);
        let mut queue = VecDeque::from([start]);
        while let Some(cell) = queue.pop_front() {
            for &neighbor in world.grid.cell_neighbors(cell) {
                if candidate_set.contains(&neighbor)
                    && !assigned.contains(&neighbor)
                    && super::super::network::safe_road_band(
                        &world.grid,
                        &[cell, neighbor],
                        blocked,
                    )
                    .is_some()
                {
                    assigned.insert(neighbor);
                    component.insert(neighbor);
                    queue.push_back(neighbor);
                }
            }
        }
        components.push(component);
    }
    components
}

fn roadable_settlement_component_diameter(world: &GenState, component: &BTreeSet<CellId>) -> usize {
    let mut diameter = 0;
    for &source in component {
        let mut distances = BTreeMap::from([(source, 0usize)]);
        let mut queue = VecDeque::from([source]);
        while let Some(cell) = queue.pop_front() {
            for &neighbor in world.grid.cell_neighbors(cell) {
                if component.contains(&neighbor) && !distances.contains_key(&neighbor) {
                    distances.insert(neighbor, distances[&cell] + 1);
                    queue.push_back(neighbor);
                }
            }
        }
        diameter = diameter.max(distances.values().copied().max().unwrap_or(0));
    }
    diameter
}

fn roadable_settlement_diameter(
    world: &GenState,
    candidates: &[CellId],
    start: CellId,
) -> (usize, usize, f32, CellId, CellId) {
    let candidate_set = candidates.iter().copied().collect::<BTreeSet<_>>();
    let mut component = BTreeSet::from([start]);
    let mut queue = VecDeque::from([start]);
    while let Some(cell) = queue.pop_front() {
        for &neighbor in world.grid.cell_neighbors(cell) {
            if candidate_set.contains(&neighbor) && component.insert(neighbor) {
                queue.push_back(neighbor);
            }
        }
    }

    let mut diameter_edges = 0;
    let mut diameter_m = 0.0;
    let mut endpoints = (start, start);
    for &source in &component {
        let mut edges = BTreeMap::from([(source, 0usize)]);
        let mut distances = BTreeMap::from([(source, 0.0f32)]);
        let mut queue = VecDeque::from([source]);
        while let Some(cell) = queue.pop_front() {
            for &neighbor in world.grid.cell_neighbors(cell) {
                if !component.contains(&neighbor) || edges.contains_key(&neighbor) {
                    continue;
                }
                edges.insert(neighbor, edges[&cell] + 1);
                distances.insert(
                    neighbor,
                    distances[&cell]
                        + world
                            .grid
                            .cell_position(cell)
                            .distance(world.grid.cell_position(neighbor)),
                );
                queue.push_back(neighbor);
            }
        }
        for (&target, &distance) in &edges {
            if distance > diameter_edges
                || (distance == diameter_edges && distances[&target] > diameter_m)
            {
                diameter_edges = distance;
                diameter_m = distances[&target];
                endpoints = (source, target);
            }
        }
    }
    (
        component.len(),
        diameter_edges,
        diameter_m,
        endpoints.0,
        endpoints.1,
    )
}

fn root(parents: &mut [usize], index: usize) -> usize {
    if parents[index] != index {
        parents[index] = root(parents, parents[index]);
    }
    parents[index]
}

fn join(parents: &mut [usize], a: usize, b: usize) {
    let (a, b) = (root(parents, a), root(parents, b));
    parents[b] = a;
}
