use bevy::prelude::{Color, Rect, Vec2, Vec3};

use crate::{level::RegionKind, theme};

/// Marker types shared by the minimap and Planet view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanetMarkerKind {
    Explorer,
    SelectedDestination,
    Settlement,
    Region(RegionKind),
    Bridge,
}

/// Marker geometry categories shared by the minimap and Planet view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanetMarkerShape {
    Circle,
    Square,
}

/// Style shared by both map presentations. Dot sizes and placement remain
/// local to each view so each projection keeps its own scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlanetMarkerPresentation {
    pub color: Color,
    pub shape: PlanetMarkerShape,
    pub outlined: bool,
}

pub fn planet_marker_presentation(kind: PlanetMarkerKind) -> PlanetMarkerPresentation {
    let (color, shape, outlined) = match kind {
        PlanetMarkerKind::Explorer => (theme::ACCENT, PlanetMarkerShape::Circle, false),
        PlanetMarkerKind::SelectedDestination => (theme::PRIMARY, PlanetMarkerShape::Circle, true),
        PlanetMarkerKind::Settlement | PlanetMarkerKind::Region(RegionKind::Settlement) => {
            (theme::WARNING, PlanetMarkerShape::Square, true)
        }
        PlanetMarkerKind::Region(kind) => {
            (region_marker_color(kind), PlanetMarkerShape::Circle, false)
        }
        PlanetMarkerKind::Bridge => (theme::WARNING, PlanetMarkerShape::Circle, true),
    };
    PlanetMarkerPresentation {
        color,
        shape,
        outlined,
    }
}

fn region_marker_color(kind: RegionKind) -> Color {
    match kind {
        RegionKind::Ocean | RegionKind::Lake | RegionKind::SaltLake | RegionKind::River => {
            theme::INFO
        }
        RegionKind::MountainRange | RegionKind::Volcano | RegionKind::Glacier => theme::ACCENT,
        RegionKind::Settlement | RegionKind::Road => theme::WARNING,
        RegionKind::Beach | RegionKind::Cliff => theme::PRIMARY,
        _ => theme::INK,
    }
}

/// Whether a marker's name should be shown at the current hover state.
pub fn planet_marker_label_visible(kind: PlanetMarkerKind, hovered: bool) -> bool {
    match kind {
        PlanetMarkerKind::Explorer
        | PlanetMarkerKind::SelectedDestination
        | PlanetMarkerKind::Settlement
        | PlanetMarkerKind::Region(RegionKind::Settlement) => true,
        PlanetMarkerKind::Region(_) | PlanetMarkerKind::Bridge => hovered,
    }
}

/// Label priority for reducing collisions. Hovered places rise above the
/// settlement layer, while bridges remain unnamed until hovered.
pub fn planet_marker_label_priority(kind: PlanetMarkerKind, hovered: bool) -> Option<u8> {
    match kind {
        PlanetMarkerKind::Explorer => Some(0),
        PlanetMarkerKind::SelectedDestination => Some(1),
        PlanetMarkerKind::Settlement | PlanetMarkerKind::Region(RegionKind::Settlement) => {
            Some(if hovered { 2 } else { 3 })
        }
        PlanetMarkerKind::Region(_) => Some(if hovered { 2 } else { 4 }),
        PlanetMarkerKind::Bridge => hovered.then_some(2),
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
    use crate::{level::RegionKind, theme};
    use bevy::prelude::{Rect, Vec2, Vec3};

    use super::{
        PlanetMarkerKind, PlanetMarkerShape, cursor_hits_planet_marker,
        marker_clears_spherical_horizon, planet_marker_label_priority, planet_marker_label_visible,
        planet_marker_presentation, project_ndc_to_logical_viewport, same_surface_location,
        triangle_area_weighted_centroid,
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
    fn marker_labels_follow_shared_hover_policy_and_priority() {
        assert!(planet_marker_label_visible(
            PlanetMarkerKind::Explorer,
            false
        ));
        assert!(planet_marker_label_visible(
            PlanetMarkerKind::SelectedDestination,
            false
        ));
        assert!(planet_marker_label_visible(
            PlanetMarkerKind::Settlement,
            false
        ));
        assert!(planet_marker_label_visible(
            PlanetMarkerKind::Region(RegionKind::Settlement),
            false
        ));
        assert!(!planet_marker_label_visible(
            PlanetMarkerKind::Region(RegionKind::Forest),
            false,
        ));
        assert!(!planet_marker_label_visible(
            PlanetMarkerKind::Bridge,
            false,
        ));
        assert!(planet_marker_label_visible(
            PlanetMarkerKind::Region(RegionKind::Forest),
            true,
        ));
        assert!(planet_marker_label_visible(PlanetMarkerKind::Bridge, true));
        assert_eq!(
            planet_marker_label_priority(PlanetMarkerKind::Explorer, false),
            Some(0),
        );
        assert_eq!(
            planet_marker_label_priority(PlanetMarkerKind::SelectedDestination, false),
            Some(1),
        );
        assert_eq!(
            planet_marker_label_priority(PlanetMarkerKind::Region(RegionKind::Forest), true),
            Some(2),
        );
        assert_eq!(
            planet_marker_label_priority(PlanetMarkerKind::Settlement, false),
            Some(3),
        );
        assert_eq!(
            planet_marker_label_priority(PlanetMarkerKind::Region(RegionKind::Forest), false),
            Some(4),
        );
    }

    #[test]
    fn both_maps_share_marker_colors_shapes_and_outlines() {
        let explorer = planet_marker_presentation(PlanetMarkerKind::Explorer);
        assert_eq!(explorer.color, theme::ACCENT);
        assert_eq!(explorer.shape, PlanetMarkerShape::Circle);
        assert!(!explorer.outlined);

        let settlement = planet_marker_presentation(PlanetMarkerKind::Settlement);
        assert_eq!(settlement.color, theme::WARNING);
        assert_eq!(settlement.shape, PlanetMarkerShape::Square);
        assert!(settlement.outlined);
        assert_eq!(
            planet_marker_presentation(PlanetMarkerKind::Region(RegionKind::Settlement)),
            settlement
        );

        let region = planet_marker_presentation(PlanetMarkerKind::Region(RegionKind::Ocean));
        assert_eq!(region.color, theme::INFO);
        assert_eq!(region.shape, PlanetMarkerShape::Circle);
        assert!(!region.outlined);
        assert_eq!(
            planet_marker_presentation(PlanetMarkerKind::Region(RegionKind::MountainRange)).color,
            theme::ACCENT
        );
        assert_eq!(
            planet_marker_presentation(PlanetMarkerKind::Region(RegionKind::Beach)).color,
            theme::PRIMARY
        );
        assert_eq!(
            planet_marker_presentation(PlanetMarkerKind::Region(RegionKind::Forest)).color,
            theme::INK
        );

        let bridge = planet_marker_presentation(PlanetMarkerKind::Bridge);
        assert_eq!(bridge.color, theme::WARNING);
        assert_eq!(bridge.shape, PlanetMarkerShape::Circle);
        assert!(bridge.outlined);

        let destination = planet_marker_presentation(PlanetMarkerKind::SelectedDestination);
        assert_eq!(destination.color, theme::PRIMARY);
        assert_eq!(destination.shape, PlanetMarkerShape::Circle);
        assert!(destination.outlined);
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
