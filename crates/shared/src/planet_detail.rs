use bevy::prelude::Vec3;

use crate::level::{FloraKind, SceneryKind};
use terra_geometry::sphere::PLANET_RADIUS;

pub const LOCAL_DETAIL_DISTANCE: f32 = 300.0;
pub const REGIONAL_DETAIL_DISTANCE: f32 = 960.0;
pub const DETAIL_HYSTERESIS: f32 = 1.15;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SceneryTier {
    Regional,
    Local,
}

impl SceneryTier {
    pub fn for_kind(kind: SceneryKind) -> Self {
        match kind {
            SceneryKind::Flora(FloraKind::Tree)
            | SceneryKind::DeadTree
            | SceneryKind::Rock
            | SceneryKind::Log => Self::Regional,
            _ => Self::Local,
        }
    }
}

/// Ground arc to the radial horizon at the camera's current altitude.
pub fn horizon_distance(camera_altitude: f32) -> f32 {
    let altitude = camera_altitude.max(0.0);
    let radius = PLANET_RADIUS;
    radius * (radius / (radius + altitude)).clamp(0.0, 1.0).acos()
}

pub fn radial_surface_distance(a: Vec3, b: Vec3) -> f32 {
    a.normalize_or(Vec3::Y)
        .angle_between(b.normalize_or(Vec3::Y))
        * PLANET_RADIUS
}

/// Surface range over which regional detail is useful at this viewing scale.
pub fn regional_detail_distance(camera_altitude: f32) -> f32 {
    REGIONAL_DETAIL_DISTANCE.max(horizon_distance(camera_altitude))
}

/// Shrink the local detail footprint as the camera leaves the surface. At a
/// camera height beyond the local range, no small-scene detail is useful.
pub fn local_detail_distance(camera_altitude: f32) -> f32 {
    let altitude = camera_altitude.max(0.0).clamp(0.0, LOCAL_DETAIL_DISTANCE);
    (LOCAL_DETAIL_DISTANCE * LOCAL_DETAIL_DISTANCE - altitude * altitude)
        .max(0.0)
        .sqrt()
}

/// Select chunk detail using radial surface distance, retaining the current
/// level inside the downgrade band. Local detail stays fixed while regional
/// detail expands to the camera's horizon.
pub fn desired_chunk_lod(surface_distance: f32, camera_altitude: f32, current_lod: u8) -> u8 {
    let regional_distance = regional_detail_distance(camera_altitude);
    let local_distance = local_detail_distance(camera_altitude);
    let desired = if surface_distance <= local_distance {
        3
    } else if surface_distance <= regional_distance {
        2
    } else {
        1
    };

    if desired < current_lod {
        let current_entry = match current_lod {
            3 => local_distance,
            2 => regional_distance,
            _ => f32::INFINITY,
        };
        if surface_distance <= current_entry * DETAIL_HYSTERESIS {
            return current_lod;
        }
    }
    desired
}

/// Keep small scenery close to the camera while allowing large scenery to
/// remain visible across the currently visible regional surface.
pub fn scenery_visible(
    tier: SceneryTier,
    camera_distance: f32,
    surface_distance: f32,
    camera_altitude: f32,
    base_range: f32,
    currently_visible: bool,
) -> bool {
    let range = match tier {
        SceneryTier::Regional => base_range.max(horizon_distance(camera_altitude)),
        SceneryTier::Local => base_range,
    };
    let hysteresis = if currently_visible {
        DETAIL_HYSTERESIS
    } else {
        1.0
    };
    let distance = match tier {
        SceneryTier::Regional => surface_distance,
        SceneryTier::Local => camera_distance,
    };
    distance <= range * hysteresis
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_altitude_expands_regional_detail_but_keeps_small_detail_local() {
        assert_eq!(desired_chunk_lod(1_800.0, 0.0, 1), 1);
        assert_eq!(desired_chunk_lod(1_800.0, 6_000.0, 1), 2);
        assert_eq!(desired_chunk_lod(400.0, 6_000.0, 1), 2);
        assert_eq!(desired_chunk_lod(100.0, 0.0, 1), 3);
        assert_eq!(desired_chunk_lod(100.0, 300.0, 1), 2);

        assert!(scenery_visible(
            SceneryTier::Regional,
            6_300.0,
            1_800.0,
            6_000.0,
            300.0,
            false,
        ));
        assert!(!scenery_visible(
            SceneryTier::Local,
            6_000.0,
            0.0,
            6_000.0,
            150.0,
            true,
        ));
    }

    #[test]
    fn detail_downgrades_and_scenery_culling_have_hysteresis() {
        assert_eq!(desired_chunk_lod(1_050.0, 0.0, 2), 2);
        assert_eq!(desired_chunk_lod(1_150.0, 0.0, 2), 1);
        assert!(scenery_visible(
            SceneryTier::Local,
            160.0,
            0.0,
            0.0,
            150.0,
            true,
        ));
        assert!(!scenery_visible(
            SceneryTier::Local,
            160.0,
            0.0,
            0.0,
            150.0,
            false,
        ));
    }
}
