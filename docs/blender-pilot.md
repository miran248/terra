# Blender representative asset pilot

This implements [Prove scripted Blender authoring with a humanoid and house](https://github.com/miran248/terra/issues/12). The humanoid/house direction was accepted for extension; the representative set awaits in-game review before production rollout. The [asset refresh decision](adr/0001-scripted-asset-refresh.md) remains the scope boundary.

## Run through Blender MCP

Open **Blender 5.2.2 LTS** and start its MCP add-on server on `localhost:9876`. The generator requires that exact Blender version and uses the installed `lab_blender_org.mcp` add-on's NUL-terminated JSON execution transport, the same local endpoint used by the MCP tool. It never starts Blender in background/CLI mode.

```sh
just asset-candidates
just asset-preview
```

`asset-candidates` runs the repository-owned `crates/gen_assets/blender/pilot.py` inside the connected Blender instance. It creates separate scenes named `actor.player`, `structure.house`, `scenery.tree.0`, `scenery.rock.0`, and `weapon.knife`; only scenes and datablocks tagged `terra_blender_pilot` are replaced on regeneration. The previous active scene is restored. Scripted pilot scenes are generated scratch state, so edit the scripts rather than hand-editing those scenes. The user's original scene is not cleared or saved.

The script writes to `crates/main/assets/models/candidates/`, separate from all four current catalog GLBs. Files and the generated manifest are ignored by Git. Regenerating the baseline through `asset-preview` leaves candidate files intact. When candidates exist, the preview shows them automatically for the matching scene names; other entries retain their baseline. Use **Candidate / baseline** to switch sources and **Compare** to place the baseline beside the candidate. Each source retains separate temporary dimension edits.

To inspect only the baseline:

```sh
python3 crates/main/examples/asset_preview_prototype.py --baseline-only
```

## Dimensions and design proposal

The character is 1.000 m tall with a roughly 0.434 m arm-to-arm width. It has a faceted head, simple facial features, separate limbs, sage tunic, clay scarf, lavender trousers, and boots. The generated `socket.hand` remains available for later attachment/rig work.

The house has an approximately 3.030 × 2.495 × 2.510 m outer envelope (width × height × depth), including eaves. Its entrance frame provides 1.20 m clearance above the 0.14 m foundation. The closed door is visual geometry; this static pilot does not implement traversable interiors or an opening door. Frames, shutters, foundation blocks, gable beams, and roof thickness establish the proposed construction detail.

The tree uses tapered branching and three asymmetrical canopy masses starting above 1.2 m. The rock has an irregular faceted silhouette and a small lichen patch. The existing knife kind was selected as a simple handheld pilot: 0.285 m long with a `socket.grip` at the handle center, ready for attachment validation during rigging.

All five share one palette defined in the script. Colors are authored in sRGB and converted to linear vertex colors, with a white PBR base factor and roughness 0.95. Flat normals retain the low-poly look. Export is texture-free, uncompressed GLB, Y-up, forward -Z, with a ground pivot and meter-sized coordinates. The recipes' Y coordinates are reflected with corrected winding before Blender's axis conversion.

Candidate runtime scale is **1**. Current baseline scaling is only applied to baseline models; applying the existing 0.55 player or 8 × 5 × 6 house factors to candidates would double-scale them. The authoritative dimensions and physical surfaces are in `crates/shared/asset_dimensions.json`. Blender fits the authored geometry to those meter dimensions before export; `shared::asset_contract` reads the same contract for runtime construction. Box sizes are full lengths; capsule length excludes its end caps. Instance scale is applied once to both visuals and physics.

The humanoid now has a scripted skeleton, skin, and distinct one-second idle, walk, and attack clips. They retain the existing scene/root/primary-mesh/material names and player hand socket; they do not replace the existing production actor catalog. The preview instantiates real Avian candidate colliders and draws them in gold, separately from cyan visual bounds and red baseline snapshots. Baseline clips remain available on baseline actors. `shared::actor_animation` binds each actor to its own source clips with 120 ms visual transitions. The preview equips the candidate knife at the moving hand socket using its shared grip anchor. Production activation remains a separate task.

The humanoid capsule is grounded and 1 m tall. The tree collider is a 0.2 m diameter trunk, 1.8 m tall; canopy and branches do not block movement. The rock uses two small boxes. The house uses a compound of foundation, walls, lintel, closed door, and two roof slopes rather than a solid envelope. Its interior is empty, but the visible closed door blocks entry; opening doors/traversable house gameplay remains out of scope. The knife has a small pickup box, not a new damage shape.

`asset_collision.rs` is the thin Avian adapter used by the preview. Its tests exercise real scaled shapes, empty canopy/interior space, trunk contact, and a headless fixed-step body settling onto ground. The isolated single-contact query gives an unstable penetration depth for a capsule on a large box, including without a compound; ground alignment is therefore verified through the actual physics solver.

## Reproducibility and validation

```sh
just asset-candidates-test
just asset-candidates-check
TERRA_PREVIEW_CAPTURE=1 just asset-preview
```

The first command tests the public MCP generation command and exported files: names/materials, meter dimensions, -Z facing, ground pivot, preservation of the original scene, byte reproducibility, and non-writing corruption detection. The second regenerates into a temporary directory and compares all generated GLBs and the manifest byte-for-byte; it changes scratch Blender scenes but never writes the checked output files.

No random sampling is currently used (`seed: 0` in the manifest). Identical exports have been observed across repeated generation in the pinned Blender version without binary normalization. Different Blender/exporter versions require an explicit pin update and rerun of the checks.

The visual walkthrough loads all catalog scenes plus all candidate dependencies through Bevy. It captures `/tmp/terra-preview-inspect.png`, `/tmp/terra-preview-compare.png`, and `/tmp/terra-preview-house-candidate.png` along with the existing measurement/tree views. The manifest records sampled animated body envelopes; tests check loop boundaries, distinct clip data, skin export, grounded feet, and the one-meter body envelope. Final representative-set approval remains a later gate.

## Animation on the planet

`just asset-showcase` places the house, tree, rock, and three candidate humanoids playing idle, walk, and attack near the existing player on the generated planet. Each carries the knife. Press C to show the actual candidate colliders. This is an opt-in review scene: production assets, player control, firing cadence, and damage are unchanged. The game uses the same per-asset playback module as the preview.

`TERRA_ASSET_SHOWCASE=1 TERRA_ASSET_CAPTURE=1 cargo run -p main` temporarily positions the review camera, captures normal gameplay, a fixed overview, and collider overlays under `/tmp/terra-planet-*.png`, and exits. `TERRA_ASSET_BASELINE=1` selects the original assets at their original runtime scales for comparison. Without capture mode the normal game camera and movement remain available. The throwaway scene lives in `crates/main/src/asset_showcase_prototype.rs`; placement queries the displaced terrain triangles rather than the unit-sphere face-lookup resource.

The script uses tapered segments with explicit bone weights and keyed poses; it samples boot clearance and compensates vertically at the root bone. All clips are in-place and retain the existing one-second duration. Feet remain planted in idle/attack; walk alternates bent knees and arm swing. The locomotion collider intentionally follows the body core rather than swinging limbs or the visual weapon. Clip changes do not trigger damage or create a new combat mechanic.

## Representative review status

[Approve the representative assets on the planet](https://github.com/miran248/terra/issues/17) was explicitly approved by the user after reopening the in-game showcase: “looking good! approved lets proceed”. Its proportions, detail, animation, and current lighting are the accepted direction for sequential catalog work. Production activation remains ticket #21.

The current game lighting renders shaded faces much darker than the workbench; no lighting change has been silently applied. An earlier review capture accidentally sampled the unit-sphere lookup mesh and is superseded by the displaced-terrain captures. Multipart scene naming also exposed a foliage-binding bug; the binder now finds canonical scene ancestors so the existing wind and matching prepass shaders apply.

A short M4 Pro/Metal debug-build comparison, with the existing world resident and display-limited rendering, measured approximately 119 FPS for baseline and 122 FPS for candidates. This is a smoke check, not evidence of a performance improvement or full-rollout capacity.

| Candidate | Triangles | Mesh primitives |
| --- | ---: | ---: |
| Humanoid (15 joints) | 788 | 26 |
| House | 890 | 76 |
| Tree | 304 | 12 |
| Rock | 68 | 2 |
| Knife | 146 | 8 |

The geometry is modest, but authoring parts currently create too many primitives for dense rollout. Consolidate compatible parts during production export and remeasure populated scenes before activation; do not extrapolate a five-asset smoke check to the full world. Dedicated LOD work is not yet justified by these measurements.

## Full scenery candidates

All 37 scenery scenes are now authored through the same MCP pipeline; see the [scenery inventory and review](scenery-review.md). The 35 additional scenes each export one mesh primitive by joining compatible static recipe parts. The already-approved pilot tree/rock are unchanged. Nonblocking foliage has an empty collider list; the Avian adapter returns no shape for it.

## Structure candidates

All 17 structures now have candidates; see the [structure inventory and review](structure-review.md) for dimensions, compound colliders, repeat sockets, baseline comparisons, and actual-terrain evidence. The 16 additions each export one mesh primitive. Production bridge decks and catalog activation remain separate.
