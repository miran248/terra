use avian3d::prelude::*;
use bevy::prelude::*;
use shared::upgrades::Upgrade;
use crate::constants::*;
use crate::turret::Projectile;
use crate::ui::UpgradeLevels;
use crate::zombie::Zombie;
use shared::state::AppState;

#[derive(Resource, Default)]
pub struct ScrapCounter(pub u32);

pub struct CombatPlugin;

impl Plugin for CombatPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ScrapCounter>()
            .add_systems(Update, projectiles_hit_zombies.run_if(in_state(AppState::Playing)));
    }
}

fn get_upgrade_count(levels: &UpgradeLevels, upgrade: Upgrade) -> u32 {
    let idx = Upgrade::ALL.iter().position(|u| *u == upgrade).unwrap();
    (Upgrade::Piercing.value(levels.levels[idx]) as u32).max(0)
}

fn projectile_behavior(levels: &UpgradeLevels) -> ProjectileBehavior {
    ProjectileBehavior {
        piercing: get_upgrade_count(levels, Upgrade::Piercing),
        bounces: get_upgrade_count(levels, Upgrade::Bounces),
        splits: get_upgrade_count(levels, Upgrade::Splits),
    }
}

#[derive(Clone)]
struct ProjectileBehavior {
    piercing: u32,
    bounces: u32,
    splits: u32,
}

fn projectiles_hit_zombies(
    mut commands: Commands,
    levels: Res<UpgradeLevels>,
    loot_assets: Res<crate::loot::LootAssets>,
    projectiles: Query<(Entity, &Transform, &Projectile)>,
    mut zombies: Query<(Entity, &Transform, &mut Zombie), Without<Projectile>>,
) {
    let behavior = projectile_behavior(&levels);
    let mut hits_this_frame: Vec<(Entity, Vec3, ProjectileBehavior, f32)> = Vec::new();

    for (p_entity, p_tf, proj) in &projectiles {
        if let Ok((z_entity, z_tf, mut zombie)) = zombies.get_mut(proj.target) {
            if p_tf.translation.distance(z_tf.translation) <= ZOMBIE_SIZE * 2.5 {
                zombie.hp -= proj.damage;
                commands.entity(p_entity).despawn();

                if zombie.hp <= 0.0 {
                    let dir = z_tf.translation.normalize();
                    commands.entity(z_entity).despawn();
                    crate::loot::drop_zombie_loot(&mut commands, &loot_assets, dir);
                }

                if behavior.piercing > 0 {
                    let mut new_behavior = behavior.clone();
                    new_behavior.piercing -= 1;
                    hits_this_frame.push((z_entity, z_tf.translation, new_behavior, proj.damage));
                }
            }
        }
    }

    for (last_target, last_pos, behavior, damage) in hits_this_frame {
        if behavior.piercing > 0 {
            let closest = find_closest_zombie(&zombies, last_target, last_pos, 30.0);
            if let Some((e, _)) = closest {
                handle_pierce(&mut commands, &loot_assets, &mut zombies, e, damage);
            }
        }
        if behavior.bounces > 0 {
            let closest = find_closest_zombie(&zombies, last_target, last_pos, 60.0);
            if let Some((e, _)) = closest {
                handle_pierce(&mut commands, &loot_assets, &mut zombies, e, damage);
            }
        }
        if behavior.splits > 0 {
            for (e, _) in find_n_closest_zombies(&zombies, last_target, last_pos, 45.0, behavior.splits) {
                handle_pierce(&mut commands, &loot_assets, &mut zombies, e, damage * 0.5);
            }
        }
    }
}

fn handle_pierce(
    commands: &mut Commands,
    loot_assets: &crate::loot::LootAssets,
    zombies: &mut Query<(Entity, &Transform, &mut Zombie), Without<Projectile>>,
    target: Entity,
    damage: f32,
) {
    if let Ok((_, z_tf, mut zombie)) = zombies.get_mut(target) {
        zombie.hp -= damage;
        if zombie.hp <= 0.0 {
            let dir = z_tf.translation.normalize();
            commands.entity(target).despawn();
            crate::loot::drop_zombie_loot(commands, loot_assets, dir);
        }
    }
}

fn find_closest_zombie(
    zombies: &Query<(Entity, &Transform, &mut Zombie), Without<Projectile>>,
    exclude: Entity,
    from: Vec3,
    max_dist: f32,
) -> Option<(Entity, Vec3)> {
    let mut best: Option<(Entity, f32, Vec3)> = None;
    for (e, tf, _) in zombies {
        if e == exclude { continue; }
        let d = from.distance(tf.translation);
        if d <= max_dist && d < best.map(|b| b.1).unwrap_or(f32::MAX) {
            best = Some((e, d, tf.translation));
        }
    }
    best.map(|b| (b.0, b.2))
}

fn find_n_closest_zombies(
    zombies: &Query<(Entity, &Transform, &mut Zombie), Without<Projectile>>,
    exclude: Entity,
    from: Vec3,
    max_dist: f32,
    n: u32,
) -> Vec<(Entity, Vec3)> {
    let mut dists: Vec<(Entity, f32, Vec3)> = zombies
        .iter()
        .filter(|(e, _, _)| *e != exclude)
        .map(|(e, tf, _)| (e, from.distance(tf.translation), tf.translation))
        .filter(|(_, d, _)| *d <= max_dist)
        .collect();
    dists.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
    dists.truncate(n as usize);
    dists.into_iter().map(|(e, _, p)| (e, p)).collect()
}
