use bevy_math::Vec3;

use crate::terrain::TerrainGen;
use crate::worldgen::{Grid, Painted, regions, size_range};
use terra_world::level::{Landform, RegionData, RegionKind, RegionMemberships};
use terra_world::terrain::Terrain;

use super::super::network::RoadGraph;

// ---- named regions: cell-owned connected geographic areas ----

/// Which natural region a cell belongs to from its surface cover. Mountain
/// ranges come from the separate landform layer.
fn terrain_region_class(terrain: Terrain) -> Option<RegionKind> {
    match terrain {
        Terrain::Ocean => Some(RegionKind::Ocean),
        Terrain::Lake => Some(RegionKind::Lake),
        Terrain::SaltLake => Some(RegionKind::SaltLake),
        Terrain::River | Terrain::RiverSpring => Some(RegionKind::River),
        Terrain::Beach => Some(RegionKind::Beach),
        Terrain::Cliff => Some(RegionKind::Cliff),
        Terrain::Forest => Some(RegionKind::Forest),
        Terrain::Desert => Some(RegionKind::Desert),
        Terrain::Mountain | Terrain::Snow => None,
        Terrain::Plains => Some(RegionKind::Plains),
        Terrain::Tundra => Some(RegionKind::Tundra),
        Terrain::Swamp => Some(RegionKind::Swamp),
        Terrain::Jungle => Some(RegionKind::Jungle),
        Terrain::Savanna => Some(RegionKind::Savanna),
        Terrain::Volcanic => Some(RegionKind::Volcano),
        Terrain::Glacier => Some(RegionKind::Glacier),
        Terrain::LakeShore | Terrain::RiverBank => None,
    }
}

/// Flood-fill cell-owned region components, then project those memberships to
/// query faces. Cell identity remains authoritative; each face chooses the
/// most represented region of each kind, with lower region ID breaking ties.
pub(in crate::worldgen) fn build_regions(
    grid: &Grid,
    terrain: &TerrainGen,
    cell_types: &[Terrain],
    landform: &[Landform],
    painted: &Painted,
    network: &RoadGraph,
) -> (Vec<RegionData>, RegionMemberships) {
    let mut cell_memberships = Vec::new();
    let mut regions: Vec<RegionData> = Vec::new();
    let mut kind_counts = [0usize; 18];

    let terrain_class = cell_types
        .iter()
        .copied()
        .map(terrain_region_class)
        .collect::<Vec<_>>();
    append_regions(
        grid,
        terrain,
        &terrain_class,
        &mut regions,
        &mut cell_memberships,
        &mut kind_counts,
    );

    let mountain_class = landform
        .iter()
        .map(|&kind| (kind == Landform::Mountains).then_some(RegionKind::MountainRange))
        .collect::<Vec<_>>();
    append_regions(
        grid,
        terrain,
        &mountain_class,
        &mut regions,
        &mut cell_memberships,
        &mut kind_counts,
    );

    let town_class = grid
        .topology
        .cells()
        .map(|cell| {
            painted
                .settlements
                .contains(cell)
                .then_some(RegionKind::Settlement)
        })
        .collect::<Vec<_>>();
    append_regions(
        grid,
        terrain,
        &town_class,
        &mut regions,
        &mut cell_memberships,
        &mut kind_counts,
    );

    for (connection_index, cells) in &network.road_cells {
        let region_index = regions.len() as u32;
        let connection = &network.connections[*connection_index as usize];
        let center = cells
            .iter()
            .map(|&cell| grid.cell_position(cell).0)
            .sum::<Vec3>()
            .normalize_or(Vec3::Y);
        regions.push(RegionData {
            name: connection.name.clone(),
            pos: center.to_array(),
            kind: RegionKind::Road,
        });
        cell_memberships.extend(cells.iter().map(|cell| (cell.index(), region_index)));
    }

    let cells = RegionMemberships::from_pairs(grid.cell_count(), cell_memberships);
    let faces = project_cell_memberships(grid, &cells, &regions);
    (regions, faces)
}

