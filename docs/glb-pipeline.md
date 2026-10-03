# Blender production asset pipeline

The approved [script-first refresh](adr/0001-scripted-asset-refresh.md) is now the runtime asset source. See [production activation review](asset-production-review.md) for validation and final visual acceptance.

## Generate and validate

Install the pinned Rust toolchain and Python 3. Open **Blender 5.2.2 LTS**, enable its installed `lab_blender_org.mcp` add-on, and start the server at `localhost:9876`. No background Blender process is used.

```sh
just assets                  # or cargo run -p gen_assets
just assets-check            # or cargo run -p gen_assets -- --check
just asset-candidates-test   # exported-file integration suite through MCP
just run                     # normal game, using generated production assets
just asset-preview           # independent development example
```

`gen_assets` is a small Rust CLI that forwards arguments to `blender/generate.py --production`. Python invokes the running MCP add-on's NUL-terminated JSON execution protocol. `--out-dir PATH` overrides the default `crates/main/assets/models/production/`. Generation writes 66 GLBs and a manifest; generated files are ignored by Git. `--check` regenerates in temporary storage and compares bytes, never writing the target directory. Missing/corrupt files fail validation. Do not hand-edit the exports.

The exact Blender pin includes its bundled glTF exporter. Export is GLB 2.0, active scene only, meter units, Y-up, forward -Z, flat normals, linear vertex colors, no textures, roughness .95, no compression, and NLA-track animation. Actor clips retain idle/walk/attack names and one-second duration; skins and hand sockets are retained. There is no random sampling; manifest seed is zero. A Blender/exporter upgrade requires an explicit pin change and repeat validation.

## Ownership and contracts

- `crates/gen_assets/blender/pilot.py` owns shared mesh helpers, palette, representative recipes and export orchestration. `scenery.py`, `structures.py`, `items.py`, `actors.py` and `rig.py` own the remaining recipes and rig. Keep this vocabulary small; do not introduce a general procedural framework.
- `crates/shared/asset_dimensions.json` owns meter dimensions, simplified collision parts, grips and repeat spacing. `shared::art::asset_names` enumerates all canonical identities; runtime files come from `asset_path`. `shared::asset_contract` consumes the same JSON.
- Every production scene exports **one compatible mesh primitive**. Static parts are joined; actor parts are joined after vertex weights/armature modifiers are authored. Vertex colors, skeleton targets, clips and sockets survive consolidation. Production and authoring exports use the same recipes; consolidation changes representation, not design.
- Only Blender scenes/datablocks tagged `terra_blender_pilot` are regenerated. This tag is retained as an ownership identifier. The user's original scene and active-scene choice are preserved; no `.blend` file is saved. Owned scenes are scratch representations of scripts.
- Imported scenes own visuals only. Runtime constructs collision from the shared contract. Structures use scale one; scenery retains its deterministic .7–1.3 instance variation applied once to both mesh and collision. Actors use centered physics capsules with a ground-pivot visual offset; the player is one meter tall. Avian rotation keeps asymmetric colliders radial.

## Identifier and path migration

All 66 semantic scene identifiers, canonical root/primary-mesh/material names, actor bone names, `socket.hand`, `socket.grip`, repeat sockets, and named clips remain available. The loader now reads `models/production/<scene>.glb` rather than four combined baseline catalogs. Each actor has its own clip source, avoiding ambiguous shared animation names across variants.

Compatible authoring-part nodes (such as individual house planks or separate body mesh nodes) consolidate into `<scene>.visual`; `<scene>.mesh` and `<scene>.material` remain canonical. These detail nodes are not gameplay attachment APIs. Runtime foliage binding resolves the semantic scene ancestor, so wind and matching prepass behavior survive consolidation. The old four catalog path constants remain only for baseline comparison and historical tooling.

## Retained preview and historical generator

The user-approved preview remains a **development example**, not production UI. Its default source is the consolidated production catalog; `python3 crates/main/examples/asset_preview_prototype.py --candidates` inspects unconsolidated authoring exports, and `--baseline-only` inspects the historical set. Dimension edits remain temporary. See [preview controls](asset-preview-prototype.md).

The old Rust mesh generator is retained explicitly as `src/bin/legacy_assets.rs`, invoked only by baseline comparison tools (`cargo run -p gen_assets --bin legacy_assets -- ...`). Its snapshots/tests preserve reproducible history; the default generator and runtime no longer consume it. The CLI requires an explicit binary choice for this path.

Opt-in terrain fixtures compile only with the `asset-review` feature. `just asset-showcase`, `asset-structure-showcase`, and `asset-item-showcase` enable that feature. Normal `just run` contains neither fixture module. `TERRA_PRODUCTION_CAPTURE=1 cargo run -p main --features asset-review` captures the real populated production world without candidate props and records frame times at six deterministic stops. No review fixture changes level data or gameplay rules.
