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
use avian3d::prelude::{Collider, LinearVelocity, Position};
use bevy::{
    camera::CameraUpdateSystems,
    input::InputSystems,
    prelude::*,
    render::view::screenshot::{Screenshot, save_to_disk},
    window::PrimaryWindow,
};
use shared::{
    level::LevelData,
    planet_view::{PLANET_VIEW_FAR_RADIUS, PLANET_VIEW_NEAR_RADIUS},
    sphere::PLANET_RADIUS,
    state::AppState,
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
const STALL_LIMIT_MS: f64 = 1000.0 / 30.0;
const ORBIT_RADIANS_PER_LOGICAL_PIXEL: f32 = 0.004;
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
            Self::Settlement => PLANET_VIEW_NEAR_RADIUS,
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

    fn elevation_degrees(self) -> f32 {
        match self {
            Self::Noon => 60.0,
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
    coverage: CrossFeatureCoverage,
}

#[derive(Default)]
struct CrossFeatureCoverage {
    destination_selected: bool,
    queued_teleport_cancelled_on_close: bool,
    teleport_request_submitted: bool,
    teleport_outcome_observed: bool,
    selector_opened: bool,
    selector_priority_preserved_view: bool,
    selector_closed_unpaused: bool,
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
    selector_opened: bool,
    selector_probe_sent: bool,
    selector_closed: bool,
    selector_open_observed: bool,
    selector_priority_observed: bool,
    selector_close_observed: bool,
    base_rate_changed: bool,
    base_rate_restored: bool,
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
    body_clearance: f32,
    virtual_rate: f32,
    virtual_paused: bool,
    sun_angle: f32,
    counts: ResidentCounts,
    counts_age: f64,
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

    app.insert_resource(PlanetAcceptance {
        directory,
        phase: Phase::WaitingForWorld,
        errors: Vec::new(),
        counts: ResidentCounts::default(),
        counts_at: 0.0,
        measured_stalls: Vec::new(),
        event_log: Vec::new(),
        current_route: None,
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

fn drive_planet_acceptance(world: &mut World) {
    if !acceptance_world_ready(world) {
        return;
    }
    let now = world.resource::<Time<Real>>().elapsed_secs_f64();
    let Some(mut run) = world.remove_resource::<PlanetAcceptance>() else {
        return;
    };

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
                Phase::Measuring {
                    route,
                    repeat: 1,
                    started_at: now,
                    actions: RouteActions::default(),
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
                if repeat < REPEATS {
                    Phase::Measuring {
                        route,
                        repeat: repeat + 1,
                        started_at: now,
                        actions: RouteActions::default(),
                        samples: Vec::with_capacity(4_000),
                    }
                } else if let Some(next) = ROUTES.get(route_index(route) + 1).copied() {
                    info!(route = next.name(), "Planet acceptance warm-up started");
                    Phase::Warmup {
                        route: next,
                        started_at: now,
                        actions: RouteActions::default(),
                    }
                } else {
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

fn acceptance_world_ready(world: &mut World) -> bool {
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
            "route,repeat,sample_count,p50_ms,p95_ms,p99_ms,max_ms,over_33_33ms,body_displacement_m,body_speed_mean_mps,minimum_clearance_m,mesh_min,mesh_max,visible_mesh_min,visible_mesh_max,collider_min,collider_max,sun_angle_start_rad,sun_angle_end_rad\n",
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
    let configuration = format!(
        "mode=live-planet-acceptance\nseed={seed}\nanchor=first-settlement-player-spawn\nviewport={}\nviews=ground,settlement,globe,opposite\nsolar_phases=noon(60deg),sunset(0deg),night(-18deg)\nroutes=entry-reversal,orbit-zoom,follow-vehicle-recovery,return-reversal\nwarmup_seconds={WARMUP_SECONDS}\nmeasured_seconds_per_repeat={MEASURE_SECONDS}\nrepeats={REPEATS}\ninterval_source=Time<Real>::delta_secs_f64\nstall_limit_ms={STALL_LIMIT_MS}\nfeatures=asset-review\nshadows=normal-production-settings\n",
        window.unwrap_or_else(|| "not-yet-available".into())
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
    let desired = sun_angle_for_elevation(body_position, phase.elevation_degrees());
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
    const SUN_TILT: f32 = 0.35;
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
    let open = elapsed < 6.0 || (9.0..=16.0).contains(&elapsed) || elapsed >= 22.0;
    world
        .resource_mut::<Exploration>()
        .set_planet_view_open(open);
    set_key(world, KeyCode::KeyW, true);
    set_key(world, KeyCode::KeyS, false);
    set_key(world, KeyCode::KeyA, false);
    set_key(world, KeyCode::KeyD, false);
    set_key(world, KeyCode::KeyR, false);

    if warmup {
        drive_live_selection_and_teleport(world, elapsed, actions, run);
        drive_selector_priority(world, elapsed, actions, run);
        if (37.0..39.0).contains(&elapsed) {
            set_key(world, KeyCode::KeyW, false);
        }
        if elapsed >= 24.0 && elapsed < 35.0 && !actions.base_rate_changed {
            if let Some(mut clock) = world.get_resource_mut::<PlanetSimulationClock>() {
                clock.set_base_rate(0.8);
                actions.base_rate_changed = true;
                record_event(run, elapsed, "base-rate-change", "set-to-0.8");
            }
        }
        if elapsed >= 39.0 && !actions.base_rate_restored {
            if let Some(mut clock) = world.get_resource_mut::<PlanetSimulationClock>() {
                clock.set_base_rate(1.0);
                actions.base_rate_restored = true;
                run.coverage.base_rate_changed_and_restored = true;
                record_event(run, elapsed, "base-rate-change", "restored-to-1.0");
            }
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
    if elapsed >= 3.0 && !selected && !actions.selection_attempted {
        if request_center_selection(world) {
            actions.selection_attempted = true;
            record_event(
                run,
                elapsed,
                "destination-selection",
                "requested-screen-center",
            );
        }
    }
    if elapsed >= 5.0 && !selected && actions.selection_attempted && !actions.selection_retried {
        if request_center_selection(world) {
            actions.selection_retried = true;
            record_event(run, elapsed, "destination-selection", "retry-screen-center");
        }
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

fn drive_selector_priority(
    world: &mut World,
    elapsed: f64,
    actions: &mut RouteActions,
    run: &mut PlanetAcceptance,
) {
    let press_v = elapsed >= 35.0 && !actions.selector_opened;
    let press_m = elapsed >= 36.0 && !actions.selector_probe_sent;
    let press_escape = elapsed >= 37.0 && !actions.selector_closed;
    set_key(world, KeyCode::KeyV, press_v);
    set_key(world, KeyCode::KeyM, press_m);
    set_key(world, KeyCode::Escape, press_escape);
    if press_v {
        actions.selector_opened = true;
        record_event(
            run,
            elapsed,
            "selector-priority",
            "pressed-V-over-planet-view",
        );
    }
    if press_m {
        actions.selector_probe_sent = true;
        record_event(
            run,
            elapsed,
            "selector-priority",
            "map-toggle-while-selector-open",
        );
    }
    if press_escape {
        actions.selector_closed = true;
        record_event(run, elapsed, "selector-priority", "pressed-Escape-to-close");
    }

    let paused = world.resource::<Time<Virtual>>().is_paused();
    let view_active = world.resource::<Exploration>().is_planet_view_active();
    if actions.selector_opened && paused && !actions.selector_open_observed {
        actions.selector_open_observed = true;
        run.coverage.selector_opened = true;
        record_event(run, elapsed, "selector-priority", "opened-and-paused");
    }
    if actions.selector_probe_sent && paused && view_active && !actions.selector_priority_observed {
        actions.selector_priority_observed = true;
        run.coverage.selector_priority_preserved_view = true;
        record_event(
            run,
            elapsed,
            "selector-priority",
            "M-ignored-while-selector-open-view-preserved",
        );
    }
    if actions.selector_closed && !paused && !actions.selector_close_observed {
        actions.selector_close_observed = true;
        run.coverage.selector_closed_unpaused = true;
        record_event(run, elapsed, "selector-priority", "closed-and-unpaused");
    }
}

fn drive_orbit_route(world: &mut World, elapsed: f64, actions: &mut RouteActions) {
    world
        .resource_mut::<Exploration>()
        .set_planet_view_open(true);
    set_key(world, KeyCode::KeyW, true);
    set_key(world, KeyCode::KeyS, false);
    set_key(world, KeyCode::KeyR, false);
    set_key(world, KeyCode::KeyA, false);
    set_key(world, KeyCode::KeyD, false);
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
    world
        .resource_mut::<Exploration>()
        .set_planet_view_open(true);
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

    if elapsed >= 1.0 && !actions.vehicle_summoned {
        world
            .resource_mut::<Exploration>()
            .request(Action::Summon(Kind::Car));
        actions.vehicle_summoned = true;
        record_event(run, elapsed, "vehicle-handoff", "summoned-car");
    }

    if !occupied {
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
        if elapsed >= 2.0 && elapsed < 37.0 && distance <= 2.5 {
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
    set_key(world, KeyCode::KeyV, false);
    set_key(world, KeyCode::KeyM, false);
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
    set_key(world, KeyCode::KeyW, true);
    set_key(world, KeyCode::KeyS, false);
    set_key(world, KeyCode::KeyA, false);
    set_key(world, KeyCode::KeyD, false);
    set_key(world, KeyCode::KeyR, false);
    set_key(world, KeyCode::KeyV, false);
    set_key(world, KeyCode::KeyM, false);
    set_key(world, KeyCode::Escape, false);
}

fn set_key(world: &mut World, key: KeyCode, pressed: bool) {
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
    let player_motion = player_motion(world).unwrap_or_else(Motion::missing);
    let car_motion = entity_motion(world, car_entity).unwrap_or_else(Motion::missing);
    let plane_motion = entity_motion(world, plane_entity).unwrap_or_else(Motion::missing);
    let (controlled, controlled_kind) = if in_vehicle {
        if car_motion.position.distance(player_motion.position) < 1.0 {
            (car_motion, "car")
        } else if plane_motion.position.distance(player_motion.position) < 1.0 {
            (plane_motion, "plane")
        } else {
            (player_motion, "vehicle-unresolved")
        }
    } else {
        (player_motion, "on-foot")
    };
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
    FrameSample {
        real_elapsed: real.elapsed_secs_f64() - started_at,
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
        body_clearance,
        virtual_rate,
        virtual_paused,
        sun_angle,
        counts,
        counts_age: real.elapsed_secs_f64() - counts_at,
    }
}

fn player_pose(world: &mut World) -> Option<(Vec3, Vec3)> {
    let mut query = world.query_filtered::<(&Position, &Player), With<Player>>();
    query
        .iter(world)
        .next()
        .map(|(position, player)| (position.0, player.heading))
}

fn player_motion(world: &mut World) -> Option<Motion> {
    let mut query = world.query_filtered::<(&Position, &LinearVelocity), With<Player>>();
    query.iter(world).next().map(|(position, velocity)| Motion {
        position: position.0,
        velocity: velocity.0,
    })
}

fn entity_motion(world: &World, entity: Option<Entity>) -> Option<Motion> {
    let entity = entity?;
    Some(Motion {
        position: world.get::<Position>(entity)?.0,
        velocity: world.get::<LinearVelocity>(entity)?.0,
    })
}

fn main_camera_pose(world: &mut World) -> Option<Transform> {
    let mut query = world.query_filtered::<&Transform, With<MainCamera>>();
    query.iter(world).next().copied()
}

fn primary_window(world: &mut World) -> Option<Window> {
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
        "real_elapsed_s,wall_interval_ms,route,view_active,view_ready,follow,requested_radius_m,attained_radius_m,camera_radius_m,controlled_kind,player_x,player_y,player_z,player_vx,player_vy,player_vz,car_x,car_y,car_z,car_vx,car_vy,car_vz,plane_x,plane_y,plane_z,plane_vx,plane_vy,plane_vz,controlled_x,controlled_y,controlled_z,controlled_vx,controlled_vy,controlled_vz,body_clearance_m,virtual_rate,virtual_paused,sun_angle_rad,resident_meshes,visible_meshes,resident_colliders,counts_age_s\n",
    );
    for sample in samples {
        raw.push_str(&sample.csv_row(route));
    }
    if let Err(error) = fs::write(&path, raw) {
        record_error(run, format!("write {}: {error}", path.display()));
    }

    let stats = summarize(samples);
    if samples
        .iter()
        .any(|sample| sample.view_active && sample.controlled.velocity.length_squared() > 0.01)
    {
        run.coverage.body_moved_during_planet_view = true;
    }
    if samples.iter().any(|sample| {
        sample.view_active
            && sample.controlled.velocity.length_squared() > 0.01
            && sample.body_clearance.is_finite()
            && sample.counts.colliders > 0
    }) {
        run.coverage.collision_world_live_during_planet_movement = true;
    }
    run.measured_stalls
        .push((route, repeat, stats.over_33_33ms));
    let row = format!(
        "{},{repeat},{},{:.4},{:.4},{:.4},{:.4},{},{:.4},{:.4},{:.4},{},{},{},{},{},{},{:.6},{:.6}\n",
        route.name(),
        samples.len(),
        stats.p50,
        stats.p95,
        stats.p99,
        stats.max,
        stats.over_33_33ms,
        stats.body_displacement,
        stats.mean_body_speed,
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
        "PLANET_ACCEPTANCE_REPEAT"
    );
    if samples.is_empty() {
        record_error(
            run,
            format!("{} repeat {repeat} produced no timing rows", route.name()),
        );
    }
    let _ = world;
}

#[derive(Default)]
struct Summary {
    p50: f64,
    p95: f64,
    p99: f64,
    max: f64,
    over_33_33ms: usize,
    body_displacement: f64,
    mean_body_speed: f64,
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
        mean_body_speed,
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
            "{:.5},{:.6},{},{},{},{},{:.4},{:.4},{:.4},{},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.6},{},{},{},{:.4}\n",
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

fn file_is_nonempty(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|metadata| metadata.is_file() && metadata.len() > 0)
}

fn csv(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn finish_run(world: &mut World, run: &mut PlanetAcceptance) {
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
        ("selector open/pause", run.coverage.selector_opened),
        (
            "selector M priority preserves Planet view",
            run.coverage.selector_priority_preserved_view,
        ),
        (
            "selector close/unpause",
            run.coverage.selector_closed_unpaused,
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
        "status={status}\nrequested_captures={}\nrequested_raw_runs={}\nrecurring_stall_threshold=at-least-two-repeats-with-one-or-more-intervals-above-{STALL_LIMIT_MS:.2}ms\ncoverage_destination_selected={}\ncoverage_queued_teleport_cancelled_on_close={}\ncoverage_teleport_request_submitted={}\ncoverage_teleport_outcome_observed={}\ncoverage_selector_opened={}\ncoverage_selector_priority_preserved_view={}\ncoverage_selector_closed_unpaused={}\ncoverage_base_rate_changed_and_restored={}\ncoverage_vehicle_entered={}\ncoverage_vehicle_recovered={}\ncoverage_vehicle_exited={}\ncoverage_body_moved_during_planet_view={}\ncoverage_collision_world_live_during_planet_movement={}\nerrors={}\n{}",
        CAPTURE_VIEWS.len() * SOLAR_PHASES.len(),
        ROUTES.len() * usize::from(REPEATS),
        run.coverage.destination_selected,
        run.coverage.queued_teleport_cancelled_on_close,
        run.coverage.teleport_request_submitted,
        run.coverage.teleport_outcome_observed,
        run.coverage.selector_opened,
        run.coverage.selector_priority_preserved_view,
        run.coverage.selector_closed_unpaused,
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
    use super::due_orbit_inputs;

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
}
