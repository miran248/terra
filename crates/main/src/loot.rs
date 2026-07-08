use bevy::prelude::*;
use rand::Rng;
use shared::items::{Material, WeaponKind};
use shared::sphere::{random_point, SpherePos};
use shared::state::AppState;
use shared::upgrades::Upgrade;
use crate::constants::*;
use crate::combat::ScrapCounter;
use crate::map::{GroundOffset, Survivor};
use crate::ui::UpgradeLevels;

const MAGNET_SPEED: f32 = 70.0; // m/s
const COLLECT_RADIUS: f32 = 2.0; // m
const MATERIAL_COUNT: usize = 400;
const WEAPON_COUNT: usize = 60;
const ZOMBIE_DROP_CHANCE: f64 = 0.35;
const ZOMBIE_WEAPON_DROP_CHANCE: f64 = 0.05;

#[derive(Component, Clone, Copy)]
pub struct LootMaterial(pub Material);

#[derive(Component, Clone, Copy)]
pub struct LootWeapon(pub WeaponKind);

/// Cached meshes/materials for loot pickups (colors from `items`).
#[derive(Resource)]
pub struct LootAssets {
    material_mesh: Handle<Mesh>,
    weapon_mesh: Handle<Mesh>,
    material_mats: [Handle<StandardMaterial>; Material::ALL.len()],
    weapon_mats: [Handle<StandardMaterial>; WeaponKind::ALL.len()],
}

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
            .add_systems(OnEnter(AppState::Playing), (setup_loot_assets, scatter_loot).chain())
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

fn setup_loot_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
) {
    let material_mats = Material::ALL.map(|m| {
        let (r, g, b) = m.color();
        mats.add(StandardMaterial::from_color(Color::srgb(r, g, b)))
    });
    let weapon_mats = WeaponKind::ALL.map(|w| {
        let (r, g, b) = w.color();
        mats.add(StandardMaterial::from_color(Color::srgb(r, g, b)))
    });
    commands.insert_resource(LootAssets {
        material_mesh: meshes.add(Cuboid::from_length(SCRAP_SIZE)),
        weapon_mesh: meshes.add(Cuboid::from_length(SCRAP_SIZE * 1.8)),
        material_mats,
        weapon_mats,
    });
}

fn scatter_loot(mut commands: Commands, assets: Res<LootAssets>) {
    let mut rng = rand::thread_rng();

    for _ in 0..MATERIAL_COUNT {
        let m = Material::ALL[rng.gen_range(0..Material::ALL.len())];
        let pos = random_point(rng.r#gen(), rng.r#gen());
        spawn_material(&mut commands, &assets, m, pos);
    }

    for _ in 0..WEAPON_COUNT {
        let w = WeaponKind::ALL[rng.gen_range(0..WeaponKind::ALL.len())];
        let pos = random_point(rng.r#gen(), rng.r#gen());
        spawn_weapon(&mut commands, &assets, w, pos);
    }
}

fn spawn_material(commands: &mut Commands, assets: &LootAssets, m: Material, pos: SpherePos) {
    let i = Material::ALL.iter().position(|x| *x == m).unwrap();
    commands.spawn((
        Mesh3d(assets.material_mesh.clone()),
        MeshMaterial3d(assets.material_mats[i].clone()),
        pos.surface_transform(0.0),
        pos,
        GroundOffset(SCRAP_SIZE * 0.5),
        LootMaterial(m),
    ));
}

fn spawn_weapon(commands: &mut Commands, assets: &LootAssets, w: WeaponKind, pos: SpherePos) {
    let i = WeaponKind::ALL.iter().position(|x| *x == w).unwrap();
    commands.spawn((
        Mesh3d(assets.weapon_mesh.clone()),
        MeshMaterial3d(assets.weapon_mats[i].clone()),
        pos.surface_transform(0.0),
        pos,
        GroundOffset(SCRAP_SIZE * 1.8 * 0.5),
        LootWeapon(w),
    ));
}

/// Roll random loot at a dead zombie's position.
pub fn drop_zombie_loot(commands: &mut Commands, assets: &LootAssets, pos: SpherePos) {
    let mut rng = rand::thread_rng();
    if rng.gen_bool(ZOMBIE_WEAPON_DROP_CHANCE) {
        let w = WeaponKind::ALL[rng.gen_range(0..WeaponKind::ALL.len())];
        spawn_weapon(commands, assets, w, pos);
    } else if rng.gen_bool(ZOMBIE_DROP_CHANCE) {
        let m = Material::ALL[rng.gen_range(0..Material::ALL.len())];
        spawn_material(commands, assets, m, pos);
    }
}

#[allow(clippy::too_many_arguments)]
fn magnet_loot(
    mut commands: Commands,
    time: Res<Time>,
    magnet: Res<MagnetRadius>,
    survivor_q: Query<&SpherePos, With<Survivor>>,
    mut loot: ResMut<LootState>,
    mut scrap: ResMut<ScrapCounter>,
    mut materials_q: Query<(Entity, &mut SpherePos, &LootMaterial), Without<Survivor>>,
    mut weapons_q: Query<(Entity, &mut SpherePos, &LootWeapon), (Without<Survivor>, Without<LootMaterial>)>,
) {
    let Ok(&center) = survivor_q.single() else { return };
    let dt = time.delta_secs();
    let radius = magnet.0;

    for (entity, mut pos, mat) in &mut materials_q {
        if pull(&mut pos, center, radius, dt) {
            commands.entity(entity).despawn();
            loot.add(mat.0, 1);
            scrap.0 += 1;
        }
    }

    for (entity, mut pos, w) in &mut weapons_q {
        if pull(&mut pos, center, radius, dt) {
            commands.entity(entity).despawn();
            loot.weapons.push(w.0);
            if loot.equipped.is_none() {
                loot.equipped = Some((w.0, w.0.stats().durability));
            }
        }
    }
}

fn pull(pos: &mut SpherePos, center: SpherePos, radius: f32, dt: f32) -> bool {
    let dist = pos.distance(center);
    if dist <= radius && dist > 0.0 {
        let speed = MAGNET_SPEED * (1.0 + (1.0 - (dist / radius)) * 2.0);
        pos.step_toward(center, speed * dt);
    }
    pos.distance(center) <= COLLECT_RADIUS
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
