//! Vehicle regressions derived from the #29 symptom reproductions.
use super::*;
use bevy::ecs::system::RunSystemOnce;

#[test]
fn wheel_contact_moves_against_forward_and_reverse_travel() {
    let mut failures = 0;
    for (speed, steering) in [3.0, -3.0]
        .into_iter()
        .flat_map(|speed| [None, Some(KeyCode::KeyA), Some(KeyCode::KeyD)].map(|key| (speed, key)))
    {
        let mut app = App::new();
        let mut time = Time::<()>::default();
        time.advance_by(std::time::Duration::from_secs_f32(1.0 / 60.0));
        app.insert_resource(time)
            .init_resource::<Exploration>()
            .init_resource::<ButtonInput<KeyCode>>()
            .add_systems(Update, (view::tag_visuals, view::animate).chain());
        let car = app
            .world_mut()
            .spawn((
                Vehicle::new(Kind::Car, Vec3::NEG_Z),
                LinearVelocity(Vec3::NEG_Z * speed),
            ))
            .id();
        app.world_mut().resource_mut::<Exploration>().occupied = Some(car);
        if let Some(key) = steering {
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(key);
        }
        // Exported vehicle.car.glb has identity rest rotations at all four joints.
        let wheels = ["front.left", "front.right", "rear.left", "rear.right"].map(|name| {
            app.world_mut()
                .spawn((
                    Name::new(format!("vehicle.wheel.{name}")),
                    Transform::default(),
                    ChildOf(car),
                ))
                .id()
        });
        app.update();
        for wheel in wheels {
            let rotation = app.world().get::<Transform>(wheel).unwrap().rotation;
            let bottom_motion = rotation * Vec3::NEG_Y - Vec3::NEG_Y;
            let against_travel = bottom_motion.dot(Vec3::NEG_Z * speed.signum());
            println!(
                "speed={speed} wheel={:?} rotation={rotation:?} bottom_along_travel={against_travel:.6}",
                app.world().get::<Name>(wheel).unwrap()
            );
            failures += usize::from(against_travel >= 0.0);
        }
    }
    assert_eq!(failures, 0, "wheel contact must move against travel");
}

