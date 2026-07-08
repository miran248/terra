use bevy::prelude::{Resource, Vec3};

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

/// Whether a path is a land road or a water bridge (a straight span).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PathKind {
    Road,
    Bridge,
}

/// A road or bridge: a polyline of surface points, pre-sampled so consumers (minimap,
/// mesh painter) can just draw them. Roads wind over land; bridges are straight over water.
#[derive(Clone)]
pub struct Road {
    pub points: Vec<SpherePos>,
    pub kind: PathKind,
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
    /// Roads-first generation: place settlements on habitable flat land, then connect each
    /// town to its nearest neighbour(s) with gentle arcs + slight noise wobble. Terrain
    /// (mountains, forests) is grown *around* these roads afterward, so routing is trivial
    /// — we only need to check the continent layer for water and avoid crossing the sea.
    /// Every town is guaranteed a road to at least its nearest reachable neighbour.
    pub fn generate(terrain: &TerrainGen) -> Self {
        let anchors = terrain.habitable_anchors(SETTLEMENTS, MIN_SEPARATION);
        let settlements: Vec<Settlement> = anchors
            .iter()
            .enumerate()
            .map(|(i, &pos)| Settlement { pos, name: settlement_name(i) })
            .collect();

        let neighbours: Vec<Vec<usize>> = anchors
            .iter()
            .enumerate()
            .map(|(i, a)| {
                let mut others: Vec<(usize, f32)> = anchors
                    .iter()
                    .enumerate()
                    .filter(|(j, _)| *j != i)
                    .map(|(j, b)| (j, a.distance(*b)))
                    .collect();
                others.sort_by(|x, y| x.1.partial_cmp(&y.1).unwrap());
                others.into_iter().map(|(j, _)| j).collect()
            })
            .collect();

        let mut roads = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut link_count = vec![0usize; anchors.len()];

        // Pass 1: every town gets a road to its nearest reachable neighbour (guaranteed).
        for i in 0..anchors.len() {
            if link_count[i] > 0 {
                continue;
            }
            for &j in &neighbours[i] {
                if try_link(terrain, &anchors, &mut roads, &mut seen, &mut link_count, i, j) {
                    break;
                }
            }
        }

        // Pass 2: add redundancy — up to LINKS_PER_SETTLEMENT links per town.
        for i in 0..anchors.len() {
            for &j in &neighbours[i] {
                if link_count[i] >= LINKS_PER_SETTLEMENT {
                    break;
                }
                try_link(terrain, &anchors, &mut roads, &mut seen, &mut link_count, i, j);
            }
        }

        Self { settlements, roads }
    }
}

fn try_link(
    terrain: &TerrainGen,
    anchors: &[SpherePos],
    roads: &mut Vec<Road>,
    seen: &mut std::collections::HashSet<(usize, usize)>,
    link_count: &mut [usize],
    i: usize,
    j: usize,
) -> bool {
    let k = (i.min(j), i.max(j));
    if seen.contains(&k) {
        return true;
    }
    if let Some(road) = build_road(terrain, anchors[i], anchors[j]) {
        roads.push(road);
        seen.insert(k);
        link_count[i] += 1;
        link_count[j] += 1;
        true
    } else {
        false
    }
}

/// Deterministic settlement name: a fixed syllable table indexed by settlement number,
/// so the same seed/order always yields the same names.
fn settlement_name(i: usize) -> String {
    const PRE: [&str; 8] = ["Ash", "Oak", "Stone", "River", "Fair", "Wind", "Cold", "Green"];
    const SUF: [&str; 6] = ["ford", "haven", "bury", "wick", "dale", "hollow"];
    format!("{}{}", PRE[i % PRE.len()], SUF[(i / PRE.len()) % SUF.len()])
}

