# Terrain Generation Architecture v3 (implemented)

**Tiles first, elevation second — and tiles live on VERTICES.** Tile identity
is per icosphere vertex ("cells", the hex-dual grid): around any junction
exactly 3 cells meet and every pair shares a full edge, so two same-type tiles
can never touch at a single point — zigzag/pinch is impossible by construction,
not repaired after the fact. Faces derive their render/physics type from their
3 corner cells (two agreeing corners win; junction faces prefer transition
kinds); mixed-corner faces are the blend band, and the mesh carries per-corner
colors so boundaries render as gradients. The tile map is decided by discrete
rules (zones, WFC, bands) on the cell graph, then the elevation field is
CONSTRAINT-SOLVED to fit the derived face map. No touchup passes.

## Orchestrator — shared/src/worldgen.rs

A decide/evolve/react command-event state machine (FIFO, deterministic,
single-responsibility commands, event log as audit trail):

```
InitTerrain         L0/L1: grid, noise, coarse zones (deterministic environment)
ProposeElevation    the PROPOSED field — a classification hint, pre-solver
ComputeClimate      moisture/temperature from noise + current field
                    (re-run after every field change, incl. post-solve)
PlanRivers          L2: coarse river waypoint paths
PlaceSettlements    L2: flattest dry anchor per settlement zone
PlanRoads           L2: road polylines between same-continent settlements
ClassifyTiles       zone-aware base class per CELL (sub=6 verts, 41k)
PaintRivers         river polylines → River cells
NormalizeWater      connectivity identity on cells: Ocean iff it reaches
                    ocean-zone cells, else Lake; min body size, min width,
                    lake rim dams, lakes trimmed ~250m clear of the sea
PaintFeatures       roads + towns face sets
ResolveTransitions  shore bands, micro WFC, coast segmentation (beach ≤30 /
                    cliff ≤12 cells, alternating, named) — all on the hex
                    cell graph; no repair passes exist or are needed
MarkBlends          faces whose corner cells disagree carry (A, B) kind
                    pairs (edge-connected strips by construction) +
                    feature flanks (road/town/bridge-entry codes)
SolveElevation      Gauss-Seidel on the sub=5 vert field:
                      • HARD per-tile elevation ranges (last word each iter)
                      • HARD shelf: water depth grows with distance from land
                      • soft per-kind-pair gradient caps (≥ forced range gap)
                      • river monotone descent (until the sea), road caps
BuildRegions        edge-linked named clusters (min sizes per kind)
SelectBridges       landmass pairs via shore-band heads, union-find connect
BuildMesh           pure projection of the solved field, PER-CORNER colors
                    (boundaries are gradients; color pinch cannot render)
BuildTags           per-face tag table (road/town/bridge/bridge-entry)
```

## Coarse layer — shared/src/zones.rs

L0 config (areas in m² → faces once) + L1 growth governed by a single
`coarse_compat` adjacency matrix (land zones never weld, lakes/settlements
inland, islands moated + seeded near land for bridgeability), articulation-safe
carving, post-generation edge validation, bounded retries.

## Data — shared/src/level.rs (format v7)

LevelData: version, seed, `vert_elev` (solved field, ~40KB — runtime loads it
via `TerrainGen::from_field`, so mesh/physics/HUD agree exactly), mesh, tiles,
`face_blend` (boundary pairs), tags, settlements, roads (bridges are runtime
entities), regions + per-face region refs (0 = none).

## Verification

`cargo test -p shared` (24): zone invariants + compat matrix, WFC rules,
solved-field invariants (tile ranges hold, no waterline walls except
cliff/mountain coasts, rivers descend to the sea, min water bodies,
face-from-cell derivation consistency, lake-ocean distance, cliff ramps,
blends), byte-determinism.
Regenerate: `cargo run -p gen_level` (env `PLANET_SEED`; prints event log).
