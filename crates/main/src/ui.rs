use crate::combat::ScrapCounter;
use crate::loot::LootState;
use crate::map::{LevelFaceTypes, LevelRegions, LevelSlope, LevelTags, Player, PlayerHp};
use crate::wave::WaveManager;
use bevy::input::mouse::MouseWheel;
use bevy::prelude::*;
use shared::items::Recipe;
use shared::planet::PlanetMesh;
use shared::terrain::Terrain;
use shared::theme;
use shared::upgrades::Upgrade;

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
struct TerrainHud;

#[derive(Resource)]
struct TerrainHudTimer(Timer);

impl Default for TerrainHudTimer {
    fn default() -> Self {
        Self(Timer::from_seconds(0.5, TimerMode::Repeating))
    }
}

#[derive(Component)]
struct CraftStatusText;

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<UpgradeLevels>()
            .init_resource::<TerrainHudTimer>()
            .add_systems(Startup, (load_font, spawn_terrain_hud).chain())
            .add_systems(
                OnEnter(shared::state::AppState::Playing),
                reset_upgrade_buttons,
            )
            .add_systems(
                Update,
                (
                    update_stats,
                    update_terrain_hud,
                    handle_upgrade_clicks,
                    apply_upgrades,
                    scroll_upgrades,
                    handle_craft_clicks,
                    update_craft_status,
                ),
            );
    }
}

fn load_font(mut commands: Commands, assets: Res<AssetServer>) {
    commands.insert_resource(UiFont(assets.load(theme::FONT_PATH)));
}

fn text_font(font: &UiFont, size: f32) -> TextFont {
    TextFont {
        font: font.0.clone().into(),
        font_size: FontSize::Px(size),
        ..default()
    }
}

