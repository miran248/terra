//! Throwaway arcade flight model; distances and speeds are in metres.
use bevy::prelude::*;

#[derive(Default, Clone, Copy)]
pub struct FlightInput {
    pub pitch: f32,
    pub bank: f32,
    pub throttle: f32,
    pub brake: bool,
}

#[derive(Clone, Copy)]
pub struct PlaneFlight {
    pub heading: Vec3,
    pub pitch: f32,
    pub bank: f32,
    pub throttle: f32,
    pub airborne: bool,
    pub stalled: bool,
}

impl PlaneFlight {
    pub fn new(heading: Vec3) -> Self {
        Self {
            heading,
            pitch: 0.0,
            bank: 0.0,
            throttle: 0.0,
            airborne: false,
            stalled: false,
        }
    }

    pub fn step(
        &mut self,
        dt: f32,
        input: FlightInput,
        velocity: Vec3,
        up: Vec3,
        ground: Option<Vec3>,
    ) -> Vec3 {
        self.heading = (self.heading - up * self.heading.dot(up)).normalize_or(self.heading);
        self.throttle = f32::from(input.throttle > 0.0);
        let speed = velocity.dot(self.heading).max(0.0);
        if !self.airborne
            && let Some(normal) = ground
        {
            self.stalled = false;
            self.bank = 0.0;
            self.pitch = 0.0;
            self.heading =
                Quat::from_axis_angle(up, input.bank * (speed / 5.0).min(1.0) * 0.8 * dt)
                    * self.heading;
            let forward = (self.heading - normal * self.heading.dot(normal)).normalize();
            let speed = velocity.dot(forward).max(0.0);
            let target = if input.brake {
                0.0
            } else {
                self.throttle * 40.0
            };
            let rate = if input.brake {
                25.0
            } else if self.throttle > 0.0 {
                12.0
            } else {
                4.0
            };
            let next_speed = speed + (target - speed).clamp(-rate * dt, rate * dt);
            if next_speed >= 24.0 && input.pitch > 0.0 && !input.brake {
                self.airborne = true;
                self.pitch = 0.16;
                return (self.heading * self.pitch.cos() + up * self.pitch.sin()) * next_speed;
            }
            return forward * next_speed + normal * velocity.dot(normal).min(0.0);
        }
        self.airborne = true;
        let nose = self.heading * self.pitch.cos() + up * self.pitch.sin();
        let forward_speed = velocity.dot(nose).max(0.0);
        self.stalled = if self.stalled {
            forward_speed < 28.0
        } else {
            forward_speed < 22.0
        };
        self.pitch = if input.pitch.abs() > 0.01 {
            wrap_pitch(self.pitch + input.pitch * 0.55 * dt)
        } else if !self.stalled {
            self.pitch * (-0.7 * dt).exp()
        } else {
            self.pitch
        };
        if self.stalled {
            let down = wrap_pitch(-std::f32::consts::FRAC_PI_2 - self.pitch);
            self.pitch = wrap_pitch(self.pitch + down.clamp(-0.65 * dt, 0.65 * dt));
        }
        self.bank += (input.bank * 0.7 - self.bank) * (1.0 - (-2.5 * dt).exp());
        let authority = if self.stalled {
            (forward_speed / 28.0).clamp(0.15, 1.0)
        } else {
            1.0
        };
        self.heading =
            Quat::from_axis_angle(up, self.bank.tan() * 0.6 * authority * dt) * self.heading;
        let nose = self.heading * self.pitch.cos() + up * self.pitch.sin();
        let speed = velocity.length();
        let drag = 2.0 + 0.0008 * speed * speed + if input.throttle < 0.0 { 12.0 } else { 0.0 };
        let thrust = self.throttle * 10.0;
        if self.stalled {
            // Lift is lost: retain momentum, apply drag/thrust, and fall under gravity.
            let retained = (1.0 - drag * dt / speed.max(0.01)).clamp(0.0, 1.0);
            velocity * retained + (nose * thrust - up * 20.0) * dt
        } else {
            // Exchange speed with height: climbing costs energy, diving gains it.
            let acceleration = thrust - drag - 20.0 * nose.dot(up);
            nose * (speed + acceleration * dt).max(0.0)
        }
    }

