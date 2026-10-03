//! THROWAWAY: opt-in planet review for candidate assets, leaving the production catalog intact.
use crate::{
    asset_collision::candidate_collider,
    map::{MainCamera, Player},
};
use avian3d::prelude::{DebugRender, PhysicsDebugPlugin, PhysicsGizmos, RigidBody};
use bevy::{
    gltf::Gltf,
    prelude::*,
    render::view::screenshot::{Screenshot, save_to_disk},
};
use shared::{
    actor_animation::{ActorAnimationPlugin, ActorPlayback},
    planet::PlanetMesh,
    sphere::PLANET_RADIUS,
    state::AppState,
};

pub struct AssetShowcasePlugin;
impl Plugin for AssetShowcasePlugin {
    fn build(&self, app: &mut App) {
        if std::env::var_os("TERRA_ASSET_SHOWCASE").is_none() {
            return;
        }
        app.add_plugins((ActorAnimationPlugin, PhysicsDebugPlugin))
            .insert_gizmo_config(
                PhysicsGizmos {
                    axis_lengths: None,
                    collider_color: None,
                    ..default()
                },
                bevy::gizmos::config::GizmoConfig {
                    enabled: false,
                    ..default()
                },
            )
            .add_systems(
                Update,
                (setup, attach_weapon, toggle_colliders).run_if(in_state(AppState::Playing)),
            )
            .add_systems(PostUpdate, capture.before(TransformSystems::Propagate));
    }
}

#[derive(Resource)]
struct Showcase {
    center: Vec3,
    up: Vec3,
    forward: Vec3,
    elapsed: f32,
    phase: usize,
    knife: Handle<WorldAsset>,
    baseline: bool,
    structures: bool,
    items: bool,
}

