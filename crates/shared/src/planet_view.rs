//! Reusable camera policy for the live planet view.
//!
//! This module owns camera intent and pose policy only. Runtime systems remain
//! responsible for writing the active camera transform and enforcing collision
//! clearance against the loaded world.

use bevy::prelude::*;

use crate::sphere::{PLANET_RADIUS, tangent_heading};

/// Nearest requested camera radius for a local overview around the controlled body.
pub const PLANET_VIEW_NEAR_RADIUS: f32 = PLANET_RADIUS + 400.0;
/// Farthest requested camera radius, enough to frame the whole planet.
pub const PLANET_VIEW_FAR_RADIUS: f32 = PLANET_RADIUS * 3.0;

/// Approximate globe rotation in radians per logical pixel at screen center.
/// Surface-hit dragging uses exact ray intersections; this gain keeps movement
/// consistent when a cursor ray misses the planet.
pub fn orbit_radians_per_logical_pixel(
    camera_radius: f32,
    logical_viewport_height: f32,
    vertical_fov: f32,
) -> f32 {
    if !camera_radius.is_finite()
        || !logical_viewport_height.is_finite()
        || logical_viewport_height <= 0.0
        || !vertical_fov.is_finite()
        || !(0.0..std::f32::consts::PI).contains(&vertical_fov)
    {
        return 0.0;
    }
    let focal_length = logical_viewport_height / (2.0 * (vertical_fov * 0.5).tan());
    let altitude = (camera_radius - PLANET_RADIUS).max(0.0);
    altitude / (PLANET_RADIUS * focal_length)
}

/// Return the nominal globe direction hit by a logical-pixel camera ray.
pub fn planet_surface_hit_direction(
    camera: Transform,
    logical_cursor: Vec2,
    logical_viewport: Vec2,
    vertical_fov: f32,
) -> Option<Vec3> {
    if !camera.translation.is_finite()
        || !camera.rotation.is_finite()
        || !logical_cursor.is_finite()
        || !logical_viewport.is_finite()
        || logical_viewport.min_element() <= 0.0
        || !vertical_fov.is_finite()
        || !(0.0..std::f32::consts::PI).contains(&vertical_fov)
    {
        return None;
    }

    let half_fov_tangent = (vertical_fov * 0.5).tan();
    let aspect = logical_viewport.x / logical_viewport.y;
    let normalized_x = logical_cursor.x / logical_viewport.x * 2.0 - 1.0;
    let normalized_y = 1.0 - logical_cursor.y / logical_viewport.y * 2.0;
    let ray = (camera.rotation
        * Vec3::new(
            normalized_x * aspect * half_fov_tangent,
            normalized_y * half_fov_tangent,
            -1.0,
        )
        .normalize())
    .normalize_or_zero();
    if ray == Vec3::ZERO {
        return None;
    }

    let origin = camera.translation;
    let radius_squared = PLANET_RADIUS * PLANET_RADIUS;
    let origin_squared = origin.length_squared();
    if origin_squared <= radius_squared {
        return None;
    }
    let along = origin.dot(ray);
    let discriminant = along * along - (origin_squared - radius_squared);
    if !discriminant.is_finite() || discriminant < 0.0 {
        return None;
    }
    let root = discriminant.sqrt();
    let near = -along - root;
    let far = -along + root;
    let distance = if near > 0.0 { near } else { far };
    if !distance.is_finite() || distance <= 0.0 {
        return None;
    }
    let hit = origin + ray * distance;
    (hit.length_squared() > 1e-8).then(|| hit.normalize())
}

/// Rotation that moves the camera so the current globe hit returns to the
/// previously grabbed globe direction. Antipodal inputs are rejected because
/// they do not define a stable rotation axis for one pointer event.
pub fn planet_surface_drag_rotation(previous_hit: Vec3, current_hit: Vec3) -> Option<Quat> {
    if !previous_hit.is_finite()
        || !current_hit.is_finite()
        || previous_hit.length_squared() <= 1e-8
        || current_hit.length_squared() <= 1e-8
    {
        return None;
    }
    let previous = previous_hit.normalize();
    let current = current_hit.normalize();
    if previous.dot(current) <= -0.9999 {
        return None;
    }
    Some(Quat::from_rotation_arc(current, previous))
}

