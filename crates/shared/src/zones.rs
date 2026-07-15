//! L0 feature configuration + L1 coarse zone placement.
//!
//! The planet is partitioned into disjoint, contiguous zones on a coarse icosphere
//! (sub=3, 1280 faces). All global invariants — feature counts, minimum sizes,
//! separation distances, containment — are guaranteed here by construction, so no
//! later pass ever needs to clean up water bodies or merge fragments.

use bevy::prelude::Vec3;

use crate::planet::{build_face_adjacency, unit_icosphere_tris};
use crate::sphere::PLANET_RADIUS;

pub const COARSE_SUB: usize = 3;
pub const FINE_SUB: usize = 7;
/// Each subdivision splits a face into 4 children pushed in order, so a fine face's
/// coarse ancestor is simply `fine_fi / FINE_FACES_PER_COARSE`.
pub const FINE_FACES_PER_COARSE: usize = 1 << (2 * (FINE_SUB - COARSE_SUB));

const UNASSIGNED: u16 = u16::MAX;
/// Sentinel for `claim_ok` when the claimant has no faces yet (seed placement).
const NO_ZONE: u16 = u16::MAX - 1;
const MAX_ATTEMPTS: u64 = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ZoneKind {
    Ocean,
    Continent,
    Island,
    Lake,
    MountainRange,
    Settlement,
}

impl ZoneKind {
    pub fn is_water(self) -> bool {
        matches!(self, ZoneKind::Ocean | ZoneKind::Lake)
    }

    /// How vertex noise maps to elevation inside this zone. Elevation is in [-1, 1]
    /// with sea level at 0; the fine classifier splits ranges further (DeepOcean at
    /// -0.30, Mountain at 0.50 — see `Terrain`).
    pub fn elevation_profile(self) -> ElevationProfile {
        match self {
            ZoneKind::Ocean => ElevationProfile {
                min: -0.85,
                max: -0.12,
                curve: 1.0,
            },
            ZoneKind::Continent => ElevationProfile {
                min: 0.02,
                max: 0.45,
                curve: 1.2,
            },
            // High enough that 2-ring blending against deep ocean can't sink a
            // small island below sea level.
            ZoneKind::Island => ElevationProfile {
                min: 0.12,
                max: 0.35,
                curve: 1.0,
            },
            // Deep enough that 2-ring blending against the host continent still
            // leaves the zone under water.
            ZoneKind::Lake => ElevationProfile {
                min: -0.45,
                max: -0.20,
                curve: 1.0,
            },
            ZoneKind::MountainRange => ElevationProfile {
                min: 0.45,
                max: 1.40,
                curve: 0.9,
            },
            // Deliberately gentle so the whole zone stays buildable.
            ZoneKind::Settlement => ElevationProfile {
                min: 0.03,
                max: 0.10,
                curve: 1.0,
            },
        }
    }
}

/// WFC-style adjacency rules for the coarse layer: which zone KINDS may share
/// an edge. Same zone id is always allowed; two *different* zones of the same
/// kind follow the matrix (two continents may not touch). During growth,
/// unassigned faces count as future Ocean. This single matrix replaces the
/// former ad-hoc rules (island moat, lake strictness, settlement interiority).
pub fn coarse_compat(a: ZoneKind, b: ZoneKind) -> bool {
    use ZoneKind::*;
    let is = |x: ZoneKind, y: ZoneKind| (a == x && b == y) || (a == y && b == x);
    if is(Ocean, Ocean) || is(Ocean, Continent) || is(Ocean, Island) || is(Ocean, MountainRange) {
        return true;
    }
    if is(Continent, Lake) || is(Continent, MountainRange) || is(Continent, Settlement) {
        return true;
    }
    if is(Lake, MountainRange) || is(Lake, Settlement) || is(MountainRange, Settlement) {
        return true;
    }
    // Everything else is forbidden: land zones never weld (Continent–Continent,
    // Continent–Island, Island–Island), water never touches other water zones
    // (Lake–Ocean, Lake–Lake), and settlements/lakes stay off the coast.
    false
}