fn setup(
    mut commands: Commands,
    server: Res<AssetServer>,
    stage: Option<Res<Showcase>>,
    player: Query<(&Transform, &Player)>,
    catalog: Res<crate::asset_catalog::AssetCatalog>,
    mut window: Single<&mut Window>,
) {
    if stage.is_some() {
        return;
    }
    let Ok((player_transform, player)) = player.single() else {
        return;
    };
    // The game's PlanetMesh resource indexes unit-sphere faces, not surface heights.
    // Review placement must sample the same displaced triangles used by rendering/physics.
    let level =
        shared::level::LevelData::from_artifact_bytes(include_bytes!("../assets/level_1337.bin"))
            .unwrap();
    let planet = PlanetMesh::new(
        level
            .terrain_tris
            .iter()
            .map(|tri| tri.map(Vec3::from_array))
            .collect(),
    );
    let up = player_transform.translation.normalize();
    let forward = player.heading.normalize();
    let right = forward.cross(up);
    let center_direction = (player_transform.translation + forward * 4.).normalize();
    let center = center_direction * planet.facet_radius(center_direction, PLANET_RADIUS);
    let baseline = std::env::var_os("TERRA_ASSET_BASELINE").is_some();
    let items = std::env::var_os("TERRA_ITEM_SHOWCASE").is_some();
    let structures = std::env::var_os("TERRA_STRUCTURE_SHOWCASE").is_some();
    assert!(
        !(baseline && (structures || items)),
        "Family terrain reviews use candidates; use the workbench for baseline comparison"
    );
    window.title = format!(
        "Terra | ASSET REVIEW PROTOTYPE | {}",
        if items {
            "items and actors"
        } else if structures {
            "structures"
        } else if baseline {
            "baseline"
        } else {
            "candidates"
        }
    );
    assert!(
        center.length() > PLANET_RADIUS * 0.9,
        "review scene must be on the terrain, not the unit lookup sphere"
    );
    let mut props = vec![
        ("structure.house", 2., 4., Vec3::new(8., 5., 6.)),
        ("scenery.tree.0", -3., 1., Vec3::ONE),
        ("scenery.rock.0", 2., -1.5, Vec3::ONE),
    ];
    if structures {
        props = vec![
            ("structure.house", -4., 7., Vec3::ONE),
            ("structure.watchtower", 0., 7., Vec3::ONE),
            ("structure.well", 4., 7., Vec3::ONE),
            ("structure.tent", -4., 1., Vec3::ONE),
            ("structure.dock", 4., 1., Vec3::ONE),
            ("structure.dock", 4., 3.4, Vec3::ONE),
            ("structure.suspension", -4., 12., Vec3::ONE),
        ];
        for (name, z) in [
            ("structure.fence", -2.),
            ("structure.wall", 12.),
            ("structure.railing", 15.),
            ("structure.guardrail", 18.),
        ] {
            let step = shared::asset_contract::candidate_contract(name)
                .unwrap()
                .repeat_step
                .unwrap()[0];
            for index in -1..=1 {
                props.push((name, index as f32 * step, z, Vec3::ONE));
            }
        }
    }
    if items {
        props = [
            "material.metal",
            "material.wood",
            "material.rope",
            "material.cloth",
            "weapon.knife",
            "weapon.spear",
            "weapon.pistol",
            "weapon.sling",
            "weapon.rifle",
        ]
        .into_iter()
        .enumerate()
        .map(|(i, name)| (name, (i as f32 - 4.) * 0.6, -1.5, Vec3::ONE))
        .collect();
    }
    for (name, x, z, baseline_scale) in props {
        let direction = (center + right * x + forward * z).normalize();
        let position = direction * planet.facet_radius(direction, PLANET_RADIUS);
        let heading = (-forward).reject_from(direction).normalize();
        let rotation = Quat::from_mat3(&Mat3::from_cols(
            heading.cross(direction),
            direction,
            -heading,
        ));
        let scene = if baseline {
            catalog.scene(name)
        } else {
            server.load(format!("models/candidates/{name}.glb#Scene0"))
        };
        let mut root = commands.spawn((
            Transform::from_translation(position).with_rotation(rotation),
            Visibility::default(),
        ));
        if !baseline && let Some(collider) = candidate_collider(name, Vec3::ONE) {
            root.insert((
                RigidBody::Static,
                collider,
                DebugRender {
                    collider_color: Some(Color::srgb(0.95, 0.72, 0.22)),
                    ..DebugRender::none()
                },
            ));
        }
        root.with_child((
            WorldAssetRoot(scene),
            Transform::from_scale(if baseline { baseline_scale } else { Vec3::ONE }),
        ));
    }
    for index in 0..if items { 9 } else { 3 } {
        let action = index % 3;
        let actor_name = if items {
            ["actor.player", "actor.zombie.0", "actor.zombie.1"][index / 3]
        } else {
            "actor.player"
        };
        let direction = (center
            + right * (action as f32 - 1.) * if items { 1.5 } else { 1.2 }
            + forward * (index / 3) as f32 * 1.8)
            .normalize();
        let position = direction * planet.facet_radius(direction, PLANET_RADIUS);
        let heading = (-forward).reject_from(direction).normalize();
        let facing = Quat::from_mat3(&Mat3::from_cols(
            heading.cross(direction),
            direction,
            -heading,
        ));
        let mut root = commands.spawn((
            Transform::from_translation(position).with_rotation(facing),
            Visibility::default(),
        ));
        if !baseline {
            root.insert((
                RigidBody::Static,
                candidate_collider(actor_name, Vec3::ONE).unwrap(),
                DebugRender {
                    collider_color: Some(Color::srgb(0.95, 0.72, 0.22)),
                    ..DebugRender::none()
                },
            ));
        }
        let weapon_name = if items {
            [
                "weapon.knife",
                "weapon.spear",
                "weapon.pistol",
                "weapon.sling",
                "weapon.rifle",
            ][index % 5]
        } else {
            "weapon.knife"
        };
        root.with_child((
            ReviewWeapon(weapon_name),
            WorldAssetRoot(if baseline {
                catalog.scene("actor.player")
            } else {
                server.load(format!("models/candidates/{actor_name}.glb#Scene0"))
            }),
            Transform::from_scale(if baseline {
                Vec3::splat(0.55)
            } else {
                Vec3::ONE
            }),
            ActorPlayback {
                source: if baseline {
                    server.load(shared::art::ACTORS_CATALOG)
                } else {
                    server.load::<Gltf>(format!("models/candidates/{actor_name}.glb"))
                },
                action,
                paused: false,
            },
        ));
    }
    commands.insert_resource(Showcase {
        center,
        up,
        forward,
        elapsed: 0.,
        phase: 0,
        knife: if baseline {
            catalog.scene("weapon.knife")
        } else {
            server.load("models/candidates/weapon.knife.glb#Scene0")
        },
        baseline,
        structures,
        items,
    });
}

