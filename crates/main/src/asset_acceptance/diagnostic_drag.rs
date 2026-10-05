use bevy::prelude::*;

/// Logical cursor samples following a great circle to the antipode through the
/// production orbit control's changing east/north tangent frame.
pub(crate) fn opposite_side_drag(start: Vec3, width: f32, height: f32) -> Option<Vec<Vec2>> {
    const STEPS: usize = 256;
    const RADIANS_PER_LOGICAL_PIXEL: f32 = 0.004;
    const MARGIN: f32 = 8.0;
    let start = start.try_normalize()?;
    let viewport = Vec2::new(width, height);
    if !viewport.is_finite() || viewport.min_element() <= 2.0 * MARGIN {
        return None;
    }
    let east = Vec3::Y
        .cross(start)
        .normalize_or(start.any_orthonormal_vector());
    let axis = start.cross(east).normalize();
    let mut direction = start;
    let mut cursor = Vec2::ZERO;
    let mut points = Vec::with_capacity(STEPS + 1);
    points.push(cursor);
    for index in 1..=STEPS {
        let progress = index as f32 / STEPS as f32;
        let target =
            (Quat::from_axis_angle(axis, -std::f32::consts::PI * progress) * start).normalize();
        let east = Vec3::Y
            .cross(direction)
            .normalize_or(direction.any_orthonormal_vector());
        let north = direction.cross(east).normalize();
        // Invert orbit_transform for this short step. A horizontal-only cursor
        // route would keep roughly the same latitude instead of reaching -start.
        let delta = Vec2::new(
            (-target.dot(east)).atan2(target.dot(direction)),
            -target.dot(north).clamp(-1.0, 1.0).asin(),
        );
        cursor += delta / RADIANS_PER_LOGICAL_PIXEL;
        points.push(cursor);
        direction = target;
    }
    let minimum = points
        .iter()
        .copied()
        .fold(Vec2::splat(f32::INFINITY), Vec2::min);
    let maximum = points
        .iter()
        .copied()
        .fold(Vec2::splat(f32::NEG_INFINITY), Vec2::max);
    let extent = maximum - minimum;
    if !extent.is_finite() || (extent + Vec2::splat(2.0 * MARGIN)).cmpgt(viewport).any() {
        return None;
    }
    let offset = (viewport - extent) * 0.5 - minimum;
    for point in &mut points {
        *point += offset;
    }
    Some(points)
}
