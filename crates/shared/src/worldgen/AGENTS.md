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
- `elevation/` — constraint storage, ordered relaxation, typed solver-vertex graph, and elevation generation.
- `features/` — typed feature ownership, widening, bridge selection, and transition resolution.
- `projection.rs` — deterministic cell-to-face reductions.
- `regions/` — typed overlay-aware face partitioning, naming, and connector pass-through.
- `router.rs` — direction-aware road A* over dense `(CellId, incoming-edge)` state.
- `water.rs` — classification, river painting, and water normalization pipeline.
- `surface/` — water surfaces, mesh construction, flora, and structure placement.
- `tests.rs` — private facade-level deterministic generation and invariant tests.

# Verification

- Run shared tests, locked seed fingerprints, strict workspace Clippy, and direct seed-1337 asset comparison.

# Child DOX Index
