/// Inland biome boundaries get a blend mark: the face keeps its derived
/// kind, but carries the pair it links so rendering/solving can transition
/// between the two. With cell-based tiles the boundary faces are simply the
/// faces whose corner cells disagree — edge-connected strips by construction.
/// Faces flanking a built feature blend toward it instead (feature codes).
pub(in crate::worldgen) fn mark_blends(
    grid: &Grid,
    cells: &[Terrain],
    tiles: &[Terrain],
    painted: &Painted,
) -> Vec<FaceBlend> {
    let plain = |t: Terrain| t.is_land();
    let overlay = |face_index: usize| {
        face_solid(grid, &painted.roads, FaceId::new(face_index))
            || face_solid(grid, &painted.settlements, FaceId::new(face_index))
            || face_solid(grid, &painted.bridge_entries, FaceId::new(face_index))
    };
    let mut out = Vec::new();
    for (face_index, &tile) in tiles.iter().enumerate().take(grid.face_count()) {
        if !plain(tile) || overlay(face_index) {
            continue;
        }
        // Feature flanks (a painted corner without ownership) blend toward the
        // feature; most specific wins (entry pad < town blob < road network).
        let feature = if painted_corners(grid, &painted.bridge_entries, FaceId::new(face_index)) > 0
        {
            Some(BlendTarget::BridgeEntry)
        } else if painted_corners(grid, &painted.settlements, FaceId::new(face_index)) > 0 {
            Some(BlendTarget::Settlement)
        } else if painted_corners(grid, &painted.roads, FaceId::new(face_index)) > 0 {
            Some(BlendTarget::Road)
        } else {
            None
        };
        if let Some(code) = feature {
            out.push(FaceBlend {
                face: face_index as u32,
                base: tiles[face_index],
                target: code,
            });
            continue;
        }
        // Corner cells that disagree with the face's derived kind: the face is
        // the linking tile between its kind and the most present other LAND
        // kind (water transitions are the shore band's job).
        let mut counts: BTreeMap<Terrain, usize> = BTreeMap::new();
        for cell_index in grid.face_cells(FaceId::new(face_index)).map(CellId::index) {
            let t = cells[cell_index];
            if plain(t) && t != tiles[face_index] {
                *counts.entry(t).or_default() += 1;
            }
        }
        if let Some((&other, _)) = counts.iter().max_by_key(|(_, count)| **count) {
            out.push(FaceBlend {
                face: face_index as u32,
                base: tiles[face_index],
                target: BlendTarget::Terrain(other),
            });
        }
    }
    out
}
use std::collections::BTreeMap;

use crate::level::{BlendTarget, FaceBlend};
use crate::terrain::Terrain;
use terra_geometry::topology::{CellId, FaceId};
use crate::worldgen::{Grid, Painted, face_solid, painted_corners};
