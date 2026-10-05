//! Shared visibility and pointer policy for Planet view controls and overlays.

use bevy::color::Alpha;
use bevy::prelude::*;

const INTERFACE_FADE_SECONDS: f32 = 0.28;
const DRAG_THRESHOLD_LOGICAL_PIXELS: f32 = 4.0;

/// Marks a UI element as part of the Planet view interface.
///
/// The main game uses this marker to apply the presentation opacity and to
/// capture gestures that begin over interface content.
#[derive(Clone, Debug, Default)]
struct UiColors {
    background: Option<Color>,
    text: Option<Color>,
    border: Option<[Color; 4]>,
    image: Option<Color>,
}

impl UiColors {
    fn apply(
        &mut self,
        opacity: f32,
        background: Option<&mut BackgroundColor>,
        text: Option<&mut TextColor>,
        border: Option<&mut BorderColor>,
        image: Option<&mut ImageNode>,
    ) {
        if let Some(background) = background {
            let original = *self.background.get_or_insert(background.0);
            background.0 = original.with_alpha(original.alpha() * opacity);
        }
        if let Some(text) = text {
            let original = *self.text.get_or_insert(text.0);
            text.0 = original.with_alpha(original.alpha() * opacity);
        }
        if let Some(border) = border {
            let original =
                *self
                    .border
                    .get_or_insert([border.top, border.right, border.bottom, border.left]);
            border.top = original[0].with_alpha(original[0].alpha() * opacity);
            border.right = original[1].with_alpha(original[1].alpha() * opacity);
            border.bottom = original[2].with_alpha(original[2].alpha() * opacity);
            border.left = original[3].with_alpha(original[3].alpha() * opacity);
        }
        if let Some(image) = image {
            let original = *self.image.get_or_insert(image.color);
            image.color = original.with_alpha(original.alpha() * opacity);
        }
    }
}

/// Marks a UI element as part of the Planet view interface and stores its base
/// colors so interrupted fades can reverse without losing their original tint.
#[derive(Component, Clone, Debug, Default)]
pub struct PlanetViewInterfaceElement(UiColors);

impl PlanetViewInterfaceElement {
    /// Apply the current interface opacity to this element's supported UI colors.
    pub fn apply_opacity(
        &mut self,
        opacity: f32,
        background: Option<&mut BackgroundColor>,
        text: Option<&mut TextColor>,
        border: Option<&mut BorderColor>,
        image: Option<&mut ImageNode>,
    ) {
        self.0
            .apply(opacity.clamp(0.0, 1.0), background, text, border, image);
    }
}

/// Marks a gameplay HUD element that fades as Planet view opens.
#[derive(Component, Clone, Debug, Default)]
pub struct GameplayHudElement(UiColors);

impl GameplayHudElement {
    /// Replace the source text color when a HUD readout changes dynamically.
    pub fn set_text_color(&mut self, color: Color) {
        self.0.text = Some(color);
    }

    /// Apply the current gameplay HUD opacity while preserving each base color.
    pub fn apply_opacity(
        &mut self,
        opacity: f32,
        background: Option<&mut BackgroundColor>,
        text: Option<&mut TextColor>,
        border: Option<&mut BorderColor>,
        image: Option<&mut ImageNode>,
    ) {
        self.0
            .apply(opacity.clamp(0.0, 1.0), background, text, border, image);
    }
}

/// Owns the UI visibility transition and the shared selection/readiness gate.
///
/// Progress is advanced with real time, so simulation slowdown and pause do
/// not make the interface sluggish.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct PlanetViewPresentation {
    requested_open: bool,
    interface_opacity: f32,
}

impl Default for PlanetViewPresentation {
    fn default() -> Self {
        Self {
            requested_open: false,
            interface_opacity: 0.0,
        }
    }
}

impl PlanetViewPresentation {
    /// Request visible Planet view controls and overlays.
    pub fn request_open(&mut self, open: bool) {
        self.requested_open = open;
    }

    /// Advance opacity toward the requested state using real seconds.
    pub fn advance(&mut self, real_delta_seconds: f32) {
        let target = if self.requested_open { 1.0 } else { 0.0 };
        let step = real_delta_seconds.max(0.0) / INTERFACE_FADE_SECONDS;
        if real_delta_seconds.is_finite() {
            if self.interface_opacity < target {
                self.interface_opacity = (self.interface_opacity + step).min(target);
            } else {
                self.interface_opacity = (self.interface_opacity - step).max(target);
            }
        }
    }

    /// Opacity for Planet view controls and overlays.
    pub fn interface_opacity(&self) -> f32 {
        self.interface_opacity
    }

    /// Complementary opacity for the gameplay HUD and minimap.
    pub fn gameplay_opacity(&self) -> f32 {
        1.0 - self.interface_opacity
    }

