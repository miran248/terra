use crate::terrain::Terrain;

/// Face render type from its three authoritative corner cells.
pub(super) fn derive_face(a: Terrain, b: Terrain, c: Terrain) -> Terrain {
    let priority = |terrain: Terrain| match terrain {
        Terrain::Beach | Terrain::Cliff | Terrain::LakeShore | Terrain::RiverBank => 0u8,
        terrain if terrain.is_water() => 1,
        _ => 2,
    };
    if a == b || a == c {
        a
    } else if b == c {
        b
    } else if [a, b, c]
        .iter()
        .filter(|terrain| terrain.is_water())
        .count()
        >= 2
    {
        [a, b, c]
            .into_iter()
            .filter(|terrain| terrain.is_water())
            .min_by_key(|terrain| *terrain as u8)
            .unwrap()
    } else {
        [a, b, c]
            .into_iter()
            .min_by_key(|terrain| (priority(*terrain), *terrain as u8))
            .unwrap()
    }
}

/// Counts ascending thresholds to produce an ordered classification bucket.
pub(super) fn bucket(value: f32, thresholds: &[f32]) -> u8 {
    thresholds
        .iter()
        .filter(|&&threshold| value >= threshold)
        .count() as u8
}

pub(super) fn size_range(terrain: Terrain) -> (usize, usize) {
    use Terrain::*;
    const UNBOUNDED: usize = usize::MAX;
    match terrain {
        Ocean => (4000, UNBOUNDED),
        Lake => (60, 3999),
        River | RiverSpring | LakeShore | RiverBank => (1, UNBOUNDED),
        Beach => (20, 60),
        Cliff => (1, 24),
        Desert | Plains | Forest | Tundra | Mountain | Snow | Swamp | Jungle | Savanna
        | Volcanic | Glacier => (40, UNBOUNDED),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn water_majority_keeps_river_mouth_open() {
        assert_eq!(
            derive_face(Terrain::River, Terrain::Ocean, Terrain::RiverBank),
            Terrain::Ocean
        );
    }
}
