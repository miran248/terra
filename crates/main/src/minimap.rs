use crate::loot::{LootMaterial, LootWeapon};
use crate::map::{LevelRegions, Player, Settlement};
use crate::ui::UiFont;
use crate::zombie::Zombie;
use avian3d::prelude::{LinearVelocity, Position};
use bevy::asset::RenderAssetUsages;
use bevy::camera::RenderTarget;
use bevy::image::Image;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};
use shared::level::RegionKind;
use shared::sphere::{PLANET_RADIUS, SpherePos};
use shared::state::AppState;
use shared::terrain::TerrainGen;
use shared::theme;

const MINIMAP_SIZE: f32 = 160.0;
/// Render-target resolution (square, downscaled into the circular UI node).
const TEX: u32 = 256;
/// How high above the player the minimap camera sits, meters.
const CAM_HEIGHT: f32 = 1400.0;
/// Ground radius covered by the flat minimap overlay, meters.
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
    fn default() -> Self {
        Self(Timer::from_seconds(0.05, TimerMode::Repeating))
    }
}

/// Full-screen teleport map (toggled with `M`).
#[derive(Resource, Default)]
struct WorldMapOpen(bool);

#[derive(Resource, Default)]
struct WorldMapView {
    center: Option<Vec3>,
    drag_cursor: Option<Vec2>,
    drag_distance: f32,
}

#[derive(Component)]
struct WorldMapCamera;

#[derive(Component)]
struct WorldMapRoot;

#[derive(Component)]
struct WorldMap;

/// Render-target resolution for the full-screen map.
const MAP_TEX: u32 = 1024;
/// How high above the player the map camera sits, meters.
const MAP_HEIGHT: f32 = 3000.0;
/// Half the tangent-plane width the map covers, meters.
const MAP_HALF_EXTENT: f32 = 1200.0;
/// The map image is a centred square this fraction of the viewport height.
const MAP_VH: f32 = 96.0;

impl Plugin for MinimapPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MinimapTimer>()
            .init_resource::<WorldMapOpen>()
            .init_resource::<WorldMapView>()
            .add_systems(Startup, (setup_minimap, setup_world_map))
            .add_systems(
                Update,
                (track_minimap_camera, draw_overlay).run_if(in_state(AppState::Playing)),
            )
            .add_systems(
                Update,
                (
                    toggle_world_map,
                    sync_world_map,
                    pan_world_map,
                    track_world_map_camera,
                    world_map_click,
                )
                    .run_if(in_state(AppState::Playing)),
            );
    }
}

fn setup_world_map(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let mut image = Image::new_fill(
        Extent3d {
            width: MAP_TEX,
            height: MAP_TEX,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &[0, 0, 0, 255],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    image.texture_descriptor.usage =
        TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST | TextureUsages::RENDER_ATTACHMENT;
    let handle = images.add(image);

    commands.spawn((
        Camera3d::default(),
        Camera {
            order: -2,
            is_active: false,
            ..default()
        },
        RenderTarget::from(handle.clone()),
        Msaa::Off,
        Projection::from(PerspectiveProjection {
            fov: 2.0 * (MAP_HALF_EXTENT / MAP_HEIGHT).atan(),
            near: 1.0,
            far: MAP_HEIGHT + 2.0 * PLANET_RADIUS,
            ..default()
        }),
        Transform::from_xyz(0.0, PLANET_RADIUS + MAP_HEIGHT, 0.0).looking_at(Vec3::ZERO, Vec3::Z),
        WorldMapCamera,
    ));

    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                top: Val::Px(0.0),
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.6)),
            GlobalZIndex(50),
            Visibility::Hidden,
            WorldMapRoot,
        ))
        .with_children(|parent| {
            parent
                .spawn((
                    Node {
                        position_type: PositionType::Relative,
                        width: Val::Vh(MAP_VH),
                        height: Val::Vh(MAP_VH),
                        border: UiRect::all(Val::Px(3.0)),
                        border_radius: BorderRadius::all(Val::Percent(50.0)),
                        overflow: Overflow::clip(),
                        ..default()
                    },
                    BorderColor::all(theme::TEXT_WEAK),
                    BackgroundColor(theme::PANEL_BG),
                    WorldMap,
                ))
                .with_child((
                    ImageNode::new(handle),
                    ZIndex(0),
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(0.0),
                        top: Val::Px(0.0),
                        width: Val::Percent(100.0),
                        height: Val::Percent(100.0),
                        border_radius: BorderRadius::all(Val::Percent(50.0)),
                        overflow: Overflow::clip(),
                        ..default()
                    },
                ));
        });
}

