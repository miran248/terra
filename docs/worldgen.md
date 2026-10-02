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
- `level.rs`: typed `LevelData` schema encoded as versioned Postcard artifacts. It stores triangles, face/corner terrain identities, blends, orthogonal water depth/phase and surface condition, tags/regions, settlements, roads, scenery, and structures; runtime validates it at startup.
- `worldgen.rs`: deterministic cell-first facade. Mutable generation state stays private; public `CompletedWorld` exposes finalized `LevelData` and statistics.

## Private module map

- `classification.rs`: terrain/face classification and ordered buckets.
- `domain.rs`: typed dense `CellField`, `FaceField`, and `CellSet` storage.
- `grid.rs`: fine generation lattice and typed cell/face geometry.
- `pipeline.rs`: command/event orchestration with FIFO reaction order.
- `elevation/`: constraints, ordered relaxation, typed solver-vertex graph, policy, ownership, and solve stages. River banks retain freeboard above rendered channels.
- `features/`: feature ownership/widening, bridge selection/painting, and transition resolution/blend/cluster stages.
- `projection.rs`: deterministic cell-to-face reductions.
- `regions/`: cell-owned connected region partitioning, naming, and face projection.
- `router.rs`: direction-aware road A* over dense `(CellId, incoming-edge)` state.
- `water/`: classification, river painting, and normalization.
- `surface/`: water surfaces, mesh construction, scenery placement (including the Flora plant-life subset), and structure placement.
- `tests/`: facade-level elevation, topology, water, mesh, placement, and determinism tests.

## Determinism

Preserve generation policy, FIFO command/event order, collection iteration order, RNG streams, and floating-point operation order for locked seeds. Use typed `CellId` and `FaceId` at algorithm boundaries and `usize` only for dense storage/solver vertices. Intentional schema or output changes regenerate locked fingerprints and the local seed-1337 asset.

## Region identity

Region extents are owned by connected `CellId` components. Terrain regions use
surface cover; mountain ranges use connected mountain landform regardless of
forest, snow, or rock cover. Town and road memberships are additive to natural
geography, and a bridge structure does not replace the geography beneath it.
Region IDs are deterministic indices for a given seed and generator version;
they are not persistent identities across generator versions.

`FaceId` region memberships are a derived exact-query projection of the three
owning cells. A face reports the most represented region of each non-road kind;
the lower region ID breaks ties, even when a winning region appears on only one
cell. Road membership preserves every road ID found among those cells. Exact
point queries use `face_at` and do not combine neighboring faces.

## Water and surface policy

- Only components touching authored lake zones normalize as freshwater or coastal saltwater lakes; isolated ocean-zone pockets fill as land.
- Freezing is local per-face `WaterPhase`, not terrain identity. Local lakes/rivers may mix frozen high-altitude and liquid low-altitude sections. Oceans use a colder saltwater threshold and retain only coherent shallow/coastal or extreme-polar ice.
- Every rendered water face owns a phase. Ice covers the full shore footprint, smooths narrow liquid notches, closes small enclosed holes, and falls back per corner from river to body radius at outlets.
- `SurfaceCondition` independently marks frozen ground across every terrain kind, including banks, shores, and beaches.
- River water is buried from the actual outer bank edge through its rendering apron.

## Bridge policy

Every bridge kind uses a 1–500 m span range, gentle edge-trimmed footings, 1000 m spacing, at least 1000 m saved walking, and explicit deck-overlap rejection. Both bridge banks must reach roadable land and the road network. A bank inside a settlement gets a distinct unused entrance from its connected internal layout; a bank outside a settlement may join an exterior road beyond the footprint. Ocean bridges favor narrow passes and near-perpendicular shore approaches. Spring-adjacent river crossings and propagated inland entry paint are forbidden. Bridges between separate cliffed plateaus are deferred; retain the current footing limits until that work is designed.

## Road policy

Road centerlines and their rendered bands use only dry, non-Cliff cells with
Flat or Gentle slopes. Each road edge also needs a neighboring roadable face to
support its width; routing and connected-network grouping honor that edge
constraint. A world may contain separate road networks where suitable terrain
and feasible bridges cannot join them. Do not force routes across cliff necks,
water, or spans beyond the current bridge limit.

Settlement counts and road access are mandatory. After terrain slope
classification, generation checks a bounded, deterministic set of candidates
inside each authored Settlement zone. It accepts only sites whose complete
internal road layout and required buildings fit; if no candidate works, it
reports the zone and stops instead of dropping a settlement. Default targets
are 3 towns, 6 villages, and 3 outposts, with configurable radii of 55 m, 35 m,
and 20 m respectively.

## Verification

Run the focused shared test, all `cargo test -p shared` tests, locked-seed fingerprints, workspace `cargo clippy --workspace --all-targets -- -D warnings`, and direct seed-1337 asset comparison. Run `cargo bench -p gen_level --bench worldgen` when performance-sensitive generation changes.

For level regeneration and comparison, follow the [level pipeline](level-pipeline.md). Regenerate only artifacts affected by the change.

## Remaining agreed direction

The [glossary](../GLOSSARY.md) defines the domain model. These choices describe
the current implementation unless marked as deferred.

### Settlements

- Generate all three kinds with default targets of 3 towns, 6 villages, and 3
  outposts. Counts and road access remain mandatory.
- Towns use 55 m footprints, at least four homes and a shared well, and a small
  connected street grid.
- Villages use 35 m footprints, at least two homes and a farm, and a connected
  main street with a branch.
- Outposts use 20 m footprints, a watchtower, tent, and at least two defensive
  barriers, with a connected access spine.
- Rank feasible outpost sites to prefer elevated views over planned road routes
  and route intersections. The current score uses terrain-planned roads that
  exist before final network routing; it does not rank bridge crossings. Full
  buildability, composition, settlement counts, and road access take precedence.
- Give external approaches distinct settlement entrances. Split pass-through
  routes at those entrances and connect them to the per-kind internal roads.
  Internal street ends and intersections keep Junction/RoadEnd roles rather
  than being labeled external entrances.
- Place required structures on safe walkable ground inside the settlement
  radius. Their full footprints must clear roads, bridge entries, and other
  buildings.
- Residents, trading, shops, and defensive gameplay are outside this initial scope.

### Travel network and location memberships

- Roads and bridges keep their own identities according to the glossary. Names
  split at every endpoint, including junctions and settlement or bridge
  entrances; no additional named route spanning several connections is planned.
- The location HUD presents every membership from the exact queried face, grouped
  into geography, settlement name and kind, and roads. Bridge names are queried
  from deck geometry when the player's center is near a named deck's top surface;
  this is separate from face-region membership. The HUD does not combine
  neighboring faces or show a bridge name from below its deck. Display
  truncation never changes the stored names or memberships.
- Map labels annotate region centroids, vertically centered to the right of their
  markers without collision offsets or leader lines. Single-line labels truncate
  at the circular map boundary without changing underlying names or memberships.

### Deferred architectural work

- TODO: evaluate moving runtime zone and climate reconstruction into baked level
  data. Retain the current reconstruction for this domain-model migration; see
  [runtime contracts](runtime.md) for the current boundary.
