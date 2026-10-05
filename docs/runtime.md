# Runtime contracts

## World and offline boundary

- Units are meters; canonical world scale lives in `shared::sphere`.
- The player spawns at the first settlement in the baked `LevelData`.
- The world is a 3D planet (`Camera3d`, PBR meshes, `DirectionalLight`). `gen_level` precomputes planet mesh, collision, roads, settlements, and typed face data into local generated `assets/level_{seed}.bin` files, embedded at compile time with `include_bytes!`. Each artifact carries the `TERA` magic and schema version 1 before its Postcard payload. Missing or unsupported headers are rejected; regenerate seed 1337 before building from a clean checkout. These generated binaries are ignored by Git.
- Runtime consumes typed, face-oriented `LevelData`. Mesh generation, region clustering, and route planning remain offline. Startup reconstructs the terrain query grid, coarse zones, and climate from the seed and baked elevation field through `TerrainGen::from_field`; this reconstruction remains intentional for now. `PlanetMesh::face_at` bridges arbitrary positions; the HUD uses the nearest authoritative face-corner terrain identity.
- `AssetCatalogPlugin` holds `AppState::Loading` until all 68 production GLB scenes and dependencies resolve. Canonical scene names map to individual `models/production/<scene>.glb` files; each actor binds its own clips. Systems requiring `setup_map` resources run only in `AppState::Playing`.

## Rendering and water

- `water.rs` renders generator-baked `River`, `RiverSpring`, and `RiverBank` face components plus a buried one-face land apron. Springs taper from inside ground, the apron clips into terrain, outlets share neighboring waterlines, each component contains a river/spring face, and cliffs stay excluded.
- `shader_motion.rs` uploads wind/player inputs and derived wind factors to one fixed-size `ShaderBuffer`, shared by water and both foliage passes at material binding 101. Keep its Rust layout aligned with the shared `shader_motion.wgsl` import and size/usage fixed so Bevy reuses the GPU allocation. Dynamic updates must not mutate material assets: their modification events trigger per-instance pipeline specialization. Foliage skips anchored vertices and tests squared trample distance before taking a square root.
- TAA requires depth and motion-vector prepasses. Any vertex-displacing material must provide a matching `prepass_vertex_shader` with identical math (`foliage.wgsl` and `foliage_prepass.wgsl`). Alpha-blended water does not enter the prepass.
- `WATER_SUBDIV` controls sea/lake mesh subdivision. Sea/lake water starts from a rest-flat mesh. In `water.wgsl`, `swell_amp` controls geometric-swell amplitude (`0` disables it for rivers), while `swell_scale` controls spatial frequency; geometric swell is shaded with its analytic gradient so fragment lighting follows the displaced crests. Both swell and normal chop drift downwind from per-frame `Weather.wind`, uploaded by `ShaderMotionPlugin`; colliders remain undisplaced.
- Per-face `WaterPhase` replaces frozen liquid sections with collider-backed ice without changing terrain identity or shore geometry. One body may mix frozen/liquid sections. Actors traversing frozen water use the same `0.4` movement multiplier as underwater movement. `SurfaceCondition` independently marks frozen ground. Ice and terrain use the same collision margin.

Opt-in lighting diagnosis (`asset-review` plus `TERRA_LIGHTING_CAPTURE`) freezes
five baked-level camera scenes for noon/sunset/night captures and isolated material
or lighting overrides. See [the baseline evidence and reproduction commands](lighting-diagnostics.md).
Normal gameplay does not include this fixture.

## Simulation and presentation clocks

- `PlanetSimulationClock` owns virtual-time rate writes. After the main camera has updated, it publishes the camera's attained radial distance; before Bevy's next `TimeSystems` advance, the arbiter applies `base_rate * (1 - 0.5 * (z*z*(3-2*z)))`, where `z` is the attained radial zoom clamped from settlement scale (`0`) to whole-planet scale (`1`). Requested framing does not affect the rate until the camera reaches it. Closing Planet view removes only that contribution; base-rate changes and the independent virtual-time pause remain intact.
- World movement, Avian physics, day/night, weather, water and foliage shader time, and vehicle animation follow virtual time together. Do not scale their deltas separately. Bevy's fixed accumulator continues to consume virtual delta at its configured timestep and substep count. Render globals receive the restored generic virtual `Time`, so shader animation follows the same simulation progression.
- Camera motion, camera gestures, interface fades and readiness, timed holds, and camera-dependent presentation refresh use `Time<Real>` so they remain responsive while virtual time is slowed or paused.

