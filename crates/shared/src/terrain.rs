use bevy::prelude::{Color, Resource, Vec3};
use noise::{Fbm, MultiFractal, NoiseFn, Perlin};
use std::collections::BTreeMap;

use crate::planet::{unit_icosphere_tris, PlanetMesh};
use crate::sphere::{slerp, SpherePos, PLANET_RADIUS};
use crate::zones::{ZoneConfig, ZoneKind, Zones, COARSE_SUB};

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

// ---- terrain generation ----

/// Road links per settlement.
const ROAD_LINKS: usize = 2;

/// The single source of world structure and height. Built deterministically from a
/// seed — gen_level and the runtime construct the identical object, so the baked mesh,
/// physics queries, and HUD all agree by construction.
///
/// Layers L0–L3 live here: coarse zones, network topology (settlements, rivers,
/// roads), and the per-vertex elevation field those constraints shape.
/// Nothing after construction ever modifies elevation.
#[derive(Resource)]
pub struct TerrainGen {
    height: Fbm<Perlin>,
    detail: Fbm<Perlin>,
    moisture: Fbm<Perlin>,
    temp_noise: Fbm<Perlin>,
    warp: Fbm<Perlin>,
    /// Sub=5 icosphere vertex grid (~10k) all field queries interpolate from.
    verts: Vec<Vec3>,
    adj_off: Vec<usize>,
    adj_data: Vec<usize>,
    vert_grid: Vec<Vec<usize>>,
    vert_elev: Vec<f32>,
    vert_moist: Vec<f32>,
    vert_temp: Vec<f32>,
    /// Coarse zone layout (L1) and a coarse mesh for position → zone lookup.
    zones: Zones,
    coarse_mesh: PlanetMesh,
    /// L2 topology, exposed for gen_level's fine passes and runtime spawning.
    pub river_paths: Vec<Vec<SpherePos>>,
    pub settlement_anchors: Vec<SpherePos>,
    pub road_paths: Vec<Vec<SpherePos>>,
    seed: u32,
}

impl TerrainGen {
    /// Convenience composition of the planning API in orchestrator order —
    /// the worldgen state machine runs the same steps as separate commands
    /// (InitTerrain → ProposeElevation → PlanRivers → PlaceSettlements →
    /// PlanRoads) so each shows up in the event trace.
    pub fn new(seed: u32) -> Self {
        let mut tg = Self::init(seed);
        tg.set_vert_elevations(tg.propose_elevation());
        tg.river_paths = tg.plan_river_paths();
        tg.settlement_anchors = tg.plan_settlement_anchors();
        tg.road_paths = tg.plan_road_paths();
        tg
    }

    /// Rebuild from a solved, baked elevation field (runtime path). Skips all
    /// topology planning — the level binary already carries its results.
    pub fn from_field(seed: u32, vert_elev: Vec<f32>) -> Self {
        let mut tg = Self::init(seed);
        assert_eq!(vert_elev.len(), tg.verts.len(), "baked field size mismatch");
        tg.vert_elev = vert_elev;
        tg.precompute_climate();
        tg
    }

    /// Grid, noise generators, and coarse zones — the deterministic
    /// environment every later planning step reads. No elevation yet.
    pub fn init(seed: u32) -> Self {
        let sub = 5;
        let (verts, adj_off, adj_data) = build_ico_grid(sub);
        let vert_grid = build_vert_grid(&verts, Self::VERT_GRID_LATS, Self::VERT_GRID_LONS);

        let height = Fbm::<Perlin>::new(seed)
            .set_octaves(6)
            .set_frequency(1.5)
            .set_lacunarity(2.3)
            .set_persistence(0.55);
        let sub_seed = |delta: u32| seed.wrapping_add(delta);

        let zones = Zones::generate(seed, &ZoneConfig::default());
        let coarse_mesh = PlanetMesh::new(unit_icosphere_tris(COARSE_SUB));

        Self {
            height,
            detail: Fbm::<Perlin>::new(sub_seed(1)).set_octaves(3).set_frequency(7.0),
            moisture: Fbm::<Perlin>::new(sub_seed(2)).set_octaves(4).set_frequency(2.5),
            temp_noise: Fbm::<Perlin>::new(sub_seed(3)).set_octaves(3).set_frequency(2.0),
            warp: Fbm::<Perlin>::new(sub_seed(4)).set_octaves(3).set_frequency(2.8),
            vert_elev: vec![0.0; verts.len()],
            vert_moist: vec![0.0; verts.len()],
            vert_temp: vec![0.0; verts.len()],
            verts,
            adj_off,
            adj_data,
            vert_grid,
            zones,
            coarse_mesh,
            river_paths: Vec::new(),
            settlement_anchors: Vec::new(),
            road_paths: Vec::new(),
            seed,
        }
    }

