// use crate::combat::ScrapCounter;
// use crate::loot::LootState;
use crate::map::{
    LevelFaceCornerTypes, LevelFaceTypes, LevelRegions, LevelSlope, LevelTags, Player,
};
// use crate::wave::WaveManager;
use bevy::color::Alpha;
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::prelude::*;
use bevy::ui::FocusPolicy;
use bevy::window::PrimaryWindow;
use shared::items::Recipe;
use shared::level::RegionKind;
use terra_geometry::planet::PlanetMesh;
use shared::terrain::Terrain;
use shared::theme;
use shared::upgrades::Upgrade;

const MAX_REGION_ROW_CHARS: usize = 56;
const SIDEBAR_WIDTH: f32 = 280.0;
const SIDEBAR_SCROLL_STEP: f32 = 48.0;

#[derive(Resource, Default)]
pub struct UpgradeLevels {
    pub levels: [u32; Upgrade::ALL.len()],
}

#[derive(Resource)]
pub struct UiFont(pub Handle<Font>);

#[derive(Component)]
pub(crate) struct Sidebar;

#[derive(Component)]
pub(crate) struct SidebarScrollArea;

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SidebarSection {
    Environment,
    Location,
    Movement,
    View,
    Context,
    Actions,
}

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SidebarReadoutSlot {
    Clock,
    Weather,
    Temperature,
    Terrain,
    Elevation,
    Settlement,
    Region,
    Road,
    Movement,
    View,
    Follow,
    VehicleAction,
    SummonVehicleAction,
    TeleportAction,
    RecoveryContext,
    RecoveryAction,
    SelectorChoice,
}

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SidebarAction {
    TogglePlanetView,
    ToggleFollow,
    Interact,
    Teleport,
    ToggleVehicleSelector,
    SelectVehicle(crate::exploration::Kind),
    CancelVehicleSelector,
    HoldRecovery,
}

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SidebarActionControl(pub(crate) SidebarAction);

#[derive(Component)]
pub(crate) struct SidebarWorldReadout;

#[derive(Component)]
pub(crate) struct SidebarExplorationReadout;

type SidebarWorldRows<'w, 's> = Query<
    'w,
    's,
    (
        &'static mut Text,
        &'static mut TextColor,
        &'static SidebarReadoutSlot,
    ),
    (
        With<SidebarWorldReadout>,
        Without<SidebarExplorationReadout>,
    ),
>;

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

#[derive(Resource)]
struct TerrainHudTimer(Timer);

impl Default for TerrainHudTimer {
    fn default() -> Self {
        Self(Timer::from_seconds(0.05, TimerMode::Repeating))
    }
}