## Planet view atmosphere

- Global ambient day/night fill follows the controlled body's position relative to the world sun. Distance-fog color follows the camera's hemisphere, so orbiting changes the viewed haze without relighting the whole world. Fog visibility and Bevy aerial-perspective reach blend smoothly from ground values to overview values using camera altitude above the nominal sphere.
- The current profile starts at 1.7 km fog visibility and 2.5 km aerial reach, blending to 100 km and 5 km by the 4 km overview altitude. These are provisional implementation values; matched ascent/descent captures in the #57 rendering matrix must tune and visually accept the combined fog and atmosphere response.
- The atmosphere shell outer radius is 2.9 km. Planet view uses a 10 km far clip, which covers the far-side atmosphere from the 6 km overview camera. The camera restores its prior far plane after return so ground depth precision stays at the normal projection range.

## Chunking, geometry, and collision

- Subdivision-7 terrain (327,680 faces) splits into 320 subdivision-2 chunks using contiguous deterministic four-child slices.
- LOD 1 is built across the planet first and remains resident for terrain/water/river/ice regardless of camera view. LOD 2 adds structures and regional scenery within the greater of 960 m or the camera's radial horizon; LOD 3 adds small scenery and subdivided water swell within a 300 m footprint that shrinks as the camera rises above local terrain.
- Distance is radial surface distance to the chunk edge (centroid arc minus chunk radius). Nearest chunks upgrade under an eight-transition frame budget, with 15% downgrade hysteresis. Structure and scenery additions/removals share a 256-root update budget, with at most 128 removals. Removals start farthest away; additions prioritize nearby structures, then regional scenery, then local ground cover. Reversing the camera changes the target for remaining work without queuing obsolete transitions. Prop visibility refreshes round-robin in batches of 2,048 every 0.2 seconds of real time.
- Chunk entities carry `Ground` for prestige cleanup. `DEBUG_CHUNK_BORDERS` is diagnostic-only and must be off for shipping.
- Terrain physics never streams. Whole-planet terrain, ice, and bridge trimeshes spawn in `setup_map`; scenery/structure colliders are independently resident around explorer/vehicle bodies and pending placement destinations, including a speed-scaled lookahead. Render chunk downgrades cannot remove their support. `setup_map` constructs terrain, ice, bridge, and player colliders directly before the first fixed physics step. Avian resolves deferred `ColliderConstructor`s in `Update`, after that frame's fixed-step loop; with a 155 ms seed-1337 state-entry frame, that race reproduced the player 0.252 m below terrain with no contacts.
- Terrain collision uses full-resolution displaced triangles. Terrain, ice, and bridge trimeshes weld shared corners and enable internal-edge correction so adjacent faces do not create spurious obstacle contacts. Bridge decks are built from baked spans and have separate static colliders matching their visual geometry.
- Non-bridge roads are visual-only 4 m ribbons subdivided at roughly 4 m intervals, sampled against displaced terrain at both edges, and vertex-colored from baked `RoadMaterial`; they never replace or add collision.

## Physics and input

