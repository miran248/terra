//! Micro WFC (L5): resolves transition tiles on the fine triangular grid.
//!
//! Cells are the faces whose base classification differs from an edge neighbor's;
//! interior faces are never touched — they enter as `Neighbor::Fixed` boundary
//! conditions. Neighbors are the 3 edge-sharing faces (not vertex-sharing), so
//! rules are local and symmetric. On contradiction a cell falls back to its base
//! classification; there is never a global retry.

use crate::terrain::Terrain;

const T: usize = Terrain::ALL.len();

fn idx(t: Terrain) -> usize {
    t as usize
}

/// Which terrain kinds may share an edge. Symmetric by construction.
pub struct Compat([[bool; T]; T]);

impl Default for Compat {
    fn default() -> Self {
        use Terrain::*;
        let mut c = [[false; T]; T];
        let mut allow = |a: Terrain, b: Terrain| {
            c[idx(a)][idx(b)] = true;
            c[idx(b)][idx(a)] = true;
        };
        for t in Terrain::ALL {
            allow(t, t);
        }
        // All non-peak land biomes freely neighbor each other and the
        // transition bands.
        let land = [Desert, Plains, Forest, Tundra, Swamp, Jungle, Savanna];
        for a in land {
            for b in land {
                allow(a, b);
            }
            allow(a, Mountain);
            allow(a, Beach);
            allow(a, Cliff);
            allow(a, LakeShore);
            allow(a, RiverBank);
        }
        // Swamp is wet lowland: it sits against water bodies directly.
        allow(Swamp, River);
        allow(Swamp, Lake);
        // Volcanic and Glacier ride the high ground with the other peaks.
        for p in [Volcanic, Glacier] {
            allow(p, Mountain);
            allow(p, Snow);
            allow(p, Cliff);
            allow(p, Tundra);
        }
        allow(Mountain, Snow);
        allow(Mountain, Cliff);
        allow(Snow, Tundra);
        allow(Snow, Cliff);
        allow(Ocean, Beach);
        allow(Ocean, Cliff);
        allow(Ocean, River);
        allow(Lake, LakeShore);
        allow(Lake, River);
        allow(LakeShore, RiverBank);
        allow(River, RiverBank);
        allow(RiverBank, Beach);
        Self(c)
    }
}

impl Compat {
    pub fn ok(&self, a: Terrain, b: Terrain) -> bool {
        self.0[idx(a)][idx(b)]
    }
}

#[derive(Clone, Copy)]
pub enum Neighbor {
    /// Another WFC cell (index into the cell arrays).
    Cell(usize),
    /// An interior face locked to its base classification.
    Fixed(Terrain),
}

/// Solve the boundary-cell constraint problem.
///
/// `domains[i]`: candidate terrains + weights for cell i (base prediction should
/// carry the highest weight). `neighbors[i]`: the up-to-3 edge neighbors.
/// `fallback[i]`: the base classification used when a cell contradicts.
pub fn solve(
    compat: &Compat,
    domains: &[Vec<(Terrain, f32)>],
    neighbors: &[Vec<Neighbor>],
    fallback: &[Terrain],
    seed: u64,
) -> Vec<Terrain> {
    let n = domains.len();
    let mut rng = fastrand::Rng::with_seed(seed);
    let mut dom: Vec<Vec<(Terrain, f32)>> = domains.to_vec();
    let mut result: Vec<Option<Terrain>> = vec![None; n];

    // Initial propagation from all cells.
    let mut queue: std::collections::VecDeque<usize> = (0..n).collect();
    propagate(compat, &mut dom, neighbors, &mut result, fallback, &mut queue);

    loop {
        // Min-entropy: the unsolved cell with the fewest remaining options.
        let Some(pick) = (0..n)
            .filter(|&i| result[i].is_none())
            .min_by_key(|&i| dom[i].len())
        else {
            break;
        };
        // Transitions only when forced: keep the base classification whenever it
        // survived propagation; roll weights only among forced alternatives.
        let choice = if dom[pick].iter().any(|&(t, _)| t == fallback[pick]) {
            fallback[pick]
        } else {
            weighted_pick(&dom[pick], &mut rng).unwrap_or(fallback[pick])
        };
        result[pick] = Some(choice);
        dom[pick] = vec![(choice, 1.0)];
        let mut queue: std::collections::VecDeque<usize> =
            neighbor_cells(&neighbors[pick]).collect();
        propagate(compat, &mut dom, neighbors, &mut result, fallback, &mut queue);
    }

    result.into_iter().map(|r| r.unwrap()).collect()
}

fn neighbor_cells(nbs: &[Neighbor]) -> impl Iterator<Item = usize> + '_ {
    nbs.iter().filter_map(|n| match n {
        Neighbor::Cell(i) => Some(*i),
        Neighbor::Fixed(_) => None,
    })
}

