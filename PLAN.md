# Terrain Generation Architecture v3 (implemented)

**Tiles first, elevation second.** The tile map is decided by discrete rules
(zones, WFC, bands), then the elevation field is CONSTRAINT-SOLVED to fit it.
There are no touchup passes: every requirement is either a tile rule or a
solver constraint.

## Orchestrator — shared/src/worldgen.rs

A decide/evolve/react command-event state machine (FIFO, deterministic,
single-responsibility commands, event log as audit trail):

```
GenTerrain          L0–L2 + PROPOSED field (TerrainGen: zones, anchors,
                    river/road paths; noise field is a classification hint only)
ClassifyTiles       zone-aware base class per fine face (sub=6, 82k)
PaintRivers         river polylines → River faces
NormalizeWater      connectivity identity: body is Ocean iff it reaches
                    ocean-zone faces, else Lake; enclosed water < 30 faces
                    filled to land (no 1-tile lakes)
PaintFeatures       roads + towns face sets
ResolveTransitions  shore bands (vertex water-contact), micro WFC (edge-
                    linked, compat matrix), coast segmentation (beach ≤60 /
                    cliff ≤24 faces, alternating, named), counting rule:
                    plain tiles need ≥2 same-kind edges or become boundary
MarkBlends          inland biome boundaries carry (A, B) kind pairs —
                    the linking tiles; rendered blended, textures later
SolveElevation      Gauss-Seidel on the sub=5 vert field:
                      • HARD per-tile elevation ranges (last word each iter)
                      • HARD shelf: water depth grows with distance from land
                      • soft per-kind-pair gradient caps (≥ forced range gap)
                      • river monotone descent (until the sea), road caps
BuildRegions        edge-linked named clusters (min sizes per kind)
SelectBridges       landmass pairs via shore-band heads, union-find connect
BuildMesh           pure projection of the solved field (zero mesh edits)
BuildTags           per-face tag table (road/town/bridge/bridge-entry)
```

## Coarse layer — shared/src/zones.rs

L0 config (areas in m² → faces once) + L1 growth governed by a single
`coarse_compat` adjacency matrix (land zones never weld, lakes/settlements
inland, islands moated + seeded near land for bridgeability), articulation-safe
carving, post-generation edge validation, bounded retries.

## Data — shared/src/level.rs (format v6)

LevelData: version, seed, `vert_elev` (solved field, ~40KB — runtime loads it
via `TerrainGen::from_field`, so mesh/physics/HUD agree exactly), mesh, tiles,
`face_blend` (boundary pairs), tags, settlements, roads (bridges are runtime
entities), regions + per-face region refs (0 = none).

## Verification

`cargo test -p shared` (22): zone invariants + compat matrix, WFC rules,
solved-field invariants (tile ranges hold, no waterline walls except
cliff/mountain coasts, rivers descend to the sea, no enclosed water < 30
faces, counting rule, blends), byte-determinism.
Regenerate: `cargo run -p gen_level` (env `PLANET_SEED`; prints event log).
