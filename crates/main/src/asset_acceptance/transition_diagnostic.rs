//! Opt-in production transition, movement, and weather visual diagnostic.

use super::planet_acceptance::{
    MAX_CONTINUOUS_BODY_STEP_M, MIN_MEASURED_BODY_EXCURSION_M, MIN_MEASURED_BODY_PATH_M,
    acceptance_world_ready, continuous_body_path, count_collision_colliders, csv, file_is_nonempty,
    main_camera_pose, player_motion, player_pose, primary_window, set_key,
};
use crate::{
    exploration::{Action, Exploration, Kind},
    map::MainCamera,
    weather::{Precip, Weather},
};
use avian3d::prelude::{Collider, LinearVelocity, Position, Rotation, Sleeping};
use bevy::{
    camera::CameraUpdateSystems,
    input::InputSystems,
    prelude::*,
    render::view::screenshot::{Screenshot, save_to_disk},
    window::PrimaryWindow,
};
use shared::{planet_view::PLANET_VIEW_NEAR_RADIUS, state::AppState};
use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

const DIAGNOSTIC_MINIMUM_COLLIDERS: usize = 100;
const DIAGNOSTIC_STABLE_WORLD_FRAMES: u8 = 30;
const DIAGNOSTIC_MAX_ENTRY_CAPTURE_LATE_SECONDS: f64 = 0.2;
const DIAGNOSTIC_INITIAL_OPEN_DELAY_SECONDS: f64 = 0.1;
const DIAGNOSTIC_REVERSAL_MINIMUM_GAP_SECONDS: f64 = 0.55;
const DIAGNOSTIC_DRAG_READY_GAP_SECONDS: f64 = 0.2;
const DIAGNOSTIC_DRAG_CONFIRMATION_TIMEOUT_SECONDS: f64 = 2.0;
const DIAGNOSTIC_OPPOSITE_DOT_THRESHOLD: f32 = -0.995;
const DIAGNOSTIC_MOVEMENT_DURATION_SECONDS: f64 = 7.0;
const DIAGNOSTIC_FINAL_RETURN_GAP_SECONDS: f64 = 0.15;
const DIAGNOSTIC_STORM_REOPEN_GAP_SECONDS: f64 = 1.0;
const DIAGNOSTIC_STORM_CLOSE_GAP_SECONDS: f64 = 1.5;
const DIAGNOSTIC_RETURN_SETTLE_SECONDS: f64 = 2.0;
const DIAGNOSTIC_ROUTE_TIMEOUT_SECONDS: f64 = 150.0;
const FOLLOW_PROBE_MOVE_SECONDS: f64 = 3.0;
const FOLLOW_PROBE_CAPTURE_DELAY_SECONDS: f64 = 0.75;
const FOLLOW_PROBE_SECOND_CAPTURE_DELAY_SECONDS: f64 = 1.0;
const FOLLOW_PROBE_STEADY_WINDOW_SECONDS: f64 = 1.0;
const FOLLOW_PROBE_MIN_STEADY_SAMPLE_COUNT: usize = 10;
const FOLLOW_PROBE_MIN_STEADY_SPAN_SECONDS: f64 = 0.75;
const FOLLOW_PROBE_RADIUS_TOLERANCE_M: f32 = 1.0;
const FOLLOW_PROBE_MAX_RADIAL_LAG_RAD: f32 = 0.000_01;
const FOLLOW_PROBE_MAX_MARKER_REGISTRATION_ERROR_PHYSICAL_PX: f32 = 1.0;
const RENDERED_POSE_CASE_TIMEOUT_SECONDS: f64 = 30.0;
const DIAGNOSTIC_PARTIAL_VIEW_MIN_RADIUS_M: f32 = 2_050.0;
const DIAGNOSTIC_EXTERIOR_READY_RADIUS_M: f32 = 5_800.0;
const DIAGNOSTIC_RETURN_MIDPOINT_SECONDS: f64 = 1.2;
const DIAGNOSTIC_RETURN_END_SECONDS: f64 = 2.6;
const DIAGNOSTIC_CAPTURE_NAMES: [&str; 20] = [
    "entry-0400ms.png",
    "entry-0800ms.png",
    "entry-1400ms.png",
    "drag-opposite.png",
    "return-midpoint.png",
    "return-end.png",
    "storm-hidden.png",
    "storm-restored.png",
    "near-follow-on-foot-01.png",
    "near-follow-on-foot-02.png",
    "near-follow-car-01.png",
    "near-follow-car-02.png",
    "near-follow-plane-01.png",
    "near-follow-plane-02.png",
    "drag-far-oblique-before.png",
    "drag-far-oblique-after.png",
    "drag-mid-heading-before.png",
    "drag-mid-heading-after.png",
    "drag-near-polar-before.png",
    "drag-near-polar-after.png",
];

#[derive(Clone, Copy)]
struct RenderedPoseCase {
    name: &'static str,
    before_capture: &'static str,
    after_capture: &'static str,
    target_radius: f32,
    target_direction: Vec3,
    start_cursor_offset: Vec2,
}