#[derive(Debug, Clone, Copy)]
pub struct ElevationProfile {
    pub min: f32,
    pub max: f32,
    pub curve: f32,
}

/// One feature family: how many zones of this kind and how big.
/// Areas are user-facing m²; they are converted to coarse face counts once at load.
pub struct FeatureSpec {
    pub kind: ZoneKind,
    pub count: usize,
    pub target_area_m2: f32,
    pub min_area_m2: f32,
    /// Minimum arc distance between seeds of this kind, meters.
    pub min_distance_m: f32,
}

pub struct ZoneConfig {
    /// Top-level land features (grown into open ocean).
    pub land: Vec<FeatureSpec>,
    /// Features carved out of continent interiors.
    pub interior: Vec<FeatureSpec>,
    /// River count (paths, not zones — routed later from mountain zones to water).
    pub rivers: usize,
}

impl Default for ZoneConfig {
    fn default() -> Self {
        Self {
            // Five substantial continents keep roughly half the planet as land:
            // enough ocean for distinct shores and crossings without leaving
            // most of the playable world underwater.
            land: vec![
                FeatureSpec {
                    kind: ZoneKind::Continent,
                    count: 5,
                    target_area_m2: 5.5e6,
                    min_area_m2: 2.5e6,
                    min_distance_m: 1600.0,
                },
                FeatureSpec {
                    kind: ZoneKind::Island,
                    count: 3,
                    target_area_m2: 3.2e5,
                    min_area_m2: 5.0e4,
                    min_distance_m: 800.0,
                },
            ],
            // Settlements first: 12 seeds at 900m spacing is the tightest packing
            // problem, so it gets the pristine continents to choose from.
            interior: vec![
                FeatureSpec {
                    kind: ZoneKind::Settlement,
                    count: 12,
                    target_area_m2: 8.0e4,
                    min_area_m2: 4.0e4,
                    min_distance_m: 900.0,
                },
                FeatureSpec {
                    kind: ZoneKind::MountainRange,
                    count: 5,
                    target_area_m2: 6.5e5,
                    min_area_m2: 2.0e5,
                    min_distance_m: 900.0,
                },
                FeatureSpec {
                    kind: ZoneKind::Lake,
                    count: 3,
                    // This is a hidden containment basin around the smaller,
                    // radial noise-shaped visible lake carved at fine scale.
                    target_area_m2: 2.5e5,
                    min_area_m2: 1.0e5,
                    min_distance_m: 600.0,
                },
            ],
            rivers: 6,
        }
    }
}

pub struct Zone {
    pub kind: ZoneKind,
    pub faces: Vec<u32>,
    /// Approximate zone center (unit vector).
    pub centroid: Vec3,
    /// For interior features: the continent zone they were carved from.
    pub host: Option<u16>,
}

pub struct Zones {
    /// Coarse face centroids (unit vectors).
    pub centroids: Vec<Vec3>,
    /// Coarse face adjacency.
    pub adj: Vec<[u32; 3]>,
    /// Zone id per coarse face.
    pub zone_of: Vec<u16>,
    pub zones: Vec<Zone>,
}

impl Zones {
    pub fn face_count(&self) -> usize {
        self.zone_of.len()
    }

    pub fn kind_of_face(&self, coarse_fi: usize) -> ZoneKind {
        self.zones[self.zone_of[coarse_fi] as usize].kind
    }

    pub fn kind_at_fine(&self, fine_fi: usize) -> ZoneKind {
        self.kind_of_face(fine_fi / FINE_FACES_PER_COARSE)
    }

    pub fn zone_at_fine(&self, fine_fi: usize) -> u16 {
        self.zone_of[fine_fi / FINE_FACES_PER_COARSE]
    }

