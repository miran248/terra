use super::*;
use crate::level::{RoadEndpointRole, SettlementKind, StructureKind};
use bevy::prelude::Vec3;

#[test]
fn default_world_has_configured_settlement_kinds() {
    let level = run(1337, |_| {}).into_level_data();
    let count = |kind| {
        level
            .settlements
            .iter()
            .filter(|settlement| settlement.kind == kind)
            .count()
    };

    assert_eq!(level.settlements.len(), 12);
    assert_eq!(count(SettlementKind::Town), 3);
    assert_eq!(count(SettlementKind::Village), 6);
    assert_eq!(count(SettlementKind::Outpost), 3);
}

#[test]
fn custom_settlement_targets_generate_matching_zones_and_level_data() {
    let config = SettlementConfig {
        towns: 2,
        villages: 3,
        outposts: 2,
        ..SettlementConfig::default()
    };
    let level = run_with_settlement_config(42, config, |_| {}).into_level_data();

    assert_eq!(level.settlements.len(), config.total());
    for kind in [
        SettlementKind::Town,
        SettlementKind::Village,
        SettlementKind::Outpost,
    ] {
        assert_eq!(
            level
                .settlements
                .iter()
                .filter(|settlement| settlement.kind == kind)
                .count(),
            match kind {
                SettlementKind::Town => config.towns,
                SettlementKind::Village => config.villages,
                SettlementKind::Outpost => config.outposts,
            }
        );
    }
    level.validate().unwrap();

    let runtime = TerrainGen::from_field_with_settlement_config(
        level.seed,
        level.vert_elev.clone(),
        level.settlement_config,
    );
    assert_eq!(
        runtime
            .zones()
            .zones_of_kind(crate::zones::ZoneKind::Settlement)
            .count(),
        config.total()
    );
}

#[test]
fn settlement_config_rejects_overflowing_target_counts() {
    let config = SettlementConfig {
        towns: usize::MAX,
        villages: 1,
        outposts: 1,
        ..SettlementConfig::default()
    };
    assert!(config.validate().unwrap_err().contains("overflow"));
}

