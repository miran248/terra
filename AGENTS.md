# Terra agent contract

## Scope

- These instructions apply repository-wide; a nearer `AGENTS.md` adds subtree-specific contracts.
- Read the root and nearest subtree `AGENTS.md` before editing. Keep durable details in the nearest context or the linked documentation instead of duplicating them.
- Update affected context and links when structure or contracts change.

## Workflow

- Use test-driven development for behavior changes: add a failing test, implement the smallest fix, then refactor while green.
- Keep reusable code in `shared`; binary crates orchestrate it.
- Use the toolchain pinned in `rust-toolchain.toml` and the repository Cargo configuration.
- Commit with Conventional Commits. Never push unless explicitly requested.

## Verification

Run the narrowest relevant check first, then the affected crate or workspace checks.
For generated assets or world-generation changes, follow the relevant pipeline and
determinism checks linked from the nearest `AGENTS.md`. For visual changes, report
any manual visual checks performed or still needed.

## Working conventions

### Issue tracker

Track issues in GitHub Issues for `miran248/terra` using `gh`. Read `docs/agents/issue-tracker.md`
before issue operations.

### Triage labels

Use the five canonical triage roles as GitHub labels. Read
`docs/agents/triage-labels.md` before triaging.

### Domain docs

Use a single-context glossary and ADR layout. Read
`docs/agents/domain.md` before exploring domain concepts or decisions.

## Child context index

- `crates/main/`: [AGENTS.md](crates/main/AGENTS.md)
- `crates/gen_assets/`: [AGENTS.md](crates/gen_assets/AGENTS.md)
- `crates/gen_level/`: [AGENTS.md](crates/gen_level/AGENTS.md)
- `crates/shared/`: [AGENTS.md](crates/shared/AGENTS.md)
