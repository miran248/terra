use bevy::prelude::*;
use shared::state::AppState;
use shared::upgrades::Upgrade;
use crate::constants::*;
use crate::map::Survivor;
use crate::zombie::Zombie;

#[derive(Component)]
pub struct Projectile {
    pub damage: f32,
    pub target: Entity,
}

#[derive(Resource)]
pub struct ProjectileSpeed(pub f32);

impl Default for ProjectileSpeed {
    fn default() -> Self {
        Self(Upgrade::ProjectileSpeed.value(0))
    }
}

pub struct TurretPlugin;

impl Plugin for TurretPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ProjectileSpeed>()
            .add_systems(Update, (survivor_shoot, move_projectiles).run_if(in_state(AppState::Playing)));
    }
}

fn survivor_shoot(
    mut commands: Commands,
    time: Res<Time>,
    mut survivor_q: Query<(Entity, &mut Survivor)>,
    survivor_tf_q: Query<&Transform, With<Survivor>>,
    zombies: Query<(Entity, &Transform), (With<Zombie>, Without<Survivor>)>,
) {
    let Ok((_entity, mut survivor)) = survivor_q.single_mut() else { return };
    let Ok(survivor_tf) = survivor_tf_q.single() else { return };
    survivor.fire_timer.tick(time.delta());
    if !survivor.fire_timer.just_finished() {
        return;
    }

    let pos = survivor_tf.translation.xy();
    let mut closest: Option<(Entity, f32)> = None;

    for (entity, z_tf) in &zombies {
        let dist = pos.distance(z_tf.translation.xy());
        if dist <= survivor.range {
            match closest {
                None => closest = Some((entity, dist)),
                Some((_, d)) if dist < d => closest = Some((entity, dist)),
                _ => {}
            }
        }
    }

    if let Some((target, _)) = closest {
        commands.spawn((
            Sprite::from_color(PROJECTILE_COLOR, Vec2::new(PROJECTILE_SIZE, PROJECTILE_SIZE)),
            Transform::from_translation(pos.extend(0.2)),
            Projectile {
                damage: survivor.damage,
                target,
            },
        ));
    }
}

fn move_projectiles(
    mut commands: Commands,
    time: Res<Time>,
    p_speed: Res<ProjectileSpeed>,
    mut projectiles: Query<(Entity, &mut Transform, &Projectile)>,
    zombies: Query<&Transform, (With<Zombie>, Without<Projectile>)>,
) {
    let dt = time.delta_secs();
    let speed = p_speed.0;

    for (entity, mut tf, proj) in &mut projectiles {
        if let Ok(target_tf) = zombies.get(proj.target) {
            let dir = (target_tf.translation.xy() - tf.translation.xy()).normalize_or_zero();
            tf.translation.x += dir.x * speed * dt;
            tf.translation.y += dir.y * speed * dt;
        } else {
            commands.entity(entity).despawn();
        }
    }
}
