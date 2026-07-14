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
- `terrain.rs` — biomes and heightmap (`TerrainGen`, seed-based). Multi-octave Perlin (6 octaves, FBM) with domain warp. Flow accumulation on the icosphere cell grid, erosion carving into elevation. Precomputed cell elevation/moisture/temperature enables fast barycentric interpolation. `RiverSpring` is the ground-contact source cell for each river. Bevy `Resource`.
- `topology.rs` — project-owned typed connectivity. `CellId` is authoritative terrain identity; `FaceId` identifies derived query/presentation triangles. Sole owner of face-to-cell incidence and face adjacency; provides allocation-free typed traversal, components, bounded multi-source distances, predicate paths, and unweighted shortest paths without external graph or mesh types.
- `worldgen.rs` — deterministic cell-first level-generation pipeline. Terrain, water bodies, rivers, feature paint, and region inputs are owned on cells; faces are derived once for mesh/query output. Final face projections, component-smoothed river radii, and `LevelData` assembly are owned by the completed pipeline.
- `roads.rs` — settlement/road network (`Roads`, deterministic per seed). `Roads::generate(&terrain)` places 12 settlements, connects with shore-routed bridges. `PathKind::Bridge` for water crossings. Wobbled slerp with fallback to straight. Bevy `Resource`.
- `planet.rs` — `PlanetMesh` (icosphere tris + grid-indexed ray intersection). `unit_icosphere_tris(n)` builds base triangles. `face_at(dir)` for face lookup. Used by `gen_level` for road/bridge face painting.
- `level.rs` — `LevelData` (postcard-serializable): precomputed tris, per-triangle colors, settlements, roads. Deserialized by main on startup.
- `state.rs` — `AppState` enum (Loading, Playing, Paused, Restarting, GameOver, Title).
- `upgrades.rs` — `Upgrade` definitions (Piercing, Bounces, Splits, etc.) and `UpgradeKind`.

# Verification

`cargo test -p shared` and `cargo clippy -p shared` from workspace root.

# Child DOX Index
