"""Acceptance-runner CLI checks; stop Cargo before rendering or building."""
from pathlib import Path
import shutil
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