    pub fn zones_of_kind(&self, kind: ZoneKind) -> impl Iterator<Item = (u16, &Zone)> {
        self.zones
            .iter()
            .enumerate()
            .filter(move |(_, z)| z.kind == kind)
            .map(|(i, z)| (i as u16, z))
    }

    /// The land body a zone belongs to: itself for continents/islands,
    /// the host continent for interior features, `None` for water.
    pub fn land_body(&self, zone_id: u16) -> Option<u16> {
        let z = &self.zones[zone_id as usize];
        match z.kind {
            ZoneKind::Continent | ZoneKind::Island => Some(zone_id),
            ZoneKind::MountainRange | ZoneKind::Settlement => z.host,
            ZoneKind::Ocean | ZoneKind::Lake => None,
        }
    }

    pub fn generate(seed: u32, cfg: &ZoneConfig) -> Zones {
        let tris = unit_icosphere_tris(COARSE_SUB);
        let n = tris.len();
        let adj = build_face_adjacency(&tris, n);
        let centroids: Vec<Vec3> = tris
            .iter()
            .map(|t| ((t[0] + t[1] + t[2]) / 3.0).normalize())
            .collect();
        let face_area = 4.0 * std::f32::consts::PI * PLANET_RADIUS * PLANET_RADIUS / n as f32;

        for attempt in 0..MAX_ATTEMPTS {
            let mut rng = fastrand::Rng::with_seed((seed as u64) << 8 | attempt);
            if let Some(z) = try_generate(&centroids, &adj, cfg, face_area, &mut rng) {
                return z;
            }
        }
        panic!("zone generation failed after {MAX_ATTEMPTS} attempts (seed {seed})");
    }
}

fn faces_for(area_m2: f32, face_area: f32) -> usize {
    ((area_m2 / face_area).ceil() as usize).max(1)
}

fn arc_m(a: Vec3, b: Vec3) -> f32 {
    a.dot(b).clamp(-1.0, 1.0).acos() * PLANET_RADIUS
}

struct Builder<'a> {
    centroids: &'a [Vec3],
    adj: &'a [[u32; 3]],
    zone_of: Vec<u16>,
    zones: Vec<Zone>,
}

impl Builder<'_> {
    fn push_zone(&mut self, kind: ZoneKind, seed_face: usize, host: Option<u16>) -> u16 {
        let id = self.zones.len() as u16;
        self.zones.push(Zone {
            kind,
            faces: vec![seed_face as u32],
            centroid: self.centroids[seed_face],
            host,
        });
        self.zone_of[seed_face] = id;
        id
    }

    /// May zone `id` (of `kind`) claim face `fi` without violating the coarse
    /// adjacency matrix? Unassigned neighbors count as future Ocean.
    fn claim_ok(&self, fi: usize, id: u16, kind: ZoneKind) -> bool {
        self.adj[fi].iter().all(|&nb| {
            let z = self.zone_of[nb as usize];
            z == id
                || coarse_compat(
                    kind,
                    if z == UNASSIGNED {
                        ZoneKind::Ocean
                    } else {
                        self.zones[z as usize].kind
                    },
                )
        })
    }

    /// Interior carving reassigns `zone_of` without editing the host's face list;
    /// rebuild all lists from the authoritative `zone_of` map.
    fn rebuild_faces(&mut self) {
        for z in &mut self.zones {
            z.faces.clear();
        }
        for (fi, &id) in self.zone_of.iter().enumerate() {
            if id != UNASSIGNED {
                self.zones[id as usize].faces.push(fi as u32);
            }
        }
    }

    fn finish_centroids(&mut self) {
        for z in &mut self.zones {
            let sum: Vec3 = z.faces.iter().map(|&f| self.centroids[f as usize]).sum();
            z.centroid = sum.normalize_or(self.centroids[z.faces[0] as usize]);
        }
    }
}

