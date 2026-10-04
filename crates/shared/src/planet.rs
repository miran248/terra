use bevy::prelude::{Resource, Vec3};

use crate::sphere::PLANET_RADIUS;

/// The displaced planet triangles (world-space corners), so actors can rest on the exact
/// faceted surface the mesh renders — not the smooth heightmap, which disagrees between
/// the low-poly vertices and would let actors clip through the visible ground.
///
/// Triangles are bucketed by centroid direction into a lat/long grid so `facet_radius`
/// only tests the few triangles near a query direction instead of the whole mesh.
#[derive(Resource)]
pub struct PlanetMesh {
    tris: Vec<[Vec3; 3]>,
    /// grid[lat_bucket * LON_BUCKETS + lon_bucket] -> triangle indices whose centroid
    /// falls in that cell (plus neighbours, so edge-straddling tris are found).
    grid: Vec<Vec<u32>>,
    /// Finer buckets keep the high-volume surface-projection queries bounded while
    /// `grid` retains the historical candidate ordering used by `face_at`.
    facet_grid: Vec<Vec<u32>>,
}

const LAT_BUCKETS: usize = 64;
const LON_BUCKETS: usize = 128;
const FACET_LAT_BUCKETS: usize = 256;
const FACET_LON_BUCKETS: usize = 512;
const ROAD_PROJECTION_BARYCENTRIC_EPSILON: f32 = 1e-5;

fn bucket(dir: Vec3) -> (usize, usize) {
    bucket_with_resolution(dir, LAT_BUCKETS, LON_BUCKETS)
}

fn bucket_with_resolution(dir: Vec3, lat_buckets: usize, lon_buckets: usize) -> (usize, usize) {
    let d = dir.normalize();
    // lat in [0, pi] from +Y, lon in [0, 2pi).
    let lat = d.y.clamp(-1.0, 1.0).acos(); // 0 at +Y pole .. pi at -Y
    let lon = d.z.atan2(d.x) + std::f32::consts::PI; // 0..2pi
    let li = ((lat / std::f32::consts::PI) * lat_buckets as f32) as usize;
    let oi = ((lon / std::f32::consts::TAU) * lon_buckets as f32) as usize;
    (li.min(lat_buckets - 1), oi.min(lon_buckets - 1))
}

fn build_grid(tris: &[[Vec3; 3]], lat_buckets: usize, lon_buckets: usize) -> Vec<Vec<u32>> {
    let mut grid = vec![Vec::new(); lat_buckets * lon_buckets];
    for (idx, triangle) in tris.iter().enumerate() {
        let centroid = (triangle[0] + triangle[1] + triangle[2]) / 3.0;
        let (lat, lon) = bucket_with_resolution(centroid, lat_buckets, lon_buckets);
        // Insert into the cell and its 8 neighbours (wrap longitude, clamp latitude)
        // so a query near a cell edge still finds triangles from the adjacent cell.
        for dlat in [-1i32, 0, 1] {
            for dlon in [-1i32, 0, 1] {
                let candidate_lat = lat as i32 + dlat;
                if candidate_lat < 0 || candidate_lat >= lat_buckets as i32 {
                    continue;
                }
                let candidate_lon = (lon as i32 + dlon).rem_euclid(lon_buckets as i32);
                grid[candidate_lat as usize * lon_buckets + candidate_lon as usize]
                    .push(idx as u32);
            }
        }
    }
    grid
}

impl PlanetMesh {
    pub fn new(tris: Vec<[Vec3; 3]>) -> Self {
        let grid = build_grid(&tris, LAT_BUCKETS, LON_BUCKETS);
        let facet_grid = build_grid(&tris, FACET_LAT_BUCKETS, FACET_LON_BUCKETS);
        Self {
            tris,
            grid,
            facet_grid,
        }
    }

    pub fn triangle(&self, face: usize) -> Option<&[Vec3; 3]> {
        self.tris.get(face)
    }

    pub fn triangle_count(&self) -> usize {
        self.tris.len()
    }

    pub fn tris(&self) -> &Vec<[Vec3; 3]> {
        &self.tris
    }

    /// Index of the surface triangle the ray from the planet centre through `dir` hits
    /// (the nearest hit — a heightmapped ray can cross several triangles, we want the one
    /// on the surface facing outward), or `None` if not found.
    pub fn face_at(&self, dir: Vec3) -> Option<usize> {
        let (li, oi) = bucket(dir);
        let mut best: Option<(usize, f32)> = None;
        for &idx in &self.grid[li * LON_BUCKETS + oi] {
            if let Some(r) = ray_triangle_radius(dir, &self.tris[idx as usize])
                && best.is_none_or(|(_, br)| r > br)
            {
                best = Some((idx as usize, r)); // outermost hit = the visible surface
            }
        }
        if let Some((idx, _)) = best {
            return Some(idx);
        }
        // Grid miss: full scan for the outermost hit.
        let mut fallback: Option<(usize, f32)> = None;
        for (idx, t) in self.tris.iter().enumerate() {
            if let Some(r) = ray_triangle_radius(dir, t)
                && fallback.is_none_or(|(_, br)| r > br)
            {
                fallback = Some((idx, r));
            }
        }
        fallback.map(|(idx, _)| idx)
    }

