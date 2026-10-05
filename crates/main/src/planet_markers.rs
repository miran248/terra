use std::collections::{HashMap, HashSet};

use avian3d::prelude::Position;
use bevy::camera::CameraUpdateSystems;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::transform::TransformSystems;
use bevy::ui::FocusPolicy;
use bevy::window::PrimaryWindow;
use shared::level::RegionKind;
use shared::planet_markers::{
    PlanetMarkerKind, PlanetMarkerShape, cursor_hits_planet_marker,
    marker_clears_spherical_horizon, planet_marker_label_priority, planet_marker_label_visible,
    planet_marker_presentation, project_ndc_to_logical_viewport, same_surface_location,
};
use shared::planet_view::{planet_compass_color, planet_compass_orientation};
use shared::sphere::{PLANET_RADIUS, SpherePos};
use shared::state::AppState;
use shared::terrain::TerrainGen;

use crate::exploration::{
    Exploration, PlanetDestination, PlanetDestinationCollection, PlanetDestinationId,
    PlanetDestinationSurface,
};
use crate::map::{LevelRegions, MainCamera, Player, WorldEpoch};
use crate::ui::UiFont;

const MARKER_DOT_HIT_RADIUS: f32 = 16.0;
const MARKER_LABEL_FONT_SIZE: f32 = 12.0;
const MARKER_LABEL_OFFSET: Vec2 = Vec2::new(12.0, -8.0);
const MARKER_LABEL_HEIGHT: f32 = 18.0;
const SETTLEMENT_REGION_DEDUP_METERS: f32 = 90.0;
const BRIDGE_REGION_DEDUP_METERS: f32 = 140.0;
const MAX_MARKER_LABEL_CHARS: usize = 24;
const LABEL_GRID_CELL_SIZE: f32 = 64.0;
const COMPASS_RIM_GAP: f32 = 18.0;
const COMPASS_LABEL_WIDTH: f32 = 12.0;
const COMPASS_LABEL_HEIGHT: f32 = 14.0;
const COMPASS_VIEWPORT_MARGIN: f32 = 10.0;

#[derive(Clone, Debug)]
pub(crate) struct BridgeMarkerAnchor {
    pub index: u32,
    pub name: String,
    pub position: Vec3,
}

/// The map setup captures settlement directions and generated deck centers so
/// marker IDs and anchors come from the loaded world rather than UI entities.
#[derive(Resource, Default)]
pub(crate) struct PlanetMarkerAnchors {
    pub settlements: Vec<Vec3>,
    pub bridges: Vec<BridgeMarkerAnchor>,
}

#[derive(Clone)]
struct NamedPlaceMarker {
    destination: PlanetDestination,
    kind: PlanetMarkerKind,
    label: String,
}

#[derive(Resource, Default)]
struct PlanetMarkerData {
    world_epoch: Option<WorldEpoch>,
    named: Vec<NamedPlaceMarker>,
}

#[derive(Clone, Debug)]
pub(crate) struct ProjectedPlanetMarker {
    pub index: usize,
    pub destination: PlanetDestination,
    pub center: Vec2,
    pub label_bounds: Option<Rect>,
    pub priority: u8,
}

/// This cache is exactly the set of currently visible named-place hit targets.
/// Hidden or clipped anchors are absent, so click selection matches the screen.
#[derive(Resource, Default)]
pub(crate) struct PlanetMarkerProjection {
    world_epoch: Option<WorldEpoch>,
    scale_factor: f32,
    pub markers: Vec<ProjectedPlanetMarker>,
    screen_by_index: Vec<Option<ScreenNamedMarker>>,
    label_candidates: Vec<LabelCandidate>,
    accepted_labels: HashSet<LabelTarget>,
    accepted_bounds_by_cell: HashMap<(i32, i32), Vec<Rect>>,
}

#[derive(Resource, Default)]
struct HoveredPlanetMarker(Option<PlanetDestinationId>);

#[derive(Component)]
struct PlanetMarkerOverlayRoot;

#[derive(Component)]
struct NamedMarkerDot(usize);

#[derive(Component)]
struct NamedMarkerLabel(usize);

#[derive(Component)]
struct ExplorerDot;

#[derive(Component)]
struct ExplorerLabel;

#[derive(Component)]
struct DestinationDot;

#[derive(Component)]
struct DestinationLabel;

#[derive(Component)]
struct CardinalLabel(usize);

#[derive(Clone, Copy, Debug)]
pub(crate) struct ExplorerMarkerLayout {
    pub physical_center: Vec2,
    pub physical_size: Vec2,
    pub visible: bool,
}

/// Read the laid-out explorer dot as the UI renderer will draw it. The
/// acceptance diagnostic compares this position with the camera projection
/// from the same frame while following a moving body.
pub(crate) fn explorer_marker_layout(world: &mut World) -> Option<ExplorerMarkerLayout> {
    let mut query =
        world.query_filtered::<(&Node, &ComputedNode, &UiGlobalTransform), With<ExplorerDot>>();
    let (node, computed, transform) = query.iter(world).next()?;
    let (_, _, physical_center) = transform.to_scale_angle_translation();
    Some(ExplorerMarkerLayout {
        physical_center,
        physical_size: computed.size,
        visible: node.display != Display::None,
    })
}

pub(crate) struct PlanetMarkersPlugin;

impl Plugin for PlanetMarkersPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PlanetMarkerProjection>()
            .init_resource::<HoveredPlanetMarker>()
            .add_systems(
                OnEnter(AppState::Playing),
                setup_planet_markers.after(crate::map::setup_map),
            )
            .add_systems(OnExit(AppState::Playing), cleanup_planet_markers)
            .add_systems(
                PostUpdate,
                update_planet_marker_overlay
                    .after(CameraUpdateSystems)
                    .before(bevy::ui::UiSystems::Prepare)
                    .before(TransformSystems::Propagate)
                    .run_if(in_state(AppState::Playing)),
            );
    }
}

/// Marker-first selection for the same projected dots and labels shown by the
/// overlay. Returning true means the caller should skip bare-surface picking.
pub(crate) fn select_named_marker_at(
    state: &mut Exploration,
    world_epoch: WorldEpoch,
    projection: Option<&PlanetMarkerProjection>,
    physical_cursor: Vec2,
) -> bool {
    if !state.planet_view_ready() || !physical_cursor.is_finite() {
        return false;
    }
    let Some(projection) =
        projection.filter(|projection| projection.world_epoch == Some(world_epoch))
    else {
        return false;
    };
    let scale_factor = projection.scale_factor;
    if !scale_factor.is_finite() || scale_factor <= 0.0 {
        return false;
    }
    let cursor = physical_cursor / scale_factor;
    let Some((_, marker)) = projection
        .markers
        .iter()
        .filter(|marker| {
            cursor_hits_planet_marker(
                cursor,
                Some(marker.center),
                MARKER_DOT_HIT_RADIUS,
                marker.label_bounds,
            )
        })
        .map(|marker| (cursor.distance_squared(marker.center), marker))
        .min_by(|(left_distance, left), (right_distance, right)| {
            left_distance
                .total_cmp(right_distance)
                .then_with(|| left.priority.cmp(&right.priority))
                .then_with(|| left.index.cmp(&right.index))
        })
    else {
        return false;
    };

    state.select_planet_destination(marker.destination.clone());
    true
}

