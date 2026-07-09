use bevy::prelude::{Color, Resource, Vec3};
use noise::{Fbm, MultiFractal, NoiseFn, Perlin};

use crate::sphere::{SpherePos, METER, PLANET_RADIUS};

/// Peak mountain height above sea level, in meters.
pub const MAX_MOUNTAIN: f32 = 500.0 * METER;
/// Deepest ocean floor below sea level, in meters.
pub const MAX_DEPTH: f32 = 500.0 * METER;

/// Habitable altitude band (meters above sea level): livable lowlands where
/// villages, towns, farms, roads and paths can later be placed.
pub const HABITABLE_MIN_ALT: f32 = 1.0 * METER;
pub const HABITABLE_MAX_ALT: f32 = 150.0 * METER;
/// Maximum ground steepness (altitude change per meter) still considered buildable.
pub const HABITABLE_MAX_SLOPE: f32 = 0.6;
/// Habitable temperature band, in degrees Celsius.
pub const HABITABLE_MIN_TEMP: f32 = -10.0;
pub const HABITABLE_MAX_TEMP: f32 = 30.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Terrain {
    DeepOcean,
    Ocean,
    Lake,
    River,
    Beach,
    Desert,
    Plains,
    Forest,
    Tundra,
    Mountain,
    Snow,
}

impl Terrain {
    pub fn color(&self) -> Color {
        match self {
            Terrain::DeepOcean => Color::srgb(0.05, 0.12, 0.35),
            Terrain::Ocean => Color::srgb(0.10, 0.25, 0.55),
            Terrain::Lake => Color::srgb(0.15, 0.35, 0.65),
            Terrain::River => Color::srgb(0.20, 0.45, 0.75),
            Terrain::Beach => Color::srgb(0.85, 0.78, 0.55),
            Terrain::Desert => Color::srgb(0.80, 0.70, 0.40),
            Terrain::Plains => Color::srgb(0.35, 0.55, 0.25),
            Terrain::Forest => Color::srgb(0.15, 0.38, 0.18),
            Terrain::Tundra => Color::srgb(0.55, 0.58, 0.52),
            Terrain::Mountain => Color::srgb(0.45, 0.42, 0.40),
            Terrain::Snow => Color::srgb(0.92, 0.94, 0.97),
        }
    }
}

/// Radius of the flattened corridor around roads/towns, meters. Mountains are fully
/// suppressed on the road and ease back to full strength past this distance.
pub const ROAD_CORRIDOR: f32 = 180.0;

/// Samples biome type at any point on the planet. Seamless: driven by 3D noise
/// evaluated at the unit-sphere direction, so there are no UV seams.
///
/// Generation is **roads-first**: the continent shape (land/sea) is decided by noise,
/// settlements + roads are placed on that flat land (see `roads.rs`), then mountains and
/// forests are grown *around* the roads — suppressed within `ROAD_CORRIDOR` of a road so
/// routes sit in natural valleys/clearings instead of fighting the terrain.
#[derive(Resource)]
pub struct TerrainGen {
    continents: Fbm<Perlin>,
    mountains: Fbm<Perlin>,
    detail: Fbm<Perlin>,
    moisture: Fbm<Perlin>,
    rivers: Fbm<Perlin>,
    warp: Fbm<Perlin>,
    temp_noise: Fbm<Perlin>,
    /// Spatial hash of road/town points; set after roads are generated. `None` before that
    /// (during road generation itself, which only reads the flat continent layer).
    roads: Option<RoadField>,
}

/// Spatial grid of road + settlement sample points, for a fast "distance to nearest road"
/// query used to carve mountain-free corridors around roads.
struct RoadField {
    cells: std::collections::HashMap<(i32, i32), Vec<Vec3>>,
}

impl RoadField {
    const CELL: f32 = ROAD_CORRIDOR; // one corridor-width per cell

