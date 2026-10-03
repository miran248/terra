# Items and actors candidates

Ticket [#20](https://github.com/miran248/terra/issues/20) completes the candidate catalog: nine items and three actors, including the approved knife and player. All 66 original scene identities now have scripted Blender candidates. Human family approval is pending; production activation remains #21.

## Reproduce and inspect

With Blender 5.2.2 and its MCP add-on running:

```sh
just asset-candidates
just asset-candidates-test
just asset-candidates-check
just asset-item-sheets
just asset-preview
just asset-item-showcase
```

The workbench exposes all actors and items in the paged selector. W cycles held weapons, A cycles idle/walk/attack, Space pauses/resumes, and C shows actual candidate physics in gold. Use Compare for the old catalog at its old runtime scales beside the meter-sized candidate. Selecting a weapon itself shows its dropped catalog pose and collider; selecting an actor shows the selected weapon attached to its moving hand.

```sh
TERRA_ITEM_CAPTURE=1 just asset-preview
TERRA_ITEM_CAPTURE=1 TERRA_ITEM_COMPARE=1 just asset-preview
TERRA_ASSET_CAPTURE=1 just asset-item-showcase
```

The first two save nine item views and all nine actor/action combinations under `/tmp/terra-items-review/`, with `-compare` suffixes for comparisons. Blender sheets in the same directory are `terra-items-materials.png`, `terra-items-weapons.png`, and `terra-actors-actors.png`. Materials use 0.25 m reference bars; weapons and actors use 1 m bars. Gun meshes are rotated for legible side silhouettes on the sheet, without rescaling them.

The planet fixture shows three rows: player, zombie.0, zombie.1, each with idle, walk, attack columns. It distributes all five weapons across the nine actors. The front row contains the four materials followed by knife, spear, pistol, sling, rifle. Items retain their upright catalog pose, matching the existing locked-rotation loot presentation. These are visual/physics review instances, not functional pickups or newly enabled enemy/combat systems. Captures are `/tmp/terra-items-gameplay.png`, `terra-items-planet.png`, and `terra-items-planet-colliders.png`.

## Asset contracts

- `items.py` owns eight new item recipes; the approved knife remains in `pilot.py`. Metal offcuts/rivets, bundled boards, a rope coil, folded cloth, spear ferrule/wrapping, gun stocks/grips/sights/trigger guards, and sling pouch/cords distinguish the items without textures.
- `actors.py` specializes the approved anatomy: zombie.0 has sage skin, a clay work jacket and repair patch; zombie.1 has stone skin, a lavender coat, broad collar and diagonal strap. Their heights are .98 m and 1.04 m. The player remains 1 m tall.
- All actors use the same 15-joint skeleton and distinct one-second looping idle/walk/attack clips. Variant mesh and skeleton dimensions are baked together before animation, avoiding inherited-scale ambiguities in glTF skin bounds. Feet are grounded at sampled poses. The collider follows the body core, not animated extremities or weapons.
- Exported `socket.hand` is bone-parented to `hand.right` on every actor. Every weapon exports `socket.grip` at the shared contract's meter coordinate. Blender object names are global, so already-exported owned socket objects are qualified before creating the next scene; files keep the existing per-scene generic socket API.
- `grip_transform` places that anchor at the hand. The default turns authored +Y forward; the sling explicitly retains its vertical orientation so the pouch hangs below the hand. The same transform is used by the workbench and planet fixture. No weapon-specific aim, reload, firing, or two-handed animation has been added; existing actions remain the agreed scope.
- All candidate runtime scales are one. Simplified pickup shapes use grounded small boxes/compounds or the spear shaft capsule. They are physical loot shapes, not damage volumes. Collection distance and gameplay balance remain untouched. The sling's thin cords use a single pickup envelope, not individual cord collision.
- New static items export one mesh primitive each. Actor parts retain their rig weights and separate primitives for now; consolidation and populated-world performance remain the production ticket's responsibility.

## Inventory

Width × height × depth, in meters. Triangle/primitive counts are from exported GLBs.

| Scene | Dimensions | Collider parts | Triangles | Primitives |
| --- | --- | ---: | ---: | ---: |
| `actor.player` | 0.434 × 1.000 × 0.173 | 1 | 788 | 26 |
| `actor.zombie.0` | 0.450 × 0.980 × 0.183 | 1 | 828 | 33 |
| `actor.zombie.1` | 0.470 × 1.040 × 0.195 | 1 | 860 | 32 |
| `material.cloth` | 0.226 × 0.100 × 0.163 | 1 | 72 | 1 |
| `material.metal` | 0.240 × 0.137 × 0.160 | 1 | 76 | 1 |
| `material.rope` | 0.263 × 0.102 × 0.215 | 1 | 600 | 1 |
| `material.wood` | 0.232 × 0.143 × 0.320 | 1 | 180 | 1 |
| `weapon.knife` | 0.065 × 0.285 × 0.026 | 1 | 146 | 8 |
| `weapon.pistol` | 0.053 × 0.250 × 0.150 | 2 | 188 | 1 |
| `weapon.rifle` | 0.052 × 0.680 × 0.180 | 3 | 212 | 1 |
| `weapon.sling` | 0.142 × 0.380 × 0.046 | 1 | 152 | 1 |
| `weapon.spear` | 0.110 × 1.230 × 0.036 | 1 | 344 | 1 |

## Validation and review

- Export coverage tests failed before the eight items/two actors existed, then passed for all nine items and three actors. Actor validation caught inherited fit scaling; baking mesh and skeleton dimensions fixed it. The grip test caught the sling pointing backward instead of hanging, then passed with its explicit orientation.
- All 13 Blender integration tests pass, including full catalog dimensions/collider envelopes, names, grip transforms, skins, loop boundaries, grounded animation envelopes, original-scene preservation, and byte reproducibility/non-writing checks. All 56 previously approved GLB files were compared directly with generation from `ee48608` and remain byte-identical.
- 118 workspace tests and 10 workbench/physics tests pass. Workspace check and Clippy pass with existing unrelated warnings; the final preview Clippy check is clean apart from the existing workspace manifest warning. Formatting and diff checks pass. Logs are under `/tmp/terra-items-*.log`.
- Bevy imported all 12 family scenes. Eighteen individual captures and eighteen baseline comparisons cover every item and every actor/action pair. Manual inspection included the Blender sheets, both new actor variants in motion, held gun/spear examples, the terrain overview and gold collider view. Review caught a stale held-weapon preview after switching W; equipment now participates in the stage rebuild identity, and recaptures show the selected weapon.
- The terrain smoke was captured while Blender rendering and workspace tests were also running, so its FPS is not a useful performance benchmark. Full populated-world performance remains #21. Current game lighting is unchanged from the approved representative review.
- Standards review: no remaining findings against the root/subtree contracts. Spec review: technical deliverables and review artifacts are complete; human family acceptance is still required before closure. The two passes were performed separately in the main session, honoring the request not to delegate.

After the family verdict, #21 can consolidate compatible primitives, activate the complete catalog and meter/collider contracts in production, and perform final runtime/performance/visual acceptance.
