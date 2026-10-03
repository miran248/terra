# Structure candidates and review

Ticket [#19](https://github.com/miran248/terra/issues/19) extends the approved house construction style to all 17 structure scenes. The house is retained; the other 16 scenes add readable joinery, masonry courses, framing, planks, roof slabs, and hardware. The family verdict is pending. Production activation remains #21.

## Reproduce and inspect

With Blender 5.2.2 and its MCP add-on running:

```sh
just asset-candidates
just asset-candidates-test
just asset-candidates-check
just asset-structure-sheets
just asset-preview
just asset-structure-showcase
```

The two labeled Blender sheets are `/tmp/terra-structures-review/terra-structures-buildings.png` and `terra-structures-props.png`. They use consistent meters within each sheet and 1 m reference bars. The existing Bevy preview exposes every scene through its paged selector, with baseline comparison, dimension controls, and real gold collider outlines (C).

```sh
TERRA_STRUCTURE_CAPTURE=1 just asset-preview
TERRA_STRUCTURE_CAPTURE=1 TERRA_STRUCTURE_COMPARE=1 just asset-preview
TERRA_ASSET_CAPTURE=1 just asset-structure-showcase
```

The first two commands save all 17 individual or baseline-comparison captures under `/tmp/terra-structures-review/`. The terrain walkthrough saves `/tmp/terra-structures-gameplay.png`, `terra-structures-planet.png`, `terra-structures-planet-colliders.png`, and `terra-structures-repeats.png`. It includes the house, watchtower, well, tent, dock modules and connected fence/wall/railing/guardrail examples beside 1 m animated characters on the displaced settlement terrain. The normal interactive recipe retains movement and the C overlay toggle. The structure terrain mode uses candidates; use the workbench for baseline comparisons.

## Dimensions, placement, and physics

- Recipes live in `crates/gen_assets/blender/structures.py`, using the same palette, modeling helpers and deterministic MCP exporter as the approved set. All 16 additional scenes batch compatible static parts into one primitive. Scene, primary mesh, and material identifiers retain their canonical names.
- `crates/shared/asset_dimensions.json` owns nominal meter dimensions and collision parts. New candidates import at scale 1; **do not apply the old `chunks::spawn_structure` multiplier to them**. The production multiplier remains untouched until cutover. The preview deliberately applies old multipliers to baseline models for an honest comparison.
- All candidate horizontal envelopes fit within the planner's existing reserved structure footprints. Shrinking authored assets therefore does not require new settlement sites, road routes, density or level data for this review. Reconciliation at #21 must retain radial placement and yaw, replace the old multipliers/collider adapter, and assess the resulting settlement spacing; planner changes are not silently bundled here.
- Ground pivots remain Y=0, including the bottom of dock piles. The dock's walking surface is about .45 m above that pivot. On an actual waterfront, placement must anchor piles to the support surface; this terrain review demonstrates modules on land and does not invent a shoreline placement system.
- Well walls leave the center open. Watchtower legs, braces, deck, rails and roof use separate simple shapes rather than one solid tower box. The tent has two sloped roof shapes, a shallow floor and a stepped rear panel, leaving its entrance/interior open. Ruin collision follows the stepped courses rather than filling missing masonry. Small trim and fittings are omitted from the collision surface.
- The farm and campfire remain nonblocking; there is no new crop interaction, heat damage, ladder climbing, lamp lighting system, door behavior or UI redesign. Roof/glass/flame colors remain rough, texture-free vertex-color materials.

## Repeated pieces and bridges

`repeat_step` is an optional local-space meter vector in the shared contract and export manifest. Matching `<scene>.socket.repeat.start` and `.end` nodes are exported in each repeatable GLB. Translating a copy by `repeat_step` aligns its start with the previous end. Fences, walls, railings, guardrails and suspension frames repeat along local X at 2 m; dock decks repeat along local Z at 2.4 m. Exported bounds/socket transforms and actual Avian contact across the join are checked.

These are catalog modules, **not generated bridge decks**. The suspension scene is a decorative pylon/cable frame. Runtime bridge decks and their terrain-dependent span/trimesh generation remain independent and unchanged. Existing world generation scatters bridge decorations; it does not yet use these sockets to construct bridge spans. The repeated strips in the review are explicit examples, not a new world-generation algorithm.

## Inventory

Dimensions are width × height × depth in meters; collider counts reflect compound parts. The house retains its approved multi-part authoring mesh pending the planned production consolidation.

| Scene | W × H × D | Collider parts | Repeat displacement | Triangles | Primitives |
| --- | --- | ---: | --- | ---: | ---: |
| `structure.barricade` | 1.70 × 0.72 × 0.44 | 3 | none | 168 | 1 |
| `structure.campfire` | 0.73 × 0.46 × 0.71 | 0 | none | 358 | 1 |
| `structure.crate` | 0.62 × 0.54 × 0.57 | 1 | none | 312 | 1 |
| `structure.dock` | 1.40 × 0.45 × 2.40 | 5 | [0, 0, 2.4] | 528 | 1 |
| `structure.farm` | 2.40 × 0.36 × 2.40 | 0 | none | 732 | 1 |
| `structure.fence` | 2.00 × 0.75 × 0.14 | 4 | [2, 0, 0] | 108 | 1 |
| `structure.guardrail` | 2.00 × 0.60 × 0.16 | 4 | [2, 0, 0] | 96 | 1 |
| `structure.house` | 3.03 × 2.49 × 2.51 | 10 | none | 890 | 76 |
| `structure.lamp_post` | 0.55 × 1.55 × 0.26 | 4 | none | 152 | 1 |
| `structure.railing` | 2.00 × 0.65 × 0.12 | 4 | [2, 0, 0] | 180 | 1 |
| `structure.ruin` | 2.00 × 1.35 × 1.65 | 9 | none | 324 | 1 |
| `structure.signpost` | 0.80 × 1.20 × 0.11 | 2 | none | 90 | 1 |
| `structure.suspension` | 2.00 × 2.40 × 0.28 | 3 | [2, 0, 0] | 168 | 1 |
| `structure.tent` | 1.65 × 1.35 × 1.85 | 6 | none | 171 | 1 |
| `structure.wall` | 2.00 × 1.05 × 0.32 | 1 | [2, 0, 0] | 336 | 1 |
| `structure.watchtower` | 1.90 × 3.65 × 1.90 | 15 | none | 660 | 1 |
| `structure.well` | 1.20 × 1.55 × 0.98 | 12 | none | 516 | 1 |

## Validation and review

- Export coverage failed with only the house present, then passed for all 17 canonical structure names. Repeated-socket checks failed before sockets were exported, then passed for actual glTF world transforms, bounds, and fixed nominal spacing. Structure materials, grounded pivots, collision envelopes (including rotated roofs), and one-primitive budgets are checked through exported files.
- Actual Avian checks cover open interiors, stepped ruin clearance, narrow lamp shafts, solid deck/roof surfaces, and contact between adjacent repeated modules. Existing player/tree ground and clearance checks remain green.
- 117 workspace tests, 10 preview tests, and 11 Blender integration tests passed. Final collision refinements were rechecked with focused Rust/export tests. Workspace check/Clippy and formatting pass with existing unrelated warnings. Candidate files/manifest reproduce byte-for-byte; baseline catalogs also still match their unchanged generator.
- All 17 candidates were imported, instantiated and captured in Bevy, including baseline comparisons. Both Blender contact sheets and representative Bevy well/fence/tower/tent views were inspected. The actual-planet overview, collision overlay, and connected-module view were also inspected. A roughly 121 FPS display-limited debug smoke sample is not a populated-world performance guarantee; that remains #21.
- Standards review: no remaining findings against the root/subtree contracts. Spec review: implementation and review artifacts complete; human family acceptance remains pending. Reviews were performed separately in the main session per the user's request to continue here rather than delegate.

The next implementation ticket is #20 (remaining items and actors), after the structure-family verdict required by #19.