    /// True only when the interface is fully visible and not closing.
    pub fn selection_ready(&self) -> bool {
        self.requested_open && self.interface_opacity >= 1.0
    }

    /// Whether the presentation is currently requested to be open.
    pub fn is_requested_open(&self) -> bool {
        self.requested_open
    }
}

/// Owner that captured the current pointer gesture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerCapture {
    /// The gesture may orbit or select the planet surface.
    World,
    /// A control or overlay captured the gesture.
    Interface,
    /// The vehicle selector owns all pointer gestures while it is open.
    Selector,
}

/// Result of releasing an owned pointer gesture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PointerRelease {
    WorldClick(Vec2),
    WorldDrag,
    CapturedClick(PointerCapture, Vec2),
    CapturedDrag(PointerCapture),
}

/// Pointer movement after applying the logical-pixel drag threshold.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PointerMotion {
    /// True once the gesture has moved at least four logical pixels.
    pub dragging: bool,
    /// True only on the update that first crosses the threshold.
    pub began_dragging: bool,
    /// Logical-pixel delta available to world orbit input.
    pub orbit_delta: Vec2,
    /// Previous logical cursor position used for this drag update.
    pub previous_position: Vec2,
    /// Current logical cursor position used for this drag update.
    pub position: Vec2,
}

#[derive(Clone, Copy, Debug)]
struct Gesture {
    capture: PointerCapture,
    start: Vec2,
    last: Vec2,
    dragging: bool,
}

/// Captures pointer ownership from press until release.
#[derive(Clone, Copy, Debug, Default)]
pub struct PlanetViewPointer {
    gesture: Option<Gesture>,
}

impl PlanetViewPointer {
    /// Begin a gesture using physical window coordinates and the current DPI scale.
    pub fn press(&mut self, physical_position: Vec2, scale_factor: f32, capture: PointerCapture) {
        let position = logical_position(physical_position, scale_factor);
        self.gesture = Some(Gesture {
            capture,
            start: position,
            last: position,
            dragging: false,
        });
    }

    /// Track movement while preserving the owner selected at press time.
    pub fn move_to(&mut self, physical_position: Vec2, scale_factor: f32) -> PointerMotion {
        let Some(gesture) = &mut self.gesture else {
            return PointerMotion::default();
        };
        let current = logical_position(physical_position, scale_factor);
        let previously_dragging = gesture.dragging;
        if !gesture.dragging
            && current.distance_squared(gesture.start)
                >= DRAG_THRESHOLD_LOGICAL_PIXELS * DRAG_THRESHOLD_LOGICAL_PIXELS
        {
            gesture.dragging = true;
        }
        let began_dragging = gesture.dragging && !previously_dragging;
        let previous_position = if began_dragging {
            gesture.start
        } else {
            gesture.last
        };
        let orbit_delta = if gesture.capture == PointerCapture::World && gesture.dragging {
            if began_dragging {
                current - gesture.start
            } else {
                current - gesture.last
            }
        } else {
            Vec2::ZERO
        };
        gesture.last = current;
        PointerMotion {
            dragging: gesture.dragging,
            began_dragging,
            orbit_delta,
            previous_position,
            position: current,
        }
    }

    /// Finish the captured gesture, including movement since the last cursor event.
    pub fn release(
        &mut self,
        physical_position: Vec2,
        scale_factor: f32,
    ) -> Option<PointerRelease> {
        let gesture = self.gesture?;
        let current = logical_position(physical_position, scale_factor);
        let dragged = gesture.dragging
            || current.distance_squared(gesture.start)
                >= DRAG_THRESHOLD_LOGICAL_PIXELS * DRAG_THRESHOLD_LOGICAL_PIXELS;
        self.gesture = None;
        Some(match (gesture.capture, dragged) {
            (PointerCapture::World, false) => PointerRelease::WorldClick(current),
            (PointerCapture::World, true) => PointerRelease::WorldDrag,
            (capture, false) => PointerRelease::CapturedClick(capture, current),
            (capture, true) => PointerRelease::CapturedDrag(capture),
        })
    }

    /// Cancel a gesture whose pointer left the window before release.
    pub fn cancel(&mut self) {
        self.gesture = None;
    }

    /// Whether a press is still owned through its release.
    pub fn is_captured(&self) -> bool {
        self.gesture.is_some()
    }

    /// Return the owner selected when the current press began.
    pub fn capture(&self) -> Option<PointerCapture> {
        self.gesture.map(|gesture| gesture.capture)
    }
}

