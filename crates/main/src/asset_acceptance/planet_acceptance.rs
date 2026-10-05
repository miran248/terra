//! Opt-in live Planet view acceptance run on the primary window.
//!
//! Unlike the frozen lighting and short production probes, this route leaves
//! the ordinary real/virtual clocks, physics, streaming, animation and camera
//! systems in charge. Only user intents are scripted here.

use crate::{
    exploration::{Action, Exploration, Kind, PlanetTeleportOutcomeKind},
    map::{CollisionTerrain, MainCamera, Player, Sun, SunLock, TimeOfDay},
    planet_time::PlanetSimulationClock,
};
use avian3d::prelude::{Collider, LinearVelocity, Position, Sleeping};
use bevy::{
    camera::CameraUpdateSystems,
    input::InputSystems,
    prelude::*,
    render::view::screenshot::{Screenshot, save_to_disk},
    window::PrimaryWindow,
};
use shared::{
    level::LevelData, planet_view::PLANET_VIEW_FAR_RADIUS, sphere::PLANET_RADIUS, state::AppState,
};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    num::NonZeroU8,
    path::{Path, PathBuf},
};

const CAPTURE_VIEWS: [CaptureView; 4] = [
    CaptureView::Ground,
    CaptureView::Settlement,
    CaptureView::Globe,
    CaptureView::Opposite,
];
const SOLAR_PHASES: [SolarPhase; 3] = [SolarPhase::Noon, SolarPhase::Sunset, SolarPhase::Night];
const ROUTES: [Route; 4] = [
    Route::EntryReversal,
    Route::OrbitZoom,
    Route::FollowVehicleRecovery,
    Route::ReturnReversal,
];
const WARMUP_SECONDS: f64 = 60.0;
const MEASURE_SECONDS: f64 = 60.0;
const REPEATS: u8 = 3;
const WORK_TRACE_ENV: &str = "TERRA_PLANET_WORK_TRACE";
const STALL_LIMIT_MS: f64 = 1000.0 / 30.0;
pub(super) const MIN_MEASURED_BODY_PATH_M: f64 = 10.0;
pub(super) const MIN_MEASURED_BODY_EXCURSION_M: f64 = 5.0;
pub(super) const MAX_CONTINUOUS_BODY_STEP_M: f32 = 12.0;
const ORBIT_RADIANS_PER_LOGICAL_PIXEL: f32 = 0.004;
const SUN_TILT: f32 = 0.35;
const MOVEMENT_TURN_SECONDS: f64 = 0.65;
const MOVEMENT_CYCLE_SECONDS: f64 = 12.0;
const ORBIT_INPUTS: [(f64, f32, Vec2); 5] = [
    (3.0, 2.4, Vec2::ZERO),
    (9.0, -2.1, Vec2::new(90.0, 16.0)),
    (17.0, 1.4, Vec2::new(-48.0, -24.0)),
    (28.0, -1.2, Vec2::ZERO),
    (38.0, 0.8, Vec2::new(60.0, 0.0)),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CaptureView {
    Ground,
    Settlement,
    Globe,
    Opposite,
}

impl CaptureView {
    fn name(self) -> &'static str {
        match self {
            Self::Ground => "ground",
            Self::Settlement => "settlement",
            Self::Globe => "globe",
            Self::Opposite => "opposite",
        }
    }

    fn radius(self) -> f32 {
        match self {
            Self::Ground => 0.0,
            Self::Settlement => 2_250.0,
            Self::Globe | Self::Opposite => PLANET_VIEW_FAR_RADIUS,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SolarPhase {
    Noon,
    Sunset,
    Night,
}

impl SolarPhase {
    fn name(self) -> &'static str {
        match self {
            Self::Noon => "noon",
            Self::Sunset => "sunset",
            Self::Night => "night",
        }
    }

    fn elevation_degrees(self, anchor: Vec3) -> f32 {
        match self {
            Self::Noon => maximum_sun_elevation_degrees(anchor),
            Self::Sunset => 0.0,
            Self::Night => -18.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Route {
    EntryReversal,
    OrbitZoom,
    FollowVehicleRecovery,
    ReturnReversal,
}

impl Route {
    fn name(self) -> &'static str {
        match self {
            Self::EntryReversal => "entry-reversal",
            Self::OrbitZoom => "orbit-zoom",
            Self::FollowVehicleRecovery => "follow-vehicle-recovery",
            Self::ReturnReversal => "return-reversal",
        }
    }
}

#[derive(Resource)]
struct PlanetAcceptance {
    directory: PathBuf,
    phase: Phase,
    errors: Vec<String>,
    counts: ResidentCounts,
    counts_at: f64,
    measured_stalls: Vec<(Route, u8, usize)>,
    event_log: Vec<String>,
    current_route: Option<Route>,
    trace_flush_route: Option<Route>,
    coverage: CrossFeatureCoverage,
}

#[derive(Default)]
struct CrossFeatureCoverage {
    destination_selected: bool,
    queued_teleport_cancelled_on_close: bool,
    teleport_request_submitted: bool,
    teleport_outcome_observed: bool,
    selector_opened: bool,
    selector_priority_when_open: bool,
    selector_closed_unpaused: bool,
    selector_blocked_during_planet_view: bool,
    selector_no_deferred_open_after_close: bool,
    base_rate_changed_and_restored: bool,
    vehicle_entered: bool,
    vehicle_recovered: bool,
    vehicle_exited: bool,
    body_moved_during_planet_view: bool,
    collision_world_live_during_planet_movement: bool,
}

enum Phase {
    WaitingForWorld,
    Capturing {
        index: usize,
        prepared: bool,
        prepared_at: f64,
        screenshot_at: Option<f64>,
    },
    Warmup {
        route: Route,
        started_at: f64,
        actions: RouteActions,
    },
    Measuring {
        route: Route,
        repeat: u8,
        started_at: f64,
        actions: RouteActions,
        samples: Vec<FrameSample>,
    },
    Finishing {
        started_at: f64,
    },
    Done,
}

#[derive(Default)]
struct RouteActions {
    selection_attempted: bool,
    selection_retried: bool,
    cancellation_queued: bool,
    confirmation_queued: bool,
    selection_event_recorded: bool,
    teleport_submission_observed: bool,
    confirmation_outcome_recorded: bool,
    selector_view_probe_sent: bool,
    selector_view_block_observed: bool,
    selector_view_close_observed: bool,
    base_rate_changed: bool,
    base_rate_restored: bool,
    vehicle_selector_open_requested: bool,
    vehicle_selector_open_observed: bool,
    vehicle_selector_probe_sent: bool,
    vehicle_selector_priority_observed: bool,
    vehicle_selector_closed_observed: bool,
    vehicle_summon_requested: bool,
    vehicle_summoned: bool,
    vehicle_interaction_requested: bool,
    vehicle_recovery_requested: bool,
    vehicle_exit_requested: bool,
    vehicle_entered_observed: bool,
    vehicle_exit_observed: bool,
    vehicle_recovery_observed: bool,
    vehicle_failure_recorded: bool,
    last_interaction_at: f64,
    last_exit_interaction_at: f64,
    orbit_inputs_sent: [bool; 5],
    orbit_follow_toggled: bool,
}

#[derive(Clone, Copy, Default)]
struct ResidentCounts {
    meshes: usize,
    visible_meshes: usize,
    colliders: usize,
}

#[derive(Clone, Copy)]
struct Motion {
    position: Vec3,
    velocity: Vec3,
}

impl Motion {
    fn missing() -> Self {
        Self {
            position: Vec3::splat(f32::NAN),
            velocity: Vec3::splat(f32::NAN),
        }
    }
}

#[derive(Clone, Copy)]
struct FrameSample {
    real_elapsed: f64,
    interval_ms: f64,
    view_active: bool,
    view_ready: bool,
    follows: bool,
    requested_radius: f32,
    attained_radius: f32,
    camera_radius: f32,
    player: Motion,
    car: Motion,
    plane: Motion,
    controlled: Motion,
    controlled_kind: &'static str,
    controlled_sleeping: bool,
    fixed_time_elapsed: f64,
    body_clearance: f32,
    virtual_rate: f32,
    virtual_paused: bool,
    sun_angle: f32,
    counts: ResidentCounts,
    counts_age: f64,
}

#[derive(Default)]
pub(super) struct BodyPathSummary {
    pub(super) traveled_m: f64,
    pub(super) max_excursion_m: f64,
}

pub(super) fn continuous_body_path<'a>(
    points: impl IntoIterator<Item = Option<(&'a str, Vec3)>>,
) -> BodyPathSummary {
    let mut summary = BodyPathSummary::default();
    let mut segment_kind: Option<&str> = None;
    let mut segment_start = None;
    let mut previous = None;

    for point in points {
        let Some((kind, position)) = point else {
            segment_kind = None;
            segment_start = None;
            previous = None;
            continue;
        };
        if !position.is_finite() {
            segment_kind = None;
            segment_start = None;
            previous = None;
            continue;
        }
        if segment_kind != Some(kind) {
            segment_kind = Some(kind);
            segment_start = Some(position);
            previous = Some(position);
            continue;
        }

        let step = previous.map_or(f32::INFINITY, |last: Vec3| last.distance(position));
        if step > MAX_CONTINUOUS_BODY_STEP_M {
            // Teleports and vehicle handoffs remain present in raw samples but
            // cannot satisfy the continuous-motion requirement.
            segment_start = Some(position);
            previous = Some(position);
            continue;
        }

        summary.traveled_m += f64::from(step);
        if let Some(start) = segment_start {
            summary.max_excursion_m = summary
                .max_excursion_m
                .max(f64::from(start.distance(position)));
        }
        previous = Some(position);
    }
    summary
}

fn path_from_samples(samples: &[FrameSample], only_while_view_active: bool) -> BodyPathSummary {
    continuous_body_path(samples.iter().map(|sample| {
        if only_while_view_active && !sample.view_active {
            None
        } else {
            Some((sample.controlled_kind, sample.controlled.position))
        }
    }))
}

fn physics_advanced_while_moving(samples: &[FrameSample], only_while_view_active: bool) -> bool {
    samples.windows(2).any(|pair| {
        let [before, after] = pair else {
            unreachable!("a two-sample window has exactly two entries")
        };
        let displacement = before
            .controlled
            .position
            .distance(after.controlled.position);
        before.controlled_kind == after.controlled_kind
            && displacement > 0.001
            && displacement <= MAX_CONTINUOUS_BODY_STEP_M
            && after.fixed_time_elapsed > before.fixed_time_elapsed
            && after.body_clearance.is_finite()
            && after.counts.colliders > 0
            && !after.controlled_sleeping
            && (!only_while_view_active || before.view_active && after.view_active)
    })
}

pub(super) fn register(app: &mut App, directory: PathBuf) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("main crate lives below the workspace root")
        .canonicalize()
        .expect("canonicalize workspace root");
    let directory = directory
        .canonicalize()
        .expect("Planet acceptance output directory must already exist");
    assert!(
        directory.is_dir(),
        "Planet acceptance output must be a directory"
    );
    assert!(
        !directory.starts_with(root),
        "Planet acceptance output must be outside the checkout"
    );
    assert!(
        !directory.join("captures").exists()
            && !directory.join("performance").exists()
            && !directory.join("capture-manifest.csv").exists()
            && !directory.join("performance-summary.csv").exists(),
        "Planet acceptance output must be fresh"
    );

    if work_trace_enabled() {
        app.insert_resource(crate::chunks::PlanetWorkTrace::default());
    }
    app.insert_resource(PlanetAcceptance {
        directory,
        phase: Phase::WaitingForWorld,
        errors: Vec::new(),
        counts: ResidentCounts::default(),
        counts_at: 0.0,
        measured_stalls: Vec::new(),
        event_log: Vec::new(),
        current_route: None,
        trace_flush_route: None,
        coverage: CrossFeatureCoverage::default(),
    })
    .add_systems(
        PreUpdate,
        drive_planet_acceptance
            .after(InputSystems)
            .before(crate::exploration::ExplorationInput)
            .run_if(in_state(AppState::Playing)),
    )
    .add_systems(
        PostUpdate,
        capture_and_measure_planet_acceptance
            .after(CameraUpdateSystems)
            .run_if(in_state(AppState::Playing)),
    );
}

fn work_trace_enabled() -> bool {
    std::env::var_os(WORK_TRACE_ENV).is_some()
}

fn trace_sidecars_enabled() -> bool {
    work_trace_enabled() || super::phase_trace::enabled()
}

fn start_measurement_traces(world: &mut World, elapsed_s: f64, route: &'static str, repeat: u8) {
    if work_trace_enabled()
        && let Some(mut probe) = world.get_resource_mut::<crate::chunks::PlanetWorkTrace>()
    {
        probe.start_repeat(elapsed_s, route, repeat);
    }
    #[cfg(feature = "asset-review-schedule-trace")]
    if super::phase_trace::enabled()
        && let Some(probe) = world.get_resource::<super::phase_trace::ScheduleTraceRecorder>()
    {
        probe.start_repeat(route, repeat);
    }
}

fn stop_measurement_traces(world: &mut World, route: &'static str, repeat: u8) {
    #[cfg(not(feature = "asset-review-schedule-trace"))]
    let _ = (route, repeat);
    if work_trace_enabled()
        && let Some(mut probe) = world.get_resource_mut::<crate::chunks::PlanetWorkTrace>()
    {
        probe.stop_repeat();
    }
    #[cfg(feature = "asset-review-schedule-trace")]
    if super::phase_trace::enabled()
        && let Some(probe) = world.get_resource::<super::phase_trace::ScheduleTraceRecorder>()
    {
        probe.stop_repeat(route, repeat);
    }
}

fn drive_planet_acceptance(world: &mut World) {
    if !acceptance_world_ready(world) {
        return;
    }
    let now = world.resource::<Time<Real>>().elapsed_secs_f64();
    let Some(mut run) = world.remove_resource::<PlanetAcceptance>() else {
        return;
    };
    if let Some(route) = run.trace_flush_route.take() {
        flush_route_trace_sidecars(world, &mut run, route);
    }

    let phase = std::mem::replace(&mut run.phase, Phase::WaitingForWorld);
    run.phase = match phase {
        Phase::WaitingForWorld => {
            initialize_output(world, &mut run);
            info!("Planet acceptance primary-window run started");
            Phase::Capturing {
                index: 0,
                prepared: false,
                prepared_at: now,
                screenshot_at: None,
            }
        }
        Phase::Capturing {
            index,
            mut prepared,
            mut prepared_at,
            screenshot_at,
        } => {
            if index < CAPTURE_VIEWS.len() * SOLAR_PHASES.len() {
                if !prepared {
                    prepare_capture(world, index);
                    prepared = true;
                    prepared_at = now;
                    info!(
                        shot = capture_name(index),
                        "preparing matched Planet view capture"
                    );
                }
                hold_capture_sun(world, index);
                Phase::Capturing {
                    index,
                    prepared,
                    prepared_at,
                    screenshot_at,
                }
            } else {
                info!(
                    route = ROUTES[0].name(),
                    "Planet acceptance warm-up started"
                );
                Phase::Warmup {
                    route: ROUTES[0],
                    started_at: now,
                    actions: RouteActions::default(),
                }
            }
        }
        Phase::Warmup {
            route,
            started_at,
            mut actions,
        } => {
            let elapsed = now - started_at;
            drive_route(world, route, elapsed, true, &mut actions, &mut run);
            if elapsed >= WARMUP_SECONDS {
                info!(route = route.name(), "Planet acceptance warm-up completed");
                let measuring_actions = RouteActions::default();
                start_measurement_traces(world, now, route.name(), 1);
                Phase::Measuring {
                    route,
                    repeat: 1,
                    started_at: now,
                    actions: measuring_actions,
                    samples: Vec::with_capacity(4_000),
                }
            } else {
                Phase::Warmup {
                    route,
                    started_at,
                    actions,
                }
            }
        }
        Phase::Measuring {
            route,
            repeat,
            started_at,
            mut actions,
            samples,
        } => {
            let elapsed = now - started_at;
            drive_route(world, route, elapsed, false, &mut actions, &mut run);
            if elapsed >= MEASURE_SECONDS {
                finish_repeat(world, &mut run, route, repeat, &samples);
                stop_measurement_traces(world, route.name(), repeat);
                if repeat < REPEATS {
                    let next_actions = RouteActions::default();
                    start_measurement_traces(world, now, route.name(), repeat + 1);
                    Phase::Measuring {
                        route,
                        repeat: repeat + 1,
                        started_at: now,
                        actions: next_actions,
                        samples: Vec::with_capacity(4_000),
                    }
                } else if let Some(next) = ROUTES.get(route_index(route) + 1).copied() {
                    if trace_sidecars_enabled() {
                        run.trace_flush_route = Some(route);
                    }
                    info!(route = next.name(), "Planet acceptance warm-up started");
                    Phase::Warmup {
                        route: next,
                        started_at: now,
                        actions: RouteActions::default(),
                    }
                } else {
                    if trace_sidecars_enabled() {
                        run.trace_flush_route = Some(route);
                    }
                    Phase::Finishing { started_at: now }
                }
            } else {
                Phase::Measuring {
                    route,
                    repeat,
                    started_at,
                    actions,
                    samples,
                }
            }
        }
        Phase::Finishing { started_at } => {
            if now - started_at >= 2.0 {
                finish_run(world, &mut run);
                Phase::Done
            } else {
                Phase::Finishing { started_at }
            }
        }
        Phase::Done => Phase::Done,
    };

    world.insert_resource(run);
}

fn capture_and_measure_planet_acceptance(world: &mut World) {
    let now = world.resource::<Time<Real>>().elapsed_secs_f64();
    let Some(mut run) = world.remove_resource::<PlanetAcceptance>() else {
        return;
    };

    let phase = std::mem::replace(&mut run.phase, Phase::WaitingForWorld);
    run.phase = match phase {
        Phase::Capturing {
            index,
            prepared,
            prepared_at,
            mut screenshot_at,
        } if index < CAPTURE_VIEWS.len() * SOLAR_PHASES.len() => {
            let spec = capture_spec(index);
            if !prepared {
                Phase::Capturing {
                    index,
                    prepared,
                    prepared_at,
                    screenshot_at,
                }
            } else if let Some(requested_at) = screenshot_at {
                let path = run.directory.join("captures").join(capture_file(index));
                if file_is_nonempty(&path) && now - requested_at >= 0.25 {
                    append_capture_row(world, &mut run, index, &path);
                    info!(
                        shot = capture_name(index),
                        "Planet acceptance capture saved"
                    );
                    Phase::Capturing {
                        index: index + 1,
                        prepared: false,
                        prepared_at: now,
                        screenshot_at: None,
                    }
                } else if now - requested_at > 5.0 {
                    record_error(
                        &mut run,
                        format!("capture write did not complete: {}", path.display()),
                    );
                    Phase::Capturing {
                        index: index + 1,
                        prepared: false,
                        prepared_at: now,
                        screenshot_at: None,
                    }
                } else {
                    Phase::Capturing {
                        index,
                        prepared,
                        prepared_at,
                        screenshot_at,
                    }
                }
            } else if now - prepared_at >= 2.0 && capture_ready(world, spec) {
                let path = run.directory.join("captures").join(capture_file(index));
                world
                    .spawn(Screenshot::primary_window())
                    .observe(save_to_disk(path.clone()));
                screenshot_at = Some(now);
                info!(shot = capture_name(index), path = %path.display(), "capturing primary window");
                Phase::Capturing {
                    index,
                    prepared,
                    prepared_at,
                    screenshot_at,
                }
            } else {
                Phase::Capturing {
                    index,
                    prepared,
                    prepared_at,
                    screenshot_at,
                }
            }
        }
        Phase::Capturing { .. } => {
            info!(
                route = ROUTES[0].name(),
                "Planet acceptance warm-up started"
            );
            Phase::Warmup {
                route: ROUTES[0],
                started_at: now,
                actions: RouteActions::default(),
            }
        }
        Phase::Measuring {
            route,
            repeat,
            started_at,
            actions,
            mut samples,
        } => {
            if now - run.counts_at >= 1.0 || run.counts_at == 0.0 {
                run.counts = sample_resident_counts(world);
                run.counts_at = now;
            }
            samples.push(sample_frame(
                world,
                route,
                started_at,
                run.counts_at,
                run.counts,
            ));
            Phase::Measuring {
                route,
                repeat,
                started_at,
                actions,
                samples,
            }
        }
        other => other,
    };

    world.insert_resource(run);
}

pub(super) fn acceptance_world_ready(world: &mut World) -> bool {
    world
        .get_resource::<State<AppState>>()
        .is_some_and(|state| *state.get() == AppState::Playing)
        && world
            .query_filtered::<Entity, With<MainCamera>>()
            .iter(world)
            .next()
            .is_some()
        && world
            .query_filtered::<Entity, With<Player>>()
            .iter(world)
            .next()
            .is_some()
        && world
            .query_filtered::<Entity, With<PrimaryWindow>>()
            .iter(world)
            .next()
            .is_some()
}

pub(super) fn count_collision_colliders(world: &mut World) -> usize {
    world
        .query_filtered::<Entity, With<Collider>>()
        .iter(world)
        .count()
}

fn initialize_output(world: &mut World, run: &mut PlanetAcceptance) {
    for directory in ["captures", "performance"] {
        if let Err(error) = fs::create_dir_all(run.directory.join(directory)) {
            record_error(run, format!("create {directory} directory: {error}"));
        }
    }
    for (path, header) in [
        (
            "capture-manifest.csv",
            "file,view,solar_phase,anchor,seed,physical_width,physical_height,logical_width,logical_height,scale_factor,present_mode,camera_x,camera_y,camera_z,camera_qx,camera_qy,camera_qz,camera_qw,camera_radius_m,requested_radius_m,attained_radius_m,body_x,body_y,body_z,sun_x,sun_y,sun_z,local_sun_elevation_deg,ambient_brightness,visible_meshes,resident_meshes,resident_colliders,light_illuminance,shadows_enabled,fog\n",
        ),
        (
            "performance-summary.csv",
            "route,repeat,sample_count,p50_ms,p95_ms,p99_ms,max_ms,over_33_33ms,body_displacement_m,body_path_m,max_body_excursion_m,body_speed_mean_mps,physics_live_samples,physics_sleeping_samples,fixed_time_span_s,simulation_advancing_samples,sun_angle_span_rad,minimum_clearance_m,mesh_min,mesh_max,visible_mesh_min,visible_mesh_max,collider_min,collider_max,sun_angle_start_rad,sun_angle_end_rad\n",
        ),
        (
            "cross-feature-events.csv",
            "route_elapsed_s,route,event,result\n",
        ),
    ] {
        if let Err(error) = fs::write(run.directory.join(path), header) {
            record_error(run, format!("initialize {path}: {error}"));
        }
    }
    let seed = LevelData::from_artifact_bytes(include_bytes!("../../assets/level_1337.bin"))
        .map(|level| level.seed)
        .unwrap_or_default();
    let window = primary_window(world).map(|window| {
        format!(
            "physical={}x{} logical={}x{} scale={} present={:?}",
            window.physical_width(),
            window.physical_height(),
            window.width(),
            window.height(),
            window.scale_factor(),
            window.present_mode
        )
    });
    let noon_elevation = player_pose(world)
        .map(|(position, _)| maximum_sun_elevation_degrees(position))
        .unwrap_or(f32::NAN);
    let stage_trace = if work_trace_enabled() {
        "enabled"
    } else {
        "disabled"
    };
    let schedule_trace = if super::phase_trace::enabled() {
        "enabled; Bevy schedule spans are captured for phase diagnosis only"
    } else {
        "disabled"
    };
    let profile_caveat = if super::phase_trace::enabled() {
        "Bevy trace spans add diagnostic overhead; this run cannot establish clean acceptance performance"
    } else {
        "not applicable; schedule span tracing is disabled"
    };
    let features = if cfg!(feature = "asset-review-schedule-trace") {
        "asset-review,asset-review-schedule-trace"
    } else {
        "asset-review"
    };
    let configuration = format!(
        "mode=live-planet-acceptance\nseed={seed}\nanchor=first-settlement-player-spawn\nviewport={}\nviews=ground,settlement,globe,opposite\nsolar_phases=noon(anchor-local-maximum:{noon_elevation:.4}deg),sunset(0deg),night(-18deg)\nroutes=entry-reversal,orbit-zoom,follow-vehicle-recovery,return-reversal\nwarmup_seconds={WARMUP_SECONDS}\nmeasured_seconds_per_repeat={MEASURE_SECONDS}\nrepeats={REPEATS}\ninterval_source=Time<Real>::delta_secs_f64\nstall_limit_ms={STALL_LIMIT_MS}\non_foot_motion=W-with-A-or-D-turns-every-6s-on-entry-orbit-and-return-routes\nminimum_continuous_body_path_m={MIN_MEASURED_BODY_PATH_M}\nminimum_body_excursion_m={MIN_MEASURED_BODY_EXCURSION_M}\nphysics_gate=non-sleeping-body-translation-with-advancing-Time<Fixed>-and-live-colliders\nfixed_physics_time_min_seconds={}\nsimulation_advancing_fraction_min=0.95\nday_night_angle_span_min_rad=0.01\nfeatures={features}\nshadows=normal-production-settings\nsystem_stage_trace={stage_trace}\nschedule_span_trace={schedule_trace}\nschedule_trace_alignment=frame-clock CSV pairs Time<Real> route elapsed with monotonic route elapsed\nschedule_trace_semantics=per-span wall duration; nested schedule and render spans overlap and are not additive\nprofile_measurement_caveat={profile_caveat}\n",
        window.unwrap_or_else(|| "not-yet-available".into()),
        MEASURE_SECONDS * 0.5,
    );
    if let Err(error) = fs::write(run.directory.join("run-configuration.txt"), configuration) {
        record_error(run, format!("write run configuration: {error}"));
    }
}

fn prepare_capture(world: &mut World, index: usize) {
    let spec = capture_spec(index);
    let Some((body_position, _)) = player_pose(world) else {
        return;
    };
    let camera = main_camera_pose(world);
    let mut state = world.resource_mut::<Exploration>();
    if spec.view == CaptureView::Ground {
        state.set_planet_view_open(false);
        return;
    }

    state.set_planet_view_open(true);
    let should_follow = spec.view != CaptureView::Opposite;
    if state.planet_view_follows_body() != should_follow
        && let Some(camera) = camera
    {
        state.toggle_planet_view_follow(camera);
    }
    let target_radius = spec.view.radius();
    let requested_radius = state.planet_view_camera_radii().0;
    if (requested_radius - target_radius).abs() > 2.0 {
        let _ = state.request_planet_view_zoom((requested_radius / target_radius).ln());
    }
    if spec.view == CaptureView::Opposite && index == 9 {
        let _ = state.request_planet_view_orbit(Vec2::new(
            std::f32::consts::PI / ORBIT_RADIANS_PER_LOGICAL_PIXEL,
            0.0,
        ));
    }
    let _ = body_position;
}

fn hold_capture_sun(world: &mut World, index: usize) {
    let Some((body_position, _)) = player_pose(world) else {
        return;
    };
    let phase = capture_spec(index).phase;
    let virtual_delta = world.resource::<Time<Virtual>>().delta_secs();
    let mut tod = world.resource_mut::<TimeOfDay>();
    let desired = sun_angle_for_elevation(body_position, phase.elevation_degrees(body_position));
    tod.angle = (desired - virtual_delta * std::f32::consts::TAU / tod.day_length)
        .rem_euclid(std::f32::consts::TAU);
    world.resource_mut::<SunLock>().0 = false;
}

fn capture_ready(world: &mut World, spec: CaptureSpec) -> bool {
    let state = world.resource::<Exploration>();
    if spec.view == CaptureView::Ground {
        return !state.is_planet_view_active();
    }
    if !state.planet_view_ready() {
        return false;
    }
    let Some(camera) = main_camera_pose(world) else {
        return false;
    };
    let radius = camera.translation.length();
    let target = spec.view.radius();
    if (radius - target).abs() > 400.0 {
        return false;
    }
    if spec.view == CaptureView::Opposite {
        let Some((body, _)) = player_pose(world) else {
            return false;
        };
        return camera.translation.normalize().dot(body.normalize()) < -0.65;
    }
    true
}

#[derive(Clone, Copy)]
struct CaptureSpec {
    view: CaptureView,
    phase: SolarPhase,
}

fn capture_spec(index: usize) -> CaptureSpec {
    CaptureSpec {
        view: CAPTURE_VIEWS[index / SOLAR_PHASES.len()],
        phase: SOLAR_PHASES[index % SOLAR_PHASES.len()],
    }
}

fn capture_name(index: usize) -> String {
    let spec = capture_spec(index);
    format!("{}-{}", spec.view.name(), spec.phase.name())
}

fn capture_file(index: usize) -> String {
    format!("{}.png", capture_name(index))
}

fn sun_angle_for_elevation(anchor: Vec3, elevation_degrees: f32) -> f32 {
    let up = anchor.normalize_or(Vec3::Y);
    let horizontal = Vec2::new(up.x, up.z).length();
    let sun_norm = (1.0 + SUN_TILT * SUN_TILT).sqrt();
    let sun_y = SUN_TILT / sun_norm;
    let sun_horizontal = 1.0 / sun_norm;
    let cos_delta = if horizontal > 1e-4 {
        ((elevation_degrees.to_radians().sin() - up.y * sun_y) / (horizontal * sun_horizontal))
            .clamp(-1.0, 1.0)
    } else {
        1.0
    };
    up.z.atan2(up.x) + cos_delta.acos()
}

fn maximum_sun_elevation_degrees(anchor: Vec3) -> f32 {
    let up = anchor.normalize_or(Vec3::Y);
    let horizontal = Vec2::new(up.x, up.z).length();
    let sun_norm = (1.0 + SUN_TILT * SUN_TILT).sqrt();
    let max_dot = (up.y * SUN_TILT + horizontal) / sun_norm;
    max_dot.clamp(-1.0, 1.0).asin().to_degrees()
}

fn drive_route(
    world: &mut World,
    route: Route,
    elapsed: f64,
    warmup: bool,
    actions: &mut RouteActions,
    run: &mut PlanetAcceptance,
) {
    run.current_route = Some(route);
    match route {
        Route::EntryReversal => drive_entry_route(world, elapsed, warmup, actions, run),
        Route::OrbitZoom => drive_orbit_route(world, elapsed, actions),
        Route::FollowVehicleRecovery => drive_vehicle_route(world, elapsed, actions, run),
        Route::ReturnReversal => drive_return_route(world, elapsed),
    }
}

fn drive_entry_route(
    world: &mut World,
    elapsed: f64,
    warmup: bool,
    actions: &mut RouteActions,
    run: &mut PlanetAcceptance,
) {
    let open = elapsed < 6.0 || (9.0..=16.0).contains(&elapsed) || (22.0..=29.9).contains(&elapsed);
    world
        .resource_mut::<Exploration>()
        .set_planet_view_open(open);
    drive_on_foot_route_keys(world, elapsed);

    if warmup {
        drive_live_selection_and_teleport(world, elapsed, actions, run);
        drive_selector_access_guard(world, elapsed, actions, run);
        if (37.0..39.0).contains(&elapsed) {
            set_key(world, KeyCode::KeyW, false);
        }
        if (24.0..35.0).contains(&elapsed)
            && !actions.base_rate_changed
            && let Some(mut clock) = world.get_resource_mut::<PlanetSimulationClock>()
        {
            clock.set_base_rate(0.8);
            actions.base_rate_changed = true;
            record_event(run, elapsed, "base-rate-change", "set-to-0.8");
        }
        if elapsed >= 39.0
            && !actions.base_rate_restored
            && let Some(mut clock) = world.get_resource_mut::<PlanetSimulationClock>()
        {
            clock.set_base_rate(1.0);
            actions.base_rate_restored = true;
            run.coverage.base_rate_changed_and_restored = true;
            record_event(run, elapsed, "base-rate-change", "restored-to-1.0");
        }
    }
}

fn drive_live_selection_and_teleport(
    world: &mut World,
    elapsed: f64,
    actions: &mut RouteActions,
    run: &mut PlanetAcceptance,
) {
    let mut selected = world
        .resource::<Exploration>()
        .selected_planet_destination()
        .is_some();
    if elapsed >= 3.0
        && !selected
        && !actions.selection_attempted
        && request_center_selection(world)
    {
        actions.selection_attempted = true;
        record_event(
            run,
            elapsed,
            "destination-selection",
            "requested-screen-center",
        );
    }
    if elapsed >= 5.0
        && !selected
        && actions.selection_attempted
        && !actions.selection_retried
        && request_center_selection(world)
    {
        actions.selection_retried = true;
        record_event(run, elapsed, "destination-selection", "retry-screen-center");
    }
    selected = world
        .resource::<Exploration>()
        .selected_planet_destination()
        .is_some();
    if selected && !actions.selection_event_recorded {
        actions.selection_event_recorded = true;
        run.coverage.destination_selected = true;
        record_event(run, elapsed, "destination-selection", "selected");
    }

    // Closing in the same input frame clears the public pending T intent before
    // the ordinary teleport submit system can enqueue a physics request.
    if selected && !actions.cancellation_queued && elapsed >= 6.0 {
        let mut state = world.resource_mut::<Exploration>();
        if state.request_planet_view_teleport() {
            state.set_planet_view_open(false);
            let cleared_by_close = !state.take_planet_view_teleport_request()
                && state.pending_planet_teleport_request().is_none();
            if cleared_by_close {
                actions.cancellation_queued = true;
                run.coverage.queued_teleport_cancelled_on_close = true;
                record_event(
                    run,
                    elapsed,
                    "teleport-cancellation",
                    "queued-T-cleared-by-close",
                );
            } else {
                record_error(
                    run,
                    "closing Planet view did not clear the queued T intent".into(),
                );
            }
        }
    }

    if actions.cancellation_queued && !actions.confirmation_queued && elapsed >= 9.0 {
        let mut state = world.resource_mut::<Exploration>();
        state.set_planet_view_open(true);
        if state.selected_planet_destination().is_some()
            && state.planet_view_ready()
            && state.request_planet_view_teleport()
        {
            actions.confirmation_queued = true;
            record_event(run, elapsed, "teleport-confirmation", "queued-T-intent");
        }
    }

    if actions.confirmation_queued
        && !actions.teleport_submission_observed
        && world
            .resource::<Exploration>()
            .pending_planet_teleport_request()
            .is_some()
    {
        actions.teleport_submission_observed = true;
        run.coverage.teleport_request_submitted = true;
        record_event(run, elapsed, "teleport-confirmation", "request-submitted");
    }

    if let Some(mut state) = world.get_resource_mut::<Exploration>()
        && let Some(outcome) = state.take_planet_teleport_outcome()
    {
        let result = match outcome.result {
            PlanetTeleportOutcomeKind::Succeeded { .. } => "succeeded",
            PlanetTeleportOutcomeKind::Rejected { .. } => "rejected",
            PlanetTeleportOutcomeKind::Cancelled { .. } => "cancelled",
        };
        if actions.confirmation_queued && !actions.confirmation_outcome_recorded {
            actions.confirmation_outcome_recorded = true;
            actions.teleport_submission_observed = true;
            run.coverage.teleport_request_submitted = true;
            run.coverage.teleport_outcome_observed = true;
            record_event(run, elapsed, "teleport-outcome", result);
        }
    }

    if actions.confirmation_queued
        && !actions.confirmation_outcome_recorded
        && elapsed >= 14.0
        && world
            .resource::<Exploration>()
            .pending_planet_teleport_request()
            .is_some()
    {
        world
            .resource_mut::<Exploration>()
            .set_planet_view_open(false);
        record_event(
            run,
            elapsed,
            "teleport-cancellation",
            "pending-request-cancelled-by-close",
        );
    }
}

fn request_center_selection(world: &mut World) -> bool {
    let Some(window) = primary_window(world) else {
        return false;
    };
    let center = Vec2::new(
        window.physical_width() as f32,
        window.physical_height() as f32,
    ) / 2.0;
    world
        .resource_mut::<Exploration>()
        .request_planet_view_selection(center)
}

fn drive_selector_access_guard(
    world: &mut World,
    elapsed: f64,
    actions: &mut RouteActions,
    run: &mut PlanetAcceptance,
) {
    let press_v = elapsed >= 26.0 && !actions.selector_view_probe_sent;
    let hold_v = actions.selector_view_probe_sent && elapsed < 34.5;
    set_key(world, KeyCode::KeyV, press_v || hold_v);
    set_key(world, KeyCode::KeyM, false);
    set_key(world, KeyCode::Escape, false);
    if press_v {
        actions.selector_view_probe_sent = true;
        record_event(
            run,
            elapsed,
            "selector-guard",
            "pressed-V-while-Planet-view-active",
        );
    }
    let state = world.resource::<Exploration>();
    let paused = world.resource::<Time<Virtual>>().is_paused();
    let view_active = state.is_planet_view_active();
    if actions.selector_view_probe_sent
        && view_active
        && !state.is_vehicle_selector_open()
        && !paused
        && !actions.selector_view_block_observed
    {
        actions.selector_view_block_observed = true;
        run.coverage.selector_blocked_during_planet_view = true;
        record_event(run, elapsed, "selector-guard", "V-ignored-without-pausing");
    }
    if actions.selector_view_probe_sent
        && elapsed >= 34.5
        && !view_active
        && !state.is_vehicle_selector_open()
        && !paused
        && !actions.selector_view_close_observed
    {
        actions.selector_view_close_observed = true;
        run.coverage.selector_no_deferred_open_after_close = true;
        record_event(
            run,
            elapsed,
            "selector-guard",
            "no-deferred-selector-after-Planet-view-return",
        );
    }
}

fn drive_orbit_route(world: &mut World, elapsed: f64, actions: &mut RouteActions) {
    world
        .resource_mut::<Exploration>()
        .set_planet_view_open(true);
    drive_on_foot_route_keys(world, elapsed);
    set_key(world, KeyCode::KeyV, false);
    set_key(world, KeyCode::KeyM, false);
    set_key(world, KeyCode::Escape, false);

    let due = due_orbit_inputs(elapsed, actions.orbit_inputs_sent);
    for (index, (_, scroll, orbit)) in ORBIT_INPUTS.into_iter().enumerate() {
        if due[index] {
            let mut state = world.resource_mut::<Exploration>();
            let mut accepted = false;
            if scroll != 0.0 {
                accepted |= state.request_planet_view_zoom(scroll);
            }
            if orbit != Vec2::ZERO {
                accepted |= state.request_planet_view_orbit(orbit);
            }
            if accepted {
                actions.orbit_inputs_sent[index] = true;
            }
        }
    }
    if elapsed >= 23.0
        && !actions.orbit_follow_toggled
        && let Some(camera) = main_camera_pose(world)
    {
        world
            .resource_mut::<Exploration>()
            .toggle_planet_view_follow(camera);
        actions.orbit_follow_toggled = true;
    }
}

fn due_orbit_inputs(elapsed: f64, sent: [bool; ORBIT_INPUTS.len()]) -> [bool; ORBIT_INPUTS.len()] {
    std::array::from_fn(|index| elapsed >= ORBIT_INPUTS[index].0 && !sent[index])
}

fn drive_vehicle_route(
    world: &mut World,
    elapsed: f64,
    actions: &mut RouteActions,
    run: &mut PlanetAcceptance,
) {
    let paused = world.resource::<Time<Virtual>>().is_paused();
    let view_active = world.resource::<Exploration>().is_planet_view_active();
    world
        .resource_mut::<Exploration>()
        .set_planet_view_open(actions.vehicle_summon_requested && !paused);

    let request_selector =
        elapsed >= 2.0 && !actions.vehicle_selector_open_requested && !paused && !view_active;
    set_key(world, KeyCode::KeyV, request_selector);
    if request_selector {
        actions.vehicle_selector_open_requested = true;
        record_event(
            run,
            elapsed,
            "vehicle-handoff",
            "opened-normal-vehicle-selector-with-V",
        );
    }

    let selector_open = world.resource::<Exploration>().is_vehicle_selector_open();
    let paused = world.resource::<Time<Virtual>>().is_paused();
    if selector_open && !actions.vehicle_selector_open_observed {
        actions.vehicle_selector_open_observed = true;
        run.coverage.selector_opened = true;
        record_event(
            run,
            elapsed,
            "vehicle-handoff",
            "selector-opened-and-paused",
        );
    }

    let probe_map = selector_open && paused && !actions.vehicle_selector_probe_sent;
    set_key(world, KeyCode::KeyM, probe_map);
    if probe_map {
        actions.vehicle_selector_probe_sent = true;
        record_event(
            run,
            elapsed,
            "vehicle-handoff",
            "probed-M-while-selector-open",
        );
    }
    if actions.vehicle_selector_probe_sent
        && selector_open
        && paused
        && !world.resource::<Exploration>().is_planet_view_active()
        && !actions.vehicle_selector_priority_observed
    {
        actions.vehicle_selector_priority_observed = true;
        run.coverage.selector_priority_when_open = true;
        record_event(
            run,
            elapsed,
            "vehicle-handoff",
            "M-ignored-while-selector-open",
        );
    }

    let choose_car = selector_open && paused && elapsed >= 2.8 && !actions.vehicle_summon_requested;
    set_key(world, KeyCode::KeyC, choose_car);
    if choose_car {
        actions.vehicle_summon_requested = true;
        record_event(
            run,
            elapsed,
            "vehicle-handoff",
            "selected-car-through-normal-selector",
        );
    }
    let selector_open = world.resource::<Exploration>().is_vehicle_selector_open();
    let paused = world.resource::<Time<Virtual>>().is_paused();
    if actions.vehicle_summon_requested
        && !selector_open
        && !paused
        && !actions.vehicle_selector_closed_observed
    {
        run.coverage.selector_closed_unpaused = true;
        actions.vehicle_selector_closed_observed = true;
        record_event(
            run,
            elapsed,
            "vehicle-handoff",
            "selector-closed-and-unpaused",
        );
    }

    let occupied = world.resource::<Exploration>().is_in_vehicle();
    let car_entity = world.resource::<Exploration>().vehicle_entity(Kind::Car);
    let player = player_pose(world);
    let vehicle =
        car_entity.and_then(|entity| world.get::<Position>(entity).map(|position| position.0));
    let distance = player
        .zip(vehicle)
        .map(|((player, _), vehicle)| player.distance(vehicle))
        .unwrap_or(f32::INFINITY);

    if occupied && !actions.vehicle_entered_observed {
        actions.vehicle_entered_observed = true;
        run.coverage.vehicle_entered = true;
        record_event(run, elapsed, "vehicle-handoff", "entered-car");
    }

    if actions.vehicle_summon_requested && car_entity.is_some() && !actions.vehicle_summoned {
        actions.vehicle_summoned = true;
        record_event(run, elapsed, "vehicle-handoff", "car-summoned-by-selector");
    }

    if !actions.vehicle_summoned || selector_open || paused {
        set_key(world, KeyCode::KeyW, false);
        set_key(world, KeyCode::KeyS, false);
        set_key(world, KeyCode::KeyA, false);
        set_key(world, KeyCode::KeyD, false);
        set_key(world, KeyCode::KeyR, false);
    } else if !occupied {
        let (forward, turn) = if let (Some((position, heading)), Some(vehicle)) = (player, vehicle)
        {
            let up = position.normalize_or(Vec3::Y);
            let target = vehicle - position;
            let tangent = target - up * target.dot(up);
            let direction = tangent.normalize_or(heading);
            let angle = up
                .dot(heading.cross(direction))
                .atan2(heading.dot(direction));
            (
                distance > 2.2,
                if angle.abs() > 0.12 {
                    angle.signum() as i8
                } else {
                    0
                },
            )
        } else {
            (elapsed > 2.0 && elapsed < 22.0, 0)
        };
        set_key(world, KeyCode::KeyW, forward);
        set_key(world, KeyCode::KeyS, false);
        set_key(world, KeyCode::KeyA, turn > 0);
        set_key(world, KeyCode::KeyD, turn < 0);
        set_key(world, KeyCode::KeyR, false);
        if (2.0..37.0).contains(&elapsed) && distance <= 2.5 {
            set_key(world, KeyCode::KeyW, false);
            if elapsed - actions.last_interaction_at >= 1.0 {
                world
                    .resource_mut::<Exploration>()
                    .request(Action::Interact);
                actions.last_interaction_at = elapsed;
                actions.vehicle_interaction_requested = true;
            }
        }
        if elapsed >= 28.0 && !occupied && !actions.vehicle_failure_recorded {
            actions.vehicle_failure_recorded = true;
            record_event(
                run,
                elapsed,
                "vehicle-handoff",
                "car-entry-not-observed-by-28s",
            );
        }
    } else {
        set_key(world, KeyCode::KeyW, elapsed < 28.0);
        set_key(world, KeyCode::KeyS, (28.0..30.0).contains(&elapsed));
        set_key(world, KeyCode::KeyA, (20.0..22.0).contains(&elapsed));
        set_key(world, KeyCode::KeyD, false);
        set_key(world, KeyCode::KeyR, (30.0..31.4).contains(&elapsed));
        if elapsed >= 30.0 && !actions.vehicle_recovery_requested {
            actions.vehicle_recovery_requested = true;
            record_event(run, elapsed, "vehicle-recovery", "held-R-for-1.4s");
        }
        if elapsed >= 31.4
            && !actions.vehicle_recovery_observed
            && readout_contains(world, "Car recovered — ready to go")
        {
            actions.vehicle_recovery_observed = true;
            run.coverage.vehicle_recovered = true;
            record_event(
                run,
                elapsed,
                "vehicle-recovery",
                "recovered-car-feedback-observed",
            );
        }
        if elapsed >= 37.0 && elapsed - actions.last_exit_interaction_at >= 1.0 {
            world
                .resource_mut::<Exploration>()
                .request(Action::Interact);
            actions.last_exit_interaction_at = elapsed;
            if !actions.vehicle_exit_requested {
                actions.vehicle_exit_requested = true;
                record_event(run, elapsed, "vehicle-handoff", "requested-exit-on-foot");
            }
        }
    }
    set_key(world, KeyCode::Escape, false);

    if actions.vehicle_exit_requested && !occupied && !actions.vehicle_exit_observed {
        actions.vehicle_exit_observed = true;
        run.coverage.vehicle_exited = true;
        record_event(run, elapsed, "vehicle-handoff", "returned-to-on-foot");
    }
}

fn readout_contains(world: &mut World, text: &str) -> bool {
    world
        .query::<&Text>()
        .iter(world)
        .any(|readout| readout.0.contains(text))
}

fn drive_return_route(world: &mut World, elapsed: f64) {
    let open = elapsed < 6.0
        || (12.0..=22.0).contains(&elapsed)
        || (30.0..=42.0).contains(&elapsed)
        || elapsed >= 50.0;
    world
        .resource_mut::<Exploration>()
        .set_planet_view_open(open);
    drive_on_foot_route_keys(world, elapsed);
    set_key(world, KeyCode::KeyV, false);
    set_key(world, KeyCode::KeyM, false);
    set_key(world, KeyCode::Escape, false);
}

fn drive_on_foot_route_keys(world: &mut World, elapsed: f64) {
    if elapsed < 0.1 {
        // Let live exploration input clear any suppression left by a selector
        // or vehicle handoff before this route starts driving.
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
        set_key(world, KeyCode::KeyR, false);
        return;
    }

    let cycle = elapsed.rem_euclid(MOVEMENT_CYCLE_SECONDS);
    set_key(world, KeyCode::KeyS, false);
    set_key(world, KeyCode::KeyR, false);
    set_key(world, KeyCode::KeyW, true);
    set_key(world, KeyCode::KeyA, cycle < MOVEMENT_TURN_SECONDS);
    set_key(
        world,
        KeyCode::KeyD,
        (MOVEMENT_CYCLE_SECONDS / 2.0..MOVEMENT_CYCLE_SECONDS / 2.0 + MOVEMENT_TURN_SECONDS)
            .contains(&cycle),
    );
}

pub(super) fn set_key(world: &mut World, key: KeyCode, pressed: bool) {
    if let Some(mut keys) = world.get_resource_mut::<ButtonInput<KeyCode>>() {
        if pressed {
            keys.press(key);
        } else {
            keys.release(key);
        }
    }
}

fn sample_resident_counts(world: &mut World) -> ResidentCounts {
    let meshes = world
        .query_filtered::<Entity, With<Mesh3d>>()
        .iter(world)
        .count();
    let visible_meshes = world
        .query_filtered::<&ViewVisibility, With<Mesh3d>>()
        .iter(world)
        .filter(|visibility| visibility.get())
        .count();
    let colliders = world
        .query_filtered::<Entity, With<Collider>>()
        .iter(world)
        .count();
    ResidentCounts {
        meshes,
        visible_meshes,
        colliders,
    }
}

fn sample_frame(
    world: &mut World,
    _route: Route,
    started_at: f64,
    counts_at: f64,
    counts: ResidentCounts,
) -> FrameSample {
    let real = *world.resource::<Time<Real>>();
    let view = world.resource::<Exploration>();
    let (requested_radius, attained_radius) = view.planet_view_camera_radii();
    let follows = view.planet_view_follows_body();
    let view_active = view.is_planet_view_active();
    let view_ready = view.planet_view_ready();
    let in_vehicle = view.is_in_vehicle();
    let car_entity = view.vehicle_entity(Kind::Car);
    let plane_entity = view.vehicle_entity(Kind::Plane);
    let player_body = player_motion(world)
        .map(|(entity, position, velocity)| (entity, Motion { position, velocity }));
    let player_entity = player_body.map(|(entity, _)| entity);
    let player_motion = player_body.map_or_else(Motion::missing, |(_, motion)| motion);
    let car_motion = entity_motion(world, car_entity).unwrap_or_else(Motion::missing);
    let plane_motion = entity_motion(world, plane_entity).unwrap_or_else(Motion::missing);
    let (controlled, controlled_kind, controlled_entity) = if in_vehicle {
        if car_motion.position.distance(player_motion.position) < 1.0 {
            (car_motion, "car", car_entity)
        } else if plane_motion.position.distance(player_motion.position) < 1.0 {
            (plane_motion, "plane", plane_entity)
        } else {
            (player_motion, "vehicle-unresolved", player_entity)
        }
    } else {
        (player_motion, "on-foot", player_entity)
    };
    let controlled_sleeping =
        controlled_entity.is_some_and(|entity| world.get::<Sleeping>(entity).is_some());
    let fixed_time_elapsed = world.resource::<Time<Fixed>>().elapsed_secs_f64();
    let camera_radius =
        main_camera_pose(world).map_or(f32::NAN, |camera| camera.translation.length());
    let body_clearance = world
        .get_resource::<CollisionTerrain>()
        .map(|terrain| {
            controlled.position.length()
                - terrain
                    .0
                    .facet_radius(controlled.position.normalize_or(Vec3::Y), PLANET_RADIUS)
        })
        .unwrap_or(f32::NAN);
    let virtual_time = world.resource::<Time<Virtual>>();
    let virtual_rate = virtual_time.relative_speed();
    let virtual_paused = virtual_time.is_paused();
    let sun_angle = world.resource::<TimeOfDay>().angle;
    let real_elapsed = real.elapsed_secs_f64() - started_at;
    #[cfg(feature = "asset-review-schedule-trace")]
    if super::phase_trace::enabled()
        && let Some(trace) = world.get_resource::<super::phase_trace::ScheduleTraceRecorder>()
    {
        trace.record_frame_sample(real_elapsed);
    }
    FrameSample {
        real_elapsed,
        interval_ms: real.delta_secs_f64() * 1000.0,
        view_active,
        view_ready,
        follows,
        requested_radius,
        attained_radius,
        camera_radius,
        player: player_motion,
        car: car_motion,
        plane: plane_motion,
        controlled,
        controlled_kind,
        controlled_sleeping,
        fixed_time_elapsed,
        body_clearance,
        virtual_rate,
        virtual_paused,
        sun_angle,
        counts,
        counts_age: real.elapsed_secs_f64() - counts_at,
    }
}

pub(super) fn player_pose(world: &mut World) -> Option<(Vec3, Vec3)> {
    let mut query = world.query_filtered::<(&Position, &Player), With<Player>>();
    query
        .iter(world)
        .next()
        .map(|(position, player)| (position.0, player.heading))
}

pub(super) fn player_motion(world: &mut World) -> Option<(Entity, Vec3, Vec3)> {
    let mut query = world.query_filtered::<(Entity, &Position, &LinearVelocity), With<Player>>();
    query
        .iter(world)
        .next()
        .map(|(entity, position, velocity)| (entity, position.0, velocity.0))
}

fn entity_motion(world: &World, entity: Option<Entity>) -> Option<Motion> {
    let entity = entity?;
    Some(Motion {
        position: world.get::<Position>(entity)?.0,
        velocity: world.get::<LinearVelocity>(entity)?.0,
    })
}

pub(super) fn main_camera_pose(world: &mut World) -> Option<Transform> {
    let mut query = world.query_filtered::<&Transform, With<MainCamera>>();
    query.iter(world).next().copied()
}

pub(super) fn primary_window(world: &mut World) -> Option<Window> {
    let mut query = world.query_filtered::<&Window, With<PrimaryWindow>>();
    query.iter(world).next().cloned()
}

fn append_capture_row(world: &mut World, run: &mut PlanetAcceptance, index: usize, path: &Path) {
    let spec = capture_spec(index);
    let Some(window) = primary_window(world) else {
        record_error(run, "primary window disappeared during capture".into());
        return;
    };
    let Some(camera) = main_camera_pose(world) else {
        record_error(run, "main camera disappeared during capture".into());
        return;
    };
    let Some((body, _)) = player_pose(world) else {
        record_error(run, "player disappeared during capture".into());
        return;
    };
    let sun = world.resource::<TimeOfDay>().sun_dir;
    let elevation = body
        .normalize()
        .dot(sun)
        .clamp(-1.0, 1.0)
        .asin()
        .to_degrees();
    let ambient = world.resource::<GlobalAmbientLight>().brightness;
    let counts = sample_resident_counts(world);
    let state = world.resource::<Exploration>();
    let (requested, attained) = state.planet_view_camera_radii();
    let camera_entity = world
        .query_filtered::<Entity, With<MainCamera>>()
        .iter(world)
        .next();
    let fog = camera_entity
        .and_then(|entity| world.get::<DistanceFog>(entity))
        .map(|fog| format!("{fog:?}"))
        .unwrap_or_else(|| "none".into());
    let (illuminance, shadows) = world
        .query_filtered::<(&DirectionalLight, &Transform), With<Sun>>()
        .iter(world)
        .next()
        .map(|(light, _)| (light.illuminance, light.shadow_maps_enabled))
        .unwrap_or((f32::NAN, false));
    let seed = LevelData::from_artifact_bytes(include_bytes!("../../assets/level_1337.bin"))
        .map(|level| level.seed)
        .unwrap_or_default();
    let file = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let row = format!(
        "{},{},{},first-settlement-player-spawn,{seed},{},{},{:.3},{:.3},{:.4},{:?},{:.4},{:.4},{:.4},{:.6},{:.6},{:.6},{:.6},{:.4},{requested:.4},{attained:.4},{:.4},{:.4},{:.4},{:.6},{:.6},{:.6},{elevation:.4},{ambient:.4},{},{},{},{illuminance:.2},{shadows},{}\n",
        csv(file),
        spec.view.name(),
        spec.phase.name(),
        window.physical_width(),
        window.physical_height(),
        window.width(),
        window.height(),
        window.scale_factor(),
        window.present_mode,
        camera.translation.x,
        camera.translation.y,
        camera.translation.z,
        camera.rotation.x,
        camera.rotation.y,
        camera.rotation.z,
        camera.rotation.w,
        camera.translation.length(),
        body.x,
        body.y,
        body.z,
        sun.x,
        sun.y,
        sun.z,
        counts.visible_meshes,
        counts.meshes,
        counts.colliders,
        csv(&fog),
    );
    append_file(&run.directory.join("capture-manifest.csv"), row, run);
}

fn finish_repeat(
    world: &mut World,
    run: &mut PlanetAcceptance,
    route: Route,
    repeat: u8,
    samples: &[FrameSample],
) {
    let directory = run.directory.join("performance").join(route.name());
    if let Err(error) = fs::create_dir_all(&directory) {
        record_error(run, format!("create {}: {error}", directory.display()));
        return;
    }
    let path = directory.join(format!("repeat-{repeat}-raw.csv"));
    let mut raw = String::from(
        "real_elapsed_s,wall_interval_ms,route,view_active,view_ready,follow,requested_radius_m,attained_radius_m,camera_radius_m,controlled_kind,controlled_sleeping,fixed_time_elapsed_s,player_x,player_y,player_z,player_vx,player_vy,player_vz,car_x,car_y,car_z,car_vx,car_vy,car_vz,plane_x,plane_y,plane_z,plane_vx,plane_vy,plane_vz,controlled_x,controlled_y,controlled_z,controlled_vx,controlled_vy,controlled_vz,body_clearance_m,virtual_rate,virtual_paused,sun_angle_rad,resident_meshes,visible_meshes,resident_colliders,counts_age_s\n",
    );
    for sample in samples {
        raw.push_str(&sample.csv_row(route));
    }
    if let Err(error) = fs::write(&path, raw) {
        record_error(run, format!("write {}: {error}", path.display()));
    }

    let stats = summarize(samples);
    let active_path = path_from_samples(samples, true);
    if active_path.traveled_m > 0.1 {
        run.coverage.body_moved_during_planet_view = true;
    }
    let physics_movement = physics_advanced_while_moving(samples, false);
    if physics_advanced_while_moving(samples, true) {
        run.coverage.collision_world_live_during_planet_movement = true;
    }
    run.measured_stalls
        .push((route, repeat, stats.over_33_33ms));
    let row = format!(
        "{},{repeat},{},{:.4},{:.4},{:.4},{:.4},{},{:.4},{:.4},{:.4},{:.4},{},{},{:.4},{},{:.6},{:.4},{},{},{},{},{},{},{:.6},{:.6}\n",
        route.name(),
        samples.len(),
        stats.p50,
        stats.p95,
        stats.p99,
        stats.max,
        stats.over_33_33ms,
        stats.body_displacement,
        stats.body_path.traveled_m,
        stats.body_path.max_excursion_m,
        stats.mean_body_speed,
        stats.physics_live_samples,
        stats.physics_sleeping_samples,
        stats.fixed_time_span,
        stats.simulation_advancing_samples,
        stats.sun_angle_span,
        stats.minimum_clearance,
        stats.mesh_min,
        stats.mesh_max,
        stats.visible_mesh_min,
        stats.visible_mesh_max,
        stats.collider_min,
        stats.collider_max,
        stats.sun_start,
        stats.sun_end,
    );
    append_file(&run.directory.join("performance-summary.csv"), row, run);
    info!(
        route = route.name(),
        repeat,
        sample_count = samples.len(),
        p50_ms = stats.p50,
        p95_ms = stats.p95,
        p99_ms = stats.p99,
        max_ms = stats.max,
        over_33_33ms = stats.over_33_33ms,
        body_displacement_m = stats.body_displacement,
        body_path_m = stats.body_path.traveled_m,
        max_body_excursion_m = stats.body_path.max_excursion_m,
        physics_sleeping_samples = stats.physics_sleeping_samples,
        fixed_time_span_s = stats.fixed_time_span,
        sun_angle_span_rad = stats.sun_angle_span,
        physics_advanced_while_moving = physics_movement,
        "PLANET_ACCEPTANCE_REPEAT"
    );
    if samples.is_empty() {
        record_error(
            run,
            format!("{} repeat {repeat} produced no timing rows", route.name()),
        );
    }
    if stats.body_path.traveled_m < MIN_MEASURED_BODY_PATH_M
        || stats.body_path.max_excursion_m < MIN_MEASURED_BODY_EXCURSION_M
    {
        record_error(
            run,
            format!(
                "{} repeat {repeat} did not show sustained controlled-body motion (path {:.2} m, excursion {:.2} m)",
                route.name(),
                stats.body_path.traveled_m,
                stats.body_path.max_excursion_m
            ),
        );
    }
    if stats.physics_live_samples < samples.len() / 2 {
        record_error(
            run,
            format!(
                "{} repeat {repeat} did not observe live colliders and finite body clearance for enough samples",
                route.name()
            ),
        );
    }
    if stats.physics_sleeping_samples > samples.len() / 2 {
        record_error(
            run,
            format!(
                "{} repeat {repeat} controlled body was sleeping for more than half of its samples",
                route.name()
            ),
        );
    }
    if stats.fixed_time_span < MEASURE_SECONDS * 0.5 {
        record_error(
            run,
            format!(
                "{} repeat {repeat} advanced fixed physics time by only {:.2} seconds",
                route.name(),
                stats.fixed_time_span
            ),
        );
    }
    if !physics_movement {
        record_error(
            run,
            format!(
                "{} repeat {repeat} did not observe a non-sleeping controlled body translate while fixed physics time advanced and terrain colliders were live",
                route.name()
            ),
        );
    }
    if stats.simulation_advancing_samples < samples.len() * 19 / 20 {
        record_error(
            run,
            format!(
                "{} repeat {repeat} spent too much time paused or with a stopped simulation clock",
                route.name()
            ),
        );
    }
    if stats.sun_angle_span < 0.01 {
        record_error(
            run,
            format!(
                "{} repeat {repeat} did not advance the world day/night animation",
                route.name()
            ),
        );
    }
    let _ = world;
}

fn flush_route_trace_sidecars(world: &World, run: &mut PlanetAcceptance, route: Route) {
    let directory = run.directory.join("performance").join(route.name());
    for repeat in 1..=REPEATS {
        if let Some(trace) = world.get_resource::<crate::chunks::PlanetWorkTrace>() {
            let path = directory.join(format!("repeat-{repeat}-work-trace.csv"));
            if let Err(error) = fs::write(&path, trace.repeat_csv(route.name(), repeat)) {
                record_error(run, format!("write {}: {error}", path.display()));
            }
            if let Some(csv) = trace.vehicle_action_csv(route.name(), repeat) {
                let path = directory.join(format!("repeat-{repeat}-action-trace.csv"));
                if let Err(error) = fs::write(&path, csv) {
                    record_error(run, format!("write {}: {error}", path.display()));
                }
            }
        }
        #[cfg(feature = "asset-review-schedule-trace")]
        {
            if let Some(trace) = world.get_resource::<super::phase_trace::ScheduleTraceRecorder>() {
                let path = directory.join(format!("repeat-{repeat}-schedule-trace.csv"));
                let csv = trace.take_repeat_csv(route.name(), repeat);
                if csv.lines().count() < 2 {
                    record_error(
                        run,
                        format!(
                            "schedule trace has no spans for {} repeat {repeat}",
                            route.name()
                        ),
                    );
                }
                if let Err(error) = fs::write(&path, csv) {
                    record_error(run, format!("write {}: {error}", path.display()));
                }
                let path = directory.join(format!("repeat-{repeat}-frame-clock.csv"));
                let csv = trace.take_frame_clock_csv(route.name(), repeat);
                if csv.lines().count() < 2 {
                    record_error(
                        run,
                        format!(
                            "frame clock trace has no samples for {} repeat {repeat}",
                            route.name()
                        ),
                    );
                }
                if let Err(error) = fs::write(&path, csv) {
                    record_error(run, format!("write {}: {error}", path.display()));
                }
            }
        }
    }
}

#[derive(Default)]
struct Summary {
    p50: f64,
    p95: f64,
    p99: f64,
    max: f64,
    over_33_33ms: usize,
    body_displacement: f64,
    body_path: BodyPathSummary,
    mean_body_speed: f64,
    physics_live_samples: usize,
    physics_sleeping_samples: usize,
    fixed_time_span: f64,
    simulation_advancing_samples: usize,
    sun_angle_span: f64,
    minimum_clearance: f64,
    mesh_min: usize,
    mesh_max: usize,
    visible_mesh_min: usize,
    visible_mesh_max: usize,
    collider_min: usize,
    collider_max: usize,
    sun_start: f64,
    sun_end: f64,
}

fn summarize(samples: &[FrameSample]) -> Summary {
    if samples.is_empty() {
        return Summary::default();
    }
    let mut intervals = samples
        .iter()
        .map(|sample| sample.interval_ms)
        .collect::<Vec<_>>();
    intervals.sort_by(f64::total_cmp);
    let nearest_rank = |q: f64| {
        let rank = (intervals.len() as f64 * q).ceil() as usize;
        intervals[rank.saturating_sub(1).min(intervals.len() - 1)]
    };
    let middle = intervals.len() / 2;
    let p50 = if intervals.len().is_multiple_of(2) {
        (intervals[middle - 1] + intervals[middle]) / 2.0
    } else {
        intervals[middle]
    };
    let first = samples[0].controlled.position;
    let last = samples[samples.len() - 1].controlled.position;
    let body_path = path_from_samples(samples, false);
    let sun_min = samples
        .iter()
        .map(|sample| f64::from(sample.sun_angle))
        .fold(f64::INFINITY, f64::min);
    let sun_max = samples
        .iter()
        .map(|sample| f64::from(sample.sun_angle))
        .fold(f64::NEG_INFINITY, f64::max);
    let fixed_time_span =
        samples[samples.len() - 1].fixed_time_elapsed - samples[0].fixed_time_elapsed;
    let mean_body_speed = samples
        .iter()
        .map(|sample| sample.controlled.velocity.length() as f64)
        .sum::<f64>()
        / samples.len() as f64;
    let mut summary = Summary {
        p50,
        p95: nearest_rank(0.95),
        p99: nearest_rank(0.99),
        max: intervals[intervals.len() - 1],
        over_33_33ms: samples
            .iter()
            .filter(|sample| sample.interval_ms > STALL_LIMIT_MS)
            .count(),
        body_displacement: first.distance(last) as f64,
        body_path,
        mean_body_speed,
        physics_live_samples: samples
            .iter()
            .filter(|sample| sample.body_clearance.is_finite() && sample.counts.colliders > 0)
            .count(),
        physics_sleeping_samples: samples
            .iter()
            .filter(|sample| sample.controlled_sleeping)
            .count(),
        fixed_time_span,
        simulation_advancing_samples: samples
            .iter()
            .filter(|sample| !sample.virtual_paused && sample.virtual_rate > 0.0)
            .count(),
        sun_angle_span: sun_max - sun_min,
        minimum_clearance: samples
            .iter()
            .map(|sample| sample.body_clearance as f64)
            .filter(|clearance| clearance.is_finite())
            .fold(f64::INFINITY, f64::min),
        mesh_min: usize::MAX,
        visible_mesh_min: usize::MAX,
        collider_min: usize::MAX,
        sun_start: samples[0].sun_angle as f64,
        sun_end: samples[samples.len() - 1].sun_angle as f64,
        ..Summary::default()
    };
    for sample in samples {
        summary.mesh_min = summary.mesh_min.min(sample.counts.meshes);
        summary.mesh_max = summary.mesh_max.max(sample.counts.meshes);
        summary.visible_mesh_min = summary.visible_mesh_min.min(sample.counts.visible_meshes);
        summary.visible_mesh_max = summary.visible_mesh_max.max(sample.counts.visible_meshes);
        summary.collider_min = summary.collider_min.min(sample.counts.colliders);
        summary.collider_max = summary.collider_max.max(sample.counts.colliders);
    }
    summary
}

impl FrameSample {
    fn csv_row(self, route: Route) -> String {
        format!(
            "{:.5},{:.6},{},{},{},{},{:.4},{:.4},{:.4},{},{},{:.6},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{},{:.6},{},{},{},{:.4}\n",
            self.real_elapsed,
            self.interval_ms,
            route.name(),
            self.view_active,
            self.view_ready,
            self.follows,
            self.requested_radius,
            self.attained_radius,
            self.camera_radius,
            self.controlled_kind,
            self.controlled_sleeping,
            self.fixed_time_elapsed,
            self.player.position.x,
            self.player.position.y,
            self.player.position.z,
            self.player.velocity.x,
            self.player.velocity.y,
            self.player.velocity.z,
            self.car.position.x,
            self.car.position.y,
            self.car.position.z,
            self.car.velocity.x,
            self.car.velocity.y,
            self.car.velocity.z,
            self.plane.position.x,
            self.plane.position.y,
            self.plane.position.z,
            self.plane.velocity.x,
            self.plane.velocity.y,
            self.plane.velocity.z,
            self.controlled.position.x,
            self.controlled.position.y,
            self.controlled.position.z,
            self.controlled.velocity.x,
            self.controlled.velocity.y,
            self.controlled.velocity.z,
            self.body_clearance,
            self.virtual_rate,
            self.virtual_paused,
            self.sun_angle,
            self.counts.meshes,
            self.counts.visible_meshes,
            self.counts.colliders,
            self.counts_age,
        )
    }
}

fn route_index(route: Route) -> usize {
    ROUTES
        .iter()
        .position(|candidate| *candidate == route)
        .unwrap_or(0)
}

fn append_file(path: &Path, row: String, run: &mut PlanetAcceptance) {
    match OpenOptions::new().create(true).append(true).open(path) {
        Ok(mut file) => {
            if let Err(error) = file.write_all(row.as_bytes()) {
                record_error(run, format!("write {}: {error}", path.display()));
            }
        }
        Err(error) => record_error(run, format!("open {}: {error}", path.display())),
    }
}

fn record_event(run: &mut PlanetAcceptance, elapsed: f64, event: &str, result: &str) {
    let route = run.current_route.map_or("cross-feature", Route::name);
    let row = format!("{elapsed:.4},{},{},{}\n", route, csv(event), csv(result),);
    run.event_log.push(row.clone());
    append_file(&run.directory.join("cross-feature-events.csv"), row, run);
}

fn record_error(run: &mut PlanetAcceptance, error: String) {
    error!("Planet acceptance: {error}");
    run.errors.push(error);
}

pub(super) fn file_is_nonempty(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|metadata| metadata.is_file() && metadata.len() > 0)
}

pub(super) fn csv(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn finish_run(world: &mut World, run: &mut PlanetAcceptance) {
    if let Some(probe) = world.get_resource::<crate::chunks::PlanetWorkTrace>()
        && let Err(error) = fs::write(run.directory.join("planet-work-trace.csv"), probe.to_csv())
    {
        record_error(run, format!("write planet work trace: {error}"));
    }
    let mut missing = Vec::new();
    for index in 0..CAPTURE_VIEWS.len() * SOLAR_PHASES.len() {
        let path = run.directory.join("captures").join(capture_file(index));
        if !file_is_nonempty(&path) {
            missing.push(path.display().to_string());
        }
    }
    for route in ROUTES {
        for repeat in 1..=REPEATS {
            let path = run
                .directory
                .join("performance")
                .join(route.name())
                .join(format!("repeat-{repeat}-raw.csv"));
            if !file_is_nonempty(&path) {
                missing.push(path.display().to_string());
            }
        }
    }
    for path in missing {
        record_error(
            run,
            format!("required acceptance output is absent or empty: {path}"),
        );
    }
    for route in ROUTES {
        let repeats_with_stalls = run
            .measured_stalls
            .iter()
            .filter(|(candidate, _, count)| *candidate == route && *count > 0)
            .count();
        if repeats_with_stalls >= 2 {
            record_error(
                run,
                format!(
                    "{} had >33.33 ms intervals in {repeats_with_stalls} measured repeats",
                    route.name()
                ),
            );
        }
    }
    for (name, observed) in [
        ("destination selection", run.coverage.destination_selected),
        (
            "queued T cancellation on view close",
            run.coverage.queued_teleport_cancelled_on_close,
        ),
        (
            "submitted T request",
            run.coverage.teleport_request_submitted,
        ),
        ("T request outcome", run.coverage.teleport_outcome_observed),
        (
            "normal vehicle selector opens and pauses time",
            run.coverage.selector_opened,
        ),
        (
            "selector owns M while already open",
            run.coverage.selector_priority_when_open,
        ),
        (
            "vehicle selection closes selector and unpauses",
            run.coverage.selector_closed_unpaused,
        ),
        (
            "V is blocked while Planet view is active without pausing",
            run.coverage.selector_blocked_during_planet_view,
        ),
        (
            "blocked V is not deferred until Planet view closes",
            run.coverage.selector_no_deferred_open_after_close,
        ),
        (
            "simulation base-rate change and restore",
            run.coverage.base_rate_changed_and_restored,
        ),
        ("vehicle entry", run.coverage.vehicle_entered),
        ("vehicle recovery", run.coverage.vehicle_recovered),
        ("vehicle exit", run.coverage.vehicle_exited),
        (
            "body movement while Planet view is active",
            run.coverage.body_moved_during_planet_view,
        ),
        (
            "body movement with collision terrain/colliders live",
            run.coverage.collision_world_live_during_planet_movement,
        ),
    ] {
        if !observed {
            record_error(
                run,
                format!("required cross-feature outcome was not observed: {name}"),
            );
        }
    }
    let status = if run.errors.is_empty() {
        "complete"
    } else {
        "incomplete-or-rejected"
    };
    let report = format!(
        "status={status}\nrequested_captures={}\nrequested_raw_runs={}\nrecurring_stall_threshold=at-least-two-repeats-with-one-or-more-intervals-above-{STALL_LIMIT_MS:.2}ms\ncoverage_destination_selected={}\ncoverage_queued_teleport_cancelled_on_close={}\ncoverage_teleport_request_submitted={}\ncoverage_teleport_outcome_observed={}\ncoverage_selector_opened={}\ncoverage_selector_priority_when_open={}\ncoverage_selector_closed_unpaused={}\ncoverage_selector_blocked_during_planet_view={}\ncoverage_selector_no_deferred_open_after_close={}\ncoverage_base_rate_changed_and_restored={}\ncoverage_vehicle_entered={}\ncoverage_vehicle_recovered={}\ncoverage_vehicle_exited={}\ncoverage_body_moved_during_planet_view={}\ncoverage_collision_world_live_during_planet_movement={}\nerrors={}\n{}",
        CAPTURE_VIEWS.len() * SOLAR_PHASES.len(),
        ROUTES.len() * usize::from(REPEATS),
        run.coverage.destination_selected,
        run.coverage.queued_teleport_cancelled_on_close,
        run.coverage.teleport_request_submitted,
        run.coverage.teleport_outcome_observed,
        run.coverage.selector_opened,
        run.coverage.selector_priority_when_open,
        run.coverage.selector_closed_unpaused,
        run.coverage.selector_blocked_during_planet_view,
        run.coverage.selector_no_deferred_open_after_close,
        run.coverage.base_rate_changed_and_restored,
        run.coverage.vehicle_entered,
        run.coverage.vehicle_recovered,
        run.coverage.vehicle_exited,
        run.coverage.body_moved_during_planet_view,
        run.coverage.collision_world_live_during_planet_movement,
        run.errors.len(),
        run.errors.join("\n")
    );
    if let Err(error) = fs::write(run.directory.join("acceptance-status.txt"), &report) {
        error!("Planet acceptance report write failed: {error}");
        run.errors.push(format!("write final status: {error}"));
    }
    if run.errors.is_empty() {
        info!("Planet acceptance completed with all expected outputs");
        world.write_message(AppExit::Success);
    } else {
        error!(
            errors = run.errors.len(),
            "Planet acceptance failed acceptance checks"
        );
        world.write_message(AppExit::Error(NonZeroU8::new(1).unwrap()));
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CaptureView, FrameSample, Motion, ResidentCounts, Route, SUN_TILT, SolarPhase,
        continuous_body_path, due_orbit_inputs, maximum_sun_elevation_degrees,
        physics_advanced_while_moving, sun_angle_for_elevation,
    };
    use bevy::prelude::Vec3;

    fn solar_elevation_degrees(anchor: Vec3, angle: f32) -> f32 {
        let up = anchor.normalize_or(Vec3::Y);
        let sun_norm = (1.0 + SUN_TILT * SUN_TILT).sqrt();
        let sun = Vec3::new(
            angle.cos() / sun_norm,
            SUN_TILT / sun_norm,
            angle.sin() / sun_norm,
        );
        up.dot(sun).clamp(-1.0, 1.0).asin().to_degrees()
    }

    #[test]
    fn orbit_inputs_fire_once_even_when_frame_cadence_skips_the_threshold() {
        let mut sent = [false; 5];
        assert_eq!(due_orbit_inputs(0.0, sent), [false; 5]);

        let due = due_orbit_inputs(9.1, sent);
        assert_eq!(due, [true, true, false, false, false]);
        for (was_sent, is_due) in sent.iter_mut().zip(due) {
            *was_sent |= is_due;
        }
        assert_eq!(due_orbit_inputs(9.2, sent), [false; 5]);

        let due = due_orbit_inputs(39.0, sent);
        assert_eq!(due, [false, false, true, true, true]);
        for (was_sent, is_due) in sent.iter_mut().zip(due) {
            *was_sent |= is_due;
        }
        assert_eq!(due_orbit_inputs(60.0, sent), [false; 5]);
    }

    #[test]
    fn noon_capture_targets_the_anchor_local_solar_maximum() {
        let anchor = Vec3::new(6_000.0, 8_000.0, 0.0);
        let maximum = maximum_sun_elevation_degrees(anchor);
        let angle = sun_angle_for_elevation(anchor, SolarPhase::Noon.elevation_degrees(anchor));

        assert!(
            maximum < 60.0,
            "test anchor should expose impossible 60-degree noon"
        );
        assert!((solar_elevation_degrees(anchor, angle) - maximum).abs() < 0.01);
    }

    #[test]
    fn settlement_capture_keeps_a_building_scale_camera_standoff() {
        let radius = CaptureView::Settlement.radius();

        assert!((2_200.0..=2_300.0).contains(&radius));
    }

    #[test]
    fn continuous_body_path_does_not_count_velocity_or_vehicle_handoff_as_motion() {
        let stationary = [
            ("on-foot", Vec3::new(1.0, 2.0, 3.0)),
            ("on-foot", Vec3::new(1.0, 2.0, 3.0)),
            ("on-foot", Vec3::new(1.0, 2.0, 3.0)),
        ];
        let moving_with_handoff = [
            ("on-foot", Vec3::ZERO),
            ("car", Vec3::new(1_000.0, 0.0, 0.0)),
            ("car", Vec3::new(1_001.0, 0.0, 0.0)),
            ("car", Vec3::new(1_003.0, 0.0, 0.0)),
        ];
        let loop_path = [
            ("on-foot", Vec3::ZERO),
            ("on-foot", Vec3::new(4.0, 0.0, 0.0)),
            ("on-foot", Vec3::new(8.0, 0.0, 0.0)),
            ("on-foot", Vec3::ZERO),
        ];

        assert_eq!(
            continuous_body_path(stationary.iter().copied().map(Some)).traveled_m,
            0.0
        );
        assert_eq!(
            continuous_body_path(moving_with_handoff.iter().copied().map(Some)).traveled_m,
            3.0
        );
        let loop_summary = continuous_body_path(loop_path.iter().copied().map(Some));
        assert_eq!(loop_summary.traveled_m, 16.0);
        assert_eq!(loop_summary.max_excursion_m, 8.0);
    }

    fn physics_sample(position: Vec3, fixed_time_elapsed: f64, sleeping: bool) -> FrameSample {
        let motion = Motion {
            position,
            velocity: Vec3::new(3.5, 0.0, 0.0),
        };
        FrameSample {
            real_elapsed: fixed_time_elapsed,
            interval_ms: 16.7,
            view_active: true,
            view_ready: true,
            follows: false,
            requested_radius: 2_250.0,
            attained_radius: 2_250.0,
            camera_radius: 2_250.0,
            player: motion,
            car: Motion::missing(),
            plane: Motion::missing(),
            controlled: motion,
            controlled_kind: "on-foot",
            controlled_sleeping: sleeping,
            fixed_time_elapsed,
            body_clearance: 0.5,
            virtual_rate: 1.0,
            virtual_paused: false,
            sun_angle: fixed_time_elapsed as f32,
            counts: ResidentCounts {
                meshes: 1,
                visible_meshes: 1,
                colliders: 1,
            },
            counts_age: 0.0,
        }
    }

    #[test]
    fn physics_probe_requires_non_sleeping_translation_during_a_physics_step() {
        let stationary = [
            physics_sample(Vec3::ZERO, 1.0, false),
            physics_sample(Vec3::ZERO, 2.0, false),
        ];
        let moved = [
            physics_sample(Vec3::ZERO, 1.0, false),
            physics_sample(Vec3::X * 0.1, 2.0, false),
        ];
        let sleeping = [
            physics_sample(Vec3::ZERO, 1.0, false),
            physics_sample(Vec3::X * 0.1, 2.0, true),
        ];
        let no_physics_step = [
            physics_sample(Vec3::ZERO, 1.0, false),
            physics_sample(Vec3::X * 0.1, 1.0, false),
        ];

        assert!(!physics_advanced_while_moving(&stationary, true));
        assert!(physics_advanced_while_moving(&moved, true));
        assert!(!physics_advanced_while_moving(&sleeping, true));
        assert!(!physics_advanced_while_moving(&no_physics_step, true));
        assert_eq!(
            moved[0].csv_row(Route::EntryReversal).split(',').count(),
            44
        );
    }
}