    // ---- solver access (worldgen::SolveElevation) ----

    pub fn vert_count(&self) -> usize { self.verts.len() }
    pub fn vert_dir(&self, vi: usize) -> Vec3 { self.verts[vi] }
    pub fn vert_elevations(&self) -> &[f32] { &self.vert_elev }
    pub fn set_vert_elevations(&mut self, v: Vec<f32>) {
        assert_eq!(v.len(), self.verts.len());
        self.vert_elev = v;
        // Temperature depends on altitude; refresh the climate tables.
        self.precompute_climate();
    }
    /// The 6-vertex interpolation kernel at a position (indices + cos-distance).
    pub fn kernel(&self, pos: SpherePos) -> [(usize, f32); 6] { self.bary_kernel(pos) }

    pub fn seed(&self) -> u32 { self.seed }
    pub fn zones(&self) -> &Zones { &self.zones }

    // ---- zone lookup ----

    pub fn zone_kind_at(&self, pos: SpherePos) -> ZoneKind {
        match self.coarse_mesh.face_at(pos.0) {
            Some(fi) => self.zones.kind_of_face(fi),
            None => self.nearest_zone_kind(pos.0),
        }
    }

    fn nearest_zone_kind(&self, dir: Vec3) -> ZoneKind {
        let mut best = 0;
        let mut best_dot = f32::NEG_INFINITY;
        for (i, c) in self.zones.centroids.iter().enumerate() {
            let d = c.dot(dir);
            if d > best_dot { best_dot = d; best = i; }
        }
        self.zones.kind_of_face(best)
    }

    // ---- unified elevation (the ONLY source of height) ----

    /// Interpolated from the vertex field — identical for mesh baking and runtime.
    pub fn elevation_at(&self, pos: SpherePos) -> f32 {
        self.interp_elevation(pos)
    }

