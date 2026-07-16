# terra

3D spherical-planet procedural world experiment. Bevy 0.19 + Avian3d physics.

![1](https://github.com/user-attachments/assets/7095a5f7-a04f-4eb5-a65a-d28b509700ed)
![2](https://github.com/user-attachments/assets/4631fd01-0fc6-4e84-aa10-97391faf8128)

## Crates

| Crate | Purpose |
|-------|---------|
| `shared` | Library: terrain gen, topology, water, roads, level schema, items, upgrades, theme |
| `gen_assets` | CLI: deterministic procedural GLB catalog generator |
| `gen_level` | CLI: precomputes planet-level data to a postcard binary |
| `main` | Bevy binary: loads precomputed world, chunks terrain/water/flora/structures, weather, minimap |

## Prerequisites

- Rust 1.96.1 (see `rust-toolchain.toml`)
- `rust-analyzer`, `clippy`

## Quickstart

```sh
# Generate GLB models (once, or after changing art definitions)
cargo run -p gen_assets -- --out-dir crates/main/assets/models

# Precompute the planet (once, or after terrain changes)
cargo run -p gen_level -- crates/main/assets/level_1337.bin

# Run the game
cargo run -p main --release
```

## Offline pipeline

1. `gen_level` precomputes terrain, water, rivers, roads, settlements → `level_{seed}.bin` (embedded via `include_bytes!`)
2. `gen_assets` generates GLB catalogs (environment, structures, items, actors) → `assets/models/`
3. `main` loads both at startup, streams the world with 3-LOD chunking

Different seed: `PLANET_SEED=42 cargo run -p gen_level -- crates/main/assets/level_42.bin`

## Verification

```sh
cargo check                    # whole workspace
cargo clippy                   # whole workspace
cargo test -p shared           # shared crate tests
cargo test -p gen_assets       # asset generator tests
cargo run -p gen_assets -- --check --out-dir crates/main/assets/models  # GLB determinism
cargo bench -p gen_level --bench worldgen    # level gen benchmark
```

## Terrain & Biomes

20 terrain types across land, water, and shore transitions on a 2000 m radius planet. Macro landforms (valley, lowland, hills, mountains up to 500 m, plateau) drive elevation; biomes (desert, plains, forest, tundra, snow, swamp, jungle, savanna, volcanic, glacier) are placed by latitude, moisture, and temperature. Water bodies include ocean up to 200 m depth, freshwater and salt lakes ~140 m mean radius, and rivers with spring sources. Shore transitions: beach, cliff, lake shore, river bank.

Per-face properties: liquid or frozen water phase, normal or frozen surface condition, slope class (flat/gentle/steep/cliff), and water depth (shallow/deep/abyss).

## Geography

- 5 continents + 3 offshore islands per world, targeting ~even land/water split
- 5 mountain ranges, 3 lakes, 6 river routes per world
- 12 settlements connected by a road network (gravel/dirt/sand/rock) with bridge spans (200 m spacing) for water crossings
- Named regions: oceans, lakes, rivers, beaches, cliffs, forests, deserts, mountains, plains, tundra, swamps, jungles, savannas, volcanoes, glaciers, towns, roads

## Flora & Structures

27 decorative flora kinds (trees, bushes, flowers, rocks, grass, logs, mushrooms, cacti, berries, dead trees, reeds, seaweed, lilypads, coral, anemones, starfish, shells, kelp, cattails, vines, tumbleweeds, skulls, snowdrifts, stumps, ferns, icicles, snowmen)

17 structure kinds (ruins, watchtowers, docks, farms, walls, wells, campfires, tents, crates, fences, barricades, lamp posts, signposts, guardrails, railings, suspension cables, houses)

Both are placed contextually per biome and streamed at 3 LOD levels (960 m / 300 m distances).

## Rendering

Custom WGSL shaders: water with geometric swell, normal-perturbation chop, and analytic depth gradient; foliage with wind-driven vertex sway. TAA with depth + motion-vector prepasses. HDR tonemapping, bloom, and Rayleigh atmosphere.

## Map

2D minimap (heading-up, compass labels) and 3D full-screen globe map (north-up, pan/drag, ray-cast terrain). Named region labels from baked centroids. Toggled with `M`.

## Time & Weather

Day/night cycle with a world sun orbiting the planet's Y axis so the lit hemisphere is day and the far side night. Toggleable sun-lock (`1`) keeps noon overhead permanently.

Dynamic weather: randomized fronts ease a global wind vector and precipitation intensity (rain or snow, resolved per-location by local temperature). 1500-particle camera-anchored pool renders streaks (rain) or flakes (snow), wrapping around the camera. Wind drives water swell, flora sway, and precipitation drift.

## Controls

| Key | Action |
|-----|--------|
| W/S | Move forward/back |
| A/D | Rotate left/right |
| Shift | Sprint (hold) |
| Space | Jump |
| 1 | Toggle day/night cycle |
| M | Toggle world map |
