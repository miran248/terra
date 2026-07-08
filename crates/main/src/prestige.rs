use bevy::prelude::*;
use shared::state::AppState;
use shared::theme;
use crate::ui::UiFont;
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
    font: Res<UiFont>,
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
            BackgroundColor(theme::PANEL_BG),
            GlobalZIndex(20),
            GameOverUi,
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new("GAME OVER"),
                TextFont {
                    font: font.0.clone().into(),
                    font_size: FontSize::Px(64.0),
                    ..default()
                },
                TextColor(theme::PRIMARY),
            ));
            parent.spawn((
                Text::new(format!(
                    "Survived to wave {} · Prestige level: {}",
                    wave.wave, prestige.0
                )),
                TextFont {
                    font: font.0.clone().into(),
                    font_size: FontSize::Px(28.0),
                    ..default()
                },
                TextColor(theme::INK),
            ));
            parent.spawn((
                Text::new("Press SPACE to restart with prestige bonus"),
                TextFont {
                    font: font.0.clone().into(),
                    font_size: FontSize::Px(20.0),
                    ..default()
                },
                TextColor(theme::TEXT_WEAK),
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
    loot_entities: Query<Entity, (Or<(With<crate::loot::LootMaterial>, With<crate::loot::LootWeapon>)>, Without<GameOverUi>)>,
    player_entities: Query<Entity, (With<crate::map::Player>, Without<GameOverUi>)>,
    ground_entities: Query<Entity, (With<crate::map::Ground>, Without<GameOverUi>)>,
    mut player_hp: ResMut<crate::map::PlayerHp>,
    mut scrap: ResMut<crate::combat::ScrapCounter>,
    mut loot: ResMut<crate::loot::LootState>,
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
    for entity in &loot_entities {
        commands.entity(entity).despawn();
    }
    for entity in &player_entities {
        commands.entity(entity).despawn();
    }
    for entity in &ground_entities {
        commands.entity(entity).despawn();
    }
    for entity in &game_over_ui {
        commands.entity(entity).despawn();
    }

    player_hp.0 = crate::constants::PLAYER_HP;
    scrap.0 = 0;
    *loot = crate::loot::LootState::default();
    *wave = WaveManager::default();
    levels.levels = [0; 13];
    prestige.0 += 1;

    next_state.set(AppState::Playing);
}
