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
use bevy::prelude::*;
use placement::Placement;
use shared::{
    car_prototype::CarMotion,
    plane_prototype::{FlightInput, PlaneFlight, gentle_landing},
    planet::PlanetMesh,
    planet_view::PlanetViewCamera,
    planet_view_interface::{PlanetViewPointer, PlanetViewPresentation},
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
    planet_presentation: PlanetViewPresentation,
    planet_pointer: PlanetViewPointer,
    pressed_planet_control: Option<Entity>,
    planet_selection_click: Option<Vec2>,
    planet_teleport_requested: bool,
    selected_destination: Option<PlanetDestination>,
}
#[derive(Clone, Copy, PartialEq)]
pub enum Action {
    Summon(Kind),
    Interact,
    Recover,
    Teleport(Vec3),
}
impl Exploration {
    pub fn request(&mut self, action: Action) {
        self.actions.push_back(action);
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
            self.planet_teleport_requested = false;
        }
    }

    pub fn is_planet_view_active(&self) -> bool {
        self.planet_camera.is_active()
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
        if !self.planet_view_ready() {
            return false;
        }
        self.planet_teleport_requested = true;
        true
    }

    /// Take the pending T intent for the destination consumer.
    pub fn take_planet_view_teleport_request(&mut self) -> bool {
        std::mem::take(&mut self.planet_teleport_requested)
    }

    /// Current opacity for gameplay HUD and minimap presentation.
    pub fn gameplay_hud_opacity(&self) -> f32 {
        self.planet_presentation.gameplay_opacity()
    }

    /// Current opacity for controls shown while Planet view is open.
    pub fn planet_view_interface_opacity(&self) -> f32 {
        self.planet_presentation.interface_opacity()
    }

    pub fn planet_view_follows_body(&self) -> bool {
        self.planet_camera.follows_body()
    }

    pub fn toggle_planet_view_follow(&mut self, camera_pose: Transform) {
        self.planet_camera.toggle_follow_from(camera_pose);
    }
}
#[derive(Resource)]
struct Liquid(PlanetMesh);
type ExplorationBody = Or<(With<Player>, With<Vehicle>)>;

#[derive(Component)]
struct Seated;
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExplorationUpdate;

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
                    world::residency,
                )
                    .chain()
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
fn tangent(heading: Vec3, up: Vec3) -> Vec3 {
    let projected = heading - up * heading.dot(up);
    if projected.length_squared() > 1e-8 {
        projected.normalize()
    } else {
        up.any_orthonormal_vector()
    }
}

