from __future__ import annotations

import json
import re
import subprocess
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
AGENTS_FILES = (
    ROOT / "AGENTS.md",
    ROOT / "crates/main/AGENTS.md",
    ROOT / "crates/gen_assets/AGENTS.md",
    ROOT / "crates/gen_level/AGENTS.md",
    ROOT / "crates/shared/AGENTS.md",
    ROOT / "crates/shared/src/worldgen/AGENTS.md",
)
EXPECTED_SKILLS = {"terra-assets", "terra-game", "terra-worldgen"}


def assert_contains(test: unittest.TestCase, path: Path, contracts: tuple[str, ...]) -> None:
    contents = path.read_text()
    for contract in contracts:
        test.assertIn(contract, contents, f"missing contract in {path.relative_to(ROOT)}")


class AgentAssetContractTests(unittest.TestCase):
    def test_canonical_skill_names_are_unique(self) -> None:
        skills_root = ROOT / ".agents/skills"
        actual = {path.parent.name for path in skills_root.glob("*/SKILL.md")}
        self.assertEqual(actual, EXPECTED_SKILLS)

        declared = set()
        for skill_file in skills_root.glob("*/SKILL.md"):
            match = re.search(r"^name:\s*([^\s]+)$", skill_file.read_text(), re.MULTILINE)
            if match is None:
                self.fail(f"missing name frontmatter: {skill_file}")
            declared.add(match.group(1))
        self.assertEqual(declared, EXPECTED_SKILLS)

    def test_legacy_agent_asset_locations_are_absent(self) -> None:
        legacy_locations = (
            ROOT / ".hermes/skills",
            ROOT / ".claude/skills",
            ROOT / ".codex/skills",
            ROOT / "skills",
        )
        self.assertFalse([path for path in legacy_locations if path.exists()])

    def test_agents_hierarchy_routes_to_project_skills(self) -> None:
        combined = "\n".join(path.read_text() for path in AGENTS_FILES)
        for skill in EXPECTED_SKILLS:
            self.assertIn(f".agents/skills/{skill}/SKILL.md", combined)
        self.assertNotIn("DOX framework", combined)

    def test_recorded_route_sizes_match_canonical_contexts(self) -> None:
        evidence_path = ROOT / ".agents/context-routes.json"
        self.assertTrue(evidence_path.is_file(), "missing route-specific size evidence")
        evidence = json.loads(evidence_path.read_text())
        self.assertEqual(evidence["measurement"], "Unicode code points")
        self.assertEqual(evidence["baseline_commit"], "77da6fd")

        expected_routes = {
            "main-runtime",
            "worldgen",
            "asset-generation",
            "level-generation-primary",
            "level-generation-with-worldgen",
        }
        self.assertEqual({route["name"] for route in evidence["routes"]}, expected_routes)

        for route in evidence["routes"]:
            with self.subTest(route=route["name"]):
                self.assertIn("baseline_files", route)
                before = sum(
                    len(
                        subprocess.run(
                            ["git", "show", f'{evidence["baseline_commit"]}:{path}'],
                            cwd=ROOT,
                            check=True,
                            capture_output=True,
                            text=True,
                        ).stdout
                    )
                    for path in route["baseline_files"]
                )
                after = sum(len((ROOT / path).read_text()) for path in route["files"])
                self.assertEqual(before, route["before_chars"])
                self.assertEqual(after, route["after_chars"])
                if after > before:
                    self.assertTrue(route.get("regression_rationale"))

    def test_detailed_contracts_remain_in_their_owning_files(self) -> None:
        assert_contains(
            self,
            ROOT / "crates/shared/AGENTS.md",
            (
                "Public APIs must remain stable or be versioned. Breaking public API or "
                "`LevelData` schema changes require workspace-wide checks and regenerated "
                "embedded assets.",
            ),
        )
        assert_contains(
            self,
            ROOT / ".agents/skills/terra-game/references/runtime-contracts.md",
            (
                "The world is a 3D planet (`Camera3d`, PBR meshes, `DirectionalLight`)",
                "`AssetCatalogPlugin` holds `AppState::Loading`",
                "matching `prepass_vertex_shader` with identical math",
                "Actors use Avian3d `RigidBody`, `Collider`, and `Forces`",
                "same `0.4` movement multiplier as underwater movement",
            ),
        )
        assert_contains(
            self,
            ROOT / ".agents/skills/terra-assets/references/pipelines.md",
            (
                "Catalog generation is byte-for-byte deterministic",
                "cargo test -p gen_assets",
                "direct seed-1337 byte comparison",
            ),
        )
        assert_contains(
            self,
            ROOT / ".agents/skills/terra-worldgen/references/architecture.md",
            (
                "public `CompletedWorld` exposes finalized `LevelData` and statistics",
                "`CellId` is authoritative terrain identity; `FaceId` is derived",
                "FIFO reaction order",
                "floating-point operation order for locked seeds",
                "locked-seed fingerprints",
                "strict workspace `cargo clippy`",
                "direct seed-1337 asset comparison",
            ),
        )


if __name__ == "__main__":
    unittest.main()
