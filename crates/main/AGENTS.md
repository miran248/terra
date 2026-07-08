# Purpose

Binary crate for the rs-zombies Bevy application. Entry point, app bootstrap, and game systems live here.

# Ownership

- Owns `crates/main/` — binary, Bevy systems, rendering, game logic
- Depends on `shared` crate for common types and utilities

# Local Contracts

- Must remain a thin orchestration layer; reusable logic belongs in `shared`
- Bevy systems are registered in `main.rs` or modules under `src/`

# Work Guidance

- **Units are meters** (1 world unit = 1 m; see `shared::sphere::METER`). Keep all new distances/sizes/speeds in meters. Reference scale: planet radius 2000 m, player/zombie 2 m, mountains ±500 m, player speed 60 m/s, attack range ~50 m. Distance-valued upgrades (`TurretRange`, `ProjectileSpeed`, `MagnetRange`) and weapon ranges are in meters too.
- The player spawns on habitable land via `TerrainGen::habitable_spawn()` (deterministic per `PLANET_SEED`), not at a fixed pole. Future villages/roads/farms should be placed on `is_habitable` terrain.

- The world is a 3D planet (`Camera3d`, PBR meshes, `DirectionalLight`). Every actor's position of truth is `shared::sphere::SpherePos` (a unit vector); `map.rs::sync_sphere_transforms` derives each entity's `Transform` from it every frame via `TerrainGen::ground_world`, lifting the mesh by its `GroundOffset` (half-height) so it rests on the terrain and never dips below ground (over ocean it rests on the water surface). Every spawned actor needs a `GroundOffset`. Never move entities by editing `Transform` directly — mutate `SpherePos`.
- The planet mesh is built in `map.rs::build_planet_mesh`: an icosphere (`ico(79)`, near Bevy's subdivision cap) displaced by the terrain heightmap, with per-vertex colors from `shared::terrain::TerrainGen` (seed `PLANET_SEED`, inserted as a resource) and recomputed normals for lighting. Actors and camera all read `surface_world`/`surface_radius` so they sit on the displaced terrain, not the base sphere.
- Player controls: W/S move forward/back along a persistent `Player.heading`; A/D rotate the heading (turn in place). The heading is a tangent vector that is re-projected onto the surface and parallel-transported each step so straight walking never flips (regression covered by `sphere::tests::heading_parallel_transport_never_flips`). Do NOT recompute movement direction from `tangent_basis()` per frame — its reference axis flips near latitude bands.
- The player mesh is oriented in `camera_follow` (not the generic sync system, which excludes `Player`). The sun (`Sun` marker `DirectionalLight`) is repositioned/aimed at the player each frame so the player's hemisphere is always lit.
- Movement/pathing/targeting/magnet all use geodesic ops and arc-length distance from `shared::sphere`. Ranges/radii stay in world units (arc length). Zombies spawn on a ring (`SPAWN_RADIUS`) around the player via `ring_point`; loot scatters with `random_point`.
- Shared mesh/material handles live in `map::GameAssets` (zombie/projectile) and `loot::LootAssets` (per-material/weapon colors) so spawners don't rebuild assets per entity. `drop_zombie_loot` takes `&LootAssets`.
- Core loop: materials (`Metal/Wood/Rope/Cloth`) and weapons are scattered across the planet at wave start (`loot.rs::scatter_loot`), collected via the magnet, and either equipped (weapons) or spent on crafting recipes. Dead zombies also drop random loot (`loot.rs::drop_zombie_loot`, called from `combat.rs` kill sites).
- `minimap.rs` renders the real 3D world with a **second `Camera3d`** (`MinimapCamera`, `order: -1`, `Msaa::Off`) into a render-target `Image` shown in a circular UI node — terrain, roads, and settlement ground patches come from this camera. Actors are too small to see from that height, so player/zombies/loot are drawn as **UI dots** over the texture (`draw_overlay`), and named `Settlement` entities as square markers + name labels. Camera sits `CAM_HEIGHT` above the player looking down, rolled so heading points up (`track_minimap_camera`); a rotating compass overlays it.
- Settlements are real world entities: a `Settlement { name }` marker entity (position via `SpherePos`, no mesh) that the minimap reads, plus a separate flat orange ground patch for the 3D view. Roads/settlements are placed by `map.rs::spawn_road_geometry` as flat ground discs (via `ground_patch_transform`), not floating spheres. Any system touching "the camera" must pick `MainCamera` vs `MinimapCamera`, never bare `With<Camera3d>`.
- The planet mesh is low-poly (`ico(40)`, flat-shaded per-face) displaced by the terrain heightmap, with per-triangle biome colors. Because the mesh is faceted but the heightmap is smooth, actors AND road/settlement patches must rest on the *rendered facet* via `shared::planet::PlanetMesh::facet_radius` (grid-accelerated ray-vs-triangle) — `map.rs::grounded` / `ground_patch_transform`. `build_planet_mesh` returns the world triangles; they go into `PlanetMesh::new`. Do NOT ground on `terrain.surface_radius` (smooth heightmap — used only for the minimap relief and initial spawn).
- Weapons override the player's fire stats and lose durability per shot; a broken weapon auto-swaps to the next collected one.
- `loot::LootState` is the single resource holding materials, collected weapons, and the equipped weapon; reset it wholesale on prestige restart.
- Bevy limits systems to ~16 params — bundle related state into one resource rather than adding params.
- UI uses the shared `theme` palette and the bundled Monaspace Neon font (`assets/fonts/`, loaded once into the `UiFont` resource). Draw all UI colors/fonts from `shared::theme` + `UiFont`.

# Verification

`cargo check` and `cargo clippy` from workspace root.

# Child DOX Index
