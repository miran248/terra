use super::{background_capture_description, diagnostic_screenshot};
use crate::{
    exploration::{Exploration, Kind},
    ui::{Sidebar, SidebarAction, SidebarActionControl, SidebarReadoutSlot, SidebarScrollArea},
};
use avian3d::prelude::Collider;
use bevy::{
    app::AppExit,
    input::{
        InputSystems,
        mouse::{MouseScrollUnit, MouseWheel},
        touch::TouchPhase,
    },
    prelude::*,
    render::view::screenshot::save_to_disk,
    ui::{ComputedNode, ScrollPosition, UiGlobalTransform},
    window::PrimaryWindow,
};
use std::{collections::HashSet, fs, path::PathBuf};

const WARMUP_COLLIDERS: usize = 100;
const WARMUP_STABLE_FRAMES: u8 = 30;
const PHASE_TIMEOUT: f64 = 18.0;
const REQUIRED_CAPTURES: [&str; 7] = [
    "sidebar-entry.png",
    "planet-context-target.png",
    "sidebar-actions-scroll.png",
    "sidebar-selector-inline.png",
    "sidebar-follow.png",
    "sidebar-recovery-progress.png",
    "sidebar-recovery-complete.png",
];

#[derive(Resource)]
pub(super) struct SidebarDiagnostic {
    directory: PathBuf,
    phase: Phase,
    phase_started_at: Option<f64>,
    started_at: Option<f64>,
    last_collider_count: Option<usize>,
    stable_world_frames: u8,
    pending_click_release: bool,
    hold_started_at: Option<f64>,
    captures: HashSet<String>,
    events: Vec<String>,
    errors: Vec<String>,
    no_zoom_proved: bool,
    wheel_radius_before: Option<f32>,
    wheel_scroll_before: Option<f32>,
    follow_before: Option<bool>,
    follow_toggled: bool,
    follow_probe_logged: bool,
    selector_paused: bool,
    cancel_resumed: bool,
    car_choice_resumed: bool,
    mouse_view_close: bool,
    recovery_early_cancelled: bool,
    recovery_completed: bool,
    finished: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Phase {
    #[default]
    Warmup,
    Entry,
    OpenPlanet,
    WaitPlanet,
    SelectTarget,
    WaitTarget,
    ScrollContext,
    CaptureContext,
    ScrollActions,
    CaptureActions,
    Teleport,
    WaitTeleport,
    WaitTeleportSettled,
    ScrollTopAfterTeleport,
    CloseAfterTeleport,
    WaitTeleportClose,
    ScrollTop,
    Follow,
    WaitFollow,
    CaptureFollow,
    ClosePlanet,
    WaitClose,
    ReopenPlanet,
    WaitReopenPlanet,
    EnsureDestination,
    WaitReselectedDestination,
    ScrollForTeleport,
    ScrollSelector,
    OpenSelector,
    WaitSelector,
    CaptureSelector,
    CancelSelector,
    WaitCancel,
    OpenSelectorForCar,
    WaitSelectorForCar,
    ChooseCar,
    WaitCarChoice,
    RecoveryEarly,
    RecoveryEarlyRelease,
    WaitRecoveryReset,
    RecoveryResetCheck,
    RecoveryComplete,
    RecoveryCompleteRelease,
    WaitRecoveryComplete,
    FinishWait,
    Done,
}

impl SidebarDiagnostic {
    fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            phase: Phase::Warmup,
            phase_started_at: None,
            started_at: None,
            last_collider_count: None,
            stable_world_frames: 0,
            pending_click_release: false,
            hold_started_at: None,
            captures: HashSet::new(),
            events: Vec::new(),
            errors: Vec::new(),
            no_zoom_proved: false,
            wheel_radius_before: None,
            wheel_scroll_before: None,
            follow_before: None,
            follow_toggled: false,
            follow_probe_logged: false,
            selector_paused: false,
            cancel_resumed: false,
            car_choice_resumed: false,
            mouse_view_close: false,
            recovery_early_cancelled: false,
            recovery_completed: false,
            finished: false,
        }
    }

    pub(super) fn start(&mut self, world: &mut World, now: f64) {
        if self.started_at.is_none() {
            self.started_at = Some(now);
            self.set_phase(Phase::Entry, now);
            let window = primary_window_description(world);
            let capture = background_capture_description(world);
            let camera = main_camera_description(world);
            self.record(
                world,
                now,
                "start",
                format!("stable world ready; {capture}; {window}; {camera}"),
            );
        }
    }

    pub(super) fn tick(&mut self, world: &mut World) {
        if self.finished || !playing(world) {
            return;
        }
        let now = world.resource::<Time<Real>>().elapsed_secs_f64();
        if self.pending_click_release {
            set_mouse_button(world, false);
            self.pending_click_release = false;
            self.record(world, now, "pointer-release", "sidebar gesture released");
            return;
        }
        if self.started_at.is_none() {
            if !self.world_is_stable(world) || !background_target_ready(world) {
                return;
            }
            self.start(world, now);
        }
        if self.phase_timed_out(now) {
            self.fail(world, now, format!("phase {:?} timed out", self.phase));
            return;
        }

        match self.phase {
            Phase::Warmup => self.set_phase(Phase::Entry, now),
            Phase::Entry => {
                self.capture(world, now, "sidebar-entry.png");
                self.set_phase(Phase::OpenPlanet, now);
            }
            Phase::OpenPlanet => {
                if self.click_action(world, now, SidebarAction::TogglePlanetView) {
                    self.set_phase(Phase::WaitPlanet, now);
                }
            }
            Phase::WaitPlanet => {
                if world.resource::<Exploration>().planet_view_ready() {
                    self.record(
                        world,
                        now,
                        "view-open",
                        "mouse sidebar action opened Planet view",
                    );
                    self.set_phase(Phase::SelectTarget, now);
                }
            }
            Phase::SelectTarget => {
                if let Some(center) = primary_window_center(world)
                    && self.click_point(world, now, center, "scene-center")
                {
                    self.set_phase(Phase::WaitTarget, now);
                }
            }
            Phase::WaitTarget => {
                if world
                    .resource::<Exploration>()
                    .selected_planet_destination()
                    .is_some()
                {
                    self.record(
                        world,
                        now,
                        "scene-select",
                        "visible scene click selected a destination",
                    );
                    self.set_phase(Phase::ScrollContext, now);
                } else if self.phase_elapsed(now) > 8.0 {
                    self.errors
                        .push("scene center did not select a Planet destination".into());
                    self.set_phase(Phase::ScrollContext, now);
                }
            }
            Phase::ScrollContext => {
                self.wheel_radius_before = world
                    .get_resource::<Exploration>()
                    .map(|state| state.planet_view_camera_radii().0);
                self.wheel_scroll_before = sidebar_scroll(world);
                self.wheel_over_sidebar(world, now, -3.0);
                self.set_phase(Phase::CaptureContext, now);
            }
            Phase::CaptureContext => {
                let camera_radius = world.resource::<Exploration>().planet_view_camera_radii().0;
                let scroll = sidebar_scroll(world).unwrap_or_default();
                let radius_unchanged = self
                    .wheel_radius_before
                    .is_some_and(|before| (before - camera_radius).abs() < 0.01);
                let scroll_advanced = self
                    .wheel_scroll_before
                    .is_some_and(|before| scroll > before);
                self.no_zoom_proved = radius_unchanged && scroll_advanced;
                self.record(
                    world,
                    now,
                    "sidebar-wheel",
                    format!("scroll_y={scroll:.1}; requested_radius={camera_radius:.1}m; radius_unchanged={radius_unchanged}; scroll_advanced={scroll_advanced}"),
                );
                self.capture(world, now, "planet-context-target.png");
                self.set_phase(Phase::ScrollActions, now);
            }
            Phase::ScrollActions => {
                self.wheel_over_sidebar(world, now, -12.0);
                self.set_phase(Phase::CaptureActions, now);
            }
            Phase::CaptureActions => {
                self.capture(world, now, "sidebar-actions-scroll.png");
                self.set_phase(Phase::ScrollTop, now);
            }
            Phase::Teleport => {
                if self.click_action(world, now, SidebarAction::Teleport) {
                    self.set_phase(Phase::WaitTeleport, now);
                }
            }
            Phase::WaitTeleport => {
                if world
                    .resource::<Exploration>()
                    .planet_teleport_status()
                    .is_some()
                {
                    self.record(world, now, "teleport-click", "mouse T action submitted");
                    self.set_phase(Phase::WaitTeleportSettled, now);
                } else if self.phase_elapsed(now) > 8.0 {
                    self.errors
                        .push("mouse teleport action did not submit".into());
                    self.set_phase(Phase::ScrollTop, now);
                }
            }
            Phase::ScrollTop => {
                self.wheel_over_sidebar(world, now, 16.0);
                self.set_phase(Phase::Follow, now);
            }
            Phase::Follow => {
                let (active, ready, follows, destination_selected) = {
                    let state = world.resource::<Exploration>();
                    (
                        state.is_planet_view_active(),
                        state.planet_view_ready(),
                        state.planet_view_follows_body(),
                        state.selected_planet_destination().is_some(),
                    )
                };
                self.record(
                    world,
                    now,
                    "follow-before",
                    format!(
                        "active={}; ready={}; follows={}; destination_selected={}",
                        active, ready, follows, destination_selected,
                    ),
                );
                if self.click_action(world, now, SidebarAction::ToggleFollow) {
                    self.follow_before = Some(follows);
                    self.set_phase(Phase::WaitFollow, now);
                }
            }
            Phase::WaitFollow => {
                let state = world.resource::<Exploration>();
                if self
                    .follow_before
                    .is_some_and(|before| state.planet_view_follows_body() != before)
                {
                    self.record(
                        world,
                        now,
                        "follow-click",
                        format!(
                            "mouse Follow action toggled follow to {}",
                            state.planet_view_follows_body()
                        ),
                    );
                    self.follow_toggled = true;
                    self.set_phase(Phase::CaptureFollow, now);
                } else if self.phase_elapsed(now) > 0.75 && !self.follow_probe_logged {
                    let (active, ready, follows, destination_selected) = {
                        let state = world.resource::<Exploration>();
                        (
                            state.is_planet_view_active(),
                            state.planet_view_ready(),
                            state.planet_view_follows_body(),
                            state.selected_planet_destination().is_some(),
                        )
                    };
                    let cursor = primary_window_entity(world)
                        .and_then(|entity| world.get::<Window>(entity))
                        .and_then(Window::physical_cursor_position);
                    let center = action_center(world, SidebarAction::ToggleFollow);
                    let camera_count = main_camera_count(world);
                    self.record(
                        world,
                        now,
                        "follow-no-change",
                        format!(
                            "active={active}; ready={ready}; follows={follows}; destination_selected={destination_selected}; cursor={cursor:?}; action_center={center:?}; main_cameras={camera_count}"
                        ),
                    );
                    self.capture(world, now, "sidebar-follow-no-change.png");
                    self.follow_probe_logged = true;
                    self.set_phase(Phase::CaptureFollow, now);
                }
            }
            Phase::CaptureFollow => {
                self.capture(world, now, "sidebar-follow.png");
                self.set_phase(Phase::ClosePlanet, now);
            }
            Phase::ClosePlanet => {
                let state = world.resource::<Exploration>();
                if state.planet_view_ready() {
                    if self.click_action(world, now, SidebarAction::TogglePlanetView) {
                        self.set_phase(Phase::WaitClose, now);
                    }
                }
            }
            Phase::WaitClose => {
                let state = world.resource::<Exploration>();
                if !state.is_planet_view_active() && !state.planet_view_ready() {
                    self.record(
                        world,
                        now,
                        "view-close",
                        "mouse sidebar action returned to gameplay",
                    );
                    self.mouse_view_close = true;
                    self.set_phase(Phase::ReopenPlanet, now);
                }
            }
            Phase::ReopenPlanet => {
                let state = world.resource::<Exploration>();
                if !state.is_planet_view_active()
                    && !state.planet_view_ready()
                    && self.click_action(world, now, SidebarAction::TogglePlanetView)
                {
                    self.set_phase(Phase::WaitReopenPlanet, now);
                }
            }
            Phase::WaitReopenPlanet => {
                if world.resource::<Exploration>().planet_view_ready() {
                    self.record(
                        world,
                        now,
                        "view-reopen",
                        "mouse sidebar action reopened Planet view",
                    );
                    self.set_phase(Phase::EnsureDestination, now);
                }
            }
            Phase::EnsureDestination => {
                if world
                    .resource::<Exploration>()
                    .selected_planet_destination()
                    .is_some()
                {
                    self.set_phase(Phase::ScrollForTeleport, now);
                } else if let Some(center) = primary_window_center(world)
                    && self.click_point(world, now, center, "scene-center-reselect")
                {
                    self.set_phase(Phase::WaitReselectedDestination, now);
                }
            }
            Phase::WaitReselectedDestination => {
                if world
                    .resource::<Exploration>()
                    .selected_planet_destination()
                    .is_some()
                {
                    self.set_phase(Phase::ScrollForTeleport, now);
                } else if self.phase_elapsed(now) > 8.0 {
                    self.errors
                        .push("reopened Planet view lost the selected destination".into());
                    self.set_phase(Phase::ScrollForTeleport, now);
                }
            }
            Phase::ScrollForTeleport => {
                self.wheel_over_sidebar(world, now, -16.0);
                self.set_phase(Phase::Teleport, now);
            }
            Phase::WaitTeleportSettled => {
                let state = world.resource::<Exploration>();
                if state.planet_view_ready() {
                    self.set_phase(Phase::ScrollTopAfterTeleport, now);
                } else if !state.is_planet_view_active() {
                    self.record(
                        world,
                        now,
                        "teleport-view-close",
                        "teleport returned to gameplay",
                    );
                    self.set_phase(Phase::ScrollSelector, now);
                }
            }
            Phase::ScrollTopAfterTeleport => {
                self.wheel_over_sidebar(world, now, 16.0);
                self.set_phase(Phase::CloseAfterTeleport, now);
            }
            Phase::CloseAfterTeleport => {
                let state = world.resource::<Exploration>();
                if state.planet_view_ready() {
                    if self.click_action(world, now, SidebarAction::TogglePlanetView) {
                        self.set_phase(Phase::WaitTeleportClose, now);
                    }
                } else if !state.is_planet_view_active() {
                    self.set_phase(Phase::ScrollSelector, now);
                }
            }
            Phase::WaitTeleportClose => {
                let state = world.resource::<Exploration>();
                if !state.is_planet_view_active() && !state.planet_view_ready() {
                    self.record(
                        world,
                        now,
                        "teleport-view-close",
                        "mouse sidebar action returned to gameplay",
                    );
                    self.set_phase(Phase::ScrollSelector, now);
                }
            }
            Phase::ScrollSelector => {
                self.wheel_over_sidebar(world, now, -16.0);
                self.set_phase(Phase::OpenSelector, now);
            }
            Phase::OpenSelector => {
                if self.click_action(world, now, SidebarAction::ToggleVehicleSelector) {
                    self.set_phase(Phase::WaitSelector, now);
                }
            }
            Phase::WaitSelector => {
                let paused = world.resource::<Time<Virtual>>().is_paused();
                let opened = world.resource::<Exploration>().is_vehicle_selector_open();
                if opened && paused {
                    self.selector_paused = true;
                    self.record(
                        world,
                        now,
                        "selector-open",
                        "inline Car/Plane/Cancel visible; virtual time paused",
                    );
                    self.set_phase(Phase::CaptureSelector, now);
                }
            }
            Phase::CaptureSelector => {
                self.capture(world, now, "sidebar-selector-inline.png");
                self.set_phase(Phase::CancelSelector, now);
            }
            Phase::CancelSelector => {
                if self.click_action(world, now, SidebarAction::CancelVehicleSelector) {
                    self.set_phase(Phase::WaitCancel, now);
                }
            }
            Phase::WaitCancel => {
                let state = world.resource::<Exploration>();
                if !state.is_vehicle_selector_open()
                    && !world.resource::<Time<Virtual>>().is_paused()
                {
                    self.cancel_resumed = true;
                    self.record(
                        world,
                        now,
                        "selector-cancel",
                        "mouse Cancel resumed virtual time",
                    );
                    self.set_phase(Phase::OpenSelectorForCar, now);
                }
            }
            Phase::OpenSelectorForCar => {
                if self.click_action(world, now, SidebarAction::ToggleVehicleSelector) {
                    self.set_phase(Phase::WaitSelectorForCar, now);
                }
            }
            Phase::WaitSelectorForCar => {
                if world.resource::<Exploration>().is_vehicle_selector_open()
                    && world.resource::<Time<Virtual>>().is_paused()
                {
                    self.set_phase(Phase::ChooseCar, now);
                }
            }
            Phase::ChooseCar => {
                if self.click_action(world, now, SidebarAction::SelectVehicle(Kind::Car)) {
                    self.set_phase(Phase::WaitCarChoice, now);
                }
            }
            Phase::WaitCarChoice => {
                if !world.resource::<Exploration>().is_vehicle_selector_open()
                    && !world.resource::<Time<Virtual>>().is_paused()
                {
                    self.car_choice_resumed = true;
                    self.record(
                        world,
                        now,
                        "selector-car",
                        "mouse Car choice resumed virtual time",
                    );
                    self.set_phase(Phase::RecoveryEarly, now);
                }
            }
            Phase::RecoveryEarly => {
                if self.start_recovery_hold(world, now) {
                    self.hold_started_at = Some(now);
                    self.set_phase(Phase::RecoveryEarlyRelease, now);
                }
            }
            Phase::RecoveryEarlyRelease => {
                let progress = recovery_percent(world).unwrap_or(0);
                if progress >= 25 && progress < 100 {
                    self.capture(world, now, "sidebar-recovery-progress.png");
                    self.set_phase(Phase::WaitRecoveryReset, now);
                } else if self.phase_elapsed(now) > 2.0 {
                    self.errors
                        .push(format!("early hold showed only {progress}% progress"));
                    self.set_phase(Phase::WaitRecoveryReset, now);
                }
            }
            Phase::WaitRecoveryReset => {
                if self.phase_elapsed(now) >= 0.55 {
                    set_mouse_button(world, false);
                    self.hold_started_at = None;
                    self.set_phase(Phase::RecoveryResetCheck, now);
                }
            }
            Phase::RecoveryResetCheck => {
                if self.phase_elapsed(now) >= 0.35 {
                    let progress = recovery_percent(world).unwrap_or(u32::MAX);
                    self.recovery_early_cancelled = progress == 0;
                    if !self.recovery_early_cancelled {
                        self.errors.push(format!(
                            "early recovery remained at {progress}% after release"
                        ));
                    }
                    self.record(
                        world,
                        now,
                        "recovery-early-cancel",
                        format!("progress_after_release={progress}%"),
                    );
                    self.capture(world, now, "sidebar-recovery-cancelled.png");
                    self.set_phase(Phase::RecoveryComplete, now);
                }
            }
            Phase::RecoveryComplete => {
                if self.start_recovery_hold(world, now) {
                    self.hold_started_at = Some(now);
                    self.set_phase(Phase::WaitRecoveryComplete, now);
                }
            }
            Phase::WaitRecoveryComplete => {
                let text =
                    readout_text(world, SidebarReadoutSlot::RecoveryContext).unwrap_or_default();
                let success_readout = text.to_ascii_lowercase();
                if success_readout.contains("recovered")
                    || success_readout.contains("returned to safe ground")
                {
                    self.recovery_completed = true;
                    self.record(world, now, "recovery-complete", text);
                    self.capture(world, now, "sidebar-recovery-complete.png");
                    self.set_phase(Phase::RecoveryCompleteRelease, now);
                } else if self
                    .hold_started_at
                    .is_some_and(|started| now - started > 2.5)
                {
                    self.errors
                        .push("one-second mouse recovery did not complete".into());
                    self.set_phase(Phase::RecoveryCompleteRelease, now);
                }
            }
            Phase::RecoveryCompleteRelease => {
                set_mouse_button(world, false);
                self.hold_started_at = None;
                self.set_phase(Phase::FinishWait, now);
            }
            Phase::FinishWait => {
                if self.phase_elapsed(now) >= 2.0 {
                    self.finish(world, now);
                }
            }
            Phase::Done => {}
        }
    }

    pub(super) fn capture(&mut self, world: &mut World, now: f64, name: &str) -> bool {
        if self.captures.contains(name) {
            return false;
        }
        let Some(screenshot) = diagnostic_screenshot(world) else {
            return false;
        };
        let path = self.directory.join("captures").join(name);
        world.spawn(screenshot).observe(save_to_disk(path.clone()));
        self.captures.insert(name.to_owned());
        self.record(world, now, "capture", name);
        true
    }

    pub(super) fn status(&self) -> &'static str {
        if self.errors.is_empty() {
            "PASS"
        } else {
            "FAIL"
        }
    }

    fn world_is_stable(&mut self, world: &mut World) -> bool {
        let mut colliders = world.query_filtered::<Entity, With<Collider>>();
        let count = colliders.iter(world).count();
        if count < WARMUP_COLLIDERS {
            self.last_collider_count = None;
            self.stable_world_frames = 0;
            return false;
        }
        if self.last_collider_count == Some(count) {
            self.stable_world_frames = self.stable_world_frames.saturating_add(1);
        } else {
            self.last_collider_count = Some(count);
            self.stable_world_frames = 1;
        }
        self.stable_world_frames >= WARMUP_STABLE_FRAMES
    }

    fn click_action(&mut self, world: &mut World, now: f64, action: SidebarAction) -> bool {
        let center = action_center(world, action);
        let Some(center) = center else {
            return false;
        };
        self.click_point(world, now, center, &format!("sidebar:{action:?}"))
    }

    fn click_point(&mut self, world: &mut World, now: f64, position: Vec2, label: &str) -> bool {
        if !set_cursor_physical(world, position) {
            return false;
        }
        set_mouse_button(world, true);
        self.pending_click_release = true;
        self.record(world, now, "pointer-press", format!("{label}@{position:?}"));
        true
    }

    fn wheel_over_sidebar(&mut self, world: &mut World, now: f64, lines: f32) {
        let Some(center) = sidebar_center(world) else {
            self.errors
                .push("sidebar root has no computed bounds".into());
            return;
        };
        let Some(window) = primary_window_entity(world) else {
            self.errors
                .push("primary window unavailable for sidebar wheel".into());
            return;
        };
        if !set_cursor_physical(world, center) {
            self.errors
                .push("could not position cursor over sidebar".into());
            return;
        }
        world.write_message(MouseWheel {
            unit: MouseScrollUnit::Line,
            x: 0.0,
            y: lines,
            window,
            phase: TouchPhase::Moved,
        });
        self.record(world, now, "wheel", format!("sidebar_lines={lines:.1}"));
    }

    fn start_recovery_hold(&mut self, world: &mut World, now: f64) -> bool {
        let Some(center) = action_center(world, SidebarAction::HoldRecovery) else {
            return false;
        };
        if set_cursor_physical(world, center) {
            set_mouse_button(world, true);
            self.record(world, now, "recovery-hold-start", format!("at {center:?}"));
            true
        } else {
            false
        }
    }

    fn set_phase(&mut self, phase: Phase, now: f64) {
        self.phase = phase;
        self.phase_started_at = Some(now);
    }

    fn phase_elapsed(&self, now: f64) -> f64 {
        self.phase_started_at.map_or(0.0, |started| now - started)
    }

    fn phase_timed_out(&self, now: f64) -> bool {
        !matches!(self.phase, Phase::Warmup | Phase::FinishWait | Phase::Done)
            && self.phase_elapsed(now) > PHASE_TIMEOUT
    }

    fn record(&mut self, world: &World, now: f64, event: &str, detail: impl std::fmt::Display) {
        let elapsed = self.started_at.map_or(0.0, |started| now - started);
        self.events.push(format!(
            "{elapsed:.3},{event},\"{}\"\n",
            csv(&detail.to_string())
        ));
        fs::write(self.directory.join("events.csv"), self.events.concat())
            .expect("write sidebar diagnostic events");
        let _ = background_capture_description(world);
    }

    fn fail(&mut self, world: &mut World, now: f64, detail: String) {
        self.errors.push(detail.clone());
        self.record(world, now, "error", detail);
        self.set_phase(Phase::FinishWait, now);
    }

    fn finish(&mut self, world: &mut World, now: f64) {
        for capture in REQUIRED_CAPTURES {
            let path = self.directory.join("captures").join(capture);
            if !path.is_file() || fs::metadata(&path).is_ok_and(|metadata| metadata.len() == 0) {
                self.errors.push(format!("missing capture {capture}"));
            }
        }
        for (name, passed) in [
            ("sidebar_wheel_no_zoom", self.no_zoom_proved),
            ("selector_open_pauses", self.selector_paused),
            ("selector_cancel_resumes", self.cancel_resumed),
            ("car_choice_resumes", self.car_choice_resumed),
            ("mouse_view_close", self.mouse_view_close),
            ("follow_toggled", self.follow_toggled),
            ("early_recovery_cancels", self.recovery_early_cancelled),
            ("recovery_completes", self.recovery_completed),
        ] {
            if !passed {
                self.errors.push(format!("acceptance check failed: {name}"));
            }
        }
        self.record(world, now, "finish", self.status());
        let status = format!(
            "{}\nselector_open_pauses={}\nselector_cancel_resumes={}\ncar_choice_resumes={}\nmouse_view_close={}\nfollow_toggled={}\nsidebar_wheel_no_zoom={}\nearly_recovery_cancels={}\nrecovery_completes={}\ncaptures={}\nerrors={}\n",
            self.status(),
            self.selector_paused,
            self.cancel_resumed,
            self.car_choice_resumed,
            self.mouse_view_close,
            self.follow_toggled,
            self.no_zoom_proved,
            self.recovery_early_cancelled,
            self.recovery_completed,
            self.captures.len(),
            self.errors.join("; "),
        );
        fs::write(self.directory.join("status.txt"), status)
            .expect("write sidebar diagnostic status");
        self.set_phase(Phase::Done, now);
        self.finished = true;
        let exit = if self.errors.is_empty() {
            AppExit::Success
        } else {
            AppExit::Error(std::num::NonZeroU8::new(1).unwrap())
        };
        world.write_message(exit);
    }
}

