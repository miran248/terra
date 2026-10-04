//! Reusable camera policy for the live planet view.
//!
//! This module owns camera intent and pose policy only. Runtime systems remain
//! responsible for writing the active camera transform and enforcing collision
//! clearance against the loaded world.

use bevy::prelude::*;

use crate::sphere::PLANET_RADIUS;

/// Nearest requested camera radius for settlement-scale inspection.
pub const PLANET_VIEW_NEAR_RADIUS: f32 = PLANET_RADIUS + 24.0;
/// Farthest requested camera radius, enough to frame the whole planet.
pub const PLANET_VIEW_FAR_RADIUS: f32 = PLANET_RADIUS * 3.0;

const CAMERA_CLEARANCE: f32 = 5.0;
const TRANSIT_CLEARANCE: f32 = 350.0;
const OPENING_SECONDS: f32 = 1.4;
const RETURN_SECONDS: f32 = 1.8;
const VIEW_RESPONSE: f32 = 12.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Phase {
    #[default]
    Chase,
    Opening,
    Browsing,
    Returning,
}

#[derive(Clone, Copy, Debug)]
struct Transition {
    from: Transform,
    from_direction: Vec3,
    elapsed: f32,
    duration: f32,
    opening: bool,
    transit_radius: f32,
}

/// Requested and attained camera framing for Planet view.
///
/// The policy is deliberately independent of Bevy systems and camera
/// components. The existing gameplay camera controller supplies the current
/// pose, chase pose, controlled-body focus, heading, and local surface radius,
/// then remains the only system that writes the main camera transform.
#[derive(Clone, Copy, Debug)]
pub struct PlanetViewCamera {
    phase: Phase,
    requested_open: bool,
    resume_on_open: bool,
    retain_orbit_target_on_open: bool,
    follow: bool,
    requested_radius: f32,
    attained_radius: f32,
    detached_direction: Vec3,
    detached_heading: Vec3,
    transition: Option<Transition>,
}

impl Default for PlanetViewCamera {
    fn default() -> Self {
        Self {
            phase: Phase::Chase,
            requested_open: false,
            resume_on_open: false,
            retain_orbit_target_on_open: false,
            follow: true,
            requested_radius: PLANET_VIEW_FAR_RADIUS,
            attained_radius: PLANET_RADIUS,
            detached_direction: Vec3::Y,
            detached_heading: Vec3::NEG_Z,
            transition: None,
        }
    }
}

impl PlanetViewCamera {
    /// Toggle entry or return. Reversals begin at the current rendered pose.
    pub fn toggle(&mut self) {
        self.requested_open = !self.requested_open;
        self.resume_on_open = false;
        self.retain_orbit_target_on_open = false;
    }

    /// Request a return to the gameplay chase camera.
    pub fn close(&mut self) {
        self.requested_open = false;
        self.resume_on_open = false;
        self.retain_orbit_target_on_open = false;
    }

    /// Whether the view is open or transitioning toward open.
    pub fn is_requested_open(&self) -> bool {
        self.requested_open
    }

    /// Whether the gameplay camera is currently under Planet view policy.
    pub fn is_active(&self) -> bool {
        self.requested_open || self.phase != Phase::Chase
    }

    /// Whether the view is following the controlled body.
    pub fn follows_body(&self) -> bool {
        self.follow
    }

    /// The requested radius, which may be held outside a local obstacle.
    pub fn requested_radius(&self) -> f32 {
        self.requested_radius
    }

    /// The radius currently attained by the camera policy.
    pub fn attained_radius(&self) -> f32 {
        self.attained_radius
    }

    /// The attained camera distance normalized between settlement and
    /// whole-planet framing. Clearance constraints are reflected in this value.
    pub fn normalized_attained_zoom(&self) -> f32 {
        ((self.attained_radius - PLANET_VIEW_NEAR_RADIUS)
            / (PLANET_VIEW_FAR_RADIUS - PLANET_VIEW_NEAR_RADIUS))
            .clamp(0.0, 1.0)
    }

    /// The currently intended radial view direction for terrain clearance.
    pub fn view_direction(&self, controlled_position: Vec3) -> Vec3 {
        if self.follow {
            controlled_position.normalize_or(Vec3::Y)
        } else {
            self.detached_direction
                .normalize_or(controlled_position.normalize_or(Vec3::Y))
        }
    }

