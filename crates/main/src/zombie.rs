use avian3d::prelude::*;
use bevy::prelude::*;
use rand::Rng;
use shared::sphere::{ring_point, SpherePos, PLANET_RADIUS};
use crate::constants::*;
use crate::physics::RadialGravity;
use crate::map::{GameAssets, Player, PlayerHp};
use crate::wave::WaveManager;
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
            .add_systems(FixedUpdate, move_zombies.run_if(in_state(AppState::Playing)))
            .add_systems(Update, zombie_hit_player.run_if(in_state(AppState::Playing)));
    }
}

fn spawn_zombies(
    mut commands: Commands,
    time: Res<Time>,
    mut wave: ResMut<WaveManager>,
    assets: Res<GameAssets>,
    player_q: Query<&SpherePos, With<Player>>,
) {
    if wave.zombies_spawned_this_wave >= wave.zombies_per_wave { return; }

    wave.spawn_timer.tick(time.delta());
    if !wave.spawn_timer.just_finished() { return; }

    let center = player_q.single().copied().unwrap_or(SpherePos::new(Vec3::Y));
    let mut rng = rand::thread_rng();
    let angle = rng.gen_range(0.0..std::f32::consts::TAU);
    let spawn_pos = ring_point(center, SPAWN_RADIUS, angle);

    wave.zombies_spawned_this_wave += 1;

    let w = wave.wave;
    let half = ZOMBIE_SIZE * 0.5;
    let spawn_r = PLANET_RADIUS + half + 1.0;
    commands.spawn((
        Mesh3d(assets.zombie_mesh.clone()),
        MeshMaterial3d(assets.zombie_mat.clone()),
        RigidBody::Dynamic,
        RadialGravity,
        ColliderConstructor::Sphere { radius: ZOMBIE_SIZE * 0.5 },
        LockedAxes::ROTATION_LOCKED,
        Restitution::ZERO,
        Friction::ZERO,
        Transform::from_translation(spawn_pos.0 * spawn_r),
        spawn_pos,
        Zombie {
            hp: crate::wave::zombie_hp(w),
            speed: crate::wave::zombie_speed(w),
        },
    ));
}

fn move_zombies(
    time: Res<Time>,
    player_q: Query<(&SpherePos, &Transform), With<Player>>,
    mut q: Query<(&mut SpherePos, &Zombie, &Transform, Forces), Without<Player>>,
) {
    let Ok((target, player_tf)) = player_q.single() else { return };
    let dt = time.delta_secs();

    for (mut pos, zombie, tf, mut forces) in &mut q {
        pos.step_toward(*target, zombie.speed * dt);
        let r = tf.translation.length().max(PLANET_RADIUS);
        let desired = pos.0 * r;
        let delta = desired - tf.translation;
        let target_vel = delta / 0.2;
        let max_vel = zombie.speed * 1.5;
        let target_vel = if target_vel.length() > max_vel {
            target_vel.normalize_or_zero() * max_vel
        } else {
            target_vel
        };
        let current_vel = forces.linear_velocity();
        forces.apply_linear_acceleration((target_vel - current_vel) / dt.clamp(0.001, 1.0));
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
    let Ok((_p_entity, player_tf)) = player_q.single() else { return };
    let mut total_hp = player_hp.0;

    for (z_entity, z_tf) in &zombies {
        if z_tf.translation.distance(player_tf.translation) <= PLAYER_SIZE / 2.0 + ZOMBIE_SIZE / 2.0 {
            commands.entity(z_entity).despawn();
            total_hp -= DAMAGE_PER_HIT;
        }
    }

    player_hp.0 = total_hp;
    if total_hp <= 0.0 {
        next_state.set(AppState::GameOver);
    }
}
