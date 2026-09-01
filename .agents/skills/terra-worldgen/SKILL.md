---
name: terra-worldgen
description: Change Terra world generation deterministically.
version: 0.1.0
author: Miran, Hermes Agent
license: MIT
platforms: [linux, macos, windows]
metadata:
  hermes:
    tags: [terra, rust, worldgen, determinism]
    related_skills: []
---

# Terra Worldgen

Use for terrain, topology, water, roads, regions, elevation, placement, mesh projection, or `LevelData` generation in `crates/shared`. Do not use for runtime-only rendering or controls.

## Procedure

1. Read the nearest `AGENTS.md` plus [references/architecture.md](references/architecture.md).
2. Add a failing focused test under the existing source test modules. Include or update a locked-seed assertion when ordering, RNG, floating-point operations, or serialized output can change.
3. Preserve typed `CellId`/`FaceId` boundaries and the public `CompletedWorld` facade. Keep mutable generation state private.
4. Run the focused test, shared tests, locked-seed fingerprints, strict workspace Clippy, and direct seed-1337 asset comparison.

## Pitfalls

- Equivalent unordered collections or traversal rewrites can still change deterministic output.
- Freezing is face metadata, not terrain identity.
- River, lake, ocean, bridge, and surface policies have coupled mesh and placement contracts.

## Verification

All focused and shared tests, locked-seed fingerprints, workspace Clippy, and the seed-1337 asset comparison must pass. Benchmark only when performance-sensitive generation code changes.
