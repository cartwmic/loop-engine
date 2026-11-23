"""Runs scripts/bookends-base.py against a scratch git history and a stand-in `gh`.

The stand-in answers the two `gh api` reads the script makes (a commit's push runs, a
run's jobs) from a table of runs, so each case is one history of pushes to main and what
their runs did. The script runs as CI runs it: a child process with `gh` first on PATH.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent / "bookends-base.py"

FAKE_GH = """#!/usr/bin/env python3
import json, os, sys
db = json.load(open(os.environ["FAKE_GH_DB"]))
argv = sys.argv[1:]
path = argv[argv.index("GET") + 1]
fields = dict(a.split("=", 1) for a in argv if "=" in a and not a.startswith("repos/"))
if path.endswith("/workflows/ci.yml/runs"):
    runs = [r for r in db["runs"] if r["head_sha"] == fields["head_sha"] and r["event"] == fields["event"]]
    print(json.dumps({"workflow_runs": runs}))
elif path.endswith("/jobs"):
    run_id = int(path.split("/")[-2])
    print(json.dumps({"jobs": db["jobs"][str(run_id)]}))
else:
    sys.exit("unexpected gh call: " + " ".join(argv))
"""


def jobs(bookends: str | None, job: str = "tests") -> list[dict]:
    """The jobs of one run: `bookends` is the conclusion of its Bookends step, or None if it never started."""
    steps = [{"name": "Workspace tests", "conclusion": "success"}]
    if bookends is not None:
        steps.append({"name": "Bookends pushed-range gate", "conclusion": bookends})
    return [{"name": "checks", "steps": []}, {"name": job, "steps": steps}]


class BaseTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        root = Path(self.tmp.name)
        self.repo = root / "repo"
        self.repo.mkdir()
        bin_dir = root / "bin"
        bin_dir.mkdir()
        gh = bin_dir / "gh"
        gh.write_text(FAKE_GH)
        gh.chmod(0o755)
        self.env = {
            **os.environ,
            "PATH": f"{bin_dir}{os.pathsep}{os.environ['PATH']}",
            "FAKE_GH_DB": str(root / "db.json"),
            "GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@example.invalid",
            "GIT_COMMITTER_NAME": "t", "GIT_COMMITTER_EMAIL": "t@example.invalid",
        }
        self.git("init", "-q", "-b", "main")
        self.shas: list[str] = []
        for i in range(6):  # c0 oldest .. c5 newest, one commit per push, as a squash-merged main
            self.git("commit", "-q", "--allow-empty", "-m", f"c{i}")
            self.shas.append(self.git("rev-parse", "HEAD").strip())
        self.runs: list[dict] = []
        self.jobs: dict[str, list[dict]] = {}

    def git(self, *args: str) -> str:
        return subprocess.run(["git", *args], cwd=self.repo, env=self.env, check=True, capture_output=True, text=True).stdout

    def add_run(self, commit: int, bookends: str | None, event: str = "push") -> None:
        run_id = 100 + len(self.runs)
        self.runs.append({"id": run_id, "head_sha": self.shas[commit], "event": event})
        self.jobs[str(run_id)] = jobs(bookends)

    def base(self, after: int) -> subprocess.CompletedProcess[str]:
        Path(self.env["FAKE_GH_DB"]).write_text(json.dumps({"runs": self.runs, "jobs": self.jobs}))
        return subprocess.run(
            [sys.executable, str(SCRIPT), self.shas[after]], cwd=self.repo, env=self.env, capture_output=True, text=True, check=False
        )

    def assert_base(self, after: int, base: int) -> None:
        done = self.base(after)
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertEqual(done.stdout.strip(), self.shas[base])

    def test_a_queue_is_checked_from_the_last_run_that_finished_bookends(self) -> None:
        self.add_run(1, "success")
        self.add_run(2, None)  # cancelled by the next push before Bookends started
        self.add_run(3, None)
        self.add_run(4, None)
        self.assert_base(5, 1)  # c2..c5 are all in the range

    def test_a_run_that_failed_bookends_still_ends_the_range(self) -> None:
        self.add_run(2, "success")
        self.add_run(3, "failure")  # its commits were judged: a revert after it is not judged against them again
        self.assert_base(4, 3)

    def test_a_run_that_failed_before_bookends_checked_nothing(self) -> None:
        self.add_run(1, "success")
        self.add_run(2, None)  # tests failed first, so Bookends never ran
        self.assert_base(3, 1)

    def test_a_run_cancelled_inside_the_bookends_step_checked_nothing(self) -> None:
        self.add_run(1, "success")
        self.add_run(2, "cancelled")
        self.assert_base(3, 1)

    def test_a_manual_run_is_not_a_push_run(self) -> None:
        self.add_run(1, "success")
        self.add_run(3, "success", event="workflow_dispatch")  # a manual run on main checks one commit only
        self.assert_base(4, 1)

    def test_the_run_of_the_commit_itself_is_not_its_base(self) -> None:
        self.add_run(1, "success")
        self.add_run(3, "success")  # the run under way for c3: it must not be read as c3's own base
        self.assert_base(3, 1)

    def test_no_earlier_run_fails_closed(self) -> None:
        self.add_run(4, "success")  # a later commit's run says nothing about c1
        done = self.base(1)
        self.assertEqual(done.returncode, 1)
        self.assertEqual(done.stdout, "")
        self.assertIn("no range to check", done.stderr)


if __name__ == "__main__":
    unittest.main()
