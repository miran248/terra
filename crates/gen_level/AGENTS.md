# Level precomputation

- Owns the thin CLI that reports, serializes, and writes `shared::worldgen::CompletedWorld` as Postcard `LevelData`.
- Output is `crates/main/assets/level_{seed}.bin`, deterministic for `PLANET_SEED` (default 1337).
- Terrain and physics share subdivision-4 displaced triangles; topology derivation, projection, assembly, and statistics remain in `shared::worldgen`.

Load [.agents/skills/terra-assets/SKILL.md](../../.agents/skills/terra-assets/SKILL.md) for generation and verification procedures, and [.agents/skills/terra-worldgen/SKILL.md](../../.agents/skills/terra-worldgen/SKILL.md) for generation-policy changes.
