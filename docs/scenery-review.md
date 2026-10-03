# Scenery candidates and review

Ticket [#18](https://github.com/miran248/terra/issues/18) extends the approved representative set to every scenery scene. These are review candidates; production selection, placement, density, and wind remain unchanged. The family verdict is still pending.

## Reproduce and inspect

With Blender 5.2.2 and its MCP add-on running:

```sh
just asset-candidates
just asset-candidates-test
just asset-candidates-check
just asset-scenery-sheets
just asset-preview
```

`asset-scenery-sheets` produces `/tmp/terra-scenery-review/terra-scenery-trees.png` and `terra-scenery-ground.png`. Every object uses the same meter scale within its sheet; the dark bars are 1 m, and labels give width × height × depth. The sheets use neutral Blender lighting to expose form; they are not in-game lighting evidence. The interactive Bevy preview provides the actual exported mesh, baseline comparison, dimension controls, and gold Avian collider overlays. `TERRA_SCENERY_CAPTURE=1 just asset-preview` captures all 37 imported scenes individually in the same directory.

## Authoring and runtime contract

- Recipes: `crates/gen_assets/blender/scenery.py`; approved representative recipes remain in `pilot.py`. Source uses Z-up and the existing reflection/export helpers; output is grounded, Y-up, texture-free GLB with flat normals, linear vertex colors, and roughness .95.
- Canonical dimensions and simplified collision parts are in `crates/shared/asset_dimensions.json`. Export fitting uses those dimensions and the manifest records actual bounds. New static recipes join compatible parts into one primitive; original pilot tree/rock retain their approved geometry and part names.
- The seven tree indices retain deciduous, jungle, pine, palm, oak, sparse, and autumn silhouettes. Cactus 1 retains the barrel shape; dead tree 1 retains snow. Existing `scenery_variant_for` terrain/hash selection is unchanged, including its existing broad terrain ranges. All selected names resolve to candidates.
- Authored size uses runtime scale 1. Production's existing positional-hash variation is 0.7–1.3; collision adapter tests cover both endpoints and the nominal size for every tree. Production activation must apply the same instance scale to visuals and candidate colliders. The old production collider adapter is deliberately untouched until #21.
- Living/dead trees use trunk-only cylinders; rocks, logs, stumps, the snowdrift and snowman use compact solid cores. Leaves, bushes, flowers, cacti, and small decorative props remain nonblocking, preserving the existing absence of gameplay collision for these kinds. No thorn damage or new interaction is introduced.
- Ground-level pivots remain the existing placement convention even for hanging-looking icicles and aquatic scenery; changing terrain placement behavior is outside this refresh.

## Coverage

Dimensions are meters at nominal scale. Physical shapes may omit thin branches, leaves, and small detail. Export tests compare every scenery name against the baseline catalog, check ground pivots, dimensions, materials, collision envelopes, primitive budgets, and deterministic bytes.

| Scene | W × H × D (m) | Collision parts | Triangles | Primitives |
| --- | --- | --- | ---: | ---: |
| `scenery.anemone` | 0.44 × 0.30 × 0.42 | none | 820 | 1 |
| `scenery.berry` | 0.82 × 0.57 × 0.80 | none | 1100 | 1 |
| `scenery.bush.0` | 0.85 × 0.58 × 0.78 | none | 320 | 1 |
| `scenery.bush.1` | 0.72 × 0.66 × 0.70 | none | 320 | 1 |
| `scenery.cactus.0` | 0.59 × 1.20 × 0.23 | none | 298 | 1 |
| `scenery.cactus.1` | 0.55 × 0.66 × 0.53 | none | 600 | 1 |
| `scenery.cattail` | 0.55 × 0.85 × 0.50 | none | 220 | 1 |
| `scenery.coral` | 0.63 × 0.55 × 0.45 | none | 180 | 1 |
| `scenery.dead_tree.0` | 1.02 × 1.68 × 0.94 | cylinder | 228 | 1 |
| `scenery.dead_tree.1` | 0.95 × 1.44 × 0.88 | cylinder | 328 | 1 |
| `scenery.fern` | 0.92 × 0.37 × 0.87 | none | 644 | 1 |
| `scenery.flower` | 0.37 × 0.43 × 0.35 | none | 360 | 1 |
| `scenery.grass` | 0.43 × 0.28 × 0.43 | none | 56 | 1 |
| `scenery.icicle` | 0.38 × 0.50 × 0.10 | none | 80 | 1 |
| `scenery.kelp` | 0.65 × 1.25 × 0.52 | none | 160 | 1 |
| `scenery.lilypad` | 0.56 × 0.11 × 0.48 | none | 68 | 1 |
| `scenery.log` | 1.13 × 0.42 × 0.25 | box | 232 | 1 |
| `scenery.mushroom` | 0.38 × 0.28 × 0.30 | none | 690 | 1 |
| `scenery.reed` | 0.56 × 0.74 × 0.50 | none | 100 | 1 |
| `scenery.rock.0` | 0.82 × 0.49 × 0.62 | box, box | 68 | 2 |
| `scenery.rock.1` | 0.76 × 0.60 × 0.57 | box, box | 68 | 1 |
| `scenery.seaweed` | 0.46 × 0.52 × 0.42 | none | 40 | 1 |
| `scenery.shell` | 0.40 × 0.06 × 0.35 | none | 72 | 1 |
| `scenery.skull` | 0.43 × 0.26 × 0.30 | none | 160 | 1 |
| `scenery.snowdrift` | 1.24 × 0.31 × 0.68 | box | 68 | 1 |
| `scenery.snowman` | 0.77 × 1.09 × 0.50 | capsule | 316 | 1 |
| `scenery.starfish` | 0.43 × 0.06 × 0.43 | none | 92 | 1 |
| `scenery.stump` | 0.54 × 0.33 × 0.50 | cylinder | 132 | 1 |
| `scenery.tree.0` | 1.86 × 2.63 × 1.29 | cylinder | 304 | 12 |
| `scenery.tree.1` | 2.05 × 3.10 × 1.80 | cylinder | 432 | 1 |
| `scenery.tree.2` | 1.45 × 3.00 × 1.45 | cylinder | 312 | 1 |
| `scenery.tree.3` | 2.05 × 2.70 × 2.00 | cylinder | 712 | 1 |
| `scenery.tree.4` | 2.65 × 2.60 × 2.20 | cylinder | 432 | 1 |
| `scenery.tree.5` | 1.50 × 2.35 × 1.30 | cylinder | 560 | 1 |
| `scenery.tree.6` | 1.90 × 2.63 × 1.65 | cylinder | 432 | 1 |
| `scenery.tumbleweed` | 0.51 × 0.50 × 0.50 | none | 540 | 1 |
| `scenery.vine` | 0.72 × 0.78 × 0.18 | none | 160 | 1 |

## Verification and review evidence

- Red/green export coverage: initially only two scenery scenes existed; all 37 now pass. Primitive-budget checks initially failed for the unjoined recipes; all 35 added scenes now use one primitive. A short-barrel silhouette check caught the wrong cactus variant and passes after correction.
- Empty collider lists initially panicked in Avian's compound constructor; the adapter now returns `None`. Physical point/contact checks cover trunk clearance and scale 0.7, 1, and 1.3.
- Workspace: 115 tests passed; preview: 8; Blender MCP integration: 9. Workspace check and Clippy pass with existing unrelated warnings. Repeated candidate exports, including the manifest, are byte-identical; non-writing corruption detection and original Blender scene preservation pass.
- All 37 candidate GLBs were imported, instantiated, and captured in Bevy. Both contact sheets were inspected, along with representative Bevy tree, fern, log, and cactus views. The review found and corrected contact-sheet framing, disconnected cactus detail, a boxy log, and preview shortcut overflow. This is a visual/import smoke review, not a populated-world performance test.
- Standards review against the root and subtree contracts: no remaining findings. Spec review against #18: implementation and artifacts complete; actual human family approval remains pending. Reviews were performed separately in the main session, following the user's request to continue here rather than delegate.

The next ticket (#19, structures) remains dependent on the human scenery verdict. Full-world cutover and population-level performance validation remain #21.
