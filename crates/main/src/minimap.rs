use bevy::prelude::*;
use shared::state::AppState;
use shared::theme;
use crate::constants::*;
use crate::loot::{LootMaterial, LootWeapon};
use crate::map::Survivor;
use crate::zombie::Zombie;

const MINIMAP_SIZE: f32 = 160.0;
const DOT: f32 = 3.0;

#[derive(Component)]
struct Minimap;

#[derive(Component)]
struct MinimapDot;

pub struct MinimapPlugin;

impl Plugin for MinimapPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_minimap)
            .add_systems(Update, update_minimap.run_if(in_state(AppState::Playing)));
    }
}

fn setup_minimap(mut commands: Commands) {
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(8.0),
            right: Val::Px(208.0),
            width: Val::Px(MINIMAP_SIZE),
            height: Val::Px(MINIMAP_SIZE),
            border: UiRect::all(Val::Px(3.0)),
            border_radius: BorderRadius::all(Val::Percent(50.0)),
            overflow: Overflow::clip(),
            ..default()
        },
        BackgroundColor(theme::PANEL_BG),
        BorderColor::all(theme::TEXT_WEAK),
        GlobalZIndex(10),
        Minimap,
    ));
}

fn update_minimap(
    mut commands: Commands,
    minimap_q: Query<Entity, With<Minimap>>,
    dots: Query<Entity, With<MinimapDot>>,
    survivor_q: Query<&Transform, With<Survivor>>,
    zombies: Query<&Transform, With<Zombie>>,
    materials: Query<&Transform, With<LootMaterial>>,
    weapons: Query<&Transform, With<LootWeapon>>,
) {
    let Ok(map_entity) = minimap_q.single() else { return };

    for dot in &dots {
        commands.entity(dot).despawn();
    }

    // Logical content size (node minus the 3px border on each side).
    let size = Vec2::splat(MINIMAP_SIZE - 6.0);
    let scale = (size.x / MAP_WIDTH).min(size.y / MAP_HEIGHT);
    let offset = Vec2::new(
        (size.x - MAP_WIDTH * scale) / 2.0,
        (size.y - MAP_HEIGHT * scale) / 2.0,
    );
    let place = |world: Vec2| -> Vec2 {
        let x = (world.x + MAP_WIDTH / 2.0) * scale + offset.x;
        let y = (MAP_HEIGHT / 2.0 - world.y) * scale + offset.y;
        Vec2::new(x, y)
    };

    let spawn_dot = |commands: &mut Commands, world: Vec2, color: Color, s: f32| {
        let p = place(world);
        commands.spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(p.x - s / 2.0),
                top: Val::Px(p.y - s / 2.0),
                width: Val::Px(s),
                height: Val::Px(s),
                ..default()
            },
            BackgroundColor(color),
            MinimapDot,
            ChildOf(map_entity),
        ));
    };

    for tf in &materials {
        spawn_dot(&mut commands, tf.translation.xy(), theme::TEXT_WEAK, DOT);
    }
    for tf in &weapons {
        spawn_dot(&mut commands, tf.translation.xy(), theme::SUCCESS, DOT);
    }
    for tf in &zombies {
        spawn_dot(&mut commands, tf.translation.xy(), theme::ERROR, DOT);
    }
    if let Ok(tf) = survivor_q.single() {
        spawn_dot(&mut commands, tf.translation.xy(), theme::ACCENT, DOT * 2.0);
    }
}
