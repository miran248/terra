//! Collision residency follows physics bodies and pending destinations, never render LOD.
use super::*;
use shared::art::AssetName;
#[derive(Clone, Copy)]
struct Bounds {
    min: Vec3,
    max: Vec3,
}
impl Bounds {
    fn from_collider(collider: &Collider, position: Vec3, rotation: Quat) -> Self {
        let bounds = collider.aabb(position, rotation);
        Self {
            min: bounds.min,
            max: bounds.max,
        }
    }

    fn intersects_swept_sphere(self, start: Vec3, end: Vec3, radius: f32) -> bool {
        let direction = end - start;
        let padding = Vec3::splat(radius.max(0.0));
        let min = self.min - padding;
        let max = self.max + padding;
        let mut enter = 0.0_f32;
        let mut exit = 1.0_f32;
        for axis in 0..3 {
            let origin = start[axis];
            let delta = direction[axis];
            if delta.abs() <= f32::EPSILON {
                if origin < min[axis] || origin > max[axis] {
                    return false;
                }
                continue;
            }
            let inverse = delta.recip();
            let first = (min[axis] - origin) * inverse;
            let second = (max[axis] - origin) * inverse;
            enter = enter.max(first.min(second));
            exit = exit.min(first.max(second));
            if enter > exit {
                return false;
            }
        }
        true
    }
}

struct Obstacle {
    position: Vec3,
    rotation: Quat,
    collider: Collider,
    bounds: Bounds,
    resident: Option<Entity>,
}
impl Obstacle {
    fn new(position: Vec3, rotation: Quat, collider: Collider) -> Self {
        let bounds = Bounds::from_collider(&collider, position, rotation);
        Self {
            position,
            rotation,
            collider,
            bounds,
            resident: None,
        }
    }

    fn camera_sweep_hit(&self, start: Vec3, end: Vec3, radius: f32) -> Option<f32> {
        let displacement = end - start;
        let max_distance = displacement.length();
        if max_distance <= f32::EPSILON {
            let separation =
                self.collider
                    .distance_to_point(self.position, self.rotation, start, true);
            return (separation <= radius).then_some(0.0);
        }

        let direction = displacement / max_distance;
        let mut traveled = 0.0;
        for _ in 0..64 {
            let point = start + direction * traveled;
            let separation =
                self.collider
                    .distance_to_point(self.position, self.rotation, point, true);
            let advance = separation - radius;
            if advance <= 0.001 {
                return Some(traveled);
            }
            traveled += advance;
            if traveled > max_distance {
                return None;
            }
        }

        // Keep the camera on the safe side if an unusually complex compound
        // shape has not converged within the conservative-advance budget.
        Some(traveled.min(max_distance))
    }
}

#[derive(Resource, Default)]
pub(super) struct CollisionWorld {
    obstacles: Vec<Obstacle>,
    pub ready: bool,
    pub last_action: Option<Action>,
}
impl CollisionWorld {
    /// Sweep the camera against baked structure/scenery geometry without
    /// changing which obstacles own physics entities.
    pub(super) fn camera_sweep_hit(&self, start: Vec3, end: Vec3, radius: f32) -> Option<f32> {
        self.obstacles
            .iter()
            .filter(|obstacle| obstacle.bounds.intersects_swept_sphere(start, end, radius))
            .filter_map(|obstacle| obstacle.camera_sweep_hit(start, end, radius))
            .min_by(f32::total_cmp)
    }
}

