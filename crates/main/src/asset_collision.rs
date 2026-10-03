//! Converts the shared, meter-based asset contract into runtime-owned Avian shapes.
use avian3d::prelude::*;
use bevy::prelude::*;

pub fn candidate_collider(name: &str, scale: Vec3) -> Option<Collider> {
    use shared::asset_contract::{CollisionPart, candidate_contract};
    let contract = candidate_contract(name)?;
    if contract.colliders.is_empty() {
        return None;
    }
    let shapes = contract
        .colliders
        .iter()
        .map(|part| match *part {
            CollisionPart::Box {
                center,
                size,
                rotation,
            } => (
                Vec3::from_array(center),
                Quat::from_array(rotation),
                Collider::cuboid(size[0], size[1], size[2]),
            ),
            CollisionPart::Capsule {
                center,
                radius,
                length,
            } => (
                Vec3::from_array(center),
                Quat::IDENTITY,
                Collider::capsule(radius, length),
            ),
            CollisionPart::Cylinder {
                center,
                radius,
                length,
            } => (
                Vec3::from_array(center),
                Quat::IDENTITY,
                Collider::cylinder(radius, length),
            ),
        })
        .collect();
    let mut collider = Collider::compound(shapes);
    collider.set_scale(scale, 16);
    Some(collider)
}