fn try_generate(
    centroids: &[Vec3],
    adj: &[[u32; 3]],
    cfg: &ZoneConfig,
    face_area: f32,
    rng: &mut fastrand::Rng,
) -> Option<Zones> {
    let n = centroids.len();
    let mut b = Builder {
        centroids,
        adj,
        zone_of: vec![UNASSIGNED; n],
        zones: Vec::new(),
    };

    // --- top-level land: continents first, then islands into remaining ocean ---
    let mut continent_ids: Vec<u16> = Vec::new();
    for spec in &cfg.land {
        let target = faces_for(spec.target_area_m2, face_area);
        // Seeds start in open water where the adjacency matrix allows the kind
        // (an island seed next to a continent is rejected by compat, keeping the
        // one-face moat). Islands additionally place NEAR existing land — the L0
        // `adjacent_to` relationship — so their water gap stays bridgeable.
        let kind = spec.kind;
        let seeds = place_seeds(&b, spec, rng, |b, fi| {
            b.zone_of[fi] == UNASSIGNED
                && b.claim_ok(fi, NO_ZONE, kind)
                && (kind != ZoneKind::Island || near_assigned(b, fi, ISLAND_OFFSHORE_MAX_M))
        })?;
        let ids: Vec<u16> = seeds
            .iter()
            .map(|&s| b.push_zone(spec.kind, s, None))
            .collect();
        grow_simultaneous(&mut b, &ids, target, rng, |b, fi, id| {
            b.zone_of[fi] == UNASSIGNED && b.claim_ok(fi, id, kind)
        });
        if spec.kind == ZoneKind::Continent {
            continent_ids = ids.clone();
        }
        for &id in &ids {
            if b.zones[id as usize].faces.len() < faces_for(spec.min_area_m2, face_area) {
                return None;
            }
        }
    }

    // --- interior features carved out of continents ---
    for spec in &cfg.interior {
        let target = faces_for(spec.target_area_m2, face_area);
        let kind = spec.kind;
        let mut placed: Vec<Vec3> = Vec::new();
        for i in 0..spec.count {
            let host = continent_ids[i % continent_ids.len()];
            let seed = pick_interior_seed(&b, host, spec, &placed, rng)?;
            placed.push(centroids[seed]);
            let id = b.push_zone(spec.kind, seed, Some(host));
            grow_simultaneous(&mut b, &[id], target, rng, |b, fi, id| {
                // Growth stays inside the host, never severs it, and obeys the
                // adjacency matrix. Lakes additionally keep 2 coarse faces
                // (~16 fine tiles) from the future ocean so lake and sea can
                // never sit within splashing distance of each other.
                b.zone_of[fi] == host
                    && carve_safe(b, fi, host)
                    && b.claim_ok(fi, id, kind)
                    && (kind != ZoneKind::Lake || no_unassigned_within(b, fi, 2))
            });
            if b.zones[id as usize].faces.len() < faces_for(spec.min_area_m2, face_area) {
                return None;
            }
        }
    }

    b.rebuild_faces();

    // Carving must not have split any continent.
    for &cid in &continent_ids {
        if !zone_contiguous(&b, cid) {
            return None;
        }
    }

    // --- everything left is ocean ---
    let ocean_id = b.zones.len() as u16;
    let ocean_faces: Vec<u32> = (0..n)
        .filter(|&fi| b.zone_of[fi] == UNASSIGNED)
        .map(|fi| fi as u32)
        .collect();
    if ocean_faces.is_empty() {
        return None;
    }
    for &fi in &ocean_faces {
        b.zone_of[fi as usize] = ocean_id;
    }
    b.zones.push(Zone {
        kind: ZoneKind::Ocean,
        faces: ocean_faces,
        centroid: Vec3::Y,
        host: None,
    });

    // Final invariant: every coarse edge satisfies the adjacency matrix.
    for fi in 0..n {
        for &nb in &b.adj[fi] {
            let (za, zb) = (b.zone_of[fi], b.zone_of[nb as usize]);
            if za != zb && !coarse_compat(b.zones[za as usize].kind, b.zones[zb as usize].kind) {
                return None;
            }
        }
    }

    b.finish_centroids();
    Some(Zones {
        centroids: centroids.to_vec(),
        adj: adj.to_vec(),
        zone_of: b.zone_of,
        zones: b.zones,
    })
}

