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

REQUIRED_KEYS = ("run_id", "slot_id", "artifact_root", "instruction_body", "capture_dir")
OPTIONAL_KEYS = (
    "context",
    "standing_assignment_ids",
    "assignment_selection",
    "invocation_input",
    "controls",
    "binding_sha256",
    "state_visit",
    "transition_history",
)


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
    extra = sorted(set(packet) - set(REQUIRED_KEYS) - set(OPTIONAL_KEYS))
    if extra:
        sys.stderr.write(f"dummy worker packet has extra keys: {', '.join(extra)}\n")
        return 1

    for key in ("context", "standing_assignment_ids", "assignment_selection"):
        if key in packet and (not isinstance(packet[key], list) or
                not all(isinstance(item, dict if key == "context" else str) for item in packet[key])):
            sys.stderr.write(f"dummy worker {key} has invalid transport shape\n")
            return 1
    if "controls" in packet and not isinstance(packet["controls"], dict):
        sys.stderr.write("dummy worker controls must be an object\n")
        return 1
    if "binding_sha256" in packet and (
        not isinstance(packet["binding_sha256"], str)
        or not packet["binding_sha256"].startswith("sha256:")
    ):
        sys.stderr.write("dummy worker binding_sha256 must be a sha256 string\n")
        return 1
    if "state_visit" in packet and (
        not isinstance(packet["state_visit"], int)
        or isinstance(packet["state_visit"], bool)
        or packet["state_visit"] < 0
    ):
        sys.stderr.write("dummy worker state_visit must be a non-negative integer\n")
        return 1
    if "transition_history" in packet and (
        not isinstance(packet["transition_history"], list)
        or not all(isinstance(item, dict) for item in packet["transition_history"])
    ):
        sys.stderr.write("dummy worker transition_history must be a list of objects\n")
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
