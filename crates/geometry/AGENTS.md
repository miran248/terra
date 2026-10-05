# Planetary geometry crate

## Ownership

- Owns spherical coordinates and geodesic operations, `PlanetMesh` construction and queries, typed topology and traversal, and geometric road helpers.
- Preserves Bevy-compatible math types and the current meter-based 2000 m planet model.
- Depends on Bevy math, transform, and ECS APIs plus standard or geometry-local code. World models, generation, `shared`, and application binaries depend on this crate directly.

## Local contracts

- Keep `CellId` authoritative for terrain identity and `FaceId` derived for face queries.
- Preserve public query behavior, candidate ordering, and geometric operation ordering during structural changes.
- Keep geometry tests beside their public interfaces; keep the reference full-scan comparisons and typed adjacency/traversal coverage.

Read [world-generation contracts](../../docs/worldgen.md) when changing geometry behavior, and the [level pipeline](../../docs/level-pipeline.md) when verifying generated artifacts.

## Verification

Run `cargo test -p terra-geometry` and `cargo clippy -p terra-geometry --all-targets -- -D warnings` from the workspace root.
