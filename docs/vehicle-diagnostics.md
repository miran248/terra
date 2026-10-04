# Wheel motion and bridge traversal evidence

## Current regression coverage (#30)

The wheel-sign and bridge-winding corrections are implemented. Run the ordinary
regressions with:

```sh
cargo test -p main exploration::diagnostics
cargo test -p shared bridge_slab_faces
```

The wheel test checks all four joints in forward/reverse with neutral, left and
right steering. Bridge tests cover every baked seed-1337 bridge in both endpoint
orders, starting at entry, mid-deck and exit, in forward/reverse gear, from rest
and with initial speed (8 m/s forward, 3 m/s reverse). Each case must exit at least
5 m beyond the span within 5000 updates without substantial deck embedding.
The shared geometry test checks outward top, underside, side-wall and end-cap
normals in both endpoint orders. The experimental winding override is removed.
Bridge HUD clearance casts downward from the player using the existing intersection
API, avoiding the planet-centred ray precision loss exposed at a shared deck edge
by the winding correction. Existing on/above/below-deck and generated-surface HUD
regressions cover this query.

Bridge entrances retain their existing embedded arch geometry and terrain/deck
colliders; no support-ray length, driving model or body-position correction is
introduced. Imported visuals remain separate from gameplay collision. This changes
runtime geometry winding only; baked world data and GLB assets need no regeneration.
Rendered acceptance with the production GLB, scenery and lighting still needs a
manual check of forward/reverse rolling and both bridge approaches and exits.

## Historical diagnostic evidence (#29)

The remainder records the original pre-fix captures and commands. The ignored-test
commands and temporary winding control below describe that historical revision,
not the current regression suite.

