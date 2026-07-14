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
- Visual mesh and physics collider use the **same** icosphere triangles (subdivision 4, ~5k tris). Bridge faces are raised to sea level in the terrain trimesh so bridge collision is seamless.
- Bridge planks are visual-only cuboids at constant radius, 200m spacing. No separate collider entities.
- Actors use Avian3d physics (`RigidBody`, `Collider`, `Forces`) with custom `RadialGravity` for spherical gravity. Position of truth is `Transform.translation` from the physics engine. Never move physics bodies by editing `Transform` or `SpherePos` directly.
- Player movement uses `Forces::linear_velocity_mut()` for instant velocity control (tangent override, radial preserved). Gravity applies force continuously in `physics.rs`.
- Player controls: W/S move along `Player.heading`; A/D rotate heading. Input is decoupled: `PlayerInput` resource collected in `Update` (`read_player_input`), consumed in `FixedUpdate` (`move_player`). Rotation locked, orientation set in `Update` (`orient_player`).
- The sun (`DirectionalLight` with `Sun` marker) follows the player so the player's hemisphere is always lit.
- Minimap uses a second `Camera3d` (`MinimapCamera`) rendering into a circular UI node. Camera tracks player `Transform.translation`. Entity dots drawn as UI overlay from `Transform.translation`. Settlement markers use `Transform` (spawned at sea-level planet radius).
- Shared mesh/material handles: `map::GameAssets` (zombie/projectile), `loot::LootAssets` (materials/weapons). `drop_zombie_loot` takes `&LootAssets`.
- Core loop: materials and weapons scattered at wave start, collected via magnet, equipped or crafted. Dead zombies drop loot.
- Weapons override player fire stats, lose durability per shot, auto-swap on break.
- `loot::LootState` is the single resource for materials, collected weapons, equipped weapon. Reset on prestige.
- Bevy 0.19 uses `Message` derive, `add_message()`, `MessageWriter`/`MessageReader` for events.
- UI uses shared `theme` palette and `UiFont` resource (Monaspace Neon font).

# Verification

`cargo check` and `cargo clippy` from workspace root.

# Child DOX Index
