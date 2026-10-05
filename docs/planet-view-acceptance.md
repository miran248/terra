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
50,000 rows per route repeat; `repeat-N-system-span-summary.csv` reports the
threshold, cap, retained rows, and dropped rows. The existing named window-acquire
and pipeline-queue probes remain unthresholded. System spans may overlap their
containing schedules and render spans, so per-span durations must not be summed.
Per-system span recording adds diagnostic overhead, so never use these traces as
acceptance performance results.

At the measurement boundary, any still-open spans are clipped and closed before
the trace sidecars are drained. Writing raw samples and summaries happens after
the trace stop, outside the measured repeat.

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

The twelve rendered captures and cross-feature behavior are complete. The final
trace-free run failed the frame-stall limit on every route. The user chose to defer
further performance work and accept current pacing for now; no timing pass is claimed.
The unchanged run is preserved at
/tmp/terra-spec-45/acceptance-57-visible-space-0104ff7, built from commit
0104ff7d4c07b4f4870d000720eb6326194fa7f9. Its manifest, raw intervals,
independent recalculation, application log, source hashes, and host sampler are in
that directory. The independent validator reports four recurring-stall failures;
all 12 repeats passed continuous movement, live awake physics, advancing simulation,
and daylight-progression checks.

The run used a 60-second warmup and three 60-second repeats per route, with the
normal asset-review feature and no schedule or work tracing. It ran on an Apple M4
Pro with a 20-core GPU and 24 GB memory. The primary viewport was 2560×1440
physical pixels (1280×720 logical, scale factor 2), FIFO presentation, on the
3024×1964 built-in display. The application stayed on the visible macOS Space;
awake display checks alone do not ensure captures are rendered on the active Space.
The manifest records production shadows disabled, so this run makes no active-shadow
performance claim. Keep every raw interval when reviewing the outliers.

All twelve captures were directly inspected. The ground, settlement, globe, and
opposite-hemisphere views show real, nonblack renders; atmosphere, night darkness,
roads, and labels remain coherent, with no new clipping or overlay defect. Fixed
body/sun ambient values match between globe and opposite views at 86.6294 for noon
and 35 for sunset and night. Static images do not establish motion or pacing.

| Route | Slow intervals by repeat (>33.33 ms) | Repeats affected | p50 range (ms) | p95 range (ms) | p99 range (ms) | Maximum (ms) |
| --- | --- | ---: | ---: | ---: | ---: |
| entry-reversal | 39 / 1 / 0 | 2 of 3 | 8.320–8.337 | 9.108–10.371 | 12.056–24.770 | 1011.830 |
| orbit-zoom | 0 / 3 / 7 | 2 of 3 | 8.335–8.363 | 9.344–9.874 | 11.808–13.568 | 88.823 |
| follow-vehicle-recovery | 1 / 0 / 2 | 2 of 3 | 9.221–9.631 | 17.895–17.953 | 18.920–19.510 | 1070.265 |
| return-reversal | 2 / 0 / 3 | 2 of 3 | 9.745–10.374 | 17.694–17.878 | 19.100–19.218 | 52.320 |

The configured rule rejects a route when at least two repeats contain any interval
above 33.33 ms. Each route meets that rejection condition. The largest isolated
intervals occurred at 1011.830 ms in entry repeat 2 and 1070.265 ms in follow repeat
1; the retained raw data does not establish their cause. The vm_stat sidecar was
sampled without per-row timestamps, so it cannot support precise host-memory/frame
alignment.

Repeat output is currently written synchronously after each measurement and before
the next repeat starts. Since Time<Real> is refreshed before the acceptance Update
system, the first interval of a following repeat may include the prior CSV write.
Those intervals remain in the raw files and in the stall counts; no gameplay sample
was filtered. This boundary effect is a known measurement limitation, but it does
not explain the second-scale stalls observed inside repeats. No boundary change was
made after the user chose to defer further performance work.

Earlier fixes addressed the readout overlap, settlement framing, road-highlight
continuity, camera transition motion, map precipitation visibility, and selector
access while Planet view is active. The separate rendered transition/weather
diagnostic at /tmp/terra-spec-45/diagnostic-57-camera-909b56f-fresh shows the
opening and return framing, snow hidden while mapped and visible again afterward,
and live body movement. A partial instrumented profile at
/tmp/terra-spec-45/phase-profile-57-f998797-exact preserves exact frame-clock
intervals and native spans for entry, orbit, and vehicle-follow routes; it stopped
before the return route and is diagnostic evidence only. Entry stalls overlap both
window-acquire waits and broader render/fixed work, so the remaining cause is not
settled. These checks complement but do not replace the unmet timing gate.
