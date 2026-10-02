# Asset scale preview prototype

Question: can a fixed 1 m humanoid reference, dimension guides, and direct baseline comparison expose inconsistent proportions before replacing the asset catalog?

This is throwaway code on the local `prototype/asset-scale-preview` branch. It is awaiting visual review; no layout or replacement dimensions have been approved. The [authoring decision](adr/0001-scripted-asset-refresh.md) records the agreed direction.

Run from the repository root:

```sh
python3 crates/main/examples/asset_preview_prototype.py
```

The launcher generates the current Rust catalogs, measures their GLB rest-pose bounds, and starts a separate native Bevy example. No level generation is needed. Generated models remain ignored by Git. The manifest lives in a temporary directory; edits exist only in memory and disappear when the app closes. Running the launcher regenerates the four baseline catalogs, so do not put hand-edited replacement GLBs in those output paths.

## Controls

- Select an asset from the paginated catalog; all 66 scenes and variants are included.
- `1`, `2`, `3` select inspection, baseline comparison, and measurement. Left/right arrows cycle these layouts. Native controls replace the prototype skill's browser URL switcher because Terra has no browser UI.
- `F` and `S` select orthographic front and side views; `O` selects perspective orbit. Right-drag orbits; middle-drag pans; wheel or zoom buttons change viewing distance.
- Dimension buttons change width, height, or depth by 1 cm or 10 cm. Uniform multipliers change all three together. `R` resets the selected asset. Each asset retains its own edits for the session.
- `A` cycles idle, walk, and attack. Space pauses/resumes. Animation controls affect actor scenes; other scenes remain static. The current catalog uses the same small whole-body rotation for all three named clips; the prototype does not invent replacement animations.
- `C` toggles rest bounds and collision overlays. Cyan is the resting visual envelope. Gold is a proposed simple box, not an approved collider. Red is the current runtime collider snapshot; in comparison mode it belongs to the unchanged baseline. Red overlays show through surfaces, including the ground, to expose offsets.

## Interpretation and limits

The reference mannequin is exactly 1 m tall and is a measuring aid, not a proposed character design. Camera scale stays fixed when selecting or resizing assets; manually zoom out for large assets. Comparison uses the same camera for both copies. The grid is 1 m; side rulers adapt their tick interval and label it in meters.

Dimensions come from transformed GLB position-accessor bounds in the resting pose, multiplied by the current runtime visual scale. They are axis-aligned envelopes, not measurements of a posed or deformed animated surface. Models are centered and grounded for inspection. Scenery uses nominal scale 1, excluding the runtime's 0.7–1.3 instance variation. Terrain curvature, foliage shaders, procedural bridges, physics simulation, gameplay camera, and world lighting are outside this first catalog preview.

Runtime scales and collider offsets are deliberately copied into the prototype from `chunks.rs`, `map.rs`, `zombie.rs`, and `loot.rs`; they are snapshots that can become stale. Scenery collider declarations are read from `shared::art`. The preview does not repair existing collision mismatches. In particular, the player currently uses a sphere, and some box call sites pass values named half-extents to Avian constructors that expect full lengths.

Next review: judge the three layouts and select sensible dimensions relative to the character. Then author representative replacement assets (character, tree, rock, house, weapon) through Blender scripts and compare them through this same importer. Export compatibility and deterministic generation remain to be proven before replacing the Rust pipeline. A box around a tree canopy or an entire doorway is not automatically a suitable gameplay collider.

## Visual smoke walkthrough

```sh
TERRA_PREVIEW_CAPTURE=1 python3 crates/main/examples/asset_preview_prototype.py
```

This opt-in walkthrough loads every scene, captures inspection, a resized house comparison, and an animated 1 m actor measurement to `/tmp/terra-preview-*.png`, then exits. It exercises actual rendering and dimension controls; it is not a substitute for reviewing every asset, mouse interaction, or animation envelope.