fn setup_planet_markers(
    mut commands: Commands,
    world_epoch: Res<WorldEpoch>,
    regions: Option<Res<LevelRegions>>,
    anchors: Option<Res<PlanetMarkerAnchors>>,
    terrain: Option<Res<TerrainGen>>,
    font: Res<UiFont>,
) {
    let marker_data = match (regions, anchors, terrain) {
        (Some(regions), Some(anchors), Some(terrain)) => {
            build_named_markers(*world_epoch, &regions, &anchors, |direction| {
                terrain.surface_radius(SpherePos::new(direction))
            })
        }
        _ => PlanetMarkerData {
            world_epoch: Some(*world_epoch),
            named: Vec::new(),
        },
    };

    let root = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                top: Val::Px(0.0),
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                display: Display::None,
                ..default()
            },
            GlobalZIndex(60),
            FocusPolicy::Pass,
            PlanetMarkerOverlayRoot,
        ))
        .id();

    commands.entity(root).with_children(|overlay| {
        for (index, marker) in marker_data.named.iter().enumerate() {
            let presentation = planet_marker_presentation(marker.kind);
            let dot_size = marker_dot_size(marker.kind);
            let border = if presentation.outlined {
                UiRect::all(Val::Px(1.0))
            } else {
                UiRect::all(Val::Px(0.0))
            };
            let border_radius = match presentation.shape {
                PlanetMarkerShape::Circle => BorderRadius::MAX,
                PlanetMarkerShape::Square => BorderRadius::ZERO,
            };
            overlay.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    width: Val::Px(dot_size),
                    height: Val::Px(dot_size),
                    border_radius,
                    border,
                    display: Display::None,
                    ..default()
                },
                BackgroundColor(presentation.color),
                BorderColor::all(shared::theme::INK),
                FocusPolicy::Pass,
                NamedMarkerDot(index),
            ));
            overlay.spawn((
                Text::new(marker.label.clone()),
                Node {
                    position_type: PositionType::Absolute,
                    display: Display::None,
                    ..default()
                },
                TextFont {
                    font: font.0.clone().into(),
                    font_size: MARKER_LABEL_FONT_SIZE.into(),
                    ..default()
                },
                TextColor(presentation.color),
                FocusPolicy::Pass,
                NamedMarkerLabel(index),
            ));
        }

        let explorer_presentation = planet_marker_presentation(PlanetMarkerKind::Explorer);
        overlay.spawn((
            Node {
                position_type: PositionType::Absolute,
                width: Val::Px(10.0),
                height: Val::Px(10.0),
                border_radius: match explorer_presentation.shape {
                    PlanetMarkerShape::Circle => BorderRadius::MAX,
                    PlanetMarkerShape::Square => BorderRadius::ZERO,
                },
                display: Display::None,
                ..default()
            },
            BackgroundColor(explorer_presentation.color),
            FocusPolicy::Pass,
            ExplorerDot,
        ));
        overlay.spawn((
            Text::new("YOU"),
            Node {
                position_type: PositionType::Absolute,
                display: Display::None,
                ..default()
            },
            TextFont {
                font: font.0.clone().into(),
                font_size: MARKER_LABEL_FONT_SIZE.into(),
                ..default()
            },
            TextColor(explorer_presentation.color),
            FocusPolicy::Pass,
            ExplorerLabel,
        ));
        let destination_presentation =
            planet_marker_presentation(PlanetMarkerKind::SelectedDestination);
        overlay.spawn((
            Node {
                position_type: PositionType::Absolute,
                width: Val::Px(12.0),
                height: Val::Px(12.0),
                border_radius: match destination_presentation.shape {
                    PlanetMarkerShape::Circle => BorderRadius::MAX,
                    PlanetMarkerShape::Square => BorderRadius::ZERO,
                },
                border: if destination_presentation.outlined {
                    UiRect::all(Val::Px(1.0))
                } else {
                    UiRect::all(Val::Px(0.0))
                },
                display: Display::None,
                ..default()
            },
            BackgroundColor(destination_presentation.color),
            BorderColor::all(shared::theme::INK),
            FocusPolicy::Pass,
            DestinationDot,
        ));
        overlay.spawn((
            Text::new("DESTINATION"),
            Node {
                position_type: PositionType::Absolute,
                display: Display::None,
                ..default()
            },
            TextFont {
                font: font.0.clone().into(),
                font_size: MARKER_LABEL_FONT_SIZE.into(),
                ..default()
            },
            TextColor(destination_presentation.color),
            FocusPolicy::Pass,
            DestinationLabel,
        ));

        for (index, name) in ["N", "S", "E", "W"].into_iter().enumerate() {
            overlay.spawn((
                Text::new(name),
                Node {
                    position_type: PositionType::Absolute,
                    display: Display::None,
                    ..default()
                },
                TextFont {
                    font: font.0.clone().into(),
                    font_size: 11.0.into(),
                    ..default()
                },
                TextColor(planet_compass_color(index == 0)),
                FocusPolicy::Pass,
                CardinalLabel(index),
            ));
        }
    });

    commands.insert_resource(marker_data);
    commands.insert_resource(PlanetMarkerProjection {
        world_epoch: Some(*world_epoch),
        markers: Vec::new(),
        ..default()
    });
    commands.insert_resource(HoveredPlanetMarker::default());
}

fn cleanup_planet_markers(
    mut commands: Commands,
    overlays: Query<Entity, With<PlanetMarkerOverlayRoot>>,
) {
    for entity in &overlays {
        commands.entity(entity).despawn();
    }
    commands.remove_resource::<PlanetMarkerData>();
    commands.remove_resource::<PlanetMarkerProjection>();
    commands.remove_resource::<PlanetMarkerAnchors>();
    commands.remove_resource::<HoveredPlanetMarker>();
}

fn build_named_markers(
    world_epoch: WorldEpoch,
    regions: &LevelRegions,
    anchors: &PlanetMarkerAnchors,
    mut surface_radius: impl FnMut(Vec3) -> f32,
) -> PlanetMarkerData {
    let mut named = Vec::new();
    for (index, (name, kind)) in regions.settlements.iter().enumerate() {
        let Some(direction) = anchors
            .settlements
            .get(index)
            .copied()
            .map(Vec3::normalize_or_zero)
            .filter(|direction| *direction != Vec3::ZERO)
        else {
            continue;
        };
        let position = direction * surface_radius(direction);
        if !position.is_finite() {
            continue;
        }
        let display = format!("{} · {name}", kind.name());
        named.push(NamedPlaceMarker {
            destination: PlanetDestination {
                id: PlanetDestinationId::collection(
                    world_epoch,
                    PlanetDestinationCollection::Settlement,
                    index as u32,
                ),
                position,
                surface: PlanetDestinationSurface::Terrain,
                display: display.clone(),
            },
            kind: PlanetMarkerKind::Settlement,
            label: truncate_marker_label(&display),
        });
    }

    for bridge in &anchors.bridges {
        if !bridge.position.is_finite() {
            continue;
        }
        let display = format!("Bridge · {}", bridge.name);
        named.push(NamedPlaceMarker {
            destination: PlanetDestination {
                id: PlanetDestinationId::collection(
                    world_epoch,
                    PlanetDestinationCollection::Bridge,
                    bridge.index,
                ),
                position: bridge.position,
                surface: PlanetDestinationSurface::BridgeDeck,
                display,
            },
            kind: PlanetMarkerKind::Bridge,
            label: truncate_marker_label(&bridge.name),
        });
    }

    let settlement_directions = anchors
        .settlements
        .iter()
        .map(|position| position.normalize_or_zero())
        .filter(|direction| *direction != Vec3::ZERO)
        .collect::<Vec<_>>();
    for (index, region) in regions.regions.iter().enumerate() {
        let direction = Vec3::from_array(region.pos).normalize_or_zero();
        if direction == Vec3::ZERO {
            continue;
        }
        if region.kind == RegionKind::Settlement
            && settlement_directions.iter().any(|settlement| {
                same_surface_location(
                    direction,
                    *settlement,
                    PLANET_RADIUS,
                    SETTLEMENT_REGION_DEDUP_METERS,
                )
            })
        {
            continue;
        }
        if region.kind == RegionKind::Road
            && anchors.bridges.iter().any(|bridge| {
                region.name == bridge.name
                    && same_surface_location(
                        direction,
                        bridge.position,
                        PLANET_RADIUS,
                        BRIDGE_REGION_DEDUP_METERS,
                    )
            })
        {
            continue;
        }

        let position = direction * surface_radius(direction);
        if !position.is_finite() {
            continue;
        }
        let display = format!("Region · {}", region.name);
        named.push(NamedPlaceMarker {
            destination: PlanetDestination {
                id: PlanetDestinationId::collection(
                    world_epoch,
                    PlanetDestinationCollection::Region,
                    index as u32,
                ),
                position,
                surface: PlanetDestinationSurface::Terrain,
                display,
            },
            kind: PlanetMarkerKind::Region(region.kind),
            label: truncate_marker_label(&region.name),
        });
    }

    PlanetMarkerData {
        world_epoch: Some(world_epoch),
        named,
    }
}

fn truncate_marker_label(label: &str) -> String {
    let mut characters = label.chars();
    let prefix = characters
        .by_ref()
        .take(MAX_MARKER_LABEL_CHARS)
        .collect::<String>();
    if characters.next().is_some() {
        let mut prefix = prefix
            .chars()
            .take(MAX_MARKER_LABEL_CHARS - 1)
            .collect::<String>();
        prefix.push('…');
        prefix
    } else {
        prefix
    }
}

#[derive(Clone, Copy)]
struct ScreenNamedMarker {
    index: usize,
    center: Vec2,
    label_bounds: Option<Rect>,
    color: Color,
}

