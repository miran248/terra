use bevy::prelude::{Color, Resource, Vec3};
use noise::{Fbm, MultiFractal, NoiseFn, Perlin};
use std::collections::HashMap;

use crate::sphere::{SpherePos, PLANET_RADIUS};

pub const MAX_MOUNTAIN: f32 = 500.0;
pub const MAX_DEPTH: f32 = 500.0;

pub const HABITABLE_MIN_ALT: f32 = 1.0;
pub const HABITABLE_MAX_ALT: f32 = 150.0;
pub const HABITABLE_MAX_SLOPE: f32 = 0.6;
pub const HABITABLE_MIN_TEMP: f32 = -10.0;
pub const HABITABLE_MAX_TEMP: f32 = 30.0;

// ---- biome types ----

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Terrain {
    DeepOcean,
    Ocean,
    Lake,
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
}

impl Terrain {
    pub const ALL: [Terrain; 14] = [
        Terrain::DeepOcean, Terrain::Ocean, Terrain::Lake, Terrain::LakeShore,
        Terrain::River, Terrain::RiverBank, Terrain::Beach, Terrain::Cliff,
        Terrain::Desert, Terrain::Plains, Terrain::Forest, Terrain::Tundra,
        Terrain::Mountain, Terrain::Snow,
    ];

    pub fn color(&self) -> Color {
        match self {
            Terrain::DeepOcean => Color::srgb(0.05, 0.12, 0.35),
            Terrain::Ocean => Color::srgb(0.10, 0.25, 0.55),
            Terrain::Lake => Color::srgb(0.15, 0.35, 0.65),
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
        }
    }

    pub fn is_water(&self) -> bool {
        matches!(self, Terrain::DeepOcean | Terrain::Ocean | Terrain::Lake | Terrain::River)
    }

    pub fn is_land(&self) -> bool { !self.is_water() }
}

// ---- transition matrix ----

pub fn transition(self_type: Terrain, neighbor: Terrain) -> Terrain {
    use Terrain::*;
    match (self_type, neighbor) {
        (Ocean, Plains | Forest | Desert | Tundra) => Beach,
        (Ocean, Mountain | Snow) => Cliff,
        (DeepOcean, _) => DeepOcean,
        (Plains | Forest | Desert | Tundra | Mountain | Snow, Ocean) => Cliff,
        (Plains | Forest | Desert | Tundra, DeepOcean) => Cliff,
        (Lake, Plains | Forest | Desert | Tundra | Mountain | Snow) => LakeShore,
        (LakeShore, Lake) => LakeShore,
        (Plains | Forest | Desert | Tundra | Mountain | Snow, Lake) => LakeShore,
        (River, Plains | Forest | Desert | Tundra | Mountain | Snow | Beach) => RiverBank,
        (RiverBank, River) => RiverBank,
        (Plains | Forest | Desert | Tundra | Mountain | Snow, River) => RiverBank,
        (Beach, Ocean | Lake | River) => Beach,
        (Cliff, Ocean | Lake | River) => Cliff,
        (Beach, Plains | Forest | Desert | Tundra) => Beach,
        (Cliff, Plains | Forest) => Cliff,
        _ => self_type,
    }
}

// ---- terrain generation ----

pub const ROAD_CORRIDOR: f32 = 180.0;

#[derive(Resource)]
pub struct TerrainGen {
    height: Fbm<Perlin>,    // unified heightmap: continents + mountains
    detail: Fbm<Perlin>,    // fine surface detail
    moisture: Fbm<Perlin>,
    temp_noise: Fbm<Perlin>,
    warp: Fbm<Perlin>,
    roads: Option<RoadField>,
    flow_dir: Vec<usize>,
    flow_accum: Vec<f32>,
    /// Per-vertex erosion depression (0 = none, 1 = deep river valley).
    river_depth: Vec<f32>,
    /// Precomputed raw vertex elevations (without erosion), for fast flow computation.
    vert_elev_raw: Vec<f32>,
    /// Precomputed per-vertex elevation (with erosion), moisture, temperature.
    vert_elev: Vec<f32>,
    vert_moist: Vec<f32>,
    vert_temp: Vec<f32>,
    verts: Vec<Vec3>,
    /// Columnar adjacency: adj_off[i] = start in adj_data, adj_off[i+1] = end.
    adj_off: Vec<usize>,
    adj_data: Vec<usize>,
    /// Spatial grid for O(1) nearest_vert lookup: lat/lon buckets of vertex indices.
    vert_grid: Vec<Vec<usize>>,
    seed: u64,
}

