#!/usr/bin/env python3
"""Black-box proof that nextest times out and cleans a spawned descendant."""

from __future__ import annotations

import argparse
import json
import os
import signal
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import Any

from nextest_policy import CONFIG_PATH, PolicyError, check_installed_version, validate_config

REPO = Path(__file__).resolve().parents[1]
OUTER_BOUND_SECONDS = 120.0
EXPECTED_NEXTEST_EXIT_STATUS = 100
PID_WAIT_SECONDS = 5.0


class ProofError(RuntimeError):
    pass


def process_exists(pid: int) -> bool:
    """Check independently of the test's PID file whether a process remains."""
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    if os.name == "posix":
        result = subprocess.run(
            ["ps", "-p", str(pid), "-o", "pid="],
            capture_output=True,
            text=True,
            check=False,
        )
        return bool(result.stdout.strip())
    return True


def terminate_process_group(process: subprocess.Popen[str]) -> None:
    if os.name == "posix":
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            return
        try:
            process.wait(timeout=1)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait()
    else:
        process.terminate()
        try:
            process.wait(timeout=1)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()


def wait_for_descendant_exit(pid: int) -> bool:
    deadline = time.monotonic() + PID_WAIT_SECONDS
    while time.monotonic() < deadline:
        if not process_exists(pid):
            return False
        time.sleep(0.05)
    return process_exists(pid)


def write_report(path: Path | None, report: dict[str, Any]) -> None:
    if path is None:
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--report", type=Path, help="write the proof record to this path")
    args = parser.parse_args()

    report: dict[str, Any] = {
        "schema_version": 1,
        "artifact_kind": "cargo-nextest-timeout-proof",
        "config_path": str(CONFIG_PATH),
        "outer_bound_seconds": OUTER_BOUND_SECONDS,
    }

    if os.name != "posix":
        report["status"] = "platform-limitation"
        report["platform_limitation"] = (
            "The checked-in wedge uses the Unix sleep executable and this outer proof "
            "does not claim descendant cleanup on non-POSIX hosts."
        )
        write_report(args.report, report)
        print(json.dumps(report, indent=2, sort_keys=True))
        return 0

    try:
        config = validate_config()
        version = check_installed_version()
        report["config"] = config
        report["version"] = version

        with tempfile.TemporaryDirectory(prefix="loop-engine-nextest-timeout-") as directory:
            root = Path(directory)
            pid_file = root / "descendant.pid"
            environment = os.environ.copy()
            environment.update(
                {
                    "CARGO_TERM_COLOR": "never",
                    "LOOP_ENGINE_PROCESS_TIMEOUT_PROBE": "nextest-wedge",
                    "LOOP_ENGINE_PROCESS_TIMEOUT_PID_FILE": str(pid_file),
                }
            )
            command = [
                sys.executable,
                str(REPO / "scripts/run-central-tests.py"),
                "--nextest",
                "--filter",
                "process_timeout_probe",
                "--nextest-run-ignored",
                "only",
            ]
            started = time.monotonic()
            process = subprocess.Popen(
                command,
                cwd=REPO,
                env=environment,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                start_new_session=True,
            )
            timed_out_outer = False
            try:
                stdout, stderr = process.communicate(timeout=OUTER_BOUND_SECONDS)
            except subprocess.TimeoutExpired:
                timed_out_outer = True
                terminate_process_group(process)
                stdout, stderr = process.communicate()
            elapsed = time.monotonic() - started
            combined = stdout + "\n" + stderr
            report["command"] = command
            report["inner_exit_status"] = process.returncode
            report["expected_nextest_exit_status"] = EXPECTED_NEXTEST_EXIT_STATUS
            report["elapsed_seconds"] = round(elapsed, 6)
            report["outer_timed_out"] = timed_out_outer
            report["stdout"] = stdout
            report["stderr"] = stderr

            if timed_out_outer:
                raise ProofError(
                    f"inner nextest proof exceeded outer bound of {OUTER_BOUND_SECONDS}s"
                )
            if process.returncode == 0:
                raise ProofError("inner nextest timeout probe unexpectedly passed")
            expected_failure = f"command failed with exit {EXPECTED_NEXTEST_EXIT_STATUS}: cargo nextest run"
            if expected_failure not in combined:
                raise ProofError(
                    "inner wrapper did not report the expected cargo-nextest exit "
                    f"{EXPECTED_NEXTEST_EXIT_STATUS}"
                )
            if "TIMEOUT" not in combined:
                raise ProofError(
                    "inner nextest result was nonzero but did not contain the TIMEOUT classification"
                )
            if "process_timeout_probe" not in combined:
                raise ProofError("inner nextest output did not name the timeout probe")
            if not pid_file.is_file():
                raise ProofError("timeout probe did not record a descendant PID")
            try:
                pid = int(pid_file.read_text(encoding="utf-8").strip())
            except (OSError, ValueError) as error:
                raise ProofError(f"timeout probe PID record is invalid: {error}") from error
            if pid <= 0:
                raise ProofError("timeout probe recorded a non-positive descendant PID")
            remaining = wait_for_descendant_exit(pid)
            report["descendant"] = {
                "pid": pid,
                "pid_file": str(pid_file),
                "exists_after_cleanup": remaining,
                "cleanup": "passed" if not remaining else "failed",
            }
            if remaining:
                raise ProofError(f"descendant PID {pid} survived nextest timeout cleanup")

        report["status"] = "passed"
        report["classification"] = "timeout"
        write_report(args.report, report)
        print(json.dumps(report, indent=2, sort_keys=True))
        return 0
    except (PolicyError, ProofError) as error:
        report["status"] = "failed"
        report["error"] = str(error)
        write_report(args.report, report)
        print(f"nextest timeout proof failed closed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