fn toggle_world_map(keys: Res<ButtonInput<KeyCode>>, mut open: ResMut<WorldMapOpen>) {
    if keys.just_pressed(KeyCode::KeyM) {
        open.0 = !open.0;
    }
}

/// Mirror the open flag onto the camera + overlay visibility.
fn sync_world_map(
    open: Res<WorldMapOpen>,
    mut cam: Query<&mut Camera, With<WorldMapCamera>>,
    mut root: Query<&mut Visibility, With<WorldMapRoot>>,
) {
    if !open.is_changed() {
        return;
    }
    if let Ok(mut c) = cam.single_mut() {
        c.is_active = open.0;
    }
    if let Ok(mut v) = root.single_mut() {
        *v = if open.0 {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
}

/// Keep the map camera high above the player, looking straight down, north-up.
fn track_world_map_camera(
    player_q: Query<&Transform, (With<Player>, Without<WorldMapCamera>)>,
    mut cam_q: Query<&mut Transform, With<WorldMapCamera>>,
    mut view: ResMut<WorldMapView>,
    open: Res<WorldMapOpen>,
) {
    let Ok(player_tf) = player_q.single() else {
        return;
    };
    let Ok(mut cam_tf) = cam_q.single_mut() else {
        return;
    };
    if !open.0 {
        view.center = None;
        view.drag_cursor = None;
        return;
    }
    let up = *view
        .center
        .get_or_insert_with(|| player_tf.translation.normalize());
    let mut north = WORLD_NORTH - up * WORLD_NORTH.dot(up);
    north = if north.length_squared() < 1e-4 {
        up.any_orthonormal_vector()
    } else {
        north.normalize()
    };
    cam_tf.translation = up * (PLANET_RADIUS + MAP_HEIGHT);
    cam_tf.look_at(up * PLANET_RADIUS, north);
}

/// Pan with a left-button drag; a left-button release without a drag teleports.
fn pan_world_map(
    open: Res<WorldMapOpen>,
    mouse: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    cam_q: Query<&Transform, With<WorldMapCamera>>,
    mut view: ResMut<WorldMapView>,
) {
    if !open.0 {
        return;
    }
    let Ok(window) = windows.single() else { return };
    let Some(cursor) = window.cursor_position() else {
        view.drag_cursor = None;
        return;
    };
    if mouse.just_pressed(MouseButton::Left) {
        view.drag_cursor = Some(cursor);
        view.drag_distance = 0.0;
        return;
    }
    if !mouse.pressed(MouseButton::Left) {
        view.drag_cursor = None;
        return;
    }
    let Some(previous) = view.drag_cursor.replace(cursor) else {
        return;
    };
    let Ok(camera) = cam_q.single() else { return };
    let Some(center) = view.center else { return };
    let side = window.height() * (MAP_VH / 100.0);
    let delta = cursor - previous;
    view.drag_distance += delta.length();
    let right = camera.rotation * Vec3::X;
    let screen_up = camera.rotation * Vec3::Y;
    let offset = (-right * delta.x + screen_up * delta.y) * (2.0 * MAP_HALF_EXTENT / side);
    view.center = Some((center * PLANET_RADIUS + offset).normalize());
}

/// Click on the open map to teleport to that surface location.
#[allow(
    clippy::type_complexity,
    reason = "Bevy ECS query filters encode access rules"
)]
fn world_map_click(
    mut open: ResMut<WorldMapOpen>,
    mouse: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    cam_q: Query<(&Camera, &GlobalTransform), (With<WorldMapCamera>, Without<Player>)>,
    terrain: Option<Res<TerrainGen>>,
    view: Res<WorldMapView>,
    mut player_q: Query<
        (&mut Position, &mut LinearVelocity),
        (With<Player>, Without<WorldMapCamera>),
    >,
) {
    const CLICK_DRAG_THRESHOLD: f32 = 4.0;
    if !open.0
        || !mouse.just_released(MouseButton::Left)
        || view.drag_distance > CLICK_DRAG_THRESHOLD
    {
        return;
    }
    let Ok(win) = windows.single() else { return };
    let Some(cursor) = win.cursor_position() else {
        return;
    };
    let (w, h) = (win.width(), win.height());
    // Click position within the centred square map image.
    let side = h * (MAP_VH / 100.0);
    let left = (w - side) / 2.0;
    let top = (h - side) / 2.0;
    let lx = (cursor.x - left) / side;
    let ly = (cursor.y - top) / side;
    if !(0.0..=1.0).contains(&lx) || !(0.0..=1.0).contains(&ly) {
        return; // clicked outside the map
    }
    if Vec2::new(lx - 0.5, ly - 0.5).length_squared() > 0.25 {
        return; // clicked outside the circular globe
    }
    let Ok((camera, camera_tf)) = cam_q.single() else {
        return;
    };
    let Ok(ray) = camera.viewport_to_world(
        camera_tf,
        Vec2::new(lx * MAP_TEX as f32, ly * MAP_TEX as f32),
    ) else {
        return;
    };
    // Intersect the camera ray with the generated terrain. This follows both the
    // perspective projection and visible elevation instead of assuming a flat
    // tangent plane or a sea-level sphere.
    let Some(terrain) = terrain else { return };
    let origin = ray.origin;
    let direction = *ray.direction;
    let altitude = |t: f32| {
        let point = origin + direction * t;
        point.length()
            - terrain
                .surface_radius(SpherePos::new(point.normalize()))
                .max(PLANET_RADIUS)
    };
    let far = MAP_HEIGHT + 2.0 * PLANET_RADIUS;
    let mut previous_t = 0.0;
    let mut previous_altitude = altitude(previous_t);
    let mut hit = None;
    for step in 1..=256 {
        let t = far * step as f32 / 256.0;
        let current_altitude = altitude(t);
        if previous_altitude > 0.0 && current_altitude <= 0.0 {
            hit = Some((previous_t, t));
            break;
        }
        previous_t = t;
        previous_altitude = current_altitude;
    }
    let Some((mut outside, mut inside)) = hit else {
        return;
    };
    for _ in 0..16 {
        let middle = (outside + inside) * 0.5;
        if altitude(middle) > 0.0 {
            outside = middle;
        } else {
            inside = middle;
        }
    }
    let dir = (origin + direction * inside).normalize();
    open.0 = false;

    // Place the player just above the surface at the target, velocity zeroed.
    let r = terrain
        .surface_radius(SpherePos::new(dir))
        .max(PLANET_RADIUS);
    if let Ok((mut position, mut vel)) = player_q.single_mut() {
        position.0 = dir * (r + 2.0);
        vel.0 = Vec3::ZERO;
    }
}

