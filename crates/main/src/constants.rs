use bevy::prelude::*;

pub const MAP_WIDTH: f32 = 1200.0;
pub const MAP_HEIGHT: f32 = 800.0;

pub const SURVIVOR_SIZE: f32 = 24.0;
pub const SURVIVOR_HP: f32 = 500.0;
pub const SURVIVOR_COLOR: Color = Color::srgb(0.1, 0.3, 0.8);

pub const ZOMBIE_SIZE: f32 = 16.0;
pub const ZOMBIE_COLOR: Color = Color::srgb(0.8, 0.15, 0.15);
pub const SPAWN_MARGIN: f32 = 32.0;

pub const GROUND_COLOR: Color = Color::srgb(0.15, 0.35, 0.15);

pub const ATTACK_RANGE: f32 = 150.0;
pub const ATTACK_INTERVAL: f32 = 0.8;
pub const ATTACK_DAMAGE: f32 = 10.0;

pub const PROJECTILE_COLOR: Color = Color::srgb(0.9, 0.9, 0.1);
pub const PROJECTILE_SIZE: f32 = 5.0;

pub const SCRAP_COLOR: Color = Color::srgb(0.9, 0.7, 0.1);
pub const SCRAP_SIZE: f32 = 8.0;
