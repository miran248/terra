use std::collections::BTreeMap;
use std::collections::VecDeque;

use bevy::prelude::Vec3;

use crate::terrain::Terrain;
use terra_geometry::topology::FaceId;
use crate::worldgen::{Grid, elevation};

pub(in crate::worldgen) const RIVER_SURFACE_CLEARANCE: f32 = 0.02;
pub(in crate::worldgen) const RIVER_TERRAIN_CLIP: f32 = 0.25;
pub(in crate::worldgen) const RIVER_SPRING_TAPER_RINGS: usize = 3;

/// Bake a smooth, terrain-following river surface, including spring taper,
/// body-water outlets, and a buried land apron.
pub(in crate::worldgen) fn river_surface_radii(
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
                        .face_neighbors(FaceId::new(face_index))
                        .map(FaceId::index)
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
            && matches!(
                face_types[face_index],
                Terrain::River | Terrain::RiverSpring
            )
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
        let Some(c) = component[face_index] else {
            continue;
        };
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

    // The outside edge of the actual RiverBank must meet the ground. The extra
    // rendering apron remains buried beyond that edge; anchoring only at the
    // apron's outer edge can leave visible water laid across a low downstream
    // bank before the surface finally tapers underground.
    for face_index in 0..grid.face_count() {
        let Some(c) = component[face_index] else {
            continue;
        };
        if !has_river[c.index()] {
            continue;
        }
        for neighbor in grid
            .face_neighbors(FaceId::new(face_index))
            .map(FaceId::index)
        {
            if core[neighbor] {
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
        let Some(c) = component[face_index] else {
            continue;
        };
        if !has_river[c.index()] {
            continue;
        }
        for neighbor in grid
            .face_neighbors(FaceId::new(face_index))
            .map(FaceId::index)
        {
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
        let Some(c) = component[face_index] else {
            continue;
        };
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