/// AC-3 style: drop options with no compatible support at any neighbor; cells that
/// reach a single option are solved, cells that reach zero fall back to base.
fn propagate(
    compat: &Compat,
    dom: &mut [Vec<(Terrain, f32)>],
    neighbors: &[Vec<Neighbor>],
    result: &mut [Option<Terrain>],
    fallback: &[Terrain],
    queue: &mut std::collections::VecDeque<usize>,
) {
    while let Some(i) = queue.pop_front() {
        if result[i].is_some() {
            continue;
        }
        let before = dom[i].len();
        let mut mine = std::mem::take(&mut dom[i]);
        mine.retain(|&(cand, _)| {
            neighbors[i].iter().all(|nb| match nb {
                Neighbor::Fixed(t) => compat.ok(cand, *t),
                Neighbor::Cell(j) => {
                    if let Some(t) = result[*j] {
                        compat.ok(cand, t)
                    } else {
                        dom[*j].iter().any(|&(t, _)| compat.ok(cand, t))
                    }
                }
            })
        });
        dom[i] = mine;
        match dom[i].len() {
            0 => {
                // Contradiction: take the base classification and keep going.
                result[i] = Some(fallback[i]);
                dom[i] = vec![(fallback[i], 1.0)];
                queue.extend(neighbor_cells(&neighbors[i]));
            }
            1 => {
                result[i] = Some(dom[i][0].0);
                queue.extend(neighbor_cells(&neighbors[i]));
            }
            len if len < before => {
                queue.extend(neighbor_cells(&neighbors[i]));
            }
            _ => {}
        }
    }
}

fn weighted_pick(options: &[(Terrain, f32)], rng: &mut fastrand::Rng) -> Option<Terrain> {
    let total: f32 = options.iter().map(|(_, w)| w).sum();
    if options.is_empty() || total <= 0.0 {
        return None;
    }
    let mut roll = rng.f32() * total;
    for &(t, w) in options {
        roll -= w;
        if roll <= 0.0 {
            return Some(t);
        }
    }
    Some(options.last().unwrap().0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use Terrain::*;

    fn boundary_domain(base: Terrain) -> Vec<(Terrain, f32)> {
        let mut d = vec![(base, 1.0)];
        for t in [Beach, Cliff, LakeShore, RiverBank] {
            if t != base {
                d.push((t, 0.3));
            }
        }
        d
    }

    #[test]
    fn shoreline_resolves_to_beach_or_cliff() {
        // A single land cell squeezed between fixed Ocean and fixed Plains.
        let compat = Compat::default();
        let domains = vec![boundary_domain(Plains)];
        let neighbors = vec![vec![Neighbor::Fixed(Ocean), Neighbor::Fixed(Plains)]];
        let out = solve(&compat, &domains, &neighbors, &[Plains], 1);
        assert!(matches!(out[0], Beach | Cliff), "got {:?}", out[0]);
    }

    #[test]
    fn interior_biome_edge_keeps_base() {
        // Plains next to Forest are directly compatible — no transition forced.
        let compat = Compat::default();
        let domains = vec![boundary_domain(Plains), boundary_domain(Forest)];
        let neighbors = vec![
            vec![Neighbor::Cell(1), Neighbor::Fixed(Plains)],
            vec![Neighbor::Cell(0), Neighbor::Fixed(Forest)],
        ];
        let out = solve(&compat, &domains, &neighbors, &[Plains, Forest], 2);
        assert_eq!(out, vec![Plains, Forest]);
    }

    #[test]
    fn contradiction_falls_back_to_base() {
        // Impossible cell: Snow base wedged between fixed Ocean and fixed Desert,
        // with no transition option offered — must fall back, not hang or panic.
        let compat = Compat::default();
        let domains = vec![vec![(Snow, 1.0)]];
        let neighbors = vec![vec![Neighbor::Fixed(Ocean), Neighbor::Fixed(Desert)]];
        let out = solve(&compat, &domains, &neighbors, &[Snow], 3);
        assert_eq!(out, vec![Snow]);
    }

    #[test]
    fn deterministic_for_seed() {
        let compat = Compat::default();
        let domains: Vec<_> = (0..50).map(|i| boundary_domain(if i % 2 == 0 { Plains } else { Forest })).collect();
        let neighbors: Vec<Vec<Neighbor>> = (0..50)
            .map(|i: usize| {
                let mut v = Vec::new();
                if i > 0 { v.push(Neighbor::Cell(i - 1)); }
                if i < 49 { v.push(Neighbor::Cell(i + 1)); }
                v.push(Neighbor::Fixed(Ocean));
                v
            })
            .collect();
        let fallback: Vec<_> = (0..50).map(|i| if i % 2 == 0 { Plains } else { Forest }).collect();
        let a = solve(&compat, &domains, &neighbors, &fallback, 42);
        let b = solve(&compat, &domains, &neighbors, &fallback, 42);
        assert_eq!(a, b);
    }
}
