use bevy::prelude::*;
use bevy::input::mouse::MouseWheel;
use shared::items::Recipe;
use shared::theme;
use shared::upgrades::Upgrade;
use crate::combat::ScrapCounter;
use crate::loot::LootState;
use crate::map::{Survivor, SurvivorHp};
use crate::wave::WaveManager;

#[derive(Resource, Default)]
pub struct UpgradeLevels {
    pub levels: [u32; Upgrade::ALL.len()],
}

#[derive(Resource)]
pub struct UiFont(pub Handle<Font>);

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

#[derive(Component)]
struct CraftButton {
    recipe: usize,
}

#[derive(Component)]
struct CraftStatusText;

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<UpgradeLevels>()
            .add_systems(Startup, (load_font, setup_sidebar, setup_crafting).chain())
            .add_systems(OnEnter(shared::state::AppState::Playing), reset_upgrade_buttons)
            .add_systems(Update, (
                update_stats,
                handle_upgrade_clicks,
                apply_upgrades,
                scroll_upgrades,
                handle_craft_clicks,
                update_craft_status,
            ));
    }
}

fn load_font(mut commands: Commands, assets: Res<AssetServer>) {
    commands.insert_resource(UiFont(assets.load(theme::FONT_PATH)));
}

fn text_font(font: &UiFont, size: f32) -> TextFont {
    TextFont { font: font.0.clone().into(), font_size: FontSize::Px(size), ..default() }
}

fn setup_sidebar(mut commands: Commands, font: Res<UiFont>) {
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
            BackgroundColor(theme::PANEL_BG),
            GlobalZIndex(10),
            Sidebar,
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new("Scrap: 0\nWave: 1\nHP: 500\n\nDPS: 0.0\nAPS: 0.0"),
                text_font(&font, 14.0),
                TextColor(theme::INK),
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
                BackgroundColor(theme::BORDER),
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
                            BorderColor::all(theme::PRIMARY),
                            BackgroundColor(theme::SURFACE),
                            ShopButton { upgrade: *upgrade },
                        )).with_child((
                            Text::new(label),
                            text_font(&font, 11.0),
                            TextColor(theme::INK),
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

fn setup_crafting(mut commands: Commands, font: Res<UiFont>) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(0.0),
                top: Val::Px(0.0),
                width: Val::Px(200.0),
                height: Val::Percent(100.0),
                display: Display::Flex,
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(Val::Px(8.0)),
                row_gap: Val::Px(4.0),
                ..default()
            },
            BackgroundColor(theme::PANEL_BG),
            GlobalZIndex(10),
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new("CRAFTING\nMetal 0  Wood 0\nRope 0  Cloth 0\nWeapon: none"),
                text_font(&font, 12.0),
                TextColor(theme::INK),
                Node { flex_shrink: 0.0, ..default() },
                CraftStatusText,
            ));

            parent.spawn((
                Node {
                    width: Val::Percent(100.0),
                    height: Val::Px(1.0),
                    flex_shrink: 0.0,
                    ..default()
                },
                BackgroundColor(theme::BORDER),
            ));

            for (i, recipe) in Recipe::ALL.iter().enumerate() {
                parent
                    .spawn((
                        Button,
                        Node {
                            padding: UiRect::all(Val::Px(6.0)),
                            flex_shrink: 0.0,
                            ..default()
                        },
                        BorderColor::all(theme::PRIMARY),
                        BackgroundColor(theme::SURFACE),
                        CraftButton { recipe: i },
                    ))
                    .with_child((
                        Text::new(recipe_label(recipe)),
                        text_font(&font, 11.0),
                        TextColor(theme::INK),
                    ));
            }
        });
}

fn recipe_label(recipe: &Recipe) -> String {
    let s = recipe.output.stats();
    let cost: Vec<String> = recipe
        .cost
        .iter()
        .map(|(m, n)| format!("{}x{}", n, m.name()))
        .collect();
    format!(
        "Craft {}\n{}\ndmg {:.0} rng {:.0} dur {}",
        recipe.output.name(),
        cost.join(" + "),
        s.damage,
        s.range,
        s.durability,
    )
}

fn handle_craft_clicks(
    interactions: Query<(&Interaction, &CraftButton), Changed<Interaction>>,
    mut loot: ResMut<LootState>,
) {
    for (interaction, button) in &interactions {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let recipe = Recipe::ALL[button.recipe];
        let affordable = recipe.cost.iter().all(|(m, n)| loot.count(*m) >= *n);
        if !affordable {
            continue;
        }
        for (m, n) in recipe.cost {
            loot.try_spend(m, n);
        }
        loot.weapons.push(recipe.output);
        if loot.equipped.is_none() {
            loot.equipped = Some((recipe.output, recipe.output.stats().durability));
        }
    }
}

fn update_craft_status(
    loot: Res<LootState>,
    mut q: Query<&mut Text, With<CraftStatusText>>,
) {
    if !loot.is_changed() {
        return;
    }
    let Ok(mut t) = q.single_mut() else { return };
    use shared::items::Material::*;
    let weapon = match loot.equipped {
        Some((kind, dur)) => format!("{} (dur {})", kind.name(), dur),
        None => "none".to_string(),
    };
    t.0 = format!(
        "CRAFTING\nMetal {}  Wood {}\nRope {}  Cloth {}\nWeapon: {}",
        loot.count(Metal),
        loot.count(Wood),
        loot.count(Rope),
        loot.count(Cloth),
        weapon,
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
