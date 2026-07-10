use bevy::asset::RenderAssetUsages;
use bevy::camera::RenderTarget;
use bevy::image::Image;
use bevy::prelude::*;
use bevy::render::render_resource::{
    Extent3d, TextureDimension, TextureFormat, TextureUsages,
};
use shared::sphere::PLANET_RADIUS;
use shared::state::AppState;
use shared::theme;
use crate::loot::{LootMaterial, LootWeapon};
use crate::map::{Settlement, Player};
use crate::ui::UiFont;
use crate::zombie::Zombie;

const MINIMAP_SIZE: f32 = 160.0;
/// Render-target resolution (square, downscaled into the circular UI node).
const TEX: u32 = 256;
/// How high above the player the minimap camera sits, meters.
const CAM_HEIGHT: f32 = 1400.0;
/// Ground radius the minimap view covers, meters (≈ CAM_HEIGHT * tan(fov/2)).
const VIEW_RADIUS: f32 = 430.0;
const DOT: f32 = 3.0;
/// World reference direction treated as "North" (the +Y pole of the planet).
const WORLD_NORTH: Vec3 = Vec3::Y;

#[derive(Component)]
struct Minimap;

#[derive(Component)]
struct CompassLabel;

#[derive(Component)]
struct MinimapDot;

/// Marks the second camera that renders the top-down world view into the minimap texture.
#[derive(Component)]
pub struct MinimapCamera;

pub struct MinimapPlugin;

#[derive(Resource)]
struct MinimapTimer(Timer);

impl Default for MinimapTimer {
    fn default() -> Self { Self(Timer::from_seconds(0.15, TimerMode::Repeating)) }
}

impl Plugin for MinimapPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MinimapTimer>()
            .add_systems(Startup, setup_minimap)
            .add_systems(
                Update,
                (track_minimap_camera, draw_overlay).run_if(in_state(AppState::Playing)),
            );
    }
}

fn setup_minimap(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    // Render target the minimap camera draws into.
    let mut image = Image::new_fill(
        Extent3d { width: TEX, height: TEX, depth_or_array_layers: 1 },
        TextureDimension::D2,
        &[0, 0, 0, 0],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    image.texture_descriptor.usage =
        TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST | TextureUsages::RENDER_ATTACHMENT;
    let handle = images.add(image);

    // Second camera: renders the 3D world top-down into the texture (before the main pass).
    commands.spawn((
        Camera3d::default(),
        Camera {
            order: -1,
            clear_color: ClearColorConfig::Custom(theme::PANEL_BG),
            ..default()
        },
        RenderTarget::from(handle.clone()),
        // Render target is single-sampled; matching MSAA avoids a blank/gray main view.
        Msaa::Off,
        Projection::from(PerspectiveProjection { fov: 0.6, ..default() }),
        Transform::from_xyz(0.0, PLANET_RADIUS + CAM_HEIGHT, 0.0).looking_at(Vec3::ZERO, Vec3::Z),
        MinimapCamera,
    ));

    // Minimap UI: circular node showing the render texture, plus the compass overlay.
    commands
        .spawn((
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
            BorderColor::all(theme::TEXT_WEAK),
            BackgroundColor(theme::PANEL_BG),
            GlobalZIndex(10),
            Minimap,
        ))
        .with_child((
            ImageNode::new(handle),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                top: Val::Px(0.0),
                width: Val::Px(MINIMAP_SIZE - 6.0),
                height: Val::Px(MINIMAP_SIZE - 6.0),
                // Round the image itself into a circle (parent clip alone doesn't clip the
                // texture to the border radius in this Bevy version).
                border_radius: BorderRadius::all(Val::Percent(50.0)),
                overflow: Overflow::clip(),
                ..default()
            },
        ));
}

/// Keep the minimap camera high above the player, looking straight down, rolled so the
/// player's heading points up in the view (matches the compass).
fn track_minimap_camera(
    player_q: Query<(&Transform, &Player), (With<Player>, Without<MinimapCamera>)>,
    mut cam_q: Query<&mut Transform, With<MinimapCamera>>,
) {
    let Ok((player_tf, player)) = player_q.single() else { return };
    let Ok(mut cam_tf) = cam_q.single_mut() else { return };

    let up = player_tf.translation.normalize();
    let eye = up * (PLANET_RADIUS + CAM_HEIGHT);
    let heading = (player.heading - up * player.heading.dot(up)).normalize();
    cam_tf.translation = eye;
    cam_tf.look_at(up * PLANET_RADIUS, heading);
}

