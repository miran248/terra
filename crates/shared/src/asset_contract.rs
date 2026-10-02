//! Authoritative candidate dimensions and simplified physical surfaces, in glTF meters.
use serde::Deserialize;
use std::{collections::BTreeMap, sync::LazyLock};

#[derive(Debug, Deserialize)]
pub struct AssetContract {
    pub grip: Option<[f32; 3]>,
    pub dimensions: [f32; 3],
    pub colliders: Vec<CollisionPart>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum CollisionPart {
    Box {
        center: [f32; 3],
        size: [f32; 3],
        rotation: [f32; 4],
    },
    /// Length excludes the two hemispheres, matching Avian's constructor.
    Capsule {
        center: [f32; 3],
        radius: f32,
        length: f32,
    },
    Cylinder {
        center: [f32; 3],
        radius: f32,
        length: f32,
    },
}

pub fn candidate_contract(name: &str) -> Option<&'static AssetContract> {
    static CONTRACTS: LazyLock<BTreeMap<String, AssetContract>> = LazyLock::new(|| {
        serde_json::from_str(include_str!("../asset_dimensions.json"))
            .expect("checked-in asset dimension contract must be valid")
    });
    CONTRACTS.get(name)
}

/// Place an authored grip on a hand socket, with the blade pointing glTF forward.
pub fn grip_transform(name: &str) -> Option<bevy::prelude::Transform> {
    use bevy::prelude::*;
    let grip = Vec3::from_array(candidate_contract(name)?.grip?);
    let rotation = Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
    Some(Transform::from_rotation(rotation).with_translation(-(rotation * grip)))
}

#[cfg(test)]
mod tests {
    #[test]
    fn equipped_knife_places_its_grip_at_the_hand_and_points_forward() {
        use bevy::prelude::*;
        let transform = super::grip_transform("weapon.knife").unwrap();
        assert!(transform.transform_point(Vec3::new(0., 0.052, 0.)).length() < 1e-6);
        assert!((transform.rotation * Vec3::Y - Vec3::NEG_Z).length() < 1e-6);
    }
}
