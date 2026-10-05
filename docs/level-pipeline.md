# Level precomputation

- `crates/gen_level` is a thin CLI over `terra_worldgen::worldgen`. It receives `CompletedWorld`, reports statistics, serializes its finalized `terra_world::level::LevelData` with the model-owned Postcard codec, and writes the local generated file `crates/main/assets/level_{seed}.bin`. Each file starts with the `TERA` magic and little-endian schema version `1` before the Postcard payload. These binaries are ignored by Git.
- The runtime rejects missing, truncated, and unsupported headers or invalid payloads. There is no old-format migration: regenerate level binaries with the current `gen_level` when the schema changes.
- Output is deterministic for `PLANET_SEED` (default 1337). `LevelData` contains displaced visual/physics triangles, base colors, typed face/corner terrain identities, direct tags and multi-region face memberships, water/river radii, water phase, surface condition, settlements, and road/bridge paths.
- Terrain rendering and physics share displaced icosphere triangles; generation resolution is controlled by `terra_worldgen::zones::FINE_SUB`. Runtime builds bridge decks and their colliders from the baked spans.
- `terra-geometry` owns spherical operations, planet mesh queries, typed topology, and geometric road helpers. `terra-world` owns terrain classifications, world records, `LevelData`, and artifact validation. Cell-to-face projection, `LevelData` assembly, and generation statistics belong to `terra-worldgen`, not the CLI.

Generate, check, and benchmark:

```sh
cargo run -p gen_level -- crates/main/assets/level_1337.bin
PLANET_SEED=42 cargo run -p gen_level -- crates/main/assets/level_42.bin
cargo check -p gen_level
cargo bench -p terra-worldgen --bench terrain_gen
cargo bench -p gen_level --bench worldgen
```

`WORLDGEN_BENCH_RUNS` controls benchmark samples; `PLANET_SEED` controls its seed. A change affecting generated output or its schema must regenerate the relevant local asset and pass direct seed-1337 byte comparison plus locked versioned-artifact fingerprints described in [world-generation contracts](worldgen.md).

The main crate and tests that use `include_bytes!` require the local seed-1337
asset. Generate it before building from a clean checkout.

Verify the current local level without overwriting it:

```sh
level_check_dir=$(mktemp -d)
PLANET_SEED=1337 cargo run -p gen_level -- "$level_check_dir/level_1337.bin"
cmp crates/main/assets/level_1337.bin "$level_check_dir/level_1337.bin"
cargo test -p terra-worldgen locked_serialized_worlds
cargo test -p terra-world
```

For an intentional output change, regenerate the local level and update the
locked fingerprints after reviewing the change, then repeat these checks.
