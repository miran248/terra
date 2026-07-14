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
- `elevation.rs` — constraint storage, ordered relaxation, result classification, and typed solver-vertex graph.
- `features.rs` — typed feature ownership and face projection rules.
- `projection.rs` — deterministic cell-to-face reductions.
- `regions.rs` — typed overlay-aware face partitioning with connector pass-through.
- `router.rs` — direction-aware road A* over dense `(CellId, incoming-edge)` state.
- `water.rs` — classification, river painting, and water normalization pipeline.
- `feature_pipeline.rs` and `feature_pipeline/` — feature painting, bridge selection, and transition resolution.
- `region_pipeline.rs` — region classification and deterministic naming.
- `surface.rs` and `surface/` — water surfaces, mesh construction, flora, and structure placement.
- `elevation_pipeline.rs` — elevation constraint construction and ordered solver passes.

# Verification

- Run shared tests, locked seed fingerprints, strict workspace Clippy, and direct seed-1337 asset comparison.

# Child DOX Index