    /// World radius of the rendered facet along direction `dir` (unit vector): intersect
    /// the ray `t*dir` with the triangle whose spherical cell contains `dir`. Uses the
    /// grid bucket so only a handful of candidate triangles are tested.
    pub fn facet_radius(&self, dir: Vec3, fallback: f32) -> f32 {
        self.facet_radius_from_grid(dir, fallback, &self.grid, LAT_BUCKETS, LON_BUCKETS, 0.0)
    }

    /// Radius query used when projecting wide road ribbons onto terrain. It keeps the
    /// general terrain lookup above unchanged and narrows the candidate set for the
    /// many lateral samples a ribbon needs.
    pub fn facet_radius_for_road_projection(&self, dir: Vec3, fallback: f32) -> f32 {
        self.facet_radius_from_grid(
            dir,
            fallback,
            &self.facet_grid,
            FACET_LAT_BUCKETS,
            FACET_LON_BUCKETS,
            ROAD_PROJECTION_BARYCENTRIC_EPSILON,
        )
    }

    fn facet_radius_from_grid(
        &self,
        dir: Vec3,
        fallback: f32,
        grid: &[Vec<u32>],
        lat_buckets: usize,
        lon_buckets: usize,
        barycentric_epsilon: f32,
    ) -> f32 {
        let (li, oi) = bucket_with_resolution(dir, lat_buckets, lon_buckets);
        for &idx in &grid[li * lon_buckets + oi] {
            if let Some(r) = ray_triangle_radius_with_tolerance(
                dir,
                &self.tris[idx as usize],
                barycentric_epsilon,
            ) {
                return r;
            }
        }
        // Keep the exact-surface fallback for large or boundary-crossing facets that
        // extend beyond their centroid's neighbouring buckets.
        for t in &self.tris {
            if let Some(r) = ray_triangle_radius_with_tolerance(dir, t, barycentric_epsilon) {
                return r;
            }
        }
        fallback
    }
}

/// If a ray from `origin` along the unit `direction` passes through triangle
/// `t`, return the distance from `origin` to the hit.
pub fn ray_triangle_intersection_distance(
    origin: Vec3,
    direction: Vec3,
    t: &[Vec3; 3],
) -> Option<f32> {
    ray_triangle_distance_from_offset(direction, origin - t[0], t)
}

/// If the ray from the origin along `dir` passes through triangle `t`, return the radius
/// (distance from origin to the hit). Möller–Trumbore, origin at planet center.
pub fn ray_triangle_radius(dir: Vec3, t: &[Vec3; 3]) -> Option<f32> {
    ray_triangle_distance_from_offset(dir, -t[0], t)
}

fn ray_triangle_radius_with_tolerance(
    dir: Vec3,
    triangle: &[Vec3; 3],
    barycentric_epsilon: f32,
) -> Option<f32> {
    ray_triangle_distance_from_offset_with_tolerance(
        dir,
        -triangle[0],
        triangle,
        barycentric_epsilon,
    )
}

fn ray_triangle_distance_from_offset(
    direction: Vec3,
    origin_to_triangle: Vec3,
    triangle: &[Vec3; 3],
) -> Option<f32> {
    ray_triangle_distance_from_offset_with_tolerance(direction, origin_to_triangle, triangle, 0.0)
}

fn ray_triangle_distance_from_offset_with_tolerance(
    direction: Vec3,
    origin_to_triangle: Vec3,
    triangle: &[Vec3; 3],
    barycentric_epsilon: f32,
) -> Option<f32> {
    let e1 = triangle[1] - triangle[0];
    let e2 = triangle[2] - triangle[0];
    let p = direction.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-6 {
        return None;
    }
    let inv = 1.0 / det;
    let u = origin_to_triangle.dot(p) * inv;
    if u < -barycentric_epsilon || u > 1.0 + barycentric_epsilon {
        return None;
    }
    let q = origin_to_triangle.cross(e1);
    let v = direction.dot(q) * inv;
    if v < -barycentric_epsilon || u + v > 1.0 + barycentric_epsilon {
        return None;
    }
    let dist = e2.dot(q) * inv;
    (dist > 0.0).then_some(dist)
}

