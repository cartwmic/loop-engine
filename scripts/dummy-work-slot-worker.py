#!/usr/bin/env python3
"""Dummy bound worker for public-boundary journey proof.

The engine waiter writes a worker packet to stdin. This process records that
packet under ``artifact_root/.work-slot-receipts`` and exits 0. It does not
perform provider work or interpret instruction bodies.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

REQUIRED_KEYS = ("run_id", "slot_id", "artifact_root", "instruction_body")


def main() -> int:
    raw = sys.stdin.read()
    try:
        packet = json.loads(raw)
    except json.JSONDecodeError as error:
        sys.stderr.write(f"dummy worker stdin is not JSON: {error}\n")
        return 1
    if not isinstance(packet, dict):
        sys.stderr.write("dummy worker packet must be a JSON object\n")
        return 1
    missing = [key for key in REQUIRED_KEYS if key not in packet]
    if missing:
        sys.stderr.write(f"dummy worker packet missing keys: {', '.join(missing)}\n")
        return 1
    extra = sorted(set(packet) - set(REQUIRED_KEYS))
    if extra:
        sys.stderr.write(f"dummy worker packet has extra keys: {', '.join(extra)}\n")
        return 1

    artifact_root = packet["artifact_root"]
    if not isinstance(artifact_root, str) or not artifact_root:
        sys.stderr.write("dummy worker artifact_root must be a non-empty string\n")
        return 1
    run_id = packet["run_id"]
    slot_id = packet["slot_id"]
    if not isinstance(run_id, str) or not isinstance(slot_id, str):
        sys.stderr.write("dummy worker run_id and slot_id must be strings\n")
        return 1

    receipts = Path(artifact_root) / ".work-slot-receipts"
    try:
        receipts.mkdir(parents=True, exist_ok=True)
        path = receipts / f"{run_id}--{slot_id}.json"
        path.write_text(json.dumps(packet, indent=2) + "\n", encoding="utf-8")
    except OSError as error:
        sys.stderr.write(f"dummy worker could not write receipt: {error}\n")
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