    fn key(dir: Vec3) -> (i32, i32) {
        let c = Self::CELL / PLANET_RADIUS;
        let lat = dir.y.clamp(-1.0, 1.0).acos();
        let lon = dir.z.atan2(dir.x) + std::f32::consts::PI;
        ((lat / c).floor() as i32, (lon / c).floor() as i32)
    }

    fn new(points: impl Iterator<Item = Vec3>) -> Self {
        let mut cells: std::collections::HashMap<(i32, i32), Vec<Vec3>> = Default::default();
        for p in points {
            cells.entry(Self::key(p)).or_default().push(p);
        }
        Self { cells }
    }

    /// Arc-distance (meters) to the nearest road point, searching the 3×3 cell block.
    fn nearest(&self, dir: Vec3) -> f32 {
        let (li, oi) = Self::key(dir);
        let mut best = f32::MAX;
        for dl in -1..=1 {
            for doo in -1..=1 {
                if let Some(pts) = self.cells.get(&(li + dl, oi + doo)) {
                    for q in pts {
                        let d = dir.dot(*q).clamp(-1.0, 1.0).acos() * PLANET_RADIUS;
                        best = best.min(d);
                    }
                }
            }
        }
        best
    }
}

impl TerrainGen {
    pub fn new(seed: u32) -> Self {
        Self {
            continents: Fbm::<Perlin>::new(seed).set_octaves(4).set_frequency(1.1),
            mountains: Fbm::<Perlin>::new(seed.wrapping_add(5)).set_octaves(5).set_frequency(3.2),
            detail: Fbm::<Perlin>::new(seed.wrapping_add(6)).set_octaves(3).set_frequency(7.0),
            moisture: Fbm::<Perlin>::new(seed.wrapping_add(1)).set_octaves(4).set_frequency(2.2),
            rivers: Fbm::<Perlin>::new(seed.wrapping_add(2)).set_octaves(3).set_frequency(3.0),
            warp: Fbm::<Perlin>::new(seed.wrapping_add(3)).set_octaves(3).set_frequency(2.8),
            temp_noise: Fbm::<Perlin>::new(seed.wrapping_add(4)).set_octaves(3).set_frequency(1.8),
            roads: None,
        }
    }

    /// Attach the road network so terrain generation can carve corridors around paths
    /// (mountains are suppressed near roads). Must be called after `Roads::generate`.
    pub fn set_roads(&mut self, roads: &crate::roads::Roads) {
        let mut pts: Vec<Vec3> = Vec::new();
        for r in &roads.roads {
            pts.extend(r.points.iter().map(|p| p.0));
        }
        for s in &roads.settlements {
            pts.push(s.pos.0);
        }
        self.roads = Some(RoadField::new(pts.into_iter()));
    }

    /// Fraction (0..1) of how close `pos` is to a road: 0 = right on a road, 1 = far away.
    /// Roads-first: prior to 25 m the land is fully flat (zero mountains); from there it
    /// eases back to full terrain strength across `ROAD_CORRIDOR` meters so mountains grow
    /// outside the road corridor without spikes near the path.
    pub fn road_proximity(&self, pos: SpherePos) -> f32 {
        self.roads
            .as_ref()
            .map(|rf| {
                let d = rf.nearest(pos.0);
                if d < 25.0 {
                    0.0 // fully flat directly on a road
                } else {
                    ((d - 25.0) / (ROAD_CORRIDOR - 25.0)).clamp(0.0, 1.0)
                }
            })
            .unwrap_or(0.0)
    }

    /// The flat continent layer only (land vs sea, no mountains). Used during road
    /// generation so roads don't have to fight terrain they'll later carve around.
    pub fn continent_elevation(&self, pos: SpherePos) -> f32 {
        self.continents.get(self.warped(pos)) as f32
    }

