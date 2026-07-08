use bevy::prelude::*;
use shared::upgrades::Upgrade;
use crate::ui::UpgradeLevels;
use crate::zombie::Zombie;

#[derive(Resource)]
pub struct WaveManager {
    pub wave: u32,
    pub zombies_per_wave: u32,
    pub zombies_spawned_this_wave: u32,
    pub spawn_timer: Timer,
    pub between_wave_timer: Timer,
}

impl Default for WaveManager {
    fn default() -> Self {
        Self {
            wave: 1,
            zombies_per_wave: 5,
            zombies_spawned_this_wave: 0,
            spawn_timer: Timer::from_seconds(1.0 / Upgrade::SpawnRate.value(0), TimerMode::Repeating),
            between_wave_timer: Timer::from_seconds(2.0, TimerMode::Once),
        }
    }
}

pub struct WavePlugin;

impl Plugin for WavePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WaveManager>()
            .add_systems(Update, (apply_spawn_rate_upgrade, check_wave_complete));
    }
}

fn apply_spawn_rate_upgrade(
    levels: Res<UpgradeLevels>,
    mut wave: ResMut<WaveManager>,
) {
    if !levels.is_changed() {
        return;
    }
    let idx = Upgrade::ALL.iter().position(|u| *u == Upgrade::SpawnRate).unwrap();
    let interval = 1.0 / Upgrade::SpawnRate.value(levels.levels[idx]);
    wave.spawn_timer = Timer::from_seconds(interval, TimerMode::Repeating);
}

fn check_wave_complete(
    mut wave: ResMut<WaveManager>,
    zombies: Query<Entity, With<Zombie>>,
    time: Res<Time>,
) {
    let alive = zombies.iter().count() as u32;

    if alive > 0 || wave.zombies_spawned_this_wave < wave.zombies_per_wave {
        wave.between_wave_timer.reset();
        return;
    }

    wave.between_wave_timer.tick(time.delta());
    if wave.between_wave_timer.just_finished() {
        wave.wave += 1;
        wave.zombies_per_wave = zombie_count(wave.wave);
        wave.zombies_spawned_this_wave = 0;
        wave.between_wave_timer = Timer::from_seconds(2.0, TimerMode::Once);
    }
}

pub fn zombie_hp(wave: u32) -> f32 {
    20.0 * 1.2f32.powi(wave as i32 - 1)
}

pub fn zombie_speed(wave: u32) -> f32 {
    18.0 * (1.0 + 0.08 * (wave as f32 - 1.0))
}

pub fn zombie_count(wave: u32) -> u32 {
    5 + (wave - 1) * 4
}

pub fn spawn_interval(wave: u32) -> f32 {
    (1.5 - 0.04 * (wave as f32 - 1.0)).max(0.15)
}
