"""Tests for the check-prd-prose.py script in the using-generate-prd skill directory and the skill section that tells agents to run it."""

from __future__ import annotations

import re
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SKILL_DIR = ROOT / "crates" / "research-provider" / "skills" / "using-generate-prd"
SCRIPT = SKILL_DIR / "scripts" / "check-prd-prose.py"
FIXTURES = ROOT / "scripts" / "fixtures" / "prd-prose"
SKILL = SKILL_DIR / "SKILL.md"


ACCEPTED_LINE = "The app must not send note text to any server. This is a promise to users."


def faithful_with(replacement: str) -> str:
    """The faithful sample with one accepted line replaced."""
    text = (FIXTURES / "faithful.md").read_text(encoding="utf-8")
    assert ACCEPTED_LINE in text
    return text.replace(ACCEPTED_LINE, replacement)


def run_texts(accepted: str | bytes, converted: str | bytes) -> subprocess.CompletedProcess[str]:
    """Run the script on two temporary files with exactly these bytes."""
    with tempfile.TemporaryDirectory() as directory:
        paths = []
        for name, content in (("accepted.md", accepted), ("converted.md", converted)):
            path = Path(directory) / name
            path.write_bytes(content if isinstance(content, bytes) else content.encode("utf-8"))
            paths.append(path)
        return run(*paths)


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

    def test_accepted_line_hidden_in_html_comment_is_reported(self) -> None:
        wrappers = {
            "comment on its own lines": f"<!--\n{ACCEPTED_LINE}\n-->",
            "comment opened in the middle of a line": f"Editor note <!-- todo\n{ACCEPTED_LINE}\n-->",
            "comment never closed": f"<!--\n{ACCEPTED_LINE}",
            "comment that closes and reopens on one line": f"<!--\nnote\n--> <!--\n{ACCEPTED_LINE}\n-->",
        }
        for label, replacement in wrappers.items():
            result = run(self.accepted, self.write(faithful_with(replacement)))
            self.assertEqual(result.returncode, 1, f"{label}: {result.stdout}")
            # An unclosed comment hides the rest of the file, so the report groups the run: "lines 15-21".
            self.assertRegex(result.stdout, r"hidden: accepted lines? 15", label)
            self.assertIn("must not send note text", result.stdout, label)
            self.assertNotIn("all 25 accepted lines kept", result.stdout, label)

    def test_accepted_line_hidden_in_code_fence_is_reported(self) -> None:
        for fence in ("```", "~~~~", "  ```", "```python"):
            closing = fence.strip().split("p")[0]
            converted = faithful_with(f"{fence}\n{ACCEPTED_LINE}\n{closing}")
            result = run(self.accepted, self.write(converted))
            self.assertEqual(result.returncode, 1, f"{fence!r}: {result.stdout}")
            self.assertIn("hidden: accepted line 15", result.stdout, repr(fence))

    def test_comment_and_fence_written_in_the_accepted_prose_may_stay(self) -> None:
        accepted = f"Intro.\n\n<!-- A note for editors. -->\n\n```\ncode line\n```\n\n<!--\nlong note\n-->\n\nOutro.\n"
        converted = accepted.replace("Intro.\n", "Intro.\n- Requirement: X-1\n")
        result = run_texts(accepted, converted)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("kept verbatim", result.stdout)

    def test_dropped_list_item_is_reported_as_dropped(self) -> None:
        converted = (FIXTURES / "faithful.md").read_text(encoding="utf-8").replace("- A note body is at most 1 MB.\n", "")
        result = run(self.accepted, self.write(converted))
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn("dropped: accepted line 37: - A note body is at most 1 MB.", result.stdout)
        self.assertNotIn("changed", result.stdout)
        self.assertNotIn("closest converted line", result.stdout)

    def test_a_repeated_accepted_line_with_its_second_copy_hidden_is_reported_as_hidden(self) -> None:
        accepted = "The app is offline.\n\nMiddle.\n\nThe app is offline.\n"
        converted = "The app is offline.\nMiddle.\n<!--\nThe app is offline.\n-->\n"
        result = run_texts(accepted, converted)
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn("hidden: accepted line 5", result.stdout)

    def test_a_converted_line_is_named_closest_for_one_accepted_line_only(self) -> None:
        accepted = "- A note body is at most 1 MB.\n- A note title is at most 1 KB.\n"
        converted = "- A note body can hold at most 1 MB.\n"
        result = run_texts(accepted, converted)
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn("changed: accepted line 1", result.stdout)
        self.assertIn("closest converted line 1", result.stdout)
        self.assertIn("dropped: accepted line 2: - A note title is at most 1 KB.", result.stdout)

    def test_reworded_list_item_names_its_own_closest_line(self) -> None:
        converted = (FIXTURES / "faithful.md").read_text(encoding="utf-8").replace(
            "- A note body is at most 1 MB.", "- A note body can hold at most 1 MB."
        )
        result = run(self.accepted, self.write(converted))
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn("changed: accepted line 37: - A note body is at most 1 MB.", result.stdout)
        self.assertIn("closest converted line 53: - A note body can hold at most 1 MB.", result.stdout)

    def test_utf8_byte_order_mark_on_either_file_is_ignored(self) -> None:
        faithful = (FIXTURES / "faithful.md").read_bytes()
        accepted = self.accepted.read_bytes()
        mark = b"\xef\xbb\xbf"
        for label, left, right in (
            ("converted", accepted, mark + faithful),
            ("accepted", mark + accepted, faithful),
            ("both", mark + accepted, mark + faithful),
        ):
            result = run_texts(left, right)
            self.assertEqual(result.returncode, 0, f"{label}: {result.stdout}{result.stderr}")

    def write(self, text: str) -> Path:
        directory = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, directory, True)
        path = Path(directory) / "converted.md"
        path.write_text(text, encoding="utf-8")
        return path

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

    def test_prose_check_names_a_script_that_ships_with_the_skill(self) -> None:
        # The release copies a skill's own directory and nothing else, so the path the skill names must resolve
        # from that directory. Read the path out of the skill text rather than assuming it.
        text = self.part("Check the prose")
        named = re.search(r"`python3 (\S+check-prd-prose\.py) ", text)
        self.assertIsNotNone(named, "the section does not give the command")
        path = named.group(1)
        self.assertFalse(path.startswith(("/", "crates/")), f"{path} is not relative to the skill's directory")
        self.assertNotIn("crates/", text)
        self.assertTrue((SKILL.parent / path).is_file(), f"{path} does not exist beside the skill")
        self.assertTrue(SCRIPT.samefile(SKILL.parent / path))
        self.assertFalse((ROOT / "scripts" / "check-prd-prose.py").exists(), "a second copy of the script exists")

if __name__ == "__main__":
    unittest.main()
