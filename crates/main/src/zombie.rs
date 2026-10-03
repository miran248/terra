use crate::constants::*;
use crate::map::{Player, PlayerHp};
use crate::physics::RadialGravity;
use crate::wave::WaveManager;
use avian3d::prelude::*;
use bevy::prelude::*;
use rand::RngExt;
use shared::sphere::PLANET_RADIUS;
use shared::state::AppState;

#[derive(Component)]
pub struct Zombie {
    pub hp: f32,
    pub speed: f32,
}

pub struct ZombiePlugin;

impl Plugin for ZombiePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, spawn_zombies.run_if(in_state(AppState::Playing)))
            .add_systems(
                FixedUpdate,
                move_zombies.run_if(in_state(AppState::Playing)),
            )
            .add_systems(
                Update,
                zombie_hit_player.run_if(in_state(AppState::Playing)),
            );
    }
}

fn spawn_zombies(
    mut commands: Commands,
    time: Res<Time>,
    mut wave: ResMut<WaveManager>,
    catalog: Res<crate::asset_catalog::AssetCatalog>,
    player_q: Query<&Transform, With<Player>>,
) {
    if wave.zombies_spawned_this_wave >= wave.zombies_per_wave {
        return;
    }

    wave.spawn_timer.tick(time.delta());
    if !wave.spawn_timer.just_finished() {
        return;
    }

    let center_pos = player_q
        .single()
        .map(|t| t.translation.normalize())
        .unwrap_or(Vec3::Y);
    let mut rng = rand::rng();
    let angle = rng.random_range(0.0..std::f32::consts::TAU);
    let perp = Vec3::new(center_pos.z, 0.0, -center_pos.x).normalize_or(Vec3::X);
    let step = SPAWN_RADIUS / PLANET_RADIUS;
    let spawn_dir =
        Quat::from_axis_angle(center_pos, angle) * perp * step.sin() + center_pos * step.cos();

    wave.zombies_spawned_this_wave += 1;

    let w = wave.wave;
    let name = if wave.wave.is_multiple_of(2) {
        "actor.zombie.1"
    } else {
        "actor.zombie.0"
    };
    let (collider, half) = crate::asset_collision::actor_body(name);
    let spawn_r = PLANET_RADIUS + half + 1.0;
    commands
        .spawn((
            RigidBody::Dynamic,
            RadialGravity,
            collider,
            crate::physics::RadialUpright,
            LockedAxes::ROTATION_LOCKED,
            Restitution::ZERO,
            Friction::ZERO,
            Transform::from_translation(spawn_dir * spawn_r)
                .with_rotation(Quat::from_rotation_arc(Vec3::Y, spawn_dir)),
            Visibility::default(),
            Zombie {
                hp: crate::wave::zombie_hp(w),
                speed: crate::wave::zombie_speed(w),
            },
        ))
        .with_child((
            WorldAssetRoot(catalog.scene(name)),
            catalog.actor(name, 1),
            Transform::from_xyz(0.0, -half, 0.0),
        ));
}

fn move_zombies(
    player_q: Query<&Transform, With<Player>>,
    mut q: Query<(&Zombie, &Transform, Forces), Without<Player>>,
) {
    let Ok(player_tf) = player_q.single() else {
        return;
    };

    for (zombie, tf, mut forces) in &mut q {
        let dir = (player_tf.translation - tf.translation).normalize_or_zero();
        forces.apply_force(dir * zombie.speed * 100.0);
    }
}

const DAMAGE_PER_HIT: f32 = 50.0;

fn zombie_hit_player(
    mut commands: Commands,
    mut player_hp: ResMut<PlayerHp>,
    player_q: Query<(Entity, &Transform), With<Player>>,
    zombies: Query<(Entity, &Transform), With<Zombie>>,
    mut next_state: ResMut<NextState<AppState>>,
) {
    let Ok((_p_entity, player_tf)) = player_q.single() else {
        return;
    };
    let mut total_hp = player_hp.0;

    for (z_entity, z_tf) in &zombies {
        if z_tf.translation.distance(player_tf.translation) <= PLAYER_SIZE / 2.0 + ZOMBIE_SIZE / 2.0
        {
            commands.entity(z_entity).despawn();
            total_hp -= DAMAGE_PER_HIT;
        }
    }

    player_hp.0 = total_hp;
    if total_hp <= 0.0 {
        next_state.set(AppState::GameOver);
    }
}
