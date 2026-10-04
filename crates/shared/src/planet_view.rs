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
    follow: bool,
    requested_radius: f32,
    attained_radius: f32,
    detached_direction: Vec3,
    transition: Option<Transition>,
}

impl Default for PlanetViewCamera {
    fn default() -> Self {
        Self {
            phase: Phase::Chase,
            requested_open: false,
            follow: true,
            requested_radius: PLANET_VIEW_FAR_RADIUS,
            attained_radius: PLANET_RADIUS,
            detached_direction: Vec3::Y,
            transition: None,
        }
    }
}

impl PlanetViewCamera {
    /// Toggle entry or return. Reversals begin at the current rendered pose.
    pub fn toggle(&mut self) {
        self.requested_open = !self.requested_open;
    }

    /// Request a return to the gameplay chase camera.
    pub fn close(&mut self) {
        self.requested_open = false;
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
        }
    }

    /// Start following the current controlled body's radial position.
    pub fn follow_body(&mut self) {
        self.follow = true;
    }

    /// Preserve the current view direction while the body moves.
    pub fn detach(&mut self, current_direction: Vec3) {
        self.follow = false;
        if current_direction.is_finite() && current_direction.length_squared() > 1e-8 {
            self.detached_direction = current_direction.normalize();
        }
    }

    /// Apply orbit deltas in radians and detach from the controlled body.
    pub fn orbit(&mut self, delta: Vec2) {
        if !delta.is_finite() || delta == Vec2::ZERO {
            return;
        }
        let direction = self.detached_direction.normalize_or(Vec3::Y);
        let east = Vec3::Y
            .cross(direction)
            .normalize_or(direction.any_orthonormal_vector());
        let north = direction.cross(east).normalize_or(Vec3::Z);
        let yaw = Quat::from_axis_angle(north, -delta.x);
        let yawed = yaw * direction;
        let orbited = Quat::from_axis_angle(yaw * east, delta.y) * yawed;
        self.detach(orbited);
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
            if opening {
                self.follow = true;
                self.requested_radius = PLANET_VIEW_FAR_RADIUS;
                self.phase = Phase::Opening;
            } else {
                self.phase = Phase::Returning;
            }
            let from_direction = current
                .translation
                .normalize_or(controlled_position.normalize_or(Vec3::Y));
            self.detached_direction = from_direction;
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
            let target_heading = tangent_heading(controlled_heading, direction);
            let target_rotation = planet_rotation(direction, target_heading);
            let transform = if transition.opening {
                let radial = radial_slerp(transition.from_direction, direction, eased);
                let start_radius = transition.from.translation.length();
                let radius = lerp(start_radius, requested_radius, eased);
                Transform {
                    translation: radial * radius,
                    rotation: transition.from.rotation.slerp(target_rotation, eased),
                    ..default()
                }
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
            let radius = requested_radius;
            let heading = tangent_heading(controlled_heading, direction);
            planet_pose(direction, radius, heading)
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
