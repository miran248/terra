use terra_world::level::{FloraKind, SceneryKind};
use terra_world::terrain::Terrain;

/// Number of generated mesh variants available for a scenery kind.
pub fn scenery_variant_count(kind: SceneryKind) -> u32 {
    match kind {
        SceneryKind::Flora(FloraKind::Tree) => 7,
        SceneryKind::Flora(FloraKind::Bush)
        | SceneryKind::Rock
        | SceneryKind::Flora(FloraKind::Cactus)
        | SceneryKind::DeadTree => 2,
        _ => 1,
    }
}

/// Pick a stable scenery mesh variant for a terrain context and position hash.
pub fn scenery_variant_for(kind: SceneryKind, terrain: Terrain, hash: u64) -> u8 {
    let total = scenery_variant_count(kind);
    if total <= 1 {
        return 0;
    }
    if kind == SceneryKind::Flora(FloraKind::Tree) {
        let range: (u64, u64) = match terrain {
            Terrain::Desert => (3, 4),
            Terrain::Savanna => (0, 5),
            Terrain::Forest | Terrain::Plains | Terrain::RiverBank | Terrain::LakeShore => (0, 6),
            Terrain::Jungle => (1, 3),
            Terrain::Swamp => (1, 5),
            Terrain::Tundra | Terrain::Snow => (2, 5),
            Terrain::Glacier => (2, 3),
            _ => (0, total as u64),
        };
        let span = range.1 - range.0;
        (range.0 + (hash % span)) as u8
    } else {
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
}