/// Dynamic actors use a body-center origin; exported scenes retain a ground pivot.
pub fn actor_body(name: &str) -> (Collider, f32) {
    use shared::asset_contract::CollisionPart;
    let contract = shared::asset_contract::candidate_contract(name).expect("actor contract");
    let [
        CollisionPart::Capsule {
            center,
            radius,
            length,
        },
    ] = contract.colliders.as_slice()
    else {
        panic!("actor body requires one upright capsule: {name}");
    };
    (Collider::capsule(*radius, *length), center[1])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_structure_collision_surfaces_meet_at_the_shared_spacing() {
        use avian3d::collision::collider::contact_query::contact;
        for name in [
            "fence",
            "wall",
            "guardrail",
            "railing",
            "dock",
            "suspension",
        ] {
            let name = format!("structure.{name}");
            let step = Vec3::from_array(
                shared::asset_contract::candidate_contract(&name)
                    .unwrap()
                    .repeat_step
                    .unwrap(),
            );
            let shape = candidate_collider(&name, Vec3::ONE).unwrap();
            assert!(
                contact(
                    &shape,
                    Vec3::ZERO,
                    Quat::IDENTITY,
                    &shape,
                    step,
                    Quat::IDENTITY,
                    0.005
                )
                .unwrap()
                .is_some(),
                "{name}"
            );
        }
    }

    #[test]
    fn structures_keep_their_open_spaces_and_solid_surfaces() {
        let inside = |name, point| {
            candidate_collider(name, Vec3::ONE).unwrap().contains_point(
                Vec3::ZERO,
                Quat::IDENTITY,
                point,
            )
        };
        assert!(!inside("structure.ruin", Vec3::new(0.7, 0.8, 0.61)));
        assert!(!inside("structure.lamp_post", Vec3::new(0.07, 0.8, 0.)));
        assert!(inside("structure.lamp_post", Vec3::new(0.31, 1.34, 0.)));
        assert!(!inside("structure.watchtower", Vec3::new(0., 1., 0.)));
        assert!(inside("structure.watchtower", Vec3::new(0., 2.04, 0.)));
        assert!(!inside("structure.well", Vec3::new(0., 0.23, 0.)));
        assert!(inside("structure.well", Vec3::new(0.39, 0.23, 0.)));
        assert!(!inside("structure.tent", Vec3::new(0., 0.6, 0.)));
        assert!(inside("structure.tent", Vec3::new(0., 0.6, 0.91)));
        assert!(inside("structure.tent", Vec3::new(0.4, 0.69, 0.)));
        assert!(inside("structure.dock", Vec3::new(0., 0.41, 0.)));
        assert!(!inside("structure.dock", Vec3::new(0., 0.15, 0.)));
    }

    #[test]
    fn nonblocking_foliage_has_no_physics_shape() {
        for name in [
            "scenery.grass",
            "scenery.fern",
            "scenery.flower",
            "scenery.reed",
        ] {
            assert!(candidate_collider(name, Vec3::ONE).is_none());
        }
    }

    #[test]
    fn scenery_instance_scale_keeps_tree_collision_under_its_canopy() {
        for variant in 0..7 {
            for scale in [0.7, 1., 1.3] {
                let tree =
                    candidate_collider(&format!("scenery.tree.{variant}"), Vec3::splat(scale))
                        .unwrap();
                assert!(tree.contains_point(
                    Vec3::ZERO,
                    Quat::IDENTITY,
                    Vec3::new(0., 0.5, 0.) * scale
                ));
                assert!(!tree.contains_point(
                    Vec3::ZERO,
                    Quat::IDENTITY,
                    Vec3::new(0.4, 0.5, 0.) * scale
                ));
            }
        }
    }

    #[test]
    fn trunk_blocks_its_wood_but_leaves_space_under_the_canopy() {
        let tree = candidate_collider("scenery.tree.0", Vec3::ONE).unwrap();
        assert!(tree.contains_point(Vec3::ZERO, Quat::IDENTITY, Vec3::new(0., 0.8, 0.)));
        assert!(!tree.contains_point(Vec3::ZERO, Quat::IDENTITY, Vec3::new(0.4, 0.8, 0.)));
        let actor = candidate_collider("actor.player", Vec3::ONE).unwrap();
        assert!(actor.contains_point(Vec3::ZERO, Quat::IDENTITY, Vec3::new(0., 0.01, 0.)));
        assert!(!actor.contains_point(Vec3::ZERO, Quat::IDENTITY, Vec3::new(0., -0.01, 0.)));
    }
    #[test]
    fn scaled_shapes_preserve_full_lengths_ground_pivots_and_house_interior() {
        let tree = candidate_collider("scenery.tree.0", Vec3::splat(2.)).unwrap();
        assert!(tree.contains_point(Vec3::ZERO, Quat::IDENTITY, Vec3::new(0.19, 3.59, 0.)));
        assert!(!tree.contains_point(Vec3::ZERO, Quat::IDENTITY, Vec3::new(0.21, 1., 0.)));
        let actor = candidate_collider("actor.player", Vec3::ONE).unwrap();
        assert!(actor.contains_point(Vec3::ZERO, Quat::IDENTITY, Vec3::new(0., 0.99, 0.)));
        assert!(!actor.contains_point(Vec3::ZERO, Quat::IDENTITY, Vec3::new(0., 1.01, 0.)));
        let house = candidate_collider("structure.house", Vec3::ONE).unwrap();
        assert!(!house.contains_point(Vec3::ZERO, Quat::IDENTITY, Vec3::new(0., 0.8, 0.)));
        assert!(house.contains_point(Vec3::ZERO, Quat::IDENTITY, Vec3::new(0., 0.8, -0.932)));
        assert!(house.contains_point(Vec3::ZERO, Quat::IDENTITY, Vec3::new(0., 0.13, 0.)));
        assert!(!house.contains_point(Vec3::ZERO, Quat::IDENTITY, Vec3::new(0., 0.16, 0.)));
    }

    #[test]
    fn humanoid_contacts_trunk_and_ground_without_contacting_empty_canopy_space() {
        use avian3d::collision::collider::contact_query::contact;
        let actor = candidate_collider("actor.player", Vec3::ONE).unwrap();
        let tree = candidate_collider("scenery.tree.0", Vec3::ONE).unwrap();
        let touches = |x| {
            contact(
                &actor,
                Vec3::X * x,
                Quat::IDENTITY,
                &tree,
                Vec3::ZERO,
                Quat::IDENTITY,
                0.,
            )
            .unwrap()
            .is_some()
        };
        assert!(!touches(0.4));
        assert!(touches(0.15));
    }

    #[test]
    fn runtime_body_settles_with_its_ground_pivot_on_the_floor() {
        use bevy::time::TimeUpdateStrategy;
        use std::time::Duration;
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            bevy::mesh::MeshPlugin,
            TransformPlugin,
            PhysicsPlugins::default(),
        ))
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
            1. / 60.,
        )));
        app.world_mut().spawn((
            RigidBody::Static,
            Collider::cuboid(10., 0.2, 10.),
            Transform::from_xyz(0., -0.1, 0.),
        ));
        let actor = app
            .world_mut()
            .spawn((
                RigidBody::Dynamic,
                candidate_collider("actor.player", Vec3::ONE).unwrap(),
                LockedAxes::ROTATION_LOCKED,
                Transform::from_xyz(0., 1., 0.),
            ))
            .id();
        app.finish();
        app.cleanup();
        for _ in 0..180 {
            app.update();
        }
        let position = app.world().get::<Position>(actor).unwrap();
        assert!(position.y.abs() < 0.03, "ground pivot: {position:?}");
    }
}