    /// Keep a blocked transition at its last requested time step.
    pub fn hold_transition_step(&mut self, delta_seconds: f32) {
        if let Some(transition) = &mut self.transition {
            transition.elapsed = (transition.elapsed - delta_seconds.max(0.0)).max(0.0);
        }
    }

    /// Finish a transition after the runtime camera owner accepts its pose.
    pub fn finish_transition_if_ready(&mut self) {
        let Some(transition) = self.transition else {
            return;
        };
        if transition.elapsed >= transition.duration {
            self.phase = if transition.opening {
                Phase::Browsing
            } else {
                Phase::Chase
            };
            self.transition = None;
        }
    }

    /// Record the radius actually reached after runtime collision constraints.
    pub fn record_attained_radius(&mut self, radius: f32) {
        if radius.is_finite() {
            self.attained_radius = radius.max(0.0);
        }
    }

    /// Request an absolute radial distance. Clearance may keep attainment
    /// farther out until the route is safe.
    pub fn request_radius(&mut self, radius: f32) {
        self.requested_radius = radius.clamp(PLANET_VIEW_NEAR_RADIUS, PLANET_VIEW_FAR_RADIUS);
    }

    /// Scale the requested radius while retaining its independent attained
    /// value. Values above one zoom out; values below one zoom in.
    pub fn zoom_by(&mut self, multiplier: f32) {
        if multiplier.is_finite() && multiplier > 0.0 {
            self.request_radius(self.requested_radius * multiplier);
            if self.is_active() && !self.requested_open {
                self.requested_open = true;
                self.resume_on_open = true;
            }
        }
    }

    /// Start following the current controlled body's radial position.
    pub fn follow_body(&mut self) {
        self.follow = true;
    }

    /// Toggle body tracking while preserving the attained camera pose when
    /// detaching. Re-enabling follow lets the normal camera response recenter
    /// the view on the current controlled body.
    pub fn toggle_follow_from(&mut self, current: Transform) {
        if self.follow {
            let direction = current
                .translation
                .normalize_or(self.detached_direction.normalize_or(Vec3::Y));
            self.follow = false;
            self.detached_direction = direction;
            self.detached_heading = tangent_heading(current.rotation * Vec3::Y, direction);
        } else {
            self.follow_body();
        }
    }

    /// Preserve the current view direction while the body moves.
    pub fn detach(&mut self, current_direction: Vec3) {
        self.follow = false;
        if current_direction.is_finite() && current_direction.length_squared() > 1e-8 {
            self.detached_direction = current_direction.normalize();
            self.detached_heading = tangent_heading(self.detached_heading, self.detached_direction);
        }
    }

    /// Apply orbit deltas in radians and detach from the controlled body.
    pub fn orbit(&mut self, delta: Vec2) {
        if !delta.is_finite() || delta == Vec2::ZERO {
            return;
        }
        let (direction, rotation) = orbit_transform(self.detached_direction, delta);
        self.detached_direction = direction;
        self.detached_heading = tangent_heading(rotation * self.detached_heading, direction);
        self.follow = false;
        if self.is_active() && !self.requested_open {
            self.requested_open = true;
            self.resume_on_open = true;
            self.retain_orbit_target_on_open = true;
        }
    }

    /// Apply an incremental orbit from the attained camera pose. The first
    /// drag therefore starts from the current followed or returning direction,
    /// while later deltas accumulate on the detached target.
    pub fn orbit_from(&mut self, current: Transform, delta: Vec2) {
        if !delta.is_finite() || delta == Vec2::ZERO {
            return;
        }
        let current_direction = current
            .translation
            .normalize_or(self.detached_direction.normalize_or(Vec3::Y));
        let returning = self.phase == Phase::Returning
            || self
                .transition
                .is_some_and(|transition| !transition.opening);
        let direction = if self.follow || returning {
            current_direction
        } else {
            self.detached_direction.normalize_or(current_direction)
        };
        let heading = if self.follow || returning {
            tangent_heading(current.rotation * Vec3::Y, direction)
        } else {
            tangent_heading(self.detached_heading, direction)
        };
        let (direction, rotation) = orbit_transform(direction, delta);
        self.detached_direction = direction;
        self.detached_heading = tangent_heading(rotation * heading, direction);
        self.follow = false;
        if self.phase == Phase::Chase {
            self.requested_radius = PLANET_VIEW_FAR_RADIUS;
        }
        if !self.requested_open {
            self.requested_open = true;
            self.resume_on_open = true;
            self.retain_orbit_target_on_open = true;
        }
    }