/// Rejection-sample `spec.count` seed faces that satisfy `ok` and pairwise separation.
fn place_seeds(
    b: &Builder,
    spec: &FeatureSpec,
    rng: &mut fastrand::Rng,
    ok: impl Fn(&Builder, usize) -> bool,
) -> Option<Vec<usize>> {
    let n = b.centroids.len();
    let mut seeds: Vec<usize> = Vec::with_capacity(spec.count);
    let mut tries = 0;
    while seeds.len() < spec.count {
        tries += 1;
        if tries > 4000 {
            return None;
        }
        let fi = rng.usize(..n);
        if !ok(b, fi) {
            continue;
        }
        if seeds
            .iter()
            .any(|&s| arc_m(b.centroids[s], b.centroids[fi]) < spec.min_distance_m)
        {
            continue;
        }
        seeds.push(fi);
    }
    Some(seeds)
}

fn pick_interior_seed(
    b: &Builder,
    host: u16,
    spec: &FeatureSpec,
    placed: &[Vec3],
    rng: &mut fastrand::Rng,
) -> Option<usize> {
    let host_faces: Vec<usize> = b.zones[host as usize]
        .faces
        .iter()
        .map(|&f| f as usize)
        .collect();
    // Lakes must start deep inland (2 rings) so they have room to grow without ever
    // touching ocean. Settlement seeds stay 1 ring from the coast so shoreline
    // elevation blending can't pull the whole zone under water.
    let interior_rings = match spec.kind {
        ZoneKind::Lake => 2,
        ZoneKind::Settlement => 1,
        _ => 0,
    };
    for _ in 0..2000 {
        let fi = host_faces[rng.usize(..host_faces.len())];
        // The host's face list goes stale as earlier features carve into it.
        if b.zone_of[fi] != host {
            continue;
        }
        if !rings_in_host(b, fi, host, interior_rings)
            || !carve_safe(b, fi, host)
            || !b.claim_ok(fi, NO_ZONE, spec.kind)
        {
            continue;
        }
        if placed
            .iter()
            .any(|p| arc_m(*p, b.centroids[fi]) < spec.min_distance_m)
        {
            continue;
        }
        return Some(fi);
    }
    None
}

/// Max arc distance from an island seed to the nearest existing land face —
/// keeps island-to-continent water gaps within bridgeable range.
const ISLAND_OFFSHORE_MAX_M: f32 = 600.0;

fn near_assigned(b: &Builder, fi: usize, max_m: f32) -> bool {
    b.zone_of
        .iter()
        .enumerate()
        .any(|(other, &z)| z != UNASSIGNED && arc_m(b.centroids[other], b.centroids[fi]) <= max_m)
}

/// Carving `fi` out of `host` must not disconnect it: all of fi's host neighbors
/// must remain mutually reachable through the host without going through fi.
fn carve_safe(b: &Builder, fi: usize, host: u16) -> bool {
    let host_nbs: Vec<usize> = b.adj[fi]
        .iter()
        .map(|&nb| nb as usize)
        .filter(|&nb| b.zone_of[nb] == host)
        .collect();
    if host_nbs.len() <= 1 {
        return true;
    }
    let mut seen = std::collections::BTreeSet::from([host_nbs[0]]);
    let mut stack = vec![host_nbs[0]];
    while let Some(cur) = stack.pop() {
        for &nb in &b.adj[cur] {
            let nb = nb as usize;
            if nb != fi && b.zone_of[nb] == host && seen.insert(nb) {
                stack.push(nb);
            }
        }
    }
    host_nbs.iter().all(|nb| seen.contains(nb))
}

