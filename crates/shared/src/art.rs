//! Canonical contracts shared by the procedural asset generator and runtime.

use crate::items::{Material, WeaponKind};
use terra_world::level::{FloraKind, SceneryKind, StructureKind};
use terra_worldgen::scenery::scenery_variant_count;

pub const ENVIRONMENT_CATALOG: &str = "models/environment.glb";
pub const STRUCTURES_CATALOG: &str = "models/structures.glb";
pub const ITEMS_CATALOG: &str = "models/items.glb";
pub const ACTORS_CATALOG: &str = "models/actors.glb";
pub const ACTOR_ANIMATIONS: [&str; 3] = ["idle", "walk", "attack"];

pub const ACTOR_SCENES: [&str; 3] = ["actor.player", "actor.zombie.0", "actor.zombie.1"];

/// Stable semantic identities; physical files are one consolidated scene each.
pub fn asset_names() -> Vec<String> {
    let mut names = Vec::new();
    for kind in SCENERY_KINDS {
        for variant in 0..scenery_variant_count(kind) {
            names.push(scenery_variant_name(kind, variant));
        }
    }
    names.extend(STRUCTURE_KINDS.map(|kind| kind.asset_name().to_owned()));
    names.extend(Material::ALL.map(|kind| kind.asset_name().to_owned()));
    names.extend(WeaponKind::ALL.map(|kind| kind.asset_name().to_owned()));
    names.extend(ACTOR_SCENES.map(str::to_owned));
    names.extend(["vehicle.car", "vehicle.plane"].map(str::to_owned));
    names
}

pub fn asset_path(name: &str) -> String {
    format!("models/production/{name}.glb")
}

