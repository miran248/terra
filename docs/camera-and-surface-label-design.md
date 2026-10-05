# Main-camera follow modes and surface labels

Status: accepted on 2026-10-06. The design interview is complete and the user has confirmed shared understanding. This document specifies intended behavior; runtime implementation is pending.

Implementation specification: [Main-camera follow modes and smooth Surface labels (#70)](https://github.com/miran248/terra/issues/70). The user confirmed reuse of the existing gameplay test app, label-policy tests, and rendered acceptance harness as the testing seams.

## Main-camera follow modes

Use the existing main camera for walking, cars, and planes. `C` opens camera choices in the left panel; the corresponding number selects a mode:

1. **Facing** follows the subject's facing direction, including aircraft pitch, with a steady horizon.
2. **Movement** follows actual travel direction, with a steady horizon. Reversing a car or walking backward smoothly turns the camera to follow that direction. Hold the last direction when stopped and use facing before the first movement.
3. **Orientation** follows the subject's full orientation, including aircraft roll.

Facing and Orientation can look identical for subjects without additional tilt or roll. Default to Facing and remember the selection throughout the session, including entering and leaving vehicles. Smooth camera changes. Planet view retains its orbit controls and restores the selected follow mode on return.

Treat `C` as a top-level command alongside `V`. Its chooser follows the vehicle chooser's modal lifecycle: pause simulation while open, display clickable choices in the left panel, select with `1`/`2`/`3`, and close on selection or cancel with `Escape`/`C`. Show the current selection. Closing restores the previous simulation-time state.

Preserve contextual command priority: `V` followed by `C` chooses Car, not the camera chooser. Do not stack choosers. The camera chooser is available on foot and in vehicles during gameplay; Planet view retains its existing controls. The current Planet-view-only View section does not provide gameplay access by itself.

## Surface labels

Use [Surface labels](../GLOSSARY.md) in the minimap and Planet view. Anchor names geographically and wrap lettering along a smooth globe-like ribbon above local terrain, rather than conforming to individual hills and valleys. Allow orientation to adjust for readability in each view. The main walking/flying view is outside this label change's scope.

Preserve existing name visibility, hover-only names, and overlap suppression. Allow lettering to disappear partially at the horizon or minimap boundary rather than shortening the name. The minimap still needs its circular mask; surface wrapping does not eliminate boundary masking or occlusion.

## Acceptance scenarios

- Open the camera chooser on foot, in a car, and in a plane; simulation pauses and keyboard or clickable choices select the same mode. Cancellation preserves the selection and closing restores the previous simulation-time state.
- Open the vehicle chooser with `V`, then press `C`; select Car without opening camera choices.
- Climb, dive, and bank: Facing tracks the nose with a steady horizon; Movement tracks travel with a steady horizon; Orientation follows aircraft roll as well as pitch and heading. Preserve existing camera collision protection and smooth transitions.
- Reverse or walk backward in Movement mode, then stop; smoothly track backward movement and retain the last direction at rest. Before any movement, use facing.
- Enter or exit vehicles and enter or leave Planet view; preserve the selected follow mode throughout the session. Planet view continues to use orbit controls.
- Rotate Planet view and change minimap heading; labels stay geographically anchored and turn for readability. Each view has its own readable orientation.
- Show labels near rugged terrain, the horizon, and the minimap rim; use smooth ribbons, natural partial disappearance, and circular minimap masking without shortening text. Preserve existing hover and overlap rules, and keep these labels out of the main gameplay view.

## Existing contracts

Keep the single camera owner established by [ADR 0004](adr/0004-live-planet-view.md). Current camera, sidebar, minimap, and overlay behavior is documented in [runtime contracts](runtime.md); this design proposes changes to those behaviors and does not replace that documentation until implementation.
