use bevy::prelude::Color;

/// Orng theme (dark palette) from opencode's `orng.json`.
pub const NEUTRAL: Color = Color::srgb(0.039, 0.039, 0.039); // #0a0a0a
pub const INK: Color = Color::srgb(0.933, 0.933, 0.933); // #eeeeee
pub const PRIMARY: Color = Color::srgb(0.925, 0.357, 0.169); // #EC5B2B
pub const ACCENT: Color = Color::srgb(1.0, 0.969, 0.945); // #FFF7F1
pub const SUCCESS: Color = Color::srgb(0.42, 0.631, 0.902); // #6ba1e6
pub const WARNING: Color = Color::srgb(0.925, 0.357, 0.169); // #EC5B2B
pub const ERROR: Color = Color::srgb(0.878, 0.424, 0.459); // #e06c75
pub const INFO: Color = Color::srgb(0.337, 0.714, 0.761); // #56b6c2
pub const TEXT_WEAK: Color = Color::srgb(0.502, 0.502, 0.502); // #808080

/// Panel background: neutral with slight transparency for the sidebars.
pub const PANEL_BG: Color = Color::srgba(0.039, 0.039, 0.039, 0.9);
/// Button surface: a touch lighter than the panel.
pub const SURFACE: Color = Color::srgb(0.09, 0.09, 0.09);
/// Divider / subtle border color.
pub const BORDER: Color = Color::srgb(0.502, 0.502, 0.502);

pub const FONT_PATH: &str = "fonts/MonaspaceNeon-Regular.otf";
