// ---- elevation synthesis (SolveElevation) ----
//
// The final elevation field is CONSTRAINT-SOLVED from the finished tile map:
// every tile kind declares the elevation range its ground may occupy, every
// kind pair declares how fast elevation may change across one vertex edge
// (~70m), rivers must descend monotonically, and road corridors stay gentle.
// A deterministic Gauss-Seidel loop drives the proposed field into the
// constraint set. There are no later touchups — the mesh projects this field.

/// Elevation range (in [-1,1] units; 1.0 land unit = 500m) a tile's ground
/// may occupy.
/// How far below its neighbors' average a water bed vertex is pushed each
/// solver step — a concave basin/channel instead of a flat plate. `None` for
/// non-bed kinds. (Rivers cut sharper than lake basins.)
pub(in crate::worldgen) fn water_concavity(t: Terrain) -> Option<f32> {
    match t {
        // Per-iteration push below the neighbour average — the deeper this, the
        // deeper the basin bowls (depth still grows with basin size). Lakes were
        // near-flat plates (0.005); deepen them so water pools with real depth.
        Terrain::Lake | Terrain::SaltLake => Some(0.050),
        Terrain::River | Terrain::RiverSpring => Some(0.090),
        _ => None,
    }
}

/// The water kinds a shore/bank vertex must sit strictly above (its adjacent
/// body). Empty for non-bank kinds.
pub(in crate::worldgen) fn bank_water(t: Terrain) -> &'static [Terrain] {
    match t {
        Terrain::RiverBank => &[Terrain::River, Terrain::RiverSpring],
        Terrain::LakeShore => &[Terrain::Lake, Terrain::SaltLake],
        Terrain::Beach => &[Terrain::Ocean],
        _ => &[],
    }
}

/// Minimum ground clearance above adjacent water beds. River water renders
/// above its channel, so river banks need enough freeboard to contain it.
pub(in crate::worldgen) fn bank_clearance(t: Terrain) -> f32 {
    match t {
        Terrain::RiverBank => 0.04,
        Terrain::LakeShore | Terrain::Beach => 0.01,
        _ => 0.0,
    }
}

/// A cover biome whose ELEVATION comes from its landform, not itself (a snowy
/// lowland stays low; snow doesn't imply a mountain). Water, shore bands and
/// rivers keep their own ranges — they aren't landforms.
pub(in crate::worldgen) fn is_cover(t: Terrain) -> bool {
    use Terrain::*;
    matches!(
        t,
        Desert
            | Plains
            | Forest
            | Tundra
            | Savanna
            | Swamp
            | Jungle
            | Mountain
            | Snow
            | Volcanic
            | Glacier
    )
}

/// Elevation range a LANDFORM's ground may occupy — the base layer that drives
/// height (cover only colors it). Bands overlap so adjacent landforms
/// (ordered lowland→hills→mountains) meet without an impossible jump.
pub(in crate::worldgen) fn landform_range(lf: Landform) -> (f32, f32) {
    match lf {
        Landform::Valley => (0.0, 0.16),
        Landform::Lowland => (0.02, 0.20),
        Landform::Hills => (0.14, 0.45),
        Landform::Mountains => (0.40, 1.0),
        Landform::Plateau => (0.36, 0.74),
        _ => (0.02, 0.50),
    }
}

/// How steep a land edge may be, from the steeper of the two landforms:
/// lowlands are gentle, mountains steep, hills between.
pub(in crate::worldgen) fn landform_edge_cap(lfa: Landform, lfb: Landform) -> f32 {
    let one = |lf: Landform| -> f32 {
        match lf {
            Landform::Valley | Landform::Lowland => 0.04,
            Landform::Hills => 0.14,
            Landform::Plateau => 0.20,
            Landform::Mountains => 0.40,
            _ => 0.10,
        }
    };
    one(lfa).max(one(lfb))
}

