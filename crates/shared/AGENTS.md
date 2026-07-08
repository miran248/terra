# Purpose

Shared library crate for rs-zombies. Common types, utilities, and shared Bevy components/resources live here. Every crate in the workspace may depend on `shared`.

# Ownership

- Owns `crates/shared/` — types, traits, constants, utility functions
- Must not depend on `main` or any other binary crate

# Local Contracts

- Public API must be stable or versioned; breaking changes require workspace-wide check
- Tests live in `#[cfg(test)]` modules within source files

# Work Guidance

- `items.rs` owns loot/crafting data: `Material`, `WeaponKind` (stats incl. durability), and `Recipe` (material cost -> weapon). Keep `Recipe::ALL` and weapon stats in sync (test enforces it).
- `theme.rs` owns the UI palette (opencode "orng" dark theme) and the bundled monospace `FONT_PATH`. All UI colors/fonts must come from here — no ad-hoc `Color::srgb` in UI code.
- `sphere.rs` owns the planet model and the **unit system: 1 world unit = 1 meter** (`METER`). `PLANET_RADIUS` is a game-sized 2 km planet (not Earth-scale — literal Earth radius would be invisible relief and break f32 precision). `SpherePos` (unit-vector position of truth), geodesic movement (`step_toward`/`step_tangent`), arc-length `distance`, tangent bases, and surface transforms. All gameplay position/distance math routes through it; tests enforce staying on the unit sphere.
- `terrain.rs` owns biomes and the heightmap (a Bevy `Resource`). Elevation is **layered for spatial coherence** — a dominant low-frequency `continents` field, plus ridged `mountains` and `detail` gated by an inland mask (never punch an ocean hole through a mountain; enforced by `elevation_is_spatially_coherent`). Biomes come from elevation + moisture + latitude-based `temperature_at` (hot equator, cold poles). `surface_radius` gives the true displaced height (land up to `MAX_MOUNTAIN` 500 m, ocean floor down to `MAX_DEPTH` 500 m) — used for logic (depth-based color, road cost). `render_radius` clamps water to sea level so the visible mesh has a **flat sea** (no sloped ocean-floor dents); the planet mesh builds from `render_radius`. `ground_world(pos, half_height)` rests an actor on the ground clamped to sea level.
- **Habitable zones** (`is_habitable`, `altitude`, `slope`): gentle land in the `HABITABLE_MIN_ALT`..`HABITABLE_MAX_ALT` band (1–300 m) and `HABITABLE_MIN_TEMP`..`HABITABLE_MAX_TEMP` (-10..30 °C), below `HABITABLE_MAX_SLOPE`. `habitable_spawn()` deterministically returns the *best-scored* start for a given seed (mild ~18 °C, inland, flat, dry). Seed for future villages/towns/farms/roads/paths — build those on habitable land.
- `temperature_at` returns **degrees Celsius** (0 °C freezes, 100 °C boils): ~+40 °C equatorial sea level, ~-40 °C poles, cooled by altitude (lapse), ±10 °C regional noise. Biome/color thresholds are in °C.
- `roads.rs` owns the settlement/road network (`Roads`, a Bevy `Resource`, deterministic per seed). `Roads::generate(&terrain)` places well-separated **named** `Settlement { pos, name }` on habitable **lowland** (`TerrainGen::habitable_anchors`, altitude capped at `HABITABLE_MAX_ALT` = 150 m so towns aren't on peaks) and connects nearest neighbours with **least-cost roads** — A* over a ground grid using `TerrainGen::travel_cost` (penalises slope + altitude, impassable at/below the waterline), so roads wind through valleys and around mountains. Paths are resampled to `SAMPLE_SPACING`; a road is dropped if a resampled chord clips the sea. Terrain cost is memoised per grid cell during search (noise sampling is the bottleneck).
- `planet.rs` owns `PlanetMesh` (the displaced faceted triangles) and `facet_radius(dir)` — the height actors rest on so they match the low-poly mesh, not the smooth heightmap. Triangles are bucketed into a lat/long grid so lookups test a handful of tris, not all ~30k (grid vs brute-force: ~45 µs vs ~6 ms for 256 queries; see `benches/facet_radius.rs`). Run `cargo bench -p shared`.
- `noise` crate is a shared dependency.

# Verification

`cargo test -p shared` and `cargo clippy -p shared` from workspace root.

# Child DOX Index
