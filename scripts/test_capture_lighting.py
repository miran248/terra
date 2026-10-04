"""Command-line smoke checks; Cargo is stopped before any renderer/build work."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


class CaptureScriptTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="terra-lighting-test-")
        self.addCleanup(self.temporary.cleanup)
        self.base = Path(self.temporary.name).resolve()
        self.repo = self.base / "repo"
        (self.repo / "scripts").mkdir(parents=True)
        self.script = self.repo / "scripts/capture_lighting.sh"
        shutil.copyfile(Path(__file__).with_name("capture_lighting.sh"), self.script)
        self.bin = self.base / "bin"
        self.bin.mkdir()
        cargo = self.bin / "cargo"
        cargo.write_text("#!/bin/sh\nexit 97\n")
        cargo.chmod(0o755)
        self.env = dict(os.environ, PATH=f"{self.bin}:{os.environ['PATH']}")
        self.git("init", "-q")
        (self.repo / ".gitignore").write_text("crates/main/assets/\n")
        (self.repo / "Cargo.lock").write_text("original\n")
        self.git("add", ".")
        self.git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                 "-c", "commit.gpgsign=false", "commit", "-qm", "fixture")
        assets = self.repo / "crates/main/assets/models"
        assets.mkdir(parents=True)
        (assets / "example.glb").write_bytes(b"abc")

    def git(self, *args):
        return subprocess.run(["git", *args], cwd=self.repo, check=True,
                              capture_output=True, text=True)

    def capture(self, output):
        return subprocess.run(["sh", str(self.script), str(output)], cwd=self.repo,
                              env=self.env, capture_output=True, text=True)

    def test_rejects_output_inside_checkout_before_build(self):
        output = self.repo / "evidence"
        result = self.capture(output)
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn("outside the checkout", result.stderr)
        self.assertFalse(output.exists())

    def test_preserves_existing_evidence(self):
        output = self.base / "evidence"
        output.mkdir()
        original = output / "capture.png"
        original.write_bytes(b"existing evidence")
        result = self.capture(output)
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn("preserving existing evidence", result.stderr)
        self.assertEqual(original.read_bytes(), b"existing evidence")
        self.assertEqual(list(output.iterdir()), [original])

    def test_records_dirty_sources_and_ignored_assets_before_build(self):
        (self.repo / "Cargo.lock").write_text("modified\n")
        (self.repo / "new.rs").write_text("new source\n")
        output = self.base / "evidence"
        result = self.capture(output)
        self.assertEqual(result.returncode, 97, result.stderr)
        self.assertEqual((output / "revision.txt").read_text().strip(),
                         self.git("rev-parse", "HEAD").stdout.strip())
        self.assertIn("+modified", (output / "source.patch").read_text())
        self.assertIn("?? new.rs", (output / "worktree-status.txt").read_text())
        sources = (output / "source-hashes.txt").read_text()
        self.assertIn("  new.rs\n", sources)
        self.assertNotIn("source-hashes.txt", sources)
        self.assertIn(
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
            "  crates/main/assets/models/example.glb\n",
            (output / "hashes.txt").read_text(),
        )


if __name__ == "__main__":
    unittest.main()
