"""Tests for scripts/release-after-merge.sh.

The script runs for real against a scratch bare origin, with a scripted gh on
PATH and the prepare step stubbed by RELEASE_PREPARE_CMD, so no cargo build,
push to GitHub or workflow dispatch happens.
"""

from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
import tempfile
import textwrap
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "release-after-merge.sh"

FAKE_GH = textwrap.dedent(
    r'''
    #!/usr/bin/env python3
    """Scripted gh: just enough of the CLI for release-after-merge.sh."""
    import json, os, subprocess, sys, tempfile
    from pathlib import Path

    home = Path(os.environ["FAKE_GH_DIR"])
    origin = os.environ["FAKE_ORIGIN"]
    argv = sys.argv[1:]
    with open(home / "calls.jsonl", "a") as f:
        f.write(json.dumps(argv) + "\n")
    state = json.loads((home / "state.json").read_text())
    scenario = json.loads((home / "scenario.json").read_text())
    BASE = "https://github.com/test/loop-engine"

    def save():
        (home / "state.json").write_text(json.dumps(state, indent=1))

    def out(value):
        print(json.dumps(value))
        save()
        sys.exit(0)

    def opt(name, default=None):
        return argv[argv.index(name) + 1] if name in argv else default

    def git(*args, cwd=None):
        return subprocess.run(["git", *args], cwd=cwd, check=True, capture_output=True, text=True).stdout.strip()

    def main_sha():
        return git("--git-dir", origin, "rev-parse", "main")

    def tick():
        state["clock"] += 1
        return "2026-10-30T00:00:%02dZ" % state["clock"]

    def finish(run):
        kind = "dry_run" if run["tag"] == "dry-run" else "publish"
        run["status"], run["conclusion"] = "completed", scenario[kind]
        if run["tag"] != "dry-run" and scenario[kind] == "success":
            git("--git-dir", origin, "tag", run["tag"], run["headSha"])
            state["releases"][run["tag"]] = f"{BASE}/releases/tag/{run['tag']}"

    cmd = argv[:2]
    if cmd == ["release", "view"]:
        url = state["releases"].get(argv[2])
        if url is None:
            print("release not found", file=sys.stderr)
            sys.exit(1)
        out({"url": url})
    if cmd == ["pr", "list"]:
        head = opt("--head")
        out([pr for pr in state["prs"] if pr["head"] == head])
    if cmd == ["pr", "create"]:
        number = 100 + len(state["prs"])
        state["prs"].append({"number": number, "state": "OPEN", "url": f"{BASE}/pull/{number}",
                             "head": opt("--head"), "title": opt("--title"), "mergeCommit": None})
        save()
        print(f"{BASE}/pull/{number}")
        sys.exit(0)
    if cmd == ["pr", "checks"]:
        mode = scenario["checks"]
        state["checks_calls"] = state.get("checks_calls", 0) + 1
        if mode == "none" or (mode == "pending_once" and state["checks_calls"] == 1):
            if mode == "none":
                save()
                print("no checks reported on the 'release' branch", file=sys.stderr)
                sys.exit(1)
            out([{"name": "checks", "bucket": "pass"}, {"name": "tests", "bucket": "pending"}])
        tests = "fail" if mode == "fail" else "pass"
        out([{"name": "checks", "bucket": "pass"}, {"name": "tests", "bucket": tests}])
    if cmd == ["pr", "merge"]:
        pr = next(p for p in state["prs"] if p["number"] == int(argv[2]))
        with tempfile.TemporaryDirectory() as tmp:
            git("clone", "--quiet", origin, tmp)
            git("merge", "--squash", "origin/" + pr["head"], cwd=tmp)
            git("commit", "--quiet", "-m", f"{pr['title']} (#{pr['number']})", cwd=tmp)
            sha = git("rev-parse", "HEAD", cwd=tmp)
            if scenario.get("move_main_after_merge"):
                Path(tmp, "other.txt").write_text("moved\n")
                git("add", "other.txt", cwd=tmp)
                git("commit", "--quiet", "-m", "another change", cwd=tmp)
            git("push", "--quiet", "origin", "HEAD:main", cwd=tmp)
        pr["state"], pr["mergeCommit"] = "MERGED", {"oid": sha}
        save()
        sys.exit(0)
    if cmd == ["pr", "view"]:
        pr = next(p for p in state["prs"] if p["number"] == int(argv[2]))
        out(pr)
    if cmd[0] == "api" and argv[1] == "repos/{owner}/{repo}/commits/main":
        out({"sha": main_sha()})
    if cmd == ["run", "list"] and opt("--workflow") == "ci.yml":
        sha, mode = opt("--commit"), scenario["ci"]
        if mode == "missing":
            out([])
        state["ci_calls"] = state.get("ci_calls", 0) + 1
        run = {"databaseId": 1, "status": "completed", "conclusion": "success", "headSha": sha,
               "url": f"{BASE}/actions/runs/1"}
        if mode == "failure":
            run["conclusion"] = "failure"
        if mode == "othersha":
            run["headSha"] = "0" * 40
        if mode == "pending_once" and state["ci_calls"] == 1:
            run["status"], run["conclusion"] = "in_progress", ""
        out([run])
    if cmd == ["run", "list"] and opt("--workflow") == "release.yml":
        if scenario.get("finish_on_list"):
            for run in state["runs"]:
                if run["status"] != "completed":
                    finish(run)
        state["release_list_calls"] = state.get("release_list_calls", 0) + 1
        if state["release_list_calls"] == scenario.get("release_list_error_on"):
            save()
            print("HTTP 502: Server Error", file=sys.stderr)
            sys.exit(1)
        out([{k: r[k] for k in ("databaseId", "status", "conclusion", "headSha", "createdAt", "url")}
             for r in reversed(state["runs"])])
    if cmd == ["workflow", "run"] and argv[2] == "release.yml":
        tag = next(a.split("=", 1)[1] for a in argv if a.startswith("tag="))
        run_id = 5000 + len(state["runs"])
        run = {"databaseId": run_id, "status": "in_progress", "conclusion": "", "headSha": main_sha(),
               "createdAt": tick(), "url": f"{BASE}/actions/runs/{run_id}", "tag": tag, "views": 0}
        state["runs"].append(run)
        if scenario.get("fast_runs"):
            finish(run)
        save()
        sys.exit(0)
    if cmd == ["run", "view"]:
        run = next(r for r in state["runs"] if r["databaseId"] == int(argv[2]))
        run["views"] += 1
        if run["status"] != "completed" and run["views"] >= 2:
            finish(run)
        out({k: run[k] for k in ("status", "conclusion", "headSha", "url")})
    print("fake gh: unsupported call " + json.dumps(argv), file=sys.stderr)
    sys.exit(2)
    '''
).lstrip()