pub(super) fn register_sidebar_diagnostic(app: &mut App, directory: PathBuf) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("main crate lives below the workspace root")
        .canonicalize()
        .expect("canonicalize workspace root");
    fs::create_dir_all(&directory).expect("create sidebar diagnostic directory");
    let directory = directory
        .canonicalize()
        .expect("canonicalize sidebar diagnostic directory");
    assert!(
        !directory.starts_with(root),
        "sidebar diagnostic output must be outside the checkout"
    );
    assert!(
        !directory.join("captures").exists()
            && !directory.join("events.csv").exists()
            && !directory.join("status.txt").exists(),
        "sidebar diagnostic outputs must not be reused"
    );
    fs::create_dir_all(directory.join("captures"))
        .expect("create sidebar diagnostic captures directory");
    fs::write(directory.join("events.csv"), "elapsed_s,event,detail\n")
        .expect("write sidebar diagnostic event header");
    fs::write(directory.join("status.txt"), "RUNNING\n")
        .expect("write sidebar diagnostic running status");
    app.insert_resource(SidebarDiagnostic::new(directory))
        .add_systems(Startup, configure_sidebar_window)
        .add_systems(
            PreUpdate,
            drive_sidebar_diagnostic
                .after(InputSystems)
                .before(crate::ui::scroll_sidebar)
                .before(crate::exploration::ExplorationInput)
                .run_if(in_state(shared::state::AppState::Playing)),
        );
}

