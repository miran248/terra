# Runtime contracts

## World and offline boundary

- Units are meters; canonical world scale lives in `shared::sphere`.
- The player spawns at the first settlement in the baked `LevelData`.
- The world is a 3D planet (`Camera3d`, PBR meshes, `DirectionalLight`). `gen_level` precomputes planet mesh, collision, roads, settlements, and typed face data into local generated `assets/level_{seed}.bin` files, embedded at compile time with `include_bytes!`. Each artifact carries the `TERA` magic and schema version 1 before its Postcard payload. Missing or unsupported headers are rejected; regenerate seed 1337 before building from a clean checkout. These generated binaries are ignored by Git.
- Runtime consumes typed, face-oriented `LevelData`. Mesh generation, region clustering, and route planning remain offline. Startup reconstructs the terrain query grid, coarse zones, and climate from the seed and baked elevation field through `TerrainGen::from_field`; this reconstruction remains intentional for now. `PlanetMesh::face_at` bridges arbitrary positions; the HUD uses the nearest authoritative face-corner terrain identity.
- `AssetCatalogPlugin` holds `AppState::Loading` until all four GLB catalogs and dependencies resolve. Systems requiring `setup_map` resources run only in `AppState::Playing`.

## Rendering and water

- `water.rs` renders generator-baked `River`, `RiverSpring`, and `RiverBank` face components plus a buried one-face land apron. Springs taper from inside ground, the apron clips into terrain, outlets share neighboring waterlines, each component contains a river/spring face, and cliffs stay excluded.
- `shader_motion.rs` uploads wind/player inputs and derived wind factors to one fixed-size `ShaderBuffer`, shared by water and both foliage passes at material binding 101. Keep its Rust layout aligned with the shared `shader_motion.wgsl` import and size/usage fixed so Bevy reuses the GPU allocation. Dynamic updates must not mutate material assets: their modification events trigger per-instance pipeline specialization. Foliage skips anchored vertices and tests squared trample distance before taking a square root.
- TAA requires depth and motion-vector prepasses. Any vertex-displacing material must provide a matching `prepass_vertex_shader` with identical math (`foliage.wgsl` and `foliage_prepass.wgsl`). Alpha-blended water does not enter the prepass.
- `WATER_SUBDIV` controls sea/lake mesh subdivision. Sea/lake water starts from a rest-flat mesh. In `water.wgsl`, `swell_amp` controls geometric-swell amplitude (`0` disables it for rivers), while `swell_scale` controls spatial frequency; geometric swell is shaded with its analytic gradient so fragment lighting follows the displaced crests. Both swell and normal chop drift downwind from per-frame `Weather.wind`, uploaded by `ShaderMotionPlugin`; colliders remain undisplaced.
- Per-face `WaterPhase` replaces frozen liquid sections with collider-backed ice without changing terrain identity or shore geometry. One body may mix frozen/liquid sections. Actors traversing frozen water use the same `0.4` movement multiplier as underwater movement. `SurfaceCondition` independently marks frozen ground. Ice and terrain use the same collision margin.

## Chunking, geometry, and collision

- Subdivision-7 terrain (327,680 faces) splits into 320 subdivision-2 chunks using contiguous deterministic four-child slices.
- LOD 1 is always resident for terrain/water/river/ice and map cameras. LOD 2 at edge distance ≤960 m adds structures and large scenery; LOD 3 at ≤300 m adds small scenery and subdivided water swell.
- Distance is camera to chunk edge (centroid distance minus radius). Transitions are incremental: static meshes build once, water rebuilds only on subdivision change, and structures/scenery apply deltas. Downgrades use 15% hysteresis; transitions are budgeted and scenery streams nearest-first under `SCENERY_PER_FRAME`.
- Chunk entities carry `Ground` for prestige cleanup. `DEBUG_CHUNK_BORDERS` is diagnostic-only and must be off for shipping.
- Physics never streams. Whole-planet terrain, ice, and bridge trimeshes spawn in `setup_map`; scenery/structure colliders live in chunks well beyond the 120 m zombie ring. `setup_map` constructs terrain, ice, bridge, and player colliders directly before the first fixed physics step. Avian resolves deferred `ColliderConstructor`s in `Update`, after that frame's fixed-step loop; with a 155 ms seed-1337 state-entry frame, that race reproduced the player 0.252 m below terrain with no contacts.
- Terrain collision uses full-resolution displaced triangles. Bridge decks are built from baked spans and have separate static colliders matching their visual geometry.
- Non-bridge roads are visual-only 4 m ribbons subdivided at roughly 4 m intervals, sampled against displaced terrain at both edges, and vertex-colored from baked `RoadMaterial`; they never replace or add collision.