#[test]
fn baked_bridge_entry_deck_and_exit_in_both_directions() {
    let level = terra_world::level::LevelData::from_artifact_bytes(include_bytes!(
        "../../assets/level_1337.bin"
    ))
    .unwrap();
    let ground = PlanetMesh::new(
        level
            .terrain_tris
            .iter()
            .map(|tri| tri.map(Vec3::from_array))
            .collect(),
    );
    let mut failures = Vec::new();
    let bridges: Vec<_> = level
        .roads
        .iter()
        .filter(|r| r.kind == terra_world::level::RoadKind::Bridge)
        .collect();
    assert!(!bridges.is_empty(), "seed 1337 must contain bridges");
    for road in bridges {
        let points: Vec<_> = road
            .points
            .iter()
            .map(|p| terra_geometry::sphere::SpherePos::new(Vec3::from_array(*p)))
            .collect();
        let deck = terra_geometry::roads::build_bridge_deck_geometry(&points, &ground, 4.0);
        let normals: Vec<_> = deck
            .top_surface
            .iter()
            .map(|tri| {
                let [a, b, c] = tri.map(Vec3::from_array);
                (b - a).cross(c - a).normalize().dot(a.normalize())
            })
            .collect();
        println!(
            "deck top normal radial dot: min={} max={}",
            normals.iter().copied().fold(f32::INFINITY, f32::min),
            normals.iter().copied().fold(f32::NEG_INFINITY, f32::max)
        );
        let deck_radius = |direction| {
            deck.top_surface
                .iter()
                .filter_map(|tri| {
                    terra_geometry::planet::ray_triangle_radius(
                        direction,
                        &tri.map(Vec3::from_array),
                    )
                })
                .max_by(f32::total_cmp)
        };
        for reverse in [false, true] {
            let (from, to) = if reverse {
                (points.last().unwrap().0, points[0].0)
            } else {
                (points[0].0, points.last().unwrap().0)
            };
            let heading = tangent(to - from, from);
            let length = from.angle_between(to) * terra_geometry::sphere::PLANET_RADIUS;
            for (reverse_gear, initial_speed) in
                [(false, 0.0), (false, 8.0), (true, 0.0), (true, 3.0)]
            {
                for (phase, distance) in [
                    ("entry", -8.0),
                    ("deck", length * 0.5),
                    ("exit", length - 8.0),
                ] {
                    let angle = distance / terra_geometry::sphere::PLANET_RADIUS;
                    let start = from * angle.cos() + heading * angle.sin();
                    let top = deck_radius(start).unwrap_or(0.0);
                    let position = start * (ground.facet_radius(start, 2000.0).max(top) + 0.48);
                    let (mut app, explorer) = tests::fixture();
                    let old_ground = app
                        .world_mut()
                        .query_filtered::<Entity, With<Ground>>()
                        .single(app.world())
                        .unwrap();
                    app.world_mut().despawn(old_ground);
                    app.world_mut()
                        .entity_mut(explorer)
                        .insert((ColliderDisabled, RigidBody::Kinematic));
                    let center = (from + to).normalize() * 2000.0;
                    let patch: Vec<_> = level
                        .terrain_tris
                        .iter()
                        .filter(|tri| {
                            tri.iter()
                                .any(|p| Vec3::from_array(*p).distance(center) < length + 50.0)
                        })
                        .copied()
                        .collect();
                    app.world_mut().spawn((
                        RigidBody::Static,
                        crate::map::build_collider(&patch),
                        CollisionMargin(0.02),
                        Transform::default(),
                        Ground,
                    ));
                    let bridge = app
                        .world_mut()
                        .spawn((
                            RigidBody::Static,
                            crate::map::build_collider(&deck.triangles),
                            Transform::default(),
                            Ground,
                        ))
                        .id();
                    let mut vehicle = Vehicle::new(
                        Kind::Car,
                        tangent(heading, start) * if reverse_gear { -1.0 } else { 1.0 },
                    );
                    vehicle.parked = false;
                    let rotation = facing(vehicle.flight.heading, start);
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
                    app.world_mut()
                        .entity_mut(car)
                        .insert(LinearVelocity(tangent(heading, start) * initial_speed));
                    app.world_mut().resource_mut::<Exploration>().occupied = Some(car);
                    app.world_mut()
                        .resource_mut::<ButtonInput<KeyCode>>()
                        .press(if reverse_gear {
                            KeyCode::KeyS
                        } else {
                            KeyCode::KeyW
                        });
                    let mut entered = false;
                    let mut middle = false;
                    let mut exited = false;
                    let mut embedded = false;
                    // Reverse travel across the longest baked span needs over 4000 updates.
                    for frame in 0..5000 {
                        app.update();
                        let p = app.world().get::<Position>(car).unwrap().0;
                        let dir = p.normalize();
                        let progress = dir.dot(heading).atan2(dir.dot(from)) * 2000.0;
                        entered |= progress >= 2.0;
                        middle |= progress >= length * 0.5;
                        exited |= progress >= length + 5.0;
                        let top = deck_radius(dir);
                        let first_embedding =
                            !embedded && top.is_some_and(|r| p.length() < r - 0.1);
                        embedded |= first_embedding;
                        if first_embedding {
                            let h = app.world().get::<Vehicle>(car).unwrap().flight.heading;
                            let probes = app
                                .world_mut()
                                .run_system_once(move |spatial: SpatialQuery| {
                                    shared::car_prototype::support_origins(p, h).map(|origin| {
                                        spatial
                                            .cast_ray(
                                                origin,
                                                Dir3::new(-p.normalize()).unwrap(),
                                                0.75,
                                                false,
                                                &SpatialQueryFilter::from_excluded_entities([car]),
                                            )
                                            .map(|hit| {
                                                (
                                                    hit.entity == bridge,
                                                    hit.distance,
                                                    hit.normal.dot(p.normalize()),
                                                )
                                            })
                                    })
                                })
                                .unwrap();
                            println!(
                                "first embedded support rays (deck, distance, radial normal): {probes:?}"
                            );
                        }
                        if frame % 120 == 0 || exited || first_embedding {
                            let v = app.world().get::<Vehicle>(car).unwrap();
                            let velocity = app.world().get::<LinearVelocity>(car).unwrap().0;
                            let contacts = app.world().get::<CollidingEntities>(car).unwrap();
                            let rotation = app.world().get::<Rotation>(car).unwrap().0;
                            println!(
                                "bridge={:?} phase={phase} reverse={reverse} reverse_gear={reverse_gear} initial_speed={initial_speed} frame={frame} length={length:.2} progress={progress:.3} position={p:?} velocity={velocity:?} rotation={rotation:?} speed={:.3} top_clearance={:?} support={:?} deck_contact={} embedded={embedded}",
                                road.name,
                                velocity.dot(v.flight.heading),
                                top.map(|r| p.length() - r),
                                v.support_normal,
                                contacts.0.contains(&bridge)
                            );
                        }
                        if exited {
                            break;
                        }
                    }
                    println!(
                        "RESULT bridge={:?} phase={phase} reverse={reverse} reverse_gear={reverse_gear} initial_speed={initial_speed} entry={entered} deck={middle} exit={exited} embedded={embedded}",
                        road.name
                    );
                    if !exited || embedded {
                        failures.push((
                            road.name.clone(),
                            phase,
                            reverse,
                            reverse_gear,
                            initial_speed,
                        ));
                    }
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "bridge traversal failures: {failures:?}"
    );
}