struct RoadField {
    cells: HashMap<(i32, i32), Vec<Vec3>>,
}

impl RoadField {
    const CELL: f32 = ROAD_CORRIDOR;

    fn key(dir: Vec3) -> (i32, i32) {
        let c = Self::CELL / PLANET_RADIUS;
        let lat = dir.y.clamp(-1.0, 1.0).acos();
        let lon = dir.z.atan2(dir.x) + std::f32::consts::PI;
        ((lat / c).floor() as i32, (lon / c).floor() as i32)
    }

    fn new(points: impl Iterator<Item = Vec3>) -> Self {
        let mut cells: HashMap<(i32, i32), Vec<Vec3>> = Default::default();
        for p in points {
            cells.entry(Self::key(p)).or_default().push(p);
        }
        Self { cells }
    }

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
        let sub = 5;
        let (verts, adj_off, adj_data) = build_ico_grid(sub);

        // Multi-octave height: 6 octaves, each half amplitude, double frequency.
        let height = Fbm::<Perlin>::new(seed)
            .set_octaves(6)
            .set_frequency(1.5)
            .set_lacunarity(2.3)
            .set_persistence(0.55);

        let sub_seed = |delta: u32| seed.wrapping_add(delta);

        let vert_grid = build_vert_grid(&verts, 32, 64);

