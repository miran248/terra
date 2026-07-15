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
- `AssetCatalogPlugin` keeps the app in `AppState::Loading` until the four generated GLB catalogs in `assets/models/` and their dependencies resolve. Runtime scene and animation lookup uses stable names from `shared::art`; regenerate with `cargo run -p gen_assets` and verify with `cargo run -p gen_assets -- --check`.
- Systems that require resources created by `setup_map` must run only in `AppState::Playing`; they cannot execute during catalog loading.
- `water.rs` renders generator-baked, terrain-following `River` + `RiverSpring` + `RiverBank` face components plus a one-face buried land apron. Springs taper from just inside ground, the apron’s outer edge clips into terrain, and outlets share neighboring lake/ocean waterlines; a component must contain a `River` or `RiverSpring` face, while cliffs remain excluded so coastlines cannot be flooded by a river mouth.
- The main camera runs TAA, which requires depth + motion-vector prepasses. Any material extension that displaces vertices (e.g. `foliage.rs` wind sway) must supply a matching `prepass_vertex_shader` with identical displacement math (`assets/shaders/foliage.wgsl` ↔ `foliage_prepass.wgsl`), or main-pass fragments fail the depth test against un-displaced prepass depth and flicker black. Water is exempt: alpha-blended, so it never enters the prepass.
- Sea/lake water is a subdivided (`WATER_SUBDIV`) rest-flat mesh; `shaders/water.wgsl` displaces it radially with a wind-driven geometric swell (`swell_amp`/`swell_scale`, 0 for rivers) shaded by its analytic gradient, plus normal-perturbation chop, both drifting downwind from `Weather.wind` pushed per-frame by `update_water_wind`. Colliders are unaffected — swell is visual-only.
- Runtime consumes typed, face-oriented `LevelData` and performs no terrain-id decoding, tag-offset decoding, terrain clustering, pathfinding, or topology derivation. `PlanetMesh::face_at` remains the query bridge for arbitrary world positions; the terrain HUD chooses the nearest authoritative face-corner terrain identity so it matches mesh coloring instead of face-majority classification.
- `chunks.rs` streams the world: the render icosphere (subdivision 7, 327,680 faces) splits into 320 subdivision-2 chunks (`fi / faces_per_chunk`, contiguous slices — deterministic 4-way child order). Every chunk is always resident at LOD 1 (terrain/water/river/ice visuals) so the minimap/world-map cameras never see holes; LOD 2 (≤ 960 m) adds structures + large flora + water swell subdivision at LOD 3 (≤ 300 m) with all flora. 15% downgrade hysteresis, budgeted rebuilds, flora streamed `FLORA_PER_FRAME` per frame nearest-first. Chunk entities are tagged `Ground` for prestige cleanup.
- Physics never streams: terrain, ice, and bridge colliders are whole-planet trimeshes spawned in `setup_map` (no fall-through at chunk borders; world-map teleports always land on ground). Only flora/structure colliders live inside chunks — both spawn well beyond the 120 m zombie ring.
- The terrain **collider** uses the full-resolution icosphere triangles; chunked visuals may render coarser water far away but terrain geometry is identical at every LOD. Bridge faces are raised to sea level in the terrain trimesh so bridge collision is seamless.
- Bridge planks are visual-only cuboids at constant radius, 200m spacing. No separate collider entities.
- Non-bridge roads render as visual-only 4 m ribbons subdivided at roughly 4 m intervals, sampled against the displaced terrain at both edges, and vertex-colored from baked `RoadMaterial`. They never add or replace collision.
- Per-face `WaterPhase` replaces animated lake, river, or coherent polar/coastal ocean water with a collider-backed ice surface wherever local solved climate freezes it. Terrain identity and shore geometry remain unchanged, one water body may transition between frozen and liquid sections, and actors traverse actual ice surfaces with the same 0.4 movement multiplier used underwater. Separate `SurfaceCondition` metadata marks frozen ground of every terrain kind for asset generation. Ice and terrain colliders share the same collision margin so actors stand at the same visual height.
- Actors use Avian3d physics (`RigidBody`, `Collider`, `Forces`) with custom `RadialGravity` for spherical gravity. Read ordinary actor position from the physics-synchronized `Transform.translation`; deliberate teleports must update Avian `Position` and velocity. Never move physics bodies by editing `Transform` directly.
- Player movement uses `Forces::linear_velocity_mut()` for instant velocity control (tangent override, radial preserved). Gravity applies force continuously in `physics.rs`.
- Player controls: W/S move along `Player.heading`; A/D rotate heading. Input is decoupled: `PlayerInput` resource collected in `Update` (`read_player_input`), consumed in `FixedUpdate` (`move_player`). Physics-body rotation stays locked and Avian-owned; `orient_player` rotates only the render child in `Update`.
- Player fall-through diagnostics emit one warning when the player's center crosses beneath the generated terrain surface, including altitude, radial and total velocity, real-frame and fixed-step durations, and Avian contact state. World-map teleportation updates Avian `Position` and velocity directly so physics remains the position authority.
- The sun (`DirectionalLight` with `Sun` marker) follows the player so the player's hemisphere is always lit.
- Minimap retains its fixed, heading-up 2D presentation and flat marker projection; its cardinal labels rotate as a compass dial while remaining on the circular edge. The full-screen map uses a north-up perspective `Camera3d` globe with the same circular border and shared actor, loot, settlement, named geographic-region, and edge-cardinal overlays; it pans with left-button drag and ray-casts visible terrain when a left click is released without dragging. Named oceans, lakes, rivers, land biomes, ranges, coasts, towns, and roads use baked `LevelRegions` centroids. Mesh-backed object markers require `ViewVisibility`, and fullscreen markers on the globe's hidden side are excluded. Map UI projection must convert `ComputedNode` physical sizes to logical pixels so overlays remain aligned at every display scale.
- Shared visual handles: `map::GameAssets` owns the procedural projectile; `loot::LootAssets` owns material/weapon GLB scenes. Player, zombie, loot, flora, and structure visuals are named GLB scenes parented beneath runtime-owned placement/physics roots; imported scenes never own gameplay collision. `drop_zombie_loot` takes `&LootAssets`.
- Actor scenes bind to the shared catalog animation graph after instantiation. The player scene owns the named `socket.hand`; `sync_equipped_weapon` keeps its visual-only weapon child aligned with `LootState::equipped`.
- Core loop: materials and weapons scattered at wave start, collected via magnet, equipped or crafted. Dead zombies drop loot.
- Weapons override player fire stats, lose durability per shot, auto-swap on break.
- `loot::LootState` is the single resource for materials, collected weapons, equipped weapon. Reset on prestige.
- Bevy 0.19 uses `Message` derive, `add_message()`, `MessageWriter`/`MessageReader` for events.
- UI uses shared `theme` palette and `UiFont` resource (Monaspace Neon font).

# Verification

`cargo check` and `cargo clippy` from workspace root.

# Child DOX Index
