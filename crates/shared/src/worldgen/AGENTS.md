# Private world-generation implementation

- Owns specialized deterministic algorithms behind `shared::worldgen::{run, CompletedWorld, GenerationStats}`.
- Preserve generation policy, FIFO command/event order, iteration order, RNG streams, and floating-point operation order for locked seeds.
- Use `CellId` and `FaceId` at algorithm boundaries; reserve `usize` for dense storage and solver vertices.
- Intentional `LevelData` changes regenerate the tracked asset and locked fingerprints.

Load [.agents/skills/terra-worldgen/SKILL.md](../../../../.agents/skills/terra-worldgen/SKILL.md) before editing this subtree. Its reference maps module ownership and the full water, bridge, surface, and verification contracts.