        let mut tg = Self {
            height,
            detail: Fbm::<Perlin>::new(sub_seed(1)).set_octaves(3).set_frequency(7.0),
            moisture: Fbm::<Perlin>::new(sub_seed(2)).set_octaves(4).set_frequency(2.5),
            temp_noise: Fbm::<Perlin>::new(sub_seed(3)).set_octaves(3).set_frequency(2.0),
            warp: Fbm::<Perlin>::new(sub_seed(4)).set_octaves(3).set_frequency(2.8),
            roads: None,
            flow_dir: vec![usize::MAX; verts.len()],
            flow_accum: vec![0.0; verts.len()],
            river_depth: vec![0.0; verts.len()],
            vert_elev_raw: vec![0.0; verts.len()],
            vert_elev: vec![0.0; verts.len()],
            vert_moist: vec![0.0; verts.len()],
            vert_temp: vec![0.0; verts.len()],
            verts,
            adj_off,
            adj_data,
            vert_grid,
            seed: seed as u64,
        };
        tg.precompute_vert_elevations();
        tg.smooth_vert_elevations();
        tg.precompute_vert_samples();
        tg.compute_flow();
        tg.compute_erosion();
        tg
    }

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

    pub fn road_proximity(&self, pos: SpherePos) -> f32 {
        self.roads.as_ref().map(|rf| {
            let d = rf.nearest(pos.0);
            if d < 25.0 { 0.0 } else { ((d - 25.0) / (ROAD_CORRIDOR - 25.0)).clamp(0.0, 1.0) }
        }).unwrap_or(0.0)
    }

    // ---- unified elevation (the ONLY source of height) ----

    pub fn elevation_at(&self, pos: SpherePos) -> f32 {
        let w = self.warped(pos);
        // Multi-octave height: produces landmass shapes with natural mountain ranges.
        let raw = self.height.get(w) as f32;
        // Fine detail on top (only on land).
        let land = smoothstep(0.0, 0.3, raw);
        let detail = self.detail.get(w) as f32 * 0.04 * land;
        // Road flattening: suppress detail near roads.
        let road_flatness = 1.0 - (1.0 - self.road_proximity(pos)).powf(2.5);
        let mut e = raw + detail * road_flatness;

        // Erosion: carve river valleys into the heightmap.
        let vi = self.nearest_vert(pos);
        let rd = self.river_depth[vi];
        if rd > 0.0 && e > -0.3 {
            // Carve a V-shaped valley: deeper in center, tapering outward.
            let valley = rd * 0.10; // max ~10% of elevation range depressed
            e -= valley;
        }

        e.clamp(-1.0, 1.0)
    }

    pub fn surface_radius(&self, pos: SpherePos) -> f32 {
        let e = self.elevation_at(pos);
        // Linear below sea level, gentle power above.
        if e > 0.0 {
            PLANET_RADIUS + e.powf(1.15) * MAX_MOUNTAIN
        } else {
            PLANET_RADIUS - (-e).powf(1.0) * MAX_DEPTH
        }
    }

    pub fn render_radius(&self, pos: SpherePos) -> f32 { self.surface_radius(pos) }
    pub fn altitude(&self, pos: SpherePos) -> f32 { self.surface_radius(pos) - PLANET_RADIUS }

    pub fn slope(&self, pos: SpherePos) -> f32 {
        let step = 3.0;
        let c = self.altitude(pos);
        let (east, north) = pos.tangent_basis();
        let ne = self.altitude(SpherePos::new((pos.0 + north * (step / PLANET_RADIUS)).normalize()));
        let ea = self.altitude(SpherePos::new((pos.0 + east * (step / PLANET_RADIUS)).normalize()));
        ((ne - c).abs() + (ea - c).abs()) / (2.0 * step)
    }

    pub fn is_habitable(&self, pos: SpherePos) -> bool {
        let a = self.altitude(pos);
        a >= HABITABLE_MIN_ALT && a <= HABITABLE_MAX_ALT
            && self.slope(pos) < HABITABLE_MAX_SLOPE
            && self.temperature_at(pos) >= HABITABLE_MIN_TEMP
            && self.temperature_at(pos) <= HABITABLE_MAX_TEMP
    }

    pub fn habitable_spawn(&self) -> SpherePos {
        fastrand::seed(self.seed);
        let mut best = SpherePos::new(Vec3::X);
        let mut best_score = f32::NEG_INFINITY;
        for _ in 0..2000 {
            let a = fastrand::f32() * std::f32::consts::TAU;
            let y = fastrand::f32() * 2.0 - 1.0;
            let r = (1.0f32 - y * y).max(0.0).sqrt();
            let p = SpherePos::new(Vec3::new(r * a.cos(), y, r * a.sin()));
            let s = self.spawn_score(p);
            if s > best_score { best = p; best_score = s; }
        }
        best
    }

    pub fn habitable_anchors(&self, count: usize, min_sep: f32) -> Vec<SpherePos> {
        fastrand::seed(self.seed);
        let mut cands: Vec<(f32, SpherePos)> = Vec::new();
        for _ in 0..6000 {
            let a = fastrand::f32() * std::f32::consts::TAU;
            let y = fastrand::f32() * 2.0 - 1.0;
            let r = (1.0f32 - y * y).max(0.0).sqrt();
            let p = SpherePos::new(Vec3::new(r * a.cos(), y, r * a.sin()));
            if self.is_habitable(p) { cands.push((self.spawn_score(p), p)); }
        }
        cands.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
        let mut anchors: Vec<SpherePos> = Vec::new();
        for (_, p) in cands {
            if anchors.iter().all(|a| a.distance(p) >= min_sep) {
                anchors.push(p);
                if anchors.len() >= count { break; }
            }
        }
        anchors
    }

    fn spawn_score(&self, pos: SpherePos) -> f32 {
        let t = self.temperature_at(pos);
        let temp_ok = 1.0 - ((t - 18.0) / 20.0).abs().clamp(0.0, 1.0);
        temp_ok * 0.6 + (1.0 - self.slope(pos).min(1.0)) * 0.4
    }

    fn moisture_at(&self, pos: SpherePos) -> f32 {
        self.moisture.get(self.warped(pos)) as f32
    }

    pub fn temperature_at(&self, pos: SpherePos) -> f32 {
        let base = 40.0;
        let lat = pos.0.y.clamp(-1.0, 1.0).asin().abs().to_degrees();
        let alt_m = self.altitude(pos);
        let lapse = alt_m * 0.0065;
        let noise = self.temp_noise.get(self.warped(pos)) as f32 * 10.0;
        (base - lat * 0.9 - lapse + noise).max(-100.0).min(60.0)
    }

    // ---- classification ----

    pub fn base_classify(&self, pos: SpherePos) -> Terrain {
        let e = self.interp_elevation(pos);
        let m = self.interp_moisture(pos);
        let t = self.interp_temperature(pos);

        if e < -0.30 { return Terrain::DeepOcean; }
        if e < 0.0 {
            if m > 0.2 && e > -0.04 { return Terrain::Lake; }
            return Terrain::Ocean;
        }

        let vi = self.nearest_vert(pos);
        if e < 0.04 && self.river_depth[vi] > 0.2 && self.flow_accum[vi] > 2.0 {
            return Terrain::River;
        }

        if t < -15.0 { return Terrain::Snow; }
        if t < 0.0 { return Terrain::Tundra; }
        if e > 0.50 { return Terrain::Mountain; }
        if t > 30.0 && m < -0.15 { Terrain::Desert }
        else if m > 0.25 { Terrain::Forest }
        else { Terrain::Plains }
    }

    pub fn classify(&self, pos: SpherePos) -> Terrain {
        let base = self.base_classify(pos);
        let step = 25.0 / PLANET_RADIUS;
        let (east, north) = pos.tangent_basis();
        for npos in &[
            SpherePos::new((pos.0 + east * step).normalize()),
            SpherePos::new((pos.0 - east * step).normalize()),
            SpherePos::new((pos.0 + north * step).normalize()),
            SpherePos::new((pos.0 - north * step).normalize()),
        ] {
            let nb = self.base_classify(*npos);
            if nb != base {
                let t = transition(base, nb);
                if t != base { return t; }
            }
        }
        base
    }

    pub fn color_at(&self, pos: SpherePos) -> Color { self.classify(pos).color() }

    // ---- internals ----

    fn warped(&self, pos: SpherePos) -> [f64; 3] {
        let d = pos.0.as_dvec3();
        let base = [d.x, d.y, d.z];
        let wx = self.warp.get([d.x + 5.2, d.y + 1.3, d.z]);
        let wy = self.warp.get([d.x, d.y + 9.7, d.z + 2.1]);
        let wz = self.warp.get([d.x + 3.4, d.y, d.z + 6.8]);
        [base[0] + wx * 0.18, base[1] + wy * 0.18, base[2] + wz * 0.18]
    }

    const VERT_GRID_LATS: usize = 32;
    const VERT_GRID_LONS: usize = 64;

    pub fn adj_of(&self, vi: usize) -> &[usize] {
        let start = self.adj_off[vi];
        let end = self.adj_off.get(vi + 1).copied().unwrap_or(self.adj_data.len());
        &self.adj_data[start..end]
    }

    pub fn nearest_vert(&self, pos: SpherePos) -> usize {
        let (li, oi) = Self::vert_grid_cell(pos.0);
        let mut best = 0;
        let mut best_dot = f32::NEG_INFINITY;
        // Check 3x3 neighborhood for wrapping longitude.
        for dl in -1i32..=1 {
            for doo in -1i32..=1 {
                let lat = (li as i32 + dl).clamp(0, Self::VERT_GRID_LATS as i32 - 1) as usize;
                let lon = ((oi as i32 + doo).rem_euclid(Self::VERT_GRID_LONS as i32)) as usize;
                let idx = lat * Self::VERT_GRID_LONS + lon;
                for &vi in &self.vert_grid[idx] {
                    let d = self.verts[vi].dot(pos.0);
                    if d > best_dot { best_dot = d; best = vi; }
                }
            }
        }
        if best_dot <= -1.0 {
            // Fallback: should never happen, but linear scan as safety.
            for (i, v) in self.verts.iter().enumerate() {
                let d = v.dot(pos.0);
                if d > best_dot { best_dot = d; best = i; }
            }
        }
        best
    }

    fn vert_grid_cell(dir: Vec3) -> (usize, usize) {
        let lat = dir.y.clamp(-1.0, 1.0).acos();
        let lon = dir.z.atan2(dir.x) + std::f32::consts::PI;
        let li = (lat / std::f32::consts::PI * Self::VERT_GRID_LATS as f32).floor() as usize;
        let oi = (lon / std::f32::consts::TAU * Self::VERT_GRID_LONS as f32).floor() as usize;
        (li.min(Self::VERT_GRID_LATS - 1), oi % Self::VERT_GRID_LONS)
    }

    fn precompute_vert_elevations(&mut self) {
        for i in 0..self.verts.len() {
            // Raw elevation without erosion, for flow computation.
            self.vert_elev_raw[i] = self.elevation_at_no_erosion(SpherePos::new(self.verts[i]));
        }
    }

    fn smooth_vert_elevations(&mut self) {
        // One iteration of Laplacian smoothing: average with neighbors.
        let n = self.verts.len();
        let mut smoothed = vec![0.0f32; n];
        for i in 0..n {
            let neighbors = self.adj_of(i);
            let sum: f32 = neighbors.iter().map(|&n| self.vert_elev_raw[n]).sum();
            smoothed[i] = self.vert_elev_raw[i] * 0.5 + (sum / neighbors.len() as f32) * 0.5;
        }
        self.vert_elev_raw = smoothed;
    }

    fn precompute_vert_samples(&mut self) {
        for i in 0..self.verts.len() {
            let pos = SpherePos::new(self.verts[i]);
            self.vert_elev[i] = self.elevation_at(pos);
            self.vert_moist[i] = self.moisture_at(pos);
            self.vert_temp[i] = self.temperature_at(pos);
        }
    }

    /// Barycentric-interpolated elevation from nearest 3 vertices (no noise).
    fn interp_elevation(&self, pos: SpherePos) -> f32 {
        self.bary_interp(pos, &self.vert_elev)
    }

    fn interp_moisture(&self, pos: SpherePos) -> f32 {
        self.bary_interp(pos, &self.vert_moist)
    }

    fn interp_temperature(&self, pos: SpherePos) -> f32 {
        self.bary_interp(pos, &self.vert_temp)
    }

    /// Find nearest 3 vertices by dot product, then barycentric interpolate their values.
    fn bary_interp(&self, pos: SpherePos, values: &[f32]) -> f32 {
        let dir = pos.0;
        // Collect indices sorted by descending dot (nearest first).
        let mut nearest: [(f32, usize); 3] = [(f32::NEG_INFINITY, 0); 3];
        let (lat_i, lon_i) = Self::vert_grid_cell(dir);
        for dl in -1i32..=1 {
            for doo in -1i32..=1 {
                let lat = (lat_i as i32 + dl).clamp(0, Self::VERT_GRID_LATS as i32 - 1) as usize;
                let lon = ((lon_i as i32 + doo).rem_euclid(Self::VERT_GRID_LONS as i32)) as usize;
                let idx = lat * Self::VERT_GRID_LONS + lon;
                for &vi in &self.vert_grid[idx] {
                    let d = self.verts[vi].dot(dir);
                    if d > nearest[0].0 {
                        nearest[2] = nearest[1];
                        nearest[1] = nearest[0];
                        nearest[0] = (d, vi);
                    } else if d > nearest[1].0 {
                        nearest[2] = nearest[1];
                        nearest[1] = (d, vi);
                    } else if d > nearest[2].0 {
                        nearest[2] = (d, vi);
                    }
                }
            }
        }
        // Barycentric weights: proportional to dot products.
        let w0 = nearest[0].0;
        let w1 = nearest[1].0;
        let w2 = nearest[2].0;
        let sum = w0 + w1 + w2;
        if sum <= 0.0 { return values[nearest[0].1]; }
        (values[nearest[0].1] * w0 + values[nearest[1].1] * w1 + values[nearest[2].1] * w2) / sum
    }


    fn elevation_at_no_erosion(&self, pos: SpherePos) -> f32 {
        let w = self.warped(pos);
        let raw = self.height.get(w) as f32;
        let land = smoothstep(0.0, 0.3, raw);
        let detail = self.detail.get(w) as f32 * 0.04 * land;
        (raw + detail).clamp(-1.0, 1.0)
    }

    // ---- flow + erosion ----

    fn compute_flow(&mut self) {
        let n = self.verts.len();
        for i in 0..n {
            let my_e = self.vert_elev_raw[i];
            let mut lowest = usize::MAX;
            let mut lowest_e = my_e;
            for &nb in self.adj_of(i) {
                let ne = self.vert_elev_raw[nb];
                if ne < lowest_e { lowest_e = ne; lowest = nb; }
            }
            self.flow_dir[i] = if lowest_e < my_e { lowest } else { usize::MAX };
        }

        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by(|&a, &b| self.vert_elev_raw[b].partial_cmp(&self.vert_elev_raw[a]).unwrap());
        for &vi in &order {
            let fd = self.flow_dir[vi];
            if fd != usize::MAX {
                self.flow_accum[fd] += self.flow_accum[vi] + 1.0;
            }
        }
    }

    fn compute_erosion(&mut self) {
        let n = self.verts.len();
        let max_accum: f32 = self.flow_accum.iter().copied().fold(0.0f32, f32::max);
        if max_accum <= 0.0 { return; }
        for i in 0..n {
            let a = self.flow_accum[i];
            if a > 0.0 {
                self.river_depth[i] = (a / max_accum).powf(0.7);
            }
        }
    }
}

