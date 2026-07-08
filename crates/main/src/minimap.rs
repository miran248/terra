use bevy::prelude::*;
use shared::sphere::SpherePos;
use shared::state::AppState;
use shared::theme;
use crate::loot::{LootMaterial, LootWeapon};
use crate::map::Survivor;
use crate::ui::UiFont;
use crate::zombie::Zombie;

const MINIMAP_SIZE: f32 = 160.0;
const DOT: f32 = 3.0;
/// World reference direction treated as "North" (the +Y pole of the planet).
const WORLD_NORTH: Vec3 = Vec3::Y;

#[derive(Component)]
struct Minimap;

#[derive(Component)]
struct MinimapDot;

#[derive(Component)]
struct CompassLabel;

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
    dots: Query<Entity, Or<(With<MinimapDot>, With<CompassLabel>)>>,
    survivor_q: Query<(&SpherePos, &Survivor)>,
    zombies: Query<&SpherePos, With<Zombie>>,
    materials: Query<&SpherePos, With<LootMaterial>>,
    weapons: Query<&SpherePos, With<LootWeapon>>,
    font: Res<UiFont>,
) {
    let Ok(map_entity) = minimap_q.single() else { return };
    let Ok((center, survivor)) = survivor_q.single() else { return };

    for dot in &dots {
        commands.entity(dot).despawn();
    }

    let radius = (MINIMAP_SIZE - 6.0) / 2.0;
    // Build the basis from the survivor's stable heading (not `tangent_basis`, whose
    // reference axis flips across latitude bands). Player faces "up" on the minimap.
    let up = center.0;
    let north = (survivor.heading - up * survivor.heading.dot(up)).normalize();
    let east = north.cross(up).normalize();

    // Orthographic projection of the hemisphere facing the player onto the disc.
    let place = |p: &SpherePos| -> Option<Vec2> {
        if center.0.dot(p.0) < 0.0 {
            return None; // on the far side of the planet
        }
        let x = p.0.dot(east);
        let y = p.0.dot(north);
        Some(Vec2::new(radius + x * radius, radius - y * radius))
    };

    let spawn_dot = |commands: &mut Commands, p: &SpherePos, color: Color, s: f32| {
        let Some(pt) = place(p) else { return };
        commands.spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(pt.x - s / 2.0),
                top: Val::Px(pt.y - s / 2.0),
                width: Val::Px(s),
                height: Val::Px(s),
                ..default()
            },
            BackgroundColor(color),
            MinimapDot,
            ChildOf(map_entity),
        ));
    };

    for p in &materials {
        spawn_dot(&mut commands, p, theme::TEXT_WEAK, DOT);
    }
    for p in &weapons {
        spawn_dot(&mut commands, p, theme::SUCCESS, DOT);
    }
    for p in &zombies {
        spawn_dot(&mut commands, p, theme::ERROR, DOT);
    }
    spawn_dot(&mut commands, center, theme::ACCENT, DOT * 2.0);

    draw_compass(&mut commands, map_entity, &font, radius, up, north, east);
}

/// Cardinal letters placed on the ring at their world bearing relative to the player's
/// heading, so they rotate as the player turns (a real compass).
fn draw_compass(
    commands: &mut Commands,
    map_entity: Entity,
    font: &UiFont,
    radius: f32,
    up: Vec3,
    north: Vec3,
    east: Vec3,
) {
    // World-north projected into the player's tangent plane, expressed in (east, north) axes.
    let wn = (WORLD_NORTH - up * WORLD_NORTH.dot(up)).normalize_or_zero();
    if wn == Vec3::ZERO {
        return; // player is at a pole; bearing undefined
    }
    let base = wn.dot(north).atan2(wn.dot(east)); // angle of world-north on the minimap
    let ring = radius - 10.0;

    for (i, letter) in ["N", "W", "S", "E"].iter().enumerate() {
        let a = base + i as f32 * std::f32::consts::FRAC_PI_2;
        // Screen: +x right, +y down; map angle measured from east (x) toward north (up = -screen y).
        let sx = radius + a.cos() * ring;
        let sy = radius - a.sin() * ring;
        let color = if i == 0 { theme::PRIMARY } else { theme::TEXT_WEAK };
        commands.spawn((
            Text::new(*letter),
            TextFont { font: font.0.clone().into(), font_size: FontSize::Px(11.0), ..default() },
            TextColor(color),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(sx - 4.0),
                top: Val::Px(sy - 7.0),
                ..default()
            },
            CompassLabel,
            ChildOf(map_entity),
        ));
    }
}

