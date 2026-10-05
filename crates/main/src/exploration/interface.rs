use super::*;
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::window::PrimaryWindow;
use shared::planet_view_interface::{
    GameplayHudElement, PlanetViewInterfaceElement, PointerCapture, PointerRelease,
};

const ORBIT_RADIANS_PER_LOGICAL_PIXEL: f32 = 0.004;

#[derive(Component)]
pub(super) struct PlanetViewControls;

#[derive(Component)]
pub(super) struct DestinationDetails;

#[derive(Component)]
pub(super) struct VehicleSelectorAction;

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
    let Some(actions_section) = sections
        .iter()
        .find(|(_, section)| **section == crate::ui::SidebarSection::Actions)
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
        .entity(context_section)
        .with_child(crate::ui::sidebar_exploration_row(
            "Recovery hold: 0% · hold Recover or R for 1 second.",
            crate::ui::SidebarReadoutSlot::RecoveryContext,
            &font,
        ));

    commands.entity(actions_section).with_children(|section| {
        for (kind, label) in [
            (Kind::Car, "Choose Car · C"),
            (Kind::Plane, "Choose Plane · P"),
        ] {
            section
                .spawn(crate::ui::sidebar_action_row(
                    label,
                    crate::ui::SidebarReadoutSlot::SelectorChoice,
                    crate::ui::SidebarAction::SelectVehicle(kind),
                    &font,
                ))
                .insert(VehicleSelectorAction);
        }
        section
            .spawn(crate::ui::sidebar_action_row(
                "Cancel · Esc / V",
                crate::ui::SidebarReadoutSlot::SelectorChoice,
                crate::ui::SidebarAction::CancelVehicleSelector,
                &font,
            ))
            .insert(VehicleSelectorAction);
    });
}

#[allow(clippy::type_complexity)]
pub(super) fn update_presentation(
    time: Res<Time<Real>>,
    mut state: ResMut<Exploration>,
    mut controls_panel: Query<
        &mut Node,
        (
            With<PlanetViewControls>,
            Without<crate::ui::SidebarActionControl>,
        ),
    >,
    mut action_nodes: Query<
        (&mut Node, Option<&VehicleSelectorAction>),
        With<crate::ui::SidebarActionControl>,
    >,
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
    for (mut node, selector_action) in &mut action_nodes {
        let display = if state.selector == selector_action.is_some() {
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
        state.mouse_recovery_held = false;
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
            state.mouse_recovery_held = false;
        }
        if mouse.just_pressed(MouseButton::Left)
            && let Some(position) = cursor
        {
            state
                .planet_pointer
                .press(position, scale_factor, PointerCapture::Selector);
            state.pressed_sidebar_action = action_controls
                .iter()
                .find(|(_, node, transform, _)| node.contains_point(**transform, position))
                .map(|(entity, _, _, _)| entity);
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

    if mouse.just_pressed(MouseButton::Left) {
        state.mouse_recovery_held = state
            .pressed_sidebar_action
            .and_then(|entity| action_controls.get(entity).ok())
            .is_some_and(|(_, _, _, control)| control.0 == crate::ui::SidebarAction::HoldRecovery);
    }

    if state.planet_pointer.is_captured()
        && cursor.is_none()
        && mouse.just_released(MouseButton::Left)
    {
        state.planet_pointer.cancel();
        state.pressed_sidebar_action = None;
        state.mouse_recovery_held = false;
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
                        apply_sidebar_action(
                            control.0,
                            &mut state,
                            cameras.iter().next(),
                            &mut virtual_time,
                        );
                    }
                }
                Some(PointerRelease::CapturedClick(PointerCapture::Selector, _)) => {
                    let pressed = state.pressed_sidebar_action.take();
                    if state.selector
                        && let Some(control_entity) = pressed
                        && let Ok((_, node, transform, control)) =
                            action_controls.get(control_entity)
                        && node.contains_point(*transform, position)
                    {
                        apply_sidebar_action(
                            control.0,
                            &mut state,
                            cameras.iter().next(),
                            &mut virtual_time,
                        );
                    }
                }
                _ => state.pressed_sidebar_action = None,
            }
        }
    }

    if mouse.just_released(MouseButton::Left) {
        state.mouse_recovery_held = false;
    }
    let recovery_capture_lost = state.mouse_recovery_held
        && (cursor.is_none()
            || state.planet_pointer.capture() != Some(PointerCapture::Interface)
            || state
                .pressed_sidebar_action
                .and_then(|entity| action_controls.get(entity).ok())
                .is_none_or(|(_, _, _, control)| {
                    control.0 != crate::ui::SidebarAction::HoldRecovery
                }));
    if recovery_capture_lost {
        state.mouse_recovery_held = false;
        state.planet_pointer.cancel();
        state.pressed_sidebar_action = None;
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
    virtual_time: &mut Time<Virtual>,
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
        crate::ui::SidebarAction::ToggleVehicleSelector => {
            super::open_vehicle_selector(state, virtual_time);
        }
        crate::ui::SidebarAction::SelectVehicle(kind) => {
            super::choose_vehicle(kind, state, virtual_time);
        }
        crate::ui::SidebarAction::CancelVehicleSelector => {
            super::cancel_vehicle_selector(state, virtual_time);
        }
        crate::ui::SidebarAction::HoldRecovery => {}
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
            &mut Time::<Virtual>::default(),
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
