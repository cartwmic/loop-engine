#!/usr/bin/env python3
"""Dummy stdin recorder for graph-runner and fan-out proofs.

Records raw stdin bytes to ``--receipt PATH``. Default exit 0; ``--exit N``
for failure cases. When stdin contains a compact run-plan-graph task payload
and PATH has no file suffix (or is a directory), writes ``{task_id}.stdin``
inside that directory so one ``--task-worker`` CLI can record every task.
With ``--record-cwd``, writes the actual process cwd beside each stdin
receipt. ``--require-cwd-entry ENTRY`` additionally requires that ENTRY is
visible from that cwd before the worker succeeds.

Detects the summarizer assignment after the compact location JSON separator.
Only that summarizer invocation may write ``artifact_root/implementation-report.json``;
ordinary dummy tasks never write that file. Pass ``--no-report`` to skip the
summarizer write (missing-report proofs). When ``PI_CODING_AGENT_SESSION_DIR``
is set (stdin-exec colocation), writes a dummy session marker there so journeys
can name session traces without a live model. Does not call a model.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import time
from pathlib import Path


SEPARATOR = "\n---\n\n"
SUMMARIZER_PREFIX = "Write artifact_root/implementation-report.json"
REPORT_FILE = "implementation-report.json"
SESSION_ENV = "PI_CODING_AGENT_SESSION_DIR"
SESSION_MARKER = "dummy-session.json"


def split_stdin(stdin: str) -> tuple[dict[str, object], str, bool]:
    if SEPARATOR not in stdin:
        return {}, stdin, False
    raw_location, rest = stdin.split(SEPARATOR, 1)
    try:
        location = json.loads(raw_location.strip())
        if not isinstance(location, dict):
            location = {}
    except json.JSONDecodeError:
        location = {}
    return location, rest, rest.startswith(SUMMARIZER_PREFIX)


def parse_task_id(rest: str, is_summarizer: bool) -> str | None:
    if is_summarizer:
        return "summarizer"
    try:
        task = json.loads(rest)
    except json.JSONDecodeError:
        return None
    task_id = task.get("id") if isinstance(task, dict) else None
    if isinstance(task_id, str) and task_id:
        return task_id
    return None


def receipt_destination(receipt: Path, task_id: str | None) -> Path:
    treat_as_dir = receipt.exists() and receipt.is_dir()
    if not treat_as_dir:
        treat_as_dir = receipt.suffix == ""
    if treat_as_dir:
        receipt.mkdir(parents=True, exist_ok=True)
        name = f"{task_id}.stdin" if task_id else "stdin"
        return receipt / name
    receipt.parent.mkdir(parents=True, exist_ok=True)
    return receipt


def write_valid_report(location: dict[str, object]) -> None:
    artifact_root = location.get("artifact_root")
    plan_path = location.get("plan_path")
    if not isinstance(artifact_root, str) or not artifact_root:
        return
    revision = ""
    resolved_plan = plan_path if isinstance(plan_path, str) and plan_path else str(
        Path(artifact_root) / "plan.json"
    )
    try:
        plan = json.loads(Path(resolved_plan).read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        plan = {}
    found = plan.get("revision") if isinstance(plan, dict) else None
    if isinstance(found, str) and found:
        revision = found
    report = {
        "revision": "1",
        "author": {"name": "dummy-stdin-worker", "kind": "script"},
        "plan_revision": revision,
        "coverage": {
            "commit": "dummy",
            "documents": [{"path": "plan.json", "revision": revision or "none"}],
        },
        "summary": "dummy summarizer wrote this report",
        "changed_surface": ["dummy"],
        "validation": ["dummy"],
    }
    Path(artifact_root, REPORT_FILE).write_text(json.dumps(report) + "\n", encoding="utf-8")


def write_session_marker() -> None:
    raw = os.environ.get(SESSION_ENV)
    if not raw:
        return
    directory = Path(raw)
    directory.mkdir(parents=True, exist_ok=True)
    (directory / SESSION_MARKER).write_text("{}\n", encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--receipt",
        required=True,
        help="Write raw stdin bytes here (file, or directory when PATH has no suffix)",
    )
    parser.add_argument(
        "--exit",
        type=int,
        default=0,
        dest="exit_code",
        help="Process exit code (default 0)",
    )
    parser.add_argument(
        "--sleep",
        type=float,
        default=0.0,
        help="Hold the process after recording stdin so sibling-reap proofs can observe it",
    )
    parser.add_argument(
        "--fail-task",
        default=None,
        help="Exit 1 when the run-plan-graph task id matches this value",
    )
    parser.add_argument(
        "--stdout",
        default=None,
        help="Write this exact string to stdout after recording stdin",
    )
    parser.add_argument(
        "--no-report",
        action="store_true",
        help="Do not write implementation-report.json even when this process is the summarizer",
    )
    parser.add_argument(
        "--record-cwd",
        action="store_true",
        help="Write the actual process cwd beside each stdin receipt",
    )
    parser.add_argument(
        "--require-cwd-entry",
        default=None,
        help="Require this path, relative to the process cwd, before succeeding",
    )
    args = parser.parse_args()

    raw = sys.stdin.buffer.read()
    text = raw.decode("utf-8", errors="replace")
    location, rest, is_summarizer = split_stdin(text)
    task_id = parse_task_id(rest, is_summarizer)
    dest = receipt_destination(Path(args.receipt), task_id)
    dest.write_bytes(raw)
    dest.with_name(dest.name + ".pid").write_text(f"{os.getpid()}\n", encoding="utf-8")

    process_cwd = Path.cwd()
    if args.require_cwd_entry:
        required_entry = Path(args.require_cwd_entry)
        if not required_entry.is_absolute():
            required_entry = process_cwd / required_entry
        if not required_entry.exists():
            print(
                f"required cwd entry is not visible: {required_entry}",
                file=sys.stderr,
            )
            return 1
        dest.with_name(dest.name + ".cwd-entry").write_text(
            str(required_entry) + "\n", encoding="utf-8"
        )
    if args.record_cwd or args.require_cwd_entry:
        dest.with_name(dest.name + ".cwd").write_text(
            str(process_cwd) + "\n", encoding="utf-8"
        )
    write_session_marker()

    if args.sleep > 0:
        time.sleep(args.sleep)

    dest.with_name(dest.name + ".done").write_text("done\n", encoding="utf-8")

    if args.stdout is not None:
        sys.stdout.write(args.stdout)
        sys.stdout.flush()

    if is_summarizer and not args.no_report:
        write_valid_report(location)

    if not is_summarizer and args.fail_task and task_id == args.fail_task:
        return 1
    return args.exit_code


if __name__ == "__main__":
    raise SystemExit(main())
