//! Shared lighting policy for the live planet view.

use crate::planet_view::PLANET_VIEW_FAR_RADIUS;
use bevy::prelude::Vec3;
use terra_geometry::sphere::PLANET_RADIUS;

/// The camera's ground-level distance-fog visibility.
pub const GROUND_FOG_VISIBILITY: f32 = 1_700.0;
/// Visibility at whole-planet scale, where Bevy's atmosphere remains the far-horizon layer.
pub const OVERVIEW_FOG_VISIBILITY: f32 = 100_000.0;
/// Aerial-perspective path length at ground-level camera heights.
pub const GROUND_AERIAL_VIEW_DISTANCE: f32 = 2_500.0;
/// Aerial-perspective path length that reaches the atmosphere shell from the overview camera.
pub const OVERVIEW_AERIAL_VIEW_DISTANCE: f32 = 5_000.0;
/// Height at which the overview rendering profile is fully attained.
pub const OVERVIEW_ALTITUDE: f32 = PLANET_VIEW_FAR_RADIUS - PLANET_RADIUS;
/// Atmosphere shell thickness around the nominal planet surface.
pub const ATMOSPHERE_SHELL_HEIGHT: f32 = 900.0;
/// Outer radius of the rendered atmosphere shell.
pub const ATMOSPHERE_OUTER_RADIUS: f32 = PLANET_RADIUS + ATMOSPHERE_SHELL_HEIGHT;
/// The furthest camera projection distance while Planet view is active.
pub const PLANET_VIEW_FAR_CLIP_DISTANCE: f32 = PLANET_VIEW_FAR_RADIUS + 2.0 * PLANET_RADIUS;

/// Camera-dependent fog and aerial-perspective settings for the planet view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlanetAtmosphereProfile {
    /// Visibility distance used to derive the camera's distance-fog density.
    pub distance_fog_visibility: f32,
    /// Maximum aerial-perspective path length used by Bevy's atmosphere LUT.
    pub aerial_view_distance: f32,
}

impl PlanetAtmosphereProfile {
    /// Blend ground haze into the overview profile using spherical camera altitude.
    pub fn at_altitude(altitude: f32) -> Self {
        let altitude = if altitude.is_finite() {
            altitude.max(0.0)
        } else {
            0.0
        };
        let t = (altitude / OVERVIEW_ALTITUDE).clamp(0.0, 1.0);
        let blend = t * t * (3.0 - 2.0 * t);

        Self {
            distance_fog_visibility: GROUND_FOG_VISIBILITY
                + (OVERVIEW_FOG_VISIBILITY - GROUND_FOG_VISIBILITY) * blend,
            aerial_view_distance: GROUND_AERIAL_VIEW_DISTANCE
                + (OVERVIEW_AERIAL_VIEW_DISTANCE - GROUND_AERIAL_VIEW_DISTANCE) * blend,
        }
    }
}

/// Altitude above the nominal spherical surface; invalid positions use the ground profile.
pub fn camera_altitude(camera_position: Vec3) -> f32 {
    let radius = camera_position.length();
    if radius.is_finite() {
        (radius - PLANET_RADIUS).max(0.0)
    } else {
        0.0
    }
}

/// Fraction of direct daylight at a surface position for a sun direction.
/// `sun_direction` points from the planet toward the sun.
pub fn daylight_factor(surface_position: Vec3, sun_direction: Vec3) -> f32 {
    surface_position
        .normalize_or(Vec3::Y)
        .dot(sun_direction.normalize_or(Vec3::Y))
        .clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daylight_factor_tracks_the_surface_hemisphere() {
        assert_eq!(daylight_factor(Vec3::Y, Vec3::Y), 1.0);
        assert_eq!(daylight_factor(Vec3::NEG_Y, Vec3::Y), 0.0);
        assert_eq!(daylight_factor(Vec3::X, Vec3::Y), 0.0);
    }

    #[test]
    fn atmosphere_profile_blends_continuously_from_ground_to_overview() {
        let ground = PlanetAtmosphereProfile::at_altitude(0.0);
        let middle = PlanetAtmosphereProfile::at_altitude(OVERVIEW_ALTITUDE * 0.5);
        let overview = PlanetAtmosphereProfile::at_altitude(OVERVIEW_ALTITUDE);

        assert_eq!(ground.distance_fog_visibility, GROUND_FOG_VISIBILITY);
        assert_eq!(ground.aerial_view_distance, GROUND_AERIAL_VIEW_DISTANCE);
        assert_eq!(
            middle.distance_fog_visibility,
            (GROUND_FOG_VISIBILITY + OVERVIEW_FOG_VISIBILITY) * 0.5
        );
        assert_eq!(
            middle.aerial_view_distance,
            (GROUND_AERIAL_VIEW_DISTANCE + OVERVIEW_AERIAL_VIEW_DISTANCE) * 0.5
        );
        assert_eq!(overview.distance_fog_visibility, OVERVIEW_FOG_VISIBILITY);
        assert_eq!(overview.aerial_view_distance, OVERVIEW_AERIAL_VIEW_DISTANCE);
        assert_eq!(
            PlanetAtmosphereProfile::at_altitude(-1.0),
            ground,
            "below-surface camera positions must retain the ground haze profile"
        );
        assert_eq!(
            PlanetAtmosphereProfile::at_altitude(f32::INFINITY),
            ground,
            "invalid altitudes must not remove the haze"
        );

        let samples = (0..=8)
            .map(|step| PlanetAtmosphereProfile::at_altitude(OVERVIEW_ALTITUDE * step as f32 / 8.0))
            .collect::<Vec<_>>();
        assert!(samples.windows(2).all(|pair| {
            pair[0].distance_fog_visibility < pair[1].distance_fog_visibility
                && pair[0].aerial_view_distance < pair[1].aerial_view_distance
        }));
    }

    #[test]
    fn far_clip_covers_the_far_side_atmosphere_from_the_overview_camera() {
        let far_side_atmosphere_distance = PLANET_VIEW_FAR_RADIUS + ATMOSPHERE_OUTER_RADIUS;
        assert!(PLANET_VIEW_FAR_CLIP_DISTANCE > far_side_atmosphere_distance);
    }
}
