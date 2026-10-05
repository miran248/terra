use bevy::prelude::*;

// All distances are in meters (1 world unit = 1 m). See `terra_geometry::sphere::METER`.

pub const PLAYER_SIZE: f32 = 0.55; // capsule totals ~1m tall
pub const PLAYER_HP: f32 = 500.0;
pub const PLAYER_TURN: f32 = 2.5; // rad/s

pub const ZOMBIE_SIZE: f32 = 2.0;
pub const SPAWN_RADIUS: f32 = 120.0; // m, just beyond the camera's footprint

pub const ATTACK_RANGE: f32 = 50.0; // m
pub const ATTACK_INTERVAL: f32 = 0.8;
pub const ATTACK_DAMAGE: f32 = 10.0;

pub const PROJECTILE_COLOR: Color = Color::srgb(0.9, 0.9, 0.1);
pub const PROJECTILE_SIZE: f32 = 0.4; // m

pub const SCRAP_SIZE: f32 = 0.6; // m