#[test]
fn each_settlement_has_its_kind_specific_structures_inside_its_footprint() {
    for seed in [42, 1337] {
        let state = run_state(seed, |_| {});
        let level = state.to_level_data();
        let road_cells = state
            .grid
            .topology
            .cells()
            .filter(|&cell| {
                state.painted.roads.contains(cell) || state.painted.bridge_entries.contains(cell)
            })
            .collect::<Vec<_>>();

        for settlement in &level.settlements {
            let center = SpherePos::new(Vec3::from_array(settlement.pos));
            let radius = level.settlement_config.radius_m(settlement.kind);
            let structures = level
                .structures
                .iter()
                .filter(|structure| {
                    let pos = Vec3::from_array(structure.pos).normalize();
                    center.distance(SpherePos::new(pos)) <= radius
                })
                .collect::<Vec<_>>();
            let count = |kind| {
                structures
                    .iter()
                    .filter(|structure| structure.kind == kind)
                    .count()
            };

            match settlement.kind {
                SettlementKind::Town => {
                    assert!(
                        count(StructureKind::House) >= 4,
                        "{} lacks four homes",
                        settlement.name
                    );
                    assert!(
                        count(StructureKind::Well) >= 1,
                        "{} lacks a shared well",
                        settlement.name
                    );
                }
                SettlementKind::Village => {
                    assert!(
                        count(StructureKind::House) >= 2,
                        "{} lacks two homes",
                        settlement.name
                    );
                    assert!(
                        count(StructureKind::Farm) >= 1,
                        "{} lacks a farm",
                        settlement.name
                    );
                }
                SettlementKind::Outpost => {
                    assert!(
                        count(StructureKind::Watchtower) >= 1,
                        "{} lacks a watchtower",
                        settlement.name
                    );
                    assert!(
                        count(StructureKind::Tent) >= 1,
                        "{} lacks shelter",
                        settlement.name
                    );
                    assert!(
                        count(StructureKind::Fence) + count(StructureKind::Barricade) >= 2,
                        "{} lacks defensive barriers",
                        settlement.name
                    );
                }
            }

            for structure in structures {
                let position = Vec3::from_array(structure.pos);
                let triangle = level.terrain_tris[structure.face as usize].map(Vec3::from_array);
                let normal = (triangle[1] - triangle[0])
                    .cross(triangle[2] - triangle[0])
                    .normalize();
                let distance_from_ground = (position - triangle[0]).dot(normal).abs();
                assert!(
                    distance_from_ground < 0.01,
                    "{} {:?} is {distance_from_ground:.2} m from its terrain face",
                    settlement.name,
                    structure.kind
                );
                assert!(
                    (structure.face as usize) < level.face_types.len(),
                    "{} structure references a missing face",
                    settlement.name
                );
                assert!(
                    !level.face_types[structure.face as usize].is_water()
                        && level.slope_class[structure.face as usize].is_walkable(),
                    "{} has a structure on unsuitable ground",
                    settlement.name
                );
                let center = SpherePos::new(Vec3::from_array(structure.pos));
                let nearest_cell = road_cells
                    .iter()
                    .copied()
                    .min_by(|&a, &b| {
                        center
                            .distance(state.grid.cell_position(a))
                            .total_cmp(&center.distance(state.grid.cell_position(b)))
                    })
                    .expect("generated world has road cells");
                let nearest_road = center.distance(state.grid.cell_position(nearest_cell));
                let minimum_clearance =
                    super::super::surface::structure_footprint_radius(structure.kind) + 6.0;
                assert!(
                    nearest_road >= minimum_clearance,
                    "{} {:?} blocks a road: {nearest_road:.1} m clearance, expected {minimum_clearance:.1} m (road={}, bridge entry={})",
                    settlement.name,
                    structure.kind,
                    state.painted.roads.contains(nearest_cell),
                    state.painted.bridge_entries.contains(nearest_cell)
                );
            }
        }

        for (first_index, first) in level.structures.iter().enumerate() {
            let first_position = SpherePos::new(Vec3::from_array(first.pos));
            for second in &level.structures[first_index + 1..] {
                let second_position = SpherePos::new(Vec3::from_array(second.pos));
                let required_involved = level.settlements.iter().any(|settlement| {
                    let anchor = SpherePos::new(Vec3::from_array(settlement.pos));
                    let radius = level.settlement_config.radius_m(settlement.kind);
                    anchor.distance(first_position) <= radius
                        || anchor.distance(second_position) <= radius
                });
                if !required_involved {
                    continue;
                }
                let minimum_clearance =
                    super::super::surface::structure_footprint_radius(first.kind)
                        + super::super::surface::structure_footprint_radius(second.kind);
                assert!(
                    first_position.distance(second_position) >= minimum_clearance,
                    "planned structure {:?} overlaps nearby {:?}",
                    first.kind,
                    second.kind
                );
            }
        }

        for (index, settlement) in level.settlements.iter().enumerate() {
            let anchor = SpherePos::new(Vec3::from_array(settlement.pos));
            let radius = level.settlement_config.radius_m(settlement.kind);
            let internal_paths = state
                .roads
                .iter()
                .filter(|road| road.from_settlement.is_none() && road.to_settlement.is_none())
                .filter(|road| {
                    road.cells.iter().all(|&cell| {
                        anchor.distance(state.grid.cell_position(cell)) <= radius + 6.0
                    })
                })
                .collect::<Vec<_>>();
            let mut adjacency = std::collections::BTreeMap::<
                terra_geometry::topology::CellId,
                std::collections::BTreeSet<terra_geometry::topology::CellId>,
            >::new();
            for road in &internal_paths {
                for pair in road.cells.windows(2) {
                    adjacency.entry(pair[0]).or_default().insert(pair[1]);
                    adjacency.entry(pair[1]).or_default().insert(pair[0]);
                }
            }
            let edge_count = adjacency
                .values()
                .map(std::collections::BTreeSet::len)
                .sum::<usize>()
                / 2;
            let junction_count = adjacency
                .values()
                .filter(|neighbors| neighbors.len() >= 3)
                .count();
            let mut assigned = std::collections::BTreeSet::new();
            let mut component_count = 0;
            for &start in adjacency.keys() {
                if !assigned.insert(start) {
                    continue;
                }
                component_count += 1;
                let mut pending = vec![start];
                while let Some(cell) = pending.pop() {
                    for &neighbor in &adjacency[&cell] {
                        if assigned.insert(neighbor) {
                            pending.push(neighbor);
                        }
                    }
                }
            }
            let cycle_rank = edge_count + component_count - adjacency.len();
            let path_summary = internal_paths
                .iter()
                .map(|road| {
                    (
                        road.cells.len(),
                        road.cells.first().map(|cell| cell.index()),
                        road.cells.last().map(|cell| cell.index()),
                    )
                })
                .collect::<Vec<_>>();
            match settlement.kind {
                SettlementKind::Town => assert!(
                    internal_paths.len() >= 3 && junction_count >= 2 && cycle_rank >= 1,
                    "town {index} should have a small street grid, found {} internal paths, {junction_count} junctions and cycle rank {cycle_rank}, paths {path_summary:?}",
                    internal_paths.len()
                ),
                SettlementKind::Village => assert!(
                    internal_paths.len() >= 2 && junction_count >= 1,
                    "village {index} should have a main street and branch, found {} internal paths and {junction_count} junctions, paths {path_summary:?}",
                    internal_paths.len()
                ),
                SettlementKind::Outpost => assert!(
                    !internal_paths.is_empty() && component_count == 1,
                    "outpost {index} should have a connected access spine"
                ),
            }

            let entrance_count = level
                .road_endpoints
                .iter()
                .filter(|endpoint| {
                    endpoint.roles.iter().any(|role| {
                        matches!(
                            role,
                            RoadEndpointRole::SettlementEntrance { settlement_index }
                                if *settlement_index as usize == index
                        )
                    })
                })
                .count();
            assert!(
                entrance_count > 0,
                "{} has no named road entrance",
                settlement.name
            );
        }
    }
}
