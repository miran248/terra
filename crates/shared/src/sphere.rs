use bevy::prelude::*;

/// World units are meters: 1.0 == 1 m. Kept explicit so dimensions read physically.
pub const METER: f32 = 1.0;

/// Planet radius. A small "game planet" (~12.6 km circumference) — big enough to explore,
/// small enough that terrain relief is visible and f32 precision stays comfortable.
pub const PLANET_RADIUS: f32 = 2000.0 * METER;

/// An actor's position on the planet: a unit vector from the sphere center.
#[derive(Component, Clone, Copy, Debug)]
pub struct SpherePos(pub Vec3);

impl SpherePos {
    pub fn new(dir: Vec3) -> Self {
        Self(dir.normalize())
    }

    /// World-space point on the surface.
    pub fn world(&self) -> Vec3 {
        self.0 * PLANET_RADIUS
    }

    /// Arc-length distance to another point along the sphere surface.
    pub fn distance(&self, other: SpherePos) -> f32 {
        self.0.angle_between(other.0) * PLANET_RADIUS
    }

    /// Move `dist` world-units toward `target` along the great circle, staying on the sphere.
    pub fn step_toward(&mut self, target: SpherePos, dist: f32) {
        let axis = self.0.cross(target.0);
        if axis.length_squared() < 1e-12 {
            return; // already there or antipodal
        }
        let axis = axis.normalize();
        let angle = (dist / PLANET_RADIUS).min(self.0.angle_between(target.0));
        self.0 = Quat::from_axis_angle(axis, angle) * self.0;
    }

    /// Move `dist` world-units along a tangent direction `tangent_dir` (a 2D input mapped
    /// into the local tangent plane using `basis`), staying on the sphere.
    pub fn step_tangent(&mut self, tangent: Vec3, dist: f32) {
        if tangent.length_squared() < 1e-12 {
            return;
        }
        let axis = self.0.cross(tangent).normalize();
        let angle = dist / PLANET_RADIUS;
        self.0 = Quat::from_axis_angle(axis, angle) * self.0;
    }

    /// Orthonormal tangent basis at this point: (east, north) both tangent to the surface.
    /// `up` is the world Y axis reference; falls back near the poles.
    pub fn tangent_basis(&self) -> (Vec3, Vec3) {
        let up = self.0;
        let reference = if up.y.abs() > 0.99 { Vec3::Z } else { Vec3::Y };
        let east = reference.cross(up).normalize();
        let north = up.cross(east).normalize();
        (east, north)
    }

    /// Transform placing a mesh flat on the surface, its local +Y pointing away from center.
    pub fn surface_transform(&self, spin: f32) -> Transform {
        let up = self.0;
        let mut t = Transform::from_translation(self.world());
        t.rotation = Quat::from_rotation_arc(Vec3::Y, up) * Quat::from_rotation_y(spin);
        t
    }
}

/// A point on the sphere a given arc-distance from `origin` in a random direction.
pub fn ring_point(origin: SpherePos, arc_dist: f32, rng_angle: f32) -> SpherePos {
    let (east, north) = origin.tangent_basis();
    let tangent = east * rng_angle.cos() + north * rng_angle.sin();
    let axis = origin.0.cross(tangent).normalize();
    let angle = arc_dist / PLANET_RADIUS;
    SpherePos(Quat::from_axis_angle(axis, angle) * origin.0)
}

/// A uniformly random point on the sphere.
pub fn random_point(u: f32, v: f32) -> SpherePos {
    let theta = u * std::f32::consts::TAU;
    let z = v * 2.0 - 1.0;
    let r = (1.0 - z * z).max(0.0).sqrt();
    SpherePos(Vec3::new(r * theta.cos(), z, r * theta.sin()))
}

/// Spherical interpolation between two points along the great circle. `t` in [0,1].
pub fn slerp(a: SpherePos, b: SpherePos, t: f32) -> SpherePos {
    let dot = a.0.dot(b.0).clamp(-1.0, 1.0);
    let omega = dot.acos();
    if omega < 1e-5 {
        return a;
    }
    let s = omega.sin();
    let wa = ((1.0 - t) * omega).sin() / s;
    let wb = (t * omega).sin() / s;
    SpherePos((a.0 * wa + b.0 * wb).normalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stays_on_sphere() {
        let mut p = SpherePos::new(Vec3::X);
        let target = SpherePos::new(Vec3::new(0.0, 1.0, 0.2));
        // Step a fixed fraction of the radius each iteration so this is radius-independent.
        let step = PLANET_RADIUS * 0.05;
        for _ in 0..100 {
            p.step_toward(target, step);
            assert!((p.0.length() - 1.0).abs() < 1e-4, "drifted off unit sphere");
        }
        assert!(p.distance(target) < 1.0, "did not converge to target");
    }

    #[test]
    fn arc_distance_symmetric() {
        let a = random_point(0.1, 0.7);
        let b = random_point(0.8, 0.3);
        assert!((a.distance(b) - b.distance(a)).abs() < 1e-3);
    }

    #[test]
    fn tangent_basis_orthonormal() {
        let p = random_point(0.3, 0.55);
        let (e, n) = p.tangent_basis();
        assert!(e.dot(p.0).abs() < 1e-4);
        assert!(n.dot(p.0).abs() < 1e-4);
        assert!(e.dot(n).abs() < 1e-4);
    }

    #[test]
    fn heading_parallel_transport_never_flips() {
        // Walking straight (constant heading, re-projected each step) must not reverse
        // direction, even crossing a pole. Guards the "walk flips 180deg" bug.
        let mut pos = SpherePos::new(Vec3::X);
        let (_, north) = pos.tangent_basis();
        let mut heading = north;
        let step = 15.0;
        for _ in 0..600 {
            let prev = heading;
            pos.step_tangent(heading, step);
            let up = pos.0;
            heading = (heading - up * heading.dot(up)).normalize();
            // Heading turns gradually along the geodesic; it must never snap ~180deg.
            assert!(prev.dot(heading) > 0.0, "heading flipped direction");
        }
    }
}
