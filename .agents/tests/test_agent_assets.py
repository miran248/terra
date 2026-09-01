from __future__ import annotations

import re
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
            ROOT / "scripts",
        )
        self.assertFalse([path for path in legacy_locations if path.exists()])

    def test_agents_hierarchy_is_compact_and_routes_to_project_skills(self) -> None:
        total_chars = sum(len(path.read_text()) for path in AGENTS_FILES)
        self.assertLess(total_chars, 14_000)
        self.assertLess(len((ROOT / "AGENTS.md").read_text()), 2_500)

        combined = "\n".join(path.read_text() for path in AGENTS_FILES)
        for skill in EXPECTED_SKILLS:
            self.assertIn(f".agents/skills/{skill}/SKILL.md", combined)
        self.assertNotIn("DOX framework", combined)

    def test_detailed_contracts_remain_in_canonical_references(self) -> None:
        references = "\n".join(
            path.read_text()
            for path in (ROOT / ".agents/skills").glob("*/references/*.md")
        )
        required_contracts = (
            "Camera3d",
            "AssetCatalogPlugin",
            "prepass_vertex_shader",
            "Avian3d",
            "LevelData",
            "byte-for-byte deterministic",
            "CompletedWorld",
            "CellId",
            "FIFO",
            "locked seeds",
        )
        for contract in required_contracts:
            self.assertIn(contract, references)


if __name__ == "__main__":
    unittest.main()
