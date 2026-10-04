use bevy::prelude::{Rect, Vec2, Vec3};

/// Screen-label categories ordered from the most useful navigation context to
/// the least useful one when labels compete for space.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanetMarkerLabelKind {
    Explorer,
    SelectedDestination,
    Settlement,
    Region,
    Bridge,
}

/// Whether a marker's name should be shown at the current zoom and hover state.
pub fn planet_marker_label_visible(
    kind: PlanetMarkerLabelKind,
    hovered: bool,
    camera_radius: f32,
    planet_radius: f32,
) -> bool {
    match kind {
        PlanetMarkerLabelKind::Explorer | PlanetMarkerLabelKind::SelectedDestination => true,
        PlanetMarkerLabelKind::Settlement => true,
        PlanetMarkerLabelKind::Region => {
            hovered
                || (camera_radius.is_finite()
                    && planet_radius.is_finite()
                    && planet_radius > 0.0
                    && camera_radius <= planet_radius * 2.0)
        }
        PlanetMarkerLabelKind::Bridge => hovered,
    }
}

/// Label priority for reducing collisions. Hovered places rise above the
/// settlement layer, while bridges remain unnamed until hovered.
pub fn planet_marker_label_priority(kind: PlanetMarkerLabelKind, hovered: bool) -> Option<u8> {
    match kind {
        PlanetMarkerLabelKind::Explorer => Some(0),
        PlanetMarkerLabelKind::SelectedDestination => Some(1),
        PlanetMarkerLabelKind::Settlement => Some(if hovered { 2 } else { 3 }),
        PlanetMarkerLabelKind::Region => Some(if hovered { 2 } else { 4 }),
        PlanetMarkerLabelKind::Bridge => hovered.then_some(2),
    }
}

/// Return the surface-area-weighted center of triangle geometry such as a
/// generated bridge deck. Degenerate and non-finite triangles do not move the
/// anchor.
pub fn triangle_area_weighted_centroid(triangles: &[[[f32; 3]; 3]]) -> Option<Vec3> {
    let mut weighted_center = Vec3::ZERO;
    let mut total_area = 0.0;
    for triangle in triangles {
        let [a, b, c] = triangle.map(Vec3::from_array);
        if !a.is_finite() || !b.is_finite() || !c.is_finite() {
            continue;
        }
        let area = (b - a).cross(c - a).length() * 0.5;
        if area.is_finite() && area > f32::EPSILON {
            weighted_center += (a + b + c) / 3.0 * area;
            total_area += area;
        }
    }
    (total_area > f32::EPSILON).then_some(weighted_center / total_area)
}

/// Compare surface positions by direction, ignoring height differences such as
/// a raised bridge deck. This lets independent typed collections deduplicate
/// duplicate presentation anchors without relying on display names.
pub fn same_surface_location(
    left: Vec3,
    right: Vec3,
    planet_radius: f32,
    tolerance_meters: f32,
) -> bool {
    if !left.is_finite()
        || !right.is_finite()
        || !planet_radius.is_finite()
        || planet_radius <= 0.0
        || !tolerance_meters.is_finite()
        || tolerance_meters < 0.0
    {
        return false;
    }
    let left = left.normalize_or_zero();
    let right = right.normalize_or_zero();
    if left == Vec3::ZERO || right == Vec3::ZERO {
        return false;
    }
    let surface_distance = left.dot(right).clamp(-1.0, 1.0).acos() * planet_radius;
    surface_distance <= tolerance_meters
}

/// Hit a visible named marker's dot or label. A missing projected center means
/// the marker is hidden or clipped and is never selectable.
pub fn cursor_hits_planet_marker(
    cursor: Vec2,
    projected_center: Option<Vec2>,
    dot_radius: f32,
    visible_label: Option<Rect>,
) -> bool {
    if !cursor.is_finite() || !dot_radius.is_finite() || dot_radius < 0.0 {
        return false;
    }
    let Some(center) = projected_center.filter(|center| center.is_finite()) else {
        return false;
    };
    cursor.distance_squared(center) <= dot_radius * dot_radius
        || visible_label.is_some_and(|bounds| bounds.contains(cursor))
}

/// Convert a camera NDC point into logical viewport coordinates, clipping
/// markers outside the frustum before they can be drawn or selected.
pub fn project_ndc_to_logical_viewport(ndc: Vec3, viewport: Rect) -> Option<Vec2> {
    if !ndc.is_finite()
        || ndc.x < -1.0
        || ndc.x > 1.0
        || ndc.y < -1.0
        || ndc.y > 1.0
        || !(0.0..=1.0).contains(&ndc.z)
        || viewport.width() <= 0.0
        || viewport.height() <= 0.0
    {
        return None;
    }

    let viewport_position = (Vec2::new(ndc.x, -ndc.y) + Vec2::ONE) * 0.5;
    Some(viewport.min + viewport_position * viewport.size())
}

/// Returns whether the straight view from the camera to an anchor clears the
/// planet's spherical limb. Surface anchors below the reference radius use
/// their own radius; raised anchors still use the planet radius so a bridge
/// deck can remain visible just beyond the terrain horizon.
pub fn marker_clears_spherical_horizon(
    camera_position: Vec3,
    anchor_position: Vec3,
    planet_radius: f32,
) -> bool {
    if !camera_position.is_finite()
        || !anchor_position.is_finite()
        || !planet_radius.is_finite()
        || planet_radius <= 0.0
    {
        return false;
    }

    let anchor_radius = anchor_position.length();
    let horizon_radius = planet_radius.min(anchor_radius);
    if camera_position.length() <= horizon_radius || anchor_radius <= f32::EPSILON {
        return false;
    }

    let segment = anchor_position - camera_position;
    let segment_length_squared = segment.length_squared();
    if segment_length_squared <= f32::EPSILON {
        return true;
    }

    let closest = camera_position
        + segment * (-camera_position.dot(segment) / segment_length_squared).clamp(0.0, 1.0);
    closest.length_squared() + 0.01 >= horizon_radius * horizon_radius
}

