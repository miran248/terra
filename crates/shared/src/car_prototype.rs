//! Throwaway arcade driving model. Inputs are normalized; distances are meters.
use bevy::prelude::*;

pub const CHASSIS_SIZE: Vec3 = Vec3::new(1.8, 0.8, 3.2);

/// Sample under the center and chassis corners rather than assuming that the
/// center is the lowest point above terrain. Positions share the body's height.
pub fn support_origins(position: Vec3, heading: Vec3) -> [Vec3; 5] {
    let up = position.normalize();
    let forward = (heading - up * heading.dot(up)).normalize();
    let side = forward.cross(up) * (CHASSIS_SIZE.x * 0.5);
    let end = forward * (CHASSIS_SIZE.z * 0.5);
    [
        position,
        position + side + end,
        position - side + end,
        position + side - end,
        position - side - end,
    ]
}

#[derive(Clone, Copy)]
pub struct CarMotion {
    pub velocity: Vec3,
    pub heading: Vec3,
}

#[derive(Default)]
pub struct CarCamera {
    distance: Option<f32>,
    rotation: Option<Quat>,
}

impl CarCamera {
    pub fn offset(position: Vec3, heading: Vec3) -> Vec3 {
        position.normalize() * 3.0 - heading * 8.0
    }

    pub fn follow(
        &mut self,
        position: Vec3,
        heading: Vec3,
        dt: f32,
        hit: Option<f32>,
    ) -> Transform {
        let up = position.normalize();
        let offset = Self::offset(position, heading);
        let maximum = hit.map_or(offset.length(), |hit| {
            (hit - 0.1).clamp(0.0, offset.length())
        });
        let blend = 1.0 - (-6.0 * dt).exp();
        let distance = self.distance.map_or(maximum, |previous| {
            if maximum < previous {
                maximum
            } else {
                previous + (maximum - previous) * blend
            }
        });
        self.distance = Some(distance);
        let mut transform = Transform::from_translation(position + offset.normalize() * distance)
            .looking_at(position + heading * 15.0, up);
        transform.rotation = self.rotation.map_or(transform.rotation, |previous| {
            previous.slerp(transform.rotation, blend)
        });
        self.rotation = Some(transform.rotation);
        transform
    }
}