PREPARE_STUB = textwrap.dedent(
    r'''
    #!/usr/bin/env python3
    import json, os, re, sys
    from pathlib import Path
    with open(Path(os.environ["FAKE_GH_DIR"]) / "calls.jsonl", "a") as f:
        f.write(json.dumps(["prepare", sys.argv[1]]) + "\n")
    path = Path("Cargo.toml")
    path.write_text(re.sub(r'version = "[^"]+"', f'version = "{sys.argv[1]}"', path.read_text(), count=1))
    lock = Path("Cargo.lock")
    lock.write_text(re.sub(r'version = "[^"]+"', f'version = "{sys.argv[1]}"', lock.read_text()))
    Path(os.environ["RELEASE_PLAN_FILE"]).write_text("{}\n")
    if os.environ.get("FAKE_PREPARE_EDIT"):
        path.write_text(path.read_text().replace("members = []", 'members = ["x"]'))
    if os.environ.get("FAKE_PREPARE_EXTRA"):
        Path("stray.txt").write_text("not a release file\n")
    '''
).lstrip()

GREEN = {"checks": "pass", "ci": "success", "dry_run": "success", "publish": "success"}


def git(*args: str, cwd: Path | str | None = None) -> str:
    return subprocess.run(["git", *args], cwd=cwd, check=True, capture_output=True, text=True).stdout.strip()