const CAMERA_CLEARANCE: f32 = 5.0;
const TRANSIT_CLEARANCE: f32 = 350.0;
const OPENING_SECONDS: f32 = 1.4;
const RETURN_SECONDS: f32 = 1.8;
const EXTRA_FAR_RETURN_SECONDS: f32 = 0.6;
const FOLLOW_REACQUIRE_ANGLE: f32 = 2.0_f32.to_radians();
// Small course changes can close directly without waiting at the transit shell.
const DIRECT_RETURN_MAX_ANGLE: f32 = 45.0_f32.to_radians();
const MOTION_RESET_LINEAR_SPEED: f32 = 50_000.0;
const MOTION_RESET_ANGULAR_SPEED: f32 = 30.0;
const INTERRUPTION_RELEASE_SECONDS: f32 = 0.3;
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
    opening_focus: Vec3,
    from_linear_velocity: Vec3,
    from_angular_velocity: Vec3,
    elapsed: f32,
    duration: f32,
    opening: bool,
    transit_radius: f32,
    direct_return: bool,
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
    follow_recenter: bool,
    requested_radius: f32,
    attained_radius: f32,
    detached_direction: Vec3,
    detached_heading: Vec3,
    // Previous runtime-accepted pose; the next update measures motion from it
    // so interrupted transitions inherit actual, collision-constrained motion.
    last_motion_pose: Option<Transform>,
    last_linear_velocity: Vec3,
    last_angular_velocity: Vec3,
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
            follow_recenter: false,
            requested_radius: PLANET_VIEW_FAR_RADIUS,
            attained_radius: PLANET_RADIUS,
            detached_direction: Vec3::Y,
            detached_heading: Vec3::NEG_Z,
            last_motion_pose: None,
            last_linear_velocity: Vec3::ZERO,
            last_angular_velocity: Vec3::ZERO,
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
            if self.is_active() && self.phase == Phase::Chase {
                // A zoom received in the same frame as the first open request
                // is an explicit user radius. Do not replace it with the
                // default far radius when the camera transition starts.
                self.resume_on_open = true;
            }
            if self.is_active() && !self.requested_open {
                self.requested_open = true;
                self.resume_on_open = true;
            }
        }
    }

    /// Start following the current controlled body's radial position.
    pub fn follow_body(&mut self) {
        self.follow = true;
        self.follow_recenter = true;
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
            self.follow_recenter = false;
            self.detached_direction = direction;
            self.detached_heading = tangent_heading(current.rotation * Vec3::Y, direction);
        } else {
            self.follow_body();
        }
    }

    /// Preserve the current view direction while the body moves.
    pub fn detach(&mut self, current_direction: Vec3) {
        self.follow = false;
        self.follow_recenter = false;
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
        self.follow_recenter = false;
        if self.is_active() && !self.requested_open {
            self.requested_open = true;
            self.resume_on_open = true;
            self.retain_orbit_target_on_open = true;
        }
    }

    /// Apply an incremental screen-space drag from the attained camera pose.
    /// The first drag starts from the current followed or returning direction,
    /// while later deltas accumulate on the detached target. `delta.y` follows
    /// the pointer's down-positive screen coordinate.
    pub fn orbit_from(&mut self, current: Transform, delta: Vec2) {
        if !delta.is_finite() || delta == Vec2::ZERO {
            return;
        }
        let screen_right = current.rotation * Vec3::X;
        let screen_up = current.rotation * Vec3::Y;
        let rotation = Quat::from_scaled_axis(-screen_up * delta.x - screen_right * delta.y);
        self.orbit_from_rotation(current, rotation);
    }

    /// Keep a nominal globe point under the pointer while dragging between two
    /// camera rays. Returns false when the pair cannot define a stable rotation.
    pub fn orbit_from_surface_drag(
        &mut self,
        current: Transform,
        previous_hit: Vec3,
        current_hit: Vec3,
    ) -> bool {
        let Some(rotation) = planet_surface_drag_rotation(previous_hit, current_hit) else {
            return false;
        };
        let (_, angle) = rotation.to_axis_angle();
        if !angle.is_finite() || angle <= 1e-7 {
            return false;
        }
        self.orbit_from_rotation(current, rotation);
        true
    }

    fn orbit_from_rotation(&mut self, current: Transform, rotation: Quat) {
        if !rotation.is_finite() {
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
        let direction = (rotation * direction).normalize_or(direction);
        self.detached_direction = direction;
        self.detached_heading = tangent_heading(rotation * heading, direction);
        self.follow = false;
        self.follow_recenter = false;
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
        if delta_seconds > 1e-6
            && let Some(previous) = self.last_motion_pose
        {
            let translation_delta = current.translation - previous.translation;
            let delta_rotation = shortest_rotation_delta(previous.rotation, current.rotation);
            let (axis, angle) = delta_rotation.to_axis_angle();
            let position_speed = translation_delta.length() / delta_seconds;
            let angular_speed = angle / delta_seconds;
            if position_speed > MOTION_RESET_LINEAR_SPEED
                || angular_speed > MOTION_RESET_ANGULAR_SPEED
            {
                self.last_linear_velocity = Vec3::ZERO;
                self.last_angular_velocity = Vec3::ZERO;
            } else {
                self.last_linear_velocity = translation_delta / delta_seconds;
                self.last_angular_velocity = axis * angular_speed;
            }
        }
        self.last_motion_pose = Some(current);

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
            let chase_direction = chase
                .translation
                .normalize_or(controlled_position.normalize_or(Vec3::Y));
            let return_angle = from_direction.angle_between(chase_direction);
            let direct_return = !opening && return_angle <= DIRECT_RETURN_MAX_ANGLE;
            if !retain_orbit_target {
                self.detached_direction = from_direction;
            }
            self.transition = Some(Transition {
                from: current,
                from_direction,
                opening_focus: if opening {
                    opening_surface_focus(current, surface_radius.max(PLANET_RADIUS))
                } else {
                    Vec3::ZERO
                },
                from_linear_velocity: self.last_linear_velocity,
                from_angular_velocity: self.last_angular_velocity,
                elapsed: 0.0,
                duration: if opening {
                    OPENING_SECONDS
                } else {
                    return_duration(return_angle)
                },
                opening,
                transit_radius: current
                    .translation
                    .length()
                    .max(PLANET_RADIUS + TRANSIT_CLEARANCE),
                direct_return,
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
        if self.follow
            && !self.follow_recenter
            && self.transition.is_none()
            && current
                .translation
                .normalize_or(direction)
                .angle_between(direction)
                > FOLLOW_REACQUIRE_ANGLE
        {
            self.follow_recenter = true;
        }

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
                let mut result = smooth_planet_pose(
                    transition.from,
                    target,
                    eased,
                    surface_radius.max(PLANET_RADIUS) + CAMERA_CLEARANCE,
                );
                result.translation = carry_initial_linear_velocity(
                    result.translation,
                    transition.from_linear_velocity,
                    transition.elapsed,
                    surface_radius.max(PLANET_RADIUS) + CAMERA_CLEARANCE,
                );
                result.rotation = opening_rotation(
                    result.translation,
                    transition.opening_focus,
                    transition.from.rotation * Vec3::NEG_Z,
                    transition.from.rotation * Vec3::Y,
                    tangent_heading(target_heading, result.translation.normalize_or(direction)),
                    eased,
                );
                let path_angular_velocity = angular_velocity_between(
                    transition.from.rotation,
                    result.rotation,
                    transition.elapsed,
                );
                let residual_angular_velocity =
                    transition.from_angular_velocity - path_angular_velocity;
                let local_residual = result.rotation.inverse()
                    * transition.from.rotation
                    * residual_angular_velocity;
                result.rotation = carry_initial_angular_velocity(
                    result.rotation,
                    local_residual,
                    transition.elapsed,
                );
                result
            } else {
                let chase_direction = chase
                    .translation
                    .normalize_or(controlled_position.normalize_or(Vec3::Y));
                let (radial, radius, frame_progress, chase_blend) = return_path(
                    transition.from_direction,
                    chase_direction,
                    transition.from.translation.length(),
                    transition.transit_radius.max(minimum_radius),
                    chase.translation.length(),
                    linear,
                    transition.direct_return,
                );
                let path_position = radial * radius;
                let path_rotation =
                    radial_rotation(transition.from_direction, chase_direction, frame_progress)
                        * transition.from.rotation;
                let minimum_radius = surface_radius.max(PLANET_RADIUS) + CAMERA_CLEARANCE;
                let translation = carry_initial_linear_velocity(
                    path_position.lerp(chase.translation, chase_blend),
                    transition.from_linear_velocity,
                    transition.elapsed,
                    minimum_radius,
                );
                Transform {
                    translation,
                    rotation: carry_initial_angular_velocity(
                        path_rotation.slerp(chase.rotation, chase_blend),
                        transition.from_angular_velocity,
                        transition.elapsed,
                    ),
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
            let response = response(VIEW_RESPONSE, delta_seconds);
            if self.follow && !self.follow_recenter {
                let radius = lerp(current.translation.length(), requested_radius, response)
                    .max(minimum_radius);
                Transform {
                    translation: direction * radius,
                    rotation: current.rotation.slerp(target.rotation, response),
                    ..default()
                }
            } else {
                smooth_planet_pose(
                    current,
                    target,
                    response,
                    surface_radius.max(PLANET_RADIUS) + CAMERA_CLEARANCE,
                )
            }
        } else {
            self.phase = Phase::Chase;
            self.attained_radius = chase.translation.length();
            chase
        };

        if self.follow
            && self.follow_recenter
            && result
                .translation
                .normalize_or(direction)
                .angle_between(direction)
                < 0.0001
        {
            self.follow_recenter = false;
        }

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

fn opening_surface_focus(from: Transform, surface_radius: f32) -> Vec3 {
    let forward = from.rotation * Vec3::NEG_Z;
    let projected_origin = from.translation.dot(forward);
    let radius = surface_radius.max(PLANET_RADIUS);
    let discriminant =
        projected_origin * projected_origin + radius * radius - from.translation.length_squared();
    if discriminant >= 0.0 {
        let distance = -projected_origin - discriminant.sqrt();
        if distance >= 0.0 {
            return from.translation + forward * distance;
        }
    }

    from.translation + forward * radius
}

fn opening_rotation(
    position: Vec3,
    surface_focus: Vec3,
    start_forward: Vec3,
    start_up: Vec3,
    target_up: Vec3,
    progress: f32,
) -> Quat {
    let focus = surface_focus.lerp(Vec3::ZERO, progress);
    let start_forward = start_forward.normalize_or(Vec3::NEG_Z);
    let forward = (focus - position).normalize_or(start_forward);
    let transported_up = rotate_direction(start_forward, forward, start_up) * start_up;
    let transported_up = project_onto_view_plane(transported_up, forward)
        .normalize_or(start_up.normalize_or(Vec3::Y));
    let desired_up = project_onto_view_plane(target_up, forward).normalize_or(transported_up);
    let roll = signed_angle(transported_up, desired_up, forward);
    let up = Quat::from_axis_angle(forward, roll * progress.clamp(0.0, 1.0)) * transported_up;
    Transform::from_translation(position)
        .looking_at(focus, up)
        .rotation
}

fn rotate_direction(from: Vec3, to: Vec3, preferred_axis: Vec3) -> Quat {
    let from = from.normalize_or(Vec3::NEG_Z);
    let to = to.normalize_or(from);
    if from.dot(to) < -0.9999 {
        let axis = project_onto_view_plane(preferred_axis, from)
            .normalize_or(from.any_orthonormal_vector());
        Quat::from_axis_angle(axis, std::f32::consts::PI)
    } else {
        Quat::from_rotation_arc(from, to)
    }
}

fn project_onto_view_plane(vector: Vec3, forward: Vec3) -> Vec3 {
    vector - forward * vector.dot(forward)
}

fn signed_angle(from: Vec3, to: Vec3, axis: Vec3) -> f32 {
    let sine = axis.dot(from.cross(to));
    let cosine = from.dot(to).clamp(-1.0, 1.0);
    if sine.abs() <= 1e-6 && cosine < 0.0 {
        std::f32::consts::PI
    } else {
        sine.atan2(cosine)
    }
}

fn shortest_rotation_delta(from: Quat, to: Quat) -> Quat {
    let delta = from.inverse() * to;
    if delta.w < 0.0 { -delta } else { delta }
}

fn angular_velocity_between(from: Quat, to: Quat, elapsed: f32) -> Vec3 {
    if elapsed <= 1e-6 {
        return Vec3::ZERO;
    }
    let (axis, angle) = shortest_rotation_delta(from, to).to_axis_angle();
    axis * (angle / elapsed)
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

fn radial_slerp(from: Vec3, to: Vec3, amount: f32) -> Vec3 {
    let from = from.normalize_or(Vec3::Y);
    (radial_rotation(from, to, amount) * from).normalize()
}

fn radial_rotation(from: Vec3, to: Vec3, amount: f32) -> Quat {
    let from = from.normalize_or(Vec3::Y);
    let to = to.normalize_or(from);
    if from.dot(to) < -0.9999 {
        let axis = from.any_orthonormal_vector();
        return Quat::from_axis_angle(axis, std::f32::consts::PI * amount);
    }
    Quat::IDENTITY.slerp(Quat::from_rotation_arc(from, to), amount)
}

fn return_path(
    from_direction: Vec3,
    to_direction: Vec3,
    from_radius: f32,
    transit_radius: f32,
    to_radius: f32,
    progress: f32,
    direct: bool,
) -> (Vec3, f32, f32, f32) {
    if direct {
        let amount = smoothstep(progress);
        return (
            radial_slerp(from_direction, to_direction, amount),
            lerp(from_radius, to_radius, amount),
            amount,
            amount,
        );
    }

    const RISE_END: f32 = 0.15;
    const SWING_END: f32 = 0.75;
    if progress < RISE_END {
        let amount = smoothstep(progress / RISE_END);
        (
            from_direction,
            lerp(from_radius, transit_radius, amount),
            0.0,
            0.0,
        )
    } else if progress < SWING_END {
        let amount = smoothstep((progress - RISE_END) / (SWING_END - RISE_END));
        (
            radial_slerp(from_direction, to_direction, amount),
            transit_radius,
            amount,
            0.0,
        )
    } else {
        let amount = smoothstep((progress - SWING_END) / (1.0 - SWING_END));
        (
            to_direction,
            lerp(transit_radius, to_radius, amount),
            1.0,
            amount,
        )
    }
}

fn return_duration(angle: f32) -> f32 {
    // Long radial swings earn extra time instead of accelerating the same
    // half-globe route into the final chase alignment.
    RETURN_SECONDS + (angle / std::f32::consts::PI).clamp(0.0, 1.0) * EXTRA_FAR_RETURN_SECONDS
}

fn carry_initial_angular_velocity(rotation: Quat, velocity: Vec3, elapsed: f32) -> Quat {
    let correction_seconds = initial_velocity_correction_seconds(elapsed);
    if velocity.length_squared() <= 1e-8 || correction_seconds <= 0.0 {
        return rotation;
    }
    rotation * Quat::from_scaled_axis(velocity * correction_seconds)
}

fn carry_initial_linear_velocity(
    position: Vec3,
    velocity: Vec3,
    elapsed: f32,
    minimum_radius: f32,
) -> Vec3 {
    let correction_seconds = initial_velocity_correction_seconds(elapsed);
    if velocity.length_squared() <= 1e-8 || correction_seconds <= 0.0 {
        return position;
    }
    let carried = position + velocity * correction_seconds;
    if carried.length() < minimum_radius {
        carried.normalize_or(position.normalize_or(Vec3::Y)) * minimum_radius
    } else {
        carried
    }
}

fn initial_velocity_correction_seconds(elapsed: f32) -> f32 {
    if elapsed <= 0.0 || elapsed >= INTERRUPTION_RELEASE_SECONDS {
        return 0.0;
    }
    let progress = elapsed / INTERRUPTION_RELEASE_SECONDS;
    // This finite pulse matches the incoming velocity at the interruption,
    // then rejoins the eased path with zero offset and zero slope.
    elapsed * (1.0 - progress).powi(2)
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

    fn project_logical(camera: Transform, world_point: Vec3, viewport: Vec2, fov_y: f32) -> Vec2 {
        let view = camera.rotation.conjugate() * (world_point - camera.translation);
        let focal_length = viewport.y / (2.0 * (fov_y * 0.5).tan());
        Vec2::new(
            viewport.x * 0.5 + focal_length * view.x / -view.z,
            viewport.y * 0.5 - focal_length * view.y / -view.z,
        )
    }

    #[test]
    fn screen_drag_tracks_pointer_across_headings_and_near_poles() {
        let start_directions = [
            Vec3::Y,
            Vec3::new(0.02, 0.999, -0.03).normalize(),
            Vec3::new(-0.62, 0.21, 0.75).normalize(),
            Vec3::new(0.24, -0.91, -0.34).normalize(),
        ];

        for start_direction in start_directions {
            let world_up = if start_direction.dot(Vec3::Y).abs() > 0.95 {
                Vec3::Z
            } else {
                Vec3::Y
            };
            let current = Transform::from_translation(start_direction * PLANET_VIEW_FAR_RADIUS)
                .looking_at(Vec3::ZERO, world_up);
            let screen_right = current.rotation * Vec3::X;
            let screen_up = current.rotation * Vec3::Y;

            for delta in [Vec2::new(0.02, 0.0), Vec2::new(-0.02, 0.0)] {
                let mut camera = PlanetViewCamera::default();
                camera.orbit_from(current, delta);
                let radial_motion = camera.view_direction(Vec3::Y) - start_direction;
                assert!(
                    radial_motion.dot(screen_right) * delta.x < 0.0,
                    "horizontal screen drag {delta:?} moved camera radially with the pointer at {start_direction:?}"
                );
            }

            for delta in [Vec2::new(0.0, 0.02), Vec2::new(0.0, -0.02)] {
                let mut camera = PlanetViewCamera::default();
                camera.orbit_from(current, delta);
                let radial_motion = camera.view_direction(Vec3::Y) - start_direction;
                assert!(
                    radial_motion.dot(screen_up) * delta.y > 0.0,
                    "vertical screen drag {delta:?} moved camera radially opposite the pointer at {start_direction:?}"
                );
            }
        }
    }

    #[test]
    fn off_center_surface_anchor_stays_under_pointer_at_multiple_zoom_levels() {
        let viewport = Vec2::new(1280.0, 720.0);
        let fov_y = 50.0_f32.to_radians();
        let previous_cursor = Vec2::new(viewport.x * 0.61, viewport.y * 0.46);
        let current_cursor = Vec2::new(viewport.x * 0.65, viewport.y * 0.43);
        let directions = [
            Vec3::Y,
            Vec3::new(0.02, 0.999, -0.03).normalize(),
            Vec3::new(-0.62, 0.21, 0.75).normalize(),
            Vec3::new(0.24, -0.91, -0.34).normalize(),
        ];

        for radius in [
            PLANET_VIEW_NEAR_RADIUS,
            (PLANET_VIEW_NEAR_RADIUS + PLANET_VIEW_FAR_RADIUS) * 0.5,
            PLANET_VIEW_FAR_RADIUS,
        ] {
            for direction in directions {
                let world_up = if direction.dot(Vec3::Y).abs() > 0.95 {
                    Vec3::Z
                } else {
                    Vec3::Y
                };
                let camera = Transform::from_translation(direction * radius)
                    .looking_at(Vec3::ZERO, world_up);
                let previous_hit =
                    planet_surface_hit_direction(camera, previous_cursor, viewport, fov_y)
                        .expect("previous pointer ray should hit the visible globe");
                let current_hit =
                    planet_surface_hit_direction(camera, current_cursor, viewport, fov_y)
                        .expect("current pointer ray should hit the visible globe");
                let rotation = planet_surface_drag_rotation(previous_hit, current_hit)
                    .expect("nearby globe points define a stable drag rotation");
                let mut policy = PlanetViewCamera::default();
                assert!(policy.orbit_from_surface_drag(camera, previous_hit, current_hit));
                assert!(
                    policy.view_direction(Vec3::Y).dot(rotation * direction) > 0.99999,
                    "camera policy did not follow the surface anchor rotation at radius {radius:.1}"
                );
                let moved_camera = Transform {
                    translation: rotation * camera.translation,
                    rotation: rotation * camera.rotation,
                    ..camera
                };
                let anchored_point = previous_hit * PLANET_RADIUS;
                let projected = project_logical(moved_camera, anchored_point, viewport, fov_y);
                let residual = projected.distance(current_cursor);
                assert!(
                    residual < 0.05,
                    "grabbed point slipped {residual:.4} logical pixels at radius {radius:.1}, direction {direction:?}"
                );
            }
        }
    }

    #[test]
    fn fallback_orbit_gain_tracks_viewport_and_attained_zoom() {
        let fov_y = 50.0_f32.to_radians();
        let near = orbit_radians_per_logical_pixel(PLANET_VIEW_NEAR_RADIUS, 720.0, fov_y);
        let middle = orbit_radians_per_logical_pixel(
            (PLANET_VIEW_NEAR_RADIUS + PLANET_VIEW_FAR_RADIUS) * 0.5,
            720.0,
            fov_y,
        );
        let far = orbit_radians_per_logical_pixel(PLANET_VIEW_FAR_RADIUS, 720.0, fov_y);

        assert!(near > 0.0 && near < middle && middle < far);
        let expected_far_to_near =
            (PLANET_VIEW_FAR_RADIUS - PLANET_RADIUS) / (PLANET_VIEW_NEAR_RADIUS - PLANET_RADIUS);
        assert!((far / near - expected_far_to_near).abs() < 1e-5);
        assert!(
            (orbit_radians_per_logical_pixel(PLANET_VIEW_NEAR_RADIUS, 1440.0, fov_y) - near * 0.5)
                .abs()
                < 1e-7
        );
        assert_eq!(
            orbit_radians_per_logical_pixel(PLANET_VIEW_FAR_RADIUS, 0.0, fov_y),
            0.0
        );

        let camera = Transform::from_translation(Vec3::Z * PLANET_VIEW_FAR_RADIUS)
            .looking_at(Vec3::ZERO, Vec3::Y);
        assert!(
            planet_surface_hit_direction(
                camera,
                Vec2::new(1280.0, 360.0),
                Vec2::new(1280.0, 720.0),
                fov_y,
            )
            .is_none()
        );
    }

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
        camera.orbit_from(before_orbit, delta);
        let target_direction = camera.view_direction(controlled_position);
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
        let selected_zoom = PLANET_VIEW_NEAR_RADIUS + 800.0;
        camera.request_radius(selected_zoom);
        for _ in 0..180 {
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
        assert!((current.translation.length() - selected_zoom).abs() < 0.1);

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
        assert!(
            (first_follow_step.translation.length() - first_detached.translation.length()).abs()
                < 1.0,
            "re-enabling follow should preserve the attained zoom while recentering"
        );
        assert_eq!(camera.requested_radius(), selected_zoom);
        current = first_follow_step;

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
        assert!(current.translation.normalize().dot(Vec3::X) > 0.9999);
        let moved_body = Quat::from_rotation_z(0.04 / 60.0) * controlled_position;
        current = camera.update(
            current,
            chase,
            moved_body,
            Vec3::Z,
            1.0 / 60.0,
            PLANET_RADIUS,
        );
        assert!(
            current
                .translation
                .normalize()
                .angle_between(moved_body.normalize())
                < 0.0001,
            "follow should track without trailing once the smooth recenter completes"
        );
    }

    #[test]
    fn active_follow_tracks_a_moving_body_without_positional_trailing() {
        let initial_position = Vec3::Y * PLANET_RADIUS;
        let chase = Transform::from_xyz(0.0, PLANET_RADIUS + 5.0, -5.0)
            .looking_at(initial_position, Vec3::Y);
        let mut current = chase;
        let mut camera = PlanetViewCamera::default();
        camera.toggle();
        for _ in 0..120 {
            current = camera.update(
                current,
                chase,
                initial_position,
                Vec3::NEG_Z,
                1.0 / 60.0,
                PLANET_RADIUS,
            );
            camera.finish_transition_if_ready();
        }

        let dt = 1.0 / 60.0;
        let angular_step = 0.04 * dt;
        for frame in 1..=60 {
            let body_position =
                Quat::from_rotation_z(angular_step * frame as f32) * initial_position;
            current = camera.update(
                current,
                chase,
                body_position,
                Vec3::NEG_Z,
                dt,
                PLANET_RADIUS,
            );
        }

        let body_position = Quat::from_rotation_z(angular_step * 60.0) * initial_position;
        let trailing_angle = current
            .translation
            .normalize()
            .angle_between(body_position.normalize());
        assert!(
            trailing_angle < 0.0001,
            "active follow should have no positional trailing after recentering, got {trailing_angle} radians"
        );
    }

    #[test]
    fn active_follow_repositions_smoothly_after_a_body_relocation() {
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

        controlled_position = Vec3::X * PLANET_RADIUS;
        let first_recenter_step = camera.update(
            current,
            chase,
            controlled_position,
            Vec3::Z,
            1.0 / 60.0,
            PLANET_RADIUS,
        );
        assert!(
            first_recenter_step
                .translation
                .normalize()
                .angle_between(current.translation.normalize())
                < 0.5,
            "a body relocation should begin with a smooth camera recenter"
        );
        current = first_recenter_step;

        for _ in 0..120 {
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
        assert!(current.translation.normalize().dot(Vec3::X) > 0.9999);
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
    fn nearest_zoom_keeps_the_camera_above_overview_altitude() {
        let mut camera = PlanetViewCamera::default();
        camera.request_radius(PLANET_RADIUS);

        assert!(
            camera.requested_radius() >= PLANET_RADIUS + 400.0,
            "nearest Planet view zoom should retain at least 400 m of surrounding context"
        );
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
