use crate::combat::ScrapCounter;
use crate::constants::*;
use crate::map::Player;
use crate::ui::UpgradeLevels;
use avian3d::prelude::*;
use bevy::prelude::*;
use rand::Rng;
use shared::items::{Material, WeaponKind};
use shared::sphere::{PLANET_RADIUS, random_point};
use shared::state::AppState;
use shared::upgrades::Upgrade;

const MAGNET_SPEED: f32 = 70.0;
const COLLECT_RADIUS: f32 = 2.0;
const MATERIAL_COUNT: usize = 400;
const WEAPON_COUNT: usize = 60;
const ZOMBIE_DROP_CHANCE: f64 = 0.35;
const ZOMBIE_WEAPON_DROP_CHANCE: f64 = 0.05;

#[derive(Component, Clone, Copy)]
pub struct LootMaterial(pub Material);

#[derive(Component, Clone, Copy)]
pub struct LootWeapon(pub WeaponKind);

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
    pub weapons: Vec<WeaponKind>,
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
            .add_systems(
                OnEnter(AppState::Playing),
                (setup_loot_assets, scatter_loot).chain(),
            )
            .add_systems(
                Update,
                (
                    collect_loot,
                    apply_magnet_upgrade,
                    drain_durability,
                    apply_weapon_fire_rate,
                )
                    .run_if(in_state(AppState::Playing)),
            )
            .add_systems(FixedUpdate, magnet_loot.run_if(in_state(AppState::Playing)));
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
    let idx = Upgrade::ALL
        .iter()
        .position(|u| *u == Upgrade::MagnetRange)
        .unwrap();
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
        spawn_material(&mut commands, &assets, m, pos.0);
    }

    for _ in 0..WEAPON_COUNT {
        let w = WeaponKind::ALL[rng.gen_range(0..WeaponKind::ALL.len())];
        let pos = random_point(rng.r#gen(), rng.r#gen());
        spawn_weapon(&mut commands, &assets, w, pos.0);
    }
}

fn spawn_material(commands: &mut Commands, assets: &LootAssets, m: Material, dir: Vec3) {
    let i = Material::ALL.iter().position(|x| *x == m).unwrap();
    let r = PLANET_RADIUS + SCRAP_SIZE * 0.5 + 0.5;
    commands.spawn((
        Mesh3d(assets.material_mesh.clone()),
        MeshMaterial3d(assets.material_mats[i].clone()),
        RigidBody::Dynamic,
        Collider::sphere(SCRAP_SIZE * 0.5),
        GravityScale(0.0),
        LinearDamping(0.95),
        AngularDamping(1.0),
        LockedAxes::ROTATION_LOCKED,
        Restitution::ZERO,
        Friction::ZERO,
        Transform::from_translation(dir * r),
        LootMaterial(m),
    ));
}

fn spawn_weapon(commands: &mut Commands, assets: &LootAssets, w: WeaponKind, dir: Vec3) {
    let i = WeaponKind::ALL.iter().position(|x| *x == w).unwrap();
    let r = PLANET_RADIUS + SCRAP_SIZE * 1.8 * 0.5 + 0.5;
    commands.spawn((
        Mesh3d(assets.weapon_mesh.clone()),
        MeshMaterial3d(assets.weapon_mats[i].clone()),
        RigidBody::Dynamic,
        Collider::sphere(SCRAP_SIZE * 1.8 * 0.5),
        GravityScale(0.0),
        LinearDamping(0.95),
        AngularDamping(1.0),
        LockedAxes::ROTATION_LOCKED,
        Restitution::ZERO,
        Friction::ZERO,
        Transform::from_translation(dir * r),
        LootWeapon(w),
    ));
}

pub fn drop_zombie_loot(commands: &mut Commands, assets: &LootAssets, dir: Vec3) {
    let mut rng = rand::thread_rng();
    if rng.gen_bool(ZOMBIE_WEAPON_DROP_CHANCE) {
        let w = WeaponKind::ALL[rng.gen_range(0..WeaponKind::ALL.len())];
        spawn_weapon(commands, assets, w, dir);
    } else if rng.gen_bool(ZOMBIE_DROP_CHANCE) {
        let m = Material::ALL[rng.gen_range(0..Material::ALL.len())];
        spawn_material(commands, assets, m, dir);
    }
}

/// Pull loot toward the player via Forces API.
fn magnet_loot(
    magnet: Res<MagnetRadius>,
    player_q: Query<&Transform, With<Player>>,
    mut materials_q: Query<(Forces, &Transform), (With<LootMaterial>, Without<Player>)>,
    mut weapons_q: Query<
        (Forces, &Transform),
        (With<LootWeapon>, Without<LootMaterial>, Without<Player>),
    >,
) {
    let Ok(player_tf) = player_q.single() else {
        return;
    };
    let radius = magnet.0;
    let center_world = player_tf.translation;

    for (mut forces, tf) in &mut materials_q {
        magnet_accel(&mut forces, tf, center_world, radius);
    }
    for (mut forces, tf) in &mut weapons_q {
        magnet_accel(&mut forces, tf, center_world, radius);
    }
}

use avian3d::dynamics::rigid_body::forces::ForcesItem;

fn magnet_accel(forces: &mut ForcesItem, tf: &Transform, center: Vec3, radius: f32) {
    let dist = tf.translation.distance(center);
    if dist <= radius && dist > 0.001 {
        let dir = (center - tf.translation).normalize();
        let speed = MAGNET_SPEED * (1.0 + (1.0 - (dist / radius)) * 2.0);
        forces.apply_force(dir * speed * 10.0);
    }
}

/// Collect loot when close enough to the player.
fn collect_loot(
    mut commands: Commands,
    player_q: Query<(Entity, &Transform), With<Player>>,
    materials_q: Query<(Entity, &Transform, &LootMaterial)>,
    weapons_q: Query<(Entity, &Transform, &LootWeapon), Without<LootMaterial>>,
    mut loot: ResMut<LootState>,
    mut scrap: ResMut<ScrapCounter>,
) {
    let Ok((player_entity, player_tf)) = player_q.single() else {
        return;
    };

    for (entity, tf, mat) in &materials_q {
        if tf.translation.distance(player_tf.translation) <= COLLECT_RADIUS {
            commands.entity(entity).despawn();
            loot.add(mat.0, 1);
            scrap.0 += 1;
        }
    }

    for (entity, tf, w) in &weapons_q {
        if tf.translation.distance(player_tf.translation) <= COLLECT_RADIUS {
            commands.entity(entity).despawn();
            loot.weapons.push(w.0);
            if loot.equipped.is_none() {
                loot.equipped = Some((w.0, w.0.stats().durability));
            }
        }
    }
}

fn drain_durability(mut fired: MessageReader<WeaponFired>, mut loot: ResMut<LootState>) {
    for _ in fired.read() {
        let Some((kind, dur)) = loot.equipped else {
            continue;
        };
        if dur <= 1 {
            loot.weapons.retain(|w| *w != kind);
            loot.equipped = loot.weapons.first().map(|w| (*w, w.stats().durability));
        } else {
            loot.equipped = Some((kind, dur - 1));
        }
    }
}

fn apply_weapon_fire_rate(loot: Res<LootState>, mut player_q: Query<&mut Player>) {
    if !loot.is_changed() {
        return;
    }
    let Ok(mut player) = player_q.single_mut() else {
        return;
    };
    if let Some((kind, _)) = loot.equipped {
        let rate = kind.stats().fire_rate;
        player.fire_timer = Timer::from_seconds(1.0 / rate, TimerMode::Repeating);
    }
}