#[derive(Clone, Copy)]
struct CompassLabelScreen {
    center: Vec2,
    opacity: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum LabelTarget {
    Named(usize),
    Explorer,
    Destination,
    Cardinal(usize),
}

struct LabelCandidate {
    target: LabelTarget,
    bounds: Rect,
    priority: u8,
}

#[derive(SystemParam)]
struct MarkerOverlayUi<'w, 's> {
    roots: Query<
        'w,
        's,
        &'static mut Node,
        (
            With<PlanetMarkerOverlayRoot>,
            Without<NamedMarkerDot>,
            Without<NamedMarkerLabel>,
            Without<ExplorerDot>,
            Without<ExplorerLabel>,
            Without<DestinationDot>,
            Without<DestinationLabel>,
            Without<CardinalLabel>,
        ),
    >,
    named_dots: Query<
        'w,
        's,
        (
            &'static mut Node,
            &'static mut BackgroundColor,
            &'static NamedMarkerDot,
        ),
        (
            Without<PlanetMarkerOverlayRoot>,
            Without<NamedMarkerLabel>,
            Without<ExplorerDot>,
            Without<ExplorerLabel>,
            Without<DestinationDot>,
            Without<DestinationLabel>,
            Without<CardinalLabel>,
        ),
    >,
    named_labels: Query<
        'w,
        's,
        (
            &'static mut Node,
            &'static mut TextColor,
            &'static NamedMarkerLabel,
        ),
        (
            Without<PlanetMarkerOverlayRoot>,
            Without<NamedMarkerDot>,
            Without<ExplorerDot>,
            Without<ExplorerLabel>,
            Without<DestinationDot>,
            Without<DestinationLabel>,
            Without<CardinalLabel>,
        ),
    >,
    explorer_dot: Query<
        'w,
        's,
        (&'static mut Node, &'static mut BackgroundColor),
        (
            With<ExplorerDot>,
            Without<PlanetMarkerOverlayRoot>,
            Without<NamedMarkerDot>,
            Without<NamedMarkerLabel>,
            Without<ExplorerLabel>,
            Without<DestinationDot>,
            Without<DestinationLabel>,
            Without<CardinalLabel>,
        ),
    >,
    explorer_label: Query<
        'w,
        's,
        (&'static mut Node, &'static mut TextColor),
        (
            With<ExplorerLabel>,
            Without<PlanetMarkerOverlayRoot>,
            Without<NamedMarkerDot>,
            Without<NamedMarkerLabel>,
            Without<ExplorerDot>,
            Without<DestinationDot>,
            Without<DestinationLabel>,
            Without<CardinalLabel>,
        ),
    >,
    destination_dot: Query<
        'w,
        's,
        (&'static mut Node, &'static mut BackgroundColor),
        (
            With<DestinationDot>,
            Without<PlanetMarkerOverlayRoot>,
            Without<NamedMarkerDot>,
            Without<NamedMarkerLabel>,
            Without<ExplorerDot>,
            Without<ExplorerLabel>,
            Without<DestinationLabel>,
            Without<CardinalLabel>,
        ),
    >,
    destination_label: Query<
        'w,
        's,
        (&'static mut Node, &'static mut Text, &'static mut TextColor),
        (
            With<DestinationLabel>,
            Without<PlanetMarkerOverlayRoot>,
            Without<NamedMarkerDot>,
            Without<NamedMarkerLabel>,
            Without<ExplorerDot>,
            Without<ExplorerLabel>,
            Without<DestinationDot>,
            Without<CardinalLabel>,
        ),
    >,
    cardinal_labels: Query<
        'w,
        's,
        (
            &'static mut Node,
            &'static mut TextColor,
            &'static CardinalLabel,
        ),
        (
            Without<PlanetMarkerOverlayRoot>,
            Without<NamedMarkerDot>,
            Without<NamedMarkerLabel>,
            Without<ExplorerDot>,
            Without<ExplorerLabel>,
            Without<DestinationDot>,
            Without<DestinationLabel>,
        ),
    >,
    sidebar_nodes: Query<
        'w,
        's,
        (
            &'static Node,
            &'static ComputedNode,
            &'static UiGlobalTransform,
        ),
        (
            With<crate::ui::Sidebar>,
            Without<PlanetMarkerOverlayRoot>,
            Without<NamedMarkerDot>,
            Without<NamedMarkerLabel>,
            Without<ExplorerDot>,
            Without<ExplorerLabel>,
            Without<DestinationDot>,
            Without<DestinationLabel>,
            Without<CardinalLabel>,
        ),
    >,
}

fn update_planet_marker_overlay(
    state: Res<Exploration>,
    data: Option<Res<PlanetMarkerData>>,
    mut projection: ResMut<PlanetMarkerProjection>,
    mut hovered: ResMut<HoveredPlanetMarker>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &Transform), With<MainCamera>>,
    explorer: Query<&Position, With<Player>>,
    mut ui: MarkerOverlayUi,
) {
    let visible = state.is_planet_view_active() && state.planet_view_interface_visible();
    for mut root in &mut ui.roots {
        let display = if visible {
            Display::Flex
        } else {
            Display::None
        };
        if root.display != display {
            root.display = display;
        }
    }
    let Some(data) = data else {
        projection.world_epoch = None;
        projection.markers.clear();
        hovered.0 = None;
        hide_all_marker_nodes(&mut ui);
        return;
    };
    let Some((camera, camera_transform)) = cameras.iter().next() else {
        projection.world_epoch = None;
        projection.markers.clear();
        hovered.0 = None;
        hide_all_marker_nodes(&mut ui);
        return;
    };
    let Some(viewport) = camera.logical_viewport_rect() else {
        projection.world_epoch = None;
        projection.markers.clear();
        hovered.0 = None;
        hide_all_marker_nodes(&mut ui);
        return;
    };
    // MainCamera is a root entity. Its Transform already contains the pose
    // written by exploration/view::camera in Update; its GlobalTransform will
    // not propagate until after this frame's UI layout.
    let camera_global = GlobalTransform::from(*camera_transform);
    let camera_position = camera_transform.translation;
    let alpha = state.planet_view_interface_opacity().clamp(0.0, 1.0);
    let scale_factor = camera.target_scaling_factor().unwrap_or(1.0);
    let cursor = windows.single().ok().and_then(|window| {
        window
            .physical_cursor_position()
            .map(|physical| physical / scale_factor.max(f32::EPSILON))
    });
    let compass_viewport =
        ui.sidebar_nodes
            .iter()
            .fold(viewport, |mut visible, (node, computed, transform)| {
                if node.display == Display::None || computed.is_empty() {
                    return visible;
                }
                let sidebar = ui_node_bounds_logical(*computed, *transform);
                let covers_full_height =
                    sidebar.min.y <= viewport.min.y && sidebar.max.y >= viewport.max.y;
                let covers_left_edge =
                    sidebar.min.x <= viewport.min.x && sidebar.max.x > viewport.min.x;
                if covers_full_height && covers_left_edge {
                    visible.min.x = visible.min.x.max(sidebar.max.x);
                }
                visible
            });

    let mut screen_named = std::mem::take(&mut projection.screen_by_index);
    screen_named.resize(data.named.len(), None);
    screen_named.fill(None);
    if visible {
        for (index, marker) in data.named.iter().enumerate() {
            let Some(center) = project_anchor(
                camera,
                &camera_global,
                viewport,
                camera_position,
                marker.destination.position,
            ) else {
                continue;
            };
            screen_named[index] = Some(ScreenNamedMarker {
                index,
                center,
                label_bounds: marker_label_bounds(center, &marker.label, viewport),
                color: planet_marker_presentation(marker.kind).color,
            });
        }
    }

    let hovered_id = cursor.and_then(|cursor| {
        screen_named
            .iter()
            .flatten()
            .filter_map(|screen| {
                let marker = &data.named[screen.index];
                let label = previously_visible_label_bounds(
                    &projection,
                    data.world_epoch,
                    screen.index,
                    marker.destination.id,
                );
                cursor_hits_planet_marker(cursor, Some(screen.center), MARKER_DOT_HIT_RADIUS, label)
                    .then_some((cursor.distance_squared(screen.center), screen.index))
            })
            .min_by(
                |(left_distance, left_index), (right_distance, right_index)| {
                    left_distance
                        .total_cmp(right_distance)
                        .then_with(|| left_index.cmp(right_index))
                },
            )
            .map(|(_, index)| data.named[index].destination.id)
    });
    hovered.0 = hovered_id;

    let player_screen = if visible {
        explorer
            .iter()
            .next()
            .map(|position| position.0)
            .and_then(|position| {
                project_anchor(camera, &camera_global, viewport, camera_position, position)
            })
    } else {
        None
    };
    let selected = state.selected_planet_destination();
    let selection_screen = selected.and_then(|destination| {
        visible
            .then(|| {
                project_anchor(
                    camera,
                    &camera_global,
                    viewport,
                    camera_position,
                    destination.position,
                )
            })
            .flatten()
    });
    let cardinal_screens = if visible {
        if compass_viewport.size().x >= COMPASS_LABEL_WIDTH + 2.0 * COMPASS_VIEWPORT_MARGIN
            && compass_viewport.size().y >= COMPASS_LABEL_HEIGHT + 2.0 * COMPASS_VIEWPORT_MARGIN
        {
            planet_compass_screen_labels(
                camera,
                &camera_global,
                camera_position,
                camera_transform.rotation,
                viewport,
                compass_viewport,
            )
        } else {
            [None; 4]
        }
    } else {
        [None; 4]
    };

    let mut label_candidates = std::mem::take(&mut projection.label_candidates);
    label_candidates.clear();
    let explorer_kind = PlanetMarkerKind::Explorer;
    let explorer_bounds = player_screen
        .filter(|_| planet_marker_label_visible(explorer_kind, false))
        .and_then(|center| marker_label_bounds(center, "YOU", viewport));
    if let Some(bounds) = explorer_bounds {
        label_candidates.push(LabelCandidate {
            target: LabelTarget::Explorer,
            bounds,
            priority: planet_marker_label_priority(explorer_kind, false).unwrap_or(0),
        });
    }
    let destination_text = selected.map(|destination| truncate_marker_label(&destination.display));
    let destination_kind = PlanetMarkerKind::SelectedDestination;
    let destination_bounds = selection_screen
        .filter(|_| planet_marker_label_visible(destination_kind, false))
        .zip(destination_text.as_deref())
        .and_then(|(center, text)| marker_label_bounds(center, text, viewport));
    if let Some(bounds) = destination_bounds {
        label_candidates.push(LabelCandidate {
            target: LabelTarget::Destination,
            bounds,
            priority: planet_marker_label_priority(destination_kind, false).unwrap_or(1),
        });
    }
    for screen in screen_named.iter().flatten() {
        let marker = &data.named[screen.index];
        let is_hovered = hovered.0 == Some(marker.destination.id);
        if !planet_marker_label_visible(marker.kind, is_hovered) {
            continue;
        }
        let Some(bounds) = screen.label_bounds else {
            continue;
        };
        let Some(priority) = planet_marker_label_priority(marker.kind, is_hovered) else {
            continue;
        };
        label_candidates.push(LabelCandidate {
            target: LabelTarget::Named(screen.index),
            bounds,
            priority,
        });
    }
    for (index, screen) in cardinal_screens.iter().enumerate() {
        let Some(screen) = screen else { continue };
        let Some(bounds) = compass_label_bounds(screen.center, compass_viewport) else {
            continue;
        };
        label_candidates.push(LabelCandidate {
            target: LabelTarget::Cardinal(index),
            bounds,
            priority: 5,
        });
    }
    let mut accepted_labels = std::mem::take(&mut projection.accepted_labels);
    accepted_labels.clear();
    let mut accepted_bounds_by_cell = std::mem::take(&mut projection.accepted_bounds_by_cell);
    for bounds in accepted_bounds_by_cell.values_mut() {
        bounds.clear();
    }
    for priority in 0..=5 {
        for candidate in label_candidates
            .iter()
            .filter(|candidate| candidate.priority == priority)
        {
            let min_cell = (candidate.bounds.min / LABEL_GRID_CELL_SIZE)
                .floor()
                .as_ivec2();
            let max_cell = (candidate.bounds.max / LABEL_GRID_CELL_SIZE)
                .floor()
                .as_ivec2();
            let overlaps = (min_cell.y..=max_cell.y).any(|y| {
                (min_cell.x..=max_cell.x).any(|x| {
                    accepted_bounds_by_cell.get(&(x, y)).is_some_and(|bounds| {
                        bounds
                            .iter()
                            .any(|bounds| rectangles_overlap(*bounds, candidate.bounds))
                    })
                })
            });
            if overlaps {
                continue;
            }
            for y in min_cell.y..=max_cell.y {
                for x in min_cell.x..=max_cell.x {
                    accepted_bounds_by_cell
                        .entry((x, y))
                        .or_default()
                        .push(candidate.bounds);
                }
            }
            accepted_labels.insert(candidate.target);
        }
    }

    for (mut node, mut color, dot) in &mut ui.named_dots {
        let Some(screen) = screen_named.get(dot.0).and_then(Option::as_ref) else {
            if node.display != Display::None {
                node.display = Display::None;
            }
            continue;
        };
        let display = if visible {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != display {
            node.display = display;
        }
        let size = marker_dot_size(data.named[dot.0].kind);
        let left = Val::Px(screen.center.x - size / 2.0);
        let top = Val::Px(screen.center.y - size / 2.0);
        if node.left != left {
            node.left = left;
        }
        if node.top != top {
            node.top = top;
        }
        let marker_color = screen.color.with_alpha(screen.color.alpha() * alpha);
        if color.0 != marker_color {
            color.0 = marker_color;
        }
    }
    for (mut node, mut color, label) in &mut ui.named_labels {
        let Some(screen) = screen_named.get(label.0).and_then(Option::as_ref) else {
            if node.display != Display::None {
                node.display = Display::None;
            }
            continue;
        };
        let marker = &data.named[label.0];
        let display = if visible && accepted_labels.contains(&LabelTarget::Named(label.0)) {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != display {
            node.display = display;
        }
        let left = Val::Px(screen.center.x + MARKER_LABEL_OFFSET.x);
        let top = Val::Px(screen.center.y + MARKER_LABEL_OFFSET.y);
        if node.left != left {
            node.left = left;
        }
        if node.top != top {
            node.top = top;
        }
        let marker_color = planet_marker_presentation(marker.kind).color;
        let marker_color = marker_color.with_alpha(marker_color.alpha() * alpha);
        if color.0 != marker_color {
            color.0 = marker_color;
        }
    }
    for (node, color) in &mut ui.explorer_dot {
        set_dynamic_marker(
            node,
            color,
            player_screen,
            10.0,
            planet_marker_presentation(PlanetMarkerKind::Explorer).color,
            alpha,
        );
    }
    for (node, color) in &mut ui.explorer_label {
        set_dynamic_label(
            node,
            color,
            player_screen,
            accepted_labels.contains(&LabelTarget::Explorer),
            planet_marker_presentation(PlanetMarkerKind::Explorer).color,
            alpha,
            MARKER_LABEL_OFFSET,
        );
    }
    for (node, color) in &mut ui.destination_dot {
        set_dynamic_marker(
            node,
            color,
            selection_screen,
            12.0,
            planet_marker_presentation(PlanetMarkerKind::SelectedDestination).color,
            alpha,
        );
    }
    for (node, mut text, color) in &mut ui.destination_label {
        let label = destination_text.as_deref();
        if let Some(label) = label
            && text.0 != label
        {
            text.0 = label.to_owned();
        }
        set_dynamic_label(
            node,
            color,
            selection_screen,
            accepted_labels.contains(&LabelTarget::Destination),
            planet_marker_presentation(PlanetMarkerKind::SelectedDestination).color,
            alpha,
            MARKER_LABEL_OFFSET,
        );
    }
    for (node, color, cardinal) in &mut ui.cardinal_labels {
        let compass_label = cardinal_screens[cardinal.0];
        let screen = compass_label.map(|label| label.center);
        let base_color = planet_compass_color(cardinal.0 == 0);
        let base_color = base_color
            .with_alpha(base_color.alpha() * compass_label.map_or(0.0, |label| label.opacity));
        set_dynamic_label(
            node,
            color,
            screen,
            accepted_labels.contains(&LabelTarget::Cardinal(cardinal.0)),
            base_color,
            alpha,
            Vec2::new(-COMPASS_LABEL_WIDTH / 2.0, -COMPASS_LABEL_HEIGHT / 2.0),
        );
    }

    projection.scale_factor = scale_factor;
    projection.world_epoch = data.world_epoch;
    projection.markers.clear();
    projection
        .markers
        .extend(screen_named.iter().flatten().map(|screen| {
            let marker = &data.named[screen.index];
            let is_hovered = hovered.0 == Some(marker.destination.id);
            let label_bounds = accepted_labels
                .contains(&LabelTarget::Named(screen.index))
                .then_some(screen.label_bounds)
                .flatten();
            ProjectedPlanetMarker {
                index: screen.index,
                destination: marker.destination.clone(),
                center: screen.center,
                label_bounds,
                priority: planet_marker_label_priority(marker.kind, is_hovered).unwrap_or(4),
            }
        }));
    projection.screen_by_index = screen_named;
    projection.label_candidates = label_candidates;
    projection.accepted_labels = accepted_labels;
    projection.accepted_bounds_by_cell = accepted_bounds_by_cell;
}

fn previously_visible_label_bounds(
    projection: &PlanetMarkerProjection,
    world_epoch: Option<WorldEpoch>,
    index: usize,
    destination_id: PlanetDestinationId,
) -> Option<Rect> {
    if projection.world_epoch != world_epoch {
        return None;
    }
    // Hover resolves before this frame's UI prepare, so these cached bounds
    // still match the last laid-out labels. The cache is index-ordered.
    let projected_index = projection
        .markers
        .binary_search_by_key(&index, |marker| marker.index)
        .ok()?;
    let marker = projection.markers.get(projected_index)?;
    (marker.destination.id == destination_id)
        .then_some(marker.label_bounds)
        .flatten()
}

#[allow(clippy::too_many_arguments)]
fn hide_all_marker_nodes(ui: &mut MarkerOverlayUi) {
    for (mut node, _, _) in ui.named_dots.iter_mut() {
        if node.display != Display::None {
            node.display = Display::None;
        }
    }
    for (mut node, _, _) in ui.named_labels.iter_mut() {
        if node.display != Display::None {
            node.display = Display::None;
        }
    }
    for (mut node, _) in ui.explorer_dot.iter_mut() {
        if node.display != Display::None {
            node.display = Display::None;
        }
    }
    for (mut node, _) in ui.explorer_label.iter_mut() {
        if node.display != Display::None {
            node.display = Display::None;
        }
    }
    for (mut node, _) in ui.destination_dot.iter_mut() {
        if node.display != Display::None {
            node.display = Display::None;
        }
    }
    for (mut node, _, _) in ui.destination_label.iter_mut() {
        if node.display != Display::None {
            node.display = Display::None;
        }
    }
    for (mut node, _, _) in ui.cardinal_labels.iter_mut() {
        if node.display != Display::None {
            node.display = Display::None;
        }
    }
}

fn project_anchor(
    camera: &Camera,
    camera_transform: &GlobalTransform,
    viewport: Rect,
    camera_position: Vec3,
    anchor: Vec3,
) -> Option<Vec2> {
    if !marker_clears_spherical_horizon(camera_position, anchor, PLANET_RADIUS) {
        return None;
    }
    let ndc = camera.world_to_ndc(camera_transform, anchor)?;
    project_ndc_to_logical_viewport(ndc, viewport)
}

fn marker_label_bounds(center: Vec2, label: &str, viewport: Rect) -> Option<Rect> {
    let width = (label.chars().count() as f32 * MARKER_LABEL_FONT_SIZE * 0.62).max(8.0);
    let min = center + MARKER_LABEL_OFFSET;
    let bounds = Rect::from_corners(min, min + Vec2::new(width, MARKER_LABEL_HEIGHT));
    (bounds.min.x >= viewport.min.x
        && bounds.min.y >= viewport.min.y
        && bounds.max.x <= viewport.max.x
        && bounds.max.y <= viewport.max.y)
        .then_some(bounds)
}

fn ui_node_bounds_logical(computed: ComputedNode, transform: UiGlobalTransform) -> Rect {
    let half_size = computed.size / 2.0;
    let affine = transform.affine();
    let corners = [
        Vec2::new(-half_size.x, -half_size.y),
        Vec2::new(-half_size.x, half_size.y),
        Vec2::new(half_size.x, -half_size.y),
        Vec2::new(half_size.x, half_size.y),
    ];
    let (min, max) = corners
        .into_iter()
        .map(|corner| affine.transform_point2(corner) * computed.inverse_scale_factor)
        .fold(
            (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY)),
            |(min, max), corner| (min.min(corner), max.max(corner)),
        );
    Rect::from_corners(min, max)
}

fn compass_label_bounds(center: Vec2, viewport: Rect) -> Option<Rect> {
    let half_size = Vec2::new(COMPASS_LABEL_WIDTH, COMPASS_LABEL_HEIGHT) / 2.0;
    let bounds = Rect::from_corners(center - half_size, center + half_size);
    (bounds.min.x >= viewport.min.x
        && bounds.min.y >= viewport.min.y
        && bounds.max.x <= viewport.max.x
        && bounds.max.y <= viewport.max.y)
        .then_some(bounds)
}

fn planet_compass_screen_labels(
    camera: &Camera,
    camera_transform: &GlobalTransform,
    camera_position: Vec3,
    camera_rotation: Quat,
    viewport: Rect,
    placement_viewport: Rect,
) -> [Option<CompassLabelScreen>; 4] {
    let Some(orientation) = planet_compass_orientation(camera_position, camera_rotation) else {
        return [None; 4];
    };
    let Some(center_ndc) = camera.world_to_ndc(camera_transform, Vec3::ZERO) else {
        return [None; 4];
    };
    let camera_distance = camera_position.length();
    if camera_distance <= PLANET_RADIUS || !camera_distance.is_finite() {
        return [None; 4];
    }

    let surface_up = camera_position / camera_distance;
    let camera_right = camera_rotation * Vec3::X;
    let tangent_right = (camera_right - surface_up * camera_right.dot(surface_up))
        .normalize_or(surface_up.any_orthonormal_vector());
    let radius_ratio = PLANET_RADIUS / camera_distance;
    let tangent_radius = PLANET_RADIUS * (1.0 - radius_ratio * radius_ratio).sqrt();
    let silhouette = surface_up * (PLANET_RADIUS * radius_ratio) + tangent_right * tangent_radius;
    let Some(silhouette_ndc) = camera.world_to_ndc(camera_transform, silhouette) else {
        return [None; 4];
    };
    let center = viewport_point_from_ndc(center_ndc, viewport);
    let silhouette_point = viewport_point_from_ndc(silhouette_ndc, viewport);
    let planet_screen_radius = center.distance(silhouette_point);
    if !center.is_finite() || !planet_screen_radius.is_finite() {
        return [None; 4];
    }

    let directions = [
        orientation.north,
        -orientation.north,
        orientation.east,
        -orientation.east,
    ];
    directions.map(|direction| {
        let max_rim_distance =
            distance_to_compass_viewport_edge(center, direction, placement_viewport)?;
        let radius = (planet_screen_radius + COMPASS_RIM_GAP).min(max_rim_distance);
        Some(CompassLabelScreen {
            center: center + direction * radius,
            opacity: orientation.opacity,
        })
    })
}

fn viewport_point_from_ndc(ndc: Vec3, viewport: Rect) -> Vec2 {
    let normalized = Vec2::new((ndc.x + 1.0) * 0.5, (1.0 - ndc.y) * 0.5);
    viewport.min + normalized * viewport.size()
}

fn distance_to_compass_viewport_edge(center: Vec2, direction: Vec2, viewport: Rect) -> Option<f32> {
    let min = viewport.min + Vec2::splat(COMPASS_VIEWPORT_MARGIN);
    let max = viewport.max - Vec2::splat(COMPASS_VIEWPORT_MARGIN);
    let x = if direction.x > f32::EPSILON {
        (max.x - center.x) / direction.x
    } else if direction.x < -f32::EPSILON {
        (min.x - center.x) / direction.x
    } else {
        f32::INFINITY
    };
    let y = if direction.y > f32::EPSILON {
        (max.y - center.y) / direction.y
    } else if direction.y < -f32::EPSILON {
        (min.y - center.y) / direction.y
    } else {
        f32::INFINITY
    };
    let distance = x.min(y);
    (distance.is_finite() && distance >= 0.0).then_some(distance)
}

fn rectangles_overlap(left: Rect, right: Rect) -> bool {
    left.min.x < right.max.x
        && left.max.x > right.min.x
        && left.min.y < right.max.y
        && left.max.y > right.min.y
}

fn marker_dot_size(kind: PlanetMarkerKind) -> f32 {
    match kind {
        PlanetMarkerKind::Explorer => 10.0,
        PlanetMarkerKind::SelectedDestination => 12.0,
        PlanetMarkerKind::Settlement => 7.0,
        PlanetMarkerKind::Region(_) => 5.0,
        PlanetMarkerKind::Bridge => 8.0,
    }
}

fn set_dynamic_marker(
    mut node: Mut<'_, Node>,
    mut color: Mut<'_, BackgroundColor>,
    screen: Option<Vec2>,
    size: f32,
    base_color: Color,
    alpha: f32,
) {
    let Some(screen) = screen else {
        if node.display != Display::None {
            node.display = Display::None;
        }
        return;
    };
    if node.display != Display::Flex {
        node.display = Display::Flex;
    }
    let left = Val::Px(screen.x - size / 2.0);
    let top = Val::Px(screen.y - size / 2.0);
    if node.left != left {
        node.left = left;
    }
    if node.top != top {
        node.top = top;
    }
    let color_value = base_color.with_alpha(base_color.alpha() * alpha);
    if color.0 != color_value {
        color.0 = color_value;
    }
}

fn set_dynamic_label(
    mut node: Mut<'_, Node>,
    mut color: Mut<'_, TextColor>,
    screen: Option<Vec2>,
    label_visible: bool,
    base_color: Color,
    alpha: f32,
    offset: Vec2,
) {
    let Some(screen) = screen.filter(|_| label_visible) else {
        if node.display != Display::None {
            node.display = Display::None;
        }
        return;
    };
    if node.display != Display::Flex {
        node.display = Display::Flex;
    }
    let left = Val::Px(screen.x + offset.x);
    let top = Val::Px(screen.y + offset.y);
    if node.left != left {
        node.left = left;
    }
    if node.top != top {
        node.top = top;
    }
    let color_value = base_color.with_alpha(base_color.alpha() * alpha);
    if color.0 != color_value {
        color.0 = color_value;
    }
}

#[cfg(test)]
mod tests {
    use avian3d::prelude::{LinearVelocity, Position};
    use bevy::camera::{CameraProjection, RenderTargetInfo, Viewport};
    use bevy::ecs::system::RunSystemOnce;
    use bevy::prelude::*;
    use bevy::state::app::{AppExtStates, StatesPlugin};
    use bevy::window::PrimaryWindow;
    use shared::level::{RegionData, RegionKind, RegionMemberships, SettlementKind};

    use super::{
        BridgeMarkerAnchor, CameraUpdateSystems, HoveredPlanetMarker, NamedMarkerDot,
        NamedMarkerLabel, NamedPlaceMarker, PlanetMarkerAnchors, PlanetMarkerData,
        PlanetMarkerProjection, ProjectedPlanetMarker, TransformSystems, build_named_markers,
        select_named_marker_at, update_planet_marker_overlay,
    };
    use crate::{
        exploration::{
            Exploration, Kind, PlanetDestination, PlanetDestinationCollection, PlanetDestinationId,
            PlanetDestinationSurface,
        },
        map::{LevelRegions, MainCamera, WorldEpoch},
    };
    use shared::planet_markers::PlanetMarkerKind;
    fn level_regions(
        regions: Vec<RegionData>,
        settlements: &[(&str, SettlementKind)],
    ) -> LevelRegions {
        LevelRegions {
            regions,
            face_regions: RegionMemberships::from_memberships(vec![]),
            settlements: settlements
                .iter()
                .map(|(name, kind)| ((*name).to_owned(), *kind))
                .collect(),
            bridge_top_surfaces_by_name: Default::default(),
        }
    }

    #[test]
    fn duplicate_place_names_keep_distinct_collection_ids() {
        let regions = level_regions(
            vec![],
            &[
                ("Twin Harbor", SettlementKind::Town),
                ("Twin Harbor", SettlementKind::Town),
            ],
        );
        let anchors = PlanetMarkerAnchors {
            settlements: vec![Vec3::Y, Vec3::X],
            bridges: vec![],
        };

        let markers = build_named_markers(WorldEpoch::new(31), &regions, &anchors, |_| 2100.0);

        assert_eq!(markers.named.len(), 2);
        assert_eq!(markers.named[0].destination.display, "Town · Twin Harbor");
        assert_eq!(
            markers.named[0].destination.display,
            markers.named[1].destination.display
        );
        assert_ne!(
            markers.named[0].destination.id,
            markers.named[1].destination.id
        );
        assert_eq!(markers.named[0].destination.position, Vec3::Y * 2100.0);
        assert_eq!(markers.named[1].destination.position, Vec3::X * 2100.0);
    }

    #[test]
    fn settlement_regions_and_bridge_regions_deduplicate_by_location() {
        let regions = level_regions(
            vec![
                RegionData {
                    name: "North Haven".into(),
                    pos: Vec3::Y.to_array(),
                    kind: RegionKind::Settlement,
                },
                RegionData {
                    name: "Aurora Span".into(),
                    pos: Vec3::NEG_Y.to_array(),
                    kind: RegionKind::Road,
                },
                RegionData {
                    name: "Aurora Span".into(),
                    pos: Vec3::X.to_array(),
                    kind: RegionKind::Road,
                },
                RegionData {
                    name: "Remote Settlement".into(),
                    pos: Vec3::Z.to_array(),
                    kind: RegionKind::Settlement,
                },
            ],
            &[("North Haven", SettlementKind::Village)],
        );
        let anchors = PlanetMarkerAnchors {
            settlements: vec![Vec3::Y],
            bridges: vec![BridgeMarkerAnchor {
                index: 14,
                name: "Aurora Span".into(),
                position: Vec3::X * 2100.0,
            }],
        };

        let markers = build_named_markers(WorldEpoch::new(32), &regions, &anchors, |_| 2100.0);
        let ids = markers
            .named
            .iter()
            .map(|marker| marker.destination.id)
            .collect::<Vec<_>>();

        assert_eq!(ids.len(), 4);
        assert_eq!(
            ids[0],
            PlanetDestinationId::collection(
                WorldEpoch::new(32),
                PlanetDestinationCollection::Settlement,
                0,
            )
        );
        assert_eq!(
            ids[1],
            PlanetDestinationId::collection(
                WorldEpoch::new(32),
                PlanetDestinationCollection::Bridge,
                14,
            )
        );
        assert_eq!(
            ids[2],
            PlanetDestinationId::collection(
                WorldEpoch::new(32),
                PlanetDestinationCollection::Region,
                1,
            )
        );
        assert_eq!(
            ids[3],
            PlanetDestinationId::collection(
                WorldEpoch::new(32),
                PlanetDestinationCollection::Region,
                3,
            )
        );
    }

    #[test]
    fn marker_click_uses_visible_projection_and_preserves_selection_when_hidden() {
        let (mut app, _) = crate::exploration::tests::fixture();
        app.world_mut()
            .resource_mut::<Exploration>()
            .set_planet_view_open(true);
        for _ in 0..24 {
            app.update();
        }
        let mut state = app.world_mut().resource_mut::<Exploration>();
        assert!(state.planet_view_ready());

        let epoch = WorldEpoch::new(33);
        let destination = PlanetDestination {
            id: PlanetDestinationId::collection(epoch, PlanetDestinationCollection::Settlement, 18),
            position: Vec3::Y * 2100.0,
            surface: PlanetDestinationSurface::Terrain,
            display: "Town · Twin Harbor".into(),
        };
        let mut projection = PlanetMarkerProjection {
            world_epoch: Some(epoch),
            scale_factor: 2.0,
            markers: vec![ProjectedPlanetMarker {
                index: 4,
                destination: destination.clone(),
                center: Vec2::new(100.0, 80.0),
                label_bounds: Some(Rect::from_corners(
                    Vec2::new(110.0, 72.0),
                    Vec2::new(190.0, 92.0),
                )),
                priority: 3,
            }],
            ..default()
        };
        assert!(select_named_marker_at(
            &mut state,
            epoch,
            Some(&projection),
            Vec2::new(280.0, 170.0),
        ));
        assert_eq!(
            state.selected_planet_destination(),
            Some(&destination),
            "physical pixels should map to the visible label at 2x DPI"
        );

        projection.markers.clear();
        assert!(!select_named_marker_at(
            &mut state,
            epoch,
            Some(&projection),
            Vec2::new(280.0, 170.0),
        ));
        assert_eq!(state.selected_planet_destination(), Some(&destination));

        assert!(select_named_marker_at(
            &mut state,
            epoch,
            Some(&PlanetMarkerProjection {
                world_epoch: Some(epoch),
                scale_factor: 2.0,
                markers: vec![ProjectedPlanetMarker {
                    index: 4,
                    destination: destination.clone(),
                    center: Vec2::new(100.0, 80.0),
                    label_bounds: None,
                    priority: 3,
                }],
                ..default()
            }),
            Vec2::new(200.0, 160.0),
        ));
        assert_eq!(state.selected_planet_destination(), Some(&destination));
    }

    #[test]
    fn marker_plugin_registers_and_runs_inside_playing_state() {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            TransformPlugin,
            StatesPlugin,
            super::PlanetMarkersPlugin,
        ))
        .init_state::<shared::state::AppState>()
        .insert_resource(Exploration::default())
        .insert_resource(WorldEpoch::new(34))
        .insert_resource(crate::ui::UiFont(default()));
        app.world_mut()
            .resource_mut::<bevy::prelude::NextState<shared::state::AppState>>()
            .set(shared::state::AppState::Playing);

        app.update();
        app.update();

        assert!(app.world().contains_resource::<PlanetMarkerProjection>());
    }