/// No unassigned (future-ocean) face within `rings` steps of `fi`.
fn no_unassigned_within(b: &Builder, fi: usize, rings: usize) -> bool {
    let mut cur = vec![fi];
    let mut seen = vec![fi];
    for _ in 0..rings {
        let mut next = Vec::new();
        for &f in &cur {
            for &nb in &b.adj[f] {
                let nb = nb as usize;
                if b.zone_of[nb] == UNASSIGNED {
                    return false;
                }
                if !seen.contains(&nb) {
                    seen.push(nb);
                    next.push(nb);
                }
            }
        }
        cur = next;
    }
    true
}

/// All faces within `rings` adjacency steps of `fi` still belong to `host`.
fn rings_in_host(b: &Builder, fi: usize, host: u16, rings: usize) -> bool {
    let mut cur = vec![fi];
    let mut seen = vec![fi];
    for _ in 0..rings {
        let mut next = Vec::new();
        for &f in &cur {
            for &nb in &b.adj[f] {
                let nb = nb as usize;
                if b.zone_of[nb] != host {
                    return false;
                }
                if !seen.contains(&nb) {
                    seen.push(nb);
                    next.push(nb);
                }
            }
        }
        cur = next;
    }
    true
}

/// Round-robin BFS growth with randomized frontier pops: keeps zones compact,
/// contiguous, and stops each at `target` faces (or when boxed in).
fn grow_simultaneous(
    b: &mut Builder,
    ids: &[u16],
    target: usize,
    rng: &mut fastrand::Rng,
    allowed: impl Fn(&Builder, usize, u16) -> bool,
) {
    let mut frontiers: Vec<Vec<usize>> =
        ids.iter().map(|&id| frontier_of(b, id, &allowed)).collect();
    loop {
        let mut progressed = false;
        for (k, &id) in ids.iter().enumerate() {
            if b.zones[id as usize].faces.len() >= target {
                continue;
            }
            // Pop random frontier candidates until one is still claimable.
            while !frontiers[k].is_empty() {
                let pick = rng.usize(..frontiers[k].len());
                let fi = frontiers[k].swap_remove(pick);
                if !allowed(b, fi, id) {
                    continue;
                }
                b.zone_of[fi] = id;
                b.zones[id as usize].faces.push(fi as u32);
                for &nb in &b.adj[fi] {
                    let nb = nb as usize;
                    if allowed(b, nb, id) {
                        frontiers[k].push(nb);
                    }
                }
                progressed = true;
                break;
            }
        }
        if !progressed {
            break;
        }
    }
}

fn frontier_of(
    b: &Builder,
    id: u16,
    allowed: &impl Fn(&Builder, usize, u16) -> bool,
) -> Vec<usize> {
    let mut f = Vec::new();
    for &face in &b.zones[id as usize].faces {
        for &nb in &b.adj[face as usize] {
            let nb = nb as usize;
            if allowed(b, nb, id) {
                f.push(nb);
            }
        }
    }
    f
}

