# Level precomputation

- `crates/gen_level` is a thin CLI over `shared::worldgen`. It receives `CompletedWorld`, reports statistics, serializes its finalized `LevelData` with Postcard, and writes `crates/main/assets/level_{seed}.bin`.
- Output is deterministic for `PLANET_SEED` (default 1337). `LevelData` contains displaced visual/physics triangles, base colors, typed face/corner terrain identities, direct tags and optional regions, water/river radii, water phase, surface condition, settlements, and road/bridge paths.
- Generation uses subdivision-4 icosphere geometry (about 5k triangles) for visuals and physics. Bridge faces are raised to `PLANET_RADIUS + 1.5`; road, town, and bridge face coloring happens offline.
- Topology derivation, cell-to-face projection, `LevelData` assembly, and generation statistics belong to `shared::worldgen`, not the CLI.

Generate, check, and benchmark:

```sh
cargo run -p gen_level -- crates/main/assets/level_1337.bin
PLANET_SEED=42 cargo run -p gen_level -- crates/main/assets/level_42.bin
cargo check -p gen_level
cargo bench -p gen_level --bench worldgen
```

`WORLDGEN_BENCH_RUNS` controls benchmark samples; `PLANET_SEED` controls its seed. A source or schema change must regenerate the relevant embedded asset and pass direct seed-1337 byte comparison plus locked fingerprints from `terra-worldgen`.
