---
name: terra-assets
description: Generate and verify Terra procedural assets safely.
version: 0.1.0
author: Miran, Hermes Agent
license: MIT
platforms: [linux, macos, windows]
metadata:
  hermes:
    tags: [terra, rust, gltf, assets]
    related_skills: []
---

# Terra Assets

Use for `gen_assets`, `gen_level`, generated GLB catalogs, embedded level binaries, `shared::art`, or `LevelData` serialization. Do not regenerate artifacts unless the source contract changed.

## Procedure

1. Read [references/pipelines.md](references/pipelines.md) and the nearest crate `AGENTS.md`.
2. Add a failing test for generator, serializer, or validation behavior before implementation.
3. Preserve stable semantic names and deterministic output. Keep `gen_level` thin; generation policy belongs in `shared::worldgen`.
4. Regenerate only the affected artifact, then run its exact check-mode or byte-comparison gate.

## Quick reference

- GLB generation: `cargo run -p gen_assets -- --out-dir crates/main/assets/models`
- GLB verification: `cargo run -p gen_assets -- --check --out-dir crates/main/assets/models`
- Level generation: `cargo run -p gen_level -- crates/main/assets/level_1337.bin`
- Alternate seed: `PLANET_SEED=42 cargo run -p gen_level -- crates/main/assets/level_42.bin`

## Pitfalls

- `gen_assets --check` must never write.
- Schema changes require synchronized runtime code and regenerated embedded data.
- Generated names and sockets are runtime API, not implementation detail.

## Verification

Run `cargo test -p gen_assets`, GLB check mode, the relevant shared tests, and `cargo check -p gen_level`. For worldgen changes, also run the `terra-worldgen` determinism gates.
