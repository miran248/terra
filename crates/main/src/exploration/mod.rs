//! Default exploration lifecycle. The explorer and reusable vehicles keep distinct bodies.
#[cfg(test)]
mod diagnostics;
mod interface;
mod placement;
#[cfg(feature = "asset-review")]
pub(crate) mod showcase;
mod view;
mod world;
use crate::{
    map::{Ground, MainCamera, Player},
    physics::{RadialGravity, RadialUpright},
};
use avian3d::prelude::*;
use bevy::{input::InputSystems, prelude::*};
use placement::Placement;
use shared::{
    car_prototype::CarMotion,
    plane_prototype::{FlightInput, PlaneFlight, gentle_landing},
    planet::PlanetMesh,
    planet_view::PlanetViewCamera,
    planet_view_interface::{GameplayHudElement, PlanetViewPointer, PlanetViewPresentation},
    sphere::tangent_heading as tangent,
    state::AppState,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Car,
    Plane,
}
impl Kind {
    fn index(self) -> usize {
        usize::from(self == Self::Plane)
    }
    fn height(self) -> f32 {
        if self == Self::Car { 0.4 } else { 0.5 }
    }
    fn summon_radius(self) -> f32 {
        if self == Self::Car { 30.0 } else { 100.0 }
    }
    fn summon_search_radius(self) -> f32 {
        self.summon_radius() - 8.0
    }
    fn summon_search_restart_distance(self) -> f32 {
        // Keep placements local to the request while allowing several search
        // slices of ordinary walking before rebuilding candidate order.
        (self.summon_search_radius() * 0.125).min(3.0)
    }
    fn name(self) -> &'static str {
        if self == Self::Car { "Car" } else { "Plane" }
    }
    fn asset(self) -> &'static str {
        if self == Self::Car {
            "vehicle.car"
        } else {
            "vehicle.plane"
        }
    }
    fn collider(self) -> Collider {
        crate::asset_collision::collider_with_origin(
            self.asset(),
            Vec3::ONE,
            Vec3::Y * self.height(),
        )
        .expect("vehicle collision contract")
    }
}

/// The stable LevelData collection used by a named Planet-view destination.
/// Indices distinguish separate places even if they share a display name or
/// coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PlanetDestinationCollection {
    Region,
    Settlement,
    Road,
    Bridge,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum PlanetDestinationKey {
    Surface([u32; 3]),
    CollectionIndex {
        collection: PlanetDestinationCollection,
        index: u32,
    },
}

/// World-local destination identity that does not depend on UI entities or names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PlanetDestinationId {
    world_epoch: crate::map::WorldEpoch,
    key: PlanetDestinationKey,
}

impl PlanetDestinationId {
    pub fn surface(world_epoch: crate::map::WorldEpoch, position: Vec3) -> Self {
        Self {
            world_epoch,
            key: PlanetDestinationKey::Surface(position.to_array().map(f32::to_bits)),
        }
    }

    pub fn region(world_epoch: crate::map::WorldEpoch, region_index: u32) -> Self {
        Self::collection(
            world_epoch,
            PlanetDestinationCollection::Region,
            region_index,
        )
    }

    pub fn collection(
        world_epoch: crate::map::WorldEpoch,
        collection: PlanetDestinationCollection,
        index: u32,
    ) -> Self {
        Self {
            world_epoch,
            key: PlanetDestinationKey::CollectionIndex { collection, index },
        }
    }

