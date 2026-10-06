use std::collections::HashMap;

// use crate::loot::{LootMaterial, LootWeapon};
use crate::ui::UiFont;
use crate::{
    exploration::{Exploration, PlanetDestination, PlanetDestinationId, PlanetDestinationSurface},
    map::{LevelRegions, MainCamera, Player, Settlement, WorldEpoch},
    surface_labels::{self, SurfaceGlyphAtlas, SurfaceLabelRibbon, SurfaceLabelView},
};
// use crate::zombie::Zombie;
use bevy::asset::RenderAssetUsages;
use bevy::camera::RenderTarget;
use bevy::camera::visibility::RenderLayers;
use bevy::image::Image;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};
use shared::planet_markers::{
    PlanetMarkerKind, PlanetMarkerShape, planet_marker_label_visible, planet_marker_presentation,
};
use shared::planet_view::planet_compass_color;
use shared::planet_view_interface::GameplayHudElement;
use shared::state::AppState;
use shared::theme;
use terra_geometry::sphere::{PLANET_RADIUS, SpherePos};
use terra_world::level::RegionKind;
use terra_worldgen::terrain::TerrainGen;

const MINIMAP_SIZE: f32 = 160.0;
/// Render-target resolution (square, downscaled into the circular UI node).
const TEX: u32 = 256;
/// How high above the player the minimap camera sits, meters.
const CAM_HEIGHT: f32 = 1400.0;
const MINIMAP_FOV: f32 = 0.6;
/// Ground radius covered by the flat minimap overlay, meters.
const VIEW_RADIUS: f32 = 430.0;
const DOT: f32 = 3.0;
const MAP_LABEL_FONT_SIZE: f32 = 8.0;
const MAP_LABEL_OFFSET: Vec2 = Vec2::new(6.0, -2.0);
const MAP_LABEL_CLEARANCE: f32 = 2.0;
const MAP_LABEL_ROTATION_EPSILON: f32 = 0.004;
const MAP_LABEL_HOVER_RADIUS: f32 = 12.0;
/// Ray origins can be this high above the planet while still finding its surface.
const SURFACE_PICKING_ALTITUDE: f32 = 3000.0;
/// World reference direction treated as "North" (the +Y pole of the planet).
const WORLD_NORTH: Vec3 = Vec3::Y;

#[derive(Component)]
struct Minimap;

#[derive(Component)]
struct CompassLabel;

#[derive(Clone, Debug, PartialEq)]
struct MapLabel {
    anchor: Vec2,
    world_anchor: Vec3,
    marker_index: usize,
    text: String,
    kind: PlanetMarkerKind,
    font_size: f32,
}

#[derive(Resource, Default)]
struct MinimapSurfaceLabelState {
    world_epoch: Option<WorldEpoch>,
    labels: Vec<MapLabel>,
}

#[derive(Resource, Default)]
struct MinimapSurfaceLabelAssets {
    world_epoch: Option<WorldEpoch>,
    atlas: Option<SurfaceGlyphAtlas>,
    materials: Vec<(Color, Handle<StandardMaterial>)>,
}

#[derive(Component)]
struct MinimapSurfaceLabelOwner {
    marker_index: usize,
    world_anchor: Vec3,
    kind: PlanetMarkerKind,
    font_size: f32,
    camera_rotation: Quat,
}

#[derive(Clone, Copy)]
struct MapContentGeometry {
    size: Vec2,
    origin_from_center: Vec2,
    inverse_scale_factor: f32,
}

fn build_minimap_surface_ribbon(
    atlas: &SurfaceGlyphAtlas,
    text: &str,
    anchor: Vec3,
    font_size: f32,
    camera_transform: &Transform,
    terrain: Option<&TerrainGen>,
) -> surface_labels::SurfaceRibbonMesh {
    let world_per_texture_pixel = 2.0 * CAM_HEIGHT * (MINIMAP_FOV * 0.5).tan() / TEX as f32;
    let texture_per_ui_pixel = TEX as f32 / (MINIMAP_SIZE - 6.0);
    let world_per_atlas_pixel = world_per_texture_pixel * texture_per_ui_pixel * font_size
        / surface_labels::ATLAS_FONT_SIZE;
    let label_offset_atlas_pixels =
        MAP_LABEL_OFFSET * (texture_per_ui_pixel * surface_labels::ATLAS_FONT_SIZE / font_size);

    surface_labels::build_surface_ribbon(
        atlas,
        text,
        anchor,
        surface_labels::SurfaceRibbonLayout {
            camera_right: camera_transform.rotation * Vec3::X,
            camera_up: camera_transform.rotation * Vec3::Y,
            world_per_atlas_pixel,
            label_offset_atlas_pixels,
            clearance: MAP_LABEL_CLEARANCE,
        },
        |direction| {
            terrain.map_or(anchor.length(), |terrain| {
                terrain.surface_radius(SpherePos::new(direction))
            })
        },
    )
}

fn map_content_geometry(
    outer_size_physical: Vec2,
    border: BorderRect,
    inverse_scale_factor: f32,
) -> MapContentGeometry {
    MapContentGeometry {
        size: (outer_size_physical - border.min_inset - border.max_inset).max(Vec2::ZERO)
            * inverse_scale_factor,
        origin_from_center: (outer_size_physical / 2.0 - border.min_inset) * inverse_scale_factor,
        inverse_scale_factor,
    }
}

fn cursor_in_map(
    cursor_physical: Vec2,
    transform: UiGlobalTransform,
    content: MapContentGeometry,
) -> Option<Vec2> {
    let local_physical = transform.try_inverse()?.transform_point2(cursor_physical);
    let cursor = local_physical * content.inverse_scale_factor + content.origin_from_center;
    let center = content.size / 2.0;
    let radius = content.size.min_element() / 2.0;
    (content.size.min_element() > 0.0 && cursor.distance_squared(center) <= radius * radius)
        .then_some(cursor)
}

fn map_label_default_visible(kind: RegionKind) -> bool {
    planet_marker_label_visible(map_region_marker_kind(kind), false)
}

fn map_region_marker_kind(kind: RegionKind) -> PlanetMarkerKind {
    PlanetMarkerKind::Region(kind)
}

fn map_marker_components(
    point: Vec2,
    size: f32,
    kind: PlanetMarkerKind,
) -> (Node, BackgroundColor, BorderColor) {
    let presentation = planet_marker_presentation(kind);
    let border_radius = match presentation.shape {
        PlanetMarkerShape::Circle => BorderRadius::MAX,
        PlanetMarkerShape::Square => BorderRadius::ZERO,
    };
    let border = if presentation.outlined {
        UiRect::all(Val::Px(1.0))
    } else {
        UiRect::all(Val::Px(0.0))
    };
    (
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(point.x - size / 2.0),
            top: Val::Px(point.y - size / 2.0),
            width: Val::Px(size),
            height: Val::Px(size),
            border_radius,
            border,
            ..default()
        },
        BackgroundColor(presentation.color),
        BorderColor::all(theme::INK),
    )
}

