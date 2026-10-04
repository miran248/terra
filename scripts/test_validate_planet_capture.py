"""Validate saved evidence through the public command-line interface."""
import csv
from pathlib import Path
import subprocess
import tempfile
import unittest

ROUTES = ('entry-reversal', 'orbit-zoom', 'follow-vehicle-recovery', 'return-reversal')


class EvidenceTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='terra-evidence-test-')
        self.addCleanup(self.temp.cleanup)
        self.output = Path(self.temp.name)
        (self.output / 'captures').mkdir()
        captures = [f'{view}-{phase}.png' for view in ('ground', 'settlement', 'globe', 'opposite')
                    for phase in ('noon', 'sunset', 'night')]
        for name in captures:
            (self.output / 'captures' / name).write_bytes(b'fixture image')
        (self.output / 'capture-manifest.csv').write_text('file\n' + '\n'.join(captures) + '\n')
        (self.output / 'performance-summary.csv').write_text('route\n' + '\n'.join(ROUTES) + '\n')
        (self.output / 'cross-feature-events.csv').write_text('event\nlive\n')
        (self.output / 'run-configuration.txt').write_text('fixture\n')
        (self.output / 'acceptance-status.txt').write_text('status=complete\n')
        for route in ROUTES:
            directory = self.output / 'performance' / route
            directory.mkdir(parents=True)
            for repeat in (1, 2, 3):
                path = directory / f'repeat-{repeat}-raw.csv'
                with path.open('w', newline='') as file:
                    writer = csv.writer(file)
                    writer.writerow(('real_elapsed_s', 'wall_interval_ms', 'route',
                                     'controlled_x', 'controlled_y', 'controlled_z', 'sun_angle_rad'))
                    for i in range(3000):
                        writer.writerow(((i+1)*.02, 20, route, i*.001, 2000, 0, i*.00001))

    def validate(self):
        return subprocess.run(['python3', str(Path(__file__).with_name('validate_planet_capture.py')),
                               str(self.output)], capture_output=True, text=True)

    def test_validates_all_routes_and_recomputes_unfiltered_statistics(self):
        result = self.validate()
        self.assertEqual(result.returncode, 0, result.stderr)
        with (self.output / 'independent-performance-summary.csv').open() as file:
            rows = list(csv.DictReader(file))
        self.assertEqual(len(rows), 12)
        self.assertEqual(float(rows[0]['p50_ms']), 20)
        self.assertEqual(float(rows[0]['p95_ms']), 20)
        self.assertEqual(float(rows[0]['duration_s']), 60)

    def test_rejects_missing_capture_even_if_driver_says_complete(self):
        (self.output / 'captures/opposite-night.png').unlink()
        result = self.validate()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('opposite-night.png', result.stderr)

    def test_rejects_truncated_measured_repeat(self):
        path = self.output / 'performance/orbit-zoom/repeat-2-raw.csv'
        lines = path.read_text().splitlines()
        path.write_text('\n'.join(lines[:1501]) + '\n')
        result = self.validate()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('shorter than 60 seconds', result.stderr)

    def test_rejects_recurring_slow_frames_without_filtering_them(self):
        for repeat in (1, 2):
            path = self.output / f'performance/orbit-zoom/repeat-{repeat}-raw.csv'
            text = path.read_text().replace('0.02,20,', '0.02,50,', 1)
            path.write_text(text)
        result = self.validate()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('Recurring intervals above 33.33 ms: orbit-zoom', result.stderr)
        with (self.output / 'independent-performance-summary.csv').open() as file:
            rows = list(csv.DictReader(file))
        slow = [row for row in rows if row['route'] == 'orbit-zoom']
        self.assertEqual([int(row['over_33_33ms']) for row in slow], [1, 1, 0])
        self.assertEqual(float(slow[0]['max_ms']), 50)


if __name__ == '__main__':
    unittest.main()