    #[test]
    fn explorer_dot_projects_the_synchronized_occupied_body_position() {
        let (mut app, explorer) = crate::exploration::tests::fixture();
        let car =
            crate::exploration::tests::summon_and_enter_vehicle(&mut app, explorer, Kind::Car);
        assert!(app.world().resource::<Exploration>().is_in_vehicle());
        assert_eq!(
            app.world()
                .resource::<Exploration>()
                .vehicle_entity(Kind::Car),
            Some(car)
        );

        app.world_mut().spawn((PrimaryWindow, Window::default()));
        let physical_size = UVec2::new(800, 600);
        let scale_factor = 2.0;
        let mut perspective = PerspectiveProjection {
            far: 20_000.0,
            ..default()
        };
        perspective.update(
            physical_size.x as f32 / scale_factor,
            physical_size.y as f32 / scale_factor,
        );
        let mut camera = Camera {
            viewport: Some(Viewport {
                physical_size,
                ..default()
            }),
            ..default()
        };
        camera.computed.target_info = Some(RenderTargetInfo {
            physical_size,
            scale_factor,
        });
        camera.computed.clip_from_view = perspective.get_clip_from_view();
        let initial_camera = Transform::from_xyz(0.0, 3_600.0, 0.0).looking_at(Vec3::ZERO, Vec3::Z);
        app.world_mut().spawn((
            MainCamera,
            camera,
            Projection::Perspective(perspective),
            initial_camera,
            GlobalTransform::from(initial_camera),
        ));
        app.world_mut()
            .resource_mut::<Exploration>()
            .set_planet_view_open(true);
        for _ in 0..24 {
            app.update();
        }
        assert!(
            app.world()
                .resource::<Exploration>()
                .is_planet_view_active()
        );

        let body_position = Vec3::new(0.25, 0.968_245_8, 0.0) * shared::sphere::PLANET_RADIUS;
        let stale_player_transform = Vec3::Y * shared::sphere::PLANET_RADIUS;
        app.world_mut()
            .entity_mut(car)
            .insert((Position(body_position), LinearVelocity::ZERO));
        app.world_mut()
            .entity_mut(explorer)
            .insert(Position(body_position));
        app.world_mut()
            .entity_mut(explorer)
            .insert(Transform::from_translation(stale_player_transform));

        let explorer_dot = app
            .world_mut()
            .spawn((
                Node {
                    position_type: PositionType::Absolute,
                    width: Val::Px(10.0),
                    height: Val::Px(10.0),
                    display: Display::None,
                    ..default()
                },
                BackgroundColor(Color::WHITE),
                super::ExplorerDot,
            ))
            .id();
        app.world_mut().insert_resource(PlanetMarkerData::default());
        app.world_mut()
            .insert_resource(PlanetMarkerProjection::default());
        app.world_mut()
            .insert_resource(HoveredPlanetMarker::default());

        app.world_mut()
            .run_system_once(update_planet_marker_overlay)
            .unwrap();

        let (current_center, stale_center) = {
            let world = app.world_mut();
            let mut query = world.query_filtered::<(&Camera, &Transform), With<MainCamera>>();
            let (camera, camera_transform) = query.iter(world).next().unwrap();
            let viewport = camera.logical_viewport_rect().unwrap();
            let camera_global = GlobalTransform::from(*camera_transform);
            let current_center = super::project_anchor(
                camera,
                &camera_global,
                viewport,
                camera_transform.translation,
                body_position,
            )
            .unwrap();
            let stale_center = super::project_anchor(
                camera,
                &camera_global,
                viewport,
                camera_transform.translation,
                stale_player_transform,
            )
            .unwrap();
            (current_center, stale_center)
        };
        assert!(current_center.distance(stale_center) > 1.0);

        let node = app.world().get::<Node>(explorer_dot).unwrap();
        let actual_center = Vec2::new(
            match node.left {
                Val::Px(value) => value + 5.0,
                other => panic!("expected pixel left, got {other:?}"),
            },
            match node.top {
                Val::Px(value) => value + 5.0,
                other => panic!("expected pixel top, got {other:?}"),
            },
        );
        assert!(
            actual_center.distance(current_center) <= 0.01,
            "occupied explorer dot {actual_center:?} should follow current body projection {current_center:?}, not stale player transform {stale_center:?}"
        );
    }

