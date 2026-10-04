# World detail design interview

Status: consolidated direction confirmed by the user. Further decision work is tracked in [Plan vehicle correctness and a richer, readable Terra](https://github.com/miran248/terra/issues/28), the canonical wayfinder map. These notes preserve the accepted interview baseline; subsequent ticket resolutions own later decisions.

## Agreed direction

- Deliver in stages: vehicle correctness first, then sunlight and material response, followed by scenery, bridge details, and local lighting. Preserve the current material setup initially; new surface detail belongs to the later painterly experiment.
- Preserve the existing visual style. Later, compare it with a seamless painterly treatment using the existing asset preview tool. The tool supports side-by-side asset comparisons; terrain/material comparison requires extending it.
- Support both vehicle-height exploration and planet overview. Target the user's MacBook Pro M4 Pro at the normal play resolution: prefer 60+ FPS, with 30 FPS as the minimum. Record the actual viewport resolution when benchmarking and derive resource budgets from measurements. Existing chunk LOD controls residency, with additional distance culling for scenery; richer representations need separate evaluation.
- Use procedural density maps for grass and other scenery placement, sampled during world generation into deterministic placements. Render repeated scenery through instancing/batching, starting with grass. Manual brush authoring is not part of the initial direction. See [the generation-time sampling decision](adr/0003-sample-scenery-density-during-generation.md).
- Use emissive fixtures with selective local illumination for nighttime navigation. Keep wilderness roads mostly dark between occasional markers, with stronger lighting at settlements, junctions, and bridges. Activate headlights automatically at dusk with a manual override. Prioritize headlight shadows and nearby navigation lights; exact budgets remain open.
- Mix neighboring vegetation at biome transitions and vary vegetation and exposed rock with landform. Preserve sparse areas, clearings, readable roads, and settlement entrances. Investigate reuse of existing transition data before choosing a representation.
- Add collidable bridge railings with simple collision shapes and sufficient driving clearance, matching bridge construction. Fix deck traversal first.
- Keep scenery placement deterministic across revisits and graphics settings. Visibility may change; collision-bearing scenery remains consistent. See [the placement decision](adr/0002-deterministic-scenery-placement.md).
- Preserve stylized water while fixing lighting. Prioritize nearby shadows from terrain, bridges, buildings, vehicles, trees, and substantial rocks; shadows from grass and tiny scenery are optional quality features.
- Trees and substantial rocks remain obstacles. Grass, flowers, and small plants are pass-through. Keep road surfaces and bridge entrances clear.
- Preserve existing terrain, road, and bridge material treatment for now. The later painterly experiment covers all categories: terrain, water, roads, bridges, buildings, vehicles, and foliage. Compare representative examples through preview material overrides before any production adoption.
- Extend the asset preview with a representative terrain patch containing a biome boundary, slope, shore, and water, plus noon/sunset/night lighting. Reuse game material code for meaningful comparisons.
- Compare material techniques incrementally: painterly color textures and terrain layer blending first, then triplanar projection and packed material channels. These are experiment candidates, not commitments to the final production pipeline.

## Verified constraints

- Existing transition data includes categorical `FaceBlend` pairs, per-corner terrain identities, and resolved corner-color gradients. It does not provide continuous biome membership weights. Scenery placement currently consumes neither these boundary descriptors nor landform data.
- Existing chunk LOD changes residency and scenery distance culling limits visibility; this does not by itself provide progressively more detailed meshes.
- Generated catalog assets currently have a texture-free GLB contract in `crates/gen_assets/AGENTS.md`. The experiment uses preview material overrides; adopting textured production assets would require a separate contract decision.

## Problems to resolve

- Car wheels appear to rotate backward.
- Cars get stuck in bridge geometry.
- Sunlight produces washed-out colors, overly dark surfaces, missing shadows, and apparent illumination from the wrong direction. Water is the worst affected surface. The [#31 diagnostic baseline](lighting-diagnostics.md) records observed causes, isolated probes, and remaining uncertainties; production correction selection is separate.
- Scenery is too sparse and lacks variety, especially at biome and landform transitions.
- Bridges need more convincing detail, including railings.
- Terrain needs surface texture detail. Splat blending, channel packing, projection, and repetition control remain design candidates, not accepted architecture.

## Delivery stages

1. Reproduce and fix wheel rotation and bridge traversal. Add failing regression tests before behavior changes; check forward/reverse travel and bridge entry, crossing, and exit visually.
2. Correct sunlight, shadow, and water response while preserving the current style. Check washed-out colors, dark surfaces, apparent light direction, and missing shadows at noon, sunset, and night.
3. Add deterministic density-driven scenery and instance/batch repeated assets, starting with grass. Reuse existing terrain boundary information where useful and incorporate landform, slope, and moisture. Preserve navigation clearance and collision consistency.
4. Add bridge railings matching bridge construction, with simplified collision and sufficient driving clearance.
5. Add selective nighttime illumination and emissive fixtures for vehicles, settlements, and the road network, including automatic headlights with a manual override. Prioritize navigation-relevant shadows.
6. Extend the preview and run the all-category painterly comparison. Keep the existing material treatment as the baseline and compare candidates under identical cameras and lighting. Production adoption is a later decision informed by visual comparison and performance.

## Acceptance scenes and verification

- Dense biome transition: placement variety, mixed vegetation, landform response, collision behavior, and scenery streaming.
- Bridge crossing: wheel motion, deck support, clear entrances, railing collision, and driving clearance.
- Shoreline at sunset: water lighting, sun direction, readable surfaces, and shadows.
- Settlement at night with headlights: navigation, local illumination, shadow priorities, and light activation.
- Planet overview: retained broad visual style, distance transitions, and performance.

Use fixed cameras or repeatable routes and consistent world data for comparisons. Measure on the user's MacBook Pro M4 Pro at the normal play resolution, targeting 60 FPS and investigating drops below 30 FPS. Record resolution, quality settings, and frame-time behavior alongside results. Tests and visual checks have not yet been run; only design documents were changed during the interview.

Follow repository TDD and the nearest crate's checks during implementation. World-generation changes require the relevant determinism and generated-level checks; schema changes require regeneration and workspace verification. Report manual visual checks performed and still needed.

## Measurement-dependent implementation details

Density-field encoding, batch organization, distance thresholds, shadow counts/ranges, and texture budgets will be selected through targeted implementation experiments. Keep established world identity and offline generation contracts. Runtime density sampling is deferred unless stored placement size demonstrates a need. No performance or visual result is claimed before measurement.

For the painterly experiment, evaluate splat blending, triplanar projection, packed masks, and repetition control individually. See primary references for [terrain layers and packed masks](https://docs.unity.com/en-us/engine/6000.5/manual/creating-environments/script-terrain/terrain-textures/class-terrain-layer) and [triplanar projection](https://docs.unity.cn/Packages/com.unity.shadergraph%4012.1/manual/Triplanar-Node); these describe techniques, not Terra's engine integration.

Architectural decisions are recorded in `docs/adr/` when their trade-offs are settled. The shared domain vocabulary remains in [the glossary](../GLOSSARY.md); no new domain terms were required by this interview.
