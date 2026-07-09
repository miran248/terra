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
        Terrain::DeepOcean,
        Terrain::Ocean,
        Terrain::Lake,
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

    pub fn is_land(&self) -> bool {
        !self.is_water()
    }
}

// ---- transition matrix ----

/// Which terrain type replaces this one when it borders a neighbor of a different type.
/// Only defined for pairs that need a transition; otherwise returns `*self` (no change).
fn transition(self_type: Terrain, neighbor: Terrain) -> Terrain {
    use Terrain::*;
    match (self_type, neighbor) {
        // Ocean → land: Beach at low slope, Cliff at high.
        (Ocean, Plains | Forest | Desert | Tundra) => Beach,
        (Ocean, Mountain | Snow) => Cliff,
        (DeepOcean, _) => DeepOcean, // deep ocean doesn't transition

        // Land → ocean: Cliffs (steep edge).
        (Plains | Forest | Desert | Tundra | Mountain | Snow, Ocean) => Cliff,
        (Plains | Forest | Desert | Tundra, DeepOcean) => Cliff,

        // Lake → land: LakeShore
        (Lake, Plains | Forest | Desert | Tundra | Mountain | Snow) => LakeShore,
        (LakeShore, Lake) => LakeShore,
        // Land → lake
        (Plains | Forest | Desert | Tundra | Mountain | Snow, Lake) => LakeShore,

        // River → land: RiverBank
        (River, Plains | Forest | Desert | Tundra | Mountain | Snow | Beach) => RiverBank,
        (RiverBank, River) => RiverBank,
        // Land → river
        (Plains | Forest | Desert | Tundra | Mountain | Snow | Beach, River) => RiverBank,

        // Beach/Cliff next to water stays
        (Beach, Ocean | Lake | River) => Beach,
        (Cliff, Ocean | Lake | River) => Cliff,

        // Beach/Cliff touching land — beach softens into plains
        (Beach, Plains | Forest | Desert | Tundra) => Beach,
        (Cliff, Plains | Forest) => Cliff,

        // Internal — no transition
        _ => self_type,
    }
}

// ---- terrain generation resource ----

pub const ROAD_CORRIDOR: f32 = 180.0;

#[derive(Resource)]
pub struct TerrainGen {
    continents: Fbm<Perlin>,
    mountains: Fbm<Perlin>,
    detail: Fbm<Perlin>,
    moisture: Fbm<Perlin>,
    temp_noise: Fbm<Perlin>,
    warp: Fbm<Perlin>,
    roads: Option<RoadField>,
    flow_dir: Vec<usize>,
    flow_accum: Vec<f32>,
    verts: Vec<Vec3>,
    vert_adj: Vec<Vec<usize>>,
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
        let (verts, vert_adj) = build_ico_grid(sub);