class Harness:
    def __init__(self, root: Path) -> None:
        self.root = root
        self.gh = root / "gh-state"
        self.gh.mkdir()
        self.origin = root / "origin.git"
        bindir = root / "bin"
        bindir.mkdir()
        (bindir / "gh").write_text(FAKE_GH)
        (bindir / "gh").chmod(0o755)
        self.prepare = root / "prepare-stub"
        self.prepare.write_text(PREPARE_STUB)
        self.prepare.chmod(0o755)
        self.notes = root / "notes.md"
        self.notes.write_text("### Added\n\n- A release note.\n")
        self.log = root / "release.log"
        self.env = {
            **os.environ,
            "PATH": f"{bindir}{os.pathsep}{os.environ['PATH']}",
            "FAKE_GH_DIR": str(self.gh),
            "FAKE_ORIGIN": str(self.origin),
            "RELEASE_PREPARE_CMD": str(self.prepare),
            "RELEASE_POLL_SECONDS": "0",
            "RELEASE_APPEAR_POLLS": "3",
            "RELEASE_LOG": str(self.log),
            "RELEASE_PLAN_FILE": str(root / "dist-plan.json"),
            "GIT_AUTHOR_NAME": "test", "GIT_AUTHOR_EMAIL": "test@example.com",
            "GIT_COMMITTER_NAME": "test", "GIT_COMMITTER_EMAIL": "test@example.com",
            "GIT_CONFIG_GLOBAL": os.devnull, "GIT_CONFIG_NOSYSTEM": "1",
        }
        for key in ("RELEASE_WORKTREE", "XDG_STATE_HOME"):
            self.env.pop(key, None)
        git("init", "--quiet", "--bare", "--initial-branch=main", str(self.origin))
        seed = root / "seed"
        git("init", "--quiet", "--initial-branch=main", str(seed))
        (seed / "Cargo.toml").write_text('[workspace]\nmembers = []\n\n[workspace.package]\nversion = "0.34.0"\n')
        (seed / "CHANGELOG.md").write_text("# Changelog\n\n## [0.34.0] - 2026-10-30\n\n- Earlier.\n")
        (seed / "Cargo.lock").write_text('version = 4\n\n[[package]]\nname = "loop-cli"\nversion = "0.34.0"\n')
        self.git(seed, "add", "-A")
        self.git(seed, "commit", "--quiet", "-m", "base")
        self.git(seed, "push", "--quiet", str(self.origin), "main")
        self.checkout = root / "loop-engine"
        self.git(root, "clone", "--quiet", str(self.origin), str(self.checkout))
        self.worktree = root / "loop-engine-release-v0.35.0"
        self.state = {"prs": [], "runs": [], "clock": 0,
                      "releases": {"v0.34.0": "https://github.com/test/loop-engine/releases/tag/v0.34.0"}}
        (self.gh / "state.json").write_text(json.dumps(self.state))
        self.scenario(**GREEN)

    def git(self, cwd: Path, *args: str) -> str:
        return subprocess.run(["git", *args], cwd=cwd, env=self.env, check=True, capture_output=True,
                              text=True).stdout.strip()

    def scenario(self, **values: object) -> None:
        (self.gh / "scenario.json").write_text(json.dumps({**GREEN, **values}))

    def run(self, *args: str, cwd: Path | None = None) -> subprocess.CompletedProcess[str]:
        args = args or ("--notes", str(self.notes))
        return subprocess.run([str(SCRIPT), *args], cwd=cwd or self.checkout, env=self.env, capture_output=True,
                              text=True, timeout=120)

    def resume_runs(self) -> None:
        """Put the last release.yml run back in progress, as if a rerun finds it still running."""
        state = self.gh_state()
        run = state["runs"][-1]
        run["status"], run["conclusion"], run["views"] = "in_progress", "", 0
        (self.gh / "state.json").write_text(json.dumps(state, indent=1))

    def calls(self) -> list[list[str]]:
        path = self.gh / "calls.jsonl"
        return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []

    def events(self) -> list[str]:
        """Calls that matter for ordering, as short labels."""
        labels = []
        for call in self.calls():
            if call[0] == "prepare":
                labels.append("prepare")
            elif call[:2] in (["pr", "create"], ["pr", "checks"], ["pr", "merge"]):
                labels.append(" ".join(call[:2]))
            elif call[:2] == ["run", "list"] and "ci.yml" in call:
                labels.append("ci list")
            elif call[:2] == ["workflow", "run"]:
                labels.append("dispatch " + next(a for a in call if a.startswith("tag=")))
        return labels

    def count(self, label: str) -> int:
        return self.events().count(label)

    def gh_state(self) -> dict:
        return json.loads((self.gh / "state.json").read_text())

    def origin_main(self) -> str:
        return git("--git-dir", str(self.origin), "rev-parse", "main")


