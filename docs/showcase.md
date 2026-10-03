# Showcase film

Status: the complete expedition and physical return are recorded and validated.
The edited film runs 3 minutes 35 seconds at 1280×720 and 30 fps. Four README screenshots show the
house entrance, rainy driving, mountain flight, and planet map. The recorder
exports these PNGs directly from the same gameplay capture.

## Agreed requirements

- Tell a connected exploration story, rather than presenting independent feature demonstrations.
- Use actual gameplay movement, vehicle handling, collision, and enter/exit actions. Do not move a visual plane along a prescribed animation path.
- Movement should look intentional and natural. In particular, the explorer turns toward a vehicle, approaches it directly, and enters within normal interaction range.
- Give the explorer and car meaningful exploration routes. Include entering and exiting vehicles, boarding the plane, and taking off.
- Show settlements and their current assets, greenery, forests, rivers or lakes, snowy mountains, and contrasting biomes.
- Include rainy driving and flying through snowy mountains, alongside clear conditions. Show the world map and time of day.
- Include rolls and loops using the flight model. Leave room for normal travel after a feature or maneuver, rather than immediately cutting away.
- The film can be longer than the original 40-second recording. Story pacing takes priority over a short checklist of features.
- Make the finished video loopable, with matching opening/closing camera coordinates and orientation and pixel-identical decoded endpoint frames.
- Replace outdated README imagery. Keep one current video and a compact preview, and amend the existing showcase commit rather than adding another committed media revision. Do not push.

## Confirmed cinematic direction

- One connected expedition, with selective cuts to compress uneventful travel.
- Cinematic tracking shots with the normal gameplay UI, HUD, and minimap visible. The actual map appears as part of exploration rather than as a substitute for returning home.
- The explorer physically returns to the starting place. The ending must not disguise a teleport or reset with a map transition.
- Space the roll and loop well apart, with substantial ordinary flight between them. Perform both over broad open areas, away from mountain peaks.

## Recorded story arc

Begin at a house entrance in Oakford, briefly establish the doorstep, then turn
and walk out to the road and the parked car. Enter it,
and drive directly out along the rural road through developing rain to the plane.
Skip the settlement’s complex internal road network. The car follows terrain
pitch and roll rather than staying level to the planet. Stop, exit, walk to the plane,
board, and take off. Explore greenery and waterways, perform a roll over open
country, pass through a precipitation front over snowy mountains and the valley, and later perform a loop over another
open area. Return toward home in changing light, land, transfer back to the car,
and park near the house. Exit and walk back to the same doorstep, facing the door
as in the opening shot.
Morning departure develops into daylight for the main journey and landing.
Evening arrives only near the end of the drive home. The arrival car shot and
walking shot share one viewing direction and camera offset, preventing an orbit
when the explorer exits. Gentle road bends retain cruising speed; parking braking
begins close to the destination.
The stationary closing map holds the evening view, advances through night over
ten seconds, and holds the next morning before the loop repeats.

Ground-to-vehicle transitions stay together in continuous scenes. Selective cuts
may omit repetitive transit while preserving the same explorer, vehicles,
journey, and direction of travel. Natural motion and time to observe each
location take priority over fitting every feature into a short montage.

Selective cuts shorten outbound and return driving, the walk back to the car,
and uneventful straight flight and runway approach. Entry, exit, takeoff, both
maneuvers, landing and the final walk remain continuous. The maneuvers remain
separated by scenic travel over distinct regions. A brief
parking turn is retained before braking and releasing the steering for exit.

The recorder validates occupancy during travel, completion of both maneuvers,
absence of crashes, and the explorer’s physical return. Full capture renders to
an offscreen image so switching to an editor cannot blank the recording. The
encoder rejects missing frames and black video and blends the closing map into
the exact opening frame for repetition. The closing camera uses the saved opening
pose; its orientation is already established during the final walk. The sun
advances to the opening angle during the closing map timelapse. The encoder reuses the opening H.264 frame at the end and
compares decoded frame hashes to guarantee identical endpoint pixels.
