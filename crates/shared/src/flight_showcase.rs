//! Scripted pilot for showcase recordings; the normal flight model owns motion.
use bevy::prelude::*;

/// A pilot issues the same bounded controls as the keyboard; the flight model owns motion.
pub fn pilot_input(
    flight: &crate::plane_prototype::PlaneFlight,
    position: Vec3,
    velocity: Vec3,
    target: Vec3,
) -> crate::plane_prototype::FlightInput {
    pilot_at_speed(flight, position, velocity, target, 94.0)
}

/// Uses thrust and airbrake to approach a requested speed without overriding momentum.
pub fn pilot_at_speed(
    flight: &crate::plane_prototype::PlaneFlight,
    position: Vec3,
    velocity: Vec3,
    target: Vec3,
    speed: f32,
) -> crate::plane_prototype::FlightInput {
    let up = position.normalize();
    let direction = (target - up * target.dot(up)).normalize_or(flight.heading);
    let turn = up
        .dot(flight.heading.cross(direction))
        .atan2(flight.heading.dot(direction));
    let desired_bank = (turn * 1.4).clamp(-0.4, 0.4);
    let desired_pitch = ((target.length() - position.length()) / 180.0)
        .atan()
        .clamp(-0.22, 0.22);
    let axis = |error: f32| (error * 3.0).clamp(-1.0, 1.0);
    crate::plane_prototype::FlightInput {
        pitch: axis(desired_pitch - flight.pitch),
        bank: axis(desired_bank - flight.bank),
        throttle: if velocity.length() > speed + 3.0 {
            -1.0
        } else {
            f32::from(velocity.length() < speed)
        },
        brake: false,
    }
}

#[cfg(test)]
mod pilot_tests {
    use super::*;
    use crate::plane_prototype::PlaneFlight;

    #[test]
    fn pilot_turns_and_climbs_using_normal_flight_controls() {
        let mut position = Vec3::Y * 2060.0;
        let mut velocity = Vec3::NEG_Z * 65.0;
        let mut flight = PlaneFlight::new(Vec3::NEG_Z);
        flight.airborne = true;
        let target = Vec3::new(300.0, 2090.0, -1200.0).normalize() * 2120.0;
        for _ in 0..600 {
            let input = pilot_input(&flight, position, velocity, target);
            assert!(input.pitch.abs() <= 1.0 && input.bank.abs() <= 1.0);
            velocity = flight.step(1.0 / 60.0, input, velocity, position.normalize(), None);
            position += velocity / 60.0;
            assert!(!flight.stalled);
        }
        assert!(position.x > 70.0, "pilot must turn toward the waypoint");
        assert!(
            position.length() > 2100.0,
            "pilot must climb toward its altitude"
        );
        assert!(velocity.length() > 40.0);
    }
}
