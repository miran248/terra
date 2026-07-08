mod combat;
mod constants;
mod loot;
mod map;
mod minimap;
mod prestige;
mod turret;
mod ui;
mod wave;
mod zombie;

use bevy::prelude::*;
use shared::state::AppState;

fn main() {
    App::new()
        .add_plugins((DefaultPlugins,))
        .init_state::<AppState>()
        .add_plugins((
            map::MapPlugin,
            wave::WavePlugin,
            zombie::ZombiePlugin,
            turret::TurretPlugin,
            combat::CombatPlugin,
            loot::LootPlugin,
            ui::UiPlugin,
            minimap::MinimapPlugin,
            prestige::PrestigePlugin,
        ))
        .add_systems(Startup, setup_camera)
        .run();
}

fn setup_camera(mut commands: Commands) {
    commands.spawn(Camera2d);
}
