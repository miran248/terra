use super::*;
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::ui::FocusPolicy;
use bevy::window::PrimaryWindow;
use shared::planet_view_interface::{
    GameplayHudElement, PlanetViewInterfaceElement, PointerCapture, PointerRelease,
};

const ORBIT_RADIANS_PER_LOGICAL_PIXEL: f32 = 0.004;

#[derive(Component)]
pub(super) struct PlanetViewControls;

#[derive(Component, Clone, Copy)]
pub(super) enum PlanetViewControl {
    ToggleFollow,
    Return,
}

#[derive(Component)]
pub(super) struct FollowStatus;

#[derive(Component)]
pub(super) struct VehicleSelectorPanel;

#[derive(Component)]
pub(super) struct VehicleSelectorChoice(Kind);

pub(super) fn setup(mut commands: Commands, font: Res<crate::ui::UiFont>) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(16.0),
                right: Val::Px(16.0),
                display: Display::Flex,
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Stretch,
                row_gap: Val::Px(8.0),
                padding: UiRect::all(Val::Px(10.0)),
                ..default()
            },
            BackgroundColor(shared::theme::PANEL_BG),
            BorderColor::all(shared::theme::TEXT_WEAK),
            GlobalZIndex(100),
            FocusPolicy::Block,
            PlanetViewInterfaceElement::default(),
            PlanetViewControls,
        ))
        .with_children(|parent| {
            parent
                .spawn((
                    Button,
                    Node {
                        min_width: Val::Px(156.0),
                        min_height: Val::Px(34.0),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        padding: UiRect::axes(Val::Px(12.0), Val::Px(6.0)),
                        ..default()
                    },
                    BackgroundColor(shared::theme::PANEL_BG),
                    BorderColor::all(shared::theme::TEXT_WEAK),
                    PlanetViewControl::ToggleFollow,
                    PlanetViewInterfaceElement::default(),
                ))
                .with_child((
                    Text::new("FOLLOW · ON"),
                    TextFont {
                        font: font.0.clone().into(),
                        font_size: 14.0.into(),
                        ..default()
                    },
                    TextColor(shared::theme::INK),
                    PlanetViewInterfaceElement::default(),
                    FollowStatus,
                ));
            parent
                .spawn((
                    Button,
                    Node {
                        min_width: Val::Px(156.0),
                        min_height: Val::Px(34.0),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        padding: UiRect::axes(Val::Px(12.0), Val::Px(6.0)),
                        ..default()
                    },
                    BackgroundColor(shared::theme::PANEL_BG),
                    BorderColor::all(shared::theme::TEXT_WEAK),
                    PlanetViewControl::Return,
                    PlanetViewInterfaceElement::default(),
                ))
                .with_child((
                    Text::new("RETURN TO EXPLORER"),
                    TextFont {
                        font: font.0.clone().into(),
                        font_size: 14.0.into(),
                        ..default()
                    },
                    TextColor(shared::theme::INK),
                    PlanetViewInterfaceElement::default(),
                ));
            parent.spawn((
                Text::new("DRAG · ORBIT     WHEEL · ZOOM"),
                TextFont {
                    font: font.0.clone().into(),
                    font_size: 11.0.into(),
                    ..default()
                },
                TextColor(shared::theme::TEXT_WEAK),
                PlanetViewInterfaceElement::default(),
            ));
        });

    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                top: Val::Px(0.0),
                bottom: Val::Px(0.0),
                display: Display::None,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(shared::theme::PANEL_BG.with_alpha(0.8)),
            GlobalZIndex(200),
            FocusPolicy::Block,
            VehicleSelectorPanel,
        ))
        .with_children(|parent| {
            parent
                .spawn((
                    Node {
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(10.0),
                        min_width: Val::Px(280.0),
                        padding: UiRect::all(Val::Px(20.0)),
                        ..default()
                    },
                    BackgroundColor(shared::theme::PANEL_BG),
                    BorderColor::all(shared::theme::ACCENT),
                ))
                .with_children(|panel| {
                    panel.spawn((
                        Text::new("VEHICLE SELECTOR"),
                        TextFont {
                            font: font.0.clone().into(),
                            font_size: 20.0.into(),
                            ..default()
                        },
                        TextColor(shared::theme::INK),
                    ));
                    for (kind, key) in [(Kind::Car, "C  ·  CAR"), (Kind::Plane, "P  ·  PLANE")] {
                        panel
                            .spawn((
                                Button,
                                Node {
                                    min_height: Val::Px(42.0),
                                    justify_content: JustifyContent::Center,
                                    align_items: AlignItems::Center,
                                    ..default()
                                },
                                BackgroundColor(shared::theme::PANEL_BG),
                                BorderColor::all(shared::theme::TEXT_WEAK),
                                VehicleSelectorChoice(kind),
                            ))
                            .with_child((
                                Text::new(key),
                                TextFont {
                                    font: font.0.clone().into(),
                                    font_size: 16.0.into(),
                                    ..default()
                                },
                                TextColor(shared::theme::INK),
                            ));
                    }
                    panel.spawn((
                        Text::new("ESC OR V TO CLOSE"),
                        TextFont {
                            font: font.0.clone().into(),
                            font_size: 11.0.into(),
                            ..default()
                        },
                        TextColor(shared::theme::TEXT_WEAK),
                    ));
                });
        });
}

