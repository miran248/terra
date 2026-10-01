# Level precomputation

- Owns the thin CLI that reports, serializes, and writes `shared::worldgen::CompletedWorld` as Postcard `LevelData`.
- Output is `crates/main/assets/level_{seed}.bin`, deterministic for `PLANET_SEED` (default 1337).
- Terrain rendering and physics share the generated displaced triangles; topology derivation, projection, assembly, and statistics remain in `shared::worldgen`.

Read the [level pipeline](../../docs/level-pipeline.md) for generation and verification procedures, and [world-generation contracts](../../docs/worldgen.md) for generation-policy changes.
