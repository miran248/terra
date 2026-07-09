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
const TEX: u32 = 256;
const CAM_HEIGHT: f32 = 1400.0;
const VIEW_RADIUS: f32 = 430.0;
const WORLD_NORTH: Vec3 = Vec3::Y;

#[derive(Component)]
struct Minimap;

#[derive(Component)]
struct CompassLabel;

#[derive(Component, Clone, Copy)]
enum DotKind { Player, Zombie(u32), Material(u32), Weapon(u32), Settlement(u32) }

#[derive(Component)]
struct MinimapDot(DotKind);

#[derive(Component)]
pub struct MinimapCamera;

#[derive(Resource)]
struct MinimapTimer(Timer);

impl Default for MinimapTimer {
    fn default() -> Self { Self(Timer::from_seconds(0.2, TimerMode::Repeating)) }
}

const MAX_ZOMBIE_DOTS: u32 = 200;
const MAX_MATERIAL_DOTS: u32 = 500;
const MAX_WEAPON_DOTS: u32 = 100;

pub struct MinimapPlugin;

impl Plugin for MinimapPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MinimapTimer>()
            .add_systems(Startup, setup_minimap)
            .add_systems(OnEnter(AppState::Playing), spawn_minimap_dots)
            .add_systems(
                Update,
                (track_minimap_camera, update_minimap_dots).run_if(in_state(AppState::Playing)),
            );
    }
}

fn setup_minimap(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
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

    commands.spawn((
        Camera3d::default(),
        Camera {
            order: -1,
            clear_color: ClearColorConfig::Custom(theme::PANEL_BG),
            ..default()
        },
        RenderTarget::from(handle.clone()),
        Msaa::Off,
        Projection::from(PerspectiveProjection { fov: 0.6, ..default() }),
        Transform::from_xyz(0.0, PLANET_RADIUS + CAM_HEIGHT, 0.0).looking_at(Vec3::ZERO, Vec3::Z),
        MinimapCamera,
    ));

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
                border_radius: BorderRadius::all(Val::Percent(50.0)),
                overflow: Overflow::clip(),
                ..default()
            },
        ));
}

/// Spawn persistent dot entities once — updated in-place every frame, never despawned.
fn spawn_minimap_dots(mut commands: Commands, minimap_q: Query<Entity, With<Minimap>>, font: Option<Res<UiFont>>) {
    let Some(font) = font else { return };
    let Ok(map_entity) = minimap_q.single() else { return };

    // Player dot
    commands.spawn((
        Node { position_type: PositionType::Absolute, ..default() },
        BackgroundColor(theme::ACCENT),
        MinimapDot(DotKind::Player),
        ChildOf(map_entity),
    ));

    // Zombie dot pool
    for i in 0..MAX_ZOMBIE_DOTS {
        commands.spawn((
            Node { position_type: PositionType::Absolute, display: Display::None, ..default() },
            BackgroundColor(theme::ERROR),
            MinimapDot(DotKind::Zombie(i)),
            ChildOf(map_entity),
        ));
    }

    // Material dot pool
    for i in 0..MAX_MATERIAL_DOTS {
        commands.spawn((
            Node { position_type: PositionType::Absolute, display: Display::None, ..default() },
            BackgroundColor(theme::TEXT_WEAK),
            MinimapDot(DotKind::Material(i)),
            ChildOf(map_entity),
        ));
    }

    // Weapon dot pool
    for i in 0..MAX_WEAPON_DOTS {
        commands.spawn((
            Node { position_type: PositionType::Absolute, display: Display::None, ..default() },
            BackgroundColor(theme::SUCCESS),
            MinimapDot(DotKind::Weapon(i)),
            ChildOf(map_entity),
        ));
    }

    // Settlement dots + labels per settlement (spawned later when settlements are known)
    // Compass labels: static
    let radius = (MINIMAP_SIZE - 6.0) / 2.0;
    for (i, letter) in ["N", "W", "S", "E"].iter().enumerate() {
        let color = if i == 0 { theme::PRIMARY } else { theme::TEXT_WEAK };
        commands.spawn((
            Text::new(*letter),
            TextFont { font: font.0.clone().into(), font_size: FontSize::Px(11.0), ..default() },
            TextColor(color),
            Node { position_type: PositionType::Absolute, ..default() },
            CompassLabel,
            ChildOf(map_entity),
        ));
    }
}

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