- Default exploration uses separate explorer, car and plane physics bodies. `exploration::Exploration` owns occupancy, selection, recovery history and the action queue. The existing `Player` stays the explorer identity and mirrors an occupied vehicle's physics position for HUD, minimap and environmental consumers; only the occupied vehicle receives driving input. Seated explorer collision/gravity are disabled. Reusable handling math lives in `shared`.
- On foot: 5/10 m/s walk/sprint, preserved 0.4 water/ice multiplier and 14 m/s jump increment. Car and plane use the accepted prototype handling, including car crest adhesion, plane loops/rolls/stalls, held thrust and climb/dive speed exchange. Prototype runtime plugins are test fixtures only, not alternate game modes.
- V opens a paused selector (C car, P plane; Escape/V cancel). E enters/exits a highlighted vehicle within 3 m of its collision surface, below 0.5 m/s with 0.25 seconds of stable support. Normal exit parks the vehicle; failed actions change nothing. A one-second R hold recovers the explorer on foot, or restores and relocates the occupied vehicle while keeping the explorer seated; release cancels. Vehicle recovery searches current, last-safe and start-area destinations using that vehicle’s footprint and takeoff-run checks, and changes nothing if all fail. No automatic crash reset or airborne ordinary exit.
- Summoning reuses one instance per kind on clear dry ground within 30 m (car) / 100 m (plane). Plane placement validates a takeoff corridor with dense support probes and swept body collision. A summon checks at most 32 candidate poses per ready update and at most two complete candidate sweeps per request across movement-triggered cursor restarts. It keeps its action queued and any existing vehicle unchanged while searching, then commits only after a live safe fit or full search exhaustion. If repeated movement consumes the total work allowance, the summon fails with feedback so later queued actions can proceed. Each pose rechecks current support, water and collision; the cursor restarts when the world epoch or reusable vehicle changes, or the explorer moves more than 12.5% of the candidate radius (capped at 3 m). Explicit cancellation discards the cursor, including before a same-kind request. Requests wait for independently resident obstacle colliders, including distant recovery or teleport destinations. `WorldObstacle` distinguishes scenery/structures from landable support despite the broad `Ground` cleanup tag.
- A single chase-camera owner blends accepted mode profiles, retracts immediately for solid obstruction, eases release, and snaps on recovery. Controlled visuals fade near the camera; nearby enterable vehicles receive an instance-local highlight. Imported vehicle rigs animate wheels/steering and the propeller without owning collision.

- Actors use Avian3d `RigidBody`, `Collider`, and `Forces` plus custom `RadialGravity`. Ordinary body and render consumers read normal position from the physics-synchronized `Transform.translation`. Camera and overlay targeting that needs the current controlled occupied-body position reads Avian `Position`; a seated `Player` proxy `Transform` can lag behind its occupied vehicle. Teleports update Avian `Position` and velocity; never author motion by editing `Transform`.
- Player movement overrides tangential velocity through `Forces::linear_velocity_mut()` while preserving radial velocity; gravity applies force continuously. The approved 1 m actor capsule uses a body-center origin; its ground-pivot visual is offset down by half its height, at scale one. Terrain margin is .02 m and swept CCD remains enabled.
- `PlayerInput` is collected in `Update` and consumed by `move_player` in `FixedUpdate`. W/S move along heading, A/D rotate heading. Physics torque remains locked. `RadialUpright` updates Avian `Rotation` in the fixed schedule so the capsule follows local up; `orient_player` computes render-child facing relative to that body rotation. The visual selects idle/walk from existing movement input without changing movement or damage rules.
- Fall-through diagnostics query the actual displaced collision triangles retained in `CollisionTerrain`, and emit one warning after crossing beneath that surface, including altitude, radial/total velocity, frame/fixed-step durations, and Avian contact state.
- The sun orbits the polar axis for the day/night cycle. Sun-lock holds it overhead the player and freezes time progression.

Cars align smoothly to ground sampled across their footprint in pitch and roll,
rather than adopting a single terrain facet, and retain their attitude through
brief losses of contact. Driving uses touching terrain normals before ray-probed
support so the car does not push into the next facet at a downhill-to-flat junction.
Sub-milliradian alignment noise does not wake resting bodies. Plane landing support includes
the collision envelope so a gentle first contact can transition to ground handling.

## Maps, visuals, and gameplay state