impl CarMotion {
    /// Advance driving from the physics velocity. `ground` is a support normal,
    /// not a wall contact. Gravity and collision response belong to physics.
    pub fn step(self, dt: f32, throttle: f32, steer: f32, up: Vec3, ground: Option<Vec3>) -> Self {
        let heading = (self.heading - up * self.heading.dot(up)).normalize_or(self.heading);
        let Some(normal) =
            ground.filter(|normal| normal.dot(up) >= std::f32::consts::FRAC_1_SQRT_2)
        else {
            return Self { heading, ..self };
        };
        let velocity = self.velocity;
        let forward = (heading - normal * heading.dot(normal)).normalize();
        let speed = velocity.dot(forward);
        let turn_rate = 1.5 / (1.0 + speed.abs() / 15.0);
        let heading = Quat::from_axis_angle(
            up,
            steer * speed.signum() * (speed.abs() / 2.0).min(1.0) * turn_rate * dt,
        ) * heading;
        let forward = (heading - normal * heading.dot(normal)).normalize();
        let (target, rate) = if throttle * speed < -0.1 {
            (0.0, 12.0)
        } else if throttle > 0.0 {
            (30.0, 18.0)
        } else if throttle < 0.0 {
            (-8.0, 18.0)
        } else {
            (0.0, 2.0)
        };
        let next_speed = speed + (target - speed).clamp(-rate * dt, rate * dt);
        let side = normal.cross(forward);
        Self {
            velocity: forward * next_speed
                + side * velocity.dot(side) * (-8.0 * dt).exp()
                // Ground adhesion: do not carry uphill momentum away from a
                // descending support. Keep inward velocity for physics to resolve.
                + normal * velocity.dot(normal).min(0.0),
            heading,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stopped() -> CarMotion {
        CarMotion {
            velocity: Vec3::ZERO,
            heading: Vec3::NEG_Z,
        }
    }

    #[test]
    fn changing_support_does_not_add_a_velocity_impulse() {
        let moving = CarMotion {
            velocity: Vec3::NEG_Z * 12.0,
            ..stopped()
        };
        let next = moving.step(0.0, 0.0, 0.0, Vec3::Y, Some(Vec3::new(0.0, 0.8660254, 0.5)));
        assert!(next.velocity.abs_diff_eq(moving.velocity, 0.001));
    }

    #[test]
    fn chase_camera_is_radial_retracts_immediately_and_recovers_smoothly() {
        for sign in [-1.0, 1.0] {
            let mut camera = CarCamera::default();
            let position = Vec3::Y * 2000.0 * sign;
            let clear = camera.follow(position, Vec3::NEG_Z, 0.1, None);
            assert!(
                clear
                    .translation
                    .abs_diff_eq(Vec3::new(0.0, 2003.0 * sign, 8.0), 0.001)
            );
            assert!((clear.rotation * Vec3::Y).dot(Vec3::Y * sign) > 0.9);
            let blocked = camera.follow(position, Vec3::NEG_Z, 0.1, Some(2.0));
            assert!((blocked.translation.distance(position) - 1.9).abs() < 0.001);
            let released = camera.follow(position, Vec3::NEG_Z, 0.1, None);
            assert!(released.translation.distance(position) > 1.9);
            assert!(released.translation.distance(position) < 8.0);
        }
    }

    #[test]
    fn grip_reduces_sideways_slip_but_air_and_steep_slopes_have_no_drive() {
        let sliding = CarMotion {
            velocity: Vec3::X * 4.0,
            ..stopped()
        };
        let gripped = sliding.step(0.1, 0.0, 0.0, Vec3::Y, Some(Vec3::Y));
        assert!(gripped.velocity.x > 0.0 && gripped.velocity.x < 2.0);
        let airborne = sliding.step(0.1, 1.0, 1.0, Vec3::Y, None);
        assert_eq!(airborne.velocity, sliding.velocity);
        assert_eq!(airborne.heading, sliding.heading);
        let steep = stopped().step(0.1, 1.0, 0.0, Vec3::Y, Some(Vec3::new(0.0, 0.5, 0.8660254)));
        assert_eq!(steep.velocity, Vec3::ZERO);
        let normal = Vec3::new(0.0, 0.8660254, 0.5);
        let slope = stopped().step(0.1, 1.0, 0.0, Vec3::Y, Some(normal));
        assert!(slope.velocity.length() > 0.5);
        assert!(slope.velocity.dot(normal).abs() < 0.001);
    }

    #[test]
    fn steering_needs_motion_and_reverses_with_reverse_gear() {
        let stationary = stopped().step(0.1, 0.0, 1.0, Vec3::Y, Some(Vec3::Y));
        assert_eq!(stationary.heading, Vec3::NEG_Z);
        let forward = CarMotion {
            velocity: Vec3::NEG_Z * 8.0,
            ..stopped()
        }
        .step(0.1, 0.0, 1.0, Vec3::Y, Some(Vec3::Y));
        let reverse = CarMotion {
            velocity: Vec3::Z * 8.0,
            ..stopped()
        }
        .step(0.1, 0.0, 1.0, Vec3::Y, Some(Vec3::Y));
        assert!(forward.heading.x < -0.01);
        assert!(reverse.heading.x > 0.01);
        let fast = CarMotion {
            velocity: Vec3::NEG_Z * 30.0,
            ..stopped()
        }
        .step(0.1, 0.0, 1.0, Vec3::Y, Some(Vec3::Y));
        assert!(fast.heading.x.abs() < forward.heading.x.abs());
    }

    #[test]
    fn powered_drive_overcomes_gravity_on_a_forty_degree_slope() {
        let normal = Vec3::new(0.0, 0.76604444, 0.6427876);
        let uphill = Vec3::new(0.0, 0.6427876, -0.76604444);
        let gravity = Vec3::NEG_Y * 20.0;
        // Contact supports the normal component of gravity; its remaining
        // downhill component still opposes the powered motion each step.
        let downhill = gravity - normal * gravity.dot(normal);
        let mut car = stopped();
        for _ in 0..20 {
            car = car.step(0.1, 1.0, 0.0, Vec3::Y, Some(normal));
            car.velocity += downhill * 0.1;
        }
        assert!(car.velocity.dot(uphill) > 5.0);
    }

    #[test]
    fn accelerates_brakes_before_reversing_and_limits_powered_speed() {
        let mut car = stopped();
        car = car.step(1.0, 1.0, 0.0, Vec3::Y, Some(Vec3::Y));
        assert!((car.velocity.length() - 18.0).abs() < 0.001);
        for _ in 0..100 {
            car = car.step(0.1, 1.0, 0.0, Vec3::Y, Some(Vec3::Y));
        }
        assert!((car.velocity.length() - 30.0).abs() < 0.001);
        car = car.step(1.0, -1.0, 0.0, Vec3::Y, Some(Vec3::Y));
        assert!((car.velocity.dot(Vec3::NEG_Z) - 18.0).abs() < 0.001);
        car = car.step(2.0, -1.0, 0.0, Vec3::Y, Some(Vec3::Y));
        assert!(car.velocity.length() < 0.001);
        for _ in 0..100 {
            car = car.step(0.1, -1.0, 0.0, Vec3::Y, Some(Vec3::Y));
        }
        assert!((car.velocity.dot(Vec3::Z) - 8.0).abs() < 0.001);
    }
}
