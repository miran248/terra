use std::collections::{BTreeSet, VecDeque};

use terra_geometry::sphere::SpherePos;
use crate::zones::{ZoneKind, Zones};

/// Dedicated coarse-zone river router. Neighbor order and BFS tie-breaking are
/// inherited directly from the serialized zone adjacency arrays.
pub(crate) fn path_to_water(
    zones: &Zones,
    source_face: usize,
    blocked_faces: &BTreeSet<usize>,
) -> Option<(Vec<SpherePos>, Vec<usize>)> {
    let mut predecessor = vec![usize::MAX; zones.centroids.len()];
    let mut queue = VecDeque::from([source_face]);
    predecessor[source_face] = source_face;
    while let Some(face) = queue.pop_front() {
        if zones.kind_of_face(face).is_water() {
            let mut path = vec![face];
            let mut current = face;
            while current != source_face {
                current = predecessor[current];
                path.push(current);
            }
            path.reverse();
            let waypoints = path
                .iter()
                .map(|&path_face| SpherePos::new(zones.centroids[path_face]))
                .collect();
            return Some((waypoints, path));
        }
        for &neighbor in &zones.adj[face] {
            let neighbor = neighbor as usize;
            if predecessor[neighbor] == usize::MAX
                && !blocked_faces.contains(&neighbor)
                && zones.kind_of_face(neighbor) != ZoneKind::Settlement
            {
                predecessor[neighbor] = face;
                queue.push_back(neighbor);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zones::ZoneConfig;

    #[test]
    fn route_is_deterministic_and_reaches_water() {
        let zones = Zones::generate(1337, &ZoneConfig::default());
        let source = (0..zones.face_count())
            .find(|&face| zones.kind_of_face(face) == ZoneKind::MountainRange)
            .expect("locked seed has mountain faces");
        let blocked = BTreeSet::new();
        let first = path_to_water(&zones, source, &blocked).expect("mountain reaches water");
        let second = path_to_water(&zones, source, &blocked).expect("route is repeatable");

        assert_eq!(first.1, second.1);
        assert_eq!(first.1.first(), Some(&source));
        assert!(zones.kind_of_face(*first.1.last().unwrap()).is_water());
        assert!(first.1.windows(2).all(|edge| {
            zones.adj[edge[0]]
                .iter()
                .any(|neighbor| *neighbor as usize == edge[1])
        }));
    }
}
