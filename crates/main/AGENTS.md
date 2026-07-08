# Purpose

Binary crate for the rs-zombies Bevy application. Entry point, app bootstrap, and game systems live here.

# Ownership

- Owns `crates/main/` — binary, Bevy systems, rendering, game logic
- Depends on `shared` crate for common types and utilities

# Local Contracts

- Must remain a thin orchestration layer; reusable logic belongs in `shared`
- Bevy systems are registered in `main.rs` or modules under `src/`

# Work Guidance

- Core loop: materials (`Metal/Wood/Rope/Cloth`) and weapons are scattered across the map at wave start (`loot.rs::scatter_loot`), collected via the magnet, and either equipped (weapons) or spent on crafting recipes. Dead zombies also drop random loot (`loot.rs::drop_zombie_loot`, called from `combat.rs` kill sites).
- The world is larger than the viewport (`MAP_WIDTH/HEIGHT`); the camera follows the survivor (`map.rs::camera_follow`) and the survivor is clamped to map bounds. Zombies spawn on a ring (`SPAWN_RADIUS`) around the survivor, not at map edges. `minimap.rs` draws a bottom-left overview (survivor/zombies/loot dots), rebuilt each frame.
- Weapons override the survivor's fire stats and lose durability per shot; a broken weapon auto-swaps to the next collected one.
- `loot::LootState` is the single resource holding materials, collected weapons, and the equipped weapon; reset it wholesale on prestige restart.
- Bevy limits systems to ~16 params — bundle related state into one resource rather than adding params.
- UI uses the shared `theme` palette and the bundled Monaspace Neon font (`assets/fonts/`, loaded once into the `UiFont` resource). Draw all UI colors/fonts from `shared::theme` + `UiFont`.

# Verification

`cargo check` and `cargo clippy` from workspace root.

# Child DOX Index
