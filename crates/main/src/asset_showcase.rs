//! Opt-in planet review for candidate assets, leaving the production catalog intact.
use crate::{
    asset_collision::candidate_collider,
    map::{MainCamera, Player},
};
use avian3d::prelude::RigidBody;
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
        app.add_plugins(ActorAnimationPlugin)
            .add_systems(
                Update,
                (setup, attach_weapon).run_if(in_state(AppState::Playing)),
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
}

fn setup(
    mut commands: Commands,
    server: Res<AssetServer>,
    stage: Option<Res<Showcase>>,
    player: Query<(&Transform, &Player)>,
    planet: Res<PlanetMesh>,
) {
    if stage.is_some() {
        return;
    }
    let Ok((player_transform, player)) = player.single() else {
        return;
    };
    let up = player_transform.translation.normalize();
    let forward = player.heading.normalize();
    let right = forward.cross(up);
    let center_direction = (player_transform.translation + forward * 4.).normalize();
    let center = center_direction * planet.facet_radius(center_direction, PLANET_RADIUS);
    let source: Handle<Gltf> = server.load("models/candidates/actor.player.glb");
    for action in 0..3 {
        let direction = (center + right * (action as f32 - 1.) * 1.2).normalize();
        let position = direction * planet.facet_radius(direction, PLANET_RADIUS);
        let heading = (-forward).reject_from(direction).normalize();
        let facing = Quat::from_mat3(&Mat3::from_cols(
            heading.cross(direction),
            direction,
            -heading,
        ));
        commands
            .spawn((
                RigidBody::Static,
                candidate_collider("actor.player", Vec3::ONE).unwrap(),
                Transform::from_translation(position).with_rotation(facing),
                Visibility::default(),
            ))
            .with_child((
                WorldAssetRoot(server.load("models/candidates/actor.player.glb#Scene0")),
                Transform::default(),
                ActorPlayback {
                    source: source.clone(),
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
        knife: server.load("models/candidates/weapon.knife.glb#Scene0"),
    });
}

#[derive(Component)]
struct Attached;
fn attach_weapon(
    mut commands: Commands,
    stage: Option<Res<Showcase>>,
    sockets: Query<(Entity, &Name), Without<Attached>>,
    parents: Query<&ChildOf>,
    roots: Query<&ActorPlayback>,
) {
    let Some(stage) = stage else {
        return;
    };
    for (entity, name) in &sockets {
        if name.as_str() == "socket.hand"
            && parents.iter_ancestors(entity).any(|e| roots.contains(e))
        {
            commands.entity(entity).insert(Attached).with_child((
                WorldAssetRoot(stage.knife.clone()),
                shared::asset_contract::grip_transform("weapon.knife").unwrap(),
            ));
        }
    }
}

fn capture(
    mut commands: Commands,
    time: Res<Time>,
    stage: Option<ResMut<Showcase>>,
    mut camera: Query<&mut Transform, With<MainCamera>>,
    mut exit: MessageWriter<AppExit>,
) {
    if std::env::var_os("TERRA_ASSET_CAPTURE").is_none() {
        return;
    }
    let Some(mut stage) = stage else {
        return;
    };
    stage.elapsed += time.delta_secs();
    if let Ok(mut camera) = camera.single_mut() {
        *camera = Transform::from_translation(stage.center - stage.forward * 4. + stage.up * 1.5)
            .looking_at(stage.center + stage.up * 0.5, stage.up);
    }
    if stage.elapsed > 8. && stage.phase == 0 {
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk("/tmp/terra-planet-animation.png"));
        stage.phase = 1;
    }
    if stage.elapsed > 10. && stage.phase == 1 {
        exit.write(AppExit::Success);
        stage.phase = 2;
    }
}