fn append_regions(
    grid: &Grid,
    terrain: &TerrainGen,
    class: &[Option<RegionKind>],
    regions: &mut Vec<RegionData>,
    memberships: &mut Vec<(usize, u32)>,
    kind_counts: &mut [usize; 18],
) {
    let partitioner = regions::RegionPartitioner::new(grid, class);
    let mut assigned = vec![None; grid.cell_count()];
    for start in grid.topology.cells() {
        let Some(kind) = class[start.index()] else {
            continue;
        };
        if assigned[start.index()].is_some() {
            continue;
        }
        let region_index = regions.len() as u32;
        let max_cells = match kind {
            RegionKind::Beach => size_range(Terrain::Beach).1,
            RegionKind::Cliff => size_range(Terrain::Cliff).1,
            _ => usize::MAX,
        };
        let cells = partitioner.claim(start, kind, region_index, max_cells, &mut assigned);
        let min_cells = match kind {
            RegionKind::Settlement | RegionKind::Road | RegionKind::River => 1,
            RegionKind::Forest => size_range(Terrain::Forest).0,
            RegionKind::Beach => size_range(Terrain::Beach).0,
            RegionKind::MountainRange => 1,
            _ => 4,
        };
        if cells.len() < min_cells {
            for cell in cells {
                assigned[cell.index()] = None;
            }
            continue;
        }
        let center = cells
            .iter()
            .map(|&cell| grid.cell_position(cell).0)
            .sum::<Vec3>()
            .normalize_or(Vec3::Y);
        let index = kind_counts[kind.rank()];
        kind_counts[kind.rank()] += 1;
        regions.push(RegionData {
            name: region_name(kind, index, center, terrain),
            pos: center.to_array(),
            kind,
        });
        memberships.extend(cells.into_iter().map(|cell| (cell.index(), region_index)));
    }
}

fn project_cell_memberships(
    grid: &Grid,
    cells: &RegionMemberships,
    regions: &[RegionData],
) -> RegionMemberships {
    let mut memberships = Vec::new();
    let mut counts = Vec::<(u32, u8)>::new();
    let mut winners = [None::<(u32, u8)>; 18];
    for face in grid.topology.faces() {
        counts.clear();
        for cell in grid.face_cells(face) {
            for &region in cells.region_ids_at(cell.index()) {
                if let Some((_, count)) = counts
                    .iter_mut()
                    .find(|(candidate, _)| *candidate == region)
                {
                    *count += 1;
                } else {
                    counts.push((region, 1));
                }
            }
        }
        winners.fill(None);
        for &(region, count) in &counts {
            let kind = regions[region as usize].kind;
            if kind == RegionKind::Road {
                // A junction can have more than one road identity at its cell.
                memberships.push((face.index(), region));
                continue;
            }
            let winner = &mut winners[kind.rank()];
            if winner.is_none_or(|(current, current_count)| {
                count > current_count || (count == current_count && region < current)
            }) {
                *winner = Some((region, count));
            }
        }
        memberships.extend(
            winners
                .iter()
                .flatten()
                .map(|(region, _)| (face.index(), *region)),
        );
    }
    RegionMemberships::from_pairs(grid.face_count(), memberships)
}

