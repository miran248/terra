use avian3d::prelude::*;
use bevy::prelude::*;

pub struct PhysicsPlugin;

#[derive(Component)]
pub struct RadialGravity;

/// Keep an asymmetric actor collider upright through Avian's body rotation.
#[derive(Component)]
pub struct RadialUpright;

impl Plugin for PhysicsPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Gravity::ZERO).add_systems(
            FixedUpdate,
            (align_radial_bodies, apply_radial_gravity).chain(),
        );
    }
}

fn apply_radial_gravity(mut bodies: Query<(Forces, &Position), With<RadialGravity>>) {
    for (mut forces, pos) in &mut bodies {
        let dir = pos.0.normalize_or_zero();
        if dir.length_squared() > 0.0 {
            forces.apply_force(-dir * 10000.0); // continuous downward
        }
    }
}

fn align_radial_bodies(mut bodies: Query<(&Position, &mut Rotation), With<RadialUpright>>) {
    for (position, mut rotation) in &mut bodies {
        rotation.0 = Quat::from_rotation_arc(Vec3::Y, position.0.normalize_or(Vec3::Y));
    }
}
