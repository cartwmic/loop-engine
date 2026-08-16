#!/usr/bin/env python3
"""Shared black-box helpers for frozen work-slot bindings in public journeys."""

from __future__ import annotations

import json
import sys
import time
from pathlib import Path
from typing import Any, Callable, Iterable, Mapping, Sequence

WORKER_SCRIPT = Path(__file__).resolve().parent / "dummy-work-slot-worker.py"
RECEIPT_DIRNAME = ".work-slot-receipts"
BOUND_SLOT_INVOCATION_REQUIRED = "bound-slot-invocation-required"
UNBOUND_WORK_SLOT = "unbound-work-slot"

EngineCall = Callable[[Sequence[str]], dict[str, Any]]


class WorkSlotJourneyFailure(RuntimeError):
    """Assertion failure while proving work-slot delegation."""


def worker_command() -> str:
    return sys.executable


def worker_args(worker: Path | None = None) -> list[str]:
    return [str(worker or WORKER_SCRIPT)]


def bindings_for(slot_ids: Iterable[str], *, worker: Path | None = None) -> dict[str, Any]:
    command = worker_command()
    args = worker_args(worker)
    return {slot_id: {"command": command, "args": list(args)} for slot_id in slot_ids}


def catalog_ids(shown: Mapping[str, Any]) -> list[str]:
    slots = shown.get("work_slots")
    if not isinstance(slots, list):
        raise WorkSlotJourneyFailure("show omitted work_slots catalog")
    ids: list[str] = []
    for slot in slots:
        if not isinstance(slot, dict) or not isinstance(slot.get("id"), str):
            raise WorkSlotJourneyFailure(f"malformed work_slots entry: {slot}")
        extra = set(slot) - {"id", "state", "event"}
        if extra:
            raise WorkSlotJourneyFailure(f"work_slots catalog leaked extra fields {sorted(extra)}")
        ids.append(slot["id"])
    return ids


def assert_catalog(shown: Mapping[str, Any], expected_ids: Sequence[str]) -> None:
    ids = catalog_ids(shown)
    if ids != list(expected_ids):
        raise WorkSlotJourneyFailure(f"unexpected work_slots catalog: {ids}")


def frozen_bindings(shown: Mapping[str, Any]) -> dict[str, Any]:
    initial = shown.get("initial_input")
    if not isinstance(initial, dict):
        raise WorkSlotJourneyFailure("show omitted initial_input")
    bindings = initial.get("work_slot_bindings")
    if not isinstance(bindings, dict):
        raise WorkSlotJourneyFailure("show omitted frozen work_slot_bindings")
    return bindings


def assert_frozen_bindings(shown: Mapping[str, Any], expected: Mapping[str, Any]) -> None:
    bindings = frozen_bindings(shown)
    if bindings != expected:
        raise WorkSlotJourneyFailure(f"frozen work_slot_bindings mismatch: {bindings}")


def assert_bound_redaction(
    shown: Mapping[str, Any],
    *,
    run_id: str,
    slot_id: str,
    command: str,
    args: Sequence[str],
) -> str:
    text = shown.get("current_state_instructions")
    if not isinstance(text, str) or not text:
        raise WorkSlotJourneyFailure("show omitted current_state_instructions")
    args_json = json.dumps(list(args))
    expected = (
        f"Bound work slot `{slot_id}` is configured. "
        f"Frozen worker CLI: command={command} args={args_json}. "
        f"Legal start: loop-engine invoke {run_id} {slot_id}."
    )
    if text != expected:
        raise WorkSlotJourneyFailure(
            f"bound current_state_instructions mismatch:\n{text}\nexpected:\n{expected}"
        )
    return text


def assert_unbound_instructions(shown: Mapping[str, Any], fragment: str) -> None:
    text = shown.get("current_state_instructions")
    if not isinstance(text, str):
        raise WorkSlotJourneyFailure("show omitted current_state_instructions")
    if "Bound work slot" in text or "Legal start: loop-engine invoke" in text:
        raise WorkSlotJourneyFailure(f"unbound state was redacted: {text}")
    if fragment not in text:
        raise WorkSlotJourneyFailure(
            f"unbound instructions missing {fragment!r}: {text}"
        )


