use bevy::prelude::Vec3;

use crate::sphere::{PLANET_RADIUS, SpherePos, slerp};

/// Spacing between sampled points along a road, meters.
pub const SAMPLE_SPACING: f32 = 40.0;

/// Return the world-space width needed to keep a surface ribbon readable under
/// a perspective camera, while preserving at least the requested road width.
/// The viewport height and desired width use the same pixel units.
pub fn projected_surface_ribbon_width(
    camera_depth_m: f32,
    vertical_fov_radians: f32,
    viewport_height_pixels: f32,
    desired_width_pixels: f32,
    minimum_width_m: f32,
) -> Option<f32> {
    if !camera_depth_m.is_finite()
        || camera_depth_m <= 0.0
        || !vertical_fov_radians.is_finite()
        || vertical_fov_radians <= 0.0
        || vertical_fov_radians >= std::f32::consts::PI
        || !viewport_height_pixels.is_finite()
        || viewport_height_pixels <= 0.0
        || !desired_width_pixels.is_finite()
        || desired_width_pixels <= 0.0
        || !minimum_width_m.is_finite()
        || minimum_width_m < 0.0
    {
        return None;
    }

    let world_width =
        2.0 * camera_depth_m * (vertical_fov_radians * 0.5).tan() * desired_width_pixels
            / viewport_height_pixels;
    world_width
        .is_finite()
        .then_some(world_width.max(minimum_width_m))
}

/// A sampled road-center point on the render surface and its outward surface normal.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceRoadSample {
    pub center: Vec3,
    pub normal: Vec3,
}

/// Resample a route and place it on displaced terrain or an actual bridge deck.
/// Raised samples come from intersections with the generated deck top surface,
/// so a road highlight follows the same arch used by the bridge renderer.
pub fn sample_surface_road_path(
    points: &[SpherePos],
    ground: &crate::planet::PlanetMesh,
    raised_surface: Option<&[[[f32; 3]; 3]]>,
    spacing_m: f32,
    surface_lift_m: f32,
) -> Vec<SurfaceRoadSample> {
    if points.len() < 2 || !spacing_m.is_finite() || spacing_m <= 0.0 || !surface_lift_m.is_finite()
    {
        return Vec::new();
    }

    let mut directions = Vec::new();
    for pair in points.windows(2) {
        let angle = pair[0].0.dot(pair[1].0).clamp(-1.0, 1.0).acos();
        let steps = ((angle * PLANET_RADIUS / spacing_m).ceil() as usize).max(1);
        for step in 0..steps {
            let t = step as f32 / steps as f32;
            directions.push(slerp(pair[0], pair[1], t).0.normalize_or(Vec3::Y));
        }
    }
    directions.push(points.last().unwrap().0.normalize_or(Vec3::Y));

    directions
        .into_iter()
        .filter_map(|direction| {
            let mut raised_hit = None;
            if let Some(surface) = raised_surface {
                for triangle in surface {
                    let triangle = triangle.map(Vec3::from_array);
                    if let Some(distance) = crate::planet::ray_triangle_intersection_distance(
                        Vec3::ZERO,
                        direction,
                        &triangle,
                    ) && raised_hit.is_none_or(|(best, _): (f32, Vec3)| distance > best)
                    {
                        let mut normal = (triangle[1] - triangle[0])
                            .cross(triangle[2] - triangle[0])
                            .normalize_or_zero();
                        if normal.dot(direction) < 0.0 {
                            normal = -normal;
                        }
                        raised_hit = Some((distance, normal));
                    }
                }
            }

            let (radius, normal) = raised_hit.unwrap_or_else(|| {
                let radius = ground.facet_radius(direction, PLANET_RADIUS);
                let normal = ground
                    .face_at(direction)
                    .and_then(|face| ground.triangle(face))
                    .map(|triangle| {
                        let mut normal = (triangle[1] - triangle[0])
                            .cross(triangle[2] - triangle[0])
                            .normalize_or_zero();
                        if normal.dot(direction) < 0.0 {
                            normal = -normal;
                        }
                        normal
                    })
                    .filter(|normal| normal.length_squared() > 0.5)
                    .unwrap_or(direction);
                (radius, normal)
            });

            let normal = normal.normalize_or(direction);
            let center = direction * radius + normal * surface_lift_m;
            (center.is_finite() && normal.is_finite())
                .then_some(SurfaceRoadSample { center, normal })
        })
        .collect()
}

