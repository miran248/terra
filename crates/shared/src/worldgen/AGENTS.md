# Private world-generation implementation

- Owns specialized deterministic algorithms behind `shared::worldgen::{run, CompletedWorld, GenerationStats}`.
- Preserve generation policy, FIFO command/event order, iteration order, RNG streams, and floating-point operation order for locked seeds.
- Import `CellId`, `FaceId`, and spherical operations from `terra_geometry`; use typed identities at algorithm boundaries and reserve `usize` for dense storage and solver vertices.
- Import terrain classifications, settlement configuration, and finalized world records from `terra_world`.
- Intentional `LevelData` changes regenerate the local ignored seed-1337 asset and locked fingerprints. Generate that asset before building targets that embed it.

Read [world-generation contracts](../../../../docs/worldgen.md) before editing this subtree for module ownership, water, bridge, surface, and verification contracts.