fn setup_minimap(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    // Render target the minimap camera draws into.
    let mut image = Image::new_fill(
        Extent3d {
            width: TEX,
            height: TEX,
            depth_or_array_layers: 1,
        },
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
        Projection::from(PerspectiveProjection {
            fov: 0.6,
            ..default()
        }),
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
#[allow(
    clippy::type_complexity,
    reason = "Bevy ECS query filters encode access rules"
)]
fn track_minimap_camera(
    player_q: Query<(&Transform, &Player), (With<Player>, Without<MinimapCamera>)>,
    mut cam_q: Query<&mut Transform, With<MinimapCamera>>,
) {
    let Ok((player_tf, player)) = player_q.single() else {
        return;
    };
    let Ok(mut cam_tf) = cam_q.single_mut() else {
        return;
    };

    let up = player_tf.translation.normalize();
    let eye = up * (PLANET_RADIUS + CAM_HEIGHT);
    let heading = (player.heading - up * player.heading.dot(up)).normalize();
    cam_tf.translation = eye;
    cam_tf.look_at(up * PLANET_RADIUS, heading);
}

/// Rotating N/E/S/W labels around the ring, plus entity blips over the rendered terrain.
/// The render-to-texture camera shows terrain/roads/settlements, but actors are too small
/// to see from that height — so player/zombies/loot are drawn as UI dots here.
#[allow(
    clippy::too_many_arguments,
    clippy::type_complexity,
    reason = "Bevy injects independent ECS system parameters"
)]
fn draw_overlay(
    mut commands: Commands,
    minimap_q: Query<(Entity, &ComputedNode), With<Minimap>>,
    world_map_q: Query<(Entity, &ComputedNode), With<WorldMap>>,
    map_cameras: Query<
        (
            &Camera,
            &GlobalTransform,
            Option<&MinimapCamera>,
            Option<&WorldMapCamera>,
        ),
        Or<(With<MinimapCamera>, With<WorldMapCamera>)>,
    >,
    stale: Query<Entity, Or<(With<CompassLabel>, With<MinimapDot>)>>,
    map_resources: (Option<Res<TerrainGen>>, Option<Res<LevelRegions>>),
    player_q: Query<(&Transform, &Player)>,
    zombies: Query<(&Transform, &ViewVisibility), With<Zombie>>,
    materials: Query<(&Transform, &ViewVisibility), With<LootMaterial>>,
    weapons: Query<(&Transform, &ViewVisibility), With<LootWeapon>>,
    settlements: Query<(&Transform, &Settlement)>,
    font: Res<UiFont>,
    time: Res<Time>,
    mut timer: ResMut<MinimapTimer>,
    world_map_open: Res<WorldMapOpen>,
    world_map_view: Res<WorldMapView>,
) {
    if !timer.0.tick(time.delta()).just_finished() {
        return;
    }
    let Ok((minimap_entity, minimap_node)) = minimap_q.single() else {
        return;
    };
    let Ok((player_tf, player)) = player_q.single() else {
        return;
    };
    let (terrain, regions) = map_resources;

    for e in &stale {
        commands.entity(e).despawn();
    }

    let player_pos = player_tf.translation;
    let player_up = player_pos.normalize();
    let heading = (player.heading - player_up * player.heading.dot(player_up)).normalize();
    let mut minimap_camera = None;
    let mut world_map_camera = None;
    for (camera, camera_tf, minimap_marker, world_map_marker) in &map_cameras {
        if minimap_marker.is_some() {
            minimap_camera = Some((camera, camera_tf));
        }
        if world_map_marker.is_some() {
            world_map_camera = Some((camera, camera_tf));
        }
    }
    let Some((minimap_camera, minimap_camera_tf)) = minimap_camera else {
        return;
    };
    let mut maps = vec![(
        minimap_entity,
        minimap_node.size().x * minimap_node.inverse_scale_factor(),
        player_up,
        heading,
        minimap_camera,
        minimap_camera_tf,
        false,
    )];
    if world_map_open.0
        && let Ok((map_entity, map_node)) = world_map_q.single()
        && let Some((map_camera, map_camera_tf)) = world_map_camera
    {
        let map_up = world_map_view.center.unwrap_or(player_up);
        let map_north = (WORLD_NORTH - map_up * WORLD_NORTH.dot(map_up)).normalize_or(heading);
        maps.push((
            map_entity,
            map_node.size().x * map_node.inverse_scale_factor(),
            map_up,
            map_north,
            map_camera,
            map_camera_tf,
            true,
        ));
    }

    for (map_entity, size, up, north, camera, camera_tf, is_globe) in maps {
        let radius = (size - 6.0) / 2.0;
        let marker_scale = (size / MINIMAP_SIZE).clamp(1.0, 2.0);
        let east = north.cross(up).normalize();
        // Keep the minimap's original flat heading-up projection. The fullscreen
        // globe instead uses its exact movable perspective camera and grounds
        // markers to the terrain to avoid altitude parallax.
        let place = |world_pos: Vec3| -> Option<Vec2> {
            if !is_globe {
                if player_pos.distance(world_pos) > VIEW_RADIUS {
                    return None;
                }
                let direction = world_pos.normalize();
                let x = direction.dot(east) * PLANET_RADIUS / VIEW_RADIUS;
                let y = direction.dot(north) * PLANET_RADIUS / VIEW_RADIUS;
                return Some(Vec2::new(radius + x * radius, radius - y * radius));
            }
            let direction = world_pos.normalize();
            let ground_pos = terrain.as_deref().map_or(world_pos, |terrain| {
                direction
                    * terrain
                        .surface_radius(SpherePos::new(direction))
                        .max(PLANET_RADIUS)
            });
            let camera_pos = camera_tf.translation();
            if direction.dot(camera_pos - ground_pos) <= 0.0 {
                return None;
            }
            let ndc = camera.world_to_ndc(camera_tf, ground_pos)?;
            let pt = Vec2::new((ndc.x + 1.0) * radius, (1.0 - ndc.y) * radius);
            let centered = pt - Vec2::splat(radius);
            if centered.length_squared() > radius * radius {
                return None;
            }
            Some(pt)
        };
        {
            let mut dot = |p: Vec3, color: Color, s: f32| {
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
                    ZIndex(1),
                    MinimapDot,
                    ChildOf(map_entity),
                ));
            };
            for (tf, visible) in &materials {
                if visible.get() {
                    dot(tf.translation, theme::TEXT_WEAK, DOT * marker_scale);
                }
            }
            for (tf, visible) in &weapons {
                if visible.get() {
                    dot(tf.translation, theme::SUCCESS, DOT * marker_scale);
                }
            }
            for (tf, visible) in &zombies {
                if visible.get() {
                    dot(tf.translation, theme::ERROR, DOT * marker_scale);
                }
            }
            dot(player_pos, theme::ACCENT, DOT * 2.0 * marker_scale);
        }

        for (tf, settlement) in &settlements {
            let Some(pt) = place(tf.translation) else {
                continue;
            };
            let s = 6.0 * marker_scale;
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
                ZIndex(1),
                MinimapDot,
                ChildOf(map_entity),
            ));
            commands.spawn((
                Text::new(settlement.name.clone()),
                TextFont {
                    font: font.0.clone().into(),
                    font_size: FontSize::Px(9.0),
                    ..default()
                },
                TextColor(theme::INK),
                ZIndex(1),
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

        if let Some(regions) = regions.as_deref() {
            for region in &regions.regions {
                let world_pos = Vec3::from_array(region.pos) * PLANET_RADIUS;
                let Some(pt) = place(world_pos) else {
                    continue;
                };
                let color = match region.kind {
                    RegionKind::Ocean
                    | RegionKind::Lake
                    | RegionKind::SaltLake
                    | RegionKind::River => theme::INFO,
                    RegionKind::Mountain | RegionKind::Volcano | RegionKind::Glacier => {
                        theme::ACCENT
                    }
                    RegionKind::Town | RegionKind::Road => theme::WARNING,
                    RegionKind::Beach | RegionKind::Cliff => theme::PRIMARY,
                    _ => theme::INK,
                };
                let s = 3.0 * marker_scale;
                commands.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(pt.x - s / 2.0),
                        top: Val::Px(pt.y - s / 2.0),
                        width: Val::Px(s),
                        height: Val::Px(s),
                        border_radius: BorderRadius::all(Val::Percent(50.0)),
                        ..default()
                    },
                    BackgroundColor(color),
                    ZIndex(1),
                    MinimapDot,
                    ChildOf(map_entity),
                ));
                commands.spawn((
                    Text::new(region.name.clone()),
                    TextFont {
                        font: font.0.clone().into(),
                        font_size: FontSize::Px(8.0 * marker_scale),
                        ..default()
                    },
                    TextColor(color),
                    ZIndex(1),
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(pt.x + 3.0 * marker_scale),
                        top: Val::Px(pt.y - 5.0 * marker_scale),
                        ..default()
                    },
                    MinimapDot,
                    ChildOf(map_entity),
                ));
            }
        }

        let world_north = (WORLD_NORTH - up * WORLD_NORTH.dot(up)).normalize_or(north);
        let base = world_north.dot(north).atan2(world_north.dot(east));
        let label_font_size = 11.0 * marker_scale;
        let label_width = 7.0 * marker_scale;
        let label_height = 14.0 * marker_scale;
        let ring = radius - label_height / 2.0 - 2.0;
        for (i, letter) in ["N", "W", "S", "E"].iter().enumerate() {
            let a = base + i as f32 * std::f32::consts::FRAC_PI_2;
            commands.spawn((
                Text::new(*letter),
                TextFont {
                    font: font.0.clone().into(),
                    font_size: FontSize::Px(label_font_size),
                    ..default()
                },
                TextColor(if i == 0 {
                    theme::PRIMARY
                } else {
                    theme::TEXT_WEAK
                }),
                ZIndex(1),
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(radius + a.cos() * ring - label_width / 2.0),
                    top: Val::Px(radius - a.sin() * ring - label_height / 2.0),
                    ..default()
                },
                CompassLabel,
                ChildOf(map_entity),
            ));
        }
    }
}