    pub const fn world_epoch(self) -> crate::map::WorldEpoch {
        self.world_epoch
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanetDestinationSurface {
    Terrain,
    BridgeDeck,
}

/// Selection data copied out of the picking and UI layers.
#[derive(Clone, Debug, PartialEq)]
pub struct PlanetDestination {
    pub id: PlanetDestinationId,
    pub position: Vec3,
    pub surface: PlanetDestinationSurface,
    pub display: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PlanetTeleportRequestId(u64);

/// Immutable selection and world snapshot associated with one T confirmation.
#[derive(Clone, Debug, PartialEq)]
pub struct PlanetTeleportRequest {
    pub id: PlanetTeleportRequestId,
    pub world_epoch: crate::map::WorldEpoch,
    pub destination: PlanetDestination,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanetTeleportRejection {
    UnsafeLanding,
    OccupiedVehicle,
    VehicleSelectorOpen,
    WorldUnavailable,
}

impl PlanetTeleportRejection {
    pub const fn message(self) -> &'static str {
        match self {
            Self::UnsafeLanding => "No safe on-foot landing is available at this exact spot.",
            Self::OccupiedVehicle => "Exit your vehicle, then confirm again.",
            Self::VehicleSelectorOpen => "Close the vehicle selector, then confirm again.",
            Self::WorldUnavailable => "The world is not ready. Try confirming again shortly.",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanetTeleportCancellation {
    ViewClosed,
    DestinationChanged,
    WorldReloaded,
    VehicleSelectorOpened,
    StaleWorld,
}

impl PlanetTeleportCancellation {
    pub const fn message(self) -> &'static str {
        match self {
            Self::ViewClosed => "Teleport request cancelled. Press T to check again.",
            Self::DestinationChanged => "Destination changed. Confirm the new choice with T.",
            Self::WorldReloaded => "World changed. Select a destination in the new world.",
            Self::VehicleSelectorOpened => "Request cancelled while choosing a vehicle.",
            Self::StaleWorld => "This destination belongs to an earlier world. Select it again.",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PlanetTeleportOutcomeKind {
    Succeeded { position: Vec3 },
    Rejected { reason: PlanetTeleportRejection },
    Cancelled { reason: PlanetTeleportCancellation },
}

/// Typed result tied to the exact confirmation that produced it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlanetTeleportOutcome {
    pub request_id: PlanetTeleportRequestId,
    pub world_epoch: crate::map::WorldEpoch,
    pub destination_id: PlanetDestinationId,
    pub result: PlanetTeleportOutcomeKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanetTeleportStatus {
    Checking {
        request_id: PlanetTeleportRequestId,
        destination_id: PlanetDestinationId,
    },
    Rejected {
        request_id: PlanetTeleportRequestId,
        destination_id: PlanetDestinationId,
        reason: PlanetTeleportRejection,
    },
    Cancelled {
        request_id: PlanetTeleportRequestId,
        destination_id: PlanetDestinationId,
        reason: PlanetTeleportCancellation,
    },
}

#[derive(Component)]
struct Vehicle {
    kind: Kind,
    flight: PlaneFlight,
    parked: bool,
    crashed: bool,
    stable: f32,
    air_time: f32,
    previous_velocity: Vec3,
    clearance: f32,
    support_normal: Option<Vec3>,
}
impl Vehicle {
    fn new(kind: Kind, heading: Vec3) -> Self {
        Self {
            kind,
            flight: PlaneFlight::new(heading),
            parked: true,
            crashed: false,
            stable: 0.0,
            air_time: 0.0,
            previous_velocity: Vec3::ZERO,
            clearance: 0.0,
            support_normal: None,
        }
    }
}
#[derive(Resource, Default)]
pub struct Exploration {
    occupied: Option<Entity>,
    vehicles: [Option<Entity>; 2],
    start: Option<Vec3>,
    safe: Option<Vec3>,
    selector: bool,
    suppress_input: bool,
    recovery: f32,
    recover_latched: bool,
    target: Option<Entity>,
    message: String,
    actions: std::collections::VecDeque<Action>,
    snap_camera: bool,
    planet_camera: PlanetViewCamera,
    planet_orbit_intent: Vec2,
    planet_zoom_intent: f32,
    planet_presentation: PlanetViewPresentation,
    planet_pointer: PlanetViewPointer,
    pressed_planet_control: Option<Entity>,
    planet_selection_click: Option<Vec2>,
    planet_teleport_requested: bool,
    selected_destination: Option<PlanetDestination>,
    next_planet_teleport_id: u64,
    pending_planet_teleport: Option<PlanetTeleportRequest>,
    planet_teleport_status: Option<PlanetTeleportStatus>,
    planet_teleport_outcomes: std::collections::VecDeque<PlanetTeleportOutcome>,
    pending_summon: Option<PendingSummonSearch>,
}
#[derive(Clone, Copy, PartialEq)]
pub enum Action {
    Summon(Kind),
    Interact,
    Recover,
    Teleport(Vec3),
    PlanetTeleport(PlanetTeleportRequestId),
}

struct PendingSummonSearch {
    kind: Kind,
    body_origin: Vec3,
    world_epoch: Option<u64>,
    existing: Option<Entity>,
    excluded: Vec<Entity>,
    shape: Collider,
    candidates: shared::placement::PlacementCandidateSearch,
    locate_ms: Option<f64>,
    dispatch_ms: Option<f64>,
    old_position: Option<Vec3>,
    fixed_elapsed_s_before: f64,
    resident_obstacles_before: usize,
    world_ready_before: bool,
    #[cfg(test)]
    poses_tested_last_update: usize,
    #[cfg(test)]
    poses_tested_total: usize,
    pose_budget_remaining: usize,
}

const SUMMON_POSE_BUDGET_PER_UPDATE: usize = 32;
// A moving body may replace an old cursor, but repeated restarts cannot consume
// unbounded work or keep later exploration actions queued forever.
const SUMMON_SEARCH_PASSES_PER_REQUEST: usize = 2;

impl Exploration {
    pub fn request(&mut self, action: Action) {
        if self.actions.is_empty() {
            self.pending_summon = None;
        }
        self.actions.push_back(action);
    }

    fn clear_actions(&mut self) {
        self.actions.clear();
        self.pending_summon = None;
    }

    pub fn set_planet_view_open(&mut self, open: bool) {
        if self.planet_camera.is_requested_open() != open {
            self.planet_camera.toggle();
        }
        self.planet_presentation.request_open(open);
        if !open {
            self.planet_pointer.cancel();
            self.pressed_planet_control = None;
            self.planet_selection_click = None;
            self.cancel_planet_teleport(PlanetTeleportCancellation::ViewClosed);
        }
    }

    pub fn is_planet_view_active(&self) -> bool {
        self.planet_camera.is_active()
    }

    pub(crate) fn is_vehicle_selector_open(&self) -> bool {
        self.selector
    }

    /// Selection and teleport are enabled only after the interface fully opens.
    pub fn planet_view_ready(&self) -> bool {
        !self.selector && self.planet_presentation.selection_ready()
    }

    /// Request selection at a physical screen position while Planet view is ready.
    pub fn request_planet_view_selection(&mut self, physical_position: Vec2) -> bool {
        if !self.planet_view_ready() || !physical_position.is_finite() {
            return false;
        }
        self.planet_selection_click = Some(physical_position);
        true
    }

    /// Take the pending physical-pixel selection position for the destination consumer.
    pub fn take_planet_view_selection(&mut self) -> Option<Vec2> {
        self.planet_selection_click.take()
    }

    /// Destination snapshot selected in the current world load, if any.
    pub fn selected_planet_destination(&self) -> Option<&PlanetDestination> {
        self.selected_destination.as_ref()
    }

    pub(crate) fn select_planet_destination(&mut self, destination: PlanetDestination) {
        self.cancel_planet_teleport(PlanetTeleportCancellation::DestinationChanged);
        self.planet_teleport_status = None;
        self.selected_destination = Some(destination);
    }

    pub(crate) fn clear_planet_destination(&mut self) {
        self.selected_destination = None;
    }

    /// Whether teleport needs the explorer to exit the currently occupied vehicle.
    pub fn destination_requires_exit_from_vehicle(&self) -> bool {
        self.selected_destination.is_some() && self.occupied.is_some()
    }

    /// Request the shared T/selection teleport action while Planet view is ready.
    pub fn request_planet_view_teleport(&mut self) -> bool {
        if !self.planet_view_ready()
            || self.selected_destination.is_none()
            || self.planet_teleport_requested
            || self.pending_planet_teleport.is_some()
        {
            return false;
        }
        self.planet_teleport_requested = true;
        true
    }

    /// Take the pending T intent for the destination consumer.
    pub fn take_planet_view_teleport_request(&mut self) -> bool {
        std::mem::take(&mut self.planet_teleport_requested)
    }

    pub fn pending_planet_teleport_request(&self) -> Option<&PlanetTeleportRequest> {
        self.pending_planet_teleport.as_ref()
    }

    pub fn planet_teleport_status(&self) -> Option<PlanetTeleportStatus> {
        self.planet_teleport_status
    }

    pub fn take_planet_teleport_outcome(&mut self) -> Option<PlanetTeleportOutcome> {
        self.planet_teleport_outcomes.pop_front()
    }

    fn submit_planet_teleport(&mut self, world_epoch: crate::map::WorldEpoch) {
        if !self.take_planet_view_teleport_request() {
            return;
        }
        let Some(destination) = self.selected_destination.clone() else {
            return;
        };
        let id = PlanetTeleportRequestId(self.next_planet_teleport_id);
        self.next_planet_teleport_id = self
            .next_planet_teleport_id
            .checked_add(1)
            .expect("planet teleport request ID exhausted");
        let request = PlanetTeleportRequest {
            id,
            world_epoch,
            destination,
        };
        if request.world_epoch != request.destination.id.world_epoch() {
            self.finish_planet_teleport(
                request,
                PlanetTeleportOutcomeKind::Cancelled {
                    reason: PlanetTeleportCancellation::StaleWorld,
                },
            );
            self.clear_planet_destination();
            return;
        }
        self.pending_planet_teleport = Some(request.clone());
        self.planet_teleport_status = Some(PlanetTeleportStatus::Checking {
            request_id: request.id,
            destination_id: request.destination.id,
        });
        self.actions.push_back(Action::PlanetTeleport(request.id));
    }

    fn cancel_planet_teleport(&mut self, reason: PlanetTeleportCancellation) {
        self.planet_teleport_requested = false;
        let Some(request) = self.pending_planet_teleport.take() else {
            return;
        };
        self.actions
            .retain(|action| *action != Action::PlanetTeleport(request.id));
        self.planet_teleport_status = Some(PlanetTeleportStatus::Cancelled {
            request_id: request.id,
            destination_id: request.destination.id,
            reason,
        });
        self.planet_teleport_outcomes
            .push_back(PlanetTeleportOutcome {
                request_id: request.id,
                world_epoch: request.world_epoch,
                destination_id: request.destination.id,
                result: PlanetTeleportOutcomeKind::Cancelled { reason },
            });
    }

    fn finish_planet_teleport(
        &mut self,
        request: PlanetTeleportRequest,
        result: PlanetTeleportOutcomeKind,
    ) {
        self.pending_planet_teleport = None;
        self.actions
            .retain(|action| *action != Action::PlanetTeleport(request.id));
        self.planet_teleport_status = match result {
            PlanetTeleportOutcomeKind::Succeeded { .. } => None,
            PlanetTeleportOutcomeKind::Rejected { reason } => {
                Some(PlanetTeleportStatus::Rejected {
                    request_id: request.id,
                    destination_id: request.destination.id,
                    reason,
                })
            }
            PlanetTeleportOutcomeKind::Cancelled { reason } => {
                Some(PlanetTeleportStatus::Cancelled {
                    request_id: request.id,
                    destination_id: request.destination.id,
                    reason,
                })
            }
        };
        self.planet_teleport_outcomes
            .push_back(PlanetTeleportOutcome {
                request_id: request.id,
                world_epoch: request.world_epoch,
                destination_id: request.destination.id,
                result,
            });
    }

    /// Current opacity for gameplay HUD and minimap presentation.
    pub fn gameplay_hud_opacity(&self) -> f32 {
        self.planet_presentation.gameplay_opacity()
    }

    /// Current opacity for controls shown while Planet view is open.
    pub fn planet_view_interface_opacity(&self) -> f32 {
        self.planet_presentation.interface_opacity()
    }

    /// Whether non-modal Planet-view controls and overlays may be shown or used.
    pub fn planet_view_interface_visible(&self) -> bool {
        !self.selector && self.planet_presentation.interface_opacity() > 0.0
    }

    pub fn planet_view_follows_body(&self) -> bool {
        self.planet_camera.follows_body()
    }

    /// Queue a planet-view orbit using the same logical-pixel delta as a drag.
    pub fn request_planet_view_orbit(&mut self, logical_delta: Vec2) -> bool {
        if self.selector
            || !self.planet_camera.is_active()
            || !logical_delta.is_finite()
            || logical_delta == Vec2::ZERO
        {
            return false;
        }
        self.planet_orbit_intent += logical_delta;
        true
    }

    /// Queue normalized wheel input after line/pixel unit conversion; positive values zoom in.
    pub fn request_planet_view_zoom(&mut self, wheel_delta: f32) -> bool {
        if self.selector
            || !self.planet_camera.is_active()
            || !wheel_delta.is_finite()
            || wheel_delta == 0.0
        {
            return false;
        }
        self.planet_zoom_intent += wheel_delta;
        true
    }

    /// Requested and attained radial camera distances in meters.
    pub fn planet_view_camera_radii(&self) -> (f32, f32) {
        (
            self.planet_camera.requested_radius(),
            self.planet_camera.attained_radius(),
        )
    }

    pub fn toggle_planet_view_follow(&mut self, camera_pose: Transform) {
        self.planet_camera.toggle_follow_from(camera_pose);
    }

    /// Existing reusable vehicle for this kind, if it has been summoned.
    pub fn vehicle_entity(&self, kind: Kind) -> Option<Entity> {
        self.vehicles[kind.index()]
    }

    /// A successful current summon result for this vehicle kind.
    #[cfg(feature = "asset-review")]
    pub(crate) fn vehicle_summon_ready(&self, kind: Kind) -> bool {
        !self
            .actions
            .iter()
            .any(|action| matches!(action, Action::Summon(pending) if *pending == kind))
            && self.message == format!("{} ready — approach and press E", kind.name())
    }

    #[cfg(feature = "asset-review")]
    pub(crate) fn action_pending(&self, action: Action) -> bool {
        self.actions.contains(&action)
    }

    /// Whether the explorer currently controls a vehicle.
    pub fn is_in_vehicle(&self) -> bool {
        self.occupied.is_some()
    }
}
#[derive(Resource)]
struct Liquid(PlanetMesh);
type ExplorationBody = Or<(With<Player>, With<Vehicle>)>;

#[derive(Component)]
struct Seated;
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExplorationUpdate;
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ExplorationInput;

pub struct ExplorationPlugin;
impl Plugin for ExplorationPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Exploration>()
            .add_systems(
                OnEnter(AppState::Playing),
                (world::setup, initialize, view::setup, interface::setup)
                    .chain()
                    .after(crate::map::setup_map),
            )
            .add_systems(
                PreUpdate,
                (
                    input,
                    interface::pointer_input,
                    crate::minimap::consume_planet_view_destination_click,
                    submit_planet_teleport,
                    world::residency,
                )
                    .chain()
                    .in_set(ExplorationInput)
                    .after(InputSystems)
                    .run_if(in_state(AppState::Playing)),
            )
            .add_systems(
                FixedUpdate,
                (drive, align).chain().run_if(in_state(AppState::Playing)),
            )
            .add_systems(
                FixedPostUpdate,
                contacts
                    .after(PhysicsSystems::Last)
                    .run_if(in_state(AppState::Playing)),
            )
            .add_systems(
                Update,
                (
                    actions,
                    sync_explorer,
                    track_safe,
                    interface::update_presentation,
                    view::tag_visuals,
                    view::camera,
                    view::animate,
                    view::readout,
                )
                    .chain()
                    .in_set(ExplorationUpdate)
                    .run_if(in_state(AppState::Playing)),
            );
    }
}
pub fn on_foot(state: Option<Res<Exploration>>) -> bool {
    state.is_none_or(|s| s.occupied.is_none() && !s.selector && !s.suppress_input)
}
fn facing(heading: Vec3, up: Vec3) -> Quat {
    Quat::from_mat3(&Mat3::from_cols(heading.cross(up), up, -heading))
}
fn initialize(mut state: ResMut<Exploration>, player: Query<&Position, With<Player>>) {
    state.cancel_planet_teleport(PlanetTeleportCancellation::WorldReloaded);
    state.planet_teleport_status = None;
    state.clear_planet_destination();
    if let Ok(p) = player.single() {
        state.start = Some(p.0);
        state.snap_camera = true;
    }
}

fn submit_planet_teleport(
    mut state: ResMut<Exploration>,
    world_epoch: Res<crate::map::WorldEpoch>,
) {
    state.submit_planet_teleport(*world_epoch);
}

fn input(
    keys: Res<ButtonInput<KeyCode>>,
    real: Res<Time<Real>>,
    mut time: ResMut<Time<Virtual>>,
    mut state: ResMut<Exploration>,
    cameras: Query<&Transform, With<MainCamera>>,
) {
    if state.suppress_input
        && [
            KeyCode::KeyW,
            KeyCode::KeyS,
            KeyCode::KeyA,
            KeyCode::KeyD,
            KeyCode::Space,
            KeyCode::ShiftLeft,
            KeyCode::ShiftRight,
            KeyCode::ControlLeft,
            KeyCode::ControlRight,
        ]
        .iter()
        .all(|k| !keys.pressed(*k))
    {
        state.suppress_input = false;
    }
    if state.selector {
        state.planet_selection_click = None;
        state.planet_teleport_requested = false;
        if keys.just_pressed(KeyCode::Escape) || keys.just_pressed(KeyCode::KeyV) {
            state.selector = false;
            time.unpause();
            state.suppress_input = true;
            state.recovery = 0.0;
            return;
        }
        let kind = if keys.just_pressed(KeyCode::KeyC) {
            Some(Kind::Car)
        } else if keys.just_pressed(KeyCode::KeyP) {
            Some(Kind::Plane)
        } else {
            None
        };
        if let Some(kind) = kind {
            state.request(Action::Summon(kind));
            state.selector = false;
            time.unpause();
            state.suppress_input = true;
        }
        state.recovery = 0.0;
        return;
    }
    if keys.just_pressed(KeyCode::KeyV) {
        if state.planet_camera.is_active() {
            // Vehicle selection is reserved for ordinary exploration. Ignore
            // the press for this frame so closing Planet view cannot defer it.
        } else if state.occupied.is_some() {
            state.message = "Stop and exit before summoning; hold R to recover if trapped".into();
        } else {
            state.selector = true;
            state.planet_selection_click = None;
            state.cancel_planet_teleport(PlanetTeleportCancellation::VehicleSelectorOpened);
            time.pause();
            state.recovery = 0.0;
            return;
        }
    }
    if keys.just_pressed(KeyCode::KeyM) {
        let open = !state.planet_camera.is_requested_open();
        state.set_planet_view_open(open);
    }
    if keys.just_pressed(KeyCode::Escape) && state.planet_camera.is_requested_open() {
        state.set_planet_view_open(false);
    }
    if keys.just_pressed(KeyCode::KeyF) && state.planet_camera.is_requested_open() {
        if let Ok(camera) = cameras.single() {
            state.toggle_planet_view_follow(*camera);
        }
    }
    if keys.just_pressed(KeyCode::KeyT) {
        state.request_planet_view_teleport();
    }
    if keys.just_pressed(KeyCode::KeyE) {
        state.request(Action::Interact);
    }
    if keys.pressed(KeyCode::KeyR) && !state.recover_latched {
        state.recovery += real.delta_secs().min(0.1);
        if state.recovery >= 1.0 {
            state.request(Action::Recover);
            state.recover_latched = true;
        }
    } else if !keys.pressed(KeyCode::KeyR) {
        state.recovery = 0.0;
        state.recover_latched = false;
    }
}
fn drive(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    state: Res<Exploration>,
    placement: Placement,
    collisions: Collisions,
    #[cfg(feature = "asset-review")] pilot: Option<Res<showcase::PilotControls>>,
    mut vehicles: Query<(Entity, &Position, &mut Vehicle, Forces)>,
) {
    for (entity, pos, mut vehicle, mut forces) in &mut vehicles {
        let up = pos.0.normalize();
        let mass = if vehicle.kind == Kind::Car {
            800.0
        } else {
            900.0
        };
        if vehicle.crashed {
            forces.apply_force(-up * mass * 20.0);
            continue;
        }
        if vehicle.parked {
            continue;
        }
        let controlled = state.occupied == Some(entity) && !state.selector && !state.suppress_input;
        let support = placement.support(entity, pos.0, vehicle.flight.heading, vehicle.kind);
        let axis = |a, b| {
            if controlled {
                f32::from(keys.pressed(a)) - f32::from(keys.pressed(b))
            } else {
                0.0
            }
        };
        #[cfg(feature = "asset-review")]
        let scripted = pilot
            .as_ref()
            .filter(|p| p.entity == Some(entity) && controlled);
        let car_axes = (
            axis(KeyCode::KeyW, KeyCode::KeyS),
            axis(KeyCode::KeyA, KeyCode::KeyD),
        );
        #[cfg(feature = "asset-review")]
        let car_axes = scripted.map_or(car_axes, |p| (p.throttle, p.steering));
        if vehicle.kind == Kind::Car {
            let velocity = forces.linear_velocity();
            // At a slope boundary the nose can already touch the next facet
            // while the ground probe still sees the old one. Drive along the
            // touching surface opposing motion, rather than pushing into it.
            let ground = collisions
                .collisions_with(entity)
                .filter(|pair| {
                    !placement.obstacle(if pair.collider1 == entity {
                        pair.collider2
                    } else {
                        pair.collider1
                    })
                })
                .flat_map(|pair| {
                    pair.manifolds.iter().map(move |manifold| {
                        if pair.collider1 == entity {
                            -manifold.normal
                        } else {
                            manifold.normal
                        }
                    })
                })
                .filter(|normal| normal.dot(up) > 0.7)
                .min_by(|a, b| a.dot(velocity).total_cmp(&b.dot(velocity)))
                .or(support);
            let motion = CarMotion {
                velocity: forces.linear_velocity(),
                heading: vehicle.flight.heading,
            }
            .step(time.delta_secs(), car_axes.0, car_axes.1, up, ground);
            vehicle.flight.heading = motion.heading;
            *forces.linear_velocity_mut() = motion.velocity;
            forces.apply_force(-up * mass * 20.0);
        } else {
            let input = FlightInput {
                pitch: axis(KeyCode::KeyS, KeyCode::KeyW),
                bank: axis(KeyCode::KeyA, KeyCode::KeyD),
                throttle: if controlled
                    && (keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight))
                {
                    1.0
                } else if controlled
                    && (keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight))
                {
                    -1.0
                } else {
                    0.0
                },
                brake: controlled && keys.pressed(KeyCode::Space),
            };
            #[cfg(feature = "asset-review")]
            let input = scripted.map_or(input, |p| p.flight);
            let velocity = vehicle.flight.step(
                time.delta_secs(),
                input,
                forces.linear_velocity(),
                up,
                support,
            );
            *forces.linear_velocity_mut() = velocity;
            vehicle.previous_velocity = velocity;
            if !vehicle.flight.airborne {
                forces.apply_force(-up * mass * 20.0);
            }
            vehicle.air_time = if vehicle.flight.airborne {
                vehicle.air_time + time.delta_secs()
            } else {
                0.0
            };
        }
    }
}
fn align(time: Res<Time>, mut vehicles: Query<(&Position, &Vehicle, &mut Rotation)>) {
    for (pos, v, mut rotation) in &mut vehicles {
        if v.crashed {
            continue;
        }
        if v.kind == Kind::Car {
            // Keep the last attitude through short hops; contact determines pitch and roll.
            let normal = v.support_normal.unwrap_or(rotation.0 * Vec3::Y);
            let desired = facing(tangent(v.flight.heading, normal), normal);
            // Avoid continually waking a resting body for sub-millimeter ray noise.
            if rotation.0.angle_between(desired) > 0.001 {
                rotation.0 = rotation
                    .0
                    .slerp(desired, 1.0 - (-12.0 * time.delta_secs()).exp());
            }
        } else {
            rotation.0 = v.flight.rotation(pos.0.normalize());
        }
    }
}
fn contacts(
    mut commands: Commands,
    time: Res<Time>,
    placement: Placement,
    disabled: Query<(), With<ColliderDisabled>>,
    mut vehicles: Query<(
        Entity,
        &Position,
        &LinearVelocity,
        &CollidingEntities,
        &mut Vehicle,
    )>,
) {
    for (e, p, velocity, contacts, mut v) in &mut vehicles {
        let support = placement.support(e, p.0, v.flight.heading, v.kind);
        v.support_normal = support.and_then(|normal| {
            if v.kind == Kind::Car {
                placement
                    .car_attitude(e, p.0, v.flight.heading)
                    .or(Some(normal))
            } else {
                Some(normal)
            }
        });
        v.clearance = placement.clearance(e, p.0);
        let landed = v.kind == Kind::Car || !v.flight.airborne || v.crashed;
        let touching_ground = v.parked
            || contacts
                .0
                .iter()
                .any(|e| !disabled.contains(*e) && !placement.obstacle(*e));
        v.stable = if landed && touching_ground && support.is_some() && velocity.length() < 0.5 {
            v.stable + time.delta_secs()
        } else {
            0.0
        };
        if v.kind != Kind::Plane || v.crashed || v.parked {
            continue;
        }
        // Disabling the seated explorer's collider can leave a contact from
        // the previous physics step; it is no longer an obstacle.
        let obstacle = contacts
            .0
            .iter()
            .any(|e| !disabled.contains(*e) && placement.obstacle(*e));
        let wet = placement.wet(p.0, 0.5);
        let gentle = !wet
            && !obstacle
            && support.is_some_and(|n| {
                gentle_landing(
                    v.previous_velocity,
                    n,
                    p.0.normalize(),
                    v.flight.pitch,
                    v.flight.bank,
                )
            });
        // The tail can still touch the runway after the support rays lose reach.
        // Only upward departure gets grace; impacts and obstacles never do.
        let departing = v.air_time <= 0.3
            && v.previous_velocity.dot(p.0.normalize()) > 0.0
            && v.flight.pitch >= 0.0
            && v.flight.bank.abs() <= 0.35;
        if wet || obstacle || (v.flight.airborne && touching_ground && !departing) {
            if gentle {
                v.flight.airborne = false;
                v.flight.stalled = false;
                v.flight.pitch = 0.0;
                v.flight.bank = 0.0;
                v.flight.throttle = 0.0;
            } else {
                v.crashed = true;
                v.flight.throttle = 0.0;
                commands.entity(e).remove::<LockedAxes>().insert((
                    Friction::new(0.6),
                    Restitution::new(0.1),
                    LinearDamping(0.1),
                    AngularDamping(1.5),
                ));
            }
        }
    }
}
fn physics_reset() -> impl Bundle {
    (
        LockedAxes::ROTATION_LOCKED,
        AngularVelocity::ZERO,
        LinearVelocity::ZERO,
        Friction::ZERO,
        Restitution::ZERO,
        LinearDamping(0.0),
        AngularDamping(0.0),
    )
}

#[allow(clippy::too_many_arguments)]
fn process_summon_action(
    commands: &mut Commands,
    state: &mut Exploration,
    placement: &Placement,
    origin: Vec3,
    heading: Vec3,
    vehicles: &mut Query<(Entity, &Position, &Rotation, &Collider, &mut Vehicle)>,
    catalog: Option<&crate::asset_catalog::AssetCatalog>,
    world: Option<&world::CollisionWorld>,
    world_epoch: Option<crate::map::WorldEpoch>,
    real_elapsed_s: f64,
    fixed_elapsed_s: f64,
    mut work_trace: Option<&mut crate::chunks::PlanetWorkTrace>,
) {
    let Some(Action::Summon(kind)) = state.actions.front().copied() else {
        state.pending_summon = None;
        return;
    };

    let tracing = work_trace.as_ref().is_some_and(|trace| trace.active());
    let dispatch_started = tracing.then(std::time::Instant::now);
    let current_epoch = world_epoch.map(crate::map::WorldEpoch::value);
    let existing = state.vehicles[kind.index()];
    let current_obstacles = world.map_or(0, world::CollisionWorld::resident_obstacle_count);
    let current_world_ready = world.is_none_or(|collision_world| collision_world.ready);

    if state.occupied.is_some() {
        state.actions.pop_front();
        state.pending_summon = None;
        state.message = "Exit before summoning".into();
        if tracing && let Some(trace) = work_trace.as_deref_mut() {
            trace.record_vehicle_summon(
                real_elapsed_s,
                real_elapsed_s,
                crate::chunks::PlanetVehicleActionSample {
                    kind: match kind {
                        Kind::Car => "car",
                        Kind::Plane => "plane",
                    },
                    existing_entity: existing.is_some(),
                    old_position: None,
                    new_position: None,
                    locate_ms: None,
                    dispatch_ms: dispatch_started
                        .map_or(0.0, |started| started.elapsed().as_secs_f64() * 1000.0),
                    scene_child_spawned: false,
                    fixed_elapsed_s_before: fixed_elapsed_s,
                    fixed_elapsed_s_after: fixed_elapsed_s,
                    resident_obstacles_before: current_obstacles,
                    world_ready_before: current_world_ready,
                },
            );
        }
        return;
    }

    let restart = state.pending_summon.as_ref().is_none_or(|pending| {
        pending.kind != kind
            || pending.existing != existing
            || pending.world_epoch != current_epoch
            || origin.distance(pending.body_origin) > kind.summon_search_restart_distance()
    });
    if restart {
        let prior = state.pending_summon.take();
        let same_request = prior.as_ref().is_some_and(|pending| {
            pending.kind == kind
                && pending.existing == existing
                && pending.world_epoch == current_epoch
        });
        let excluded = existing.into_iter().collect::<Vec<_>>();
        let preferred = origin + tangent(heading, origin.normalize()) * 8.0;
        let radius = kind.summon_search_radius();
        let candidates =
            shared::placement::PlacementCandidateSearch::vehicle(preferred, heading, radius);
        let pose_budget_remaining = prior.as_ref().filter(|_| same_request).map_or_else(
            || {
                candidates
                    .len()
                    .saturating_mul(SUMMON_SEARCH_PASSES_PER_REQUEST)
            },
            |pending| pending.pose_budget_remaining,
        );
        let mut locate_ms = prior
            .as_ref()
            .filter(|_| same_request)
            .and_then(|pending| pending.locate_ms);
        let shape_started = tracing.then(std::time::Instant::now);
        let shape = kind.collider();
        if let Some(started) = shape_started {
            *locate_ms.get_or_insert(0.0) += started.elapsed().as_secs_f64() * 1000.0;
        }
        state.pending_summon = Some(PendingSummonSearch {
            kind,
            body_origin: origin,
            world_epoch: current_epoch,
            existing,
            excluded,
            shape,
            candidates,
            locate_ms,
            dispatch_ms: prior
                .as_ref()
                .filter(|_| same_request)
                .and_then(|pending| pending.dispatch_ms),
            old_position: prior
                .as_ref()
                .filter(|_| same_request)
                .and_then(|pending| pending.old_position)
                .or_else(|| {
                    tracing
                        .then(|| {
                            existing.and_then(|entity| {
                                vehicles.get(entity).ok().map(|(_, p, _, _, _)| p.0)
                            })
                        })
                        .flatten()
                }),
            fixed_elapsed_s_before: prior
                .as_ref()
                .filter(|_| same_request)
                .map_or(fixed_elapsed_s, |pending| pending.fixed_elapsed_s_before),
            resident_obstacles_before: prior
                .as_ref()
                .filter(|_| same_request)
                .map_or(current_obstacles, |pending| {
                    pending.resident_obstacles_before
                }),
            world_ready_before: prior
                .as_ref()
                .filter(|_| same_request)
                .map_or(current_world_ready, |pending| pending.world_ready_before),
            pose_budget_remaining,
            #[cfg(test)]
            poses_tested_last_update: 0,
            #[cfg(test)]
            poses_tested_total: prior
                .filter(|_| same_request)
                .map_or(0, |pending| pending.poses_tested_total),
        });
    }

    let pending = state
        .pending_summon
        .as_mut()
        .expect("summon search initialized");
    #[cfg(test)]
    {
        pending.poses_tested_last_update = 0;
    }
    let locate_started = tracing.then(std::time::Instant::now);
    let mut found = None;
    let pose_budget = SUMMON_POSE_BUDGET_PER_UPDATE.min(pending.pose_budget_remaining);
    for _ in 0..pose_budget {
        let Some((candidate, candidate_heading)) = pending.candidates.next() else {
            break;
        };
        pending.pose_budget_remaining -= 1;
        #[cfg(test)]
        {
            pending.poses_tested_last_update += 1;
            pending.poses_tested_total += 1;
        }
        if let Some(position) = placement.at_with_shape(
            candidate,
            candidate_heading,
            Some(kind),
            &pending.excluded,
            &pending.shape,
        ) {
            found = Some((position, tangent(candidate_heading, position.normalize())));
            break;
        }
    }
    if let Some(started) = locate_started {
        *pending.locate_ms.get_or_insert(0.0) += started.elapsed().as_secs_f64() * 1000.0;
    }
    let dispatch_ms = pending.dispatch_ms.get_or_insert(0.0);
    if let Some(started) = dispatch_started {
        *dispatch_ms += started.elapsed().as_secs_f64() * 1000.0;
    }

    let budget_exhausted = pending.pose_budget_remaining == 0;
    let exhausted = found.is_none() && (pending.candidates.len() == 0 || budget_exhausted);
    if found.is_none() && !exhausted {
        return;
    }

    let pending = state
        .pending_summon
        .take()
        .expect("completed summon search exists");
    state.actions.pop_front();
    let mut new_position = None;
    let mut scene_child_spawned = false;
    if let Some((position, found_heading)) = found {
        let entity = existing.unwrap_or_else(|| commands.spawn_empty().id());
        let rotation = facing(found_heading, position.normalize());
        commands.entity(entity).insert((
            Vehicle::new(kind, found_heading),
            RigidBody::Static,
            pending.shape.clone(),
            Mass(if kind == Kind::Car { 800.0 } else { 900.0 }),
            Position(position),
            Rotation(rotation),
            Transform::from_translation(position).with_rotation(rotation),
            Visibility::default(),
            SweptCcd::default(),
            CollidingEntities::default(),
            physics_reset(),
        ));
        scene_child_spawned = existing.is_none() && catalog.is_some();
        if scene_child_spawned && let Some(catalog) = catalog {
            commands.entity(entity).with_child((
                WorldAssetRoot(catalog.scene(kind.asset())),
                Transform::from_translation(Vec3::NEG_Y * kind.height()),
                view::VehicleVisual,
            ));
        }
        state.vehicles[kind.index()] = Some(entity);
        state.message = format!("{} ready — approach and press E", kind.name());
        new_position = Some(position);
    } else {
        state.message = if budget_exhausted {
            "Summoning cancelled after repeated movement — try again nearby".into()
        } else {
            "No clear dry ground with enough room / takeoff run nearby".into()
        };
    }

    if tracing && let Some(trace) = work_trace.as_deref_mut() {
        trace.record_vehicle_summon(
            real_elapsed_s,
            real_elapsed_s,
            crate::chunks::PlanetVehicleActionSample {
                kind: match kind {
                    Kind::Car => "car",
                    Kind::Plane => "plane",
                },
                existing_entity: existing.is_some(),
                old_position: pending.old_position,
                new_position,
                locate_ms: pending.locate_ms,
                dispatch_ms: pending.dispatch_ms.unwrap_or_default(),
                scene_child_spawned,
                fixed_elapsed_s_before: pending.fixed_elapsed_s_before,
                fixed_elapsed_s_after: fixed_elapsed_s,
                resident_obstacles_before: pending.resident_obstacles_before,
                world_ready_before: pending.world_ready_before,
            },
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn actions(
    mut commands: Commands,
    mut state: ResMut<Exploration>,
    placement: Placement,
    mut player: Query<(Entity, &Position, &mut Player), Without<Vehicle>>,
    mut vehicles: Query<(Entity, &Position, &Rotation, &Collider, &mut Vehicle)>,
    catalog: Option<Res<crate::asset_catalog::AssetCatalog>>,
    world: Option<Res<world::CollisionWorld>>,
    world_epoch: Option<Res<crate::map::WorldEpoch>>,
    real_time: Res<Time<Real>>,
    fixed_time: Res<Time<bevy::time::Fixed>>,
    mut work_trace: Option<ResMut<crate::chunks::PlanetWorkTrace>>,
) {
    if let Some(trace) = work_trace.as_mut() {
        trace.record_vehicle_followup(
            real_time.elapsed_secs_f64(),
            real_time.elapsed_secs_f64(),
            fixed_time.elapsed_secs_f64(),
            world.as_ref().map_or(0, |collision_world| {
                collision_world.resident_obstacle_count()
            }),
            world
                .as_ref()
                .is_none_or(|collision_world| collision_world.ready),
            state.actions.front().is_some(),
        );
    }

    let Ok((explorer, position, mut player)) = player.single_mut() else {
        return;
    };
    let origin = state
        .occupied
        .and_then(|e| vehicles.get(e).ok().map(|(_, p, _, _, _)| p.0))
        .unwrap_or(position.0);
    state.target = vehicles
        .iter()
        .filter(|(_, _, _, _, v)| v.stable >= 0.25)
        .filter_map(|(e, p, r, c, _)| {
            let nearest = c.project_point(p.0, r.0, origin, true);
            let d = nearest.0.distance(origin);
            (d <= 3.0).then_some((e, d))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)))
        .map(|(e, _)| e);
    // Residency runs before physics; one fully built query frame is required for
    // distant requests. Requests stay queued while collider preparation completes.
    if world
        .as_ref()
        .is_some_and(|w| !w.ready || w.last_action != state.actions.front().copied())
    {
        return;
    }
    let heading = state
        .occupied
        .and_then(|e| vehicles.get(e).ok().map(|(_, _, _, _, v)| v.flight.heading))
        .unwrap_or(player.heading);
    if matches!(state.actions.front(), Some(Action::Summon(_))) {
        process_summon_action(
            &mut commands,
            &mut state,
            &placement,
            origin,
            heading,
            &mut vehicles,
            catalog.as_deref(),
            world.as_deref(),
            world_epoch.as_deref().copied(),
            real_time.elapsed_secs_f64(),
            fixed_time.elapsed_secs_f64(),
            work_trace.as_deref_mut(),
        );
        return;
    }
    // Process only the destination prepared by this frame's residency pass.
    for action in state.actions.pop_front().into_iter() {
        match action {
            Action::Summon(_) => unreachable!("summons are processed incrementally"),
            Action::Interact => {
                if let Some(e) = state.occupied {
                    let Ok((_, p, _, _, mut v)) = vehicles.get_mut(e) else {
                        continue;
                    };
                    if v.stable < 0.25 {
                        state.message =
                            "Stop on stable ground before exiting; hold R to recover".into();
                        continue;
                    }
                    let side = tangent(v.flight.heading, p.0.normalize()).cross(p.0.normalize());
                    let width = if v.kind == Kind::Car { 1.6 } else { 4.8 };
                    let exit = [-1.0, 1.0].into_iter().find_map(|sign| {
                        placement.at(
                            p.0 + side * width * sign,
                            v.flight.heading,
                            None,
                            &[explorer],
                        )
                    });
                    let Some(exit) = exit else {
                        state.message = "Both exits blocked — hold R to recover".into();
                        continue;
                    };
                    if !v.crashed {
                        v.parked = true;
                        commands.entity(e).insert((
                            RigidBody::Static,
                            LinearVelocity::ZERO,
                            AngularVelocity::ZERO,
                        ));
                    }
                    player.heading = tangent(v.flight.heading, exit.normalize());
                    release(&mut commands, explorer, exit, &mut state);
                } else if let Some(e) = state.target {
                    let Ok((_, _, _, _, mut v)) = vehicles.get_mut(e) else {
                        continue;
                    };
                    if v.crashed {
                        state.message = "Wreck — summon the vehicle to restore it".into();
                        continue;
                    }
                    v.parked = false;
                    commands
                        .entity(e)
                        .insert((RigidBody::Dynamic, physics_reset()));
                    commands
                        .entity(explorer)
                        .insert((
                            RigidBody::Kinematic,
                            ColliderDisabled,
                            Seated,
                            Visibility::Hidden,
                            LinearVelocity::ZERO,
                        ))
                        .remove::<(RadialGravity, RadialUpright)>();
                    state.occupied = Some(e);
                    state.message = "Entered vehicle".into();
                } else {
                    state.message = "No stopped vehicle within reach".into();
                }
            }
            Action::Recover if state.occupied.is_some() => {
                let e = state.occupied.unwrap();
                let Ok((_, _, _, _, mut vehicle)) = vehicles.get_mut(e) else {
                    continue;
                };
                let kind = vehicle.kind;
                let radius = if kind == Kind::Car { 30.0 } else { 100.0 };
                let destination = std::iter::once(origin)
                    .chain(state.safe)
                    .chain(state.start)
                    .find_map(|center| {
                        placement.locate(center, heading, Some(kind), &[explorer, e], radius)
                    });
                let Some((p, h)) = destination else {
                    state.message =
                        "Vehicle recovery failed: no clear dry ground / takeoff run".into();
                    continue;
                };
                *vehicle = Vehicle::new(kind, h);
                vehicle.parked = false;
                commands.entity(e).insert((
                    RigidBody::Dynamic,
                    Position(p),
                    Rotation(facing(h, p.normalize())),
                    physics_reset(),
                ));
                state.recovery = 0.0;
                state.snap_camera = true;
                state.message = format!("{} recovered — ready to go", kind.name());
            }
            Action::Recover | Action::Teleport(_) => {
                if matches!(action, Action::Teleport(_))
                    && (state.occupied.is_some() || state.selector)
                {
                    state.message = "Map teleport is available on foot only".into();
                    continue;
                }
                let destination = if let Action::Teleport(p) = action {
                    placement
                        .at(p, heading, None, &[explorer])
                        .map(|p| (p, heading))
                } else {
                    state
                        .safe
                        .into_iter()
                        .chain(state.start)
                        .find_map(|center| {
                            placement.locate(center, heading, None, &[explorer], 30.0)
                        })
                };
                if let Some((p, h)) = destination {
                    player.heading = tangent(h, p.normalize());
                    release(&mut commands, explorer, p, &mut state);
                    state.snap_camera = true;
                    state.message = "Returned to safe ground".into();
                } else {
                    state.message = "Recovery failed: no clear dry standing location".into();
                }
            }
            Action::PlanetTeleport(request_id) => {
                let Some(request) = state
                    .pending_planet_teleport
                    .as_ref()
                    .filter(|request| request.id == request_id)
                    .cloned()
                else {
                    continue;
                };
                let Some(current_epoch) = world_epoch.as_deref().copied() else {
                    state.finish_planet_teleport(
                        request,
                        PlanetTeleportOutcomeKind::Rejected {
                            reason: PlanetTeleportRejection::WorldUnavailable,
                        },
                    );
                    continue;
                };
                if current_epoch != request.world_epoch {
                    state.finish_planet_teleport(
                        request,
                        PlanetTeleportOutcomeKind::Cancelled {
                            reason: PlanetTeleportCancellation::StaleWorld,
                        },
                    );
                    state.clear_planet_destination();
                    continue;
                }
                if state
                    .selected_destination
                    .as_ref()
                    .is_none_or(|selected| selected.id != request.destination.id)
                {
                    state.finish_planet_teleport(
                        request,
                        PlanetTeleportOutcomeKind::Cancelled {
                            reason: PlanetTeleportCancellation::DestinationChanged,
                        },
                    );
                    continue;
                }
                if !state.planet_camera.is_requested_open() {
                    state.finish_planet_teleport(
                        request,
                        PlanetTeleportOutcomeKind::Cancelled {
                            reason: PlanetTeleportCancellation::ViewClosed,
                        },
                    );
                    continue;
                }
                if state.occupied.is_some() {
                    state.finish_planet_teleport(
                        request,
                        PlanetTeleportOutcomeKind::Rejected {
                            reason: PlanetTeleportRejection::OccupiedVehicle,
                        },
                    );
                    continue;
                }
                if state.selector {
                    state.finish_planet_teleport(
                        request,
                        PlanetTeleportOutcomeKind::Rejected {
                            reason: PlanetTeleportRejection::VehicleSelectorOpen,
                        },
                    );
                    continue;
                }
                let Some(destination) =
                    placement.at(request.destination.position, heading, None, &[explorer])
                else {
                    state.finish_planet_teleport(
                        request,
                        PlanetTeleportOutcomeKind::Rejected {
                            reason: PlanetTeleportRejection::UnsafeLanding,
                        },
                    );
                    continue;
                };
                let heading = tangent(heading, destination.normalize());
                player.heading = heading;
                release(&mut commands, explorer, destination, &mut state);
                state.finish_planet_teleport(
                    request,
                    PlanetTeleportOutcomeKind::Succeeded {
                        position: destination,
                    },
                );
                state.clear_planet_destination();
                state.snap_camera = false;
                state.set_planet_view_open(false);
            }
        }
    }
}
fn release(commands: &mut Commands, explorer: Entity, position: Vec3, state: &mut Exploration) {
    commands
        .entity(explorer)
        .remove::<(ColliderDisabled, Seated)>()
        .insert((
            RigidBody::Dynamic,
            RadialGravity,
            RadialUpright,
            Position(position),
            Rotation(Quat::from_rotation_arc(Vec3::Y, position.normalize())),
            LinearVelocity::ZERO,
            Visibility::Visible,
        ));
    state.occupied = None;
    state.safe = Some(position);
    state.recovery = 0.0;
}
fn sync_explorer(
    state: Res<Exploration>,
    vehicles: Query<(&Position, &Vehicle), Without<Player>>,
    mut player: Query<(&mut Position, &mut Player), With<Seated>>,
) {
    let Some(e) = state.occupied else {
        return;
    };
    if let (Ok((p, v)), Ok((mut pos, mut player))) = (vehicles.get(e), player.single_mut()) {
        pos.0 = p.0;
        player.heading = v.flight.heading;
    }
}
fn track_safe(
    mut state: ResMut<Exploration>,
    placement: Placement,
    player: Query<
        (
            Entity,
            &Position,
            &Player,
            &CollidingEntities,
            &LinearVelocity,
        ),
        Without<Vehicle>,
    >,
    vehicles: Query<(&Position, &Vehicle)>,
) {
    let Ok((e, p, player, contacts, velocity)) = player.single() else {
        return;
    };
    if let Some(occupied) = state.occupied {
        if let Ok((p, v)) = vehicles.get(occupied)
            && v.stable >= 0.25
        {
            let side = v.flight.heading.cross(p.0.normalize());
            let width = if v.kind == Kind::Car { 1.6 } else { 4.8 };
            if let Some(safe) = [-1.0, 1.0]
                .into_iter()
                .find_map(|s| placement.at(p.0 + side * width * s, v.flight.heading, None, &[e]))
            {
                state.safe = Some(safe);
            }
        }
    } else if contacts.0.iter().any(|e| !placement.obstacle(*e))
        && velocity.dot(p.0.normalize()).abs() < 0.5
        && let Some(safe) = placement.at(p.0, player.heading, None, &[e])
        && safe.distance(p.0) < 0.2
    {
        state.safe = Some(safe);
    }
}
#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use bevy::{color::Alpha, ecs::system::RunSystemOnce};

    #[test]
    fn planet_view_interface_visibility_excludes_the_selector_modal() {
        let mut state = Exploration::default();
        state.set_planet_view_open(true);
        state.planet_presentation.advance(0.28);
        assert!(state.planet_view_interface_visible());

        state.selector = true;
        assert!(!state.planet_view_interface_visible());
    }

    #[test]
    fn destination_selection_and_teleport_share_public_readiness_intents() {
        let mut state = Exploration::default();
        let screen_position = Vec2::new(240.0, 180.0);

        assert!(!state.planet_view_ready());
        assert_eq!(state.planet_view_interface_opacity(), 0.0);
        assert!(!state.request_planet_view_selection(screen_position));
        assert!(!state.request_planet_view_teleport());

        state.set_planet_view_open(true);
        assert!(!state.request_planet_view_selection(screen_position));
        assert!(!state.request_planet_view_teleport());
        state.planet_presentation.advance(0.28);

        assert!(state.planet_view_ready());
        assert_eq!(state.planet_view_interface_opacity(), 1.0);
        state.selector = true;
        assert!(!state.planet_view_ready());
        assert!(!state.request_planet_view_selection(screen_position));
        assert!(!state.request_planet_view_teleport());
        state.selector = false;
        assert!(state.planet_view_ready());
        let position = Vec3::new(12.0, 2001.0, 4.0);
        state.select_planet_destination(PlanetDestination {
            id: PlanetDestinationId::surface(crate::map::WorldEpoch::new(17), position),
            position,
            surface: PlanetDestinationSurface::Terrain,
            display: "Terrain · 1°N, 2°E".into(),
        });
        assert!(state.request_planet_view_selection(screen_position));
        assert_eq!(state.take_planet_view_selection(), Some(screen_position));
        assert!(state.request_planet_view_teleport());
        assert!(!state.request_planet_view_teleport());
        assert!(state.take_planet_view_teleport_request());
        assert!(!state.take_planet_view_teleport_request());

        state.set_planet_view_open(false);
        assert!(!state.planet_view_ready());
        assert!(!state.request_planet_view_selection(screen_position));
        assert!(!state.request_planet_view_teleport());
    }

    #[test]
    fn destination_identity_is_world_scoped_and_supports_duplicate_named_places() {
        let epoch = crate::map::WorldEpoch::new(17);
        let point = Vec3::new(12.0, 2001.0, -9.0);

        let surface = PlanetDestinationId::surface(epoch, point);
        let same_surface = PlanetDestinationId::surface(epoch, point);
        let next_world = PlanetDestinationId::surface(crate::map::WorldEpoch::new(18), point);
        let first_region = PlanetDestinationId::region(epoch, 4);
        let second_region = PlanetDestinationId::region(epoch, 5);
        let first_settlement =
            PlanetDestinationId::collection(epoch, PlanetDestinationCollection::Settlement, 4);
        let second_settlement =
            PlanetDestinationId::collection(epoch, PlanetDestinationCollection::Settlement, 5);
        let road = PlanetDestinationId::collection(epoch, PlanetDestinationCollection::Road, 4);
        let bridge = PlanetDestinationId::collection(epoch, PlanetDestinationCollection::Bridge, 4);

        assert_eq!(surface, same_surface);
        assert_ne!(surface, next_world);
        assert_ne!(first_region, second_region);
        assert_ne!(first_settlement, second_settlement);
        assert_ne!(first_settlement, road);
        assert_ne!(road, bridge);
    }

    #[test]
    fn selected_destination_survives_view_ui_and_vehicle_changes() {
        let epoch = crate::map::WorldEpoch::new(23);
        let position = Vec3::new(30.0, 2001.0, -45.0);
        let selection = PlanetDestination {
            id: PlanetDestinationId::surface(epoch, position),
            position,
            surface: PlanetDestinationSurface::BridgeDeck,
            display: "Bridge deck · 12.4°N, 33.7°W".into(),
        };
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(Exploration {
                selected_destination: Some(selection.clone()),
                ..default()
            })
            .add_systems(Update, interface::update_presentation);
        {
            let mut state = app.world_mut().resource_mut::<Exploration>();
            state.set_planet_view_open(true);
            state.planet_camera.zoom_by(0.8);
            state.toggle_planet_view_follow(Transform::from_translation(Vec3::new(
                0.0, 2001.0, 20.0,
            )));
            state.occupied = Some(Entity::PLACEHOLDER);
        }

        let details_entity = app
            .world_mut()
            .spawn((
                Node::default(),
                interface::DestinationPanel,
                shared::planet_view_interface::PlanetViewInterfaceElement::default(),
            ))
            .with_children(|panel| {
                panel.spawn((
                    Text::new("old UI"),
                    interface::DestinationDetails,
                    shared::planet_view_interface::PlanetViewInterfaceElement::default(),
                ));
            })
            .id();
        app.update();
        let old_details = app
            .world_mut()
            .query_filtered::<&Text, With<interface::DestinationDetails>>()
            .single(app.world())
            .expect("destination details are shown in the first UI");
        assert!(old_details.0.contains("Bridge deck · 12.4°N, 33.7°W"));
        assert!(old_details.0.contains("Exit your vehicle"));

        app.world_mut().despawn(details_entity);
        let rebuilt_panel = app
            .world_mut()
            .spawn((
                Node::default(),
                interface::DestinationPanel,
                shared::planet_view_interface::PlanetViewInterfaceElement::default(),
            ))
            .with_children(|panel| {
                panel.spawn((
                    Text::new("fresh UI"),
                    interface::DestinationDetails,
                    shared::planet_view_interface::PlanetViewInterfaceElement::default(),
                ));
            })
            .id();
        app.update();
        let rebuilt_details = app
            .world_mut()
            .query_filtered::<&Text, With<interface::DestinationDetails>>()
            .single(app.world())
            .expect("destination details are reconstructed");
        assert!(rebuilt_details.0.contains("Bridge deck · 12.4°N, 33.7°W"));
        assert!(rebuilt_details.0.contains("Exit your vehicle"));
        assert_ne!(details_entity, rebuilt_panel);

        {
            let mut state = app.world_mut().resource_mut::<Exploration>();
            assert_eq!(state.selected_destination.as_ref(), Some(&selection));
            assert!(state.destination_requires_exit_from_vehicle());
            state.set_planet_view_open(false);
            state.set_planet_view_open(true);
            state.occupied = None;
        }
        app.update();

        let state = app.world().resource::<Exploration>();
        assert_eq!(state.selected_destination.as_ref(), Some(&selection));
        assert!(!state.destination_requires_exit_from_vehicle());
        let rebuilt_details = app
            .world_mut()
            .query_filtered::<&Text, With<interface::DestinationDetails>>()
            .single(app.world())
            .expect("reopened destination details remain available");
        assert!(
            rebuilt_details
                .0
                .contains("Press T to check a safe on-foot landing")
        );
    }

    #[test]
    fn entering_a_new_world_clears_the_previous_selection() {
        let position = Vec3::new(30.0, 2001.0, -45.0);
        let epoch = crate::map::WorldEpoch::new(2);
        let destination = PlanetDestination {
            id: PlanetDestinationId::surface(epoch, position),
            position,
            surface: PlanetDestinationSurface::Terrain,
            display: "Terrain · 12.4°N, 33.7°W".into(),
        };
        let request = PlanetTeleportRequest {
            id: PlanetTeleportRequestId(7),
            world_epoch: epoch,
            destination: destination.clone(),
        };
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(Exploration {
                selected_destination: Some(destination),
                actions: [Action::PlanetTeleport(request.id), Action::Interact]
                    .into_iter()
                    .collect(),
                pending_planet_teleport: Some(request.clone()),
                planet_teleport_status: Some(PlanetTeleportStatus::Checking {
                    request_id: request.id,
                    destination_id: request.destination.id,
                }),
                ..default()
            })
            .add_systems(Update, initialize);
        app.world_mut().spawn((
            crate::map::player_physics_bundle(position, Vec3::Z),
            Position(position),
        ));

        app.update();

        assert!(
            app.world()
                .resource::<Exploration>()
                .selected_destination
                .is_none()
        );
        let state = app.world().resource::<Exploration>();
        assert!(state.pending_planet_teleport_request().is_none());
        assert_eq!(state.actions.len(), 1);
        assert!(state.actions.contains(&Action::Interact));
        assert!(matches!(
            state.planet_teleport_outcomes.front(),
            Some(PlanetTeleportOutcome {
                request_id,
                result: PlanetTeleportOutcomeKind::Cancelled {
                    reason: PlanetTeleportCancellation::WorldReloaded
                },
                ..
            }) if *request_id == request.id
        ));
    }

    #[test]
    fn destination_details_show_the_location_and_current_on_foot_requirement() {
        let epoch = crate::map::WorldEpoch::new(31);
        let position = Vec3::new(-40.0, 1999.0, 60.0);
        let mut state = Exploration::default();
        state.select_planet_destination(PlanetDestination {
            id: PlanetDestinationId::surface(epoch, position),
            position,
            surface: PlanetDestinationSurface::BridgeDeck,
            display: "Bridge deck · 27.5°S, 56.3°E".into(),
        });
        state.occupied = Some(Entity::PLACEHOLDER);

        let details = interface::destination_readout(&state);
        assert!(details.contains("Bridge deck · 27.5°S, 56.3°E"));
        assert!(details.contains("Exit your vehicle"));
        assert!(details.contains("checked after you press T"));

        state.occupied = None;
        let details = interface::destination_readout(&state);
        assert!(details.contains("Press T to check a safe on-foot landing"));
    }

    #[test]
    fn interface_readiness_advances_while_simulation_time_is_paused() {
        let (mut app, _) = fixture();
        app.world_mut()
            .resource_mut::<Exploration>()
            .set_planet_view_open(true);
        app.world_mut().resource_mut::<Time<Virtual>>().pause();

        for _ in 0..20 {
            app.update();
        }

        assert!(app.world().resource::<Time<Virtual>>().is_paused());
        assert!(app.world().resource::<Exploration>().planet_view_ready());
    }

    #[test]
    fn planet_camera_transition_advances_while_simulation_time_is_paused() {
        let (mut app, _) = fixture();
        let planet_radius = shared::sphere::PLANET_RADIUS;
        let camera = app
            .world_mut()
            .spawn((
                MainCamera,
                Transform::from_xyz(0.0, planet_radius + 5.0, -5.0).looking_at(Vec3::ZERO, Vec3::Y),
                Projection::Perspective(PerspectiveProjection::default()),
            ))
            .id();
        let starting_radius = app
            .world()
            .get::<Transform>(camera)
            .unwrap()
            .translation
            .length();
        app.world_mut()
            .resource_mut::<Exploration>()
            .set_planet_view_open(true);
        app.world_mut().resource_mut::<Time<Virtual>>().pause();

        for _ in 0..120 {
            app.update();
        }

        assert!(app.world().resource::<Time<Virtual>>().is_paused());
        let final_radius = app
            .world()
            .get::<Transform>(camera)
            .unwrap()
            .translation
            .length();
        assert!(final_radius > starting_radius + 1000.0);
    }

    #[test]
    fn recovery_hold_advances_while_simulation_time_is_paused() {
        let (mut app, _) = fixture();
        app.world_mut().resource_mut::<Time<Virtual>>().pause();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyR);

        for _ in 0..30 {
            app.update();
        }

        assert!(app.world().resource::<Time<Virtual>>().is_paused());
        assert!(app.world().resource::<Exploration>().recovery > 0.4);
    }

    #[test]
    fn exploration_readout_fades_and_restores_through_the_live_presentation() {
        let (mut app, _) = fixture();
        app.world_mut()
            .insert_resource(crate::ui::UiFont(Handle::default()));
        app.world_mut().run_system_once(view::setup).unwrap();
        app.update();

        let readout = {
            let world = app.world_mut();
            let mut query = world.query_filtered::<Entity, With<view::Readout>>();
            query.single(world).unwrap()
        };
        let first_text = app.world().get::<Text>(readout).unwrap().0.clone();
        assert!(first_text.starts_with("On foot · WASD move"));
        let original_text_alpha = app.world().get::<TextColor>(readout).unwrap().0.alpha();
        let original_background_alpha = app
            .world()
            .get::<BackgroundColor>(readout)
            .unwrap()
            .0
            .alpha();
        assert!(original_text_alpha > 0.9);
        assert!(original_background_alpha > 0.0);

        app.world_mut()
            .resource_mut::<Exploration>()
            .set_planet_view_open(true);
        for _ in 0..20 {
            app.update();
        }
        assert!(app.world().resource::<Exploration>().planet_view_ready());
        assert!(app.world().get::<TextColor>(readout).unwrap().0.alpha() < 0.01);
        assert!(
            app.world()
                .get::<BackgroundColor>(readout)
                .unwrap()
                .0
                .alpha()
                < 0.01
        );
        assert_eq!(app.world().get::<Text>(readout).unwrap().0, first_text);

        app.world_mut()
            .resource_mut::<Exploration>()
            .set_planet_view_open(false);
        for _ in 0..5 {
            app.update();
        }
        let partially_restored_alpha = app.world().get::<TextColor>(readout).unwrap().0.alpha();
        assert!(partially_restored_alpha > 0.0 && partially_restored_alpha < original_text_alpha);

        app.world_mut()
            .resource_mut::<Exploration>()
            .set_planet_view_open(true);
        app.update();
        let reversed_alpha = app.world().get::<TextColor>(readout).unwrap().0.alpha();
        assert!(reversed_alpha < partially_restored_alpha);

        for _ in 0..20 {
            app.update();
        }
        assert!(app.world().resource::<Exploration>().planet_view_ready());
        assert!(app.world().get::<TextColor>(readout).unwrap().0.alpha() < 0.01);

        app.world_mut()
            .resource_mut::<Exploration>()
            .set_planet_view_open(false);
        for _ in 0..20 {
            app.update();
        }
        let restored_text_alpha = app.world().get::<TextColor>(readout).unwrap().0.alpha();
        let restored_background_alpha = app
            .world()
            .get::<BackgroundColor>(readout)
            .unwrap()
            .0
            .alpha();
        assert!((restored_text_alpha - original_text_alpha).abs() < 0.01);
        assert!((restored_background_alpha - original_background_alpha).abs() < 0.01);
        assert!(
            app.world()
                .get::<Text>(readout)
                .unwrap()
                .0
                .starts_with("On foot · WASD move")
        );

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyV);
        app.update();
        let state = app.world().resource::<Exploration>();
        assert!(state.selector);
        assert!(!state.planet_camera.is_requested_open());
        assert!(!state.planet_view_ready());
        assert!(
            app.world()
                .get::<Text>(readout)
                .unwrap()
                .0
                .starts_with("SUMMON VEHICLE")
        );
        assert!(
            (app.world().get::<TextColor>(readout).unwrap().0.alpha() - original_text_alpha).abs()
                < 0.01
        );

        {
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keys.clear_just_pressed(KeyCode::KeyV);
            keys.release(KeyCode::KeyV);
            keys.press(KeyCode::Escape);
        }
        app.update();
        let state = app.world().resource::<Exploration>();
        assert!(!state.selector);
        assert!(!state.planet_camera.is_requested_open());
        assert!(!state.planet_view_ready());
        let post_selector_text_alpha = app.world().get::<TextColor>(readout).unwrap().0.alpha();
        let post_selector_background_alpha = app
            .world()
            .get::<BackgroundColor>(readout)
            .unwrap()
            .0
            .alpha();
        assert!((post_selector_text_alpha - original_text_alpha).abs() < 0.01);
        assert!((post_selector_background_alpha - original_background_alpha).abs() < 0.01);
        assert!(
            app.world()
                .get::<Text>(readout)
                .unwrap()
                .0
                .starts_with("On foot · WASD move")
        );
    }

    #[test]
    fn t_publishes_a_teleport_intent_only_while_planet_view_is_ready() {
        let (mut app, _) = fixture();
        let epoch = crate::map::WorldEpoch::new(62);
        let position = Vec3::new(40.0, 2000.0, 0.0);
        let destination = PlanetDestination {
            id: PlanetDestinationId::surface(epoch, position),
            position,
            surface: PlanetDestinationSurface::Terrain,
            display: "Terrain · 0°N, 0°E".into(),
        };
        app.world_mut().insert_resource(epoch);
        app.world_mut()
            .resource_mut::<Exploration>()
            .select_planet_destination(destination.clone());
        app.world_mut()
            .resource_mut::<Exploration>()
            .set_planet_view_open(true);
        for _ in 0..20 {
            app.update();
        }
        assert!(app.world().resource::<Exploration>().planet_view_ready());
        app.world_mut()
            .insert_resource(world::CollisionWorld::default());

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyT);
        app.update();
        let state = app.world().resource::<Exploration>();
        let request = state
            .pending_planet_teleport_request()
            .expect("T snapshots one pending destination request");
        assert_eq!(request.world_epoch, epoch);
        assert_eq!(request.destination, destination);
        assert!(state.actions.contains(&Action::PlanetTeleport(request.id)));
        assert!(
            !app.world_mut()
                .resource_mut::<Exploration>()
                .request_planet_view_teleport()
        );

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .clear_just_pressed(KeyCode::KeyT);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .release(KeyCode::KeyT);
        app.world_mut()
            .resource_mut::<Exploration>()
            .set_planet_view_open(false);
        assert!(
            app.world()
                .resource::<Exploration>()
                .pending_planet_teleport_request()
                .is_none()
        );
        assert!(matches!(
            app.world_mut()
                .resource_mut::<Exploration>()
                .take_planet_teleport_outcome(),
            Some(PlanetTeleportOutcome {
                result: PlanetTeleportOutcomeKind::Cancelled {
                    reason: PlanetTeleportCancellation::ViewClosed
                },
                ..
            })
        ));
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyT);
        app.update();
        assert!(
            !app.world_mut()
                .resource_mut::<Exploration>()
                .take_planet_view_teleport_request()
        );
    }

    fn selected_surface(epoch: crate::map::WorldEpoch, position: Vec3) -> PlanetDestination {
        PlanetDestination {
            id: PlanetDestinationId::surface(epoch, position),
            position,
            surface: PlanetDestinationSurface::Terrain,
            display: "Terrain · 0°N, 0°E".into(),
        }
    }

    fn press_t(app: &mut App) {
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyT);
        app.update();
        let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        keys.clear_just_pressed(KeyCode::KeyT);
        keys.release(KeyCode::KeyT);
    }