fn facing(heading: Vec3, up: Vec3) -> Quat {
    Quat::from_mat3(&Mat3::from_cols(heading.cross(up), up, -heading))
}
fn initialize(mut state: ResMut<Exploration>, player: Query<&Position, With<Player>>) {
    state.clear_planet_destination();
    if let Ok(p) = player.single() {
        state.start = Some(p.0);
        state.snap_camera = true;
    }
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
        if state.occupied.is_some() {
            state.message = "Stop and exit before summoning; hold R to recover if trapped".into();
        } else {
            state.selector = true;
            state.planet_selection_click = None;
            state.planet_teleport_requested = false;
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
fn actions(
    mut commands: Commands,
    mut state: ResMut<Exploration>,
    placement: Placement,
    mut player: Query<(Entity, &Position, &mut Player), Without<Vehicle>>,
    mut vehicles: Query<(Entity, &Position, &Rotation, &Collider, &mut Vehicle)>,
    catalog: Option<Res<crate::asset_catalog::AssetCatalog>>,
    world: Option<Res<world::CollisionWorld>>,
) {
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
    // Process only the destination prepared by this frame's residency pass.
    for action in state.actions.pop_front().into_iter() {
        let heading = state
            .occupied
            .and_then(|e| vehicles.get(e).ok().map(|(_, _, _, _, v)| v.flight.heading))
            .unwrap_or(player.heading);
        match action {
            Action::Summon(kind) => {
                if state.occupied.is_some() {
                    state.message = "Exit before summoning".into();
                    continue;
                }
                let existing = state.vehicles[kind.index()];
                let excluded = existing.into_iter().collect::<Vec<_>>();
                let preferred = origin + tangent(heading, origin.normalize()) * 8.0;
                let radius = if kind == Kind::Car { 30.0 } else { 100.0 };
                let Some((p, h)) =
                    placement.locate(preferred, heading, Some(kind), &excluded, radius - 8.0)
                else {
                    state.message =
                        "No clear dry ground with enough room / takeoff run nearby".into();
                    continue;
                };
                let e = existing.unwrap_or_else(|| commands.spawn_empty().id());
                commands.entity(e).insert((
                    Vehicle::new(kind, h),
                    RigidBody::Static,
                    kind.collider(),
                    Mass(if kind == Kind::Car { 800.0 } else { 900.0 }),
                    Position(p),
                    Rotation(facing(h, p.normalize())),
                    Transform::from_translation(p).with_rotation(facing(h, p.normalize())),
                    Visibility::default(),
                    SweptCcd::default(),
                    CollidingEntities::default(),
                    physics_reset(),
                ));
                if existing.is_none()
                    && let Some(catalog) = catalog.as_ref()
                {
                    commands.entity(e).with_child((
                        WorldAssetRoot(catalog.scene(kind.asset())),
                        Transform::from_translation(Vec3::NEG_Y * kind.height()),
                        view::VehicleVisual,
                    ));
                }
                state.vehicles[kind.index()] = Some(e);
                state.message = format!("{} ready — approach and press E", kind.name());
            }
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
        assert!(state.request_planet_view_selection(screen_position));
        assert_eq!(state.take_planet_view_selection(), Some(screen_position));
        assert!(state.request_planet_view_teleport());
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
        let destination = PlanetDestination {
            id: PlanetDestinationId::surface(crate::map::WorldEpoch::new(2), position),
            position,
            surface: PlanetDestinationSurface::Terrain,
            display: "Terrain · 12.4°N, 33.7°W".into(),
        };
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(Exploration {
                selected_destination: Some(destination),
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
    fn t_publishes_a_teleport_intent_only_while_planet_view_is_ready() {
        let (mut app, _) = fixture();
        app.world_mut()
            .resource_mut::<Exploration>()
            .set_planet_view_open(true);
        for _ in 0..20 {
            app.update();
        }
        assert!(app.world().resource::<Exploration>().planet_view_ready());

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyT);
        app.update();
        assert!(
            app.world_mut()
                .resource_mut::<Exploration>()
                .take_planet_view_teleport_request()
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
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyV);
        app.update();
        assert!(app.world().resource::<Exploration>().selector);

        {
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keys.clear_just_pressed(KeyCode::KeyV);
            keys.release(KeyCode::KeyV);
            keys.press(KeyCode::Escape);
            keys.press(KeyCode::KeyM);
            keys.press(KeyCode::KeyF);
            keys.press(KeyCode::KeyT);
        }
        app.update();

        let state = app.world().resource::<Exploration>();
        assert!(!state.selector);
        assert!(!state.planet_camera.is_requested_open());
        assert!(!state.planet_view_ready());
        assert!(state.planet_view_follows_body());
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
        for _ in 0..140 {
            app.update();
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
    fn follow_tracks_the_moving_explorer_heading_and_closes_to_its_latest_pose() {
        let (mut app, explorer) = fixture();
        let camera = spawn_planet_view_camera(&mut app);
        open_planet_view(&mut app);
        let starting_position = app.world().get::<Position>(explorer).unwrap().0;

        app.world_mut()
            .entity_mut(explorer)
            .insert(LinearVelocity(Vec3::X * 4.0));
        for _ in 0..60 {
            app.update();
        }
        let moving_position = app.world().get::<Position>(explorer).unwrap().0;
        assert!(moving_position.distance(starting_position) > 1.0);
        let following_pose = app.world().get::<Transform>(camera).unwrap();
        assert!(
            following_pose
                .translation
                .normalize()
                .dot(moving_position.normalize())
                > 0.9999,
            "follow should track the physics-synchronized explorer position"
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

        let structure = app
            .world_mut()
            .spawn((
                RigidBody::Static,
                Collider::cuboid(20.0, 30.0, 20.0),
                Transform::from_xyz(0.0, planet_radius + 80.0, 0.0),
            ))
            .id();
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

        app.world_mut().despawn(structure);
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

        app.world_mut().spawn((
            RigidBody::Static,
            Collider::cuboid(20.0, 30.0, 20.0),
            Transform::from_translation(opposite * (planet_radius + 80.0)),
        ));
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
        for _ in 0..140 {
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
                .is_active()
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
