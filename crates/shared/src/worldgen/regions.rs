use std::collections::VecDeque;

use super::Grid;
use crate::level::RegionKind;
use crate::topology::FaceId;

/// Typed face partitioner for named region overlays.
pub(super) struct RegionPartitioner<'a> {
    grid: &'a Grid,
    class: &'a [Option<RegionKind>],
    terrain_class: &'a [Option<RegionKind>],
}

impl<'a> RegionPartitioner<'a> {
    pub(super) fn new(
        grid: &'a Grid,
        class: &'a [Option<RegionKind>],
        terrain_class: &'a [Option<RegionKind>],
    ) -> Self {
        Self {
            grid,
            class,
            terrain_class,
        }
    }

    /// Claims one edge-connected cluster. Matching terrain beneath a road or
    /// town can be traversed as a connector without being claimed.
    pub(super) fn claim(
        &self,
        start: FaceId,
        kind: RegionKind,
        region_index: u32,
        max_faces: usize,
        face_region: &mut [Option<u32>],
    ) -> Vec<FaceId> {
        let overlay_kind = matches!(kind, RegionKind::Road | RegionKind::Town);
        let mut faces = vec![start];
        let mut queue = VecDeque::from([start]);
        let mut visited_connector = vec![false; self.grid.face_count()];
        face_region[start.index()] = Some(region_index);
        while let Some(current) = queue.pop_front() {
            if faces.len() >= max_faces {
                break;
            }
            for neighbor in self.grid.topology.face_neighbors(current) {
                let index = neighbor.index();
                if self.class[index] == Some(kind) && face_region[index].is_none() {
                    face_region[index] = Some(region_index);
                    faces.push(neighbor);
                    queue.push_back(neighbor);
                } else if !overlay_kind
                    && self.class[index] != Some(kind)
                    && self.terrain_class[index] == Some(kind)
                    && !visited_connector[index]
                {
                    visited_connector[index] = true;
                    queue.push_back(neighbor);
                }
            }
        }
        faces
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_connector_is_traversed_but_not_claimed() {
        let grid = Grid::new(1337);
        let start = grid.topology.face(0).unwrap();
        let connector = grid.topology.face_neighbors(start)[0];
        let beyond = grid
            .topology
            .face_neighbors(connector)
            .into_iter()
            .find(|face| *face != start)
            .unwrap();
        let mut class = vec![None; grid.face_count()];
        let mut terrain_class = vec![None; grid.face_count()];
        class[start.index()] = Some(RegionKind::Plains);
        class[connector.index()] = Some(RegionKind::Road);
        class[beyond.index()] = Some(RegionKind::Plains);
        terrain_class[start.index()] = Some(RegionKind::Plains);
        terrain_class[connector.index()] = Some(RegionKind::Plains);
        terrain_class[beyond.index()] = Some(RegionKind::Plains);
        let mut assignments = vec![None; grid.face_count()];

        let faces = RegionPartitioner::new(&grid, &class, &terrain_class).claim(
            start,
            RegionKind::Plains,
            1,
            usize::MAX,
            &mut assignments,
        );

        assert!(faces.contains(&start));
        assert!(faces.contains(&beyond));
        assert_eq!(assignments[connector.index()], None);
    }
}
