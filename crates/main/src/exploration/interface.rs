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

#[derive(Component)]
pub(super) struct VehicleSelectorPanel;

#[derive(Component)]
pub(super) struct DestinationDetails;

#[derive(Component)]
pub(super) struct VehicleSelectorChoice(Kind);

pub(super) fn setup(
    mut commands: Commands,
    font: Res<crate::ui::UiFont>,
    sections: Query<(Entity, &crate::ui::SidebarSection)>,
) {
    let Some(view_section) = sections
        .iter()
        .find(|(_, section)| **section == crate::ui::SidebarSection::View)
        .map(|(entity, _)| entity)
    else {
        return;
    };
    let Some(context_section) = sections
        .iter()
        .find(|(_, section)| **section == crate::ui::SidebarSection::Context)
        .map(|(entity, _)| entity)
    else {
        return;
    };

    commands.entity(view_section).with_children(|section| {
        section
            .spawn((
                Node {
                    width: Val::Percent(100.0),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Stretch,
                    row_gap: Val::Px(6.0),
                    display: Display::None,
                    ..default()
                },
                PlanetViewControls,
            ))
            .with_children(|parent| {
                parent.spawn((crate::ui::sidebar_action_row(
                    "Follow body · F · ON",
                    crate::ui::SidebarReadoutSlot::Follow,
                    crate::ui::SidebarAction::ToggleFollow,
                    &font,
                ),));
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
    });

    commands.entity(context_section).with_child((
        Text::new("Open Planet view to inspect a destination."),
        TextFont {
            font: font.0.clone().into(),
            font_size: 12.0.into(),
            ..default()
        },
        TextColor(shared::theme::INK),
        DestinationDetails,
        Node {
            width: Val::Percent(100.0),
            flex_shrink: 0.0,
            ..default()
        },
    ));

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
    mut fade_elements: Query<
        (
            Option<&mut BackgroundColor>,
            Option<&mut Text>,
            Option<&mut TextColor>,
            Option<&mut BorderColor>,
            Option<&mut ImageNode>,
            Option<&mut PlanetViewInterfaceElement>,
            Option<&mut GameplayHudElement>,
            Option<&DestinationDetails>,
        ),
        Or<(
            With<PlanetViewInterfaceElement>,
            With<GameplayHudElement>,
            With<DestinationDetails>,
        )>,
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
    for (
        background,
        label_text,
        text_color,
        border,
        image,
        interface,
        gameplay,
        destination_details,
    ) in &mut fade_elements
    {
        let mut background = background;
        let mut label_text = label_text;
        let mut text_color = text_color;
        let mut border = border;
        let mut image = image;
        if destination_details.is_some()
            && let Some(text) = label_text.as_deref_mut()
        {
            let content = destination_readout(&state);
            if text.0 != content {
                *text = Text::new(content);
            }
        }
        if let Some(mut interface) = interface {
            interface.apply_opacity(
                interface_opacity,
                background.as_deref_mut(),
                text_color.as_deref_mut(),
                border.as_deref_mut(),
                image.as_deref_mut(),
            );
        }
        if let Some(mut gameplay) = gameplay {
            gameplay.apply_opacity(
                gameplay_opacity,
                background.as_deref_mut(),
                text_color.as_deref_mut(),
                border.as_deref_mut(),
                image.as_deref_mut(),
            );
        }
    }
}

pub(super) fn destination_readout(state: &Exploration) -> String {
    let Some(destination) = state.selected_planet_destination() else {
        return if state.planet_view_interface_visible() {
            "Click visible terrain or a bridge deck to inspect it.".into()
        } else {
            "Open Planet view to inspect a destination.".into()
        };
    };

    let request_status = state
        .planet_teleport_status()
        .filter(|status| match status {
            PlanetTeleportStatus::Checking { destination_id, .. }
            | PlanetTeleportStatus::Rejected { destination_id, .. }
            | PlanetTeleportStatus::Cancelled { destination_id, .. } => {
                *destination_id == destination.id
            }
        });
    let feedback = match request_status {
        Some(PlanetTeleportStatus::Checking { .. }) => {
            "Checking this exact landing spot. Nearby collision is being prepared.".to_owned()
        }
        Some(PlanetTeleportStatus::Rejected { reason, .. }) => format!(
            "{} Select another spot or press T to try again.",
            reason.message()
        ),
        Some(PlanetTeleportStatus::Cancelled { reason, .. }) => reason.message().to_owned(),
        None => {
            let eligibility = if state.destination_requires_exit_from_vehicle() {
                "Exit your vehicle before pressing T."
            } else {
                "Press T to check a safe on-foot landing."
            };
            format!("{eligibility}\nA safe landing is checked after you press T.")
        }
    };
    format!(
        "Selected destination: {}\n{}",
        destination.display, feedback
    )
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
    sidebar_nodes: Query<(&ComputedNode, &UiGlobalTransform), With<crate::ui::Sidebar>>,
    action_controls: Query<(
        Entity,
        &ComputedNode,
        &UiGlobalTransform,
        &crate::ui::SidebarActionControl,
    )>,
    selector_choices: Query<(
        Entity,
        &ComputedNode,
        &UiGlobalTransform,
        &VehicleSelectorChoice,
    )>,
) {
    let orbit_intent = std::mem::take(&mut state.planet_orbit_intent);
    let zoom_intent = std::mem::take(&mut state.planet_zoom_intent);
    if !state.selector && state.planet_camera.is_active() {
        if orbit_intent != Vec2::ZERO
            && let Some(camera) = cameras.iter().next()
        {
            state
                .planet_camera
                .orbit_from(*camera, orbit_intent * ORBIT_RADIANS_PER_LOGICAL_PIXEL);
            state.set_planet_view_open(true);
        }
        if zoom_intent != 0.0 {
            state.planet_camera.zoom_by((-zoom_intent).exp());
            state.set_planet_view_open(true);
        }
    }

    let Ok(window) = windows.single() else {
        let _ = wheels.read().count();
        state.planet_pointer.cancel();
        state.pressed_sidebar_action = None;
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
            state.pressed_sidebar_action = None;
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
    {
        let over_sidebar = sidebar_nodes
            .iter()
            .any(|(node, transform)| node.contains_point(*transform, position));
        if over_sidebar {
            state
                .planet_pointer
                .press(position, scale_factor, PointerCapture::Interface);
            state.pressed_sidebar_action = action_controls
                .iter()
                .find(|(_, node, transform, _)| node.contains_point(**transform, position))
                .map(|(entity, _, _, _)| entity)
        } else if state.planet_camera.is_active() {
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
            state.pressed_sidebar_action = if capture == PointerCapture::Interface {
                action_controls
                    .iter()
                    .find(|(_, node, transform, _)| node.contains_point(**transform, position))
                    .map(|(entity, _, _, _)| entity)
            } else {
                None
            };
        }
    }

    if state.planet_pointer.is_captured()
        && cursor.is_none()
        && mouse.just_released(MouseButton::Left)
    {
        state.planet_pointer.cancel();
        state.pressed_sidebar_action = None;
    } else if state.planet_pointer.is_captured()
        && (mouse.pressed(MouseButton::Left) || mouse.just_released(MouseButton::Left))
        && let Some(position) = cursor
    {
        let motion = state.planet_pointer.move_to(position, scale_factor);
        if motion.dragging
            && let Some(camera) = cameras.iter().next()
            && state.planet_pointer.capture() == Some(PointerCapture::World)
        {
            state.planet_camera.orbit_from(
                *camera,
                motion.orbit_delta * ORBIT_RADIANS_PER_LOGICAL_PIXEL,
            );
            state.set_planet_view_open(true);
        }
        if mouse.just_released(MouseButton::Left) {
            let release = state.planet_pointer.release(position, scale_factor);
            match release {
                Some(PointerRelease::WorldClick(_)) => {
                    state.request_planet_view_selection(position);
                }
                Some(PointerRelease::CapturedClick(PointerCapture::Interface, _)) => {
                    if let Some(control_entity) = state.pressed_sidebar_action.take()
                        && let Ok((_, node, transform, control)) =
                            action_controls.get(control_entity)
                        && node.contains_point(*transform, position)
                    {
                        apply_sidebar_action(control.0, &mut state, cameras.iter().next());
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
                _ => state.pressed_sidebar_action = None,
            }
        }
    }

    let over_interface = cursor.is_some_and(|position| {
        sidebar_nodes
            .iter()
            .any(|(node, transform)| node.contains_point(*transform, position))
            || (state.planet_presentation.interface_opacity() > 0.0
                && interface_nodes
                    .iter()
                    .any(|(node, transform)| node.contains_point(*transform, position)))
    });
    if scroll != 0.0 && !state.selector && !over_interface && state.planet_camera.is_active() {
        state.planet_camera.zoom_by((-scroll).exp());
        state.set_planet_view_open(true);
    }
}

fn apply_sidebar_action(
    action: crate::ui::SidebarAction,
    state: &mut Exploration,
    camera: Option<&Transform>,
) {
    match action {
        crate::ui::SidebarAction::TogglePlanetView => state.toggle_planet_view(),
        crate::ui::SidebarAction::ToggleFollow => {
            if state.planet_camera.is_requested_open()
                && let Some(camera) = camera
            {
                state.toggle_planet_view_follow(*camera);
            }
        }
        crate::ui::SidebarAction::Interact => state.request(Action::Interact),
        crate::ui::SidebarAction::Teleport => {
            state.request_planet_view_teleport();
        }
    }
}

pub(super) fn update_action_hover(
    windows: Query<&Window, With<PrimaryWindow>>,
    mut controls: Query<
        (&ComputedNode, &UiGlobalTransform, &mut TextColor),
        With<crate::ui::SidebarActionControl>,
    >,
) {
    let cursor = windows
        .single()
        .ok()
        .and_then(Window::physical_cursor_position);
    for (node, transform, mut color) in &mut controls {
        let hovered = cursor.is_some_and(|position| node.contains_point(*transform, position));
        let target = if hovered {
            shared::theme::ACCENT
        } else {
            shared::theme::INK
        };
        if color.0 != target {
            color.0 = target;
        }
    }
}

fn choose_vehicle(kind: Kind, state: &mut Exploration, virtual_time: &mut Time<Virtual>) {
    state.request(Action::Summon(kind));
    state.selector = false;
    virtual_time.unpause();
    state.suppress_input = true;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follow_button_preserves_the_attained_camera_pose_when_detaching() {
        let body_position = Vec3::Y * shared::sphere::PLANET_RADIUS;
        let chase = Transform::from_xyz(0.0, shared::sphere::PLANET_RADIUS + 5.0, -5.0)
            .looking_at(body_position, Vec3::Y);
        let mut current = chase;
        let mut state = Exploration::default();
        state.set_planet_view_open(true);
        for _ in 0..120 {
            current = state.planet_camera.update(
                current,
                chase,
                body_position,
                Vec3::NEG_Z,
                1.0 / 60.0,
                shared::sphere::PLANET_RADIUS,
            );
            state.planet_camera.finish_transition_if_ready();
        }
        current = state.planet_camera.update(
            current,
            chase,
            body_position,
            Vec3::X,
            1.0 / 60.0,
            shared::sphere::PLANET_RADIUS,
        );
        let attained = current;

        apply_sidebar_action(
            crate::ui::SidebarAction::ToggleFollow,
            &mut state,
            Some(&attained),
        );
        assert!(!state.planet_view_follows_body());
        let detached = state.planet_camera.update(
            attained,
            chase,
            Vec3::X * shared::sphere::PLANET_RADIUS,
            Vec3::Z,
            1.0 / 60.0,
            shared::sphere::PLANET_RADIUS,
        );
        assert!(
            detached
                .translation
                .normalize()
                .dot(attained.translation.normalize())
                > 0.9999
        );
        assert!(detached.rotation.angle_between(attained.rotation) < 0.01);
    }
}
