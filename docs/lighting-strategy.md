# Lighting correction strategy (#32)

Accepted direction for [#32](https://github.com/miran248/terra/issues/32), based on the [M4 Pro baseline](lighting-diagnostics.md). The user accepted the correction scope, distance-first shadow trade-off, and permission to reduce water wave/normal strength while preserving its style. This is an implementation-ready decision; production corrections and their visual/performance acceptance remain subsequent work.

## Selected behavior

Keep the present palette, texture-free catalog, and terrain/road/bridge material treatment. Correct lighting before evaluating the later painterly treatment. Keep wilderness dark at night, with selective local illumination and headlights delivered in their planned stage. Do not compensate for absent headlights by making the entire night bright.

### Sun and normals

Retain the existing world-space convention: `sun_dir` points toward the sun, and the directional light's forward direction points along the incoming rays (`forward · sun_dir = -1`). Terrain normals and unperturbed water normals point outward from the spherical world. A directional light's translation is not a point-light origin.

The baseline found no global sign error: all 327,680 terrain faces had finite outward normals. Do not reverse winding or the light to address the unconfirmed water-highlight complaint. If a specific highlight still appears reversed after the changes below, reproduce it with recorded camera/sun vectors and isolate its material terms before changing a sign.

### Nearby shadows

Explicitly enable sunlight shadow maps. Include nearby terrain, bridges, buildings, vehicles, trees, and substantial rocks as casters and receivers where the material supports receiving shadows. Keep the explorer eligible too. Exclude grass, flowers, and other tiny scenery from casting in the initial configuration; their optional shadows must not consume the budget needed by navigation-relevant geometry.

Preserve the existing non-casting policy for water, rivers, ice, and debug borders during this correction; do not turn on every mesh caster indiscriminately.

Apply participation to actual rendered mesh entities, including imported GLB descendants. A marker on a scenery root is insufficient. Reapply the policy when scenes instantiate and chunks stream; never change placement, collider identity, or collision activation as a shadow-quality adjustment. Classification should reuse existing asset/category metadata, rather than maintaining an unrelated list of names.

Start from the measured configuration: four cascades, 150 m maximum distance, 10 m first-cascade distance, and a 2048 shadow map. Explicitly author these settings so an engine-default change does not silently change the budget. Begin with the current depth/normal biases (0.02/1.8), then tune only if bridge decks, slopes, trees, or vehicles show acne or detached shadows. These values are an experimental starting point, not accepted final quality settings.

Preserve nearby shadow clarity before distant coverage. If live measurements miss the preferred budget, shorten maximum range first; 100 m then 75 m are useful trial points, not mandatory shipping presets. Evaluate the same routes and captures after each change. Lower map resolution only after distance reduction fails or further reduction visibly harms navigation. Do not silently remove priority caster categories, disable all shadows, or change world obstacles to meet a frame-time target. If the minimum budget still fails, report the measured bottleneck and reopen the quality trade-off.

### Ground fog and planet overview

Retain atmosphere and ground-level haze. Make distance-fog strength depend continuously on camera altitude above the spherical surface so it fades out for planet overview; avoid a binary camera-mode switch or an abrupt flight transition. Establish the ground and overview endpoints with the existing captures, then tune the transition through an actual ascent/descent. Do not use global fog removal or atmosphere removal as the production correction.

The required result is a readable sunlit hemisphere at the overview camera while preserving nighttime darkness, the day/night boundary, and ground-view depth cues. Distance fog and atmospheric aerial perspective must be evaluated together so the same view does not accumulate an unintended opaque veil. Exact transition heights are reversible visual tuning parameters, selected through that comparison rather than inferred from the diagnostic fog-disable override.

### Exposure and ambient fill

Keep the existing HDR/PBR and tone-mapping path. Use a stable exposure setting initially; do not introduce automatic exposure adaptation as part of this correction. Tune exposure and daylight ambient fill together under identical noon, sunset, and night cameras. Preserve palette differences in direct light while keeping shaded object shape readable. Increasing EV alone is insufficient: the EV12 probe darkened already-dark surfaces along with highlights.

Retain a day-dependent ambient contribution and the established dark-night intent. Avoid per-asset color edits, emissive compensation, or a blanket night ambient increase to mask missing shadows or future local lights. Select final numeric values from paired visual captures, with the baseline EV9.7 and current ambient curve as the starting controls. Record accepted values and rejected alternatives in the implementation evidence.

### Water and PBR integration

Retain stylized moving water, transparency, shoreline/depth detail, and the current shared sunlight/PBR integration. Do not replace water with unlit color or remove its motion to make a stationary screenshot pass. Keep its spherical outward base normal and let the engine handle the common light/shadow response; avoid an additional independent sun model in the water shader.

Separate geometric displacement from normal perturbation in diagnostic controls. Compare baseline, geometry-only, normal-only, and reduced-strength combined waves under identical lighting; then inspect Fresnel, depth blending/foam, roughness, and specular response one factor at a time as needed. The combined flat-water probe removed mottling, but did not identify which wave contribution caused it, and increasing roughness alone did not remove it.

Reducing wave/normal strength is approved if it removes distracting mottled highlights while retaining visible water movement and shoreline character. A user-confirmed visual comparison is still required for the final appearance. Do not claim the original wrong-direction complaint is solved solely because mottling is reduced.

## Frame-time acceptance and measurement budget

Use the actual M4 Pro, recording physical/logical resolution, scale factor, presentation mode, build profile, quality settings, hardware/OS, complete source state, and generated asset hashes. Compare at the same normal play resolution. The #31 reference is 2560 × 1440 physical in the default window; it is not evidence for a different maximized window or release profile.

Use the five established scenes: dense biome transition, bridge crossing, shoreline sunset, settlement night, and planet overview. Run fixed-camera noon/sunset/night comparisons for every scene, then repeated live routes with physics, animation, streaming, and water motion active. Include bridge travel, dense scenery travel, shoreline camera rotation, and ascent/descent into overview. Separate startup/compilation/screenshot costs from steady samples; retain traversal/streaming stalls in live-route results.

The engineering measurement gate is:

- Prefer 60 FPS: target p95 frame intervals at or below 16.67 ms on each repeatable live route. Also report median, p99, maximum, and every interval above 33.33 ms; a percentile alone does not prove consistently smooth play.
- Require 30 FPS minimum in representative warmed gameplay: investigate every observed interval above 33.33 ms. Recurring route/streaming/shadow stalls fail acceptance until corrected or an explicit trade-off is approved. Do not discard the maximum as an outlier without diagnosis.
- Collect at least three 60-second runs per route/configuration after warmup. Compare shadows off/on with all other controls fixed to attribute incremental cost. Measure fog/water changes separately before the combined configuration; use CPU/GPU profiling when total intervals do not identify the bottleneck.
- Choose the largest tested shadow range satisfying the gates and visual requirements. Record the final cascade layout, range, map resolution, bias, caster categories, and paired frame distributions. Do not assign a speculative fixed millisecond allowance to shadows or infer one from another machine.

The existing dense-scene shadow probe (p95 18.988 ms, maximum 24.333 ms) is a useful starting sample, but already exceeds the preferred p95 target and freezes gameplay. It cannot approve 60 FPS or the live 30 FPS floor. The original 48.078 ms dense-sunset hitch remains unexplained despite two short clean reruns; include that scene in follow-up profiling.

The current runtime has no point/spot lights. Settlement-night evidence therefore approves only the base sun/ambient/fog behavior. Headlight and local-light shadow budgets remain unmeasured and must pass the same combined-scene gate when those features exist; do not mark the full night-with-headlights scene accepted prematurely.

## Visual acceptance

| Condition | Required evidence |
| --- | --- |
| Noon | Distinct pastel colors in direct light; readable shaded sides; attached, stable priority shadows on terrain, slopes, bridge decks, and vehicles; no pervasive acne. |
| Sunset | Shadows agree with the logged sun direction; water remains visibly animated with coherent highlights, transparent depth transitions, and shoreline detail; no distracting mottled flashing during camera motion. |
| Night | Dark wilderness and day/night separation remain; no bright ambient wash or daylight-like water highlights. Mark headlight navigation acceptance unavailable until implemented. |
| Overview and flight transition | Sunlit hemisphere remains readable; night side stays dark; fog changes smoothly during ascent/descent; shadow range/cascade transitions do not produce objectionable pops near the camera. |

Capture before/after images with matching world, cameras, lighting, and settings. Also inspect live movement: frozen captures cannot establish shadow stability, temporal accumulation quality, or animated-water acceptance. Obtain the user's visual acceptance of the final material response before calling the production correction complete.

## Regression checks and implementation sequence

Implement in small slices, beginning with a failing check at an agreed public test boundary. Before adding tests, confirm the precise seams with the user under the TDD workflow. Suitable boundaries are the configured runtime lighting state, spawned GLB mesh participation after scene readiness/streaming, and renderer capture output; do not test private helper arithmetic solely to mirror the implementation.

1. **Sunlight shadows:** verify explicit enabled/configured sun state and inclusion/exclusion on spawned mesh descendants. Check caster/receiver behavior in the real renderer, with before/after performance samples. Assert quality changes leave world placements and collision-bearing objects unchanged.
2. **Overview fog:** check finite, continuous ground-to-overview configuration, ground haze preservation, and the readable overview. Exercise both ascent and descent instead of only two endpoints.
3. **Exposure/fill:** preserve day/night behavior and compare all three solar conditions before selecting final values. Do not use image-byte equality as a cross-driver lighting oracle.
4. **Water response:** keep existing water regression tests; add coverage at the agreed boundary for any changed behavior. Capture each isolated wave/material probe and inspect animated results before choosing strengths.
5. **Combined acceptance:** repeat the full fixed-camera matrix and live-route budget checks after the selected settings are combined. Run affected crate checks, workspace typecheck/clippy, and the full suite once at the end; document remaining manual checks explicitly.

Preserve main/depth/shadow-prepass displacement parity for displaced geometry. If foliage motion-vector code is changed, previous positions must use previous animation state rather than the current sway offset; otherwise temporal history cannot describe the motion correctly. Test/inspect moving foliage and camera motion together. Existing parity between main and prepass displacement does not by itself prove correct motion vectors.

Blended water currently reads opaque scene depth and does not participate in opaque depth/motion-vector prepasses. Preserve that contract in this correction. Do not make transparent water write opaque depth just to suppress temporal artifacts; any change to that architecture requires separate evidence and a decision. Keep foam/thickness tied to the same displaced surface/depth convention and check intersections during animation.

## Decision placement

No new domain term is needed: use the existing glossary's scenery, flora, structure, terrain, and water distinctions. Shadow visibility is a rendering choice, not world identity. This respects the deterministic-placement and generation-time-sampling ADRs. These tuning/policy choices are reversible and do not justify a new architectural decision record; this document owns the implementation handoff.

## Decision validation

The implementation constraints were checked against the current renderer and #31 evidence. Local documentation links and Git whitespace checks passed. This change only records a decision; runtime tests and benchmarks were not rerun, and no production visual acceptance is claimed.

### Standards review

No documented-standard breaches or strong baseline smells. The strategy preserves the glossary, deterministic placement/generation decisions, and the transparent-water runtime contract.

### Spec review

No missing requirements, incorrect implementation claims, or scope creep. The decision covers the requested lighting policies, acceptance scenes, M4 Pro measurement gates, regression checks, and shader parity obligations.

Review totals: Standards 0 findings; Spec 0 findings.
