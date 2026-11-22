"""Checks that the four skills that carry the finding-triage tests keep their shared text in sync.

Each skill ships alone, so each one states the four tests itself. The text in SHARED_LINES
is the part every copy must repeat word for word (the first one sits inside a paragraph, the
rest are whole lines). Each skill words the part about what the
tests act on in its own way (the target, the brief, the change), so that part is checked by a
prefix, not word for word.
"""

from __future__ import annotations

import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SKILLS = {
    "engine": ROOT / "skills" / "using-loop-engine" / "SKILL.md",
    "lean-change": ROOT / "crates" / "lean-change-provider" / "skills" / "using-lean-change-provider" / "SKILL.md",
    "research": ROOT / "crates" / "research-provider" / "skills" / "using-research-provider" / "SKILL.md",
    "policy-document": ROOT / "crates" / "policy-document-provider" / "skills" / "using-policy-document-provider" / "SKILL.md",
}
# The two skills that decline a failed candidate and have no per-finding disposition record.
DECLINING = ("research", "policy-document")

SHARED_LINES = (
    "Run the tests in order. When `~/.local/share/agent-harness/finding-triage.md` exists, it states the same rule at more length; this text stands alone.",
    "1. **Real.** It holds when checked against the code, the document or a run. Say how you checked.",
    "2. **Useful.** Fixing it changes something a user, a caller or the next maintainer would notice. Taste and hypotheticals fail, and so does a fix that an existing job or check already does.",
    "3. **Maintainable.** The finding and its fix hold up against the owner's design principles (`~/.local/share/agent-harness/design-principles.md`). A fix that breaks a principle fails. So does a fix that adds machinery the need does not require.",
)
FOUR_TESTS_HEADING = "**Triage each finding on four tests**"
IN_INTENT_PREFIX = "4. **In intent.** It fits the "
FOLLOW_UP_PHRASE = 'for a follow-up: ..."'


def lines_of(skill: str) -> list[str]:
    """The skill's lines with the indentation of a numbered list removed."""
    return [line.strip() for line in SKILLS[skill].read_text(encoding="utf-8").splitlines()]


def triage_section(skill: str) -> str:
    """The text from the four-tests heading to the end of the paragraph that names the failed test."""
    lines = lines_of(skill)
    start = next(i for i, line in enumerate(lines) if line.startswith(FOUR_TESTS_HEADING))
    stop = next(i for i, line in enumerate(lines) if i > start and line.startswith("**Name the failed test:**"))
    return "\n".join(lines[start : stop + 1])


class TriageSkillCopies(unittest.TestCase):
    def test_every_copy_has_the_heading_and_the_shared_lines(self) -> None:
        for skill in SKILLS:
            lines = lines_of(skill)
            text = "\n".join(lines)
            with self.subTest(skill=skill):
                self.assertEqual(
                    sum(line.startswith(FOUR_TESTS_HEADING) for line in lines), 1, f"{SKILLS[skill]}: one four-tests heading"
                )
                for shared in SHARED_LINES:
                    self.assertEqual(
                        text.count(shared), 1, f"{SKILLS[skill]}: expected this text exactly once: {shared[:60]}..."
                    )

    def test_every_copy_has_the_in_intent_test_in_order(self) -> None:
        for skill in SKILLS:
            lines = lines_of(skill)
            with self.subTest(skill=skill):
                fourth = [line for line in lines if line.startswith(IN_INTENT_PREFIX)]
                self.assertEqual(len(fourth), 1, f"{SKILLS[skill]}: one test 4 line starting {IN_INTENT_PREFIX!r}")
                self.assertRegex(fourth[0], r"\balways passes\b", f"{SKILLS[skill]}: test 4 keeps the defect-it-introduces rule")
                self.assertLess(
                    lines.index(SHARED_LINES[3]), lines.index(fourth[0]), f"{SKILLS[skill]}: test 4 follows test 3"
                )

    def test_every_copy_calls_an_in_intent_failure_a_follow_up(self) -> None:
        for skill in SKILLS:
            with self.subTest(skill=skill):
                text = SKILLS[skill].read_text(encoding="utf-8")
                self.assertIn(FOLLOW_UP_PHRASE, text, f"{SKILLS[skill]}: mentions it as {FOLLOW_UP_PHRASE}")

    def test_declining_skills_follow_decision_4664_and_finding_triage(self) -> None:
        for skill in DECLINING:
            with self.subTest(skill=skill):
                section = triage_section(skill)
                self.assertNotIn("such findings", section, f"{SKILLS[skill]}: 'such findings' reads as in-intent only")
                self.assertIn("findings that fail a test (any of the four)", section, f"{SKILLS[skill]}: any of the four tests")
                self.assertIn("is not declined", section, f"{SKILLS[skill]}: an in-intent failure is not declined")
                prefixes = re.search(r"reason that starts with it \(([^)]*)\)", section)
                self.assertIsNotNone(prefixes, f"{SKILLS[skill]}: lists the reason prefixes")
                self.assertNotIn("in intent:", prefixes.group(1), f"{SKILLS[skill]}: 'in intent:' is not a decline prefix")


if __name__ == "__main__":
    unittest.main()