/// A gently curving road between two towns that stays on land where possible. The arc
/// wobbles organically and is biased away from coastlines (pushed toward higher continent
/// elevation) so roads rarely clip the sea. Only when the straight arc itself crosses
/// a permanent water gap does it fall back to a straight bridge.
fn build_road(terrain: &TerrainGen, a: SpherePos, b: SpherePos) -> Option<Road> {
    let arc = a.distance(b);
    let steps = (arc / SAMPLE_SPACING).ceil().max(1.0) as usize;
    let seed = (a.0 + b.0).normalize();

    // Pre-scan the straight arc: find the minimum continent elevation. If the straight
    // line itself dips below zero, this is a true water gap (a strait/bay) — bridge it.
    let min_straight = (0..=steps)
        .map(|k| terrain.continent_elevation(slerp(a, b, k as f32 / steps as f32)))
        .fold(f32::MAX, f32::min);
    if min_straight < -0.15 {
        let straight: Vec<SpherePos> =
            (0..=steps).map(|k| slerp(a, b, k as f32 / steps as f32)).collect();
        return Some(Road { points: straight, kind: PathKind::Bridge });
    }

    // Land road: wobble organically and push inland away from shallow water.
    let ambient = (min_straight + 1.0) / 2.0; // ~how "landy" the corridor is (0=watery, 1=dry)
    let mut points = Vec::with_capacity(steps + 1);
    for k in 0..=steps {
        let t = k as f32 / steps as f32;
        let mut p = if k == 0 {
            a
        } else if k == steps {
            b
        } else {
            // Wobble amplitude inversely proportional to how close the straight line is
            // to water — a dry corridor can afford more wobble; a coastal one stays tight.
            // Amplitude: up to 80 m for fully dry, only ~25 m near water.
            let amp = 25.0 + ambient * ambient * 55.0;
            wobbled_slerp(a, b, t, seed, amp)
        };
        // Nudge away from water: if this point is near sea level, push it back toward the
        // straight arc centre (which is further from the coast).
        if terrain.continent_elevation(p) < 0.0 {
            p = slerp(a, b, t); // snap back to the straight arc
        }
        points.push(p);
    }
    points[0] = a;
    *points.last_mut().unwrap() = b;

    // Re-check: if the corrected path still dips below the continent, fall back to bridge.
    if points.iter().any(|p| terrain.continent_elevation(*p) < 0.0) {
        let straight: Vec<SpherePos> =
            (0..=steps).map(|k| slerp(a, b, k as f32 / steps as f32)).collect();
        return Some(Road { points: straight, kind: PathKind::Bridge });
    }
    Some(Road { points, kind: PathKind::Road })
}

/// Great-circle interpolation with a multi-frequency lateral wobble for organic curve,
/// displaced perpendicularly to the arc by up to `amplitude` meters.
fn wobbled_slerp(a: SpherePos, b: SpherePos, t: f32, seed: Vec3, amplitude: f32) -> SpherePos {
    let base = slerp(a, b, t);
    let n = a.0.cross(b.0);
    if n.length_squared() < 1e-12 {
        return base;
    }
    let n = n.normalize();
    // Rich wobble: three frequencies give natural-looking variation without sharp turns.
    let phase1 = t * 7.3 + seed.x * 3.1 + seed.y * 5.7;
    let phase2 = t * 4.1 + seed.z * 6.5 + seed.x * 2.3;
    let phase3 = t * 11.7 + seed.y * 4.9 + seed.z * 1.8;
    let wobble = (phase1 as f64).sin() as f32 * 0.6
        + (phase2 as f64).sin() as f32 * 0.3
        + (phase3 as f64).sin() as f32 * 0.1;
    let offset = n * (wobble * amplitude / PLANET_RADIUS);
    SpherePos::new(base.0 + offset)
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
        // Roads stay on the continent (land), checked against the continent layer since
        // mountains are grown around the roads later.
        for (ri, road) in net.roads.iter().enumerate() {
            assert!(road.points.len() >= 2);
            let a = *road.points.first().unwrap();
            let b = *road.points.last().unwrap();
            match road.kind {
                PathKind::Road => {
                    for p in &road.points {
                        assert!(
                            terrain.continent_elevation(*p) > 0.0,
                            "road {ri} ran into the ocean (continent layer)"
                        );
                    }
                }
                PathKind::Bridge => {
                    // A bridge is the straight great-circle span.
                    for (k, p) in road.points.iter().enumerate() {
                        let t = k as f32 / (road.points.len() - 1) as f32;
                        let expected = crate::sphere::slerp(a, b, t);
                        assert!(p.distance(expected) < 1.0, "bridge {ri} not straight");
                    }
                }
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

    #[test]
    fn every_settlement_is_connected() {
        // Roads-first generation connects towns before terrain forms, so every settlement
        // gets a road (island towns still bridge within range).
        for seed in [1u32, 7, 42, 1337, 99] {
            let terrain = TerrainGen::new(seed);
            let net = Roads::generate(&terrain);
            for s in &net.settlements {
                let connected = net.roads.iter().any(|r| {
                    r.points.first().is_some_and(|p| p.distance(s.pos) < 100.0)
                        || r.points.last().is_some_and(|p| p.distance(s.pos) < 100.0)
                });
                assert!(connected, "settlement {} has no road (seed {seed})", s.name);
            }
        }
    }
}
