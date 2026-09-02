# Runtime contracts

## World and offline boundary

- Units are meters. Reference scale: planet radius 2000 m, actors 2 m, mountains ±500 m, player speed 60 m/s, attack range about 50 m.
- `TerrainGen::habitable_spawn()` chooses a deterministic land spawn from `PLANET_SEED`.
- The world is a 3D planet (`Camera3d`, PBR meshes, `DirectionalLight`). `gen_level` precomputes planet mesh, collision, roads, settlements, and typed face data into tracked `assets/level_{seed}.bin`, embedded at compile time with `include_bytes!` and deserialized with Postcard.
- Runtime consumes typed, face-oriented `LevelData`. It performs no terrain-id/tag-offset decoding, clustering, pathfinding, topology derivation, or generation policy. `PlanetMesh::face_at` bridges arbitrary positions; the HUD uses the nearest authoritative face-corner terrain identity.
- `AssetCatalogPlugin` holds `AppState::Loading` until all four GLB catalogs and dependencies resolve. Systems requiring `setup_map` resources run only in `AppState::Playing`.

## Rendering and water

- `water.rs` renders generator-baked `River`, `RiverSpring`, and `RiverBank` face components plus a buried one-face land apron. Springs taper from inside ground, the apron clips into terrain, outlets share neighboring waterlines, each component contains a river/spring face, and cliffs stay excluded.
- TAA requires depth and motion-vector prepasses. Any vertex-displacing material must provide a matching `prepass_vertex_shader` with identical math (`foliage.wgsl` and `foliage_prepass.wgsl`). Alpha-blended water does not enter the prepass.
- `WATER_SUBDIV` controls sea/lake mesh subdivision. Sea/lake water starts from a rest-flat mesh. In `water.wgsl`, `swell_amp` controls geometric-swell amplitude (`0` disables it for rivers), while `swell_scale` controls spatial frequency; geometric swell is shaded with its analytic gradient so fragment lighting follows the displaced crests. Both swell and normal chop drift downwind from per-frame `Weather.wind`, pushed by `update_water_wind`; colliders remain undisplaced.
- Per-face `WaterPhase` replaces frozen liquid sections with collider-backed ice without changing terrain identity or shore geometry. One body may mix frozen/liquid sections. Actors traversing frozen water use the same `0.4` movement multiplier as underwater movement. `SurfaceCondition` independently marks frozen ground. Ice and terrain use the same collision margin.

## Chunking, geometry, and collision

- Subdivision-7 terrain (327,680 faces) splits into 320 subdivision-2 chunks using contiguous deterministic four-child slices.
- LOD 1 is always resident for terrain/water/river/ice and map cameras. LOD 2 at edge distance ≤960 m adds structures and large flora; LOD 3 at ≤300 m adds small flora and subdivided water swell.
- Distance is camera to chunk edge (centroid distance minus radius). Transitions are incremental: static meshes build once, water rebuilds only on subdivision change, and structures/flora apply deltas. Downgrades use 15% hysteresis; transitions are budgeted and flora streams nearest-first under `FLORA_PER_FRAME`.
- Chunk entities carry `Ground` for prestige cleanup. `DEBUG_CHUNK_BORDERS` is diagnostic-only and must be off for shipping.
- Physics never streams. Whole-planet terrain, ice, and bridge trimeshes spawn in `setup_map`; flora/structure colliders live in chunks well beyond the 120 m zombie ring.
- Terrain collision always uses full-resolution triangles. Bridge faces are raised to sea level in that trimesh. Bridge planks are visual-only cuboids at constant radius and 200 m spacing; there are no separate bridge collider entities.
- Non-bridge roads are visual-only 4 m ribbons subdivided at roughly 4 m intervals, sampled against displaced terrain at both edges, and vertex-colored from baked `RoadMaterial`; they never replace or add collision.

## Physics and input

- Actors use Avian3d `RigidBody`, `Collider`, and `Forces` plus custom `RadialGravity`. Read normal position from the physics-synchronized `Transform.translation`. Teleports update Avian `Position` and velocity; never author motion by editing `Transform`.
- Player movement overrides tangential velocity through `Forces::linear_velocity_mut()` while preserving radial velocity; gravity applies force continuously.
- `PlayerInput` is collected in `Update` and consumed by `move_player` in `FixedUpdate`. W/S move along heading, A/D rotate heading. Physics-body rotation remains locked and Avian-owned; `orient_player` rotates only the render child.
- Fall-through diagnostics emit one warning after crossing beneath generated terrain, including altitude, radial/total velocity, frame/fixed-step durations, and Avian contact state.
- The `Sun` directional light follows the player so the current hemisphere stays lit.

## Maps, visuals, and gameplay state

- Minimap is fixed heading-up 2D with a rotating edge compass. Minimap markers use flat projection. Fullscreen map is a north-up perspective `Camera3d` globe with the same circular border and shared actor, loot, settlement, named-region, and edge-cardinal overlays; it supports pan/drag and release-without-drag terrain ray casting.
- Baked `LevelRegions` coverage includes named oceans, lakes, rivers, land biomes, ranges, coasts, towns, and roads; markers use region centroids. Mesh-backed markers require `ViewVisibility`; hidden-side globe markers are excluded. Convert `ComputedNode` physical sizes to logical pixels for overlay projection.
- `map::GameAssets` owns the procedural projectile. `loot::LootAssets` owns material/weapon GLB scenes. Player, zombie, loot, flora, and structure scenes are visual children of runtime-owned placement/physics roots; imported scenes never own gameplay collision.
- Actor scenes bind to the shared animation graph after instantiation. The player scene owns `socket.hand`; `sync_equipped_weapon` mirrors `LootState::equipped` with a visual-only child.
- `LootState` is the single resource for materials, collected weapons, and equipped weapon; reset it on prestige. Weapons override fire stats, lose durability per shot, and auto-swap on break. Dead zombies drop loot; wave-start loot supports magnet collection, equipping, and crafting.
- Bevy 0.19 events use `Message`, `add_message()`, `MessageWriter`, and `MessageReader`.
- UI uses the shared theme palette and bundled Monaspace Neon `UiFont`.

## Verification

Run the focused test first, then workspace `cargo check` and `cargo clippy`. If GLB catalogs or embedded `LevelData` are affected, follow `terra-assets` generation and deterministic comparison checks.
