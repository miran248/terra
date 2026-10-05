# Level precomputation

- Owns the thin CLI that reports `terra_worldgen::worldgen::CompletedWorld` and writes its `terra_world::level::LevelData` artifact with the model-owned codec.
- Output is `crates/main/assets/level_{seed}.bin`, deterministic for `PLANET_SEED` (default 1337).
- Terrain rendering and physics share the generated displaced triangles. `terra_geometry` owns topology and mesh queries; projection, assembly, and statistics belong to `terra-worldgen`.

Read the [level pipeline](../../docs/level-pipeline.md) for generation and verification procedures, and [world-generation contracts](../../docs/worldgen.md) for generation-policy changes.
