#!/usr/bin/env python3
"""Run the repository's ordinary Rust tests through the pinned cargo-nextest path.

The central runner builds current production and reference-fixture binaries,
then keeps their verified handoff alive while nextest runs workspace unit and
integration tests.  Cargo is invoked separately for doctests because nextest
does not execute doctests.
"""

from __future__ import annotations

import argparse
import os
import shlex
import subprocess
import sys
from pathlib import Path

from nextest_policy import PolicyError, check_installed_version, self_test, validate_config

REPO = Path(__file__).resolve().parents[1]


class RunnerError(RuntimeError):
    pass


def run_phase(command: list[str], environment: dict[str, str]) -> subprocess.CompletedProcess[str]:
    print("+ " + " ".join(shlex.quote(value) for value in command), flush=True)
    result = subprocess.run(command, cwd=REPO, env=environment, check=False)
    if result.returncode:
        raise RunnerError(f"phase failed with exit {result.returncode}: {' '.join(command)}")
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--filter",
        help="focused substring for the central integration target; skips the doctest phase",
    )
    parser.add_argument(
        "--run-ignored",
        choices=("only", "all"),
        help="run ignored central tests (for the opt-in timeout proof)",
    )
    parser.add_argument(
        "--no-doctests",
        action="store_true",
        help="skip Cargo doctests; intended for focused test runs",
    )
    parser.add_argument("--self-test", action="store_true", help="exercise policy rejection cases")
    parser.add_argument(
        "--check-version",
        action="store_true",
        help="check the exact configured cargo-nextest version and stop",
    )
    args = parser.parse_args()

    try:
        if args.self_test:
            self_test()
            print("nextest policy self-test passed")
            return 0

        config = validate_config()
        version = check_installed_version()
        print(
            f"nextest policy: {config['pinned_version']} "
            f"({config['profile']['status_level']}/"
            f"{config['profile']['final_status_level']})",
            flush=True,
        )
        if args.check_version:
            print(version["stdout"], end="")
            return 0

        if args.run_ignored and not args.filter:
            raise RunnerError("--run-ignored requires --filter so an opt-in run cannot broaden scope")

        environment = os.environ.copy()
        environment["CARGO_TERM_COLOR"] = "never"
        central = [sys.executable, str(REPO / "scripts/run-central-tests.py"), "--nextest"]
        if args.filter:
            central.extend(["--filter", args.filter])
        if args.run_ignored:
            central.extend(["--nextest-run-ignored", args.run_ignored])
        run_phase(central, environment)

        if not args.no_doctests and not args.filter and not args.run_ignored:
            run_phase(["cargo", "test", "--locked", "--workspace", "--doc"], environment)
        else:
            print("+ doctest phase skipped for focused/ignored-only invocation", flush=True)
        return 0
    except (PolicyError, RunnerError) as error:
        print(f"ordinary nextest runner failed closed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