/// Face adjacency by shared edge: adj[fi] = the 3 neighbouring face indices.
pub fn build_face_adjacency(tris: &[[Vec3; 3]], n: usize) -> Vec<[u32; 3]> {
    let mut em: std::collections::BTreeMap<[u64; 2], Vec<u32>> = Default::default();
    for (fi, tri) in tris.iter().enumerate() {
        let fi = fi as u32;
        for (x, y) in [(0, 1), (1, 2), (2, 0)] {
            let (ka, kb) = (hv(tri[x]), hv(tri[y]));
            em.entry(if ka <= kb { [ka, kb] } else { [kb, ka] })
                .or_default()
                .push(fi);
        }
    }
    let mut adj = vec![[u32::MAX; 3]; n];
    for (fi, tri) in tris.iter().enumerate() {
        let mut k = 0;
        for (x, y) in [(0, 1), (1, 2), (2, 0)] {
            let (ka, kb) = (hv(tri[x]), hv(tri[y]));
            if let Some(ns) = em.get(&if ka <= kb { [ka, kb] } else { [kb, ka] }) {
                for &n in ns {
                    if n as usize != fi && k < 3 {
                        adj[fi][k] = n;
                        k += 1;
                        break;
                    }
                }
            }
        }
    }
    adj
}

fn hv(v: Vec3) -> u64 {
    let a = v.to_array();
    a[0].to_bits() as u64
        ^ (a[1].to_bits() as u64).wrapping_mul(6364136223846793005)
        ^ (a[2].to_bits() as u64).wrapping_mul(1442695040888963407)
}

