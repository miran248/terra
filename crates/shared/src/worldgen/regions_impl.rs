// ---- named regions: contiguous feature clusters (edge-connected) ----

/// Which nameable feature a face belongs to. Tags win over terrain so towns and
/// roads cluster as themselves; LakeShore/RiverBank separate regions and stay
/// unnamed.
fn region_class(
    grid: &Grid,
    face_types: &[Terrain],
    painted: Option<&Painted>,
    fi: usize,
) -> Option<RegionKind> {
    if let Some(p) = painted {
        if face_solid(grid, &p.towns, fi) {
            return Some(RegionKind::Town);
        }
        if face_solid(grid, &p.roads, fi) {
            return Some(RegionKind::Road);
        }
    }
    match face_types[fi] {
        Terrain::Ocean => Some(RegionKind::Ocean),
        Terrain::Lake => Some(RegionKind::Lake),
        Terrain::River | Terrain::RiverSpring => Some(RegionKind::River),
        Terrain::Beach => Some(RegionKind::Beach),
        Terrain::Cliff => Some(RegionKind::Cliff),
        Terrain::Forest => Some(RegionKind::Forest),
        Terrain::Desert => Some(RegionKind::Desert),
        Terrain::Mountain | Terrain::Snow => Some(RegionKind::Mountain),
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

/// Flood-fill same-class faces into clusters via edge adjacency (tiles sharing
/// only a single vertex are NOT linked), name each cluster, and record the
/// per-face region id for HUD lookup.
fn build_regions(
    grid: &Grid,
    terrain: &TerrainGen,
    face_types: &[Terrain],
    painted: &Painted,
) -> (Vec<RegionData>, Vec<u32>) {
    let class: Vec<Option<RegionKind>> = (0..grid.face_count())
        .map(|fi| region_class(grid, face_types, Some(painted), fi))
        .collect();
    // Terrain-derived class ignoring the road/town overlay: a road slicing
    // through a desert must not split it into two regions, so terrain clusters
    // may flow THROUGH overlay faces whose underlying terrain matches (without
    // claiming them — those faces belong to their Road/Town region).
    let terrain_class: Vec<Option<RegionKind>> = (0..grid.face_count())
        .map(|fi| region_class(grid, face_types, None, fi))
        .collect();

    let mut face_region = vec![NO_REGION; grid.face_count()];
    let mut regions: Vec<RegionData> = Vec::new();
    let mut kind_counts: BTreeMap<u8, usize> = BTreeMap::new();

    let partitioner = regions::RegionPartitioner::new(grid, &class, &terrain_class);
    for start in grid.topology.faces() {
        let Some(kind) = class[start.index()] else {
            continue;
        };
        if face_region[start.index()] != NO_REGION {
            continue;
        }
        // Collect the edge-connected cluster. Stored refs are region id + 1
        // (0 = no region, see level::region_index).
        let re = regions.len() as u32 + 1;
        // Long coastlines split into multiple named regions while naming —
        // the tiles themselves are never retyped for naming's sake.
        let max_faces = match kind {
            RegionKind::Beach => size_range(Terrain::Beach).1 * 2,
            RegionKind::Cliff => size_range(Terrain::Cliff).1 * 2,
            _ => usize::MAX,
        };
        let faces = partitioner.claim(start, kind, re, max_faces, &mut face_region);
        // Tiny scraps stay unnamed (towns and roads always name).
        let min_faces = match kind {
            RegionKind::Town | RegionKind::Road | RegionKind::River => 1,
            // Cell minimums expressed in faces (one cell ≈ two faces of area).
            RegionKind::Forest => size_range(Terrain::Forest).0 * 2,
            RegionKind::Beach => size_range(Terrain::Beach).0 * 2,
            _ => 8,
        };
        if faces.len() < min_faces {
            for face in faces {
                face_region[face.index()] = NO_REGION;
            }
            continue;
        }
        let cent = faces
            .iter()
            .map(|face| grid.centroid(face.index()).0)
            .sum::<Vec3>()
            .normalize_or(Vec3::Y);
        let idx = *kind_counts
            .entry(kind as u8)
            .and_modify(|c| *c += 1)
            .or_insert(0);
        let name = region_name(kind, idx, cent, terrain);
        regions.push(RegionData {
            name,
            pos: cent.to_array(),
            kind,
        });
    }
    (regions, face_region)
}

fn region_name(kind: RegionKind, idx: usize, cent: Vec3, terrain: &TerrainGen) -> String {
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
        RegionKind::River => pick(&RIVER, &["River"]),
        RegionKind::Beach => pick(&BEACH, &["Beach", "Coast", "Sands"]),
        RegionKind::Cliff => pick(&CLIFF, &["Cliffs", "Bluffs"]),
        RegionKind::Forest => pick(&FOREST, &["Forest", "Woods"]),
        RegionKind::Desert => pick(&DESERT, &["Desert", "Dunes"]),
        RegionKind::Mountain => pick(&MOUNTAIN, &["Peaks", "Range"]),
        RegionKind::Plains => pick(&PLAINS, &["Plains", "Fields"]),
        RegionKind::Tundra => pick(&TUNDRA, &["Tundra", "Wastes"]),
        RegionKind::Swamp => pick(&SWAMP, &["Swamp", "Marsh", "Fen"]),
        RegionKind::Jungle => pick(&JUNGLE, &["Jungle", "Rainforest"]),
        RegionKind::Savanna => pick(&SAVANNA, &["Savanna", "Plains"]),
        RegionKind::Volcano => pick(&VOLCANO, &["Peaks", "Fields", "Wastes"]),
        RegionKind::Glacier => pick(&GLACIER, &["Glacier", "Ice", "Wastes"]),
        RegionKind::Road => pick(&ROAD, &["Road"]),
        // Towns take the name of the settlement they surround.
        RegionKind::Town => terrain
            .settlement_anchors
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| a.0.dot(cent).partial_cmp(&b.0.dot(cent)).unwrap().reverse())
            .map(|(i, _)| crate::roads::settlement_name(i))
            .unwrap_or_else(|| format!("Town {idx}")),
    }
}