    /// Update the main-camera pose from current gameplay and world inputs.
    /// `surface_radius` is the generated surface radius under the current
    /// planet-view target direction; collision systems can further constrain
    /// the returned path before writing it.
    pub fn update(
        &mut self,
        current: Transform,
        chase: Transform,
        controlled_position: Vec3,
        controlled_heading: Vec3,
        delta_seconds: f32,
        surface_radius: f32,
    ) -> Transform {
        let transitioning_to_open = self.transition.map_or(
            self.phase == Phase::Opening || self.phase == Phase::Browsing,
            |t| t.opening,
        );
        if self.requested_open != transitioning_to_open {
            let opening = self.requested_open;
            let retain_orbit_target = opening && self.retain_orbit_target_on_open;
            if opening {
                if !self.resume_on_open {
                    self.follow = true;
                    self.requested_radius = PLANET_VIEW_FAR_RADIUS;
                }
                self.resume_on_open = false;
                self.retain_orbit_target_on_open = false;
                self.phase = Phase::Opening;
            } else {
                self.phase = Phase::Returning;
            }
            let from_direction = current
                .translation
                .normalize_or(controlled_position.normalize_or(Vec3::Y));
            if !retain_orbit_target {
                self.detached_direction = from_direction;
            }
            self.transition = Some(Transition {
                from: current,
                from_direction,
                elapsed: 0.0,
                duration: if opening {
                    OPENING_SECONDS
                } else {
                    RETURN_SECONDS
                },
                opening,
                transit_radius: current
                    .translation
                    .length()
                    .max(PLANET_RADIUS + TRANSIT_CLEARANCE),
            });
        }

        let focus = controlled_position.normalize_or(Vec3::Y);
        if self.follow {
            self.detached_direction = focus;
            self.detached_heading = tangent_heading(controlled_heading, focus);
        }
        let direction = if self.follow {
            focus
        } else {
            self.detached_direction.normalize_or(focus)
        };
        let minimum_radius = surface_radius.max(PLANET_RADIUS) + CAMERA_CLEARANCE;
        let requested_radius = self.requested_radius.max(minimum_radius);

        let result = if let Some(mut transition) = self.transition {
            transition.elapsed =
                (transition.elapsed + delta_seconds.max(0.0)).min(transition.duration);
            let linear = (transition.elapsed / transition.duration).clamp(0.0, 1.0);
            let eased = smoothstep(linear);
            let target_heading = if self.follow {
                tangent_heading(controlled_heading, direction)
            } else {
                tangent_heading(self.detached_heading, direction)
            };
            let transform = if transition.opening {
                let target = planet_pose(direction, requested_radius, target_heading);
                smooth_planet_pose(
                    current,
                    target,
                    eased,
                    surface_radius.max(PLANET_RADIUS) + CAMERA_CLEARANCE,
                )
            } else {
                let chase_direction = chase
                    .translation
                    .normalize_or(controlled_position.normalize_or(Vec3::Y));
                let (radial, radius) = return_path(
                    transition.from_direction,
                    chase_direction,
                    transition.from.translation.length(),
                    transition.transit_radius.max(minimum_radius),
                    chase.translation.length(),
                    linear,
                );
                let path_position = radial * radius;
                let final_approach = smoothstep(((linear - 0.94) / 0.06).clamp(0.0, 1.0));
                Transform {
                    translation: path_position.lerp(chase.translation, final_approach),
                    rotation: transition.from.rotation.slerp(chase.rotation, eased),
                    ..default()
                }
            };
            self.transition = Some(transition);
            transform
        } else if self.requested_open {
            self.phase = Phase::Browsing;
            let heading = if self.follow {
                tangent_heading(controlled_heading, direction)
            } else {
                tangent_heading(self.detached_heading, direction)
            };
            let target = planet_pose(direction, requested_radius, heading);
            smooth_planet_pose(
                current,
                target,
                response(VIEW_RESPONSE, delta_seconds),
                surface_radius.max(PLANET_RADIUS) + CAMERA_CLEARANCE,
            )
        } else {
            self.phase = Phase::Chase;
            self.attained_radius = chase.translation.length();
            return chase;
        };

        self.attained_radius = result.translation.length();
        result
    }
}

fn planet_pose(direction: Vec3, radius: f32, heading: Vec3) -> Transform {
    Transform {
        translation: direction * radius,
        rotation: planet_rotation(direction, heading),
        ..default()
    }
}