    #[test]
    fn hover_does_not_open_a_collision_suppressed_label() {
        let (mut app, _) = crate::exploration::tests::fixture();
        app.world_mut()
            .resource_mut::<Exploration>()
            .set_planet_view_open(true);
        for _ in 0..24 {
            app.update();
        }
        assert!(app.world().resource::<Exploration>().planet_view_ready());
        app.world_mut()
            .insert_resource(State::new(shared::state::AppState::Loading));

        let epoch = WorldEpoch::new(36);
        let settlement = PlanetDestination {
            id: PlanetDestinationId::collection(epoch, PlanetDestinationCollection::Settlement, 0),
            position: Vec3::Z * shared::sphere::PLANET_RADIUS,
            surface: PlanetDestinationSurface::Terrain,
            display: "Settlement · S".into(),
        };
        let region = PlanetDestination {
            id: PlanetDestinationId::collection(epoch, PlanetDestinationCollection::Region, 0),
            position: Vec3::Z * shared::sphere::PLANET_RADIUS,
            surface: PlanetDestinationSurface::Terrain,
            display: "Region · A faraway named region".into(),
        };
        app.world_mut().insert_resource(PlanetMarkerData {
            world_epoch: Some(epoch),
            named: vec![
                NamedPlaceMarker {
                    destination: settlement,
                    kind: PlanetMarkerKind::Settlement,
                    label: "S".into(),
                },
                NamedPlaceMarker {
                    destination: region,
                    kind: PlanetMarkerKind::Region(RegionKind::Forest),
                    label: "A faraway named region".into(),
                },
            ],
        });
        app.world_mut()
            .insert_resource(PlanetMarkerProjection::default());
        app.world_mut()
            .insert_resource(HoveredPlanetMarker::default());

        let mut window = Window::default();
        // The labels share an anchor. This cursor point lies in only the long
        // region label's theoretical bounds, beyond the visible short label.
        window.set_cursor_position(None);
        let window_entity = app.world_mut().spawn((PrimaryWindow, window)).id();

        let size = UVec2::new(800, 600);
        let mut perspective = PerspectiveProjection {
            far: 20_000.0,
            ..default()
        };
        perspective.update(size.x as f32, size.y as f32);
        let mut camera = Camera {
            viewport: Some(Viewport {
                physical_size: size,
                ..default()
            }),
            ..default()
        };
        camera.computed.target_info = Some(RenderTargetInfo {
            physical_size: size,
            scale_factor: 1.0,
        });
        camera.computed.clip_from_view = perspective.get_clip_from_view();
        let camera_transform =
            Transform::from_xyz(0.0, 0.0, 3_600.0).looking_at(Vec3::ZERO, Vec3::Y);
        app.world_mut().spawn((
            MainCamera,
            camera,
            Projection::Perspective(perspective),
            camera_transform,
            GlobalTransform::from(camera_transform),
        ));

        for (index, label) in ["S", "A faraway named region"].into_iter().enumerate() {
            app.world_mut().spawn((
                Node {
                    position_type: PositionType::Absolute,
                    width: Val::Px(7.0),
                    height: Val::Px(7.0),
                    display: Display::None,
                    ..default()
                },
                BackgroundColor(Color::WHITE),
                NamedMarkerDot(index),
            ));
            app.world_mut().spawn((
                Text::new(label),
                Node {
                    position_type: PositionType::Absolute,
                    display: Display::None,
                    ..default()
                },
                TextColor(Color::WHITE),
                NamedMarkerLabel(index),
            ));
        }
        app.add_systems(
            PostUpdate,
            update_planet_marker_overlay
                .after(CameraUpdateSystems)
                .before(bevy::ui::UiSystems::Prepare)
                .before(TransformSystems::Propagate),
        );

        app.update();
        let previous_projection = app.world().resource::<PlanetMarkerProjection>();
        assert_eq!(previous_projection.markers.len(), 2);
        assert!(previous_projection.markers[0].label_bounds.is_some());
        assert_eq!(previous_projection.markers[1].label_bounds, None);

        app.world_mut()
            .get_mut::<Window>(window_entity)
            .unwrap()
            .set_cursor_position(Some(Vec2::new(500.0, 300.0)));
        app.update();

        assert_eq!(
            app.world().resource::<HoveredPlanetMarker>().0,
            None,
            "a suppressed label's invisible text area must not create a hover target"
        );
        let mut labels = app.world_mut().query::<(&NamedMarkerLabel, &Node)>();
        assert_eq!(
            labels
                .iter(app.world())
                .find(|(label, _)| label.0 == 1)
                .unwrap()
                .1
                .display,
            Display::None
        );

        let settlement_id = app.world().resource::<PlanetMarkerData>().named[0]
            .destination
            .id;
        let settlement_dot = app.world().resource::<PlanetMarkerProjection>().markers[0].center;
        app.world_mut()
            .get_mut::<Window>(window_entity)
            .unwrap()
            .set_cursor_position(Some(settlement_dot));
        app.update();
        assert_eq!(
            app.world().resource::<HoveredPlanetMarker>().0,
            Some(settlement_id),
            "dot hit targets stay interactive even when a label was suppressed"
        );
    }

