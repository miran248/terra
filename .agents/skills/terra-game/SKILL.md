---
name: terra-game
description: Change Terra runtime systems without breaking contracts.
version: 0.1.0
author: Miran, Hermes Agent
license: MIT
platforms: [linux, macos, windows]
metadata:
  hermes:
    tags: [terra, rust, bevy, avian3d]
    related_skills: []
---

# Terra Game

Use for changes to `crates/main`, runtime rendering, physics, input, UI, maps, weather, water, combat, loot, or chunk streaming. Do not use for offline world-generation policy; load `terra-worldgen` instead.

## Procedure

1. Read `crates/main/AGENTS.md` and [references/runtime-contracts.md](references/runtime-contracts.md); identify the affected runtime contracts.
2. Add one failing behavior test before production code. For visual behavior without an existing harness, add the narrowest deterministic unit or data-contract test and record the manual visual check.
3. Keep reusable data and algorithms in `shared`; keep Bevy scheduling and presentation in `main`.
4. Run the targeted test, then `cargo check` and `cargo clippy` from the workspace root. When assets or `LevelData` change, also load `terra-assets` and run its gates.

## Pitfalls

- Editing `Transform` does not authoritatively teleport Avian3d bodies.
- Vertex displacement without identical prepass displacement causes TAA depth failures.
- Loading-state systems cannot assume resources created by `setup_map` exist.

## Verification

Targeted tests, workspace `cargo check`, and workspace `cargo clippy` must pass. Report any required visual check explicitly.
