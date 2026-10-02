# Blender humanoid and house pilot

This implements [Prove scripted Blender authoring with a humanoid and house](https://github.com/miran248/terra/issues/12). The designs are candidates awaiting human review, not the production catalog. The [asset refresh decision](adr/0001-scripted-asset-refresh.md) remains the scope boundary.

## Run through Blender MCP

Open **Blender 5.2.2 LTS** and start its MCP add-on server on `localhost:9876`. The generator requires that exact Blender version and uses the installed `lab_blender_org.mcp` add-on's NUL-terminated JSON execution transport, the same local endpoint used by the MCP tool. It never starts Blender in background/CLI mode.

```sh
just asset-candidates
just asset-preview
```

`asset-candidates` runs the repository-owned `crates/gen_assets/blender/pilot.py` inside the connected Blender instance. It creates separate scenes named `actor.player` and `structure.house`; only scenes and datablocks tagged `terra_blender_pilot` are replaced on regeneration. The previous active scene is restored. Scripted pilot scenes are generated scratch state, so edit the scripts rather than hand-editing those scenes. The user's original scene is not cleared or saved.

The script writes to `crates/main/assets/models/candidates/`, separate from all four current catalog GLBs. Files and the generated manifest are ignored by Git. Regenerating the baseline through `asset-preview` leaves candidate files intact. When candidates exist, the preview shows them automatically for the matching scene names; other entries retain their baseline. Use **Candidate / baseline** to switch sources and **Compare** to place the baseline beside the candidate. Each source retains separate temporary dimension edits.

To inspect only the baseline:

```sh
python3 crates/main/examples/asset_preview_prototype.py --baseline-only
```

## Dimensions and design proposal

The character is 1.000 m tall with a roughly 0.434 m arm-to-arm width. It has a faceted head, simple facial features, separate limbs, sage tunic, clay scarf, lavender trousers, and boots. The generated `socket.hand` remains available for later attachment/rig work.

The house has an approximately 3.030 × 2.495 × 2.510 m outer envelope (width × height × depth), including eaves. Its entrance frame provides 1.20 m clearance above the 0.14 m foundation. The closed door is visual geometry; this static pilot does not implement traversable interiors or an opening door. Frames, shutters, foundation blocks, gable beams, and roof thickness establish the proposed construction detail.

Both share one palette defined in the script. Colors are authored in sRGB and converted to linear vertex colors, with a white PBR base factor and roughness 0.95. Flat normals retain the low-poly look. Export is texture-free, uncompressed GLB, Y-up, forward -Z, with a ground pivot and meter-sized coordinates. The recipes' Y coordinates are reflected with corrected winding before Blender's axis conversion.

Candidate runtime scale is **1**. Current baseline scaling is only applied to baseline models; applying the existing 0.55 player or 8 × 5 × 6 house factors to candidates would double-scale them. Moving dimensions into a shared production contract and matching simplified colliders belongs to the later collision task.

These are static candidate scenes. They retain the existing scene/root/primary-mesh/material names and player hand socket; they do not replace the existing actor catalog or claim new animation support. The preview shows no candidate collider and labels candidates static. Baseline clips remain available on baseline actors. Rigging, idle/walk/attack, and production collision changes remain separate tasks.

## Reproducibility and validation

```sh
just asset-candidates-test
just asset-candidates-check
TERRA_PREVIEW_CAPTURE=1 just asset-preview
```

The first command tests the public MCP generation command and exported files: names/materials, meter dimensions, -Z facing, ground pivot, preservation of the original scene, byte reproducibility, and non-writing corruption detection. The second regenerates into a temporary directory and compares both GLBs and the manifest byte-for-byte; it changes scratch Blender scenes but never writes the checked output files.

No random sampling is currently used (`seed: 0` in the manifest). Identical exports have been observed across repeated generation in the pinned Blender version without binary normalization. Different Blender/exporter versions require an explicit pin update and rerun of the checks.

The visual walkthrough loads all catalog scenes plus both candidate dependencies through Bevy. It captures `/tmp/terra-preview-inspect.png`, `/tmp/terra-preview-compare.png`, and `/tmp/terra-preview-house-candidate.png` along with the existing measurement/tree views. Full animation bounds, actual physics, planet placement, and final visual approval are not established by this pilot.