    #[test]
    fn same_frame_camera_pose_updates_marker_node_and_hit_cache_before_ui_layout() {
        let (mut app, _) = crate::exploration::tests::fixture();
        let epoch = WorldEpoch::new(35);
        let direction = Vec3::Z;
        let marker = PlanetDestination {
            id: PlanetDestinationId::collection(epoch, PlanetDestinationCollection::Settlement, 0),
            position: direction * shared::sphere::PLANET_RADIUS,
            surface: PlanetDestinationSurface::Terrain,
            display: "Town · Center".into(),
        };
        app.world_mut().insert_resource(PlanetMarkerData {
            world_epoch: Some(epoch),
            named: vec![NamedPlaceMarker {
                destination: marker,
                kind: PlanetMarkerKind::Settlement,
                label: "Town · Center".into(),
            }],
        });
        app.world_mut()
            .insert_resource(PlanetMarkerProjection::default());
        app.world_mut()
            .insert_resource(HoveredPlanetMarker::default());

        let size = UVec2::new(800, 600);
        let scale_factor = 2.0;
        let mut perspective = PerspectiveProjection {
            far: 20_000.0,
            ..default()
        };
        perspective.update(size.x as f32 / scale_factor, size.y as f32 / scale_factor);
        let mut camera = Camera {
            viewport: Some(Viewport {
                physical_size: size,
                ..default()
            }),
            ..default()
        };
        camera.computed.target_info = Some(RenderTargetInfo {
            physical_size: size,
            scale_factor,
        });
        camera.computed.clip_from_view = perspective.get_clip_from_view();
        let initial_camera = Transform::from_xyz(0.0, 0.0, 3_000.0).looking_at(Vec3::ZERO, Vec3::Y);
        let camera_entity = app
            .world_mut()
            .spawn((
                MainCamera,
                camera,
                Projection::Perspective(perspective),
                initial_camera,
                GlobalTransform::from(initial_camera),
            ))
            .id();
        let dot = app
            .world_mut()
            .spawn((
                Node {
                    position_type: PositionType::Absolute,
                    width: Val::Px(7.0),
                    height: Val::Px(7.0),
                    display: Display::None,
                    ..default()
                },
                BackgroundColor(Color::WHITE),
                NamedMarkerDot(0),
            ))
            .id();
        app.add_systems(
            PostUpdate,
            update_planet_marker_overlay
                .after(CameraUpdateSystems)
                .before(bevy::ui::UiSystems::Prepare)
                .before(TransformSystems::Propagate),
        );

        app.world_mut()
            .resource_mut::<Exploration>()
            .set_planet_view_open(true);
        for _ in 0..24 {
            app.update();
        }
        assert!(app.world().resource::<Exploration>().planet_view_ready());

        // Freeze exploration's camera controller while leaving the UI overlay
        // active, then move only Transform. GlobalTransform will update later
        // in PostUpdate, after the marker and UI layout schedules.
        app.world_mut()
            .insert_resource(State::new(shared::state::AppState::Loading));
        let first_pose = Transform::from_xyz(0.0, 0.0, 3_000.0).looking_at(Vec3::ZERO, Vec3::Y);
        app.world_mut()
            .entity_mut(camera_entity)
            .insert((first_pose, GlobalTransform::from(first_pose)));
        app.update();
        let first_center = app.world().resource::<PlanetMarkerProjection>().markers[0].center;
        assert!(
            app.world()
                .resource::<PlanetMarkerProjection>()
                .scale_factor
                == scale_factor
        );

        let second_pose = Transform::from_xyz(900.0, 0.0, 2_860.0).looking_at(Vec3::ZERO, Vec3::Y);
        app.world_mut()
            .entity_mut(camera_entity)
            .insert(second_pose);
        app.update();

        let projection = app.world().resource::<PlanetMarkerProjection>();
        let second_center = projection.markers[0].center;
        assert!(first_center.distance(second_center) > 20.0);
        assert_eq!(projection.scale_factor, scale_factor);
        let node = app.world().get::<Node>(dot).unwrap();
        assert_eq!(node.display, Display::Flex);
        assert!(
            matches!(node.left, Val::Px(left) if (left - (second_center.x - 3.5)).abs() < 0.01)
        );
        assert!(matches!(node.top, Val::Px(top) if (top - (second_center.y - 3.5)).abs() < 0.01));
    }

