# Production asset activation (#21)

The complete approved catalog now drives normal gameplay: 37 scenery scenes, 17 structures, four materials, five weapons, and three actors. Generation uses the running Blender MCP server through `just assets`; `just assets-check` compares fresh exports without writing the production directory. See [pipeline and identifier migration](glb-pipeline.md).

## Integration

Every production scene has one mesh primitive. The player is a one-meter humanoid with a centered radial capsule. Structure meshes use their authored meter dimensions; scenery applies its existing deterministic variation once to both visuals and simplified collision. Tree trunks block movement while empty canopy space remains traversable. House collision preserves the interior and doorway. Runtime actor clips come from each actor's own GLB.

The terrain skin is .02 m, with swept CCD retained. Fall diagnostics now measure the actual displaced collision triangles instead of interpolated generator heights, which differed from the collision surface during travel. Movement, combat parameters, UI, level data and world-generation footprints are unchanged. Loot and zombie gameplay modules remain dormant; their asset references/scales were migrated, but this work does not claim to restore or validate those disabled gameplay systems. Their assets, grips and animations are covered by the separate preview and export contracts.

The approved asset workbench stays a development example. Planet review fixtures require the `asset-review` feature and an explicit environment flag; normal gameplay excludes their modules. The historical Rust generator remains byte-for-byte preserved as the explicitly selected `legacy_assets` binary.

## Populated-world review

On 2026-10-03, the actual production world was captured at six deterministic route stops using the existing gameplay camera, M4 Pro/Metal, optimized debug build, 2048×1152 window. Each stop includes two seconds of forward input and a settling period. Frame times sample seconds 8–12; results include normal rendering and streaming, not an isolated GPU benchmark.

| Route stop | Median ms | p95 ms | Visible meshes | Minimum body-center clearance, m |
|---|---:|---:|---:|---:|
| Settlement | 8.33 | 9.14 | 12,059 | .510 |
| Forest | 8.30 | 9.40 | 19,245 | .520 |
| Jungle | 9.18 | 12.79 | 27,538 | .521 |
| Desert | 11.17 | 16.72 | 2,327 | .507 |
| Snow | 8.33 | 8.89 | 2,695 | .511 |
| Swamp | 8.86 | 11.70 | 25,068 | .513 |

Stop names describe their initial baked face classification; movement can cross a face boundary, so the capture HUD may show another biome. Sparse terrain and dense vegetation were both inspected. All six captures contain valid rendered images, the humanoid remains upright, and no sampled body center fell below the actual terrain. These short samples do not establish performance on other hardware or long-session streaming behavior.

Local evidence: `/tmp/terra-production-review/*.png`, `/tmp/terra-production-walkthrough-final.log`, and `/tmp/terra-production-preview.log` (18 item/actor review views). The first capture attempt produced black images; stationary and moving rechecks and the repeated six-stop route rendered correctly. The initial capture failure did not reproduce and its cause remains unknown; that attempt was excluded from visual evidence.

## Verification and review

Export integration checks cover all 66 identities, one primitive per production scene, canonical mesh names, ground pivots, actor skins/bones/clips, sockets and byte determinism. Runtime regression checks cover the actual structure/scenery spawn paths, radial player capsule, canopy clearance, house interior, repeat spacing, and the seed-1337 long startup frame that previously exposed a physics race.

The final validation commands are `cargo test --workspace --all-features`, `cargo test -p main --example asset_preview_prototype`, `just asset-candidates-test`, `cargo check --workspace --all-targets --all-features`, `cargo clippy --workspace --all-targets --all-features`, `cargo fmt --all --check`, `just assets-check`, and `cargo run -p gen_assets --bin legacy_assets -- --check`.

All final commands passed: 120 workspace tests, 10 preview tests, and 14 Blender integration tests. Check/clippy completed with existing dormant-code/style warnings; production and historical catalog byte checks passed. A final settlement smoke run after the diagnostic change showed no asset errors or fall warnings (`/tmp/terra-final-smoke.log`).

Standards review against `dfe9f9d`: shared contracts remain in `shared`, physics uses Avian state, and generated files remain reproducible and ignored. No blocking findings. Historical dormant code is explicitly identified above.

Spec review against #21: production migration, consolidation, deterministic generation, independent preview retention, collision/animation checks and populated-world evidence are implemented. The user approved the final production version on 2026-10-03 with “looking good” after the normal game was reopened at local commit `c857929`. This satisfies the final visual acceptance gate for #21 and completes the asset-refresh map #11.
