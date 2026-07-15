# Purpose

Binary crate for the rs-zombies Bevy application. Entry point, app bootstrap, and game systems live here.

# Ownership

- Owns `crates/main/` — binary, Bevy systems, rendering, game logic
- Depends on `shared` crate for common types and utilities

# Local Contracts

- Must remain a thin orchestration layer; reusable logic belongs in `shared`
- Bevy systems are registered in `main.rs` or modules under `src/`

# Work Guidance

- **Units are meters** (1 world unit = 1 m; see `shared::sphere::METER`). Keep all new distances/sizes/speeds in meters. Reference scale: planet radius 2000 m, player/zombie 2 m, mountains ±500 m, player speed 60 m/s, attack range ~50 m.
- The player spawns on habitable land via `TerrainGen::habitable_spawn()` (deterministic per `PLANET_SEED`).
- The world is a 3D planet (`Camera3d`, PBR meshes, `DirectionalLight`). The planet mesh, collision data, roads, and settlements are precomputed by `gen_level` into `assets/level_{seed}.bin` and loaded via `include_bytes!` + `postcard`. Run `cargo run -p gen_level -- crates/main/assets/level_{seed}.bin` to regenerate.
- `water.rs` renders generator-baked, terrain-following `River` + `RiverSpring` + `RiverBank` face components plus a one-face buried land apron. Springs taper from just inside ground, the apron’s outer edge clips into terrain, and outlets share neighboring lake/ocean waterlines; a component must contain a `River` or `RiverSpring` face, while cliffs remain excluded so coastlines cannot be flooded by a river mouth.
- Runtime consumes typed, face-oriented `LevelData` and performs no terrain-id decoding, tag-offset decoding, terrain clustering, pathfinding, or topology derivation. `PlanetMesh::face_at` remains the query bridge for arbitrary world positions; the terrain HUD chooses the nearest authoritative face-corner terrain identity so it matches mesh coloring instead of face-majority classification.
- Visual mesh and physics collider use the **same** icosphere triangles (subdivision 4, ~5k tris). Bridge faces are raised to sea level in the terrain trimesh so bridge collision is seamless.
- Bridge planks are visual-only cuboids at constant radius, 200m spacing. No separate collider entities.
- Per-face `WaterPhase` replaces animated lake, river, or coherent polar/coastal ocean water with a collider-backed ice surface wherever local solved climate freezes it. Terrain identity and shore geometry remain unchanged, one water body may transition between frozen and liquid sections, and actors traverse actual ice surfaces with the same 0.4 movement multiplier used underwater. Separate `SurfaceCondition` metadata marks frozen ground of every terrain kind for asset generation. Ice and terrain colliders share the same collision margin so actors stand at the same visual height.
- Actors use Avian3d physics (`RigidBody`, `Collider`, `Forces`) with custom `RadialGravity` for spherical gravity. Read ordinary actor position from the physics-synchronized `Transform.translation`; deliberate teleports must update Avian `Position` and velocity. Never move physics bodies by editing `Transform` directly.
- Player movement uses `Forces::linear_velocity_mut()` for instant velocity control (tangent override, radial preserved). Gravity applies force continuously in `physics.rs`.
- Player controls: W/S move along `Player.heading`; A/D rotate heading. Input is decoupled: `PlayerInput` resource collected in `Update` (`read_player_input`), consumed in `FixedUpdate` (`move_player`). Physics-body rotation stays locked and Avian-owned; `orient_player` rotates only the render child in `Update`.
- Player fall-through diagnostics emit one warning when the player's center crosses beneath the generated terrain surface, including altitude, radial and total velocity, real-frame and fixed-step durations, and Avian contact state. World-map teleportation updates Avian `Position` and velocity directly so physics remains the position authority.
- The sun (`DirectionalLight` with `Sun` marker) follows the player so the player's hemisphere is always lit.
- Minimap retains its fixed, heading-up 2D presentation and flat marker projection; its cardinal labels rotate as a compass dial while remaining on the circular edge. The full-screen map uses a north-up perspective `Camera3d` globe with the same circular border and shared actor, loot, settlement, named geographic-region, and edge-cardinal overlays; it pans with left-button drag and ray-casts visible terrain when a left click is released without dragging. Named oceans, lakes, rivers, land biomes, ranges, coasts, towns, and roads use baked `LevelRegions` centroids. Mesh-backed object markers require `ViewVisibility`, and fullscreen markers on the globe's hidden side are excluded. Map UI projection must convert `ComputedNode` physical sizes to logical pixels so overlays remain aligned at every display scale.
- Shared mesh/material handles: `map::GameAssets` (zombie/projectile), `loot::LootAssets` (materials/weapons). `drop_zombie_loot` takes `&LootAssets`.
- Core loop: materials and weapons scattered at wave start, collected via magnet, equipped or crafted. Dead zombies drop loot.
- Weapons override player fire stats, lose durability per shot, auto-swap on break.
- `loot::LootState` is the single resource for materials, collected weapons, equipped weapon. Reset on prestige.
- Bevy 0.19 uses `Message` derive, `add_message()`, `MessageWriter`/`MessageReader` for events.
- UI uses shared `theme` palette and `UiFont` resource (Monaspace Neon font).

# Verification

`cargo check` and `cargo clippy` from workspace root.

# Child DOX Index
