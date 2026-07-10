use bevy::prelude::Vec3;

use crate::sphere::{slerp, SpherePos, PLANET_RADIUS};

/// Spacing between sampled points along a road, meters.
pub const SAMPLE_SPACING: f32 = 40.0;

/// Deterministic settlement name: a fixed syllable table indexed by settlement number,
/// so the same seed/order always yields the same names.
pub fn settlement_name(i: usize) -> String {
    const PRE: [&str; 8] = ["Ash", "Oak", "Stone", "River", "Fair", "Wind", "Cold", "Green"];
    const SUF: [&str; 6] = ["ford", "haven", "bury", "wick", "dale", "hollow"];
    format!("{}{}", PRE[i % PRE.len()], SUF[(i / PRE.len()) % SUF.len()])
}

/// A gently wobbled great-circle polyline between two points — the shape of a road.
pub fn build_land_road_path(a: SpherePos, b: SpherePos, steps: usize, seed: Vec3) -> Vec<SpherePos> {
    (0..=steps).map(|k| {
        let t = k as f32 / steps as f32;
        if k == 0 { a } else if k == steps { b } else { wobbled_slerp(a, b, t, seed, 40.0) }
    }).collect()
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

/// Triangles for a bridge deck over `points` (a shore-to-shore span): a gently
/// arched strip `half_width` meters wide, riding above water and terrain.
/// Bridges are runtime entities — gen_level only records the span polyline.
///
/// `ground` must be the DISPLACED terrain mesh (the one that renders and
/// collides), not the smooth field — the baked mesh is faceted and lifts cliff
/// tops, and a deck built from the smooth field can end below it.
pub fn build_bridge_deck(
    points: &[SpherePos],
    ground: &crate::planet::PlanetMesh,
    half_width: f32,
) -> Vec<[[f32; 3]; 3]> {
    let mut tris = Vec::new();
    if points.len() < 2 {
        return tris;
    }
    let sr = ground.facet_radius(points[0].0, PLANET_RADIUS);
    let er = ground.facet_radius(points[points.len() - 1].0, PLANET_RADIUS);
    let sea = PLANET_RADIUS;
    let spacing = 2.0;
    let mut dirs = Vec::new();
    let mut pos = 0.0f32;
    for seg in points.windows(2) {
        let sl = seg[0].distance(seg[1]);
        while pos <= sl + 1e-6 {
            dirs.push(slerp(seg[0], seg[1], (pos / sl).min(1.0)).0);
            if pos >= sl { break; }
            pos = (pos + spacing).min(sl);
        }
        pos -= sl;
    }
    let last = points.last().unwrap().0;
    if (dirs.last().unwrap().dot(last)).abs() < 0.999 {
        dirs.push(last);
    }
    let n = dirs.len();
    let mut rings: Vec<(Vec3, Vec3)> = Vec::new();
    for i in 0..n {
        let dir = dirs[i];
        let t = i as f32 / (n - 1).max(1) as f32;
        let arch = 2.0 * (4.0 * t * (1.0 - t));
        let base = sr + (er - sr) * t;
        let tr = ground.facet_radius(dir.normalize(), sea).max(sea);
        let r = base.max(tr) + arch;
        let up = dir.normalize();
        let fwd = if i == 0 { (dirs[1] - dirs[0]).normalize() }
        else if i == n - 1 { (dirs[n - 1] - dirs[n - 2]).normalize() }
        else {
            let p = (dirs[i] - dirs[i - 1]).normalize();
            let nx = (dirs[i + 1] - dirs[i]).normalize();
            (p + nx).normalize_or((dirs[1] - dirs[0]).normalize())
        };
        let left = fwd.cross(up).normalize_or(Vec3::X) * (half_width / r);
        rings.push(((dir + left).normalize() * r, (dir - left).normalize() * r));
    }
    for p in rings.windows(2) {
        tris.push([p[0].0.to_array(), p[0].1.to_array(), p[1].0.to_array()]);
        tris.push([p[0].1.to_array(), p[1].1.to_array(), p[1].0.to_array()]);
    }
    tris
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sphere::random_point;

    #[test]
    fn road_path_hits_endpoints_and_stays_near_arc() {
        let a = random_point(0.1, 0.7);
        let b = random_point(0.4, 0.5);
        let path = build_land_road_path(a, b, 32, (a.0 + b.0).normalize());
        assert!(path.first().unwrap().distance(a) < 0.5);
        assert!(path.last().unwrap().distance(b) < 0.5);
        for (k, p) in path.iter().enumerate() {
            let t = k as f32 / (path.len() - 1) as f32;
            let straight = slerp(a, b, t);
            assert!(p.distance(straight) < 60.0, "wobble exceeded amplitude bound");
        }
    }
}
