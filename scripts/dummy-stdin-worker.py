#!/usr/bin/env python3
"""Dummy stdin recorder for graph-runner and fan-out proofs.

Records raw stdin bytes to ``--receipt PATH``. Default exit 0; ``--exit N``
for failure cases. When stdin contains a run-plan-graph task section and
PATH has no file suffix (or is a directory), writes ``{task_id}.stdin``
inside that directory so one ``--task-worker`` CLI can record every task.
Does not call a model.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import time
from pathlib import Path


def parse_task_id(stdin: str) -> str | None:
    marker = "## task"
    if marker not in stdin:
        return None
    _, raw = stdin.split(marker, 1)
    raw = raw.lstrip("\n")
    try:
        task = json.loads(raw)
    except json.JSONDecodeError:
        return None
    task_id = task.get("id")
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
    args = parser.parse_args()

    raw = sys.stdin.buffer.read()
    text = raw.decode("utf-8", errors="replace")
    task_id = parse_task_id(text)
    dest = receipt_destination(Path(args.receipt), task_id)
    dest.write_bytes(raw)
    dest.with_name(dest.name + ".pid").write_text(f"{os.getpid()}\n", encoding="utf-8")

    if args.sleep > 0:
        time.sleep(args.sleep)

    dest.with_name(dest.name + ".done").write_text("done\n", encoding="utf-8")

    if args.stdout is not None:
        sys.stdout.write(args.stdout)
        sys.stdout.flush()

    if args.fail_task and task_id == args.fail_task:
        return 1
    return args.exit_code


if __name__ == "__main__":
    raise SystemExit(main())
