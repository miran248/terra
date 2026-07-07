use bevy::prelude::*;
use shared::upgrades::Upgrade;
use crate::combat::Scrap;
use crate::combat::ScrapCounter;
use crate::map::Survivor;
use crate::ui::UpgradeLevels;
use shared::state::AppState;

const MAGNET_SPEED: f32 = 200.0;
const COLLECT_RADIUS: f32 = 10.0;

#[derive(Resource)]
pub struct MagnetRadius(pub f32);

impl Default for MagnetRadius {
    fn default() -> Self {
        Self(Upgrade::MagnetRange.value(0))
    }
}

pub struct PickupPlugin;

impl Plugin for PickupPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MagnetRadius>()
            .add_systems(Update, (magnet_scrap, apply_magnet_upgrade).run_if(in_state(AppState::Playing)));
    }
}

fn apply_magnet_upgrade(
    levels: Res<UpgradeLevels>,
    mut magnet: ResMut<MagnetRadius>,
) {
    if !levels.is_changed() {
        return;
    }
    let idx = Upgrade::ALL.iter().position(|u| *u == Upgrade::MagnetRange).unwrap();
    magnet.0 = Upgrade::MagnetRange.value(levels.levels[idx]);
}

fn magnet_scrap(
    mut commands: Commands,
    time: Res<Time>,
    magnet: Res<MagnetRadius>,
    survivor_q: Query<&Transform, With<Survivor>>,
    mut scrap_counter: ResMut<ScrapCounter>,
    mut scrap_items: Query<(Entity, &mut Transform, &Scrap), Without<Survivor>>,
) {
    let Ok(survivor_tf) = survivor_q.single() else { return };
    let center = survivor_tf.translation.xy();
    let dt = time.delta_secs();
    let radius = magnet.0;

    for (entity, mut tf, scrap) in &mut scrap_items {
        let to_center = center - tf.translation.xy();
        let dist = to_center.length();

        if dist <= radius {
            let dir = to_center.normalize_or_zero();
            let speed = MAGNET_SPEED * (1.0 + (1.0 - (dist / radius)) * 2.0);
            tf.translation.x += dir.x * speed * dt;
            tf.translation.y += dir.y * speed * dt;
        }

        if tf.translation.xy().distance(center) <= COLLECT_RADIUS {
            commands.entity(entity).despawn();
            scrap_counter.0 += scrap.value;
        }
    }
}