class ReleaseAfterMergeTest(unittest.TestCase):
    def setUp(self) -> None:
        # Subtests call setUp again for a fresh origin; each scratch dir is cleaned up.
        tmp = Path(tempfile.mkdtemp(prefix="release-after-merge-"))
        self.addCleanup(shutil.rmtree, tmp, ignore_errors=True)
        self.h = Harness(tmp)

    def assert_stopped(self, result: subprocess.CompletedProcess[str], step: str, text: str) -> None:
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn(f"STOP [{step}]", result.stderr)
        self.assertIn(text, result.stderr)

    def assert_released(self, result: subprocess.CompletedProcess[str]) -> None:
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("released v0.35.0", result.stdout)
        self.assertIn("release: https://github.com/test/loop-engine/releases/tag/v0.35.0", result.stdout)

    # l1
    def test_happy_path_runs_each_step_once_in_order(self) -> None:
        result = self.h.run()
        self.assert_released(result)
        merge = self.h.gh_state()["prs"][0]["mergeCommit"]["oid"]
        self.assertEqual(merge, self.h.origin_main())
        self.assertIn(f"commit: {merge}", result.stdout)
        self.assertIn("import_release_skills.py bump v0.35.0", result.stdout)
        self.assertEqual(self.h.events(), [
            "prepare", "pr create", "pr checks", "pr merge", "ci list", "dispatch tag=dry-run", "dispatch tag=v0.35.0",
        ])
        tree = git("--git-dir", str(self.h.origin), "show", "main:CHANGELOG.md")
        self.assertRegex(tree, r"^# Changelog\n\n## \[0\.35\.0\] - \d{4}-\d{2}-\d{2}\n\n### Added\n\n- A release note\.\n\n## \[0\.34\.0\]")
        self.assertIn('version = "0.35.0"', git("--git-dir", str(self.h.origin), "show", "main:Cargo.toml"))
        self.assertEqual(git("--git-dir", str(self.h.origin), "log", "-1", "--format=%s", "main"),
                         "release: prepare v0.35.0 (#100)")
        runs = self.h.gh_state()["runs"]
        self.assertEqual([(r["tag"], r["headSha"]) for r in runs], [("dry-run", merge), ("v0.35.0", merge)])
        self.assertFalse(self.h.worktree.exists())
        self.assertFalse(Path(self.h.env["RELEASE_PLAN_FILE"]).exists())
        self.assertIn("staged: M CHANGELOG.md;M Cargo.lock;M Cargo.toml;", result.stdout)
        log = self.h.log.read_text()
        self.assertRegex(log, r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z release-\S+ step=publish dispatching release.yml tag=v0.35.0")
        self.assertRegex(log, r"step=done end exit=0 duration=\d+s")

    def test_waits_for_pending_checks_and_ci(self) -> None:
        self.h.scenario(checks="pending_once", ci="pending_once")
        self.assert_released(self.h.run())
        self.assertEqual(self.h.count("pr checks"), 2)
        self.assertEqual(self.h.count("ci list"), 2)

    def test_explicit_version(self) -> None:
        result = self.h.run("--version", "1.0.0", "--notes", str(self.h.notes))
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("released v1.0.0", result.stdout)
        self.assertEqual(self.h.events()[0], "prepare")

    # l2
    def test_already_released_prints_url_and_does_nothing(self) -> None:
        result = self.h.run("--version", "0.34.0")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("v0.34.0 is already released", result.stdout)
        self.assertIn("release: https://github.com/test/loop-engine/releases/tag/v0.34.0", result.stdout)
        self.assertEqual(self.h.events(), [])

    def test_stop_dirty_tree(self) -> None:
        self.h.git(self.h.checkout, "worktree", "add", "--quiet", "-b", "release/v0.35.0", str(self.h.worktree), "origin/main")
        (self.h.worktree / "Cargo.toml").write_text("half prepared\n")
        result = self.h.run()
        self.assert_stopped(result, "prepare", "dirty tree")
        self.assertEqual(self.h.events(), [])

    def test_stop_existing_plan_file(self) -> None:
        Path(self.h.env["RELEASE_PLAN_FILE"]).write_text("{}\n")
        self.assert_stopped(self.h.run(), "prepare", "dist-plan.json exists")
        self.assertEqual(self.h.events(), [])

    def test_stop_unexpected_prepared_file(self) -> None:
        self.h.env["FAKE_PREPARE_EXTRA"] = "1"
        self.assert_stopped(self.h.run(), "pr", "changes more than Cargo.toml, Cargo.lock and CHANGELOG.md: stray.txt")
        self.assertEqual(self.h.events(), ["prepare"])

    def test_stop_unexpected_prepared_content(self) -> None:
        self.h.env["FAKE_PREPARE_EDIT"] = "1"
        result = self.h.run()
        self.assert_stopped(result, "pr", "lines other than version bumps")
        self.assertIn('Cargo.toml: +members = ["x"]', result.stderr)
        self.assertEqual(self.h.events(), ["prepare"])

    def test_default_version_with_nothing_new_on_main_stops(self) -> None:
        self.assert_released(self.h.run())
        before = len(self.h.calls())
        result = self.h.run()
        self.assert_stopped(result, "version", "nothing merged since v0.35.0")
        self.assertNotIn(["pr", "create"], [c[:2] for c in self.h.calls()[before:]])

    def test_stop_prints_pinned_resume_command(self) -> None:
        self.h.scenario(ci="failure")
        result = self.h.run()
        self.assertIn(f"resume with: scripts/release-after-merge.sh --version 0.35.0 --notes {self.h.notes}\n", result.stderr)

    def test_stop_without_notes(self) -> None:
        self.assert_stopped(self.h.run("--version", "0.35.0"), "prepare", "--notes FILE is required")
        self.assertEqual(self.h.events(), [])

    def test_stop_checks_red(self) -> None:
        self.h.scenario(checks="fail")
        self.assert_stopped(self.h.run(), "checks", "checks red on https://github.com/test/loop-engine/pull/100: tests")
        self.assertEqual(self.h.events(), ["prepare", "pr create", "pr checks"])

    def test_stop_no_checks(self) -> None:
        self.h.scenario(checks="none")
        self.assert_stopped(self.h.run(), "checks", "no checks reported")
        self.assertNotIn("pr merge", self.h.events())

    def test_stop_ci_missing_failed_or_other_sha(self) -> None:
        for mode, text in (("missing", "push CI missing"), ("failure", "push CI failed"),
                           ("othersha", "is on another SHA")):
            with self.subTest(mode=mode):
                self.setUp()
                self.h.scenario(ci=mode)
                self.assert_stopped(self.h.run(), "ci", text)
                self.assertEqual(self.h.count("pr merge"), 1)
                self.assertFalse([e for e in self.h.events() if e.startswith("dispatch")])

    def test_stop_main_moved(self) -> None:
        self.h.scenario(move_main_after_merge=True)
        self.assert_stopped(self.h.run(), "ci", "main moved")
        self.assertNotIn("ci list", self.h.events())
        self.assertFalse([e for e in self.h.events() if e.startswith("dispatch")])

    def test_stop_dry_run_failed_without_publish(self) -> None:
        self.h.scenario(dry_run="failure")
        self.assert_stopped(self.h.run(), "dry-run", "dry-run run failed")
        self.assertEqual(self.h.count("dispatch tag=dry-run"), 1)
        self.assertEqual(self.h.count("dispatch tag=v0.35.0"), 0)

    def test_stop_publish_failed(self) -> None:
        self.h.scenario(publish="failure")
        self.assert_stopped(self.h.run(), "publish", "publish run failed")
        self.assertEqual(self.h.count("dispatch tag=v0.35.0"), 1)

    def test_run_that_failed_before_first_read_stops(self) -> None:
        for failure, step, dispatches in (
            ({"dry_run": "failure"}, "dry-run", {"dispatch tag=dry-run": 1, "dispatch tag=v0.35.0": 0}),
            ({"publish": "failure"}, "publish", {"dispatch tag=dry-run": 1, "dispatch tag=v0.35.0": 1}),
        ):
            with self.subTest(failure=failure):
                self.setUp()
                self.h.scenario(fast_runs=True, **failure)
                self.assert_stopped(self.h.run(), step, f"{step} run failed")
                for label, expected in dispatches.items():
                    self.assertEqual(self.h.count(label), expected, (label, self.h.events()))

    def test_resumed_publish_finishing_between_reads_is_not_dispatched_again(self) -> None:
        # A rerun finds the publish running; it finishes (and makes the release) during the run-list read.
        self.h.scenario(publish="failure")
        self.assertEqual(self.h.run().returncode, 1)
        self.h.resume_runs()
        self.h.scenario(finish_on_list=True)
        self.assert_released(self.h.run("--version", "0.35.0"))
        self.assertEqual(self.h.count("dispatch tag=v0.35.0"), 1)
        self.assertEqual(self.h.count("dispatch tag=dry-run"), 1)

    def test_resumed_failed_run_watched_by_this_invocation_stops(self) -> None:
        # A rerun waits on a running publish from an earlier invocation; when it fails, that is a stop.
        self.h.scenario(publish="failure")
        self.assertEqual(self.h.run().returncode, 1)
        self.h.resume_runs()
        self.assert_stopped(self.h.run("--version", "0.35.0"), "publish", "publish run failed")
        self.assertEqual(self.h.count("dispatch tag=v0.35.0"), 1)

    def test_run_from_inside_the_reused_release_worktree(self) -> None:
        self.h.git(self.h.checkout, "worktree", "add", "--quiet", "-b", "release/v0.35.0", str(self.h.worktree), "origin/main")
        self.assert_released(self.h.run(cwd=self.h.worktree))
        self.assertFalse(self.h.worktree.exists())
        self.assertEqual(self.h.count("dispatch tag=v0.35.0"), 1)

    def test_fast_runs_release(self) -> None:
        self.h.scenario(fast_runs=True)
        self.assert_released(self.h.run())
        self.assertEqual(self.h.count("dispatch tag=dry-run"), 1)
        self.assertEqual(self.h.count("dispatch tag=v0.35.0"), 1)

    def test_release_run_list_error_stops_with_resume_command(self) -> None:
        # Call 1 is the state read, 2 the pre-dispatch read, 3 the post-dispatch read.
        for call, step, dispatches in ((1, "release", 0), (2, "dry-run", 0), (3, "dry-run", 1)):
            with self.subTest(call=call):
                self.setUp()
                self.h.scenario(release_list_error_on=call)
                result = self.h.run()
                self.assert_stopped(result, step, "gh run list release.yml failed")
                self.assertNotIn("Traceback", result.stderr)
                self.assertIn("resume with: scripts/release-after-merge.sh --version 0.35.0", result.stderr)
                self.assertEqual(self.h.count("dispatch tag=dry-run"), dispatches)

    # l3
    def test_rerun_resumes_after_each_stop(self) -> None:
        cases = (
            ({"checks": "fail"}, {"pr create": 1, "pr merge": 1}),
            ({"ci": "failure"}, {"pr merge": 1, "dispatch tag=dry-run": 1}),
            ({"ci": "missing"}, {"pr merge": 1, "dispatch tag=dry-run": 1}),
            ({"dry_run": "failure"}, {"pr merge": 1, "dispatch tag=dry-run": 2, "dispatch tag=v0.35.0": 1}),
            ({"publish": "failure"}, {"pr merge": 1, "dispatch tag=dry-run": 1, "dispatch tag=v0.35.0": 2}),
        )
        for failure, counts in cases:
            with self.subTest(failure=failure):
                self.setUp()
                self.h.scenario(**failure)
                self.assertEqual(self.h.run().returncode, 1)
                self.h.scenario()
                # The rerun needs no --notes: the prep PR already exists.
                self.assert_released(self.h.run("--version", "0.35.0"))
                for label, expected in counts.items():
                    self.assertEqual(self.h.count(label), expected, (label, self.h.events()))
                self.assertEqual(self.h.count("prepare"), 1)

    def test_rerun_with_default_version_after_merge_picks_unreleased_main(self) -> None:
        self.h.scenario(ci="failure")
        self.assertEqual(self.h.run().returncode, 1)
        self.h.scenario()
        result = self.h.run("--notes", str(self.h.notes))
        self.assert_released(result)
        self.assertIn("main is at 0.35.0 with no release yet", result.stdout)
        self.assertEqual(self.h.count("pr merge"), 1)

    def test_rerun_after_main_moved_stops_again_without_merging(self) -> None:
        self.h.scenario(move_main_after_merge=True)
        self.assertEqual(self.h.run().returncode, 1)
        self.assert_stopped(self.h.run("--version", "0.35.0"), "ci", "main moved")
        self.assertEqual(self.h.count("pr merge"), 1)

    def test_stop_unowned_path_at_worktree(self) -> None:
        self.h.worktree.mkdir()
        (self.h.worktree / "keep.txt").write_text("someone else's\n")
        self.assert_stopped(self.h.run(), "prepare", "is not this repo's worktree on release/v0.35.0")
        self.assertEqual((self.h.worktree / "keep.txt").read_text(), "someone else's\n")
        self.assertEqual(self.h.count("prepare"), 0)

    def test_stop_worktree_on_another_branch(self) -> None:
        self.h.git(self.h.checkout, "worktree", "add", "--quiet", "-b", "mine", str(self.h.worktree), "origin/main")
        self.assert_stopped(self.h.run(), "prepare", "is not this repo's worktree on release/v0.35.0")
        self.assertEqual(self.h.count("prepare"), 0)

    def test_stop_foreign_commit_on_release_branch(self) -> None:
        self.h.git(self.h.checkout, "worktree", "add", "--quiet", "-b", "release/v0.35.0", str(self.h.worktree), "origin/main")
        (self.h.worktree / "work.txt").write_text("work\n")
        self.h.git(self.h.worktree, "add", "work.txt")
        self.h.git(self.h.worktree, "commit", "--quiet", "-m", "unrelated work")
        head = self.h.git(self.h.worktree, "rev-parse", "HEAD")
        self.assert_stopped(self.h.run(), "prepare", "not the release prep")
        self.assertEqual(self.h.git(self.h.worktree, "rev-parse", "HEAD"), head)
        self.assertEqual(self.h.count("prepare"), 0)

    def test_stop_local_release_branch_with_foreign_commit(self) -> None:
        self.h.git(self.h.checkout, "branch", "release/v0.35.0", "origin/main")
        self.h.git(self.h.checkout, "worktree", "add", "--quiet", str(self.h.root / "tmp-wt"), "release/v0.35.0")
        (self.h.root / "tmp-wt" / "work.txt").write_text("work\n")
        self.h.git(self.h.root / "tmp-wt", "add", "work.txt")
        self.h.git(self.h.root / "tmp-wt", "commit", "--quiet", "-m", "unrelated work")
        self.h.git(self.h.checkout, "worktree", "remove", str(self.h.root / "tmp-wt"))
        self.assert_stopped(self.h.run(), "prepare", "local branch release/v0.35.0 has commits not on main")

    def test_rerun_after_dirty_tree_is_cleaned(self) -> None:
        self.h.git(self.h.checkout, "worktree", "add", "--quiet", "-b", "release/v0.35.0", str(self.h.worktree), "origin/main")
        (self.h.worktree / "Cargo.toml").write_text("half prepared\n")
        self.assertEqual(self.h.run().returncode, 1)
        self.h.git(self.h.worktree, "checkout", "--", "Cargo.toml")
        self.assert_released(self.h.run())
        self.assertEqual(self.h.count("prepare"), 1)

    def test_rerun_waits_on_a_running_dispatch_instead_of_dispatching(self) -> None:
        self.h.scenario(ci="failure")
        self.assertEqual(self.h.run().returncode, 1)
        state = self.h.gh_state()
        merge = state["prs"][0]["mergeCommit"]["oid"]
        state["runs"].append({"databaseId": 4000, "status": "in_progress", "conclusion": "", "headSha": merge,
                              "createdAt": "2026-10-30T00:00:00Z", "tag": "dry-run", "views": 0,
                              "url": "https://github.com/test/loop-engine/actions/runs/4000"})
        (self.h.gh / "state.json").write_text(json.dumps(state))
        self.h.scenario()
        result = self.h.run("--version", "0.35.0")
        self.assert_released(result)
        self.assertIn("waiting for https://github.com/test/loop-engine/actions/runs/4000", result.stdout)
        self.assertEqual(self.h.count("dispatch tag=dry-run"), 0)
        self.assertEqual(self.h.count("dispatch tag=v0.35.0"), 1)

    def test_rerun_after_release_does_nothing(self) -> None:
        self.assert_released(self.h.run())
        before = len(self.h.calls())
        result = self.h.run("--version", "0.35.0")
        self.assertEqual(result.returncode, 0)
        self.assertIn("v0.35.0 is already released", result.stdout)
        self.assertEqual([c[:2] for c in self.h.calls()[before:]], [["release", "view"]])


class AgentsContractTest(unittest.TestCase):
    """l4: AGENTS.md names the script and its standing authorization."""

    def test_release_section_names_script_and_authority(self) -> None:
        text = (ROOT / "AGENTS.md").read_text()
        match = re.search(r"^### Authorized release sequence\n(.*?)(?=^## )", text, re.M | re.S)
        self.assertIsNotNone(match, "AGENTS.md has no Authorized release sequence section")
        section = " ".join(match.group(1).split())
        self.assertIn("scripts/release-after-merge.sh", section)
        self.assertIn("grant 19", section)
        self.assertIn("d9fa9f06", section)
        self.assertRegex(section, r"not\b[^.]*\bLoop Engine run starts")


if __name__ == "__main__":
    unittest.main()