#[expect(dead_code, reason = "sidebar UI is intentionally dormant")]
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
                Node {
                    flex_shrink: 0.0,
                    ..default()
                },
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
                            cur,
                            nxt,
                            upgrade.value(1) / upgrade.value(0) * 100.0 - 100.0,
                            upgrade.cost(0),
                        );

                        scroll
                            .spawn((
                                Button,
                                Node {
                                    padding: UiRect::all(Val::Px(6.0)),
                                    flex_shrink: 0.0,
                                    ..default()
                                },
                                BorderColor::all(theme::PRIMARY),
                                BackgroundColor(theme::SURFACE),
                                ShopButton { upgrade: *upgrade },
                            ))
                            .with_child((
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
    hp: Res<PlayerHp>,
    levels: Res<UpgradeLevels>,
    mut q: Query<&mut Text, With<StatsText>>,
) {
    let Ok(mut t) = q.single_mut() else { return };

    let dmg_idx = Upgrade::ALL
        .iter()
        .position(|u| *u == Upgrade::TurretDamage)
        .unwrap();
    let spd_idx = Upgrade::ALL
        .iter()
        .position(|u| *u == Upgrade::TurretSpeed)
        .unwrap();
    let dmg = Upgrade::TurretDamage.value(levels.levels[dmg_idx]);
    let aps = Upgrade::TurretSpeed.value(levels.levels[spd_idx]);
    let dps = dmg * aps;

    t.0 = format!(
        "Scrap: {}\nWave: {}  ({}/{})\nHP: {:.0}\nDPS: {:.1}\nAPS: {:.2}",
        scrap.0, wave.wave, wave.zombies_spawned_this_wave, wave.zombies_per_wave, hp.0, dps, aps,
    );
}

#[expect(dead_code, reason = "crafting UI is intentionally dormant")]
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
                Node {
                    flex_shrink: 0.0,
                    ..default()
                },
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

fn update_craft_status(loot: Res<LootState>, mut q: Query<&mut Text, With<CraftStatusText>>) {
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
    mut player_hp: ResMut<PlayerHp>,
) {
    for (interaction, button, children) in &interactions {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let idx = Upgrade::ALL
            .iter()
            .position(|u| *u == button.upgrade)
            .unwrap();
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
            player_hp.0 = val;
        }

        for &child in children {
            if let Ok(mut t) = text_q.get_mut(child) {
                let cur = button.upgrade.format_value(button.upgrade.value(new_level));
                let (next_str, arrow) = if new_level < 99 {
                    (
                        button
                            .upgrade
                            .format_value(button.upgrade.value(new_level + 1)),
                        "->".to_string(),
                    )
                } else {
                    ("MAX".to_string(), "".to_string())
                };
                let delta = if new_level < 99 {
                    format!(
                        "+{:.0}%",
                        button.upgrade.value(new_level + 1) / button.upgrade.value(new_level)
                            * 100.0
                            - 100.0
                    )
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
            commands
                .entity(entity)
                .insert(ScrollPosition(Vec2::new(0.0, new_y)));
        }
    }
}

fn reset_upgrade_buttons(buttons: Query<(&ShopButton, &Children)>, mut text_q: Query<&mut Text>) {
    for (button, children) in &buttons {
        let lvl0 = button.upgrade.value(0);
        let cur = button.upgrade.format_value(lvl0);
        let nxt = button.upgrade.format_value(button.upgrade.value(1));
        let delta = format!(
            "+{:.0}%",
            button.upgrade.value(1) / button.upgrade.value(0) * 100.0 - 100.0
        );
        let label = format!(
            "{} Lv.0\n{} -> {} ({})\nCost: {}",
            button.upgrade.name(),
            cur,
            nxt,
            delta,
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
    mut player_q: Query<&mut Player>,
    mut p_speed: ResMut<crate::turret::ProjectileSpeed>,
) {
    if !levels.is_changed() {
        return;
    }

    let dmg_idx = Upgrade::ALL
        .iter()
        .position(|u| *u == Upgrade::TurretDamage)
        .unwrap();
    let spd_idx = Upgrade::ALL
        .iter()
        .position(|u| *u == Upgrade::TurretSpeed)
        .unwrap();
    let rng_idx = Upgrade::ALL
        .iter()
        .position(|u| *u == Upgrade::TurretRange)
        .unwrap();
    let proj_idx = Upgrade::ALL
        .iter()
        .position(|u| *u == Upgrade::ProjectileSpeed)
        .unwrap();

    let damage = Upgrade::TurretDamage.value(levels.levels[dmg_idx]);
    let speed = Upgrade::TurretSpeed.value(levels.levels[spd_idx]);
    let range = Upgrade::TurretRange.value(levels.levels[rng_idx]);
    p_speed.0 = Upgrade::ProjectileSpeed.value(levels.levels[proj_idx]);

    for mut player in &mut player_q {
        player.damage = damage;
        player.range = range;
        player.fire_timer = Timer::from_seconds(1.0 / speed, TimerMode::Repeating);
    }
}

// ---- terrain HUD ----

fn spawn_terrain_hud(mut commands: Commands, font: Res<UiFont>) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(8.0),
                top: Val::Px(8.0),
                padding: UiRect::all(Val::Px(6.0)),
                ..default()
            },
            BackgroundColor(theme::PANEL_BG),
            GlobalZIndex(10),
            TerrainHud,
        ))
        .with_child((Text::new(""), text_font(&font, 12.0), TextColor(theme::INK)));
}

