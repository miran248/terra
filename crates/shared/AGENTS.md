# Purpose

Shared library crate for rs-zombies. Common types, utilities, and shared Bevy components/resources live here. Every crate in the workspace may depend on `shared`.

# Ownership

- Owns `crates/shared/` — types, traits, constants, utility functions
- Must not depend on `main`, `gen_level`, or any other binary crate

# Local Contracts

- Public API must be stable or versioned; breaking changes require workspace-wide check
- Tests live in `#[cfg(test)]` modules within source files

# Work Guidance

- `items.rs` — loot/crafting data: `Material`, `WeaponKind` (stats incl. durability), `Recipe`
- `theme.rs` — UI palette (opencode "orng" dark theme) and bundled Monaspace Neon `FONT_PATH`
- `sphere.rs` — planet model, unit system (`METER` = 1 m), `PLANET_RADIUS` = 2000 m, `SpherePos` (unit-vector), geodesic ops (`step_toward`, `step_tangent`, `distance`, `ring_point`, `random_point`, `slerp`), tangent bases. Used by terrain/road gen and precompute tool. Not used by game systems (physics provides real positions).
- `terrain.rs` — biomes and heightmap (`TerrainGen`, seed-based). Layered elevation: continents + mountains + detail. `surface_radius` (true height), `render_radius` (water clamped to sea level). Habitable zones, `habitable_spawn()`, temperature/moisture. Bevy `Resource`.
- `roads.rs` — settlement/road network (`Roads`, deterministic per seed). `Roads::generate(&terrain)` places settlements, connects with least-cost paths. `Bridge` kind for water crossings.
- `planet.rs` — `PlanetMesh` (icosphere tris + grid-indexed ray intersection). `unit_icosphere_tris(n)` builds base triangles. `face_at(dir)` for face lookup. Used by `gen_level` for road/bridge face painting.
- `level.rs` — `LevelData` (postcard-serializable): precomputed tris, per-triangle colors, settlements, roads. Deserialized by main on startup.
- `state.rs` — `AppState` enum (Loading, Playing, Paused, Restarting, GameOver, Title).
- `upgrades.rs` — `Upgrade` definitions (Piercing, Bounces, Splits, etc.) and `UpgradeKind`.

# Verification

`cargo test -p shared` and `cargo clippy -p shared` from workspace root.

# Child DOX Index
