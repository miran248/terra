# On-foot exploration prototype

Decision ticket: [Tune on-foot movement and camera](https://github.com/miran248/terra/issues/23).

Run `just on-foot-prototype` from the repository root. This uses the normal
generated assets and planet. Press **F6** to switch between the proposed settings
and the original baseline without moving the player. Normal `just run` does not
enable this prototype. All tuning state is in memory.

| Setting | Proposed | Baseline |
| --- | --- | --- |
| Walk / sprint | 5 / 10 m/s | 60 / 150 m/s |
| Underwater or frozen water | 2 / 4 m/s | 24 / 60 m/s |
| Camera height / back / look ahead | 2 / 5 / 10 m | 4 / 9 / 45 m |
| Obstruction handling | 0.2 m sphere cast, 0.1 m clearance | Original camera |

W/S move, A/D turn, Shift sprints, and Space jumps. Camera offsets are relative
to the player's body center, matching the original runtime convention. The
overlay shows the selected mode, settings, measured tangential speed, and actual
camera distance. Existing fullscreen-map teleporting can help reach scenarios.

Jumping retains the original 14 m/s radial velocity increment, and turning
retains 2.5 rad/s. Slower travel therefore shortens horizontal jump distance;
vertical jump behavior is unchanged. This is a comparison point, not a validated
jump-design decision. Underwater and frozen water retain their existing 0.4
multiplier; frozen ground alone does not trigger that multiplier.

## Human evaluation

Compare both modes at the same locations:

1. On flat ground, inspect nearby scenery, walk, sprint, stop, and turn. Can you
   read the terrain ahead without the player obscuring too much of the view?
2. Walk uphill, downhill, and across a slope. Does the lower camera show enough
   terrain to choose a route?
3. Back toward a slope, wall, tree, and bridge. Check camera retraction and
   restoration, including when turning beside obstacles.
4. Jump while stationary, walking, and sprinting. Judge the unchanged vertical
   impulse with the shorter horizontal travel.
5. Try underwater and frozen-water movement and compare the resulting 2/4 m/s
   pace with dry ground. Use the map to reach suitable terrain where available.
6. Visit different sides of the planet. Check that the camera stays upright
   relative to the local ground.

## Limits and evidence

This is throwaway, feature-gated code awaiting live feedback. The proposed camera
uses direct placement and orientation to isolate framing; it does not yet ease
obstruction release or mode changes. It tests physics colliders, so non-colliding
foliage can still obscure the view. If the cast origin already overlaps an
obstacle, the camera retracts to the player center; that extreme case may show
the player interior. The sphere is a near-plane approximation rather than an
exact camera-frustum sweep.

Automated tests cover speed selection and water/ice slowdown, camera orientation
on opposite sides of the planet, and retraction/restoration from hit distances.
They do not establish subjective movement feel or prove all rendered terrain
and collider combinations. Record visual feedback on the ticket before choosing
final values and closing it. Production integration remains a later map ticket.