- Planet view is an animated pullback of the main gameplay camera. `shared::planet_view::PlanetViewCamera` owns reusable framing and transition policy; `exploration::view::camera` remains the sole main-camera transform writer and constrains each movement segment against terrain, ice, bridges and resident physics colliders. It also uses bounds-filtered, read-only sweeps over baked structure and scenery colliders so distant nonresident obstacles do not clip the camera. Those camera queries do not change body- and pending-destination-based physics residency. At minimum zoom, framing stays at least 400 m above the nominal planet radius, with terrain and obstacle clearance able to move it farther out. Follow starts enabled; once recentering finishes, active follow tracks the controlled body's radial direction without positional lag. Re-enabling follow and abrupt relocations recenter smoothly at the requested zoom. Press `M` or `Esc` to return toward the current controlled body. The ordinary heading-up minimap stays visible during exploration and hides while Planet view is open or transitioning; no separate full-map camera, image target or veil remains.
- Follow uses the controlled body's position and tangent heading without vehicle bank or pitch. Dragging detaches; wheel zoom preserves follow. Entry, return, follow handoff and recovery preserve attained camera continuity. Recovery leaves Planet view open, retargeting follow or retaining detached framing. Camera travel, interface fades and hold durations use real time; gameplay movement continues under virtual time.
- Planet view gestures distinguish logical-pixel drags from short clicks and capture interface presses before world picking. The same full-height left sidebar remains visible in gameplay and Planet view, with a continuous faint background and one scroll area for Environment, Location, Movement, View, Context, and Actions. Environment and Location continue to describe the explorer while browsing; Context labels the selected destination. Text actions open or close Planet view (`M`), toggle follow while the view is open (`F`), enter or exit a vehicle (`E`), summon a vehicle (`V`), and confirm a selected destination teleport (`T`). They use the keyboard routes' state transitions and eligibility checks. Summon opens the same paused Car/Plane/Cancel selector as `V` and `C`/`P`/Escape; choosing or canceling resumes gameplay, while Planet view phases and occupied vehicles exclude summoning. Recover can be held by mouse or `R` for one real-time second, with progress in Context; early release or lost pointer capture cancels the hold. A subtle accent tint marks hovered action text. Context and action rows remain reachable by scrolling; wheel input over the sidebar scrolls it without zooming the planet. The sidebar draws above map markers and captures presses over its full area, so action clicks cannot pick or orbit the scene behind it even while Planet view is closed. Destination picking remains available outside the sidebar. The minimap remains a separate graphical overlay and follows its existing gameplay visibility fade. An already-open selector retains input priority over map input. Selection and `T` require a fully visible interface; camera control remains available during travel.
- A selected `PlanetDestination` is a typed, world-epoch-scoped snapshot. Named marker hits take priority over main-camera ground/bridge picking; empty sky preserves selection. Selection does not prepare collision. `T` snapshots one request and enters checking feedback; the existing placement boundary waits for destination collision and validates safe standing before committing. Duplicate confirmation does not queue another request. Closing, reselection, selector opening or world replacement cancels the pending request without canceling unrelated exploration actions. Vehicle eligibility is checked again at commit. Successful relocation clears selection before animated return; reopening otherwise preserves it, and world reload invalidates it.
- `planet_markers` projects anchors from the attained main-camera pose before UI layout, with a matching visible hit cache. Both maps consume `shared::planet_markers` for category colors, shapes, and label visibility: settlement labels are shown by default, while region and bridge labels appear on hover. Planet view does not reveal region labels at closer zoom. Its rim compass projects geographic north/east through the attained camera pose, highlights north in the minimap accent, and fades all directions as the view approaches either pole; the labels stay inside the viewport when closer zoom crops the globe's rim. Markers respect the spherical horizon and viewport; clutter-suppressed labels have no invisible text hit target. Anchor identities use collection indices plus world epoch, so duplicate names remain distinct.
- Rain and snow particle presentation is hidden throughout Planet view opening, browsing and return, then restored when the gameplay camera resumes. Weather simulation remains live.
- `planet_roads` automatically builds a separate depth-tested surface/deck highlight from cached centerline samples while Planet view is active. Per-sample widths adapt to camera depth and viewport, with a screen-width floor for overview readability; physical road geometry and collision stay unchanged. Rebuilds process at most eight paths per update, retain the committed mesh during construction, and assemble/replace the complete mesh on a later update. Camera changes coalesce into the next request; closing the view cancels unfinished work, and world changes invalidate stale geometry. The highlight follows interface alpha and selector visibility.
- The heading-up minimap uses a flat projection with a rotating edge compass and nearby markers. Baked `LevelRegions` coverage includes named oceans, lakes, rivers, land biomes, ranges, coasts, settlements and roads. Shared label policy shows settlement names by default and other region names only for the nearest marker hovered within 12 logical pixels inside the circular minimap content. Bridge markers use area-weighted centers of generated deck top surfaces. Labels annotate markers, stay on one line and truncate at the circular boundary without shifting away from their markers or using leader lines. The minimap omits centers outside its viewport and refreshes its overlay less often than each frame.
- The sidebar Location section reads every region ID from the exact queried face and groups those memberships into geography, settlement name/kind, and roads. It never unions adjacent faces. Named bridges are queried separately against the actual deck top surfaces used for rendering and collision; standing below a bridge does not show its name. Long rows use display-only ellipsis; the region and membership data stay complete.
- `map::GameAssets` owns the procedural projectile. `loot::LootAssets` owns material/weapon GLB scenes. Player, zombie, loot, scenery, and structure scenes are visual children of runtime-owned placement/physics roots; imported scenes never own gameplay collision.
- Production and opt-in review actors carry `shared::actor_animation::ActorPlayback`, which selects clips from each actor’s own GLB and crossfades visual actions. `just asset-showcase` places these review actors on the planet without changing the production catalog. The player scene owns `socket.hand`; `sync_equipped_weapon` mirrors `LootState::equipped` with a visual-only child.
- `LootState` is the single resource for materials, collected weapons, and equipped weapon; reset it on prestige. Weapons override fire stats, lose durability per shot, and auto-swap on break. Dead zombies drop loot; wave-start loot supports magnet collection and equipping. Crafting UI and handlers are currently dormant; restoring crafting is deferred.
- Bevy 0.19 events use `Message`, `add_message()`, `MessageWriter`, and `MessageReader`.
- UI uses the shared theme palette and bundled Monaspace Neon `UiFont`.

