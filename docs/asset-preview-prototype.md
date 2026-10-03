# Asset scale preview prototype

Question: can a fixed 1 m humanoid reference, dimension guides, and direct baseline comparison expose inconsistent proportions before replacing the asset catalog?

This is throwaway code on the local `prototype/asset-scale-preview` branch. The user accepted the corrected preview as sufficient to proceed; replacement designs and dimensions still require review. The [authoring decision](adr/0001-scripted-asset-refresh.md) records the agreed direction.

Run from the repository root:

```sh
just asset-preview
```

The launcher generates the current Rust catalogs, measures their GLB rest-pose bounds, and starts a separate native Bevy example. No level generation is needed. Generated models remain ignored by Git. The manifest lives in a temporary directory; edits exist only in memory and disappear when the app closes. Running the launcher regenerates the four baseline catalogs, so do not put hand-edited replacement GLBs in those output paths.

Run `just asset-candidates` with Blender MCP connected to generate the [representative asset pilot](blender-pilot.md) in a separate candidate directory. The preview then loads those candidates automatically without modifying their files. **Candidate / baseline** switches the selected source; **Compare** shows original and candidate at the same camera scale. Candidates use their authored meter dimensions directly, and dimension edits are kept separately for each source. The candidate humanoid is rigged; simplified candidate colliders come from the shared dimension contract.

## Controls

- Select an asset from the paginated catalog; all 66 scenes and variants are included.
- `1`, `2`, `3` select inspection, baseline comparison, and measurement. Left/right arrows cycle these layouts. Native controls replace the prototype skill's browser URL switcher because Terra has no browser UI.
- `F` and `S` select orthographic front and side views; `O` selects perspective orbit. Right-drag orbits; middle-drag pans; wheel or zoom buttons change viewing distance.
- Dimension buttons change width, height, or depth by 1 cm or 10 cm. Uniform multipliers change all three together. `R` resets the selected asset. Each asset retains its own edits for the session.
- `A` cycles idle, walk, and attack. Space pauses/resumes. Animation controls affect actor scenes; other scenes remain static. The current catalog uses the same small whole-body rotation for all three named clips; the candidate has distinct scripted skeletal clips and uses its own animation graph.
- `C` toggles visual bounds and the current collider snapshot. Cyan outlines are measurement-only visual bounds; they do not represent collision. Red shows the current runtime collider snapshot, and assets with no collider have no red outline. In comparison mode, red shows the unchanged baseline collider. Red overlays show through surfaces, including the ground, to expose offsets. Gold renders actual candidate Avian colliders; it is not a visual bounding box.

## Interpretation and limits

The reference mannequin is exactly 1 m tall and is a measuring aid, not a proposed character design. Camera scale stays fixed when selecting or resizing assets; manually zoom out for large assets. Comparison uses the same camera for both copies. The grid is 1 m; side rulers adapt their tick interval and label it in meters.

Dimensions come from transformed GLB position-accessor bounds in the resting pose, multiplied by the current runtime visual scale. They are axis-aligned envelopes, not measurements of a posed or deformed animated surface. Models are centered and grounded for inspection. Scenery uses nominal scale 1, excluding the runtime's 0.7–1.3 instance variation. Terrain curvature, foliage shaders, procedural bridges, gameplay camera, and world lighting are outside this first catalog preview.

Runtime scales and collider offsets are deliberately copied into the prototype from `chunks.rs`, `map.rs`, `zombie.rs`, and `loot.rs`; they are snapshots that can become stale. Scenery collider declarations are read from `shared::art`. Baseline mismatches remain visible for comparison. Candidate physics roots use `shared::asset_contract` through the runtime Avian adapter, and imported scenes remain visual children. In particular, the player currently uses a sphere, and some box call sites pass values named half-extents to Avian constructors that expect full lengths.

Next review: judge the representative set on the planet after rigging. The pilot demonstrates compatible, repeatable exports and simplified candidate collision; the full catalog and representative planet approval remain later work. A box around a tree canopy or an entire doorway is not automatically a suitable gameplay collider.

## Visual smoke walkthrough

```sh
TERRA_PREVIEW_CAPTURE=1 just asset-preview
```

This opt-in walkthrough loads every scene and available candidate, captures inspection, a house comparison, the tree bounds/collider distinction, a 1 m actor measurement, and a close house view to `/tmp/terra-preview-*.png`, then exits. With candidates, it compares authored dimensions unchanged; without them, it demonstrates a temporary house resize. The candidate actor plays its own clips and carries the knife on its animated hand socket. Side-view walk/attack captures are included. It exercises actual rendering and dimension controls; it is not a substitute for reviewing every asset, mouse interaction, or animation envelope.

## Scenery family review

All 37 scenery candidates are available through the paged catalog selector; the five original pilot shortcuts remain fixed so the sidebar fits. Use the candidate/baseline toggle and C for measurement and collision overlays. `TERRA_SCENERY_CAPTURE=1 just asset-preview` imports and instantiates every scenery scene, captures it under `/tmp/terra-scenery-review/`, then exits. It does not change the production catalog. See [scenery inventory and review](scenery-review.md) for contact sheets and canonical dimensions.

## Structure family review

The same paged selector includes all 17 structures. `TERRA_STRUCTURE_CAPTURE=1 just asset-preview` captures each candidate; add `TERRA_STRUCTURE_COMPARE=1` for paired baseline views. `just asset-structure-showcase` opens representative buildings and joined modules on actual terrain. See [structure review](structure-review.md) for artifacts and repeat spacing.
