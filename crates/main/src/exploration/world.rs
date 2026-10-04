//! Collision residency follows physics bodies and pending destinations, never render LOD.
use super::*;
use shared::art::AssetName;
struct Obstacle {
    position: Vec3,
    rotation: Quat,
    collider: Collider,
    resident: Option<Entity>,
}
#[derive(Resource, Default)]
pub(super) struct CollisionWorld {
    obstacles: Vec<Obstacle>,
    pub ready: bool,
    pub last_action: Option<Action>,
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
            obstacles.push(Obstacle {
                position,
                rotation: Quat::from_rotation_arc(Vec3::Y, position.normalize())
                    * Quat::from_rotation_y(s.yaw),
                collider,
                resident: None,
            });
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
            obstacles.push(Obstacle {
                position,
                rotation: Quat::from_rotation_arc(Vec3::Y, position.normalize())
                    * Quat::from_rotation_y(yaw),
                collider,
                resident: None,
            });
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

        let obstacle = |position| Obstacle {
            position,
            rotation: Quat::IDENTITY,
            collider: Collider::sphere(2.0),
            resident: None,
        };
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