    pub fn surface_radius(&self, pos: SpherePos) -> f32 {
        let e = self.elevation_at(pos);
        if e > 0.0 {
            PLANET_RADIUS + e.powf(1.15) * MAX_MOUNTAIN
        } else {
            PLANET_RADIUS - (-e) * MAX_DEPTH
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

    pub fn temperature_at(&self, pos: SpherePos) -> f32 {
        let base = 40.0;
        let lat = pos.0.y.clamp(-1.0, 1.0).asin().abs().to_degrees();
        let alt_m = self.altitude(pos);
        let lapse = alt_m * 0.0065;
        let noise = self.temp_noise.get(self.warped(pos)) as f32 * 10.0;
        (base - lat * 0.9 - lapse + noise).max(-100.0).min(60.0)
    }

    fn moisture_at(&self, pos: SpherePos) -> f32 {
        self.moisture.get(self.warped(pos)) as f32
    }

    // ---- classification (zone-aware) ----

    pub fn base_classify(&self, pos: SpherePos) -> Terrain {
        self.classify_in_zone(pos, self.zone_kind_at(pos))
    }

    /// Fast path when the caller already knows the fine face index.
    pub fn base_classify_fine(&self, fine_fi: usize, pos: SpherePos) -> Terrain {
        self.classify_in_zone(pos, self.zones.kind_at_fine(fine_fi))
    }

    fn classify_in_zone(&self, pos: SpherePos, kind: ZoneKind) -> Terrain {
        let e = self.interp_elevation(pos);
        match kind {
            ZoneKind::Ocean => if e < -0.30 { Terrain::DeepOcean } else { Terrain::Ocean },
            ZoneKind::Lake => if e < 0.0 { Terrain::Lake } else { self.land_biome(pos, e) },
            ZoneKind::MountainRange => {
                if e < 0.0 { Terrain::Ocean }
                else if self.interp_temperature(pos) < -5.0 { Terrain::Snow }
                else { Terrain::Mountain }
            }
            _ => if e < 0.0 { Terrain::Ocean } else { self.land_biome(pos, e) },
        }
    }

    fn land_biome(&self, pos: SpherePos, e: f32) -> Terrain {
        let t = self.interp_temperature(pos);
        let m = self.interp_moisture(pos);
        if t < -15.0 { return Terrain::Snow; }
        if t < 0.0 { return Terrain::Tundra; }
        if e > 0.50 { return Terrain::Mountain; }
        if t > 30.0 && m < -0.15 { Terrain::Desert }
        else if m > 0.25 { Terrain::Forest }
        else { Terrain::Plains }
    }

    /// Point classification without face data (HUD fallback).
    pub fn classify(&self, pos: SpherePos) -> Terrain {
        self.base_classify(pos)
    }

    pub fn color_at(&self, pos: SpherePos) -> Color { self.classify(pos).color() }

    // ---- L3 stage 1: zone-remapped base elevation ----

    /// The PROPOSED field: zone-remapped noise, smoothed, identity-clamped.
    /// It is a classification hint only — the final elevation is synthesized by
    /// the constraint solver (worldgen::SolveElevation) from the tile map.
    pub fn propose_elevation(&self) -> Vec<f32> {
        let mut e = self.compute_base_elevations();
        self.smooth_vert_elevations(&mut e, 2);
        self.clamp_zone_identity(&mut e);
        e
    }

    fn compute_base_elevations(&self) -> Vec<f32> {
        let mut out = vec![0.0f32; self.verts.len()];
        for i in 0..self.verts.len() {
            let pos = SpherePos::new(self.verts[i]);
            let w = self.warped(pos);
            let raw = self.height.get(w) as f32;
            let (min, max, curve, land) = self.blended_profile(pos.0);
            let detail = if land { self.detail.get(w) as f32 * 0.06 } else { 0.0 };
            let t = ((raw + detail + 1.0) / 2.0).clamp(0.0, 1.0);
            out[i] = (min + (max - min) * t.powf(curve)).clamp(-1.0, 1.0);
        }
        out
    }

    /// Elevation profile blended across the vertex's coarse face and its ring,
    /// weighted by inverse angular distance — smooth transitions at zone borders.
    fn blended_profile(&self, dir: Vec3) -> (f32, f32, f32, bool) {
        let fi = self.coarse_mesh.face_at(dir).unwrap_or_else(|| {
            let mut best = 0;
            let mut best_dot = f32::NEG_INFINITY;
            for (i, c) in self.zones.centroids.iter().enumerate() {
                let d = c.dot(dir);
                if d > best_dot { best_dot = d; best = i; }
            }
            best
        });
        let mut min = 0.0f32;
        let mut max = 0.0f32;
        let mut curve = 0.0f32;
        let mut wsum = 0.0f32;
        let consider = |fi: usize, dir: Vec3, min: &mut f32, max: &mut f32, curve: &mut f32, wsum: &mut f32| {
            let angle = self.zones.centroids[fi].dot(dir).clamp(-1.0, 1.0).acos();
            let w = 1.0 / (0.08 + angle);
            let p = self.zones.kind_of_face(fi).elevation_profile();
            *min += p.min * w;
            *max += p.max * w;
            *curve += p.curve * w;
            *wsum += w;
        };
        // Two rings of coarse faces with a soft falloff: stretches the ocean →
        // continent gradient over ~2 coarse faces so coasts shelve gently instead
        // of dropping off a wall.
        let mut ring: Vec<usize> = vec![fi];
        for &nb in &self.zones.adj[fi] {
            ring.push(nb as usize);
        }
        for i in 1..4.min(ring.len()) {
            for &nb in &self.zones.adj[ring[i]] {
                let nb = nb as usize;
                if !ring.contains(&nb) {
                    ring.push(nb);
                }
            }
        }
        for f in ring {
            consider(f, dir, &mut min, &mut max, &mut curve, &mut wsum);
        }
        let land = !self.zones.kind_of_face(fi).is_water();
        (min / wsum, max / wsum, curve / wsum, land)
    }

    fn smooth_vert_elevations(&self, e: &mut Vec<f32>, passes: usize) {
        for _ in 0..passes {
            let n = self.verts.len();
            let mut smoothed = vec![0.0f32; n];
            for i in 0..n {
                let neighbors = self.adj_of(i);
                let sum: f32 = neighbors.iter().map(|&n| e[n]).sum();
                smoothed[i] = e[i] * 0.5 + (sum / neighbors.len() as f32) * 0.5;
            }
            *e = smoothed;
        }
    }

    /// L1 zone identity is authoritative: blending may cross the land/water
    /// boundary only within one coarse face of it. Vertices whose coarse face is
    /// *interior* to a zone (whole ring shares the same water/land identity) are
    /// clamped to that identity's sign — islands can't sink, lakes can't dry,
    /// and no inland dip reads as ocean. River carving (after this) is the one
    /// deliberate exception.
    fn clamp_zone_identity(&self, e: &mut [f32]) {
        let n = self.zones.centroids.len();
        let interior: Vec<Option<bool>> = (0..n).map(|fi| {
            let w = self.zones.kind_of_face(fi).is_water();
            let uniform = self.zones.adj[fi].iter()
                .all(|&nb| self.zones.kind_of_face(nb as usize).is_water() == w);
            uniform.then_some(w)
        }).collect();
        for vi in 0..self.verts.len() {
            let Some(fi) = self.coarse_mesh.face_at(self.verts[vi]) else { continue };
            match interior[fi] {
                Some(true) => e[vi] = e[vi].min(-0.02),
                Some(false) => e[vi] = e[vi].max(0.02),
                None => {}
            }
        }
    }

    // ---- L2: river path planning (channels are dug by the elevation solver) ----

    pub fn plan_river_paths(&self) -> Vec<Vec<SpherePos>> {
        let mountain_zones: Vec<u16> =
            self.zones.zones_of_kind(ZoneKind::MountainRange).map(|(id, _)| id).collect();
        let river_count = ZoneConfig::default().rivers;
        let mut rng = fastrand::Rng::with_seed(self.seed as u64 ^ RIVER_RNG_SALT);
        let mut paths: Vec<Vec<SpherePos>> = Vec::new();
        // Faces (and their rings) claimed by earlier rivers: later rivers must route
        // around them. Overlapping channels create mutual dips that leave one river
        // flowing uphill past the other, so rivers keep ≥1 coarse face apart.
        let mut river_faces = std::collections::BTreeSet::new();
        for k in 0..river_count {
            let Some(&zid) = mountain_zones.get(k % mountain_zones.len().max(1)) else { break };
            let faces = &self.zones.zones[zid as usize].faces;
            let src = faces[rng.usize(..faces.len())] as usize;
            if river_faces.contains(&src) {
                continue;
            }
            if let Some((way, path_faces)) = self.coarse_path_to_water(src, &river_faces) {
                let samples = densify(&way, 25.0);
                if samples.len() >= 3 {
                    paths.push(samples);
                    for f in path_faces {
                        river_faces.insert(f);
                        for &nb in &self.zones.adj[f] {
                            river_faces.insert(nb as usize);
                        }
                    }
                }
            }
        }
        paths
    }

    /// BFS over coarse faces from `src` to the nearest water-zone face, routing
    /// around faces already claimed by other rivers; returns the waypoint polyline
    /// and the coarse faces it passes through.
    fn coarse_path_to_water(
        &self,
        src: usize,
        blocked: &std::collections::BTreeSet<usize>,
    ) -> Option<(Vec<SpherePos>, Vec<usize>)> {
        let n = self.zones.centroids.len();
        let mut prev = vec![usize::MAX; n];
        let mut q = std::collections::VecDeque::from([src]);
        prev[src] = src;
        while let Some(cur) = q.pop_front() {
            if self.zones.kind_of_face(cur).is_water() {
                let mut path = vec![cur];
                let mut c = cur;
                while c != src { c = prev[c]; path.push(c); }
                path.reverse();
                let way = path.iter().map(|&f| SpherePos::new(self.zones.centroids[f])).collect();
                return Some((way, path));
            }
            for &nb in &self.zones.adj[cur] {
                let nb = nb as usize;
                // Rivers route around other rivers and around settlement zones —
                // a carved channel would drown the town.
                if prev[nb] == usize::MAX
                    && !blocked.contains(&nb)
                    && self.zones.kind_of_face(nb) != ZoneKind::Settlement
                {
                    prev[nb] = cur;
                    q.push_back(nb);
                }
            }
        }
        None
    }

    // ---- L3 stage 3: settlement anchors (read-only refinement of L1 zones) ----

    pub fn plan_settlement_anchors(&self) -> Vec<SpherePos> {
        let mut anchors = Vec::new();
        for (_, zone) in self.zones.zones_of_kind(ZoneKind::Settlement) {
            let cands: Vec<SpherePos> = zone.faces.iter()
                .map(|&f| SpherePos::new(self.zones.centroids[f as usize]))
                .collect();
            // Flattest dry candidate; coastal blending can pull zone edges below
            // sea level, so fall back to the highest point if all are wet.
            let best = cands.iter()
                .filter(|p| self.interp_elevation(**p) > 0.02)
                .min_by(|a, b| self.slope(**a).partial_cmp(&self.slope(**b)).unwrap())
                .copied()
                .unwrap_or_else(|| {
                    cands.iter()
                        .max_by(|a, b| {
                            self.interp_elevation(**a).partial_cmp(&self.interp_elevation(**b)).unwrap()
                        })
                        .copied()
                        .unwrap_or(SpherePos::new(zone.centroid))
                });
            anchors.push(best);
        }
        anchors
    }

    // ---- L3 stage 4: roads (corridor smoothing on the shared vertices) ----

    /// Reads `settlement_anchors` — evolve must fold SettlementsPlaced first.
    pub fn plan_road_paths(&self) -> Vec<Vec<SpherePos>> {
        let hosts: Vec<u16> = self.zones.zones_of_kind(ZoneKind::Settlement)
            .map(|(_, z)| z.host.expect("settlement zone has host"))
            .collect();
        let anchors = self.settlement_anchors.clone();
        let mut linked = std::collections::BTreeSet::new();
        let mut paths: Vec<Vec<SpherePos>> = Vec::new();

        for (ai, a) in anchors.iter().enumerate() {
            let mut nbrs: Vec<(usize, f32)> = anchors.iter().enumerate()
                .filter(|(bi, _)| *bi != ai && hosts[*bi] == hosts[ai])
                .map(|(bi, b)| (bi, a.distance(*b)))
                .collect();
            nbrs.sort_by(|x, y| x.1.partial_cmp(&y.1).unwrap());
            for &(bi, dist) in nbrs.iter().take(ROAD_LINKS) {
                if !linked.insert((ai.min(bi), ai.max(bi))) {
                    continue;
                }
                let b = anchors[bi];
                let steps = (dist / crate::roads::SAMPLE_SPACING).ceil().max(1.0) as usize;
                let path = crate::roads::build_land_road_path(*a, b, steps, (a.0 + b.0).normalize());
                let on_land = path.iter().all(|p| self.interp_elevation(*p) > 0.02);
                if on_land {
                    paths.push(path);
                }
            }
        }
        paths
    }

    fn precompute_climate(&mut self) {
        for i in 0..self.verts.len() {
            let pos = SpherePos::new(self.verts[i]);
            self.vert_moist[i] = self.moisture_at(pos);
            self.vert_temp[i] = self.temperature_at(pos);
        }
    }

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

    fn interp_elevation(&self, pos: SpherePos) -> f32 {
        self.bary_interp(pos, &self.vert_elev)
    }

    fn interp_moisture(&self, pos: SpherePos) -> f32 {
        self.bary_interp(pos, &self.vert_moist)
    }

    fn interp_temperature(&self, pos: SpherePos) -> f32 {
        self.bary_interp(pos, &self.vert_temp)
    }

    /// The 6 nearest vertices to `pos` (the interpolation kernel), with cos-distances.
    fn bary_kernel(&self, pos: SpherePos) -> [(usize, f32); 6] {
        let dir = pos.0;
        const K: usize = 6;
        let mut nearest: [(f32, usize); K] = [(f32::NEG_INFINITY, 0); K];
        let (lat_i, lon_i) = Self::vert_grid_cell(dir);
        for dl in -1i32..=1 {
            for doo in -1i32..=1 {
                let lat = (lat_i as i32 + dl).clamp(0, Self::VERT_GRID_LATS as i32 - 1) as usize;
                let lon = ((lon_i as i32 + doo).rem_euclid(Self::VERT_GRID_LONS as i32)) as usize;
                let idx = lat * Self::VERT_GRID_LONS + lon;
                for &vi in &self.vert_grid[idx] {
                    let d = self.verts[vi].dot(dir);
                    for i in 0..K {
                        if d > nearest[i].0 {
                            nearest[i..].rotate_right(1);
                            nearest[i] = (d, vi);
                            break;
                        }
                    }
                }
            }
        }
        nearest.map(|(d, vi)| (vi, d))
    }

    /// Inverse-distance-weighted interpolation from nearest 6 vertices.
    fn bary_interp(&self, pos: SpherePos, values: &[f32]) -> f32 {
        let kernel = self.bary_kernel(pos);
        // Inverse distance weighting: weight = 1/(1 - dot) avoids singularities.
        let mut sum = 0.0f32;
        let mut weighted = 0.0f32;
        for (vi, dot) in kernel {
            let w = 1.0 / (1.01 - dot).max(0.01);
            weighted += values[vi] * w;
            sum += w;
        }
        if sum > 0.0 { weighted / sum } else { values[kernel[0].0] }
    }
}

/// Densify a waypoint polyline with slerp samples roughly `spacing` meters apart.
fn densify(waypoints: &[SpherePos], spacing: f32) -> Vec<SpherePos> {
    let mut out = Vec::new();
    for seg in waypoints.windows(2) {
        let d = seg[0].distance(seg[1]);
        let steps = (d / spacing).ceil().max(1.0) as usize;
        for k in 0..steps {
            out.push(slerp(seg[0], seg[1], k as f32 / steps as f32));
        }
    }
    if let Some(last) = waypoints.last() {
        out.push(*last);
    }
    out
}

/// Distinct RNG stream per layer, so tweaking one layer never reshuffles another.
const RIVER_RNG_SALT: u64 = 0x9e3779b97f4a7c15;

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
        let mut mid_map: BTreeMap<(usize, usize), usize> = BTreeMap::new();
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

fn mid_edge(verts: &mut Vec<Vec3>, map: &mut BTreeMap<(usize, usize), usize>, a: usize, b: usize) -> usize {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_reconstruction() {
        // Runtime `TerrainGen::new(seed)` must reproduce gen-time elevations exactly.
        let a = TerrainGen::new(1337);
        let b = TerrainGen::new(1337);
        assert_eq!(a.vert_elev, b.vert_elev);
        assert_eq!(a.settlement_anchors.len(), b.settlement_anchors.len());
        assert_eq!(a.road_paths.len(), b.road_paths.len());
    }

    // River descent is enforced by the elevation solver on the FINAL field —
    // see worldgen::tests::solved_field_invariants.

    #[test]
    fn settlements_sit_on_land() {
        let tg = TerrainGen::new(1337);
        assert_eq!(tg.settlement_anchors.len(), 12);
        for a in &tg.settlement_anchors {
            assert!(tg.elevation_at(*a) > 0.0, "settlement anchor under water");
        }
    }

    #[test]
    fn roads_stay_on_land() {
        let tg = TerrainGen::new(1337);
        for (ri, path) in tg.road_paths.iter().enumerate() {
            for p in path {
                assert!(tg.elevation_at(*p) > 0.0, "road {ri} dips below sea level");
            }
        }
    }
}
