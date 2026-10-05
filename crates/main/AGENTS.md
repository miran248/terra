# Main game crate

## Purpose and ownership

- Owns the Bevy binary, bootstrap, rendering, physics-facing game systems, UI, and runtime asset loading.
- Keep it as an orchestration layer; import reusable domain logic from its owning crate, including geometry from `terra_geometry` and world records from `terra_world`.

## Local contracts

- Use meters for all world distances, sizes, and speeds (`terra_geometry::sphere::METER`).
- Runtime consumes typed `terra_world::level::LevelData`; terrain classifications also come from `terra_world`. Terrain reconstruction remains in `shared` during this extraction step; topology planning, pathfinding, and tag decoding stay offline. Runtime mesh queries and geometric road construction use `terra_geometry`.
- Systems requiring map resources run only in `AppState::Playing`.
- Avian3d owns physics-body state. Ordinary body and render consumers read synchronized transforms. For camera or overlay targeting that needs the current occupied-body position, read Avian `Position`; a seated `Player` proxy `Transform` can lag. Teleports update Avian `Position` and velocity rather than moving `Transform` directly.
- Generated GLB scene, node, socket, material, and animation names are runtime API. Imported scenes are visual children and never own gameplay collision.
- Preserve TAA depth/motion-vector prepass parity for vertex-displaced materials.

Read [runtime contracts](../../docs/runtime.md) before changing runtime systems. For generated catalogs or embedded level data, also read the [GLB pipeline](../../docs/glb-pipeline.md) or [level pipeline](../../docs/level-pipeline.md), respectively.

## Verification

Run `cargo check` and `cargo clippy` from the workspace root after targeted checks.
