use avian3d::prelude::*;
use bevy::prelude::*;

pub struct PhysicsPlugin;

#[derive(Component)]
pub struct RadialGravity;

impl Plugin for PhysicsPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Gravity::ZERO)
            .add_systems(FixedUpdate, apply_radial_gravity);
    }
}

fn apply_radial_gravity(
    mut bodies: Query<(Forces, &Position), With<RadialGravity>>,
) {
    for (mut forces, pos) in &mut bodies {
        let dir = pos.0.normalize_or_zero();
        if dir.length_squared() > 0.0 {
            forces.apply_linear_acceleration(-dir * 200.0);
        }
    }
}
