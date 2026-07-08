use bevy::prelude::Resource;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

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
/// A* step size along the ground, meters (grid resolution of the route search).
const STEP: f32 = 80.0;
/// Cap A* expansions so routing always terminates even in bad terrain.
const MAX_EXPANSIONS: usize = 16_000;

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

/// Route a road from `a` to `b` along the path of least resistance (A* over a ground
/// grid, cost = distance × terrain `travel_cost`), so roads wind through valleys and
/// around mountains. Returns `None` if no land route is found (e.g. across ocean).
fn build_road(terrain: &TerrainGen, a: SpherePos, b: SpherePos) -> Option<Road> {
    let path = astar(terrain, a, b)?;
    let points = resample(&path);
    // Reject if resampling (straight chords between A* nodes) clipped the sea near a coast.
    if points.iter().any(|p| terrain.surface_radius(*p) < PLANET_RADIUS) {
        return None;
    }
    Some(Road { points })
}

/// Grid key: quantize a direction to a lat/long cell so A* has finite, dedupable nodes.
fn key(p: SpherePos) -> (i32, i32) {
    let cell = STEP / PLANET_RADIUS; // angular step
    let lat = p.0.y.clamp(-1.0, 1.0).acos();
    let lon = p.0.z.atan2(p.0.x) + std::f32::consts::PI;
    ((lat / cell).round() as i32, (lon / cell).round() as i32)
}

#[derive(Clone, Copy)]
struct Node {
    f: f32,
    g: f32,
    pos: SpherePos,
}
impl PartialEq for Node {
    fn eq(&self, o: &Self) -> bool {
        self.f == o.f
    }
}
impl Eq for Node {}
impl PartialOrd for Node {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Node {
    // Reverse so BinaryHeap (max-heap) pops the lowest f first.
    fn cmp(&self, o: &Self) -> Ordering {
        o.f.partial_cmp(&self.f).unwrap_or(Ordering::Equal)
    }
}

fn astar(terrain: &TerrainGen, start: SpherePos, goal: SpherePos) -> Option<Vec<SpherePos>> {
    let mut open = BinaryHeap::new();
    let mut g_score: HashMap<(i32, i32), f32> = HashMap::new();
    let mut came_from: HashMap<(i32, i32), SpherePos> = HashMap::new();
    // Memoize the expensive terrain cost per grid cell (noise sampling dominates).
    let mut cost_cache: HashMap<(i32, i32), f32> = HashMap::new();

    open.push(Node { f: start.distance(goal), g: 0.0, pos: start });
    g_score.insert(key(start), 0.0);

    let mut expansions = 0;
    while let Some(current) = open.pop() {
        if current.pos.distance(goal) <= STEP {
            // Reconstruct, then append the exact goal.
            let mut path = vec![goal];
            let mut k = key(current.pos);
            let mut p = current.pos;
            path.push(p);
            while let Some(&prev) = came_from.get(&k) {
                p = prev;
                k = key(p);
                path.push(p);
                if k == key(start) {
                    break;
                }
            }
            path.reverse();
            return Some(path);
        }

        expansions += 1;
        if expansions > MAX_EXPANSIONS {
            return None;
        }

        let ck = key(current.pos);
        if current.g > *g_score.get(&ck).unwrap_or(&f32::INFINITY) {
            continue; // stale heap entry
        }

        let (east, north) = current.pos.tangent_basis();
        for angle_i in 0..8 {
            let a = angle_i as f32 * std::f32::consts::FRAC_PI_4;
            let tangent = east * a.cos() + north * a.sin();
            let next = SpherePos::new(current.pos.0 + tangent * (STEP / PLANET_RADIUS));
            let nk = key(next);
            let cost = *cost_cache
                .entry(nk)
                .or_insert_with(|| terrain.travel_cost(next));
            if !cost.is_finite() {
                continue; // impassable (water)
            }
            let tentative = current.g + STEP * cost;
            if tentative < *g_score.get(&nk).unwrap_or(&f32::INFINITY) {
                g_score.insert(nk, tentative);
                came_from.insert(nk, current.pos);
                open.push(Node { f: tentative + next.distance(goal), g: tentative, pos: next });
            }
        }
    }
    None
}

/// Resample a polyline of surface points to even ~SAMPLE_SPACING spacing (great-circle
/// interpolation between consecutive nodes) for smooth road rendering.
fn resample(path: &[SpherePos]) -> Vec<SpherePos> {
    let mut out = Vec::new();
    for seg in path.windows(2) {
        let (a, b) = (seg[0], seg[1]);
        let d = a.distance(b);
        let steps = (d / SAMPLE_SPACING).ceil().max(1.0) as usize;
        for k in 0..steps {
            out.push(slerp(a, b, k as f32 / steps as f32));
        }
    }
    if let Some(last) = path.last() {
        out.push(*last);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mean terrain travel cost sampled along a path (finite-only), for comparing routes.
    fn path_cost(terrain: &TerrainGen, pts: impl Iterator<Item = SpherePos>) -> f32 {
        let mut sum = 0.0;
        let mut n = 0.0;
        for p in pts {
            let c = terrain.travel_cost(p);
            if c.is_finite() {
                sum += c;
                n += 1.0;
            }
        }
        if n == 0.0 { f32::INFINITY } else { sum / n }
    }

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
        // Every road point must be on land, and the least-cost route must have no higher
        // total terrain cost than the naive straight line (it's what A* minimises).
        for (ri, road) in net.roads.iter().enumerate() {
            assert!(road.points.len() >= 2);
            for p in &road.points {
                assert!(terrain.surface_radius(*p) >= PLANET_RADIUS, "road ran into the ocean");
            }
            let a = *road.points.first().unwrap();
            let b = *road.points.last().unwrap();
            let route_cost = path_cost(&terrain, road.points.iter().copied());
            let straight: Vec<SpherePos> =
                (0..=80).map(|k| crate::sphere::slerp(a, b, k as f32 / 80.0)).collect();
            let straight_cost = path_cost(&terrain, straight.into_iter());
            assert!(
                route_cost <= straight_cost + 1.0,
                "road {ri} costlier than straight line: route {route_cost} vs straight {straight_cost}"
            );
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