fn planet_rotation(direction: Vec3, heading: Vec3) -> Quat {
    Transform::from_translation(direction * PLANET_VIEW_FAR_RADIUS)
        .looking_at(Vec3::ZERO, heading)
        .rotation
}

fn orbit_transform(direction: Vec3, delta: Vec2) -> (Vec3, Quat) {
    let direction = direction.normalize_or(Vec3::Y);
    let east = Vec3::Y
        .cross(direction)
        .normalize_or(direction.any_orthonormal_vector());
    let north = direction.cross(east).normalize_or(Vec3::Z);
    let yaw = Quat::from_axis_angle(north, -delta.x);
    let orbit = Quat::from_axis_angle(yaw * east, delta.y) * yaw;
    ((orbit * direction).normalize_or(direction), orbit)
}

fn smooth_planet_pose(from: Transform, to: Transform, amount: f32, inner_radius: f32) -> Transform {
    let from_radius = from.translation.length();
    let target_radius = to.translation.length();
    let radius = lerp(from_radius, target_radius, amount);
    let from_direction = from.translation.normalize_or(Vec3::Y);
    let target_direction = to.translation.normalize_or(from_direction);
    let angle = from_direction.angle_between(target_direction);
    let safe_radius = from_radius.min(radius);
    let maximum_chord_angle = if safe_radius > inner_radius {
        2.0 * (inner_radius / safe_radius).clamp(0.0, 1.0).acos()
    } else {
        0.0
    };
    let angular_amount = if angle > 1e-6 {
        (amount * angle).min(maximum_chord_angle) / angle
    } else {
        amount
    };
    Transform {
        translation: radial_slerp(from_direction, target_direction, angular_amount) * radius,
        rotation: from.rotation.slerp(to.rotation, amount),
        ..default()
    }
}

fn response(rate: f32, delta_seconds: f32) -> f32 {
    (1.0 - (-rate * delta_seconds.max(0.0)).exp()).clamp(0.0, 1.0)
}

fn tangent_heading(heading: Vec3, up: Vec3) -> Vec3 {
    let projected = heading - up * heading.dot(up);
    if projected.length_squared() > 1e-8 {
        projected.normalize()
    } else {
        up.any_orthonormal_vector()
    }
}

fn radial_slerp(from: Vec3, to: Vec3, amount: f32) -> Vec3 {
    let from = from.normalize_or(Vec3::Y);
    let to = to.normalize_or(from);
    if from.dot(to) < -0.9999 {
        let axis = from.any_orthonormal_vector();
        return (Quat::from_axis_angle(axis, std::f32::consts::PI * amount) * from).normalize();
    }
    (Quat::IDENTITY.slerp(Quat::from_rotation_arc(from, to), amount) * from).normalize()
}

fn return_path(
    from_direction: Vec3,
    to_direction: Vec3,
    from_radius: f32,
    transit_radius: f32,
    to_radius: f32,
    progress: f32,
) -> (Vec3, f32) {
    const RISE_END: f32 = 0.2;
    const SWING_END: f32 = 0.8;
    if progress < RISE_END {
        let amount = smoothstep(progress / RISE_END);
        (from_direction, lerp(from_radius, transit_radius, amount))
    } else if progress < SWING_END {
        let amount = smoothstep((progress - RISE_END) / (SWING_END - RISE_END));
        (
            radial_slerp(from_direction, to_direction, amount),
            transit_radius,
        )
    } else {
        let amount = smoothstep((progress - SWING_END) / (1.0 - SWING_END));
        (to_direction, lerp(transit_radius, to_radius, amount))
    }
}

fn smoothstep(value: f32) -> f32 {
    let value = value.clamp(0.0, 1.0);
    value * value * (3.0 - 2.0 * value)
}

