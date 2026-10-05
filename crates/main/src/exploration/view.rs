use super::*;
#[derive(Component)]
pub(super) struct VehicleVisual;
#[derive(Default)]
pub(super) struct Chase {
    target: Option<Entity>,
    offset: Vec3,
    distance: f32,
    rotation: Quat,
}
#[derive(Component)]
pub(super) struct FadeMaterial {
    original: Handle<StandardMaterial>,
    unique: Handle<StandardMaterial>,
}

#[derive(Component)]
pub(super) struct VisualOwner(Entity);
#[derive(Component)]
pub(super) struct MovingPart {
    owner: Entity,
    rest: Quat,
    angle: f32,
    steering: f32,
    propeller: bool,
    front: bool,
}
pub(super) fn tag_visuals(
    mut commands: Commands,
    parents: Query<&ChildOf>,
    roots: Query<(), ExplorationBody>,
    meshes: Query<Entity, Added<MeshMaterial3d<StandardMaterial>>>,
    names: Query<(Entity, &Name, &Transform), Added<Name>>,
) {
    let owner = |mut e: Entity| {
        for _ in 0..24 {
            if roots.contains(e) {
                return Some(e);
            }
            let Ok(p) = parents.get(e) else {
                return None;
            };
            e = p.parent();
        }
        None
    };
    for e in &meshes {
        if let Some(root) = owner(e) {
            commands.entity(e).insert(VisualOwner(root));
        }
    }
    for (e, name, tf) in &names {
        if (name.starts_with("vehicle.wheel") || name.as_str() == "vehicle.propeller")
            && let Some(root) = owner(e)
        {
            commands.entity(e).insert(MovingPart {
                owner: root,
                rest: tf.rotation,
                angle: 0.0,
                steering: 0.0,
                propeller: name.as_str() == "vehicle.propeller",
                front: name.contains("front"),
            });
        }
    }
}