def expect_rejection(response: Mapping[str, Any], code: str, *, action: str) -> None:
    if response.get("status") != "rejected":
        raise WorkSlotJourneyFailure(f"{action} expected rejected, got {response}")
    if response.get("code") != code:
        raise WorkSlotJourneyFailure(
            f"{action} expected {code}, got {response.get('code')}: {response}"
        )


def invoke_until_succeeded(
    engine_call: EngineCall,
    run_id: str,
    slot_id: str,
    *,
    timeout_s: float = 10.0,
) -> dict[str, Any]:
    started = engine_call(["invoke", run_id, slot_id])
    if started.get("status") != "completed":
        raise WorkSlotJourneyFailure(f"invoke failed: {started}")
    result = started.get("result") or {}
    invocation_id = result.get("invocation_id")
    if not isinstance(invocation_id, str) or not invocation_id:
        raise WorkSlotJourneyFailure(f"invoke omitted invocation_id: {started}")
    if result.get("slot_id") != slot_id:
        raise WorkSlotJourneyFailure(f"invoke returned wrong slot_id: {started}")

    deadline = time.monotonic() + timeout_s
    last_status = None
    while time.monotonic() < deadline:
        shown = engine_call(["show", run_id])
        if shown.get("status") != "completed":
            raise WorkSlotJourneyFailure(f"show after invoke failed: {shown}")
        projection = shown["result"]
        invocations = projection.get("work_slot_invocations")
        if not isinstance(invocations, list):
            raise WorkSlotJourneyFailure("show omitted work_slot_invocations")
        match = next(
            (
                item
                for item in invocations
                if isinstance(item, dict) and item.get("invocation_id") == invocation_id
            ),
            None,
        )
        if match is None:
            last_status = "missing"
        else:
            if "waiter_pid" in match:
                raise WorkSlotJourneyFailure("show leaked waiter_pid")
            last_status = match.get("status")
            if last_status == "succeeded":
                if match.get("slot_id") != slot_id:
                    raise WorkSlotJourneyFailure(f"succeeded overlay has wrong slot: {match}")
                return match
            if last_status in ("failed", "overrun"):
                raise WorkSlotJourneyFailure(
                    f"bound worker ended {last_status}: {match}"
                )
        time.sleep(0.05)
    raise WorkSlotJourneyFailure(
        f"timed out waiting for succeeded overlay on {slot_id} (last={last_status})"
    )


def receipt_path(artifact_root: str | Path, run_id: str, slot_id: str) -> Path:
    return Path(artifact_root) / RECEIPT_DIRNAME / f"{run_id}--{slot_id}.json"


def assert_packet_receipt(
    artifact_root: str | Path,
    *,
    run_id: str,
    slot_id: str,
    redacted_instructions: str,
) -> dict[str, Any]:
    path = receipt_path(artifact_root, run_id, slot_id)
    try:
        packet = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise WorkSlotJourneyFailure(f"missing dummy worker receipt {path}: {error}") from error
    if not isinstance(packet, dict):
        raise WorkSlotJourneyFailure(f"receipt is not an object: {path}")
    if set(packet) != {"run_id", "slot_id", "artifact_root", "instruction_body"}:
        raise WorkSlotJourneyFailure(f"receipt field set mismatch: {sorted(packet)}")
    if packet["run_id"] != run_id or packet["slot_id"] != slot_id:
        raise WorkSlotJourneyFailure(f"receipt identity mismatch: {packet}")
    if packet["artifact_root"] != str(artifact_root):
        raise WorkSlotJourneyFailure(
            f"receipt artifact_root {packet['artifact_root']!r} != {str(artifact_root)!r}"
        )
    body = packet["instruction_body"]
    if not isinstance(body, str) or not body:
        raise WorkSlotJourneyFailure("receipt omitted instruction_body")
    if body == redacted_instructions or "Legal start: loop-engine invoke" in body:
        raise WorkSlotJourneyFailure("worker packet received redacted show instructions")
    return packet


