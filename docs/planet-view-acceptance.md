# Planet view acceptance

This records the production acceptance boundary for [#45](https://github.com/miran248/terra/issues/45)
and [#57](https://github.com/miran248/terra/issues/57). Behavioral tests alone do
not establish rendered correctness or live performance.

## Reproduce

From the checkout root on the target M4 Pro, choose a new output directory whose
parent exists:

```sh
CARGO_TARGET_DIR=/tmp/terra-spec-45/target-57 \
TERRA_PLANET_ASSET_ROOT=/Users/miran/projects/miran248/terra/crates/main \
  scripts/capture_planet.sh /tmp/terra-spec-45/acceptance-57-run3
```

The asset-root override is only needed when using a worktree without its own
generated assets. A normal checkout defaults to its `crates/main` directory.
The runner uses the pinned toolchain and builds the `asset-review` feature in the
repository's development profile. It honors `CARGO_TARGET_DIR`, retains build and
application logs, and never replaces an existing evidence directory. It validates
all required files and independently recalculates the raw timing statistics in
`independent-performance-summary.csv`. It also rejects stationary or handoff-only
traces and requires observed movement while the physics clock advances, the body
is awake, and collision support is present. Recheck saved evidence without launching:

```sh
python3 scripts/validate_planet_capture.py /tmp/terra-spec-45/acceptance-57-run3
```

The opt-in driver uses `TERRA_PLANET_ACCEPTANCE_CAPTURE`. It runs the production
camera and live simulation; the frozen lighting and other showcase capture modes
must not be combined with it. The runner clears those other capture flags. Keep
the normal primary window on the normal display without resizing it during a
run. The recorded physical/logical viewport, scale factor, and present mode are
the authority for the measured configuration.

A short transition and precipitation diagnostic uses the same provenance runner:

```sh
CARGO_TARGET_DIR=/tmp/terra-spec-45/target-57 \
TERRA_PLANET_ASSET_ROOT=/Users/miran/projects/miran248/terra/crates/main \
  scripts/capture_planet.sh --diagnostic /tmp/terra-spec-45/transition-diagnostic-1
```

This writes a camera trace, transition/storm captures, and `diagnostic-status.txt`.
It checks continuous body motion with live physics and precipitation visibility,
but does not replace the full capture matrix or warmed performance repeats.

## Refinements during acceptance

The user [refined the behavior](https://github.com/miran248/terra/issues/45#issuecomment-5983545954):
rain/snow presentation is hidden throughout Planet view, and vehicle selection
cannot be opened from it. The driver must verify that `V` neither opens a selector
nor pauses the live simulation while opening, browsing or returning. Vehicle
setup occurs outside the view; existing vehicle entry, movement, follow and
recovery remain part of the live routes.

## Evidence contract

- Source revision, dirty patch/status, SHA-256 source and generated-asset hashes,
  hardware/display information, toolchain, build command and runtime settings.
- Twelve primary-window captures: ground, settlement, globe, and opposite
  hemisphere, each at noon, sunset, and night. The manifest records the actual
  camera, body, sun, and rendering settings.
- Four live routes: entry/reversal, orbit/zoom, follow/vehicle/recovery, and
  return/reversal. Each has a warmup and three measured 60-second repeats with
  moving bodies, physics, streaming, and world animation active.
- Unfiltered real-time frame intervals in a raw CSV per repeat, plus median,
  p95, p99, maximum, and counts over 33.33 ms. Screenshots are outside measured
  windows. Cross-feature events and the final acceptance status remain alongside
  the raw evidence.

Prefer p95 at or below 16.67 ms. Investigate intervals over 33.33 ms and reject
recurring stalls below the 30 FPS minimum. For this acceptance run, a route is
rejected when at least two of its three repeats contain an interval above
1000/30 ms; no slow frames are filtered out. If tuning is needed, reduce distant
visual detail and shadow cost before navigation readability or simulation
correctness. Validate fog/atmosphere continuity, depth and clipping, ambient
stability during detached orbit, night navigation, labels, progressive detail,
camera clearance, fades, and displaced-material prepass parity.

## Recorded outcome

Acceptance remains pending. The first actual renderer pass is preserved at
`/tmp/terra-spec-45/acceptance-57-run1`; it is rejected and contains no accepted
performance measurements.

That pass produced all twelve Metal-rendered primary-window PNGs on an Apple
M4 Pro (20-core GPU). The application viewport was 2560×1440 physical pixels,
1280×720 logical pixels, scale factor 2, with FIFO presentation on the normal
3024×1964 Retina display. Inspection found that the gameplay readout remained
opaque over the destination panel, and the settlement capture used a near-ground
radius that could not show the settlement. The capture configuration also called
noon 60° even though the fixed solar orbit reaches only 23.4° at this anchor.
These require correction before acceptance; the actual vectors and elevations
remain recorded in the manifest.

The inspected globe/night and opposite captures retain a dark night side and
readable settlement labels and roads. Ambient brightness matches exactly between
globe and opposite views for each fixed body/sun phase (86.6294 at local noon,
35 at sunset/night). These are limited observations from the rejected matrix,
not a completed visual or performance acceptance claim.


A second partial pass is preserved at
`/tmp/terra-spec-45/acceptance-57-final` (the directory name does not imply
acceptance). It contains twelve corrected captures and one measured 60-second
entry/reversal repeat after warmup. The readout overlap and settlement framing
were corrected. Inspection found discontinuities in the road highlights, and the
user reported uneven speed, unwanted rotation and unsatisfactory paths during
entry/exit. These remain under investigation.

The partial repeat recorded 3,500 intervals: median 16.677 ms, p95 18.808 ms,
p99 28.197 ms, maximum 268.487 ms, and nine intervals above 33.33 ms. Recorded
controlled-body positions did not move, so this repeat does not meet the live
route contract. The run was stopped before repeat two. Its missing completion
status correctly causes independent validation to reject it. Normal settings
recorded shadows disabled; no active-shadow performance claim is made.
