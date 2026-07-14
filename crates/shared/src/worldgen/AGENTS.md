# Purpose

Private implementation modules for deterministic world generation.

# Ownership

- Owns specialized generation algorithms extracted from the public `worldgen` facade.

# Local Contracts

- Preserve serialized output, FIFO command/event order, iteration order, RNG streams, and floating-point operation order for locked seeds.
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
- `classification_water_impl.rs` — classification, river painting, and water normalization pipeline bodies included by the facade.
- `features_bridges_impl.rs` — feature painting, transition bands, bridge selection, and shared cell-cluster helpers included by the facade.
- `regions_impl.rs` — region classification and deterministic naming bodies included by the facade.
- `surface_placement_impl.rs` — water/river surfaces, mesh projection, flora, structures, and face-tag bodies included by the facade.
- `elevation_impl.rs` — elevation constraint construction and ordered solver passes included by the facade.

# Verification

- Run shared tests, locked seed fingerprints, strict workspace Clippy, and direct seed-1337 asset comparison.

# Child DOX Index
