---
status: accepted
---

# Use the main camera for a live planet view

Replace the separate full-screen map presentation with an animated pullback of the main camera to a live, rotatable planet view for navigation and teleportation. Sharing the actual scene keeps geography, lighting, and navigation information together; it requires one camera owner, explicit input and clock boundaries, and rendering that supports both ground and planet-scale views. Keep simulation/collision ownership independent of camera visibility, reusable policy in its owning domain crate, and runtime orchestration in `main`.

The [planet-view decision map](https://github.com/miran248/terra/issues/37) indexes the specification. Detailed decisions and their acceptance cases live in the linked ticket resolutions:

- [Camera ownership and transitions](https://github.com/miran248/terra/issues/38#issuecomment-5980412571): single-owner camera modes, heading-follow, safe exterior paths, interruption, radial zoom, and teleport handoff.
- [Simulation time integration](https://github.com/miran248/terra/issues/39#issuecomment-5980483655): smooth zoom multiplier, independent pause/base rate, fixed-step preservation, and real-time presentation.
- [Input and teleport lifecycle](https://github.com/miran248/terra/issues/40#issuecomment-5980551717): persistent selection, captured gestures, modal priority, cancellable requests, and validation at commit.
- [Rendering compatibility](https://github.com/miran248/terra/issues/41#issuecomment-5980617647): altitude fog, explorer-referenced ambient, progressive detail independent of collisions, and visual/performance gates.
- [Overlays and destination picking](https://github.com/miran248/terra/issues/42#issuecomment-5980692517): projected navigation, road highlights, layer/label policy, picking, validation timing, and UI fades.
- [Final specification handoff](https://github.com/miran248/terra/issues/43#issuecomment-5980745754): cross-contract clarifications, acceptance, and implementation verification requirements.

Exact zoom distances, animation timing, detail thresholds, and rendering settings are bounded implementation tuning, subject to those acceptance criteria. Visible sun-disc rendering and broader lighting/art-direction improvements remain outside this effort; necessary fog, clipping, and detail compatibility adjustments are included. No disposable prototype is requested. Accepting this design does not perform or authorize production implementation.