pub(super) fn setup(mut commands: Commands, font: Res<crate::ui::UiFont>) {
    crate::ui::spawn_sidebar(&mut commands, &font);
}
#[allow(clippy::too_many_arguments)]
pub(super) fn camera(
    mut commands: Commands,
    time: Res<Time<Real>>,
    mut state: ResMut<Exploration>,
    spatial: SpatialQuery,
    collision_world: Option<Res<world::CollisionWorld>>,
    surface: Option<Res<terra_worldgen::terrain::TerrainGen>>,
    player: Query<(Entity, &Position, &Player), Without<MainCamera>>,
    vehicles: Query<(&Position, &Vehicle), Without<MainCamera>>,
    mut cameras: Query<(&mut Transform, Option<&mut Projection>), With<MainCamera>>,
    mut visuals: Query<(
        Entity,
        &mut MeshMaterial3d<StandardMaterial>,
        Option<&FadeMaterial>,
        &VisualOwner,
    )>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut chase: Local<Chase>,
    mut original_far_plane: Local<Option<f32>>,
) {
    let Ok((explorer, p, player)) = player.single() else {
        return;
    };
    let target = state.occupied.unwrap_or(explorer);
    let (position, heading, height, back, ahead, radius) = if let Some(e) = state.occupied {
        let Ok((p, v)) = vehicles.get(e) else {
            return;
        };
        if v.kind == Kind::Car {
            (p.0, v.flight.heading, 3.0, 8.0, 15.0, 0.2)
        } else {
            (p.0, v.flight.heading, 6.0, 18.0, 30.0, 0.3)
        }
    } else {
        (p.0, player.heading, 2.0, 5.0, 10.0, 0.2)
    };
    let Ok((mut camera, mut projection)) = cameras.single_mut() else {
        return;
    };
    let current_camera = *camera;
    let planet_view_active = state.planet_camera.is_active();
    let up = position.normalize();
    let desired = up * height - heading * back;
    let snap = !planet_view_active && (state.snap_camera || chase.target.is_none());
    if planet_view_active && state.snap_camera {
        state.snap_camera = false;
    }
    if snap {
        chase.offset = desired;
        chase.distance = desired.length();
        state.snap_camera = false;
    } else {
        if chase.target != Some(target) {
            if planet_view_active {
                chase.offset = desired;
                chase.distance = desired.length();
            } else {
                chase.offset = current_camera.translation - position;
                chase.distance = chase.offset.length();
            }
        }
        chase.offset = chase
            .offset
            .lerp(desired, 1.0 - (-8.0 * time.delta_secs()).exp());
    }
    chase.target = Some(target);
    // Cast the blended boom every frame: interpolation must never bypass collision.
    let direction = chase.offset.normalize_or(up);
    let hit = spatial.cast_shape(
        &Collider::sphere(radius),
        position,
        Quat::IDENTITY,
        Dir3::new(direction).unwrap(),
        &ShapeCastConfig::from_max_distance(chase.offset.length()),
        &SpatialQueryFilter::from_excluded_entities([target, explorer]),
    );
    let max = hit.map_or(chase.offset.length(), |h| (h.distance - 0.1).max(0.0));
    chase.distance = if max < chase.distance || snap {
        max
    } else {
        chase.distance + (max - chase.distance) * (1.0 - (-6.0 * time.delta_secs()).exp())
    };
    let chase_position = position + direction * chase.distance;
    let rotation = Transform::from_translation(chase_position)
        .looking_at(position + heading * ahead, up)
        .rotation;
    chase.rotation = if snap {
        rotation
    } else {
        chase
            .rotation
            .slerp(rotation, 1.0 - (-8.0 * time.delta_secs()).exp())
    };
    let chase_pose = Transform {
        translation: chase_position,
        rotation: chase.rotation,
        ..default()
    };
    let view_direction = state.planet_camera.view_direction(position);
    let surface_radius = surface
        .as_deref()
        .map_or(terra_geometry::sphere::PLANET_RADIUS, |terrain| {
            terrain.surface_radius(terra_geometry::sphere::SpherePos::new(view_direction))
        });
    let mut planet_pose = state.planet_camera.update(
        current_camera,
        chase_pose,
        position,
        heading,
        time.delta_secs(),
        surface_radius,
    );
    if state.planet_camera.is_active() {
        let displacement = planet_pose.translation - current_camera.translation;
        let distance = displacement.length();
        let mut nearest_hit = None;
        if distance > 1e-4
            && let Ok(direction) = Dir3::new(displacement)
        {
            nearest_hit = spatial
                .cast_shape(
                    &Collider::sphere(radius),
                    current_camera.translation,
                    current_camera.rotation,
                    direction,
                    &ShapeCastConfig::from_max_distance(distance),
                    &SpatialQueryFilter::from_excluded_entities([target, explorer]),
                )
                .map(|hit| hit.distance);
            if let Some(hit) = collision_world.as_deref().and_then(|world| {
                world.camera_sweep_hit(current_camera.translation, planet_pose.translation, radius)
            }) {
                nearest_hit = Some(nearest_hit.map_or(hit, |nearest: f32| nearest.min(hit)));
            }
        }
        if let Some(hit_distance) = nearest_hit.filter(|hit| *hit <= distance) {
            let permitted = (hit_distance - 0.05).max(0.0);
            let fraction = (permitted / distance).clamp(0.0, 1.0);
            planet_pose.translation =
                current_camera.translation + displacement.normalize_or_zero() * permitted;
            planet_pose.rotation = current_camera
                .rotation
                .slerp(planet_pose.rotation, fraction);
            state.planet_camera.hold_transition_step(time.delta_secs());
        } else {
            state.planet_camera.finish_transition_if_ready();
        }
    }
    state
        .planet_camera
        .record_attained_radius(planet_pose.translation.length());
    *camera = planet_pose;
    if let Some(Projection::Perspective(perspective)) = projection.as_deref_mut() {
        if state.planet_camera.is_active() {
            original_far_plane.get_or_insert(perspective.far);
            perspective.far = shared::planet_atmosphere::PLANET_VIEW_FAR_CLIP_DISTANCE;
        } else if let Some(far) = original_far_plane.take() {
            perspective.far = far;
        }
    }
    let alpha = ((chase.distance - 0.25) / 1.25).clamp(0.0, 1.0);
    for (e, mut handle, fade, owner) in &mut visuals {
        let controlled = owner.0 == target;
        let highlighted = state.occupied.is_none() && Some(owner.0) == state.target;
        if (controlled && alpha < 0.999) || highlighted {
            let opacity = if controlled { alpha } else { 1.0 };
            let glow = if highlighted {
                LinearRgba::rgb(0.35, 0.22, 0.06)
            } else {
                LinearRgba::BLACK
            };
            if let Some(fade) = fade {
                if let Some(mut m) = materials.get_mut(&fade.unique) {
                    m.base_color.set_alpha(opacity);
                    m.emissive = glow;
                    m.alpha_mode = if opacity < 0.999 {
                        AlphaMode::Blend
                    } else {
                        AlphaMode::Opaque
                    };
                }
            } else if let Some(original) = materials.get(&handle.0).cloned() {
                let mut material = original;
                material.base_color.set_alpha(opacity);
                material.emissive = glow;
                material.alpha_mode = if opacity < 0.999 {
                    AlphaMode::Blend
                } else {
                    AlphaMode::Opaque
                };
                let unique = materials.add(material);
                commands.entity(e).insert(FadeMaterial {
                    original: handle.0.clone(),
                    unique: unique.clone(),
                });
                handle.0 = unique;
            }
        } else if let Some(fade) = fade {
            handle.0 = fade.original.clone();
            commands.entity(e).remove::<FadeMaterial>();
        }
    }
}
pub(super) fn readout(
    state: Res<Exploration>,
    player: Query<(&Position, &LinearVelocity), With<Player>>,
    vehicles: Query<(&Vehicle, &LinearVelocity)>,
    mut texts: Query<
        (&mut Text, &crate::ui::SidebarReadoutSlot),
        With<crate::ui::SidebarExplorationReadout>,
    >,
) {
    let (
        movement,
        view,
        follow,
        vehicle_action,
        summon_vehicle_action,
        teleport_action,
        recovery_context,
        recovery_action,
    ) = if state.selector {
        (
            "Travel mode: Vehicle selection · simulation paused".to_owned(),
            "Planet view: unavailable during vehicle selection".to_owned(),
            "Follow body · F · unavailable during vehicle selection".to_owned(),
            "Vehicle selection is open · V".to_owned(),
            "Vehicle selection is open".to_owned(),
            "Teleport to selected destination · T".to_owned(),
            "Choose Car (C) or Plane (P). Click Cancel or press Esc / V. Gameplay simulation is paused."
                .to_owned(),
            "Recover · Hold 1s (R)".to_owned(),
        )
    } else {
        let movement = state
            .occupied
            .and_then(|entity| vehicles.get(entity).ok())
            .map(|(vehicle, velocity)| {
                let altitude = player.single().map_or(0.0, |(position, _)| {
                    position.0.length() - terra_geometry::sphere::PLANET_RADIUS
                });
                let condition = if vehicle.crashed {
                    " · Crashed"
                } else if vehicle.flight.stalled {
                    " · Stall — lower nose + Shift"
                } else {
                    ""
                };
                let controls = if vehicle.kind == Kind::Car {
                    "W/S drive · A/D steer · E exit"
                } else {
                    "W/S pitch · A/D roll · Shift thrust · Ctrl airbrake · E exit"
                };
                format!(
                    "Travel mode: {}{condition}\nSpeed: {:.1} m/s\nAltitude: {altitude:.0} m · Clearance: {:.1} m\n{controls}",
                    vehicle.kind.name(),
                    velocity.length(),
                    vehicle.clearance,
                )
            })
            .or_else(|| {
                player.single().ok().map(|(position, velocity)| {
                    let up = position.0.normalize_or(Vec3::Y);
                    let speed = (velocity.0 - up * velocity.0.dot(up)).length();
                    format!(
                        "Travel mode: On foot\nSpeed: {speed:.1} m/s\nWASD move · Shift sprint · Space jump{}",
                        if state.target.is_some() {
                            " · E enter nearby vehicle"
                        } else {
                            ""
                        }
                    )
                })
            })
            .unwrap_or_else(|| "Travel mode: —".to_owned());
        let view = if state.planet_camera.is_requested_open() {
            "Close Planet view · M".to_owned()
        } else {
            "Open Planet view · M".to_owned()
        };
        let follow = format!(
            "Follow body · F · {}",
            if state.planet_view_follows_body() {
                "on"
            } else {
                "off"
            }
        );
        let vehicle_action = if state.is_in_vehicle() {
            "Exit vehicle · E".to_owned()
        } else {
            "Enter vehicle · E".to_owned()
        };
        let summon_vehicle_action = if state.is_in_vehicle() {
            "Summon vehicle · V · on foot only".to_owned()
        } else if state.planet_camera.is_active() {
            "Summon vehicle · V · unavailable in Planet view".to_owned()
        } else {
            "Summon vehicle · V".to_owned()
        };
        let teleport_action = "Teleport to selected destination · T".to_owned();
        let mut recovery_context = format!(
            "Recovery hold: {:.0}% · hold Recover or R for 1 second.",
            state.recovery.min(1.0) * 100.0,
        );
        if !state.message.is_empty() {
            recovery_context.push('\n');
            recovery_context.push_str(&state.message);
        }
        (
            movement,
            view,
            follow,
            vehicle_action,
            summon_vehicle_action,
            teleport_action,
            recovery_context,
            "Recover · Hold 1s (R)".to_owned(),
        )
    };

    for (mut text, slot) in &mut texts {
        let value = match slot {
            crate::ui::SidebarReadoutSlot::Movement => &movement,
            crate::ui::SidebarReadoutSlot::View => &view,
            crate::ui::SidebarReadoutSlot::Follow => &follow,
            crate::ui::SidebarReadoutSlot::VehicleAction => &vehicle_action,
            crate::ui::SidebarReadoutSlot::SummonVehicleAction => &summon_vehicle_action,
            crate::ui::SidebarReadoutSlot::TeleportAction => &teleport_action,
            crate::ui::SidebarReadoutSlot::RecoveryContext => &recovery_context,
            crate::ui::SidebarReadoutSlot::RecoveryAction => &recovery_action,
            crate::ui::SidebarReadoutSlot::SelectorChoice => continue,
            _ => continue,
        };
        if text.0 != *value {
            *text = Text::new(value.clone());
        }
    }
}
pub(super) fn animate(
    time: Res<Time>,
    state: Res<Exploration>,
    keys: Res<ButtonInput<KeyCode>>,
    #[cfg(feature = "asset-review")] pilot: Option<Res<super::showcase::PilotControls>>,
    vehicles: Query<(&Vehicle, &LinearVelocity)>,
    mut parts: Query<(&mut Transform, &mut MovingPart)>,
) {
    if state.selector {
        return;
    }
    for (mut transform, mut part) in &mut parts {
        let Ok((v, velocity)) = vehicles.get(part.owner) else {
            continue;
        };
        if part.propeller {
            if !v.crashed {
                part.angle +=
                    time.delta_secs() * (v.flight.throttle * 35.0 + velocity.length() * 0.2);
            }
            transform.rotation = part.rest * Quat::from_rotation_z(part.angle);
        } else {
            // Forward is local -Z; rolling about +X would move the contact forward.
            part.angle -= time.delta_secs() * velocity.dot(v.flight.heading) / 0.3;
            let desired =
                if part.front && state.occupied == Some(part.owner) && !state.suppress_input {
                    (f32::from(keys.pressed(KeyCode::KeyA))
                        - f32::from(keys.pressed(KeyCode::KeyD)))
                        * 0.4
                } else {
                    0.0
                };
            #[cfg(feature = "asset-review")]
            let desired = pilot
                .as_ref()
                .filter(|p| p.entity == Some(part.owner) && part.front)
                .map_or(desired, |p| p.steering * 0.4);
            part.steering += (desired - part.steering) * (1.0 - (-10.0 * time.delta_secs()).exp());
            transform.rotation = part.rest
                * Quat::from_rotation_y(part.steering)
                * Quat::from_rotation_x(part.angle);
        }
        part.angle = part.angle.rem_euclid(std::f32::consts::TAU);
    }
}