#[cfg(test)]
mod tests {
    use bevy::prelude::{Rect, Vec2, Vec3};

    use super::{
        PlanetMarkerLabelKind, cursor_hits_planet_marker, marker_clears_spherical_horizon,
        planet_marker_label_priority, planet_marker_label_visible, project_ndc_to_logical_viewport,
        same_surface_location, triangle_area_weighted_centroid,
    };

    #[test]
    fn raised_named_marker_can_be_seen_past_the_surface_horizon() {
        let camera = Vec3::new(0.0, 0.0, 3_000.0);
        let raised_far_side_anchor = Vec3::new(-1_992.0, 0.0, 1_150.0);

        assert!(marker_clears_spherical_horizon(
            camera,
            raised_far_side_anchor,
            2_000.0,
        ));
    }

    #[test]
    fn ordinary_far_side_surface_marker_is_hidden_by_the_sphere() {
        assert!(!marker_clears_spherical_horizon(
            Vec3::new(0.0, 0.0, 3_000.0),
            Vec3::new(-1_732.0, 0.0, 1_000.0),
            2_000.0,
        ));
    }

    #[test]
    fn a_near_side_marker_is_not_occluded_by_local_terrain() {
        // The policy deliberately receives only the spherical radius; local
        // terrain is not tested so named overlays remain readable through it.
        assert!(marker_clears_spherical_horizon(
            Vec3::new(0.0, 0.0, 3_000.0),
            Vec3::new(1_800.0, 0.0, 1_000.0),
            2_000.0,
        ));
    }

    #[test]
    fn projection_maps_ndc_into_the_camera_logical_viewport() {
        let viewport = Rect::from_corners(Vec2::new(20.0, 10.0), Vec2::new(820.0, 610.0));

        assert_eq!(
            project_ndc_to_logical_viewport(Vec3::new(0.0, 0.0, 0.5), viewport),
            Some(Vec2::new(420.0, 310.0)),
        );
        assert_eq!(
            project_ndc_to_logical_viewport(Vec3::new(1.01, 0.0, 0.5), viewport),
            None,
        );
        assert_eq!(
            project_ndc_to_logical_viewport(Vec3::new(0.0, 0.0, -0.1), viewport),
            None,
        );
    }

    #[test]
    fn marker_labels_follow_hover_zoom_and_priority_rules() {
        assert!(planet_marker_label_visible(
            PlanetMarkerLabelKind::Settlement,
            false,
            6_000.0,
            2_000.0,
        ));
        assert!(!planet_marker_label_visible(
            PlanetMarkerLabelKind::Region,
            false,
            6_000.0,
            2_000.0,
        ));
        assert!(planet_marker_label_visible(
            PlanetMarkerLabelKind::Region,
            false,
            4_000.0,
            2_000.0,
        ));
        assert!(!planet_marker_label_visible(
            PlanetMarkerLabelKind::Bridge,
            false,
            2_024.0,
            2_000.0,
        ));
        assert!(planet_marker_label_visible(
            PlanetMarkerLabelKind::Bridge,
            true,
            6_000.0,
            2_000.0,
        ));
        assert_eq!(
            planet_marker_label_priority(PlanetMarkerLabelKind::Explorer, false),
            Some(0),
        );
        assert_eq!(
            planet_marker_label_priority(PlanetMarkerLabelKind::SelectedDestination, false),
            Some(1),
        );
        assert_eq!(
            planet_marker_label_priority(PlanetMarkerLabelKind::Region, true),
            Some(2),
        );
        assert_eq!(
            planet_marker_label_priority(PlanetMarkerLabelKind::Settlement, false),
            Some(3),
        );
        assert_eq!(
            planet_marker_label_priority(PlanetMarkerLabelKind::Region, false),
            Some(4),
        );
    }

    #[test]
    fn bridge_marker_uses_the_area_weighted_deck_center() {
        let triangles = [
            [[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            [[10.0, 0.0, 0.0], [14.0, 0.0, 0.0], [10.0, 2.0, 0.0]],
        ];

        let center = triangle_area_weighted_centroid(&triangles).unwrap();

        assert!(center.distance(Vec3::new(9.2, 0.6, 0.0)) < 0.001);
    }

    #[test]
    fn marker_hit_area_matches_its_dot_or_visible_name() {
        let center = Vec2::new(100.0, 100.0);
        let label = Rect::from_corners(Vec2::new(112.0, 92.0), Vec2::new(190.0, 116.0));

        assert!(cursor_hits_planet_marker(
            Vec2::new(114.0, 100.0),
            Some(center),
            16.0,
            Some(label),
        ));
        assert!(cursor_hits_planet_marker(
            Vec2::new(180.0, 108.0),
            Some(center),
            16.0,
            Some(label),
        ));
        assert!(!cursor_hits_planet_marker(
            Vec2::new(180.0, 108.0),
            None,
            16.0,
            Some(label),
        ));
    }

    #[test]
    fn surface_identity_compares_anchor_location_instead_of_display_name() {
        let first_anchor = Vec3::new(2_010.0, 0.0, 0.0);
        let same_place_with_another_height = Vec3::new(2_080.0, 0.0, 0.0);
        let distinct_place = Vec3::new(2_000.0, 0.0, 120.0);

        assert!(same_surface_location(
            first_anchor,
            same_place_with_another_height,
            2_000.0,
            10.0,
        ));
        assert!(!same_surface_location(
            first_anchor,
            distinct_place,
            2_000.0,
            10.0,
        ));
    }
}
