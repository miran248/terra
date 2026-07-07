use bevy::prelude::*;
use rand::Rng;
use shared::upgrades::Upgrade;
use crate::constants::*;
use crate::turret::Projectile;
use crate::ui::UpgradeLevels;
use crate::zombie::Zombie;
use shared::state::AppState;

#[derive(Component)]
pub struct Scrap {
    pub value: u32,
}

#[derive(Component)]
pub struct RareScrap;

#[derive(Resource, Default)]
pub struct ScrapCounter(pub u32);

pub struct CombatPlugin;

impl Plugin for CombatPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ScrapCounter>()
            .add_systems(Update, projectiles_hit_zombies.run_if(in_state(AppState::Playing)));
    }
}

fn scrap_value(levels: &UpgradeLevels) -> u32 {
    let idx = Upgrade::ALL.iter().position(|u| *u == Upgrade::ScrapValue).unwrap();
    Upgrade::ScrapValue.value(levels.levels[idx]) as u32
}

fn rare_chance(levels: &UpgradeLevels) -> f32 {
    let idx = Upgrade::ALL.iter().position(|u| *u == Upgrade::RareScrap).unwrap();
    Upgrade::RareScrap.value(levels.levels[idx])
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

fn spawn_scrap(commands: &mut Commands, pos: Vec2, levels: &UpgradeLevels) {
    let value = scrap_value(levels);
    let is_rare = rand::thread_rng().gen_bool(rare_chance(levels) as f64);
    let final_value = if is_rare { value * 3 } else { value };
    let color = if is_rare {
        Color::srgb(1.0, 0.3, 1.0)
    } else {
        SCRAP_COLOR
    };
    let mut rng = rand::thread_rng();
    let offset = Vec2::new(
        rng.gen_range(-10.0..10.0),
        rng.gen_range(-10.0..10.0),
    );
    let mut entity = commands.spawn((
        Sprite::from_color(color, Vec2::new(SCRAP_SIZE, SCRAP_SIZE)),
        Transform::from_translation((pos + offset).extend(0.3)),
        Scrap { value: final_value },
    ));
    if is_rare {
        entity.insert(RareScrap);
    }
}

fn projectiles_hit_zombies(
    mut commands: Commands,
    levels: Res<UpgradeLevels>,
    projectiles: Query<(Entity, &Transform, &Projectile)>,
    mut zombies: Query<(Entity, &Transform, &mut Zombie), Without<Projectile>>,
) {
    let behavior = projectile_behavior(&levels);
    let multishot_count = {
        let idx = Upgrade::ALL.iter().position(|u| *u == Upgrade::Multishot).unwrap();
        Upgrade::Multishot.value(levels.levels[idx]) as u32
    };

    let mut hits_this_frame: Vec<(Entity, Vec2, ProjectileBehavior, f32)> = Vec::new();

    for (p_entity, p_tf, proj) in &projectiles {
        let p_pos = p_tf.translation.xy();

        if let Ok((z_entity, z_tf, mut zombie)) = zombies.get_mut(proj.target) {
            let z_pos = z_tf.translation.xy();
            if p_pos.distance(z_pos) <= ZOMBIE_SIZE {
                zombie.hp -= proj.damage;
                commands.entity(p_entity).despawn();

                if zombie.hp <= 0.0 {
                    commands.entity(z_entity).despawn();
                    spawn_scrap(&mut commands, z_pos, &levels);

                    if multishot_count > 1 {
                        for _ in 1..multishot_count {
                            spawn_scrap(&mut commands, z_pos, &levels);
                        }
                    }
                }

                if behavior.piercing > 0 {
                    let mut new_behavior = behavior.clone();
                    new_behavior.piercing -= 1;
                    hits_this_frame.push((z_entity, z_pos, new_behavior, proj.damage));
                }
            }
        }
    }

    for (last_target, last_pos, behavior, damage) in hits_this_frame {
        if behavior.piercing > 0 {
            let closest = find_closest_zombie(&zombies, last_target, last_pos, 100.0);
            if let Some((e, _)) = closest {
                handle_pierce(&mut commands, &mut zombies, &levels, e, damage, behavior.clone(), multishot_count);
            }
        }
        if behavior.bounces > 0 {
            let closest = find_closest_zombie(&zombies, last_target, last_pos, 200.0);
            if let Some((e, _)) = closest {
                let mut b = behavior.clone();
                b.bounces -= 1;
                handle_pierce(&mut commands, &mut zombies, &levels, e, damage, b, multishot_count);
            }
        }
        if behavior.splits > 0 {
            for (e, _) in find_n_closest_zombies(&zombies, last_target, last_pos, 150.0, behavior.splits) {
                let mut b = behavior.clone();
                b.splits -= 1;
                let split_dmg = damage * 0.5;
                handle_pierce(&mut commands, &mut zombies, &levels, e, split_dmg, b, multishot_count);
            }
        }
    }
}

fn handle_pierce(
    commands: &mut Commands,
    zombies: &mut Query<(Entity, &Transform, &mut Zombie), Without<Projectile>>,
    levels: &UpgradeLevels,
    target: Entity,
    damage: f32,
    _behavior: ProjectileBehavior,
    multishot: u32,
) {
    if let Ok((_, z_tf, mut zombie)) = zombies.get_mut(target) {
        zombie.hp -= damage;
        let z_pos = z_tf.translation.xy();
        if zombie.hp <= 0.0 {
            commands.entity(target).despawn();
            spawn_scrap(commands, z_pos, levels);
            for _ in 1..multishot {
                spawn_scrap(commands, z_pos, levels);
            }
        }
    }
}

fn find_closest_zombie(
    zombies: &Query<(Entity, &Transform, &mut Zombie), Without<Projectile>>,
    exclude: Entity,
    from: Vec2,
    max_dist: f32,
) -> Option<(Entity, Vec2)> {
    let mut best: Option<(Entity, f32, Vec2)> = None;
    for (e, tf, _) in zombies {
        if e == exclude { continue; }
        let d = from.distance(tf.translation.xy());
        if d <= max_dist && d < best.map(|b| b.1).unwrap_or(f32::MAX) {
            best = Some((e, d, tf.translation.xy()));
        }
    }
    best.map(|b| (b.0, b.2))
}

fn find_n_closest_zombies(
    zombies: &Query<(Entity, &Transform, &mut Zombie), Without<Projectile>>,
    exclude: Entity,
    from: Vec2,
    max_dist: f32,
    n: u32,
) -> Vec<(Entity, Vec2)> {
    let mut dists: Vec<(Entity, f32, Vec2)> = zombies
        .iter()
        .filter(|(e, _, _)| *e != exclude)
        .map(|(e, tf, _)| (e, from.distance(tf.translation.xy()), tf.translation.xy()))
        .filter(|(_, d, _)| *d <= max_dist)
        .collect();
    dists.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
    dists.truncate(n as usize);
    dists.into_iter().map(|(e, _, p)| (e, p)).collect()
}