const RENDERED_POSE_CASES: [RenderedPoseCase; 3] = [
    RenderedPoseCase {
        name: "far-oblique",
        before_capture: "drag-far-oblique-before.png",
        after_capture: "drag-far-oblique-after.png",
        target_radius: shared::planet_view::PLANET_VIEW_FAR_RADIUS,
        target_direction: Vec3::new(0.44, 0.62, -0.64),
        start_cursor_offset: Vec2::new(90.0, -55.0),
    },
    RenderedPoseCase {
        name: "mid-heading",
        before_capture: "drag-mid-heading-before.png",
        after_capture: "drag-mid-heading-after.png",
        target_radius: (shared::planet_view::PLANET_VIEW_NEAR_RADIUS
            + shared::planet_view::PLANET_VIEW_FAR_RADIUS)
            * 0.5,
        target_direction: Vec3::new(-0.75, -0.3, 0.59),
        start_cursor_offset: Vec2::new(-65.0, 35.0),
    },
    RenderedPoseCase {
        name: "near-polar",
        before_capture: "drag-near-polar-before.png",
        after_capture: "drag-near-polar-after.png",
        target_radius: shared::planet_view::PLANET_VIEW_NEAR_RADIUS + 250.0,
        target_direction: Vec3::new(0.07, 0.995, -0.05),
        start_cursor_offset: Vec2::new(80.0, 45.0),
    },
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum RenderedPoseStage {
    #[default]
    Waiting,
    Zooming,
    WaitingForBeforeCapture,
    Dragging,
    WaitingForAfterCapture,
    WaitingForAfterCaptureSave,
    Complete,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FollowProbeMode {
    OnFoot,
    Car,
    Plane,
}

impl FollowProbeMode {
    fn name(self) -> &'static str {
        match self {
            Self::OnFoot => "on-foot",
            Self::Car => "car",
            Self::Plane => "plane",
        }
    }

    fn capture_names(self) -> [&'static str; 2] {
        match self {
            Self::OnFoot => ["near-follow-on-foot-01.png", "near-follow-on-foot-02.png"],
            Self::Car => ["near-follow-car-01.png", "near-follow-car-02.png"],
            Self::Plane => ["near-follow-plane-01.png", "near-follow-plane-02.png"],
        }
    }

    fn vehicle_kind(self) -> Option<Kind> {
        match self {
            Self::OnFoot => None,
            Self::Car => Some(Kind::Car),
            Self::Plane => Some(Kind::Plane),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum FollowProbeStage {
    #[default]
    Waiting,
    OnFoot,
    CloseView,
    SummonVehicle,
    ApproachVehicle,
    EnterVehicle,
    Vehicle,
    ExitVehicle,
    Complete,
    Failed,
}

#[derive(Resource, Default)]
struct PlanetTransitionDiagnostic {
    directory: PathBuf,
    started_at: Option<f64>,
    world_warmup: DiagnosticWorldWarmup,
    m_events_sent: [bool; 6],
    m_event_times: [Option<f64>; 6],
    drag_started: bool,
    drag_started_at: Option<f64>,
    drag_input_frames: u16,
    drag_frames: Vec<super::diagnostic_drag::DragInputFrame>,
    drag_next_frame: usize,
    drag_finished: bool,
    drag_finished_at: Option<f64>,
    drag_failed_at: Option<f64>,
    drag_start_direction: Vec3,
    drag_end_direction_dot: f32,
    drag_start_camera: Option<Transform>,
    drag_failure_camera: Option<Transform>,
    drag_start_cursor: Option<Vec2>,
    drag_end_cursor: Option<Vec2>,
    opposite_pose_confirmed: bool,
    opposite_pose_at: Option<f64>,
    rendered_pose_stage: RenderedPoseStage,
    rendered_pose_case_index: usize,
    rendered_pose_failed_cases: usize,
    rendered_pose_stage_started_at: Option<f64>,
    rendered_pose_zoom_requested: bool,
    rendered_pose_drag_frames: Vec<super::diagnostic_drag::DragInputFrame>,
    rendered_pose_drag_next_frame: usize,
    movement_started: bool,
    movement_started_at: Option<f64>,
    movement_finished: bool,
    movement_finished_at: Option<f64>,
    follow_probe_stage: FollowProbeStage,
    follow_probe_stage_started_at: Option<f64>,
    follow_probe_mode: Option<FollowProbeMode>,
    follow_probe_motion_started_at: Option<f64>,
    follow_probe_motion_start: Option<Vec3>,
    follow_probe_zoom_requested: bool,
    follow_probe_action_sent: bool,
    follow_probe_samples: Vec<FollowProbeSample>,
    storm_started: bool,
    route_timed_out: bool,
    requested_captures: HashSet<String>,
    samples: Vec<DiagnosticSample>,
    capture_metadata: Vec<String>,
    entry_capture_lateness_s: [f64; 3],
    errors: Vec<String>,
    finished: bool,
}

#[derive(Default)]
pub(super) struct DiagnosticWorldWarmup {
    last_collision_count: Option<usize>,
    stable_frames: u8,
}

impl DiagnosticWorldWarmup {
    pub(super) fn observe(&mut self, collision_count: usize) -> bool {
        if collision_count < DIAGNOSTIC_MINIMUM_COLLIDERS {
            self.last_collision_count = None;
            self.stable_frames = 0;
            return false;
        }
        if self.last_collision_count == Some(collision_count) {
            self.stable_frames = self.stable_frames.saturating_add(1);
        } else {
            self.last_collision_count = Some(collision_count);
            self.stable_frames = 1;
        }
        self.stable_frames >= DIAGNOSTIC_STABLE_WORLD_FRAMES
    }
}

fn transition_action_due(
    elapsed: f64,
    previous_action_at: Option<f64>,
    minimum_gap: f64,
    stage_ready: bool,
) -> bool {
    stage_ready && previous_action_at.is_some_and(|previous| elapsed - previous >= minimum_gap)
}

fn opposite_pose_reached(radial_dot: f32) -> bool {
    radial_dot.is_finite() && radial_dot <= DIAGNOSTIC_OPPOSITE_DOT_THRESHOLD
}

fn drag_confirmation_expired(
    drag_finished_at: Option<f64>,
    elapsed: f64,
    opposite_pose_confirmed: bool,
) -> bool {
    !opposite_pose_confirmed
        && drag_finished_at.is_some_and(|finished_at| {
            elapsed - finished_at >= DIAGNOSTIC_DRAG_CONFIRMATION_TIMEOUT_SECONDS
        })
}

fn drag_probe_ready(diagnostic: &PlanetTransitionDiagnostic) -> bool {
    diagnostic.opposite_pose_confirmed || diagnostic.drag_failed_at.is_some()
}

fn drag_progress_at(diagnostic: &PlanetTransitionDiagnostic) -> Option<f64> {
    diagnostic.opposite_pose_at.or(diagnostic.drag_failed_at)
}

fn rendered_pose_cases_passed(diagnostic: &PlanetTransitionDiagnostic) -> bool {
    diagnostic.rendered_pose_stage == RenderedPoseStage::Complete
        && diagnostic.rendered_pose_case_index == RENDERED_POSE_CASES.len()
        && diagnostic.rendered_pose_failed_cases == 0
}

fn radius_motion_flags(
    attained_radius: f32,
    previous_sample_radius: Option<f32>,
    latest_sample_radius: Option<f32>,
) -> (bool, bool) {
    const MIN_RADIUS_STEP_M: f32 = 0.25;

    // The driver runs in PreUpdate, before the camera update, while samples are
    // captured in PostUpdate. If the current radius still equals the latest
    // completed sample, infer the in-flight direction from the last two
    // completed camera poses instead of comparing the same value to itself.
    let latest_step = latest_sample_radius
        .map(|latest| attained_radius - latest)
        .unwrap_or(0.0);
    let sampled_step = previous_sample_radius
        .zip(latest_sample_radius)
        .map(|(previous, latest)| latest - previous)
        .unwrap_or(0.0);
    let step = if latest_step.abs() > MIN_RADIUS_STEP_M {
        latest_step
    } else {
        sampled_step
    };
    (step > MIN_RADIUS_STEP_M, step < -MIN_RADIUS_STEP_M)
}

#[derive(Clone, Copy, Default)]
struct DiagnosticSample {
    elapsed: f64,
    real_delta_ms: f64,
    fixed_time_elapsed: f64,
    planet_active: bool,
    planet_ready: bool,
    follows: bool,
    requested_radius: f32,
    attained_radius: f32,
    camera: Transform,
    player_position: Vec3,
    player_velocity: Vec3,
    player_sleeping: bool,
    collision_colliders: usize,
    virtual_rate: f32,
    virtual_paused: bool,
    weather_intensity: f32,
    weather_kind: &'static str,
    precipitation_active: usize,
    precipitation_hidden: usize,
    precipitation_visible: usize,
    first_particle: Vec3,
}

#[derive(Clone, Copy, Default)]
struct FollowProbeSample {
    elapsed: f64,
    motion_elapsed_s: f64,
    mode: Option<FollowProbeMode>,
    view_active: bool,
    follows: bool,
    requested_radius: f32,
    attained_radius: f32,
    body_speed_m_s: f32,
    radial_lag_rad: f32,
    projected_center_logical: Vec2,
    marker_center_physical: Vec2,
    marker_size_physical: Vec2,
    marker_anchor_body_separation_m: f32,
    marker_anchor_registration_error_physical_px: f32,
    marker_visible: bool,
    registration_error_physical_px: f32,
}

impl FollowProbeSample {
    fn csv_row(self) -> String {
        format!(
            "{:.5},{:.5},{},{},{},{:.4},{:.4},{:.4},{:.6},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{}\n",
            self.elapsed,
            self.motion_elapsed_s,
            self.mode.map_or("none", FollowProbeMode::name),
            self.view_active,
            self.follows,
            self.requested_radius,
            self.attained_radius,
            self.body_speed_m_s,
            self.radial_lag_rad,
            self.projected_center_logical.x,
            self.projected_center_logical.y,
            self.marker_center_physical.x,
            self.marker_center_physical.y,
            self.marker_size_physical.x,
            self.marker_size_physical.y,
            self.marker_anchor_body_separation_m,
            self.marker_anchor_registration_error_physical_px,
            self.registration_error_physical_px,
            self.marker_visible,
        )
    }
}

impl DiagnosticSample {
    fn csv_row(self) -> String {
        format!(
            "{:.5},{:.3},{:.6},{},{},{},{:.4},{:.4},{:.4},{:.4},{:.4},{:.6},{:.6},{:.6},{:.6},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{},{},{:.4},{},{:.4},{},{},{},{},{:.4},{:.4},{:.4}\n",
            self.elapsed,
            self.real_delta_ms,
            self.fixed_time_elapsed,
            self.planet_active,
            self.planet_ready,
            self.follows,
            self.requested_radius,
            self.attained_radius,
            self.camera.translation.x,
            self.camera.translation.y,
            self.camera.translation.z,
            self.camera.rotation.x,
            self.camera.rotation.y,
            self.camera.rotation.z,
            self.camera.rotation.w,
            self.player_position.x,
            self.player_position.y,
            self.player_position.z,
            self.player_velocity.x,
            self.player_velocity.y,
            self.player_velocity.z,
            self.player_sleeping,
            self.collision_colliders,
            self.virtual_rate,
            self.virtual_paused,
            self.weather_intensity,
            self.weather_kind,
            self.precipitation_active,
            self.precipitation_hidden,
            self.precipitation_visible,
            self.first_particle.x,
            self.first_particle.y,
            self.first_particle.z,
        )
    }
}

pub(super) fn register_transition_diagnostic(app: &mut App, directory: PathBuf) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("main crate lives below the workspace root")
        .canonicalize()
        .expect("canonicalize workspace root");
    fs::create_dir_all(&directory).expect("create Planet transition diagnostic directory");
    let directory = directory
        .canonicalize()
        .expect("canonicalize Planet transition diagnostic directory");
    assert!(
        !directory.starts_with(root),
        "Planet transition diagnostic output must be outside the checkout"
    );
    assert!(
        !directory.join("captures").exists()
            && !directory.join("camera-transition-trace.csv").exists()
            && !directory.join("capture-metadata.csv").exists()
            && !directory.join("events.csv").exists()
            && !directory.join("diagnostic-status.txt").exists()
            && !directory.join("diagnostic-configuration.txt").exists()
            && !directory.join("follow-probe.csv").exists(),
        "Planet transition diagnostic outputs must not be reused"
    );
    fs::create_dir_all(directory.join("captures"))
        .expect("create Planet transition diagnostic captures directory");
    fs::write(
        directory.join("camera-transition-trace.csv"),
        "elapsed_s,real_delta_ms,fixed_time_s,planet_active,planet_ready,follows,requested_radius_m,attained_radius_m,camera_x,camera_y,camera_z,camera_qx,camera_qy,camera_qz,camera_qw,player_x,player_y,player_z,player_vx,player_vy,player_vz,player_sleeping,physics_colliders,virtual_rate,virtual_paused,weather_precip,weather_kind,precipitation_active,precipitation_hidden,precipitation_visible,particle0_x,particle0_y,particle0_z\n",
    )
    .expect("write Planet transition diagnostic trace header");
    fs::write(
        directory.join("capture-metadata.csv"),
        "capture,nominal_elapsed_s,request_elapsed_s,late_by_s,camera_radius_m,camera_x,camera_y,camera_z\n",
    )
    .expect("write Planet transition capture metadata header");
    fs::write(directory.join("events.csv"), "elapsed_s,event,result\n")
        .expect("write Planet transition diagnostic event header");
    fs::write(
        directory.join("follow-probe.csv"),
        "elapsed_s,motion_elapsed_s,mode,view_active,follow,requested_radius_m,attained_radius_m,body_speed_m_s,radial_lag_rad,projected_x_logical,projected_y_logical,marker_x_physical,marker_y_physical,marker_width_physical,marker_height_physical,marker_anchor_body_separation_m,marker_anchor_registration_error_physical_px,registration_error_physical_px,marker_visible\n",
    )
    .expect("write Planet near-follow diagnostic header");
    app.insert_resource(PlanetTransitionDiagnostic {
        directory,
        started_at: None,
        world_warmup: DiagnosticWorldWarmup::default(),
        m_events_sent: [false; 6],
        m_event_times: [None; 6],
        drag_started: false,
        drag_started_at: None,
        drag_input_frames: 0,
        drag_frames: Vec::new(),
        drag_next_frame: 0,
        drag_finished: false,
        drag_finished_at: None,
        drag_failed_at: None,
        drag_start_direction: Vec3::Y,
        drag_end_direction_dot: f32::NAN,
        drag_start_camera: None,
        drag_failure_camera: None,
        drag_start_cursor: None,
        drag_end_cursor: None,
        opposite_pose_confirmed: false,
        opposite_pose_at: None,
        rendered_pose_stage: RenderedPoseStage::Waiting,
        rendered_pose_case_index: 0,
        rendered_pose_failed_cases: 0,
        rendered_pose_stage_started_at: None,
        rendered_pose_zoom_requested: false,
        rendered_pose_drag_frames: Vec::new(),
        rendered_pose_drag_next_frame: 0,
        movement_started: false,
        movement_started_at: None,
        movement_finished: false,
        movement_finished_at: None,
        follow_probe_stage: FollowProbeStage::Waiting,
        follow_probe_stage_started_at: None,
        follow_probe_mode: None,
        follow_probe_motion_started_at: None,
        follow_probe_motion_start: None,
        follow_probe_zoom_requested: false,
        follow_probe_action_sent: false,
        follow_probe_samples: Vec::with_capacity(1_000),
        storm_started: false,
        route_timed_out: false,
        requested_captures: HashSet::new(),
        samples: Vec::with_capacity(1_300),
        capture_metadata: Vec::with_capacity(DIAGNOSTIC_CAPTURE_NAMES.len()),
        entry_capture_lateness_s: [f64::NAN; 3],
        errors: Vec::new(),
        finished: false,
    })
    .add_systems(
        PreUpdate,
        drive_transition_diagnostic
            .after(InputSystems)
            .before(crate::exploration::ExplorationInput)
            .run_if(in_state(AppState::Playing)),
    )
    .add_systems(
        PostUpdate,
        capture_transition_diagnostic
            .after(CameraUpdateSystems)
            .after(bevy::ui::UiSystems::PostLayout)
            .after(bevy::transform::TransformSystems::Propagate)
            .run_if(in_state(AppState::Playing)),
    );
}

fn drive_transition_diagnostic(world: &mut World) {
    if !acceptance_world_ready(world) {
        return;
    }
    let Some(mut diagnostic) = world.remove_resource::<PlanetTransitionDiagnostic>() else {
        return;
    };
    if diagnostic.finished {
        world.insert_resource(diagnostic);
        return;
    }
    if diagnostic.started_at.is_none()
        && !diagnostic
            .world_warmup
            .observe(count_collision_colliders(world))
    {
        world.insert_resource(diagnostic);
        return;
    }
    let now = world.resource::<Time<Real>>().elapsed_secs_f64();
    if diagnostic.started_at.is_none() {
        let direction = player_pose(world)
            .map(|(position, _)| position.normalize_or(Vec3::Y))
            .unwrap_or(Vec3::Y);
        diagnostic.drag_start_direction = direction;
        write_transition_diagnostic_configuration(world, &diagnostic);
        write_diagnostic_event(
            &mut diagnostic,
            0.0,
            "fixture-start",
            "ordinary M input, continuous production drag, movement, and storm check",
        );
        diagnostic.started_at = Some(now);
    }
    let started_at = diagnostic
        .started_at
        .expect("diagnostic start time was initialized");
    let elapsed = now - started_at;

    let (planet_active, planet_ready, attained_radius) = {
        let state = world.resource::<Exploration>();
        let (_, attained_radius) = state.planet_view_camera_radii();
        (
            state.is_planet_view_active(),
            state.planet_view_ready(),
            attained_radius,
        )
    };
    let previous_sample_radius = diagnostic
        .samples
        .iter()
        .rev()
        .nth(1)
        .map(|sample| sample.attained_radius);
    let latest_sample_radius = diagnostic
        .samples
        .last()
        .map(|sample| sample.attained_radius);
    let (radius_increasing, radius_decreasing) = radius_motion_flags(
        attained_radius,
        previous_sample_radius,
        latest_sample_radius,
    );
    let weather_is_hidden = diagnostic.samples.last().is_some_and(|sample| {
        sample.weather_intensity > 0.2
            && sample.precipitation_active >= 100
            && sample.precipitation_hidden == sample.precipitation_active
    });
    let event_count = diagnostic.m_events_sent.len();
    let next_event = diagnostic.m_events_sent[..event_count]
        .iter()
        .position(|sent| !sent);
    let action_ready = next_event.is_some_and(|event_index| match event_index {
        0 => elapsed >= DIAGNOSTIC_INITIAL_OPEN_DELAY_SECONDS,
        1 => {
            let is_partial_open = planet_active
                && attained_radius > DIAGNOSTIC_PARTIAL_VIEW_MIN_RADIUS_M
                && attained_radius < DIAGNOSTIC_EXTERIOR_READY_RADIUS_M
                && radius_increasing;
            transition_action_due(
                elapsed,
                diagnostic.m_event_times[0],
                DIAGNOSTIC_REVERSAL_MINIMUM_GAP_SECONDS,
                is_partial_open,
            )
        }
        2 => {
            let is_partial_return = planet_active
                && attained_radius > DIAGNOSTIC_PARTIAL_VIEW_MIN_RADIUS_M
                && attained_radius < DIAGNOSTIC_EXTERIOR_READY_RADIUS_M
                && radius_decreasing;
            transition_action_due(
                elapsed,
                diagnostic.m_event_times[1],
                DIAGNOSTIC_REVERSAL_MINIMUM_GAP_SECONDS,
                is_partial_return,
            )
        }
        3 => transition_action_due(
            elapsed,
            diagnostic.movement_finished_at,
            DIAGNOSTIC_FINAL_RETURN_GAP_SECONDS,
            planet_active
                && diagnostic.movement_finished
                && drag_probe_ready(&diagnostic)
                && diagnostic.follow_probe_stage == FollowProbeStage::Complete,
        ),
        4 => transition_action_due(
            elapsed,
            diagnostic.m_event_times[3],
            DIAGNOSTIC_STORM_REOPEN_GAP_SECONDS,
            !planet_active && return_captures_saved(&diagnostic),
        ),
        5 => transition_action_due(
            elapsed,
            diagnostic.m_event_times[4],
            DIAGNOSTIC_STORM_CLOSE_GAP_SECONDS,
            planet_active && weather_is_hidden,
        ),
        _ => false,
    });
    let tapped_map = action_ready;
    if let Some(event_index) = next_event.filter(|_| action_ready) {
        diagnostic.m_events_sent[event_index] = true;
        diagnostic.m_event_times[event_index] = Some(elapsed);
        write_diagnostic_event(
            &mut diagnostic,
            elapsed,
            "production-key",
            match event_index {
                0 => "pressed M to open Planet view",
                1 => "pressed M to reverse opening into return after a partial pullback",
                2 => "pressed M to reverse return into opening after a partial return",
                3 => "pressed M to return after the moving nearest-radius follow scenes",
                4 => "pressed M to reopen after the previous return completed",
                _ => "pressed M to return after precipitation was hidden",
            },
        );
    }
    set_key(world, KeyCode::KeyM, tapped_map);

    set_key(world, KeyCode::Escape, false);
    set_key(world, KeyCode::KeyV, false);
    set_key(world, KeyCode::KeyT, false);
    if !diagnostic.drag_started
        && transition_action_due(
            elapsed,
            diagnostic.m_event_times[2],
            DIAGNOSTIC_DRAG_READY_GAP_SECONDS,
            planet_active && planet_ready && attained_radius >= DIAGNOSTIC_EXTERIOR_READY_RADIUS_M,
        )
    {
        diagnostic.drag_started = true;
        diagnostic.drag_started_at = Some(elapsed);
        diagnostic.drag_start_camera = main_camera_pose(world);
        write_diagnostic_event(
            &mut diagnostic,
            elapsed,
            "production-pointer",
            "began released multi-stroke left-button drag after the exterior camera reached its full radius",
        );
    }
    if diagnostic.drag_started && !diagnostic.drag_finished {
        if diagnostic.drag_frames.is_empty() {
            if let Some(frames) = diagnostic_cursor_path(world, diagnostic.drag_start_direction) {
                diagnostic.drag_start_cursor = frames.first().map(|frame| frame.position);
                diagnostic.drag_end_cursor = frames.last().map(|frame| frame.position);
                diagnostic.drag_frames = frames;
            } else {
                diagnostic.errors.push("could not construct the released multi-stroke drag route inside the logical viewport".into());
                diagnostic.drag_finished = true;
                diagnostic.drag_finished_at = Some(elapsed);
            }
        }
        if let Some(frame) = diagnostic
            .drag_frames
            .get(diagnostic.drag_next_frame)
            .copied()
        {
            set_primary_cursor(world, frame.position);
            set_mouse_button(world, MouseButton::Left, frame.pressed);
            diagnostic.drag_next_frame += 1;
            if frame.pressed {
                diagnostic.drag_input_frames = diagnostic.drag_input_frames.saturating_add(1);
            }
        } else if diagnostic.drag_frames.is_empty() {
            set_mouse_button(world, MouseButton::Left, false);
        } else {
            set_mouse_button(world, MouseButton::Left, false);
            diagnostic.drag_finished = true;
            diagnostic.drag_finished_at = Some(elapsed);
            let drag_input_frames = diagnostic.drag_input_frames;
            let planned_frames = diagnostic.drag_frames.len();
            write_diagnostic_event(
                &mut diagnostic,
                elapsed,
                "production-pointer",
                format!(
                    "released after replaying all {planned_frames} planned input frames ({drag_input_frames} pressed); waiting for measured opposite pose"
                ),
            );
        }
    } else if !diagnostic.drag_started {
        set_mouse_button(world, MouseButton::Left, false);
    }

    drive_rendered_pose_probe(world, &mut diagnostic, elapsed, planet_active);

    if !diagnostic.movement_started
        && transition_action_due(
            elapsed,
            drag_progress_at(&diagnostic),
            DIAGNOSTIC_DRAG_READY_GAP_SECONDS,
            planet_active
                && drag_probe_ready(&diagnostic)
                && matches!(diagnostic.rendered_pose_stage, RenderedPoseStage::Complete),
        )
    {
        diagnostic.movement_started = true;
        diagnostic.movement_started_at = Some(elapsed);
        let movement_message = if diagnostic.opposite_pose_confirmed {
            "began ordinary on-foot W with short steering input while Planet view is active at the opposite pose"
        } else {
            "began ordinary on-foot W to continue independent diagnostics after the antipode drag failed"
        };
        write_diagnostic_event(&mut diagnostic, elapsed, "physics-probe", movement_message);
    }
    if let Some(movement_started_at) = diagnostic.movement_started_at {
        let movement_elapsed = elapsed - movement_started_at;
        if movement_elapsed < DIAGNOSTIC_MOVEMENT_DURATION_SECONDS {
            drive_diagnostic_movement(world, movement_elapsed);
        } else {
            set_key(world, KeyCode::KeyW, false);
            set_key(world, KeyCode::KeyA, false);
            set_key(world, KeyCode::KeyD, false);
            if !diagnostic.movement_finished {
                diagnostic.movement_finished = true;
                diagnostic.movement_finished_at = Some(elapsed);
                write_diagnostic_event(
                    &mut diagnostic,
                    elapsed,
                    "physics-probe",
                    "released movement keys after the measured continuous route",
                );
            }
        }
    } else {
        set_key(world, KeyCode::KeyW, false);
        set_key(world, KeyCode::KeyA, false);
        set_key(world, KeyCode::KeyD, false);
    }

    drive_near_follow_probe(world, &mut diagnostic, elapsed);
    if diagnostic.follow_probe_stage == FollowProbeStage::Failed {
        diagnostic.route_timed_out = true;
        finish_transition_diagnostic(&mut diagnostic);
        world.insert_resource(diagnostic);
        world.write_message(AppExit::Success);
        return;
    }

    if diagnostic.m_event_times[4].is_some() && planet_active {
        let mut weather = world.resource_mut::<Weather>();
        // Hold an authored storm intensity while leaving production particle
        // movement, local rain/snow choice, and visibility systems in charge.
        weather.precip = 0.85;
        weather.wind = Vec3::new(1.5, 0.0, -0.75);
        if !diagnostic.storm_started {
            diagnostic.storm_started = true;
            write_diagnostic_event(
                &mut diagnostic,
                elapsed,
                "weather-probe",
                "held production precipitation intensity at 0.85 with moving wind",
            );
        }
    }

    let final_event = 5;
    let finished_after_return = diagnostic.m_event_times[final_event]
        .is_some_and(|at| !planet_active && elapsed >= at + 0.8 + DIAGNOSTIC_RETURN_SETTLE_SECONDS);
    let timed_out = elapsed >= DIAGNOSTIC_ROUTE_TIMEOUT_SECONDS;

    if finished_after_return || timed_out {
        diagnostic.route_timed_out = timed_out && !finished_after_return;
        if diagnostic.route_timed_out {
            write_diagnostic_event(
                &mut diagnostic,
                elapsed,
                "route-timeout",
                "required stage readiness did not arrive before the diagnostic deadline",
            );
        }
        finish_transition_diagnostic(&mut diagnostic);
        world.insert_resource(diagnostic);
        world.write_message(AppExit::Success);
        return;
    }
    world.insert_resource(diagnostic);
}

fn write_transition_diagnostic_configuration(
    world: &mut World,
    diagnostic: &PlanetTransitionDiagnostic,
) {
    let window = primary_window(world).map(|window| {
        format!(
            "physical={}x{} logical={}x{} scale_factor={} present_mode={:?} visible={} focused={}",
            window.physical_width(),
            window.physical_height(),
            window.width(),
            window.height(),
            window.scale_factor(),
            window.present_mode,
            window.visible,
            window.focused,
        )
    });
    let drag = window
        .as_deref()
        .and_then(|_| diagnostic_cursor_path(world, diagnostic.drag_start_direction))
        .map(|frames| {
            let start = frames.first().expect("generated drag frame").position;
            let end = frames.last().expect("generated drag frame").position;
            let strokes = frames
                .windows(2)
                .filter(|pair| pair[0].pressed && !pair[1].pressed)
                .count();
            format!(
                "start=({:.2},{:.2}) end=({:.2},{:.2}) frames={} strokes={} net_cursor=({:.2},{:.2}) logical_pixels",
                start.x,
                start.y,
                end.x,
                end.y,
                frames.len(),
                strokes,
                end.x - start.x,
                end.y - start.y,
            )
        })
        .unwrap_or_else(|| "unavailable".into());
    let mode = "visual-transition-diagnostic";
    let capture_target = super::background_capture_description(world);
    let map_schedule = format!(
        "open after warmup+{DIAGNOSTIC_INITIAL_OPEN_DELAY_SECONDS:.2}s; reverse after each prior tap+{DIAGNOSTIC_REVERSAL_MINIMUM_GAP_SECONDS:.2}s while transition is partial; detached on-foot movement for {DIAGNOSTIC_MOVEMENT_DURATION_SECONDS:.2}s; rendered pointer-drag poses at far, middle, and near-polar headings; nearest-radius active-follow captures for on-foot, car, and plane; final return after these scenes; storm reopen after return+{DIAGNOSTIC_STORM_REOPEN_GAP_SECONDS:.2}s; close after {DIAGNOSTIC_STORM_CLOSE_GAP_SECONDS:.2}s hidden"
    );
    let entry_captures = "0.40,0.80,1.40 after initial M";
    let return_captures = "1.20,2.60 after final return M";
    let weather_fixture =
        "precipitation intensity 0.85 with moving wind; production particle visibility";
    let weather_captures = "storm-hidden.png,storm-restored.png";
    let text = format!(
        "mode={}\nacceptance_claim=none\nsource_revision={}\nsource_branch={}\nasset_root={}\ncargo_target_dir={}\npackage_version={}\nwindow={}\ncapture_target={}\nbackground_capture_is_performance_comparable=false\nprewarm_minimum_physics_colliders={DIAGNOSTIC_MINIMUM_COLLIDERS}\nprewarm_stable_world_frames={DIAGNOSTIC_STABLE_WORLD_FRAMES}\nM_action_schedule={}\nentry_screenshots={}\nreturn_screenshots={}\nreleased_multi_stroke_drag=all planned cursor/button frames replayed once at the exterior-ready radius {DIAGNOSTIC_EXTERIOR_READY_RADIUS_M:.0}m\ndrag_path={drag}\non_foot_movement={DIAGNOSTIC_MOVEMENT_DURATION_SECONDS:.2}s after confirmed opposite pose\nnear_follow_probe=on-foot,car,plane; nearest requested radius {PLANET_VIEW_NEAR_RADIUS:.1}m; two captures 250ms apart after attained radius is within {FOLLOW_PROBE_RADIUS_TOLERANCE_M:.1}m\nfollow_steady_acceptance=max radial lag {FOLLOW_PROBE_MAX_RADIAL_LAG_RAD:.5}rad; max overlay registration {FOLLOW_PROBE_MAX_MARKER_REGISTRATION_ERROR_PHYSICAL_PX:.1} physical px\nroute_timeout_s={DIAGNOSTIC_ROUTE_TIMEOUT_SECONDS:.0}\nweather_fixture={}\nweather_screenshots={}\ncapture_metadata=target_vs_actual_request_time_and_camera_pose\n",
        mode,
        std::env::var("TERRA_SOURCE_REVISION").unwrap_or_else(|_| "unset".into()),
        std::env::var("TERRA_SOURCE_BRANCH").unwrap_or_else(|_| "unset".into()),
        std::env::var("BEVY_ASSET_ROOT").unwrap_or_else(|_| "default-asset-root".into()),
        std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "cargo-default".into()),
        env!("CARGO_PKG_VERSION"),
        window.unwrap_or_else(|| "unavailable".into()),
        capture_target,
        map_schedule,
        entry_captures,
        return_captures,
        weather_fixture,
        weather_captures,
    );
    if let Err(error) = fs::write(
        diagnostic.directory.join("diagnostic-configuration.txt"),
        text,
    ) {
        error!("Planet transition diagnostic configuration: {error}");
    }
}

fn diagnostic_cursor_path(
    world: &mut World,
    start_direction: Vec3,
) -> Option<Vec<super::diagnostic_drag::DragInputFrame>> {
    let window = primary_window(world)?;
    super::diagnostic_drag::opposite_side_drag(
        start_direction,
        window.width(),
        window.height(),
        super::diagnostic_drag::DEFAULT_POINTS_PER_STROKE,
    )
}

fn set_primary_cursor(world: &mut World, position: Vec2) {
    let mut query = world.query_filtered::<&mut Window, With<PrimaryWindow>>();
    if let Ok(mut window) = query.single_mut(world) {
        window.set_cursor_position(Some(position));
    }
}

fn set_mouse_button(world: &mut World, button: MouseButton, pressed: bool) {
    if let Some(mut mouse) = world.get_resource_mut::<ButtonInput<MouseButton>>() {
        if pressed {
            mouse.press(button);
        } else {
            mouse.release(button);
        }
    }
}

fn drive_rendered_pose_probe(
    world: &mut World,
    diagnostic: &mut PlanetTransitionDiagnostic,
    elapsed: f64,
    planet_active: bool,
) {
    if diagnostic.rendered_pose_stage == RenderedPoseStage::Waiting {
        if !drag_probe_ready(diagnostic) {
            return;
        }
        diagnostic.rendered_pose_stage = RenderedPoseStage::Zooming;
        diagnostic.rendered_pose_stage_started_at = Some(elapsed);
        write_diagnostic_event(
            diagnostic,
            elapsed,
            "rendered-pose-probe",
            "began far-oblique, mid-heading, and near-polar pointer-drags",
        );
    }
    if matches!(diagnostic.rendered_pose_stage, RenderedPoseStage::Complete) {
        set_mouse_button(world, MouseButton::Left, false);
        return;
    }

    let Some(case) = RENDERED_POSE_CASES
        .get(diagnostic.rendered_pose_case_index)
        .copied()
    else {
        diagnostic.rendered_pose_stage = RenderedPoseStage::Complete;
        diagnostic.rendered_pose_stage_started_at = Some(elapsed);
        write_diagnostic_event(
            diagnostic,
            elapsed,
            "rendered-pose-probe",
            "all rendered heading and zoom cases completed",
        );
        set_mouse_button(world, MouseButton::Left, false);
        return;
    };
    let stage_started_at = diagnostic.rendered_pose_stage_started_at.unwrap_or(elapsed);
    if elapsed - stage_started_at > RENDERED_POSE_CASE_TIMEOUT_SECONDS {
        let (requested, attained) = world.resource::<Exploration>().planet_view_camera_radii();
        let direction_error = main_camera_pose(world).map_or(f32::NAN, |pose| {
            pose.translation
                .normalize_or(Vec3::Y)
                .angle_between(case.target_direction.normalize_or(Vec3::Y))
        });
        fail_rendered_pose_probe(
            diagnostic,
            elapsed,
            case,
            &format!(
                "timed out after {:.0}s (requested_radius_m={requested:.2}, attained_radius_m={attained:.2}, target_direction_error_rad={direction_error:.5})",
                RENDERED_POSE_CASE_TIMEOUT_SECONDS
            ),
        );
        set_mouse_button(world, MouseButton::Left, false);
        return;
    }

    match diagnostic.rendered_pose_stage {
        RenderedPoseStage::Waiting => unreachable!("waiting handled above"),
        RenderedPoseStage::Zooming => {
            set_mouse_button(world, MouseButton::Left, false);
            if !diagnostic.rendered_pose_zoom_requested {
                let (requested, _) = world.resource::<Exploration>().planet_view_camera_radii();
                let wheel_delta = (requested / case.target_radius).ln();
                if wheel_delta.abs() > 0.001
                    && !world
                        .resource_mut::<Exploration>()
                        .request_planet_view_zoom(wheel_delta)
                {
                    fail_rendered_pose_probe(
                        diagnostic,
                        elapsed,
                        case,
                        "could not request camera zoom",
                    );
                    return;
                }
                diagnostic.rendered_pose_zoom_requested = true;
                write_diagnostic_event(
                    diagnostic,
                    elapsed,
                    "rendered-pose-probe",
                    format!(
                        "{} requested radius {:.1}m for rendered surface drag",
                        case.name, case.target_radius
                    ),
                );
            }
            let (requested, attained) = world.resource::<Exploration>().planet_view_camera_radii();
            if planet_active
                && (requested - case.target_radius).abs() <= FOLLOW_PROBE_RADIUS_TOLERANCE_M
                && (attained - case.target_radius).abs() <= FOLLOW_PROBE_RADIUS_TOLERANCE_M
            {
                if capture_once(world, diagnostic, case.before_capture, elapsed, |_| true) {
                    let camera_pose = main_camera_pose(world).unwrap_or_default();
                    let direction_error = camera_pose
                        .translation
                        .normalize_or(Vec3::Y)
                        .angle_between(case.target_direction.normalize_or(Vec3::Y));
                    write_diagnostic_event(
                        diagnostic,
                        elapsed,
                        "rendered-pose-capture",
                        format!(
                            "{} before drag; requested_radius_m={requested:.2}; attained_radius_m={attained:.2}; target_direction_error_rad={direction_error:.5}",
                            case.name
                        ),
                    );
                    diagnostic.rendered_pose_stage = RenderedPoseStage::WaitingForBeforeCapture;
                    diagnostic.rendered_pose_stage_started_at = Some(elapsed);
                }
            }
        }
        RenderedPoseStage::WaitingForBeforeCapture => {
            set_mouse_button(world, MouseButton::Left, false);
            if file_is_nonempty(
                &diagnostic
                    .directory
                    .join("captures")
                    .join(case.before_capture),
            ) {
                let Some(window) = primary_window(world) else {
                    fail_rendered_pose_probe(
                        diagnostic,
                        elapsed,
                        case,
                        "primary window missing before pointer drag",
                    );
                    return;
                };
                let Some(camera_pose) = main_camera_pose(world) else {
                    fail_rendered_pose_probe(
                        diagnostic,
                        elapsed,
                        case,
                        "main camera missing before pointer drag",
                    );
                    return;
                };
                let Some(vertical_fov) = main_camera_vertical_fov(world) else {
                    fail_rendered_pose_probe(
                        diagnostic,
                        elapsed,
                        case,
                        "perspective projection missing before pointer drag",
                    );
                    return;
                };
                let start_cursor =
                    Vec2::new(window.width(), window.height()) * 0.5 + case.start_cursor_offset;
                let Some(frames) = super::diagnostic_drag::surface_drag_toward_direction(
                    camera_pose,
                    case.target_direction,
                    start_cursor,
                    window.width(),
                    window.height(),
                    vertical_fov,
                ) else {
                    fail_rendered_pose_probe(
                        diagnostic,
                        elapsed,
                        case,
                        "could not construct a surface-grab route inside the logical viewport",
                    );
                    return;
                };
                let frame_count = frames.len();
                diagnostic.rendered_pose_drag_frames = frames;
                diagnostic.rendered_pose_drag_next_frame = 0;
                diagnostic.rendered_pose_stage = RenderedPoseStage::Dragging;
                diagnostic.rendered_pose_stage_started_at = Some(elapsed);
                write_diagnostic_event(
                    diagnostic,
                    elapsed,
                    "rendered-pose-probe",
                    format!(
                        "{} began production left-button surface drag with {frame_count} cursor frames",
                        case.name
                    ),
                );
            }
        }
        RenderedPoseStage::Dragging => {
            if let Some(frame) = diagnostic
                .rendered_pose_drag_frames
                .get(diagnostic.rendered_pose_drag_next_frame)
                .copied()
            {
                set_primary_cursor(world, frame.position);
                set_mouse_button(world, MouseButton::Left, frame.pressed);
                diagnostic.rendered_pose_drag_next_frame += 1;
            } else {
                set_mouse_button(world, MouseButton::Left, false);
                diagnostic.rendered_pose_stage = RenderedPoseStage::WaitingForAfterCapture;
                diagnostic.rendered_pose_stage_started_at = Some(elapsed);
            }
        }
        RenderedPoseStage::WaitingForAfterCapture => {
            set_mouse_button(world, MouseButton::Left, false);
            let (requested, attained) = world.resource::<Exploration>().planet_view_camera_radii();
            let camera_pose = main_camera_pose(world).unwrap_or_default();
            let direction_error = camera_pose
                .translation
                .normalize_or(Vec3::Y)
                .angle_between(case.target_direction.normalize_or(Vec3::Y));
            if planet_active
                && direction_error <= 0.025
                && (requested - case.target_radius).abs() <= FOLLOW_PROBE_RADIUS_TOLERANCE_M
                && (attained - case.target_radius).abs() <= FOLLOW_PROBE_RADIUS_TOLERANCE_M
                && elapsed - stage_started_at >= 0.05
                && capture_once(world, diagnostic, case.after_capture, elapsed, |_| true)
            {
                write_diagnostic_event(
                    diagnostic,
                    elapsed,
                    "rendered-pose-capture",
                    format!(
                        "{} after drag; requested_radius_m={requested:.2}; attained_radius_m={attained:.2}; target_direction_error_rad={direction_error:.5}",
                        case.name
                    ),
                );
                diagnostic.rendered_pose_stage = RenderedPoseStage::WaitingForAfterCaptureSave;
                diagnostic.rendered_pose_stage_started_at = Some(elapsed);
            }
        }
        RenderedPoseStage::WaitingForAfterCaptureSave => {
            set_mouse_button(world, MouseButton::Left, false);
            if file_is_nonempty(
                &diagnostic
                    .directory
                    .join("captures")
                    .join(case.after_capture),
            ) {
                diagnostic.rendered_pose_case_index += 1;
                diagnostic.rendered_pose_zoom_requested = false;
                diagnostic.rendered_pose_drag_frames.clear();
                diagnostic.rendered_pose_drag_next_frame = 0;
                diagnostic.rendered_pose_stage =
                    if diagnostic.rendered_pose_case_index == RENDERED_POSE_CASES.len() {
                        RenderedPoseStage::Complete
                    } else {
                        RenderedPoseStage::Zooming
                    };
                diagnostic.rendered_pose_stage_started_at = Some(elapsed);
                write_diagnostic_event(
                    diagnostic,
                    elapsed,
                    "rendered-pose-probe",
                    format!("{} surface drag capture pair saved", case.name),
                );
            }
        }
        RenderedPoseStage::Complete => {
            set_mouse_button(world, MouseButton::Left, false);
        }
    }
}

fn main_camera_vertical_fov(world: &mut World) -> Option<f32> {
    let mut projections = world.query_filtered::<&Projection, With<MainCamera>>();
    projections
        .iter(world)
        .find_map(|projection| match projection {
            Projection::Perspective(perspective) => Some(perspective.fov),
            Projection::Orthographic(_) | Projection::Custom(_) => None,
        })
}

fn fail_rendered_pose_probe(
    diagnostic: &mut PlanetTransitionDiagnostic,
    elapsed: f64,
    case: RenderedPoseCase,
    reason: &str,
) {
    let reason = format!("{}: {reason}", case.name);
    diagnostic.rendered_pose_failed_cases += 1;
    diagnostic.rendered_pose_case_index += 1;
    diagnostic.rendered_pose_zoom_requested = false;
    diagnostic.rendered_pose_drag_frames.clear();
    diagnostic.rendered_pose_drag_next_frame = 0;
    diagnostic.rendered_pose_stage =
        if diagnostic.rendered_pose_case_index == RENDERED_POSE_CASES.len() {
            RenderedPoseStage::Complete
        } else {
            RenderedPoseStage::Zooming
        };
    diagnostic.rendered_pose_stage_started_at = Some(elapsed);
    diagnostic.errors.push(reason.clone());
    write_diagnostic_event(diagnostic, elapsed, "rendered-pose-probe-failed", reason);
}

fn drive_diagnostic_movement(world: &mut World, elapsed: f64) {
    if elapsed < 0.1 {
        for key in [
            KeyCode::KeyW,
            KeyCode::KeyS,
            KeyCode::KeyA,
            KeyCode::KeyD,
            KeyCode::Space,
            KeyCode::ShiftLeft,
            KeyCode::ShiftRight,
            KeyCode::ControlLeft,
            KeyCode::ControlRight,
        ] {
            set_key(world, key, false);
        }
        return;
    }
    let cycle = elapsed.rem_euclid(2.5);
    set_key(world, KeyCode::KeyW, true);
    set_key(world, KeyCode::KeyA, cycle < 0.35);
    set_key(world, KeyCode::KeyD, (1.25..1.60).contains(&cycle));
}

fn enter_follow_probe_stage(
    diagnostic: &mut PlanetTransitionDiagnostic,
    stage: FollowProbeStage,
    mode: Option<FollowProbeMode>,
    elapsed: f64,
) {
    diagnostic.follow_probe_stage = stage;
    diagnostic.follow_probe_stage_started_at = Some(elapsed);
    diagnostic.follow_probe_mode = mode;
    diagnostic.follow_probe_motion_started_at = None;
    diagnostic.follow_probe_motion_start = None;
    diagnostic.follow_probe_zoom_requested = false;
    diagnostic.follow_probe_action_sent = false;
}

fn drive_near_follow_probe(
    world: &mut World,
    diagnostic: &mut PlanetTransitionDiagnostic,
    elapsed: f64,
) {
    if diagnostic.follow_probe_stage == FollowProbeStage::Waiting && diagnostic.movement_finished {
        enter_follow_probe_stage(
            diagnostic,
            FollowProbeStage::OnFoot,
            Some(FollowProbeMode::OnFoot),
            elapsed,
        );
        write_diagnostic_event(
            diagnostic,
            elapsed,
            "near-follow-probe",
            "began the moving on-foot closest-zoom capture",
        );
    }

    let stage_started_at = diagnostic.follow_probe_stage_started_at.unwrap_or(elapsed);
    match diagnostic.follow_probe_stage {
        FollowProbeStage::Waiting => {}
        FollowProbeStage::OnFoot | FollowProbeStage::Vehicle => {
            let mode = diagnostic
                .follow_probe_mode
                .expect("moving follow-probe stages have a mode");
            if drive_follow_probe_view(world, diagnostic, mode, elapsed) {
                stop_movement_keys(world);
                if mode == FollowProbeMode::Plane {
                    enter_follow_probe_stage(diagnostic, FollowProbeStage::Complete, None, elapsed);
                    write_diagnostic_event(
                        diagnostic,
                        elapsed,
                        "near-follow-probe",
                        "Plane capture saved; follow probes complete",
                    );
                } else {
                    world
                        .resource_mut::<Exploration>()
                        .set_planet_view_open(false);
                    enter_follow_probe_stage(
                        diagnostic,
                        FollowProbeStage::CloseView,
                        Some(mode),
                        elapsed,
                    );
                    write_diagnostic_event(
                        diagnostic,
                        elapsed,
                        "near-follow-probe",
                        format!("{} capture saved; closing Planet view", mode.name()),
                    );
                }
            }
        }
        FollowProbeStage::CloseView => {
            stop_movement_keys(world);
            world
                .resource_mut::<Exploration>()
                .set_planet_view_open(false);
            if !world.resource::<Exploration>().is_planet_view_active() {
                match diagnostic.follow_probe_mode {
                    Some(FollowProbeMode::OnFoot) => enter_follow_probe_stage(
                        diagnostic,
                        FollowProbeStage::SummonVehicle,
                        Some(FollowProbeMode::Car),
                        elapsed,
                    ),
                    Some(FollowProbeMode::Car) => enter_follow_probe_stage(
                        diagnostic,
                        FollowProbeStage::ExitVehicle,
                        Some(FollowProbeMode::Car),
                        elapsed,
                    ),
                    _ => enter_follow_probe_stage(
                        diagnostic,
                        FollowProbeStage::Failed,
                        None,
                        elapsed,
                    ),
                }
            }
        }
        FollowProbeStage::SummonVehicle => {
            stop_movement_keys(world);
            let mode = diagnostic
                .follow_probe_mode
                .expect("vehicle summon stage has a mode");
            let kind = mode
                .vehicle_kind()
                .expect("vehicle summon stage cannot use the on-foot mode");
            if !diagnostic.follow_probe_action_sent {
                world
                    .resource_mut::<Exploration>()
                    .request(Action::Summon(kind));
                diagnostic.follow_probe_action_sent = true;
                write_diagnostic_event(
                    diagnostic,
                    elapsed,
                    "near-follow-probe",
                    format!("requested production {mode:?} summon"),
                );
            }
            let state = world.resource::<Exploration>();
            if state.vehicle_summon_ready(kind) && state.vehicle_entity(kind).is_some() {
                enter_follow_probe_stage(
                    diagnostic,
                    FollowProbeStage::ApproachVehicle,
                    Some(mode),
                    elapsed,
                );
                write_diagnostic_event(
                    diagnostic,
                    elapsed,
                    "near-follow-probe",
                    format!("{mode:?} summon completed; approaching the vehicle"),
                );
            } else if elapsed - stage_started_at > 45.0 {
                enter_follow_probe_stage(diagnostic, FollowProbeStage::Failed, None, elapsed);
                write_diagnostic_event(
                    diagnostic,
                    elapsed,
                    "near-follow-probe-failed",
                    format!("production {mode:?} summon did not complete"),
                );
            }
        }
        FollowProbeStage::ApproachVehicle => {
            let mode = diagnostic
                .follow_probe_mode
                .expect("vehicle approach stage has a mode");
            let kind = mode
                .vehicle_kind()
                .expect("vehicle approach stage cannot use the on-foot mode");
            if approach_vehicle(world, kind) {
                stop_movement_keys(world);
                enter_follow_probe_stage(
                    diagnostic,
                    FollowProbeStage::EnterVehicle,
                    Some(mode),
                    elapsed,
                );
                write_diagnostic_event(
                    diagnostic,
                    elapsed,
                    "near-follow-probe",
                    format!("approached {mode:?}; waiting for stable interaction range"),
                );
            } else if elapsed - stage_started_at > 15.0 {
                enter_follow_probe_stage(diagnostic, FollowProbeStage::Failed, None, elapsed);
                write_diagnostic_event(
                    diagnostic,
                    elapsed,
                    "near-follow-probe-failed",
                    format!("could not approach {mode:?} to interaction range"),
                );
            }
        }
        FollowProbeStage::EnterVehicle => {
            let mode = diagnostic
                .follow_probe_mode
                .expect("vehicle entry stage has a mode");
            if world.resource::<Exploration>().is_in_vehicle() {
                enter_follow_probe_stage(
                    diagnostic,
                    FollowProbeStage::Vehicle,
                    Some(mode),
                    elapsed,
                );
                write_diagnostic_event(
                    diagnostic,
                    elapsed,
                    "near-follow-probe",
                    format!("entered {mode:?} for closest-zoom movement capture"),
                );
            } else if controlled_body_motion(world)
                .is_some_and(|(_, velocity)| velocity.length() < 0.5)
                && elapsed - stage_started_at >= 0.5
                && !diagnostic.follow_probe_action_sent
            {
                world
                    .resource_mut::<Exploration>()
                    .request(Action::Interact);
                diagnostic.follow_probe_action_sent = true;
            } else {
                if diagnostic.follow_probe_action_sent && elapsed - stage_started_at >= 1.0 {
                    diagnostic.follow_probe_action_sent = false;
                }
                if elapsed - stage_started_at > 10.0 {
                    enter_follow_probe_stage(diagnostic, FollowProbeStage::Failed, None, elapsed);
                    write_diagnostic_event(
                        diagnostic,
                        elapsed,
                        "near-follow-probe-failed",
                        format!("could not enter {mode:?} from stable interaction range"),
                    );
                }
            }
        }
        FollowProbeStage::ExitVehicle => {
            let mode = diagnostic
                .follow_probe_mode
                .expect("vehicle exit stage has a mode");
            if !world.resource::<Exploration>().is_in_vehicle() {
                enter_follow_probe_stage(
                    diagnostic,
                    FollowProbeStage::SummonVehicle,
                    Some(FollowProbeMode::Plane),
                    elapsed,
                );
                write_diagnostic_event(
                    diagnostic,
                    elapsed,
                    "near-follow-probe",
                    "exited Car before requesting the production Plane summon",
                );
            } else if controlled_body_motion(world)
                .is_some_and(|(_, velocity)| velocity.length() < 0.5)
                && elapsed - stage_started_at >= 0.8
                && !diagnostic.follow_probe_action_sent
            {
                world
                    .resource_mut::<Exploration>()
                    .request(Action::Interact);
                diagnostic.follow_probe_action_sent = true;
            } else {
                if diagnostic.follow_probe_action_sent && elapsed - stage_started_at >= 1.3 {
                    diagnostic.follow_probe_action_sent = false;
                }
                if elapsed - stage_started_at > 15.0 {
                    enter_follow_probe_stage(diagnostic, FollowProbeStage::Failed, None, elapsed);
                    write_diagnostic_event(
                        diagnostic,
                        elapsed,
                        "near-follow-probe-failed",
                        format!("could not exit {mode:?} on stable ground"),
                    );
                }
            }
        }
        FollowProbeStage::Complete | FollowProbeStage::Failed => {
            stop_movement_keys(world);
        }
    }
}

fn drive_follow_probe_view(
    world: &mut World,
    diagnostic: &mut PlanetTransitionDiagnostic,
    mode: FollowProbeMode,
    elapsed: f64,
) -> bool {
    let (active, follows, ready, requested, attained) = {
        let state = world.resource::<Exploration>();
        let (requested, attained) = state.planet_view_camera_radii();
        (
            state.is_planet_view_active(),
            state.planet_view_follows_body(),
            state.planet_view_ready(),
            requested,
            attained,
        )
    };
    if !active {
        world
            .resource_mut::<Exploration>()
            .set_planet_view_open(true);
    } else if !follows && let Some(camera) = main_camera_pose(world) {
        world
            .resource_mut::<Exploration>()
            .toggle_planet_view_follow(camera);
    }
    if active && !diagnostic.follow_probe_zoom_requested {
        world
            .resource_mut::<Exploration>()
            .request_planet_view_zoom(10.0);
        diagnostic.follow_probe_zoom_requested = true;
        write_diagnostic_event(
            diagnostic,
            elapsed,
            "near-follow-probe",
            format!("requested closest zoom while following {}", mode.name()),
        );
    }

    let settled_radial_lag = controlled_body_motion(world)
        .zip(main_camera_pose(world))
        .map(|((body_position, _), camera_pose)| {
            radial_angle(camera_pose.translation, body_position)
        })
        .unwrap_or(f32::INFINITY);
    let target_reached = ready
        && follows
        && requested <= PLANET_VIEW_NEAR_RADIUS + FOLLOW_PROBE_RADIUS_TOLERANCE_M
        && (attained - PLANET_VIEW_NEAR_RADIUS).abs() <= FOLLOW_PROBE_RADIUS_TOLERANCE_M
        && settled_radial_lag <= FOLLOW_PROBE_MAX_RADIAL_LAG_RAD;
    if target_reached {
        if diagnostic.follow_probe_motion_started_at.is_none() {
            diagnostic.follow_probe_motion_started_at = Some(elapsed);
            diagnostic.follow_probe_motion_start = controlled_body_motion(world).map(|(p, _)| p);
            write_diagnostic_event(
                diagnostic,
                elapsed,
                "near-follow-probe",
                format!(
                    "started moving {} at attained radius {attained:.1}m",
                    mode.name()
                ),
            );
        }
        let motion_elapsed = elapsed - diagnostic.follow_probe_motion_started_at.unwrap_or(elapsed);
        drive_follow_probe_movement(world, mode, motion_elapsed);
        if motion_elapsed >= FOLLOW_PROBE_MOVE_SECONDS
            && mode
                .capture_names()
                .into_iter()
                .all(|name| file_is_nonempty(&diagnostic.directory.join("captures").join(name)))
        {
            return true;
        }
    } else {
        set_key(world, KeyCode::KeyW, false);
        set_key(world, KeyCode::KeyA, false);
        set_key(world, KeyCode::KeyD, false);
        let stage_started_at = diagnostic.follow_probe_stage_started_at.unwrap_or(elapsed);
        if elapsed - stage_started_at > 25.0 {
            enter_follow_probe_stage(diagnostic, FollowProbeStage::Failed, None, elapsed);
            write_diagnostic_event(
                diagnostic,
                elapsed,
                "near-follow-probe-failed",
                format!(
                    "closest follow radius was not attained (requested={requested:.1}m attained={attained:.1}m)"
                ),
            );
        }
    }
    false
}

fn drive_follow_probe_movement(world: &mut World, mode: FollowProbeMode, elapsed: f64) {
    if mode == FollowProbeMode::OnFoot {
        drive_diagnostic_movement(world, elapsed);
    } else if mode == FollowProbeMode::Plane {
        set_key(world, KeyCode::KeyW, false);
        set_key(world, KeyCode::KeyS, false);
        set_key(world, KeyCode::KeyA, false);
        set_key(world, KeyCode::KeyD, false);
        set_key(world, KeyCode::ShiftLeft, true);
        set_key(world, KeyCode::ShiftRight, false);
        set_key(world, KeyCode::ControlLeft, false);
        set_key(world, KeyCode::ControlRight, false);
    } else {
        let cycle = elapsed.rem_euclid(1.6);
        set_key(world, KeyCode::KeyW, true);
        set_key(world, KeyCode::KeyS, false);
        set_key(world, KeyCode::KeyA, cycle < 0.18);
        set_key(world, KeyCode::KeyD, (0.8..0.98).contains(&cycle));
    }
}

fn stop_movement_keys(world: &mut World) {
    for key in [
        KeyCode::KeyW,
        KeyCode::KeyS,
        KeyCode::KeyA,
        KeyCode::KeyD,
        KeyCode::KeyE,
        KeyCode::ShiftLeft,
        KeyCode::ShiftRight,
        KeyCode::ControlLeft,
        KeyCode::ControlRight,
    ] {
        set_key(world, key, false);
    }
}

fn approach_vehicle(world: &mut World, kind: Kind) -> bool {
    let Some((player, heading)) = player_pose(world) else {
        return false;
    };
    let Some((vehicle_position, rotation, collider)) = world
        .resource::<Exploration>()
        .vehicle_entity(kind)
        .and_then(|entity| {
            Some((
                world.get::<Position>(entity)?.0,
                world.get::<Rotation>(entity)?.0,
                world.get::<Collider>(entity)?,
            ))
        })
    else {
        return false;
    };
    let target_surface = collider
        .project_point(vehicle_position, rotation, player, true)
        .0;
    let target = target_surface - player;
    let distance = target.length();
    if distance <= 2.5 {
        set_key(world, KeyCode::KeyW, false);
        set_key(world, KeyCode::KeyA, false);
        set_key(world, KeyCode::KeyD, false);
        return true;
    }
    let up = player.normalize_or(Vec3::Y);
    let tangent = target - up * target.dot(up);
    let direction = tangent.normalize_or(heading);
    let angle = up
        .dot(heading.cross(direction))
        .atan2(heading.dot(direction));
    set_key(world, KeyCode::KeyW, distance > 2.2);
    set_key(world, KeyCode::KeyA, angle > 0.12);
    set_key(world, KeyCode::KeyD, angle < -0.12);
    false
}

fn controlled_body_motion(world: &mut World) -> Option<(Vec3, Vec3)> {
    let (player_entity, player, player_velocity) = player_motion(world)?;
    let state = world.resource::<Exploration>();
    if state.is_in_vehicle() {
        let car = state.vehicle_entity(Kind::Car);
        let plane = state.vehicle_entity(Kind::Plane);
        let candidate = [car, plane]
            .into_iter()
            .flatten()
            .filter_map(|entity| {
                let position = world.get::<Position>(entity)?.0;
                let velocity = world
                    .get::<LinearVelocity>(entity)
                    .map_or(Vec3::ZERO, |velocity| velocity.0);
                Some((position.distance(player), position, velocity))
            })
            .min_by(|left, right| left.0.total_cmp(&right.0));
        if let Some((distance, position, velocity)) = candidate
            && distance < 1.0
        {
            return Some((position, velocity));
        }
    }
    let _ = player_entity;
    Some((player, player_velocity))
}

fn capture_transition_diagnostic(world: &mut World) {
    let now = world.resource::<Time<Real>>().elapsed_secs_f64();
    let Some(mut diagnostic) = world.remove_resource::<PlanetTransitionDiagnostic>() else {
        return;
    };
    let Some(started_at) = diagnostic.started_at else {
        world.insert_resource(diagnostic);
        return;
    };
    if diagnostic.finished {
        world.insert_resource(diagnostic);
        return;
    }
    let elapsed = now - started_at;
    let movement_window = diagnostic
        .movement_started_at
        .is_some_and(|movement_started_at| elapsed >= movement_started_at)
        && diagnostic
            .movement_finished_at
            .is_none_or(|movement_finished_at| elapsed <= movement_finished_at)
        || diagnostic.follow_probe_mode.is_some()
            && diagnostic
                .follow_probe_motion_started_at
                .is_some_and(|started_at| elapsed >= started_at);
    let sample = diagnostic_sample(world, elapsed, movement_window);
    diagnostic.samples.push(sample);
    let follow_probe_mode = match diagnostic.follow_probe_stage {
        FollowProbeStage::OnFoot | FollowProbeStage::Vehicle => diagnostic.follow_probe_mode,
        _ => None,
    };
    if let Some(mode) = follow_probe_mode {
        let motion_elapsed_s = diagnostic
            .follow_probe_motion_started_at
            .map_or(f64::NAN, |motion_started_at| elapsed - motion_started_at);
        diagnostic.follow_probe_samples.push(follow_probe_sample(
            world,
            elapsed,
            motion_elapsed_s,
            mode,
        ));
    }
    if diagnostic.drag_finished
        && !diagnostic.opposite_pose_confirmed
        && diagnostic.drag_failed_at.is_none()
        && sample.planet_active
    {
        diagnostic.drag_end_direction_dot = sample
            .camera
            .translation
            .normalize_or(Vec3::Y)
            .dot(diagnostic.drag_start_direction);
        if opposite_pose_reached(diagnostic.drag_end_direction_dot) {
            diagnostic.opposite_pose_confirmed = true;
            diagnostic.opposite_pose_at = Some(elapsed);
            let drag_end_direction_dot = diagnostic.drag_end_direction_dot;
            write_diagnostic_event(
                &mut diagnostic,
                elapsed,
                "production-pointer",
                format!(
                    "camera reached the opposite pose; radial dot with starting body={:.5}",
                    drag_end_direction_dot
                ),
            );
        } else if drag_confirmation_expired(
            diagnostic.drag_finished_at,
            elapsed,
            diagnostic.opposite_pose_confirmed,
        ) {
            diagnostic.drag_failed_at = Some(elapsed);
            diagnostic.drag_failure_camera = Some(sample.camera);
            let start_position = diagnostic
                .drag_start_camera
                .map_or(Vec3::splat(f32::NAN), |camera| camera.translation);
            let failure_position = sample.camera.translation;
            let start_cursor = diagnostic
                .drag_start_cursor
                .map_or(Vec2::splat(f32::NAN), |cursor| cursor);
            let end_cursor = diagnostic
                .drag_end_cursor
                .map_or(Vec2::splat(f32::NAN), |cursor| cursor);
            let planned_frames = diagnostic.drag_frames.len();
            let reason = format!(
                "antipode drag did not reach dot <= {DIAGNOSTIC_OPPOSITE_DOT_THRESHOLD:.3} within {DIAGNOSTIC_DRAG_CONFIRMATION_TIMEOUT_SECONDS:.1}s; dot={:.5}; frames={}/{}; start_cursor=({:.1},{:.1}); end_cursor=({:.1},{:.1}); start_camera=({:.2},{:.2},{:.2}); end_camera=({:.2},{:.2},{:.2})",
                diagnostic.drag_end_direction_dot,
                diagnostic.drag_input_frames,
                planned_frames,
                start_cursor.x,
                start_cursor.y,
                end_cursor.x,
                end_cursor.y,
                start_position.x,
                start_position.y,
                start_position.z,
                failure_position.x,
                failure_position.y,
                failure_position.z,
            );
            diagnostic.errors.push(reason.clone());
            write_diagnostic_event(
                &mut diagnostic,
                elapsed,
                "production-pointer-failed",
                reason,
            );
        }
    }
    let mut screenshot_requested = false;
    if let Some(opened_at) = diagnostic.m_event_times[0] {
        for (index, (offset, name)) in [
            (0.4, "entry-0400ms.png"),
            (0.8, "entry-0800ms.png"),
            (1.4, "entry-1400ms.png"),
        ]
        .into_iter()
        .enumerate()
        {
            let target = opened_at + offset;
            if elapsed >= target && !diagnostic.requested_captures.contains(name) {
                screenshot_requested = request_diagnostic_screenshot(world, &mut diagnostic, name);
                if screenshot_requested {
                    diagnostic.entry_capture_lateness_s[index] = (elapsed - target).max(0.0);
                    record_capture_metadata(&mut diagnostic, name, target, sample);
                    write_diagnostic_event(&mut diagnostic, elapsed, "screenshot", name);
                    break;
                }
            }
        }
    }
    if !screenshot_requested {
        if let Some(return_started_at) = diagnostic.m_event_times[3] {
            for (offset, name) in [
                (DIAGNOSTIC_RETURN_MIDPOINT_SECONDS, "return-midpoint.png"),
                (DIAGNOSTIC_RETURN_END_SECONDS, "return-end.png"),
            ]
            .into_iter()
            {
                let target = return_started_at + offset;
                if elapsed >= target && !diagnostic.requested_captures.contains(name) {
                    screenshot_requested =
                        request_diagnostic_screenshot(world, &mut diagnostic, name);
                    if screenshot_requested {
                        record_capture_metadata(&mut diagnostic, name, target, sample);
                        write_diagnostic_event(&mut diagnostic, elapsed, "screenshot", name);
                        break;
                    }
                }
            }
        }
    }
    if !screenshot_requested && diagnostic.opposite_pose_confirmed {
        let target = diagnostic.opposite_pose_at.unwrap_or(elapsed);
        screenshot_requested = capture_once(
            world,
            &mut diagnostic,
            "drag-opposite.png",
            target,
            |diag| {
                diag.opposite_pose_confirmed && opposite_pose_reached(diag.drag_end_direction_dot)
            },
        );
    }
    if !screenshot_requested
        && let Some(mode) = follow_probe_mode
        && let Some(motion_started_at) = diagnostic.follow_probe_motion_started_at
        && let Some((capture_index, target)) = [
            (0, FOLLOW_PROBE_CAPTURE_DELAY_SECONDS),
            (1, FOLLOW_PROBE_SECOND_CAPTURE_DELAY_SECONDS),
        ]
        .into_iter()
        .find_map(|(index, target)| {
            (!diagnostic
                .requested_captures
                .contains(mode.capture_names()[index])
                && elapsed - motion_started_at >= target)
                .then_some((index, motion_started_at + target))
        })
        && sample.planet_active
        && sample.planet_ready
        && sample.follows
        && (sample.attained_radius - PLANET_VIEW_NEAR_RADIUS).abs()
            <= FOLLOW_PROBE_RADIUS_TOLERANCE_M
        && diagnostic
            .follow_probe_motion_start
            .zip(controlled_body_motion(world).map(|(position, _)| position))
            .is_some_and(|(start, current)| start.distance(current) >= 0.5)
    {
        screenshot_requested = capture_once(
            world,
            &mut diagnostic,
            mode.capture_names()[capture_index],
            target,
            |_| true,
        );
    }
    if !screenshot_requested && let Some(opened_at) = diagnostic.m_event_times[4] {
        let target = opened_at + 0.8;
        if elapsed >= target && sample.planet_active {
            screenshot_requested =
                capture_once(world, &mut diagnostic, "storm-hidden.png", target, |diag| {
                    diag.m_event_times[4].is_some_and(|open_at| {
                        diag.samples.iter().any(|row| {
                            row.elapsed >= open_at
                                && row.planet_active
                                && row.weather_intensity > 0.2
                                && row.precipitation_active >= 100
                                && row.precipitation_hidden == row.precipitation_active
                        })
                    })
                });
        }
    }
    if !screenshot_requested && let Some(closed_at) = diagnostic.m_event_times[5] {
        let target = closed_at + 0.8;
        if elapsed >= target && !sample.planet_active {
            capture_once(
                world,
                &mut diagnostic,
                "storm-restored.png",
                target,
                |diag| {
                    diag.m_event_times[5].is_some_and(|close_at| {
                        diag.samples.iter().any(|row| {
                            row.elapsed >= close_at
                                && !row.planet_active
                                && row.weather_intensity > 0.2
                                && row.precipitation_visible > 0
                        })
                    })
                },
            );
        }
    }
    let final_event = 5;
    let finish_delay = 0.8 + DIAGNOSTIC_RETURN_SETTLE_SECONDS;
    let finished_after_return = diagnostic.m_event_times[final_event]
        .is_some_and(|return_at| !sample.planet_active && elapsed >= return_at + finish_delay);
    let timed_out = elapsed >= DIAGNOSTIC_ROUTE_TIMEOUT_SECONDS;
    if finished_after_return || timed_out {
        diagnostic.route_timed_out = timed_out && !finished_after_return;
        if diagnostic.route_timed_out {
            write_diagnostic_event(
                &mut diagnostic,
                elapsed,
                "route-timeout",
                "required stage readiness did not arrive before the diagnostic deadline",
            );
        }
        finish_transition_diagnostic(&mut diagnostic);
        world.insert_resource(diagnostic);
        // Leave the detailed diagnostic pass/reject decision to the wrapper,
        // which can report the saved metrics even when this probe is rejected.
        world.write_message(AppExit::Success);
        return;
    }
    world.insert_resource(diagnostic);
}

fn diagnostic_sample(world: &mut World, elapsed: f64, movement_window: bool) -> DiagnosticSample {
    let (player, velocity, sleeping) = player_motion(world)
        .map(|(entity, player, velocity)| {
            (player, velocity, world.get::<Sleeping>(entity).is_some())
        })
        .unwrap_or((Vec3::splat(f32::NAN), Vec3::splat(f32::NAN), false));
    let collision_colliders = if movement_window {
        world
            .query_filtered::<Entity, With<Collider>>()
            .iter(world)
            .count()
    } else {
        0
    };
    let state = world.resource::<Exploration>();
    let (requested_radius, attained_radius) = state.planet_view_camera_radii();
    let planet_active = state.is_planet_view_active();
    let planet_ready = state.planet_view_ready();
    let follows = state.planet_view_follows_body();
    let camera = main_camera_pose(world).unwrap_or_default();
    let virtual_time = world.resource::<Time<Virtual>>();
    let weather = world.resource::<Weather>();
    let weather_intensity = weather.precip;
    let weather_kind = world
        .get_resource::<shared::terrain::TerrainGen>()
        .map(|terrain| {
            if crate::weather::is_snow(
                terrain.temperature_at(shared::sphere::SpherePos::new(player)),
            ) {
                "snow"
            } else {
                "rain"
            }
        })
        .unwrap_or("unknown");
    let fixed_time_elapsed = world.resource::<Time<Fixed>>().elapsed_secs_f64();
    let virtual_rate = virtual_time.relative_speed();
    let virtual_paused = virtual_time.is_paused();
    let mut particles = world.query::<(&Precip, &Transform, &Visibility)>();
    let (precipitation_active, precipitation_hidden, precipitation_visible, first_particle) =
        particles.iter(world).fold(
            (0, 0, 0, Vec3::splat(f32::NAN)),
            |(active, hidden, visible, first), (particle, transform, visibility)| {
                if transform.scale == Vec3::ZERO {
                    return (active, hidden, visible, first);
                }
                let first = if particle.idx == 0 {
                    transform.translation
                } else {
                    first
                };
                if *visibility == Visibility::Hidden {
                    (active + 1, hidden + 1, visible, first)
                } else {
                    (active + 1, hidden, visible + 1, first)
                }
            },
        );
    DiagnosticSample {
        elapsed,
        real_delta_ms: world.resource::<Time<Real>>().delta_secs_f64() * 1000.0,
        fixed_time_elapsed,
        planet_active,
        planet_ready,
        follows,
        requested_radius,
        attained_radius,
        camera,
        player_position: player,
        player_velocity: velocity,
        player_sleeping: sleeping,
        collision_colliders,
        virtual_rate,
        virtual_paused,
        weather_intensity,
        weather_kind,
        precipitation_active,
        precipitation_hidden,
        precipitation_visible,
        first_particle,
    }
}

fn follow_probe_sample(
    world: &mut World,
    elapsed: f64,
    motion_elapsed_s: f64,
    mode: FollowProbeMode,
) -> FollowProbeSample {
    let (view_active, follows, requested_radius, attained_radius) = {
        let state = world.resource::<Exploration>();
        let (requested, attained) = state.planet_view_camera_radii();
        (
            state.is_planet_view_active(),
            state.planet_view_follows_body(),
            requested,
            attained,
        )
    };
    let (body_position, body_velocity) =
        controlled_body_motion(world).unwrap_or((Vec3::splat(f32::NAN), Vec3::splat(f32::NAN)));
    let camera_pose = main_camera_pose(world).unwrap_or_default();
    let camera_direction = camera_pose.translation.normalize_or(Vec3::Y);
    let body_direction = body_position.normalize_or(Vec3::Y);
    let radial_lag_rad = radial_angle(camera_direction, body_direction);
    let marker_anchor_position = {
        let mut players = world.query_filtered::<&Position, With<crate::map::Player>>();
        players
            .iter(world)
            .next()
            .map(|position| position.0)
            .unwrap_or(Vec3::splat(f32::NAN))
    };
    let (projected_center_logical, projected_marker_anchor_logical) = {
        let mut cameras = world.query_filtered::<(&Camera, &GlobalTransform), With<MainCamera>>();
        cameras
            .iter(world)
            .find_map(|(camera, transform)| {
                let body = camera.world_to_viewport(transform, body_position).ok()?;
                let anchor = camera
                    .world_to_viewport(transform, marker_anchor_position)
                    .unwrap_or(Vec2::splat(f32::NAN));
                Some((body, anchor))
            })
            .unwrap_or((Vec2::splat(f32::NAN), Vec2::splat(f32::NAN)))
    };
    let scale_factor = primary_window(world).map_or(f32::NAN, |window| window.scale_factor());
    let marker = crate::planet_markers::explorer_marker_layout(world);
    let marker_center_physical =
        marker.map_or(Vec2::splat(f32::NAN), |layout| layout.physical_center);
    let marker_size_physical = marker.map_or(Vec2::splat(f32::NAN), |layout| layout.physical_size);
    let marker_visible = marker.is_some_and(|layout| layout.visible);
    let expected_center_physical = projected_center_logical * scale_factor;
    let expected_marker_anchor_physical = projected_marker_anchor_logical * scale_factor;
    let marker_anchor_body_separation_m = marker_anchor_position.distance(body_position);
    let marker_anchor_registration_error_physical_px = marker
        .map(|layout| {
            layout
                .physical_center
                .distance(expected_marker_anchor_physical)
        })
        .unwrap_or(f32::NAN);
    let registration_error_physical_px = marker
        .map(|layout| layout.physical_center.distance(expected_center_physical))
        .unwrap_or(f32::NAN);
    FollowProbeSample {
        elapsed,
        motion_elapsed_s,
        mode: Some(mode),
        view_active,
        follows,
        requested_radius,
        attained_radius,
        body_speed_m_s: body_velocity.length(),
        radial_lag_rad,
        projected_center_logical,
        marker_center_physical,
        marker_size_physical,
        marker_anchor_body_separation_m,
        marker_anchor_registration_error_physical_px,
        marker_visible,
        registration_error_physical_px,
    }
}

fn radial_angle(from: Vec3, to: Vec3) -> f32 {
    let from = from.normalize_or(Vec3::Y);
    let to = to.normalize_or(Vec3::Y);
    from.cross(to).length().atan2(from.dot(to))
}

fn capture_once(
    world: &mut World,
    diagnostic: &mut PlanetTransitionDiagnostic,
    name: &str,
    nominal_elapsed: f64,
    ready: impl FnOnce(&PlanetTransitionDiagnostic) -> bool,
) -> bool {
    if diagnostic.requested_captures.contains(name) || !ready(diagnostic) {
        return false;
    }
    let Some(sample) = diagnostic.samples.last().copied() else {
        return false;
    };
    if !request_diagnostic_screenshot(world, diagnostic, name) {
        return false;
    }
    record_capture_metadata(diagnostic, name, nominal_elapsed, sample);
    write_diagnostic_event(diagnostic, sample.elapsed, "screenshot", name);
    true
}

fn record_capture_metadata(
    diagnostic: &mut PlanetTransitionDiagnostic,
    name: &str,
    nominal_elapsed: f64,
    sample: DiagnosticSample,
) {
    let late_by = (sample.elapsed - nominal_elapsed).max(0.0);
    diagnostic.capture_metadata.push(format!(
        "{},{nominal_elapsed:.5},{:.5},{late_by:.5},{:.4},{:.4},{:.4},{:.4}\n",
        csv(name),
        sample.elapsed,
        sample.attained_radius,
        sample.camera.translation.x,
        sample.camera.translation.y,
        sample.camera.translation.z,
    ));
}

fn request_diagnostic_screenshot(
    world: &mut World,
    diagnostic: &mut PlanetTransitionDiagnostic,
    name: &str,
) -> bool {
    if diagnostic.requested_captures.contains(name) {
        return false;
    }
    let Some(screenshot) = super::diagnostic_screenshot(world) else {
        return false;
    };
    diagnostic.requested_captures.insert(name.to_owned());
    let path = diagnostic.directory.join("captures").join(name);
    world.spawn(screenshot).observe(save_to_disk(path.clone()));
    info!(path = %path.display(), "Planet transition diagnostic screenshot requested");
    true
}

fn finish_transition_diagnostic(diagnostic: &mut PlanetTransitionDiagnostic) {
    let mut trace = String::new();
    for sample in &diagnostic.samples {
        trace.push_str(&sample.csv_row());
    }
    if let Err(error) = fs::write(
        diagnostic.directory.join("camera-transition-trace.csv"),
        format!(
            "elapsed_s,real_delta_ms,fixed_time_s,planet_active,planet_ready,follows,requested_radius_m,attained_radius_m,camera_x,camera_y,camera_z,camera_qx,camera_qy,camera_qz,camera_qw,player_x,player_y,player_z,player_vx,player_vy,player_vz,player_sleeping,physics_colliders,virtual_rate,virtual_paused,weather_precip,weather_kind,precipitation_active,precipitation_hidden,precipitation_visible,particle0_x,particle0_y,particle0_z\n{trace}"
        ),
    ) {
        diagnostic
            .errors
            .push(format!("write camera transition trace: {error}"));
    }
    let follow_trace = format!(
        "elapsed_s,motion_elapsed_s,mode,view_active,follow,requested_radius_m,attained_radius_m,body_speed_m_s,radial_lag_rad,projected_x_logical,projected_y_logical,marker_x_physical,marker_y_physical,marker_width_physical,marker_height_physical,marker_anchor_body_separation_m,marker_anchor_registration_error_physical_px,registration_error_physical_px,marker_visible\n{}",
        diagnostic
            .follow_probe_samples
            .iter()
            .map(|sample| sample.csv_row())
            .collect::<String>()
    );
    if let Err(error) = fs::write(diagnostic.directory.join("follow-probe.csv"), follow_trace) {
        diagnostic
            .errors
            .push(format!("write near-follow diagnostic trace: {error}"));
    }
    let capture_metadata = format!(
        "capture,nominal_elapsed_s,request_elapsed_s,late_by_s,camera_radius_m,camera_x,camera_y,camera_z\n{}",
        diagnostic.capture_metadata.concat()
    );
    if let Err(error) = fs::write(
        diagnostic.directory.join("capture-metadata.csv"),
        capture_metadata,
    ) {
        diagnostic
            .errors
            .push(format!("write capture metadata: {error}"));
    }
    let movement_start = diagnostic.movement_started_at.unwrap_or(f64::INFINITY);
    let movement_end = diagnostic.movement_finished_at.unwrap_or(f64::NEG_INFINITY);
    let motion_samples = diagnostic
        .samples
        .iter()
        .filter(|sample| {
            (movement_start..=movement_end).contains(&sample.elapsed) && sample.planet_active
        })
        .collect::<Vec<_>>();
    let movement = continuous_body_path(
        motion_samples
            .iter()
            .map(|sample| Some(("on-foot", sample.player_position))),
    );
    let fixed_advanced = motion_samples
        .first()
        .zip(motion_samples.last())
        .is_some_and(|(first, last)| last.fixed_time_elapsed > first.fixed_time_elapsed + 0.5);
    let physics_live = motion_samples
        .iter()
        .any(|sample| sample.collision_colliders > 0 && !sample.player_sleeping);
    let physics_advanced_while_moving = motion_samples.windows(2).any(|pair| {
        let [before, after] = pair else {
            unreachable!("a two-sample window has exactly two entries")
        };
        let displacement = before.player_position.distance(after.player_position);
        displacement > 0.001
            && displacement <= MAX_CONTINUOUS_BODY_STEP_M
            && after.fixed_time_elapsed > before.fixed_time_elapsed
            && !after.player_sleeping
            && after.collision_colliders > 0
            && before.planet_active
            && after.planet_active
    });
    let movement_passed = movement.traveled_m >= MIN_MEASURED_BODY_PATH_M
        && movement.max_excursion_m >= MIN_MEASURED_BODY_EXCURSION_M
        && fixed_advanced
        && physics_live
        && physics_advanced_while_moving;
    let storm_open_at = diagnostic.m_event_times[4].unwrap_or(f64::INFINITY);
    let storm_close_at = diagnostic.m_event_times[5].unwrap_or(f64::INFINITY);
    let weather_hidden = diagnostic.samples.iter().any(|sample| {
        sample.elapsed >= storm_open_at
            && sample.planet_active
            && sample.weather_intensity > 0.2
            && sample.precipitation_active >= 100
            && sample.precipitation_hidden == sample.precipitation_active
    });
    let weather_restored = diagnostic.samples.iter().any(|sample| {
        sample.elapsed >= storm_close_at
            && !sample.planet_active
            && sample.weather_intensity > 0.2
            && sample.precipitation_visible > 0
    });
    let missing_captures = DIAGNOSTIC_CAPTURE_NAMES
        .into_iter()
        .filter(|name| !file_is_nonempty(&diagnostic.directory.join("captures").join(name)))
        .collect::<Vec<_>>();
    let capture_metadata_valid = capture_metadata_is_exactly_once(&diagnostic.capture_metadata);
    let drag_passed = diagnostic.opposite_pose_confirmed
        && diagnostic.drag_input_frames >= 2
        && opposite_pose_reached(diagnostic.drag_end_direction_dot);
    let entry_captures_timely = diagnostic
        .entry_capture_lateness_s
        .iter()
        .all(|late| late.is_finite() && *late <= DIAGNOSTIC_MAX_ENTRY_CAPTURE_LATE_SECONDS);
    let on_foot_steady_samples =
        steady_follow_probe_samples(&diagnostic.follow_probe_samples, FollowProbeMode::OnFoot);
    let car_steady_samples =
        steady_follow_probe_samples(&diagnostic.follow_probe_samples, FollowProbeMode::Car);
    let plane_steady_samples =
        steady_follow_probe_samples(&diagnostic.follow_probe_samples, FollowProbeMode::Plane);
    let on_foot_follow_passed = follow_probe_window_passed(&on_foot_steady_samples);
    let car_follow_passed = follow_probe_window_passed(&car_steady_samples);
    let plane_follow_passed = follow_probe_window_passed(&plane_steady_samples);
    let follow_probes_passed = on_foot_follow_passed && car_follow_passed && plane_follow_passed;
    let rendered_pose_cases_passed = rendered_pose_cases_passed(diagnostic);
    let steady_samples = on_foot_steady_samples
        .iter()
        .chain(&car_steady_samples)
        .chain(&plane_steady_samples)
        .copied()
        .collect::<Vec<_>>();
    let near_follow_steady_sample_count = steady_samples.len();
    let max_follow_radial_lag_rad = steady_samples
        .iter()
        .map(|sample| sample.radial_lag_rad)
        .fold(0.0_f32, f32::max);
    let max_registration_error_physical_px = steady_samples
        .iter()
        .map(|sample| sample.registration_error_physical_px)
        .fold(0.0_f32, f32::max);
    let max_marker_anchor_registration_error_physical_px = steady_samples
        .iter()
        .map(|sample| sample.marker_anchor_registration_error_physical_px)
        .fold(0.0_f32, f32::max);
    let near_follow_sample_count = diagnostic.follow_probe_samples.len();
    let on_foot_steady_window_s = follow_probe_window_span(&on_foot_steady_samples);
    let car_steady_window_s = follow_probe_window_span(&car_steady_samples);
    let plane_steady_window_s = follow_probe_window_span(&plane_steady_samples);
    let m_event_elapsed_s = diagnostic
        .m_event_times
        .iter()
        .map(|at| at.map_or_else(|| "missing".to_owned(), |at| format!("{at:.5}")))
        .collect::<Vec<_>>()
        .join(",");
    let route_complete = diagnostic.m_events_sent[..4].iter().all(|sent| *sent)
        && diagnostic.drag_finished
        && diagnostic.drag_finished_at.is_some()
        && drag_passed
        && diagnostic.movement_finished
        && diagnostic.movement_started_at.is_some()
        && diagnostic.movement_finished_at.is_some()
        && movement_passed
        && diagnostic.follow_probe_stage == FollowProbeStage::Complete
        && follow_probes_passed
        && rendered_pose_cases_passed
        && diagnostic.samples.iter().any(|sample| sample.planet_active)
        && diagnostic
            .samples
            .last()
            .is_some_and(|sample| !sample.planet_active);
    let visual_route_complete = route_complete && diagnostic.m_events_sent.iter().all(|sent| *sent);
    let passed = diagnostic.errors.is_empty()
        && visual_route_complete
        && missing_captures.is_empty()
        && capture_metadata_valid
        && entry_captures_timely
        && drag_passed
        && rendered_pose_cases_passed
        && movement_passed
        && follow_probes_passed
        && weather_hidden
        && weather_restored;
    let status = if passed {
        "diagnostic-complete"
    } else {
        "diagnostic-rejected"
    };
    let route_timed_out = diagnostic.route_timed_out;
    let m_events_sent = diagnostic
        .m_events_sent
        .iter()
        .filter(|sent| **sent)
        .count();
    let drag_started_s = diagnostic
        .drag_started_at
        .map_or_else(|| "missing".to_owned(), |at| format!("{at:.5}"));
    let drag_released_s = diagnostic
        .drag_finished_at
        .map_or_else(|| "missing".to_owned(), |at| format!("{at:.5}"));
    let drag_failed_s = diagnostic
        .drag_failed_at
        .map_or_else(|| "missing".to_owned(), |at| format!("{at:.5}"));
    let opposite_pose_s = diagnostic
        .opposite_pose_at
        .map_or_else(|| "missing".to_owned(), |at| format!("{at:.5}"));
    let drag_input_frames = diagnostic.drag_input_frames;
    let drag_planned_frames = diagnostic.drag_frames.len();
    let drag_opposite_dot = diagnostic.drag_end_direction_dot;
    let drag_start_camera_position = diagnostic.drag_start_camera.map_or_else(
        || "missing".to_owned(),
        |camera| {
            format!(
                "{:.2},{:.2},{:.2}",
                camera.translation.x, camera.translation.y, camera.translation.z
            )
        },
    );
    let drag_failure_camera_position = diagnostic.drag_failure_camera.map_or_else(
        || "missing".to_owned(),
        |camera| {
            format!(
                "{:.2},{:.2},{:.2}",
                camera.translation.x, camera.translation.y, camera.translation.z
            )
        },
    );
    let drag_start_cursor = diagnostic.drag_start_cursor.map_or_else(
        || "missing".to_owned(),
        |cursor| format!("{:.1},{:.1}", cursor.x, cursor.y),
    );
    let drag_end_cursor = diagnostic.drag_end_cursor.map_or_else(
        || "missing".to_owned(),
        |cursor| format!("{:.1},{:.1}", cursor.x, cursor.y),
    );
    let body_path_m = movement.traveled_m;
    let max_body_excursion_m = movement.max_excursion_m;
    let movement_started_s = diagnostic
        .movement_started_at
        .map_or_else(|| "missing".to_owned(), |at| format!("{at:.5}"));
    let movement_finished_s = diagnostic
        .movement_finished_at
        .map_or_else(|| "missing".to_owned(), |at| format!("{at:.5}"));
    let collision_world_live_during_movement = motion_samples
        .iter()
        .any(|sample| sample.collision_colliders > 0);
    let entry_capture_lateness_s = diagnostic
        .entry_capture_lateness_s
        .into_iter()
        .map(|late| format!("{late:.5}"))
        .collect::<Vec<_>>()
        .join(",");
    let on_foot_steady_sample_count = on_foot_steady_samples.len();
    let car_steady_sample_count = car_steady_samples.len();
    let plane_steady_sample_count = plane_steady_samples.len();
    let capture_metadata_rows = diagnostic.capture_metadata.len();
    let rendered_pose_case_index = diagnostic.rendered_pose_case_index;
    let rendered_pose_failed_cases = diagnostic.rendered_pose_failed_cases;
    let rendered_pose_stage = format!("{:?}", diagnostic.rendered_pose_stage);
    let weather_kind_at_player = diagnostic
        .samples
        .iter()
        .find(|sample| sample.weather_intensity > 0.2)
        .map_or("unknown", |sample| sample.weather_kind);
    let missing_captures = missing_captures.join(";");
    let errors = diagnostic.errors.join(";");
    let report = format!(
        "mode=visual-diagnostic\nacceptance_claim=none\nstatus={status}\nroute_timed_out={route_timed_out}\nm_events_sent={m_events_sent}\nm_event_elapsed_s={m_event_elapsed_s}\nroute_complete={route_complete}\ndrag_started_s={drag_started_s}\ndrag_released_s={drag_released_s}\ndrag_failed_s={drag_failed_s}\nopposite_pose_s={opposite_pose_s}\ndrag_input_frames={drag_input_frames}\ndrag_planned_frames={drag_planned_frames}\ndrag_opposite_dot={drag_opposite_dot:.5}\ndrag_start_camera_position={drag_start_camera_position}\ndrag_failure_camera_position={drag_failure_camera_position}\ndrag_start_cursor={drag_start_cursor}\ndrag_end_cursor={drag_end_cursor}\nrendered_pose_cases_passed={rendered_pose_cases_passed}\nrendered_pose_case_index={rendered_pose_case_index}\nrendered_pose_failed_cases={rendered_pose_failed_cases}\nrendered_pose_stage={rendered_pose_stage}\nbody_path_m={body_path_m:.4}\nmax_body_excursion_m={max_body_excursion_m:.4}\nmovement_started_s={movement_started_s}\nmovement_finished_s={movement_finished_s}\nfixed_time_advanced_during_movement={fixed_advanced}\nphysics_live_during_movement={physics_live}\ncollision_world_live_during_movement={collision_world_live_during_movement}\nphysics_advanced_while_moving={physics_advanced_while_moving}\nmovement_passed={movement_passed}\nnear_follow_on_foot_passed={on_foot_follow_passed}\nnear_follow_car_passed={car_follow_passed}\nnear_follow_plane_passed={plane_follow_passed}\nnear_follow_sample_count={near_follow_sample_count}\nnear_follow_steady_sample_count={near_follow_steady_sample_count}\nnear_follow_on_foot_steady_sample_count={on_foot_steady_sample_count}\nnear_follow_on_foot_steady_window_s={on_foot_steady_window_s:.3}\nnear_follow_car_steady_sample_count={car_steady_sample_count}\nnear_follow_car_steady_window_s={car_steady_window_s:.3}\nnear_follow_plane_steady_sample_count={plane_steady_sample_count}\nnear_follow_plane_steady_window_s={plane_steady_window_s:.3}\nmax_follow_radial_lag_rad={max_follow_radial_lag_rad:.6}\nmax_marker_registration_error_physical_px={max_registration_error_physical_px:.3}\nmax_marker_anchor_registration_error_physical_px={max_marker_anchor_registration_error_physical_px:.3}\nentry_capture_lateness_s={entry_capture_lateness_s}\nentry_captures_timely={entry_captures_timely}\ncapture_metadata_rows={capture_metadata_rows}\ncapture_metadata_valid={capture_metadata_valid}\nweather_kind_at_player={weather_kind_at_player}\nweather_hidden_while_planet_active={weather_hidden}\nweather_restored_after_return={weather_restored}\nmissing_captures={missing_captures}\nerrors={errors}\n",
    );
    if let Err(error) = fs::write(diagnostic.directory.join("diagnostic-status.txt"), report) {
        diagnostic
            .errors
            .push(format!("write diagnostic status: {error}"));
    }
    diagnostic.finished = true;
    info!(
        status,
        body_path_m = movement.traveled_m,
        drag_dot = diagnostic.drag_end_direction_dot,
        "Planet transition diagnostic finished"
    );
}

fn write_diagnostic_event(
    diagnostic: &mut PlanetTransitionDiagnostic,
    elapsed: f64,
    event: &str,
    result: impl AsRef<str>,
) {
    let row = format!("{elapsed:.5},{},{}\n", csv(event), csv(result.as_ref()));
    append_diagnostic_line(
        &diagnostic.directory,
        "events.csv",
        row,
        &mut diagnostic.errors,
    );
}

fn append_diagnostic_line(
    directory: &Path,
    file_name: &str,
    row: String,
    errors: &mut Vec<String>,
) {
    let path = directory.join(file_name);
    match OpenOptions::new().create(true).append(true).open(&path) {
        Ok(mut file) => {
            if let Err(error) = file.write_all(row.as_bytes()) {
                errors.push(format!("write {}: {error}", path.display()));
            }
        }
        Err(error) => errors.push(format!("open {}: {error}", path.display())),
    }
}

fn return_captures_saved(diagnostic: &PlanetTransitionDiagnostic) -> bool {
    ["return-midpoint.png", "return-end.png"]
        .into_iter()
        .all(|name| file_is_nonempty(&diagnostic.directory.join("captures").join(name)))
}

fn steady_follow_probe_samples(
    samples: &[FollowProbeSample],
    mode: FollowProbeMode,
) -> Vec<&FollowProbeSample> {
    let window_start = FOLLOW_PROBE_MOVE_SECONDS - FOLLOW_PROBE_STEADY_WINDOW_SECONDS;
    samples
        .iter()
        .filter(|sample| {
            sample.mode == Some(mode)
                && sample.motion_elapsed_s >= window_start
                && sample.motion_elapsed_s <= FOLLOW_PROBE_MOVE_SECONDS
                && sample.view_active
                && sample.follows
                && sample.requested_radius
                    <= PLANET_VIEW_NEAR_RADIUS + FOLLOW_PROBE_RADIUS_TOLERANCE_M
                && (sample.attained_radius - PLANET_VIEW_NEAR_RADIUS).abs()
                    <= FOLLOW_PROBE_RADIUS_TOLERANCE_M
                && sample.body_speed_m_s >= 0.5
                && sample.marker_visible
                && sample.radial_lag_rad.is_finite()
                && sample.registration_error_physical_px.is_finite()
        })
        .collect()
}

fn follow_probe_window_span(samples: &[&FollowProbeSample]) -> f64 {
    samples
        .first()
        .zip(samples.last())
        .map_or(0.0, |(first, last)| {
            last.motion_elapsed_s - first.motion_elapsed_s
        })
}

fn follow_probe_window_passed(samples: &[&FollowProbeSample]) -> bool {
    samples.len() >= FOLLOW_PROBE_MIN_STEADY_SAMPLE_COUNT
        && follow_probe_window_span(samples) >= FOLLOW_PROBE_MIN_STEADY_SPAN_SECONDS
        && samples.iter().all(|sample| {
            sample.radial_lag_rad <= FOLLOW_PROBE_MAX_RADIAL_LAG_RAD
                && sample.registration_error_physical_px
                    <= FOLLOW_PROBE_MAX_MARKER_REGISTRATION_ERROR_PHYSICAL_PX
        })
}

fn capture_metadata_is_exactly_once(rows: &[String]) -> bool {
    rows.len() == DIAGNOSTIC_CAPTURE_NAMES.len()
        && DIAGNOSTIC_CAPTURE_NAMES.iter().all(|name| {
            let prefix = format!("{},", csv(name));
            rows.iter().filter(|row| row.starts_with(&prefix)).count() == 1
        })
}

#[cfg(test)]
mod transition_diagnostic_tests {
    use super::{
        DIAGNOSTIC_CAPTURE_NAMES, DIAGNOSTIC_MINIMUM_COLLIDERS, DIAGNOSTIC_STABLE_WORLD_FRAMES,
        DiagnosticWorldWarmup, FollowProbeSample, PlanetTransitionDiagnostic,
        finish_transition_diagnostic, radius_motion_flags, transition_action_due,
    };
    use bevy::{
        input::InputSystems,
        prelude::*,
        window::{PrimaryWindow, WindowResolution},
    };
    use shared::planet_view::PLANET_VIEW_FAR_RADIUS;

    use crate::{exploration::Exploration, map::MainCamera};

    fn clear_mouse_edges(mut mouse: ResMut<ButtonInput<MouseButton>>) {
        mouse.clear_just_pressed(MouseButton::Left);
        mouse.clear_just_released(MouseButton::Left);
    }

    fn transition_driver_app() -> (App, Entity) {
        let (mut app, _) = crate::exploration::tests::fixture();
        app.world_mut().spawn((
            PrimaryWindow,
            Window {
                resolution: WindowResolution::new(2560, 1440).with_scale_factor_override(2.0),
                ..default()
            },
        ));
        let camera = app
            .world_mut()
            .spawn((
                MainCamera,
                Transform::from_xyz(0.0, 2005.0, -5.0)
                    .looking_at(Vec3::new(0.0, 2000.6, 0.0), Vec3::Y),
                Projection::Perspective(PerspectiveProjection::default()),
            ))
            .id();
        app.world_mut()
            .resource_mut::<Exploration>()
            .set_planet_view_open(true);
        for _ in 0..180 {
            app.update();
        }
        app.add_systems(PreUpdate, clear_mouse_edges.before(InputSystems));
        app.add_systems(
            PreUpdate,
            super::drive_transition_diagnostic
                .after(InputSystems)
                .before(crate::exploration::ExplorationInput),
        );
        (app, camera)
    }

    fn diagnostic_test_directory(label: &str) -> std::path::PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time follows the Unix epoch")
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("terra-{label}-{}-{unique}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("create diagnostic output directory");
        directory
    }

    #[test]
    fn diagnostic_status_preserves_named_follow_and_capture_metrics() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time follows the Unix epoch")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "terra-transition-status-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).expect("create diagnostic output directory");

        let mut diagnostic = PlanetTransitionDiagnostic {
            directory: directory.clone(),
            entry_capture_lateness_s: [0.11, 0.22, 0.33],
            capture_metadata: vec!["entry-a,0.0\n".into(), "entry-b,0.0\n".into()],
            follow_probe_samples: {
                let probe =
                    |elapsed,
                     motion_elapsed_s,
                     attained_radius,
                     body_speed_m_s,
                     radial_lag_rad,
                     registration_error_physical_px| FollowProbeSample {
                        elapsed,
                        motion_elapsed_s,
                        mode: Some(super::FollowProbeMode::OnFoot),
                        view_active: true,
                        follows: true,
                        requested_radius: super::PLANET_VIEW_NEAR_RADIUS,
                        attained_radius,
                        body_speed_m_s,
                        radial_lag_rad,
                        marker_visible: true,
                        marker_anchor_body_separation_m: 0.75,
                        marker_anchor_registration_error_physical_px: 0.5,
                        registration_error_physical_px,
                        ..default()
                    };
                vec![
                    probe(9.0, 0.4, super::PLANET_VIEW_NEAR_RADIUS, 3.0, 0.9, 70.0),
                    probe(
                        10.0,
                        2.1,
                        super::PLANET_VIEW_NEAR_RADIUS + 500.0,
                        3.0,
                        0.8,
                        80.0,
                    ),
                    probe(
                        10.1,
                        2.1,
                        super::PLANET_VIEW_NEAR_RADIUS,
                        3.0,
                        0.000001,
                        1.0,
                    ),
                    probe(10.3, 2.4, super::PLANET_VIEW_NEAR_RADIUS, 0.0, 0.7, 60.0),
                    probe(
                        10.5,
                        2.6,
                        super::PLANET_VIEW_NEAR_RADIUS,
                        3.0,
                        0.000001,
                        1.5,
                    ),
                    probe(
                        10.8,
                        2.9,
                        super::PLANET_VIEW_NEAR_RADIUS,
                        3.0,
                        0.000001,
                        2.0,
                    ),
                ]
            },
            ..default()
        };
        finish_transition_diagnostic(&mut diagnostic);

        let status = std::fs::read_to_string(directory.join("diagnostic-status.txt"))
            .expect("read actual status report");
        assert!(status.contains("near_follow_sample_count=6\n"), "{status}");
        assert!(
            status.contains("near_follow_steady_sample_count=3\n"),
            "{status}"
        );
        assert!(
            status.contains("near_follow_on_foot_steady_sample_count=3\n"),
            "{status}"
        );
        assert!(
            status.contains("near_follow_on_foot_steady_window_s=0.800\n"),
            "{status}"
        );
        assert!(
            status.contains("max_follow_radial_lag_rad=0.000001\n"),
            "{status}"
        );
        assert!(
            status.contains("max_marker_registration_error_physical_px=2.000\n"),
            "{status}"
        );
        assert!(
            status.contains("rendered_pose_cases_passed=false\n"),
            "{status}"
        );
        assert!(status.contains("rendered_pose_case_index=0\n"), "{status}");
        assert!(
            status.contains("entry_capture_lateness_s=0.11000,0.22000,0.33000\n"),
            "{status}"
        );
        assert!(status.contains("capture_metadata_rows=2\n"), "{status}");
        assert!(
            status.contains("max_marker_anchor_registration_error_physical_px=0.500\n"),
            "{status}"
        );
        let follow_csv = std::fs::read_to_string(directory.join("follow-probe.csv"))
            .expect("read actual follow probe CSV");
        assert!(follow_csv.starts_with("elapsed_s,motion_elapsed_s,mode,"));
        assert!(follow_csv.contains("10.50000,2.60000,on-foot"));
        assert!(follow_csv.contains(",0.7500,0.5000,1.5000,true\n"));

        std::fs::remove_dir_all(directory).expect("remove diagnostic output directory");
    }

    #[test]
    fn steady_follow_window_requires_attained_zoom_tracking_and_pixel_rounding_bound() {
        let sample = |motion_elapsed_s, attained_radius, radial_lag_rad, registration_error| {
            FollowProbeSample {
                motion_elapsed_s,
                mode: Some(super::FollowProbeMode::OnFoot),
                view_active: true,
                follows: true,
                requested_radius: super::PLANET_VIEW_NEAR_RADIUS,
                attained_radius,
                body_speed_m_s: 3.0,
                radial_lag_rad,
                marker_visible: true,
                registration_error_physical_px: registration_error,
                ..default()
            }
        };
        let samples = (0..12)
            .map(|index| {
                let motion_elapsed_s = 2.0 + f64::from(index) * 0.08;
                sample(
                    motion_elapsed_s,
                    super::PLANET_VIEW_NEAR_RADIUS + 0.5,
                    0.000001,
                    0.7,
                )
            })
            .collect::<Vec<_>>();
        let steady = super::steady_follow_probe_samples(&samples, super::FollowProbeMode::OnFoot);
        assert!(super::follow_probe_window_passed(&steady));

        let excessive_zoom_offset =
            sample(2.5, super::PLANET_VIEW_NEAR_RADIUS + 1.1, 0.000001, 0.7);
        let samples_with_unsettled_zoom = [samples.clone(), vec![excessive_zoom_offset]].concat();
        assert_eq!(
            super::steady_follow_probe_samples(
                &samples_with_unsettled_zoom,
                super::FollowProbeMode::OnFoot
            )
            .len(),
            samples.len()
        );

        let mut excessive_lag = samples.clone();
        excessive_lag[0].radial_lag_rad = 0.0001;
        assert!(!super::follow_probe_window_passed(
            &super::steady_follow_probe_samples(&excessive_lag, super::FollowProbeMode::OnFoot)
        ));

        let mut excessive_registration = samples.clone();
        excessive_registration[0].registration_error_physical_px = 1.01;
        assert!(!super::follow_probe_window_passed(
            &super::steady_follow_probe_samples(
                &excessive_registration,
                super::FollowProbeMode::OnFoot
            )
        ));
    }

    #[test]
    fn radial_angle_keeps_sub_milliradian_error_above_float_dot_precision_floor() {
        let angle = 0.000_01_f32;
        let from = Vec3::Y;
        let to = Vec3::new(angle.sin(), angle.cos(), 0.0);

        assert!((super::radial_angle(from, to) - angle).abs() < 1e-7);
    }

    #[test]
    fn failed_rendered_pose_case_records_failure_and_advances_to_remaining_cases() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time follows the Unix epoch")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "terra-rendered-pose-failure-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).expect("create diagnostic output directory");
        let mut diagnostic = PlanetTransitionDiagnostic {
            directory: directory.clone(),
            ..default()
        };

        super::fail_rendered_pose_probe(
            &mut diagnostic,
            10.0,
            super::RENDERED_POSE_CASES[0],
            "fixture failure",
        );
        assert_eq!(diagnostic.rendered_pose_case_index, 1);
        assert_eq!(diagnostic.rendered_pose_failed_cases, 1);
        assert_eq!(
            diagnostic.rendered_pose_stage,
            super::RenderedPoseStage::Zooming
        );
        assert!(!super::rendered_pose_cases_passed(&diagnostic));

        super::fail_rendered_pose_probe(
            &mut diagnostic,
            20.0,
            super::RENDERED_POSE_CASES[1],
            "fixture failure",
        );
        super::fail_rendered_pose_probe(
            &mut diagnostic,
            30.0,
            super::RENDERED_POSE_CASES[2],
            "fixture failure",
        );
        assert_eq!(
            diagnostic.rendered_pose_case_index,
            super::RENDERED_POSE_CASES.len()
        );
        assert_eq!(diagnostic.rendered_pose_failed_cases, 3);
        assert_eq!(
            diagnostic.rendered_pose_stage,
            super::RenderedPoseStage::Complete
        );
        assert!(!super::rendered_pose_cases_passed(&diagnostic));
        std::fs::remove_dir_all(directory).expect("remove diagnostic output directory");
    }

    #[test]
    fn opposite_pose_measurement_requires_the_strict_antipode_threshold() {
        assert!(super::opposite_pose_reached(-1.0));
        assert!(super::opposite_pose_reached(-0.995));
        assert!(!super::opposite_pose_reached(-0.9949));
        assert!(!super::opposite_pose_reached(-0.98399));
        assert!(!super::opposite_pose_reached(f32::NAN));
    }

    #[test]
    fn failed_antipode_does_not_block_independent_pose_probes() {
        let directory = diagnostic_test_directory("transition-driver-initial-drag");
        let mut diagnostic = super::PlanetTransitionDiagnostic {
            directory: directory.clone(),
            ..default()
        };
        assert!(!super::drag_probe_ready(&diagnostic));
        assert_eq!(super::drag_progress_at(&diagnostic), None);

        diagnostic.drag_failed_at = Some(4.0);
        assert!(super::drag_probe_ready(&diagnostic));
        assert_eq!(super::drag_progress_at(&diagnostic), Some(4.0));

        diagnostic.opposite_pose_confirmed = true;
        diagnostic.opposite_pose_at = Some(3.5);
        assert!(super::drag_probe_ready(&diagnostic));
        assert_eq!(super::drag_progress_at(&diagnostic), Some(3.5));
    }

    #[test]
    fn waiting_rendered_pose_probe_preserves_the_active_antipode_drag_button() {
        let mut diagnostic = super::PlanetTransitionDiagnostic::default();
        let mut world = World::new();
        world.insert_resource(ButtonInput::<MouseButton>::default());
        world
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);

        super::drive_rendered_pose_probe(&mut world, &mut diagnostic, 2.5, true);

        assert!(
            world
                .resource::<ButtonInput<MouseButton>>()
                .pressed(MouseButton::Left)
        );
    }

    #[test]
    fn antipode_confirmation_timeout_is_bounded_and_only_applies_before_success() {
        assert!(!super::drag_confirmation_expired(Some(2.0), 3.999, false));
        assert!(super::drag_confirmation_expired(Some(2.0), 4.0, false));
        assert!(!super::drag_confirmation_expired(Some(2.0), 4.0, true));
        assert!(!super::drag_confirmation_expired(None, 100.0, false));
    }

    #[test]
    fn full_transition_driver_leaves_the_initial_drag_button_owned_until_release() {
        let (mut app, camera) = transition_driver_app();
        let start_direction = app
            .world()
            .get::<Transform>(camera)
            .expect("main camera transform")
            .translation
            .normalize();
        let started_at = app.world().resource::<Time<Real>>().elapsed_secs_f64();
        let drag_positions = [100.0, 160.0, 220.0, 280.0, 340.0, 400.0];
        let directory = diagnostic_test_directory("transition-driver-initial-drag");
        let mut diagnostic = super::PlanetTransitionDiagnostic {
            directory: directory.clone(),
            ..default()
        };
        diagnostic.started_at = Some(started_at);
        diagnostic.drag_started = true;
        diagnostic.drag_started_at = Some(0.0);
        diagnostic.drag_start_direction = start_direction;
        diagnostic.drag_frames = drag_positions
            .into_iter()
            .map(|y| super::super::diagnostic_drag::DragInputFrame {
                position: Vec2::new(1272.0, y),
                pressed: true,
            })
            .chain(std::iter::once(
                super::super::diagnostic_drag::DragInputFrame {
                    position: Vec2::new(1272.0, 400.0),
                    pressed: false,
                },
            ))
            .collect();
        app.insert_resource(diagnostic);

        for _ in 0..8 {
            app.update();
        }

        let diagnostic = app.world().resource::<super::PlanetTransitionDiagnostic>();
        assert!(
            diagnostic.drag_finished,
            "planned production drag did not release"
        );
        let end_direction = app
            .world()
            .get::<Transform>(camera)
            .expect("main camera transform")
            .translation
            .normalize();
        assert!(
            start_direction.angle_between(end_direction) > 0.01,
            "the full driver left the production camera fixed at {end_direction:?}"
        );
        std::fs::remove_dir_all(directory).expect("remove diagnostic output directory");
    }

    #[test]
    fn full_transition_driver_does_not_repress_a_rendered_drag_each_frame() {
        let (mut app, _) = transition_driver_app();
        let started_at = app.world().resource::<Time<Real>>().elapsed_secs_f64();
        let directory = diagnostic_test_directory("transition-driver-rendered-drag");
        let mut diagnostic = super::PlanetTransitionDiagnostic {
            directory: directory.clone(),
            ..default()
        };
        diagnostic.started_at = Some(started_at);
        diagnostic.drag_started = true;
        diagnostic.drag_finished = true;
        diagnostic.drag_finished_at = Some(started_at);
        diagnostic.drag_frames = vec![super::super::diagnostic_drag::DragInputFrame {
            position: Vec2::new(1272.0, 400.0),
            pressed: false,
        }];
        diagnostic.rendered_pose_stage = super::RenderedPoseStage::Dragging;
        diagnostic.rendered_pose_stage_started_at = Some(started_at);
        diagnostic.rendered_pose_drag_frames = vec![
            super::super::diagnostic_drag::DragInputFrame {
                position: Vec2::new(700.0, 350.0),
                pressed: true,
            },
            super::super::diagnostic_drag::DragInputFrame {
                position: Vec2::new(740.0, 370.0),
                pressed: true,
            },
        ];
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.insert_resource(diagnostic);

        app.update();

        let mouse = app.world().resource::<ButtonInput<MouseButton>>();
        assert!(mouse.pressed(MouseButton::Left));
        assert!(
            !mouse.just_released(MouseButton::Left),
            "the initial-drag owner released the rendered-pose button before pointer_input"
        );
        std::fs::remove_dir_all(directory).expect("remove diagnostic output directory");
    }

    #[test]
    fn opposite_pose_capture_is_requested_once_even_while_the_camera_holds_position() {
        let mut diagnostic = super::PlanetTransitionDiagnostic::default();
        diagnostic.directory = std::path::PathBuf::from("/tmp/planet-capture-latch-test");
        diagnostic.samples.push(super::DiagnosticSample {
            elapsed: 1.0,
            ..default()
        });
        let mut world = World::new();

        assert!(super::capture_once(
            &mut world,
            &mut diagnostic,
            "drag-opposite.png",
            1.0,
            |_| true,
        ));
        assert!(!super::capture_once(
            &mut world,
            &mut diagnostic,
            "drag-opposite.png",
            1.0,
            |_| true,
        ));
        let screenshot_count = world
            .query_filtered::<Entity, With<super::Screenshot>>()
            .iter(&world)
            .count();
        assert_eq!(screenshot_count, 1);
        assert_eq!(diagnostic.capture_metadata.len(), 1);
    }

    #[test]
    fn every_expected_diagnostic_screenshot_uses_one_request_per_filename() {
        let mut diagnostic = super::PlanetTransitionDiagnostic::default();
        diagnostic.directory = std::path::PathBuf::from("/tmp/planet-capture-set-test");
        let mut world = World::new();

        for name in super::DIAGNOSTIC_CAPTURE_NAMES {
            assert!(super::request_diagnostic_screenshot(
                &mut world,
                &mut diagnostic,
                name,
            ));
            assert!(!super::request_diagnostic_screenshot(
                &mut world,
                &mut diagnostic,
                name,
            ));
        }
        assert_eq!(
            diagnostic.requested_captures.len(),
            DIAGNOSTIC_CAPTURE_NAMES.len()
        );
        let screenshot_count = world
            .query_filtered::<Entity, With<super::Screenshot>>()
            .iter(&world)
            .count();
        assert_eq!(screenshot_count, DIAGNOSTIC_CAPTURE_NAMES.len());
    }

    #[test]
    fn diagnostic_capture_set_includes_each_moving_follow_vehicle_mode() {
        for capture in [
            "near-follow-on-foot-01.png",
            "near-follow-on-foot-02.png",
            "near-follow-car-01.png",
            "near-follow-car-02.png",
            "near-follow-plane-01.png",
            "near-follow-plane-02.png",
        ] {
            assert!(super::DIAGNOSTIC_CAPTURE_NAMES.contains(&capture));
        }
    }

    #[test]
    fn diagnostic_capture_set_includes_before_and_after_frames_for_each_drag_pose() {
        for capture in [
            "drag-far-oblique-before.png",
            "drag-far-oblique-after.png",
            "drag-mid-heading-before.png",
            "drag-mid-heading-after.png",
            "drag-near-polar-before.png",
            "drag-near-polar-after.png",
        ] {
            assert!(super::DIAGNOSTIC_CAPTURE_NAMES.contains(&capture));
        }
    }

    #[test]
    fn visual_capture_metadata_requires_exactly_one_row_for_each_expected_name() {
        let rows = super::DIAGNOSTIC_CAPTURE_NAMES
            .iter()
            .map(|name| format!("{},0.0\n", super::csv(name)))
            .collect::<Vec<_>>();
        assert!(super::capture_metadata_is_exactly_once(&rows));

        let mut duplicated = rows.clone();
        duplicated.push(rows[0].clone());
        assert!(!super::capture_metadata_is_exactly_once(&duplicated));
        assert!(!super::capture_metadata_is_exactly_once(&rows[1..]));

        let mut unexpected = rows;
        unexpected[0] = "unexpected.png,0.0\n".into();
        assert!(!super::capture_metadata_is_exactly_once(&unexpected));
    }

    #[test]
    fn return_reopen_waits_until_both_return_screenshots_are_saved() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time follows the Unix epoch")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "terra-planet-return-captures-{}-{unique}",
            std::process::id()
        ));
        let captures = directory.join("captures");
        std::fs::create_dir_all(&captures).expect("create temporary screenshot directory");
        let mut diagnostic = super::PlanetTransitionDiagnostic::default();
        diagnostic.directory = directory.clone();

        assert!(!super::return_captures_saved(&diagnostic));
        std::fs::write(captures.join("return-midpoint.png"), b"captured")
            .expect("write midpoint completion marker");
        assert!(!super::return_captures_saved(&diagnostic));
        std::fs::write(captures.join("return-end.png"), b"captured")
            .expect("write settled return completion marker");
        assert!(super::return_captures_saved(&diagnostic));

        std::fs::remove_dir_all(directory).expect("remove temporary screenshot directory");
    }

    #[derive(Resource)]
    struct InjectedCursorPath {
        frames: Vec<super::super::diagnostic_drag::DragInputFrame>,
        next: usize,
    }

    fn inject_cursor_path(
        mut path: ResMut<InjectedCursorPath>,
        mut windows: Query<&mut Window, With<PrimaryWindow>>,
        mut mouse: ResMut<ButtonInput<MouseButton>>,
    ) {
        let Ok(mut window) = windows.single_mut() else {
            return;
        };
        mouse.clear_just_pressed(MouseButton::Left);
        mouse.clear_just_released(MouseButton::Left);
        if let Some(frame) = path.frames.get(path.next).copied() {
            window.set_cursor_position(Some(frame.position));
            if frame.pressed {
                mouse.press(MouseButton::Left);
            } else {
                mouse.release(MouseButton::Left);
            }
            path.next += 1;
        } else {
            window.set_cursor_position(path.frames.last().map(|frame| frame.position));
            mouse.release(MouseButton::Left);
        }
    }

    fn production_pointer_drag_dot(
        frames: Vec<super::super::diagnostic_drag::DragInputFrame>,
        start_direction: Vec3,
    ) -> f32 {
        let end_direction =
            production_pointer_drag_end_direction(frames, start_direction, PLANET_VIEW_FAR_RADIUS);
        start_direction.normalize().dot(end_direction)
    }

    fn production_pointer_drag_end_direction(
        frames: Vec<super::super::diagnostic_drag::DragInputFrame>,
        start_direction: Vec3,
        camera_radius: f32,
    ) -> Vec3 {
        let (mut app, _) = crate::exploration::tests::fixture();
        app.world_mut().spawn((
            PrimaryWindow,
            Window {
                resolution: WindowResolution::new(2560, 1440).with_scale_factor_override(2.0),
                ..default()
            },
        ));
        let camera = app
            .world_mut()
            .spawn((
                MainCamera,
                Transform::from_xyz(0.0, 2005.0, -5.0)
                    .looking_at(Vec3::new(0.0, 2000.6, 0.0), Vec3::Y),
                Projection::Perspective(PerspectiveProjection::default()),
            ))
            .id();
        app.world_mut()
            .resource_mut::<Exploration>()
            .set_planet_view_open(true);
        app.world_mut()
            .resource_mut::<Exploration>()
            .request_planet_view_zoom((PLANET_VIEW_FAR_RADIUS / camera_radius).ln());
        for _ in 0..180 {
            app.update();
        }

        let start_direction = start_direction.normalize();
        let world_up = if start_direction.dot(Vec3::Y).abs() > 0.95 {
            Vec3::Z
        } else {
            Vec3::Y
        };
        let start_pose = Transform::from_translation(start_direction * camera_radius)
            .looking_at(Vec3::ZERO, world_up);
        app.world_mut()
            .resource_mut::<Exploration>()
            .toggle_planet_view_follow(start_pose);
        *app.world_mut()
            .entity_mut(camera)
            .get_mut::<Transform>()
            .expect("main camera transform") = start_pose;
        for _ in 0..180 {
            app.update();
        }
        let actual_start = app
            .world()
            .entity(camera)
            .get::<Transform>()
            .expect("main camera transform")
            .translation
            .normalize();
        assert!(
            actual_start.dot(start_direction) > 0.999,
            "fixture start camera was not seeded at the requested direction: {actual_start:?} vs {start_direction:?}"
        );

        app.insert_resource(InjectedCursorPath { frames, next: 0 });
        app.add_systems(
            PreUpdate,
            inject_cursor_path
                .after(InputSystems)
                .before(crate::exploration::ExplorationInput),
        );
        let path_frames = app.world().resource::<InjectedCursorPath>().frames.len() + 1;
        for _ in 0..path_frames {
            app.update();
        }
        for _ in 0..180 {
            app.update();
        }

        app.world()
            .entity(camera)
            .get::<Transform>()
            .expect("main camera transform")
            .translation
            .normalize()
    }

    #[test]
    fn rendered_surface_drag_reaches_oblique_and_polar_headings_across_zoom_levels() {
        let cases = [
            (
                PLANET_VIEW_FAR_RADIUS,
                Vec3::new(-0.67, -0.73, 0.08).normalize(),
                Vec3::new(0.25, 0.22, -0.94).normalize(),
            ),
            (
                (shared::planet_view::PLANET_VIEW_NEAR_RADIUS + PLANET_VIEW_FAR_RADIUS) * 0.5,
                Vec3::new(0.7, -0.2, -0.68).normalize(),
                Vec3::new(-0.52, 0.55, 0.65).normalize(),
            ),
            (
                shared::planet_view::PLANET_VIEW_NEAR_RADIUS + 250.0,
                Vec3::new(-0.6, 0.45, 0.66).normalize(),
                Vec3::Y,
            ),
        ];
        for (radius, start_direction, target_direction) in cases {
            let viewport = Vec2::new(1280.0, 720.0);
            let start_cursor = Vec2::new(710.0, 320.0);
            let up = if start_direction.dot(Vec3::Y).abs() > 0.95 {
                Vec3::Z
            } else {
                Vec3::Y
            };
            let start =
                Transform::from_translation(start_direction * radius).looking_at(Vec3::ZERO, up);
            let frames = super::super::diagnostic_drag::surface_drag_toward_direction(
                start,
                target_direction,
                start_cursor,
                viewport.x,
                viewport.y,
                PerspectiveProjection::default().fov,
            )
            .expect("viewport can contain the diagnostic surface drag");
            let actual_end = production_pointer_drag_end_direction(frames, start_direction, radius);
            let error_rad = actual_end.angle_between(target_direction);
            assert!(
                error_rad < 0.02,
                "production drag missed target by {error_rad:.4}rad at radius {radius:.1}, actual={actual_end:?}, target={target_direction:?}"
            );
        }
    }

    #[test]
    fn opposite_side_drag_reaches_antipode_through_scale_two_production_pointer_input() {
        let start_direction = Vec3::new(-0.6714, -0.7367, 0.0806).normalize();
        let frames = super::super::diagnostic_drag::opposite_side_drag(
            start_direction,
            1280.0,
            720.0,
            super::super::diagnostic_drag::DEFAULT_POINTS_PER_STROKE,
        )
        .expect("scale-two logical viewport can contain the released two-stroke route");
        let dot = production_pointer_drag_dot(frames, start_direction);

        assert!(
            dot < -0.995,
            "production pointer path ended at dot {dot} instead of the antipode"
        );
    }

    #[test]
    fn opposite_side_drag_reaches_antipode_with_coarse_stroke_sampling() {
        let start_direction = Vec3::new(-0.6714, -0.7367, 0.0806).normalize();
        let frames =
            super::super::diagnostic_drag::opposite_side_drag(start_direction, 1280.0, 720.0, 15)
                .expect("coarse samples should preserve both released strokes");
        let dot = production_pointer_drag_dot(frames, start_direction);

        assert!(
            dot < -0.995,
            "coarse production pointer path ended at dot {dot} instead of the antipode"
        );
    }

    #[test]
    fn diagnostic_drag_plan_uses_incremental_points_inside_the_window() {
        let width = 1280.0;
        let height = 720.0;
        let frames = super::super::diagnostic_drag::opposite_side_drag(
            Vec3::new(-0.6714, -0.7367, 0.0806),
            width,
            height,
            super::super::diagnostic_drag::DEFAULT_POINTS_PER_STROKE,
        )
        .expect("viewport can contain the planned orbit");

        assert!(frames.iter().all(|frame| {
            frame.position.x >= 8.0
                && frame.position.x <= width - 8.0
                && frame.position.y >= 8.0
                && frame.position.y <= height - 8.0
        }));
        let stroke_starts = frames
            .windows(2)
            .filter(|pair| pair[0].pressed && !pair[1].pressed)
            .count();
        assert_eq!(
            stroke_starts, 2,
            "each captured stroke has an explicit release"
        );
        assert!(frames.iter().all(|frame| frame.position.x == width - 8.0));
        let total_motion = frames
            .windows(2)
            .filter(|pair| pair[0].pressed && pair[1].pressed)
            .map(|pair| pair[0].position.distance(pair[1].position))
            .sum::<f32>();
        let gain = shared::planet_view::orbit_radians_per_logical_pixel(
            PLANET_VIEW_FAR_RADIUS,
            height,
            PerspectiveProjection::default().fov,
        );
        assert!((total_motion * gain - std::f32::consts::PI).abs() < 0.01);
    }

    #[test]
    fn diagnostic_waits_for_a_populated_stable_physics_world_before_starting_its_clock() {
        let mut warmup = DiagnosticWorldWarmup::default();
        assert!(!warmup.observe(0));
        assert!(!warmup.observe(DIAGNOSTIC_MINIMUM_COLLIDERS - 1));

        for _ in 0..DIAGNOSTIC_STABLE_WORLD_FRAMES - 1 {
            assert!(!warmup.observe(DIAGNOSTIC_MINIMUM_COLLIDERS));
        }
        assert!(warmup.observe(DIAGNOSTIC_MINIMUM_COLLIDERS));

        // Streaming that changes the collision world restarts the warmup.
        assert!(!warmup.observe(DIAGNOSTIC_MINIMUM_COLLIDERS + 1));
        for _ in 0..DIAGNOSTIC_STABLE_WORLD_FRAMES - 2 {
            assert!(!warmup.observe(DIAGNOSTIC_MINIMUM_COLLIDERS + 1));
        }
        assert!(warmup.observe(DIAGNOSTIC_MINIMUM_COLLIDERS + 1));
    }

    #[test]
    fn transition_action_waits_for_both_elapsed_gap_and_stage_readiness() {
        assert!(!transition_action_due(0.54, Some(0.0), 0.55, true));
        assert!(!transition_action_due(0.55, Some(0.0), 0.55, false));
        assert!(transition_action_due(0.55, Some(0.0), 0.55, true));
        assert!(!transition_action_due(0.55, None, 0.55, true));
    }

    #[test]
    fn radius_direction_uses_post_update_samples_when_preupdate_state_is_stale() {
        assert_eq!(
            radius_motion_flags(3_000.0, Some(2_800.0), Some(3_000.0)),
            (true, false)
        );
        assert_eq!(
            radius_motion_flags(3_000.0, Some(3_200.0), Some(3_000.0)),
            (false, true)
        );
    }
}