    #[test]
    fn confirmed_destination_waits_for_collision_then_returns_after_relocation() {
        let (mut app, explorer) = fixture();
        let camera = spawn_planet_view_camera(&mut app);
        open_planet_view(&mut app);
        let epoch = *app.world().resource::<crate::map::WorldEpoch>();
        let destination = selected_surface(epoch, Vec3::new(25.0, 2000.0, 0.0));
        let original_position = app.world().get::<Position>(explorer).unwrap().0;
        app.world_mut()
            .resource_mut::<Exploration>()
            .select_planet_destination(destination.clone());
        app.world_mut()
            .insert_resource(world::CollisionWorld::default());

        press_t(&mut app);

        let request = app
            .world()
            .resource::<Exploration>()
            .pending_planet_teleport_request()
            .expect("T starts an identified request");
        let request_id = request.id;
        assert_eq!(request.destination, destination);
        assert_eq!(
            app.world().get::<Position>(explorer).unwrap().0,
            original_position,
            "the first residency frame must not relocate the explorer"
        );
        assert!(matches!(
            app.world().resource::<Exploration>().planet_teleport_status(),
            Some(PlanetTeleportStatus::Checking { request_id: id, .. }) if id == request_id
        ));
        assert!(
            interface::destination_readout(app.world().resource::<Exploration>())
                .contains("Checking this exact landing spot")
        );
        assert!(
            !app.world_mut()
                .resource_mut::<Exploration>()
                .request_planet_view_teleport(),
            "repeated confirmation cannot duplicate a pending request"
        );

        // Browsing controls do not revoke a confirmed destination.
        {
            let camera_pose = *app.world().get::<Transform>(camera).unwrap();
            let mut state = app.world_mut().resource_mut::<Exploration>();
            state.planet_camera.zoom_by(0.9);
            state
                .planet_camera
                .orbit_from(camera_pose, Vec2::new(0.02, -0.01));
        }
        assert_eq!(
            app.world()
                .resource::<Exploration>()
                .pending_planet_teleport_request()
                .map(|request| request.id),
            Some(request_id)
        );

        app.update();

        let arrived_position = app.world().get::<Position>(explorer).unwrap().0;
        assert!(
            arrived_position.x > 24.0,
            "relocation committed: {arrived_position:?}"
        );
        assert_eq!(
            app.world_mut()
                .resource_mut::<Exploration>()
                .take_planet_teleport_outcome(),
            Some(PlanetTeleportOutcome {
                request_id,
                world_epoch: epoch,
                destination_id: destination.id,
                result: PlanetTeleportOutcomeKind::Succeeded {
                    position: arrived_position,
                },
            })
        );
        let state = app.world().resource::<Exploration>();
        assert!(state.pending_planet_teleport_request().is_none());
        assert!(state.selected_planet_destination().is_none());
        assert!(!state.planet_camera.is_requested_open());
        assert!(
            !state.snap_camera,
            "success must not use the legacy snap path"
        );
        assert!(
            app.world()
                .get::<Transform>(camera)
                .unwrap()
                .translation
                .distance(arrived_position)
                > 100.0,
            "the first return frame remains on the animated flight path"
        );

        for _ in 0..130 {
            app.update();
        }
        assert!(
            app.world()
                .get::<Transform>(camera)
                .unwrap()
                .translation
                .distance(app.world().get::<Position>(explorer).unwrap().0)
                < 20.0,
            "the return flight should finish at the relocated explorer"
        );
    }

