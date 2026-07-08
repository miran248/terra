# Purpose

Binary crate for the rs-zombies Bevy application. Entry point, app bootstrap, and game systems live here.

# Ownership

- Owns `crates/main/` — binary, Bevy systems, rendering, game logic
- Depends on `shared` crate for common types and utilities

# Local Contracts

- Must remain a thin orchestration layer; reusable logic belongs in `shared`
- Bevy systems are registered in `main.rs` or modules under `src/`

# Work Guidance

- The world is a 3D planet (`Camera3d`, PBR meshes, `DirectionalLight`). Every actor's position of truth is `shared::sphere::SpherePos` (a unit vector); `map.rs::sync_sphere_transforms` derives each entity's `Transform` from it every frame. Never move entities by editing `Transform` directly — mutate `SpherePos` and let the sync system place them.
- The planet mesh is built in `map.rs::build_planet_mesh`: an icosphere with per-vertex colors sampled from `shared::terrain::TerrainGen` (seed `PLANET_SEED`). Terrain is visual-only; the material is `base_color: WHITE` so vertex colors show. Bump `ico()` subdivision for finer biome edges.
- Survivor controls: W/S move forward/back along a persistent `Survivor.heading`; A/D rotate the heading (turn in place). The heading is a tangent vector that is re-projected onto the surface and parallel-transported each step so straight walking never flips (regression covered by `sphere::tests::heading_parallel_transport_never_flips`). Do NOT recompute movement direction from `tangent_basis()` per frame — its reference axis flips near latitude bands.
- The survivor mesh is oriented in `camera_follow` (not the generic sync system, which excludes `Survivor`). The sun (`Sun` marker `DirectionalLight`) is repositioned/aimed at the player each frame so the player's hemisphere is always lit.
- Movement/pathing/targeting/magnet all use geodesic ops and arc-length distance from `shared::sphere`. Ranges/radii stay in world units (arc length). Zombies spawn on a ring (`SPAWN_RADIUS`) around the survivor via `ring_point`; loot scatters with `random_point`.
- Shared mesh/material handles live in `map::GameAssets` (zombie/projectile) and `loot::LootAssets` (per-material/weapon colors) so spawners don't rebuild assets per entity. `drop_zombie_loot` takes `&LootAssets`.
- Core loop: materials (`Metal/Wood/Rope/Cloth`) and weapons are scattered across the planet at wave start (`loot.rs::scatter_loot`), collected via the magnet, and either equipped (weapons) or spent on crafting recipes. Dead zombies also drop random loot (`loot.rs::drop_zombie_loot`, called from `combat.rs` kill sites).
- `minimap.rs` orthographically projects the hemisphere facing the survivor onto a circular panel (bottom-right, clears the crafting sidebar), rebuilt each frame. Its projection basis comes from the survivor's `heading` (player faces up), NOT `tangent_basis()` — the latter's reference axis flips and would flip the minimap.
- Weapons override the survivor's fire stats and lose durability per shot; a broken weapon auto-swaps to the next collected one.
- `loot::LootState` is the single resource holding materials, collected weapons, and the equipped weapon; reset it wholesale on prestige restart.
- Bevy limits systems to ~16 params — bundle related state into one resource rather than adding params.
- UI uses the shared `theme` palette and the bundled Monaspace Neon font (`assets/fonts/`, loaded once into the `UiFont` resource). Draw all UI colors/fonts from `shared::theme` + `UiFont`.

# Verification

`cargo check` and `cargo clippy` from workspace root.

# Child DOX Index
