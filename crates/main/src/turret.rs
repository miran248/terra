use crate::constants::*;
use crate::loot::LootState;
use crate::map::{GameAssets, Player};
use crate::zombie::Zombie;
use avian3d::prelude::*;
use bevy::prelude::*;
use shared::state::AppState;

#[derive(Message)]
pub struct WeaponFired;

#[derive(Resource)]
pub struct ProjectileSpeed(pub f32);

#[derive(Component)]
pub struct Projectile {
    pub damage: f32,
    pub target: Entity,
}

pub struct TurretPlugin;

impl Plugin for TurretPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<WeaponFired>()
            .insert_resource(ProjectileSpeed(50.0))
            .add_systems(Update, player_shoot.run_if(in_state(AppState::Playing)))
            .add_systems(
                FixedUpdate,
                move_projectiles.run_if(in_state(AppState::Playing)),
            );
    }
}

fn player_shoot(
    mut commands: Commands,
    time: Res<Time>,
    equipped: Res<LootState>,
    assets: Res<GameAssets>,
    mut fired: MessageWriter<WeaponFired>,
    mut player_q: Query<(&Transform, &mut Player)>,
    zombies: Query<(Entity, &Transform), (With<Zombie>, Without<Player>)>,
) {
    let Ok((tf, mut player)) = player_q.single_mut() else {
        return;
    };
    player.fire_timer.tick(time.delta());
    if !player.fire_timer.just_finished() {
        return;
    }

    let (damage, range, has_weapon) = match equipped.equipped {
        Some((kind, _)) => {
            let s = kind.stats();
            (s.damage, s.range, true)
        }
        None => (player.damage, player.range, false),
    };

    let p = tf.translation;
    let mut closest: Option<(Entity, f32)> = None;
    for (entity, z_tf) in &zombies {
        let dist = p.distance(z_tf.translation);
        if dist <= range {
            match closest {
                None => closest = Some((entity, dist)),
                Some((_, d)) if dist < d => closest = Some((entity, dist)),
                _ => {}
            }
        }
    }

    if let Some((target, _)) = closest {
        commands.spawn((
            Mesh3d(assets.projectile_mesh.clone()),
            MeshMaterial3d(assets.projectile_mat.clone()),
            RigidBody::Dynamic,
            Collider::sphere(PROJECTILE_SIZE),
            GravityScale(0.0),
            LinearDamping(0.0),
            LockedAxes::ROTATION_LOCKED,
            Restitution::ZERO,
            Transform::from_translation(p),
            Projectile { damage, target },
        ));
        if has_weapon {
            fired.write(WeaponFired);
        }
    }
}

fn move_projectiles(
    mut commands: Commands,
    p_speed: Res<ProjectileSpeed>,
    mut projectiles: Query<(Entity, &Transform, Forces, &Projectile)>,
    zombies: Query<&Transform, (With<Zombie>, Without<Projectile>)>,
) {
    let speed = p_speed.0;

    for (entity, tf, mut forces, proj) in &mut projectiles {
        if let Ok(target_tf) = zombies.get(proj.target) {
            let dir = (target_tf.translation - tf.translation).normalize_or_zero();
            forces.apply_force(dir * speed * 50.0);
        } else {
            commands.entity(entity).despawn();
        }
    }
}