    /// Domain-warped sample point: shifts the lookup so biome edges are wavy and organic
    /// instead of following the raw noise contours. Result stays on the unit sphere.
    fn warped(&self, pos: SpherePos) -> [f64; 3] {
        let d = pos.0.as_dvec3();
        let base = [d.x, d.y, d.z];
        let wx = self.warp.get([d.x + 5.2, d.y + 1.3, d.z]);
        let wy = self.warp.get([d.x, d.y + 9.7, d.z + 2.1]);
        let wz = self.warp.get([d.x + 3.4, d.y, d.z + 6.8]);
        const AMT: f64 = 0.18;
        [base[0] + wx * AMT, base[1] + wy * AMT, base[2] + wz * AMT]
    }

    /// Structured elevation in roughly [-1, 1] (negative = below sea level).
    /// Layered so the continent shape dominates and finer layers can only add relief
    /// *within* land — they can never punch an ocean hole through a mountain.
    pub fn elevation_at(&self, pos: SpherePos) -> f32 {
        let w = self.warped(pos);

        // 1. Continents: the dominant term. Positive = land, negative = sea.
        let continent = self.continents.get(w) as f32;

        // 2. How far "inland" we are: 0 at/below the coast, ramping up over land.
        let land = smoothstep(0.0, 0.35, continent);

        // 3. Road flatness: near roads the land is flat (mountains suppressed); far from
        //    roads, mountains grow at full strength. 0 = completely flat, 1 = full relief.
        let road_flatness = self.road_proximity(pos).powf(2.5);

        let ridged = 1.0 - (self.mountains.get(w) as f32).abs();
        let mountains = ridged.powi(2) * 0.55 * land;

        let detail = self.detail.get(w) as f32 * 0.06 * land;

        (continent + mountains + detail).clamp(-1.0, 1.0)
    }

    /// World-space radius of the terrain surface at `pos`. Sea level is `PLANET_RADIUS`.
    /// Land rises up to `MAX_MOUNTAIN`; the ocean floor sinks down to `MAX_DEPTH`.
    pub fn surface_radius(&self, pos: SpherePos) -> f32 {
        let e = self.elevation_at(pos);
        if e > 0.0 {
            // Ease the low end so coastlines rise gently, peaks get the full height.
            PLANET_RADIUS + e.powf(1.3) * MAX_MOUNTAIN
        } else {
            // Ocean floor deepens with negative elevation (basins/trenches).
            PLANET_RADIUS - (-e).powf(1.3) * MAX_DEPTH
        }
    }

    /// Radius the terrain *mesh* should render at: land follows `surface_radius`, but water
    /// is a flat surface at sea level. The ocean floor's true depth stays data-only (color,
    /// travel cost) rather than a sloped dent in the visible sea.
    pub fn render_radius(&self, pos: SpherePos) -> f32 {
        self.surface_radius(pos)
    }

    /// World-space point on the displaced terrain surface at `pos`.
    pub fn surface_world(&self, pos: SpherePos) -> bevy::prelude::Vec3 {
        pos.0 * self.surface_radius(pos)
    }

    /// World-space point for an actor of the given `half_height` resting *on* the ground:
    /// its mesh center is lifted so the bottom touches the surface, never sinking below it.
    /// Over ocean the ground is clamped to sea level, so actors rest on the water surface
    /// (they may be under water, but never below the seabed).
    pub fn ground_world(&self, pos: SpherePos, half_height: f32) -> bevy::prelude::Vec3 {
        let ground = self.surface_radius(pos).max(PLANET_RADIUS);
        pos.0 * (ground + half_height)
    }

    /// Height of the surface above sea level, in meters (negative under the ocean).
    pub fn altitude(&self, pos: SpherePos) -> f32 {
        self.surface_radius(pos) - PLANET_RADIUS
    }

    /// Local terrain steepness in meters of altitude change per meter travelled,
    /// sampled over a small tangent neighbourhood. 0 = flat, higher = steeper.
    pub fn slope(&self, pos: SpherePos) -> f32 {
        let (east, north) = pos.tangent_basis();
        const STEP: f32 = 4.0 * METER;
        let h = self.altitude(pos);
        let mut worst = 0.0f32;
        for dir in [east, -east, north, -north] {
            let n = SpherePos::new(pos.0 + dir * (STEP / PLANET_RADIUS));
            worst = worst.max((self.altitude(n) - h).abs() / STEP);
        }
        worst
    }

