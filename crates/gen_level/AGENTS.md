# Purpose

CLI tool for precomputing level data. Generates terrain meshes, road networks, settlement positions, and bakes them into a postcard binary loaded by the main game at startup.

# Ownership

- Owns `crates/gen_level/` — level precomputation binary
- Depends on `shared` for terrain generation, road generation, icosphere mesh

# Local Contracts

- Output file: `crates/main/assets/level_{seed}.bin`
- Deterministic per seed (`PLANET_SEED` env var, default 1337)
- Produces `LevelData` serialized via `postcard`

# Work Guidance

- `LevelData` contains: displaced icosphere triangles (visual + physics), per-triangle base colors (including RiverSpring), baked water and river-surface radii, settlement positions+names, road/bridge paths
- Uses subdivision 4 icosphere (~5k tris) for both visual and physics
- Bridge faces are raised to `PLANET_RADIUS + 1.5` in the trimesh so collision is seamless
- Road/town/bridge face coloring is done during precompute (not at load time)
- All topology derivation, cell-to-face projection, `LevelData` assembly, and generation statistics complete in `shared::worldgen`; this crate only reports, encodes, and writes a `CompletedWorld`.
- Run: `cargo run -p gen_level` (writes to default path) or `cargo run -p gen_level -- <output_path>`
- Change seed: `PLANET_SEED=42 cargo run -p gen_level`
- Repeatable timing: `WORLDGEN_BENCH_RUNS=3 cargo run --release -p gen_level -- <output_path>` reports every sample and the mean while writing only the final run.

# Verification

`cargo check -p gen_level` from workspace root.

# Child DOX Index