def prove_bound_visit(
    engine_call: EngineCall,
    *,
    run_id: str,
    catalog: Sequence[str],
    bindings: Mapping[str, Any],
    bound_slot_id: str,
    unbound_slot_id: str,
    gated_event: str,
    artifact_root: str | Path,
    expected_state: str,
    timeout_s: float = 15.0,
) -> dict[str, Any]:
    """Prove sparse binding, redaction, gate, dummy packet, and history."""
    shown_response = engine_call(["show", run_id])
    if shown_response.get("status") != "completed":
        raise WorkSlotJourneyFailure(f"show failed: {shown_response}")
    shown = shown_response.get("result")
    if not isinstance(shown, dict):
        raise WorkSlotJourneyFailure(f"show omitted result: {shown_response}")
    if shown.get("current_state") != expected_state:
        raise WorkSlotJourneyFailure(
            f"expected state {expected_state}, got {shown.get('current_state')}"
        )
    assert_catalog(shown, catalog)
    assert_frozen_bindings(shown, bindings)
    redacted = assert_bound_redaction(
        shown,
        run_id=run_id,
        slot_id=bound_slot_id,
        command=worker_command(),
        args=worker_args(),
    )
    expect_rejection(
        engine_call(["invoke", run_id, unbound_slot_id]),
        UNBOUND_WORK_SLOT,
        action="unbound invoke",
    )
    expect_rejection(
        engine_call(["event", run_id, gated_event]),
        BOUND_SLOT_INVOCATION_REQUIRED,
        action="gated event",
    )
    still = engine_call(["show", run_id])
    if still.get("status") != "completed":
        raise WorkSlotJourneyFailure(f"show after gated event failed: {still}")
    still_result = still.get("result")
    if not isinstance(still_result, dict) or still_result.get("current_state") != expected_state:
        raise WorkSlotJourneyFailure(
            f"gated event moved state: {still_result}"
        )
    overlay = invoke_until_succeeded(
        engine_call, run_id, bound_slot_id, timeout_s=timeout_s
    )
    assert_packet_receipt(
        artifact_root,
        run_id=run_id,
        slot_id=bound_slot_id,
        redacted_instructions=redacted,
    )
    history = engine_call(["history", run_id])
    if history.get("status") != "completed":
        raise WorkSlotJourneyFailure(f"history failed: {history}")
    entries = history.get("result")
    if not isinstance(entries, list):
        raise WorkSlotJourneyFailure(f"history omitted result list: {history}")
    invocation_id = overlay.get("invocation_id")
    if not isinstance(invocation_id, str) or not invocation_id:
        raise WorkSlotJourneyFailure(f"overlay omitted invocation_id: {overlay}")
    assert_invocation_history(entries, invocation_id=invocation_id)
    return overlay


def assert_invocation_history(
    entries: Sequence[Mapping[str, Any]],
    *,
    invocation_id: str,
) -> None:
    started = False
    succeeded = False
    for entry in entries:
        action = entry.get("action") if isinstance(entry, dict) else None
        if not isinstance(action, dict):
            continue
        if action.get("kind") == "invocation_started" and action.get("invocation_id") == invocation_id:
            started = True
        if (
            action.get("kind") == "invocation_status_changed"
            and action.get("invocation_id") == invocation_id
            and action.get("status") == "succeeded"
        ):
            succeeded = True
        if action.get("kind") == "invocation_status_changed" and action.get("status") == "overrun":
            raise WorkSlotJourneyFailure("history recorded overlay overrun as an action")
    if not started:
        raise WorkSlotJourneyFailure(
            f"history omitted invocation_started for {invocation_id}"
        )
    if not succeeded:
        raise WorkSlotJourneyFailure(
            f"history omitted succeeded invocation_status_changed for {invocation_id}"
        )
