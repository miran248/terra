use bevy::prelude::Resource;

use crate::sphere::{slerp, SpherePos, PLANET_RADIUS};
use crate::terrain::TerrainGen;

/// How many settlement anchors to place.
const SETTLEMENTS: usize = 12;
/// Minimum arc-distance between settlements, meters.
const MIN_SEPARATION: f32 = 900.0;
/// Each settlement links to its nearest neighbours, up to this many.
const LINKS_PER_SETTLEMENT: usize = 2;
/// Spacing between sampled points along a road, meters.
const SAMPLE_SPACING: f32 = 40.0;

/// A road is a polyline of surface points (a great-circle arc between two settlements),
/// pre-sampled so consumers (minimap, future in-world rendering) can just draw the points.
#[derive(Clone)]
pub struct Road {
    pub points: Vec<SpherePos>,
}

/// A named place on the planet — the anchor for a future village/town/farm.
#[derive(Clone)]
pub struct Settlement {
    pub pos: SpherePos,
    pub name: String,
}

/// The planet's settlements and the road network connecting them. Deterministic per seed.
#[derive(Resource, Default)]
pub struct Roads {
    pub settlements: Vec<Settlement>,
    pub roads: Vec<Road>,
}

impl Roads {
    /// Build settlements on habitable land and connect nearby ones with roads that
    /// stay on land (segments crossing ocean are rejected).
    pub fn generate(terrain: &TerrainGen) -> Self {
        let anchors = terrain.habitable_anchors(SETTLEMENTS, MIN_SEPARATION);
        let settlements: Vec<Settlement> = anchors
            .iter()
            .enumerate()
            .map(|(i, &pos)| Settlement { pos, name: settlement_name(i) })
            .collect();

        let mut roads = Vec::new();
        let mut seen = std::collections::HashSet::new();

        for (i, a) in anchors.iter().enumerate() {
            // Nearest neighbours by arc distance.
            let mut others: Vec<(usize, f32)> = anchors
                .iter()
                .enumerate()
                .filter(|(j, _)| *j != i)
                .map(|(j, b)| (j, a.distance(*b)))
                .collect();
            others.sort_by(|x, y| x.1.partial_cmp(&y.1).unwrap());

            for &(j, _) in others.iter().take(LINKS_PER_SETTLEMENT) {
                let key = (i.min(j), i.max(j));
                if !seen.insert(key) {
                    continue; // link already built from the other end
                }
                if let Some(road) = build_road(terrain, *a, anchors[j]) {
                    roads.push(road);
                }
            }
        }

        Self { settlements, roads }
    }
}

/// Deterministic settlement name: a fixed syllable table indexed by settlement number,
/// so the same seed/order always yields the same names.
fn settlement_name(i: usize) -> String {
    const PRE: [&str; 8] = ["Ash", "Oak", "Stone", "River", "Fair", "Wind", "Cold", "Green"];
    const SUF: [&str; 6] = ["ford", "haven", "bury", "wick", "dale", "hollow"];
    format!("{}{}", PRE[i % PRE.len()], SUF[(i / PRE.len()) % SUF.len()])
}

/// Sample a great-circle road between two points; returns `None` if it would run over
/// ocean (a road can't cross the sea). No bridges/hydrology yet.
fn build_road(terrain: &TerrainGen, a: SpherePos, b: SpherePos) -> Option<Road> {
    let arc = a.distance(b);
    let steps = (arc / SAMPLE_SPACING).ceil().max(1.0) as usize;
    let mut points = Vec::with_capacity(steps + 1);
    for k in 0..=steps {
        let t = k as f32 / steps as f32;
        let p = slerp(a, b, t);
        // Reject the whole road if any sample dips below sea level.
        if terrain.surface_radius(p) < PLANET_RADIUS {
            return None;
        }
        points.push(p);
    }
    Some(Road { points })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roads_stay_on_land_and_connect_settlements() {
        let terrain = TerrainGen::new(1337);
        let net = Roads::generate(&terrain);
        assert!(!net.settlements.is_empty(), "should place some settlements");
        // Every settlement must be habitable and named.
        for s in &net.settlements {
            assert!(terrain.is_habitable(s.pos));
            assert!(!s.name.is_empty());
        }
        // Every road point must be on land (never below sea level).
        for road in &net.roads {
            assert!(road.points.len() >= 2);
            for p in &road.points {
                assert!(terrain.surface_radius(*p) >= PLANET_RADIUS, "road ran into the ocean");
            }
        }
    }

    #[test]
    fn deterministic_per_seed() {
        let t = TerrainGen::new(7);
        let a = Roads::generate(&t);
        let b = Roads::generate(&t);
        assert_eq!(a.settlements.len(), b.settlements.len());
        assert_eq!(a.roads.len(), b.roads.len());
    }
}
