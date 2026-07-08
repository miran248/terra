use bevy::prelude::*;
use rand::Rng;
use shared::items::{Material, WeaponKind};
use shared::state::AppState;
use shared::upgrades::Upgrade;
use crate::constants::*;
use crate::combat::ScrapCounter;
use crate::map::Survivor;
use crate::ui::UpgradeLevels;

const MAGNET_SPEED: f32 = 200.0;
const COLLECT_RADIUS: f32 = 6.0;
const MATERIAL_COUNT: usize = 120;
const WEAPON_COUNT: usize = 18;
const ZOMBIE_DROP_CHANCE: f64 = 0.35;
const ZOMBIE_WEAPON_DROP_CHANCE: f64 = 0.05;

#[derive(Component, Clone, Copy)]
pub struct LootMaterial(pub Material);

#[derive(Component, Clone, Copy)]
pub struct LootWeapon(pub WeaponKind);

#[derive(Resource, Default)]
pub struct LootState {
    pub materials: [u32; Material::ALL.len()],
    /// Weapons the player has collected (excluding the equipped one when broken).
    pub weapons: Vec<WeaponKind>,
    /// Currently equipped weapon and its remaining durability.
    pub equipped: Option<(WeaponKind, u32)>,
}

impl LootState {
    pub fn count(&self, m: Material) -> u32 {
        self.materials[Material::ALL.iter().position(|x| *x == m).unwrap()]
    }
    pub fn add(&mut self, m: Material, n: u32) {
        self.materials[Material::ALL.iter().position(|x| *x == m).unwrap()] += n;
    }
    pub fn try_spend(&mut self, m: Material, n: u32) -> bool {
        let i = Material::ALL.iter().position(|x| *x == m).unwrap();
        if self.materials[i] < n {
            return false;
        }
        self.materials[i] -= n;
        true
    }
}

#[derive(Message)]
pub struct WeaponFired;

pub struct LootPlugin;

impl Plugin for LootPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LootState>()
            .init_resource::<MagnetRadius>()
            .add_message::<WeaponFired>()
            .add_systems(OnEnter(AppState::Playing), scatter_loot)
            .add_systems(
                Update,
                (magnet_loot, apply_magnet_upgrade, drain_durability, apply_weapon_fire_rate)
                    .run_if(in_state(AppState::Playing)),
            );
    }
}

#[derive(Resource)]
pub struct MagnetRadius(pub f32);

impl Default for MagnetRadius {
    fn default() -> Self {
        Self(Upgrade::MagnetRange.value(0))
    }
}

fn apply_magnet_upgrade(levels: Res<UpgradeLevels>, mut magnet: ResMut<MagnetRadius>) {
    if !levels.is_changed() {
        return;
    }
    let idx = Upgrade::ALL.iter().position(|u| *u == Upgrade::MagnetRange).unwrap();
    magnet.0 = Upgrade::MagnetRange.value(levels.levels[idx]);
}

fn scatter_loot(mut commands: Commands) {
    let mut rng = rand::thread_rng();
    let hw = MAP_WIDTH / 2.0 - 20.0;
    let hh = MAP_HEIGHT / 2.0 - 20.0;

    for _ in 0..MATERIAL_COUNT {
        let m = Material::ALL[rng.gen_range(0..Material::ALL.len())];
        let pos = Vec2::new(rng.gen_range(-hw..hw), rng.gen_range(-hh..hh));
        spawn_material(&mut commands, m, pos);
    }

    for _ in 0..WEAPON_COUNT {
        let w = WeaponKind::ALL[rng.gen_range(0..WeaponKind::ALL.len())];
        let pos = Vec2::new(rng.gen_range(-hw..hw), rng.gen_range(-hh..hh));
        spawn_weapon(&mut commands, w, pos);
    }
}

fn spawn_material(commands: &mut Commands, m: Material, pos: Vec2) {
    let (r, g, b) = m.color();
    commands.spawn((
        Sprite::from_color(Color::srgb(r, g, b), Vec2::splat(SCRAP_SIZE)),
        Transform::from_translation(pos.extend(0.3)),
        LootMaterial(m),
    ));
}

