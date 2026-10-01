# Private world-generation implementation

- Owns specialized deterministic algorithms behind `shared::worldgen::{run, CompletedWorld, GenerationStats}`.
- Preserve generation policy, FIFO command/event order, iteration order, RNG streams, and floating-point operation order for locked seeds.
- Use `CellId` and `FaceId` at algorithm boundaries; reserve `usize` for dense storage and solver vertices.
- Intentional `LevelData` changes regenerate the tracked asset and locked fingerprints.

Read [world-generation contracts](../../../../docs/worldgen.md) before editing this subtree for module ownership, water, bridge, surface, and verification contracts.
