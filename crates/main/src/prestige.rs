use bevy::prelude::*;
use shared::state::AppState;
use crate::wave::WaveManager;

#[derive(Resource, Default)]
pub struct PrestigeLevel(pub u32);

pub struct PrestigePlugin;

impl Plugin for PrestigePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PrestigeLevel>()
            .add_systems(OnEnter(AppState::GameOver), show_game_over)
            .add_systems(Update, restart_game.run_if(in_state(AppState::GameOver)));
    }
}

#[derive(Component)]
struct GameOverUi;

fn show_game_over(
    mut commands: Commands,
    wave: Res<WaveManager>,
    prestige: Res<PrestigeLevel>,
) {
    commands
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                display: Display::Flex,
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                row_gap: Val::Px(20.0),
                ..default()
            },
            GlobalZIndex(20),
            GameOverUi,
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new("GAME OVER"),
                TextFont {
                    font_size: FontSize::Px(64.0),
                    ..default()
                },
                TextColor(Color::srgb(0.9, 0.2, 0.2)),
            ));
            parent.spawn((
                Text::new(format!(
                    "Survived to wave {} · Prestige level: {}",
                    wave.wave, prestige.0
                )),
                TextFont {
                    font_size: FontSize::Px(28.0),
                    ..default()
                },
                TextColor(Color::srgb(0.9, 0.9, 0.9)),
            ));
            parent.spawn((
                Text::new("Press SPACE to restart with prestige bonus"),
                TextFont {
                    font_size: FontSize::Px(20.0),
                    ..default()
                },
                TextColor(Color::srgb(0.7, 0.7, 0.7)),
            ));
        });
}

fn restart_game(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    mut next_state: ResMut<NextState<AppState>>,
    mut prestige: ResMut<PrestigeLevel>,
    game_over_ui: Query<Entity, With<GameOverUi>>,
    game_entities: Query<Entity, (With<crate::zombie::Zombie>, Without<GameOverUi>)>,
    projectile_entities: Query<Entity, (With<crate::turret::Projectile>, Without<GameOverUi>)>,
    scrap_entities: Query<Entity, (With<crate::combat::Scrap>, Without<GameOverUi>)>,
    survivor_entities: Query<Entity, (With<crate::map::Survivor>, Without<GameOverUi>)>,
    ground_entities: Query<Entity, (With<crate::map::Ground>, Without<GameOverUi>)>,
    mut survivor_hp: ResMut<crate::map::SurvivorHp>,
    mut scrap: ResMut<crate::combat::ScrapCounter>,
    mut wave: ResMut<crate::wave::WaveManager>,
    mut levels: ResMut<crate::ui::UpgradeLevels>,
) {
    if !keys.just_pressed(KeyCode::Space) {
        return;
    }

    for entity in &game_entities {
        commands.entity(entity).despawn();
    }
    for entity in &projectile_entities {
        commands.entity(entity).despawn();
    }
    for entity in &scrap_entities {
        commands.entity(entity).despawn();
    }
    for entity in &survivor_entities {
        commands.entity(entity).despawn();
    }
    for entity in &ground_entities {
        commands.entity(entity).despawn();
    }
    for entity in &game_over_ui {
        commands.entity(entity).despawn();
    }

    survivor_hp.0 = crate::constants::SURVIVOR_HP;
    scrap.0 = 0;
    *wave = WaveManager::default();
    levels.levels = [0; 13];
    prestige.0 += 1;

    next_state.set(AppState::Playing);
}