    #[test]
    fn unsafe_confirmed_destination_is_rejected_without_relocating_or_closing() {
        let (mut app, explorer) = fixture();
        let camera = spawn_planet_view_camera(&mut app);
        let target = Vec3::new(40.0, 2000.0, 0.0);
        app.world_mut().spawn((
            RigidBody::Static,
            Collider::cuboid(4.0, 2.0, 4.0),
            Transform::from_translation(target + Vec3::Y),
            Ground,
            crate::chunks::WorldObstacle,
        ));
        for _ in 0..3 {
            app.update();
        }
        open_planet_view(&mut app);
        let epoch = *app.world().resource::<crate::map::WorldEpoch>();
        let destination = selected_surface(epoch, target);
        let original_position = app.world().get::<Position>(explorer).unwrap().0;
        app.world_mut()
            .resource_mut::<Exploration>()
            .select_planet_destination(destination.clone());
        app.world_mut()
            .insert_resource(world::CollisionWorld::default());

        press_t(&mut app);
        let request_id = app
            .world()
            .resource::<Exploration>()
            .pending_planet_teleport_request()
            .unwrap()
            .id;
        app.update();

        assert_eq!(
            app.world().get::<Position>(explorer).unwrap().0,
            original_position
        );
        let state = app.world().resource::<Exploration>();
        assert!(state.pending_planet_teleport_request().is_none());
        assert_eq!(state.selected_planet_destination(), Some(&destination));
        assert!(state.planet_camera.is_requested_open());
        assert!(matches!(
            state.planet_teleport_status(),
            Some(PlanetTeleportStatus::Rejected {
                request_id: id,
                reason: PlanetTeleportRejection::UnsafeLanding,
                ..
            }) if id == request_id
        ));
        assert!(interface::destination_readout(state).contains("No safe on-foot landing"));
        assert!(matches!(
            app.world_mut()
                .resource_mut::<Exploration>()
                .take_planet_teleport_outcome(),
            Some(PlanetTeleportOutcome {
                result: PlanetTeleportOutcomeKind::Rejected {
                    reason: PlanetTeleportRejection::UnsafeLanding,
                },
                ..
            })
        ));
        assert!(app.world().get::<Transform>(camera).is_some());
    }

    #[test]
    fn reselection_cancels_a_pending_request_and_preserves_other_actions() {
        let (mut app, _) = fixture();
        open_planet_view(&mut app);
        let epoch = *app.world().resource::<crate::map::WorldEpoch>();
        let first = selected_surface(epoch, Vec3::new(20.0, 2000.0, 0.0));
        let second = selected_surface(epoch, Vec3::new(30.0, 2000.0, 0.0));
        app.world_mut()
            .resource_mut::<Exploration>()
            .select_planet_destination(first.clone());
        app.world_mut()
            .insert_resource(world::CollisionWorld::default());
        press_t(&mut app);
        let first_request = app
            .world()
            .resource::<Exploration>()
            .pending_planet_teleport_request()
            .unwrap()
            .clone();
        app.world_mut()
            .resource_mut::<Exploration>()
            .request(Action::Interact);

        app.world_mut()
            .resource_mut::<Exploration>()
            .select_planet_destination(second.clone());

        {
            let state = app.world().resource::<Exploration>();
            assert!(state.pending_planet_teleport_request().is_none());
            assert_eq!(state.selected_planet_destination(), Some(&second));
            assert_eq!(state.actions.len(), 1);
            assert!(state.actions.contains(&Action::Interact));
        }
        assert!(matches!(
            app.world_mut()
                .resource_mut::<Exploration>()
                .take_planet_teleport_outcome(),
            Some(PlanetTeleportOutcome {
                request_id,
                result: PlanetTeleportOutcomeKind::Cancelled {
                    reason: PlanetTeleportCancellation::DestinationChanged,
                },
                ..
            }) if request_id == first_request.id
        ));

        assert!(
            app.world_mut()
                .resource_mut::<Exploration>()
                .request_planet_view_teleport()
        );
        app.update();
        app.update();
        let fresh_request = app
            .world()
            .resource::<Exploration>()
            .pending_planet_teleport_request()
            .expect("the new destination needs a fresh T request");
        assert_ne!(fresh_request.id, first_request.id);
        assert_eq!(fresh_request.destination, second);
        assert!(
            app.world()
                .resource::<Exploration>()
                .actions
                .contains(&Action::PlanetTeleport(fresh_request.id))
        );
        assert!(
            app.world()
                .resource::<Exploration>()
                .message
                .contains("No stopped vehicle within reach"),
            "the unrelated interaction should execute before the fresh teleport"
        );
    }

    #[test]
    fn epoch_change_before_commit_cancels_without_moving_the_explorer() {
        let (mut app, explorer) = fixture();
        open_planet_view(&mut app);
        let epoch = *app.world().resource::<crate::map::WorldEpoch>();
        let destination = selected_surface(epoch, Vec3::new(35.0, 2000.0, 0.0));
        let original_position = app.world().get::<Position>(explorer).unwrap().0;
        app.world_mut()
            .resource_mut::<Exploration>()
            .select_planet_destination(destination);
        app.world_mut()
            .insert_resource(world::CollisionWorld::default());
        press_t(&mut app);
        let request_id = app
            .world()
            .resource::<Exploration>()
            .pending_planet_teleport_request()
            .unwrap()
            .id;
        app.world_mut()
            .insert_resource(crate::map::WorldEpoch::new(epoch.value() + 1));

        app.update();

        assert_eq!(
            app.world().get::<Position>(explorer).unwrap().0,
            original_position
        );
        let state = app.world().resource::<Exploration>();
        assert!(state.pending_planet_teleport_request().is_none());
        assert!(state.selected_planet_destination().is_none());
        assert!(matches!(
            state.planet_teleport_status(),
            Some(PlanetTeleportStatus::Cancelled {
                request_id: id,
                reason: PlanetTeleportCancellation::StaleWorld,
                ..
            }) if id == request_id
        ));
        assert!(matches!(
            app.world_mut()
                .resource_mut::<Exploration>()
                .take_planet_teleport_outcome(),
            Some(PlanetTeleportOutcome {
                request_id: id,
                result: PlanetTeleportOutcomeKind::Cancelled {
                    reason: PlanetTeleportCancellation::StaleWorld,
                },
                ..
            }) if id == request_id
        ));
    }

    #[test]
    fn vehicle_eligibility_is_rechecked_after_collision_wait() {
        let (mut app, explorer) = fixture();
        open_planet_view(&mut app);
        let epoch = *app.world().resource::<crate::map::WorldEpoch>();
        let destination = selected_surface(epoch, Vec3::new(45.0, 2000.0, 0.0));
        let original_position = app.world().get::<Position>(explorer).unwrap().0;
        app.world_mut()
            .resource_mut::<Exploration>()
            .select_planet_destination(destination.clone());
        app.world_mut()
            .insert_resource(world::CollisionWorld::default());
        press_t(&mut app);
        let request_id = app
            .world()
            .resource::<Exploration>()
            .pending_planet_teleport_request()
            .unwrap()
            .id;
        app.world_mut().resource_mut::<Exploration>().occupied = Some(Entity::PLACEHOLDER);

        app.update();

        let state = app.world().resource::<Exploration>();
        assert_eq!(
            app.world().get::<Position>(explorer).unwrap().0,
            original_position
        );
        assert_eq!(state.selected_planet_destination(), Some(&destination));
        assert!(state.planet_camera.is_requested_open());
        assert!(matches!(
            state.planet_teleport_status(),
            Some(PlanetTeleportStatus::Rejected {
                request_id: id,
                reason: PlanetTeleportRejection::OccupiedVehicle,
                ..
            }) if id == request_id
        ));
    }

    #[test]
    fn f_toggles_follow_outside_the_vehicle_selector() {
        let (mut app, _) = fixture();
        let camera = app
            .world_mut()
            .spawn((MainCamera, Transform::from_xyz(0.0, 2005.0, -5.0)))
            .id();
        app.world_mut()
            .resource_mut::<Exploration>()
            .set_planet_view_open(true);
        app.update();

        for expected_follow in [false, true] {
            let before = *app.world().get::<Transform>(camera).unwrap();
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(KeyCode::KeyF);
            app.update();
            let after = app.world().get::<Transform>(camera).unwrap();
            assert_eq!(
                app.world()
                    .resource::<Exploration>()
                    .planet_view_follows_body(),
                expected_follow
            );
            assert!(
                after
                    .translation
                    .normalize()
                    .dot(before.translation.normalize())
                    > 0.999,
                "F should preserve the attained radial view while toggling follow"
            );
            assert!(after.rotation.angle_between(before.rotation) < 0.03);
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .clear_just_pressed(KeyCode::KeyF);
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .release(KeyCode::KeyF);
        }

        assert!(
            app.world()
                .resource::<Exploration>()
                .planet_view_follows_body()
        );
    }

    #[test]
    fn vehicle_selector_owns_map_keys_and_escape_cannot_leak_to_planet_view() {
        let (mut app, _) = fixture();
        app.world_mut().spawn((
            MainCamera,
            Transform::from_xyz(0.0, 2005.0, -5.0).looking_at(Vec3::new(0.0, 2000.6, 0.0), Vec3::Y),
        ));
        app.add_systems(
            PreUpdate,
            inject_selector_priority_keys
                .after(InputSystems)
                .before(ExplorationInput),
        );
        app.update();
        assert!(app.world().resource::<Exploration>().selector);
        app.update();

        let state = app.world().resource::<Exploration>();
        assert!(!state.selector);
        assert!(!state.planet_camera.is_requested_open());
        assert!(!state.planet_view_ready());
        assert!(state.planet_view_follows_body());
    }

    #[test]
    fn vehicle_selector_cannot_open_during_planet_view_or_be_deferred_until_close() {
        let (mut app, _) = fixture();
        spawn_planet_view_camera(&mut app);
        app.world_mut()
            .resource_mut::<Exploration>()
            .set_planet_view_open(true);

        // V is rejected while the camera is entering Planet view.
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyV);
        app.update();
        assert!(!app.world().resource::<Exploration>().selector);
        assert!(!app.world().resource::<Time<Virtual>>().is_paused());
        {
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keys.clear_just_pressed(KeyCode::KeyV);
            keys.release(KeyCode::KeyV);
        }