## Physics and input

- Actors use Avian3d `RigidBody`, `Collider`, and `Forces` plus custom `RadialGravity`. Read normal position from the physics-synchronized `Transform.translation`. Teleports update Avian `Position` and velocity; never author motion by editing `Transform`.
- Player movement overrides tangential velocity through `Forces::linear_velocity_mut()` while preserving radial velocity; gravity applies force continuously.
- `PlayerInput` is collected in `Update` and consumed by `move_player` in `FixedUpdate`. W/S move along heading, A/D rotate heading. Physics-body rotation remains locked and Avian-owned; `orient_player` rotates only the render child.
- Fall-through diagnostics emit one warning after crossing beneath generated terrain, including altitude, radial/total velocity, frame/fixed-step durations, and Avian contact state.
- The sun orbits the polar axis for the day/night cycle. Sun-lock holds it overhead the player and freezes time progression.

## Maps, visuals, and gameplay state

- Minimap is fixed heading-up 2D with a rotating edge compass. Minimap markers use flat projection. Fullscreen map is a north-up perspective `Camera3d` globe with the same circular border and shared actor, loot, settlement, named-region, and edge-cardinal overlays; it supports pan/drag and release-without-drag ray casting to the nearest visible terrain, sea-level water, or bridge deck. A bridge deck receives clicks only when its top surface is in front of terrain along the camera ray. Region labels annotate centroids, vertically centered immediately to the right of their markers. Labels stay on one line and truncate at the circular boundary; they never shift away from their markers or use leader lines. Blank map clicks still teleport.
- Baked `LevelRegions` coverage includes named oceans, lakes, rivers, land biomes, ranges, coasts, settlements, and roads. Both map views keep markers for every visible kind; settlement names stay visible, while other region names appear for the nearest marker hovered within 12 logical pixels inside the circular map content. Bridge markers use area-weighted centers of generated deck top surfaces; the globe projects those centers at deck height, and bridge names use the same nearest-marker hover. The cursor is measured from the inner image area, past the 3 px border. While the fullscreen map is open, it owns label hover and the covered minimap stays quiet; its overlays follow camera pans each frame, while the closed minimap overlay refreshes less often. The minimap omits centers outside its viewport. Mesh-backed markers require `ViewVisibility`; hidden-side globe markers are excluded. Convert `ComputedNode` physical sizes to logical pixels for overlay projection.
- The location HUD reads every region ID from the exact queried face and groups those memberships into geography, settlement name/kind, and roads. It never unions adjacent faces. Named bridges are queried separately against the actual deck top surfaces used for rendering and collision; standing below a bridge does not show its name. Long rows use display-only ellipsis; the region and membership data stay complete.
- `map::GameAssets` owns the procedural projectile. `loot::LootAssets` owns material/weapon GLB scenes. Player, zombie, loot, scenery, and structure scenes are visual children of runtime-owned placement/physics roots; imported scenes never own gameplay collision.
- Production actor scenes bind to the catalog animation graph after instantiation. Opt-in candidate actors carry `shared::actor_animation::ActorPlayback`, which selects their own GLB clips and crossfades visual actions; they are excluded from baseline binding. `just asset-showcase` places these review actors on the planet without changing the production catalog. The player scene owns `socket.hand`; `sync_equipped_weapon` mirrors `LootState::equipped` with a visual-only child.
- `LootState` is the single resource for materials, collected weapons, and equipped weapon; reset it on prestige. Weapons override fire stats, lose durability per shot, and auto-swap on break. Dead zombies drop loot; wave-start loot supports magnet collection and equipping. Crafting UI and handlers are currently dormant; restoring crafting is deferred.
- Bevy 0.19 events use `Message`, `add_message()`, `MessageWriter`, and `MessageReader`.
- UI uses the shared theme palette and bundled Monaspace Neon `UiFont`.

## Verification

Run the focused test first, then workspace `cargo check` and `cargo clippy`. If GLB catalogs or embedded `LevelData` are affected, follow the [GLB pipeline](glb-pipeline.md) or [level pipeline](level-pipeline.md) generation and deterministic comparison checks.
