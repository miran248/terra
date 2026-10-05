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

For frame-spike diagnosis, set `TERRA_PLANET_WORK_TRACE=1` with the normal
launcher. This adds `planet-work-trace.csv` for all four measured routes and records
`system_stage_trace=enabled` in the configuration. Rows identify the route, repeat,
route-relative time, absolute `Time<Real>` elapsed time, and scene/road work. Use
these timestamps to align work with raw frame intervals, allowing for deferred
render work in following frames. The sidecar does not change captures, route
timing, warm-ups, repeats, or acceptance thresholds.

For deeper phase attribution, also set `TERRA_PLANET_SCHEDULE_TRACE=1`. The runner
selects the separate `asset-review-schedule-trace` feature and records a profile
caveat alongside the exact build command. This collects native Bevy schedule spans,
including fixed-step and render schedules, plus the existing `main_render_schedule`
and `present_frames` spans. The latter distinguishes RenderGraph time from the
remaining render submit and window-present path. Treat this as a diagnostic build
and repeat final performance acceptance without schedule tracing.

`repeat-N-frame-clock.csv` also records `time_interval_start_s` and
`time_interval_end_s` from `Time<Real>::last_update() - delta` through
`last_update()`, relative to the route repeat's monotonic clock. The late
`wall_elapsed_s` sample remains available for comparison but is not an exact span
join. Other named Bevy system spans are retained only at or above 1 ms, capped at
10,000 rows per route repeat; `repeat-N-system-span-summary.csv` reports the
threshold, cap, retained rows, and dropped rows. The existing named window-acquire
and pipeline-queue probes remain unthresholded. Per-system span recording adds
diagnostic overhead, so never use these traces as acceptance performance results.

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

The camera refinement covers uneven speed or snaps, unwanted rotation or roll,
and travel path or framing. Check ordinary entry and return as well as interrupted
transitions. The short diagnostic records requested and actual capture times so
an image taken during a reversal cannot be mistaken for uninterrupted opening.

## Evidence contract

- Source revision, dirty patch/status, SHA-256 source and generated-asset hashes,
  hardware/display information, toolchain, build command and runtime settings.
- Twelve primary-window captures: ground, settlement, globe, and opposite
  hemisphere, each at noon, sunset, and night. The manifest records the actual
  camera, body, sun, and rendering settings.
- Four live routes: entry/reversal, orbit/zoom, follow/vehicle/recovery, and
  return/reversal. Each has a warmup and three measured 60-second repeats with
  moving bodies, physics, streaming, and world animation active.
- The vehicle route validates one live grounded player pose at startup. Before
  its warmup and each measured repeat, it restores that pose through the normal
  safe Teleport action, waits for grounded settling, and completes a normal V/C
  summon within the local search radius. This setup is outside the route clocks;
  each measured 60-second repeat still runs its ordinary timed V/C handoff.
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

The third pass, `/tmp/terra-spec-45/acceptance-57-run3`, contains twelve inspected
captures with continuous road highlights and corrected interface presentation.
The companion `/tmp/terra-spec-45/diagnostic-57-camera-909b56f-fresh` completed all
eight transition/weather captures, six map toggles, opposite-side orbit, and
15.23 m of movement with live physics. Its early opening image keeps the terrain
and controlled-body marker in frame; the return image shows the settled chase
view. Snow is absent in the map image and visible again after return.

Run three is nevertheless rejected for recurring entry/reversal stalls. All
three warmed 60-second repeats passed movement and physics gates, with paths of
210.19–224.68 m and 49.61–49.63 seconds of advancing fixed simulation time. Their
p95 frame times were 11.48–11.64 ms, but each contained 39–46 intervals above
33.33 ms, and maxima were 156.58–184.48 ms. The remaining routes were stopped
after this rejection. Subsequent short diagnostic probes isolated bulk scenery
removal as a major contributor; they do not replace full performance acceptance.