fn lerp(from: f32, to: f32, amount: f32) -> f32 {
    from + (to - from) * amount
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zooming_during_return_resumes_view_without_detaching_follow() {
        let controlled_position = Vec3::Y * PLANET_RADIUS;
        let chase = Transform::from_xyz(0.0, PLANET_RADIUS + 5.0, -5.0)
            .looking_at(controlled_position, Vec3::Y);
        let mut current = chase;
        let mut camera = PlanetViewCamera::default();
        camera.toggle();
        for _ in 0..120 {
            current = camera.update(
                current,
                chase,
                controlled_position,
                Vec3::NEG_Z,
                1.0 / 60.0,
                PLANET_RADIUS,
            );
            camera.finish_transition_if_ready();
        }

        camera.close();
        for _ in 0..20 {
            current = camera.update(
                current,
                chase,
                controlled_position,
                Vec3::NEG_Z,
                1.0 / 60.0,
                PLANET_RADIUS,
            );
        }
        let before_zoom = current;

        let requested = camera.requested_radius() * 0.8;
        camera.zoom_by(0.8);
        assert!(camera.is_requested_open());
        assert!(camera.follows_body());
        assert_eq!(camera.requested_radius(), requested);
        let after_zoom = camera.update(
            before_zoom,
            chase,
            controlled_position,
            Vec3::NEG_Z,
            1.0 / 60.0,
            PLANET_RADIUS,
        );
        assert!(after_zoom.translation.distance(before_zoom.translation) < 100.0);
        assert_eq!(camera.requested_radius(), requested);
    }

    #[test]
    fn first_orbit_uses_the_attained_follow_pose() {
        let controlled_position = Vec3::Y * PLANET_RADIUS;
        let chase = Transform::from_xyz(PLANET_RADIUS + 5.0, 0.0, 0.0)
            .looking_at(controlled_position, Vec3::Y);
        let mut current = chase;
        let mut camera = PlanetViewCamera::default();
        camera.toggle();
        for _ in 0..120 {
            current = camera.update(
                current,
                chase,
                controlled_position,
                Vec3::NEG_Z,
                1.0 / 60.0,
                PLANET_RADIUS,
            );
            camera.finish_transition_if_ready();
        }
        assert!(current.translation.normalize().dot(Vec3::Y) > 0.999);
        assert!(camera.follows_body());

        let before_orbit = current;
        camera.orbit_from(before_orbit, Vec2::new(0.01, 0.0));
        assert!(!camera.follows_body());
        assert!(camera.is_requested_open());
        let after_orbit = camera.update(
            before_orbit,
            chase,
            controlled_position,
            Vec3::NEG_Z,
            1.0 / 60.0,
            PLANET_RADIUS,
        );

        assert!(after_orbit.translation.distance(before_orbit.translation) < 100.0);
        assert!(after_orbit.translation.length() > PLANET_RADIUS * 2.0);
    }

    #[test]
    fn first_orbit_during_return_survives_the_reopen_transition() {
        let controlled_position = Vec3::Y * PLANET_RADIUS;
        let chase = Transform::from_xyz(0.0, PLANET_RADIUS + 5.0, -5.0)
            .looking_at(controlled_position, Vec3::Y);
        let mut current = chase;
        let mut camera = PlanetViewCamera::default();
        camera.toggle();
        for _ in 0..120 {
            current = camera.update(
                current,
                chase,
                controlled_position,
                Vec3::NEG_Z,
                1.0 / 60.0,
                PLANET_RADIUS,
            );
            camera.finish_transition_if_ready();
        }

        camera.close();
        for _ in 0..20 {
            current = camera.update(
                current,
                chase,
                controlled_position,
                Vec3::NEG_Z,
                1.0 / 60.0,
                PLANET_RADIUS,
            );
        }
        let before_orbit = current;
        let delta = Vec2::new(0.4, -0.1);
        let target_direction = orbit_transform(before_orbit.translation.normalize(), delta).0;
        camera.orbit_from(before_orbit, delta);
        assert!(camera.is_requested_open());
        assert!(!camera.follows_body());

        let resumed = camera.update(
            before_orbit,
            chase,
            controlled_position,
            Vec3::NEG_Z,
            1.0 / 60.0,
            PLANET_RADIUS,
        );
        assert!(
            resumed.translation.normalize().dot(target_direction)
                > before_orbit.translation.normalize().dot(target_direction) + 1e-5,
            "the first return-orbit delta was lost when reopening: before={before_orbit:?}, resumed={resumed:?}, target={target_direction:?}"
        );
    }

    #[test]
    fn toggling_follow_detaches_from_the_attained_pose_and_reenables_smoothly() {
        let mut controlled_position = Vec3::Y * PLANET_RADIUS;
        let chase = Transform::from_xyz(0.0, PLANET_RADIUS + 5.0, -5.0)
            .looking_at(controlled_position, Vec3::Y);
        let mut current = chase;
        let mut camera = PlanetViewCamera::default();
        camera.toggle();
        for _ in 0..120 {
            current = camera.update(
                current,
                chase,
                controlled_position,
                Vec3::NEG_Z,
                1.0 / 60.0,
                PLANET_RADIUS,
            );
            camera.finish_transition_if_ready();
        }

        // Capture a camera pose that is still easing toward the body's new
        // heading. Detaching should preserve what the player currently sees.
        current = camera.update(
            current,
            chase,
            controlled_position,
            Vec3::X,
            1.0 / 60.0,
            PLANET_RADIUS,
        );
        let attained = current;
        camera.toggle_follow_from(attained);
        assert!(!camera.follows_body());

        controlled_position = Vec3::X * PLANET_RADIUS;
        let first_detached = camera.update(
            attained,
            chase,
            controlled_position,
            Vec3::Z,
            1.0 / 60.0,
            PLANET_RADIUS,
        );
        assert!(
            first_detached
                .translation
                .normalize()
                .dot(attained.translation.normalize())
                > 0.9999
        );
        assert!(first_detached.rotation.angle_between(attained.rotation) < 0.01);

        camera.toggle_follow_from(first_detached);
        assert!(camera.follows_body());
        let first_follow_step = camera.update(
            first_detached,
            chase,
            controlled_position,
            Vec3::Z,
            1.0 / 60.0,
            PLANET_RADIUS,
        );
        assert!(
            first_follow_step
                .translation
                .normalize()
                .angle_between(first_detached.translation.normalize())
                < 0.5,
            "follow should recenter over multiple frames"
        );

        for _ in 0..90 {
            current = camera.update(
                current,
                chase,
                controlled_position,
                Vec3::Z,
                1.0 / 60.0,
                PLANET_RADIUS,
            );
            camera.finish_transition_if_ready();
        }
        assert!(current.translation.normalize().dot(Vec3::X) > 0.99);
    }

    #[test]
    fn requested_zoom_approaches_its_distance_smoothly() {
        let controlled_position = Vec3::Y * PLANET_RADIUS;
        let chase = Transform::from_xyz(0.0, PLANET_RADIUS + 5.0, -5.0)
            .looking_at(controlled_position, Vec3::Y);
        let mut current = chase;
        let mut camera = PlanetViewCamera::default();
        camera.toggle();
        for _ in 0..120 {
            current = camera.update(
                current,
                chase,
                controlled_position,
                Vec3::NEG_Z,
                1.0 / 60.0,
                PLANET_RADIUS,
            );
            camera.finish_transition_if_ready();
        }
        let before_zoom = current;
        camera.zoom_by(0.5);

        let first_zoom_step = camera.update(
            before_zoom,
            chase,
            controlled_position,
            Vec3::NEG_Z,
            1.0 / 60.0,
            PLANET_RADIUS,
        );
        assert!(
            before_zoom
                .translation
                .distance(first_zoom_step.translation)
                < 1000.0
        );
        assert!(camera.normalized_attained_zoom() > 0.7);
        camera.record_attained_radius((PLANET_VIEW_NEAR_RADIUS + PLANET_VIEW_FAR_RADIUS) / 2.0);
        assert!((camera.normalized_attained_zoom() - 0.5).abs() < 0.001);
    }

    #[test]
    fn detached_view_keeps_its_heading_when_the_body_turns() {
        let controlled_position = Vec3::Y * PLANET_RADIUS;
        let chase = Transform::from_xyz(0.0, PLANET_RADIUS + 5.0, -5.0)
            .looking_at(controlled_position, Vec3::Y);
        let mut current = chase;
        let mut camera = PlanetViewCamera::default();
        camera.toggle();
        for _ in 0..120 {
            current = camera.update(
                current,
                chase,
                controlled_position,
                Vec3::NEG_Z,
                1.0 / 60.0,
                PLANET_RADIUS,
            );
            camera.finish_transition_if_ready();
        }
        camera.orbit_from(current, Vec2::new(0.04, -0.02));
        for _ in 0..30 {
            current = camera.update(
                current,
                chase,
                controlled_position,
                Vec3::NEG_Z,
                1.0 / 60.0,
                PLANET_RADIUS,
            );
        }
        let detached_rotation = current.rotation;

        for _ in 0..30 {
            current = camera.update(
                current,
                chase,
                controlled_position,
                Vec3::X,
                1.0 / 60.0,
                PLANET_RADIUS,
            );
        }

        assert!(current.rotation.angle_between(detached_rotation) < 0.01);
    }
}
