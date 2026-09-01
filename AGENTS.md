# Terra agent contract

## Scope

- These instructions apply repository-wide; a nearer `AGENTS.md` adds subtree-specific contracts.
- Read the root and nearest subtree `AGENTS.md` before editing. Keep durable details in the nearest context or the project skills below instead of duplicating them.
- Update affected context and links when structure or contracts change.

## Workflow

- Use test-driven development for behavior changes: add a failing test, implement the smallest fix, then refactor while green.
- Keep reusable code in `shared`; binary crates orchestrate it.
- Use Rust edition 2024 on the pinned nightly toolchain with Cranelift.
- Commit with Conventional Commits. Never push unless explicitly requested.

## Project skills

Project-local skills are canonical under `.agents/skills/`:

- Game runtime, rendering, physics, UI, and streaming: [.agents/skills/terra-game/SKILL.md](.agents/skills/terra-game/SKILL.md)
- Procedural GLB and level-data pipelines: [.agents/skills/terra-assets/SKILL.md](.agents/skills/terra-assets/SKILL.md)
- Shared terrain, topology, and deterministic generation: [.agents/skills/terra-worldgen/SKILL.md](.agents/skills/terra-worldgen/SKILL.md)

Load only the relevant skill. Do not copy generic agent skills into this repository.

## Verification

Run the narrowest relevant check first, then the affected workspace checks. The canonical commands and determinism gates live in the relevant project skill.

## Child context index

- `crates/main/`: [AGENTS.md](crates/main/AGENTS.md)
- `crates/gen_assets/`: [AGENTS.md](crates/gen_assets/AGENTS.md)
- `crates/gen_level/`: [AGENTS.md](crates/gen_level/AGENTS.md)
- `crates/shared/`: [AGENTS.md](crates/shared/AGENTS.md)
