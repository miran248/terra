use bevy::prelude::*;

// All distances are in meters (1 world unit = 1 m). See `shared::sphere::METER`.

pub const PLAYER_SIZE: f32 = 0.55; // capsule totals ~1m tall
pub const PLAYER_HP: f32 = 500.0;
pub const PLAYER_COLOR: Color = Color::srgb(0.1, 0.3, 0.8);
pub const PLAYER_SPEED: f32 = 60.0; // m/s
pub const PLAYER_TURN: f32 = 2.5; // rad/s

pub const ZOMBIE_SIZE: f32 = 2.0;
pub const ZOMBIE_COLOR: Color = Color::srgb(0.8, 0.15, 0.15);
pub const SPAWN_RADIUS: f32 = 120.0; // m, just beyond the camera's footprint

pub const CAMERA_HEIGHT: f32 = 4.0; // m above the player
pub const CAMERA_BACK: f32 = 9.0; // m behind the heading
pub const CAMERA_LOOK_AHEAD: f32 = 45.0; // m ahead — low camera, aimed at the horizon

pub const ATTACK_RANGE: f32 = 50.0; // m
pub const ATTACK_INTERVAL: f32 = 0.8;
pub const ATTACK_DAMAGE: f32 = 10.0;

pub const PROJECTILE_COLOR: Color = Color::srgb(0.9, 0.9, 0.1);
pub const PROJECTILE_SIZE: f32 = 0.4; // m

pub const SCRAP_SIZE: f32 = 0.6; // m