fn nearest_hovered_label(cursor: Option<Vec2>, labels: &[MapLabel]) -> Option<usize> {
    let cursor = cursor?;
    let radius_squared = MAP_LABEL_HOVER_RADIUS * MAP_LABEL_HOVER_RADIUS;
    labels
        .iter()
        .enumerate()
        .filter_map(|(index, label)| {
            let distance_squared = cursor.distance_squared(label.anchor);
            (distance_squared <= radius_squared).then_some((distance_squared, index))
        })
        .min_by(
            |(left_distance, left_index), (right_distance, right_index)| {
                left_distance
                    .total_cmp(right_distance)
                    .then_with(|| left_index.cmp(right_index))
            },
        )
        .map(|(_, index)| index)
}

fn bridge_surface_centroid(triangles: &[[[f32; 3]; 3]]) -> Option<Vec3> {
    let mut weighted_center = Vec3::ZERO;
    let mut total_area = 0.0;
    for triangle in triangles {
        let [a, b, c] = triangle.map(Vec3::from_array);
        let area = (b - a).cross(c - a).length() * 0.5;
        if !area.is_finite() || area <= 1e-8 {
            continue;
        }
        weighted_center += ((a + b + c) / 3.0) * area;
        total_area += area;
    }
    (total_area > 0.0).then(|| weighted_center / total_area)
}

fn map_marker_position(
    world_position: Vec3,
    terrain: Option<&TerrainGen>,
    snap_to_terrain: bool,
) -> Vec3 {
    if !snap_to_terrain {
        return world_position;
    }
    let direction = world_position.normalize();
    terrain.map_or(world_position, |terrain| {
        direction
            * terrain
                .surface_radius(SpherePos::new(direction))
                .max(PLANET_RADIUS)
    })
}

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

impl Plugin for MinimapPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MinimapTimer>()
            .init_resource::<MinimapSurfaceLabelState>()
            .init_resource::<MinimapSurfaceLabelAssets>()
            .add_systems(Startup, setup_minimap)
            .add_systems(
                Update,
                (
                    track_minimap_camera,
                    draw_overlay,
                    update_minimap_surface_labels,
                    sync_minimap_visibility,
                )
                    .chain()
                    .run_if(in_state(AppState::Playing)),
            );
    }
}

fn sync_minimap_visibility(
    exploration: Option<Res<crate::exploration::Exploration>>,
    mut minimaps: Query<&mut Node, With<Minimap>>,
) {
    let hidden = exploration
        .is_some_and(|state| state.is_planet_view_active() && state.gameplay_hud_opacity() <= 0.0);
    for mut node in &mut minimaps {
        node.display = if hidden { Display::None } else { Display::Flex };
    }
}

/// Surface intersection data shared by Planet-view picking and its fixtures.
#[derive(Clone, Copy)]
pub(crate) struct MapSurfaceHit {
    pub distance: f32,
    pub position: Vec3,
    pub normal: Vec3,
    pub surface: PlanetDestinationSurface,
}

fn map_click_surface_hit(
    origin: Vec3,
    direction: Vec3,
    terrain: &TerrainGen,
    bridge_surfaces: Option<&std::collections::BTreeMap<String, Vec<[[f32; 3]; 3]>>>,
) -> Option<MapSurfaceHit> {
    let terrain_hit = terrain_surface_hit(origin, direction, terrain);
    let bridge_hit = bridge_surfaces
        .into_iter()
        .flat_map(|surfaces| surfaces.values())
        .flat_map(|triangles| triangles.iter())
        .filter_map(|triangle| {
            let triangle = triangle.map(Vec3::from_array);
            let distance = terra_geometry::planet::ray_triangle_intersection_distance(
                origin, direction, &triangle,
            )?;
            let position = origin + direction.normalize_or_zero() * distance;
            let mut normal = (triangle[1] - triangle[0]).cross(triangle[2] - triangle[0]);
            if normal.length_squared() < 1e-8 {
                return None;
            }
            normal = normal.normalize();
            if normal.dot(position) < 0.0 {
                normal = -normal;
            }
            Some(MapSurfaceHit {
                distance,
                position,
                normal,
                surface: PlanetDestinationSurface::BridgeDeck,
            })
        })
        .min_by(|left, right| left.distance.total_cmp(&right.distance));

    match (terrain_hit, bridge_hit) {
        (Some(terrain), Some(bridge)) if bridge.distance < terrain.distance => Some(bridge),
        (Some(terrain), _) => Some(terrain),
        (None, bridge) => bridge,
    }
}

pub(crate) fn consume_planet_view_destination_click(
    mut state: ResMut<Exploration>,
    world_epoch: Option<Res<WorldEpoch>>,
    terrain: Option<Res<TerrainGen>>,
    regions: Option<Res<LevelRegions>>,
    projection: Option<Res<crate::planet_markers::PlanetMarkerProjection>>,
    cameras: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
) {
    let Some(cursor) = state.take_planet_view_selection() else {
        return;
    };
    if !state.planet_view_ready() {
        return;
    }
    let (Some(world_epoch), Some(terrain), Ok((camera, camera_transform))) =
        (world_epoch, terrain, cameras.single())
    else {
        return;
    };
    if crate::planet_markers::select_named_marker_at(
        &mut state,
        *world_epoch,
        projection.as_deref(),
        cursor,
    ) {
        return;
    }
    let bridge_surfaces = regions
        .as_deref()
        .map(|regions| &regions.bridge_top_surfaces_by_name);
    let Some(hit) =
        map_camera_surface_hit(camera, camera_transform, cursor, &terrain, bridge_surfaces)
    else {
        // A sky or occluded-surface click leaves the previous selection intact.
        return;
    };

    state.select_planet_destination(destination_from_surface_hit(*world_epoch, hit));
}

fn destination_from_surface_hit(world_epoch: WorldEpoch, hit: MapSurfaceHit) -> PlanetDestination {
    let direction = hit.position.normalize_or(Vec3::Y);
    let latitude = direction.y.clamp(-1.0, 1.0).asin().to_degrees();
    let longitude = direction.z.atan2(direction.x).to_degrees();
    let surface_name = match hit.surface {
        PlanetDestinationSurface::Terrain => "Terrain",
        PlanetDestinationSurface::BridgeDeck => "Bridge deck",
    };
    let display = format!(
        "{surface_name} · {:.1}°{}, {:.1}°{}",
        latitude.abs(),
        if latitude >= 0.0 { 'N' } else { 'S' },
        longitude.abs(),
        if longitude >= 0.0 { 'E' } else { 'W' },
    );
    PlanetDestination {
        id: PlanetDestinationId::surface(world_epoch, hit.position),
        position: hit.position,
        surface: hit.surface,
        display,
    }
}

