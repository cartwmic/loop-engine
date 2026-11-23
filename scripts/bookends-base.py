#!/usr/bin/env python3
"""Print the commit the push run's Bookends gate should check from, for a push to main at AFTER.

A push run of ci.yml on main can be cancelled by a newer push, so a run's Bookends range
cannot be "what this push added": the commits of a cancelled run would go unchecked. The
range starts instead at the newest commit before AFTER (first-parent) whose push run
reached its Bookends step to the end, passing or failing. Everything after that commit is
in this run's range, so a queue of merges is checked once, by the run that is not cancelled.

A run that failed earlier (before the Bookends step ran) or was cancelled does not count: it
never checked its commits. A run whose Bookends step failed does count: its commits were
judged, and a revert after it must not be judged against the bad commit again.

Usage: bookends-base.py AFTER     (run from a checkout with full history; needs `gh`)
Prints the base sha on stdout. Exits 1, printing the reason on stderr, when no commit
before AFTER has such a run: the gate then has no range to check and must not pass.
"""

from __future__ import annotations

import json
import subprocess
import sys

JOB = "tests"  # keep in sync with the job and step names in .github/workflows/ci.yml
STEP = "Bookends pushed-range gate"
FINISHED = ("success", "failure")  # a step that ran to its end


def run(*argv: str) -> str:
    done = subprocess.run(argv, capture_output=True, text=True, check=False)
    if done.returncode != 0:
        sys.exit(f"{' '.join(argv)} failed: {done.stderr.strip()}")
    return done.stdout


def gh_api(path: str, **fields: str) -> dict:
    args = ["gh", "api", "--method", "GET", path]
    for key, value in fields.items():
        args += ["-f", f"{key}={value}"]
    return json.loads(run(*args))


def bookends_ran(run_id: int) -> bool:
    """Whether any attempt of the run finished its Bookends step (filter=all reads every attempt)."""
    jobs = gh_api(f"repos/{{owner}}/{{repo}}/actions/runs/{run_id}/jobs", filter="all", per_page="100")["jobs"]
    return any(
        step["name"] == STEP and step["conclusion"] in FINISHED
        for job in jobs
        if job["name"] == JOB
        for step in job["steps"]
    )


def base_for(after: str) -> str | None:
    # An unreadable answer exits; it is never taken for "no run".
    parents = run("git", "rev-list", "--first-parent", f"{after}^1").split()
    for sha in parents:
        runs = gh_api(
            "repos/{owner}/{repo}/actions/workflows/ci.yml/runs",
            event="push", branch="main", head_sha=sha, per_page="100",
        )["workflow_runs"]
        for candidate in runs:
            if candidate["head_sha"] == sha and candidate["event"] == "push" and bookends_ran(candidate["id"]):
                print(f"bookends-base: {sha} (run {candidate['id']} finished its Bookends step)", file=sys.stderr)
                return sha
    return None


def main() -> None:
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    base = base_for(sys.argv[1])
    if base is None:
        sys.exit(f"no earlier push run on main finished its Bookends step before {sys.argv[1]}, so there is no range to check")
    print(base)


if __name__ == "__main__":
    main()
