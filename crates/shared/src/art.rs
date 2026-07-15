//! Canonical contracts shared by the procedural asset generator and runtime.

use crate::{
    items::{Material, WeaponKind},
    level::{FloraKind, StructureKind},
    terrain::Terrain,
};

pub const ENVIRONMENT_CATALOG: &str = "models/environment.glb";
pub const STRUCTURES_CATALOG: &str = "models/structures.glb";
pub const ITEMS_CATALOG: &str = "models/items.glb";
pub const ACTORS_CATALOG: &str = "models/actors.glb";
pub const ACTOR_ANIMATIONS: [&str; 3] = ["idle", "walk", "attack"];

pub const FLORA_KINDS: [FloraKind; 25] = [
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
    FloraKind::Seaweed,
    FloraKind::Lilypad,
    FloraKind::Coral,
    FloraKind::Anemone,
    FloraKind::Starfish,
    FloraKind::Shell,
    FloraKind::Kelp,
    FloraKind::Cattail,
    FloraKind::Vine,
    FloraKind::Tumbleweed,
    FloraKind::Skull,
    FloraKind::Snowdrift,
    FloraKind::Stump,
    FloraKind::Fern,
];
pub const STRUCTURE_KINDS: [StructureKind; 17] = [
    StructureKind::Ruin,
    StructureKind::Watchtower,
    StructureKind::Dock,
    StructureKind::Farm,
    StructureKind::Wall,
    StructureKind::Well,
    StructureKind::Campfire,
    StructureKind::Tent,
    StructureKind::Crate,
    StructureKind::Fence,
    StructureKind::Barricade,
    StructureKind::LampPost,
    StructureKind::Signpost,
    StructureKind::Guardrail,
    StructureKind::Railing,
    StructureKind::Suspension,
    StructureKind::House,
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
            Self::Seaweed => "flora.seaweed",
            Self::Lilypad => "flora.lilypad",
            Self::Coral => "flora.coral",
            Self::Anemone => "flora.anemone",
            Self::Starfish => "flora.starfish",
            Self::Shell => "flora.shell",
            Self::Kelp => "flora.kelp",
            Self::Cattail => "flora.cattail",
            Self::Vine => "flora.vine",
            Self::Tumbleweed => "flora.tumbleweed",
            Self::Skull => "flora.skull",
            Self::Snowdrift => "flora.snowdrift",
            Self::Stump => "flora.stump",
            Self::Fern => "flora.fern",
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
            Self::Tent => "structure.tent",
            Self::Crate => "structure.crate",
            Self::Fence => "structure.fence",
            Self::Barricade => "structure.barricade",
            Self::LampPost => "structure.lamp_post",
            Self::Signpost => "structure.signpost",
            Self::Guardrail => "structure.guardrail",
            Self::Railing => "structure.railing",
            Self::Suspension => "structure.suspension",
            Self::House => "structure.house",
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
        FloraKind::Rock => ColliderSpec::Box {
            half_extents: [0.7, 0.5, 0.7],
        },
        FloraKind::Snowdrift => ColliderSpec::Box {
            half_extents: [0.5, 0.3, 0.5],
        },
        _ => ColliderSpec::None,
    }
}

/// How many mesh variants a flora kind has (indexed 0..count).
pub fn flora_variant_count(kind: FloraKind) -> u32 {
    match kind {
        FloraKind::Tree => 4,
        FloraKind::Bush => 2,
        FloraKind::Rock => 2,
        FloraKind::Cactus => 2,
        FloraKind::DeadTree => 2,
        _ => 1,
    }
}

/// Stable scene name for a specific flora kind + variant index.
pub fn flora_variant_name(kind: FloraKind, variant: u32) -> String {
    if flora_variant_count(kind) <= 1 {
        kind.asset_name().to_owned()
    } else {
        format!("{}.{variant}", kind.asset_name())
    }
}

/// Pick the flora variant index for a given terrain context, deterministically
/// from position hash so nearby instances of the same kind on the same terrain
/// vary.
pub fn flora_variant_for(kind: FloraKind, terrain: Terrain, hash: u64) -> u8 {
    let total = flora_variant_count(kind);
    if total <= 1 {
        return 0;
    }
    // biome key — groups terrains that should share shapes
    let biome_key: u8 = match terrain {
        Terrain::Desert | Terrain::Savanna => 0,
        Terrain::Forest | Terrain::Plains | Terrain::RiverBank | Terrain::LakeShore => 1,
        Terrain::Jungle | Terrain::Swamp => 2,
        Terrain::Tundra | Terrain::Snow | Terrain::Glacier => 3,
        _ => 1,
    };
    let seed = (biome_key as u64).wrapping_mul(0x9e37_79b9) ^ hash;
    (seed % total as u64) as u8
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
    #[test]
    fn flora_variants_valid() {
        for kind in FLORA_KINDS {
            let n = flora_variant_count(kind);
            assert!(n >= 1);
            for v in 0..n {
                let name = flora_variant_name(kind, v);
                if n <= 1 {
                    assert_eq!(name, kind.asset_name());
                } else {
                    assert!(name.starts_with(kind.asset_name()));
                }
            }
        }
    }
}