pub(crate) fn map_camera_surface_hit(
    camera: &Camera,
    camera_transform: &GlobalTransform,
    physical_cursor: Vec2,
    terrain: &TerrainGen,
    bridge_surfaces: Option<&std::collections::BTreeMap<String, Vec<[[f32; 3]; 3]>>>,
) -> Option<MapSurfaceHit> {
    let scale_factor = camera
        .target_scaling_factor()
        .filter(|scale| scale.is_finite() && *scale > 0.0)
        .unwrap_or(1.0);
    let ray = camera
        .viewport_to_world(camera_transform, physical_cursor / scale_factor)
        .ok()?;
    map_click_surface_hit(
        ray.origin,
        ray.direction.as_vec3(),
        terrain,
        bridge_surfaces,
    )
}

fn terrain_surface_hit(
    origin: Vec3,
    direction: Vec3,
    terrain: &TerrainGen,
) -> Option<MapSurfaceHit> {
    let direction = direction.normalize_or_zero();
    if direction == Vec3::ZERO {
        return None;
    }
    let altitude = |distance: f32| {
        let point = origin + direction * distance;
        point.length()
            - terrain
                .surface_radius(SpherePos::new(point.normalize()))
                .max(PLANET_RADIUS)
    };
    let far = SURFACE_PICKING_ALTITUDE + 2.0 * PLANET_RADIUS;
    let mut previous_distance = 0.0;
    let mut previous_altitude = altitude(previous_distance);
    let mut bracket = None;
    for step in 1..=256 {
        let distance = far * step as f32 / 256.0;
        let current_altitude = altitude(distance);
        if previous_altitude > 0.0 && current_altitude <= 0.0 {
            bracket = Some((previous_distance, distance));
            break;
        }
        previous_distance = distance;
        previous_altitude = current_altitude;
    }
    let (mut outside, mut inside) = bracket?;
    for _ in 0..16 {
        let middle = (outside + inside) * 0.5;
        if altitude(middle) > 0.0 {
            outside = middle;
        } else {
            inside = middle;
        }
    }
    let point = origin + direction * inside;
    let up = point.normalize();
    let radius = terrain
        .surface_radius(SpherePos::new(up))
        .max(PLANET_RADIUS);
    Some(MapSurfaceHit {
        distance: inside,
        position: up * radius,
        normal: up,
        surface: PlanetDestinationSurface::Terrain,
    })
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
            fov: MINIMAP_FOV,
            ..default()
        }),
        Transform::from_xyz(0.0, PLANET_RADIUS + CAM_HEIGHT, 0.0).looking_at(Vec3::ZERO, Vec3::Z),
        RenderLayers::from_layers(&[0, surface_labels::MINIMAP_LABEL_LAYER]),
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
            GameplayHudElement::default(),
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
            GameplayHudElement::default(),
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
    minimap_q: Query<(Entity, &ComputedNode, &UiGlobalTransform), With<Minimap>>,
    windows: Query<&Window>,
    stale: Query<Entity, Or<(With<CompassLabel>, With<MinimapDot>)>>,
    regions: Option<Res<LevelRegions>>,
    player_q: Query<(&Transform, &Player)>,
    // zombies: Query<(&Transform, &ViewVisibility), With<Zombie>>,
    // materials: Query<(&Transform, &ViewVisibility), With<LootMaterial>>,
    // weapons: Query<(&Transform, &ViewVisibility), With<LootWeapon>>,
    settlements: Query<(&Transform, &Settlement)>,
    font: Res<UiFont>,
    time: Res<Time<Real>>,
    mut timer: ResMut<MinimapTimer>,
    world_epoch: Res<WorldEpoch>,
    mut label_state: ResMut<MinimapSurfaceLabelState>,
) {
    let timer_finished = timer.0.tick(time.delta()).just_finished();
    if !timer_finished {
        return;
    }
    let Ok((minimap_entity, minimap_node, minimap_transform)) = minimap_q.single() else {
        return;
    };
    let Ok((player_tf, player)) = player_q.single() else {
        return;
    };
    for e in &stale {
        commands.entity(e).despawn();
    }

    let player_pos = player_tf.translation;
    let player_up = player_pos.normalize();
    let heading = (player.heading - player_up * player.heading.dot(player_up)).normalize();
    let cursor_physical = windows
        .single()
        .ok()
        .and_then(Window::physical_cursor_position);
    let minimap_content = map_content_geometry(
        minimap_node.size(),
        minimap_node.border(),
        minimap_node.inverse_scale_factor(),
    );
    let minimap_size = minimap_content.size.x;
    let minimap_cursor = cursor_physical
        .and_then(|cursor| cursor_in_map(cursor, *minimap_transform, minimap_content));
    let size = minimap_size;
    let up = player_up;
    let north = heading;
    let map_cursor = minimap_cursor;
    let radius = size / 2.0;
    let marker_scale = (size / MINIMAP_SIZE).clamp(1.0, 2.0);
    let east = north.cross(up).normalize();
    let place = |world_pos: Vec3| -> Option<Vec2> {
        if player_pos.distance(world_pos) > VIEW_RADIUS {
            return None;
        }
        let direction = world_pos.normalize();
        let x = direction.dot(east) * PLANET_RADIUS / VIEW_RADIUS;
        let y = direction.dot(north) * PLANET_RADIUS / VIEW_RADIUS;
        Some(Vec2::new(radius + x * radius, radius - y * radius))
    };
    let mut labels = Vec::new();
    let mut hover_labels = Vec::new();
    {
        let mut dot = |p: Vec3, kind: PlanetMarkerKind, s: f32| {
            let Some(pt) = place(p) else { return };
            let (node, color, border) = map_marker_components(pt, s, kind);
            commands.spawn((
                node,
                color,
                border,
                ZIndex(1),
                GameplayHudElement::default(),
                MinimapDot,
                ChildOf(minimap_entity),
            ));
        };
        // for (tf, visible) in &materials {
        //     if visible.get() {
        //         dot(tf.translation, theme::TEXT_WEAK, DOT * marker_scale);
        //     }
        // }
        // for (tf, visible) in &weapons {
        //     if visible.get() {
        //         dot(tf.translation, theme::SUCCESS, DOT * marker_scale);
        //     }
        // }
        // for (tf, visible) in &zombies {
        //     if visible.get() {
        //         dot(tf.translation, theme::ERROR, DOT * marker_scale);
        //     }
        // }
        dot(
            player_pos,
            PlanetMarkerKind::Explorer,
            DOT * 2.0 * marker_scale,
        );
    }

    for (tf, _) in &settlements {
        let Some(pt) = place(tf.translation) else {
            continue;
        };
        let s = 6.0 * marker_scale;
        let (node, color, border) = map_marker_components(pt, s, PlanetMarkerKind::Settlement);
        commands.spawn((
            node,
            color,
            border,
            ZIndex(1),
            GameplayHudElement::default(),
            MinimapDot,
            ChildOf(minimap_entity),
        ));
    }

    if let Some(regions) = regions.as_deref() {
        for (region_index, region) in regions.regions.iter().enumerate() {
            if region.kind == RegionKind::Road
                && regions
                    .bridge_top_surfaces_by_name
                    .contains_key(&region.name)
            {
                continue;
            }
            let world_pos = Vec3::from_array(region.pos) * PLANET_RADIUS;
            let Some(pt) = place(world_pos) else {
                continue;
            };
            let marker_kind = map_region_marker_kind(region.kind);
            let display_name = if region.kind == RegionKind::Settlement {
                settlements
                    .iter()
                    .find(|(_, settlement)| settlement.name == region.name)
                    .map_or(region.name.as_str(), |(_, settlement)| {
                        settlement.name.as_str()
                    })
            } else {
                region.name.as_str()
            };
            let s = 3.0 * marker_scale;
            let (node, color, border) = map_marker_components(pt, s, marker_kind);
            commands.spawn((
                node,
                color,
                border,
                ZIndex(1),
                GameplayHudElement::default(),
                MinimapDot,
                ChildOf(minimap_entity),
            ));
            let label = MapLabel {
                anchor: pt,
                world_anchor: world_pos,
                marker_index: region_index,
                text: display_name.to_owned(),
                kind: marker_kind,
                font_size: MAP_LABEL_FONT_SIZE * marker_scale,
            };
            if map_label_default_visible(region.kind) {
                labels.push(label);
            } else if planet_marker_label_visible(marker_kind, true) {
                hover_labels.push(label);
            }
        }

        for (bridge_index, (name, triangles)) in
            regions.bridge_top_surfaces_by_name.iter().enumerate()
        {
            let Some(world_pos) = bridge_surface_centroid(triangles) else {
                continue;
            };
            let Some(pt) = place(world_pos) else {
                continue;
            };
            let s = 5.0 * marker_scale;
            let (node, color, border) = map_marker_components(pt, s, PlanetMarkerKind::Bridge);
            commands.spawn((
                node,
                color,
                border,
                ZIndex(1),
                GameplayHudElement::default(),
                MinimapDot,
                ChildOf(minimap_entity),
            ));
            if !planet_marker_label_visible(PlanetMarkerKind::Bridge, false)
                && planet_marker_label_visible(PlanetMarkerKind::Bridge, true)
            {
                hover_labels.push(MapLabel {
                    anchor: pt,
                    world_anchor: world_pos,
                    marker_index: regions.regions.len() + bridge_index,
                    text: name.clone(),
                    kind: PlanetMarkerKind::Bridge,
                    font_size: MAP_LABEL_FONT_SIZE * marker_scale,
                });
            }
        }
    }

    if let Some(index) = nearest_hovered_label(map_cursor, &hover_labels) {
        labels.push(hover_labels[index].clone());
    }
    label_state.world_epoch = Some(*world_epoch);
    label_state.labels = labels;

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
            TextColor(planet_compass_color(i == 0)),
            GameplayHudElement::default(),
            ZIndex(1),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(radius + a.cos() * ring - label_width / 2.0),
                top: Val::Px(radius - a.sin() * ring - label_height / 2.0),
                ..default()
            },
            CompassLabel,
            ChildOf(minimap_entity),
        ));
    }
}