/// Build a depth-tested ribbon from sampled terrain or deck centers.
/// `widths_m` supplies the full world-space width at each sample, allowing a
/// renderer to preserve a chosen screen width as perspective depth changes.
pub fn build_surface_road_ribbon(
    samples: &[SurfaceRoadSample],
    widths_m: &[f32],
) -> Vec<[[f32; 3]; 3]> {
    if samples.len() < 2
        || widths_m.len() != samples.len()
        || samples.iter().any(|sample| {
            !sample.center.is_finite()
                || !sample.normal.is_finite()
                || sample.normal.length_squared() < 1e-8
        })
        || widths_m
            .iter()
            .any(|width| !width.is_finite() || *width <= 0.0)
    {
        return Vec::new();
    }

    let mut edges = Vec::with_capacity(samples.len());
    for (index, sample) in samples.iter().enumerate() {
        let normal = sample.normal.normalize();
        let previous = samples[index.saturating_sub(1)].center;
        let next = samples[(index + 1).min(samples.len() - 1)].center;
        let tangent = (next - previous).reject_from(normal).normalize_or_zero();
        if tangent.length_squared() < 1e-8 {
            return Vec::new();
        }
        let side = normal.cross(tangent).normalize_or_zero();
        if side.length_squared() < 1e-8 {
            return Vec::new();
        }
        let half_width = widths_m[index] * 0.5;
        edges.push([
            sample.center + side * half_width,
            sample.center - side * half_width,
        ]);
    }

    let mut triangles = Vec::with_capacity((samples.len() - 1) * 2);
    for pair in edges.windows(2) {
        let [left, right] = pair[0];
        let [next_left, next_right] = pair[1];
        triangles.push([left.to_array(), right.to_array(), next_left.to_array()]);
        triangles.push([
            right.to_array(),
            next_right.to_array(),
            next_left.to_array(),
        ]);
    }
    triangles
}

/// Deterministic settlement name: a fixed syllable table indexed by settlement number,
/// so the same seed/order always yields the same names.
pub fn settlement_name(i: usize) -> String {
    const PRE: [&str; 8] = [
        "Ash", "Oak", "Stone", "River", "Fair", "Wind", "Cold", "Green",
    ];
    const SUF: [&str; 6] = ["ford", "haven", "bury", "wick", "dale", "hollow"];
    format!("{}{}", PRE[i % PRE.len()], SUF[(i / PRE.len()) % SUF.len()])
}