#[derive(Component)]
struct CraftStatusText;

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<UpgradeLevels>()
            .init_resource::<TerrainHudTimer>()
            .add_systems(Startup, load_font)
            .add_systems(
                OnEnter(shared::state::AppState::Playing),
                reset_upgrade_buttons,
            )
            .add_systems(
                PreUpdate,
                scroll_sidebar
                    .after(bevy::input::InputSystems)
                    .before(crate::exploration::ExplorationInput)
                    .run_if(in_state(shared::state::AppState::Playing)),
            )
            .add_systems(
                Update,
                (
                    // update_stats,
                    update_sidebar_world_readout,
                    // handle_upgrade_clicks,
                    // apply_upgrades,
                    // handle_craft_clicks,
                    // update_craft_status,
                )
                    .run_if(in_state(shared::state::AppState::Playing)),
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

pub(crate) fn spawn_sidebar(commands: &mut Commands, font: &UiFont) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                top: Val::Px(0.0),
                width: Val::Px(SIDEBAR_WIDTH),
                height: Val::Percent(100.0),
                display: Display::Flex,
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(Val::Px(10.0)),
                ..default()
            },
            BackgroundColor(theme::PANEL_BG.with_alpha(0.84)),
            GlobalZIndex(100),
            FocusPolicy::Block,
            Sidebar,
        ))
        .with_children(|parent| {
            parent
                .spawn((
                    Node {
                        width: Val::Percent(100.0),
                        height: Val::Percent(100.0),
                        min_height: Val::Px(0.0),
                        flex_grow: 1.0,
                        flex_shrink: 1.0,
                        overflow: Overflow::scroll(),
                        display: Display::Flex,
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(12.0),
                        ..default()
                    },
                    ScrollPosition::default(),
                    SidebarScrollArea,
                ))
                .with_children(|scroll| {
                    scroll
                        .spawn(sidebar_section(SidebarSection::Environment))
                        .with_children(|section| {
                            section.spawn(sidebar_title("ENVIRONMENT", font));
                            section.spawn(sidebar_world_row(
                                "Clock: —",
                                SidebarReadoutSlot::Clock,
                                font,
                            ));
                            section.spawn(sidebar_world_row(
                                "Weather: —",
                                SidebarReadoutSlot::Weather,
                                font,
                            ));
                            section.spawn(sidebar_world_row(
                                "Temperature: —",
                                SidebarReadoutSlot::Temperature,
                                font,
                            ));
                        });

                    scroll
                        .spawn(sidebar_section(SidebarSection::Location))
                        .with_children(|section| {
                            section.spawn(sidebar_title("LOCATION", font));
                            section.spawn(sidebar_world_row(
                                "Terrain: —",
                                SidebarReadoutSlot::Terrain,
                                font,
                            ));
                            section.spawn(sidebar_world_row(
                                "Elevation: —",
                                SidebarReadoutSlot::Elevation,
                                font,
                            ));
                            section.spawn(sidebar_world_row(
                                "Settlement: —",
                                SidebarReadoutSlot::Settlement,
                                font,
                            ));
                            section.spawn(sidebar_world_row(
                                "Region: —",
                                SidebarReadoutSlot::Region,
                                font,
                            ));
                            section.spawn(sidebar_world_row(
                                "Road / bridge: —",
                                SidebarReadoutSlot::Road,
                                font,
                            ));
                        });

                    scroll
                        .spawn(sidebar_section(SidebarSection::Movement))
                        .with_children(|section| {
                            section.spawn(sidebar_title("MOVEMENT", font));
                            section.spawn(sidebar_exploration_row(
                                "Travel mode: On foot",
                                SidebarReadoutSlot::Movement,
                                font,
                            ));
                        });

                    scroll
                        .spawn(sidebar_section(SidebarSection::View))
                        .with_children(|section| {
                            section.spawn(sidebar_title("VIEW", font));
                            section.spawn(sidebar_action_row(
                                "Open Planet view · M",
                                SidebarReadoutSlot::View,
                                SidebarAction::TogglePlanetView,
                                font,
                            ));
                        });

                    scroll
                        .spawn(sidebar_section(SidebarSection::Context))
                        .with_children(|section| {
                            section.spawn(sidebar_title("CONTEXT", font));
                        });

                    scroll
                        .spawn(sidebar_section(SidebarSection::Actions))
                        .with_children(|section| {
                            section.spawn(sidebar_title("ACTIONS", font));
                            section.spawn(sidebar_action_row(
                                "Enter vehicle · E",
                                SidebarReadoutSlot::VehicleAction,
                                SidebarAction::Interact,
                                font,
                            ));
                            section.spawn(sidebar_action_row(
                                "Summon vehicle · V",
                                SidebarReadoutSlot::SummonVehicleAction,
                                SidebarAction::ToggleVehicleSelector,
                                font,
                            ));
                            section.spawn(sidebar_action_row(
                                "Teleport to selected destination · T",
                                SidebarReadoutSlot::TeleportAction,
                                SidebarAction::Teleport,
                                font,
                            ));
                            section.spawn(sidebar_action_row(
                                "Recover · Hold 1s (R)",
                                SidebarReadoutSlot::RecoveryAction,
                                SidebarAction::HoldRecovery,
                                font,
                            ));
                        });
                });
        });
}

fn sidebar_section(section: SidebarSection) -> (Node, SidebarSection) {
    (
        Node {
            width: Val::Percent(100.0),
            display: Display::Flex,
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(4.0),
            flex_shrink: 0.0,
            ..default()
        },
        section,
    )
}

fn sidebar_title(title: &str, font: &UiFont) -> impl Bundle {
    (
        Text::new(title),
        text_font(font, 12.0),
        TextColor(theme::ACCENT),
        Node {
            flex_shrink: 0.0,
            ..default()
        },
    )
}