#[allow(
    clippy::too_many_arguments,
    reason = "Bevy injects independent ECS system parameters"
)]
fn update_terrain_hud(
    player_q: Query<&Transform, With<Player>>,
    terrain: Option<Res<shared::terrain::TerrainGen>>,
    planet: Option<Res<PlanetMesh>>,
    tags: Option<Res<LevelTags>>,
    regions: Option<Res<LevelRegions>>,
    blends: Option<Res<crate::map::LevelBlends>>,
    face_types: Option<Res<LevelFaceTypes>>,
    slope_class: Option<Res<LevelSlope>>,
    water_depth: Option<Res<crate::map::LevelWaterDepth>>,
    landform_r: Option<Res<crate::map::LevelLandform>>,
    road_mat: Option<Res<crate::map::LevelRoadMaterial>>,
    hud_q: Query<&Children, With<TerrainHud>>,
    mut text_q: Query<(&mut Text, &mut TextColor)>,
    time: Res<Time>,
    // Combined into one tuple param to stay within Bevy's 16-param system limit.
    sky: (
        Option<Res<crate::map::TimeOfDay>>,
        Option<Res<crate::weather::Weather>>,
    ),
    mut timer: ResMut<TerrainHudTimer>,
) {
    let (tod, weather) = sky;
    let Ok(children) = hud_q.single() else { return };
    let Some(child) = children.first() else {
        return;
    };
    let Ok((mut text, mut color)) = text_q.get_mut(*child) else {
        return;
    };
    let Some(terrain) = terrain else {
        text.0.clear();
        return;
    };
    let Ok(tf) = player_q.single() else {
        text.0.clear();
        return;
    };

    if !timer.0.tick(time.delta()).just_finished() {
        return;
    }
    let pos = shared::sphere::SpherePos::new(tf.translation);
    // Read precomputed face type from level data — guaranteed to match terrain colors.
    let tile = if let (Some(planet), Some(ft)) = (planet.as_ref(), face_types.as_ref()) {
        planet
            .face_at(tf.translation.normalize())
            .map(|fi| ft.0.get(fi).copied().unwrap_or(Terrain::Plains))
            .unwrap_or(Terrain::Plains)
    } else {
        terrain.classify(pos)
    };
    let altitude = terrain.altitude(pos);
    let temp = terrain.temperature_at(pos);
    let _slope = terrain.slope(pos);
    let hab = terrain.is_habitable(pos);
    let landform: String = if let Some(planet) = planet.as_ref() {
        planet
            .face_at(tf.translation.normalize())
            .map(|fi| {
                if tile.is_water() {
                    water_depth
                        .as_ref()
                        .and_then(|wd| wd.0.get(fi).copied().flatten())
                        .map(|d| d.name().to_string())
                        .unwrap_or_default()
                } else {
                    let lf = landform_r
                        .as_ref()
                        .and_then(|l| l.0.get(fi).copied())
                        .map(shared::level::Landform::name)
                        .unwrap_or("");
                    let sl = slope_class
                        .as_ref()
                        .and_then(|sc| sc.0.get(fi).copied())
                        .map(shared::level::SlopeClass::name)
                        .unwrap_or("");
                    format!("{lf} ({sl})")
                }
            })
            .unwrap_or_default()
    } else {
        String::new()
    };

    // Tile line: everything about the ground under the player, in one place —
    // type (both types when the face is a blend), built tags, habitability.
    let tile_name = |terrain: Terrain| match terrain {
        Terrain::RiverSpring => "River Spring".to_string(),
        _ => format!("{terrain:?}"),
    };
    let mut tile_line = tile_name(tile);
    let mut region_name = String::new();
    if let Some(planet) = planet
        && let Some(fi) = planet.face_at(tf.translation.normalize())
    {
        if let Some(blends) = blends
            && let Some(&(base, target)) = blends.0.get(&(fi as u32))
        {
            let other = match target {
                shared::level::BlendTarget::Terrain(other) if other == tile => base,
                shared::level::BlendTarget::Terrain(other) => other,
                _ => base,
            };
            tile_line = match target.name() {
                Some(name) => format!("{} + {name}", tile_name(tile)),
                None => format!("{} + {}", tile_name(tile), tile_name(other)),
            };
        }
        if let Some(tags) = tags {
            for &tag in &tags.0[fi] {
                tile_line.push_str("  ");
                tile_line.push_str(tag.name());
                if tag == shared::level::FaceTag::Road
                    && let Some(m) = road_mat
                        .as_ref()
                        .and_then(|r| r.0.get(fi).copied().flatten())
                {
                    tile_line.push_str(" (");
                    tile_line.push_str(m.name());
                    tile_line.push(')');
                }
            }
        }
        if let Some(regions) = regions
            && let Some(ri) = regions
                .face_region
                .get(fi)
                .copied()
                .flatten()
                .map(|index| index as usize)
        {
            region_name = format!("\n{}", regions.regions[ri].name);
        }
    }
    if hab {
        tile_line.push_str("  Habitable");
    }

    let sky_line = format!(
        "\n{}  {}",
        clock_string(tod.as_deref(), tf.translation.normalize()),
        weather_label(weather.as_deref(), temp),
    );

    *text = Text::new(format!(
        "{tile_line}{region_name}\nAlt: {altitude:.0}m  {landform}  Temp: {temp:.0}°C{sky_line}",
    ));
    *color = TextColor(hud_tile_color(tile));
}