pub(in crate::worldgen) fn region_name(
    kind: RegionKind,
    idx: usize,
    cent: Vec3,
    terrain: &TerrainGen,
) -> String {
    const OCEAN: [&str; 20] = [
        "Azure",
        "Cobalt",
        "Cerulean",
        "Sapphire",
        "Indigo",
        "Teal",
        "Aquamarine",
        "Turquoise",
        "Navy",
        "Sky",
        "Marine",
        "Coral",
        "Lagoon",
        "Reef",
        "Abyss",
        "Trench",
        "Gulf",
        "Bay",
        "Strait",
        "Channel",
    ];
    const LAKE: [&str; 12] = [
        "Mirror", "Crystal", "Emerald", "Silver", "Misty", "Clear", "Loch", "Mere", "Tarn", "Pond",
        "Basin", "Hollow",
    ];
    const RIVER: [&str; 12] = [
        "Serpent", "Winding", "Rushing", "Silver", "Mossy", "Deep", "Brook", "Stream", "Creek",
        "Fork", "Bend", "Rapids",
    ];
    const BEACH: [&str; 10] = [
        "Silver",
        "Golden",
        "Pebble",
        "Shell",
        "Driftwood",
        "Coral",
        "Windswept",
        "Quiet",
        "Gull",
        "Smuggler's",
    ];
    const CLIFF: [&str; 8] = [
        "Raven", "Grey", "Storm", "White", "Shear", "Widow's", "Falcon", "Chalk",
    ];
    const FOREST: [&str; 10] = [
        "Elder", "Whisper", "Thorn", "Mossy", "Shadow", "Bright", "Tangle", "Hollow", "Fern",
        "Wolf",
    ];
    const DESERT: [&str; 6] = ["Amber", "Bone", "Shimmer", "Red", "Glass", "Silent"];
    const MOUNTAIN: [&str; 8] = [
        "Iron", "Grey", "Storm", "Frost", "Raven", "Broken", "Cloud", "Thunder",
    ];
    const PLAINS: [&str; 8] = [
        "Green", "Wide", "Amber", "Rolling", "Sunlit", "Long", "Low", "Open",
    ];
    const TUNDRA: [&str; 6] = ["Pale", "Frozen", "White", "Bitter", "Still", "North"];
    const ROAD: [&str; 8] = [
        "Old",
        "King's",
        "Salt",
        "Trade",
        "Pilgrim's",
        "Coastal",
        "High",
        "Low",
    ];
    const SWAMP: [&str; 6] = ["Murk", "Fen", "Bog", "Mire", "Black", "Sunken"];
    const JUNGLE: [&str; 6] = ["Verdant", "Emerald", "Tangle", "Vine", "Fever", "Green"];
    const SAVANNA: [&str; 6] = ["Amber", "Sun", "Dust", "Lion", "Wide", "Gold"];
    const VOLCANO: [&str; 6] = ["Ash", "Ember", "Cinder", "Smoke", "Molten", "Black"];
    const GLACIER: [&str; 6] = ["Frost", "White", "Blue", "Silent", "Everice", "North"];

    let pick = |pool: &[&str], suffixes: &[&str]| {
        format!(
            "{} {}",
            pool[idx % pool.len()],
            suffixes[(idx / pool.len()) % suffixes.len()]
        )
    };
    match kind {
        RegionKind::Ocean => pick(&OCEAN, &["Ocean", "Sea"]),
        RegionKind::Lake => pick(&LAKE, &["Lake"]),
        RegionKind::SaltLake => pick(&LAKE, &["Salt Lake"]),
        RegionKind::River => pick(&RIVER, &["River"]),
        RegionKind::Beach => pick(&BEACH, &["Beach", "Coast", "Sands"]),
        RegionKind::Cliff => pick(&CLIFF, &["Cliffs", "Bluffs"]),
        RegionKind::Forest => pick(&FOREST, &["Forest", "Woods"]),
        RegionKind::Desert => pick(&DESERT, &["Desert", "Dunes"]),
        RegionKind::MountainRange => pick(&MOUNTAIN, &["Peaks", "Range"]),
        RegionKind::Plains => pick(&PLAINS, &["Plains", "Fields"]),
        RegionKind::Tundra => pick(&TUNDRA, &["Tundra", "Wastes"]),
        RegionKind::Swamp => pick(&SWAMP, &["Swamp", "Marsh", "Fen"]),
        RegionKind::Jungle => pick(&JUNGLE, &["Jungle", "Rainforest"]),
        RegionKind::Savanna => pick(&SAVANNA, &["Savanna", "Plains"]),
        RegionKind::Volcano => pick(&VOLCANO, &["Peaks", "Fields", "Wastes"]),
        RegionKind::Glacier => pick(&GLACIER, &["Glacier", "Ice", "Wastes"]),
        RegionKind::Road => pick(&ROAD, &["Road"]),
        // Towns take the name of the settlement they surround.
        RegionKind::Settlement => terrain
            .settlement_anchors
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| a.0.dot(cent).partial_cmp(&b.0.dot(cent)).unwrap().reverse())
            .map(|(i, _)| crate::worldgen::settlement_name(i))
            .unwrap_or_else(|| format!("Town {idx}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn face_projection_keeps_single_cell_winners_and_breaks_same_kind_ties() {
        let grid = Grid::new(1337);
        let face = grid.topology.faces().next().unwrap();
        let [first, second, third] = grid.face_cells(face);
        let cells = RegionMemberships::from_pairs(
            grid.cell_count(),
            vec![
                (first.index(), 0),
                (first.index(), 2),
                (first.index(), 3),
                (second.index(), 1),
                (second.index(), 2),
                (second.index(), 3),
                (third.index(), 2),
                (third.index(), 3),
            ],
        );
        let regions = [
            RegionKind::Forest,
            RegionKind::Forest,
            RegionKind::MountainRange,
            RegionKind::Road,
        ]
        .map(|kind| RegionData {
            name: String::new(),
            pos: [0.0; 3],
            kind,
        });

        let projected = project_cell_memberships(&grid, &cells, &regions);

        assert_eq!(projected.region_ids_at(face.index()), &[0, 2, 3]);
    }
}