fn update_minimap_dots(
    time: Res<Time>,
    mut timer: ResMut<MinimapTimer>,
    player_q: Query<(&Transform, &Player)>,
    zombies: Query<&Transform, With<Zombie>>,
    materials: Query<&Transform, With<LootMaterial>>,
    weapons: Query<&Transform, With<LootWeapon>>,
    settlements: Query<(&Transform, &Settlement)>,
    mut dots: Query<(&MinimapDot, &mut Node, &mut BackgroundColor), Without<CompassLabel>>,
    mut compass: Query<(&CompassLabel, &mut Node)>,
) {
    if !timer.0.tick(time.delta()).just_finished() { return; }

    let Ok((player_tf, player)) = player_q.single() else { return };
    let radius = (MINIMAP_SIZE - 6.0) / 2.0;
    let player_pos = player_tf.translation;
    let up = player_pos.normalize();
    let north = (player.heading - up * player.heading.dot(up)).normalize();
    let east = north.cross(up).normalize();

    let place = |world_pos: Vec3| -> Option<Vec2> {
        let dir = world_pos.normalize();
        let dist = player_pos.distance(world_pos);
        if dist > VIEW_RADIUS { return None; }
        let x = dir.dot(east) * PLANET_RADIUS / VIEW_RADIUS;
        let y = dir.dot(north) * PLANET_RADIUS / VIEW_RADIUS;
        Some(Vec2::new(radius + x * radius, radius - y * radius))
    };

    let set_dot = |node: &mut Node, pt: Vec2, s: f32| {
        *node = Node {
            position_type: PositionType::Absolute,
            left: Val::Px(pt.x - s / 2.0),
            top: Val::Px(pt.y - s / 2.0),
            width: Val::Px(s),
            height: Val::Px(s),
            display: Display::Flex,
            ..default()
        };
    };

    let hide = |node: &mut Node| { node.display = Display::None; };

    // Collect world positions by kind
    let zombie_positions: Vec<Vec3> = zombies.iter().map(|t| t.translation).collect();
    let material_positions: Vec<Vec3> = materials.iter().map(|t| t.translation).collect();
    let weapon_positions: Vec<Vec3> = weapons.iter().map(|t| t.translation).collect();

    // Update all dots in one pass
    for (dot_kind, mut node, mut bg) in &mut dots {
        let kind = dot_kind.0;
        match kind {
            DotKind::Player => {
                let Some(pt) = place(player_pos) else { hide(&mut node); continue; };
                set_dot(&mut node, pt, 6.0);
            }
            DotKind::Zombie(i) => {
                if let Some(pos) = zombie_positions.get(i as usize) {
                    let Some(pt) = place(*pos) else { hide(&mut node); continue; };
                    set_dot(&mut node, pt, 3.0);
                } else { hide(&mut node); }
            }
            DotKind::Material(i) => {
                if let Some(pos) = material_positions.get(i as usize) {
                    let Some(pt) = place(*pos) else { hide(&mut node); continue; };
                    set_dot(&mut node, pt, 3.0);
                } else { hide(&mut node); }
            }
            DotKind::Weapon(i) => {
                if let Some(pos) = weapon_positions.get(i as usize) {
                    let Some(pt) = place(*pos) else { hide(&mut node); continue; };
                    set_dot(&mut node, pt, 3.0);
                } else { hide(&mut node); }
            }
            DotKind::Settlement(i) => {
                if let Some((tf, _)) = settlements.iter().nth(i as usize) {
                    let Some(pt) = place(tf.translation) else { hide(&mut node); continue; };
                    // Settlement marker
                    set_dot(&mut node, pt, 6.0);
                    *bg = BackgroundColor(theme::WARNING);
                } else { hide(&mut node); }
            }
        }
    }

    // Compass labels: update positions based on current heading
    let wn = (WORLD_NORTH - up * WORLD_NORTH.dot(up)).normalize_or_zero();
    if wn != Vec3::ZERO {
        let base = wn.dot(north).atan2(wn.dot(east));
        let ring = radius - 10.0;
        for (i, (_, mut node)) in compass.iter_mut().enumerate() {
            let a = base + i as f32 * std::f32::consts::FRAC_PI_2;
            node.left = Val::Px(radius + a.cos() * ring - 4.0);
            node.top = Val::Px(radius - a.sin() * ring - 7.0);
        }
    }
}