fn spawn_weapon(commands: &mut Commands, w: WeaponKind, pos: Vec2) {
    let (r, g, b) = w.color();
    commands.spawn((
        Sprite::from_color(Color::srgb(r, g, b), Vec2::splat(SCRAP_SIZE * 1.8)),
        Transform::from_translation(pos.extend(0.3)),
        LootWeapon(w),
    ));
}

/// Roll random loot at a dead zombie's position.
pub fn drop_zombie_loot(commands: &mut Commands, pos: Vec2) {
    let mut rng = rand::thread_rng();
    if rng.gen_bool(ZOMBIE_WEAPON_DROP_CHANCE) {
        let w = WeaponKind::ALL[rng.gen_range(0..WeaponKind::ALL.len())];
        spawn_weapon(commands, w, pos);
    } else if rng.gen_bool(ZOMBIE_DROP_CHANCE) {
        let m = Material::ALL[rng.gen_range(0..Material::ALL.len())];
        spawn_material(commands, m, pos);
    }
}

#[allow(clippy::too_many_arguments)]
fn magnet_loot(
    mut commands: Commands,
    time: Res<Time>,
    magnet: Res<MagnetRadius>,
    survivor_q: Query<&Transform, With<Survivor>>,
    mut loot: ResMut<LootState>,
    mut scrap: ResMut<ScrapCounter>,
    mut materials_q: Query<(Entity, &mut Transform, &LootMaterial), Without<Survivor>>,
    mut weapons_q: Query<(Entity, &mut Transform, &LootWeapon), (Without<Survivor>, Without<LootMaterial>)>,
) {
    let Ok(survivor_tf) = survivor_q.single() else { return };
    let center = survivor_tf.translation.xy();
    let dt = time.delta_secs();
    let radius = magnet.0;

    for (entity, mut tf, mat) in &mut materials_q {
        if pull(&mut tf, center, radius, dt) {
            commands.entity(entity).despawn();
            loot.add(mat.0, 1);
            scrap.0 += 1;
        }
    }

    for (entity, mut tf, w) in &mut weapons_q {
        if pull(&mut tf, center, radius, dt) {
            commands.entity(entity).despawn();
            loot.weapons.push(w.0);
            if loot.equipped.is_none() {
                loot.equipped = Some((w.0, w.0.stats().durability));
            }
        }
    }
}

fn pull(tf: &mut Transform, center: Vec2, radius: f32, dt: f32) -> bool {
    let to_center = center - tf.translation.xy();
    let dist = to_center.length();
    if dist <= radius {
        let dir = to_center.normalize_or_zero();
        let speed = MAGNET_SPEED * (1.0 + (1.0 - (dist / radius)) * 2.0);
        tf.translation.x += dir.x * speed * dt;
        tf.translation.y += dir.y * speed * dt;
    }
    tf.translation.xy().distance(center) <= COLLECT_RADIUS
}

fn drain_durability(
    mut fired: MessageReader<WeaponFired>,
    mut loot: ResMut<LootState>,
) {
    for _ in fired.read() {
        let Some((kind, dur)) = loot.equipped else { continue };
        if dur <= 1 {
            // ponytail: broken weapon auto-swaps to next in inventory, else unarmed
            loot.weapons.retain(|w| *w != kind);
            loot.equipped = loot.weapons.first().map(|w| (*w, w.stats().durability));
        } else {
            loot.equipped = Some((kind, dur - 1));
        }
    }
}

fn apply_weapon_fire_rate(
    loot: Res<LootState>,
    mut survivor_q: Query<&mut Survivor>,
) {
    if !loot.is_changed() {
        return;
    }
    let Ok(mut survivor) = survivor_q.single_mut() else { return };
    if let Some((kind, _)) = loot.equipped {
        let rate = kind.stats().fire_rate;
        survivor.fire_timer = Timer::from_seconds(1.0 / rate, TimerMode::Repeating);
    }
}
