use bevy::prelude::*;
use bevy::input::mouse::MouseWheel;
use shared::upgrades::Upgrade;
use crate::combat::ScrapCounter;
use crate::map::{Survivor, SurvivorHp};
use crate::wave::WaveManager;

#[derive(Resource, Default)]
pub struct UpgradeLevels {
    pub levels: [u32; Upgrade::ALL.len()],
}

#[derive(Component)]
struct Sidebar;

#[derive(Component)]
struct ScrollArea;

#[derive(Component)]
struct StatsText;

#[derive(Component)]
struct ShopButton {
    upgrade: Upgrade,
}

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<UpgradeLevels>()
            .add_systems(Startup, setup_sidebar)
            .add_systems(OnEnter(shared::state::AppState::Playing), reset_upgrade_buttons)
            .add_systems(Update, (
                update_stats,
                handle_upgrade_clicks,
                apply_upgrades,
                scroll_upgrades,
            ));
    }
}

fn setup_sidebar(mut commands: Commands) {
    let font = FontSize::Px(14.0);
    let small_font = FontSize::Px(11.0);

    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                top: Val::Px(0.0),
                width: Val::Px(200.0),
                height: Val::Percent(100.0),
                display: Display::Flex,
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(Val::Px(8.0)),
                row_gap: Val::Px(4.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.05, 0.05, 0.1, 0.9)),
            GlobalZIndex(10),
            Sidebar,
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new("Scrap: 0\nWave: 1\nHP: 500\n\nDPS: 0.0\nAPS: 0.0"),
                TextFont { font_size: font, ..default() },
                TextColor(Color::srgb(0.9, 0.9, 0.9)),
                Node { flex_shrink: 0.0, ..default() },
                StatsText,
            ));

            parent.spawn((
                Node {
                    width: Val::Percent(100.0),
                    height: Val::Px(1.0),
                    flex_shrink: 0.0,
                    ..default()
                },
                BackgroundColor(Color::srgb(0.3, 0.3, 0.3)),
            ));

            parent
                .spawn((
                    Node {
                        width: Val::Percent(100.0),
                        flex_grow: 1.0,
                        flex_shrink: 1.0,
                        min_height: Val::Px(0.0),
                        overflow: Overflow::scroll(),
                        display: Display::Flex,
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(4.0),
                        ..default()
                    },
                    ScrollPosition::default(),
                    ScrollArea,
                ))
                .with_children(|scroll| {
                    for upgrade in &Upgrade::ALL {
                        let lvl0 = upgrade.value(0);
                        let cur = upgrade.format_value(lvl0);
                        let nxt = upgrade.format_value(upgrade.value(1));
                        let label = format!(
                            "{} Lv.0\n{} -> {} (+{:.0}%)\nCost: {}",
                            upgrade.name(),
                            cur, nxt,
                            upgrade.value(1) / upgrade.value(0) * 100.0 - 100.0,
                            upgrade.cost(0),
                        );

                        scroll.spawn((
                            Button,
                            Node {
                                padding: UiRect::all(Val::Px(6.0)),
                                flex_shrink: 0.0,
                                ..default()
                            },
                            BorderColor::all(Color::srgb(0.3, 0.3, 0.4)),
                            BackgroundColor(Color::srgb(0.08, 0.08, 0.13)),
                            ShopButton { upgrade: *upgrade },
                        )).with_child((
                            Text::new(label),
                            TextFont { font_size: small_font, ..default() },
                            TextColor(Color::srgb(0.85, 0.85, 0.85)),
                        ));
                    }
                });
        });
}

fn update_stats(
    scrap: Res<ScrapCounter>,
    wave: Res<WaveManager>,
    hp: Res<SurvivorHp>,
    levels: Res<UpgradeLevels>,
    mut q: Query<&mut Text, With<StatsText>>,
) {
    let Ok(mut t) = q.single_mut() else { return };

    let dmg_idx = Upgrade::ALL.iter().position(|u| *u == Upgrade::TurretDamage).unwrap();
    let spd_idx = Upgrade::ALL.iter().position(|u| *u == Upgrade::TurretSpeed).unwrap();
    let dmg = Upgrade::TurretDamage.value(levels.levels[dmg_idx]);
    let aps = Upgrade::TurretSpeed.value(levels.levels[spd_idx]);
    let dps = dmg * aps;

    t.0 = format!(
        "Scrap: {}\nWave: {}  ({}/{})\nHP: {:.0}\nDPS: {:.1}\nAPS: {:.2}",
        scrap.0,
        wave.wave, wave.zombies_spawned_this_wave, wave.zombies_per_wave,
        hp.0,
        dps,
        aps,
    );
}

