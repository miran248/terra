use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::level::{RoadData, RoadEndpointData, RoadEndpointRole, RoadKind, SlopeClass};
use crate::sphere::SpherePos;
use crate::terrain::{Terrain, TerrainGen};
use crate::topology::CellId;

use super::{
    CellSet, Grid, Painted, RoadPath, TOWN_RADIUS, features, link_feature_pinches, nearest_cell,
    router,
};

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
    let blocked = |cell: CellId| !features::roadable(cells[cell.index()], slopes[cell.index()]);
    let unsafe_edges = unsafe_road_edges(grid, blocked);
    terrain
        .zones()
        .zones_of_kind(crate::zones::ZoneKind::Settlement)
        .zip(&terrain.settlement_anchors)
        .enumerate()
        .map(|(settlement_index, ((zone_id, zone), &anchor))| {
            let mut candidates = zone
                .faces
                .iter()
                .map(|&face| SpherePos::new(terrain.zones().centroids[face as usize]))
                .collect::<Vec<_>>();
            candidates.sort_by(|a, b| {
                anchor
                    .distance(*a)
                    .total_cmp(&anchor.distance(*b))
                    .then_with(|| a.0.x.total_cmp(&b.0.x))
                    .then_with(|| a.0.y.total_cmp(&b.0.y))
                    .then_with(|| a.0.z.total_cmp(&b.0.z))
            });
            if let Some(candidate) = candidates
                .into_iter()
                .find(|&candidate| {
                    local_road_diameter(grid, cells, slopes, candidate, &unsafe_edges) >= 2
                })
            {
                return Ok(candidate);
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
            fine_candidates.sort_by(|a, b| {
                anchor
                    .distance(*a)
                    .total_cmp(&anchor.distance(*b))
                    .then_with(|| a.0.x.total_cmp(&b.0.x))
                    .then_with(|| a.0.y.total_cmp(&b.0.y))
                    .then_with(|| a.0.z.total_cmp(&b.0.z))
            });
            fine_candidates
                .into_iter()
                .find(|&candidate| {
                    local_road_diameter(grid, cells, slopes, candidate, &unsafe_edges) >= 2
                })
                .ok_or_else(|| {
                    format!(
                        "settlement {settlement_index} in zone {zone_id} has no roadable site with a safe multi-edge road corridor inside a {} m footprint",
                        TOWN_RADIUS
                    )
                })
        })
        .collect()
}

