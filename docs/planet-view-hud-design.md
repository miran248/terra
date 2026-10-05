# Planet view and HUD refinement

Status: design confirmed by the user. Ready for implementation; no production
implementation has been performed.

Uses [Planet view](../GLOSSARY.md) terminology and retains the live-camera
architecture in [ADR 0004](adr/0004-live-planet-view.md).

## Settled decisions

- Planet view remains an overview at its closest zoom. Its minimum camera
  distance leaves comfortable space around the moving explorer or occupied
  vehicle. Investigate the reported visual tearing separately; a distance limit
  alone is not evidence that the defect is fixed.
- Dragging grabs the planet: the surface follows mouse movement in screen space
  consistently across viewing angles, keeping the grabbed point under the pointer
  as closely as possible.
- Active follow tracks the moving explorer or occupied vehicle without positional
  trailing. Preserve smooth view transitions and zoom, and align markers with
  the rendered scene each frame.
- Dragging disables follow and leaves the camera detached. Enabling follow
  smoothly recenters at the current zoom, then tracks without trailing. A fresh
  opening of Planet view starts with follow enabled.
- Consolidate text HUDs into one narrow, full-height left overlay with a minimal
  appearance and one continuous faint background. Use the same component for
  titled sections containing `field: value` rows. Order permanent information
  first, temporary status next, and contextual actions last.
- Keep the minimap as a separate graphical element.
- Order the titled sections as Environment (clock, weather, temperature),
  Location (terrain, elevation, settlement, region, road), Movement (travel mode
  and relevant measurements), View (Planet view controls when active), and
  Context (recovery, destination, prompts, messages), followed by Actions.
  Contextual information and controls appear above the final action rows.
- Present actions as clickable text rows with subtle hover feedback and retain
  keyboard shortcuts. Every discrete action gets clickable access. Continuous
  movement, steering, and throttle remain gameplay inputs.
- Environment and Location always describe the explorer, including while browsing
  Planet view. Selected-destination information belongs in Context and is
  explicitly labeled.
- Scroll the column when content exceeds available height, preserving readable
  text size. Wheel input over the column scrolls it; over the scene it zooms.
- Include compass directions around the planet using the minimap's visual
  language, with readable, geographically meaningful placement during rotation.
  Use directional letters around the rim with north highlighted, replacing fixed
  globe-point labels. Fade ambiguous directions near poles rather than allowing
  flips or jitter.
- Recovery supports a one-second hold on its clickable text row, displays
  progress, and cancels when released early, matching the existing keyboard hold.
- Move vehicle selection into the sidebar. Summon vehicle reveals Car, Plane,
  and Cancel rows in Actions, with instructions above in Context. Preserve the
  selector's existing simulation pause.

## Marker consistency

Planet view reuses minimap marker colors, shapes, and label rules. Settlement
names remain visible; region and bridge names appear on hover. Retain label
overlap suppression in crowded views. Remove marker-category toggles and use
minimap visibility rules. Planet view no longer automatically reveals region
names at closer zoom levels.

Remove the separate road-overlay toggle too. Show roads automatically where
readable, subject to appropriate distance and detail limits.

## Clickable action coverage

Provide contextual text controls for the existing actions and preserve their
eligibility rules and keyboard equivalents:

- Open and close Planet view.
- Enable and disable follow.
- Open vehicle selection, choose Car or Plane, and cancel selection.
- Enter and exit a vehicle.
- Commit teleportation to the selected destination.
- Hold to recover the explorer or occupied vehicle.

Destination picking remains directly clickable in the scene. Continuous gameplay
inputs and drag/zoom gestures retain their input mechanisms. Removed marker and
road visibility toggles have no replacement action.

## Acceptance criteria

- At closest Planet view zoom, the moving explorer, car, and plane retain
  comfortable surrounding context. Check the reported tearing during movement;
  distinguish camera trailing from any remaining rendering or overlay defect.
- Follow does not visibly trail its subject after recentering completes. Dragging
  detaches it; re-enabling follows smoothly without changing the selected zoom.
- Dragged surfaces follow the pointer consistently at different headings and
  viewing angles, including near poles, without reversing direction or jumping.
- Both maps use matching marker colors, shapes, and settlement-versus-hover label
  rules. Crowded Planet view labels remain readable. Marker and road toggles are
  absent; roads appear automatically at appropriate detail levels.
- Compass letters remain readable around the planet, with highlighted north and
  stable behavior as polar directions become ambiguous.
- Gameplay and Planet view use the same left sidebar presentation: one continuous
  full-height faint background, titled sections, and `field: value` rows.
  Contextual information and controls precede Actions. The minimap stays separate.
- In a short window, every section and action remains reachable by scrolling.
  Sidebar wheel input scrolls the column without zooming the scene.
- Each listed action is accessible by mouse as well as its existing keyboard
  input, under the same eligibility conditions. A recovery hold shows progress,
  cancels early, and activates after one second by either input method.
- Sidebar vehicle selection offers Car, Plane, and Cancel while preserving the
  simulation pause lifecycle.
- Browsing or selecting a destination does not change the explorer-based
  Environment and Location readouts. Destination details are labeled in Context.

## Verification boundary

Implementation follows test-driven development for behavior changes, with narrow
tests before affected-crate checks. Rendered checks must cover moving subjects,
dragging at different angles, marker consistency, HUD readability, short windows,
and view transitions. Existing [Planet view acceptance](planet-view-acceptance.md)
remains the baseline; no new visual checks have been performed during this
interview. Exact zoom limits, spacing, and fade thresholds are implementation
tuning subject to these acceptance criteria.