#[allow(clippy::type_complexity)]
pub(super) fn update_presentation(
    time: Res<Time<Real>>,
    mut state: ResMut<Exploration>,
    mut panel: Query<&mut Node, (With<VehicleSelectorPanel>, Without<PlanetViewControls>)>,
    mut controls_panel: Query<&mut Node, (With<PlanetViewControls>, Without<VehicleSelectorPanel>)>,
    mut follow_status: Query<&mut Text, With<FollowStatus>>,
    mut fade_elements: Query<
        (
            Option<&mut BackgroundColor>,
            Option<&mut TextColor>,
            Option<&mut BorderColor>,
            Option<&mut ImageNode>,
            Option<&mut PlanetViewInterfaceElement>,
            Option<&mut GameplayHudElement>,
        ),
        Or<(With<PlanetViewInterfaceElement>, With<GameplayHudElement>)>,
    >,
) {
    let requested_open = state.planet_camera.is_requested_open();
    state.planet_presentation.request_open(requested_open);
    state.planet_presentation.advance(time.delta_secs());
    let interface_opacity = state.planet_presentation.interface_opacity();
    let gameplay_opacity = state.planet_presentation.gameplay_opacity();

    for mut node in &mut controls_panel {
        let display = if interface_opacity > 0.0 {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != display {
            node.display = display;
        }
    }
    for mut node in &mut panel {
        let display = if state.selector {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != display {
            node.display = display;
        }
    }
    if let Ok(mut text) = follow_status.single_mut() {
        let label = if state.planet_view_follows_body() {
            "FOLLOW · ON"
        } else {
            "FOLLOW · OFF"
        };
        if text.0 != label {
            *text = Text::new(label);
        }
    }

    for (background, text, border, image, interface, gameplay) in &mut fade_elements {
        let mut background = background;
        let mut text = text;
        let mut border = border;
        let mut image = image;
        if let Some(mut interface) = interface {
            interface.apply_opacity(
                interface_opacity,
                background.as_deref_mut(),
                text.as_deref_mut(),
                border.as_deref_mut(),
                image.as_deref_mut(),
            );
        }
        if let Some(mut gameplay) = gameplay {
            gameplay.apply_opacity(
                gameplay_opacity,
                background.as_deref_mut(),
                text.as_deref_mut(),
                border.as_deref_mut(),
                image.as_deref_mut(),
            );
        }
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(super) fn pointer_input(
    mut state: ResMut<Exploration>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut wheels: MessageReader<MouseWheel>,
    mut virtual_time: ResMut<Time<Virtual>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<&Transform, With<MainCamera>>,
    interface_nodes: Query<(&ComputedNode, &UiGlobalTransform), With<PlanetViewInterfaceElement>>,
    controls: Query<(
        Entity,
        &ComputedNode,
        &UiGlobalTransform,
        &PlanetViewControl,
    )>,
    selector_choices: Query<(
        Entity,
        &ComputedNode,
        &UiGlobalTransform,
        &VehicleSelectorChoice,
    )>,
) {
    let Ok(window) = windows.single() else {
        let _ = wheels.read().count();
        state.planet_pointer.cancel();
        state.pressed_planet_control = None;
        return;
    };
    let scale_factor = window.scale_factor();
    let cursor = window.physical_cursor_position();

    let mut scroll = 0.0;
    for wheel in wheels.read() {
        scroll += match wheel.unit {
            MouseScrollUnit::Line => wheel.y * 0.12,
            MouseScrollUnit::Pixel => wheel.y * 0.002,
        };
    }
    if state.selector {
        if state.planet_pointer.capture() != Some(PointerCapture::Selector) {
            state.planet_pointer.cancel();
            state.pressed_planet_control = None;
        }
        if mouse.just_pressed(MouseButton::Left)
            && let Some(position) = cursor
        {
            state
                .planet_pointer
                .press(position, scale_factor, PointerCapture::Selector);
        }
    } else if mouse.just_pressed(MouseButton::Left)
        && let Some(position) = cursor
        && state.planet_camera.is_active()
    {
        let over_interface = state.planet_presentation.interface_opacity() > 0.0
            && interface_nodes
                .iter()
                .any(|(node, transform)| node.contains_point(*transform, position));
        let capture = if over_interface {
            PointerCapture::Interface
        } else {
            PointerCapture::World
        };
        state.planet_pointer.press(position, scale_factor, capture);
        state.pressed_planet_control = if capture == PointerCapture::Interface {
            controls
                .iter()
                .find(|(_, node, transform, _)| node.contains_point(**transform, position))
                .map(|(entity, _, _, _)| entity)
        } else {
            None
        };
    }

    if state.planet_pointer.is_captured()
        && cursor.is_none()
        && mouse.just_released(MouseButton::Left)
    {
        state.planet_pointer.cancel();
        state.pressed_planet_control = None;
    } else if state.planet_pointer.is_captured()
        && (mouse.pressed(MouseButton::Left) || mouse.just_released(MouseButton::Left))
        && let Some(position) = cursor
    {
        let motion = state.planet_pointer.move_to(position, scale_factor);
        if motion.dragging
            && let Some(camera) = cameras.iter().next()
            && state.planet_pointer.capture() == Some(PointerCapture::World)
        {
            if motion.began_dragging {
                state
                    .planet_camera
                    .detach(camera.translation.normalize_or(Vec3::Y));
            }
            state
                .planet_camera
                .orbit(motion.orbit_delta * ORBIT_RADIANS_PER_LOGICAL_PIXEL);
            state.set_planet_view_open(true);
        }
        if mouse.just_released(MouseButton::Left) {
            let release = state.planet_pointer.release(position, scale_factor);
            match release {
                Some(PointerRelease::WorldClick(_)) => {
                    state.request_planet_view_selection(position);
                }
                Some(PointerRelease::CapturedClick(PointerCapture::Interface, _)) => {
                    if let Some(control_entity) = state.pressed_planet_control.take()
                        && let Ok((_, node, transform, control)) = controls.get(control_entity)
                        && node.contains_point(*transform, position)
                    {
                        apply_planet_control(*control, &mut state, cameras.iter().next());
                    }
                }
                Some(PointerRelease::CapturedClick(PointerCapture::Selector, _)) => {
                    if state.selector
                        && let Some((_, _, _, choice)) =
                            selector_choices.iter().find(|(_, node, transform, _)| {
                                node.contains_point(**transform, position)
                            })
                    {
                        choose_vehicle(choice.0, &mut state, &mut virtual_time);
                    }
                }
                _ => state.pressed_planet_control = None,
            }
        }
    }

    let over_interface = cursor.is_some_and(|position| {
        state.planet_presentation.interface_opacity() > 0.0
            && interface_nodes
                .iter()
                .any(|(node, transform)| node.contains_point(*transform, position))
    });
    if scroll != 0.0 && !state.selector && !over_interface && state.planet_camera.is_active() {
        state.planet_camera.zoom_by((-scroll).exp());
        state.set_planet_view_open(true);
    }
}

fn apply_planet_control(
    control: PlanetViewControl,
    state: &mut Exploration,
    camera: Option<&Transform>,
) {
    match control {
        PlanetViewControl::ToggleFollow => {
            if let Some(camera) = camera {
                state.toggle_planet_view_follow(camera.translation);
            }
        }
        PlanetViewControl::Return => state.set_planet_view_open(false),
    }
}

fn choose_vehicle(kind: Kind, state: &mut Exploration, virtual_time: &mut Time<Virtual>) {
    state.request(Action::Summon(kind));
    state.selector = false;
    virtual_time.unpause();
    state.suppress_input = true;
}
