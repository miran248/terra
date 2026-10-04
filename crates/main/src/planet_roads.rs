use bevy::prelude::*;
use bevy::ui::FocusPolicy;
use shared::planet_view_interface::PlanetViewInterfaceElement;
use shared::roads::{SurfaceRoadSample, build_surface_road_ribbon, projected_surface_ribbon_width};

use crate::{
    exploration::{Exploration, ExplorationUpdate},
    map::{MainCamera, build_visual_mesh},
    ui::UiFont,
};

const MIN_SCREEN_WIDTH_PX: f32 = 2.5;
const MIN_ROAD_WIDTH_METERS: f32 = 4.0;
pub(crate) const ROAD_SAMPLE_SPACING_METERS: f32 = 4.0;
pub(crate) const ROAD_SURFACE_LIFT_METERS: f32 = 0.22;
const WIDTH_REFRESH_SECONDS: f32 = 0.2;
const WIDTH_REFRESH_POSITION_METERS: f32 = 24.0;

#[derive(Resource, Default)]
pub(crate) struct RoadHighlightPaths(pub Vec<Vec<SurfaceRoadSample>>);

#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RoadHighlightLayer {
    enabled: bool,
}

impl Default for RoadHighlightLayer {
    fn default() -> Self {
        Self { enabled: true }
    }
}

impl RoadHighlightLayer {
    fn toggle(&mut self) {
        self.enabled = !self.enabled;
    }

    fn is_enabled(self) -> bool {
        self.enabled
    }
}

#[derive(Component)]
struct RoadHighlightMesh;

#[derive(Component)]
struct RoadHighlightOpacity(u32);

#[derive(Component)]
struct RoadsLayerPanel;

#[derive(Component)]
struct RoadsLayerToggle;

#[derive(Component)]
struct RoadsLayerLabel;

#[derive(Default)]
struct RoadMeshRefresh {
    elapsed: f32,
    last_camera_position: Option<Vec3>,
    last_viewport_height: Option<f32>,
    last_vertical_fov: Option<f32>,
}

pub(crate) struct PlanetRoadsPlugin;

impl Plugin for PlanetRoadsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RoadHighlightLayer>()
            .add_systems(
                OnEnter(shared::state::AppState::Playing),
                setup_road_highlight.after(crate::map::setup_map),
            )
            .add_systems(
                OnExit(shared::state::AppState::Playing),
                cleanup_road_highlight,
            )
            .add_systems(
                Update,
                (
                    handle_roads_toggle,
                    update_roads_layer_presentation,
                    update_road_highlight_widths,
                )
                    .chain()
                    .after(ExplorationUpdate)
                    .run_if(in_state(shared::state::AppState::Playing)),
            );
    }
}

fn setup_road_highlight(
    mut commands: Commands,
    paths: Res<RoadHighlightPaths>,
    font: Res<UiFont>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let triangles = build_highlight_triangles(&paths.0, MIN_ROAD_WIDTH_METERS);
    if !triangles.is_empty() {
        let colors = vec![[[1.0, 1.0, 1.0, 1.0]; 3]; triangles.len()];
        let highlight_material = materials.add(road_highlight_material(1.0));
        commands.spawn((
            Mesh3d(meshes.add(build_visual_mesh(&triangles, &colors))),
            MeshMaterial3d(highlight_material),
            Transform::default(),
            Visibility::Hidden,
            RoadHighlightMesh,
            RoadHighlightOpacity(u32::MAX),
        ));
    }

    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(16.0),
                top: Val::Px(16.0),
                padding: UiRect::all(Val::Px(8.0)),
                display: Display::None,
                ..default()
            },
            BackgroundColor(shared::theme::PANEL_BG),
            BorderColor::all(shared::theme::TEXT_WEAK),
            GlobalZIndex(100),
            FocusPolicy::Pass,
            PlanetViewInterfaceElement::default(),
            RoadsLayerPanel,
        ))
        .with_children(|panel| {
            panel
                .spawn((
                    Button,
                    Node {
                        min_width: Val::Px(132.0),
                        min_height: Val::Px(34.0),
                        padding: UiRect::axes(Val::Px(12.0), Val::Px(6.0)),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    BackgroundColor(shared::theme::PANEL_BG),
                    BorderColor::all(shared::theme::ACCENT),
                    PlanetViewInterfaceElement::default(),
                    RoadsLayerToggle,
                ))
                .with_child((
                    Text::new("ROADS · ON"),
                    TextFont {
                        font: font.0.clone().into(),
                        font_size: 14.0.into(),
                        ..default()
                    },
                    TextColor(shared::theme::INK),
                    PlanetViewInterfaceElement::default(),
                    RoadsLayerLabel,
                ));
        });
}

