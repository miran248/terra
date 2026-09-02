# Shared crate

## Ownership

- Owns stable shared types, constants, utilities, terrain/topology/world generation, item data, app state, and procedural-art contracts.
- Must not depend on `main`, `gen_level`, or another binary crate. Tests live in source `#[cfg(test)]` modules.

## Local contracts

- Public APIs must remain stable or be versioned. Breaking public API or `LevelData` schema changes require workspace-wide checks and regenerated embedded assets.
- `CellId` is authoritative terrain identity; `FaceId` is derived query/presentation identity.
- `art.rs` names, catalog paths, deterministic variants, and collider specifications are shared runtime/generator API.
- `sphere.rs` owns the meter-based 2000 m planet model and geodesic operations.
- `worldgen.rs` exposes only finalized `CompletedWorld` data and statistics; mutable generation state stays private.

Load [.agents/skills/terra-worldgen/SKILL.md](../../.agents/skills/terra-worldgen/SKILL.md) for architecture and determinism contracts. Load [.agents/skills/terra-assets/SKILL.md](../../.agents/skills/terra-assets/SKILL.md) when changing art or serialized assets.

## Verification

Run `cargo test -p shared` and `cargo clippy -p shared` from the workspace root.

## Child context index

- `src/worldgen/`: [AGENTS.md](src/worldgen/AGENTS.md)
