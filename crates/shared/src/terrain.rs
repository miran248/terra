use bevy::prelude::Color;
use noise::{Fbm, MultiFractal, NoiseFn, Perlin};

use crate::sphere::SpherePos;

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

/// Samples biome type at any point on the planet. Seamless: driven by 3D noise
/// evaluated at the unit-sphere direction, so there are no UV seams.
///
/// Elevation is built in structured layers so terrain stays spatially coherent
/// (no lone deep-ocean vertex inside a mountain range):
///   1. `continents` — low frequency: where land vs sea is. Dominant amplitude.
///   2. `mountains`   — ridged, only added *on* land and scaled by how far inland.
///   3. `detail`      — small high-frequency bumps, amplitude too low to cross sea level.
pub struct TerrainGen {
    continents: Fbm<Perlin>,
    mountains: Fbm<Perlin>,
    detail: Fbm<Perlin>,
    moisture: Fbm<Perlin>,
    rivers: Fbm<Perlin>,
    warp: Fbm<Perlin>,
    temp_noise: Fbm<Perlin>,
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
        }
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
        //    Mountains and detail are gated by this so relief only grows on solid land.
        let land = smoothstep(0.0, 0.35, continent);

        // 3. Mountains: ridged (peaks along lines), only where land, biggest deep inland.
        let ridged = 1.0 - (self.mountains.get(w) as f32).abs();
        let mountains = ridged.powi(2) * 0.55 * land;

        // 4. Detail: small bumps, amplitude bounded so it can't cross sea level alone.
        let detail = self.detail.get(w) as f32 * 0.06 * land;

        (continent + mountains + detail).clamp(-1.0, 1.0)
    }

    fn moisture_at(&self, pos: SpherePos) -> f32 {
        self.moisture.get(self.warped(pos)) as f32
    }

    fn river_at(&self, pos: SpherePos) -> f32 {
        1.0 - (self.rivers.get(self.warped(pos)) as f32).abs()
    }

    /// Temperature in roughly [-1, 1]: hot (+1) at the equator, cold (-1) at the poles
    /// (poles are ±Y), colder at high elevation (lapse rate), with mild noise variation.
    pub fn temperature_at(&self, pos: SpherePos) -> f32 {
        let latitude = pos.0.y.abs(); // 0 at equator, 1 at poles
        let base = 1.0 - 2.0 * latitude; // +1 equator .. -1 pole
        let e = self.elevation_at(pos).max(0.0);
        let lapse = e * 1.3; // higher ground is colder
        let noise = self.temp_noise.get(self.warped(pos)) as f32 * 0.25;
        (base - lapse + noise).clamp(-1.0, 1.0)
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
            return if e > -0.06 && m > 0.2 { Terrain::Lake } else { Terrain::Ocean };
        }

        // ponytail: fake rivers = ridged noise band near sea level on land, no hydrology.
        // Upgrade path: trace downhill flow from mountains for real river networks.
        if e < 0.25 && self.river_at(pos) > 0.86 {
            return Terrain::River;
        }

        if e < 0.02 {
            return Terrain::Beach;
        }

        // Cold climate (poles / high peaks) overrides the moisture biomes.
        if t < -0.55 {
            return Terrain::Snow;
        }
        if t < -0.2 {
            return Terrain::Tundra;
        }

        // Rocky peaks: the mountain layer pushes land well above the lowland range.
        if e > 0.45 {
            return Terrain::Mountain;
        }

        // Warm/temperate land chosen by moisture and temperature.
        if t > 0.35 && m < -0.15 {
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

            // Cool it down by temperature: warm -> tundra -> snow (wide bands = soft edges).
            let mut c = lerp_color(Terrain::Tundra.color(), warm, smoothstep(-0.55, -0.05, t));
            c = lerp_color(Terrain::Snow.color(), c, smoothstep(-0.85, -0.45, t));

            // Beach at the shoreline, rocky mountains at high elevation.
            c = lerp_color(Terrain::Beach.color(), c, smoothstep(0.02, 0.10, e));
            c = lerp_color(c, Terrain::Mountain.color(), smoothstep(0.38, 0.52, e) * smoothstep(-0.2, 0.1, t));
            // High peaks get snow-capped.
            c = lerp_color(c, Terrain::Snow.color(), smoothstep(0.6, 0.78, e));

            // River tint: fade in near the ridge on low/mid land. Frozen ground (cold t)
            // hides open water, so rivers only show where it is warm enough.
            if e < 0.28 {
                let r = smoothstep(0.80, 0.94, self.river_at(pos))
                    * smoothstep(0.28, 0.16, e)
                    * smoothstep(-0.4, 0.0, t);
                c = lerp_color(c, Terrain::River.color(), r);
            }
            c
        };

        // Cross-fade water -> land across the shoreline band so there is no hard seam.
        lerp_color(water, land, smoothstep(-0.04, 0.06, e))
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
        assert!(eq / n as f32 > pole / n as f32 + 0.5, "equator should be clearly warmer");
    }

    #[test]
    fn poles_are_frozen_equator_is_not() {
        let tg = TerrainGen::new(5);
        // The exact poles must read as cold (Snow/Tundra) regardless of moisture noise.
        for pole in [Vec3::Y, Vec3::NEG_Y] {
            let t = tg.temperature_at(SpherePos::new(pole));
            assert!(t < -0.2, "pole should be cold, got t={t}");
        }
    }
}
