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
    // Ground height at the two grounded ends; the deck is a SMOOTH CONVEX arch
    // between them (a straight ground line + a parabola), sampling terrain only
    // at the ends — so no per-face steps, no stair-stepped planks, and the top
    // is convex by construction.
    let sr = ground.facet_radius(points[0].0, PLANET_RADIUS);
    let er = ground.facet_radius(points[points.len() - 1].0, PLANET_RADIUS);

    // Resample the centreline UNIFORMLY by arc length: perfectly even rings
    // (no per-segment leftover, no tiny planks at joints), so the planks line
    // up and the arch reads smooth.
    let seglen: Vec<f32> = points.windows(2).map(|w| w[0].distance(w[1])).collect();
    let total: f32 = seglen.iter().sum();
    if total < 1e-3 {
        return tris;
    }
    let spacing = 1.5;
    let n = ((total / spacing).round() as usize).max(2) + 1;
    let mut dirs = Vec::with_capacity(n);
    for i in 0..n {
        let mut want = total * i as f32 / (n - 1) as f32;
        // Find the segment holding `want` and interpolate within it.
        let mut si = 0;
        while si + 1 < points.len() && want > seglen[si] {
            want -= seglen[si];
            si += 1;
        }
        let f = if seglen[si] > 1e-6 { (want / seglen[si]).clamp(0.0, 1.0) } else { 0.0 };
        dirs.push(slerp(points[si], points[si + 1], f).0);
    }

    let span_len = points[0].distance(*points.last().unwrap());
    let clearance = (span_len * 0.05).clamp(2.5, 10.0); // arch rise at mid-span
    let thickness = 1.4; // deck slab depth (a solid, not a ribbon)
    let embed = 1.5;     // sink the grounded ends into the terrain — no float

    // Per-ring frame: top-left/right and bottom-left/right world points.
    let mut rings: Vec<[Vec3; 4]> = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 / (n - 1) as f32;
        let up = dirs[i].normalize();
        let ground_r = sr + (er - sr) * t;
        let arch = clearance * (4.0 * t * (1.0 - t));
        // End rings dip below the ground line so the deck grounds INTO the
        // terrain rather than floating over a slope.
        // Smooth cosine dip only in the outer ~15% at each end.
        let edge = (1.0 - (2.0 * t - 1.0).abs()).min(0.15) / 0.15; // 0 at ends, 1 inward
        let end_dip = embed * (1.0 - edge) * (1.0 - edge);
        let top_r = ground_r + arch - end_dip;
        let fwd = if i == 0 {
            (dirs[1] - dirs[0]).normalize()
        } else if i == n - 1 {
            (dirs[n - 1] - dirs[n - 2]).normalize()
        } else {
            ((dirs[i] - dirs[i - 1]).normalize() + (dirs[i + 1] - dirs[i]).normalize())
                .normalize_or((dirs[1] - dirs[0]).normalize())
        };
        let left = fwd.cross(up).normalize_or(Vec3::X) * (half_width / top_r);
        let tl = (dirs[i] + left).normalize() * top_r;
        let tr_ = (dirs[i] - left).normalize() * top_r;
        let bl = (dirs[i] + left).normalize() * (top_r - thickness);
        let br = (dirs[i] - left).normalize() * (top_r - thickness);
        rings.push([tl, tr_, bl, br]);
    }

    let mut quad = |a: Vec3, b: Vec3, c: Vec3, d: Vec3| {
        tris.push([a.to_array(), b.to_array(), c.to_array()]);
        tris.push([b.to_array(), d.to_array(), c.to_array()]);
    };
    for w in rings.windows(2) {
        let (r0, r1) = (w[0], w[1]);
        // top, bottom, and the two side walls — a solid slab.
        quad(r0[0], r0[1], r1[0], r1[1]); // top
        quad(r0[3], r0[2], r1[3], r1[2]); // bottom
        quad(r0[1], r0[3], r1[1], r1[3]); // right side
        quad(r0[2], r0[0], r1[2], r1[0]); // left side
    }
    // End caps close the slab.
    let f = &rings[0];
    quad(f[1], f[0], f[3], f[2]);
    let l = &rings[n - 1];
    quad(l[0], l[1], l[2], l[3]);
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
