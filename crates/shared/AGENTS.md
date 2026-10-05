# Shared crate

## Ownership

- Owns remaining shared types, constants, utilities, item data, app state, and procedural-art contracts. World classifications, configuration, records, and artifacts belong to `terra-world`; terrain and world generation belong to `terra-worldgen`.
- Must not depend on `main`, `gen_level`, or another binary crate. Tests live in source `#[cfg(test)]` modules.
- Depend on `terra-geometry` for spherical operations, `PlanetMesh`, typed topology, and geometric road helpers; import those interfaces directly from that crate.
- Import model types directly from `terra-world`; `shared` does not re-export them.
- Use `terra-worldgen` directly for generation-owned scenery variant counts and selection; keep names, catalog paths, and collider specifications in `art.rs`.

## Local contracts

- Public APIs must remain stable or be versioned. `terra-world` owns `LevelData` schema changes, which require workspace-wide checks and regenerated embedded assets.
- `art.rs` names, catalog paths, deterministic variants, and collider specifications are shared runtime/generator API.
- Shared owns no terrain or world-generation implementation or compatibility exports.

Read [world-generation contracts](../../docs/worldgen.md) for terrain and topology changes, the [GLB pipeline](../../docs/glb-pipeline.md) for procedural art, and the [level pipeline](../../docs/level-pipeline.md) for serialized level data.

## Verification

Run `cargo test -p shared` and `cargo clippy -p shared` from the workspace root.

## Child context index