fn minimap_label_material(
    cache: &mut MinimapSurfaceLabelAssets,
    materials: &mut Assets<StandardMaterial>,
    color: Color,
) -> Option<Handle<StandardMaterial>> {
    if let Some((_, handle)) = cache
        .materials
        .iter()
        .find(|(cached_color, _)| *cached_color == color)
    {
        return Some(handle.clone());
    }
    let atlas = cache.atlas.as_ref()?;
    let handle = surface_labels::surface_label_material(atlas, materials, color);
    cache.materials.push((color, handle.clone()));
    Some(handle)
}

#[allow(
    clippy::too_many_arguments,
    clippy::type_complexity,
    reason = "Bevy injects independent ECS assets and camera parameters"
)]
fn update_minimap_surface_labels(
    mut commands: Commands,
    state: Res<MinimapSurfaceLabelState>,
    world_epoch: Res<WorldEpoch>,
    regions: Option<Res<LevelRegions>>,
    terrain: Option<Res<TerrainGen>>,
    settlements: Query<&Settlement>,
    camera: Query<&Transform, With<MinimapCamera>>,
    mut cache: ResMut<MinimapSurfaceLabelAssets>,
    image_assets: Option<ResMut<Assets<Image>>>,
    material_assets: Option<ResMut<Assets<StandardMaterial>>>,
    mesh_assets: Option<ResMut<Assets<Mesh>>>,
    mut labels: Query<(
        Entity,
        &mut MinimapSurfaceLabelOwner,
        &mut SurfaceLabelRibbon,
        &Mesh3d,
        &mut MeshMaterial3d<StandardMaterial>,
        &mut Visibility,
    )>,
) {
    let hide_all = |labels: &mut Query<(
        Entity,
        &mut MinimapSurfaceLabelOwner,
        &mut SurfaceLabelRibbon,
        &Mesh3d,
        &mut MeshMaterial3d<StandardMaterial>,
        &mut Visibility,
    )>| {
        for (_, _, _, _, _, mut visibility) in labels.iter_mut() {
            *visibility = Visibility::Hidden;
        }
    };

    let Ok(camera_transform) = camera.single() else {
        hide_all(&mut labels);
        return;
    };
    if state.world_epoch != Some(*world_epoch) {
        hide_all(&mut labels);
        return;
    }
    let Some(regions) = regions.as_deref() else {
        hide_all(&mut labels);
        return;
    };
    let (Some(mut image_assets), Some(mut material_assets), Some(mut mesh_assets)) =
        (image_assets, material_assets, mesh_assets)
    else {
        hide_all(&mut labels);
        return;
    };

    let epoch_changed = cache.world_epoch != Some(*world_epoch);
    if epoch_changed {
        let stale = labels
            .iter_mut()
            .map(|(entity, ..)| entity)
            .collect::<Vec<_>>();
        for entity in stale {
            commands.entity(entity).despawn();
        }
        cache.world_epoch = Some(*world_epoch);
        cache.atlas = None;
        cache.materials.clear();
    }
    if cache.atlas.is_none() {
        let names = regions
            .regions
            .iter()
            .map(|region| region.name.clone())
            .chain(regions.bridge_top_surfaces_by_name.keys().cloned())
            .chain(settlements.iter().map(|settlement| settlement.name.clone()))
            .collect::<Vec<_>>();
        cache.atlas = surface_labels::build_glyph_atlas(names, &mut image_assets);
    }
    if cache.atlas.is_none() {
        hide_all(&mut labels);
        return;
    }

    let camera_rotation = camera_transform.rotation.normalize();
    if !camera_rotation.is_finite() {
        hide_all(&mut labels);
        return;
    }
    let desired_indices = state
        .labels
        .iter()
        .map(|label| label.marker_index)
        .collect::<std::collections::HashSet<_>>();
    let mut existing_by_index = HashMap::new();
    if !epoch_changed {
        for (entity, owner, _, _, _, mut visibility) in labels.iter_mut() {
            existing_by_index.insert(owner.marker_index, entity);
            if !desired_indices.contains(&owner.marker_index) {
                *visibility = Visibility::Hidden;
            }
        }
    }

    for label in &state.labels {
        let color = planet_marker_presentation(label.kind).color;
        let Some(material) = minimap_label_material(&mut cache, &mut material_assets, color) else {
            continue;
        };
        let Some(atlas) = cache.atlas.as_ref() else {
            continue;
        };
        let Some(entity) = existing_by_index.remove(&label.marker_index) else {
            let ribbon = build_minimap_surface_ribbon(
                atlas,
                &label.text,
                label.world_anchor,
                label.font_size,
                camera_transform,
                terrain.as_deref(),
            );
            let mesh = mesh_assets.add(surface_labels::ribbon_mesh(&ribbon, 1.0));
            commands.spawn((
                Mesh3d(mesh),
                MeshMaterial3d(material),
                Transform::default(),
                Visibility::Visible,
                RenderLayers::layer(surface_labels::MINIMAP_LABEL_LAYER),
                SurfaceLabelRibbon {
                    marker_index: label.marker_index,
                    view: SurfaceLabelView::Minimap,
                    label: label.text.clone(),
                },
                MinimapSurfaceLabelOwner {
                    marker_index: label.marker_index,
                    world_anchor: label.world_anchor,
                    kind: label.kind,
                    font_size: label.font_size,
                    camera_rotation,
                },
            ));
            continue;
        };

        let Ok((_, mut owner, mut ribbon, mesh, mut current_material, mut visibility)) =
            labels.get_mut(entity)
        else {
            continue;
        };
        let text_changed = ribbon.label != label.text;
        let anchor_changed = owner.world_anchor.distance_squared(label.world_anchor) > 1e-4;
        let style_changed = owner.kind != label.kind || owner.font_size != label.font_size;
        let heading_changed =
            owner.camera_rotation.angle_between(camera_rotation) > MAP_LABEL_ROTATION_EPSILON;
        if text_changed || anchor_changed || style_changed || heading_changed {
            let rebuilt = build_minimap_surface_ribbon(
                atlas,
                &label.text,
                label.world_anchor,
                label.font_size,
                camera_transform,
                terrain.as_deref(),
            );
            if let Some(mut mesh) = mesh_assets.get_mut(&mesh.0) {
                surface_labels::write_ribbon_mesh(&mut mesh, &rebuilt, 1.0);
            }
            owner.camera_rotation = camera_rotation;
        }
        if current_material.0 != material {
            *current_material = MeshMaterial3d(material);
        }
        ribbon.marker_index = label.marker_index;
        ribbon.view = SurfaceLabelView::Minimap;
        ribbon.label.clone_from(&label.text);
        owner.marker_index = label.marker_index;
        owner.world_anchor = label.world_anchor;
        owner.kind = label.kind;
        owner.font_size = label.font_size;
        *visibility = Visibility::Visible;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface_labels::{SurfaceLabelRibbon, SurfaceLabelView};
    use terra_world::level::{LevelData, RegionKind, RoadKind};

    #[test]
    fn minimap_camera_sees_terrain_and_its_own_ribbons_only() {
        let mut app = App::new();
        app.insert_resource(Assets::<Image>::default())
            .add_systems(Startup, setup_minimap);

        app.update();

        let world = app.world_mut();
        let mut cameras = world.query_filtered::<&RenderLayers, With<MinimapCamera>>();
        let layers = cameras.single(world).expect("minimap camera");
        assert_eq!(
            layers,
            &RenderLayers::from_layers(&[0, surface_labels::MINIMAP_LABEL_LAYER])
        );
        assert!(layers.intersects(&RenderLayers::layer(0)));
        assert!(layers.intersects(&RenderLayers::layer(surface_labels::MINIMAP_LABEL_LAYER)));
        assert!(!layers.intersects(&RenderLayers::layer(surface_labels::PLANET_LABEL_LAYER)));
    }

    #[test]
    fn minimap_draw_spawns_one_complete_visible_minimap_ribbon_for_a_long_name() {
        use std::time::Duration;

        let name = "Étoile des montagnes enneigées — Łódź";
        let mut app = App::new();
        app.insert_resource(Time::<Real>::default())
            .insert_resource(UiFont(Handle::default()))
            .insert_resource(WorldEpoch::new(1))
            .insert_resource(Assets::<Image>::default())
            .insert_resource(Assets::<Mesh>::default())
            .insert_resource(Assets::<StandardMaterial>::default())
            .insert_resource(MinimapSurfaceLabelState::default())
            .insert_resource(MinimapSurfaceLabelAssets::default())
            .insert_resource(LevelRegions {
                regions: vec![
                    terra_world::level::RegionData {
                        name: name.to_owned(),
                        pos: Vec3::Y.to_array(),
                        kind: RegionKind::Settlement,
                    },
                    terra_world::level::RegionData {
                        name: "Pine forest".to_owned(),
                        pos: (Vec3::Y + Vec3::Z * (40.0 / PLANET_RADIUS))
                            .normalize()
                            .to_array(),
                        kind: RegionKind::Forest,
                    },
                ],
                face_regions: terra_world::level::RegionMemberships::from_memberships(vec![]),
                settlements: vec![(name.to_owned(), terra_world::level::SettlementKind::Town)],
                bridge_top_surfaces_by_name: Default::default(),
            })
            .insert_resource(MinimapTimer(Timer::from_seconds(
                0.001,
                TimerMode::Repeating,
            )))
            .add_systems(
                Update,
                (
                    track_minimap_camera,
                    draw_overlay,
                    update_minimap_surface_labels,
                )
                    .chain(),
            );
        let window_entity = app.world_mut().spawn(Window::default()).id();
        app.world_mut().spawn((
            Minimap,
            ComputedNode {
                size: Vec2::splat(MINIMAP_SIZE),
                ..default()
            },
            UiGlobalTransform::default(),
        ));
        let player_position = Vec3::Y * PLANET_RADIUS;
        let player_entity = app
            .world_mut()
            .spawn((
                Transform::from_translation(player_position),
                Player {
                    fire_timer: Timer::default(),
                    damage: 0.0,
                    range: 0.0,
                    heading: Vec3::X,
                },
            ))
            .id();
        let camera_entity = app
            .world_mut()
            .spawn((
                Transform::from_xyz(0.0, PLANET_RADIUS + CAM_HEIGHT, 0.0),
                RenderLayers::from_layers(&[0, surface_labels::MINIMAP_LABEL_LAYER]),
                MinimapCamera,
            ))
            .id();

        app.update();
        app.world_mut()
            .resource_mut::<Time<Real>>()
            .advance_by(Duration::from_millis(50));
        app.world_mut().run_schedule(Update);
        assert_eq!(
            app.world()
                .resource::<MinimapSurfaceLabelState>()
                .labels
                .len(),
            1
        );
        let labels = minimap_ribbon_labels(app.world_mut());
        assert_eq!(labels.len(), 1, "the minimap label is a single 3D ribbon");
        assert_eq!(
            labels[0].1, name,
            "the complete Unicode name stays attached"
        );
        assert_eq!(labels[0].2, SurfaceLabelView::Minimap);
        assert_eq!(labels[0].3, Visibility::Visible);
        assert_eq!(
            labels[0].5,
            RenderLayers::layer(surface_labels::MINIMAP_LABEL_LAYER)
        );
        assert!(!labels.iter().any(|label| label.1 == "Pine forest"));
        assert!(
            !labels[0]
                .5
                .intersects(&RenderLayers::layer(surface_labels::PLANET_LABEL_LAYER))
        );
        let mut text_query = app.world_mut().query::<&Text>();
        let flat_duplicate = text_query.iter(app.world()).any(|text| text.0 == name);
        assert!(!flat_duplicate, "no flat UI label duplicates the ribbon");

        app.world_mut()
            .get_mut::<Window>(window_entity)
            .expect("fixture window")
            .set_cursor_position(Some(Vec2::new(8.0, 0.0)));
        app.world_mut()
            .resource_mut::<Time<Real>>()
            .advance_by(Duration::from_millis(50));
        app.world_mut().run_schedule(Update);
        assert!(
            minimap_ribbon_labels(app.world_mut())
                .iter()
                .any(|label| label.1 == "Pine forest")
        );
        app.world_mut()
            .get_mut::<Window>(window_entity)
            .expect("fixture window")
            .set_cursor_position(None);
        app.world_mut()
            .resource_mut::<Time<Real>>()
            .advance_by(Duration::from_millis(50));
        app.world_mut().run_schedule(Update);
        assert_eq!(minimap_ribbon_labels(app.world_mut()).len(), 1);

        let original_camera_rotation = labels[0].4;
        app.world_mut()
            .get_mut::<Player>(player_entity)
            .expect("fixture player")
            .heading = Vec3::Z;
        app.world_mut()
            .resource_mut::<Time<Real>>()
            .advance_by(Duration::from_millis(50));
        app.world_mut().run_schedule(Update);

        let after_turn = minimap_ribbon_labels(app.world_mut());
        let camera_rotation = app
            .world()
            .get::<Transform>(camera_entity)
            .expect("minimap camera")
            .rotation;
        assert!(original_camera_rotation.angle_between(camera_rotation) > 0.5);
        assert!(after_turn[0].4.angle_between(camera_rotation) < 1e-4);

        let last_rebuilt_rotation = after_turn[0].4;
        for step in 1..=10 {
            let angle = step as f32 * 0.001;
            app.world_mut()
                .get_mut::<Player>(player_entity)
                .expect("fixture player")
                .heading = Vec3::Z * angle.cos() + Vec3::X * angle.sin();
            app.world_mut()
                .resource_mut::<Time<Real>>()
                .advance_by(Duration::from_millis(50));
            app.world_mut().run_schedule(Update);
            if step == 1 {
                let first_small_turn = minimap_ribbon_labels(app.world_mut());
                assert!(last_rebuilt_rotation.angle_between(first_small_turn[0].4) < 1e-5);
            }
        }
        let after_slow_turn = minimap_ribbon_labels(app.world_mut());
        let final_camera_rotation = app
            .world()
            .get::<Transform>(camera_entity)
            .expect("minimap camera")
            .rotation;
        assert!(last_rebuilt_rotation.angle_between(after_slow_turn[0].4) > 0.004);
        assert!(after_slow_turn[0].4.angle_between(final_camera_rotation) < 0.004);
    }

    fn minimap_ribbon_labels(
        world: &mut World,
    ) -> Vec<(
        Entity,
        String,
        SurfaceLabelView,
        Visibility,
        Quat,
        RenderLayers,
    )> {
        let mut query = world.query::<(
            Entity,
            &SurfaceLabelRibbon,
            &MinimapSurfaceLabelOwner,
            &Visibility,
            &RenderLayers,
        )>();
        query
            .iter(world)
            .filter(|(_, ribbon, _, visibility, _)| {
                ribbon.view == SurfaceLabelView::Minimap && **visibility == Visibility::Visible
            })
            .map(|(entity, ribbon, owner, visibility, layers)| {
                (
                    entity,
                    ribbon.label.clone(),
                    ribbon.view,
                    *visibility,
                    owner.camera_rotation,
                    layers.clone(),
                )
            })
            .collect()
    }

    fn region_label(kind: RegionKind, anchor: Vec2) -> MapLabel {
        MapLabel {
            anchor,
            world_anchor: Vec3::Y * PLANET_RADIUS,
            marker_index: 0,
            text: format!("{kind:?}"),
            kind: map_region_marker_kind(kind),
            font_size: MAP_LABEL_FONT_SIZE,
        }
    }

    #[test]
    fn settlement_names_are_default_visible_and_other_region_names_are_hovered() {
        assert!(map_label_default_visible(RegionKind::Settlement));
        assert!(!map_label_default_visible(RegionKind::Road));
        assert!(!map_label_default_visible(RegionKind::Forest));
        assert!(!map_label_default_visible(RegionKind::Ocean));
    }

    #[test]
    fn map_hover_selects_the_nearest_region_marker_within_twelve_logical_pixels() {
        let labels = [
            region_label(RegionKind::Forest, Vec2::new(50.0, 50.0)),
            region_label(RegionKind::Road, Vec2::new(58.0, 50.0)),
        ];

        assert_eq!(
            nearest_hovered_label(Some(Vec2::new(53.0, 50.0)), &labels),
            Some(0)
        );
        assert_eq!(
            nearest_hovered_label(Some(Vec2::new(56.0, 50.0)), &labels),
            Some(1)
        );
        assert_eq!(
            nearest_hovered_label(Some(Vec2::new(80.0, 80.0)), &labels),
            None
        );
        assert_eq!(nearest_hovered_label(None, &labels), None);
    }

    #[test]
    fn map_cursor_uses_the_content_box_and_rejects_outside_the_circle() {
        let border = BorderRect {
            min_inset: Vec2::splat(6.0),
            max_inset: Vec2::splat(6.0),
        };
        let content = map_content_geometry(Vec2::splat(320.0), border, 0.5);
        let transform = UiGlobalTransform::from_translation(Vec2::new(200.0, 120.0));

        assert_eq!(content.size, Vec2::splat(154.0));
        assert_eq!(content.origin_from_center, Vec2::splat(77.0));
        assert_eq!(
            cursor_in_map(Vec2::new(200.0, 120.0), transform, content),
            Some(Vec2::splat(77.0))
        );
        assert_eq!(
            cursor_in_map(Vec2::new(200.0, -34.0), transform, content),
            Some(Vec2::new(77.0, 0.0))
        );
        assert_eq!(
            cursor_in_map(Vec2::new(46.0, -34.0), transform, content),
            None,
            "a cursor in the square corner outside the circular image is not a map hover"
        );
    }

    #[test]
    fn bridge_surface_centroid_weights_triangle_area_and_keeps_deck_height() {
        let triangles = [
            [[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            [[10.0, 0.0, 12.0], [16.0, 0.0, 12.0], [10.0, 1.0, 12.0]],
        ];

        let center = bridge_surface_centroid(&triangles).expect("non-empty bridge surface");

        assert!(center.distance(Vec3::new(9.166_667, 1.0 / 3.0, 9.0)) < 1e-5);
    }

    #[test]
    fn bridge_marker_position_preserves_the_generated_deck_height() {
        let fixture = seed_1337_map_surface_fixture();
        let deck = fixture
            .bridge_surfaces
            .get("Bridge 4")
            .expect("seed 1337 contains Bridge 4");
        let center = bridge_surface_centroid(deck).expect("Bridge 4 has a deck surface");

        let bridge_position = map_marker_position(center, Some(&fixture.terrain), false);
        let terrain_position = map_marker_position(center, Some(&fixture.terrain), true);

        assert_eq!(bridge_position, center);
        assert!(bridge_position.distance(terrain_position) > 0.1);
    }

    struct MapSurfaceFixture {
        level: LevelData,
        terrain: TerrainGen,
        bridge_surfaces: std::collections::BTreeMap<String, Vec<[[f32; 3]; 3]>>,
    }

    fn seed_1337_map_surface_fixture() -> MapSurfaceFixture {
        let level = LevelData::from_artifact_bytes(include_bytes!("../assets/level_1337.bin"))
            .expect("load generated level");
        let terrain = TerrainGen::from_field_with_settlement_config(
            level.seed,
            level.vert_elev.clone(),
            level.settlement_config,
        );
        let ground = terra_geometry::planet::PlanetMesh::new(
            level
                .terrain_tris
                .iter()
                .map(|triangle| triangle.map(Vec3::from_array))
                .collect(),
        );
        let bridge_surfaces = level
            .roads
            .iter()
            .filter(|road| road.kind == RoadKind::Bridge)
            .map(|road| {
                let span = road
                    .points
                    .iter()
                    .map(|point| SpherePos::new(Vec3::from_array(*point)))
                    .collect::<Vec<_>>();
                let geometry =
                    terra_geometry::roads::build_bridge_deck_geometry(&span, &ground, 4.0);
                (road.name.clone(), geometry.top_surface)
            })
            .collect();

        MapSurfaceFixture {
            level,
            terrain,
            bridge_surfaces,
        }
    }

    #[test]
    fn main_camera_center_ray_selects_the_visible_bridge_deck_at_non_unit_scale() {
        let MapSurfaceFixture {
            terrain,
            bridge_surfaces,
            ..
        } = seed_1337_map_surface_fixture();
        let deck = &bridge_surfaces["Bridge 4"];
        let triangle = deck[deck.len() / 2].map(Vec3::from_array);
        let deck_point = (triangle[0] + triangle[1] + triangle[2]) / 3.0;
        let deck_up = (triangle[1] - triangle[0])
            .cross(triangle[2] - triangle[0])
            .normalize();
        let deck_up = if deck_up.dot(deck_point) < 0.0 {
            -deck_up
        } else {
            deck_up
        };
        let tangent = deck_up.cross(Vec3::Y).normalize_or(Vec3::X);
        let camera_position = deck_point + deck_up * SURFACE_PICKING_ALTITUDE + tangent * 500.0;
        let size = UVec2::new(1280, 720);
        let scale_factor = 2.0;
        let viewport = bevy::camera::Viewport {
            physical_size: size,
            ..default()
        };
        let mut projection = Projection::Perspective(PerspectiveProjection::default());
        let mut camera = Camera {
            viewport: Some(viewport.clone()),
            ..default()
        };
        camera.computed.target_info = Some(bevy::camera::RenderTargetInfo {
            physical_size: size,
            scale_factor,
        });
        projection.update(size.x as f32 / scale_factor, size.y as f32 / scale_factor);
        camera.computed.clip_from_view = projection.get_clip_from_view();
        let camera_transform = GlobalTransform::from(
            Transform::from_translation(camera_position).looking_at(deck_point, deck_up),
        );

        let selected = map_camera_surface_hit(
            &camera,
            &camera_transform,
            size.as_vec2() * 0.5,
            &terrain,
            Some(&bridge_surfaces),
        )
        .expect("main-camera ray to the visible bridge hits a surface");

        assert_eq!(selected.surface, PlanetDestinationSurface::BridgeDeck);
        assert!(selected.position.distance(deck_point) < 0.02);
    }

    #[test]
    fn ready_main_camera_click_becomes_a_persistent_world_destination() {
        let MapSurfaceFixture {
            level,
            terrain,
            bridge_surfaces,
        } = seed_1337_map_surface_fixture();
        let deck = &bridge_surfaces["Bridge 4"];
        let triangle = deck[deck.len() / 2].map(Vec3::from_array);
        let deck_point = (triangle[0] + triangle[1] + triangle[2]) / 3.0;
        let deck_up = (triangle[1] - triangle[0])
            .cross(triangle[2] - triangle[0])
            .normalize();
        let deck_up = if deck_up.dot(deck_point) < 0.0 {
            -deck_up
        } else {
            deck_up
        };
        let tangent = deck_up.cross(Vec3::Y).normalize_or(Vec3::X);
        let camera_position = deck_point + deck_up * SURFACE_PICKING_ALTITUDE + tangent * 500.0;
        let size = UVec2::new(1280, 720);
        let viewport = bevy::camera::Viewport {
            physical_size: size,
            ..default()
        };
        let mut projection = Projection::Perspective(PerspectiveProjection::default());
        let mut camera = Camera {
            viewport: Some(viewport.clone()),
            ..default()
        };
        camera.computed.target_info = Some(bevy::camera::RenderTargetInfo {
            physical_size: size,
            scale_factor: 1.0,
        });
        projection.update(size.x as f32, size.y as f32);
        camera.computed.clip_from_view = projection.get_clip_from_view();
        let camera_transform =
            Transform::from_translation(camera_position).looking_at(deck_point, deck_up);

        let (mut app, _) = crate::exploration::tests::fixture();
        app.world_mut().insert_resource(terrain);
        app.world_mut().insert_resource(WorldEpoch::new(44));
        app.world_mut().insert_resource(LevelRegions {
            regions: level.regions.clone(),
            face_regions: terra_world::level::RegionMemberships::from_memberships(vec![]),
            settlements: level
                .settlements
                .iter()
                .map(|settlement| (settlement.name.clone(), settlement.kind))
                .collect(),
            bridge_top_surfaces_by_name: bridge_surfaces,
        });
        let camera_entity = app
            .world_mut()
            .spawn((
                camera,
                MainCamera,
                camera_transform,
                GlobalTransform::from(camera_transform),
            ))
            .id();
        app.world_mut()
            .resource_mut::<Exploration>()
            .set_planet_view_open(true);
        for _ in 0..24 {
            app.update();
        }
        assert!(app.world().resource::<Exploration>().planet_view_ready());

        app.world_mut()
            .entity_mut(camera_entity)
            .insert((camera_transform, GlobalTransform::from(camera_transform)));
        let cursor = size.as_vec2() * 0.5;
        assert!(
            app.world_mut()
                .resource_mut::<Exploration>()
                .request_planet_view_selection(cursor)
        );
        app.update();

        let selection = app
            .world()
            .resource::<Exploration>()
            .selected_planet_destination()
            .expect("visible deck click stores a destination");
        assert_eq!(selection.id.world_epoch().value(), 44);
        assert_eq!(selection.surface, PlanetDestinationSurface::BridgeDeck);
        assert!(selection.position.distance(deck_point) < 0.02);
        let selected_id = selection.id;

        let sky_transform = Transform::from_translation(camera_position)
            .looking_at(camera_position + deck_up, Vec3::Y);
        app.world_mut()
            .entity_mut(camera_entity)
            .insert((sky_transform, GlobalTransform::from(sky_transform)));
        assert!(
            app.world_mut()
                .resource_mut::<Exploration>()
                .request_planet_view_selection(cursor)
        );
        app.update();
        assert_eq!(
            app.world()
                .resource::<Exploration>()
                .selected_planet_destination()
                .map(|destination| destination.id),
            Some(selected_id),
            "a sky click keeps the selected destination"
        );
    }

    #[test]
    fn map_click_targets_visible_bridge_4_and_preserves_land_water_and_occlusion() {
        let MapSurfaceFixture {
            level,
            terrain,
            bridge_surfaces,
        } = seed_1337_map_surface_fixture();
        let bridge = level
            .roads
            .iter()
            .find(|road| road.name == "Bridge 4" && road.kind == RoadKind::Bridge)
            .expect("seed 1337 contains Bridge 4");
        let deck = &bridge_surfaces["Bridge 4"];
        let triangle = deck[deck.len() / 2].map(Vec3::from_array);
        let deck_point = (triangle[0] + triangle[1] + triangle[2]) / 3.0;
        let deck_up = (triangle[1] - triangle[0])
            .cross(triangle[2] - triangle[0])
            .normalize();
        let deck_up = if deck_up.dot(deck_point) < 0.0 {
            -deck_up
        } else {
            deck_up
        };
        let tangent = deck_up.cross(Vec3::Y).normalize_or(Vec3::X);
        let camera = deck_point + deck_up * SURFACE_PICKING_ALTITUDE + tangent * 500.0;
        let ray_direction = (deck_point - camera).normalize();

        let selected =
            map_click_surface_hit(camera, ray_direction, &terrain, Some(&bridge_surfaces))
                .expect("visible bridge ray has a surface hit");
        assert!(
            ray_direction.cross(deck_point.normalize()).length() > 0.05,
            "fixture ray must be oblique to the radial fallback"
        );
        assert!(
            selected.position.distance(deck_point) < 0.02,
            "visible Bridge 4 hit should use its generated deck triangle, got {:?} for {:?}",
            selected.position,
            deck_point
        );
        assert!(selected.normal.dot(deck_point.normalize()) > 0.7);

        let midpoint = SpherePos::new(Vec3::from_array(bridge.points[bridge.points.len() / 2]));
        let first = Vec3::from_array(bridge.points[0]);
        let last = Vec3::from_array(*bridge.points.last().unwrap());
        let forward = (last - first).reject_from(midpoint.0).normalize();
        let side = midpoint.0.cross(forward).normalize();
        let adjacent_water = (midpoint.0 + side * (12.0 / PLANET_RADIUS)).normalize();
        assert!(terrain.surface_radius(SpherePos::new(adjacent_water)) < PLANET_RADIUS);
        let water_hit = map_click_surface_hit(
            adjacent_water * (PLANET_RADIUS + SURFACE_PICKING_ALTITUDE),
            -adjacent_water,
            &terrain,
            Some(&bridge_surfaces),
        )
        .expect("adjacent ocean click has a surface hit");
        assert!(water_hit.position.distance(adjacent_water * PLANET_RADIUS) < 0.02);

        let land = SpherePos::new(Vec3::from_array(level.settlements[0].pos));
        let land_radius = terrain.surface_radius(land);
        assert!(land_radius > PLANET_RADIUS);
        let land_hit = map_click_surface_hit(
            land.0 * (land_radius + SURFACE_PICKING_ALTITUDE),
            -land.0,
            &terrain,
            Some(&bridge_surfaces),
        )
        .expect("ordinary terrain click has a surface hit");
        assert!(land_hit.position.distance(land.0 * land_radius) < 0.02);

        let radial = deck_point.normalize();
        let hidden_camera = -radial * (PLANET_RADIUS + SURFACE_PICKING_ALTITUDE);
        let hidden_ray = radial;
        let hidden_deck_distance = terra_geometry::planet::ray_triangle_intersection_distance(
            hidden_camera,
            hidden_ray,
            &triangle,
        )
        .expect("occlusion ray intersects the generated bridge deck");
        let hidden_hit =
            map_click_surface_hit(hidden_camera, hidden_ray, &terrain, Some(&bridge_surfaces))
                .expect("occlusion ray first hits the generated terrain");
        assert!(hidden_hit.distance < hidden_deck_distance);
        assert!(hidden_hit.position.dot(radial) < 0.0);
    }
}

/// Development capture opens Planet view through the normal gameplay camera owner.
#[cfg(feature = "asset-review")]
pub(crate) fn showcase_map(world: &mut World, open: bool, center: Vec3) {
    let _ = center;
    world
        .resource_mut::<crate::exploration::Exploration>()
        .set_planet_view_open(open);
}
