# Planet view acceptance

This records the production acceptance boundary for [#45](https://github.com/miran248/terra/issues/45)
[#57](https://github.com/miran248/terra/issues/57), and [#58](https://github.com/miran248/terra/issues/58). Behavioral tests alone do
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

For a visual diagnostic that must not show or focus an application window, add
`--background`:

```sh
CARGO_TARGET_DIR=/tmp/terra-spec58/target61 \
TERRA_PLANET_ASSET_ROOT=/Users/miran/projects/miran248/terra/crates/main \
  scripts/capture_planet.sh --diagnostic --background \
  /tmp/terra-spec58/diagnostic-background
```

This keeps a hidden, unfocused primary window for the diagnostic input fixture,
renders the production main camera and UI into a same-size offscreen image, and
uses continuous updates while unfocused. `capture-mode.txt` and
`diagnostic-configuration.txt` record the backend, window visibility/focus,
image size, scale factor, and source provenance. Background captures can support
visual inspection only; they are not comparable to primary-window performance
measurements and cannot satisfy the rendered acceptance matrix.

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
independent-performance-summary.csv, acceptance-status.txt, application log, source
hashes, and copied host-vmstat.log are in that directory. The sampler's original
file is /tmp/terra-spec-45/host-acceptance-57-visible-space-0104ff7-vmstat.log.
Validator output is /tmp/terra-spec-45/visible-space-validation.txt; it reports
four recurring-stall failures. All 12 repeats passed continuous movement, live
awake physics, advancing simulation, and daylight-progression checks.

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
| --- | --- | ---: | ---: | ---: | ---: | ---: |
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

## Planet navigation and sidebar verification for #58

The final camera diagnostic at `/tmp/terra-spec58/registration-bb77636` was
captured from `bb776360f317d3a74c0234a8ae77f16488786c88` using a hidden offscreen
2560×1440 target (1280×720 logical, scale factor 2):

```sh
CARGO_TARGET_DIR=/tmp/terra-spec-45/target-57 \
TERRA_PLANET_ASSET_ROOT=/Users/miran/projects/miran248/terra/crates/main \
  scripts/capture_planet.sh --diagnostic --background /tmp/terra-spec58/registration-fresh
```

It reports `diagnostic-complete`, 20 captures, no missing captures or errors,
live moving physics, and weather hidden during Planet view and restored on return.
The initial drag reached the opposite hemisphere (direction dot product -0.99980).
Additional real-pointer drags reached far-oblique, middle-distance, and near-polar
poses, with final target errors of 0.00179, 0.00913, and 0.00981 radians. The
before/after images and moving-follow images were inspected for scene, marker,
sidebar, and compass registration. Compass labels remain outside the sidebar;
near-pole directions fade as designed.

The closest requested radius is 2400 m, 400 m above the nominal surface. Follow
measurements use the final steady second of each moving route after zoom and
recenter settle, sampled after camera, UI layout, and transform propagation:

| Moving body | Steady samples | Window (s) | Maximum marker error (physical px) |
| --- | ---: | ---: | ---: |
| Explorer on foot | 166 | 0.994 | 0.337 |
| Car | 128 | 0.997 | 0.408 |
| Plane | 146 | 0.993 | 0.485 |

All three windows measured zero radial lag at the report's six-decimal precision
and zero marker-anchor/body separation. Attained radius stayed within 0.002 m of
2400 m. Gates require radial lag at most 0.00001 radians and marker error at most
1 physical pixel; nearest-pixel layout rounding remains expected. Raw samples,
pose events, capture metadata, source hashes, and build provenance are retained.

Two defects were reproduced before fixing them. Follow reacquisition previously
waited for a smoothing error threshold that a continuously moving target could
keep from reaching; recenter now decays the initial angular offset over one second
relative to the current body direction, then tracks exactly. Occupied-vehicle
markers previously projected the Explorer's stale physics transform; they now
use its synchronized position. The earlier rendered trace showed car and plane
registration errors of 2.618 and 2.112 physical pixels. Regression tests fail with
the old behavior and pass with the fixes. Additional tests cover entry, zoom,
handoffs, recovery, dragging, pointer capture, compass, and sidebar actions.

The short-window sidebar diagnostic can be reproduced from the checkout root:

```sh
TERRA_PLANET_CAPTURE_BACKGROUND=1 \
TERRA_PLANET_SIDEBAR_DIAGNOSTIC=/tmp/terra-spec58/sidebar-fresh \
  cargo run --locked -p main --features asset-review
```

The rendered sidebar route uses 2560×840 physical pixels (1280×420 logical,
scale factor 2), exercising overflow and scrolling. Evidence at
`/tmp/terra-spec58/sidebar-acceptance-63e` reports eight captures and passing
selector pause/Cancel/Car, mouse view close, Follow, wheel isolation, early
recovery cancellation, and completed on-foot recovery checks. Inline selection,
recovery progress and completion, Context, and scrolling captures were directly
inspected. Production fixture tests complement this route for Plane selection,
enter/exit, keyboard recovery, occupied recovery, and capture loss.

Final automated verification passed 209 runtime tests and 157 shared tests.
These background renders establish the recorded camera, overlay, and UI behavior;
they do not establish normal-window performance or absence of display scanout
tearing. The previously deferred #57 timing gate above remains unchanged. Earlier
rejected diagnostic attempts are retained separately and are not acceptance evidence.