fn zone_contiguous(b: &Builder, id: u16) -> bool {
    let faces = &b.zones[id as usize].faces;
    if faces.is_empty() {
        return false;
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut stack = vec![faces[0] as usize];
    seen.insert(faces[0] as usize);
    while let Some(fi) = stack.pop() {
        for &nb in &b.adj[fi] {
            let nb = nb as usize;
            if b.zone_of[nb] == id && seen.insert(nb) {
                stack.push(nb);
            }
        }
    }
    seen.len() == faces.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make(seed: u32) -> Zones {
        Zones::generate(seed, &ZoneConfig::default())
    }

    #[test]
    fn zones_cover_all_faces_disjointly() {
        for seed in [1u32, 7, 42, 1337] {
            let z = make(seed);
            assert!(z.zone_of.iter().all(|&id| (id as usize) < z.zones.len()));
            let total: usize = z.zones.iter().map(|zn| zn.faces.len()).sum();
            assert_eq!(
                total,
                z.face_count(),
                "faces assigned exactly once (seed {seed})"
            );
        }
    }

    #[test]
    fn zones_are_contiguous() {
        let z = make(1337);
        for (id, zone) in z.zones.iter().enumerate() {
            let mut seen = std::collections::BTreeSet::new();
            let mut stack = vec![zone.faces[0] as usize];
            seen.insert(zone.faces[0] as usize);
            while let Some(fi) = stack.pop() {
                for &nb in &z.adj[fi] {
                    let nb = nb as usize;
                    if z.zone_of[nb] as usize == id && seen.insert(nb) {
                        stack.push(nb);
                    }
                }
            }
            // The single ocean zone may legitimately be split by land; all others must connect.
            if zone.kind != ZoneKind::Ocean {
                assert_eq!(
                    seen.len(),
                    zone.faces.len(),
                    "zone {id} ({:?}) fragmented",
                    zone.kind
                );
            }
        }
    }

    #[test]
    fn feature_counts_and_containment() {
        let cfg = ZoneConfig::default();
        let z = make(42);
        assert_eq!(z.zones_of_kind(ZoneKind::Continent).count(), 5);
        assert_eq!(z.zones_of_kind(ZoneKind::Island).count(), 3);
        assert_eq!(z.zones_of_kind(ZoneKind::MountainRange).count(), 5);
        assert_eq!(z.zones_of_kind(ZoneKind::Lake).count(), 3);
        assert_eq!(z.zones_of_kind(ZoneKind::Settlement).count(), 12);
        let _ = cfg;
        // Interior features name a continent host; lakes never touch ocean.
        for (id, zone) in z.zones.iter().enumerate() {
            if matches!(
                zone.kind,
                ZoneKind::Lake | ZoneKind::Settlement | ZoneKind::MountainRange
            ) {
                let host = zone.host.expect("interior feature has host");
                assert_eq!(z.zones[host as usize].kind, ZoneKind::Continent);
            }
            if zone.kind == ZoneKind::Lake {
                for &fi in &zone.faces {
                    for &nb in &z.adj[fi as usize] {
                        let nk = z.zones[z.zone_of[nb as usize] as usize].kind;
                        assert_ne!(nk, ZoneKind::Ocean, "lake zone {id} touches ocean");
                    }
                }
            }
        }
    }

    #[test]
    fn coarse_adjacency_matrix_holds() {
        for seed in [1u32, 7, 42, 555, 1337, 9999] {
            let z = make(seed);
            for fi in 0..z.face_count() {
                for &nb in &z.adj[fi] {
                    let (za, zb) = (z.zone_of[fi], z.zone_of[nb as usize]);
                    if za != zb {
                        let (ka, kb) = (z.zones[za as usize].kind, z.zones[zb as usize].kind);
                        assert!(
                            coarse_compat(ka, kb),
                            "seed {seed}: incompatible edge {ka:?}–{kb:?} at face {fi}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn deterministic() {
        let a = make(7);
        let b = make(7);
        assert_eq!(a.zone_of, b.zone_of);
    }

    #[test]
    fn fine_to_coarse_mapping_matches_geometry() {
        // A fine face's centroid must land inside its computed coarse parent.
        let fine = unit_icosphere_tris(FINE_SUB);
        let coarse = crate::planet::PlanetMesh::new(unit_icosphere_tris(COARSE_SUB));
        for fi in (0..fine.len()).step_by(997) {
            let cent = ((fine[fi][0] + fine[fi][1] + fine[fi][2]) / 3.0).normalize();
            let parent = fi / FINE_FACES_PER_COARSE;
            let hit = coarse.face_at(cent).expect("centroid hits coarse mesh");
            assert_eq!(
                hit, parent,
                "fine face {fi}: geometric parent {hit} != computed {parent}"
            );
        }
    }
}
