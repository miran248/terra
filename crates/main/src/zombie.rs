use bevy::prelude::*;
use rand::Rng;
use crate::constants::*;
use crate::map::{Survivor, SurvivorHp};
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
            .add_systems(Update, (move_zombies, zombie_hit_survivor).chain().run_if(in_state(AppState::Playing)));
    }
}

fn spawn_zombies(
    mut commands: Commands,
    time: Res<Time>,
    mut wave: ResMut<WaveManager>,
    survivor_q: Query<&Transform, With<Survivor>>,
) {
    if wave.zombies_spawned_this_wave >= wave.zombies_per_wave {
        return;
    }

    wave.spawn_timer.tick(time.delta());
    if !wave.spawn_timer.just_finished() {
        return;
    }

    let center = survivor_q.single().map(|t| t.translation.xy()).unwrap_or(Vec2::ZERO);
    let mut rng = rand::thread_rng();
    let angle = rng.gen_range(0.0..std::f32::consts::TAU);
    let spawn_pos = center + Vec2::from_angle(angle) * SPAWN_RADIUS;

    wave.zombies_spawned_this_wave += 1;

    let w = wave.wave;
    commands.spawn((
        Sprite::from_color(ZOMBIE_COLOR, Vec2::new(ZOMBIE_SIZE, ZOMBIE_SIZE)),
        Transform::from_translation(spawn_pos.extend(0.0)),
        Zombie {
            hp: crate::wave::zombie_hp(w),
            speed: crate::wave::zombie_speed(w),
        },
    ));
}

fn move_zombies(
    time: Res<Time>,
    survivor_q: Query<&Transform, With<Survivor>>,
    mut q: Query<(&mut Transform, &Zombie), Without<Survivor>>,
) {
    let Ok(survivor_tf) = survivor_q.single() else { return };
    let target = survivor_tf.translation.xy();

    for (mut tf, zombie) in &mut q {
        let dir = (target - tf.translation.xy()).normalize_or_zero();
        tf.translation.x += dir.x * zombie.speed * time.delta_secs();
        tf.translation.y += dir.y * zombie.speed * time.delta_secs();
    }
}

fn zombie_hit_survivor(
    mut commands: Commands,
    mut survivor_hp: ResMut<SurvivorHp>,
    survivor_q: Query<(&Transform, &Survivor)>,
    zombies: Query<(Entity, &Transform), With<Zombie>>,
    mut next_state: ResMut<NextState<AppState>>,
) {
    let Ok((s_tf, _s)) = survivor_q.single() else { return };
    let s_pos = s_tf.translation.xy();
    const DAMAGE_PER_HIT: f32 = 50.0;

    let mut total_hp = survivor_hp.0;
    for (z_entity, z_tf) in &zombies {
        if z_tf.translation.xy().distance(s_pos) <= SURVIVOR_SIZE / 2.0 + ZOMBIE_SIZE / 2.0 {
            commands.entity(z_entity).despawn();
            total_hp -= DAMAGE_PER_HIT;
        }
    }

    survivor_hp.0 = total_hp;

    if total_hp <= 0.0 {
        next_state.set(AppState::GameOver);
    }
}
