use bevy::prelude::*;

const SEARCH_RINGS: usize = 6;
const RING_SECTORS: usize = 16;
const VEHICLE_HEADINGS: usize = 8;
const ANCHOR_COUNT: usize = 1 + SEARCH_RINGS * RING_SECTORS;

/// Ordered surface-placement candidates around an origin.
///
/// Anchors are emitted center-first, then by ring and sector. Vehicle searches
/// emit all eight headings at each anchor before advancing to the next one.
#[derive(Clone, Debug)]
pub struct PlacementCandidateSearch {
    origin: Vec3,
    forward: Vec3,
    side: Vec3,
    radius: f32,
    heading_count: usize,
    next_index: usize,
}

impl PlacementCandidateSearch {
    /// Search each anchor with the vehicle's eight 45-degree heading variants.
    pub fn vehicle(origin: Vec3, heading: Vec3, radius: f32) -> Self {
        Self::new(origin, heading, radius, VEHICLE_HEADINGS)
    }

    /// Search each anchor with one fixed heading.
    pub fn fixed_heading(origin: Vec3, heading: Vec3, radius: f32) -> Self {
        Self::new(origin, heading, radius, 1)
    }

    fn new(origin: Vec3, heading: Vec3, radius: f32, heading_count: usize) -> Self {
        let up = origin.normalize();
        let forward = tangent_heading(heading, up);
        Self {
            origin,
            forward,
            side: forward.cross(up),
            radius,
            heading_count,
            next_index: 0,
        }
    }
}

impl Iterator for PlacementCandidateSearch {
    type Item = (Vec3, Vec3);

    fn next(&mut self) -> Option<Self::Item> {
        if self.next_index >= ANCHOR_COUNT * self.heading_count {
            return None;
        }

        let anchor_index = self.next_index / self.heading_count;
        let heading_index = self.next_index % self.heading_count;
        self.next_index += 1;

        let (ring, sector) = if anchor_index == 0 {
            (0, 0)
        } else {
            let index = anchor_index - 1;
            (index / RING_SECTORS + 1, index % RING_SECTORS)
        };
        let distance = self.radius * ring as f32 / SEARCH_RINGS as f32;
        let angle = sector as f32 * std::f32::consts::TAU / RING_SECTORS as f32;
        let candidate =
            self.origin + (self.forward * angle.cos() + self.side * angle.sin()) * distance;
        let heading = Quat::from_axis_angle(
            candidate.normalize(),
            heading_index as f32 * std::f32::consts::TAU / VEHICLE_HEADINGS as f32,
        ) * self.forward;
        Some((candidate, heading))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = ANCHOR_COUNT * self.heading_count - self.next_index;
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for PlacementCandidateSearch {}
impl std::iter::FusedIterator for PlacementCandidateSearch {}

fn tangent_heading(heading: Vec3, up: Vec3) -> Vec3 {
    let projected = heading - up * heading.dot(up);
    if projected.length_squared() > 1e-8 {
        projected.normalize()
    } else {
        up.any_orthonormal_vector()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_close(actual: Vec3, expected: Vec3) {
        assert!(
            actual.distance(expected) < 1e-5,
            "expected {expected:?}, got {actual:?}"
        );
    }

    #[test]
    fn vehicle_candidates_keep_anchor_sector_and_heading_priority() {
        let origin = Vec3::Y * 2000.0;
        let mut candidates = PlacementCandidateSearch::vehicle(origin, Vec3::NEG_Z, 12.0);

        for yaw in 0..VEHICLE_HEADINGS {
            let (position, heading) = candidates.next().unwrap();
            assert_close(position, origin);
            let expected =
                Quat::from_rotation_y(yaw as f32 * std::f32::consts::TAU / 8.0) * Vec3::NEG_Z;
            assert_close(heading, expected);
        }

        let (ring_one, heading) = candidates.next().unwrap();
        assert_close(ring_one, origin + Vec3::NEG_Z * 2.0);
        assert_close(heading, Vec3::NEG_Z);

        for _ in 1..VEHICLE_HEADINGS {
            candidates.next().unwrap();
        }
        let (next_sector, heading) = candidates.next().unwrap();
        let angle = std::f32::consts::TAU / RING_SECTORS as f32;
        let expected_offset = Vec3::NEG_Z * (2.0 * angle.cos()) + Vec3::X * (2.0 * angle.sin());
        assert_close(next_sector, origin + expected_offset);
        assert_close(heading, Vec3::NEG_Z);
    }

    #[test]
    fn candidate_search_has_a_finite_ordered_extent() {
        let origin = Vec3::Y * 2000.0;
        assert_eq!(
            PlacementCandidateSearch::vehicle(origin, Vec3::NEG_Z, 22.0).count(),
            ANCHOR_COUNT * VEHICLE_HEADINGS
        );
        assert_eq!(
            PlacementCandidateSearch::fixed_heading(origin, Vec3::NEG_Z, 30.0).count(),
            ANCHOR_COUNT
        );
    }
}