#[derive(Component)]
struct ReviewWeapon(&'static str);

#[derive(Component)]
struct Attached;
fn attach_weapon(
    mut commands: Commands,
    stage: Option<Res<Showcase>>,
    sockets: Query<(Entity, &Name), Without<Attached>>,
    parents: Query<&ChildOf>,
    roots: Query<&ReviewWeapon>,
    server: Res<AssetServer>,
) {
    let Some(stage) = stage else {
        return;
    };
    for (entity, name) in &sockets {
        if name.as_str() != "socket.hand" {
            continue;
        }
        let Some(weapon) = parents
            .iter_ancestors(entity)
            .find_map(|e| roots.get(e).ok())
        else {
            continue;
        };
        commands.entity(entity).insert(Attached).with_child((
            WorldAssetRoot(if stage.baseline {
                stage.knife.clone()
            } else {
                server.load(format!("models/candidates/{}.glb#Scene0", weapon.0))
            }),
            if stage.baseline {
                Transform::from_rotation(Quat::from_rotation_y(std::f32::consts::PI))
                    .with_scale(Vec3::splat(0.7))
            } else {
                shared::asset_contract::grip_transform(weapon.0).unwrap()
            },
        ));
    }
}

fn capture(
    mut commands: Commands,
    time: Res<Time>,
    stage: Option<ResMut<Showcase>>,
    mut camera: Query<&mut Transform, With<MainCamera>>,
    mut exit: MessageWriter<AppExit>,
    diagnostics: Res<bevy::diagnostic::DiagnosticsStore>,
    mut gizmos: ResMut<GizmoConfigStore>,
) {
    if std::env::var_os("TERRA_ASSET_CAPTURE").is_none() {
        return;
    }
    let Some(mut stage) = stage else {
        return;
    };
    stage.elapsed += time.delta_secs();
    if stage.phase > 0
        && let Ok(mut camera) = camera.single_mut()
    {
        let (eye, target) = if stage.items {
            (
                stage.center - stage.forward * 7. + stage.up * 4.,
                stage.center + stage.forward + stage.up * 0.45,
            )
        } else if stage.structures && stage.phase >= 5 {
            let center = stage.center + stage.forward * 14.;
            (
                center + stage.forward.cross(stage.up) * 10. - stage.forward * 4. + stage.up * 6.,
                center + stage.up * 0.5,
            )
        } else if stage.structures {
            (
                stage.center - stage.forward * 17. + stage.up * 10.,
                stage.center + stage.forward * 6. + stage.up,
            )
        } else {
            (
                stage.center - stage.forward * 10. + stage.up * 4.,
                stage.center + stage.forward + stage.up * 1.5,
            )
        };
        *camera = Transform::from_translation(eye).looking_at(target, stage.up);
    }
    if stage.elapsed > 8. && stage.phase == 0 {
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(if stage.items {
                "/tmp/terra-items-gameplay.png"
            } else if stage.structures {
                "/tmp/terra-structures-gameplay.png"
            } else if stage.baseline {
                "/tmp/terra-planet-gameplay-baseline.png"
            } else {
                "/tmp/terra-planet-gameplay-candidates.png"
            }));
        stage.phase = 1;
    }
    if stage.elapsed > 12. && stage.phase == 1 {
        let path = if stage.items {
            "/tmp/terra-items-planet.png"
        } else if stage.structures {
            "/tmp/terra-structures-planet.png"
        } else if stage.baseline {
            "/tmp/terra-planet-baseline.png"
        } else {
            "/tmp/terra-planet-candidates.png"
        };
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path));
        if let Some(fps) = diagnostics
            .get(&bevy::diagnostic::FrameTimeDiagnosticsPlugin::FPS)
            .and_then(|d| d.smoothed())
        {
            info!(
                fps,
                baseline = stage.baseline,
                "asset review frame rate (debug build, existing world resident)"
            );
        }
        stage.phase = 2;
    }
    if stage.elapsed > 14. && stage.phase == 2 {
        gizmos.config_mut::<PhysicsGizmos>().0.enabled = true;
        stage.phase = 3;
    }
    if stage.elapsed > 15. && stage.phase == 3 {
        if !stage.baseline {
            commands
                .spawn(Screenshot::primary_window())
                .observe(save_to_disk(if stage.items {
                    "/tmp/terra-items-planet-colliders.png"
                } else if stage.structures {
                    "/tmp/terra-structures-planet-colliders.png"
                } else {
                    "/tmp/terra-planet-colliders.png"
                }));
        }
        stage.phase = 4;
    }
    if stage.elapsed > 16. && stage.phase == 4 {
        if !stage.structures {
            exit.write(AppExit::Success);
        }
        gizmos.config_mut::<PhysicsGizmos>().0.enabled = false;
        stage.phase = 5;
    }
    if stage.structures && stage.elapsed > 19. && stage.phase == 5 {
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk("/tmp/terra-structures-repeats.png"));
        stage.phase = 6;
    }
    if stage.structures && stage.elapsed > 20. && stage.phase == 6 {
        exit.write(AppExit::Success);
        stage.phase = 7;
    }
}

fn toggle_colliders(keys: Res<ButtonInput<KeyCode>>, mut config: ResMut<GizmoConfigStore>) {
    if keys.just_pressed(KeyCode::KeyC) {
        let (config, _) = config.config_mut::<PhysicsGizmos>();
        config.enabled = !config.enabled;
    }
}
