use bevy_color::Color;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Terrain {
    Ocean,
    Lake,
    SaltLake,
    LakeShore,
    River,
    RiverBank,
    Beach,
    Cliff,
    Desert,
    Plains,
    Forest,
    Tundra,
    Mountain,
    Snow,
    Swamp,
    Jungle,
    Savanna,
    Volcanic,
    Glacier,
    /// Ground-contact source of a river.
    RiverSpring,
}

impl Terrain {
    pub const ALL: [Terrain; 20] = [
        Terrain::Ocean,
        Terrain::Lake,
        Terrain::SaltLake,
        Terrain::LakeShore,
        Terrain::River,
        Terrain::RiverBank,
        Terrain::Beach,
        Terrain::Cliff,
        Terrain::Desert,
        Terrain::Plains,
        Terrain::Forest,
        Terrain::Tundra,
        Terrain::Mountain,
        Terrain::Snow,
        Terrain::Swamp,
        Terrain::Jungle,
        Terrain::Savanna,
        Terrain::Volcanic,
        Terrain::Glacier,
        Terrain::RiverSpring,
    ];

    pub fn color(&self) -> Color {
        match self {
            Terrain::Ocean => Color::srgb(0.10, 0.25, 0.55),
            Terrain::Lake => Color::srgb(0.15, 0.35, 0.65),
            Terrain::SaltLake => Color::srgb(0.18, 0.48, 0.58),
            Terrain::LakeShore => Color::srgb(0.20, 0.48, 0.55),
            Terrain::River => Color::srgb(0.20, 0.45, 0.75),
            Terrain::RiverBank => Color::srgb(0.25, 0.50, 0.55),
            Terrain::Beach => Color::srgb(0.85, 0.78, 0.55),
            Terrain::Cliff => Color::srgb(0.50, 0.40, 0.35),
            Terrain::Desert => Color::srgb(0.80, 0.70, 0.40),
            Terrain::Plains => Color::srgb(0.35, 0.55, 0.25),
            Terrain::Forest => Color::srgb(0.15, 0.38, 0.18),
            Terrain::Tundra => Color::srgb(0.55, 0.58, 0.52),
            Terrain::Mountain => Color::srgb(0.45, 0.42, 0.40),
            Terrain::Snow => Color::srgb(0.92, 0.94, 0.97),
            Terrain::Swamp => Color::srgb(0.28, 0.35, 0.22),
            Terrain::Jungle => Color::srgb(0.09, 0.30, 0.11),
            Terrain::Savanna => Color::srgb(0.64, 0.60, 0.30),
            Terrain::Volcanic => Color::srgb(0.19, 0.15, 0.15),
            Terrain::Glacier => Color::srgb(0.80, 0.88, 0.93),
            Terrain::RiverSpring => Color::srgb(0.32, 0.65, 0.88),
        }
    }

    pub fn is_water(&self) -> bool {
        matches!(
            self,
            Terrain::Ocean
                | Terrain::Lake
                | Terrain::SaltLake
                | Terrain::River
                | Terrain::RiverSpring
        )
    }

    pub fn is_lake(&self) -> bool {
        matches!(self, Terrain::Lake | Terrain::SaltLake)
    }

    pub fn is_land(&self) -> bool {
        !self.is_water()
    }

    /// A shore/transition kind — the band where water meets land. Not water
    /// itself, but not a solid land biome either.
    pub fn is_shore(&self) -> bool {
        matches!(
            self,
            Terrain::LakeShore | Terrain::RiverBank | Terrain::Beach | Terrain::Cliff
        )
    }

    /// A solid land biome — land that is neither water nor a shore transition.
    pub fn is_land_biome(&self) -> bool {
        self.is_land() && !self.is_shore()
    }
}