/// The plant-life subset of environmental scenery.
pub const FLORA_KINDS: [FloraKind; 14] = [
    FloraKind::Tree,
    FloraKind::Bush,
    FloraKind::Flower,
    FloraKind::Grass,
    FloraKind::Cactus,
    FloraKind::Berry,
    FloraKind::Reed,
    FloraKind::Seaweed,
    FloraKind::Lilypad,
    FloraKind::Kelp,
    FloraKind::Cattail,
    FloraKind::Vine,
    FloraKind::Tumbleweed,
    FloraKind::Fern,
];
/// Every environmental object generated into the environment catalog.
pub const SCENERY_KINDS: [SceneryKind; 27] = [
    SceneryKind::Flora(FloraKind::Tree),
    SceneryKind::Flora(FloraKind::Bush),
    SceneryKind::Flora(FloraKind::Flower),
    SceneryKind::Rock,
    SceneryKind::Flora(FloraKind::Grass),
    SceneryKind::Log,
    SceneryKind::Mushroom,
    SceneryKind::Flora(FloraKind::Cactus),
    SceneryKind::Flora(FloraKind::Berry),
    SceneryKind::DeadTree,
    SceneryKind::Flora(FloraKind::Reed),
    SceneryKind::Flora(FloraKind::Seaweed),
    SceneryKind::Flora(FloraKind::Lilypad),
    SceneryKind::Coral,
    SceneryKind::Anemone,
    SceneryKind::Starfish,
    SceneryKind::Shell,
    SceneryKind::Flora(FloraKind::Kelp),
    SceneryKind::Flora(FloraKind::Cattail),
    SceneryKind::Flora(FloraKind::Vine),
    SceneryKind::Flora(FloraKind::Tumbleweed),
    SceneryKind::Skull,
    SceneryKind::Snowdrift,
    SceneryKind::Stump,
    SceneryKind::Flora(FloraKind::Fern),
    SceneryKind::Icicle,
    SceneryKind::Snowman,
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

impl AssetName for SceneryKind {
    fn asset_name(self) -> &'static str {
        match self {
            Self::Flora(FloraKind::Tree) => "scenery.tree",
            Self::Flora(FloraKind::Bush) => "scenery.bush",
            Self::Flora(FloraKind::Flower) => "scenery.flower",
            Self::Rock => "scenery.rock",
            Self::Flora(FloraKind::Grass) => "scenery.grass",
            Self::Log => "scenery.log",
            Self::Mushroom => "scenery.mushroom",
            Self::Flora(FloraKind::Cactus) => "scenery.cactus",
            Self::Flora(FloraKind::Berry) => "scenery.berry",
            Self::DeadTree => "scenery.dead_tree",
            Self::Flora(FloraKind::Reed) => "scenery.reed",
            Self::Flora(FloraKind::Seaweed) => "scenery.seaweed",
            Self::Flora(FloraKind::Lilypad) => "scenery.lilypad",
            Self::Coral => "scenery.coral",
            Self::Anemone => "scenery.anemone",
            Self::Starfish => "scenery.starfish",
            Self::Shell => "scenery.shell",
            Self::Flora(FloraKind::Kelp) => "scenery.kelp",
            Self::Flora(FloraKind::Cattail) => "scenery.cattail",
            Self::Flora(FloraKind::Vine) => "scenery.vine",
            Self::Flora(FloraKind::Tumbleweed) => "scenery.tumbleweed",
            Self::Skull => "scenery.skull",
            Self::Snowdrift => "scenery.snowdrift",
            Self::Stump => "scenery.stump",
            Self::Flora(FloraKind::Fern) => "scenery.fern",
            Self::Icicle => "scenery.icicle",
            Self::Snowman => "scenery.snowman",
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

pub fn scenery_collider(kind: SceneryKind) -> ColliderSpec {
    match kind {
        SceneryKind::Rock => ColliderSpec::Box {
            half_extents: [0.7, 0.5, 0.7],
        },
        SceneryKind::Snowdrift => ColliderSpec::Box {
            half_extents: [0.5, 0.3, 0.5],
        },
        _ => ColliderSpec::None,
    }
}

/// Stable scene name for a specific scenery kind + variant index.
pub fn scenery_variant_name(kind: SceneryKind, variant: u32) -> String {
    if scenery_variant_count(kind) <= 1 {
        kind.asset_name().to_owned()
    } else {
        format!("{}.{variant}", kind.asset_name())
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
        let mut names: Vec<_> = SCENERY_KINDS
            .into_iter()
            .map(AssetName::asset_name)
            .collect();
        names.extend(STRUCTURE_KINDS.into_iter().map(AssetName::asset_name));
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), SCENERY_KINDS.len() + STRUCTURE_KINDS.len());
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
    fn scenery_variants_valid() {
        for kind in SCENERY_KINDS {
            let n = scenery_variant_count(kind);
            assert!(n >= 1);
            for v in 0..n {
                let name = scenery_variant_name(kind, v);
                if n <= 1 {
                    assert_eq!(name, kind.asset_name());
                } else {
                    assert!(name.starts_with(kind.asset_name()));
                }
            }
        }
    }

    #[test]
    fn scenery_catalog_has_unique_scenery_names_for_every_kind_and_variant() {
        let mut names = Vec::new();
        for kind in SCENERY_KINDS {
            let count = scenery_variant_count(kind);
            for variant in 0..count {
                let name = scenery_variant_name(kind, variant);
                assert!(name.starts_with("scenery."));
                names.push(name);
            }
        }

        names.sort();
        names.dedup();
        assert_eq!(
            names.len(),
            SCENERY_KINDS
                .iter()
                .map(|kind| scenery_variant_count(*kind) as usize)
                .sum::<usize>()
        );
    }

    #[test]
    fn flora_is_the_plant_subset_of_scenery() {
        let scenery_flora = SCENERY_KINDS
            .into_iter()
            .filter_map(|kind| match kind {
                SceneryKind::Flora(flora) => Some(flora),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(scenery_flora.as_slice(), &FLORA_KINDS);
        assert!(SCENERY_KINDS.contains(&SceneryKind::Rock));
        assert!(SCENERY_KINDS.contains(&SceneryKind::Mushroom));
    }
}
