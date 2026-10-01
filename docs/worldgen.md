# World-generation architecture and contracts

## Shared ownership

- `art.rs`: canonical procedural-art names, catalog paths, deterministic variants, and collider specifications shared by generator and runtime.
- `items.rs`: `Material`, `WeaponKind` statistics/durability, and `Recipe`.
- `theme.rs`: shared UI palette and bundled Monaspace Neon `FONT_PATH`.
- `sphere.rs`: meter unit, 2000 m `PLANET_RADIUS`, `SpherePos`, geodesic stepping/distance/ring/random/slerp, and tangent bases.
- `terrain.rs`: deterministic `TerrainGen`; six-octave FBM, domain warp, moisture/temperature, lake beds/containment, water identity, flow accumulation, erosion, barycentric sampling, and ground-contact `RiverSpring`.
- `zones.rs`: five separated medium continents, three offshore islands, approximately even land/water, five mountain ranges, three lakes, and six river routes.
- `coarse_river.rs`: deterministic BFS routing with zone-adjacency tie order and avoidance of prior rivers/settlements.
- `topology.rs`: sole owner of canonical cells/faces, adjacency, incidence, component traversal, bounded distance, predicate paths, and unweighted shortest paths. `CellId` is authoritative terrain identity; `FaceId` is derived query/presentation identity.
- `roads.rs`: deterministic 12-settlement network, shore-routed bridges, `PathKind::Bridge`, wobbled slerp, and straight fallback.
- `planet.rs`: icosphere triangles plus grid-indexed ray intersection and `face_at` queries.
- `level.rs`: typed Postcard `LevelData` schema. It stores triangles, face/corner terrain identities, blends, orthogonal water depth/phase and surface condition, tags/regions, settlements, and roads; runtime validates it at startup.
- `worldgen.rs`: deterministic cell-first facade. Mutable generation state stays private; public `CompletedWorld` exposes finalized `LevelData` and statistics.

## Private module map

- `classification.rs`: terrain/face classification and ordered buckets.
- `domain.rs`: typed dense `CellField`, `FaceField`, and `CellSet` storage.
- `grid.rs`: fine generation lattice and typed cell/face geometry.
- `pipeline.rs`: command/event orchestration with FIFO reaction order.
- `elevation/`: constraints, ordered relaxation, typed solver-vertex graph, policy, ownership, and solve stages. River banks retain freeboard above rendered channels.
- `features/`: feature ownership/widening, bridge selection/painting, and transition resolution/blend/cluster stages.
- `projection.rs`: deterministic cell-to-face reductions.
- `regions/`: typed overlay-aware partitioning, naming, and connector pass-through.
- `router.rs`: direction-aware road A* over dense `(CellId, incoming-edge)` state.
- `water/`: classification, river painting, and normalization.
- `surface/`: water surfaces, mesh construction, flora, and structure placement.
- `tests/`: facade-level elevation, topology, water, mesh, placement, and determinism tests.

## Determinism

Preserve generation policy, FIFO command/event order, collection iteration order, RNG streams, and floating-point operation order for locked seeds. Use typed `CellId` and `FaceId` at algorithm boundaries and `usize` only for dense storage/solver vertices. Intentional schema or output changes regenerate locked fingerprints and the seed-1337 tracked asset.

## Water and surface policy

- Only components touching authored lake zones normalize as freshwater or coastal saltwater lakes; isolated ocean-zone pockets fill as land.
- Freezing is local per-face `WaterPhase`, not terrain identity. Local lakes/rivers may mix frozen high-altitude and liquid low-altitude sections. Oceans use a colder saltwater threshold and retain only coherent shallow/coastal or extreme-polar ice.
- Every rendered water face owns a phase. Ice covers the full shore footprint, smooths narrow liquid notches, closes small enclosed holes, and falls back per corner from river to body radius at outlets.
- `SurfaceCondition` independently marks frozen ground across every terrain kind, including banks, shores, and beaches.
- River water is buried from the actual outer bank edge through its rendering apron.

## Bridge policy

Every bridge kind uses a 1–500 m span range, gentle edge-trimmed footings, 1000 m spacing, at least 1000 m saved walking, and explicit deck-overlap rejection. Ocean bridges favor narrow passes and near-perpendicular shore approaches. Spring-adjacent river crossings and propagated inland entry paint are forbidden.

## Verification

Run the focused shared test, all `cargo test -p shared` tests, locked-seed fingerprints, workspace `cargo clippy --workspace --all-targets -- -D warnings`, and direct seed-1337 asset comparison. Run `cargo bench -p gen_level --bench worldgen` when performance-sensitive generation changes.

For level regeneration and comparison, follow the [level pipeline](level-pipeline.md). Regenerate only artifacts affected by the change.

## Agreed direction — not yet implemented

The [glossary](../GLOSSARY.md) defines the intended domain model. The following
choices guide later implementation; they do not describe current capabilities.

### Settlements

- Generate all three settlement kinds in every world. Initial configurable targets
  are 3 towns, 6 villages, and 3 outposts; the configured counts and road access
  for every settlement are mandatory.
- Towns have larger footprints, multiple internal roads, homes, and shared facilities.
- Villages have smaller footprints, fewer roads, homes, and agricultural features.
- Outposts have compact footprints, watchtowers, defensive barriers, and basic shelter.
- Prefer strategic outpost locations overlooking roads or crossings, especially
  elevated sites. Buildable footprints, settlement counts, and road access take
  precedence over that preference; use the best valid sites available.
- Each settlement has distinct entrances connected by internal roads. Internal
  roads follow the same endpoint and naming rules as external roads.
- Residents, trading, shops, and defensive gameplay are outside this initial scope.

### Travel network and regions

- Implement road and bridge identities according to the glossary. Names split at
  every endpoint, including junctions and settlement or bridge entrances; no
  additional named route spanning several connections is planned.
- Support simultaneous membership in regions of different kinds, including forest
  cover within a named mountain range. Regions of the same kind remain separate,
  except for the roads meeting at a junction.
- Show all region memberships in the location HUD. Label placement on the map
  must not change the underlying memberships.

### Deferred architectural work

- TODO: evaluate moving runtime zone and climate reconstruction into baked level
  data. Retain the current reconstruction for this domain-model migration; see
  [runtime contracts](runtime.md) for the current boundary.