/// Build the sphere's base triangles (undisplaced), for tests/benches without Bevy meshes.
pub fn unit_icosphere_tris(subdivisions: usize) -> Vec<[Vec3; 3]> {
    // Simple recursive icosahedron subdivision, scaled to PLANET_RADIUS.
    let t = (1.0 + 5.0_f32.sqrt()) / 2.0;
    let mut verts = [
        Vec3::new(-1.0, t, 0.0),
        Vec3::new(1.0, t, 0.0),
        Vec3::new(-1.0, -t, 0.0),
        Vec3::new(1.0, -t, 0.0),
        Vec3::new(0.0, -1.0, t),
        Vec3::new(0.0, 1.0, t),
        Vec3::new(0.0, -1.0, -t),
        Vec3::new(0.0, 1.0, -t),
        Vec3::new(t, 0.0, -1.0),
        Vec3::new(t, 0.0, 1.0),
        Vec3::new(-t, 0.0, -1.0),
        Vec3::new(-t, 0.0, 1.0),
    ];
    for v in &mut verts {
        *v = v.normalize();
    }
    let faces: [[usize; 3]; 20] = [
        [0, 11, 5],
        [0, 5, 1],
        [0, 1, 7],
        [0, 7, 10],
        [0, 10, 11],
        [1, 5, 9],
        [5, 11, 4],
        [11, 10, 2],
        [10, 7, 6],
        [7, 1, 8],
        [3, 9, 4],
        [3, 4, 2],
        [3, 2, 6],
        [3, 6, 8],
        [3, 8, 9],
        [4, 9, 5],
        [2, 4, 11],
        [6, 2, 10],
        [8, 6, 7],
        [9, 8, 1],
    ];
    let mut tris: Vec<[Vec3; 3]> = faces
        .iter()
        .map(|f| [verts[f[0]], verts[f[1]], verts[f[2]]])
        .collect();
    for _ in 0..subdivisions {
        let mut next = Vec::with_capacity(tris.len() * 4);
        for [a, b, c] in tris {
            let ab = ((a + b) / 2.0).normalize();
            let bc = ((b + c) / 2.0).normalize();
            let ca = ((c + a) / 2.0).normalize();
            next.push([a, ab, ca]);
            next.push([b, bc, ab]);
            next.push([c, ca, bc]);
            next.push([ab, bc, ca]);
        }
        tris = next;
    }
    for tri in &mut tris {
        for v in tri.iter_mut() {
            *v *= PLANET_RADIUS;
        }
    }
    tris
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facet_radius_matches_full_scan() {
        let tris = unit_icosphere_tris(3);
        let mesh = PlanetMesh::new(tris.clone());
        // For many directions, the grid lookup must agree with a brute-force scan.
        for i in 0..500 {
            let u = (i as f32 * 0.6180339) % 1.0;
            let v = (i as f32 * 0.7548776) % 1.0;
            let theta = u * std::f32::consts::TAU;
            let z = v * 2.0 - 1.0;
            let r = (1.0 - z * z).max(0.0).sqrt();
            let dir = Vec3::new(r * theta.cos(), z, r * theta.sin());

            let grid = mesh.facet_radius(dir, -1.0);
            let brute = tris
                .iter()
                .find_map(|t| ray_triangle_radius(dir, t))
                .unwrap_or(-1.0);
            assert!((grid - brute).abs() < 1e-2, "grid {grid} != brute {brute}");
        }
    }

    #[test]
    fn road_projection_index_matches_full_scan_with_fewer_candidates() {
        let tris = unit_icosphere_tris(5);
        let mesh = PlanetMesh::new(tris.clone());
        let mut coarse_candidate_total = 0usize;
        let mut road_candidate_total = 0usize;

        for i in 0..512 {
            let u = (i as f32 * 0.6180339) % 1.0;
            let v = (i as f32 * 0.7548776) % 1.0;
            let theta = u * std::f32::consts::TAU;
            let y = v * 2.0 - 1.0;
            let radial = (1.0 - y * y).max(0.0).sqrt();
            let direction = Vec3::new(radial * theta.cos(), y, radial * theta.sin());

            let (coarse_lat, coarse_lon) = bucket(direction);
            coarse_candidate_total += mesh.grid[coarse_lat * LON_BUCKETS + coarse_lon].len();
            let (road_lat, road_lon) =
                bucket_with_resolution(direction, FACET_LAT_BUCKETS, FACET_LON_BUCKETS);
            road_candidate_total += mesh.facet_grid[road_lat * FACET_LON_BUCKETS + road_lon].len();

            let expected = tris
                .iter()
                .find_map(|triangle| ray_triangle_radius(direction, triangle))
                .expect("the closed test sphere intersects every direction");
            let actual = mesh.facet_radius_for_road_projection(direction, -1.0);
            assert!(
                (actual - expected).abs() < 1e-2,
                "road index radius {actual} != full-scan radius {expected}"
            );
        }

        assert!(
            road_candidate_total * 4 < coarse_candidate_total,
            "road index candidates {road_candidate_total} should be far below coarse candidates {coarse_candidate_total}"
        );
    }

    #[test]
    fn ray_triangle_intersection_distance_handles_oblique_rays_and_misses() {
        let triangle = [
            Vec3::new(-10.0, -10.0, 10.0),
            Vec3::new(10.0, -10.0, 10.0),
            Vec3::new(0.0, 10.0, 10.0),
        ];
        let origin = Vec3::new(0.0, 0.0, 0.0);
        let direction = Vec3::new(1.0, 0.0, 10.0).normalize();

        let distance = ray_triangle_intersection_distance(origin, direction, &triangle)
            .expect("oblique ray crosses the triangle");
        assert!((distance - 101.0_f32.sqrt()).abs() < 1e-5);
        assert_eq!(
            ray_triangle_intersection_distance(Vec3::new(0.0, 20.0, 0.0), Vec3::Z, &triangle),
            None
        );
    }

    #[test]
    fn offset_ray_collision_predicate_keeps_its_strict_triangle_bounds() {
        let triangle = [
            Vec3::new(-10.0, -10.0, 10.0),
            Vec3::new(10.0, -10.0, 10.0),
            Vec3::new(0.0, 10.0, 10.0),
        ];
        let origin = Vec3::new(1.5, -3.0, 2.0);
        let near_vertex = Vec3::new(0.0, 10.00001, 10.0);
        let direction = (near_vertex - origin).normalize();

        assert_eq!(
            ray_triangle_intersection_distance(origin, direction, &triangle),
            None,
            "road-only edge tolerance must not change offset rays used by collision queries"
        );
    }

    #[test]
    fn road_projection_tolerance_recovers_both_sides_of_a_shared_edge() {
        let shared_a = Vec3::new(-10.0, -10.0, 10.0);
        let shared_b = Vec3::new(10.0, 10.0, 10.0);
        let first = [shared_a, Vec3::new(10.0, -10.0, 10.0), shared_b];
        let second = [shared_a, shared_b, Vec3::new(-10.0, 10.0, 10.0)];
        let direction = Vec3::new(0.0, 0.00001, 10.0).normalize();
        let mesh = PlanetMesh::new(vec![first, second]);

        let road_distance = mesh.facet_radius_for_road_projection(direction, -1.0);
        let expected = 10.0 / direction.z;

        assert!((road_distance - expected).abs() < 0.001);
    }

    #[test]
    fn ray_triangle_intersection_rejects_points_outside_barycentric_tolerance() {
        let triangle = [
            Vec3::new(-10.0, -10.0, 10.0),
            Vec3::new(10.0, -10.0, 10.0),
            Vec3::new(0.0, 10.0, 10.0),
        ];
        let origin = Vec3::ZERO;
        let outside = Vec3::new(0.0, 10.01, 10.0);
        let direction = outside.normalize();

        assert_eq!(
            ray_triangle_intersection_distance(origin, direction, &triangle),
            None
        );
        assert_eq!(
            PlanetMesh::new(vec![triangle]).facet_radius_for_road_projection(direction, -1.0),
            -1.0
        );
    }
}
