# Purpose

Private implementation modules for deterministic world generation.

# Ownership

- Owns specialized generation algorithms extracted from the public `worldgen` facade.

# Local Contracts

- Preserve generation policy, FIFO command/event order, iteration order, RNG streams, and floating-point operation order for locked seeds. Intentional `LevelData` schema changes regenerate the tracked asset and locked fingerprints.
- Use `CellId` and `FaceId` at algorithm boundaries; reserve `usize` for dense storage and solver-mesh vertices.
- Keep `shared::worldgen::{run, CompletedWorld, GenerationStats}` as the public facade.

# Work Guidance

- `classification.rs` — terrain/face classification primitives and ordered buckets.
- `domain.rs` — typed dense `CellField`, `FaceField`, and `CellSet` storage.
- `grid.rs` — fine generation lattice and typed cell/face geometry access.
- `pipeline.rs` — command/event orchestration and FIFO reaction order.
- `elevation/` — constraint storage, ordered relaxation, typed solver-vertex graph, and policy/ownership/solve generation stages. River banks retain freeboard above the raised rendered channel surface so water stays contained.
- `features/` — typed feature ownership and widening, with separate bridge painting/selection and transition resolution/blend/cluster stages. Every bridge kind shares a 1–500 m span range, gentle edge-trimmed footings, 1,000 m spacing, at least 1,000 m of saved walking, and explicit deck overlap rejection. Ocean candidates favor narrow passes and near-perpendicular shore approaches; spring-adjacent river crossings and propagated inland entry paint are forbidden.
- `projection.rs` — deterministic cell-to-face reductions.
- `regions/` — typed overlay-aware face partitioning, naming, and connector pass-through.
- `router.rs` — direction-aware road A* over dense `(CellId, incoming-edge)` state.
- `water/` — separate classification, river painting, and water normalization stages. Only components touching authored lake zones may normalize as freshwater or coastal saltwater lakes; isolated ocean-zone pockets are filled as land. Freezing is local per-face phase metadata, not terrain identity.
- `surface/` — component/body/river water surfaces, mesh construction, flora, and structure placement. River water is buried from the actual outer bank edge through its rendering apron. Local lake and river phase follows solved climate, allowing frozen high-altitude sections and liquid low-altitude sections in one named body. Ocean phase uses a colder saltwater threshold and retains only coherent shallow/coastal or extreme-polar ice sheets. Every rendered surface face owns a phase; ice covers the full shore footprint, smooths narrow liquid notches, closes small enclosed liquid holes, and falls back per corner from river to body radius at outlets. Separate per-face `SurfaceCondition` marks frozen ground across every terrain kind, including banks, lake shores, and beaches, for downstream asset placement.
- `tests/` — private facade-level elevation, topology, water, mesh, placement, and determinism tests.

# Verification

- Run shared tests, locked seed fingerprints, strict workspace Clippy, and direct seed-1337 asset comparison.

# Child DOX Index