fn logical_position(physical_position: Vec2, scale_factor: f32) -> Vec2 {
    let scale_factor = if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    };
    physical_position / scale_factor
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::prelude::Vec2;

    #[test]
    fn reversing_the_interface_fade_preserves_opacity_and_restores_gameplay_ui() {
        let mut presentation = PlanetViewPresentation::default();
        presentation.request_open(true);
        presentation.advance(0.14);
        let opening_opacity = presentation.interface_opacity();
        assert!(opening_opacity > 0.0 && opening_opacity < 1.0);
        assert!(!presentation.selection_ready());

        presentation.request_open(false);
        assert!(!presentation.selection_ready());
        presentation.advance(0.07);
        let closing_opacity = presentation.interface_opacity();
        assert!(closing_opacity < opening_opacity);
        assert!((presentation.gameplay_opacity() - (1.0 - closing_opacity)).abs() < 0.001);

        presentation.request_open(true);
        assert!((presentation.interface_opacity() - closing_opacity).abs() < 0.001);
        presentation.advance(0.07);
        assert!(presentation.interface_opacity() > closing_opacity);
        presentation.advance(0.28);
        assert_eq!(presentation.interface_opacity(), 1.0);
        assert_eq!(presentation.gameplay_opacity(), 0.0);
        assert!(presentation.selection_ready());

        presentation.request_open(false);
        assert!(!presentation.selection_ready());
        presentation.advance(0.28);
        assert_eq!(presentation.interface_opacity(), 0.0);
        assert_eq!(presentation.gameplay_opacity(), 1.0);
    }

    #[test]
    fn four_logical_pixels_start_a_drag_at_any_window_scale() {
        for scale_factor in [1.0, 2.0] {
            let mut pointer = PlanetViewPointer::default();
            pointer.press(Vec2::ZERO, scale_factor, PointerCapture::World);

            let below_threshold = pointer.move_to(Vec2::new(3.9 * scale_factor, 0.0), scale_factor);
            assert!(!below_threshold.dragging);
            assert_eq!(below_threshold.orbit_delta, Vec2::ZERO);

            let crossed = pointer.move_to(Vec2::new(4.0 * scale_factor, 0.0), scale_factor);
            assert!(crossed.dragging);
            assert!(crossed.began_dragging);
            assert_eq!(crossed.orbit_delta, Vec2::new(4.0, 0.0));
            assert_eq!(
                pointer.release(Vec2::new(4.0 * scale_factor, 0.0), scale_factor),
                Some(PointerRelease::WorldDrag)
            );
        }
    }

    #[test]
    fn drag_motion_reports_logical_ray_endpoints_across_the_threshold() {
        let mut pointer = PlanetViewPointer::default();
        pointer.press(Vec2::new(100.0, 80.0), 2.0, PointerCapture::World);
        let below_threshold = pointer.move_to(Vec2::new(106.0, 80.0), 2.0);
        assert_eq!(below_threshold.position, Vec2::new(53.0, 40.0));

        let crossed = pointer.move_to(Vec2::new(114.0, 84.0), 2.0);
        assert!(crossed.began_dragging);
        assert_eq!(crossed.previous_position, Vec2::new(50.0, 40.0));
        assert_eq!(crossed.position, Vec2::new(57.0, 42.0));

        let continued = pointer.move_to(Vec2::new(118.0, 82.0), 2.0);
        assert_eq!(continued.previous_position, crossed.position);
        assert_eq!(continued.position, Vec2::new(59.0, 41.0));
    }

    #[test]
    fn a_control_gesture_remains_captured_through_release() {
        let mut pointer = PlanetViewPointer::default();
        pointer.press(Vec2::new(20.0, 30.0), 2.0, PointerCapture::Interface);
        let motion = pointer.move_to(Vec2::new(40.0, 30.0), 2.0);

        assert!(motion.dragging);
        assert_eq!(motion.orbit_delta, Vec2::ZERO);
        assert!(pointer.is_captured());
        assert_eq!(
            pointer.release(Vec2::new(40.0, 30.0), 2.0),
            Some(PointerRelease::CapturedDrag(PointerCapture::Interface))
        );
        assert!(!pointer.is_captured());
    }

    #[test]
    fn a_short_world_gesture_is_a_click() {
        let mut pointer = PlanetViewPointer::default();
        pointer.press(Vec2::new(10.0, 15.0), 1.0, PointerCapture::World);
        assert_eq!(
            pointer.move_to(Vec2::new(13.0, 15.0), 1.0).orbit_delta,
            Vec2::ZERO
        );
        assert_eq!(
            pointer.release(Vec2::new(13.0, 15.0), 1.0),
            Some(PointerRelease::WorldClick(Vec2::new(13.0, 15.0)))
        );
    }

    #[test]
    fn ui_fade_restores_each_elements_base_alpha_after_a_reversal() {
        let mut element = PlanetViewInterfaceElement::default();
        let original = Color::srgba(0.2, 0.4, 0.6, 0.5);
        let mut background = BackgroundColor(original);

        element.apply_opacity(0.25, Some(&mut background), None, None, None);
        assert!((background.0.alpha() - 0.125).abs() < 0.001);
        element.apply_opacity(1.0, Some(&mut background), None, None, None);
        assert!((background.0.alpha() - 0.5).abs() < 0.001);
    }
}