fn local_road_diameter(
    grid: &Grid,
    cells: &[Terrain],
    slopes: &[SlopeClass],
    anchor: SpherePos,
    unsafe_edges: &BTreeSet<(CellId, CellId)>,
) -> usize {
    let candidates = grid
        .topology
        .cells()
        .filter(|&cell| anchor.distance(grid.cell_position(cell)) <= TOWN_RADIUS)
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
pub(super) fn connect_settlements(
    grid: &Grid,
    cells: &[Terrain],
    slopes: &[SlopeClass],
    settlements: &[SpherePos],
    painted: &mut Painted,
    roads: &mut Vec<RoadPath>,
) -> Vec<Option<CellId>> {
    let blocked = |cell: CellId| !features::roadable(cells[cell.index()], slopes[cell.index()]);
    let extra = |cell: CellId| match slopes[cell.index()] {
        SlopeClass::Flat => 0,
        SlopeClass::Gentle => 400,
        _ => 4000,
    };
    let unsafe_edges = unsafe_road_edges(grid, blocked);
    let mut land_groups = RoadableGroups::new(grid, blocked, &unsafe_edges);
    let entrance_candidates =
        settlement_entrance_candidates(grid, cells, slopes, settlements, &painted.towns);
    let anchors = entrance_candidates
        .iter()
        .map(|candidates| {
            candidates
                .iter()
                .copied()
                .find(|cell| roads.iter().any(|road| road.cells.contains(cell)))
                .or_else(|| candidates.first().copied())
        })
        .collect::<Vec<_>>();
    let mut pairs = Vec::new();
    for (from_index, from) in anchors.iter().enumerate() {
        let Some(from) = *from else { continue };
        let Some(group) = land_groups.cell(from) else {
            continue;
        };
        for (to_index, to) in anchors.iter().enumerate().skip(from_index + 1) {
            let Some(to) = *to else { continue };
            if land_groups.cell(to) == Some(group) {
                pairs.push((
                    settlements[from_index].distance(settlements[to_index]),
                    from_index as u32,
                    to_index as u32,
                    from,
                    to,
                ));
            }
        }
    }
    pairs.sort_by(|a, b| {
        a.0.total_cmp(&b.0)
            .then_with(|| (a.1, a.2).cmp(&(b.1, b.2)))
    });

    let mut components = CellComponents::new(grid.cell_count());
    for road in roads.iter() {
        components.connect_path(&road.cells);
    }
    for (_, from_settlement, to_settlement, from, to) in pairs {
        if components.same(from, to) {
            continue;
        }
        let mut path = router::lattice_path_with_edge(
            grid,
            blocked,
            extra,
            |from, to| unsafe_edges.contains(&(from, to)),
            from,
            to,
        );
        let mut reversed = false;
        if path.len() < 2 {
            path = router::lattice_path_with_edge(
                grid,
                blocked,
                extra,
                |from, to| unsafe_edges.contains(&(from, to)),
                to,
                from,
            );
            reversed = true;
        }
        if path.len() < 2 {
            continue;
        }
        let Some(band) = safe_road_band(grid, &path, blocked) else {
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
        });
    }
    for (settlement_index, (&entrance, candidates)) in
        anchors.iter().zip(&entrance_candidates).enumerate()
    {
        let Some(entrance) = entrance else { continue };
        if roads.iter().any(|road| road.cells.contains(&entrance)) {
            continue;
        }
        let Some(group) = land_groups.cell(entrance) else {
            continue;
        };
        let Some(target) = candidates
            .iter()
            .copied()
            .filter(|&cell| cell != entrance && land_groups.cell(cell) == Some(group))
            .max_by(|&a, &b| {
                grid.cell_position(a)
                    .distance(grid.cell_position(entrance))
                    .total_cmp(&grid.cell_position(b).distance(grid.cell_position(entrance)))
                    .then_with(|| b.cmp(&a))
            })
        else {
            continue;
        };
        let mut path = router::lattice_path_with_edge(
            grid,
            blocked,
            extra,
            |from, to| unsafe_edges.contains(&(from, to)),
            entrance,
            target,
        );
        if path.len() < 2 {
            path = router::lattice_path_with_edge(
                grid,
                blocked,
                extra,
                |from, to| unsafe_edges.contains(&(from, to)),
                target,
                entrance,
            );
        }
        if path.len() < 2 {
            continue;
        }
        let Some(band) = safe_road_band(grid, &path, blocked) else {
            continue;
        };
        components.connect_path(&path);
        for cell in band {
            painted.roads.insert(cell);
        }
        roads.push(RoadPath {
            cells: path,
            from_settlement: Some(settlement_index as u32),
            to_settlement: Some(settlement_index as u32),
        });
    }
    link_feature_pinches(grid, &mut painted.roads, |cell| !blocked(cell));
    anchors
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
pub(super) fn connect_bridges(
    grid: &Grid,
    cells: &[Terrain],
    slopes: &[SlopeClass],
    settlement_entrances: &[Option<CellId>],
    candidates: Vec<Vec<SpherePos>>,
    painted: &mut Painted,
    roads: &mut Vec<RoadPath>,
) -> Vec<Vec<SpherePos>> {
    let blocked = |cell: CellId| !features::roadable(cells[cell.index()], slopes[cell.index()]);
    let extra = |cell: CellId| match slopes[cell.index()] {
        SlopeClass::Flat => 0,
        SlopeClass::Gentle => 400,
        _ => 4000,
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
        let mut staged = Vec::new();
        let mut staged_bands = Vec::new();
        let mut connectable = true;
        for entrance in [from, to] {
            let Some(entrance_component) = roadable_groups.cell(entrance) else {
                connectable = false;
                break;
            };
            if roads
                .iter()
                .chain(&staged)
                .any(|road: &RoadPath| road.cells.contains(&entrance))
            {
                continue;
            }
            let mut targets = BTreeMap::new();
            for cell in roads
                .iter()
                .chain(&staged)
                .flat_map(|road| road.cells.iter().copied())
            {
                targets.insert(cell, None);
            }
            for (index, &cell) in settlement_entrances.iter().enumerate() {
                if let Some(cell) = cell {
                    targets.entry(cell).or_insert(Some(index as u32));
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
                staged.push(RoadPath {
                    cells: chain,
                    from_settlement: None,
                    to_settlement: settlement_index,
                });
                staged_bands.push(band);
            } else if !roads
                .iter()
                .chain(&staged)
                .any(|road: &RoadPath| road.cells.contains(&entrance))
            {
                connectable = false;
                break;
            }
        }
        if connectable {
            roads.extend(staged);
            for band in staged_bands {
                for cell in band {
                    painted.roads.insert(cell);
                }
            }
            for cell in crate::worldgen::cell_chain(grid, &span) {
                painted.bridges.insert(cell);
            }
            for end in [span.first(), span.last()].into_iter().flatten() {
                let Some(face_index) = grid.planet.face_at(end.0) else {
                    continue;
                };
                for cell in grid.face_cells(crate::topology::FaceId::new(face_index)) {
                    if cells[cell.index()].is_land() {
                        painted.bridge_entries.insert(cell);
                    }
                }
            }
            bridges.push(span);
        }
    }
    link_feature_pinches(grid, &mut painted.roads, |cell| !blocked(cell));
    bridges
}

/// Pick a roadable cell in each painted settlement footprint for its entrance.
fn settlement_entrance_candidates(
    grid: &Grid,
    cells: &[Terrain],
    slopes: &[SlopeClass],
    settlements: &[SpherePos],
    town_cells: &CellSet,
) -> Vec<Vec<CellId>> {
    settlements
        .iter()
        .map(|&settlement| {
            let mut candidates = grid
                .topology
                .cells()
                .filter(|&cell| {
                    town_cells.contains(cell)
                        && settlement.distance(grid.cell_position(cell)) <= TOWN_RADIUS
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

pub(super) fn build_road_graph(
    grid: &Grid,
    roads: &[RoadPath],
    bridges: &[Vec<SpherePos>],
    settlement_entrances: &[Option<CellId>],
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
