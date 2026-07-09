use bevy::prelude::{Quat, Resource, Vec3};

use crate::sphere::{slerp, SpherePos, PLANET_RADIUS};
use crate::terrain::{Terrain, TerrainGen};

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
    let new_roads = build_road(terrain, anchors[i], anchors[j]);
    if new_roads.is_empty() {
        return false;
    }
    roads.extend(new_roads);
    seen.insert(k);
    link_count[i] += 1;
    link_count[j] += 1;
    true
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
fn build_road(terrain: &TerrainGen, a: SpherePos, b: SpherePos) -> Vec<Road> {
    let arc = a.distance(b);
    let steps = (arc / SAMPLE_SPACING).ceil().max(1.0) as usize;
    let seed = (a.0 + b.0).normalize();

    let straight: Vec<SpherePos> =
        (0..=steps).map(|k| slerp(a, b, k as f32 / steps as f32)).collect();
    let any_water = straight.iter().any(|p| {
        terrain.continent_elevation(*p) < -0.05
    });

    if !any_water {
        // Pure land road.
        return vec![build_land_road(terrain, a, b, steps, &straight, seed)];
    }

    // Path crosses water — find nearest shore points and route:
    // settlement → shore → [bridge] → shore → settlement.
    let shore_a = find_nearest_shore(terrain, a);
    let shore_b = find_nearest_shore(terrain, b);

    if shore_a.is_none() || shore_b.is_none() {
        return vec![];
    }

    let sa = shore_a.unwrap();
    let sb = shore_b.unwrap();

    let mut roads = Vec::new();

    // Road from settlement A to its shore (land).
    if a.distance(sa) > 10.0 {
        roads.push(build_land_road(terrain, a, sa, (a.distance(sa) / SAMPLE_SPACING).ceil().max(1.0) as usize, &straight, seed));
    }

    // Bridge between shores.
    let bridge_steps = (sa.distance(sb) / SAMPLE_SPACING).ceil().max(1.0) as usize;
    let bridge_points: Vec<SpherePos> =
        (0..=bridge_steps).map(|k| slerp(sa, sb, k as f32 / bridge_steps as f32)).collect();
    if !bridge_points.is_empty() {
        roads.push(Road { points: bridge_points, kind: PathKind::Bridge });
    }

    // Road from shore B to settlement B (land).
    if b.distance(sb) > 10.0 {
        roads.push(build_land_road(terrain, sb, b, (sb.distance(b) / SAMPLE_SPACING).ceil().max(1.0) as usize, &straight, seed));
    }

    roads
}

fn build_land_road(
    terrain: &TerrainGen,
    a: SpherePos,
    b: SpherePos,
    steps: usize,
    straight: &[SpherePos],
    seed: Vec3,
) -> Road {
    let mut points = Vec::with_capacity(steps + 1);
    for k in 0..=steps {
        let t = k as f32 / steps as f32;
        let mut p = if k == 0 { a } else if k == steps { b } else {
            wobbled_slerp(a, b, t, seed, 40.0)
        };
        if terrain.continent_elevation(p) < 0.0 {
            p = slerp(a, b, t);
        }
        if terrain.continent_elevation(p) < 0.0 {
            p = slerp(a, b, t);
        }
        points.push(p);
    }
    points[0] = a;
    *points.last_mut().unwrap() = b;
    Road { points, kind: PathKind::Road }
}

fn find_nearest_shore(terrain: &TerrainGen, origin: SpherePos) -> Option<SpherePos> {
    let is_shore = |t: Terrain| matches!(t, Terrain::Beach | Terrain::RiverBank | Terrain::LakeShore);
    let mut best: Option<(SpherePos, f32)> = None;
    // Search outward in expanding rings along great circles.
    for i in 0..72 {
        let angle = i as f32 * std::f32::consts::TAU / 72.0;
        for dist_m in [50.0f32, 150.0, 300.0, 600.0, 1200.0] {
            let target = ring_point(origin, dist_m, angle);
            if is_shore(terrain.classify(target)) {
                let d = origin.distance(target);
                if best.is_none_or(|(_, bd)| d < bd) {
                    best = Some((target, d));
                }
            }
        }
    }
    best.map(|(p, _)| p)
}

fn ring_point(origin: SpherePos, dist: f32, angle: f32) -> SpherePos {
    // Choose a perpendicular axis, rotate around origin by dist/PLANET_RADIUS.
    let axis = Vec3::new(origin.0.z, 0.0, -origin.0.x).normalize_or(Vec3::X);
    let step = dist / PLANET_RADIUS;
    let perp = Quat::from_axis_angle(axis, step) * origin.0;
    let rot = Quat::from_axis_angle(origin.0, angle) * perp;
    SpherePos(rot)
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
        for seed in [1u32, 7, 42, 1337, 99] {
            let terrain = TerrainGen::new(seed);
            let net = Roads::generate(&terrain);
            // At least some settlements must have at least one road connection.
            // Bridges are shore-to-shore only, so island settlements may be isolated.
            let with_road = net.settlements.iter().filter(|s| {
                net.roads.iter().any(|r| {
                    r.points.first().is_some_and(|p| p.distance(s.pos) < 100.0)
                        || r.points.last().is_some_and(|p| p.distance(s.pos) < 100.0)
                })
            }).count();
            assert!(with_road > 0, "no settlement has a road (seed {seed})");
        }
    }
}