fn smoothstep(a: f32, b: f32, t: f32) -> f32 {
    let x = ((t - a) / (b - a)).clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

// ---- icosphere grid ----

fn build_vert_grid(verts: &[Vec3], lats: usize, lons: usize) -> Vec<Vec<usize>> {
    let mut grid = vec![Vec::new(); lats * lons];
    for (i, v) in verts.iter().enumerate() {
        let lat = v.y.clamp(-1.0, 1.0).acos();
        let lon = v.z.atan2(v.x) + std::f32::consts::PI;
        let li = (lat / std::f32::consts::PI * lats as f32).floor() as usize;
        let oi = (lon / std::f32::consts::TAU * lons as f32).floor() as usize;
        grid[li.min(lats - 1) * lons + (oi % lons)].push(i);
    }
    grid
}

fn build_ico_grid(sub: usize) -> (Vec<Vec3>, Vec<usize>, Vec<usize>) {
    let t = (1.0 + 5.0_f32.sqrt()) / 2.0;
    let mut verts = vec![
        Vec3::new(-1.0, t, 0.0), Vec3::new(1.0, t, 0.0),
        Vec3::new(-1.0, -t, 0.0), Vec3::new(1.0, -t, 0.0),
        Vec3::new(0.0, -1.0, t), Vec3::new(0.0, 1.0, t),
        Vec3::new(0.0, -1.0, -t), Vec3::new(0.0, 1.0, -t),
        Vec3::new(t, 0.0, -1.0), Vec3::new(t, 0.0, 1.0),
        Vec3::new(-t, 0.0, -1.0), Vec3::new(-t, 0.0, 1.0),
    ];
    for v in &mut verts { *v = v.normalize(); }
    let faces: [[usize; 3]; 20] = [
        [0,11,5],[0,5,1],[0,1,7],[0,7,10],[0,10,11],[1,5,9],
        [5,11,4],[11,10,2],[10,7,6],[7,1,8],[3,9,4],[3,4,2],
        [3,2,6],[3,6,8],[3,8,9],[4,9,5],[2,4,11],[6,2,10],
        [8,6,7],[9,8,1],
    ];
    let mut tris: Vec<[usize; 3]> = faces.to_vec();

    for _ in 0..sub {
        let mut next_tris = Vec::with_capacity(tris.len() * 4);
        let mut mid_map: HashMap<(usize, usize), usize> = HashMap::new();
        for &[a, b, c] in &tris {
            let ab = mid_edge(&mut verts, &mut mid_map, a, b);
            let bc = mid_edge(&mut verts, &mut mid_map, b, c);
            let ca = mid_edge(&mut verts, &mut mid_map, c, a);
            next_tris.push([a, ab, ca]);
            next_tris.push([b, bc, ab]);
            next_tris.push([c, ca, bc]);
            next_tris.push([ab, bc, ca]);
        }
        tris = next_tris;
    }

    let (adj_off, adj_data) = build_vert_adj(&tris, verts.len());
    (verts, adj_off, adj_data)
}

fn mid_edge(verts: &mut Vec<Vec3>, map: &mut HashMap<(usize, usize), usize>, a: usize, b: usize) -> usize {
    let key = (a.min(b), a.max(b));
    if let Some(&idx) = map.get(&key) { return idx; }
    let idx = verts.len();
    verts.push(((verts[a] + verts[b]) / 2.0).normalize());
    map.insert(key, idx);
    idx
}

fn build_vert_adj(tris: &[[usize; 3]], n_verts: usize) -> (Vec<usize>, Vec<usize>) {
    let mut buckets = vec![Vec::new(); n_verts];
    for &[a, b, c] in tris {
        buckets[a].push(b); buckets[a].push(c);
        buckets[b].push(a); buckets[b].push(c);
        buckets[c].push(a); buckets[c].push(b);
    }
    let mut off = Vec::with_capacity(n_verts + 1);
    let mut data = Vec::new();
    off.push(0);
    for mut list in buckets {
        list.sort_unstable();
        list.dedup();
        data.extend_from_slice(&list);
        off.push(data.len());
    }
    (off, data)
}
