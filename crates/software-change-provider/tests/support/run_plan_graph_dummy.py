#!/usr/bin/env python3
"""Dummy inner task worker for run-plan-graph tests. Does not call a model."""

from __future__ import annotations

import argparse
import json
import os
import sys
import time
from pathlib import Path


def parse_task_id(stdin: str) -> str:
    marker = "## task"
    if marker not in stdin:
        return "unknown"
    _, raw = stdin.split(marker, 1)
    raw = raw.lstrip("\n")
    try:
        task = json.loads(raw)
    except json.JSONDecodeError:
        return "unknown"
    task_id = task.get("id")
    if isinstance(task_id, str) and task_id:
        return task_id
    return "unknown"


def parse_artifact_root(stdin: str) -> str | None:
    for line in stdin.splitlines():
        if line.startswith("artifact_root: "):
            return line[len("artifact_root: ") :]
    return None


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--receipt-dir", required=True)
    parser.add_argument("--sleep", type=float, default=0.0)
    parser.add_argument("--exit-code", type=int, default=0)
    parser.add_argument("--fail-task")
    parser.add_argument("--write-report", action="store_true")
    parser.add_argument("--spawn-marker")
    parser.add_argument("--wait-peers", type=int, default=0)
    args = parser.parse_args()

    if args.spawn_marker:
        Path(args.spawn_marker).write_text("spawned\n", encoding="utf-8")

    stdin = sys.stdin.read()
    sys.stdout.write(stdin)
    sys.stdout.flush()

    task_id = parse_task_id(stdin)
    artifact_root = parse_artifact_root(stdin)
    receipt = Path(args.receipt_dir)
    receipt.mkdir(parents=True, exist_ok=True)

    start = time.time()
    (receipt / f"{task_id}.start").write_text(f"{start:.9f}\n", encoding="utf-8")
    (receipt / f"{task_id}.stdin").write_text(stdin, encoding="utf-8")
    (receipt / f"{task_id}.pid").write_text(f"{os.getpid()}\n", encoding="utf-8")

    if args.wait_peers > 0:
        deadline = time.time() + 2.0
        overlapped = False
        while time.time() < deadline:
            starts = list(receipt.glob("*.start"))
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

    if args.write_report and artifact_root:
        Path(artifact_root, "implementation-report.json").write_text("{}\n", encoding="utf-8")

    if args.fail_task == task_id:
        return 1
    return args.exit_code


if __name__ == "__main__":
    raise SystemExit(main())