    /// Whether a location is livable — the seed for future villages/roads/farms.
    /// Gentle low-altitude land in a temperate climate, away from cliffs and water.
    pub fn is_habitable(&self, pos: SpherePos) -> bool {
        let alt = self.altitude(pos);
        let temp = self.temperature_at(pos);
        alt >= HABITABLE_MIN_ALT
            && alt <= HABITABLE_MAX_ALT
            && temp >= HABITABLE_MIN_TEMP
            && temp <= HABITABLE_MAX_TEMP
            && self.slope(pos) < HABITABLE_MAX_SLOPE
    }

    /// A deterministic habitable spawn point for this planet's seed. Scans a fixed
    /// golden-spiral set of surface points and returns the *best* habitable one —
    /// mild climate, comfortably inland (dry land, not river/coast), gently sloped —
    /// so a given seed always spawns the player in the same pleasant place.
    pub fn habitable_spawn(&self) -> SpherePos {
        const N: u32 = 4096;
        const GOLDEN: f32 = 2.399_963_2; // 2*pi*(1 - 1/phi)
        let mut best = SpherePos::new(bevy::prelude::Vec3::X);
        let mut best_score = f32::MIN;
        for i in 0..N {
            let t = (i as f32 + 0.5) / N as f32;
            let y = 1.0 - 2.0 * t; // -1..1
            let r = (1.0 - y * y).max(0.0).sqrt();
            let a = GOLDEN * i as f32;
            let p = SpherePos::new(bevy::prelude::Vec3::new(r * a.cos(), y, r * a.sin()));
            if !self.is_habitable(p) {
                continue;
            }
            let score = self.spawn_score(p);
            if score > best_score {
                best_score = score;
                best = p;
            }
        }
        best
    }

    /// Deterministic, well-separated habitable settlement anchors (villages/towns).
    /// Greedily picks the best-scoring habitable points at least `min_sep` meters apart,
    /// so settlements spread across the map instead of clustering.
    pub fn habitable_anchors(&self, count: usize, min_sep: f32) -> Vec<SpherePos> {
        const N: u32 = 8192;
        const GOLDEN: f32 = 2.399_963_2;
        let mut cands: Vec<(f32, SpherePos)> = Vec::new();
        for i in 0..N {
            let t = (i as f32 + 0.5) / N as f32;
            let y = 1.0 - 2.0 * t;
            let r = (1.0 - y * y).max(0.0).sqrt();
            let a = GOLDEN * i as f32;
            let p = SpherePos::new(bevy::prelude::Vec3::new(r * a.cos(), y, r * a.sin()));
            if self.is_habitable(p) {
                cands.push((self.spawn_score(p), p));
            }
        }
        cands.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());

