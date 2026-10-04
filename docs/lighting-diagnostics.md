# Sunlight, water, shadows, and rendering baseline (#31)

Diagnostic evidence for [#31](https://github.com/miran248/terra/issues/31), supporting the lighting decision in [#32](https://github.com/miran248/terra/issues/32). This work adds an opt-in capture fixture; it does not change production shaders, materials, sunlight, or world data. Measurements were made on 2026-10-04.

## Reproduce

From the repository root, with seed 1337 and the production catalog already generated:

```sh
scripts/capture_lighting.sh /tmp/terra-lighting-new
```

The output directory must be new and outside the checkout, with an existing parent directory; the script rejects an in-checkout destination so evidence cannot hash itself. The script builds the pinned toolchain's optimized development profile, opens the actual game renderer, captures all five scenes at noon/sunset/night, then runs isolated diagnostic overrides. It sets `BEVY_ASSET_ROOT` for direct binary execution, preserves logs, the Git revision, worktree status/diff, source hashes, and hashes of all runtime assets (including ignored generated GLBs), and exits automatically. Allow several minutes; do not run builds or other benchmarks concurrently with capture.

For one shot:

```sh
TERRA_LIGHTING_CAPTURE=/tmp/terra-lighting-one \
TERRA_LIGHTING_SCENE=dense-transition TERRA_LIGHTING_PHASE=noon \
cargo run --locked -p main --features asset-review
```

`TERRA_LIGHTING_PROBE` accepts `baseline` (default), `shadows`, `no-fog`, `no-atmosphere`, `no-ambient`, `exposure`, `rough-water`, or `flat-water`. These overrides exist only in the explicitly enabled review fixture. Each process changes one factor; `flat-water` disables both geometric and normal waves as one combined wave-isolation experiment, so it cannot distinguish those two effects.

The fixture freezes virtual time at zero, holds calm weather, parks the explorer through Avian, mirrors the parked transform because no fixed physics steps run, and overrides the camera before transform propagation. The gameplay HUD has time-gated labels and can retain the initial location/hour while time is frozen; use the recorded anchor and solar elevation, not the HUD label. The parked explorer is not an animation or collision acceptance fixture. Real frame intervals remain unmodified. After eight seconds of settling, it records approximately five seconds of `Time<Real>` frame intervals, then captures a PNG outside the measurement interval. Per-shot CSVs contain one interval in milliseconds per line, in acquisition order. `measurements.txt` includes physical/logical resolution, presentation mode, full camera transform, anchor, solar elevation, actual light direction, illuminance, ambient, exposure, visible mesh count (across views, not draw calls), light counts, and percentiles. Percentiles use the nearest rounded index of the sorted sample.

These are stationary rendering baselines, not driving/flight throughput, GPU timestamps, or a maximum-quality shipping budget. Physics and animation are paused; rendering, streaming, UI, and the minimap remain active. Scene changes can retain different LOD histories. Startup, first pipeline compilation, teleport transients, and screenshot readback are excluded from the steady sample. Moving-water temporal artifacts and long-session frame pacing remain unmeasured.

## Machine and controls

- MacBook Pro, Apple M4 Pro, 14 CPU cores, 20 GPU cores, 24 GB memory; macOS 15.7.7 (24G720), Metal backend confirmed by renderer logs.
- Default play window: **2560 × 1440 physical**, 1280 × 720 logical, scale factor 2, `Fifo` presentation. Built-in display is 3024 × 1964. This is the default window used in this session; a separately maximized user play window was not measured.
- Bevy 0.19.1, Avian 0.7.0, Rust 1.99.0; workspace development optimization level 1, dependencies level 3. The local Cargo config adds debug information and disables incremental compilation. No release-build performance claim.
- HDR, TonyMcMapface, TAA, MSAA off, deband dither disabled; bloom intensity .18, threshold .7. Default camera exposure is EV100 9.7. No environment-map lighting.
- Directional illuminance stays at 13,000 lux at every solar angle. Global ambient is `35 + 130 * max(dot(camera_up, sun_dir), 0)` with a cool blue color. Fog visibility is 1700 m; its color also follows camera illumination. Atmosphere radii 1976/2900 m, density multiplier 50, aerial LUT distance 2500 m, dimensions 24 × 24 × 32, 24 aerial samples and 16 sky samples.
- Seed-1337 artifact SHA-256: `0d9b241d57e01a35356cb434bbbb5b78ec1f74227dd1a83d022018971db082bf`. Cargo.lock SHA-256: `4195e9b511f9c5759af3b33479522ebab766cb8398c9201ee875a9f3e25b6b3a`.

Scene selection is deterministic from the baked level. Dense transition is face 58000, the dry mixed-corner face with the most scenery on that face (52 placements; lowest face index breaks ties). Bridge is Bridge 1 at its first entrance, looking along its span. Shoreline is liquid-water face 264699 nearest Ashford, looking away from the settlement across water. Settlement looks toward the nearest house to Ashford. Overview uses the Ashford radial direction at radius 6500 m, looking at the origin. Full transforms are recorded with each capture.

“Noon” is the highest sun on the game's fixed-declination orbit, not an overhead sun-lock. “Sunset” solves the local horizon equation using the existing 0.35 axial tilt; “night” is the opposite solar longitude. These selected scenes have both sunrise and sunset; this solver is not a polar-day/night fixture.

## Observations

### Direction and normals

The actual directional-light forward vector has dot product **−1.000000** with `TimeOfDay.sun_dir` in the captures: rays travel from the sun toward the planet. Its translation at radius 800 is inside the 2000 m planet, but a directional light uses rotation for ray direction; that translation is not a point-light origin. No evidence supports globally reversing the sun transform. Bevy’s PBR shader also multiplies direct sunlight by atmospheric transmittance and sun visibility; 13,000 configured lux does not mean 13,000 unattenuated lux reaches each surface.

All 327,680 baked terrain triangles have finite outward normals; the minimum face-normal/radial dot product is .27122882, with zero inward or nonfinite faces. The audit uses the same winding as `map::build_visual_mesh`. It rules out a globally inverted terrain mesh, not every possible imported-asset or interpolation error.

Sea/lake mesh normals are radial in `water::subdivide_water_tri`. Water shading perturbs `pbr_input.N` only in its tangent plane, then normalizes it. That construction preserves the outward hemisphere. The geometric swell's analytic gradient is subtracted from the bump; normal ripples and geometry therefore share the intended gradient sign. The gradient is evaluated at the displaced fragment position, so this is not proof of exact normal/geometry agreement everywhere. Camera-dependent specular response must not be interpreted as the direction of diffuse illumination.

### Missing shadows

The live sun has **`shadow_maps_enabled=false`**, inherited from Bevy's default. Contact shadows are also disabled. The initial rendered tree view has no projected tree shadow on the ground. A direct assertion on the captured live setting failed as expected:

```text
AssertionError: MISSING SUN SHADOWS: live directional light has shadow_maps_enabled=false
```

The red-capable configuration check is:

```sh
python3 - /tmp/terra-lighting-new/baseline/measurements.txt <<'PY'
from pathlib import Path
import sys
rows = Path(sys.argv[1]).read_text().splitlines()
assert rows and all('shadows=true' in row for row in rows), \
    'MISSING SUN SHADOWS: live directional light has shadow_maps_enabled=false'
PY
```

Run it against the `shadows` probe's measurements to check the enabled case. This is a configuration diagnosis paired with rendered A/B inspection, not a pixel-level shadow acceptance test.

The unmodified defaults, if enabled, are four cascades, a 10 m first bound, 150 m maximum distance, 2048-pixel maps, .02 depth bias and 1.8 normal bias. These are defaults, not an approved Terra shadow budget. Terrain meshes can cast; liquid water and ice carry `NotShadowCaster`. Scenery places that marker on a transform root, while the Bevy shadow visibility query checks mesh entities; root placement alone must not be assumed to exclude imported mesh children. Validate caster membership at the actual mesh ownership boundary when designing selective shadows.

### Materials, exposure, and atmosphere

The representative production tree, house, car, and explorer GLBs have vertex colors and normals, metallic factor 0, roughness .95, and double-sided materials. They are texture-free. Terrain uses white base color with baked vertex colors, roughness .95 and reflectance .02. Water uses roughness .08, reflectance .5, alpha blending and no face culling, with shader-authored linear shallow/deep colors and view-angle-dependent color/opacity.

Noon captures show pale lit foliage and dark underside faces; night captures show silhouettes with little ground detail. Those observations alone do not establish a broken PBR normal. A single camera exposure and a weak cool ambient floor produce a large contrast between sunlit and unlit geometry. Fog and atmospheric in-scattering can further lift and desaturate distant surfaces. The isolated probes below distinguish their contributions; their settings are diagnostic extremes, not proposed production defaults.

### Shader and prepass behavior

TAA requires depth and motion-vector prepasses. Foliage supplies a custom prepass vertex shader; its `foliage_offset` implementation matches the main pass after comments are removed. It reuses the current offset for the previous position, deliberately omitting sway velocity from the motion vector. That preserves depth geometry parity but is not a complete motion-vector solution for animated foliage.

Alpha-blended water is outside the opaque prepass. Its fragment shader reads opaque scene depth to estimate water thickness, interpolates shallow/deep color and alpha, adds Fresnel brightening and shoreline foam, then calls standard PBR lighting and post-lighting processing. Thus the visible water is a combination of shading, underlying terrain, and atmosphere/fog. Water has no own depth/motion-vector history in this path. Frozen captures cannot determine whether moving-wave TAA artifacts contribute to the reported appearance.

### Night navigation

The live captures contain **zero point lights and zero spotlights**. The requested “settlement at night with headlights” is therefore only a settlement-night baseline: headlights and their shadow cost are unavailable in the current runtime. No synthetic headlights were added to make the scene appear complete. Dark wilderness is an agreed design choice; readable settlement/vehicle navigation requires the separately planned local lighting.

## Measurements and isolated probes

| Scene / phase | Samples | Median ms | p95 ms | p99 ms | Max ms | Frames >33.33 ms |
|---|---:|---:|---:|---:|---:|---:|
| dense-transition/noon | 398 | 15.546 | 18.855 | 19.199 | 25.286 | 0 |
| dense-transition/sunset | 394 | 16.230 | 18.775 | 23.352 | 48.078 | 1 |
| dense-transition/night | 399 | 15.832 | 18.864 | 19.424 | 23.614 | 0 |
| bridge/noon | 505 | 8.589 | 16.665 | 16.883 | 17.515 | 0 |
| bridge/sunset | 499 | 8.624 | 16.751 | 17.026 | 18.686 | 0 |
| bridge/night | 500 | 8.617 | 16.670 | 17.183 | 21.697 | 0 |
| shoreline/noon | 562 | 8.563 | 15.795 | 16.517 | 17.160 | 0 |
| shoreline/sunset | 555 | 8.594 | 16.265 | 16.955 | 18.561 | 0 |
| shoreline/night | 564 | 8.588 | 15.503 | 16.816 | 19.316 | 0 |
| settlement/noon | 598 | 8.337 | 8.792 | 10.210 | 15.861 | 0 |
| settlement/sunset | 597 | 8.342 | 8.936 | 10.705 | 16.348 | 0 |
| settlement/night | 599 | 8.336 | 8.772 | 9.375 | 12.404 | 0 |
| overview/noon | 306 | 16.621 | 17.818 | 22.203 | 25.975 | 0 |
| overview/sunset | 305 | 16.626 | 18.971 | 24.726 | 27.045 | 0 |
| overview/night | 307 | 16.611 | 17.408 | 19.478 | 23.862 | 0 |

Probe measurements (single stationary samples, not paired statistical cost estimates):

| Probe | Scene / phase | Median ms | p95 ms | Max ms | Frames >33.33 ms |
|---|---|---:|---:|---:|---:|
| no-fog | dense-transition/noon | 15.073 | 18.855 | 26.252 | 0 |
| exposure | dense-transition/noon | 14.555 | 18.841 | 20.463 | 0 |
| shadows | dense-transition/noon | 15.177 | 18.988 | 24.333 | 0 |
| water-no-fog | shoreline/sunset | 8.547 | 15.777 | 17.375 | 0 |
| water-rough-water | shoreline/sunset | 8.508 | 16.237 | 50.246 | 1 |
| no-ambient | dense-transition/noon | 15.352 | 18.772 | 24.187 | 0 |
| water-flat-water | shoreline/sunset | 8.503 | 15.289 | 17.961 | 0 |
| no-atmosphere | dense-transition/noon | 13.088 | 17.699 | 22.468 | 0 |
| water-no-atmosphere | shoreline/sunset | 8.262 | 10.126 | 18.155 | 0 |
| overview-no-atmosphere | overview/noon | 16.572 | 17.008 | 27.996 | 0 |
| overview-no-fog | overview/noon | 16.614 | 17.062 | 28.125 | 0 |
| repeat-1 | dense-transition/sunset | 14.393 | 18.855 | 25.508 | 0 |
| repeat-2 | dense-transition/sunset | 16.042 | 18.757 | 25.888 | 0 |

The dense-transition sunset sample contains one 48.078 ms interval, preceded/followed by 18.320/8.214 ms. Two independent reruns of that exact camera/solar state had maxima 25.508 and 25.888 ms, with zero intervals above 33.33 ms. The hitch did not recur in those reruns; no CPU/GPU trace was captured, so its cause is **unlocalized**, not attributed to a shader or dismissed as harmless. Dense-transition p95 remains about 18.8 ms, so these results do not establish a consistent 60 FPS budget. No sustained sub-30 FPS interval was observed in the five-second steady windows. Use a longer live route and CPU/GPU profiling before allocating the remaining budget.

A separate, unfrozen gameplay smoke capture at the spawn settlement (existing `TERRA_PRODUCTION_CAPTURE=1 TERRA_PRODUCTION_FIRST_ONLY=1 TERRA_CAPTURE_STILL=1 cargo run -p main --features asset-review`) measured p50 8.313 ms and p95 9.628 ms with physics active. This is one stationary gameplay sample, not validation of all five moving routes. Log: `/tmp/terra31-initial.log`.

### Hypotheses tested and outcomes

Ranked before probing: (1) disabled shadows explain missing projected shadows; (2) exposure contributes to pale highlights; (3) fog/atmosphere contributes to surface contrast and haze; (4) water waves/roughness contribute to apparent direction. The later dark-overview capture narrowed the third hypothesis to a direct fog-versus-atmosphere comparison.

| Isolated change | Observed result | What the evidence supports |
|---|---|---|
| Sun shadow maps enabled | [Tree self-shadows and projected ground shadows appear](diagnostics/lighting-31/shadows/dense-transition-noon.png); compare [baseline](diagnostics/lighting-31/baseline/dense-transition-noon.png). | Disabled shadow maps cause the missing projected shadows. All-size scenery also casts in this probe; this is not selective-caster acceptance. |
| EV100 9.7 → 12 | [Lit canopy becomes darker](diagnostics/lighting-31/exposure/dense-transition-noon.png); shaded regions also darken. | Exposure is a strong lever, but globally darkening the image does not resolve already dark surfaces. No preferred exposure is selected. |
| Ambient brightness → 0 | Lit canopy changes little; the underside loses some blue fill. | Ambient is not the dominant cause of the pale sunlit canopy in this scene. This does not select the nighttime ambient floor. |
| Fog effectively disabled at ground level | Distant hill loses some blue haze; near canopy changes little. | Fog contributes chiefly to distant appearance in this ground shot. |
| Fog effectively disabled in overview | [Sunlit planet and water colors become visible](diagnostics/lighting-31/overview-no-fog/overview-noon.png), versus the [nearly black baseline](diagnostics/lighting-31/baseline/overview-noon.png), with atmosphere retained. | The current 1700 m distance-fog treatment causes the dark overview. A 6500 m-radius camera views the nearest ground from about 4500 m away. |
| Atmosphere disabled before first render extraction | Ground-level sky disappears and surface shading changes modestly; overview remains dark with distance fog active. | Atmosphere affects light transport/sky, but removing it alone does not fix the dark overview. |
| Water roughness .08 → .5 | Sunset mottling remains, with altered contrast/color. | Roughness alone does not remove the water shading pattern. |
| Geometric swell and normal ripples → 0 | [Mottled cyan/green wave highlights disappear](diagnostics/lighting-31/water-flat-water/shoreline-sunset.png); compare [baseline](diagnostics/lighting-31/baseline/shoreline-sunset.png). | The pattern is tied to the wave path. The combined probe does not distinguish geometry, perturbed normals, or the resulting depth/opacity changes. It is not proof of an inverted normal. |

For a reproducible color comparison, the lit-canopy rectangle `(1380,570)–(1430,610)` in the original 2560 × 1440 PNG has mean 8-bit sRGB `(145.67,166.44,150.28)` at baseline, `(78.32,92.38,83.27)` at EV12, `(145.46,165.46,147.41)` without ambient, and `(145.80,166.41,149.41)` without fog. These are output-pixel statistics, not radiometric measurements or an artist-approved reference. [Pixel samples](diagnostics/lighting-31/pixel-samples.txt) also record a fixed water rectangle.

The shoreline sunset camera faces partly away from the sun (camera-forward dot sun direction −.465), while tilted wave normals still produce diffuse patches. The captures do not demonstrate a consistent reversal of illumination. A specific wrong-direction specular symptom remains unconfirmed; preserve the current ray-direction convention until a more specific counterexample exists.

### Evidence inventory

[Recorded evidence](diagnostics/lighting-31/) contains measurements and raw frame intervals for all 28 accepted shots, normal audits, scene selections, input hashes, and nine representative unmodified PNGs. `input-hashes.txt` was recorded for the accepted runs but omits generated GLBs and dirty source state. `asset-hashes-after-capture.txt` inventories the unchanged local assets after review; it is not capture-time proof. The final script records those missing inputs for future runs. The initial baseline preceded the corrected atmosphere teardown, and a later match-guard refactor was behavior-preserving; the original source snapshot for every run was not retained. The full 15-image baseline and initial probes are local at `/tmp/terra31-reviewed`; corrected atmosphere probes, overview isolation, and hitch reruns are at `/tmp/terra31-corrected`. Reproduction generates every PNG again. The committed selections include all five scene categories and the important A/B comparisons; night navigation is visible in the [settlement-night capture](diagnostics/lighting-31/baseline/settlement-night.png).

Excluded attempts: an initial shoreline camera faced inland and was corrected before the accepted series. Removing only `AtmosphereSettings` left atmosphere-dependent mesh lighting active and produced an invalid comparison; live removal of the atmosphere then triggered a Bevy render-resource panic. The accepted no-atmosphere probe removes both at startup, before render extraction. Neither failed experiment is used in the tables. The successful accepted runs contain no asset-load or shader-validation errors.


## Design handoff and remaining uncertainty

The evidence is sufficient for #32 to select a bounded correction strategy, with these constraints:

1. Keep the established world-space sun ray convention and outward terrain/water normals. Do not “fix” the apparent direction by reversing the light or mesh winding.
2. Enable and budget nearby substantial-geometry shadows explicitly. The dense-scene default-cascade probe has p95 18.988 ms and maximum 24.333 ms; it fits 30 FPS in that short frozen sample but does not establish consistent 60 FPS with gameplay/headlights. Choose caster categories at mesh entities, cascade distances and bias in #32, then measure again on live routes.
3. Give ground views and orbital views an intentional fog policy. The overview A/B makes distance fog a confirmed correction target. Evaluate fog and atmospheric aerial perspective together instead of assuming that removing atmosphere repairs it.
4. Tune exposure, ambient fill and water response together under the same noon/sunset/night cameras, preserving the current material palette. EV12 is diagnostic, not a proposed default. Separate water geometric swell, normal perturbation, Fresnel, depth blending and specular response in any follow-up that targets the remaining apparent-direction complaint.
5. Preserve foliage main/prepass displacement parity and explicitly decide how moving foliage/water should interact with temporal accumulation. This frozen baseline cannot approve temporal behavior.
6. Measure actual headlights and selective nighttime lights when implemented; zero-light night captures cannot establish those budgets. Repeat driving, bridge traversal, streaming and flight at the final normal play resolution with physics active. Investigate any recurring >33.33 ms frames with profiling.

No production correction or new art direction is accepted here. #32 remains the human decision point. Remaining unavailable measurements are GPU pass timings, long-run/route frame times, animated-water temporal behavior, headlight/local-light cost, and a user-confirmed reference for the exact wrong-direction highlight. The one observed 48 ms hitch remains unexplained after two clean short reruns.

## Verification

The opt-in fixture was built and typechecked while developing it, then exercised against the actual M4 Pro renderer. The shadow configuration check fails for the baseline and passes for the shadow override; visual A/B confirms the projected shadows. No production behavior changed. The new fixture itself was validated through renderer runs rather than test-first automated coverage; that is a deviation from the repository’s TDD workflow, not evidence that fixture behavior needs no tests. Existing water tests and workspace checks passed as listed below.

Completed successfully:

- `cargo test -p main --all-features water::tests`: 3 existing water tests passed.
- `cargo check --workspace --all-targets --all-features`.
- `cargo clippy --workspace --all-targets --all-features`; a new fixture style warning was fixed, and `cargo clippy -p main --all-targets --all-features` then had only pre-existing warnings.
- `cargo test --workspace --all-targets --all-features`: **196 tests passed** (3 generator, 84 runtime, 10 preview, 99 shared), plus benchmark targets' test-mode smoke checks.
- `python3 scripts/test_capture_lighting.py`: 3 command-line smoke tests passed at the user-agreed capture-script boundary. The in-checkout output case failed before the guard was added; existing-output preservation and dirty-source/ignored-asset provenance also pass. Cargo is stopped at the process boundary before build/render work.
- `cargo fmt --all --check`, shell syntax, staged whitespace checks, all 15 baseline image dimensions/nonconstant-pixel checks, all 15 baseline phase/direction/shadow records, and positive raw frame intervals.

The full suite ran once after the capture work; the subsequent match-guard cleanup was behavior-preserving and rechecked by clippy. Logs remain at `/tmp/terra31-targeted-tests.log`, `/tmp/terra31-workspace-check.log`, `/tmp/terra31-final-clippy.log`, and `/tmp/terra31-full-tests.log`.

Manually inspected representative noon, sunset, night, and overview captures and the shadow, fog, atmosphere, exposure, ambient, rough-water and flat-water comparisons. Moving-world visual acceptance and the exact reported wrong-direction highlight remain follow-up as described above.

## Standards

The independent review identified three findings:

- **TDD workflow deviation (root `AGENTS.md`):** the new capture fixture has no automated test-first coverage for scheduling, phase selection, or probes. Real renderer runs cover the accepted matrix, but do not replace regression tests. Retained as a disclosed limitation for renderer fixture internals. After the user approved the capture-script boundary, three automated smoke tests were added; the new output-location guard followed red → green. Phase/probe visuals remain covered by the recorded renderer matrix.
- **Possible primitive obsession (judgment call):** probe strings and phase indices could be enums. Retained for this bounded diagnostic fixture; inputs are validated before capture, and all accepted choices were exercised.
- **Capture provenance:** the original script omitted ignored generated GLBs and dirty source state. Fixed for future runs by recording worktree status/diff and source/asset hashes. The script now rejects destinations inside the checkout to prevent evidence hashing itself, with a regression test. Historical capture-time provenance cannot be recreated; the evidence inventory identifies that limitation explicitly.

## Spec

The independent review found no missing requirements or scope creep. All five scene categories at noon, sunset, and night were measured on the actual M4 Pro. Observations and hypotheses are separated, and production shaders remain unchanged. Headlights are unavailable because the runtime has no point/spot lights; GPU/route timings, animated-water temporal behavior, the exact reported highlight, and the hitch cause are explicitly unresolved.

Review totals: Standards **3 findings** (provenance corrected prospectively; test-first coverage remains the principal workflow limitation); Spec **0 findings**.