pub(in crate::worldgen) fn elev_range(t: Terrain) -> (f32, f32) {
    use Terrain::*;
    // Ranges of kinds that may sit next to each other must overlap (or lie
    // within one edge's gradient cap) or the constraint set is unsatisfiable.
    match t {
        // One ocean identity; the shelf constraint deepens the floor with
        // distance from land, so the range spans shore-shallows to abyss.
        Ocean => (-1.0, -0.01),
        // Lakes and rivers carry their OWN water level — a mountain lake may
        // sit high above the sea; only its shores must stay above it.
        Lake | SaltLake => (-0.25, 0.55),
        LakeShore => (0.0, 0.60),
        // Rivers descend from mountains to the sea; their range must span it.
        River => (-1.0, 0.60),
        RiverSpring => (0.0, 0.60),
        RiverBank => (0.0, 0.65),
        Beach => (0.0, 0.05),
        // Cliff tiles are RAMPS, not plateaus: the toe verts sit at shore
        // level and the crest verts track the hinterland (see the cliff
        // tracking step in the solver), so the whole drop happens across the
        // cliff face. The range here is just the envelope.
        Cliff => (-0.02, 0.60),
        Desert | Plains | Forest | Tundra | Savanna => (0.02, 0.50),
        // Swamp is low, wet, near-flat ground just above the water line.
        Swamp => (0.0, 0.15),
        // Jungle covers lowland to hills.
        Jungle => (0.02, 0.55),
        Mountain => (0.45, 1.0),
        Snow => (0.50, 1.0),
        // Volcanic peaks and glaciers ride the high ground like Mountain/Snow.
        Volcanic => (0.45, 1.0),
        Glacier => (0.45, 1.0),
    }
}

/// Max elevation change across one vertex edge (~70m) between two tile kinds.
/// Small at shores (continental shelf), large into mountains and at cliffs.
pub(in crate::worldgen) fn max_gradient(a: Terrain, b: Terrain) -> f32 {
    use Terrain::*;
    let water = |t: Terrain| matches!(t, Ocean | Lake | SaltLake);
    let peak = |t: Terrain| matches!(t, Mountain | Snow | Volcanic | Glacier);
    // Rivers are canyons: their walls may be steep wherever they cut through.
    // Caps are per vertex edge (~35m at field sub=6).
    let base: f32 = if a == LakeShore && b == LakeShore {
        // The shore ring is the waterline: keep it nearly level so the flat lake
        // surface meets it evenly all the way round (an uneven rim floats the
        // surface at the low end). The land rising above a hillside lake is other
        // terrain, not the shore, so this doesn't flatten the surroundings.
        0.005
    } else if matches!(a, River | RiverBank) || matches!(b, River | RiverBank) {
        0.22
    } else if a == Cliff || b == Cliff {
        // The whole cliff drop can happen across one vertex edge (toe → crest).
        0.30
    } else if peak(a) || peak(b) {
        0.14
    } else if water(a) && water(b) {
        0.04
    } else if water(a) || water(b) || a == Beach || b == Beach {
        0.015
    } else {
        // Ordinary land: ~0.02 e per ~35m edge ≈ 15° — walkable country,
        // not ski slopes. Steepness is a property of mountains, cliffs and
        // canyons (their caps above), not of plains.
        0.02
    };
    // Feasibility: a cap can never be tighter than the jump the two kinds'
    // disjoint elevation ranges force — otherwise range clamp and gradient cap
    // fight forever (e.g. a mountain vertex right beside a lakeshore vertex).
    let (alo, ahi) = elev_range(a);
    let (blo, bhi) = elev_range(b);
    let forced_gap = (blo - ahi).max(alo - bhi).max(0.0);
    base.max(forced_gap + 0.01)
}

/// Tight cap along road corridors so roads stay walkable.
pub(in crate::worldgen) const ROAD_EDGE_GRADIENT: f32 = 0.01;
pub(in crate::worldgen) const SOLVER_MAX_ITERS: usize = 250;
pub(in crate::worldgen) const SOLVER_EPS: f32 = 0.002;
use crate::level::Landform;
use crate::terrain::Terrain;