    pub fn rotation(&self, up: Vec3) -> Quat {
        Quat::from_mat3(&Mat3::from_cols(self.heading.cross(up), up, -self.heading))
            * Quat::from_rotation_x(self.pitch)
            * Quat::from_rotation_z(self.bank)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn taxi_accelerates_brakes_and_needs_speed_and_pull_up_to_take_off() {
        let mut plane = PlaneFlight::new(Vec3::NEG_Z);
        let mut velocity = Vec3::ZERO;
        velocity = plane.step(
            0.1,
            FlightInput {
                pitch: 1.0,
                ..default()
            },
            velocity,
            Vec3::Y,
            Some(Vec3::Y),
        );
        assert!(!plane.airborne);
        assert!(velocity.length() < 1.0);
        for _ in 0..50 {
            velocity = plane.step(
                0.1,
                FlightInput {
                    throttle: 1.0,
                    ..default()
                },
                velocity,
                Vec3::Y,
                Some(Vec3::Y),
            );
        }
        assert!(velocity.length() > 24.0);
        assert!(!plane.airborne);
        let mut braking = plane;
        let slowed = braking.step(
            0.5,
            FlightInput {
                brake: true,
                ..default()
            },
            velocity,
            Vec3::Y,
            Some(Vec3::Y),
        );
        assert!(slowed.length() < velocity.length() - 5.0);
        velocity = plane.step(
            0.1,
            FlightInput {
                pitch: 1.0,
                ..default()
            },
            velocity,
            Vec3::Y,
            Some(Vec3::Y),
        );
        assert!(plane.airborne && velocity.y > 1.0);
    }
}

fn wrap_pitch(pitch: f32) -> f32 {
    (pitch + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}

/// Landing thresholds for this experiment; lifecycle recovery is runtime-owned.
pub fn gentle_landing(velocity: Vec3, normal: Vec3, up: Vec3, pitch: f32, bank: f32) -> bool {
    normal.dot(up) >= 15.0_f32.to_radians().cos()
        && velocity.length() <= 45.0
        && velocity.dot(normal) >= -6.0
        && pitch.abs() <= 0.27
        && bank.abs() <= 0.35
}

#[cfg(test)]
mod landing_tests {
    use super::*;
    #[test]
    fn gentle_touchdowns_exclude_fast_steep_and_badly_banked_impacts() {
        assert!(gentle_landing(
            Vec3::new(0.0, -3.0, -30.0),
            Vec3::Y,
            Vec3::Y,
            0.1,
            0.1
        ));
        for velocity in [Vec3::new(0.0, -9.0, -30.0), Vec3::NEG_Z * 70.0] {
            assert!(!gentle_landing(velocity, Vec3::Y, Vec3::Y, 0.0, 0.0));
        }
        assert!(!gentle_landing(
            Vec3::NEG_Z * 30.0,
            Vec3::X,
            Vec3::Y,
            0.0,
            0.0
        ));
        assert!(!gentle_landing(
            Vec3::NEG_Z * 30.0,
            Vec3::Y,
            Vec3::Y,
            0.0,
            0.7
        ));
    }
}

#[derive(Default)]
pub struct PlaneCamera {
    distance: Option<f32>,
}

impl PlaneCamera {
    pub fn offset(position: Vec3, heading: Vec3) -> Vec3 {
        position.normalize() * 6.0 - heading * 18.0
    }
    pub fn follow(
        &mut self,
        position: Vec3,
        heading: Vec3,
        dt: f32,
        hit: Option<f32>,
    ) -> Transform {
        let offset = Self::offset(position, heading);
        let maximum = hit.map_or(offset.length(), |hit| {
            (hit - 0.2).clamp(0.0, offset.length())
        });
        let distance = self.distance.map_or(maximum, |previous| {
            if maximum < previous {
                maximum
            } else {
                previous + (maximum - previous) * (1.0 - (-5.0 * dt).exp())
            }
        });
        self.distance = Some(distance);
        Transform::from_translation(position + offset.normalize() * distance)
            .looking_at(position + heading * 30.0, position.normalize())
    }
}

#[cfg(test)]
mod flight_tests {
    use super::*;

    #[test]
    fn level_flight_follows_the_sphere_and_banking_levels_after_release() {
        let mut plane = PlaneFlight::new(Vec3::NEG_Z);
        plane.airborne = true;
        plane.throttle = 1.0;
        let mut position = Vec3::Y * 2100.0;
        let mut velocity = Vec3::NEG_Z * 100.0;
        for _ in 0..6500 {
            velocity = plane.step(
                0.02,
                FlightInput {
                    throttle: 1.0,
                    ..default()
                },
                velocity,
                position.normalize(),
                None,
            );
            position += velocity * 0.02;
        }
        assert!((position.length() - 2100.0).abs() < 8.0);
        assert!(plane.heading.dot(position.normalize()).abs() < 0.002);
        let before = plane.heading;
        for _ in 0..50 {
            velocity = plane.step(
                0.02,
                FlightInput {
                    bank: 1.0,
                    throttle: 1.0,
                    ..default()
                },
                velocity,
                position.normalize(),
                None,
            );
        }
        assert!(before.angle_between(plane.heading) > 0.1);
        for _ in 0..200 {
            velocity = plane.step(
                0.02,
                FlightInput {
                    throttle: 1.0,
                    ..default()
                },
                velocity,
                position.normalize(),
                None,
            );
        }
        assert!(plane.bank.abs() < 0.001);
    }

    #[test]
    fn sustained_pitch_completes_a_loop_without_reversing_away_from_the_nose() {
        let mut plane = PlaneFlight::new(Vec3::NEG_Z);
        plane.airborne = true;
        plane.throttle = 1.0;
        let mut velocity = Vec3::NEG_Z * 100.0;
        let mut passed_inverted = false;
        let mut completed_loop = false;
        for _ in 0..650 {
            velocity = plane.step(
                0.02,
                FlightInput {
                    pitch: 1.0,
                    throttle: 1.0,
                    ..default()
                },
                velocity,
                Vec3::Y,
                None,
            );
            let nose = plane.rotation(Vec3::Y) * Vec3::NEG_Z;
            assert!(
                velocity.normalize().dot(nose) > 0.999,
                "velocity diverged from the nose at pitch {}",
                plane.pitch
            );
            passed_inverted |= velocity.z > 25.0;
            completed_loop |= passed_inverted && plane.pitch.abs() < 0.03 && velocity.z < -25.0;
        }
        assert!(
            passed_inverted && completed_loop,
            "pitch never completed a loop"
        );
        for _ in 0..500 {
            velocity = plane.step(
                0.02,
                FlightInput {
                    throttle: 1.0,
                    ..default()
                },
                velocity,
                Vec3::Y,
                None,
            );
        }
        assert!(plane.pitch.abs() < 0.005);
    }

    #[test]
    fn releasing_thrust_bleeds_speed_and_pitch_changes_energy() {
        let mut plane = PlaneFlight::new(Vec3::NEG_Z);
        plane.airborne = true;
        plane.throttle = 1.0;
        let coast = plane.step(
            0.1,
            FlightInput::default(),
            Vec3::NEG_Z * 60.0,
            Vec3::Y,
            None,
        );
        assert!(coast.length() < 59.9);
        assert_eq!(plane.throttle, 0.0);
        let mut climb = plane;
        climb.pitch = 0.4;
        let mut dive = plane;
        dive.pitch = -0.4;
        let input = FlightInput {
            throttle: 1.0,
            ..default()
        };
        let climb_velocity = climb.step(0.1, input, Vec3::NEG_Z * 60.0, Vec3::Y, None);
        let dive_velocity = dive.step(0.1, input, Vec3::NEG_Z * 60.0, Vec3::Y, None);
        let level = plane.step(0.1, input, Vec3::NEG_Z * 60.0, Vec3::Y, None);
        assert!(climb_velocity.length() < level.length());
        assert!(dive_velocity.length() > level.length());
    }

    #[test]
    fn low_airspeed_stalls_and_regained_airspeed_restores_flight() {
        let mut plane = PlaneFlight::new(Vec3::NEG_Z);
        plane.airborne = true;
        let falling = plane.step(
            0.1,
            FlightInput::default(),
            Vec3::NEG_Z * 18.0,
            Vec3::Y,
            None,
        );
        assert!(plane.stalled);
        assert!(falling.y < -1.0);
        assert!(plane.pitch < 0.0);
        let nose = plane.rotation(Vec3::Y) * Vec3::NEG_Z;
        let recovered = plane.step(
            0.1,
            FlightInput {
                throttle: 1.0,
                ..default()
            },
            nose * 30.0,
            Vec3::Y,
            None,
        );
        assert!(!plane.stalled);
        assert!(recovered.length() > 28.0);
    }

    #[test]
    fn camera_keeps_local_horizon_and_retracts_for_obstructions() {
        for up in [Vec3::Y, Vec3::X, Vec3::NEG_Y] {
            let position = up * 2100.0;
            let mut camera = PlaneCamera::default();
            let clear = camera.follow(position, Vec3::NEG_Z, 0.1, None);
            assert!(clear.translation.distance(position) > 18.0);
            assert!((clear.rotation * Vec3::X).dot(up).abs() < 0.001);
            let blocked = camera.follow(position, Vec3::NEG_Z, 0.1, Some(3.0));
            assert!((blocked.translation.distance(position) - 2.8).abs() < 0.001);
            let released = camera.follow(position, Vec3::NEG_Z, 0.1, None);
            assert!(released.translation.distance(position) > 2.8);
            assert!(released.translation.distance(position) < 18.0);
        }
    }
}