fn sidebar_world_row(text: &str, slot: SidebarReadoutSlot, font: &UiFont) -> impl Bundle {
    (
        Text::new(text),
        text_font(font, 12.0),
        TextColor(theme::INK),
        Node {
            width: Val::Percent(100.0),
            flex_shrink: 0.0,
            ..default()
        },
        slot,
        SidebarWorldReadout,
    )
}

pub(crate) fn sidebar_exploration_row(
    text: &str,
    slot: SidebarReadoutSlot,
    font: &UiFont,
) -> impl Bundle {
    (
        Text::new(text),
        text_font(font, 12.0),
        TextColor(theme::INK),
        Node {
            width: Val::Percent(100.0),
            flex_shrink: 0.0,
            ..default()
        },
        slot,
        SidebarExplorationReadout,
    )
}

pub(crate) fn sidebar_action_row(
    text: &str,
    slot: SidebarReadoutSlot,
    action: SidebarAction,
    font: &UiFont,
) -> impl Bundle {
    (
        Text::new(text),
        text_font(font, 12.0),
        TextColor(theme::INK),
        Node {
            width: Val::Percent(100.0),
            flex_shrink: 0.0,
            ..default()
        },
        slot,
        SidebarExplorationReadout,
        SidebarActionControl(action),
    )
}

// ponytail: module refs disabled
// fn update_stats(
//     scrap: Res<ScrapCounter>,
//     wave: Res<WaveManager>,
//     hp: Res<PlayerHp>,
//     levels: Res<UpgradeLevels>,
//     mut q: Query<&mut Text, With<StatsText>>,
// ) {
//     let Ok(mut t) = q.single_mut() else { return };
//
//     let dmg_idx = Upgrade::ALL
//         .iter()
//         .position(|u| *u == Upgrade::TurretDamage)
//         .unwrap();
//     let spd_idx = Upgrade::ALL
//         .iter()
//         .position(|u| *u == Upgrade::TurretSpeed)
//         .unwrap();
//     let dmg = Upgrade::TurretDamage.value(levels.levels[dmg_idx]);
//     let aps = Upgrade::TurretSpeed.value(levels.levels[spd_idx]);
//     let dps = dmg * aps;
//
//     t.0 = format!(
//         "Scrap: {}\nWave: {}  ({}/{})\nHP: {:.0}\nDPS: {:.1}\nAPS: {:.2}",
//         scrap.0, wave.wave, wave.zombies_spawned_this_wave, wave.zombies_per_wave, hp.0, dps, aps,
//     );
// }

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

// ponytail: module refs disabled
// fn handle_craft_clicks(
//     interactions: Query<(&Interaction, &CraftButton), Changed<Interaction>>,
//     mut loot: ResMut<LootState>,
// ) {
//     for (interaction, button) in &interactions {
//         if *interaction != Interaction::Pressed {
//             continue;
//         }
//         let recipe = Recipe::ALL[button.recipe];
//         let affordable = recipe.cost.iter().all(|(m, n)| loot.count(*m) >= *n);
//         if !affordable {
//             continue;
//         }
//         for (m, n) in recipe.cost {
//             loot.try_spend(m, n);
//         }
//         loot.weapons.push(recipe.output);
//         if loot.equipped.is_none() {
//             loot.equipped = Some((recipe.output, recipe.output.stats().durability));
//         }
//     }
// }
//
// fn update_craft_status(loot: Res<LootState>, mut q: Query<&mut Text, With<CraftStatusText>>) {
//     if !loot.is_changed() {
//         return;
//     }
//     let Ok(mut t) = q.single_mut() else { return };
//     use shared::items::Material::*;
//     let weapon = match loot.equipped {
//         Some((kind, dur)) => format!("{} (dur {})", kind.name(), dur),
//         None => "none".to_string(),
//     };
//     t.0 = format!(
//         "CRAFTING\nMetal {}  Wood {}\nRope {}  Cloth {}\nWeapon: {}",
//         loot.count(Metal),
//         loot.count(Wood),
//         loot.count(Rope),
//         loot.count(Cloth),
//         weapon,
//     );
// }

