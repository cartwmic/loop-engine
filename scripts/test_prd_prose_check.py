"""Tests for scripts/check-prd-prose.py and the skill section that tells agents to run it."""

from __future__ import annotations

import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "check-prd-prose.py"
FIXTURES = ROOT / "scripts" / "fixtures" / "prd-prose"
SKILL = ROOT / "crates" / "research-provider" / "skills" / "using-generate-prd" / "SKILL.md"


def run(accepted: Path, converted: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(SCRIPT), str(accepted), str(converted)],
        capture_output=True,
        text=True,
        check=False,
    )


class CheckScriptTest(unittest.TestCase):
    accepted = FIXTURES / "accepted.md"

    def test_faithful_conversion_passes(self) -> None:
        # The faithful sample adds an index, ids and status lines around the prose.
        result = run(self.accepted, FIXTURES / "faithful.md")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("all 25 accepted lines kept verbatim", result.stdout)

    def test_line_endings_and_trailing_spaces_are_ignored(self) -> None:
        text = (FIXTURES / "faithful.md").read_text(encoding="utf-8")
        with tempfile.TemporaryDirectory() as directory:
            converted = Path(directory) / "converted.md"
            converted.write_bytes(text.replace("\n", "  \r\n").encode("utf-8"))
            result = run(self.accepted, converted)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_dropped_obligation_is_reported_by_line_and_text(self) -> None:
        # Every id is still listed in the index and its record is there, so only the prose check can see it.
        result = run(self.accepted, FIXTURES / "dropped.md")
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn("dropped: accepted line 9", result.stdout)
        self.assertIn("shown with a red mark", result.stdout)

    def test_paraphrase_is_reported_with_the_closest_converted_line(self) -> None:
        result = run(self.accepted, FIXTURES / "paraphrased.md")
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn("changed: accepted line 13", result.stdout)
        self.assertIn("closest converted line 20", result.stdout)
        self.assertIn("live as the user types", result.stdout)

    def test_reordered_prose_is_reported_as_moved(self) -> None:
        result = run(self.accepted, FIXTURES / "reordered.md")
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn("moved: accepted line 15", result.stdout)
        self.assertIn("must not send note text", result.stdout)

    def test_a_reflowed_line_is_a_change(self) -> None:
        text = (FIXTURES / "faithful.md").read_text(encoding="utf-8")
        reflowed = text.replace("Results appear as the user types.", "\nResults appear as the user types.")
        with tempfile.TemporaryDirectory() as directory:
            converted = Path(directory) / "converted.md"
            converted.write_text(reflowed, encoding="utf-8")
            result = run(self.accepted, converted)
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn("changed: accepted line 13", result.stdout)

    def test_a_repeated_accepted_line_needs_its_own_copy(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            accepted = Path(directory) / "accepted.md"
            converted = Path(directory) / "converted.md"
            accepted.write_text("The app is offline.\n\nThe app is offline.\n", encoding="utf-8")
            converted.write_text("The app is offline.\n- Status: live\n", encoding="utf-8")
            result = run(accepted, converted)
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn("dropped: accepted line 3", result.stdout)

    def test_unreadable_input_exits_2_with_a_message(self) -> None:
        result = run(self.accepted, FIXTURES / "no-such-file.md")
        self.assertEqual(result.returncode, 2)
        self.assertIn("cannot read input", result.stderr)

    def test_wrong_argument_count_exits_2(self) -> None:
        result = subprocess.run([sys.executable, str(SCRIPT)], capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 2)
        self.assertIn("usage:", result.stderr)


class SkillSectionTest(unittest.TestCase):
    """The skill tells an agent what to do; this checks the parts a conversion depends on are there."""

    @classmethod
    def setUpClass(cls) -> None:
        text = SKILL.read_text(encoding="utf-8")
        start = text.find("\n## Converting an accepted PRD\n")
        assert start != -1, "skill has no '## Converting an accepted PRD' section"
        end = text.find("\n## ", start + 1)
        cls.section = text[start:end if end != -1 else len(text)]

    def part(self, title: str) -> str:
        start = self.section.find(f"\n### {title}\n")
        self.assertNotEqual(start, -1, f"section has no '### {title}' part")
        end = self.section.find("\n### ", start + 1)
        return self.section[start:end if end != -1 else len(self.section)]

    def test_has_the_five_parts(self) -> None:
        for title in (
            "Inclusion test",
            "Keep the accepted prose",
            "Owner sign-off on the outline",
            "Defective samples",
            "Check the prose",
        ):
            self.part(title)

    def test_inclusion_test_names_what_is_not_a_requirement(self) -> None:
        text = self.part("Inclusion test")
        for phrase in ("independently falsifiable", "non-goal", "journey", "open question"):
            self.assertIn(phrase, text)

    def test_sign_off_comes_before_ids(self) -> None:
        text = self.part("Owner sign-off on the outline")
        self.assertIn("before", text)
        self.assertIn("id", text)

    def test_defective_samples_list_each_kind_and_say_what_the_script_catches(self) -> None:
        text = self.part("Defective samples")
        for kind in (
            "non-goal",
            "measurable",
            "journey",
            "contradiction",
            "unverifiable",
            "omitted obligation",
        ):
            self.assertIn(kind, text)
        self.assertIn("The script catches", text)
        self.assertIn("The script does not catch", text)

    def test_prose_check_names_the_script_and_it_exists(self) -> None:
        text = self.part("Check the prose")
        self.assertIn("scripts/check-prd-prose.py", text)
        self.assertTrue(SCRIPT.is_file())


if __name__ == "__main__":
    unittest.main()