fn cleanup_road_highlight(
    mut commands: Commands,
    entities: Query<Entity, Or<(With<RoadHighlightMesh>, With<RoadsLayerPanel>)>>,
) {
    for entity in &entities {
        commands.entity(entity).despawn();
    }
}

fn handle_roads_toggle(
    state: Res<Exploration>,
    mut layer: ResMut<RoadHighlightLayer>,
    toggles: Query<&Interaction, (With<RoadsLayerToggle>, Changed<Interaction>)>,
) {
    if !roads_layer_toggle_allowed(
        state.is_planet_view_active(),
        state.planet_view_interface_visible(),
    ) {
        return;
    }
    if toggles
        .iter()
        .any(|interaction| *interaction == Interaction::Pressed)
    {
        layer.toggle();
    }
}

fn update_roads_layer_presentation(
    state: Res<Exploration>,
    layer: Res<RoadHighlightLayer>,
    mut panel: Query<&mut Node, With<RoadsLayerPanel>>,
    mut label: Query<&mut Text, With<RoadsLayerLabel>>,
    mut highlights: Query<
        (
            &mut Visibility,
            &MeshMaterial3d<StandardMaterial>,
            &mut RoadHighlightOpacity,
        ),
        With<RoadHighlightMesh>,
    >,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let opacity = state.planet_view_interface_opacity().clamp(0.0, 1.0);
    let view_active = state.is_planet_view_active();
    let show_controls =
        planet_roads_controls_display(view_active, state.planet_view_interface_visible());
    let show_highlight = planet_roads_highlight_visible(view_active, layer.is_enabled(), opacity);

    for mut node in &mut panel {
        if node.display != show_controls {
            node.display = show_controls;
        }
    }
    let text = if layer.is_enabled() {
        "ROADS · ON"
    } else {
        "ROADS · OFF"
    };
    for mut value in &mut label {
        if value.0 != text {
            value.0 = text.to_owned();
        }
    }
    for (mut visibility, material_handle, mut applied_opacity) in &mut highlights {
        let desired_visibility = if show_highlight {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        if *visibility != desired_visibility {
            *visibility = desired_visibility;
        }
        let opacity_bits = opacity.to_bits();
        if applied_opacity.0 != opacity_bits {
            if let Some(mut material) = materials.get_mut(&material_handle.0) {
                *material = road_highlight_material(opacity);
            }
            applied_opacity.0 = opacity_bits;
        }
    }
}

fn road_highlight_material(opacity: f32) -> StandardMaterial {
    StandardMaterial {
        base_color: Color::srgba(0.9, 0.64, 0.18, opacity.clamp(0.0, 1.0)),
        emissive: LinearRgba::rgb(0.26, 0.13, 0.025),
        perceptual_roughness: 0.9,
        alpha_mode: AlphaMode::Blend,
        ..default()
    }
}

fn planet_roads_controls_display(view_active: bool, interface_visible: bool) -> Display {
    if view_active && interface_visible {
        Display::Flex
    } else {
        Display::None
    }
}

fn roads_layer_toggle_allowed(view_active: bool, interface_visible: bool) -> bool {
    view_active && interface_visible
}

fn planet_roads_highlight_visible(view_active: bool, layer_enabled: bool, opacity: f32) -> bool {
    view_active && layer_enabled && opacity > 0.0
}

fn build_highlight_triangles(paths: &[Vec<SurfaceRoadSample>], width_m: f32) -> Vec<[[f32; 3]; 3]> {
    paths
        .iter()
        .flat_map(|samples| {
            let widths = vec![width_m; samples.len()];
            build_surface_road_ribbon(samples, &widths)
        })
        .collect()
}

fn update_road_highlight_widths(
    time: Res<Time<Real>>,
    state: Res<Exploration>,
    layer: Res<RoadHighlightLayer>,
    paths: Res<RoadHighlightPaths>,
    cameras: Query<(&Camera, &Projection, &Transform), With<MainCamera>>,
    highlight_mesh: Query<&Mesh3d, With<RoadHighlightMesh>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut refresh: Local<RoadMeshRefresh>,
) {
    if !state.is_planet_view_active() || !layer.is_enabled() {
        return;
    }
    let Ok((camera, projection, camera_transform)) = cameras.single() else {
        return;
    };
    let Some(viewport_size) = camera.logical_viewport_size() else {
        return;
    };
    let Projection::Perspective(perspective) = projection else {
        return;
    };
    if viewport_size.y <= 0.0 {
        return;
    }

    refresh.elapsed += time.delta_secs();
    let moved = refresh.last_camera_position.is_none_or(|last| {
        last.distance(camera_transform.translation) >= WIDTH_REFRESH_POSITION_METERS
    });
    let resized = refresh
        .last_viewport_height
        .is_none_or(|height| (height - viewport_size.y).abs() > 1.0);
    let changed_fov = refresh
        .last_vertical_fov
        .is_none_or(|fov| (fov - perspective.fov).abs() > 0.001);
    if !moved && !resized && !changed_fov {
        return;
    }
    if refresh.elapsed < WIDTH_REFRESH_SECONDS {
        return;
    }

    let mut triangles = Vec::new();
    for samples in &paths.0 {
        let widths: Vec<f32> = samples
            .iter()
            .map(|sample| {
                let depth = perspective_camera_depth(*camera_transform, sample.center);
                projected_surface_ribbon_width(
                    depth.max(f32::EPSILON),
                    perspective.fov,
                    viewport_size.y,
                    MIN_SCREEN_WIDTH_PX,
                    MIN_ROAD_WIDTH_METERS,
                )
                .unwrap_or(MIN_ROAD_WIDTH_METERS)
            })
            .collect();
        triangles.extend(build_surface_road_ribbon(samples, &widths));
    }
    let colors = vec![[[1.0, 1.0, 1.0, 1.0]; 3]; triangles.len()];
    let mesh = build_visual_mesh(&triangles, &colors);
    if let Ok(handle) = highlight_mesh.single()
        && let Some(mut existing) = meshes.get_mut(&handle.0)
    {
        *existing = mesh;
    }
    refresh.elapsed = 0.0;
    refresh.last_camera_position = Some(camera_transform.translation);
    refresh.last_viewport_height = Some(viewport_size.y);
    refresh.last_vertical_fov = Some(perspective.fov);
}

fn perspective_camera_depth(camera_transform: Transform, world_position: Vec3) -> f32 {
    -(camera_transform.rotation.inverse() * (world_position - camera_transform.translation)).z
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roads_control_has_no_target_when_the_planet_interface_is_hidden() {
        assert_eq!(planet_roads_controls_display(true, false), Display::None);
        assert_eq!(planet_roads_controls_display(true, true), Display::Flex);
        assert_eq!(planet_roads_controls_display(false, true), Display::None);
    }

    #[test]
    fn roads_layer_is_initially_enabled_and_remembers_its_choice_when_view_reopens() {
        let mut layer = RoadHighlightLayer::default();
        assert!(layer.is_enabled());
        layer.toggle();
        assert!(!layer.is_enabled());
        let mut exploration = Exploration::default();
        exploration.set_planet_view_open(true);
        exploration.set_planet_view_open(false);
        exploration.set_planet_view_open(true);
        assert!(!layer.is_enabled());
        layer.toggle();
        assert!(layer.is_enabled());
    }

    #[test]
    fn road_highlight_only_shows_for_an_enabled_planet_view_layer() {
        assert!(!planet_roads_highlight_visible(false, true, 1.0));
        assert!(!planet_roads_highlight_visible(true, false, 1.0));
        assert!(!planet_roads_highlight_visible(true, true, 0.0));
        assert!(planet_roads_highlight_visible(true, true, 0.5));
    }

    #[test]
    fn road_highlight_fades_with_alpha_and_uses_standard_depth_testing() {
        let material = road_highlight_material(0.4);
        let color = material.base_color.to_srgba();

        assert_eq!(material.alpha_mode, AlphaMode::Blend);
        assert!(!material.unlit);
        assert!((color.red - 0.9).abs() < 0.001);
        assert!((color.green - 0.64).abs() < 0.001);
        assert!((color.blue - 0.18).abs() < 0.001);
        assert!((color.alpha - 0.4).abs() < 0.001);
    }

    #[test]
    fn roads_control_is_hidden_and_cannot_toggle_while_the_selector_owns_interface() {
        assert_eq!(planet_roads_controls_display(true, false), Display::None);
        assert_eq!(planet_roads_controls_display(true, true), Display::Flex);
        assert!(!roads_layer_toggle_allowed(true, false));
        assert!(!roads_layer_toggle_allowed(false, true));
        assert!(roads_layer_toggle_allowed(true, true));
    }

    #[test]
    fn road_width_uses_depth_along_the_perspective_camera_axis() {
        let camera = Transform::from_xyz(0.0, 0.0, 10.0).looking_at(Vec3::ZERO, Vec3::Y);
        let point = Vec3::new(100.0, 0.0, 0.0);

        assert!((perspective_camera_depth(camera, point) - 10.0).abs() < 0.001);
    }
}