// ponytail: module refs disabled
// fn handle_upgrade_clicks(
//     interactions: Query<(&Interaction, &ShopButton, &Children), Changed<Interaction>>,
//     mut text_q: Query<&mut Text>,
//     mut scrap: ResMut<ScrapCounter>,
//     mut levels: ResMut<UpgradeLevels>,
//     mut player_hp: ResMut<PlayerHp>,
// ) {
//     for (interaction, button, children) in &interactions {
//         if *interaction != Interaction::Pressed {
//             continue;
//         }
//         let idx = Upgrade::ALL
//             .iter()
//             .position(|u| *u == button.upgrade)
//             .unwrap();
//         let level = levels.levels[idx];
//         let cost = button.upgrade.cost(level);
//
//         if scrap.0 < cost {
//             continue;
//         }
//
//         scrap.0 -= cost;
//         levels.levels[idx] += 1;
//         let new_level = levels.levels[idx];
//
//         if button.upgrade == Upgrade::WallHp {
//             let val = button.upgrade.value(new_level);
//             player_hp.0 = val;
//         }
//
//         for &child in children {
//             if let Ok(mut t) = text_q.get_mut(child) {
//                 let cur = button.upgrade.format_value(button.upgrade.value(new_level));
//                 let (next_str, arrow) = if new_level < 99 {
//                     (
//                         button
//                             .upgrade
//                             .format_value(button.upgrade.value(new_level + 1)),
//                         "->".to_string(),
//                     )
//                 } else {
//                     ("MAX".to_string(), "".to_string())
//                 };
//                 let delta = if new_level < 99 {
//                     format!(
//                         "+{:.0}%",
//                         button.upgrade.value(new_level + 1) / button.upgrade.value(new_level)
//                             * 100.0
//                             - 100.0
//                     )
//                 } else {
//                     String::new()
//                 };
//                 t.0 = format!(
//                     "{} Lv.{}\n{} {} {}\n{} Cost: {}",
//                     button.upgrade.name(),
//                     new_level,
//                     cur,
//                     arrow,
//                     next_str,
//                     delta,
//                     button.upgrade.cost(new_level),
//                 );
//             }
//         }
//     }
// }

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

// ponytail: module refs disabled
// fn apply_upgrades(
//     levels: Res<UpgradeLevels>,
//     mut player_q: Query<&mut Player>,
//     mut p_speed: ResMut<crate::turret::ProjectileSpeed>,
// ) {
//     if !levels.is_changed() {
//         return;
//     }
//
//     let dmg_idx = Upgrade::ALL
//         .iter()
//         .position(|u| *u == Upgrade::TurretDamage)
//         .unwrap();
//     let spd_idx = Upgrade::ALL
//         .iter()
//         .position(|u| *u == Upgrade::TurretSpeed)
//         .unwrap();
//     let rng_idx = Upgrade::ALL
//         .iter()
//         .position(|u| *u == Upgrade::TurretRange)
//         .unwrap();
//     let proj_idx = Upgrade::ALL
//         .iter()
//         .position(|u| *u == Upgrade::ProjectileSpeed)
//         .unwrap();
//
//     let damage = Upgrade::TurretDamage.value(levels.levels[dmg_idx]);
//     let speed = Upgrade::TurretSpeed.value(levels.levels[spd_idx]);
//     let range = Upgrade::TurretRange.value(levels.levels[rng_idx]);
//     p_speed.0 = Upgrade::ProjectileSpeed.value(levels.levels[proj_idx]);
//
//     for mut player in &mut player_q {
//         player.damage = damage;
//         player.range = range;
//         player.fire_timer = Timer::from_seconds(1.0 / speed, TimerMode::Repeating);
//     }
// }

// ---- shared gameplay and Planet view sidebar ----