fn configure_sidebar_window(mut window: Query<&mut Window, With<PrimaryWindow>>) {
    if let Ok(mut window) = window.single_mut() {
        window.resolution.set(1280.0, 420.0);
        window.resolution.set_scale_factor_override(Some(2.0));
    }
}

fn drive_sidebar_diagnostic(world: &mut World) {
    let Some(mut diagnostic) = world.remove_resource::<SidebarDiagnostic>() else {
        return;
    };
    diagnostic.tick(world);
    world.insert_resource(diagnostic);
}

fn playing(world: &World) -> bool {
    world
        .get_resource::<State<shared::state::AppState>>()
        .is_some_and(|state| *state.get() == shared::state::AppState::Playing)
}

fn background_target_ready(world: &World) -> bool {
    world.contains_resource::<super::BackgroundCaptureTarget>()
        && world.contains_resource::<Exploration>()
        && !world.resource::<Time<Virtual>>().is_paused()
}

fn action_center(world: &mut World, action: SidebarAction) -> Option<Vec2> {
    let candidate = {
        let mut actions =
            world.query::<(&ComputedNode, &UiGlobalTransform, &SidebarActionControl)>();
        actions
            .iter(world)
            .find(|(_, _, control)| control.0 == action)
            .map(|(node, transform, _)| {
                let center = transform.affine().transform_point2(Vec2::ZERO);
                (node.size().min_element() > 0.0 && node.contains_point(*transform, center))
                    .then_some(center)
            })
            .flatten()
    }?;
    let mut sidebars = world.query_filtered::<(&ComputedNode, &UiGlobalTransform), With<Sidebar>>();
    sidebars
        .iter(world)
        .any(|(node, transform)| node.contains_point(*transform, candidate))
        .then_some(candidate)
}