        let mut anchors: Vec<SpherePos> = Vec::new();
        for (_, p) in cands {
            if anchors.len() >= count {
                break;
            }
            if anchors.iter().all(|a| a.distance(p) >= min_sep) {
                anchors.push(p);
            }
        }
        anchors
    }

    /// Higher = a more pleasant place to start: mild ~18 °C climate, dry solid land
    /// (penalise rivers/coast), gentle ground, comfortably above the waterline.
    fn spawn_score(&self, pos: SpherePos) -> f32 {
        let temp = self.temperature_at(pos);
        let alt = self.altitude(pos);
        let climate = 1.0 - (temp - 18.0).abs() / 20.0; // peak at a mild 18 °C
        let altitude = smoothstep(1.0 * METER, 60.0 * METER, alt); // a bit inland/uphill
        let flatness = 1.0 - self.slope(pos);
        let dry = if matches!(self.classify(pos), Terrain::River | Terrain::Beach) { -1.0 } else { 0.0 };
        climate + altitude + flatness + dry
    }

    fn moisture_at(&self, pos: SpherePos) -> f32 {
        self.moisture.get(self.warped(pos)) as f32
    }

    fn river_at(&self, pos: SpherePos) -> f32 {
        1.0 - (self.rivers.get(self.warped(pos)) as f32).abs()
    }

    /// Surface temperature in degrees Celsius (0 °C freezes, 100 °C boils).
    /// Warm at the equator (~+40 °C at sea level), cold at the poles (~-40 °C),
    /// cooled by altitude (lapse rate), with mild regional noise.
    pub fn temperature_at(&self, pos: SpherePos) -> f32 {
        let latitude = pos.0.y.abs(); // 0 at equator, 1 at poles
        let base = 40.0 - 80.0 * latitude; // +40 °C equator .. -40 °C pole
        let altitude = self.altitude(pos).max(0.0);
        let lapse = altitude * 0.04; // ~ -20 °C on a 500 m peak (game-exaggerated)
        let noise = self.temp_noise.get(self.warped(pos)) as f32 * 10.0; // ±10 °C
        base - lapse + noise
    }

    /// Discrete biome, used for gameplay/logic. Rendering should prefer [`color_at`]
    /// for smooth transitions.
    pub fn classify(&self, pos: SpherePos) -> Terrain {
        let e = self.elevation_at(pos);
        let m = self.moisture_at(pos);
        let t = self.temperature_at(pos);

        const SEA: f32 = 0.0;

        if e < SEA {
            if e < -0.30 {
                return Terrain::DeepOcean;
            }
            return if e > -0.04 && m > 0.2 { Terrain::Lake } else { Terrain::Ocean };
        }

        // ponytail: fake rivers = ridged noise band near sea level on land, no hydrology.
        // Upgrade path: trace downhill flow from mountains for real river networks.
        if e < 0.08 && self.river_at(pos) > 0.86 {
            return Terrain::River;
        }

        if e < 0.02 {
            return Terrain::Beach;
        }

        // Cold climate (poles / high peaks) overrides the moisture biomes.
        if t < -15.0 {
            return Terrain::Snow;
        }
        if t < 0.0 {
            return Terrain::Tundra;
        }

        // Rocky peaks: the mountain layer pushes land well above the lowland range.
        if e > 0.45 {
            return Terrain::Mountain;
        }

        // Warm/temperate land chosen by moisture and temperature.
        if t > 30.0 && m < -0.15 {
            Terrain::Desert
        } else if m > 0.25 {
            Terrain::Forest
        } else {
            Terrain::Plains
        }
    }

    /// Blended terrain color for rendering, from the same elevation/moisture/temperature
    /// fields as [`classify`]. Cross-fades biomes across thresholds for pleasant gradients;
    /// sharp features (rivers, coastlines) stay crisp.
    pub fn color_at(&self, pos: SpherePos) -> Color {
        let e = self.elevation_at(pos);
        let m = self.moisture_at(pos);
        let t = self.temperature_at(pos);
        // World-space radius of this point; used to suppress water colors on inland lakes.
        let world_r = self.surface_radius(pos);

        // Water gradient (deep -> ocean -> shallow shore), valid across the whole range;
        // only shows where elevation is at/below sea level.
        let water = {
            let deep_ocean = lerp_color(
                Terrain::DeepOcean.color(),
                Terrain::Ocean.color(),
                smoothstep(-0.5, -0.15, e),
            );
            lerp_color(deep_ocean, Terrain::Lake.color(), smoothstep(-0.15, 0.0, e))
        };

        // Land gradient.
        let land = {
            // Warm-zone base color from moisture: desert -> plains -> forest.
            let dryland = lerp_color(
                Terrain::Desert.color(),
                Terrain::Plains.color(),
                smoothstep(-0.35, -0.05, m),
            );
            let warm = lerp_color(dryland, Terrain::Forest.color(), smoothstep(0.05, 0.35, m));

            // Cool it down by temperature (°C): warm -> tundra -> snow (wide, soft bands).
            let mut c = lerp_color(Terrain::Tundra.color(), warm, smoothstep(-15.0, 0.0, t));
            c = lerp_color(Terrain::Snow.color(), c, smoothstep(-25.0, -12.0, t));

            // Beach at the shoreline, rocky mountains at high elevation (only above freezing).
            c = lerp_color(Terrain::Beach.color(), c, smoothstep(0.02, 0.10, e));
            c = lerp_color(c, Terrain::Mountain.color(), smoothstep(0.38, 0.52, e) * smoothstep(-8.0, 4.0, t));
            // High peaks get snow-capped.
            c = lerp_color(c, Terrain::Snow.color(), smoothstep(0.6, 0.78, e));

            // River tint: fade in near the ridge on low/mid land. Frozen ground hides open
            // water, so rivers only show where it is above freezing.
            if e < 0.28 {
                let r = smoothstep(0.80, 0.94, self.river_at(pos))
                    * smoothstep(0.28, 0.16, e)
                    * smoothstep(-12.0, 0.0, t);
                c = lerp_color(c, Terrain::River.color(), r);
            }
            c
        };

        // Cross-fade water -> land across the shoreline band so there is no hard seam.
        // Suppress water colors on inland depressions (lakes above sea level) by using the
        // world-space height: only render water where the surface is at or below sea level.
        let shore_t = if world_r <= PLANET_RADIUS {
            smoothstep(-0.04, 0.06, e)
        } else {
            1.0 // fully land
        };
        lerp_color(water, land, shore_t)
    }
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn lerp_color(a: Color, b: Color, t: f32) -> Color {
    let a = a.to_linear();
    let b = b.to_linear();
    Color::linear_rgb(
        a.red + (b.red - a.red) * t,
        a.green + (b.green - a.green) * t,
        a.blue + (b.blue - a.blue) * t,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sphere::random_point;
    use bevy::prelude::Vec3;

    #[test]
    fn classifies_and_has_variety() {
        let tg = TerrainGen::new(42);
        let mut seen = std::collections::HashSet::new();
        for i in 0..2000 {
            let u = (i as f32 * 0.6180339) % 1.0;
            let v = (i as f32 * 0.7548776) % 1.0;
            let t = tg.classify(random_point(u, v));
            seen.insert(t);
        }
        // A whole planet should contain both water and land biomes.
        assert!(seen.contains(&Terrain::Ocean) || seen.contains(&Terrain::DeepOcean));
        assert!(seen.len() >= 4, "expected biome variety, got {:?}", seen);
    }

    #[test]
    fn elevation_is_spatially_coherent() {
        // The artifact we fixed: a deep-ocean vertex must never sit inside high land.
        // Check every deep-ocean sample's neighbours are also low (not mountains).
        let tg = TerrainGen::new(99);
        for i in 0..3000 {
            let u = (i as f32 * 0.6180339) % 1.0;
            let v = (i as f32 * 0.7548776) % 1.0;
            let p = random_point(u, v);
            let e = tg.elevation_at(p);
            if e >= -0.30 {
                continue; // only inspect deep ocean
            }
            let (east, north) = p.tangent_basis();
            for dir in [east, -east, north, -north] {
                // ~0.05 rad away — a few mesh vertices over.
                let n = SpherePos::new(p.0 + dir * 0.05);
                let en = tg.elevation_at(n);
                assert!(
                    en < 0.25,
                    "deep ocean (e={e:.2}) next to high land (e={en:.2}) — incoherent terrain"
                );
            }
        }
    }

    #[test]
    fn deterministic() {
        let g2 = TerrainGen::new(7);
        let p = random_point(0.3, 0.6);
        assert_eq!(g2.classify(p), g2.classify(p));
    }

    #[test]
    fn equator_warmer_than_poles() {
        let tg = TerrainGen::new(5);
        // Average temperature over an equatorial band vs. near a pole.
        let mut eq = 0.0;
        let mut pole = 0.0;
        let n = 64;
        for i in 0..n {
            let a = std::f32::consts::TAU * i as f32 / n as f32;
            eq += tg.temperature_at(SpherePos::new(Vec3::new(a.cos(), 0.0, a.sin())));
            pole += tg.temperature_at(SpherePos::new(Vec3::new(a.cos() * 0.1, 0.99, a.sin() * 0.1)));
        }
        assert!(eq / n as f32 > pole / n as f32 + 30.0, "equator should be much warmer (°C)");
    }

    #[test]
    fn poles_are_frozen_equator_is_not() {
        let tg = TerrainGen::new(5);
        // The exact poles must read as freezing (below 0 °C) regardless of moisture noise.
        for pole in [Vec3::Y, Vec3::NEG_Y] {
            let t = tg.temperature_at(SpherePos::new(pole));
            assert!(t < 0.0, "pole should be below freezing, got t={t} °C");
        }
    }

    #[test]
    fn habitable_spawn_is_actually_habitable() {
        for seed in [1u32, 7, 42, 1337, 99] {
            let tg = TerrainGen::new(seed);
            let s = tg.habitable_spawn();
            assert_eq!(s.0, tg.habitable_spawn().0, "spawn must be deterministic per seed");
            assert!(tg.is_habitable(s), "spawn not habitable (seed {seed})");
            let t = tg.temperature_at(s);
            assert!(
                (HABITABLE_MIN_TEMP..=HABITABLE_MAX_TEMP).contains(&t),
                "spawn temperature {t} °C outside habitable band (seed {seed})"
            );
        }
    }

    #[test]
    fn temperature_is_celsius_ranged() {
        let tg = TerrainGen::new(3);
        // Sea-level equator should be hot (tens of °C), not the old [-1,1] scale.
        let eq = tg.temperature_at(SpherePos::new(Vec3::new(1.0, 0.0, 0.0)));
        assert!(eq > 15.0, "equatorial sea level should be warm, got {eq} °C");
    }

    #[test]
    fn habitable_stays_in_band() {
        let tg = TerrainGen::new(42);
        for i in 0..3000 {
            let u = (i as f32 * 0.6180339) % 1.0;
            let v = (i as f32 * 0.7548776) % 1.0;
            let p = random_point(u, v);
            if tg.is_habitable(p) {
                let alt = tg.altitude(p);
                assert!(alt >= HABITABLE_MIN_ALT && alt <= HABITABLE_MAX_ALT,
                    "habitable point outside altitude band: {alt}m");
            }
        }
    }

    #[test]
    fn ground_world_never_below_surface() {
        // Actors must rest on/above the terrain: on land above the ground, over ocean
        // on the water surface (sea level) — never below the seabed.
        let tg = TerrainGen::new(11);
        let half = 1.0f32;
        for i in 0..2000 {
            let u = (i as f32 * 0.6180339) % 1.0;
            let v = (i as f32 * 0.7548776) % 1.0;
            let p = random_point(u, v);
            let r = tg.ground_world(p, half).length();
            let ground = tg.surface_radius(p).max(PLANET_RADIUS);
            assert!(r >= ground - 1e-2, "actor sank below ground");
            assert!((r - (ground + half)).abs() < 1e-2, "actor not resting exactly on ground");
        }
    }

    #[test]
    fn heightmap_lifts_land_and_sinks_ocean() {
        let tg = TerrainGen::new(11);
        for i in 0..2000 {
            let u = (i as f32 * 0.6180339) % 1.0;
            let v = (i as f32 * 0.7548776) % 1.0;
            let p = random_point(u, v);
            let e = tg.elevation_at(p);
            let r = tg.surface_radius(p);
            if e > 0.05 {
                assert!(r > PLANET_RADIUS, "land should rise above sea level");
            } else if e < -0.05 {
                assert!(r < PLANET_RADIUS, "ocean floor should sink below sea level");
            }
            // Relief stays within the configured budget (no runaway spikes/trenches).
            assert!(r <= PLANET_RADIUS + MAX_MOUNTAIN + 1e-3);
            assert!(r >= PLANET_RADIUS - MAX_DEPTH - 1e-3);
        }
    }
}
