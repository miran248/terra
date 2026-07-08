use bevy::prelude::*;
use shared::sphere::SpherePos;
use shared::state::AppState;
use shared::upgrades::Upgrade;
use crate::constants::PROJECTILE_SIZE;
use crate::loot::{LootState, WeaponFired};
use crate::map::{GameAssets, GroundOffset, Player};
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
            .add_systems(Update, (player_shoot, move_projectiles).run_if(in_state(AppState::Playing)));
    }
}

fn player_shoot(
    mut commands: Commands,
    time: Res<Time>,
    equipped: Res<LootState>,
    assets: Res<GameAssets>,
    mut fired: MessageWriter<WeaponFired>,
    mut player_q: Query<(&SpherePos, &mut Player)>,
    zombies: Query<(Entity, &SpherePos), (With<Zombie>, Without<Player>)>,
) {
    let Ok((pos, mut player)) = player_q.single_mut() else { return };
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

    let mut closest: Option<(Entity, f32)> = None;
    for (entity, z_pos) in &zombies {
        let dist = pos.distance(*z_pos);
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
            pos.surface_transform(0.0),
            *pos,
            GroundOffset(PROJECTILE_SIZE),
            Projectile { damage, target },
        ));
        if has_weapon {
            fired.write(WeaponFired);
        }
    }
}

fn move_projectiles(
    mut commands: Commands,
    time: Res<Time>,
    p_speed: Res<ProjectileSpeed>,
    mut projectiles: Query<(Entity, &mut SpherePos, &Projectile)>,
    zombies: Query<&SpherePos, (With<Zombie>, Without<Projectile>)>,
) {
    let dt = time.delta_secs();
    let speed = p_speed.0;

    for (entity, mut pos, proj) in &mut projectiles {
        if let Ok(target) = zombies.get(proj.target) {
            pos.step_toward(*target, speed * dt);
        } else {
            commands.entity(entity).despawn();
        }
    }
}