fn sidebar_center(world: &mut World) -> Option<Vec2> {
    let mut sidebars = world.query_filtered::<(&ComputedNode, &UiGlobalTransform), With<Sidebar>>();
    sidebars
        .iter(world)
        .next()
        .map(|(_, transform)| transform.affine().transform_point2(Vec2::ZERO))
}

fn primary_window_entity(world: &mut World) -> Option<Entity> {
    let mut windows = world.query_filtered::<Entity, With<PrimaryWindow>>();
    windows.iter(world).next()
}

fn primary_window_center(world: &mut World) -> Option<Vec2> {
    let entity = primary_window_entity(world)?;
    let window = world.get::<Window>(entity)?;
    Some(Vec2::new(
        window.physical_width() as f32 * 0.5,
        window.physical_height() as f32 * 0.5,
    ))
}

fn primary_window_description(world: &mut World) -> String {
    let Some(entity) = primary_window_entity(world) else {
        return "window=missing".into();
    };
    let Some(window) = world.get::<Window>(entity) else {
        return "window=missing".into();
    };
    format!(
        "window_physical={}x{} logical={:.1}x{:.1} scale_factor={:.2}",
        window.physical_width(),
        window.physical_height(),
        window.width(),
        window.height(),
        window.scale_factor(),
    )
}