pub(crate) fn scroll_sidebar(
    mut wheels: bevy::ecs::message::MessageReader<MouseWheel>,
    windows: Query<&Window, With<PrimaryWindow>>,
    sidebar: Query<(&ComputedNode, &UiGlobalTransform), With<Sidebar>>,
    mut scroll_areas: Query<(&mut ScrollPosition, &ComputedNode), With<SidebarScrollArea>>,
    exploration: Option<Res<crate::exploration::Exploration>>,
) {
    let scroll_delta = wheels
        .read()
        .map(|wheel| match wheel.unit {
            MouseScrollUnit::Line => wheel.y * SIDEBAR_SCROLL_STEP,
            MouseScrollUnit::Pixel => wheel.y,
        })
        .sum::<f32>();
    if scroll_delta == 0.0 || exploration.is_some_and(|state| state.is_vehicle_selector_open()) {
        return;
    }
    let Some(cursor) = windows
        .single()
        .ok()
        .and_then(Window::physical_cursor_position)
    else {
        return;
    };
    if !sidebar
        .iter()
        .any(|(node, transform)| node.contains_point(*transform, cursor))
    {
        return;
    }

    for (mut position, node) in &mut scroll_areas {
        let max_scroll = (node.content_size().y - node.size().y).max(0.0);
        position.0.y = (position.0.y - scroll_delta).clamp(0.0, max_scroll);
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "Bevy injects independent ECS system parameters"
)]
fn update_sidebar_world_readout(
    player_q: Query<&Transform, With<Player>>,
    terrain: Option<Res<shared::terrain::TerrainGen>>,
    planet: Option<Res<PlanetMesh>>,
    tags: Option<Res<LevelTags>>,
    regions: Option<Res<LevelRegions>>,
    blends: Option<Res<crate::map::LevelBlends>>,
    terrain_faces: (
        Option<Res<LevelFaceTypes>>,
        Option<Res<LevelFaceCornerTypes>>,
    ),
    slope_class: Option<Res<LevelSlope>>,
    water: (
        Option<Res<crate::map::LevelWaterDepth>>,
        Option<Res<crate::map::LevelWaterPhase>>,
    ),
    landform_r: Option<Res<crate::map::LevelLandform>>,
    road_mat: Option<Res<crate::map::LevelRoadMaterial>>,
    time: Res<Time<Real>>,
    mut text_q: SidebarWorldRows<'_, '_>,
    // Combined into one tuple param to stay within Bevy's 16-param system limit.
    sky: (
        Option<Res<crate::map::TimeOfDay>>,
        Option<Res<crate::weather::Weather>>,
    ),
    mut timer: ResMut<TerrainHudTimer>,
) {
    let (tod, weather) = sky;
    let (water_depth, water_phase) = water;
    let (face_types, face_corner_types) = terrain_faces;
    if !timer.0.tick(time.delta()).just_finished() {
        return;
    }
    let empty_values = [
        (SidebarReadoutSlot::Clock, "Clock: —".to_owned()),
        (SidebarReadoutSlot::Weather, "Weather: —".to_owned()),
        (SidebarReadoutSlot::Temperature, "Temperature: —".to_owned()),
        (SidebarReadoutSlot::Terrain, "Terrain: —".to_owned()),
        (SidebarReadoutSlot::Elevation, "Elevation: —".to_owned()),
        (SidebarReadoutSlot::Settlement, "Settlement: —".to_owned()),
        (SidebarReadoutSlot::Region, "Region: —".to_owned()),
        (SidebarReadoutSlot::Road, "Road / bridge: —".to_owned()),
    ];
    let Some(terrain) = terrain else {
        write_sidebar_world_rows(&mut text_q, &empty_values, Color::WHITE);
        return;
    };
    let Ok(tf) = player_q.single() else {
        write_sidebar_world_rows(&mut text_q, &empty_values, Color::WHITE);
        return;
    };
    let mut values = Vec::new();
    let pos = terra_geometry::sphere::SpherePos::new(tf.translation);
    // Read precomputed face type from level data — guaranteed to match terrain colors.
    let tile = if let (Some(planet), Some(ft), Some(corners)) = (
        planet.as_ref(),
        face_types.as_ref(),
        face_corner_types.as_ref(),
    ) {
        let direction = tf.translation.normalize();
        planet.face_at(direction).map_or(Terrain::Plains, |face| {
            let fallback = ft.0.get(face).copied().unwrap_or(Terrain::Plains);
            let Some(triangle) = planet.triangle(face) else {
                return fallback;
            };
            let Some(types) = corners.0.get(face) else {
                return fallback;
            };
            let nearest = (0..3)
                .max_by(|&a, &b| {
                    triangle[a]
                        .dot(direction)
                        .total_cmp(&triangle[b].dot(direction))
                })
                .unwrap();
            types[nearest]
        })
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
                    let depth = water_depth
                        .as_ref()
                        .and_then(|wd| wd.0.get(fi).copied().flatten())
                        .map(|d| d.name().to_string())
                        .unwrap_or_default();
                    if water_phase
                        .as_ref()
                        .and_then(|phase| phase.0.get(fi).copied().flatten())
                        == Some(shared::level::WaterPhase::Frozen)
                    {
                        format!("Frozen {depth}")
                    } else {
                        depth
                    }
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
    let mut location_values = [
        (SidebarReadoutSlot::Settlement, "Settlement: —".to_owned()),
        (SidebarReadoutSlot::Region, "Region: —".to_owned()),
        (SidebarReadoutSlot::Road, "Road / bridge: —".to_owned()),
    ];
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
        if let Some(tags) = tags
            && let Some(face_tags) = tags.0.get(fi)
        {
            for &tag in face_tags {
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
        if let Some(regions) = regions {
            for line in format_region_lines(fi, &regions, tf.translation) {
                let row = if let Some(value) = line.strip_prefix("Geography: ") {
                    Some((1, "Region", value))
                } else if let Some(value) = line.strip_prefix("Settlement: ") {
                    Some((0, "Settlement", value))
                } else if let Some(value) = line.strip_prefix("Roads: ") {
                    Some((2, "Road / bridge", value))
                } else if let Some(value) = line.strip_prefix("Bridge: ") {
                    let previous = location_values[2]
                        .1
                        .strip_prefix("Road / bridge: ")
                        .unwrap();
                    let combined = if previous == "—" {
                        format!("Bridge: {value}")
                    } else {
                        format!("{previous} · Bridge: {value}")
                    };
                    location_values[2].1 = truncate_region_row(
                        &format!("Road / bridge: {combined}"),
                        MAX_REGION_ROW_CHARS,
                    );
                    None
                } else {
                    None
                };
                if let Some((index, label, value)) = row {
                    location_values[index].1 =
                        truncate_region_row(&format!("{label}: {value}"), MAX_REGION_ROW_CHARS);
                }
            }
        }
    }
    if hab {
        tile_line.push_str("  Habitable");
    }

    let clock = clock_string(tod.as_deref(), tf.translation.normalize());
    let weather = weather_label(weather.as_deref(), temp);
    values.extend([
        (
            SidebarReadoutSlot::Clock,
            format!("Clock: {}", if clock.is_empty() { "—" } else { &clock }),
        ),
        (
            SidebarReadoutSlot::Weather,
            format!(
                "Weather: {}",
                if weather.is_empty() { "—" } else { &weather }
            ),
        ),
        (
            SidebarReadoutSlot::Temperature,
            format!("Temperature: {temp:.0} °C"),
        ),
        (SidebarReadoutSlot::Terrain, format!("Terrain: {tile_line}")),
        (
            SidebarReadoutSlot::Elevation,
            format!("Elevation: {altitude:.0} m · {landform}"),
        ),
    ]);
    values.extend(location_values);
    write_sidebar_world_rows(&mut text_q, &values, hud_tile_color(tile));
}

fn write_sidebar_world_rows(
    rows: &mut SidebarWorldRows<'_, '_>,
    values: &[(SidebarReadoutSlot, String)],
    terrain_color: Color,
) {
    for (mut text, mut color, slot) in rows.iter_mut() {
        if let Some((_, value)) = values.iter().find(|(candidate, _)| candidate == slot) {
            if text.0 != *value {
                *text = Text::new(value.clone());
            }
            if *slot == SidebarReadoutSlot::Terrain {
                *color = TextColor(terrain_color);
            }
        }
    }
}

fn format_region_lines(face: usize, regions: &LevelRegions, player_position: Vec3) -> Vec<String> {
    let mut geography = Vec::new();
    let mut settlements = Vec::new();
    let mut roads = Vec::new();

    for &region_id in regions.face_regions.region_ids_at(face) {
        let Some(region) = regions.regions.get(region_id as usize) else {
            continue;
        };
        match region.kind {
            RegionKind::Settlement => {
                let kind = regions
                    .settlements
                    .iter()
                    .find(|(name, _)| name == &region.name)
                    .map(|(_, kind)| kind.name());
                settlements.push(match kind {
                    Some(kind) => format!("{} ({kind})", region.name),
                    None => region.name.clone(),
                });
            }
            RegionKind::Road => roads.push(region.name.clone()),
            kind => geography.push((kind.rank(), region.name.clone())),
        }
    }

    geography.sort_unstable();
    settlements.sort_unstable();
    roads.sort_unstable();
    let mut lines = Vec::new();
    if !geography.is_empty() {
        lines.push(format!(
            "Geography: {}",
            geography
                .into_iter()
                .map(|(_, name)| name)
                .collect::<Vec<_>>()
                .join(" · ")
        ));
    }
    if !settlements.is_empty() {
        lines.push(format!("Settlement: {}", settlements.join(" · ")));
    }
    if !roads.is_empty() {
        lines.push(format!("Roads: {}", roads.join(" · ")));
    }
    let bridges = regions.bridge_names_at_position(player_position);
    if !bridges.is_empty() {
        lines.push(format!("Bridge: {}", bridges.join(" · ")));
    }
    lines
}

fn truncate_region_row(row: &str, max_chars: usize) -> String {
    if row.chars().count() <= max_chars {
        return row.to_owned();
    }
    let mut truncated = row
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>()
        .trim_end()
        .to_owned();
    truncated.push('…');
    truncated
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

#[cfg(test)]
mod tests {
    use super::*;
    use shared::level::{RegionData, RegionMemberships, SettlementKind};

    #[test]
    fn region_hud_groups_all_memberships_and_names_settlement_kind() {
        let regions = LevelRegions {
            regions: vec![
                RegionData {
                    name: "Elder Forest".into(),
                    pos: [0.0, 1.0, 0.0],
                    kind: RegionKind::Forest,
                },
                RegionData {
                    name: "Grey Range".into(),
                    pos: [0.0, 1.0, 0.0],
                    kind: RegionKind::MountainRange,
                },
                RegionData {
                    name: "Ashford".into(),
                    pos: [0.0, 1.0, 0.0],
                    kind: RegionKind::Settlement,
                },
                RegionData {
                    name: "King's Road".into(),
                    pos: [0.0, 1.0, 0.0],
                    kind: RegionKind::Road,
                },
                RegionData {
                    name: "Salt Road".into(),
                    pos: [0.0, 1.0, 0.0],
                    kind: RegionKind::Road,
                },
            ],
            face_regions: RegionMemberships::from_memberships(vec![vec![0, 1, 2, 3, 4]]),
            settlements: vec![("Ashford".into(), SettlementKind::Town)],
            bridge_top_surfaces_by_name: std::collections::BTreeMap::from([(
                "Toll Bridge".into(),
                vec![[
                    [-100.0, 2001.0, -100.0],
                    [100.0, 2001.0, -100.0],
                    [0.0, 2001.0, 100.0],
                ]],
            )]),
        };

        assert_eq!(
            format_region_lines(
                0,
                &regions,
                Vec3::Y * (2001.0 + crate::constants::PLAYER_SIZE * 0.5),
            ),
            [
                "Geography: Elder Forest · Grey Range",
                "Settlement: Ashford (Town)",
                "Roads: King's Road · Salt Road",
                "Bridge: Toll Bridge",
            ]
        );
    }

    #[test]
    fn region_hud_uses_only_the_queried_faces_memberships() {
        let regions = LevelRegions {
            regions: vec![
                RegionData {
                    name: "Elder Forest".into(),
                    pos: [0.0, 1.0, 0.0],
                    kind: RegionKind::Forest,
                },
                RegionData {
                    name: "King's Road".into(),
                    pos: [0.0, 1.0, 0.0],
                    kind: RegionKind::Road,
                },
            ],
            face_regions: RegionMemberships::from_memberships(vec![vec![0], vec![1]]),
            settlements: vec![],
            bridge_top_surfaces_by_name: std::collections::BTreeMap::new(),
        };

        assert_eq!(
            format_region_lines(0, &regions, Vec3::ZERO),
            ["Geography: Elder Forest"]
        );
    }

    #[test]
    fn long_region_rows_are_truncated_only_for_display() {
        assert_eq!(
            truncate_region_row("Roads: King's Road · Coastal Road", 20),
            "Roads: King's Road…"
        );
        assert_eq!(truncate_region_row("Forest: Elder", 20), "Forest: Elder");
        assert_eq!(
            truncate_region_row("Forest: Örnskog Woodlands", 16),
            "Forest: Örnskog…"
        );
    }
}
