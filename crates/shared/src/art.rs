//! Canonical contracts shared by the procedural asset generator and runtime.

use crate::{
    items::{Material, WeaponKind},
    level::{FloraKind, StructureKind},
};

pub const ENVIRONMENT_CATALOG: &str = "models/environment.glb";
pub const STRUCTURES_CATALOG: &str = "models/structures.glb";
pub const ITEMS_CATALOG: &str = "models/items.glb";
pub const ACTORS_CATALOG: &str = "models/actors.glb";
pub const ACTOR_ANIMATIONS: [&str; 3] = ["idle", "walk", "attack"];

pub const FLORA_KINDS: [FloraKind; 11] = [
    FloraKind::Tree,
    FloraKind::Bush,
    FloraKind::Flower,
    FloraKind::Rock,
    FloraKind::Grass,
    FloraKind::Log,
    FloraKind::Mushroom,
    FloraKind::Cactus,
    FloraKind::Berry,
    FloraKind::DeadTree,
    FloraKind::Reed,
];
pub const STRUCTURE_KINDS: [StructureKind; 7] = [
    StructureKind::Ruin,
    StructureKind::Watchtower,
    StructureKind::Dock,
    StructureKind::Farm,
    StructureKind::Wall,
    StructureKind::Well,
    StructureKind::Campfire,
];

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ColliderSpec {
    None,
    Box { half_extents: [f32; 3] },
    Capsule { radius: f32, half_length: f32 },
}

pub trait AssetName {
    fn asset_name(self) -> &'static str;
}

impl AssetName for FloraKind {
    fn asset_name(self) -> &'static str {
        match self {
            Self::Tree => "flora.tree",
            Self::Bush => "flora.bush",
            Self::Flower => "flora.flower",
            Self::Rock => "flora.rock",
            Self::Grass => "flora.grass",
            Self::Log => "flora.log",
            Self::Mushroom => "flora.mushroom",
            Self::Cactus => "flora.cactus",
            Self::Berry => "flora.berry",
            Self::DeadTree => "flora.dead_tree",
            Self::Reed => "flora.reed",
        }
    }
}

impl AssetName for StructureKind {
    fn asset_name(self) -> &'static str {
        match self {
            Self::Ruin => "structure.ruin",
            Self::Watchtower => "structure.watchtower",
            Self::Dock => "structure.dock",
            Self::Farm => "structure.farm",
            Self::Wall => "structure.wall",
            Self::Well => "structure.well",
            Self::Campfire => "structure.campfire",
        }
    }
}

impl AssetName for Material {
    fn asset_name(self) -> &'static str {
        match self {
            Self::Metal => "material.metal",
            Self::Wood => "material.wood",
            Self::Rope => "material.rope",
            Self::Cloth => "material.cloth",
        }
    }
}

impl AssetName for WeaponKind {
    fn asset_name(self) -> &'static str {
        match self {
            Self::Knife => "weapon.knife",
            Self::Spear => "weapon.spear",
            Self::Pistol => "weapon.pistol",
            Self::Sling => "weapon.sling",
            Self::Rifle => "weapon.rifle",
        }
    }
}

pub fn flora_collider(kind: FloraKind) -> ColliderSpec {
    match kind {
        FloraKind::Tree | FloraKind::DeadTree => ColliderSpec::Capsule {
            radius: 0.45,
            half_length: 2.0,
        },
        FloraKind::Cactus => ColliderSpec::Capsule {
            radius: 0.3,
            half_length: 0.8,
        },
        FloraKind::Rock | FloraKind::Log => ColliderSpec::Box {
            half_extents: [0.7, 0.5, 0.7],
        },
        _ => ColliderSpec::None,
    }
}

pub fn deterministic_variant(seed: u32, stable_index: u32, variants: u32) -> u32 {
    assert!(variants > 0);
    let mut x = seed ^ stable_index.wrapping_mul(0x9e37_79b9);
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x % variants
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_are_unique_and_complete() {
        let mut names: Vec<_> = FLORA_KINDS.into_iter().map(AssetName::asset_name).collect();
        names.extend(STRUCTURE_KINDS.into_iter().map(AssetName::asset_name));
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), FLORA_KINDS.len() + STRUCTURE_KINDS.len());
    }
    #[test]
    fn variants_are_deterministic_and_bounded() {
        assert_eq!(
            deterministic_variant(42, 7, 3),
            deterministic_variant(42, 7, 3)
        );
        assert!(deterministic_variant(42, 7, 3) < 3);
    }
}
