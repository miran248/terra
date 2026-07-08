use bevy::prelude::*;
use rand::Rng;
use shared::sphere::{ring_point, SpherePos};
use crate::constants::*;
use crate::map::{GameAssets, Survivor, SurvivorHp};
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
    assets: Res<GameAssets>,
    survivor_q: Query<&SpherePos, With<Survivor>>,
) {
    if wave.zombies_spawned_this_wave >= wave.zombies_per_wave {
        return;
    }

    wave.spawn_timer.tick(time.delta());
    if !wave.spawn_timer.just_finished() {
        return;
    }

    let center = survivor_q.single().copied().unwrap_or(SpherePos::new(Vec3::Y));
    let mut rng = rand::thread_rng();
    let angle = rng.gen_range(0.0..std::f32::consts::TAU);
    let spawn_pos = ring_point(center, SPAWN_RADIUS, angle);

    wave.zombies_spawned_this_wave += 1;

    let w = wave.wave;
    commands.spawn((
        Mesh3d(assets.zombie_mesh.clone()),
        MeshMaterial3d(assets.zombie_mat.clone()),
        spawn_pos.surface_transform(0.0),
        spawn_pos,
        Zombie {
            hp: crate::wave::zombie_hp(w),
            speed: crate::wave::zombie_speed(w),
        },
    ));
}

fn move_zombies(
    time: Res<Time>,
    survivor_q: Query<&SpherePos, With<Survivor>>,
    mut q: Query<(&mut SpherePos, &Zombie), Without<Survivor>>,
) {
    let Ok(target) = survivor_q.single() else { return };
    let dt = time.delta_secs();

    for (mut pos, zombie) in &mut q {
        pos.step_toward(*target, zombie.speed * dt);
    }
}

fn zombie_hit_survivor(
    mut commands: Commands,
    mut survivor_hp: ResMut<SurvivorHp>,
    survivor_q: Query<&SpherePos, With<Survivor>>,
    zombies: Query<(Entity, &SpherePos), With<Zombie>>,
    mut next_state: ResMut<NextState<AppState>>,
) {
    let Ok(s_pos) = survivor_q.single() else { return };
    const DAMAGE_PER_HIT: f32 = 50.0;

    let mut total_hp = survivor_hp.0;
    for (z_entity, z_pos) in &zombies {
        if s_pos.distance(*z_pos) <= SURVIVOR_SIZE / 2.0 + ZOMBIE_SIZE / 2.0 {
            commands.entity(z_entity).despawn();
            total_hp -= DAMAGE_PER_HIT;
        }
    }

    survivor_hp.0 = total_hp;

    if total_hp <= 0.0 {
        next_state.set(AppState::GameOver);
    }
}
