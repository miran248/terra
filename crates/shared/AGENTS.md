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
- `sphere.rs` owns the planet model: `SpherePos` (unit-vector position of truth), `PLANET_RADIUS`, geodesic movement (`step_toward`/`step_tangent`), arc-length `distance`, tangent bases, and surface transforms. All gameplay position/distance math routes through it; tests enforce staying on the unit sphere.

# Verification

`cargo test -p shared` and `cargo clippy -p shared` from workspace root.

# Child DOX Index
