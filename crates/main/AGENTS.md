# Main game crate

## Purpose and ownership

- Owns the Bevy binary, bootstrap, rendering, physics-facing game systems, UI, and runtime asset loading.
- Keep it as an orchestration layer; reusable logic belongs in `shared`.

## Local contracts

- Use meters for all world distances, sizes, and speeds (`shared::sphere::METER`).
- Runtime consumes typed `LevelData`; terrain derivation, topology, pathfinding, and tag decoding stay offline.
- Systems requiring map resources run only in `AppState::Playing`.
- Avian3d owns physics-body state. Read synchronized transforms; teleports update Avian `Position` and velocity rather than moving `Transform` directly.
- Generated GLB scene, node, socket, material, and animation names are runtime API. Imported scenes are visual children and never own gameplay collision.
- Preserve TAA depth/motion-vector prepass parity for vertex-displaced materials.

Load [.agents/skills/terra-game/SKILL.md](../../.agents/skills/terra-game/SKILL.md) before changing runtime systems. Load [.agents/skills/terra-assets/SKILL.md](../../.agents/skills/terra-assets/SKILL.md) when generated catalogs or embedded level data are affected.

## Verification

Run `cargo check` and `cargo clippy` from the workspace root after targeted checks.
