#!/usr/bin/env python3
"""Check saved Planet view evidence and independently summarize every raw frame."""
import csv
import math
from pathlib import Path
import statistics
import sys

ROUTES = ('entry-reversal', 'orbit-zoom', 'follow-vehicle-recovery', 'return-reversal')
CAPTURES = tuple(f'{view}-{phase}.png'
                 for view in ('ground', 'settlement', 'globe', 'opposite')
                 for phase in ('noon', 'sunset', 'night'))


def require_file(path):
    if not path.is_file() or not path.stat().st_size:
        raise ValueError(f'Missing or empty acceptance evidence: {path}')
    return path


def read_csv(path):
    with require_file(path).open(newline='') as source:
        rows = list(csv.DictReader(source))
    if not rows:
        raise ValueError(f'Acceptance CSV contains no observations: {path}')
    return rows


def finite(value):
    number = float(value)
    if not math.isfinite(number):
        raise ValueError(f'Non-finite observation: {value}')
    return number


def validate(output):
    for name in CAPTURES:
        require_file(output / 'captures' / name)
    manifest = read_csv(output / 'capture-manifest.csv')
    if sorted(Path(row['file']).name for row in manifest) != sorted(CAPTURES):
        raise ValueError('Capture manifest does not identify exactly the twelve required images')
    read_csv(output / 'performance-summary.csv')
    read_csv(output / 'cross-feature-events.csv')
    require_file(output / 'run-configuration.txt')
    status = require_file(output / 'acceptance-status.txt').read_text().splitlines()
    summaries = []
    failures = []
    for route in ROUTES:
        slow_repeats = 0
        for repeat in (1, 2, 3):
            path = output / 'performance' / route / f'repeat-{repeat}-raw.csv'
            rows = read_csv(path)
            if any(row['route'] != route for row in rows):
                raise ValueError(f'Incorrect route labels: {path}')
            elapsed = [finite(row['real_elapsed_s']) for row in rows]
            intervals = [finite(row['wall_interval_ms']) for row in rows]
            if any(value <= 0 for value in intervals):
                raise ValueError(f'Non-positive frame interval: {path}')
            if any(after <= before for before, after in zip(elapsed, elapsed[1:])):
                raise ValueError(f'Frame elapsed time does not increase: {path}')
            duration = math.fsum(intervals) / 1000
            if duration < 59.9 or elapsed[-1] < 59.9 or elapsed[0] > 1:
                raise ValueError(f'Measured repeat is shorter than 60 seconds: {path}')
            positions = [tuple(finite(row[f'controlled_{axis}']) for axis in 'xyz') for row in rows]
            fixed = [finite(row['fixed_time_elapsed_s']) for row in rows]
            traveled = 0.0
            live_steps = 0
            for index in range(1, len(rows)):
                before, after = rows[index - 1], rows[index]
                step = math.dist(positions[index - 1], positions[index])
                # Exclude body handoffs and discontinuous relocations from travel.
                if before['controlled_kind'] != after['controlled_kind'] or step > 12:
                    continue
                traveled += step
                if (step > 1e-5 and fixed[index] > fixed[index - 1]
                        and after['controlled_sleeping'] == 'false'
                        and finite(after['resident_colliders']) > 0):
                    live_steps += 1
            if traveled < 10:
                failures.append(f'Insufficient continuous controlled-body motion: {path}')
            if not live_steps:
                failures.append(f'No observed live physics motion: {path}')
            sun_angles = [finite(row['sun_angle_rad']) for row in rows]
            if max(sun_angles) - min(sun_angles) <= 1e-6:
                failures.append(f'No observed day/night progression: {path}')
            ordered = sorted(intervals)
            # Nearest-rank percentiles; retain every interval, including stalls.
            p95 = ordered[max(0, math.ceil(len(ordered) * .95) - 1)]
            p99 = ordered[max(0, math.ceil(len(ordered) * .99) - 1)]
            over = sum(value > 1000 / 30 for value in intervals)
            slow_repeats += over > 0
            summaries.append(dict(route=route, repeat=repeat, sample_count=len(rows),
                                  duration_s=duration, p50_ms=statistics.median(intervals),
                                  p95_ms=p95, p99_ms=p99, max_ms=max(intervals),
                                  over_33_33ms=over, controlled_travel_m=traveled,
                                  live_physics_steps=live_steps))
        if slow_repeats >= 2:
            failures.append(f'Recurring intervals above 33.33 ms: {route} ({slow_repeats}/3 repeats)')
    with (output / 'independent-performance-summary.csv').open('w', newline='') as result:
        writer = csv.DictWriter(result, fieldnames=list(summaries[0]))
        writer.writeheader()
        writer.writerows(summaries)
    if 'status=complete' not in status:
        failures.append('Driver did not report status=complete; inspect acceptance-status.txt')
    if failures:
        raise ValueError('\n'.join(failures))
    print(f'Complete live evidence verified in {output}; rendered inspection is still required.')


if __name__ == '__main__':
    if len(sys.argv) != 2:
        sys.exit('Usage: validate_planet_capture.py EVIDENCE_DIRECTORY')
    try:
        validate(Path(sys.argv[1]))
    except (OSError, ValueError, KeyError) as error:
        sys.exit(str(error))
