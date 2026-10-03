# Arcade plane prototype

Decision: [Validate plane flight and landing](https://github.com/miran248/terra/issues/25).

Run `just plane-prototype`. This throwaway experiment starts seated in a primitive
plane on the actual planet. It searches for a nearby clear ground run; if none
is found, it starts in flight and reports that on the overlay. Plane controls
and camera take precedence if several prototype features are enabled. Normal
on-foot and car commands retain their own controls.

| Control | Behavior |
| --- | --- |
| Hold Shift | Apply thrust; releasing it cuts thrust and lets drag slow the plane |
| Hold Ctrl | Additional air braking |
| W / S | Pitch down / up through full loops; S initiates takeoff above 24 m/s |
| A / D | Roll through full rotations with assisted turning; steer on the ground |
| Space | Ground braking |
| R | Manual flight reset, 60 m above the last ground reset, at 60 m/s |
| T | Manual search for a nearby clear ground reset and takeoff run |

Hold Shift to accelerate, then hold S once speed reaches 24 m/s. Release pitch
to ease back toward level; continue holding it for a loop. Build speed and height
before looping. For landing, release Shift and use Ctrl as needed to reduce
speed, descend toward clear ground with W, then gently raise the nose to reduce
descent. Space brakes after touchdown. There is no entry, exit, summoning,
damage, fuel, or repair.

## Starting behavior and values

- Ground thrust targets 40 m/s with a 12 m/s² drive adjustment; releasing Shift
  coasts toward rest at 4 m/s²; Space brakes at 25 m/s². Collision friction and
  slope also affect ground acceleration.
- Air thrust is 10 m/s² while Shift is held. Drag is `2 + 0.0008 × speed²`
  m/s², plus 12 m/s² with Ctrl. This balances near 100 m/s in level powered
  flight; dives can exceed that speed. Thrust is not a latched throttle.
- Climbing reduces speed and diving increases it through the component of
  20 m/s² gravity along the nose. There is no minimum flight-speed clamp.
- Stall begins below 22 m/s forward airspeed and clears above 28 m/s. While
  stalled, lift is lost, gravity pulls the plane down, the nose drops at up to
  0.65 rad/s, and assisted turning weakens. Lowering the nose and applying
  thrust can restore airspeed. This is forgiving arcade behavior, not an
  angle-of-attack or aerodynamic simulation.
- Pitch input is 0.55 rad/s without a pitch limit; angle wrapping permits full
  loops. Outside a stall, released pitch eases toward level at 0.7/s.
- Held bank input rolls at 1 rad/s through full rotations; releasing it levels
  at 2.5/s. Assisted yaw is `0.8 × sin(bank)` rad/s at normal airspeed,
  remaining finite through vertical bank. Non-stalled velocity follows the
  nose, including through vertical and inverted flight.
- Flight direction uses radial up at the current position. Powered level flight
  approximately maintains radius around the planet; terrain can rise into the
  flight path. Discrete steps introduce a few metres of radius drift per full
  high-speed circuit. There is no automatic terrain avoidance.
- Mass is 900 kg. Avian owns collisions and synchronized poses. Compound
  collision matches a 5 m fuselage, 8 m wings, and 3 m tail. The controller owns
  rotation; banking and pitch are visible on primitive geometry.
- Camera: 6 m up, 18 m behind the local heading reference, looking 30 m ahead.
  It keeps a radial horizon without bank roll or a camera flip at vertical
  pitch. A 0.3 m obstruction sphere retracts immediately with 0.2 m clearance;
  release eases at 5/s. Manual resets intentionally reposition immediately.

A gentle landing requires actual contact with ground/bridge/ice, speed at most
45 m/s, downward speed at most 6 m/s into the surface, slope at most 15°, pitch
within about 15°, and bank within about 20°. It returns to ground controls.
Other airborne impacts cut thrust and release the body to physics at the impact
site; any obstacle contact does likewise, including low-speed taxiing and
contacts during takeoff. Collision-resolved
momentum is preserved, rotation unlocks, gravity continues, and friction plus
linear/angular damping lets the wreck tumble, slide, and settle. The first 0.3 seconds
after takeoff/reset allow existing ground contacts to clear; this grace period
never ignores obstacles. Liquid-water
contact also releases the wreck to physics. Frozen water remains ground. There is
no automatic crash teleport: R/T are explicit manual recovery controls. There is no buoyancy model, so water wrecks sink toward the terrain. Manual
reset restores controlled rotation, clears angular velocity, and resets the
collision material and damping.

The overlay shows plane altitude ASL (radius minus sea-level radius) separately
from clearance to the nearest solid or liquid surface directly below. The
existing terrain HUD is labelled “Terrain elevation”; it describes terrain,
not aircraft altitude. Obstacle tops, water, and terrain approximation can make
clearance differ from plane altitude minus that terrain estimate.

Ground reset samples up to 60 locations within about 500 m. It checks an 8°
slope limit, a 50 m forward run, liquid surfaces, and currently present obstacle
colliders. Discrete checks are not a production safe-placement guarantee:
narrow obstacles and later-streamed scenery can interfere. Liquid clearance
uses baked surface facets rather than animated wave displacement. Use R/T
rather than the on-foot world-map teleport.

## Live evaluation

Initial feedback accepted the general flight/camera direction but requested
manual-only crash recovery, full loops, stalls, held thrust, angle-dependent
speed, and distinct aircraft-altitude/ground-clearance readouts. These changes
supersede the ticket's original no-stall/adjustable-throttle starting direction.
The updated handling still needs a live verdict.

Try takeoff, low circling, a gentle landing, a powered loop with enough height,
a low-speed stall and recovery, obstacle/water contact, and manual R/T recovery.
Check that the stable horizon shows enough ground to judge height and approach.

Shared tests cover thrust/coasting, climb/dive energy, loops, stalls/recovery,
spherical travel, leveling, landing classification, and camera obstruction.
Headless Avian tests cover takeoff, touchdown, crash-in-place/manual reset,
water contact, low-speed and takeoff wing/trunk contact, airborne stall/recovery,
ground reset from high altitude, and
nose-impact tumbling with a continuously updated wreck-clearance readout.