/// A gently wobbled great-circle polyline between two points — the shape of a road.
pub fn build_land_road_path(
    a: SpherePos,
    b: SpherePos,
    steps: usize,
    seed: Vec3,
) -> Vec<SpherePos> {
    (0..=steps)
        .map(|k| {
            let t = k as f32 / steps as f32;
            if k == 0 {
                a
            } else if k == steps {
                b
            } else {
                wobbled_slerp(a, b, t, seed, 40.0)
            }
        })
        .collect()
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
#[derive(Default)]
pub struct BridgeDeckGeometry {
    /// Complete solid deck mesh: top, underside, sides, and end caps.
    pub triangles: Vec<[[f32; 3]; 3]>,
    /// Top-facing triangles from the same geometry used by the collider and renderer.
    pub top_surface: Vec<[[f32; 3]; 3]>,
}

pub fn build_bridge_deck(
    points: &[SpherePos],
    ground: &crate::planet::PlanetMesh,
    half_width: f32,
) -> Vec<[[f32; 3]; 3]> {
    build_bridge_deck_geometry(points, ground, half_width).triangles
}

pub fn build_bridge_deck_geometry(
    points: &[SpherePos],
    ground: &crate::planet::PlanetMesh,
    half_width: f32,
) -> BridgeDeckGeometry {
    let mut deck = BridgeDeckGeometry::default();
    if points.len() < 2 {
        return deck;
    }
    // Resample the centreline UNIFORMLY by arc length: perfectly even rings
    // (no per-segment leftover, no tiny planks at joints), so the planks line
    // up and the arch reads smooth.
    let seglen: Vec<f32> = points.windows(2).map(|w| w[0].distance(w[1])).collect();
    let total: f32 = seglen.iter().sum();
    if total < 1e-3 {
        return deck;
    }
    let spacing = 1.5;
    let n = ((total / spacing).round() as usize).max(2) + 1;
    let mut dirs = Vec::with_capacity(n);
    for i in 0..n {
        let mut want = total * i as f32 / (n - 1) as f32;
        // Find the segment holding `want` and interpolate within it.
        let mut si = 0;
        while si + 1 < seglen.len() && want > seglen[si] {
            want -= seglen[si];
            si += 1;
        }
        // Clamp to the last real segment (float rounding on the final sample).
        let si = si.min(seglen.len() - 1);
        let f = if seglen[si] > 1e-6 {
            (want / seglen[si]).clamp(0.0, 1.0)
        } else {
            0.0
        };
        dirs.push(slerp(points[si], points[si + 1], f).0);
    }

    let span_len = points[0].distance(*points.last().unwrap());
    let clearance = (span_len * 0.05).clamp(2.5, 10.0); // arch rise at mid-span
    let thickness = 1.4; // deck slab depth (a solid, not a ribbon)
    let embed = 1.5; // sink the grounded ends into the terrain — no float

    // Per-ring lateral frame (fwd → left offset), reused for the end grounding.
    let ring_left = |i: usize, r: f32| -> Vec3 {
        let up = dirs[i].normalize();
        let fwd = if i == 0 {
            (dirs[1] - dirs[0]).normalize()
        } else if i == n - 1 {
            (dirs[n - 1] - dirs[n - 2]).normalize()
        } else {
            ((dirs[i] - dirs[i - 1]).normalize() + (dirs[i + 1] - dirs[i]).normalize())
                .normalize_or((dirs[1] - dirs[0]).normalize())
        };
        fwd.cross(up).normalize_or(Vec3::X) * (half_width / r)
    };
    // The deck is ONE smooth convex arch: the two end radii are grounded just
    // under the LOWEST terrain across each end ring (so the flat ends never
    // clip on sloped ground), and a single parabola arches between them. No
    // separate entrance ramp — one curve end to end.
    let end_ground = |i: usize| -> f32 {
        let l = ring_left(i, PLANET_RADIUS);
        let ld = (dirs[i] + l).normalize();
        let rd = (dirs[i] - l).normalize();
        ground
            .facet_radius(ld, PLANET_RADIUS)
            .min(ground.facet_radius(rd, PLANET_RADIUS))
            .min(ground.facet_radius(dirs[i], PLANET_RADIUS))
            - embed
    };
    let sr = end_ground(0);
    let er = end_ground(n - 1);

    let mut rings: Vec<[Vec3; 4]> = Vec::with_capacity(n);
    for (i, _) in dirs.iter().enumerate().take(n) {
        let t = i as f32 / (n - 1) as f32;
        let r = sr + (er - sr) * t + clearance * (4.0 * t * (1.0 - t));
        let left = ring_left(i, r);
        let ld = (dirs[i] + left).normalize();
        let rd = (dirs[i] - left).normalize();
        rings.push([ld * r, rd * r, ld * (r - thickness), rd * (r - thickness)]);
    }

    let mut quad = |a: Vec3, b: Vec3, c: Vec3, d: Vec3| {
        let triangles = [
            [a.to_array(), c.to_array(), b.to_array()],
            [b.to_array(), c.to_array(), d.to_array()],
        ];
        deck.triangles.extend(triangles);
        triangles
    };
    for w in rings.windows(2) {
        let (r0, r1) = (w[0], w[1]);
        // top, bottom, and the two side walls — a solid slab.
        deck.top_surface.extend(quad(r0[0], r0[1], r1[0], r1[1])); // top
        quad(r0[3], r0[2], r1[3], r1[2]); // bottom
        quad(r0[1], r0[3], r1[1], r1[3]); // right side
        quad(r0[2], r0[0], r1[2], r1[0]); // left side
    }
    // End caps close the slab.
    let f = &rings[0];
    quad(f[1], f[0], f[3], f[2]);
    let l = &rings[n - 1];
    quad(l[0], l[1], l[2], l[3]);
    deck
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sphere::random_point;
    use bevy::camera::CameraProjection;

    #[test]
    fn bridge_slab_faces_point_outward_in_both_endpoint_orders() {
        let ground = crate::planet::PlanetMesh::new(crate::planet::unit_icosphere_tris(3));
        let endpoints = [
            SpherePos::new(Vec3::new(-0.01, 1.0, 0.0)),
            SpherePos::new(Vec3::new(0.01, 1.0, 0.0)),
        ];
        for points in [endpoints, [endpoints[1], endpoints[0]]] {
            let deck = build_bridge_deck_geometry(&points, &ground, 4.0);
            let top_vertices: Vec<_> = deck.top_surface.iter().flatten().copied().collect();
            let mut faces = [0; 6];
            for tri in &deck.triangles {
                let [a, b, c] = tri.map(Vec3::from_array);
                let center = (a + b + c) / 3.0;
                let normal = (b - a).cross(c - a).normalize();
                let top_count = tri.iter().filter(|v| top_vertices.contains(v)).count();
                let (face, outward) = if top_count == 3 {
                    (0, center.normalize())
                } else if top_count == 0 {
                    (1, -center.normalize())
                } else if tri.iter().all(|v| v[2] > 0.0) {
                    (2, Vec3::Z)
                } else if tri.iter().all(|v| v[2] < 0.0) {
                    (3, Vec3::NEG_Z)
                } else if center.x > 0.0 {
                    (4, Vec3::X)
                } else {
                    (5, Vec3::NEG_X)
                };
                assert!(
                    normal.dot(outward) > 0.9,
                    "inward slab face {face}: {tri:?}"
                );
                faces[face] += 1;
            }
            assert!(
                faces.iter().all(|count| *count > 0),
                "all six faces covered"
            );
        }
    }

    #[test]
    fn bridge_deck_builds_without_panicking() {
        // Guards the resampling off-by-one that panicked at runtime.
        let ground = crate::planet::PlanetMesh::new(crate::planet::unit_icosphere_tris(3));
        for steps in [2usize, 3, 5, 17, 33, 50] {
            let a = random_point(0.2, 0.3);
            let b = random_point(0.25, 0.32);
            let span: Vec<SpherePos> = (0..=steps)
                .map(|k| slerp(a, b, k as f32 / steps as f32))
                .collect();
            let deck = build_bridge_deck(&span, &ground, 4.0);
            assert!(!deck.is_empty(), "empty deck for {steps} steps");
            for t in &deck {
                for v in t {
                    assert!(v.iter().all(|c| c.is_finite()), "non-finite deck vertex");
                }
            }
        }
    }

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
            assert!(
                p.distance(straight) < 60.0,
                "wobble exceeded amplitude bound"
            );
        }
    }

    #[test]
    fn projected_road_highlight_width_preserves_its_screen_width() {
        let depth = 4_000.0;
        let projection = bevy::prelude::PerspectiveProjection {
            fov: std::f32::consts::FRAC_PI_3,
            aspect_ratio: 16.0 / 9.0,
            ..Default::default()
        };
        let viewport_height = 900.0;
        let desired_pixels = 2.5;
        let width = projected_surface_ribbon_width(
            depth,
            projection.fov,
            viewport_height,
            desired_pixels,
            4.0,
        )
        .expect("valid perspective inputs should produce a width");
        let clip_from_view = projection.get_clip_from_view();
        let left = clip_from_view.project_point3(Vec3::new(-width * 0.5, 0.0, -depth));
        let right = clip_from_view.project_point3(Vec3::new(width * 0.5, 0.0, -depth));
        let projected_pixels = (right.x - left.x).abs() * viewport_height * 16.0 / 9.0 * 0.5;

        assert!(width >= 4.0);
        assert!((projected_pixels - desired_pixels).abs() < 0.01);
    }

    #[test]
    fn bridge_road_samples_follow_the_generated_deck_height() {
        let ground = crate::planet::PlanetMesh::new(crate::planet::unit_icosphere_tris(3));
        let span = [
            SpherePos::new(Vec3::new(-0.04, 1.0, 0.0)),
            SpherePos::new(Vec3::new(0.04, 1.0, 0.0)),
        ];
        let deck = build_bridge_deck_geometry(&span, &ground, 4.0);
        let samples = sample_surface_road_path(&span, &ground, Some(&deck.top_surface), 2.0, 0.15);

        assert!(samples.len() > 2);
        let middle = samples[samples.len() / 2];
        let ground_radius = ground.facet_radius(middle.center.normalize(), PLANET_RADIUS);
        assert!(
            middle.center.length() > ground_radius + 2.0,
            "bridge highlight must stay above the terrain under the deck"
        );
        let deck_radius = deck
            .top_surface
            .iter()
            .filter_map(|triangle| {
                crate::planet::ray_triangle_intersection_distance(
                    Vec3::ZERO,
                    Vec3::Y,
                    &triangle.map(Vec3::from_array),
                )
            })
            .max_by(f32::total_cmp)
            .expect("midspan ray should cross the generated deck");
        assert!((middle.center.length() - (deck_radius + 0.15)).abs() < 0.05);
        assert!(middle.normal.dot(middle.center.normalize()) > 0.9);
    }

    #[test]
    fn terrain_road_samples_stay_on_the_displaced_surface() {
        let ground = crate::planet::PlanetMesh::new(crate::planet::unit_icosphere_tris(3));
        let path = [
            SpherePos::new(Vec3::new(-0.15, 1.0, 0.0)),
            SpherePos::new(Vec3::new(0.15, 1.0, 0.0)),
        ];
        let lift = 0.22;
        let samples = sample_surface_road_path(&path, &ground, None, 8.0, lift);

        assert!(samples.len() > 2);
        for sample in samples {
            let surface_point = sample.center - sample.normal * lift;
            let direction = surface_point.normalize();
            let surface_radius = ground.facet_radius(direction, PLANET_RADIUS);
            assert!((surface_point.length() - surface_radius).abs() < 0.05);
            assert!(sample.normal.dot(direction) > 0.9);
        }
    }

    #[test]
    fn surface_road_ribbon_keeps_requested_width_on_the_sampled_surface() {
        let samples = [
            SurfaceRoadSample {
                center: Vec3::new(0.0, PLANET_RADIUS + 0.2, 0.0),
                normal: Vec3::Y,
            },
            SurfaceRoadSample {
                center: Vec3::new(0.0, PLANET_RADIUS + 0.2, 20.0),
                normal: Vec3::Y,
            },
        ];
        let triangles = build_surface_road_ribbon(&samples, &[6.0, 6.0]);

        assert_eq!(triangles.len(), 2);
        let [left, right, next_left] = triangles[0].map(Vec3::from_array);
        assert!((left.distance(right) - 6.0).abs() < 0.01);
        assert!(((left + right) * 0.5).distance(samples[0].center) < 0.01);
        assert!(
            (right - left)
                .cross(next_left - left)
                .normalize()
                .dot(Vec3::Y)
                > 0.99
        );
    }
}
