//! Authoritative candidate dimensions and simplified physical surfaces, in glTF meters.
use serde::Deserialize;
use std::{collections::BTreeMap, sync::LazyLock};

#[derive(Debug, Deserialize)]
pub struct AssetContract {
    /// Nominal local-space displacement between adjacent repeatable modules.
    pub repeat_step: Option<[f32; 3]>,
    pub grip: Option<[f32; 3]>,
    /// Optional hand-local orientation; absent means authored +Y points forward.
    pub grip_rotation: Option<[f32; 4]>,
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

/// Place an authored grip on a hand socket, using its authored hand-local orientation.
pub fn grip_transform(name: &str) -> Option<bevy::prelude::Transform> {
    use bevy::prelude::*;
    let contract = candidate_contract(name)?;
    let grip = Vec3::from_array(contract.grip?);
    let rotation = contract
        .grip_rotation
        .map(Quat::from_array)
        .unwrap_or_else(|| Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2));
    Some(Transform::from_rotation(rotation).with_translation(-(rotation * grip)))
}

#[cfg(test)]
mod tests {
    #[test]
    fn all_weapons_anchor_to_the_hand_with_sling_hanging_below_it() {
        use crate::art::AssetName;
        use bevy::prelude::*;
        for kind in crate::items::WeaponKind::ALL {
            let name = kind.asset_name();
            let contract = super::candidate_contract(name).unwrap();
            let transform = super::grip_transform(name).unwrap();
            assert!(
                transform
                    .transform_point(Vec3::from_array(contract.grip.unwrap()))
                    .length()
                    < 1e-6
            );
            if name == "weapon.sling" {
                assert!(transform.transform_point(Vec3::ZERO).y < -0.3);
            } else {
                assert!((transform.rotation * Vec3::Y - Vec3::NEG_Z).length() < 1e-6);
            }
        }
    }

    #[test]
    fn every_terrain_selected_scenery_variant_has_a_dimension_contract() {
        use crate::art::{SCENERY_KINDS, scenery_variant_name};
        use terra_world::terrain::Terrain;
        use terra_worldgen::scenery::{scenery_variant_count, scenery_variant_for};
        let mut names = std::collections::BTreeSet::new();
        for kind in SCENERY_KINDS {
            for variant in 0..scenery_variant_count(kind) {
                let name = scenery_variant_name(kind, variant);
                let contract = super::candidate_contract(&name).expect(&name);
                assert!(contract.dimensions.iter().all(|d| d.is_finite() && *d > 0.));
                names.insert(name);
            }
            for terrain in [
                Terrain::Forest,
                Terrain::Jungle,
                Terrain::Desert,
                Terrain::Savanna,
                Terrain::Swamp,
                Terrain::Snow,
                Terrain::Glacier,
            ] {
                for hash in 0..64 {
                    let variant = scenery_variant_for(kind, terrain, hash);
                    assert!(
                        super::candidate_contract(&scenery_variant_name(kind, variant.into()))
                            .is_some()
                    );
                }
            }
        }
        assert_eq!(names.len(), 37);
    }

    #[test]
    fn equipped_knife_places_its_grip_at_the_hand_and_points_forward() {
        use bevy::prelude::*;
        let transform = super::grip_transform("weapon.knife").unwrap();
        assert!(transform.transform_point(Vec3::new(0., 0.052, 0.)).length() < 1e-6);
        assert!((transform.rotation * Vec3::Y - Vec3::NEG_Z).length() < 1e-6);
    }
}
