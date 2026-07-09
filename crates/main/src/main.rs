mod combat;
mod constants;
mod loot;
mod map;
mod minimap;
mod physics;
mod prestige;
mod turret;
mod ui;
mod wave;
mod zombie;

use avian3d::prelude::*;
use bevy::diagnostic::FrameTimeDiagnosticsPlugin;
use bevy::prelude::*;
use shared::state::AppState;

fn main() {
    App::new()
        .add_plugins((
            DefaultPlugins,
            PhysicsPlugins::default().build().disable::<PhysicsInterpolationPlugin>(),
            PhysicsDiagnosticsPlugin,
            PhysicsDiagnosticsUiPlugin,
            FrameTimeDiagnosticsPlugin::default(),
            PhysicsDebugPlugin,
        ))
        .insert_resource(SubstepCount(12))
        .insert_resource(PhysicsDiagnosticsUiSettings { enabled: true, ..default() })
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
            physics::PhysicsPlugin,
        ))
        .add_systems(Startup, setup_camera)
        .run();
}

fn setup_camera(mut commands: Commands) {
    commands.spawn((
        Camera3d::default(),
        map::MainCamera,
        Transform::from_xyz(0.0, 900.0, 0.0).looking_at(Vec3::ZERO, Vec3::Z),
    ));
}