/// Local solar clock at the player's position: derived from where the sun is
/// relative to this spot, so it differs across the planet (no timezones).
fn clock_string(tod: Option<&crate::map::TimeOfDay>, up: Vec3) -> String {
    let Some(tod) = tod else { return String::new() };
    let hours = tod.local_hours(up);
    let h = hours.floor() as u32;
    let m = ((hours - hours.floor()) * 60.0).floor() as u32;
    format!("Day {}  {h:02}:{m:02}", tod.day)
}

/// Weather label for the player's location: precipitation type is resolved from
/// the local temperature (snow when cold, rain when warm).
fn weather_label(weather: Option<&crate::weather::Weather>, temp: f32) -> String {
    let Some(weather) = weather else {
        return String::new();
    };
    if weather.precip < 0.05 {
        return "Clear".to_string();
    }
    let kind = if crate::weather::is_snow(temp) {
        "Snow"
    } else {
        "Rain"
    };
    let sev = if weather.precip > 0.66 {
        "Heavy "
    } else if weather.precip > 0.33 {
        ""
    } else {
        "Light "
    };
    format!("{sev}{kind}")
}

fn hud_tile_color(tile: shared::terrain::Terrain) -> Color {
    match tile {
        shared::terrain::Terrain::Ocean
        | shared::terrain::Terrain::Lake
        | shared::terrain::Terrain::SaltLake
        | shared::terrain::Terrain::River
        | shared::terrain::Terrain::RiverSpring => Color::srgb(0.2, 0.5, 1.0),
        shared::terrain::Terrain::FrozenLake => Color::srgb(0.68, 0.86, 0.94),
        shared::terrain::Terrain::Beach
        | shared::terrain::Terrain::Cliff
        | shared::terrain::Terrain::LakeShore
        | shared::terrain::Terrain::RiverBank => Color::srgb(0.9, 0.85, 0.6),
        shared::terrain::Terrain::Desert => Color::srgb(0.85, 0.75, 0.5),
        shared::terrain::Terrain::Plains | shared::terrain::Terrain::Forest => {
            Color::srgb(0.3, 0.7, 0.3)
        }
        shared::terrain::Terrain::Tundra => Color::srgb(0.6, 0.65, 0.6),
        shared::terrain::Terrain::Mountain => Color::srgb(0.5, 0.45, 0.4),
        shared::terrain::Terrain::Snow => Color::srgb(0.95, 0.97, 1.0),
        shared::terrain::Terrain::Swamp => Color::srgb(0.45, 0.55, 0.35),
        shared::terrain::Terrain::Jungle => Color::srgb(0.2, 0.65, 0.25),
        shared::terrain::Terrain::Savanna => Color::srgb(0.8, 0.75, 0.4),
        shared::terrain::Terrain::Volcanic => Color::srgb(0.6, 0.4, 0.35),
        shared::terrain::Terrain::Glacier => Color::srgb(0.85, 0.92, 1.0),
    }
}
