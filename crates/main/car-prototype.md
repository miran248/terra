# Arcade car handling prototype

Decision ticket: [Validate arcade car handling](https://github.com/miran248/terra/issues/24).

Run `just car-prototype`. This feature-gated, throwaway experiment starts you
already seated in a placeholder car at the normal settlement spawn. It uses the
existing planet, roads, physics terrain, bridges, and obstacle colliders. It
does not implement summoning, vehicle ownership, or entry/exit. Normal `just run`
and `just on-foot-prototype` retain their on-foot movement and cameras. The mesh
contact correction is shared by all modes. When both prototype features are
enabled, the car owns movement and camera unless the plane prototype is also enabled.

- **W** accelerates forward, or brakes when reversing.
- **S** brakes forward motion to a stop, then reverses while held.
- **A/D** steer; reversing reverses the turning direction. Steering requires
  motion and becomes gentler at higher speed.
- **R** resets position, heading, and velocity to the prototype's spawn.

The overlay shows signed forward speed, support slope/airborne state, and whether
the car is nearly stopped (supported and below 0.5 m/s). The latter is an
observation for the later exit decision, not an exit implementation.

## Starting parameters

| Parameter | Value |
| --- | --- |
| Forward / reverse powered speed | 30 / 8 m/s |
| Acceleration / braking / coasting deceleration | 18 / 12 / 2 m/s² |
| Sideways grip | Exponential decay at 8/s |
| Steering | Up to 1.5 rad/s, reduced with speed; fades below 2 m/s |
| Powered slope limit | 45° relative to local spherical up |
| Body / mass | 1.8 × 0.8 × 3.2 m box / 800 kg |
| Gravity | 20 m/s² toward the planet center |
| Camera height / back / look ahead | 3 / 8 / 15 m from body center |
| Camera obstruction | 0.2 m sphere with 0.1 m clearance |
| Camera release and orientation easing | Exponential blend at 6/s |

Gravity and collisions remain Avian-owned. Driving starts from the actual
physics velocity and applies traction along the surface. While a driveable
support is within the existing probe reach, it removes velocity directed away
from that surface so leftover uphill momentum does not launch the car over
small crests. Inward velocity remains for physics to resolve. There is no throttle, grip, or steering while airborne or
on supports steeper than 45°. Gravity can still carry the car downhill and
external tangential impulses can exceed powered-speed limits temporarily. Ground
adhesion suppresses outward impulses while driveable support is still in reach;
once airborne, the controller leaves velocity unchanged and steering disabled.

Terrain, ice, and bridge meshes correct internal triangle-edge contacts so
shared edges do not act like obstacles. Support changes do not rotate the
existing velocity: collisions resolve changes in travel direction, avoiding
an extra controller impulse when a probe switches surfaces.

Rollover policy for this experiment is an upright chassis relative to the
planet. This intentionally favors easy exploration over simulated suspension.
The colored body, cabin, and static wheels are primitive placeholders, not
production assets. Wheels are visual only; the chassis box owns collision.

## Human evaluation

1. On a road, accelerate, release W, brake with S, then continue holding S to
   reverse. Compare stopping distance with visible landmarks.
2. Turn at low, medium, and full speed; back around a corner. Does steering feel
   predictable and forgiving without being too sharp?
3. Leave the road for uneven ground. Check traction, obstacle contacts, and
   whether the rigid chassis catches too often on terrain.
4. Approach gentle and steep slopes. Check the powered climbing cutoff,
   downhill motion, landings, and upright assistance. Reset with R if stuck.
5. Drive beside walls, trees, slopes, and bridges. Check camera retraction,
   smooth release, local horizon, and enough visibility ahead.
6. Come to a complete stop and inspect the nearly-stopped indicator. Actual
   exiting remains in the vehicle lifecycle decision.

Record feedback on the ticket before resolving handling and camera values.

Initial live feedback requested more torque and higher acceleration. The second
tuning pass raises drive acceleration from 6 to 18 m/s², keeping powered speed
limits and braking unchanged. This directly increases available uphill drive;
the prototype does not model an engine torque curve or gearbox. This stronger
setting still needs a live verdict.

## Limits to carry into integration

Support uses five 0.75 m downward rays at the center and chassis corners. The
nearest upward-facing hit supplies the support slope. This keeps the car
grounded when an uphill edge supports the upright body but the center ray alone
cannot reach the terrain. Probe placement shares the physics chassis dimensions.
These discrete samples can still miss a narrow support between probes. There
are no wheel contacts or suspension, and the chassis does not pitch or roll to
match slopes. Sharp crests and uneven ground remain points for feedback.
Reset always returns to the starting spot; there is no water recovery rule or
safe-placement search. Use R rather than the on-foot map teleport, whose
placement clearance has not been adapted to the car.
Spawn and reset place the body above the surface for clearance; it briefly falls
before the support ray reaches the ground and enables driving.

Camera collision follows physics colliders; foliage without colliders can
obscure the view. If the camera cast begins overlapped, it retracts to the body
center. The sphere approximates near-plane clearance. Camera orientation and
obstruction release are smoothed; emergency retraction is immediate.

The shared-model tests cover acceleration, braking before reverse, speed limits,
steering, grip, unsupported/steep-slope behavior, and spherical camera retraction
and recovery. Live testing on generated terrain is still required to judge feel
and find limits of the simple support model. No production vehicle lifecycle or
asset decision is implied by this prototype.

A headless Avian regression places the real chassis on a 20° ramp and exercises
the actual driving plugin. It checks support and powered movement, then lifts
the car a metre and checks that it becomes airborne. The original center-only
probe failed this scenario; the footprint probes pass.

Three headless replays use positions, headings, and velocities captured from live
seed-1337 driving. One crosses a gentle terrain edge that previously stopped
the car from 30 m/s; the other climbs an ice bank that previously launched it.
A third holds steering over a captured crest that previously caused a short
airborne interval and a steering pause. They exercise the production mesh-collider
builder and driving plugin, checking continued speed, ground support, and steering. These replace the synthetic flat-to-30°
join test, which did not capture the live failure. A shared-model regression
checks that changing support does not add a velocity impulse. Further live
terrain testing is still required to judge handling.
