use bevy::prelude::*;

// All distances are in meters (1 world unit = 1 m). See `shared::sphere::METER`.

pub const SURVIVOR_SIZE: f32 = 2.0;
pub const SURVIVOR_HP: f32 = 500.0;
pub const SURVIVOR_COLOR: Color = Color::srgb(0.1, 0.3, 0.8);
pub const SURVIVOR_SPEED: f32 = 60.0; // m/s
pub const SURVIVOR_TURN: f32 = 2.5; // rad/s

pub const ZOMBIE_SIZE: f32 = 2.0;
pub const ZOMBIE_COLOR: Color = Color::srgb(0.8, 0.15, 0.15);
pub const SPAWN_RADIUS: f32 = 120.0; // m, just beyond the camera's footprint

pub const PLANET_SEED: u32 = 1337;

pub const CAMERA_HEIGHT: f32 = 90.0; // m above the player
pub const CAMERA_BACK: f32 = 45.0; // m behind the heading

pub const ATTACK_RANGE: f32 = 50.0; // m
pub const ATTACK_INTERVAL: f32 = 0.8;
pub const ATTACK_DAMAGE: f32 = 10.0;

pub const PROJECTILE_COLOR: Color = Color::srgb(0.9, 0.9, 0.1);
pub const PROJECTILE_SIZE: f32 = 0.4; // m

pub const SCRAP_SIZE: f32 = 0.6; // m
