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

1. Read the root and every applicable subtree `AGENTS.md`.
2. Load only the relevant pipeline reference: [GLB catalog](references/glb-pipeline.md) for `gen_assets`, or [level precomputation](references/level-pipeline.md) for `gen_level`.
3. For `gen_level` generation-policy changes, also load `terra-worldgen` and its architecture reference.
4. Add a failing test for generator, serializer, or validation behavior before implementation.
5. Preserve stable semantic names and deterministic output. Keep `gen_level` thin; generation policy belongs in `shared::worldgen`.
6. Regenerate only the affected artifact, then run its exact check-mode or byte-comparison gate.

## Verification

Run the command sequence in the selected pipeline reference. For worldgen-policy changes, also run the `terra-worldgen` determinism gates.
