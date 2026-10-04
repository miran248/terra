"""Acceptance-runner CLI checks; stop Cargo before rendering or building."""
from pathlib import Path
import shutil
import subprocess
import test_capture_lighting


class PlanetCaptureScriptTests(test_capture_lighting.CaptureScriptTests):
    def setUp(self):
        super().setUp()
        self.script = self.repo / "scripts/capture_planet.sh"
        shutil.copyfile(Path(__file__).with_name("capture_planet.sh"), self.script)
        for name in ("system_profiler", "sw_vers", "rustc"):
            command = self.bin / name
            command.write_text("#!/bin/sh\nprintf 'fixture hardware/toolchain\\n'\n")
            command.chmod(0o755)

    def test_diagnostic_mode_selects_only_diagnostic_driver_and_records_provenance(self):
        cargo = self.bin / 'cargo'
        cargo.write_text("#!/bin/sh\nprintf '%s\\n' \"diagnostic=$TERRA_PLANET_TRANSITION_DIAGNOSTIC\" \"acceptance=${TERRA_PLANET_ACCEPTANCE_CAPTURE-unset}\" \"revision=$TERRA_SOURCE_REVISION\"\nexit 97\n")
        self.env['TERRA_PLANET_ACCEPTANCE_CAPTURE'] = '/stale/acceptance'
        output = self.base / 'diagnostic'
        result = subprocess.run(['sh', str(self.script), '--diagnostic', str(output)],
                                cwd=self.repo, env=self.env, capture_output=True, text=True)
        self.assertEqual(result.returncode, 97, result.stderr)
        log = (output / 'build.log').read_text()
        self.assertIn(f'diagnostic={output}', log)
        self.assertIn('acceptance=unset', log)
        self.assertIn('revision=' + self.git('rev-parse', 'HEAD').stdout.strip(), log)
        self.assertTrue((output / 'source-hashes.txt').is_file())

    def test_hashes_assets_inside_linked_model_directory(self):
        models = self.repo / "crates/main/assets/models"
        external = self.base / "models"
        models.rename(external)
        models.symlink_to(external, target_is_directory=True)
        output = self.base / "evidence"
        result = self.capture(output)
        self.assertEqual(result.returncode, 97, result.stderr)
        self.assertIn("models/example.glb", (output / "hashes.txt").read_text())


if __name__ == "__main__":
    import unittest
    unittest.main()