    #[test]
    fn planet_compass_nodes_stay_outside_the_full_height_sidebar_at_near_zoom() {
        let (mut app, _) = crate::exploration::tests::fixture();
        let epoch = WorldEpoch::new(37);
        app.world_mut().insert_resource(PlanetMarkerData {
            world_epoch: Some(epoch),
            named: Vec::new(),
        });
        app.world_mut()
            .insert_resource(PlanetMarkerProjection::default());
        app.world_mut()
            .insert_resource(HoveredPlanetMarker::default());

        let scale_factor = 2.0;
        let physical_size = UVec2::new(2560, 1440);
        let mut perspective = PerspectiveProjection {
            far: 20_000.0,
            ..default()
        };
        perspective.update(
            physical_size.x as f32 / scale_factor,
            physical_size.y as f32 / scale_factor,
        );
        let mut camera = Camera {
            viewport: Some(Viewport {
                physical_size,
                ..default()
            }),
            ..default()
        };
        camera.computed.target_info = Some(RenderTargetInfo {
            physical_size,
            scale_factor,
        });
        camera.computed.clip_from_view = perspective.get_clip_from_view();
        let camera_pose = Transform::from_xyz(0.0, 0.0, shared::sphere::PLANET_RADIUS + 400.0)
            .looking_at(Vec3::ZERO, Vec3::Y);
        let camera_entity = app
            .world_mut()
            .spawn((
                MainCamera,
                camera,
                Projection::Perspective(perspective),
                camera_pose,
                GlobalTransform::from(camera_pose),
            ))
            .id();

        app.world_mut().spawn((
            crate::ui::Sidebar,
            Node {
                display: Display::Flex,
                ..default()
            },
            ComputedNode {
                size: Vec2::new(560.0, 1440.0),
                inverse_scale_factor: 0.5,
                ..default()
            },
            UiGlobalTransform::from_xy(280.0, 720.0),
        ));
        let labels = ["N", "S", "E", "W"].map(|letter| {
            app.world_mut()
                .spawn((
                    Text::new(letter),
                    Node {
                        position_type: PositionType::Absolute,
                        display: Display::None,
                        ..default()
                    },
                    TextColor(Color::WHITE),
                ))
                .id()
        });
        for (index, entity) in labels.into_iter().enumerate() {
            app.world_mut()
                .entity_mut(entity)
                .insert(super::CardinalLabel(index));
        }
        app.add_systems(
            PostUpdate,
            update_planet_marker_overlay
                .after(CameraUpdateSystems)
                .before(bevy::ui::UiSystems::Prepare)
                .before(TransformSystems::Propagate),
        );

        app.world_mut()
            .resource_mut::<Exploration>()
            .set_planet_view_open(true);
        for _ in 0..24 {
            app.update();
        }
        assert!(app.world().resource::<Exploration>().planet_view_ready());
        app.world_mut()
            .insert_resource(State::new(shared::state::AppState::Loading));
        app.world_mut()
            .entity_mut(camera_entity)
            .insert((camera_pose, GlobalTransform::from(camera_pose)));
        app.update();

        let mut lefts = [0.0; 4];
        let mut tops = [0.0; 4];
        for (index, entity) in labels.into_iter().enumerate() {
            let node = app.world().get::<Node>(entity).unwrap();
            assert_eq!(
                node.display,
                Display::Flex,
                "{} remains visible",
                ["N", "S", "E", "W"][index]
            );
            let Val::Px(left) = node.left else {
                panic!(
                    "{} has a computed horizontal placement",
                    ["N", "S", "E", "W"][index]
                );
            };
            let Val::Px(top) = node.top else {
                panic!(
                    "{} has a computed vertical placement",
                    ["N", "S", "E", "W"][index]
                );
            };
            lefts[index] = left;
            tops[index] = top;
            assert!(
                left >= 280.0,
                "{} text must not sit under the full-height sidebar: {:?}",
                ["N", "S", "E", "W"][index],
                node.left,
            );
        }
        assert!(lefts[3] < lefts[2], "west remains left of east: {lefts:?}");
        assert!(tops[0] < tops[1], "north remains above south: {tops:?}");
    }
}