/// Rotating N/E/S/W labels around the ring, plus entity blips over the rendered terrain.
/// The render-to-texture camera shows terrain/roads/settlements, but actors are too small
/// to see from that height — so player/zombies/loot are drawn as UI dots here.
fn draw_overlay(
    mut commands: Commands,
    minimap_q: Query<Entity, With<Minimap>>,
    stale: Query<Entity, Or<(With<CompassLabel>, With<MinimapDot>)>>,
    player_q: Query<(&Transform, &Player)>,
    zombies: Query<&Transform, With<Zombie>>,
    materials: Query<&Transform, With<LootMaterial>>,
    weapons: Query<&Transform, With<LootWeapon>>,
    settlements: Query<(&Transform, &Settlement)>,
    font: Res<UiFont>,
    time: Res<Time>,
    mut timer: ResMut<MinimapTimer>,
) {
    if !timer.0.tick(time.delta()).just_finished() { return; }
    let Ok(map_entity) = minimap_q.single() else { return };
    let Ok((player_tf, player)) = player_q.single() else { return };

    for e in &stale {
        commands.entity(e).despawn();
    }

    let radius = (MINIMAP_SIZE - 6.0) / 2.0;
    let player_pos = player_tf.translation;
    let up = player_pos.normalize();
    let north = (player.heading - up * player.heading.dot(up)).normalize();
    let east = north.cross(up).normalize();

    // Project a world point onto the disc, matching the minimap camera's coverage.
    let place = |world_pos: Vec3| -> Option<Vec2> {
        let dir = world_pos.normalize();
        let dist = player_pos.distance(world_pos);
        if dist > VIEW_RADIUS {
            return None;
        }
        let x = dir.dot(east) * PLANET_RADIUS / VIEW_RADIUS;
        let y = dir.dot(north) * PLANET_RADIUS / VIEW_RADIUS;
        Some(Vec2::new(radius + x * radius, radius - y * radius))
    };

    let dot = |commands: &mut Commands, p: Vec3, color: Color, s: f32| {
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

    for tf in &materials { dot(&mut commands, tf.translation, theme::TEXT_WEAK, DOT); }
    for tf in &weapons { dot(&mut commands, tf.translation, theme::SUCCESS, DOT); }
    for tf in &zombies { dot(&mut commands, tf.translation, theme::ERROR, DOT); }

    for (tf, settlement) in &settlements {
        let Some(pt) = place(tf.translation) else { continue };
        let s = 6.0;
        commands.spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(pt.x - s / 2.0),
                top: Val::Px(pt.y - s / 2.0),
                width: Val::Px(s),
                height: Val::Px(s),
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BackgroundColor(theme::WARNING),
            BorderColor::all(theme::INK),
            MinimapDot,
            ChildOf(map_entity),
        ));
        commands.spawn((
            Text::new(settlement.name.clone()),
            TextFont { font: font.0.clone().into(), font_size: FontSize::Px(9.0), ..default() },
            TextColor(theme::INK),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(pt.x + 5.0),
                top: Val::Px(pt.y - 5.0),
                ..default()
            },
            MinimapDot,
            ChildOf(map_entity),
        ));
    }

    // Water body markers

    dot(&mut commands, player_pos, theme::ACCENT, DOT * 2.0);

    // Compass ring labels.
    let wn = (WORLD_NORTH - up * WORLD_NORTH.dot(up)).normalize_or_zero();
    if wn == Vec3::ZERO {
        return; // at a pole, bearing undefined
    }
    let base = wn.dot(north).atan2(wn.dot(east));
    let ring = radius - 10.0;
    for (i, letter) in ["N", "W", "S", "E"].iter().enumerate() {
        let a = base + i as f32 * std::f32::consts::FRAC_PI_2;
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
