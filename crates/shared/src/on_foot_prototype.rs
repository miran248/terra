//! Accepted on-foot exploration tuning, retaining the baseline for comparison tests.

use bevy::prelude::*;

#[derive(Resource, Default)]
pub struct OnFootPrototype {
    pub baseline: bool,
}

impl OnFootPrototype {
    pub fn camera(&self, position: Vec3, heading: Vec3, hit_distance: Option<f32>) -> Transform {
        let up = position.normalize();
        let offset = up * 2.0 - heading * 5.0;
        let distance = hit_distance
            .map(|hit| (hit - 0.1).clamp(0.0, offset.length()))
            .unwrap_or(offset.length());
        Transform::from_translation(position + offset.normalize() * distance)
            .looking_at(position + heading * 10.0, up)
    }

    pub fn speed(&self, sprint: bool, slowed_by_water: bool) -> f32 {
        let speed = match (self.baseline, sprint) {
            (true, false) => 60.0,
            (true, true) => 150.0,
            (false, false) => 5.0,
            (false, true) => 10.0,
        };
        speed * if slowed_by_water { 0.4 } else { 1.0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn obstruction_retracts_camera_and_clear_view_restores_it() {
        let tuning = OnFootPrototype::default();
        let position = Vec3::Y * 2000.0;
        let clear = tuning.camera(position, Vec3::Z, None);
        let blocked = tuning.camera(position, Vec3::Z, Some(2.0));
        assert!((blocked.translation.distance(position) - 1.9).abs() < 0.001);
        let touching = tuning.camera(position, Vec3::Z, Some(0.0));
        assert_eq!(touching.translation, position);
        let beyond = tuning.camera(position, Vec3::Z, Some(100.0));
        assert_eq!(beyond.translation, clear.translation);
        assert_eq!(tuning.camera(position, Vec3::Z, None), clear);
    }

    #[test]
    fn rear_camera_follows_local_up_on_opposite_sides_of_planet() {
        let tuning = OnFootPrototype::default();
        for sign in [-1.0, 1.0] {
            let position = Vec3::Y * 2000.0 * sign;
            let camera = tuning.camera(position, Vec3::Z, None);
            assert!(
                camera
                    .translation
                    .abs_diff_eq(Vec3::new(0.0, 2002.0 * sign, -5.0), 0.001)
            );
            let target_direction = (position + Vec3::Z * 10.0 - camera.translation).normalize();
            assert!((camera.rotation * Vec3::NEG_Z).abs_diff_eq(target_direction, 0.001));
            assert!((camera.rotation * Vec3::Y).dot(Vec3::Y * sign) > 0.9);
        }
    }

    #[test]
    fn exploration_speed_preserves_water_and_ice_slowdown() {
        let tuning = OnFootPrototype::default();
        assert_eq!(tuning.speed(false, false), 5.0);
        assert_eq!(tuning.speed(true, false), 10.0);
        assert_eq!(tuning.speed(false, true), 2.0);
        assert_eq!(tuning.speed(true, true), 4.0);
    }
}