## Verification

Run the focused test first, then workspace `cargo check` and `cargo clippy`. If GLB catalogs or embedded `LevelData` are affected, follow the [GLB pipeline](glb-pipeline.md) or [level pipeline](level-pipeline.md) generation and deterministic comparison checks.

[Vehicle regressions](vehicle-diagnostics.md) cover forward/reverse wheel rolling
with steering, and baked bridge entry, deck travel and exit in both endpoint
orders and gears, from rest and with initial speed. They run in the ordinary suite.
Wheel spin uses local -Z forward and the X axle; bridge slab triangles face outward
on every surface. Avian collision remains authoritative for support; support probes
orient driving and attitude without repositioning the body.

[Planet view acceptance](planet-view-acceptance.md) records the live renderer,
matched capture matrix, frame-time evidence and reproduction command. Frozen
lighting captures do not substitute for the moving-body performance routes.

## Showcase capture

The `asset-review` development feature includes `exploration::showcase`. Setting
`TERRA_FLIGHT_CAPTURE` to an output directory runs one connected expedition at
30 Hz after a streaming warmup, captures PNG frames from an offscreen render
target so window focus cannot interrupt recording, and exits after the explorer
physically returns home. `TERRA_FLIGHT_PREVIEW=1` saves one still every three
seconds. `beats.tsv` records story transitions and `scenes.tsv` records the
completed frame count; an incomplete journey fails rather than producing a film.

Only initial placement authors actor positions. Subsequent walking, driving,
flying, collision and vehicle transfers use the production systems. Shared
`flight_showcase` supplies bounded flight controls; the capture director chooses
baked destinations and overrides only the cinematic camera, weather and light.
The normal gameplay UI, HUD and minimap remain visible. Both vehicles persist throughout the journey. The real world
map brackets the expedition, and the return is validated before closing.

`just flight-showcase` records and encodes the repository media. The editor may
compress uneventful ground travel and straight flight with selective dissolves;
interactions, takeoff, maneuvers and landing remain continuous. The closing camera
returns to the opening pose after a matching-direction walk. Departure is in
morning light, flight and landing stay in daylight, and the final return reaches
evening light; the closing map advances through night
to the next morning. Decoded first/last video frames must hash identically. See [the cinematic brief](showcase.md) and the root README
for prerequisites and preview usage.