fn handle_upgrade_clicks(
    interactions: Query<(&Interaction, &ShopButton, &Children), Changed<Interaction>>,
    mut text_q: Query<&mut Text>,
    mut scrap: ResMut<ScrapCounter>,
    mut levels: ResMut<UpgradeLevels>,
    mut survivor_hp: ResMut<SurvivorHp>,
) {
    for (interaction, button, children) in &interactions {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let idx = Upgrade::ALL.iter().position(|u| *u == button.upgrade).unwrap();
        let level = levels.levels[idx];
        let cost = button.upgrade.cost(level);

        if scrap.0 < cost {
            continue;
        }

        scrap.0 -= cost;
        levels.levels[idx] += 1;
        let new_level = levels.levels[idx];

        if button.upgrade == Upgrade::WallHp {
            let val = button.upgrade.value(new_level);
            survivor_hp.0 = val;
        }

        for &child in children {
            if let Ok(mut t) = text_q.get_mut(child) {
                let cur = button.upgrade.format_value(button.upgrade.value(new_level));
                let (next_str, arrow) = if new_level < 99 {
                    (button.upgrade.format_value(button.upgrade.value(new_level + 1)), "->".to_string())
                } else {
                    ("MAX".to_string(), "".to_string())
                };
                let delta = if new_level < 99 {
                    format!("+{:.0}%", button.upgrade.value(new_level + 1) / button.upgrade.value(new_level) * 100.0 - 100.0)
                } else {
                    String::new()
                };
                t.0 = format!(
                    "{} Lv.{}\n{} {} {}\n{} Cost: {}",
                    button.upgrade.name(),
                    new_level,
                    cur,
                    arrow,
                    next_str,
                    delta,
                    button.upgrade.cost(new_level),
                );
            }
        }
    }
}

fn scroll_upgrades(
    mut commands: Commands,
    mut evr: bevy::ecs::message::MessageReader<MouseWheel>,
    scroll_q: Query<(Entity, &ScrollPosition, &ComputedNode), With<ScrollArea>>,
) {
    for ev in evr.read() {
        for (entity, scroll, computed) in &scroll_q {
            let max_scroll = (computed.size.y - 400.0).max(0.0);
            let new_y = (scroll.0.y - ev.y * 50.0).max(0.0).min(max_scroll);
            commands.entity(entity).insert(ScrollPosition(Vec2::new(0.0, new_y)));
        }
    }
}

fn reset_upgrade_buttons(
    buttons: Query<(&ShopButton, &Children)>,
    mut text_q: Query<&mut Text>,
) {
    for (button, children) in &buttons {
        let lvl0 = button.upgrade.value(0);
        let cur = button.upgrade.format_value(lvl0);
        let nxt = button.upgrade.format_value(button.upgrade.value(1));
        let delta = format!("+{:.0}%", button.upgrade.value(1) / button.upgrade.value(0) * 100.0 - 100.0);
        let label = format!(
            "{} Lv.0\n{} -> {} ({})\nCost: {}",
            button.upgrade.name(),
            cur, nxt, delta,
            button.upgrade.cost(0),
        );
        for &child in children {
            if let Ok(mut t) = text_q.get_mut(child) {
                *t = Text::new(label.clone());
            }
        }
    }
}

fn apply_upgrades(
    levels: Res<UpgradeLevels>,
    mut survivor_q: Query<&mut Survivor>,
    mut p_speed: ResMut<crate::turret::ProjectileSpeed>,
) {
    if !levels.is_changed() {
        return;
    }

    let dmg_idx = Upgrade::ALL.iter().position(|u| *u == Upgrade::TurretDamage).unwrap();
    let spd_idx = Upgrade::ALL.iter().position(|u| *u == Upgrade::TurretSpeed).unwrap();
    let rng_idx = Upgrade::ALL.iter().position(|u| *u == Upgrade::TurretRange).unwrap();
    let proj_idx = Upgrade::ALL.iter().position(|u| *u == Upgrade::ProjectileSpeed).unwrap();

    let damage = Upgrade::TurretDamage.value(levels.levels[dmg_idx]);
    let speed = Upgrade::TurretSpeed.value(levels.levels[spd_idx]);
    let range = Upgrade::TurretRange.value(levels.levels[rng_idx]);
    p_speed.0 = Upgrade::ProjectileSpeed.value(levels.levels[proj_idx]);

    for mut survivor in &mut survivor_q {
        survivor.damage = damage;
        survivor.range = range;
        survivor.fire_timer = Timer::from_seconds(1.0 / speed, TimerMode::Repeating);
    }
}