Diagnostic prerequisite [#29](https://github.com/miran248/terra/issues/29) for
[the correction decision #30](https://github.com/miran248/terra/issues/30).
Measured on 2026-10-04 against `f444b871f8a55541584d35f31b9d632be9aa901c`.
This records reproductions and controlled experiments, not a production fix.

## Repeat the captures

The opt-in tests in `crates/main/src/exploration/diagnostics.rs` exercise the
production wheel systems and the production Bevy/Avian exploration plugin.
They are ignored in the ordinary suite because the expected behavior still
fails; remove the ignores when implementing the corrections.

```sh
cargo test -p main exploration::diagnostics -- --ignored --nocapture
```

Expected baseline: both tests fail. The wheel test reports all four joints at
both +3 and -3 m/s. The bridge test reports position, velocity, rotation, signed
speed, deck-top clearance, support normal, bridge contacts, and the five support
rays at first embedding. Repeated baseline runs reproduce both symptoms.
The six bridge cases took approximately 3–13 seconds after compilation without
concurrent full-suite work (33 seconds alongside that workload); the wheel
case takes approximately 0.01 seconds. These are headless execution times, not
game frame-time measurements.

Requirements: the pinned Rust 1.99.0 toolchain and existing generated seed-1337
artifact (see [level pipeline](level-pipeline.md)). No world or catalog was
regenerated. SHA-256 of the measured inputs:

| Input | SHA-256 |
| --- | --- |
| `crates/main/assets/level_1337.bin` | `0d9b241d57e01a35356cb434bbbb5b78ec1f74227dd1a83d022018971db082bf` |
| `crates/main/assets/models/production/vehicle.car.glb` | `a2713d04703a4feda6226d033e622097729094a7b615cce288535f147b16f61d` |

## Wheel reproduction

Minimal scene: one car travelling along local -Z, four named wheel joints with
identity rest rotations, no steering, and one 1/60-second animation step.
The real `tag_visuals` and `animate` systems discover and rotate the joints.
The assertion follows a bottom-of-wheel marker: its rotational displacement
must oppose the car's signed travel. This checks rolling direction independently
of the implementation's angle formula.

| Travel | Actual rotation about X | Bottom marker displacement along travel | Expected |
| --- | --- | --- | --- |
| Forward, +3 m/s | +0.166667 rad | +0.165896, all four joints | Negative displacement |
| Reverse, -3 m/s | -0.166667 rad | +0.165896, all four joints | Negative displacement |

Displacement uses a unit-radius marker, not metres travelled by the car.
The GLB JSON confirms identity rotations/scales through the wheel parent chain;
front axle centres are `(±0.771522, 0.31, -0.969397)` metres and rear centres
are `(±0.771522, 0.31, +0.969397)`. Generator wheel geometry lies in the YZ
plane around an X axle. There is no mirrored right-side joint transform that
would explain opposite rotation conventions.

Ranked hypotheses were animation sign, exported axle transform, and signed
heading/velocity mismatch. A temporary change from `part.angle += ...` to
`part.angle -= ...` in `view::animate` made all eight observations pass, with
bottom displacement -0.165896. The production change was then reverted and the
baseline failed again. The reduced fixture has a known heading and velocity,
and the inspected GLB eliminates a rest-transform sign reversal for this asset.
The evidence supports correcting the animation sign. The 0.3 m animation radius
versus roughly 0.31 m authored wheel radius is a separate magnitude detail.

## Bridge reproduction

Seed 1337, `Bridge 2`, endpoint arc distance 442.25 m. Each case uses the real
terrain triangles near the span, `build_bridge_deck_geometry` at 4 m half-width,
the production welded/internal-edge-corrected collider builder, and the real
car collider, driving, attitude and contact systems. The 800 kg car has swept
CCD, locked angular axes, twelve physics substeps, and held W input. The fixture
advances app time by 1/60 second per update and retains the production fixed
schedule. There is no scenery, water resource, renderer, or human input.

Both endpoint orders are exercised, driving forward in each. These are not
reverse-gear bridge runs. The independent starts are 8 m before entry, halfway
along the deck, and 8 m before exit. Each starts stationary 0.48 m above the
higher of terrain or deck. After initial setup, only gameplay systems move the
car. A case succeeds only if it travels 5 m beyond the far endpoint within 1500
updates without its body centre going more than 0.1 m below the deck top.
That threshold catches substantial embedding; it is not a complete test of
wheel clearance or every possible collision penetration.

| Start | Endpoint order | Baseline result | Collider winding reversed |
| --- | --- | --- | --- |
| Entry | Original | Enters, embeds, stalls around 15 m | Crosses and exits |
| Entry | Reversed | Enters, embeds, stalls around 13 m | Crosses and exits |
| Mid-deck | Original | Sinks through top, travels inside slab, fails exit | Crosses and exits |
| Mid-deck | Reversed | Sinks through top, travels inside slab, fails exit | Crosses and exits |
| Exit | Original | Exits without embedding | Exits without embedding |
| Exit | Reversed | Exits without embedding | Exits without embedding |

Independent exit starts sit on the higher terrain near the embedded end; their
success does not show that a car arriving inside the slab can escape it.
The initial survey also reproduced failed complete crossings on all six baked
bridges in both endpoint orders; the committed capture was reduced to Bridge 2.
That survey was diagnostic sampling, not an exhaustive acceptance suite.

Representative baseline observations:

- Original-order entry at update 240: position
  `(-754.0776, 1590.4877, 951.8211)`, progress 15.137 m, signed speed approximately
  zero, body centre 0.358 m below deck top, bridge contact present, support normal
  `(-0.302427, 0.804460, 0.511256)`.
- Reversed-order entry at update 120: position
  `(-1099.9154, 1494.8795, 746.96686)`, progress 13.375 m, signed speed zero,
  body centre 0.366 m below deck top, bridge contact present.
- Starting on mid-deck, both directions reach approximately -0.999 m centre
  clearance by update 120. The slab is 1.4 m thick and the car collision box is
  0.8 m tall: this is consistent with the box resting on the slab's underside
  from inside, rather than being supported by its top.

At first embedding on entry, rear-corner probes still hit the bridge with
radial normals around +0.996 while front probes miss; the original-order centre
probe hits terrain. At first embedding from mid-deck, all five 0.75 m support
rays miss. The trace therefore distinguishes collision contacts and support
sampling from actual top-surface support: neither a contact nor a positive
reported support normal proves the body is above the deck.

## Discriminating experiment

Ranked bridge hypotheses were inward triangle winding, support-ray selection,
and high-speed tunnelling. Every Bridge 2 top triangle has inward geometric
normal: radial dot range `[-0.9999981, -0.9958230]`.
The top-surface list and complete slab come from the same builder. Runtime
rendering uses these triangles with `cull_mode: None`, so a visible deck does not
prove that the physical surface is oriented correctly. The car visual is a
child offset by -0.4 m; its collision is a centred 1.8 × 0.8 × 3.2 m box,
independent of the GLB. No visual/collision placement mismatch is needed to
reproduce the failure.

The test-only control swaps vertices 1 and 2 in every collider triangle, leaving
vertex positions, measured deck-top height, car, support rays, timestep, input,
and CCD unchanged:

```sh
TERRA_DIAGNOSTIC_FLIP_WINDING=1 cargo test -p main baked_bridge_entry -- --ignored --nocapture
```

All six cases pass under that single change. Starting stationary on mid-deck
also reproduces the baseline, so high entry speed is not required. Winding is a
demonstrated cause for this fixture; a longer support cast or different driving
model is not needed to make these six cases pass. This does not establish that
there are no additional support or geometry defects elsewhere.

## Regression outline and follow-up decision

For #30, select the wheel-sign and bridge-winding corrections with these tests
as acceptance cases. Before shipping, promote the diagnostic assertions into
ordinary regressions, remove the experimental winding override, and add a
shared geometry boundary check for outward top/underside/side/end-cap winding.
Run all baked bridges in both endpoint orders and cover nonzero initial speed,
steering and reverse gear. Preserve the existing terrain-edge/crest regressions.

The captures here are numerical and headless. No rendered or manual visual
check was performed. Follow-up visual acceptance must show forward/reverse
wheel rolling, both entrances, deck travel, and both exits with the production
GLB, terrain, scenery and lighting. Generation/schema/catalog contracts and
production behavior remain unchanged by this diagnostic work.

## Verification

`cargo test --workspace` passed: 176 tests, with the two diagnostic reproductions
ignored as documented. Workspace all-targets `cargo check` and `cargo clippy`,
plus `cargo fmt --all -- --check`, passed; existing unrelated warnings remain.
After the review's local query deduplication, the focused captures still fail
on both baseline symptoms and the winding control still passes all six cases.
Standards and spec reviews have no outstanding findings.