fn main_camera_description(world: &mut World) -> String {
    let mut cameras = world.query_filtered::<&Camera, With<crate::map::MainCamera>>();
    let Some(camera) = cameras.iter(world).next() else {
        return "camera_target=missing".into();
    };
    format!(
        "camera_target_physical={:?} logical={:?} scale_factor={:?}",
        camera.physical_target_size(),
        camera.logical_target_size(),
        camera.target_scaling_factor(),
    )
}

fn main_camera_count(world: &mut World) -> usize {
    let mut cameras = world.query_filtered::<Entity, With<crate::map::MainCamera>>();
    cameras.iter(world).count()
}

fn set_cursor_physical(world: &mut World, physical_position: Vec2) -> bool {
    let Some(entity) = primary_window_entity(world) else {
        return false;
    };
    let Some(mut window) = world.get_mut::<Window>(entity) else {
        return false;
    };
    let scale = window.scale_factor().max(f32::EPSILON);
    window.set_cursor_position(Some(physical_position / scale));
    true
}

fn set_mouse_button(world: &mut World, pressed: bool) {
    if let Some(mut mouse) = world.get_resource_mut::<ButtonInput<MouseButton>>() {
        if pressed {
            mouse.press(MouseButton::Left);
        } else {
            mouse.release(MouseButton::Left);
        }
    }
}

fn sidebar_scroll(world: &mut World) -> Option<f32> {
    let mut areas = world.query_filtered::<&ScrollPosition, With<SidebarScrollArea>>();
    areas.iter(world).next().map(|scroll| scroll.0.y)
}

fn readout_text(world: &mut World, slot: SidebarReadoutSlot) -> Option<String> {
    let mut rows = world.query::<(&Text, &SidebarReadoutSlot)>();
    rows.iter(world)
        .find(|(_, candidate)| **candidate == slot)
        .map(|(text, _)| text.0.clone())
}

fn recovery_percent(world: &mut World) -> Option<u32> {
    let text = readout_text(world, SidebarReadoutSlot::RecoveryContext)?;
    let before_percent = text.split('%').next()?;
    before_percent.rsplit_once(':')?.1.trim().parse().ok()
}

fn csv(value: &str) -> String {
    value.replace('"', "\"\"").replace('\n', " ")
}
