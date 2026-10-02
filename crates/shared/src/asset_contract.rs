//! Authoritative candidate dimensions and simplified physical surfaces, in glTF meters.
use serde::Deserialize;
use std::{collections::BTreeMap, sync::LazyLock};

#[derive(Debug, Deserialize)]
pub struct AssetContract {
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