        let mut tg = Self {
            continents: Fbm::<Perlin>::new(seed).set_octaves(4).set_frequency(1.1),
            mountains: Fbm::<Perlin>::new(seed.wrapping_add(5)).set_octaves(5).set_frequency(3.2),
            detail: Fbm::<Perlin>::new(seed.wrapping_add(6)).set_octaves(3).set_frequency(7.0),
            moisture: Fbm::<Perlin>::new(seed.wrapping_add(1)).set_octaves(4).set_frequency(2.2),
            temp_noise: Fbm::<Perlin>::new(seed.wrapping_add(4)).set_octaves(3).set_frequency(1.8),
            warp: Fbm::<Perlin>::new(seed.wrapping_add(3)).set_octaves(3).set_frequency(2.8),
            roads: None,
            flow_dir: vec![usize::MAX; verts.len()],
            flow_accum: vec![0.0; verts.len()],
            verts,
            vert_adj,
            seed: seed as u64,
        };
        tg.compute_flow();
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
        self.roads
            .as_ref()
            .map(|rf| {
                let d = rf.nearest(pos.0);
                if d < 25.0 { 0.0 } else { ((d - 25.0) / (ROAD_CORRIDOR - 25.0)).clamp(0.0, 1.0) }
            })
            .unwrap_or(0.0)
    }

    pub fn continent_elevation(&self, pos: SpherePos) -> f32 {
        self.continents.get(self.warped(pos)) as f32
    }

    pub fn elevation_at(&self, pos: SpherePos) -> f32 {
        let w = self.warped(pos);
        let continent = self.continents.get(w) as f32;
        let land = smoothstep(0.0, 0.35, continent);
        let road_flatness = 1.0 - (1.0 - self.road_proximity(pos)).powf(2.5);

        let ridged = 1.0 - (self.mountains.get(w) as f32).abs();
        let mountains = ridged.powi(2) * 0.55 * land * road_flatness;
        let detail = self.detail.get(w) as f32 * 0.06 * land * road_flatness;

        (continent + mountains + detail).clamp(-1.0, 1.0)
    }

    pub fn surface_radius(&self, pos: SpherePos) -> f32 {
        let e = self.elevation_at(pos);
        if e > 0.0 {
            PLANET_RADIUS + e.powf(1.3) * MAX_MOUNTAIN
        } else {
            PLANET_RADIUS - (-e).powf(1.3) * MAX_DEPTH
        }
    }

    pub fn render_radius(&self, pos: SpherePos) -> f32 {
        self.surface_radius(pos).max(PLANET_RADIUS)
    }

    pub fn surface_world(&self, pos: SpherePos) -> bevy::prelude::Vec3 {
        pos.0 * self.surface_radius(pos)
    }

    pub fn ground_world(&self, pos: SpherePos, half_height: f32) -> bevy::prelude::Vec3 {
        let ground = self.surface_radius(pos).max(PLANET_RADIUS);
        pos.0 * (ground + half_height)
    }

    pub fn altitude(&self, pos: SpherePos) -> f32 {
        self.surface_radius(pos) - PLANET_RADIUS
    }

    pub fn slope(&self, pos: SpherePos) -> f32 {
        let step = 3.0;
        let center = self.altitude(pos);
        let (east, north) = pos.tangent_basis();
        let ne = self.altitude(SpherePos::new((pos.0 + north * (step / PLANET_RADIUS)).normalize()));
        let ea = self.altitude(SpherePos::new((pos.0 + east * (step / PLANET_RADIUS)).normalize()));
        ((ne - center).abs() + (ea - center).abs()) / (2.0 * step)
    }

    pub fn is_habitable(&self, pos: SpherePos) -> bool {
        let alt = self.altitude(pos);
        alt >= HABITABLE_MIN_ALT
            && alt <= HABITABLE_MAX_ALT
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
            if s > best_score {
                best = p;
                best_score = s;
            }
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
            if self.is_habitable(p) {
                cands.push((self.spawn_score(p), p));
            }
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
        let temp = self.temperature_at(pos);
        let temp_ok = 1.0 - ((temp - 18.0) / 20.0).abs().clamp(0.0, 1.0);
        let flat = 1.0 - self.slope(pos).min(1.0);
        temp_ok * 0.6 + flat * 0.4
    }

    fn moisture_at(&self, pos: SpherePos) -> f32 {
        self.moisture.get(self.warped(pos)) as f32
    }

    pub fn temperature_at(&self, pos: SpherePos) -> f32 {
        let base = 40.0;
        let lat = pos.0.y.clamp(-1.0, 1.0).asin().abs().to_degrees();
        let e = self.elevation_at(pos);
        let alt_m = if e > 0.0 { e * MAX_MOUNTAIN } else { e * MAX_DEPTH };
        let lapse = alt_m * 0.0065;
        let noise = self.temp_noise.get(self.warped(pos)) as f32 * 10.0;
        let lat_cool = lat * 0.9;
        base - lat_cool - lapse + noise
    }

    // ---- classification using transition matrix ----

    /// Base biome before edge transitions.
    fn base_classify(&self, pos: SpherePos) -> Terrain {
        let e = self.elevation_at(pos);
        let m = self.moisture_at(pos);
        let t = self.temperature_at(pos);

        if e < -0.30 { return Terrain::DeepOcean; }
        if e < 0.0 {
            if m > 0.2 && e > -0.04 { return Terrain::Lake; }
            return Terrain::Ocean;
        }

        let vi = self.nearest_vert(pos);
        let accum = self.flow_accum[vi];
        if e < 0.03 && accum > 3.0 && self.is_valley(vi) {
            return Terrain::River;
        }

        if t < -15.0 { return Terrain::Snow; }
        if t < 0.0 { return Terrain::Tundra; }
        if e > 0.45 { return Terrain::Mountain; }
        if t > 30.0 && m < -0.15 { Terrain::Desert }
        else if m > 0.25 { Terrain::Forest }
        else { Terrain::Plains }
    }

    /// Full classification with transition-matrix-based edge blending.
    /// Samples neighboring faces: if any neighbor differs and the matrix defines
    /// a transition, returns the transition type instead.
    pub fn classify(&self, pos: SpherePos) -> Terrain {
        let base = self.base_classify(pos);
        let step = 25.0 / PLANET_RADIUS;
        let (east, north) = pos.tangent_basis();
        let dirs = [
            SpherePos::new((pos.0 + east * step).normalize()),
            SpherePos::new((pos.0 - east * step).normalize()),
            SpherePos::new((pos.0 + north * step).normalize()),
            SpherePos::new((pos.0 - north * step).normalize()),
        ];
        for npos in &dirs {
            let nbase = self.base_classify(*npos);
            if nbase != base {
                let t = transition(base, nbase);
                if t != base {
                    return t;
                }
            }
        }
        base
    }

    pub fn color_at(&self, pos: SpherePos) -> Color {
        self.classify(pos).color()
    }

    // ---- drainage internals ----

    fn warped(&self, pos: SpherePos) -> [f64; 3] {
        let d = pos.0.as_dvec3();
        let base = [d.x, d.y, d.z];
        let wx = self.warp.get([d.x + 5.2, d.y + 1.3, d.z]);
        let wy = self.warp.get([d.x, d.y + 9.7, d.z + 2.1]);
        let wz = self.warp.get([d.x + 3.4, d.y, d.z + 6.8]);
        const AMT: f64 = 0.18;
        [base[0] + wx * AMT, base[1] + wy * AMT, base[2] + wz * AMT]
    }

    fn nearest_vert(&self, pos: SpherePos) -> usize {
        let mut best = 0;
        let mut best_dot = self.verts[0].dot(pos.0);
        for (i, v) in self.verts.iter().enumerate().skip(1) {
            let d = v.dot(pos.0);
            if d > best_dot { best_dot = d; best = i; }
        }
        best
    }

    fn vert_elevation(&self, vi: usize) -> f32 {
        self.elevation_at(SpherePos::new(self.verts[vi]))
    }

    fn is_valley(&self, vi: usize) -> bool {
        let my_e = self.vert_elevation(vi);
        let mut lower = 0;
        let mut higher = 0;
        for &n in &self.vert_adj[vi] {
            let ne = self.vert_elevation(n);
            if ne < my_e - 0.02 { lower += 1; }
            if ne > my_e + 0.02 { higher += 1; }
        }
        higher > 0 && lower == 0
    }

    fn compute_flow(&mut self) {
        for i in 0..self.verts.len() {
            let my_e = self.vert_elevation(i);
            let mut lowest_idx = usize::MAX;
            let mut lowest_e = my_e;
            for &n in &self.vert_adj[i] {
                let ne = self.vert_elevation(n);
                if ne < lowest_e { lowest_e = ne; lowest_idx = n; }
            }
            self.flow_dir[i] = if lowest_e < my_e { lowest_idx } else { usize::MAX };
        }

        let mut order: Vec<usize> = (0..self.verts.len()).collect();
        order.sort_by(|&a, &b| {
            self.vert_elevation(b).partial_cmp(&self.vert_elevation(a)).unwrap()
        });
        for &vi in &order {
            let fd = self.flow_dir[vi];
            if fd != usize::MAX {
                self.flow_accum[fd] += self.flow_accum[vi] + 1.0;
            }
        }
    }
}

fn smoothstep(a: f32, b: f32, t: f32) -> f32 {
    let x = ((t - a) / (b - a)).clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

// ---- icosphere grid ----

fn build_ico_grid(sub: usize) -> (Vec<Vec3>, Vec<Vec<usize>>) {
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

    let adj = build_vert_adj(&tris, verts.len());
    (verts, adj)
}

fn mid_edge(
    verts: &mut Vec<Vec3>,
    map: &mut HashMap<(usize, usize), usize>,
    a: usize, b: usize,
) -> usize {
    let key = (a.min(b), a.max(b));
    if let Some(&idx) = map.get(&key) { return idx; }
    let idx = verts.len();
    verts.push(((verts[a] + verts[b]) / 2.0).normalize());
    map.insert(key, idx);
    idx
}

fn build_vert_adj(tris: &[[usize; 3]], n_verts: usize) -> Vec<Vec<usize>> {
    let mut adj = vec![Vec::new(); n_verts];
    for &[a, b, c] in tris {
        adj[a].push(b); adj[a].push(c);
        adj[b].push(a); adj[b].push(c);
        adj[c].push(a); adj[c].push(b);
    }
    for list in &mut adj {
        list.sort_unstable();
        list.dedup();
    }
    adj
}