pub(super) fn setup(mut commands: Commands) {
    let level = shared::level::LevelData::from_artifact_bytes(include_bytes!(
        "../../assets/level_1337.bin"
    ))
    .expect("embedded level");
    let triangles = level
        .terrain_tris
        .iter()
        .enumerate()
        .filter_map(|(face, triangle)| {
            if level.water_phase[face] == Some(shared::level::WaterPhase::Frozen) {
                return None;
            }
            let radii = level.face_river_r[face]
                .map(|r| if r > 0.0 { r } else { level.face_water_r[face] });
            radii.iter().all(|r| *r > 0.0).then(|| {
                std::array::from_fn(|i| Vec3::from_array(triangle[i]).normalize() * radii[i])
            })
        })
        .collect();
    commands.insert_resource(Liquid(PlanetMesh::new(triangles)));
    let mut obstacles = Vec::new();
    for s in &level.structures {
        let position = Vec3::from_array(s.pos);
        if let Some(collider) =
            crate::asset_collision::candidate_collider(s.kind.asset_name(), Vec3::ONE)
        {
            let rotation = Quat::from_rotation_arc(Vec3::Y, position.normalize())
                * Quat::from_rotation_y(s.yaw);
            obstacles.push(Obstacle::new(position, rotation, collider));
        }
    }
    for s in &level.scenery {
        let position = Vec3::from_array(s.pos);
        let hash = s.pos[0].to_bits()
            ^ s.pos[1].to_bits().rotate_left(13)
            ^ s.pos[2].to_bits().rotate_left(27);
        let scale = 0.7 + (hash & 0xff) as f32 / 255.0 * 0.6;
        let yaw = (hash >> 8 & 0xff) as f32 / 255.0 * std::f32::consts::TAU;
        if let Some(collider) = crate::asset_collision::candidate_collider(
            &shared::art::scenery_variant_name(s.kind, s.variant as u32),
            Vec3::splat(scale),
        ) {
            let rotation =
                Quat::from_rotation_arc(Vec3::Y, position.normalize()) * Quat::from_rotation_y(yaw);
            obstacles.push(Obstacle::new(position, rotation, collider));
        }
    }
    commands.insert_resource(CollisionWorld {
        obstacles,
        ready: false,
        last_action: None,
    });
}
pub(super) fn residency(
    mut commands: Commands,
    mut world: Option<ResMut<CollisionWorld>>,
    state: Res<Exploration>,
    world_epoch: Option<Res<crate::map::WorldEpoch>>,
    bodies: Query<(&Position, &LinearVelocity), ExplorationBody>,
) {
    let Some(world) = world.as_mut() else {
        return;
    };
    let request = state.actions.front().and_then(|action| match action {
        Action::Teleport(p) => Some(*p),
        Action::PlanetTeleport(request_id) => state
            .pending_planet_teleport
            .as_ref()
            .filter(|request| {
                request.id == *request_id
                    && world_epoch.is_some_and(|epoch| *epoch == request.world_epoch)
            })
            .map(|request| request.destination.position),
        Action::Recover => state.safe.or(state.start),
        _ => None,
    });
    let mut centers: Vec<(Vec3, f32)> = bodies
        .iter()
        .map(|(p, v)| (p.0, 200.0 + v.length() * 2.0))
        .collect();
    if let Some(p) = request {
        centers.push((p, 200.0));
    }
    if matches!(state.actions.front(), Some(Action::Recover))
        && let Some(p) = state.start
    {
        centers.push((p, 200.0));
    }
    let mut changed = false;
    for obstacle in &mut world.obstacles {
        // Extra margin includes large structures whose center is outside the corridor.
        let near = centers
            .iter()
            .any(|(p, r)| obstacle.position.distance_squared(*p) < (r + 50.0).powi(2));
        if near && obstacle.resident.is_none() {
            obstacle.resident = Some(
                commands
                    .spawn((
                        RigidBody::Static,
                        obstacle.collider.clone(),
                        Transform::from_translation(obstacle.position)
                            .with_rotation(obstacle.rotation),
                        Ground,
                        crate::chunks::WorldObstacle,
                        Friction::ZERO,
                        Restitution::ZERO,
                    ))
                    .id(),
            );
            changed = true;
        } else if !near && let Some(e) = obstacle.resident.take() {
            commands.entity(e).despawn();
        }
    }
    world.ready = !changed && world.last_action == state.actions.front().copied();
    world.last_action = state.actions.front().copied();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunks::WorldObstacle;
    use crate::exploration::{Action, tests::fixture};
    use crate::map::MainCamera;

    #[test]
    fn detached_planet_view_and_detail_changes_keep_moving_body_support_resident() {
        let (mut app, explorer) = fixture();
        app.world_mut().insert_resource(Gravity(Vec3::NEG_Y * 9.81));
        let start = app.world().get::<Position>(explorer).unwrap().0;
        let floor = {
            let mut query = app
                .world_mut()
                .query_filtered::<Entity, (With<Ground>, With<Collider>, Without<WorldObstacle>)>();
            query.single(app.world()).unwrap()
        };
        let regional = start + Vec3::X * 270.0;
        let pending_target = start + Vec3::X * 600.0;
        let obstacle =
            |position| Obstacle::new(position, Quat::IDENTITY, Collider::cuboid(2.0, 2.0, 2.0));
        app.world_mut().insert_resource(CollisionWorld {
            obstacles: vec![
                obstacle(start - Vec3::X * 100.0),
                obstacle(regional),
                obstacle(pending_target + Vec3::X * 100.0),
            ],
            ready: false,
            last_action: None,
        });
        let camera = app
            .world_mut()
            .spawn((
                MainCamera,
                Transform::from_translation(start + Vec3::Y * 5.0 - Vec3::Z * 5.0)
                    .looking_at(start, Vec3::Y),
            ))
            .id();

        app.update();
        let start_obstacle = app.world().resource::<CollisionWorld>().obstacles[0]
            .resident
            .expect("body-centered obstacle should be resident");

        app.world_mut()
            .resource_mut::<Exploration>()
            .set_planet_view_open(true);
        app.update();
        app.world_mut()
            .resource_mut::<Exploration>()
            .planet_camera
            .detach(Vec3::X);
        for _ in 0..120 {
            app.update();
        }

        let camera_pose = *app.world().get::<Transform>(camera).unwrap();
        let altitude = crate::map::altitude_above_surface(camera_pose.translation, None);
        assert!(camera_pose.translation.normalize().dot(Vec3::X) > 0.999);
        assert_eq!(shared::planet_detail::desired_chunk_lod(1_800.0, 0.0, 1), 1);
        assert_eq!(
            shared::planet_detail::desired_chunk_lod(1_800.0, altitude, 1),
            2
        );
        assert!(app.world().get::<Collider>(start_obstacle).is_some());

        let supported_y = app.world().get::<Position>(explorer).unwrap().0.y;
        for step in 1..=5 {
            let mut travel = start + Vec3::X * (step as f32 * 50.0);
            travel.y = supported_y;
            app.world_mut()
                .entity_mut(explorer)
                .insert(Position(travel));
            app.update();
        }
        for _ in 0..2 {
            app.update();
        }
        let regional_resident = {
            let collision = app.world().resource::<CollisionWorld>();
            assert!(collision.obstacles[0].resident.is_none());
            collision.obstacles[1]
                .resident
                .expect("regional body obstacle should follow the moving body")
        };
        assert!(app.world().get::<Collider>(floor).is_some());
        let contacts = app.world().get::<CollidingEntities>(explorer).unwrap();
        assert!(
            contacts.0.contains(&floor),
            "the moving body lost terrain support: position={:?}, contacts={:?}, floor={floor:?}",
            app.world().get::<Position>(explorer),
            contacts.0
        );

        app.world_mut()
            .resource_mut::<Exploration>()
            .planet_camera
            .request_radius(shared::planet_view::PLANET_VIEW_NEAR_RADIUS);
        for _ in 0..120 {
            app.update();
        }
        let camera_altitude = crate::map::altitude_above_surface(
            app.world().get::<Transform>(camera).unwrap().translation,
            None,
        );
        assert!(camera_altitude < 100.0);
        assert_eq!(
            shared::planet_detail::desired_chunk_lod(1_800.0, camera_altitude, 1),
            1
        );
        assert!(app.world().get::<Collider>(floor).is_some());
        assert!(app.world().get::<Collider>(regional_resident).is_some());

        let target_action = Action::Teleport(pending_target);
        {
            let mut state = app.world_mut().resource_mut::<Exploration>();
            state.selector = true;
            state.request(target_action);
        }
        app.update();
        assert!(
            !app.world().resource::<CollisionWorld>().ready,
            "pending destination must wait while its obstacle colliders load"
        );
        assert!(
            app.world().resource::<CollisionWorld>().obstacles[2]
                .resident
                .is_some()
        );
        app.update();
        let collision = app.world().resource::<CollisionWorld>();
        assert!(collision.ready);
        assert!(collision.last_action == Some(target_action));
        assert!(app.world().resource::<Exploration>().actions.is_empty());
        assert!(
            app.world()
                .get::<Collider>(collision.obstacles[2].resident.unwrap())
                .is_some()
        );
        assert!(app.world().get::<Collider>(floor).is_some());
    }

    #[test]
    fn distant_nonresident_structure_clears_camera_zoom_without_losing_body_support() {
        let (mut app, explorer) = fixture();
        app.world_mut().insert_resource(Gravity(Vec3::NEG_Y * 9.81));
        let floor = {
            let mut query = app
                .world_mut()
                .query_filtered::<Entity, (With<Ground>, With<Collider>, Without<WorldObstacle>)>();
            query.single(app.world()).unwrap()
        };
        for _ in 0..30 {
            app.update();
        }
        let supported_position = app.world().get::<Position>(explorer).unwrap().0;
        assert!(
            app.world()
                .get::<CollidingEntities>(explorer)
                .unwrap()
                .0
                .contains(&floor)
        );

        let view_direction = Vec3::X;
        let structure_position = view_direction * (shared::sphere::PLANET_RADIUS + 30.0);
        let structure_rotation = Quat::from_rotation_arc(Vec3::Y, view_direction);
        app.world_mut().insert_resource(CollisionWorld {
            obstacles: vec![Obstacle::new(
                structure_position,
                structure_rotation,
                Collider::cuboid(10.0, 50.0, 10.0),
            )],
            ready: false,
            last_action: None,
        });
        let camera = app
            .world_mut()
            .spawn((
                MainCamera,
                Transform::from_xyz(0.0, shared::sphere::PLANET_RADIUS + 5.0, -5.0)
                    .looking_at(Vec3::ZERO, Vec3::Y),
            ))
            .id();
        app.update();
        assert!(
            app.world().resource::<CollisionWorld>().obstacles[0]
                .resident
                .is_none()
        );

        app.world_mut()
            .resource_mut::<Exploration>()
            .set_planet_view_open(true);
        for _ in 0..140 {
            app.update();
        }
        app.world_mut()
            .resource_mut::<Exploration>()
            .planet_camera
            .detach(view_direction);
        for _ in 0..180 {
            app.update();
        }
        assert!(
            app.world()
                .get::<Transform>(camera)
                .unwrap()
                .translation
                .normalize()
                .dot(view_direction)
                > 0.999
        );

        app.world_mut()
            .resource_mut::<Exploration>()
            .planet_camera
            .request_radius(shared::planet_view::PLANET_VIEW_NEAR_RADIUS);
        for _ in 0..150 {
            app.update();
        }
        let camera_pose = *app.world().get::<Transform>(camera).unwrap();
        let collision = app.world().resource::<CollisionWorld>();
        let structure = &collision.obstacles[0];
        let clearance = structure.collider.distance_to_point(
            structure.position,
            structure.rotation,
            camera_pose.translation,
            true,
        );
        assert!(
            clearance > 0.24,
            "camera clipped the distant nonresident structure: distance={clearance}, pose={camera_pose:?}"
        );
        assert!(structure.resident.is_none());
        assert!(app.world().get::<Collider>(floor).is_some());
        assert!(
            app.world()
                .get::<CollidingEntities>(explorer)
                .unwrap()
                .0
                .contains(&floor)
        );
        assert!(
            app.world()
                .get::<Position>(explorer)
                .unwrap()
                .0
                .distance(supported_position)
                < 0.1
        );

        app.world_mut()
            .resource_mut::<CollisionWorld>()
            .obstacles
            .clear();
        for _ in 0..120 {
            app.update();
        }
        let released = app.world().get::<Transform>(camera).unwrap();
        assert!(
            (released.translation.length() - shared::planet_view::PLANET_VIEW_NEAR_RADIUS).abs()
                < 3.0,
            "requested zoom did not resume after removing the structure: pose={released:?}"
        );
        assert_eq!(
            app.world()
                .resource::<Exploration>()
                .planet_camera
                .requested_radius(),
            shared::planet_view::PLANET_VIEW_NEAR_RADIUS
        );
        assert!(
            app.world()
                .get::<CollidingEntities>(explorer)
                .unwrap()
                .0
                .contains(&floor)
        );
    }

    #[test]
    fn selecting_a_destination_does_not_prepare_its_collision_obstacles() {
        let home = Vec3::new(0.0, 2001.0, 0.0);
        let destination = Vec3::new(0.0, -2001.0, 0.0);
        let mut state = Exploration::default();
        state.select_planet_destination(PlanetDestination {
            id: PlanetDestinationId::surface(crate::map::WorldEpoch::new(7), destination),
            position: destination,
            surface: PlanetDestinationSurface::Terrain,
            display: "Terrain · 0°S, 0°W".into(),
        });

        let obstacle = |position| Obstacle::new(position, Quat::IDENTITY, Collider::sphere(2.0));
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(state)
            .insert_resource(CollisionWorld {
                obstacles: vec![obstacle(home), obstacle(destination)],
                ready: false,
                last_action: None,
            })
            .add_systems(Update, residency);
        app.world_mut().spawn((
            crate::map::player_physics_bundle(home, Vec3::NEG_Z),
            Position(home),
            LinearVelocity::ZERO,
        ));

        app.update();

        let collision = app.world().resource::<CollisionWorld>();
        assert!(collision.obstacles[0].resident.is_some());
        assert!(collision.obstacles[1].resident.is_none());
        assert!(app.world().resource::<Exploration>().actions.is_empty());
    }
}