        for _ in 0..100 {
            app.update();
        }
        assert!(app.world().resource::<Exploration>().planet_view_ready());
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyV);
        app.update();
        assert!(!app.world().resource::<Exploration>().selector);
        assert!(!app.world().resource::<Time<Virtual>>().is_paused());
        {
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keys.clear_just_pressed(KeyCode::KeyV);
            keys.release(KeyCode::KeyV);
        }

        // V is also rejected during the return transition. Holding it through
        // the rest of that transition must not open the selector afterwards.
        app.world_mut()
            .resource_mut::<Exploration>()
            .set_planet_view_open(false);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyV);
        app.update();
        assert!(!app.world().resource::<Exploration>().selector);
        assert!(!app.world().resource::<Time<Virtual>>().is_paused());
        for _ in 0..150 {
            app.update();
        }
        let state = app.world().resource::<Exploration>();
        assert!(!state.is_planet_view_active());
        assert!(!state.selector);
        assert!(!app.world().resource::<Time<Virtual>>().is_paused());
    }

    fn inject_selector_priority_keys(mut frame: Local<u8>, mut keys: ResMut<ButtonInput<KeyCode>>) {
        match *frame {
            0 => keys.press(KeyCode::KeyV),
            1 => {
                keys.press(KeyCode::Escape);
                keys.press(KeyCode::KeyM);
                keys.press(KeyCode::KeyF);
                keys.press(KeyCode::KeyT);
            }
            _ => {}
        }
        *frame = frame.saturating_add(1);
    }

    pub(crate) fn fixture() -> (App, Entity) {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            TransformPlugin,
            AssetPlugin::default(),
            bevy::state::app::StatesPlugin,
            PhysicsPlugins::default(),
        ))
        .init_state::<AppState>()
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<ButtonInput<MouseButton>>()
        .add_message::<bevy::input::mouse::MouseWheel>()
        .init_asset::<Mesh>()
        .init_asset::<StandardMaterial>()
        .insert_resource(Gravity::ZERO)
        .insert_resource(crate::map::WorldEpoch::new(1))
        .insert_resource(SubstepCount(12))
        .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f32(1.0 / 60.0),
        ))
        .add_plugins(ExplorationPlugin);
        app.world_mut().spawn((
            RigidBody::Static,
            Collider::cuboid(1000.0, 1.0, 1000.0),
            Transform::from_xyz(0.0, 1999.5, 0.0),
            Ground,
        ));
        let explorer = app
            .world_mut()
            .spawn(crate::map::player_physics_bundle(
                Vec3::new(0.0, 2000.6, 0.0),
                Vec3::NEG_Z,
            ))
            .id();
        app.finish();
        app.cleanup();
        for _ in 0..3 {
            app.update();
        }
        app.world_mut()
            .insert_resource(State::new(AppState::Playing));
        app.world_mut().resource_mut::<Exploration>().start = Some(Vec3::new(0.0, 2000.6, 0.0));
        (app, explorer)
    }
    fn act(app: &mut App, action: Action) {
        app.world_mut()
            .resource_mut::<Exploration>()
            .request(action);
        app.update();
    }
    fn block_initial_summon_poses(app: &mut App) {
        // Blocks the center and first three sectors of the first ring, while
        // leaving the next ordered anchor available.
        app.world_mut().spawn((
            RigidBody::Static,
            Collider::cuboid(32.0, 3.0, 12.0),
            Transform::from_xyz(-13.0, 2000.0, -10.0),
        ));
        app.update();
    }
    fn synchronous_vehicle_placement_oracle(
        placement: &Placement,
        origin: Vec3,
        heading: Vec3,
        kind: Kind,
        excluded: &[Entity],
    ) -> Vec3 {
        let up = origin.normalize();
        let forward = tangent(heading, up);
        let side = forward.cross(up);
        let radius = kind.summon_search_radius();
        for ring in 0..=6 {
            let distance = radius * ring as f32 / 6.0;
            for sector in 0..if ring == 0 { 1 } else { 16 } {
                let angle = sector as f32 * std::f32::consts::TAU / 16.0;
                let candidate = origin + (forward * angle.cos() + side * angle.sin()) * distance;
                for yaw in 0..8 {
                    let candidate_heading = Quat::from_axis_angle(
                        candidate.normalize(),
                        yaw as f32 * std::f32::consts::TAU / 8.0,
                    ) * forward;
                    if let Some(position) =
                        placement.at(candidate, candidate_heading, Some(kind), excluded)
                    {
                        return position;
                    }
                }
            }
        }
        panic!("synchronous placement oracle found no safe candidate");
    }
    fn spawn_planet_view_camera(app: &mut App) -> Entity {
        app.world_mut()
            .spawn((
                MainCamera,
                Transform::from_xyz(0.0, 2005.0, -5.0)
                    .looking_at(Vec3::new(0.0, 2000.6, 0.0), Vec3::Y),
                Projection::Perspective(PerspectiveProjection::default()),
            ))
            .id()
    }
    fn open_planet_view(app: &mut App) {
        app.world_mut()
            .resource_mut::<Exploration>()
            .set_planet_view_open(true);
        for _ in 0..180 {
            app.update();
        }
    }

    #[derive(Clone, Copy)]
    struct CameraMotionSample {
        time: f32,
        pose: Transform,
        body: Vec3,
        body_aim_degrees: f32,
        planet_center_aim_degrees: f32,
        screen_roll_degrees: f32,
        max_angular_speed: f32,
        max_screen_roll_speed: f32,
    }

    fn m_open_trace(hz: usize, move_body: bool) -> Vec<CameraMotionSample> {
        assert!(hz >= 30 && hz.is_multiple_of(30));
        let (mut app, explorer) = fixture();
        let delta = 1.0 / hz as f32;
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f32(delta),
        ));
        let camera = spawn_planet_view_camera(&mut app);
        // Let the production camera system establish its normal chase pose before
        // starting the M-driven opening trace. The spawned transform is only a
        // fixture placeholder and points back at the player.
        app.update();
        let sample_stride = hz / 30;
        let mut trace = Vec::with_capacity(30);
        let mut previous_pose = *app.world().get::<Transform>(camera).unwrap();
        let mut max_angular_speed = 0.0_f32;
        let mut max_screen_roll_speed = 0.0_f32;

        for frame in 0..hz {
            let time = (frame + 1) as f32 * delta;
            if move_body {
                let body_rotation = Quat::from_rotation_z(time * 0.01);
                app.world_mut().get_mut::<Position>(explorer).unwrap().0 =
                    body_rotation * Vec3::Y * (shared::sphere::PLANET_RADIUS + 0.6);
                app.world_mut().get_mut::<Player>(explorer).unwrap().heading =
                    body_rotation * Vec3::NEG_Z;
            }
            if frame == 0 {
                app.world_mut()
                    .resource_mut::<ButtonInput<KeyCode>>()
                    .press(KeyCode::KeyM);
            }
            app.update();
            let pose = *app.world().get::<Transform>(camera).unwrap();
            max_angular_speed = max_angular_speed.max(
                shortest_angular_velocity(previous_pose.rotation, pose.rotation, delta).length(),
            );
            max_screen_roll_speed =
                max_screen_roll_speed.max(screen_roll_delta(previous_pose, pose) / delta);
            previous_pose = pose;
            if frame == 0 {
                let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
                keys.clear_just_pressed(KeyCode::KeyM);
                keys.release(KeyCode::KeyM);
            }
            if (frame + 1) % sample_stride == 0 {
                let body = app.world().get::<Position>(explorer).unwrap().0;
                let heading = app.world().get::<Player>(explorer).unwrap().heading;
                let forward = pose.rotation * Vec3::NEG_Z;
                let desired_screen_up =
                    (heading - forward * heading.dot(forward)).normalize_or(Vec3::Y);
                trace.push(CameraMotionSample {
                    time,
                    pose,
                    body,
                    body_aim_degrees: (body - pose.translation)
                        .normalize()
                        .angle_between(forward)
                        .to_degrees(),
                    planet_center_aim_degrees: (-pose.translation)
                        .normalize()
                        .angle_between(forward)
                        .to_degrees(),
                    screen_roll_degrees: (pose.rotation * Vec3::Y)
                        .angle_between(desired_screen_up)
                        .to_degrees(),
                    max_angular_speed,
                    max_screen_roll_speed,
                });
            }
        }

        for tick in [12, 24, 30] {
            let sample = trace[tick - 1];
            println!(
                "M_OPEN hz={hz} moving={move_body} t={:.3} pos=({:.1},{:.1},{:.1}) r={:.1} body=({:.1},{:.1},{:.1}) body_aim={:.2}deg planet_center_aim={:.2}deg screen_roll={:.2}deg max_angular_speed={:.2}rad/s max_screen_roll_speed={:.2}rad/s",
                sample.time,
                sample.pose.translation.x,
                sample.pose.translation.y,
                sample.pose.translation.z,
                sample.pose.translation.length(),
                sample.body.x,
                sample.body.y,
                sample.body.z,
                sample.body_aim_degrees,
                sample.planet_center_aim_degrees,
                sample.screen_roll_degrees,
                sample.max_angular_speed,
                sample.max_screen_roll_speed,
            );
        }
        trace
    }

    #[test]
    fn m_open_transition_is_frame_rate_stable_with_a_moving_body() {
        let stationary = [30, 60, 120].map(|hz| m_open_trace(hz, false));
        let moving = [30, 60, 120].map(|hz| m_open_trace(hz, true));
        for (label, traces) in [("stationary", &stationary), ("moving", &moving)] {
            for (rate, trace) in [60, 120].into_iter().zip(traces.iter().skip(1)) {
                let baseline = &traces[0];
                let (max_position_delta, max_rotation_delta) = baseline
                    .iter()
                    .zip(trace)
                    .map(|(a, b)| {
                        (
                            a.pose.translation.distance(b.pose.translation),
                            a.pose.rotation.angle_between(b.pose.rotation),
                        )
                    })
                    .fold((0.0_f32, 0.0_f32), |maxima, delta| {
                        (maxima.0.max(delta.0), maxima.1.max(delta.1))
                    });
                println!(
                    "M_OPEN_RATE_DELTA case={label} hz={rate} max_position={max_position_delta:.1}m max_rotation={:.1}deg",
                    max_rotation_delta.to_degrees(),
                );
                assert!(
                    max_position_delta < 10.0 && max_rotation_delta < 0.03,
                    "M open framing depends on frame rate ({label} body): {rate} Hz differs from 30 Hz by {max_position_delta:.1} m and {:.1} degrees",
                    max_rotation_delta.to_degrees(),
                );
            }
        }
    }

    #[test]
    fn m_open_keeps_the_planet_center_within_the_vertical_viewport() {
        for hz in [30, 60, 120] {
            let trace = m_open_trace(hz, false);
            let half_vertical_fov = PerspectiveProjection::default().fov * 0.5;
            for sample in &trace {
                if sample.time >= 0.4 {
                    assert!(
                        sample.planet_center_aim_degrees.to_radians() < half_vertical_fov,
                        "planet center is below the viewport at {hz} Hz, t={:.3}s, radius={:.1}m, aim error={:.1}deg, half FOV={:.1}deg",
                        sample.time,
                        sample.pose.translation.length(),
                        sample.planet_center_aim_degrees,
                        half_vertical_fov.to_degrees(),
                    );
                }
                assert!(
                    sample.screen_roll_degrees < 5.0,
                    "opening roll diverged from the projected heading at {hz} Hz, t={:.3}s: {:.1}deg",
                    sample.time,
                    sample.screen_roll_degrees,
                );
                assert!(
                    sample.max_screen_roll_speed < 2.0,
                    "opening screen-up changed too quickly at {hz} Hz, t={:.3}s: {:.2}rad/s",
                    sample.time,
                    sample.max_screen_roll_speed,
                );
            }
        }
    }

    #[derive(Debug)]
    struct OppositeReturnMetrics {
        hz: usize,
        max_center_aim_degrees: f32,
        max_roll_degrees: f32,
        max_path_error_degrees: f32,
        min_swing_radius: f32,
        max_linear_speed: f32,
        max_angular_speed: f32,
        final_body_distance: f32,
    }

    const PLANET_VIEW_FAR_RETURN_SECONDS_FOR_TRACE: f32 = 2.4;

    fn opposite_side_return_trace(hz: usize) -> OppositeReturnMetrics {
        assert!(hz >= 30 && hz.is_multiple_of(30));
        let (mut app, explorer) = fixture();
        let delta = 1.0 / hz as f32;
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f32(delta),
        ));
        let camera = spawn_planet_view_camera(&mut app);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyM);
        app.update();
        {
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keys.clear_just_pressed(KeyCode::KeyM);
            keys.release(KeyCode::KeyM);
        }
        for _ in 1..(hz * 3 / 2) {
            app.update();
        }

        let start_direction = Vec3::NEG_Y;
        let start_pose = Transform::from_translation(
            start_direction * shared::planet_view::PLANET_VIEW_FAR_RADIUS,
        )
        .looking_at(Vec3::ZERO, Vec3::NEG_Z);
        app.world_mut()
            .resource_mut::<Exploration>()
            .planet_camera
            .detach(start_direction);
        *app.world_mut().get_mut::<Transform>(camera).unwrap() = start_pose;

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyM);
        app.update();
        {
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keys.clear_just_pressed(KeyCode::KeyM);
            keys.release(KeyCode::KeyM);
        }

        let antipodal_axis = start_direction.any_orthonormal_vector().normalize();
        let initial_up = start_pose.rotation * Vec3::Y;
        let mut metrics = OppositeReturnMetrics {
            hz,
            max_center_aim_degrees: 0.0,
            max_roll_degrees: 0.0,
            max_path_error_degrees: 0.0,
            min_swing_radius: f32::INFINITY,
            max_linear_speed: 0.0,
            max_angular_speed: 0.0,
            final_body_distance: f32::INFINITY,
        };
        let mut previous_pose = start_pose;
        let frame_count = hz * 3;
        for frame in 0..frame_count {
            app.update();
            let pose = *app.world().get::<Transform>(camera).unwrap();
            let elapsed = (frame + 2) as f32 * delta;
            let progress = (elapsed / PLANET_VIEW_FAR_RETURN_SECONDS_FOR_TRACE).clamp(0.0, 1.0);
            let linear_speed = pose.translation.distance(previous_pose.translation) / delta;
            let angular_speed = previous_pose.rotation.angle_between(pose.rotation) / delta;
            metrics.max_linear_speed = metrics.max_linear_speed.max(linear_speed);
            metrics.max_angular_speed = metrics.max_angular_speed.max(angular_speed);
            previous_pose = pose;

            if (0.15..=0.75).contains(&progress) {
                let swing = ((progress - 0.15) / 0.6).clamp(0.0, 1.0);
                let eased = swing * swing * (3.0 - 2.0 * swing);
                let transport = Quat::from_axis_angle(antipodal_axis, std::f32::consts::PI * eased);
                let expected_direction = (transport * start_direction).normalize();
                let expected_forward = -expected_direction;
                let expected_up = transport * initial_up;
                let actual_forward = pose.rotation * Vec3::NEG_Z;
                let actual_up = pose.rotation * Vec3::Y;
                let aim_error = actual_forward.angle_between(expected_forward).to_degrees();
                let actual_up = (actual_up - expected_forward * actual_up.dot(expected_forward))
                    .normalize_or(expected_up);
                let expected_up = (expected_up
                    - expected_forward * expected_up.dot(expected_forward))
                .normalize_or(Vec3::Z);
                let roll_error = expected_forward
                    .dot(expected_up.cross(actual_up))
                    .atan2(expected_up.dot(actual_up))
                    .abs()
                    .to_degrees();
                let path_error = pose
                    .translation
                    .normalize()
                    .angle_between(expected_direction)
                    .to_degrees();
                metrics.max_center_aim_degrees = metrics.max_center_aim_degrees.max(aim_error);
                metrics.max_roll_degrees = metrics.max_roll_degrees.max(roll_error);
                metrics.max_path_error_degrees = metrics.max_path_error_degrees.max(path_error);
                metrics.min_swing_radius = metrics.min_swing_radius.min(pose.translation.length());
            }
        }
        let body = app.world().get::<Position>(explorer).unwrap().0;
        metrics.final_body_distance = app
            .world()
            .get::<Transform>(camera)
            .unwrap()
            .translation
            .distance(body);
        metrics
    }

    #[test]
    fn opposite_side_m_return_keeps_the_planet_centered_and_tracks_its_path() {
        let metrics = [30, 60, 120].map(opposite_side_return_trace);
        for measurement in &metrics {
            assert!(measurement.min_swing_radius > shared::sphere::PLANET_RADIUS + 300.0);
            assert!(measurement.max_path_error_degrees < 2.0);
            assert!(measurement.final_body_distance < 30.0);
            assert!(
                measurement.max_linear_speed < 22_000.0,
                "{} Hz far-side return reaches {:.0} m/s",
                measurement.hz,
                measurement.max_linear_speed,
            );
            assert!(
                measurement.max_angular_speed < 8.0,
                "{} Hz far-side return reaches {:.1} rad/s",
                measurement.hz,
                measurement.max_angular_speed,
            );
            assert!(
                measurement.max_center_aim_degrees < 20.0,
                "{} Hz opposite-side return turns away from the planet by {:.1} degrees",
                measurement.hz,
                measurement.max_center_aim_degrees,
            );
            assert!(
                measurement.max_roll_degrees < 20.0,
                "{} Hz opposite-side return rolls {:.1} degrees off the transported frame",
                measurement.hz,
                measurement.max_roll_degrees,
            );
        }
    }

    #[derive(Debug)]
    struct SameSideReturnMetrics {
        hz: usize,
        max_radial_speed: f32,
        max_angular_speed: f32,
        max_radius_increase: f32,
        final_body_distance: f32,
    }

    fn same_side_return_trace(hz: usize, orbit_radians: f32) -> SameSideReturnMetrics {
        let (mut app, explorer) = fixture();
        let delta = 1.0 / hz as f32;
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f32(delta),
        ));
        let camera = spawn_planet_view_camera(&mut app);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyM);
        app.update();
        {
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keys.clear_just_pressed(KeyCode::KeyM);
            keys.release(KeyCode::KeyM);
        }
        for _ in 1..(hz * 3 / 2) {
            app.update();
        }

        if orbit_radians > 0.0 {
            let current = *app.world().get::<Transform>(camera).unwrap();
            app.world_mut()
                .resource_mut::<Exploration>()
                .planet_camera
                .orbit_from(current, Vec2::new(orbit_radians, 0.0));
            for _ in 0..hz {
                app.update();
            }
        }

        let start_pose = *app.world().get::<Transform>(camera).unwrap();
        let start_radius = start_pose.translation.length();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyM);
        app.update();
        {
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keys.clear_just_pressed(KeyCode::KeyM);
            keys.release(KeyCode::KeyM);
        }

        let mut metrics = SameSideReturnMetrics {
            hz,
            max_radial_speed: 0.0,
            max_angular_speed: 0.0,
            max_radius_increase: 0.0,
            final_body_distance: f32::INFINITY,
        };
        let mut previous = start_pose;
        for _ in 0..(hz * 2) {
            app.update();
            let pose = *app.world().get::<Transform>(camera).unwrap();
            let radial_speed =
                (pose.translation.length() - previous.translation.length()).abs() / delta;
            let angular_speed = previous.rotation.angle_between(pose.rotation) / delta;
            metrics.max_radial_speed = metrics.max_radial_speed.max(radial_speed);
            metrics.max_angular_speed = metrics.max_angular_speed.max(angular_speed);
            metrics.max_radius_increase = metrics
                .max_radius_increase
                .max(pose.translation.length() - start_radius);
            previous = pose;
        }
        let body = app.world().get::<Position>(explorer).unwrap().0;
        metrics.final_body_distance = app
            .world()
            .get::<Transform>(camera)
            .unwrap()
            .translation
            .distance(body);
        metrics
    }

    #[test]
    fn same_side_m_return_has_no_radial_or_angular_speed_snap() {
        for measurement in [30, 60, 120].map(|hz| same_side_return_trace(hz, 0.0)) {
            assert!(measurement.max_radius_increase < 10.0);
            assert!(measurement.final_body_distance < 30.0);
            assert!(
                measurement.max_radial_speed < 8_000.0,
                "{} Hz same-side return has a {:.0} m/s radial snap",
                measurement.hz,
                measurement.max_radial_speed,
            );
            assert!(
                measurement.max_angular_speed < 8.0,
                "{} Hz same-side return has a {:.1} rad/s angular snap",
                measurement.hz,
                measurement.max_angular_speed,
            );
        }
    }

    #[test]
    fn nearby_orbit_m_return_takes_a_direct_approach() {
        let orbit_angle = 30.0_f32.to_radians();
        for measurement in [30, 60, 120].map(|hz| same_side_return_trace(hz, orbit_angle)) {
            assert!(measurement.max_radius_increase < 10.0);
            assert!(measurement.final_body_distance < 30.0);
            assert!(
                measurement.max_radial_speed < 8_000.0,
                "{} Hz nearby return holds the transit shell then descends at {:.0} m/s",
                measurement.hz,
                measurement.max_radial_speed,
            );
            assert!(measurement.max_angular_speed < 8.0);
        }
    }

    fn reversal_motion_velocity(hz: usize) -> (Vec3, Vec3, Vec3, Vec3, Vec3) {
        let (mut app, _) = fixture();
        let delta = 1.0 / hz as f32;
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f32(delta),
        ));
        let camera = spawn_planet_view_camera(&mut app);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyM);
        app.update();
        {
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keys.clear_just_pressed(KeyCode::KeyM);
            keys.release(KeyCode::KeyM);
        }
        for _ in 1..(hz * 3 / 2) {
            app.update();
        }

        let start_direction = Vec3::NEG_Y;
        let start_pose = Transform::from_translation(
            start_direction * shared::planet_view::PLANET_VIEW_FAR_RADIUS,
        )
        .looking_at(Vec3::ZERO, Vec3::NEG_Z);
        app.world_mut()
            .resource_mut::<Exploration>()
            .planet_camera
            .detach(start_direction);
        *app.world_mut().get_mut::<Transform>(camera).unwrap() = start_pose;

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyM);
        app.update();
        {
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keys.clear_just_pressed(KeyCode::KeyM);
            keys.release(KeyCode::KeyM);
        }

        let mut previous = *app.world().get::<Transform>(camera).unwrap();
        let mut incoming_linear_velocity = Vec3::ZERO;
        let mut incoming_angular_velocity = Vec3::ZERO;
        for _ in 0..(hz * 6 / 5) {
            app.update();
            let pose = *app.world().get::<Transform>(camera).unwrap();
            incoming_linear_velocity = (pose.translation - previous.translation) / delta;
            incoming_angular_velocity =
                shortest_angular_velocity(previous.rotation, pose.rotation, delta);
            previous = pose;
        }

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyM);
        app.update();
        {
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keys.clear_just_pressed(KeyCode::KeyM);
            keys.release(KeyCode::KeyM);
        }
        let after_reversal = *app.world().get::<Transform>(camera).unwrap();
        let outgoing_linear_velocity = (after_reversal.translation - previous.translation) / delta;
        let outgoing_angular_velocity =
            shortest_angular_velocity(previous.rotation, after_reversal.rotation, delta);
        app.update();
        let after_second_reopen_frame = *app.world().get::<Transform>(camera).unwrap();
        let second_frame_angular_velocity = shortest_angular_velocity(
            after_reversal.rotation,
            after_second_reopen_frame.rotation,
            delta,
        );
        (
            incoming_linear_velocity,
            outgoing_linear_velocity,
            incoming_angular_velocity,
            outgoing_angular_velocity,
            second_frame_angular_velocity,
        )
    }

    fn shortest_angular_velocity(from: Quat, to: Quat, delta_seconds: f32) -> Vec3 {
        let delta = from.inverse() * to;
        let delta = if delta.w < 0.0 { -delta } else { delta };
        let (axis, angle) = delta.to_axis_angle();
        axis * (angle / delta_seconds)
    }

    fn screen_roll_delta(previous: Transform, current: Transform) -> f32 {
        let previous_forward = previous.rotation * Vec3::NEG_Z;
        let current_forward = current.rotation * Vec3::NEG_Z;
        let transport = Quat::from_rotation_arc(previous_forward, current_forward);
        let transported_up = transport * (previous.rotation * Vec3::Y);
        let transported_up = (transported_up
            - current_forward * transported_up.dot(current_forward))
        .normalize_or(Vec3::Y);
        let current_up = current.rotation * Vec3::Y;
        let current_up = (current_up - current_forward * current_up.dot(current_forward))
            .normalize_or(transported_up);
        current_forward
            .dot(transported_up.cross(current_up))
            .atan2(transported_up.dot(current_up))
            .abs()
    }

    #[test]
    fn m_reversal_preserves_transition_velocity() {
        for hz in [30, 60, 120] {
            let (
                incoming_linear,
                outgoing_linear,
                incoming_angular,
                outgoing_angular,
                second_frame_angular,
            ) = reversal_motion_velocity(hz);
            let linear_delta = incoming_linear.distance(outgoing_linear);
            let angular_delta = incoming_angular.distance(outgoing_angular);
            println!(
                "REVERSAL_SPEED hz={hz} linear={:.0}->{:.0}m/s angular={:.3}->{:.3}->{:.3}rad/s",
                incoming_linear.length(),
                outgoing_linear.length(),
                incoming_angular.length(),
                outgoing_angular.length(),
                second_frame_angular.length(),
            );
            assert!(
                incoming_linear.is_finite()
                    && outgoing_linear.is_finite()
                    && incoming_angular.is_finite()
                    && outgoing_angular.is_finite()
                    && second_frame_angular.is_finite(),
                "{hz} Hz M reversal produced a non-finite trajectory"
            );
            assert!(
                incoming_angular.length() > 1.0,
                "{hz} Hz fixture did not reach an active turn"
            );
            assert!(
                angular_delta < 1.5,
                "{hz} Hz M reversal snaps angular velocity by {angular_delta:.2} rad/s"
            );
            assert!(
                second_frame_angular.length() < 8.0
                    && second_frame_angular.distance(outgoing_angular) < 4.0,
                "{hz} Hz M reversal angular velocity did not settle smoothly over the next frame"
            );
            assert!(
                linear_delta < 5_500.0,
                "{hz} Hz M reversal snaps linear velocity by {linear_delta:.0} m/s"
            );
        }
    }

    fn summon_and_enter_vehicle(app: &mut App, explorer: Entity, kind: Kind) -> Entity {
        act(app, Action::Summon(kind));
        let vehicle = app.world().resource::<Exploration>().vehicles[kind.index()].unwrap();
        for _ in 0..30 {
            app.update();
        }
        let position = app.world().get::<Position>(vehicle).unwrap().0;
        let side = app.world().get::<Rotation>(vehicle).unwrap().0 * Vec3::X;
        let width = if kind == Kind::Car { 2.0 } else { 4.8 };
        app.world_mut()
            .entity_mut(explorer)
            .insert(Position(position + side * width));
        act(app, Action::Interact);
        assert_eq!(
            app.world().resource::<Exploration>().occupied,
            Some(vehicle)
        );
        vehicle
    }

    #[test]
    fn planet_view_opens_without_a_camera_snap_and_returns_to_exploration() {
        let (mut app, explorer) = fixture();
        let camera_start =
            Transform::from_xyz(0.0, 2005.0, -5.0).looking_at(Vec3::new(0.0, 2000.6, 0.0), Vec3::Y);
        let camera = app
            .world_mut()
            .spawn((
                MainCamera,
                camera_start,
                Projection::Perspective(PerspectiveProjection::default()),
            ))
            .id();
        let default_far = PerspectiveProjection::default().far;
        let body_start = app.world().get::<Position>(explorer).unwrap().0;
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyM);

        app.update();

        let opening = *app.world().get::<Transform>(camera).unwrap();
        assert!(opening.translation.distance(camera_start.translation) < 10.0);
        assert!(opening.rotation.angle_between(camera_start.rotation) < 0.05);
        assert!(matches!(
            app.world().get::<Projection>(camera).unwrap(),
            Projection::Perspective(projection) if projection.far >= shared::planet_atmosphere::PLANET_VIEW_FAR_CLIP_DISTANCE
        ));
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .clear_just_pressed(KeyCode::KeyM);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .release(KeyCode::KeyM);
        for _ in 0..120 {
            app.update();
            assert!(app.world().get::<Transform>(camera).unwrap().translation.y > 2000.2);
        }
        let planet_view = app.world().get::<Transform>(camera).unwrap();
        assert!(planet_view.translation.length() > shared::sphere::PLANET_RADIUS * 2.0);
        assert!(planet_view.translation.length() < shared::sphere::PLANET_RADIUS * 4.0);
        assert!(
            app.world()
                .get::<Position>(explorer)
                .unwrap()
                .0
                .distance(body_start)
                < 0.05
        );

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Escape);
        app.update();
        let returning = app.world().get::<Transform>(camera).unwrap();
        assert!(returning.translation.length() > shared::sphere::PLANET_RADIUS * 2.0);
        assert!(returning.translation.y > 2000.2);

        for _ in 0..120 {
            app.update();
            assert!(app.world().get::<Transform>(camera).unwrap().translation.y > 2000.2);
        }
        let returned = app.world().get::<Transform>(camera).unwrap();
        let body = app.world().get::<Position>(explorer).unwrap().0;
        assert!(returned.translation.distance(body) < 30.0);
        assert!(returned.translation.length() > body.length());
        assert!(matches!(
            app.world().get::<Projection>(camera).unwrap(),
            Projection::Perspective(projection) if projection.far == default_far
        ));

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .clear_just_pressed(KeyCode::Escape);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .release(KeyCode::Escape);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyM);
        app.update();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .clear_just_pressed(KeyCode::KeyM);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .release(KeyCode::KeyM);
        for _ in 0..120 {
            app.update();
        }
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyM);
        app.update();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .clear_just_pressed(KeyCode::KeyM);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .release(KeyCode::KeyM);
        assert!(
            !app.world()
                .resource::<Exploration>()
                .planet_camera
                .is_requested_open()
        );
        for _ in 0..120 {
            app.update();
            assert!(app.world().get::<Transform>(camera).unwrap().translation.y > 2000.2);
        }
        assert!(
            app.world()
                .get::<Transform>(camera)
                .unwrap()
                .translation
                .distance(body)
                < 30.0
        );
    }

    #[test]
    fn live_camera_intents_zoom_without_detaching_and_orbit_detaches() {
        let (mut app, _) = fixture();
        app.world_mut().spawn((
            MainCamera,
            Transform::from_xyz(0.0, 2005.0, -5.0).looking_at(Vec3::new(0.0, 2000.6, 0.0), Vec3::Y),
            Projection::Perspective(PerspectiveProjection::default()),
        ));
        let far = shared::planet_view::PLANET_VIEW_FAR_RADIUS;
        {
            let mut state = app.world_mut().resource_mut::<Exploration>();
            state.set_planet_view_open(true);
            assert!(state.request_planet_view_zoom(0.5));
        }

        app.update();

        {
            let state = app.world().resource::<Exploration>();
            assert!(state.planet_view_camera_radii().0 < far);
            assert!(state.planet_view_follows_body());
        }
        assert!(
            app.world_mut()
                .resource_mut::<Exploration>()
                .request_planet_view_orbit(Vec2::new(80.0, 0.0))
        );

        app.update();

        let state = app.world().resource::<Exploration>();
        assert!(!state.planet_view_follows_body());
        assert!(state.planet_view_camera_radii().1.is_finite());
    }

    #[test]
    fn follow_tracks_the_moving_explorer_heading_and_closes_to_its_latest_pose() {
        let (mut app, explorer) = fixture();
        let camera = spawn_planet_view_camera(&mut app);
        open_planet_view(&mut app);
        let starting_position = app.world().get::<Position>(explorer).unwrap().0;

        app.world_mut()
            .entity_mut(explorer)
            .insert(LinearVelocity(Vec3::X * 40.0));
        for _ in 0..60 {
            app.update();
        }
        let moving_position = app.world().get::<Position>(explorer).unwrap().0;
        assert!(moving_position.distance(starting_position) > 1.0);
        let following_pose = app.world().get::<Transform>(camera).unwrap();
        let follow_trailing_angle = following_pose
            .translation
            .normalize()
            .angle_between(moving_position.normalize());
        assert!(
            follow_trailing_angle < 0.0001,
            "follow should track the physics-synchronized explorer position without trailing, got {follow_trailing_angle} radians"
        );

        let heading = tangent(Vec3::Z, moving_position.normalize());
        app.world_mut().get_mut::<Player>(explorer).unwrap().heading = heading;
        for _ in 0..40 {
            app.update();
        }
        let following_pose = app.world().get::<Transform>(camera).unwrap();
        assert!(
            (following_pose.rotation * Vec3::Y).dot(heading) > 0.99,
            "follow should align camera up with the controlled heading tangent"
        );

        app.world_mut()
            .resource_mut::<Exploration>()
            .set_planet_view_open(false);
        let closing_position = app.world().get::<Position>(explorer).unwrap().0;
        for _ in 0..120 {
            app.update();
        }
        let latest_position = app.world().get::<Position>(explorer).unwrap().0;
        assert!(latest_position.distance(closing_position) > 1.0);
        assert!(
            app.world()
                .get::<Transform>(camera)
                .unwrap()
                .translation
                .distance(latest_position)
                < 30.0,
            "closing should return toward the explorer's latest moving position"
        );
    }

    #[test]
    fn following_a_piloting_plane_tracks_heading_without_pitch_or_bank() {
        let (mut app, explorer) = fixture();
        let camera = spawn_planet_view_camera(&mut app);
        open_planet_view(&mut app);
        let plane = summon_and_enter_vehicle(&mut app, explorer, Kind::Plane);

        let start = app.world().get::<Position>(plane).unwrap().0;
        let airborne = start + start.normalize() * 16.0;
        {
            let mut vehicle = app.world_mut().get_mut::<Vehicle>(plane).unwrap();
            vehicle.parked = false;
            vehicle.flight.airborne = true;
            vehicle.flight.pitch = 0.2;
            vehicle.flight.bank = 0.4;
        }
        let heading = app.world().get::<Vehicle>(plane).unwrap().flight.heading;
        let rotation = app
            .world()
            .get::<Vehicle>(plane)
            .unwrap()
            .flight
            .rotation(airborne.normalize());
        app.world_mut().entity_mut(plane).insert((
            Position(airborne),
            Rotation(rotation),
            LinearVelocity(heading * 30.0),
        ));
        {
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keys.press(KeyCode::KeyS);
            keys.press(KeyCode::KeyA);
            keys.press(KeyCode::ShiftLeft);
        }
        for _ in 0..60 {
            app.update();
        }
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .clear();

        let body_position = app.world().get::<Position>(plane).unwrap().0;
        let vehicle = app.world().get::<Vehicle>(plane).unwrap();
        let body_rotation = app.world().get::<Rotation>(plane).unwrap().0;
        let expected_heading = tangent(vehicle.flight.heading, body_position.normalize());
        assert!(body_position.distance(airborne) > 1.0);
        assert!(vehicle.flight.pitch.abs() > 0.2);
        assert!(vehicle.flight.bank.abs() > 0.2);

        let camera_pose = app.world().get::<Transform>(camera).unwrap();
        assert!(
            camera_pose
                .translation
                .normalize()
                .dot(body_position.normalize())
                > 0.9999,
            "follow should track the occupied plane's physics position"
        );
        let camera_up = camera_pose.rotation * Vec3::Y;
        assert!(camera_up.dot(expected_heading) > 0.99);
        assert!(
            camera_up.dot(body_rotation * Vec3::Y) < 0.95,
            "camera heading should not inherit the plane's pitch or bank"
        );
    }

    #[test]
    fn vehicle_entry_exit_transfers_follow_and_keeps_detached_framing() {
        for detached in [false, true] {
            let (mut app, explorer) = fixture();
            let camera = spawn_planet_view_camera(&mut app);
            open_planet_view(&mut app);
            act(&mut app, Action::Summon(Kind::Car));
            let car = app.world().resource::<Exploration>().vehicles[Kind::Car.index()].unwrap();
            for _ in 0..30 {
                app.update();
            }
            let car_position = app.world().get::<Position>(car).unwrap().0;
            let side = app.world().get::<Rotation>(car).unwrap().0 * Vec3::X;
            app.world_mut()
                .entity_mut(explorer)
                .insert(Position(car_position + side * 2.0));
            let before_entry = *app.world().get::<Transform>(camera).unwrap();
            act(&mut app, Action::Interact);
            let after_entry = *app.world().get::<Transform>(camera).unwrap();
            assert!(
                after_entry
                    .translation
                    .normalize()
                    .angle_between(before_entry.translation.normalize())
                    < 0.03,
                "vehicle entry should transfer follow without a camera snap"
            );

            if detached {
                app.world_mut()
                    .resource_mut::<Exploration>()
                    .toggle_planet_view_follow(after_entry);
            }
            let detached_direction = app
                .world()
                .get::<Transform>(camera)
                .unwrap()
                .translation
                .normalize();
            let starting_car_position = app.world().get::<Position>(car).unwrap().0;
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(KeyCode::KeyW);
            for _ in 0..45 {
                app.update();
            }
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .clear();
            let moving_car_position = app.world().get::<Position>(car).unwrap().0;
            assert!(moving_car_position.distance(starting_car_position) > 0.5);

            if detached {
                let view = app.world().get::<Transform>(camera).unwrap();
                assert!(view.translation.normalize().dot(detached_direction) > 0.9999);
            } else {
                for _ in 0..30 {
                    app.update();
                }
                let view = app.world().get::<Transform>(camera).unwrap();
                assert!(
                    view.translation
                        .normalize()
                        .dot(moving_car_position.normalize())
                        > 0.9999
                );
            }

            app.world_mut().entity_mut(car).insert(LinearVelocity::ZERO);
            app.world_mut().get_mut::<Vehicle>(car).unwrap().stable = 0.5;
            act(&mut app, Action::Interact);
            assert!(
                app.world().resource::<Exploration>().occupied.is_none(),
                "detached={detached}, stable={}, message={}",
                app.world().get::<Vehicle>(car).unwrap().stable,
                app.world().resource::<Exploration>().message
            );
            for _ in 0..45 {
                app.update();
            }
            let explorer_position = app.world().get::<Position>(explorer).unwrap().0;
            let view = app.world().get::<Transform>(camera).unwrap();
            assert!(
                app.world()
                    .resource::<Exploration>()
                    .planet_camera
                    .is_requested_open()
            );
            if detached {
                assert!(view.translation.normalize().dot(detached_direction) > 0.9999);
            } else {
                assert!(
                    view.translation
                        .normalize()
                        .dot(explorer_position.normalize())
                        > 0.9999
                );
            }
        }
    }

    #[test]
    fn explorer_and_vehicle_recovery_keep_planet_view_and_respect_follow_mode() {
        for detached in [false, true] {
            let (mut app, explorer) = fixture();
            let camera = spawn_planet_view_camera(&mut app);
            open_planet_view(&mut app);
            if detached {
                let pose = *app.world().get::<Transform>(camera).unwrap();
                app.world_mut()
                    .resource_mut::<Exploration>()
                    .toggle_planet_view_follow(pose);
            }
            let detached_direction = app
                .world()
                .get::<Transform>(camera)
                .unwrap()
                .translation
                .normalize();

            let safe = app.world().get::<Position>(explorer).unwrap().0;
            app.world_mut().resource_mut::<Exploration>().safe = Some(safe);
            app.world_mut().entity_mut(explorer).insert((
                Position(Vec3::new(800.0, 2000.6, 0.0)),
                LinearVelocity::ZERO,
            ));
            act(&mut app, Action::Recover);
            let recovered_explorer = app.world().get::<Position>(explorer).unwrap().0;
            assert!(recovered_explorer.x.abs() < 100.0);
            assert!(
                app.world()
                    .resource::<Exploration>()
                    .planet_camera
                    .is_requested_open()
            );
            for _ in 0..100 {
                app.update();
            }
            let view = app.world().get::<Transform>(camera).unwrap();
            if detached {
                assert!(view.translation.normalize().dot(detached_direction) > 0.9999);
            } else {
                assert!(
                    view.translation
                        .normalize()
                        .dot(recovered_explorer.normalize())
                        > 0.9999
                );
            }

            let car = summon_and_enter_vehicle(&mut app, explorer, Kind::Car);
            for _ in 0..35 {
                app.update();
            }
            let safe_vehicle_position = app.world().get::<Position>(car).unwrap().0;
            assert!(app.world().resource::<Exploration>().safe.is_some());
            app.world_mut().get_mut::<Vehicle>(car).unwrap().crashed = true;
            let invalid_vehicle_position = Vec3::new(800.0, 2000.6, 0.0);
            app.world_mut().entity_mut(car).insert((
                RigidBody::Static,
                Position(invalid_vehicle_position),
                Rotation(Quat::from_rotation_x(1.2)),
                LinearVelocity::ZERO,
            ));
            act(&mut app, Action::Recover);
            let recovered_vehicle = app.world().get::<Position>(car).unwrap().0;
            assert!(recovered_vehicle.distance(invalid_vehicle_position) > 100.0);
            assert!(recovered_vehicle.distance(safe_vehicle_position) < 10.0);
            assert_eq!(app.world().resource::<Exploration>().occupied, Some(car));
            assert!(!app.world().get::<Vehicle>(car).unwrap().crashed);
            assert!(
                app.world()
                    .resource::<Exploration>()
                    .planet_camera
                    .is_requested_open()
            );
            for _ in 0..100 {
                app.update();
            }
            let view = app.world().get::<Transform>(camera).unwrap();
            if detached {
                assert!(view.translation.normalize().dot(detached_direction) > 0.9999);
            } else {
                assert!(
                    view.translation
                        .normalize()
                        .dot(recovered_vehicle.normalize())
                        > 0.9999
                );
            }
        }
    }

    #[test]
    fn clearance_limited_zoom_resumes_smoothly_after_a_structure_is_removed() {
        let (mut app, _) = fixture();
        let planet_radius = shared::sphere::PLANET_RADIUS;
        app.world_mut().spawn((
            RigidBody::Static,
            Collider::sphere(planet_radius),
            Transform::default(),
            Ground,
        ));
        let camera = app
            .world_mut()
            .spawn((
                MainCamera,
                Transform::from_xyz(0.0, planet_radius + 5.0, -5.0).looking_at(Vec3::ZERO, Vec3::Y),
                Projection::Perspective(PerspectiveProjection::default()),
            ))
            .id();
        app.world_mut()
            .resource_mut::<Exploration>()
            .planet_camera
            .toggle();
        for _ in 0..140 {
            app.update();
        }
        assert!(
            app.world()
                .get::<Transform>(camera)
                .unwrap()
                .translation
                .length()
                > planet_radius * 2.0
        );

        let mut collision = world::CollisionWorld::default();
        collision.obstacles.push(world::Obstacle::new(
            Vec3::Y * (planet_radius + 500.0),
            Quat::IDENTITY,
            Collider::cuboid(20.0, 30.0, 20.0),
        ));
        app.world_mut().insert_resource(collision);
        app.update();
        app.world_mut()
            .resource_mut::<Exploration>()
            .planet_camera
            .request_radius(shared::planet_view::PLANET_VIEW_NEAR_RADIUS);

        for _ in 0..120 {
            app.update();
            let pose = app.world().get::<Transform>(camera).unwrap();
            assert!(
                pose.translation.length() > shared::planet_view::PLANET_VIEW_NEAR_RADIUS + 50.0,
                "camera radius was {} while zoom was obstructed",
                pose.translation.length()
            );
        }
        let blocked_radius = app
            .world()
            .get::<Transform>(camera)
            .unwrap()
            .translation
            .length();
        let requested_radius = app
            .world()
            .resource::<Exploration>()
            .planet_camera
            .requested_radius();
        assert_eq!(
            requested_radius,
            shared::planet_view::PLANET_VIEW_NEAR_RADIUS
        );
        assert!(blocked_radius > requested_radius + 50.0);
        let blocked_zoom = app
            .world()
            .resource::<Exploration>()
            .planet_camera
            .normalized_attained_zoom();
        assert!(blocked_zoom > 0.01 && blocked_zoom < 0.1);

        app.world_mut()
            .resource_mut::<world::CollisionWorld>()
            .obstacles
            .clear();
        app.update();
        let first_release_radius = app
            .world()
            .get::<Transform>(camera)
            .unwrap()
            .translation
            .length();
        assert!(blocked_radius - first_release_radius < 60.0);
        assert!(first_release_radius > requested_radius);
        for _ in 0..90 {
            app.update();
            let pose = app.world().get::<Transform>(camera).unwrap();
            assert!(pose.translation.length() > planet_radius + 0.1);
        }
        let released_radius = app
            .world()
            .get::<Transform>(camera)
            .unwrap()
            .translation
            .length();
        assert!((released_radius - requested_radius).abs() < 5.0);
        let normalized = app
            .world()
            .resource::<Exploration>()
            .planet_camera
            .normalized_attained_zoom();
        assert!(normalized < 0.01);
    }

    #[test]
    fn antipodal_orbit_and_interrupted_return_stay_outside_the_planet() {
        let (mut app, explorer) = fixture();
        let planet_radius = shared::sphere::PLANET_RADIUS;
        app.world_mut().spawn((
            RigidBody::Static,
            Collider::sphere(planet_radius),
            Transform::default(),
        ));
        let camera = app
            .world_mut()
            .spawn((
                MainCamera,
                Transform::from_xyz(0.0, planet_radius + 5.0, -5.0).looking_at(Vec3::ZERO, Vec3::Y),
                Projection::Perspective(PerspectiveProjection::default()),
            ))
            .id();
        app.world_mut()
            .resource_mut::<Exploration>()
            .planet_camera
            .toggle();
        for _ in 0..20 {
            app.update();
        }
        let before_opening_reversal = *app.world().get::<Transform>(camera).unwrap();
        app.world_mut()
            .resource_mut::<Exploration>()
            .planet_camera
            .close();
        app.update();
        let closing_reversal = *app.world().get::<Transform>(camera).unwrap();
        assert!(
            closing_reversal
                .translation
                .distance(before_opening_reversal.translation)
                < 300.0
        );
        app.world_mut()
            .resource_mut::<Exploration>()
            .planet_camera
            .toggle();
        app.update();
        let opening_reversal = *app.world().get::<Transform>(camera).unwrap();
        assert!(
            opening_reversal
                .translation
                .distance(closing_reversal.translation)
                < 300.0
        );
        for _ in 0..140 {
            app.update();
        }

        let opposite = Vec3::new(0.001, -1.0, 0.0).normalize();
        app.world_mut()
            .resource_mut::<Exploration>()
            .planet_camera
            .detach(opposite);
        for _ in 0..180 {
            app.update();
            let pose = app.world().get::<Transform>(camera).unwrap();
            assert!(pose.translation.length() > planet_radius + 0.1);
        }
        let opposite_view = app.world().get::<Transform>(camera).unwrap();
        assert!(opposite_view.translation.normalize().dot(opposite) > 0.99);

        let mut collision = world::CollisionWorld::default();
        collision.obstacles.push(world::Obstacle::new(
            opposite * (planet_radius + 500.0),
            Quat::from_rotation_arc(Vec3::Y, opposite),
            Collider::cuboid(20.0, 30.0, 20.0),
        ));
        app.world_mut().insert_resource(collision);
        app.update();
        app.world_mut()
            .resource_mut::<Exploration>()
            .planet_camera
            .request_radius(shared::planet_view::PLANET_VIEW_NEAR_RADIUS);
        for _ in 0..120 {
            app.update();
        }
        let clearance_limited_view = *app.world().get::<Transform>(camera).unwrap();
        assert!(
            clearance_limited_view.translation.length()
                > shared::planet_view::PLANET_VIEW_NEAR_RADIUS + 50.0
        );
        app.world_mut().remove_resource::<world::CollisionWorld>();

        app.world_mut()
            .resource_mut::<Exploration>()
            .planet_camera
            .close();
        for _ in 0..12 {
            app.update();
            let pose = app.world().get::<Transform>(camera).unwrap();
            assert!(pose.translation.length() > planet_radius + 0.1);
        }
        let before_snap = *app.world().get::<Transform>(camera).unwrap();
        act(
            &mut app,
            Action::Teleport(Vec3::new(20.0, planet_radius, 0.0)),
        );
        let after_snap = *app.world().get::<Transform>(camera).unwrap();
        let relocated_body = app.world().get::<Position>(explorer).unwrap().0;
        assert!(relocated_body.x > 10.0);
        assert!(after_snap.translation.distance(before_snap.translation) < 500.0);
        assert!(after_snap.translation.length() > planet_radius + 0.1);
        assert!(!app.world().resource::<Exploration>().snap_camera);

        app.world_mut()
            .resource_mut::<Exploration>()
            .planet_camera
            .zoom_by(0.8);
        assert!(
            app.world()
                .resource::<Exploration>()
                .planet_camera
                .is_requested_open()
        );
        assert!(
            !app.world()
                .resource::<Exploration>()
                .planet_camera
                .follows_body()
        );
        let before_resume = after_snap;
        app.update();
        let resumed = *app.world().get::<Transform>(camera).unwrap();
        assert!(resumed.translation.distance(before_resume.translation) < 100.0);

        app.world_mut()
            .resource_mut::<Exploration>()
            .planet_camera
            .close();
        for _ in 0..180 {
            app.update();
            let pose = app.world().get::<Transform>(camera).unwrap();
            assert!(pose.translation.length() > planet_radius + 0.1);
        }
        let body = app.world().get::<Position>(explorer).unwrap().0;
        let returned = app.world().get::<Transform>(camera).unwrap();
        assert!(returned.translation.distance(body) < 30.0);
        assert!(
            !app.world()
                .resource::<Exploration>()
                .planet_camera
                .is_active(),
            "Planet view stayed active at camera radius {:.1} m, attained radius {:.1} m, requested open {}",
            returned.translation.length(),
            app.world()
                .resource::<Exploration>()
                .planet_camera
                .attained_radius(),
            app.world()
                .resource::<Exploration>()
                .planet_camera
                .is_requested_open(),
        );
    }
    #[test]
    fn car_carries_speed_from_the_captured_downhill_onto_flat_ground() {
        let (mut app, explorer) = fixture();
        let ground = app
            .world_mut()
            .query_filtered::<Entity, With<Ground>>()
            .single(app.world())
            .unwrap();
        app.world_mut().despawn(ground);
        let level = shared::level::LevelData::from_artifact_bytes(include_bytes!(
            "../../assets/level_1337.bin"
        ))
        .unwrap();
        let position = Vec3::new(-1108.6824, 1206.2986, -1174.9358);
        let heading = Vec3::new(-0.52003485, 0.30083457, 0.7994138);
        let normal = Vec3::new(-0.76097286, 0.6037492, -0.23750219);
        let patch = level
            .terrain_tris
            .iter()
            .filter(|tri| {
                tri.iter()
                    .any(|p| Vec3::from_array(*p).distance(position) < 60.0)
            })
            .copied()
            .collect::<Vec<_>>();
        app.world_mut().spawn((
            RigidBody::Static,
            crate::map::build_collider(&patch),
            Transform::default(),
            Ground,
        ));
        app.world_mut()
            .entity_mut(explorer)
            .insert((ColliderDisabled, RigidBody::Kinematic));
        let mut vehicle = Vehicle::new(Kind::Car, heading);
        vehicle.parked = false;
        let rotation = facing(tangent(heading, normal), normal);
        let car = app
            .world_mut()
            .spawn((
                vehicle,
                RigidBody::Dynamic,
                Kind::Car.collider(),
                Mass(800.0),
                Position(position),
                Rotation(rotation),
                Transform::from_translation(position).with_rotation(rotation),
                CollidingEntities::default(),
                SweptCcd::default(),
                physics_reset(),
            ))
            .insert(LinearVelocity(Vec3::new(-3.1557772, 0.7239393, 11.952755)))
            .id();
        app.world_mut().resource_mut::<Exploration>().occupied = Some(car);
        let mut minimum = f32::INFINITY;
        for _ in 0..180 {
            let speed = app.world().get::<LinearVelocity>(car).unwrap().length();
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keys.reset_all();
            if speed < 11.8 {
                keys.press(KeyCode::KeyW);
            } else if speed > 12.4 {
                keys.press(KeyCode::KeyS);
            }
            app.update();
            minimum = minimum.min(app.world().get::<LinearVelocity>(car).unwrap().length());
        }
        assert!(
            minimum > 10.0,
            "a terrain transition must not stall a cruising car: minimum={minimum}"
        );
        assert!(
            app.world()
                .get::<Position>(car)
                .unwrap()
                .0
                .distance(position)
                > 25.0
        );
    }

    #[test]
    fn car_settles_at_captured_road_stop() {
        let (mut app, explorer) = fixture();
        let ground = app
            .world_mut()
            .query_filtered::<Entity, With<Ground>>()
            .single(app.world())
            .unwrap();
        app.world_mut().despawn(ground);
        let level = shared::level::LevelData::from_artifact_bytes(include_bytes!(
            "../../assets/level_1337.bin"
        ))
        .unwrap();
        let position = Vec3::new(-1120.5538, 1211.4692, -1145.928);
        let patch = level
            .terrain_tris
            .iter()
            .filter(|tri| {
                tri.iter()
                    .any(|p| Vec3::from_array(*p).distance(position) < 30.0)
            })
            .copied()
            .collect::<Vec<_>>();
        app.world_mut().spawn((
            RigidBody::Static,
            crate::map::build_collider(&patch),
            Transform::default(),
            Ground,
        ));
        app.world_mut()
            .entity_mut(explorer)
            .insert((ColliderDisabled, RigidBody::Kinematic));
        let heading = tangent(
            Vec3::from_array(level.roads[20].points[38])
                - Vec3::from_array(level.roads[20].points[37]),
            position.normalize(),
        );
        let mut vehicle = Vehicle::new(Kind::Car, heading);
        vehicle.parked = false;
        let rotation = facing(heading, position.normalize());
        let car = app
            .world_mut()
            .spawn((
                vehicle,
                RigidBody::Dynamic,
                Kind::Car.collider(),
                Mass(800.0),
                Position(position),
                Rotation(rotation),
                Transform::from_translation(position).with_rotation(rotation),
                CollidingEntities::default(),
                SweptCcd::default(),
                physics_reset(),
            ))
            .id();
        app.world_mut().resource_mut::<Exploration>().occupied = Some(car);
        for _ in 0..600 {
            app.update();
        }
        assert!(
            app.world().get::<Vehicle>(car).unwrap().stable > 0.3,
            "a stopped car must settle: velocity={:?}, support={:?}, position={:?}, contacts={:?}, stable={}",
            app.world().get::<LinearVelocity>(car),
            app.world().get::<Vehicle>(car).unwrap().support_normal,
            app.world().get::<Position>(car),
            app.world().get::<CollidingEntities>(car),
            app.world().get::<Vehicle>(car).unwrap().stable
        );
    }

    #[test]
    fn car_spanning_a_crest_keeps_both_axles_level() {
        let (mut app, explorer) = fixture();
        let ground = app
            .world_mut()
            .query_filtered::<Entity, With<Ground>>()
            .single(app.world())
            .unwrap();
        app.world_mut().despawn(ground);
        app.world_mut()
            .entity_mut(explorer)
            .insert(ColliderDisabled);
        let mut triangles = Vec::new();
        for (a, b) in [(-10.0_f32, 0.0_f32), (0.0, 10.0)] {
            let p = |x, z: f32| [x, 2000.0 - z.abs() * 0.3, z];
            triangles.push([p(-10.0, a), p(-10.0, b), p(10.0, a)]);
            triangles.push([p(10.0, a), p(-10.0, b), p(10.0, b)]);
        }
        app.world_mut().spawn((
            RigidBody::Static,
            crate::map::build_collider(&triangles),
            Transform::default(),
            Ground,
        ));
        let car = app
            .world_mut()
            .spawn((
                Vehicle::new(Kind::Car, Vec3::NEG_Z),
                RigidBody::Static,
                Kind::Car.collider(),
                Position(Vec3::Y * 2000.55),
                Rotation::default(),
                Transform::from_xyz(0.0, 2000.55, 0.0),
                CollidingEntities::default(),
            ))
            .id();
        for _ in 0..90 {
            app.update();
        }
        let forward = app.world().get::<Rotation>(car).unwrap().0 * Vec3::NEG_Z;
        assert!(
            forward.y.abs() < 0.02,
            "equal-height front and rear ground must not tip the car onto one axle: {forward:?}"
        );
    }

    #[test]
    fn car_body_follows_the_supporting_slope_in_pitch_and_roll() {
        let (mut app, _) = fixture();
        let ground = app
            .world_mut()
            .query_filtered::<Entity, With<Ground>>()
            .single(app.world())
            .unwrap();
        let slope = Quat::from_rotation_x(0.15) * Quat::from_rotation_z(0.10);
        app.world_mut().entity_mut(ground).insert(Rotation(slope));
        let car = app
            .world_mut()
            .spawn((
                Vehicle::new(Kind::Car, Vec3::NEG_Z),
                RigidBody::Static,
                Kind::Car.collider(),
                Position(Vec3::Y * 2000.55),
                Rotation::default(),
                Transform::from_xyz(0.0, 2000.55, 0.0),
                CollidingEntities::default(),
            ))
            .id();
        for _ in 0..90 {
            app.update();
        }
        let rotation = app.world().get::<Rotation>(car).unwrap().0;
        let normal = slope * Vec3::Y;
        assert!(
            (rotation * Vec3::Y).dot(normal) > 0.999,
            "car must follow both slope axes"
        );
        assert!(
            (rotation * Vec3::NEG_Z).dot(tangent(Vec3::NEG_Z, normal)) > 0.999,
            "terrain alignment must preserve the driver's heading"
        );
    }

    #[test]
    fn selector_pause_and_cancel_do_not_leak_held_airbrake() {
        let (mut app, _) = fixture();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyV);
        app.update();
        assert!(app.world().resource::<Time<Virtual>>().is_paused());
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .clear();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::ControlLeft);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Escape);
        app.update();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .clear();
        app.update();
        assert!(!app.world().resource::<Time<Virtual>>().is_paused());
        assert!(app.world().resource::<Exploration>().suppress_input);
        assert!(
            app.world()
                .resource::<Exploration>()
                .vehicles
                .iter()
                .all(Option::is_none)
        );
    }

    #[test]
    fn plane_runway_follows_the_curved_planet() {
        let (mut app, _) = fixture();
        let floor = app
            .world_mut()
            .query_filtered::<Entity, With<Ground>>()
            .single(app.world())
            .unwrap();
        app.world_mut().despawn(floor);
        app.world_mut().spawn((
            RigidBody::Static,
            Collider::sphere(2000.0),
            Transform::default(),
            Ground,
        ));
        for _ in 0..3 {
            app.update();
        }
        act(&mut app, Action::Summon(Kind::Plane));
        assert!(
            app.world().resource::<Exploration>().vehicles[1].is_some(),
            "{}",
            app.world().resource::<Exploration>().message
        );
    }

    #[test]
    fn failed_vehicle_recovery_keeps_the_occupied_wreck_unchanged() {
        let (mut app, explorer) = fixture();
        act(&mut app, Action::Summon(Kind::Car));
        let car = app.world().resource::<Exploration>().vehicles[0].unwrap();
        for _ in 0..30 {
            app.update();
        }
        let p = app.world().get::<Position>(car).unwrap().0;
        app.world_mut()
            .entity_mut(explorer)
            .insert(Position(p + Vec3::X * 2.0));
        act(&mut app, Action::Interact);
        app.world_mut().get_mut::<Vehicle>(car).unwrap().crashed = true;
        let wreck_pose = Rotation(Quat::from_rotation_x(1.3));
        app.world_mut()
            .entity_mut(car)
            .remove::<LockedAxes>()
            .insert((RigidBody::Static, wreck_pose));
        let floor = app
            .world_mut()
            .query_filtered::<Entity, With<Ground>>()
            .single(app.world())
            .unwrap();
        app.world_mut().despawn(floor);
        for _ in 0..3 {
            app.update();
        }
        let before = *app.world().get::<Position>(car).unwrap();
        act(&mut app, Action::Recover);
        assert_eq!(app.world().resource::<Exploration>().occupied, Some(car));
        assert!(app.world().get::<Vehicle>(car).unwrap().crashed);
        assert_eq!(*app.world().get::<Position>(car).unwrap(), before);
        assert_eq!(*app.world().get::<Rotation>(car).unwrap(), wreck_pose);
        assert!(app.world().get::<LockedAxes>(car).is_none());
        assert!(
            app.world()
                .resource::<Exploration>()
                .message
                .contains("recovery failed")
        );
    }

    #[test]
    fn occupied_recovery_repairs_and_repositions_the_same_vehicle() {
        for kind in [Kind::Car, Kind::Plane] {
            let (mut app, explorer) = fixture();
            act(&mut app, Action::Summon(kind));
            let vehicle = app.world().resource::<Exploration>().vehicles[kind.index()].unwrap();
            for _ in 0..30 {
                app.update();
            }
            let p = app.world().get::<Position>(vehicle).unwrap().0;
            let side = app.world().get::<Rotation>(vehicle).unwrap().0 * Vec3::X;
            let width = if kind == Kind::Car { 2.0 } else { 4.8 };
            app.world_mut()
                .entity_mut(explorer)
                .insert(Position(p + side * width));
            act(&mut app, Action::Interact);
            assert_eq!(
                app.world().resource::<Exploration>().occupied,
                Some(vehicle)
            );
            app.world_mut().get_mut::<Vehicle>(vehicle).unwrap().crashed = true;
            app.world_mut()
                .entity_mut(vehicle)
                .remove::<LockedAxes>()
                .insert((
                    Position(p + Vec3::Y * 20.0),
                    Rotation(Quat::from_rotation_x(1.3)),
                    LinearVelocity(Vec3::X * 5.0),
                    AngularVelocity(Vec3::Y),
                    Friction::new(0.6),
                    LinearDamping(0.1),
                    AngularDamping(1.5),
                ));
            act(&mut app, Action::Recover);
            assert_eq!(
                app.world().resource::<Exploration>().occupied,
                Some(vehicle)
            );
            let v = app.world().get::<Vehicle>(vehicle).unwrap();
            assert!(!v.crashed && !v.parked && !v.flight.airborne);
            assert_eq!(
                app.world().get::<LinearVelocity>(vehicle).unwrap().0,
                Vec3::ZERO
            );
            assert_eq!(
                app.world().get::<AngularVelocity>(vehicle).unwrap().0,
                Vec3::ZERO
            );
            assert!(app.world().get::<LockedAxes>(vehicle).is_some());
            assert_eq!(app.world().get::<LinearDamping>(vehicle).unwrap().0, 0.0);
            assert!(app.world().get::<Position>(vehicle).unwrap().y < 2002.0);
            assert!(app.world().get::<ColliderDisabled>(explorer).is_some());
            assert_eq!(
                app.world().resource::<Exploration>().vehicles[kind.index()],
                Some(vehicle)
            );
        }
    }

    #[test]
    fn occupied_actions_are_atomic_and_recovery_requires_a_complete_hold() {
        let (mut app, explorer) = fixture();
        act(&mut app, Action::Summon(Kind::Car));
        let car = app.world().resource::<Exploration>().vehicles[0].unwrap();
        for _ in 0..30 {
            app.update();
        }
        let p = app.world().get::<Position>(car).unwrap().0;
        app.world_mut()
            .entity_mut(explorer)
            .insert(Position(p + Vec3::X * 2.0));
        act(&mut app, Action::Interact);
        assert_eq!(app.world().resource::<Exploration>().occupied, Some(car));
        act(&mut app, Action::Summon(Kind::Plane));
        assert!(app.world().resource::<Exploration>().vehicles[1].is_none());
        act(&mut app, Action::Teleport(Vec3::Y * 2100.0));
        assert_eq!(app.world().resource::<Exploration>().occupied, Some(car));
        assert!(app.world().get::<Position>(car).unwrap().y < 2002.0);
        app.world_mut()
            .entity_mut(car)
            .insert(Position(Vec3::new(30.0, 2050.0, 0.0)));
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyR);
        for _ in 0..20 {
            app.update();
        }
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .release(KeyCode::KeyR);
        app.update();
        assert_eq!(app.world().resource::<Exploration>().occupied, Some(car));
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyR);
        for _ in 0..65 {
            app.update();
        }
        assert_eq!(app.world().resource::<Exploration>().occupied, Some(car));
        assert!(app.world().get::<Vehicle>(car).is_some());
        assert!(app.world().get::<Position>(explorer).unwrap().y < 2002.0);
        assert!(app.world().get::<Position>(car).unwrap().y < 2002.0);
    }

    #[test]
    fn each_distant_request_waits_for_its_own_collision_preparation() {
        let (mut app, explorer) = fixture();
        let first = Action::Teleport(Vec3::new(50.0, 2000.0, 0.0));
        let second = Action::Teleport(Vec3::new(-50.0, 2000.0, 0.0));
        let mut collision = world::CollisionWorld::default();
        collision.ready = true;
        collision.last_action = Some(first);
        app.world_mut().insert_resource(collision);
        app.world_mut().resource_mut::<Exploration>().request(first);
        app.world_mut()
            .resource_mut::<Exploration>()
            .request(second);
        app.update();
        assert_eq!(app.world().resource::<Exploration>().actions.len(), 1);
        assert!(app.world().get::<Position>(explorer).unwrap().x > 40.0);
        app.update();
        app.update();
        assert!(app.world().resource::<Exploration>().actions.is_empty());
        assert!(app.world().get::<Position>(explorer).unwrap().x < -40.0);
    }

    #[test]
    fn blocked_map_target_fails_instead_of_searching_somewhere_else() {
        let (mut app, explorer) = fixture();
        app.world_mut().spawn((
            RigidBody::Static,
            Collider::cuboid(5.0, 2.0, 5.0),
            Transform::from_xyz(20.0, 2001.0, 0.0),
            Ground,
            crate::chunks::WorldObstacle,
        ));
        for _ in 0..3 {
            app.update();
        }
        act(&mut app, Action::Teleport(Vec3::new(20.0, 2000.0, 0.0)));
        assert!(app.world().get::<Position>(explorer).unwrap().x.abs() < 1.0);
    }

    #[test]
    fn blocked_exit_preserves_occupancy_then_uses_the_clear_side() {
        let (mut app, explorer) = fixture();
        act(&mut app, Action::Summon(Kind::Car));
        let car = app.world().resource::<Exploration>().vehicles[0].unwrap();
        for _ in 0..30 {
            app.update();
        }
        let p = app.world().get::<Position>(car).unwrap().0;
        app.world_mut()
            .entity_mut(explorer)
            .insert(Position(p + Vec3::X * 2.0));
        act(&mut app, Action::Interact);
        for _ in 0..30 {
            app.update();
        }
        let mut walls = Vec::new();
        for side in [-1.0, 1.0] {
            walls.push(
                app.world_mut()
                    .spawn((
                        RigidBody::Static,
                        Collider::cuboid(0.6, 3.0, 3.0),
                        Transform::from_translation(p + Vec3::X * side * 1.6),
                        Ground,
                        crate::chunks::WorldObstacle,
                    ))
                    .id(),
            );
        }
        for _ in 0..3 {
            app.update();
        }
        act(&mut app, Action::Interact);
        assert_eq!(app.world().resource::<Exploration>().occupied, Some(car));
        assert!(!app.world().get::<Vehicle>(car).unwrap().parked);
        app.world_mut().despawn(walls[1]);
        for _ in 0..3 {
            app.update();
        }
        act(&mut app, Action::Interact);
        assert!(app.world().resource::<Exploration>().occupied.is_none());
        assert!(app.world().get::<Position>(explorer).unwrap().x > p.x + 1.0);
        assert!(app.world().get::<Vehicle>(car).unwrap().parked);
    }

    #[test]
    fn production_obstacle_contact_crashes_taxiing_and_departing_planes() {
        for airborne in [false, true] {
            let (mut app, _) = fixture();
            act(&mut app, Action::Summon(Kind::Plane));
            let plane = app.world().resource::<Exploration>().vehicles[1].unwrap();
            let p = app.world().get::<Position>(plane).unwrap().0;
            {
                let mut v = app.world_mut().get_mut::<Vehicle>(plane).unwrap();
                v.parked = false;
                v.flight.airborne = airborne;
            }
            app.world_mut()
                .entity_mut(plane)
                .insert((RigidBody::Dynamic, LinearVelocity(Vec3::X)));
            let side = app.world().get::<Rotation>(plane).unwrap().0 * Vec3::X;
            app.world_mut().spawn((
                RigidBody::Static,
                Collider::cuboid(0.5, 4.0, 0.5),
                Transform::from_translation(p + side * 3.0),
                Ground,
                crate::chunks::WorldObstacle,
            ));
            for _ in 0..5 {
                app.update();
            }
            assert!(
                app.world().get::<Vehicle>(plane).unwrap().crashed,
                "airborne={airborne}, p={:?}, rotation={:?}, contacts={:?}",
                app.world().get::<Position>(plane),
                app.world().get::<Rotation>(plane),
                app.world().get::<CollidingEntities>(plane)
            );
            assert!(app.world().get::<LockedAxes>(plane).is_none());
            assert!(app.world().get::<Position>(plane).unwrap().distance(p) < 5.0);
        }
    }

    #[test]
    fn car_placement_tries_a_rotated_pose_on_a_narrow_platform() {
        let (mut app, explorer) = fixture();
        let floor = app
            .world_mut()
            .query_filtered::<Entity, With<Ground>>()
            .single(app.world())
            .unwrap();
        app.world_mut().despawn(floor);
        app.world_mut().spawn((
            RigidBody::Static,
            Collider::cuboid(3.8, 1.0, 2.2),
            Transform::from_xyz(0.0, 1999.5, 0.0),
            Ground,
        ));
        for _ in 0..3 {
            app.update();
        }
        let mut query = bevy::ecs::system::SystemState::<Placement>::new(app.world_mut());
        let placement = query.get(app.world()).unwrap();
        let result = placement.locate(
            Vec3::Y * 2000.6,
            Vec3::NEG_Z,
            Some(Kind::Car),
            &[explorer],
            0.0,
        );
        let (_, heading) = result.expect("the car fits sideways on the platform");
        assert!(heading.dot(Vec3::X).abs() > 0.99);
    }

    #[test]
    fn airborne_explorer_does_not_update_last_safe_ground() {
        let (mut app, explorer) = fixture();
        let previous = Vec3::new(10.0, 2000.58, 0.0);
        app.world_mut().resource_mut::<Exploration>().safe = Some(previous);
        app.world_mut().entity_mut(explorer).insert((
            Position(Vec3::new(0.0, 2000.65, 0.0)),
            LinearVelocity(Vec3::Y),
        ));
        app.update();
        assert_eq!(app.world().resource::<Exploration>().safe, Some(previous));
        app.world_mut().insert_resource(Gravity(Vec3::NEG_Y * 20.0));
        for _ in 0..60 {
            app.update();
        }
        let safe = app.world().resource::<Exploration>().safe.unwrap();
        assert!(safe.distance(Vec3::new(0.0, 2000.58, 0.0)) < 0.2);
    }

    #[test]
    fn remote_relocation_updates_heading_before_camera_snap() {
        for recover in [false, true] {
            let (mut app, explorer) = fixture();
            let destination = Vec3::NEG_Z * 2000.6;
            let floor = app
                .world_mut()
                .query_filtered::<Entity, With<Ground>>()
                .single(app.world())
                .unwrap();
            app.world_mut().despawn(floor);
            app.world_mut().spawn((
                RigidBody::Static,
                Collider::sphere(2000.0),
                Transform::default(),
                Ground,
            ));
            let camera = app
                .world_mut()
                .spawn((MainCamera, Transform::default()))
                .id();
            for _ in 0..3 {
                app.update();
            }
            app.world_mut().resource_mut::<Exploration>().safe = Some(destination);
            act(
                &mut app,
                if recover {
                    Action::Recover
                } else {
                    Action::Teleport(destination)
                },
            );
            let p = app.world().get::<Position>(explorer).unwrap().0;
            assert!(
                p.distance(destination) < 0.2,
                "{p:?}: {}",
                app.world().resource::<Exploration>().message
            );
            let heading = app.world().get::<Player>(explorer).unwrap().heading;
            assert!(
                heading.dot(p.normalize()).abs() < 0.001,
                "relocated heading must be tangent"
            );
            let view = app.world().get::<Transform>(camera).unwrap();
            assert!(view.rotation.is_finite());
            assert!(view.translation.length() > p.length());
        }
    }

    #[test]
    fn entering_vehicle_blends_from_previous_camera_position() {
        let (mut app, explorer) = fixture();
        let camera = app
            .world_mut()
            .spawn((MainCamera, Transform::default()))
            .id();
        act(&mut app, Action::Summon(Kind::Car));
        let car = app.world().resource::<Exploration>().vehicles[0].unwrap();
        let p = app.world().get::<Position>(car).unwrap().0;
        app.world_mut()
            .entity_mut(explorer)
            .insert(Position(p + Vec3::X * 2.0));
        for _ in 0..60 {
            app.update();
        }
        let before = app.world().get::<Transform>(camera).unwrap().translation;
        act(&mut app, Action::Interact);
        assert_eq!(app.world().resource::<Exploration>().occupied, Some(car));
        let after = app.world().get::<Transform>(camera).unwrap().translation;
        assert!(
            before.distance(after) < 1.0,
            "entry camera jumped {} m",
            before.distance(after)
        );
    }

    #[test]
    fn highlighted_vehicle_keeps_opaque_depth_rendering() {
        let (mut app, explorer) = fixture();
        app.world_mut().spawn((MainCamera, Transform::default()));
        act(&mut app, Action::Summon(Kind::Car));
        let car = app.world().resource::<Exploration>().vehicles[0].unwrap();
        for _ in 0..30 {
            app.update();
        }
        let p = app.world().get::<Position>(car).unwrap().0;
        app.world_mut()
            .entity_mut(explorer)
            .insert(Position(p + Vec3::X * 2.0));
        let material = app
            .world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial::default());
        let mesh = app
            .world_mut()
            .spawn((ChildOf(car), MeshMaterial3d(material), Transform::default()))
            .id();
        app.update();
        assert_eq!(app.world().resource::<Exploration>().target, Some(car));
        let handle = &app
            .world()
            .get::<MeshMaterial3d<StandardMaterial>>(mesh)
            .unwrap()
            .0;
        assert_eq!(
            app.world()
                .resource::<Assets<StandardMaterial>>()
                .get(handle)
                .unwrap()
                .alpha_mode,
            AlphaMode::Opaque
        );
    }

    #[test]
    fn plane_near_ground_is_not_enterable_while_airborne() {
        let (mut app, explorer) = fixture();
        act(&mut app, Action::Summon(Kind::Plane));
        let plane = app.world().resource::<Exploration>().vehicles[1].unwrap();
        for _ in 0..30 {
            app.update();
        }
        let p = app.world().get::<Position>(plane).unwrap().0;
        app.world_mut()
            .entity_mut(explorer)
            .insert(Position(p + Vec3::X * 4.8));
        {
            let mut v = app.world_mut().get_mut::<Vehicle>(plane).unwrap();
            v.flight.airborne = true;
            v.parked = false;
            v.stable = 1.0;
        }
        app.world_mut()
            .entity_mut(plane)
            .insert((RigidBody::Dynamic, Position(p + Vec3::Y * 0.2)));
        act(&mut app, Action::Interact);
        assert!(app.world().resource::<Exploration>().occupied.is_none());
    }

    #[test]
    fn entering_a_plane_while_touching_its_wing_does_not_crash() {
        let (mut app, explorer) = fixture();
        act(&mut app, Action::Summon(Kind::Plane));
        let plane = app.world().resource::<Exploration>().vehicles[1].unwrap();
        for _ in 0..30 {
            app.update();
        }
        let p = app.world().get::<Position>(plane).unwrap().0;
        let side = app.world().get::<Rotation>(plane).unwrap().0 * Vec3::X;
        app.world_mut()
            .entity_mut(explorer)
            .insert(Position(p + side * 3.8 + p.normalize() * 0.6));
        for _ in 0..3 {
            app.update();
        }
        act(&mut app, Action::Interact);
        assert_eq!(app.world().resource::<Exploration>().occupied, Some(plane));
        for frame in 0..10 {
            app.update();
            assert!(
                !app.world().get::<Vehicle>(plane).unwrap().crashed,
                "entry crash at frame {frame}"
            );
        }
    }

    #[test]
    fn summoned_plane_takes_off_without_crashing_on_departing_ground() {
        let (mut app, explorer) = fixture();
        act(&mut app, Action::Summon(Kind::Plane));
        let plane = app.world().resource::<Exploration>().vehicles[1].unwrap();
        for _ in 0..30 {
            app.update();
        }
        let p = app.world().get::<Position>(plane).unwrap().0;
        let side = app.world().get::<Rotation>(plane).unwrap().0 * Vec3::X;
        app.world_mut()
            .entity_mut(explorer)
            .insert(Position(p + side * 4.8));
        act(&mut app, Action::Interact);
        assert_eq!(app.world().resource::<Exploration>().occupied, Some(plane));
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::ShiftLeft);
        for frame in 0..600 {
            if app.world().get::<LinearVelocity>(plane).unwrap().length() >= 24.0 {
                app.world_mut()
                    .resource_mut::<ButtonInput<KeyCode>>()
                    .press(KeyCode::KeyS);
            }
            app.update();
            let v = app.world().get::<Vehicle>(plane).unwrap();
            assert!(
                !v.crashed,
                "takeoff crashed at frame {frame}, clearance={}, pitch={}, air_time={}",
                v.clearance, v.flight.pitch, v.air_time
            );
            if v.flight.airborne && v.clearance > 5.0 {
                return;
            }
        }
        panic!("plane did not leave the runway");
    }

    #[test]
    fn hard_ground_impact_crashes_during_takeoff_grace() {
        let (mut app, _) = fixture();
        act(&mut app, Action::Summon(Kind::Plane));
        let plane = app.world().resource::<Exploration>().vehicles[1].unwrap();
        {
            let mut v = app.world_mut().get_mut::<Vehicle>(plane).unwrap();
            v.parked = false;
            v.flight.airborne = true;
            v.flight.pitch = -0.55;
        }
        app.world_mut().entity_mut(plane).insert((
            RigidBody::Dynamic,
            Position(Vec3::new(0.0, 2000.65, -8.0)),
            LinearVelocity(Vec3::new(0.0, -20.0, -30.0)),
        ));
        for _ in 0..5 {
            app.update();
        }
        assert!(app.world().get::<Vehicle>(plane).unwrap().crashed);
    }

    #[test]
    fn summon_search_pause_and_requeue_do_not_reuse_a_partial_cursor() {
        let (mut app, explorer) = fixture();
        act(&mut app, Action::Summon(Kind::Car));
        let car = app.world().resource::<Exploration>().vehicles[0].unwrap();
        let original_position = app.world().get::<Position>(car).unwrap().0;
        block_initial_summon_poses(&mut app);

        act(&mut app, Action::Summon(Kind::Car));

        let state = app.world().resource::<Exploration>();
        assert!(matches!(
            state.actions.front(),
            Some(Action::Summon(Kind::Car))
        ));
        assert_eq!(
            app.world().get::<Position>(car).unwrap().0,
            original_position
        );
        assert!(!state.message.contains("No clear dry ground"));
        assert_eq!(
            state
                .pending_summon
                .as_ref()
                .map(|pending| pending.poses_tested_last_update),
            Some(SUMMON_POSE_BUDGET_PER_UPDATE)
        );
        assert_eq!(
            state
                .pending_summon
                .as_ref()
                .map(|pending| pending.poses_tested_total),
            Some(SUMMON_POSE_BUDGET_PER_UPDATE)
        );
        let request_origin = state.pending_summon.as_ref().unwrap().body_origin;

        // While world preparation is paused, movement does not consume or
        // replace the existing cursor.
        let player_position = app.world().get::<Position>(explorer).unwrap().0;
        app.world_mut()
            .entity_mut(explorer)
            .insert(Position(player_position + Vec3::X));
        app.world_mut()
            .insert_resource(world::CollisionWorld::default());
        app.update();

        assert!(matches!(
            app.world().resource::<Exploration>().actions.front(),
            Some(Action::Summon(Kind::Car))
        ));
        assert_eq!(
            app.world().get::<Position>(car).unwrap().0,
            original_position
        );
        assert_eq!(
            app.world()
                .resource::<Exploration>()
                .pending_summon
                .as_ref()
                .map(|pending| pending.poses_tested_total),
            Some(SUMMON_POSE_BUDGET_PER_UPDATE),
            "an unready collision world must pause the search without consuming poses"
        );
        assert_eq!(
            app.world()
                .resource::<Exploration>()
                .pending_summon
                .as_ref()
                .unwrap()
                .body_origin,
            request_origin,
            "a small walk should retain the request-time candidate basis"
        );

        // A cancelled request followed by the same action gets a fresh cursor;
        // it must not resume at the later pose from the old request.
        app.world_mut()
            .resource_mut::<Exploration>()
            .clear_actions();
        assert!(
            app.world()
                .resource::<Exploration>()
                .pending_summon
                .is_none()
        );
        act(&mut app, Action::Summon(Kind::Car));
        for _ in 0..4 {
            if app
                .world()
                .resource::<Exploration>()
                .pending_summon
                .is_some()
                || app.world().resource::<Exploration>().actions.is_empty()
            {
                break;
            }
            app.update();
        }
        let state = app.world().resource::<Exploration>();
        assert!(matches!(
            state.actions.front(),
            Some(Action::Summon(Kind::Car))
        ));
        assert_eq!(
            state
                .pending_summon
                .as_ref()
                .map(|pending| pending.poses_tested_last_update),
            Some(SUMMON_POSE_BUDGET_PER_UPDATE),
            "a same-kind request starts at the origin candidates again"
        );
        app.world_mut()
            .resource_mut::<Exploration>()
            .request(Action::Interact);
        app.update();
        assert!(matches!(
            app.world().resource::<Exploration>().actions.front(),
            Some(Action::Interact)
        ));

        for _ in 0..4 {
            if app.world().resource::<Exploration>().actions.is_empty() {
                break;
            }
            app.update();
        }

        assert!(app.world().resource::<Exploration>().actions.is_empty());
        assert_eq!(app.world().resource::<Exploration>().vehicles[0], Some(car));
        let repositioned = app.world().get::<Position>(car).unwrap().0;
        let moved = repositioned.distance(original_position);
        assert!(
            (3.0..5.0).contains(&moved),
            "the search should continue from its next ordered anchor, moved {moved:.2} m"
        );
    }

    #[test]
    fn vehicle_selector_v_then_c_runs_the_sliced_summon_path() {
        let (mut app, _) = fixture();
        block_initial_summon_poses(&mut app);

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyV);
        app.update();
        assert!(app.world().resource::<Exploration>().selector);
        assert!(app.world().resource::<Time<Virtual>>().is_paused());

        {
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keys.clear_just_pressed(KeyCode::KeyV);
            keys.release(KeyCode::KeyV);
            keys.press(KeyCode::KeyC);
        }
        app.update();

        {
            let state = app.world().resource::<Exploration>();
            assert!(!state.selector);
            assert!(matches!(
                state.actions.front(),
                Some(Action::Summon(Kind::Car))
            ));
            assert_eq!(
                state
                    .pending_summon
                    .as_ref()
                    .map(|pending| pending.poses_tested_last_update),
                Some(SUMMON_POSE_BUDGET_PER_UPDATE),
                "selector input must enter the same bounded search as direct summon requests"
            );
            assert!(state.vehicles[Kind::Car.index()].is_none());
        }
        assert!(!app.world().resource::<Time<Virtual>>().is_paused());

        {
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keys.clear_just_pressed(KeyCode::KeyC);
            keys.release(KeyCode::KeyC);
        }
        for _ in 0..4 {
            if app.world().resource::<Exploration>().actions.is_empty() {
                break;
            }
            app.update();
        }

        let state = app.world().resource::<Exploration>();
        assert!(state.actions.is_empty());
        let car = state.vehicles[Kind::Car.index()].expect("C summons the selected car");
        assert!(app.world().get::<Vehicle>(car).is_some());
        assert!(state.message.starts_with("Car ready"));
    }

    #[test]
    fn summon_search_keeps_its_candidate_order_during_ordinary_walking() {
        let (mut app, explorer) = fixture();
        act(&mut app, Action::Summon(Kind::Car));
        let car = app.world().resource::<Exploration>().vehicles[0].unwrap();
        block_initial_summon_poses(&mut app);

        act(&mut app, Action::Summon(Kind::Car));
        let request_origin = {
            let state = app.world().resource::<Exploration>();
            assert_eq!(
                state
                    .pending_summon
                    .as_ref()
                    .map(|pending| pending.poses_tested_last_update),
                Some(SUMMON_POSE_BUDGET_PER_UPDATE)
            );
            state.pending_summon.as_ref().unwrap().body_origin
        };

        let player_position = app.world().get::<Position>(explorer).unwrap().0;
        let walked_position = player_position + Vec3::X;
        let player_heading = app.world().get::<Player>(explorer).unwrap().heading;
        let (original_basis_candidate, walked_basis_candidate) = {
            let mut system = bevy::ecs::system::SystemState::<Placement>::new(app.world_mut());
            let placement = system.get(app.world()).unwrap();
            let preferred =
                request_origin + tangent(player_heading, request_origin.normalize()) * 8.0;
            let walked_preferred =
                walked_position + tangent(player_heading, walked_position.normalize()) * 8.0;
            let original = synchronous_vehicle_placement_oracle(
                &placement,
                preferred,
                player_heading,
                Kind::Car,
                &[car],
            );
            let walked = synchronous_vehicle_placement_oracle(
                &placement,
                walked_preferred,
                player_heading,
                Kind::Car,
                &[car],
            );
            (original, walked)
        };
        assert!(
            original_basis_candidate.distance(walked_basis_candidate) > 1.0,
            "fixture must distinguish retaining the request cursor from restarting"
        );
        app.world_mut()
            .entity_mut(explorer)
            .insert(Position(walked_position));
        app.update();
        if let Some(pending) = app
            .world()
            .resource::<Exploration>()
            .pending_summon
            .as_ref()
        {
            assert_eq!(
                pending.body_origin, request_origin,
                "a small walk should continue the request-time candidate basis"
            );
        }
        for _ in 0..4 {
            if app.world().resource::<Exploration>().actions.is_empty() {
                break;
            }
            app.update();
        }

        assert!(app.world().resource::<Exploration>().actions.is_empty());
        let actual = app.world().get::<Position>(car).unwrap().0;
        assert!(
            actual.distance(original_basis_candidate) < 0.01,
            "a one-metre walk should continue the exact request-time candidate order; expected {original_basis_candidate:?}, got {actual:?}"
        );
    }

    #[test]
    fn summon_search_restarts_near_the_body_after_a_material_relocation() {
        let (mut app, explorer) = fixture();
        act(&mut app, Action::Summon(Kind::Car));
        let car = app.world().resource::<Exploration>().vehicles[0].unwrap();
        block_initial_summon_poses(&mut app);

        act(&mut app, Action::Summon(Kind::Car));
        assert!(matches!(
            app.world().resource::<Exploration>().actions.front(),
            Some(Action::Summon(Kind::Car))
        ));

        let relocated = app.world().get::<Position>(explorer).unwrap().0 + Vec3::X * 50.0;
        app.world_mut()
            .entity_mut(explorer)
            .insert(Position(relocated));
        for _ in 0..4 {
            if app.world().resource::<Exploration>().actions.is_empty() {
                break;
            }
            app.update();
        }

        assert!(app.world().resource::<Exploration>().actions.is_empty());
        assert_eq!(app.world().resource::<Exploration>().vehicles[0], Some(car));
        let car_position = app.world().get::<Position>(car).unwrap().0;
        assert!(
            car_position.distance(relocated) < Kind::Car.summon_radius(),
            "material movement should restart locally instead of parking the vehicle at an old candidate"
        );
    }

    #[test]
    fn sustained_movement_cannot_starve_queued_teleport_behind_vehicle_summoning() {
        let (mut app, explorer) = fixture();
        act(&mut app, Action::Summon(Kind::Car));
        let car = app.world().resource::<Exploration>().vehicles[0].unwrap();

        // Keep every nearby candidate blocked so only finite request work can
        // release the queued action; no successful placement can hide starvation.
        app.world_mut().spawn((
            RigidBody::Static,
            Collider::cuboid(600.0, 20.0, 600.0),
            Transform::from_xyz(0.0, 2000.0, 0.0),
        ));
        for _ in 0..3 {
            app.update();
        }
        let origin = app.world().get::<Position>(explorer).unwrap().0;
        let heading = app.world().get::<Player>(explorer).unwrap().heading;
        let synchronous_result = {
            let mut system = bevy::ecs::system::SystemState::<Placement>::new(app.world_mut());
            let placement = system.get(app.world()).unwrap();
            placement.locate(
                origin,
                heading,
                Some(Kind::Car),
                &[car],
                Kind::Car.summon_search_radius(),
            )
        };
        assert!(
            synchronous_result.is_none(),
            "the fixture must keep the synchronous search from finding a fit"
        );

        app.world_mut()
            .resource_mut::<Exploration>()
            .request(Action::Summon(Kind::Car));
        app.update();
        let teleport_target =
            Vec3::new(420.0, (2000.6_f32.powi(2) - 420.0_f32.powi(2)).sqrt(), 0.0);
        app.world_mut()
            .resource_mut::<Exploration>()
            .request(Action::Teleport(teleport_target));

        let mut saw_movement_cancellation = false;
        for _ in 0..80 {
            let position = app.world().get::<Position>(explorer).unwrap().0;
            app.world_mut()
                .entity_mut(explorer)
                .insert(Position(position + Vec3::X * 0.12));
            app.update();
            let state = app.world().resource::<Exploration>();
            if matches!(state.actions.front(), Some(Action::Teleport(_))) {
                assert!(state.message.contains("repeated movement"));
                saw_movement_cancellation = true;
            }
            if app.world().resource::<Exploration>().actions.is_empty() {
                break;
            }
        }

        let state = app.world().resource::<Exploration>();
        assert!(saw_movement_cancellation);
        assert!(
            state.actions.is_empty(),
            "ongoing movement must not leave Summon ahead of later actions forever"
        );
        assert!(state.pending_summon.is_none());
        assert!(state.occupied.is_none());
        assert_eq!(state.message, "Returned to safe ground");
        let final_position = app.world().get::<Position>(explorer).unwrap().0;
        assert!(
            final_position.distance(teleport_target) < 100.0,
            "the queued teleport should execute after the bounded summon failure"
        );
    }

    #[test]
    fn summon_enter_exit_reuses_a_distinct_vehicle() {
        let (mut app, explorer) = fixture();
        act(&mut app, Action::Summon(Kind::Car));
        let car = app.world().resource::<Exploration>().vehicles[0].unwrap();
        assert_ne!(car, explorer);
        assert!(app.world().resource::<Exploration>().occupied.is_none());
        for _ in 0..30 {
            app.update();
        }
        // Move within reach of the parked vehicle, through the same physics pose path.
        let pos = app.world().get::<Position>(car).unwrap().0;
        app.world_mut()
            .entity_mut(explorer)
            .insert(Position(pos + Vec3::X * 2.0));
        act(&mut app, Action::Interact);
        assert_eq!(app.world().resource::<Exploration>().occupied, Some(car));
        for _ in 0..30 {
            app.update();
        }
        act(&mut app, Action::Interact);
        assert!(app.world().resource::<Exploration>().occupied.is_none());
        assert!(app.world().get::<Vehicle>(car).unwrap().parked);
        act(&mut app, Action::Summon(Kind::Car));
        assert_eq!(app.world().resource::<Exploration>().vehicles[0], Some(car));
    }
}
