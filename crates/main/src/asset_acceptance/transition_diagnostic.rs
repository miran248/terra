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
const DIAGNOSTIC_DRAG_DURATION_SECONDS: f64 = 1.1;
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
const FOLLOW_PROBE_NEAR_RADIUS_MARGIN_M: f32 = 900.0;
const DIAGNOSTIC_PARTIAL_VIEW_MIN_RADIUS_M: f32 = 2_050.0;
const DIAGNOSTIC_EXTERIOR_READY_RADIUS_M: f32 = 5_800.0;
const DIAGNOSTIC_RETURN_MIDPOINT_SECONDS: f64 = 1.2;
const DIAGNOSTIC_RETURN_END_SECONDS: f64 = 2.6;
const DIAGNOSTIC_CAPTURE_NAMES: [&str; 14] = [
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
];

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
    drag_finished: bool,
    drag_finished_at: Option<f64>,
    drag_start_direction: Vec3,
    drag_end_direction_dot: f32,
    opposite_pose_confirmed: bool,
    opposite_pose_at: Option<f64>,
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
struct DiagnosticWorldWarmup {
    last_collision_count: Option<usize>,
    stable_frames: u8,
}

impl DiagnosticWorldWarmup {
    fn observe(&mut self, collision_count: usize) -> bool {
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
    marker_visible: bool,
    registration_error_physical_px: f32,
}

impl FollowProbeSample {
    fn csv_row(self) -> String {
        format!(
            "{:.5},{},{},{},{:.4},{:.4},{:.4},{:.6},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{}\n",
            self.elapsed,
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
        "elapsed_s,mode,view_active,follow,requested_radius_m,attained_radius_m,body_speed_m_s,radial_lag_rad,projected_x_logical,projected_y_logical,marker_x_physical,marker_y_physical,marker_width_physical,marker_height_physical,registration_error_physical_px,marker_visible\n",
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
        drag_finished: false,
        drag_finished_at: None,
        drag_start_direction: Vec3::Y,
        drag_end_direction_dot: f32::NAN,
        opposite_pose_confirmed: false,
        opposite_pose_at: None,
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
                && diagnostic.opposite_pose_confirmed
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
        write_diagnostic_event(
            &mut diagnostic,
            elapsed,
            "production-pointer",
            "began released multi-stroke left-button drag after the exterior camera reached its full radius",
        );
    }
    if diagnostic.drag_started && !diagnostic.drag_finished {
        let drag_started_at = diagnostic
            .drag_started_at
            .expect("drag start time is set when drag begins");
        let drag_elapsed = elapsed - drag_started_at;
        if let Some(frames) = diagnostic_cursor_path(world, diagnostic.drag_start_direction) {
            if drag_elapsed < DIAGNOSTIC_DRAG_DURATION_SECONDS {
                let fraction =
                    (drag_elapsed / DIAGNOSTIC_DRAG_DURATION_SECONDS).clamp(0.0, 1.0) as f32;
                if let Some(frame) = sample_cursor_frame(&frames, fraction) {
                    set_primary_cursor(world, frame.position);
                    set_mouse_button(world, MouseButton::Left, frame.pressed);
                    if frame.pressed {
                        diagnostic.drag_input_frames =
                            diagnostic.drag_input_frames.saturating_add(1);
                    }
                }
            } else {
                if let Some(end) = frames.last() {
                    set_primary_cursor(world, end.position);
                }
                set_mouse_button(world, MouseButton::Left, false);
                diagnostic.drag_finished = true;
                diagnostic.drag_finished_at = Some(elapsed);
                let drag_input_frames = diagnostic.drag_input_frames;
                write_diagnostic_event(
                    &mut diagnostic,
                    elapsed,
                    "production-pointer",
                    format!(
                        "released drag after {} pressed frames; waiting for measured opposite pose",
                        drag_input_frames
                    ),
                );
            }
        }
    } else {
        set_mouse_button(world, MouseButton::Left, false);
    }

    if !diagnostic.movement_started
        && transition_action_due(
            elapsed,
            diagnostic.opposite_pose_at,
            DIAGNOSTIC_DRAG_READY_GAP_SECONDS,
            planet_active && diagnostic.opposite_pose_confirmed,
        )
    {
        diagnostic.movement_started = true;
        diagnostic.movement_started_at = Some(elapsed);
        write_diagnostic_event(
            &mut diagnostic,
            elapsed,
            "physics-probe",
            "began ordinary on-foot W with short steering input while Planet view is active at the opposite pose",
        );
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
        "open after warmup+{DIAGNOSTIC_INITIAL_OPEN_DELAY_SECONDS:.2}s; reverse after each prior tap+{DIAGNOSTIC_REVERSAL_MINIMUM_GAP_SECONDS:.2}s while transition is partial; detached on-foot movement for {DIAGNOSTIC_MOVEMENT_DURATION_SECONDS:.2}s; nearest-radius active-follow captures for on-foot, car, and plane; final return after these scenes; storm reopen after return+{DIAGNOSTIC_STORM_REOPEN_GAP_SECONDS:.2}s; close after {DIAGNOSTIC_STORM_CLOSE_GAP_SECONDS:.2}s hidden"
    );
    let entry_captures = "0.40,0.80,1.40 after initial M";
    let return_captures = "1.20,2.60 after final return M";
    let weather_fixture =
        "precipitation intensity 0.85 with moving wind; production particle visibility";
    let weather_captures = "storm-hidden.png,storm-restored.png";
    let text = format!(
        "mode={}\nacceptance_claim=none\nsource_revision={}\nsource_branch={}\nasset_root={}\ncargo_target_dir={}\npackage_version={}\nwindow={}\ncapture_target={}\nbackground_capture_is_performance_comparable=false\nprewarm_minimum_physics_colliders={DIAGNOSTIC_MINIMUM_COLLIDERS}\nprewarm_stable_world_frames={DIAGNOSTIC_STABLE_WORLD_FRAMES}\nM_action_schedule={}\nentry_screenshots={}\nreturn_screenshots={}\nreleased_multi_stroke_drag={DIAGNOSTIC_DRAG_DURATION_SECONDS:.2}s after exterior-ready radius {DIAGNOSTIC_EXTERIOR_READY_RADIUS_M:.0}m\ndrag_path={drag}\non_foot_movement={DIAGNOSTIC_MOVEMENT_DURATION_SECONDS:.2}s after confirmed opposite pose\nnear_follow_probe=on-foot,car,plane; nearest requested radius {PLANET_VIEW_NEAR_RADIUS:.1}m; two captures 250ms apart after attained radius is within {FOLLOW_PROBE_NEAR_RADIUS_MARGIN_M:.1}m\nroute_timeout_s={DIAGNOSTIC_ROUTE_TIMEOUT_SECONDS:.0}\nweather_fixture={}\nweather_screenshots={}\ncapture_metadata=target_vs_actual_request_time_and_camera_pose\n",
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

fn sample_cursor_frame(
    frames: &[super::diagnostic_drag::DragInputFrame],
    fraction: f32,
) -> Option<super::diagnostic_drag::DragInputFrame> {
    super::diagnostic_drag::sample_drag_frame(frames, fraction)
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

    let target_reached = ready
        && requested <= PLANET_VIEW_NEAR_RADIUS + 1.0
        && attained <= PLANET_VIEW_NEAR_RADIUS + FOLLOW_PROBE_NEAR_RADIUS_MARGIN_M;
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
        diagnostic
            .follow_probe_samples
            .push(follow_probe_sample(world, elapsed, mode));
    }
    if diagnostic.drag_finished && !diagnostic.opposite_pose_confirmed && sample.planet_active {
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
        && sample.attained_radius <= PLANET_VIEW_NEAR_RADIUS + FOLLOW_PROBE_NEAR_RADIUS_MARGIN_M
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
    let radial_lag_rad = camera_pose
        .translation
        .normalize_or(Vec3::Y)
        .dot(body_position.normalize_or(Vec3::Y))
        .clamp(-1.0, 1.0)
        .acos();
    let projected_center_logical = {
        let mut cameras = world.query_filtered::<(&Camera, &GlobalTransform), With<MainCamera>>();
        cameras
            .iter(world)
            .find_map(|(camera, transform)| camera.world_to_viewport(transform, body_position).ok())
            .unwrap_or(Vec2::splat(f32::NAN))
    };
    let scale_factor = primary_window(world).map_or(f32::NAN, |window| window.scale_factor());
    let marker = crate::planet_markers::explorer_marker_layout(world);
    let marker_center_physical =
        marker.map_or(Vec2::splat(f32::NAN), |layout| layout.physical_center);
    let marker_size_physical = marker.map_or(Vec2::splat(f32::NAN), |layout| layout.physical_size);
    let marker_visible = marker.is_some_and(|layout| layout.visible);
    let expected_center_physical = projected_center_logical * scale_factor;
    let registration_error_physical_px = marker
        .map(|layout| layout.physical_center.distance(expected_center_physical))
        .unwrap_or(f32::NAN);
    FollowProbeSample {
        elapsed,
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
        marker_visible,
        registration_error_physical_px,
    }
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
        "elapsed_s,mode,view_active,follow,requested_radius_m,attained_radius_m,body_speed_m_s,radial_lag_rad,projected_x_logical,projected_y_logical,marker_x_physical,marker_y_physical,marker_width_physical,marker_height_physical,registration_error_physical_px,marker_visible\n{}",
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
    let on_foot_follow_passed =
        follow_probe_mode_passed(&diagnostic.follow_probe_samples, FollowProbeMode::OnFoot);
    let car_follow_passed =
        follow_probe_mode_passed(&diagnostic.follow_probe_samples, FollowProbeMode::Car);
    let plane_follow_passed =
        follow_probe_mode_passed(&diagnostic.follow_probe_samples, FollowProbeMode::Plane);
    let follow_probes_passed = on_foot_follow_passed && car_follow_passed && plane_follow_passed;
    let max_follow_radial_lag_rad = diagnostic
        .follow_probe_samples
        .iter()
        .filter(|sample| sample.mode.is_some() && sample.radial_lag_rad.is_finite())
        .filter(|sample| {
            sample.view_active
                && sample.follows
                && sample.requested_radius <= PLANET_VIEW_NEAR_RADIUS + 1.0
        })
        .map(|sample| sample.radial_lag_rad)
        .fold(0.0_f32, f32::max);
    let max_registration_error_physical_px = diagnostic
        .follow_probe_samples
        .iter()
        .filter(|sample| sample.mode.is_some() && sample.registration_error_physical_px.is_finite())
        .filter(|sample| {
            sample.view_active
                && sample.follows
                && sample.requested_radius <= PLANET_VIEW_NEAR_RADIUS + 1.0
        })
        .map(|sample| sample.registration_error_physical_px)
        .fold(0.0_f32, f32::max);
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
        && movement_passed
        && follow_probes_passed
        && weather_hidden
        && weather_restored;
    let status = if passed {
        "diagnostic-complete"
    } else {
        "diagnostic-rejected"
    };
    let report = format!(
        "mode=visual-diagnostic\nacceptance_claim=none\nstatus={status}\nroute_timed_out={}\nm_events_sent={}\nm_event_elapsed_s={m_event_elapsed_s}\nroute_complete={route_complete}\ndrag_started_s={}\ndrag_released_s={}\nopposite_pose_s={}\ndrag_input_frames={}\ndrag_opposite_dot={:.5}\nbody_path_m={:.4}\nmax_body_excursion_m={:.4}\nmovement_started_s={}\nmovement_finished_s={}\nfixed_time_advanced_during_movement={fixed_advanced}\nphysics_live_during_movement={physics_live}\ncollision_world_live_during_movement={}\nphysics_advanced_while_moving={physics_advanced_while_moving}\nmovement_passed={movement_passed}\nnear_follow_on_foot_passed={on_foot_follow_passed}\nnear_follow_car_passed={car_follow_passed}\nnear_follow_plane_passed={plane_follow_passed}\nnear_follow_sample_count={}\nmax_follow_radial_lag_rad={max_follow_radial_lag_rad:.6}\nmax_marker_registration_error_physical_px={max_registration_error_physical_px:.3}\nentry_capture_lateness_s={:.5},{:.5},{:.5}\nentry_captures_timely={entry_captures_timely}\ncapture_metadata_rows={}\ncapture_metadata_valid={capture_metadata_valid}\nweather_kind_at_player={}\nweather_hidden_while_planet_active={weather_hidden}\nweather_restored_after_return={weather_restored}\nmissing_captures={}\nerrors={}\n",
        diagnostic.route_timed_out,
        diagnostic
            .m_events_sent
            .iter()
            .filter(|sent| **sent)
            .count(),
        diagnostic
            .drag_started_at
            .map_or_else(|| "missing".to_owned(), |at| format!("{at:.5}")),
        diagnostic
            .drag_finished_at
            .map_or_else(|| "missing".to_owned(), |at| format!("{at:.5}")),
        diagnostic
            .opposite_pose_at
            .map_or_else(|| "missing".to_owned(), |at| format!("{at:.5}")),
        diagnostic.drag_input_frames,
        diagnostic.drag_end_direction_dot,
        movement.traveled_m,
        movement.max_excursion_m,
        diagnostic
            .movement_started_at
            .map_or_else(|| "missing".to_owned(), |at| format!("{at:.5}")),
        diagnostic
            .movement_finished_at
            .map_or_else(|| "missing".to_owned(), |at| format!("{at:.5}")),
        motion_samples
            .iter()
            .any(|sample| sample.collision_colliders > 0),
        diagnostic.entry_capture_lateness_s[0],
        diagnostic.entry_capture_lateness_s[1],
        diagnostic.entry_capture_lateness_s[2],
        diagnostic.follow_probe_samples.len(),
        diagnostic.capture_metadata.len(),
        diagnostic
            .samples
            .iter()
            .find(|sample| sample.weather_intensity > 0.2)
            .map_or("unknown", |sample| sample.weather_kind),
        missing_captures.join(";"),
        diagnostic.errors.join(";"),
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

fn follow_probe_mode_passed(samples: &[FollowProbeSample], mode: FollowProbeMode) -> bool {
    samples.iter().any(|sample| {
        sample.mode == Some(mode)
            && sample.view_active
            && sample.follows
            && sample.requested_radius <= PLANET_VIEW_NEAR_RADIUS + 1.0
            && sample.attained_radius <= PLANET_VIEW_NEAR_RADIUS + FOLLOW_PROBE_NEAR_RADIUS_MARGIN_M
            && sample.body_speed_m_s >= 0.5
            && sample.radial_lag_rad.is_finite()
            && sample.radial_lag_rad <= 0.01
            && sample.marker_visible
            && sample.registration_error_physical_px.is_finite()
            && sample.registration_error_physical_px <= 4.0
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
        DiagnosticWorldWarmup, radius_motion_flags, transition_action_due,
    };
    use bevy::{
        input::InputSystems,
        prelude::*,
        window::{PrimaryWindow, WindowResolution},
    };
    use shared::planet_view::PLANET_VIEW_FAR_RADIUS;

    use crate::{exploration::Exploration, map::MainCamera};

    #[test]
    fn opposite_pose_measurement_requires_the_strict_antipode_threshold() {
        assert!(super::opposite_pose_reached(-1.0));
        assert!(super::opposite_pose_reached(-0.995));
        assert!(!super::opposite_pose_reached(-0.9949));
        assert!(!super::opposite_pose_reached(-0.98399));
        assert!(!super::opposite_pose_reached(f32::NAN));
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

        let start_direction = start_direction.normalize();
        let start_pose = Transform::from_translation(start_direction * PLANET_VIEW_FAR_RADIUS)
            .looking_at(Vec3::ZERO, Vec3::Y);
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
        assert!(actual_start.dot(start_direction) > 0.999);

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

        let actual_end = app
            .world()
            .entity(camera)
            .get::<Transform>()
            .expect("main camera transform")
            .translation
            .normalize();
        actual_start.dot(actual_end)
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
