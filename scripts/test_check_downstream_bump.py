"""Tests for scripts/check-downstream-bump.sh.

The script runs for real against two scratch git repositories: a "release" checkout (the tree about to be tagged)
and a "downstream" repository whose importer is a small stand-in that checks one file. The importer's own rules are
tested in the dotfiles repository; this tests what the script adds: it reads the downstream default branch, runs the
importer's check on the right commit, stops with the importer's message, fails closed when the downstream cannot be read,
and leaves nothing behind. No network, no secret.
"""

from __future__ import annotations

import json
import os
import subprocess
import tempfile
import textwrap
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "check-downstream-bump.sh"
IMPORTER = "dot_local/user_scripts/executable_import_release_skills.py"

# Stands in for the dotfiles importer's `check`: it needs docs/needed.md in the commit, the plan file and its own source tree.
STUB_IMPORTER = textwrap.dedent(
    """\
    import subprocess, sys
    args = dict(zip(sys.argv[2::2], sys.argv[3::2]))
    assert sys.argv[1] == "check", sys.argv
    files = subprocess.run(["git", "-C", args["--repo-dir"], "ls-tree", "-r", "--name-only", args["--rev"]],
                           capture_output=True, text=True, check=True).stdout.split()
    open(args["--plan"]).read()
    open(args["--source"] + "/marker").read()
    if "docs/needed.md" not in files:
        print("import_release_skills: using-x expects docs/needed.md in the tag, which no longer has it", file=sys.stderr)
        print("second line of the refusal", file=sys.stderr)
        sys.exit(1)
    print("stub importer: %d files, ok" % len(files))
    """
)


def git(repo: Path, *argv: str) -> str:
    done = subprocess.run(["git", "-C", str(repo), "-c", "user.name=t", "-c", "user.email=t@example.com", *argv],
                          check=True, capture_output=True, text=True)
    return done.stdout.strip()


class CheckDownstreamBumpTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.dir = Path(self.temp.name)
        self.release = self.dir / "release"
        self.release.mkdir()
        git(self.release, "init", "-q", "-b", "main")
        (self.release / "docs").mkdir()
        (self.release / "docs/needed.md").write_text("needed\n")
        git(self.release, "add", "-A")
        git(self.release, "commit", "-q", "-m", "with the file")
        self.with_file = git(self.release, "rev-parse", "HEAD")
        git(self.release, "rm", "-q", "docs/needed.md")
        git(self.release, "commit", "-q", "-m", "without the file")
        self.downstream = self.dir / "downstream"
        (self.downstream / IMPORTER).parent.mkdir(parents=True)
        (self.downstream / IMPORTER).write_text(STUB_IMPORTER)
        (self.downstream / "marker").write_text("x\n")
        git(self.downstream, "init", "-q", "-b", "main")
        git(self.downstream, "add", "-A")
        git(self.downstream, "commit", "-q", "-m", "importer")
        self.plan = self.dir / "plan.json"
        self.plan.write_text(json.dumps({"releases": []}))
        self.scratch = self.dir / "scratch"
        self.scratch.mkdir()

    def run_script(self, *flags: str, **env: str) -> subprocess.CompletedProcess:
        merged = {"PATH": os.environ["PATH"], "HOME": str(self.dir), "TMPDIR": str(self.scratch),
                  "DOWNSTREAM_REPO": self.downstream.as_uri(), **env}  # no token, no other inherited variable
        return subprocess.run(["bash", str(SCRIPT), "--plan", str(self.plan), *flags], cwd=self.release,
                              capture_output=True, text=True, env=merged, timeout=60)

    def test_a_commit_the_importer_can_take_passes(self):
        result = self.run_script("--rev", self.with_file)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("stub importer: 1 files, ok", result.stdout)

    def test_a_commit_that_removes_a_file_the_importer_expects_is_refused_naming_the_file(self):
        result = self.run_script()  # HEAD is the commit without the file
        self.assertEqual(result.returncode, 1)
        self.assertIn("Release refused", result.stderr)
        self.assertIn("docs/needed.md", result.stderr)

    def test_on_github_actions_the_refusal_is_one_annotation_line(self):
        result = self.run_script(GITHUB_ACTIONS="true")
        self.assertEqual(result.returncode, 1)
        annotation = [line for line in result.stdout.splitlines() if line.startswith("::error")]
        self.assertEqual(len(annotation), 1)
        self.assertIn("docs/needed.md", annotation[0])
        self.assertIn("%0Asecond line of the refusal", annotation[0])  # the importer's lines are joined, not cut at the first

    def test_a_downstream_that_cannot_be_read_stops_the_release(self):
        result = self.run_script(DOWNSTREAM_REPO=(self.dir / "nowhere").as_uri())
        self.assertEqual(result.returncode, 1)
        self.assertIn("cannot read", result.stderr)

    def test_a_downstream_with_no_importer_stops_the_release(self):
        git(self.downstream, "rm", "-q", IMPORTER)
        git(self.downstream, "commit", "-q", "-m", "no importer")
        result = self.run_script()
        self.assertEqual(result.returncode, 1)
        self.assertIn("has no " + IMPORTER, result.stderr)

    def test_an_importer_without_the_check_mode_stops_the_release(self):
        (self.downstream / IMPORTER).write_text("import sys\nprint(\"invalid choice: 'check'\", file=sys.stderr)\nsys.exit(2)\n")
        git(self.downstream, "commit", "-q", "-am", "old importer")
        result = self.run_script()
        self.assertEqual(result.returncode, 1)
        self.assertIn("invalid choice", result.stderr)

    def test_it_reads_the_downstream_branch_it_is_given(self):
        git(self.downstream, "switch", "-q", "-c", "other")
        (self.downstream / IMPORTER).write_text("import sys\nprint('the other branch')\nsys.exit(0)\n")
        git(self.downstream, "commit", "-q", "-am", "other importer")
        git(self.downstream, "switch", "-q", "main")
        result = self.run_script(DOWNSTREAM_REF="other")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("the other branch", result.stdout)

    def test_it_leaves_nothing_behind_and_changes_neither_repository(self):
        before = (git(self.release, "status", "--porcelain"), git(self.downstream, "rev-parse", "HEAD"))
        self.run_script("--rev", self.with_file)
        self.run_script()
        self.assertEqual(list(self.scratch.iterdir()), [])
        self.assertEqual((git(self.release, "status", "--porcelain"), git(self.downstream, "rev-parse", "HEAD")), before)

    def test_a_missing_plan_flag_is_a_usage_error(self):
        result = subprocess.run(["bash", str(SCRIPT)], cwd=self.release, capture_output=True, text=True)
        self.assertEqual(result.returncode, 2)


if __name__ == "__main__":
    unittest.main()
