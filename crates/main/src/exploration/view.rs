use super::*;
#[derive(Component)]
pub(super) struct VehicleVisual;
#[derive(Component)]
pub(super) struct Readout;
#[derive(Default)]
pub(super) struct Chase {
    target: Option<Entity>,
    offset: Vec3,
    distance: f32,
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
    commands.spawn((
        Text::new(""),
        TextFont {
            font: font.0.clone().into(),
            font_size: 16.0.into(),
            ..default()
        },
        TextColor(shared::theme::INK),
        BackgroundColor(shared::theme::PANEL_BG),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(20.0),
            bottom: Val::Px(20.0),
            max_width: Val::Percent(66.0),
            padding: UiRect::all(Val::Px(10.0)),
            ..default()
        },
        Readout,
    ));
}
#[allow(clippy::too_many_arguments)]
pub(super) fn camera(
    mut commands: Commands,
    time: Res<Time>,
    mut state: ResMut<Exploration>,
    spatial: SpatialQuery,
    player: Query<(Entity, &Position, &Player), Without<MainCamera>>,
    vehicles: Query<(&Position, &Vehicle), Without<MainCamera>>,
    mut cameras: Query<&mut Transform, With<MainCamera>>,
    mut visuals: Query<(
        Entity,
        &mut MeshMaterial3d<StandardMaterial>,
        Option<&FadeMaterial>,
        &VisualOwner,
    )>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut chase: Local<Chase>,
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
    let Ok(mut camera) = cameras.single_mut() else {
        return;
    };
    let up = position.normalize();
    let desired = up * height - heading * back;
    let snap = state.snap_camera || chase.target.is_none();
    if snap {
        chase.offset = desired;
        chase.distance = desired.length();
        state.snap_camera = false;
    } else {
        if chase.target != Some(target) {
            chase.offset = camera.translation - position;
            chase.distance = chase.offset.length();
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
    camera.translation = position + direction * chase.distance;
    let rotation = Transform::from_translation(camera.translation)
        .looking_at(position + heading * ahead, up)
        .rotation;
    camera.rotation = if snap {
        rotation
    } else {
        camera
            .rotation
            .slerp(rotation, 1.0 - (-8.0 * time.delta_secs()).exp())
    };
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
    player: Query<&Position, With<Player>>,
    vehicles: Query<(&Vehicle, &LinearVelocity)>,
    mut texts: Query<&mut Text, With<Readout>>,
) {
    let Ok(mut text) = texts.single_mut() else {
        return;
    };
    if state.selector {
        text.0 = "SUMMON VEHICLE — simulation paused\nC  Car    P  Plane    Esc / V  Cancel".into();
        return;
    }
    let status = state
        .occupied
        .and_then(|e| vehicles.get(e).ok())
        .map(|(v, vel)| {
            format!(
                "{} {}  {:.1} m/s  Altitude ASL {:.1} m  Clearance {:.1} m\n{}",
                v.kind.name(),
                if v.crashed {
                    "CRASHED"
                } else if v.flight.stalled {
                    "STALL — lower nose + Shift"
                } else {
                    ""
                },
                vel.length(),
                player
                    .single()
                    .map_or(0.0, |p| p.length() - shared::sphere::PLANET_RADIUS),
                v.clearance,
                if v.kind == Kind::Car {
                    "W/S drive/brake/reverse · A/D steer · E exit"
                } else {
                    "W/S pitch · A/D roll · Shift thrust · Ctrl airbrake · Space brake · E exit"
                }
            )
        })
        .unwrap_or_else(|| {
            format!(
                "On foot · WASD move · Shift sprint · Space jump · V summon{}",
                if state.target.is_some() {
                    " · [E Enter nearby vehicle]"
                } else {
                    ""
                }
            )
        });
    text.0 = format!(
        "{status}\nHold R to recover {:.0}% · M map\n{}",
        state.recovery.min(1.0) * 100.0,
        state.message
    );
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
