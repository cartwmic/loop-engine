#!/usr/bin/env python3
"""Dummy inner task worker for run-plan-graph tests. Does not call a model."""

from __future__ import annotations

import argparse
import json
import os
import signal
import sys
import time
from pathlib import Path

SEPARATOR = "\n---\n\n"
SUMMARIZER_PREFIX = "Write artifact_root/implementation-report.json"


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


def parse_task_id(rest: str, is_summarizer: bool) -> str:
    if is_summarizer:
        return "summarizer"
    try:
        task = json.loads(rest)
    except json.JSONDecodeError:
        return "unknown"
    task_id = task.get("id") if isinstance(task, dict) else None
    if isinstance(task_id, str) and task_id:
        return task_id
    return "unknown"


def write_valid_report(location: dict[str, object]) -> None:
    artifact_root = location.get("artifact_root")
    plan_path = location.get("plan_path")
    if not isinstance(artifact_root, str) or not artifact_root:
        return
    revision = "1"
    if isinstance(plan_path, str) and plan_path:
        try:
            plan = json.loads(Path(plan_path).read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError):
            plan = {}
        found = plan.get("revision") if isinstance(plan, dict) else None
        if isinstance(found, str) and found:
            revision = found
    report = {
        "revision": "1",
        "author": {"name": "run-plan-graph-dummy", "kind": "script"},
        "plan_revision": revision,
        "coverage": {
            "commit": "dummy",
            "documents": [{"path": "plan.json", "revision": revision}],
        },
        "summary": "dummy summarizer wrote this report",
        "changed_surface": ["dummy"],
        "validation": ["dummy"],
    }
    Path(artifact_root, "implementation-report.json").write_text(
        json.dumps(report) + "\n", encoding="utf-8"
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--receipt-dir", required=True)
    parser.add_argument("--sleep", type=float, default=0.0)
    parser.add_argument("--exit-code", type=int, default=0)
    parser.add_argument("--fail-task")
    parser.add_argument("--write-report", action="store_true")
    parser.add_argument("--summarizer-kill", action="store_true")
    parser.add_argument("--spawn-marker")
    parser.add_argument("--wait-peers", type=int, default=0)
    args = parser.parse_args()

    if args.spawn_marker:
        Path(args.spawn_marker).write_text("spawned\n", encoding="utf-8")

    stdin = sys.stdin.read()
    sys.stdout.write(stdin)
    sys.stdout.flush()

    location, rest, is_summarizer = split_stdin(stdin)
    task_id = parse_task_id(rest, is_summarizer)
    receipt = Path(args.receipt_dir)
    receipt.mkdir(parents=True, exist_ok=True)

    start = time.time()
    (receipt / f"{task_id}.start").write_text(f"{start:.9f}\n", encoding="utf-8")
    (receipt / f"{task_id}.stdin").write_text(stdin, encoding="utf-8")
    (receipt / f"{task_id}.pid").write_text(f"{os.getpid()}\n", encoding="utf-8")

    if is_summarizer and args.summarizer_kill:
        os.kill(os.getpid(), signal.SIGKILL)

    if args.wait_peers > 0 and not is_summarizer:
        deadline = time.time() + 2.0
        overlapped = False
        while time.time() < deadline:
            starts = list(receipt.glob("*.start"))
            starts = [path for path in starts if path.name != "summarizer.start"]
            if len(starts) >= args.wait_peers:
                overlapped = True
                break
            time.sleep(0.01)
        (receipt / f"{task_id}.overlap").write_text(
            "1\n" if overlapped else "0\n", encoding="utf-8"
        )

    if args.sleep > 0:
        time.sleep(args.sleep)

    (receipt / f"{task_id}.end").write_text(f"{time.time():.9f}\n", encoding="utf-8")

    if args.write_report and is_summarizer:
        write_valid_report(location)

    if not is_summarizer and args.fail_task == task_id:
        return 1
    return args.exit_code


if __name__ == "__main__":
    raise SystemExit(main())
