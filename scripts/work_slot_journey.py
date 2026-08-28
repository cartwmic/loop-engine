#!/usr/bin/env python3
"""Shared black-box helpers for frozen work-slot bindings in public journeys."""

from __future__ import annotations

import hashlib
import json
import os
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import Any, Callable, Iterable, Mapping, Sequence

WORKER_SCRIPT = Path(__file__).resolve().parent / "dummy-work-slot-worker.py"
STDIN_WORKER_SCRIPT = Path(__file__).resolve().parent / "dummy-stdin-worker.py"
RECEIPT_DIRNAME = ".work-slot-receipts"
BOUND_SLOT_INVOCATION_REQUIRED = "bound-slot-invocation-required"
UNBOUND_WORK_SLOT = "unbound-work-slot"
PATH_SOFTWARE_CHANGE = "software-change"
PATH_LOOP_ENGINE = "loop-engine"
SHIPPED_IMPLEMENT_ARGS = ["run-plan-graph"]
SHIPPED_FAN_OUT_ARGS = ["fan-out"]
SHIPPED_REVIEW_SLOT_IDS = ("design-review", "plan-review", "implementation-review")
SHIPPED_UNBOUND_SLOT_IDS = (
    "implement",
    "design-review",
    "plan-review",
    "implementation-review",
    "validation-draft",
)
SHIPPED_PROFILE_RELATIVE = Path("crates/software-change-provider/data/configs")
SHIPPED_PROFILE_NAMES = ("minimal.json", "standard.json", "high-rigor.json")
SOFTWARE_CHANGE_FIXTURES = (
    ("intent.json", "intent-good.json"),
    ("design.json", "design-good.json"),
    ("plan.json", "plan-good.json"),
    ("implementation-report.json", "implementation-report-good.json"),
    ("validation-report.json", "validation-report-good.json"),
)
SOFTWARE_CHANGE_GATE_SUBJECT = {
    "intent-review": "intent.json",
    "intent-adversarial-review": "intent.json",
    "design-review": "design.json",
    "design-adversarial-review": "design.json",
    "plan-review": "plan.json",
    "plan-adversarial-review": "plan.json",
    "implementation-review": "implementation-report.json",
    "implementation-adversarial-review": "implementation-report.json",
    "validation-review": "validation-report.json",
    "validation-adversarial-review": "validation-report.json",
}
SOFTWARE_CHANGE_ADVANCE_STEPS = (
    ("explore", None, "intent-ready", "intent-review"),
    ("intent-review", "intent-review", "approved", "intent-adversarial-review"),
    ("intent-adversarial-review", "intent-adversarial-review", "approved", "design"),
    ("design", None, "design-ready", "design-review"),
    ("design-review", "design-review", "approved", "design-adversarial-review"),
    ("design-adversarial-review", "design-adversarial-review", "approved", "plan"),
    ("plan", None, "plan-ready", "plan-review"),
    ("plan-review", "plan-review", "approved", "plan-adversarial-review"),
    ("plan-adversarial-review", "plan-adversarial-review", "approved", "implement"),
)
PACKET_KEYS = frozenset(
    {"run_id", "slot_id", "artifact_root", "instruction_body", "capture_dir"}
)
OVERLAY_MEANING_SUCCEEDED = (
    "Overlay succeeded means the bound CLI exited 0, not that the provider accepted the work."
)
OVERLAY_MEANING_RUNNING = (
    "Overlay running means the waiter is alive and allowed time has not elapsed."
)
BOUND_PREAMBLE_SEPARATOR = b"---\n\n"
GRAPH_STDIN_SEPARATOR = "\n---\n\n"
SUMMARIZER_ASSIGNMENT_PREFIX = "Write artifact_root/implementation-report.json"
OVERLAY_MEANING_FAILED = (
    "Overlay failed means the bound CLI exited nonzero or the waiter vanished."
)
GRAPH_STEP_STATES = frozenset({"not_started", "running", "reaped"})
PROGRESS_SNAPSHOT_FORBIDDEN = frozenset(
    {
        "overlay",
        "overlay_meaning",
        "elapsed_ms",
        "remaining_allowed_ms",
        "inner_workers",
    }
)
DUMMY_SESSION_MARKER = "dummy-session.json"
DEFAULT_PI_SANDBOX_ARGS = ["--print", "--no-skills", "--no-extensions"]
FORBIDDEN_PI_FLAGS = ("--no-context-files", "--tools")
PI_NO_EXTENSIONS_WITHOUT_E = "has --no-extensions and no -e"
REVIEW_BINDING_SLOT_IDS = SHIPPED_REVIEW_SLOT_IDS + (
    "semantic-review",
    "deterministic-review",
    "verify",
    "synthesize",
)

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
        extra = set(slot) - {"id", "state", "event", "stdin_context_kinds"}
        if extra:
            raise WorkSlotJourneyFailure(f"work_slots catalog leaked extra fields {sorted(extra)}")
        kinds = slot.get("stdin_context_kinds")
        if kinds is not None and (
            not isinstance(kinds, list) or not all(isinstance(kind, str) for kind in kinds)
        ):
            raise WorkSlotJourneyFailure(
                f"slot {slot['id']} stdin_context_kinds must be a list of strings: {kinds}"
            )
        ids.append(slot["id"])
    return ids


def assert_catalog(
    shown: Mapping[str, Any],
    expected_ids: Sequence[str],
    *,
    stdin_context_kinds: Mapping[str, Sequence[str]] | None = None,
) -> None:
    ids = catalog_ids(shown)
    if ids != list(expected_ids):
        raise WorkSlotJourneyFailure(f"unexpected work_slots catalog: {ids}")
    if stdin_context_kinds is None:
        return
    slots = shown.get("work_slots")
    if not isinstance(slots, list):
        raise WorkSlotJourneyFailure("show omitted work_slots catalog")
    for slot in slots:
        slot_id = slot["id"]
        expected = list(stdin_context_kinds.get(slot_id, ()))
        actual = slot.get("stdin_context_kinds")
        if expected:
            if actual != expected:
                raise WorkSlotJourneyFailure(
                    f"slot {slot_id} stdin_context_kinds {actual} != {expected}"
                )
        elif actual:
            raise WorkSlotJourneyFailure(
                f"slot {slot_id} declared stdin_context_kinds {actual}"
            )


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
        f"Legal start: loop-engine invoke {run_id} {slot_id}. "
        f"{OVERLAY_MEANING_SUCCEEDED} "
        "Captures are at the named capture directory on the invocation view and invoke result. "
        "The driver triages worker output, appends provider-shaped records, then requests the shown event. "
        "On overrun run show immediately before re-invoking the same slot. "
        "On failed inspect capture_dir/summary.json and captured stdout before stderr. "
        "Consult the change report of record before reuse. "
        "For review reuse, append one evidence-applicability record referencing the "
        "original evidence, current target, attesting driver, and short reason; "
        "semantic applicability remains the driver's judgment."
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


def assert_inner_worker(worker: Mapping[str, Any]) -> None:
    if not isinstance(worker, dict):
        raise WorkSlotJourneyFailure(f"inner worker is not an object: {worker}")
    if "label" in worker:
        raise WorkSlotJourneyFailure(f"inner worker must not have a label field: {worker}")
    command = worker.get("command")
    args = worker.get("args")
    exit_code = worker.get("exit_code")
    if not isinstance(command, str) or not command:
        raise WorkSlotJourneyFailure(f"inner worker omitted command: {worker}")
    if not isinstance(args, list) or not all(isinstance(item, str) for item in args):
        raise WorkSlotJourneyFailure(f"inner worker args must be a string list: {worker}")
    if not isinstance(exit_code, int):
        raise WorkSlotJourneyFailure(f"inner worker omitted int exit_code: {worker}")


def assert_succeeded_heartbeat(match: Mapping[str, Any], *, slot_id: str) -> None:
    if match.get("slot_id") != slot_id:
        raise WorkSlotJourneyFailure(f"succeeded overlay has wrong slot: {match}")
    if match.get("status") != "succeeded":
        raise WorkSlotJourneyFailure(f"heartbeat status is not succeeded: {match}")
    if match.get("overlay_meaning") != OVERLAY_MEANING_SUCCEEDED:
        raise WorkSlotJourneyFailure(
            f"overlay_meaning mismatch: {match.get('overlay_meaning')!r}"
        )
    elapsed = match.get("elapsed_ms")
    remaining = match.get("remaining_allowed_ms")
    if not isinstance(elapsed, int) or elapsed < 0:
        raise WorkSlotJourneyFailure(f"elapsed_ms invalid: {elapsed!r}")
    if remaining != 0:
        raise WorkSlotJourneyFailure(
            f"remaining_allowed_ms must be 0 after succeeded: {remaining!r}"
        )
    capture_dir = match.get("capture_dir")
    if not isinstance(capture_dir, str) or not capture_dir:
        raise WorkSlotJourneyFailure("succeeded overlay omitted capture_dir")
    if not Path(capture_dir).is_dir():
        raise WorkSlotJourneyFailure(f"capture_dir does not exist: {capture_dir}")
    inner_workers = match.get("inner_workers")
    if not isinstance(inner_workers, list):
        raise WorkSlotJourneyFailure(f"inner_workers missing: {match}")
    allowed = {
        "assignment_id",
        "command",
        "args",
        "exit_code",
        "selected_attempt",
        "selected_output_sha256",
        "selected_output_path",
        "declared_output_contract",
        "routed_inputs",
        "task_definition",
        "task_packet",
        "dependencies",
        "repository_effect",
    }
    assignment_ids = []
    for worker in inner_workers:
        assert_inner_worker(worker)
        extra = set(worker) - allowed
        if extra:
            raise WorkSlotJourneyFailure(
                f"show inner_workers leaked extra fields {sorted(extra)}: {worker}"
            )
        assignment_id = worker.get("assignment_id")
        if not isinstance(assignment_id, str) or not assignment_id:
            raise WorkSlotJourneyFailure(f"show inner worker omitted assignment_id: {worker}")
        assignment_ids.append(assignment_id)
        selected_attempt = worker.get("selected_attempt")
        selected_digest = worker.get("selected_output_sha256")
        selected_path = worker.get("selected_output_path")
        if selected_attempt is None:
            if selected_digest is not None or selected_path is not None:
                raise WorkSlotJourneyFailure(
                    f"coverage gap unexpectedly has selected output facts: {worker}"
                )
        elif (
            not isinstance(selected_attempt, int)
            or selected_attempt < 1
            or not isinstance(selected_digest, str)
            or not selected_digest.startswith("sha256:")
            or not isinstance(selected_path, str)
            or not selected_path
        ):
            raise WorkSlotJourneyFailure(
                f"selected worker omitted attempt/digest/path identity: {worker}"
            )
    if len(assignment_ids) != len(set(assignment_ids)):
        raise WorkSlotJourneyFailure(f"assignment identities are not unique: {inner_workers}")
    if "waiter_pid" in match:
        raise WorkSlotJourneyFailure("show leaked waiter_pid")


def _assert_inner_exit_codes(
    overlay: Mapping[str, Any], expected: Sequence[int]
) -> None:
    inner = overlay.get("inner_workers")
    if not isinstance(inner, list):
        raise WorkSlotJourneyFailure(f"overlay omitted inner_workers: {overlay}")
    actual = [worker.get("exit_code") for worker in inner]
    if actual != list(expected):
        raise WorkSlotJourneyFailure(
            f"inner_workers exit codes {actual} != {list(expected)}"
        )


def invoke_until_succeeded(
    engine_call: EngineCall,
    run_id: str,
    slot_id: str,
    *,
    timeout_s: float = 10.0,
    invoke_args: Sequence[str] = (),
) -> dict[str, Any]:
    started = engine_call(["invoke", run_id, slot_id, *invoke_args])
    if started.get("status") != "completed":
        raise WorkSlotJourneyFailure(f"invoke failed: {started}")
    result = started.get("result") or {}
    invocation_id = result.get("invocation_id")
    if not isinstance(invocation_id, str) or not invocation_id:
        raise WorkSlotJourneyFailure(f"invoke omitted invocation_id: {started}")
    if result.get("slot_id") != slot_id:
        raise WorkSlotJourneyFailure(f"invoke returned wrong slot_id: {started}")
    invoke_capture = result.get("capture_dir")
    if not isinstance(invoke_capture, str) or not invoke_capture:
        raise WorkSlotJourneyFailure(f"invoke omitted capture_dir: {started}")
    expected_capture = (
        Path(invoke_capture).resolve()
        if Path(invoke_capture).is_absolute()
        else Path(invoke_capture)
    )
    if expected_capture.name != invocation_id:
        raise WorkSlotJourneyFailure(
            f"capture_dir must end with invocation_id {invocation_id}: {invoke_capture}"
        )
    if slot_id not in Path(invoke_capture).parts:
        raise WorkSlotJourneyFailure(
            f"capture_dir must include slot_id {slot_id}: {invoke_capture}"
        )

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
                assert_succeeded_heartbeat(match, slot_id=slot_id)
                if match.get("capture_dir") != invoke_capture:
                    raise WorkSlotJourneyFailure(
                        f"show capture_dir {match.get('capture_dir')!r} != invoke {invoke_capture!r}"
                    )
                return match
            if last_status == "failed":
                # A fresh show can briefly observe the waiter between its
                # child exit and its durable completion write. Do not turn
                # that provisional, exit-less overlay into a false journey
                # failure; a real failed waiter remains failed or times out.
                if match.get("completed_at") is None and match.get("exit_code") is None:
                    time.sleep(0.05)
                    continue
                raise WorkSlotJourneyFailure(
                    f"bound worker ended {last_status}: {match}"
                )
            if last_status == "overrun":
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
    if set(packet) != PACKET_KEYS:
        raise WorkSlotJourneyFailure(f"receipt field set mismatch: {sorted(packet)}")
    if packet["run_id"] != run_id or packet["slot_id"] != slot_id:
        raise WorkSlotJourneyFailure(f"receipt identity mismatch: {packet}")
    if packet["artifact_root"] != str(artifact_root):
        raise WorkSlotJourneyFailure(
            f"receipt artifact_root {packet['artifact_root']!r} != {str(artifact_root)!r}"
        )
    capture_dir = packet["capture_dir"]
    if not isinstance(capture_dir, str) or not capture_dir:
        raise WorkSlotJourneyFailure("receipt omitted capture_dir")
    if not Path(capture_dir).is_dir():
        raise WorkSlotJourneyFailure(f"receipt capture_dir does not exist: {capture_dir}")
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
    stdin_context_kinds: Mapping[str, Sequence[str]] | None = None,
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
    assert_catalog(shown, catalog, stdin_context_kinds=stdin_context_kinds)
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
    packet = assert_packet_receipt(
        artifact_root,
        run_id=run_id,
        slot_id=bound_slot_id,
        redacted_instructions=redacted,
    )
    if packet.get("capture_dir") != overlay.get("capture_dir"):
        raise WorkSlotJourneyFailure(
            f"receipt capture_dir {packet.get('capture_dir')!r} != overlay {overlay.get('capture_dir')!r}"
        )
    expected_digest = hashlib.sha256(
        packet["instruction_body"].encode("utf-8")
    ).hexdigest()
    if overlay.get("instruction_digest") != expected_digest:
        raise WorkSlotJourneyFailure(
            f"invoke instruction_digest {overlay.get('instruction_digest')!r} != {expected_digest!r}"
        )
    if not isinstance(overlay.get("subject"), str) or not overlay["subject"]:
        raise WorkSlotJourneyFailure("invoke did not snapshot a current slot-visit subject")
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


def rewrite_path_commands(
    bindings: Mapping[str, Any],
    *,
    engine: str | Path | None = None,
    provider: str | Path | None = None,
) -> dict[str, Any]:
    """Rewrite shipped PATH names to binaries under test. Other commands stay as-is."""
    mapping: dict[str, str] = {}
    if engine is not None:
        mapping[PATH_LOOP_ENGINE] = str(engine)
    if provider is not None:
        mapping[PATH_SOFTWARE_CHANGE] = str(provider)
    rewritten: dict[str, Any] = {}
    for slot_id, binding in bindings.items():
        if not isinstance(binding, dict):
            raise WorkSlotJourneyFailure(
                f"work_slot_bindings[{slot_id}] must be an object, got {binding!r}"
            )
        command = binding.get("command")
        args = binding.get("args")
        if not isinstance(command, str) or not isinstance(args, list):
            raise WorkSlotJourneyFailure(
                f"work_slot_bindings[{slot_id}] must be {{command, args}}"
            )
        rewritten[slot_id] = {
            "command": mapping.get(command, command),
            "args": list(args),
        }
    return rewritten


def assert_no_review_bindings(bindings: Any, *, source: str) -> None:
    if bindings is None:
        return
    if not isinstance(bindings, dict):
        raise WorkSlotJourneyFailure(
            f"{source} work_slot_bindings is not an object: {bindings!r}"
        )
    present = [slot_id for slot_id in REVIEW_BINDING_SLOT_IDS if slot_id in bindings]
    if present:
        raise WorkSlotJourneyFailure(
            f"{source} unexpectedly bound review slots {present}"
        )


def assert_shipped_path_names(bindings: Mapping[str, Any] | None) -> None:
    if bindings is None:
        return
    if not isinstance(bindings, dict):
        raise WorkSlotJourneyFailure(
            f"shipped work_slot_bindings must be omitted or an object, got {bindings!r}"
        )
    present = [slot_id for slot_id in SHIPPED_UNBOUND_SLOT_IDS if slot_id in bindings]
    if present:
        raise WorkSlotJourneyFailure(
            f"shipped bindings unexpectedly bound {present}"
        )
    extra = sorted(str(key) for key in bindings.keys())
    if extra:
        raise WorkSlotJourneyFailure(
            f"shipped work_slot_bindings must be omitted or empty, got keys {extra}"
        )


def assert_rewritten_binaries(
    bindings: Mapping[str, Any],
    *,
    engine: str | Path,
    provider: str | Path,
) -> None:
    implement = bindings.get("implement")
    if not isinstance(implement, dict) or implement.get("command") != str(provider):
        raise WorkSlotJourneyFailure(
            f"rewritten implement command {implement} != {provider}"
        )
    if implement.get("args") != SHIPPED_IMPLEMENT_ARGS:
        raise WorkSlotJourneyFailure(
            f"PATH rewrite must not change shipped implement args: {implement.get('args')}"
        )
    for slot_id in SHIPPED_REVIEW_SLOT_IDS:
        if slot_id in bindings:
            raise WorkSlotJourneyFailure(
                f"rewritten bindings unexpectedly bound {slot_id} (engine={engine})"
            )


def stdin_worker_cli(receipt: Path, extra: Sequence[str] = ()) -> dict[str, Any]:
    return {
        "command": sys.executable,
        "args": [str(STDIN_WORKER_SCRIPT), "--receipt", str(receipt), *extra],
    }


def worker_cli_json(cli: Mapping[str, Any]) -> str:
    return json.dumps(
        {"command": cli["command"], "args": list(cli["args"])},
        separators=(",", ":"),
    )


def fan_out_worker_json(cli: Mapping[str, Any]) -> str:
    return json.dumps(dict(cli), separators=(",", ":"))


def contracted_stdin_worker_cli(
    receipt: Path, output: str, *, preamble: str, required: Sequence[str]
) -> dict[str, Any]:
    cli = stdin_worker_cli(receipt)
    cli["args"] = list(cli["args"]) + ["--stdout", output]
    cli["preamble"] = preamble
    cli["output_schema"] = {"required": list(required)}
    return cli


def compact_artifact_root_stdin(artifact_root: Path | str) -> bytes:
    return (
        json.dumps({"artifact_root": str(artifact_root)}, separators=(",", ":")).encode(
            "utf-8"
        )
        + b"\n"
    )


def assert_bound_preamble_stdin(
    raw: bytes,
    *,
    preamble: str,
    artifact_root: Path,
    run_id: str | None = None,
    slot_id: str | None = None,
) -> None:
    """Assert bound opted-in stdin: preamble + compact location JSON + separator and no instruction_body."""
    del run_id, slot_id
    preamble_bytes = preamble.encode("utf-8")
    if not preamble_bytes.endswith(b"\n"):
        preamble_bytes += b"\n"
    if not raw.startswith(preamble_bytes):
        raise WorkSlotJourneyFailure(
            f"bound contracted stdin did not start with preamble bytes: {raw!r}"
        )
    rest = raw[len(preamble_bytes) :]
    separator_at = rest.find(BOUND_PREAMBLE_SEPARATOR)
    if separator_at < 0:
        raise WorkSlotJourneyFailure(
            f"bound contracted stdin omitted literal separator after preamble: {raw!r}"
        )
    context_bytes = rest[:separator_at]
    if not context_bytes.endswith(b"\n"):
        raise WorkSlotJourneyFailure(
            f"artifact_root context block omitted trailing LF: {context_bytes!r}"
        )
    try:
        parsed_context = json.loads(context_bytes)
    except json.JSONDecodeError as error:
        raise WorkSlotJourneyFailure(
            f"artifact_root context was not JSON: {context_bytes!r}"
        ) from error
    if not isinstance(parsed_context, dict):
        raise WorkSlotJourneyFailure(
            f"artifact_root context was not an object: {parsed_context!r}"
        )
    keys = list(parsed_context.keys())
    if not keys or keys[0] != "artifact_root":
        raise WorkSlotJourneyFailure(
            f"artifact_root context keys {keys} did not start with artifact_root"
        )
    extra = set(keys) - {"artifact_root", "context"}
    if extra:
        raise WorkSlotJourneyFailure(
            f"artifact_root context included unexpected keys {sorted(extra)}"
        )
    if "context" in parsed_context and not isinstance(parsed_context["context"], list):
        raise WorkSlotJourneyFailure(
            f"artifact_root context.context was not an array: {parsed_context['context']!r}"
        )
    if "context" not in parsed_context:
        expected_context = compact_artifact_root_stdin(artifact_root)
        if context_bytes != expected_context:
            raise WorkSlotJourneyFailure(
                "artifact_root context bytes were not compact one-key JSON: "
                f"{context_bytes!r} != {expected_context!r}"
            )
    if parsed_context["artifact_root"] != str(artifact_root):
        raise WorkSlotJourneyFailure(
            f"artifact_root context value {parsed_context['artifact_root']!r} "
            f"!= {str(artifact_root)!r}"
        )
    if "capture_dir" in parsed_context or "run_id" in parsed_context or "slot_id" in parsed_context:
        raise WorkSlotJourneyFailure(
            f"artifact_root context included capture_dir or duplicate identity: {parsed_context}"
        )
    body = rest[separator_at + len(BOUND_PREAMBLE_SEPARATOR) :]
    if body:
        raise WorkSlotJourneyFailure(
            f"bound contracted stdin dumped extra bytes after separator: {body!r}"
        )
    if b"instruction_body" in raw:
        raise WorkSlotJourneyFailure(
            f"bound contracted stdin dumped instruction_body: {raw!r}"
        )


def append_task_worker(
    binding: Mapping[str, Any],
    task_worker: Mapping[str, Any],
    *,
    working_directory: Path,
    max_active: int | None = None,
) -> dict[str, Any]:
    if not working_directory.is_absolute() or not working_directory.is_dir():
        raise WorkSlotJourneyFailure(
            "run-plan-graph working directory must be an existing absolute directory: "
            f"{working_directory}"
        )
    args = list(binding["args"])
    args.extend(["--working-directory", str(working_directory)])
    if max_active is not None:
        args.extend(["--max-active", str(max_active)])
    args.extend(["--task-worker", worker_cli_json(task_worker)])
    return {"command": binding["command"], "args": args}


def append_fan_out_workers(
    binding: Mapping[str, Any],
    workers: Sequence[Mapping[str, Any]],
    *,
    instructions: Path | None = None,
    max_active: int | None = None,
) -> dict[str, Any]:
    args = list(binding["args"])
    if max_active is not None:
        args.extend(["--max-active", str(max_active)])
    if instructions is not None:
        args.extend(["--instructions", str(instructions)])
    for worker in workers:
        args.extend(["--worker", fan_out_worker_json(worker)])
    return {"command": binding["command"], "args": args}


def implement_graph_runner_binding(
    *,
    provider: Path,
    task_worker: Mapping[str, Any],
    working_directory: Path,
    max_active: int | None = None,
) -> dict[str, Any]:
    shipped = {
        "implement": {"command": PATH_SOFTWARE_CHANGE, "args": list(SHIPPED_IMPLEMENT_ARGS)}
    }
    rewritten = rewrite_path_commands(shipped, provider=provider)
    return append_task_worker(
        rewritten["implement"],
        task_worker,
        working_directory=working_directory,
        max_active=max_active,
    )


def fan_out_binding(
    *,
    engine: Path,
    workers: Sequence[Mapping[str, Any]] = (),
    instructions: Path | None = None,
    max_active: int | None = None,
) -> dict[str, Any]:
    shipped = {
        "design-review": {"command": PATH_LOOP_ENGINE, "args": list(SHIPPED_FAN_OUT_ARGS)}
    }
    rewritten = rewrite_path_commands(shipped, engine=engine)
    return append_fan_out_workers(
        rewritten["design-review"],
        workers,
        instructions=instructions,
        max_active=max_active,
    )


def small_plan_document() -> dict[str, Any]:
    """Two independent tasks plus one dependent task. Not the calibration fixture."""
    return {
        "revision": "1",
        "tasks": [
            {
                "id": "alpha",
                "objective": "Independent A",
                "dependencies": [],
            },
            {
                "id": "beta",
                "objective": "Independent B",
                "dependencies": [],
            },
            {
                "id": "gamma",
                "objective": "Depends on A and B",
                "dependencies": ["alpha", "beta"],
            },
        ],
        "dependency_graph": [
            {"from": "alpha", "to": "gamma"},
            {"from": "beta", "to": "gamma"},
        ],
    }


def independent_plan_document(count: int = 3) -> dict[str, Any]:
    """Independent plan tasks so a set --max-active can be observed."""
    names = ("alpha", "beta", "gamma", "delta", "epsilon")
    if count < 1:
        raise WorkSlotJourneyFailure(f"independent plan needs at least one task, got {count}")
    task_ids = list(names[:count]) if count <= len(names) else [f"t{index}" for index in range(count)]
    return {
        "revision": "1",
        "tasks": [
            {
                "id": task_id,
                "objective": f"Independent {task_id}",
                "dependencies": [],
            }
            for task_id in task_ids
        ],
        "dependency_graph": [],
    }


def parse_graph_runner_stdin(text: str) -> dict[str, Any]:
    if GRAPH_STDIN_SEPARATOR not in text:
        raise WorkSlotJourneyFailure(
            f"inner stdin omitted compact location/duty separator: {text!r}"
        )
    raw_location, rest = text.split(GRAPH_STDIN_SEPARATOR, 1)
    try:
        location = json.loads(raw_location)
    except json.JSONDecodeError as error:
        raise WorkSlotJourneyFailure(
            f"inner stdin location is not JSON: {raw_location!r}"
        ) from error
    if not isinstance(location, dict):
        raise WorkSlotJourneyFailure(
            f"inner stdin location is not an object: {location!r}"
        )
    if rest.startswith(SUMMARIZER_ASSIGNMENT_PREFIX):
        raise WorkSlotJourneyFailure(
            f"task receipt contained summarizer assignment: {text!r}"
        )
    if set(location.keys()) != {"artifact_root"}:
        raise WorkSlotJourneyFailure(
            f"task stdin location keys {sorted(location)} != ['artifact_root']"
        )
    if "instruction_body" in text.split(GRAPH_STDIN_SEPARATOR, 1)[0]:
        raise WorkSlotJourneyFailure(
            f"inner stdin dumped instruction_body: {text!r}"
        )
    try:
        task = json.loads(rest)
    except json.JSONDecodeError as error:
        raise WorkSlotJourneyFailure(f"inner stdin task is not JSON: {error}") from error
    if not isinstance(task, dict):
        raise WorkSlotJourneyFailure(f"inner stdin task is not an object: {task!r}")
    artifact_root = location["artifact_root"]
    if not isinstance(artifact_root, str) or not Path(artifact_root).is_absolute():
        raise WorkSlotJourneyFailure(
            f"inner stdin artifact_root is not absolute: {artifact_root!r}"
        )
    return {
        "artifact_root": artifact_root,
        "task": task,
    }


def pid_is_alive(pid: int) -> bool:
    try:
        os.kill(pid, 0)
    except OSError:
        return False
    return True


def _read_pid(path: Path) -> int:
    try:
        return int(path.read_text(encoding="utf-8").strip())
    except (OSError, ValueError) as error:
        raise WorkSlotJourneyFailure(f"could not read pid file {path}: {error}") from error


def _run_binding(
    binding: Mapping[str, Any],
    *,
    stdin: bytes,
    cwd: Path | None = None,
    env: Mapping[str, str] | None = None,
) -> subprocess.CompletedProcess[bytes]:
    command = binding.get("command")
    args = binding.get("args")
    if not isinstance(command, str) or not isinstance(args, list):
        raise WorkSlotJourneyFailure(f"binding is not {{command, args}}: {binding}")
    return subprocess.run(
        [command, *args],
        input=stdin,
        capture_output=True,
        check=False,
        cwd=cwd,
        env=None if env is None else dict(env),
    )


def _write_pi_stub(directory: Path) -> Path:
    """Write a PATH stub named pi that records argv and exits 0.

    When stdin is the summarizer assignment, write a schema-shaped
    implementation-report.json. Ordinary task stdin never writes that file.
    """
    directory.mkdir(parents=True, exist_ok=True)
    stub = directory / "pi"
    stub.write_text(
        "#!/usr/bin/env python3\n"
        "import json\n"
        "import os\n"
        "import sys\n"
        "from pathlib import Path\n"
        "\n"
        "log_dir = os.environ.get('PI_STUB_LOG_DIR')\n"
        "if not log_dir:\n"
        "    log_dir = str(Path(__file__).resolve().parent.parent / 'pi-argv')\n"
        "os.makedirs(log_dir, exist_ok=True)\n"
        "path = os.path.join(log_dir, f'{os.getpid()}.argv.json')\n"
        "with open(path, 'w', encoding='utf-8') as handle:\n"
        "    json.dump(sys.argv[1:], handle)\n"
        "raw = sys.stdin.read()\n"
        "sep = '\\n---\\n\\n'\n"
        "if sep not in raw:\n"
        "    sys.exit(0)\n"
        "location_raw, rest = raw.split(sep, 1)\n"
        "if not rest.startswith('Write artifact_root/implementation-report.json'):\n"
        "    sys.exit(0)\n"
        "try:\n"
        "    location = json.loads(location_raw)\n"
        "except json.JSONDecodeError:\n"
        "    sys.exit(0)\n"
        "artifact_root = location.get('artifact_root') if isinstance(location, dict) else None\n"
        "plan_path = location.get('plan_path') if isinstance(location, dict) else None\n"
        "revision = ''\n"
        "if isinstance(plan_path, str) and plan_path:\n"
        "    try:\n"
        "        plan = json.loads(Path(plan_path).read_text(encoding='utf-8'))\n"
        "    except (OSError, json.JSONDecodeError):\n"
        "        plan = {}\n"
        "    found = plan.get('revision') if isinstance(plan, dict) else None\n"
        "    if isinstance(found, str) and found:\n"
        "        revision = found\n"
        "if isinstance(artifact_root, str) and artifact_root:\n"
        "    report = {\n"
        "        'revision': '1',\n"
        "        'author': {'name': 'pi-stub', 'kind': 'script'},\n"
        "        'plan_revision': revision,\n"
        "        'coverage': {\n"
        "            'commit': 'dummy',\n"
        "            'documents': [{'path': 'plan.json', 'revision': revision or 'none'}],\n"
        "        },\n"
        "        'summary': 'pi stub summarizer wrote this report',\n"
        "        'changed_surface': ['dummy'],\n"
        "        'validation': ['dummy'],\n"
        "    }\n"
        "    Path(artifact_root, 'implementation-report.json').write_text(\n"
        "        json.dumps(report) + '\\n', encoding='utf-8'\n"
        "    )\n",
        encoding="utf-8",
    )
    stub.chmod(0o755)
    return stub


def _invoke_packet(
    *,
    run_id: str,
    slot_id: str,
    artifact_root: Path,
    instruction_body: str,
    capture_dir: Path,
    context: Sequence[Mapping[str, Any]] | None = None,
) -> bytes:
    packet: dict[str, Any] = {
        "run_id": run_id,
        "slot_id": slot_id,
        "artifact_root": str(artifact_root),
        "instruction_body": instruction_body,
        "capture_dir": str(capture_dir),
    }
    if context is not None:
        packet["context"] = list(context)
    return json.dumps(packet, separators=(",", ":")).encode("utf-8")


def _load_summary_workers(capture_dir: Path) -> list[dict[str, Any]]:
    path = capture_dir / "summary.json"
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise WorkSlotJourneyFailure(f"missing or invalid {path}: {error}") from error
    if not isinstance(payload, dict):
        raise WorkSlotJourneyFailure(f"summary.json is not an object: {path}")
    workers = payload.get("workers")
    if not isinstance(workers, list):
        raise WorkSlotJourneyFailure(f"summary.json omitted workers array: {path}")
    for worker in workers:
        assert_inner_worker(worker)
    return workers


def _assert_capture_files(capture_dir: Path, worker_ids: Sequence[str]) -> None:
    if not capture_dir.is_dir():
        raise WorkSlotJourneyFailure(f"capture_dir does not exist: {capture_dir}")
    _load_summary_workers(capture_dir)
    for worker_id in worker_ids:
        stdout = capture_dir / worker_id / "stdout"
        stderr = capture_dir / worker_id / "stderr"
        if not stdout.is_file():
            raise WorkSlotJourneyFailure(f"missing capture stdout {stdout}")
        if not stderr.is_file():
            raise WorkSlotJourneyFailure(f"missing capture stderr {stderr}")


def _emitted_graph_yaml(capture_dir: Path) -> str:
    dags = capture_dir / "dagu-home" / "dags"
    if not dags.is_dir():
        raise WorkSlotJourneyFailure(f"missing emitted dags directory {dags}")
    yaml_files = sorted(
        path for path in dags.iterdir() if path.is_file() and path.suffix == ".yaml"
    )
    if not yaml_files:
        raise WorkSlotJourneyFailure(f"no emitted DAG yaml in {dags}")
    try:
        return yaml_files[0].read_text(encoding="utf-8")
    except OSError as error:
        raise WorkSlotJourneyFailure(f"could not read emitted yaml {yaml_files[0]}: {error}") from error


def _assert_yaml_omits_max_active_steps(capture_dir: Path) -> None:
    yaml = _emitted_graph_yaml(capture_dir)
    if "max_active_steps" in yaml:
        raise WorkSlotJourneyFailure(
            f"omitted fan-out yaml unexpectedly contains max_active_steps: {yaml}"
        )


def _assert_yaml_max_active_steps(capture_dir: Path, expected: int) -> None:
    yaml = _emitted_graph_yaml(capture_dir)
    needle = f"max_active_steps: {expected}"
    if needle not in yaml:
        raise WorkSlotJourneyFailure(f"emitted yaml omitted {needle}: {yaml}")


def _assert_yaml_working_directory(capture_dir: Path, selected_directory: Path) -> None:
    yaml = _emitted_graph_yaml(capture_dir)
    needle = f"working_dir: {json.dumps(str(selected_directory))}"
    if yaml.count("working_dir:") != 1 or needle not in yaml:
        raise WorkSlotJourneyFailure(
            f"emitted graph did not contain one graph-level {needle}: {yaml}"
        )
    if "    working_dir:" in yaml:
        raise WorkSlotJourneyFailure(f"emitted graph repeated working_dir per step: {yaml}")


def run_preview_bindings(
    engine: Path,
    payload: Mapping[str, Any] | str,
    *,
    database: Path | None = None,
    extra: Sequence[str] = (),
    cwd: Path | None = None,
) -> tuple[int, dict[str, Any]]:
    """Run preview-bindings. Does not require a database."""
    operand = payload if isinstance(payload, str) else json.dumps(payload)
    command = [str(engine), "--json", *extra]
    if database is not None:
        command.extend(["--database", str(database)])
    command.extend(["preview-bindings", operand])
    completed = subprocess.run(
        command, text=True, capture_output=True, check=False, cwd=cwd
    )
    if not completed.stdout.strip():
        raise WorkSlotJourneyFailure(
            "preview-bindings returned no JSON "
            f"(exit={completed.returncode}): {completed.stderr}"
        )
    try:
        value = json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        raise WorkSlotJourneyFailure(
            f"preview-bindings returned non-JSON (exit={completed.returncode}): {error}"
        ) from error
    if not isinstance(value, dict):
        raise WorkSlotJourneyFailure(f"preview-bindings response is not an object: {value}")
    return completed.returncode, value


def _json_stdout(completed: subprocess.CompletedProcess[bytes]) -> dict[str, Any]:
    text = completed.stdout.decode("utf-8", "replace").strip()
    if not text:
        raise WorkSlotJourneyFailure(
            "helper returned empty stdout "
            f"(exit={completed.returncode}): "
            f"{completed.stderr.decode('utf-8', 'replace')}"
        )
    try:
        value = json.loads(text)
    except json.JSONDecodeError as error:
        raise WorkSlotJourneyFailure(
            f"helper stdout is not JSON: {error}: {text[:500]}"
        ) from error
    if not isinstance(value, dict):
        raise WorkSlotJourneyFailure(f"helper stdout is not an object: {value}")
    return value


def prove_shipped_software_change_profiles(data_root: Path) -> list[str]:
    """Copied shipped profiles omit work_slot_bindings so implement and reviews are unbound."""
    names: list[str] = []
    for name in SHIPPED_PROFILE_NAMES:
        path = data_root / SHIPPED_PROFILE_RELATIVE / name
        try:
            profile = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as error:
            raise WorkSlotJourneyFailure(
                f"could not read shipped profile {path}: {error}"
            ) from error
        if not isinstance(profile, dict):
            raise WorkSlotJourneyFailure(f"shipped profile is not an object: {path}")
        bindings = profile.get("work_slot_bindings")
        if bindings is not None and not isinstance(bindings, dict):
            raise WorkSlotJourneyFailure(
                f"{path} work_slot_bindings must be omitted or an object: {bindings!r}"
            )
        assert_shipped_path_names(bindings)
        names.append(name)
    return names


def _copy_software_change_fixtures(fixture_root: Path, artifact_root: Path) -> None:
    artifact_root.mkdir(parents=True, exist_ok=True)
    for subject, fixture in SOFTWARE_CHANGE_FIXTURES:
        source = fixture_root / fixture
        if not source.is_file():
            raise WorkSlotJourneyFailure(f"missing software-change fixture {source}")
        shutil.copy2(source, artifact_root / subject)


def _append_synthetic_gate_evidence(
    engine_call: EngineCall,
    *,
    run_id: str,
    profile: Mapping[str, Any],
    artifact_root: Path,
    gate: str,
    record_prefix: str = "",
) -> None:
    subject = SOFTWARE_CHANGE_GATE_SUBJECT[gate]
    path = artifact_root / subject
    try:
        document = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise WorkSlotJourneyFailure(f"could not read {path}: {error}") from error
    revision = document.get("revision") if isinstance(document, dict) else None
    if not isinstance(revision, str) or not revision:
        raise WorkSlotJourneyFailure(f"{subject} omitted revision")
    policies = profile.get("review_policies")
    if not isinstance(policies, dict):
        raise WorkSlotJourneyFailure("profile omitted review_policies")
    axes = policies.get(gate)
    if not isinstance(axes, list):
        raise WorkSlotJourneyFailure(f"profile omitted {gate} review_policies")
    config_version = profile.get("config_version")
    if not isinstance(config_version, str) or not config_version:
        raise WorkSlotJourneyFailure("profile omitted config_version")
    for entry in axes:
        if not isinstance(entry, dict) or not isinstance(entry.get("id"), str):
            raise WorkSlotJourneyFailure(f"malformed {gate} axis: {entry}")
        axis = entry["id"]
        required_authors = int(entry.get("required_authors", 1))
        for index, suffix in enumerate(("a", "b")):
            if index >= max(2, required_authors):
                break
            record_id = f"{record_prefix}{gate}-{axis}-{suffix}"
            data = {
                "gate": gate,
                "policy_id": axis,
                "result": "pass",
                "findings": "",
                "author": {
                    "name": f"synthetic-{gate}-{axis}-{suffix}",
                    "kind": "script",
                },
                "subject": subject,
                "subject_revision": revision,
                "config_version": config_version,
            }
            appended = engine_call(
                [
                    "append",
                    f"--record-id={record_id}",
                    run_id,
                    "review-evidence",
                    json.dumps(data, separators=(",", ":")),
                ]
            )
            if appended.get("status") != "completed":
                raise WorkSlotJourneyFailure(
                    f"{gate}/{axis} evidence append failed: {appended}"
                )


def _append_synthetic_finding_ledger(
    engine_call: EngineCall,
    *,
    run_id: str,
    profile: Mapping[str, Any],
    artifact_root: Path,
    gate: str,
    record_prefix: str = "",
) -> None:
    subject = SOFTWARE_CHANGE_GATE_SUBJECT[gate]
    path = artifact_root / subject
    try:
        document = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise WorkSlotJourneyFailure(f"could not read {path}: {error}") from error
    revision = document.get("revision") if isinstance(document, dict) else None
    if not isinstance(revision, str) or not revision:
        raise WorkSlotJourneyFailure(f"{subject} omitted revision")
    record_id = f"{record_prefix}finding-ledger-{gate}"
    data = {
        "schema_version": "1",
        "gate": gate,
        "subject": subject,
        "subject_revision": revision,
        "author": {"name": "journey-driver", "kind": "agent"},
        "findings": [],
    }
    appended = engine_call(
        [
            "append",
            f"--record-id={record_id}",
            run_id,
            "finding-ledger",
            json.dumps(data, separators=(",", ":")),
        ]
    )
    if appended.get("status") != "completed":
        raise WorkSlotJourneyFailure(
            f"{gate} finding-ledger append failed: {appended}"
        )


def _expect_event_state(
    engine_call: EngineCall,
    run_id: str,
    event: str,
    target: str,
) -> None:
    response = engine_call(["event", run_id, event])
    if response.get("status") != "completed":
        raise WorkSlotJourneyFailure(f"{event} failed: {response}")
    current = ((response.get("result") or {}).get("run") or {}).get("current_state")
    if current != target:
        raise WorkSlotJourneyFailure(
            f"{event} expected {target}, got {current}: {response}"
        )


def _advance_software_change_to(
    engine_call: EngineCall,
    *,
    run_id: str,
    profile: Mapping[str, Any],
    artifact_root: Path,
    target: str,
) -> None:
    shown = engine_call(["show", run_id])
    if shown.get("status") != "completed":
        raise WorkSlotJourneyFailure(f"show before advance failed: {shown}")
    current = (shown.get("result") or {}).get("current_state")
    if current != "explore":
        raise WorkSlotJourneyFailure(
            f"isolated run did not start in explore: {current}"
        )
    if target == "explore":
        return
    invoke_until_succeeded(engine_call, run_id, "intent-draft", timeout_s=20.0)
    for current_state, gate, event, nxt in SOFTWARE_CHANGE_ADVANCE_STEPS:
        if current_state == target:
            return
        if gate is not None:
            _append_synthetic_gate_evidence(
                engine_call,
                run_id=run_id,
                profile=profile,
                artifact_root=artifact_root,
                gate=gate,
                record_prefix=f"{run_id}-",
            )
            _append_synthetic_finding_ledger(
                engine_call,
                run_id=run_id,
                profile=profile,
                artifact_root=artifact_root,
                gate=gate,
                record_prefix=f"{run_id}-",
            )
        _expect_event_state(engine_call, run_id, event, nxt)
        observed = engine_call(["show", run_id])
        if observed.get("status") != "completed":
            raise WorkSlotJourneyFailure(
                f"show after advancing to {nxt} failed: {observed}"
            )
        if nxt == target:
            shown = engine_call(["show", run_id])
            if shown.get("status") != "completed":
                raise WorkSlotJourneyFailure(
                    f"show after advancing to {target} failed: {shown}"
                )
            landed = (shown.get("result") or {}).get("current_state")
            if landed != target:
                raise WorkSlotJourneyFailure(
                    f"advance show expected {target}, got {landed}"
                )
            return
    raise WorkSlotJourneyFailure(f"advance did not reach {target}")


def _start_isolated_software_change(
    *,
    engine: Path,
    provider: Path,
    profile_source: Path,
    fixture_root: Path,
    work_dir: Path,
    run_id: str,
    extra_bindings: Mapping[str, Any],
    observe_start: bool = True,
) -> tuple[EngineCall, Path, dict[str, Any]]:
    work_dir.mkdir(parents=True, exist_ok=True)
    artifact_root = work_dir / "artifacts"
    _copy_software_change_fixtures(fixture_root, artifact_root)
    try:
        profile = json.loads(profile_source.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise WorkSlotJourneyFailure(
            f"could not read isolated profile {profile_source}: {error}"
        ) from error
    if not isinstance(profile, dict):
        raise WorkSlotJourneyFailure("isolated profile is not an object")
    profile = dict(profile)
    profile["artifact_root"] = str(artifact_root)
    profile["work_slot_bindings"] = {
        **bindings_for(["intent-draft"]),
        **dict(extra_bindings),
    }
    profile_path = work_dir / "profile.json"
    profile_path.write_text(json.dumps(profile, indent=2) + "\n", encoding="utf-8")
    database = work_dir / "loop.sqlite"
    providers = work_dir / "providers.toml"
    providers.write_text(
        "[providers.software-change]\n"
        f"command = {json.dumps(str(provider))}\n"
        "args = []\n",
        encoding="utf-8",
    )

    def engine_call(operation: Sequence[str]) -> dict[str, Any]:
        response = _engine_json(engine, database, operation)
        if (
            operation
            and operation[0] in {"event", "invoke", "terminate"}
            and len(operation) > 1
            and response.get("status") in {"completed", "rejected"}
        ):
            observed = _engine_json(engine, database, ["show", run_id])
            if observed.get("status") != "completed":
                raise WorkSlotJourneyFailure(
                    f"show after {operation[0]} failed: {observed}"
                )
        return response

    started = _engine_json(
        engine,
        database,
        [
            "--config",
            str(providers),
            "--timeout-ms",
            "30000",
            "start",
            "--id",
            run_id,
            "software-change",
            "@" + str(profile_path),
            "isolated work-slot proof",
        ],
    )
    if started.get("status") != "completed":
        raise WorkSlotJourneyFailure(f"isolated start failed: {started}")
    if observe_start:
        observed = engine_call(["show", run_id])
        if observed.get("status") != "completed":
            raise WorkSlotJourneyFailure(f"isolated start observation failed: {observed}")
    return engine_call, artifact_root, profile


def _assert_capture_isolation(
    first: Mapping[str, Any],
    second: Mapping[str, Any],
    worker_ids: Sequence[str],
) -> None:
    first_dir = first.get("capture_dir")
    second_dir = second.get("capture_dir")
    if not isinstance(first_dir, str) or not isinstance(second_dir, str):
        raise WorkSlotJourneyFailure("invoke overlay omitted capture_dir")
    first_path = Path(first_dir)
    second_path = Path(second_dir)
    if first_path == second_path:
        raise WorkSlotJourneyFailure(
            f"second invoke reused capture_dir {first_dir}"
        )
    if first.get("invocation_id") == second.get("invocation_id"):
        raise WorkSlotJourneyFailure(
            f"second invoke reused invocation_id {first.get('invocation_id')}"
        )
    first_summary = first_path / "summary.json"
    if not first_summary.is_file():
        raise WorkSlotJourneyFailure(
            f"first capture_dir was removed after retry: {first_path}"
        )
    _assert_capture_files(first_path, worker_ids)
    _assert_capture_files(second_path, worker_ids)


def _write_small_plan(artifact_root: Path) -> dict[str, Any]:
    artifact_root.mkdir(parents=True, exist_ok=True)
    # Successful plan graphs now emit a provider checkpoint. Keep standalone
    # fixtures small while supplying the three revision-bearing documents that
    # checkpoint generation reads; full software-change fixtures are preserved.
    for name in ("intent.json", "design.json"):
        path = artifact_root / name
        if not path.exists():
            path.write_text(
                json.dumps({"revision": "1"}, separators=(",", ":")),
                encoding="utf-8",
            )
    plan = small_plan_document()
    (artifact_root / "plan.json").write_text(
        json.dumps(plan, indent=2) + "\n", encoding="utf-8"
    )
    return plan


def _write_plan_document(artifact_root: Path, plan: Mapping[str, Any]) -> dict[str, Any]:
    artifact_root.mkdir(parents=True, exist_ok=True)
    payload = dict(plan)
    (artifact_root / "plan.json").write_text(
        json.dumps(payload, indent=2) + "\n", encoding="utf-8"
    )
    return payload


def _assert_dummy_report(artifact_root: Path, plan_revision: str) -> None:
    path = artifact_root / "implementation-report.json"
    try:
        report = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise WorkSlotJourneyFailure(
            f"dummy summarizer did not write valid {path}: {error}"
        ) from error
    if not isinstance(report, dict):
        raise WorkSlotJourneyFailure(f"implementation-report.json is not an object: {path}")
    required = (
        "revision",
        "author",
        "plan_revision",
        "coverage",
        "summary",
        "changed_surface",
        "validation",
    )
    missing = [key for key in required if key not in report]
    if missing:
        raise WorkSlotJourneyFailure(
            f"implementation-report.json omitted {missing}: {report}"
        )
    if report.get("plan_revision") != plan_revision:
        raise WorkSlotJourneyFailure(
            f"implementation-report.json plan_revision {report.get('plan_revision')!r} "
            f"!= {plan_revision!r}"
        )


def _assert_cwd_receipt(
    receipt_dir: Path,
    worker_id: str,
    *,
    selected_directory: Path,
) -> None:
    path = receipt_dir / f"{worker_id}.stdin.cwd"
    try:
        reported = Path(path.read_text(encoding="utf-8").strip())
    except OSError as error:
        raise WorkSlotJourneyFailure(f"missing cwd receipt {path}: {error}") from error
    if not reported.is_absolute() or not reported.is_dir():
        raise WorkSlotJourneyFailure(
            f"worker {worker_id} reported a non-directory cwd: {reported}"
        )
    try:
        equivalent = reported.samefile(selected_directory)
    except OSError as error:
        raise WorkSlotJourneyFailure(
            f"could not compare worker {worker_id} cwd {reported} with "
            f"selected directory {selected_directory}: {error}"
        ) from error
    if not equivalent:
        raise WorkSlotJourneyFailure(
            f"worker {worker_id} cwd {reported} is not filesystem-equivalent to "
            f"selected directory {selected_directory}"
        )
    marker = receipt_dir / f"{worker_id}.stdin.cwd-entry"
    if not marker.is_file():
        raise WorkSlotJourneyFailure(
            f"worker {worker_id} omitted required cwd-entry receipt {marker}"
        )


def _assert_task_receipt(
    receipt_dir: Path,
    task: Mapping[str, Any],
    *,
    artifact_root: Path,
) -> None:
    task_id = task["id"]
    path = receipt_dir / f"{task_id}.stdin"
    try:
        recorded = path.read_text(encoding="utf-8")
    except OSError as error:
        raise WorkSlotJourneyFailure(f"missing graph-runner receipt {path}: {error}") from error
    parsed = parse_graph_runner_stdin(recorded)
    if parsed["artifact_root"] != str(artifact_root):
        raise WorkSlotJourneyFailure(
            f"receipt artifact_root {parsed['artifact_root']!r} != {str(artifact_root)!r}"
        )
    if parsed["task"] != task:
        raise WorkSlotJourneyFailure(
            f"receipt task record mismatch for {task_id}: {parsed['task']}"
        )
    if "instruction_body" in recorded.split(GRAPH_STDIN_SEPARATOR, 1)[0]:
        raise WorkSlotJourneyFailure(
            f"task {task_id} stdin dumped instruction_body"
        )


def _ensure_git_repository(path: Path) -> None:
    """Give standalone plan-graph fixtures the real repository boundary they prove."""
    path.mkdir(parents=True, exist_ok=True)
    if (path / ".git").exists():
        return
    marker = path / ".journey-baseline"
    marker.write_text("baseline\n", encoding="utf-8")
    for git_args in (
        ["init", "-q"],
        ["config", "user.name", "software-change journey"],
        ["config", "user.email", "journey@example.invalid"],
        ["config", "commit.gpgsign", "false"],
        ["add", "-A"],
        ["commit", "-qm", "journey baseline"],
    ):
        completed = subprocess.run(
            ["git", *git_args],
            cwd=path,
            text=True,
            capture_output=True,
            check=False,
        )
        if completed.returncode != 0:
            raise WorkSlotJourneyFailure(
                f"could not initialize plan-graph fixture repository: "
                f"{completed.stderr.strip() or completed.stdout.strip()}"
            )


def prove_graph_runner(*, provider: Path, work_dir: Path) -> list[str]:
    """Prove run-plan-graph with dummy --task-worker. Never calls a live model."""
    work_dir.mkdir(parents=True, exist_ok=True)
    _ensure_git_repository(work_dir)
    plan = small_plan_document()
    task_ids = [task["id"] for task in plan["tasks"]]
    tasks_by_id = {task["id"]: task for task in plan["tasks"]}
    run_id = "graph-runner-proof"
    slot_id = "implement"
    instruction_body = "Implement the small fixture plan."
    plan_revision = str(plan["revision"])

    success_root = work_dir / "success"
    success_receipts = success_root / "receipts"
    success_capture = success_root / "captures" / "inv-success"
    _write_small_plan(success_root)
    leftover = success_root / "implementation-report.json"
    leftover.write_text("{}\n", encoding="utf-8")
    success_binding = implement_graph_runner_binding(
        provider=provider,
        task_worker=stdin_worker_cli(success_receipts),
        working_directory=work_dir,
    )
    if success_binding["command"] != str(provider):
        raise WorkSlotJourneyFailure(
            f"graph-runner binding command was not rewritten: {success_binding}"
        )
    if success_binding["args"][:1] != SHIPPED_IMPLEMENT_ARGS:
        raise WorkSlotJourneyFailure(
            f"graph-runner binding args must start with run-plan-graph: {success_binding}"
        )
    success = _run_binding(
        success_binding,
        stdin=_invoke_packet(
            run_id=run_id,
            slot_id=slot_id,
            artifact_root=success_root,
            instruction_body=instruction_body,
            capture_dir=success_capture,
        ),
    )
    if success.returncode != 0:
        raise WorkSlotJourneyFailure(
            "graph-runner success path exited "
            f"{success.returncode}: {success.stderr.decode('utf-8', 'replace')}"
        )
    _assert_capture_files(success_capture, task_ids)
    success_workers = _load_summary_workers(success_capture)
    if [worker.get("exit_code") for worker in success_workers] != [0, 0, 0]:
        raise WorkSlotJourneyFailure(
            f"success summary exit codes mismatch: {success_workers}"
        )
    for task in plan["tasks"]:
        _assert_task_receipt(
            success_receipts,
            task,
            artifact_root=success_root,
        )
    summarizer_receipt = success_receipts / "summarizer.stdin"
    if not summarizer_receipt.is_file():
        raise WorkSlotJourneyFailure("success path omitted summarizer stdin receipt")
    summarizer_text = summarizer_receipt.read_text(encoding="utf-8")
    if SUMMARIZER_ASSIGNMENT_PREFIX not in summarizer_text:
        raise WorkSlotJourneyFailure(
            f"summarizer receipt omitted assignment: {summarizer_text!r}"
        )
    if leftover.read_text(encoding="utf-8") == "{}\n":
        raise WorkSlotJourneyFailure("leftover empty report still satisfied overlay success")
    _assert_dummy_report(success_root, plan_revision)

    _assert_capture_files(success_capture, ("summarizer",))
    _assert_yaml_max_active_steps(success_capture, 4)
    success_workers = _load_summary_workers(success_capture)
    if any(worker.get("command") == "summarizer" for worker in success_workers):
        raise WorkSlotJourneyFailure(
            f"success summary included summarizer: {success_workers}"
        )

    # Ledger routing is an operator-visible provider path: pass immutable
    # context through separate CLI processes and assert only the exact task
    # receives the current implementation-owned entry. A proposal-only run
    # proves advisory classification is inert before the driver appends a
    # disposition.
    proposal_root = work_dir / "finding-proposal-only"
    proposal_receipts = proposal_root / "receipts"
    proposal_capture = proposal_root / "captures" / "inv-proposal-only"
    _write_small_plan(proposal_root)
    proposal_context = [
        {
            "id": "proposal-only-1",
            "kind": "advisory-finding-proposal",
            "data": {"proposals": [{"candidate_source_ids": ["source-1"]}]},
            "sequence": 1,
            "created_at": 1,
        }
    ]
    proposal_run = _run_binding(
        implement_graph_runner_binding(
            provider=provider,
            task_worker=stdin_worker_cli(proposal_receipts),
            working_directory=work_dir,
        ),
        stdin=_invoke_packet(
            run_id=run_id,
            slot_id=slot_id,
            artifact_root=proposal_root,
            instruction_body=instruction_body,
            capture_dir=proposal_capture,
            context=proposal_context,
        ),
    )
    if proposal_run.returncode != 0:
        raise WorkSlotJourneyFailure(
            f"proposal-only graph exited {proposal_run.returncode}: "
            f"{proposal_run.stderr.decode('utf-8', 'replace')}"
        )
    for task in plan["tasks"]:
        recorded = parse_graph_runner_stdin(
            (proposal_receipts / f"{task['id']}.stdin").read_text(encoding="utf-8")
        )
        expected_proposal_task = dict(task)
        expected_proposal_task["finding_context"] = []
        if recorded["task"] != expected_proposal_task:
            raise WorkSlotJourneyFailure(
                f"advisory proposal changed task {task['id']} before driver disposition: "
                f"{recorded['task']}"
            )

    # bookends:LE-92 — only the driver-confirmed ledger entry routes to alpha; the advisory proposal remains absent from every task packet.
    routing_root = work_dir / "finding-routing"
    routing_receipts = routing_root / "receipts"
    routing_capture = routing_root / "captures" / "inv-routing"
    _write_small_plan(routing_root)
    routing_finding = {
        "id": "F-route-alpha",
        "source": {"kind": "context-record", "id": "routing-evidence-1"},
        "policy_id": "axis",
        "statement": "route alpha",
        "disposition": "accepted",
        "reason": "driver accepted and routed the finding",
        "owner_phase": "implementation",
        "task_ids": ["alpha"],
        "review_axes": ["axis"],
        "status": "unresolved",
    }
    routing_context = [
        {
            "id": "routing-evidence-1",
            "kind": "review-evidence",
            "data": {
                "gate": "plan-review",
                "policy_id": "axis",
                "result": "fail",
                "findings": "route alpha",
                "author": {"name": "routing-reviewer", "kind": "agent"},
                "subject": "plan.json",
                "subject_revision": "1",
                "config_version": "test-1",
            },
            "sequence": 1,
            "created_at": 1,
        },
        {
            "id": "routing-ledger-1",
            "kind": "finding-ledger",
            "data": {
                "schema_version": "1",
                "gate": "plan-review",
                "subject": "plan.json",
                "subject_revision": "1",
                "author": {"name": "driver", "kind": "agent"},
                "findings": [routing_finding],
            },
            "sequence": 2,
            "created_at": 2,
        },
        {
            "id": "routing-proposal-1",
            "kind": "advisory-finding-proposal",
            "data": {"proposals": [{"candidate_source_ids": ["source-1"]}]},
            "sequence": 3,
            "created_at": 3,
        },
    ]
    routing = _run_binding(
        implement_graph_runner_binding(
            provider=provider,
            task_worker=stdin_worker_cli(routing_receipts),
            working_directory=work_dir,
        ),
        stdin=_invoke_packet(
            run_id=run_id,
            slot_id=slot_id,
            artifact_root=routing_root,
            instruction_body=instruction_body,
            capture_dir=routing_capture,
            context=routing_context,
        ),
    )
    if routing.returncode != 0:
        raise WorkSlotJourneyFailure(
            "finding-routing graph exited "
            f"{routing.returncode}: {routing.stderr.decode('utf-8', 'replace')}"
        )
    for task in plan["tasks"]:
        task_id = task["id"]
        recorded = parse_graph_runner_stdin(
            (routing_receipts / f"{task_id}.stdin").read_text(encoding="utf-8")
        )
        expected = dict(task)
        expected["finding_context"] = (
            [routing_finding] if task_id == "alpha" else []
        )
        if recorded["task"] != expected:
            raise WorkSlotJourneyFailure(
                f"finding routing mismatch for {task_id}: {recorded['task']}"
            )
    _assert_capture_files(routing_capture, (*task_ids, "summarizer"))

    missing_root = work_dir / "missing-report"
    missing_receipts = missing_root / "receipts"
    missing_capture = missing_root / "captures" / "inv-missing"
    _write_small_plan(missing_root)
    missing = _run_binding(
        implement_graph_runner_binding(
            provider=provider,
            task_worker=stdin_worker_cli(missing_receipts, ("--no-report",)),
            working_directory=work_dir,
        ),
        stdin=_invoke_packet(
            run_id=run_id,
            slot_id=slot_id,
            artifact_root=missing_root,
            instruction_body=instruction_body,
            capture_dir=missing_capture,
        ),
    )
    if missing.returncode == 0:
        raise WorkSlotJourneyFailure(
            "graph-runner missing implementation-report.json unexpectedly exited 0"
        )
    _assert_capture_files(missing_capture, task_ids)
    for task_id in ("alpha", "beta", "gamma"):
        if not (missing_receipts / f"{task_id}.stdin").is_file():
            raise WorkSlotJourneyFailure(
                f"missing-report path did not run task {task_id} before failing"
            )
        _assert_task_receipt(
            missing_receipts,
            tasks_by_id[task_id],
            artifact_root=missing_root,
        )
    if (missing_root / "implementation-report.json").is_file():
        raise WorkSlotJourneyFailure("missing-report path found a report file")
    if not (missing_receipts / "summarizer.stdin").is_file():
        raise WorkSlotJourneyFailure(
            "missing-report path did not run summarizer before failing the report gate"
        )

    reap_root = work_dir / "reap"
    reap_receipts = reap_root / "receipts"
    reap_capture = reap_root / "captures" / "inv-reap"
    _write_small_plan(reap_root)
    reap = _run_binding(
        implement_graph_runner_binding(
            provider=provider,
            task_worker=stdin_worker_cli(
                reap_receipts, ("--sleep", "0.4", "--fail-task", "alpha")
            ),
            working_directory=work_dir,
        ),
        stdin=_invoke_packet(
            run_id=run_id,
            slot_id=slot_id,
            artifact_root=reap_root,
            instruction_body=instruction_body,
            capture_dir=reap_capture,
        ),
    )
    if reap.returncode == 0:
        raise WorkSlotJourneyFailure("graph-runner failing sibling path exited 0")
    beta_done = reap_receipts / "beta.stdin.done"
    if not beta_done.is_file():
        raise WorkSlotJourneyFailure(
            "failing sibling was not reaped before run-plan-graph exited"
        )
    beta_pid = _read_pid(reap_receipts / "beta.stdin.pid")
    if pid_is_alive(beta_pid):
        raise WorkSlotJourneyFailure(
            f"failing-sibling pid {beta_pid} still alive after runner exit"
        )
    if (reap_receipts / "gamma.stdin").exists():
        raise WorkSlotJourneyFailure("dependent task started after a sibling failed")
    if (reap_receipts / "summarizer.stdin").exists():
        raise WorkSlotJourneyFailure("summarizer started after a sibling failed")
    _assert_task_receipt(
        reap_receipts,
        tasks_by_id["alpha"],
        artifact_root=reap_root,
    )
    _assert_task_receipt(
        reap_receipts,
        tasks_by_id["beta"],
        artifact_root=reap_root,
    )
    reap_workers = _load_summary_workers(reap_capture)
    reap_ids = ["alpha", "beta"]
    _assert_capture_files(reap_capture, reap_ids)
    if [worker.get("exit_code") for worker in reap_workers] != [1, 0]:
        raise WorkSlotJourneyFailure(
            f"reap summary must list alpha then beta exits: {reap_workers}"
        )
    if (reap_capture / "gamma" / "stdout").exists():
        raise WorkSlotJourneyFailure("reap path captured unstarted dependent task stdout")
    summary_paths = [str(worker.get("stdout_path") or "") for worker in reap_workers]
    if any(path.endswith("/gamma/stdout") or path.endswith("gamma/stdout") for path in summary_paths):
        raise WorkSlotJourneyFailure(f"reap summary listed unstarted gamma: {reap_workers}")
    if (reap_capture / "summarizer" / "stdout").exists():
        raise WorkSlotJourneyFailure("summarizer stdout exists after a sibling failed")
    if (reap_root / "implementation-report.json").is_file():
        raise WorkSlotJourneyFailure("ordinary failing tasks wrote implementation-report.json")

    # bookends:LE-102 — plan-graph subset selection refuses missing prerequisites and summarizes the resulting tree.
    # Plan-graph subset: missing prerequisites refuse before Dagu, while a
    # selected root brings in its dependants and the sole summarizer observes
    # the resulting working tree rather than a selected-task projection.
    subset_root = work_dir / "subset-resulting-tree"
    subset_receipts = subset_root / "receipts"
    subset_capture = subset_root / "captures" / "inv-subset"
    subset_root.mkdir(parents=True, exist_ok=True)
    _ensure_git_repository(subset_root)
    subset_plan = {
        "revision": "subset-1",
        "tasks": [
            {"id": "alpha", "objective": "root", "dependencies": []},
            {"id": "beta", "objective": "dependent", "dependencies": ["alpha"]},
            {"id": "gamma", "objective": "other dependent", "dependencies": ["alpha"]},
        ],
        "dependency_graph": [
            {"from": "alpha", "to": "beta"},
            {"from": "alpha", "to": "gamma"},
        ],
    }
    _write_plan_document(subset_root, subset_plan)
    (subset_root / "intent.json").write_text('{"revision":"1"}', encoding="utf-8")
    (subset_root / "design.json").write_text('{"revision":"1"}', encoding="utf-8")
    subset_worker = subset_root / "subset-task-worker.py"
    subset_worker.write_text(
        "import json, sys\nfrom pathlib import Path\n"
        "raw = sys.stdin.buffer.read(); location_raw, rest = raw.decode().split('\\n---\\n\\n', 1)\n"
        "location = json.loads(location_raw); root = Path(location['artifact_root'])\n"
        "if rest.startswith('Write artifact_root/implementation-report.json'):\n"
        "    plan = json.loads((root / 'plan.json').read_text())\n"
        "    report = {'revision':'1','author':{'name':'subset-worker','kind':'script'},'plan_revision':plan['revision'],'coverage':{'commit':'subset','documents':[{'path':'plan.json','revision':plan['revision']}]},'summary':'resulting tree','changed_surface':sorted(p.name for p in Path.cwd().iterdir()),'validation':['subset']}\n"
        "    (root / 'implementation-report.json').write_text(json.dumps(report)+'\\n')\n"
        "else:\n"
        "    task = json.loads(rest); (Path.cwd() / ('effect-' + task['id'] + '.txt')).write_text('present\\n')\n",
        encoding="utf-8",
    )
    base_binding = implement_graph_runner_binding(
        provider=provider,
        task_worker=stdin_worker_cli(subset_receipts),
        working_directory=subset_root,
    )
    base_args = list(base_binding["args"][:-2])
    # Put task-worker after the selection flags: construct the same public
    # command shape accepted by the provider CLI.
    task_worker_json = worker_cli_json({"command": sys.executable, "args": [str(subset_worker)]})
    subset_binding = {"command": base_binding["command"], "args": base_args + ["--task", "beta", "--task-worker", task_worker_json]}
    missing = _run_binding(
        subset_binding,
        stdin=_invoke_packet(
            run_id="subset-missing",
            slot_id="implement",
            artifact_root=subset_root,
            instruction_body=instruction_body,
            capture_dir=subset_capture / "missing",
        ),
    )
    if missing.returncode == 0 or (subset_root / "effect-beta.txt").exists():
        raise WorkSlotJourneyFailure("plan subset auto-included missing prerequisite or started a task")

    for invalid_name, invalid_selection in (
        ("empty", ["--tasks", ""]),
        ("empty-json", ["--tasks", "[]"]),
        ("unknown", ["--task", "missing"]),
        ("duplicate", ["--task", "alpha", "--task", "alpha"]),
    ):
        invalid_capture = subset_capture / ("invalid-" + invalid_name)
        invalid_args = base_args + invalid_selection + ["--task-worker", task_worker_json]
        invalid_binding = {"command": base_binding["command"], "args": invalid_args}
        invalid = _run_binding(
            invalid_binding,
            stdin=_invoke_packet(
                run_id="subset-invalid",
                slot_id="implement",
                artifact_root=subset_root,
                instruction_body=instruction_body,
                capture_dir=invalid_capture,
            ),
        )
        if invalid.returncode == 0:
            raise WorkSlotJourneyFailure(
                f"invalid plan subset unexpectedly succeeded: {invalid_selection}"
            )
        if any((subset_root / f"effect-{task_id}.txt").exists() for task_id in subset_plan["tasks"]):
            raise WorkSlotJourneyFailure(
                f"invalid plan subset started a task: {invalid_selection}"
            )

    subset_binding["args"] = base_args + ["--task", "alpha", "--task-worker", task_worker_json]
    selected = _run_binding(
        subset_binding,
        stdin=_invoke_packet(
            run_id="subset-selected",
            slot_id="implement",
            artifact_root=subset_root,
            instruction_body=instruction_body,
            capture_dir=subset_capture / "selected",
        ),
    )
    if selected.returncode != 0:
        raise WorkSlotJourneyFailure(
            f"plan subset root did not run dependants: {selected.stderr.decode('utf-8', 'replace')}"
        )
    for task_id in ("alpha", "beta", "gamma"):
        if not (subset_root / f"effect-{task_id}.txt").is_file():
            raise WorkSlotJourneyFailure(f"selected root omitted dependant effect {task_id}")
    report = json.loads((subset_root / "implementation-report.json").read_text(encoding="utf-8"))
    if not all(name in report.get("changed_surface", []) for name in ("effect-alpha.txt", "effect-beta.txt", "effect-gamma.txt")):
        raise WorkSlotJourneyFailure(f"resulting-tree report omitted task effects: {report}")
    if not (subset_root / "implementation-checkpoint.json").is_file():
        raise WorkSlotJourneyFailure("subset graph omitted resulting-tree checkpoint")

    # A direct provider invocation has no engine standing-id packet, so a
    # matching successful sidecar result may satisfy alpha. Select beta only
    # and prove the summarizer still reports the leftover gamma effect from the
    # resulting working tree; selecting alpha here would hide the leftover.
    leftover_capture = subset_capture / "leftover"
    leftover_binding = {
        "command": base_binding["command"],
        "args": base_args + ["--task", "beta", "--task-worker", worker_cli_json({"command": sys.executable, "args": [str(subset_worker)]})],
    }
    leftover = _run_binding(
        leftover_binding,
        stdin=_invoke_packet(
            run_id="subset-leftover",
            slot_id="implement",
            artifact_root=subset_root,
            instruction_body=instruction_body,
            capture_dir=leftover_capture,
        ),
    )
    if leftover.returncode != 0:
        raise WorkSlotJourneyFailure(
            f"direct beta subset with sidecar-standing alpha failed: {leftover.stderr.decode('utf-8', 'replace')}"
        )
    leftover_workers = _load_summary_workers(leftover_capture)
    if [worker.get("assignment_id") for worker in leftover_workers] != ["beta"]:
        raise WorkSlotJourneyFailure(
            f"beta subset recreated an unselected task: {leftover_workers}"
        )
    report = json.loads((subset_root / "implementation-report.json").read_text(encoding="utf-8"))
    changed_surface = report.get("changed_surface", [])
    if "effect-gamma.txt" not in changed_surface:
        raise WorkSlotJourneyFailure(
            f"leftover gamma effect disappeared from changed_surface: {report}"
        )
    checkpoint = json.loads((subset_root / "implementation-checkpoint.json").read_text(encoding="utf-8"))
    checkpoint_text = json.dumps(checkpoint, sort_keys=True)
    if "effect-gamma.txt" not in checkpoint_text:
        raise WorkSlotJourneyFailure(
            f"leftover gamma effect disappeared from checkpoint: {checkpoint}"
        )

    # bookends:LE-102 — bound invocation_input selects plan-task roots through
    # the provider boundary and refuses malformed or ambiguous packets before
    # any graph process, worker, summarizer, report, or checkpoint starts.
    invalid_input_root = work_dir / "bound-invocation-input-invalid"

    def packet_with_invocation_input(
        *,
        case_run_id: str,
        artifact_root: Path,
        capture_dir: Path,
        invocation_input: Any,
    ) -> bytes:
        packet = json.loads(
            _invoke_packet(
                run_id=case_run_id,
                slot_id=slot_id,
                artifact_root=artifact_root,
                instruction_body=instruction_body,
                capture_dir=capture_dir,
            )
        )
        packet["invocation_input"] = invocation_input
        return json.dumps(packet, separators=(",", ":")).encode("utf-8")

    def tree_entries(path: Path) -> list[str]:
        return sorted(
            item.relative_to(path).as_posix()
            for item in path.rglob("*")
        )

    def assert_invalid_invocation_input(
        label: str,
        invocation_input: Any,
        *,
        frozen_selection_args: Sequence[str] = (),
        expected_error: str,
    ) -> None:
        case_root = invalid_input_root / label
        case_capture = case_root / "captures" / "invocation"
        case_receipts = case_root / "receipts"
        _write_small_plan(case_root)
        case_capture.mkdir(parents=True, exist_ok=True)
        case_receipts.mkdir(parents=True, exist_ok=True)
        artifact_sentinel = case_root / "artifact-sentinel"
        artifact_sentinel.write_text(f"artifact:{label}\n", encoding="utf-8")
        report_sentinel = b'{"sentinel":"implementation-report"}\n'
        checkpoint_sentinel = b'{"sentinel":"implementation-checkpoint"}\n'
        report_path = case_root / "implementation-report.json"
        checkpoint_path = case_root / "implementation-checkpoint.json"
        report_path.write_bytes(report_sentinel)
        checkpoint_path.write_bytes(checkpoint_sentinel)
        capture_sentinel = case_capture / "capture-sentinel"
        receipt_sentinel = case_receipts / "receipt-sentinel"
        capture_sentinel.write_text(f"capture:{label}\n", encoding="utf-8")
        receipt_sentinel.write_text(f"receipt:{label}\n", encoding="utf-8")

        dagu_bin = case_root / "fake-dagu-bin"
        dagu_bin.mkdir(parents=True, exist_ok=True)
        dagu = dagu_bin / "dagu"
        dagu.write_text(
            "#!/bin/sh\n"
            "printf 'probed\\n' >> \"$(dirname \"$0\")/probed\"\n"
            "if [ \"$1\" = version ]; then printf '2.14.0\\n'; fi\n"
            "exit 99\n",
            encoding="utf-8",
        )
        dagu.chmod(0o755)
        probe_marker = dagu_bin / "probed"

        base_binding = implement_graph_runner_binding(
            provider=provider,
            task_worker=stdin_worker_cli(case_receipts),
            working_directory=work_dir,
        )
        worker_json = base_binding["args"][-1]
        case_binding = {
            "command": base_binding["command"],
            "args": list(base_binding["args"][:-2])
            + list(frozen_selection_args)
            + ["--task-worker", worker_json],
        }
        environment = os.environ.copy()
        environment["PATH"] = str(dagu_bin)
        refused = _run_binding(
            case_binding,
            stdin=packet_with_invocation_input(
                case_run_id=f"bound-invalid-{label}",
                artifact_root=case_root,
                capture_dir=case_capture,
                invocation_input=invocation_input,
            ),
            env=environment,
        )
        stderr = refused.stderr.decode("utf-8", "replace")
        if refused.returncode != 2:
            raise WorkSlotJourneyFailure(
                f"bound invocation_input {label} returned {refused.returncode}, "
                f"expected usage refusal: {stderr}"
            )
        if expected_error not in stderr:
            raise WorkSlotJourneyFailure(
                f"bound invocation_input {label} omitted {expected_error!r}: {stderr}"
            )
        if probe_marker.exists():
            raise WorkSlotJourneyFailure(
                f"bound invocation_input {label} probed Dagu: {stderr}"
            )
        if artifact_sentinel.read_text(encoding="utf-8") != f"artifact:{label}\n":
            raise WorkSlotJourneyFailure(
                f"bound invocation_input {label} modified artifact sentinel"
            )
        if report_path.read_bytes() != report_sentinel:
            raise WorkSlotJourneyFailure(
                f"bound invocation_input {label} modified implementation-report.json"
            )
        if checkpoint_path.read_bytes() != checkpoint_sentinel:
            raise WorkSlotJourneyFailure(
                f"bound invocation_input {label} modified implementation checkpoint"
            )
        if tree_entries(case_capture) != ["capture-sentinel"]:
            raise WorkSlotJourneyFailure(
                f"bound invocation_input {label} created capture output: "
                f"{tree_entries(case_capture)}"
            )
        if tree_entries(case_receipts) != ["receipt-sentinel"]:
            raise WorkSlotJourneyFailure(
                f"bound invocation_input {label} started an ordinary worker: "
                f"{tree_entries(case_receipts)}"
            )
        if (case_root / "plan-task-results.json").exists():
            raise WorkSlotJourneyFailure(
                f"bound invocation_input {label} wrote plan-task-results.json"
            )

    for label, invocation_input, frozen_selection_args, expected_error in (
        ("malformed", "not an object", (), "invocation_input must be exactly"),
        ("null", None, (), "must be an object when present"),
        (
            "wrong-types",
            {"plan_revision": 7, "task_roots": ["alpha"]},
            (),
            "invocation_input must be exactly",
        ),
        (
            "extra",
            {"plan_revision": plan_revision, "task_roots": ["alpha"], "extra": True},
            (),
            "invocation_input must be exactly",
        ),
        (
            "empty-roots",
            {"plan_revision": plan_revision, "task_roots": []},
            (),
            "task_roots must be a non-empty",
        ),
        (
            "duplicate-roots",
            {"plan_revision": plan_revision, "task_roots": ["alpha", "alpha"]},
            (),
            "duplicate task",
        ),
        (
            "unknown-root",
            {"plan_revision": plan_revision, "task_roots": ["missing"]},
            (),
            "unknown task",
        ),
        (
            "stale-plan-revision",
            {"plan_revision": "stale-plan", "task_roots": ["alpha"]},
            (),
            "does not match plan.json revision",
        ),
        (
            "missing-prerequisite",
            {"plan_revision": plan_revision, "task_roots": ["gamma"]},
            (),
            "missing standing prerequisites",
        ),
        (
            "packet-plus-task",
            {"plan_revision": plan_revision, "task_roots": ["alpha"]},
            ("--task", "alpha"),
            "may not be combined",
        ),
        (
            "packet-plus-tasks",
            {"plan_revision": plan_revision, "task_roots": ["alpha"]},
            ("--tasks", "alpha"),
            "may not be combined",
        ),
    ):
        assert_invalid_invocation_input(
            label,
            invocation_input,
            frozen_selection_args=frozen_selection_args,
            expected_error=expected_error,
        )

    selection_root = work_dir / "bound-invocation-input-selection"
    selection_capture = selection_root / "captures" / "invocation-selected"
    selection_receipts = selection_root / "receipts"
    selection_plan = {
        "revision": "invocation-selection-1",
        "tasks": [
            {"id": "root", "objective": "selected root", "dependencies": []},
            {
                "id": "dependent",
                "objective": "transitive dependant",
                "dependencies": ["root"],
            },
            {
                "id": "leaf",
                "objective": "transitive leaf",
                "dependencies": ["dependent"],
            },
            {"id": "unselected", "objective": "outside closure", "dependencies": []},
        ],
        "dependency_graph": [
            {"from": "root", "to": "dependent"},
            {"from": "dependent", "to": "leaf"},
        ],
    }
    _write_plan_document(selection_root, selection_plan)
    (selection_root / "intent.json").write_text('{"revision":"1"}', encoding="utf-8")
    (selection_root / "design.json").write_text('{"revision":"1"}', encoding="utf-8")
    selection_binding = implement_graph_runner_binding(
        provider=provider,
        task_worker=stdin_worker_cli(selection_receipts),
        working_directory=work_dir,
    )
    selection_input = {
        "plan_revision": selection_plan["revision"],
        "task_roots": ["root"],
    }
    selected = _run_binding(
        selection_binding,
        stdin=packet_with_invocation_input(
            case_run_id="bound-selection",
            artifact_root=selection_root,
            capture_dir=selection_capture,
            invocation_input=selection_input,
        ),
    )
    if selected.returncode != 0:
        raise WorkSlotJourneyFailure(
            "bound invocation_input selected graph exited "
            f"{selected.returncode}: {selected.stderr.decode('utf-8', 'replace')}"
        )
    selected_ids = ["root", "dependent", "leaf"]
    for task_id in (*selected_ids, "summarizer"):
        if not (selection_receipts / f"{task_id}.stdin").is_file():
            raise WorkSlotJourneyFailure(
                f"bound invocation_input selection omitted {task_id} receipt"
            )
    if (selection_receipts / "unselected.stdin").exists():
        raise WorkSlotJourneyFailure(
            "bound invocation_input selection started an unselected task"
        )
    _assert_capture_files(selection_capture, (*selected_ids, "summarizer"))
    selected_workers = _load_summary_workers(selection_capture)
    if [worker.get("assignment_id") for worker in selected_workers] != selected_ids:
        raise WorkSlotJourneyFailure(
            f"bound invocation_input selection summary mismatch: {selected_workers}"
        )
    selection_path = selection_capture / "selection.json"
    try:
        selection_record = json.loads(selection_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise WorkSlotJourneyFailure(
            f"bound invocation_input selection omitted durable selection.json: {error}"
        ) from error
    expected_selection = {
        "schema_version": "1",
        "requested": ["root"],
        "tasks": selected_ids,
    }
    if selection_record != expected_selection:
        raise WorkSlotJourneyFailure(
            f"bound invocation_input selection record mismatch: {selection_record}"
        )
    _assert_dummy_report(selection_root, selection_plan["revision"])
    if not (selection_root / "implementation-checkpoint.json").is_file():
        raise WorkSlotJourneyFailure(
            "bound invocation_input selection omitted implementation checkpoint"
        )

    return [
        "PATH rewrite software-change -> built provider",
        "implement args [run-plan-graph, --task-worker, dummy-json]",
        "small plan two independent plus one dependent",
        "success path exits 0 when summarizer writes implementation-report.json",
        "dummy receipts match compact artifact_root stdin layout",
        "capture_dir summary.json and per-task stdout/stderr",
        "advisory proposal alone is inert in every task packet",
        "driver-confirmed ledger routes only the exact implementation task",
        "missing implementation-report.json exits nonzero",
        "failing sibling reaped before runner exits",
        "legacy --task/--tasks selection remains covered",
        "bound invocation_input invalid packets refuse before Dagu and all workers",
        "bound invocation_input selected root expands through transitive dependants",
        "bound invocation_input selection.json records the durable selection",
        "no live model",
    ]


def prove_engine_standing_join(
    *, engine: Path, provider: Path, work_dir: Path
) -> list[str]:
    """Prove bound plan subsets intersect sidecar success with engine standing."""
    work_dir.mkdir(parents=True, exist_ok=True)
    _ensure_git_repository(work_dir)
    artifact_root = work_dir / "artifacts"
    _write_small_plan(artifact_root)
    record_receipts = work_dir / "record-receipts"
    select_receipts = work_dir / "select-receipts"
    known_effect = ('--stdout', '{"repository_effect":{"kind":"dummy"}}')
    record_binding = implement_graph_runner_binding(
        provider=provider,
        task_worker=stdin_worker_cli(record_receipts, known_effect),
        working_directory=work_dir,
    )
    select_args = list(record_binding["args"][:-2]) + [
        "--task",
        "gamma",
        *record_binding["args"][-2:],
    ]
    select_args[-1] = worker_cli_json(
        stdin_worker_cli(select_receipts, known_effect)
    )
    select_binding = {"command": str(provider), "args": select_args}

    describe_provider = work_dir / "standing-provider.py"
    workflow = {
        "id": "standing-join-proof",
        "initial_state": "work",
        "states": [
            {
                "id": "work",
                "title": "Work",
                "instructions": "Run the small plan.",
                "final": False,
            }
        ],
        "transitions": [],
        "work_slots": [
            {
                "id": "record",
                "state": "work",
                "event": "recorded",
                "stdin_context_kinds": ["finding-ledger"],
            },
            {
                "id": "select",
                "state": "work",
                "event": "selected",
                "stdin_context_kinds": ["finding-ledger"],
            },
        ],
    }
    describe_provider.write_text(
        "#!/usr/bin/env python3\n"
        "import json, sys\n"
        "json.load(sys.stdin)\n"
        f"json.dump({workflow!r}, sys.stdout)\n",
        encoding="utf-8",
    )
    describe_provider.chmod(0o755)
    providers = work_dir / "providers.toml"
    providers.write_text(
        "[providers.standing-proof]\n"
        f"command = {json.dumps(str(describe_provider))}\n"
        "args = []\n",
        encoding="utf-8",
    )
    database = work_dir / "loop.sqlite"
    run_id = "engine-standing-join"
    initial_input = {
        "artifact_root": str(artifact_root),
        "work_slot_bindings": {
            "record": record_binding,
            "select": select_binding,
        },
    }
    started = _engine_json(
        engine,
        database,
        [
            "--config",
            str(providers),
            "--timeout-ms",
            "30000",
            "start",
            "--id",
            run_id,
            "standing-proof",
            json.dumps(initial_input, separators=(",", ":")),
        ],
    )
    if started.get("status") != "completed":
        raise WorkSlotJourneyFailure(f"standing join start failed: {started}")

    def engine_call(operation: Sequence[str]) -> dict[str, Any]:
        return _engine_json(engine, database, operation)

    shown = engine_call(["show", run_id])
    if shown.get("status") != "completed":
        raise WorkSlotJourneyFailure(f"standing join initial show failed: {shown}")
    recorded = invoke_until_succeeded(
        engine_call, run_id, "record", timeout_s=30.0
    )
    record_workers = recorded.get("inner_workers")
    if not isinstance(record_workers, list) or [
        worker.get("assignment_id") for worker in record_workers
    ] != ["alpha", "beta", "gamma"]:
        raise WorkSlotJourneyFailure(
            f"standing join full graph did not record A/B/C: {recorded}"
        )

    connection = sqlite3.connect(database)
    try:
        row = connection.execute(
            "SELECT inner_workers_json FROM work_slot_invocations WHERE invocation_id = ?1",
            (recorded["invocation_id"],),
        ).fetchone()
        if row is None:
            raise WorkSlotJourneyFailure("standing join record invocation was not durable")
        current_workers = json.loads(row[0])
        alpha = next(
            worker
            for worker in current_workers
            if worker.get("assignment_id") == "alpha"
        )
        task_definition = dict(alpha.get("task_definition") or {})
        task_definition["objective"] = "Dirty A without changing plan revision"
        alpha["task_definition"] = task_definition
        connection.execute(
            "UPDATE work_slot_invocations SET inner_workers_json = ?1 WHERE invocation_id = ?2",
            (json.dumps(current_workers, separators=(",", ":")), recorded["invocation_id"]),
        )
        connection.commit()
    finally:
        connection.close()

    dirty_show = engine_call(["show", run_id])
    plan_results = dirty_show.get("result", {}).get("change_report", {}).get(
        "plan_task_results", []
    )
    alpha_result = next(
        result for result in plan_results if result.get("assignment_id") == "alpha"
    )
    if (
        alpha_result.get("standing") is not False
        or alpha_result.get("dimensions", {})
        .get("task_definition", {})
        .get("changed")
        is not True
    ):
        raise WorkSlotJourneyFailure(
            f"dirty A remained standing in engine change report: {alpha_result}"
        )

    refused = invoke_until_status(
        engine_call, run_id, "select", expected="failed", timeout_s=20.0
    )
    refused_capture = Path(refused["capture_dir"])
    if (refused_capture / "dagu-home").exists() or (select_receipts / "gamma.stdin").exists():
        raise WorkSlotJourneyFailure(
            "dirty A subset reached Dagu or started C before explicit carry"
        )

    # A dirty engine-generated task result remains non-standing. The stable
    # reference contract does not turn generic plan results into semantic
    # carry acts or ask the driver to enumerate changed inputs.
    for kind in ("unchanged-carry", "override-carry"):
        retired = engine_call(
            [
                "append",
                f"--record-id=standing-retired-{kind}",
                run_id,
                kind,
                json.dumps(
                    {
                        "source_record_id": "",
                        "invocation_id": recorded["invocation_id"],
                        "assignment_id": "alpha",
                        "attesting_driver": {"name": "standing-driver", "kind": "agent"},
                    },
                    separators=(",", ":"),
                ),
            ]
        )
        if retired.get("status") != "rejected" or retired.get("code") != "carry-refused":
            raise WorkSlotJourneyFailure(
                f"retired {kind} operation was not rejected: {retired}"
            )

    post_retirement_show = engine_call(["show", run_id])
    if post_retirement_show.get("status") != "completed":
        raise WorkSlotJourneyFailure(f"standing join re-show failed: {post_retirement_show}")
    post_results = post_retirement_show.get("result", {}).get("change_report", {}).get(
        "plan_task_results", []
    )
    post_alpha = next(
        result for result in post_results if result.get("assignment_id") == "alpha"
    )
    if post_alpha.get("standing") is not False:
        raise WorkSlotJourneyFailure(
            f"retired carry attempt changed dirty task standing: {post_alpha}"
        )
    return [
        "engine show marks dirty A non-standing while sidecar success remains",
        "bound --task gamma refuses before Dagu when a prerequisite is dirty",
        "historical carry acts are rejected rather than retained as an ordinary path",
        "retired carry attempts do not change engine standing or durable task results",
        "no live model",
    ]


def prove_fan_out(*, engine: Path, work_dir: Path) -> list[str]:
    """Prove loop-engine fan-out with dummy --worker CLIs. Never calls a live model."""
    work_dir.mkdir(parents=True, exist_ok=True)

    bound_root = work_dir / "bound"
    artifact_root = bound_root / "artifacts"
    artifact_root.mkdir(parents=True, exist_ok=True)
    bound_capture = bound_root / "captures" / "inv-bound"
    receipt_a = bound_root / "a.stdin"
    receipt_b = bound_root / "b.stdin"
    run_id = "fan-out-bound-run"
    slot_id = "design-review"
    instruction_body = "Review the design"
    bound_binding = fan_out_binding(
        engine=engine,
        workers=[
            stdin_worker_cli(receipt_a, ("--sleep", "0.2")),
            stdin_worker_cli(receipt_b, ("--sleep", "0.2")),
        ],
    )
    if bound_binding["command"] != str(engine):
        raise WorkSlotJourneyFailure(
            f"fan-out binding command was not rewritten: {bound_binding}"
        )
    if bound_binding["args"][:1] != SHIPPED_FAN_OUT_ARGS:
        raise WorkSlotJourneyFailure(
            f"fan-out binding args must start with fan-out: {bound_binding}"
        )
    bound = _run_binding(
        bound_binding,
        stdin=_invoke_packet(
            run_id=run_id,
            slot_id=slot_id,
            artifact_root=artifact_root,
            instruction_body=instruction_body,
            capture_dir=bound_capture,
        ),
        cwd=bound_root,
    )
    if bound.returncode != 0:
        raise WorkSlotJourneyFailure(
            "bound fan-out exited "
            f"{bound.returncode}: {bound.stderr.decode('utf-8', 'replace')}"
        )
    expected_bound = compact_artifact_root_stdin(artifact_root)
    recorded_a = receipt_a.read_bytes()
    recorded_b = receipt_b.read_bytes()
    if recorded_a != expected_bound or recorded_b != expected_bound:
        raise WorkSlotJourneyFailure(
            f"bound fan-out stdin mismatch: {recorded_a!r} / {recorded_b!r}"
        )
    if b"instruction_body" in recorded_a:
        raise WorkSlotJourneyFailure(
            f"bound fan-out dumped instruction_body: {recorded_a!r}"
        )
    if recorded_a != recorded_b:
        raise WorkSlotJourneyFailure("bound workers recorded different stdin")
    for receipt in (receipt_a, receipt_b):
        if not receipt.with_name(receipt.name + ".done").is_file():
            raise WorkSlotJourneyFailure(
                f"bound worker {receipt.name} was not reaped before fan-out exited"
            )
        pid = _read_pid(receipt.with_name(receipt.name + ".pid"))
        if pid_is_alive(pid):
            raise WorkSlotJourneyFailure(
                f"bound worker pid {pid} still alive after fan-out exit"
            )
    _assert_capture_files(bound_capture, ("0", "1"))
    _assert_yaml_omits_max_active_steps(bound_capture)
    bound_workers = _load_summary_workers(bound_capture)
    if [worker.get("exit_code") for worker in bound_workers] != [0, 0]:
        raise WorkSlotJourneyFailure(
            f"bound summary exit codes mismatch: {bound_workers}"
        )
    bound_summary = _json_stdout(bound)
    if bound_summary.get("output_dir") != str(bound_capture):
        raise WorkSlotJourneyFailure(
            f"bound collector output_dir {bound_summary.get('output_dir')!r} "
            f"!= capture_dir {bound_capture}"
        )
    legacy = artifact_root / "fan-out" / slot_id
    if legacy.exists():
        raise WorkSlotJourneyFailure(
            f"bound fan-out still wrote the legacy path {legacy}"
        )

    fail_root = work_dir / "inner-nonzero"
    fail_root.mkdir(parents=True, exist_ok=True)
    fail_capture = fail_root / "captures" / "inv-fail"
    fail_receipt = fail_root / "fail.stdin"
    ok_receipt = fail_root / "ok.stdin"
    fail = _run_binding(
        fan_out_binding(
            engine=engine,
            workers=[
                stdin_worker_cli(fail_receipt, ("--exit", "7")),
                stdin_worker_cli(ok_receipt),
            ],
        ),
        stdin=_invoke_packet(
            run_id=run_id,
            slot_id=slot_id,
            artifact_root=fail_root / "artifacts",
            instruction_body=instruction_body,
            capture_dir=fail_capture,
        ),
        cwd=fail_root,
    )
    if fail.returncode != 0:
        raise WorkSlotJourneyFailure(
            "inner-nonzero collector exited "
            f"{fail.returncode}: {fail.stderr.decode('utf-8', 'replace')}"
        )
    _assert_capture_files(fail_capture, ("0", "1"))
    fail_workers = _load_summary_workers(fail_capture)
    if [worker.get("exit_code") for worker in fail_workers] != [7, 0]:
        raise WorkSlotJourneyFailure(
            f"inner-nonzero summary must keep collector success with inner 7: {fail_workers}"
        )

    adhoc_root = work_dir / "adhoc"
    adhoc_root.mkdir(parents=True, exist_ok=True)
    shared = b"ad-hoc-shared-bytes-without-trailer"
    instructions = adhoc_root / "instructions.bin"
    instructions.write_bytes(shared)
    adhoc_a = adhoc_root / "a.stdin"
    adhoc_b = adhoc_root / "b.stdin"
    adhoc_binding = fan_out_binding(
        engine=engine,
        workers=[
            stdin_worker_cli(adhoc_a, ("--sleep", "0.2")),
            stdin_worker_cli(adhoc_b, ("--sleep", "0.2")),
        ],
        instructions=instructions,
    )
    adhoc = _run_binding(adhoc_binding, stdin=b"not a packet", cwd=adhoc_root)
    if adhoc.returncode != 0:
        raise WorkSlotJourneyFailure(
            "ad hoc fan-out exited "
            f"{adhoc.returncode}: {adhoc.stderr.decode('utf-8', 'replace')}"
        )
    if adhoc_a.read_bytes() != shared or adhoc_b.read_bytes() != shared:
        raise WorkSlotJourneyFailure(
            f"ad hoc fan-out stdin mismatch: {adhoc_a.read_bytes()!r} / {adhoc_b.read_bytes()!r}"
        )
    for receipt in (adhoc_a, adhoc_b):
        if not receipt.with_name(receipt.name + ".done").is_file():
            raise WorkSlotJourneyFailure(
                f"ad hoc worker {receipt.name} was not reaped before fan-out exited"
            )
        pid = _read_pid(receipt.with_name(receipt.name + ".pid"))
        if pid_is_alive(pid):
            raise WorkSlotJourneyFailure(
                f"ad hoc worker pid {pid} still alive after fan-out exit"
            )
    adhoc_summary = _json_stdout(adhoc)
    adhoc_dir = adhoc_summary.get("output_dir")
    if not isinstance(adhoc_dir, str) or not adhoc_dir:
        raise WorkSlotJourneyFailure(f"ad hoc collector omitted output_dir: {adhoc_summary}")
    _assert_capture_files(Path(adhoc_dir), ("0", "1"))
    _assert_yaml_omits_max_active_steps(Path(adhoc_dir))

    return [
        "PATH rewrite loop-engine -> built engine",
        "bound mode dummy workers record compact artifact_root stdin",
        "ad hoc mode dummy workers record exact --instructions bytes",
        "fan-out reaps every worker before exit",
        "bound outputs under packet.capture_dir",
        "inner nonzero exit with collector exit 0",
        "ad hoc summary.json present",
        "no live model",
    ]


def prove_full_schema_retry(*, engine: Path, work_dir: Path) -> list[str]:
    """Drive both bounded same-worker conformance outcomes through fan-out CLI."""
    work_dir.mkdir(parents=True, exist_ok=True)
    worker_script = work_dir / "full-schema-worker.py"
    worker_script.write_text(
        "import argparse, json\n"
        "from pathlib import Path\n"
        "parser = argparse.ArgumentParser()\n"
        "parser.add_argument('--state', required=True)\n"
        "parser.add_argument('--receipt', required=True)\n"
        "parser.add_argument('--mode', choices=('success', 'exhaust'), required=True)\n"
        "args = parser.parse_args()\n"
        "raw = __import__('sys').stdin.buffer.read()\n"
        "state = Path(args.state)\n"
        "count = int(state.read_text()) if state.exists() else 0\n"
        "state.write_text(str(count + 1))\n"
        "Path(args.receipt).write_bytes(raw)\n"
        "if args.mode == 'success' and count == 0:\n"
        "    __import__('sys').stdout.buffer.write(b'{\\\"axis\\\":\\\"wrong\\\"}')\n"
        "else:\n"
        "    output = {'axis':'retry-axis','author':{'name':'retry-author','kind':'script'},'result':'pass','findings':''}\n"
        "    if args.mode == 'exhaust':\n"
        "        output['axis'] = 'always-wrong'\n"
        "    __import__('sys').stdout.write(json.dumps(output, separators=(',', ':')))\n"
        "",
        encoding="utf-8",
    )
    worker_script.chmod(0o755)
    schema = {
        "type": "object",
        "required": ["axis", "author", "result", "findings"],
        "properties": {
            "axis": {"const": "retry-axis"},
            "author": {"const": {"name": "retry-author", "kind": "script"}},
            "result": {"enum": ["pass", "fail"]},
            "findings": {"type": "string"},
        },
        "additionalProperties": False,
    }

    def run_case(name: str, mode: str) -> tuple[Path, dict[str, Any], bytes, bytes]:
        root = work_dir / name
        root.mkdir(parents=True, exist_ok=True)
        instructions = root / "instructions.txt"
        instructions.write_bytes(b"frozen review assignment\n")
        state = root / "attempt-count"
        receipt = root / "last-stdin"
        worker = {
            "command": sys.executable,
            "args": [
                str(worker_script),
                "--state",
                str(state),
                "--receipt",
                str(receipt),
                "--mode",
                mode,
            ],
            "preamble": "same frozen reviewer assignment",
            "full_output_schema": schema,
        }
        completed = _run_binding(
            fan_out_binding(
                engine=engine,
                workers=[worker],
                instructions=instructions,
            ),
            stdin=b"not-an-invoke-packet",
            cwd=root,
        )
        capture_root = root / "fan-out-adhoc"
        captures = sorted(capture_root.iterdir()) if capture_root.is_dir() else []
        if len(captures) != 1 or not captures[0].is_dir():
            raise WorkSlotJourneyFailure(
                f"{name} did not leave exactly one capture root: {captures}"
            )
        capture = captures[0]
        summary_path = capture / "summary.json"
        if not summary_path.is_file():
            raise WorkSlotJourneyFailure(f"{name} omitted persisted summary: {capture}")
        summary = json.loads(summary_path.read_text(encoding="utf-8"))
        workers = summary.get("workers") if isinstance(summary, dict) else None
        if not isinstance(workers, list) or len(workers) != 1:
            raise WorkSlotJourneyFailure(f"{name} summary omitted one worker: {summary}")
        worker_summary = workers[0]
        if worker_summary.get("assignment_id") != "worker-0":
            raise WorkSlotJourneyFailure(
                f"{name} omitted stable assignment identity: {worker_summary}"
            )
        if worker_summary.get("command") != sys.executable or worker_summary.get("args") != worker["args"]:
            raise WorkSlotJourneyFailure(
                f"{name} changed the frozen worker command or args: {worker_summary}"
            )
        if worker_summary.get("attempts_path") != "0/attempts.json":
            raise WorkSlotJourneyFailure(
                f"{name} did not publish the relative attempts path: {worker_summary}"
            )
        attempts_path = capture / "0" / "attempts.json"
        manifest = json.loads(attempts_path.read_text(encoding="utf-8"))
        attempts = manifest.get("attempts")
        if manifest.get("schema_version") != "1" or not isinstance(attempts, list) or len(attempts) != 2:
            raise WorkSlotJourneyFailure(f"{name} did not preserve two attempts: {manifest}")
        for number, attempt in enumerate(attempts, start=1):
            if attempt.get("number") != number:
                raise WorkSlotJourneyFailure(f"{name} attempt numbering changed: {manifest}")
            stdout = (capture / "0" / "attempts" / str(number) / "stdout").read_bytes()
            stderr = (capture / "0" / "attempts" / str(number) / "stderr").read_bytes()
            if attempt.get("stdout_sha256") != "sha256:" + hashlib.sha256(stdout).hexdigest():
                raise WorkSlotJourneyFailure(f"{name} stdout digest lost raw bytes: {manifest}")
            if attempt.get("stderr_sha256") != "sha256:" + hashlib.sha256(stderr).hexdigest():
                raise WorkSlotJourneyFailure(f"{name} stderr digest lost raw bytes: {manifest}")
        compatibility_stdout = (capture / "0" / "stdout").read_bytes()
        compatibility_stderr = (capture / "0" / "stderr").read_bytes()
        return capture, worker_summary, compatibility_stdout, compatibility_stderr

    # bookends:LE-93 — one malformed response is corrected by the identical frozen worker, with both raw attempts and exact retry diagnostics retained.
    success_capture, success_summary, success_stdout, success_stderr = run_case(
        "invalid-then-valid", "success"
    )
    success_manifest = json.loads(
        (success_capture / "0" / "attempts.json").read_text(encoding="utf-8")
    )
    selected_attempt_path = success_capture / "0" / "attempts" / "2" / "stdout"
    if (
        success_summary.get("status") != "succeeded"
        or success_summary.get("selected_attempt") != 2
        or success_summary.get("selected_output_sha256")
        != "sha256:" + hashlib.sha256(selected_attempt_path.read_bytes()).hexdigest()
        or success_summary.get("selected_output_path") != str(selected_attempt_path)
        or success_manifest.get("selected_attempt") != 2
        or success_manifest.get("exhausted") is not False
        or success_stdout != (success_capture / "0" / "attempts" / "2" / "stdout").read_bytes()
        or success_stderr != (success_capture / "0" / "attempts" / "2" / "stderr").read_bytes()
        or json.loads(success_stdout).get("axis") != "retry-axis"
        or (work_dir / "invalid-then-valid" / "attempt-count").read_text() != "2"
    ):
        raise WorkSlotJourneyFailure(f"invalid-then-valid did not select attempt 2: {success_summary}")
    first_stdout = (success_capture / "0" / "attempts" / "1" / "stdout").read_bytes()
    second_assignment = (work_dir / "invalid-then-valid" / "last-stdin").read_bytes()
    original_assignment = (
        b"same frozen reviewer assignment\n---\n\nfrozen review assignment\n"
    )
    if not second_assignment.startswith(original_assignment):
        raise WorkSlotJourneyFailure(
            "retry assignment changed the frozen preamble or original assignment"
        )
    if first_stdout not in second_assignment:
        raise WorkSlotJourneyFailure("retry assignment omitted unchanged first stdout")
    for error in success_manifest["attempts"][0]["validation_errors"]:
        if error.encode("utf-8") not in second_assignment:
            raise WorkSlotJourneyFailure(
                f"retry assignment omitted exact validation error {error!r}"
            )

    exhaustion_capture, exhaustion_summary, exhaustion_stdout, exhaustion_stderr = run_case(
        "invalid-twice", "exhaust"
    )
    exhaustion_manifest = json.loads(
        (exhaustion_capture / "0" / "attempts.json").read_text(encoding="utf-8")
    )
    if (
        exhaustion_summary.get("status") != "failed"
        or exhaustion_summary.get("assignment_id") != "worker-0"
        or exhaustion_summary.get("selected_attempt", "missing") is not None
        or exhaustion_summary.get("selected_output_sha256", "missing") is not None
        or exhaustion_summary.get("selected_output_path", "missing") is not None
        or exhaustion_manifest.get("selected_attempt", "missing") is not None
        or exhaustion_manifest.get("exhausted") is not True
        or not isinstance(exhaustion_summary.get("conformance_error"), str)
        or "exhausted" not in exhaustion_summary["conformance_error"]
        or "result" in exhaustion_summary
        or exhaustion_stdout != (exhaustion_capture / "0" / "attempts" / "2" / "stdout").read_bytes()
        or exhaustion_stderr != (exhaustion_capture / "0" / "attempts" / "2" / "stderr").read_bytes()
        or (work_dir / "invalid-twice" / "attempt-count").read_text() != "2"
    ):
        raise WorkSlotJourneyFailure(
            f"invalid-twice did not fail closed after two attempts: {exhaustion_summary}"
        )
    return [
        "same frozen worker command/args and assignment on retry",
        "invalid-then-valid selects attempt 2 and preserves attempt 1 bytes",
        "attempts.json digests and exact validation errors are retrievable",
        "invalid-twice exhausts once with no substitute semantic verdict",
        "no live model",
    ]


def _prove_self_loop_requires_reobservation(*, engine: Path, work_dir: Path) -> None:
    """Prove a check-free self-loop creates a new unobserved state visit."""
    work_dir.mkdir(parents=True, exist_ok=True)
    provider = work_dir / "self-loop-provider.py"
    provider.write_text(
        "#!/usr/bin/env python3\n"
        "import json, sys\n"
        "json.load(sys.stdin)\n"
        "json.dump({"
        "'id':'self-loop-proof','initial_state':'start',"
        "'states':[{'id':'start','title':'Start','instructions':'Loop','final':False}],"
        "'transitions':[{'source':'start','event':'loop','target':'start','kind':'check-free'}]"
        "}, sys.stdout)\n",
        encoding="utf-8",
    )
    provider.chmod(0o755)
    providers = work_dir / "providers.toml"
    providers.write_text(
        "[providers.self-loop]\n"
        f"command = {json.dumps(str(provider))}\n"
        "args = []\n",
        encoding="utf-8",
    )
    database = work_dir / "loop.sqlite"
    run_id = "self-loop-reobservation"
    started = _engine_json(
        engine,
        database,
        ["--config", str(providers), "start", "--id", run_id, "self-loop", "{}"],
    )
    if started.get("status") != "completed":
        raise WorkSlotJourneyFailure(f"self-loop proof start failed: {started}")
    observed = _engine_json(engine, database, ["show", run_id])
    if observed.get("status") != "completed":
        raise WorkSlotJourneyFailure(f"self-loop proof show failed: {observed}")
    looped = _engine_json(engine, database, ["event", run_id, "loop"])
    if looped.get("status") != "completed":
        raise WorkSlotJourneyFailure(f"self-loop event failed: {looped}")
    unobserved = _engine_json(engine, database, ["append", run_id, "self-loop-proof", "{}"])
    if unobserved.get("status") != "rejected" or unobserved.get("code") != "run-not-observed":
        raise WorkSlotJourneyFailure(
            f"self-loop reused the prior observation token: {unobserved}"
        )
    reobserved = _engine_json(engine, database, ["show", run_id])
    if reobserved.get("status") != "completed":
        raise WorkSlotJourneyFailure(f"self-loop re-show failed: {reobserved}")
    appended = _engine_json(
        engine, database, ["append", run_id, "self-loop-proof", '{"ok":true}']
    )
    if appended.get("status") != "completed":
        raise WorkSlotJourneyFailure(
            f"self-loop append after re-show failed: {appended}"
        )


def prove_observation_before_mutation(
    *,
    engine: Path,
    provider: Path,
    profile_source: Path,
    fixture_root: Path,
    work_dir: Path,
) -> list[str]:
    """Prove the public show-before-mutation guard for every guarded act."""
    work_dir.mkdir(parents=True, exist_ok=True)
    review_binding = bindings_for(["intent-review"])
    engine_call, _, _ = _start_isolated_software_change(
        engine=engine,
        provider=provider,
        profile_source=profile_source,
        fixture_root=fixture_root,
        work_dir=work_dir,
        run_id="observation-guard",
        extra_bindings=review_binding,
        observe_start=False,
    )
    run_id = "observation-guard"
    database = work_dir / "loop.sqlite"
    # Safe reads after start do not arm a mutation. This check intentionally
    # runs before the first show in this isolated fixture.
    listed = _engine_json(engine, database, ["list"])
    if listed.get("status") != "completed":
        raise WorkSlotJourneyFailure(f"list failed after start: {listed}")
    progress = _engine_json(engine, database, ["invocation-progress", run_id])
    if progress.get("status") != "error" or progress.get("code") != "no-invocations":
        raise WorkSlotJourneyFailure(
            f"invocation-progress before the first invoke had an unexpected outcome: {progress}"
        )
    unarmed_append = _engine_json(engine, database, ["append", run_id, "after-safe-reads", "{}"])
    if unarmed_append.get("status") != "rejected" or unarmed_append.get("code") != "run-not-observed":
        raise WorkSlotJourneyFailure(
            f"list/invocation-progress unexpectedly armed append: {unarmed_append}"
        )
    observed_start = _engine_json(engine, database, ["show", run_id])
    if observed_start.get("status") != "completed":
        raise WorkSlotJourneyFailure(f"show after safe-read proof failed: {observed_start}")
    invoke_until_succeeded(engine_call, run_id, "intent-draft", timeout_s=20.0)
    database = work_dir / "loop.sqlite"
    # Do not use the journey wrapper here: each operation is a separate public
    # CLI process and must see the state visit as unobserved after this event.
    event = _engine_json(engine, database, ["event", run_id, "intent-ready"])
    if event.get("status") != "completed":
        raise WorkSlotJourneyFailure(f"observation fixture did not enter review: {event}")
    # Do not call show here: it would intentionally re-arm the new visit. A
    # history read is safe and gives us the post-transition comparison point.
    history_after_event = _engine_json(engine, database, ["history", run_id])
    if history_after_event.get("status") != "completed":
        raise WorkSlotJourneyFailure("observation fixture could not capture post-transition history")
    # Existing invalidity keeps its original refusal even while the visit is
    # unobserved. The checked event cannot run before its bound slot succeeds.
    missing_bound = _engine_json(engine, database, ["event", run_id, "approved"])
    if (
        missing_bound.get("status") != "rejected"
        or missing_bound.get("code") != "bound-slot-invocation-required"
    ):
        raise WorkSlotJourneyFailure(
            f"observation guard displaced existing bound-slot refusal: {missing_bound}"
        )
    # The event above replaced the observation token. All four otherwise-valid
    # guarded public operations must now refuse without a semantic side effect.
    operations = [
        ["append", run_id, "observation-test", "{}"],
        ["event", run_id, "revise"],
        ["invoke", run_id, "intent-review"],
        ["terminate", run_id],
    ]
    refusals = [_engine_json(engine, database, operation) for operation in operations]
    for operation, response in zip(operations, refusals):
        if response.get("status") != "rejected" or response.get("code") != "run-not-observed":
            raise WorkSlotJourneyFailure(f"unobserved {operation[0]} was not refused: {response}")
    after = _engine_json(engine, database, ["show", run_id])
    history_after = _engine_json(engine, database, ["history", run_id])
    if after.get("result", {}).get("current_state") != "intent-review":
        raise WorkSlotJourneyFailure(f"unobserved mutations changed state: {after}")
    if history_after.get("result") != history_after_event.get("result"):
        raise WorkSlotJourneyFailure("unobserved mutations changed semantic history")
    armed = _engine_json(engine, database, ["show", run_id])
    if armed.get("status") != "completed":
        raise WorkSlotJourneyFailure(f"show did not arm mutation: {armed}")
    appended = _engine_json(
        engine, database, ["append", run_id, "observation-test", '{"ok":true}']
    )
    if appended.get("status") != "completed":
        raise WorkSlotJourneyFailure(f"observed append did not succeed: {appended}")
    _prove_self_loop_requires_reobservation(
        engine=engine, work_dir=work_dir / "self-loop"
    )
    return [
        "list and invocation-progress do not arm mutation",
        "append, event, invoke, and terminate refuse without current show observation",
        "observation does not displace the existing bound-slot event refusal",
        "refusals preserve state and semantic history",
        "a check-free self-loop requires a fresh show before mutation",
    ]


def prove_invoke_subset(
    *,
    engine: Path,
    provider: Path,
    profile_source: Path,
    fixture_root: Path,
    work_dir: Path,
) -> list[str]:
    """Prove invoke subset filtering through separate public CLI processes."""
    work_dir.mkdir(parents=True, exist_ok=True)
    worker_script = work_dir / "subset-worker.py"
    worker_script.write_text(
        "import sys\nfrom pathlib import Path\n"
        "Path(sys.argv[1]).write_text('started\\n', encoding='utf-8')\n"
        "sys.stdin.buffer.read()\n",
        encoding="utf-8",
    )
    worker_script.chmod(0o755)
    zero = work_dir / "worker-0.started"
    one = work_dir / "worker-1.started"
    binding = append_fan_out_workers(
        {"command": str(engine), "args": ["fan-out"]},
        [
            {"command": sys.executable, "args": [str(worker_script), str(zero)]},
            {"command": sys.executable, "args": [str(worker_script), str(one)]},
        ],
    )
    engine_call, _, _ = _start_isolated_software_change(
        engine=engine,
        provider=provider,
        profile_source=profile_source,
        fixture_root=fixture_root,
        work_dir=work_dir / "run",
        run_id="invoke-subset",
        extra_bindings={"intent-draft": binding},
    )
    for invalid_args, expected_code in [
        (("--assignments=[]",), "empty-assignment-selection"),
        (("--assignment", "missing"), "unknown-assignment"),
        (("--assignment", "worker-0", "--assignment", "worker-0"), "duplicate-assignment-selection"),
    ]:
        refused = engine_call(["invoke", "invoke-subset", "intent-draft", *invalid_args])
        if refused.get("status") != "rejected" or refused.get("code") != expected_code:
            raise WorkSlotJourneyFailure(
                f"invalid public assignment selection was not refused as {expected_code}: {refused}"
            )
        if zero.exists() or one.exists():
            raise WorkSlotJourneyFailure(
                f"invalid assignment selection started a worker: zero={zero.exists()} one={one.exists()}"
            )

    overlay = invoke_until_succeeded(
        engine_call,
        "invoke-subset",
        "intent-draft",
        timeout_s=20.0,
        invoke_args=("--assignment", "worker-1"),
    )
    if zero.exists() or not one.is_file():
        raise WorkSlotJourneyFailure(
            f"invoke subset started the wrong assignments: zero={zero.exists()} one={one.exists()}"
        )
    if overlay.get("assignment_selection") != ["worker-1"]:
        raise WorkSlotJourneyFailure(f"invoke subset was not durable: {overlay}")
    shown = engine_call(["show", "invoke-subset"])
    if shown.get("status") != "completed":
        raise WorkSlotJourneyFailure(f"subset show failed: {shown}")
    frozen = shown.get("result", {}).get("initial_input", {}).get("work_slot_bindings", {})
    if frozen.get("intent-draft") != binding:
        raise WorkSlotJourneyFailure("invoke subset rewrote frozen binding")

    opaque_marker = work_dir / "opaque.started"
    opaque_binding = {
        "command": sys.executable,
        "args": [str(worker_script), str(opaque_marker)],
    }
    opaque_call, _, _ = _start_isolated_software_change(
        engine=engine,
        provider=provider,
        profile_source=profile_source,
        fixture_root=fixture_root,
        work_dir=work_dir / "opaque-run",
        run_id="invoke-opaque-selection",
        extra_bindings={"intent-draft": opaque_binding},
    )
    opaque = opaque_call(
        ["invoke", "invoke-opaque-selection", "intent-draft", "--assignment", "worker-0"]
    )
    if opaque.get("status") != "rejected" or opaque.get("code") != "assignments-not-enumerable":
        raise WorkSlotJourneyFailure(
            f"selection on an opaque binding was not refused before spawn: {opaque}"
        )
    if opaque_marker.exists():
        raise WorkSlotJourneyFailure("opaque assignment selection started its worker")

    # Argv that resembles Loop Engine fan-out behind another executable is
    # still opaque: only the actual engine command is known to enforce the
    # assignment_selection packet.
    spoof_script = work_dir / "spoof-fan-out.py"
    spoof_script.write_text(
        "#!/usr/bin/env python3\n"
        "import json, subprocess, sys\n"
        "workers=[]\n"
        "for i, arg in enumerate(sys.argv):\n"
        "    if arg == '--worker' and i + 1 < len(sys.argv): workers.append(json.loads(sys.argv[i+1]))\n"
        "payload=sys.stdin.buffer.read()\n"
        "for worker in workers: subprocess.run([worker['command'], *worker['args']], input=payload, check=False)\n",
        encoding="utf-8",
    )
    spoof_script.chmod(0o755)
    spoof_zero = work_dir / "spoof-worker-0.started"
    spoof_one = work_dir / "spoof-worker-1.started"
    spoof_binding = append_fan_out_workers(
        {"command": str(spoof_script), "args": ["fan-out"]},
        [
            {"command": sys.executable, "args": [str(worker_script), str(spoof_zero)]},
            {"command": sys.executable, "args": [str(worker_script), str(spoof_one)]},
        ],
    )
    spoof_call, _, _ = _start_isolated_software_change(
        engine=engine,
        provider=provider,
        profile_source=profile_source,
        fixture_root=fixture_root,
        work_dir=work_dir / "spoof-run",
        run_id="invoke-spoof-selection",
        extra_bindings={"intent-draft": spoof_binding},
    )
    spoof = spoof_call(
        ["invoke", "invoke-spoof-selection", "intent-draft", "--assignment", "worker-1"]
    )
    if spoof.get("status") != "rejected" or spoof.get("code") != "assignments-not-enumerable":
        raise WorkSlotJourneyFailure(
            f"selection on fan-out-shaped opaque command was not refused: {spoof}"
        )
    if spoof_zero.exists() or spoof_one.exists():
        raise WorkSlotJourneyFailure("fan-out-shaped opaque command started a worker")

    return [
        "empty, unknown, and duplicate assignment selections refuse before spawn",
        "selection on plain and fan-out-shaped non-enumerable bindings refuses before spawn",
        "only the selected enumerable assignment started",
        "assignment selection is durable and frozen binding is unchanged",
    ]


def prove_selected_attempt_ledger_linkage(
    *,
    engine: Path,
    provider: Path,
    profile_source: Path,
    fixture_root: Path,
    work_dir: Path,
) -> list[str]:
    """Prove a selected retry attempt can be linked by a public ledger gate."""
    work_dir.mkdir(parents=True, exist_ok=True)
    worker_script = work_dir / "selected-attempt-worker.py"
    worker_script.write_text(
        "import argparse, json, sys\n"
        "from pathlib import Path\n"
        "parser = argparse.ArgumentParser()\n"
        "parser.add_argument('--state', required=True)\n"
        "args = parser.parse_args()\n"
        "state = Path(args.state)\n"
        "count = int(state.read_text()) if state.exists() else 0\n"
        "state.write_text(str(count + 1))\n"
        "sys.stdin.buffer.read()\n"
        "if count == 0:\n"
        "    sys.stdout.write('{\\\"axis\\\":\\\"wrong\\\"}')\n"
        "else:\n"
        "    sys.stdout.write(json.dumps({'axis':'selected-axis','author':{'name':'selected-worker','kind':'script'},'result':'fail','findings':'selected-attempt finding'}, separators=(',', ':')))\n"
        "",
        encoding="utf-8",
    )
    worker_script.chmod(0o755)
    custom_profile = work_dir / "selected-attempt-profile.json"
    profile = json.loads(profile_source.read_text(encoding="utf-8"))
    policies = profile["review_policies"]
    for gate in list(policies):
        policies[gate] = []
    policies["design-review"] = [
        {
            "id": "selected-axis",
            "description": "Selected attempt linkage proof",
            "example_prompt": "Judge selected-axis only.",
            "required_authors": 1,
        }
    ]
    custom_profile.write_text(json.dumps(profile, indent=2) + "\n", encoding="utf-8")
    schema = {
        "type": "object",
        "required": ["axis", "author", "result", "findings"],
        "properties": {
            "axis": {"const": "selected-axis"},
            "author": {"const": {"name": "selected-worker", "kind": "script"}},
            "result": {"enum": ["pass", "fail"]},
            "findings": {"type": "string"},
        },
        "additionalProperties": False,
    }
    binding = fan_out_binding(
        engine=engine,
        workers=[
            {
                "command": sys.executable,
                "args": [str(worker_script), "--state", str(work_dir / "attempt-count")],
                "preamble": "selected-attempt reviewer",
                "full_output_schema": schema,
            }
        ],
    )
    engine_call, artifact_root, frozen_profile = _start_isolated_software_change(
        engine=engine,
        provider=provider,
        profile_source=custom_profile,
        fixture_root=fixture_root,
        work_dir=work_dir / "run",
        run_id="selected-attempt-ledger",
        extra_bindings={"design-review": binding},
    )
    run_id = "selected-attempt-ledger"
    baseline_subject_revision = json.loads((artifact_root / "design.json").read_text(encoding="utf-8"))["revision"]
    baseline_ledger = {
        "schema_version": "1",
        "gate": "design-review",
        "subject": "design.json",
        "subject_revision": baseline_subject_revision,
        "author": {"name": "selected-driver", "kind": "agent"},
        "findings": [],
    }
    baseline_ledger_result = engine_call(
        [
            "append",
            "--record-id=selected-attempt-ledger-baseline",
            run_id,
            "finding-ledger",
            json.dumps(baseline_ledger, separators=(",", ":")),
        ]
    )
    if baseline_ledger_result.get("status") != "completed":
        raise WorkSlotJourneyFailure(f"baseline finding ledger append failed: {baseline_ledger_result}")
    invoke_until_succeeded(engine_call, run_id, "intent-draft", timeout_s=20.0)
    _expect_event_state(engine_call, run_id, "intent-ready", "design")
    _expect_event_state(engine_call, run_id, "design-ready", "design-review")
    overlay = invoke_until_succeeded(engine_call, run_id, "design-review", timeout_s=20.0)
    capture = Path(overlay["capture_dir"])
    shown_workers = overlay.get("inner_workers")
    if not isinstance(shown_workers, list) or len(shown_workers) != 1:
        raise WorkSlotJourneyFailure(
            f"completed invocation omitted its durable worker view: {overlay}"
        )
    shown_worker = shown_workers[0]
    selected_attempt_path = capture / "0" / "attempts" / "2" / "stdout"
    expected_digest = "sha256:" + hashlib.sha256(selected_attempt_path.read_bytes()).hexdigest()
    if (
        shown_worker.get("assignment_id") != "worker-0"
        or shown_worker.get("selected_attempt") != 2
        or shown_worker.get("selected_output_sha256") != expected_digest
        or shown_worker.get("selected_output_path") != str(selected_attempt_path)
    ):
        raise WorkSlotJourneyFailure(
            f"show did not expose the originating selected attempt: {shown_worker}"
        )
    manifest = json.loads((capture / "0" / "attempts.json").read_text(encoding="utf-8"))
    if manifest.get("selected_attempt") != 2 or manifest.get("exhausted") is not False:
        raise WorkSlotJourneyFailure(
            f"selected-attempt fixture did not select retry attempt 2: {manifest}"
        )
    selected_stdout = (capture / "0" / "attempts" / "2" / "stdout").read_bytes()
    output_digest = "sha256:" + hashlib.sha256(selected_stdout).hexdigest()
    selected = json.loads(selected_stdout)
    if selected.get("result") != "fail" or selected.get("findings") != "selected-attempt finding":
        raise WorkSlotJourneyFailure(
            f"selected attempt did not contain the expected candidate finding: {selected}"
        )
    # bookends:LE-100 — repeated public show projects the durable fail-closed change report without provider/capture reads.
    invocation_report = overlay.get("change_report")
    if not isinstance(invocation_report, dict):
        raise WorkSlotJourneyFailure(f"show omitted durable change report: {overlay}")
    dimensions = invocation_report.get("dimensions")
    if not isinstance(dimensions, dict) or not {
        "subject_bytes",
        "worker_assignment",
        "frozen_binding",
        "governing_policy_configuration",
        "declared_output_contract",
        "routed_inputs",
    }.issubset(dimensions):
        raise WorkSlotJourneyFailure(f"judgment change report omitted covered dimensions: {invocation_report}")
    repeated = engine_call(["show", run_id])
    repeated_invocation = next(
        item for item in repeated.get("result", {}).get("work_slot_invocations", [])
        if item.get("invocation_id") == overlay.get("invocation_id")
    )
    repeated_report = repeated_invocation.get("change_report")
    if repeated_report != invocation_report:
        raise WorkSlotJourneyFailure("repeated show changed the durable change report")

    subject = "design.json"
    revision = json.loads((artifact_root / subject).read_text(encoding="utf-8"))["revision"]
    selected_origin = {
        "kind": "selected-assignment-output",
        "id": overlay["invocation_id"],
        "assignment_id": shown_worker["assignment_id"],
    }
    evidence = {
        "gate": "design-review",
        "policy_id": "selected-axis",
        "result": "fail",
        "findings": "selected-attempt finding",
        "author": {"name": "selected-worker", "kind": "script"},
        "subject": subject,
        "subject_revision": revision,
        "config_version": frozen_profile["config_version"],
        "origin": selected_origin,
    }
    # bookends:LE-105 — append accepts only the concise invocation/assignment
    # origin; core resolves the selected attempt and provider evaluation proves
    # that raw selected bytes cannot bless disagreeing judgment content.
    fabricated = dict(evidence)
    fabricated["origin"] = dict(selected_origin)
    fabricated["origin"]["assignment_id"] = "not-selected"
    fabricated_result = engine_call(
        [
            "append",
            "--record-id=selected-attempt-evidence-fabricated",
            run_id,
            "review-evidence",
            json.dumps(fabricated, separators=(",", ":")),
        ]
    )
    if (
        fabricated_result.get("status") != "rejected"
        or fabricated_result.get("code") != "selected-output-linkage-refused"
    ):
        raise WorkSlotJourneyFailure(
            f"append accepted fabricated selected-output identity: {fabricated_result}"
        )
    mismatched = dict(evidence)
    mismatched["result"] = "pass"
    mismatched["findings"] = ""
    mismatch_result = engine_call(
        [
            "append",
            "--record-id=selected-attempt-evidence-mismatch",
            run_id,
            "review-evidence",
            json.dumps(mismatched, separators=(",", ":")),
        ]
    )
    if mismatch_result.get("status") != "completed":
        raise WorkSlotJourneyFailure(f"mismatched linked evidence append failed: {mismatch_result}")
    mismatch_denied = engine_call(["event", run_id, "approved"])
    if mismatch_denied.get("status") != "rejected" or "unverified" not in json.dumps(mismatch_denied):
        raise WorkSlotJourneyFailure(
            f"provider accepted disagreement with originating judgment: {mismatch_denied}"
        )
    evidence_result = engine_call(
        [
            "append",
            "--record-id=selected-attempt-evidence-fail",
            run_id,
            "review-evidence",
            json.dumps(evidence, separators=(",", ":")),
        ]
    )
    if evidence_result.get("status") != "completed":
        raise WorkSlotJourneyFailure(f"selected-attempt fail evidence append failed: {evidence_result}")
    finding = {
        "id": "F-selected-attempt",
        "source": {
            "kind": "context-record",
            "id": "selected-attempt-evidence-fail",
        },
        "policy_id": "selected-axis",
        "statement": "selected-attempt finding",
        "disposition": "accepted",
        "reason": "driver retained and routed the selected reviewer output",
        "owner_phase": "design",
        "task_ids": [],
        "review_axes": ["selected-axis"],
        "status": "unresolved",
    }
    ledger = {
        "schema_version": "1",
        "gate": "design-review",
        "subject": subject,
        "subject_revision": revision,
        "author": {"name": "selected-driver", "kind": "agent"},
        "findings": [finding],
    }
    ledger_result = engine_call(
        [
            "append",
            "--record-id=selected-attempt-ledger-fail",
            run_id,
            "finding-ledger",
            json.dumps(ledger, separators=(",", ":")),
        ]
    )
    if ledger_result.get("status") != "completed":
        raise WorkSlotJourneyFailure(
            f"selected-attempt ledger append failed: {ledger_result}"
        )
    # bookends:LE-92 — a public provider denial reaches evidence only after the driver ledger validates the selected retry attempt through its immutable context-record source.
    denied = engine_call(["event", run_id, "approved"])
    if denied.get("status") != "rejected" or denied.get("code") != "software-change-review-incomplete":
        raise WorkSlotJourneyFailure(
            f"selected-attempt ledger linkage did not reach review evidence: {denied}"
        )
    failure_show = engine_call(["show", run_id])
    failure_context = failure_show.get("result", {}).get("context", [])
    failure_ledger = next(
        (
            record
            for record in failure_context
            if isinstance(record, dict)
            and record.get("id") == "selected-attempt-ledger-fail"
        ),
        None,
    )
    if failure_ledger is None:
        raise WorkSlotJourneyFailure("fresh show omitted the current finding ledger")
    findings = failure_ledger.get("data", {}).get("findings", [])
    if (
        not isinstance(findings, list)
        or len(findings) != 1
        or findings[0].get("source")
        != {"kind": "context-record", "id": "selected-attempt-evidence-fail"}
    ):
        raise WorkSlotJourneyFailure(
            f"fresh show did not expose the stable finding source: {failure_ledger}"
        )
    selected_evidence = next(
        (
            record
            for record in failure_context
            if isinstance(record, dict)
            and record.get("id") == "selected-attempt-evidence-fail"
        ),
        None,
    )
    selected_data = selected_evidence.get("data", {}) if selected_evidence else {}
    resolved_origin = selected_data.get("loop_engine_origin", {})
    if (
        selected_evidence is None
        or selected_data.get("origin") != selected_origin
        or not isinstance(resolved_origin, dict)
        or resolved_origin.get("selected_attempt") != 2
        or resolved_origin.get("selected_output_path") != str(selected_attempt_path)
        or resolved_origin.get("selected_output_sha256") != output_digest
    ):
        raise WorkSlotJourneyFailure(
            f"fresh show did not retain the engine-resolved selected capture origin: {selected_evidence}"
        )
    expected_evidence_fields = {
        "gate",
        "policy_id",
        "result",
        "findings",
        "author",
        "subject",
        "subject_revision",
        "config_version",
        "origin",
        "loop_engine_origin",
    }
    if set(selected_data) != expected_evidence_fields:
        raise WorkSlotJourneyFailure(
            f"fresh evidence duplicated or omitted mechanical provenance: {selected_evidence}"
        )
    evidence["result"] = "pass"
    evidence["findings"] = ""
    evidence.pop("origin")
    pass_result = engine_call(
        [
            "append",
            "--record-id=selected-attempt-evidence-pass",
            run_id,
            "review-evidence",
            json.dumps(evidence, separators=(",", ":")),
        ]
    )
    if pass_result.get("status") != "completed":
        raise WorkSlotJourneyFailure(f"selected-attempt pass evidence append failed: {pass_result}")
    finding["status"] = "resolved"
    ledger["findings"] = [finding]
    resolved_result = engine_call(
        [
            "append",
            "--record-id=selected-attempt-ledger-resolved",
            run_id,
            "finding-ledger",
            json.dumps(ledger, separators=(",", ":")),
        ]
    )
    if resolved_result.get("status") != "completed":
        raise WorkSlotJourneyFailure(
            f"selected-attempt resolved ledger append failed: {resolved_result}"
        )
    # Stable invocation/assignment references are resolved by core before any
    # context record is written. Exercise missing, wrong-run, and unknown
    # references as public append refusals.
    for label, origin in (
        (
            "missing-invocation",
            {"kind": "selected-assignment-output", "id": "missing", "assignment_id": "worker-0"},
        ),
        (
            "missing-assignment",
            {"kind": "selected-assignment-output", "id": overlay["invocation_id"]},
        ),
        (
            "unknown-assignment",
            {
                "kind": "selected-assignment-output",
                "id": overlay["invocation_id"],
                "assignment_id": "missing-assignment",
            },
        ),
    ):
        invalid = dict(evidence)
        invalid["origin"] = origin
        refused = engine_call(
            [
                "append",
                f"--record-id=selected-attempt-{label}",
                run_id,
                "review-evidence",
                json.dumps(invalid, separators=(",", ":")),
            ]
        )
        if refused.get("status") != "rejected" or refused.get("code") != "selected-output-linkage-refused":
            raise WorkSlotJourneyFailure(f"{label} reference was not refused: {refused}")

    foreign_call, _, _ = _start_isolated_software_change(
        engine=engine,
        provider=provider,
        profile_source=custom_profile,
        fixture_root=fixture_root,
        work_dir=work_dir / "foreign-run",
        run_id="selected-attempt-foreign-run",
        extra_bindings={"design-review": binding},
    )
    foreign_overlay = invoke_until_succeeded(
        foreign_call, "selected-attempt-foreign-run", "intent-draft", timeout_s=20.0
    )
    wrong_run = dict(evidence)
    wrong_run["origin"] = {
        "kind": "selected-assignment-output",
        "id": foreign_overlay["invocation_id"],
        "assignment_id": "worker-0",
    }
    wrong_run_result = engine_call(
        [
            "append",
            "--record-id=selected-attempt-wrong-run",
            run_id,
            "review-evidence",
            json.dumps(wrong_run, separators=(",", ":")),
        ]
    )
    if (
        wrong_run_result.get("status") != "rejected"
        or wrong_run_result.get("code") != "selected-output-linkage-refused"
        or "not found on run" not in json.dumps(wrong_run_result)
    ):
        raise WorkSlotJourneyFailure(f"cross-run origin was not refused: {wrong_run_result}")

    # Provider identity checks remain distinct from core's same-run locator
    # check. These malformed applicability targets are durable denials until a
    # later conforming declaration is appended.
    for label, target in (
        (
            "stale-subject",
            {"subject": "plan.json", "revision": revision, "checkpoint": None},
        ),
        (
            "stale-revision",
            {"subject": subject, "revision": "stale-revision", "checkpoint": None},
        ),
        (
            "stale-checkpoint",
            {
                "subject": subject,
                "revision": revision,
                "checkpoint": {"phase": "design", "report_revision": "stale"},
            },
        ),
    ):
        invalid = {
            "origin": {"kind": "context-record", "id": "selected-attempt-evidence-pass"},
            "target": target,
            "attesting_driver": {"name": "selected-driver", "kind": "agent"},
            "reason": f"negative {label} reference",
        }
        appended = engine_call(
            [
                "append",
                f"--record-id=selected-attempt-{label}",
                run_id,
                "evidence-applicability",
                json.dumps(invalid, separators=(",", ":")),
            ]
        )
        if appended.get("status") != "completed":
            raise WorkSlotJourneyFailure(f"{label} applicability append failed: {appended}")
        denied = engine_call(["event", run_id, "approved"])
        if denied.get("status") != "rejected" or "evidence-applicability" not in json.dumps(denied):
            raise WorkSlotJourneyFailure(f"{label} applicability was not denied: {denied}")

    # A later conforming declaration clears the mechanical applicability
    # blockers. The driver still owns the semantic reason; the provider only
    # checks the named source and current target.
    applicability_reset = {
        "origin": {"kind": "context-record", "id": "selected-attempt-evidence-pass"},
        "target": {"subject": subject, "revision": revision, "checkpoint": None},
        "attesting_driver": {"name": "selected-driver", "kind": "agent"},
        "reason": "The original pass remains applicable to the current design.",
    }
    reset = engine_call(
        [
            "append",
            "--record-id=selected-attempt-applicability-reset",
            run_id,
            "evidence-applicability",
            json.dumps(applicability_reset, separators=(",", ":")),
        ]
    )
    if reset.get("status") != "completed":
        raise WorkSlotJourneyFailure(f"applicability reset append failed: {reset}")

    # The provider rejects current-looking ledgers that name missing policy,
    # task, or source identities before a worker can be selected. Restore the
    # empty current ledger after each negative record.
    negative_findings = (
        (
            "unknown-policy",
            {
                "id": "F-unknown-policy",
                "source": {"kind": "context-record", "id": "selected-attempt-evidence-fail"},
                "policy_id": "missing-policy",
                "statement": "selected-attempt finding",
                "disposition": "rejected",
                "reason": "negative policy reference",
                "owner_phase": None,
                "task_ids": [],
                "review_axes": [],
                "status": "recorded",
            },
            "not configured",
        ),
        (
            "unknown-task",
            {
                "id": "F-unknown-task",
                "source": {"kind": "context-record", "id": "selected-attempt-evidence-fail"},
                "policy_id": "selected-axis",
                "statement": "selected-attempt finding",
                "disposition": "rejected",
                "reason": "negative task reference",
                "owner_phase": None,
                "task_ids": ["missing-task"],
                "review_axes": [],
                "status": "recorded",
            },
            "missing-task",
        ),
        (
            "missing-finding",
            {
                "id": "F-missing-source",
                "source": {"kind": "context-record", "id": "missing-evidence"},
                "policy_id": "selected-axis",
                "statement": "selected-attempt finding",
                "disposition": "rejected",
                "reason": "negative finding reference",
                "owner_phase": None,
                "task_ids": [],
                "review_axes": [],
                "status": "recorded",
            },
            "missing-evidence",
        ),
    )
    for label, negative_finding, needle in negative_findings:
        invalid_ledger = dict(baseline_ledger)
        invalid_ledger["findings"] = [negative_finding]
        appended = engine_call(
            [
                "append",
                f"--record-id=selected-attempt-{label}-ledger",
                run_id,
                "finding-ledger",
                json.dumps(invalid_ledger, separators=(",", ":")),
            ]
        )
        if appended.get("status") != "completed":
            raise WorkSlotJourneyFailure(f"{label} ledger append failed: {appended}")
        denied = engine_call(["event", run_id, "approved"])
        if (
            denied.get("status") != "rejected"
            or denied.get("code") != "software-change-finding-ledger-invalid"
            or needle not in json.dumps(denied)
        ):
            raise WorkSlotJourneyFailure(f"{label} ledger reference was not denied: {denied}")
        restored = engine_call(
            [
                "append",
                f"--record-id=selected-attempt-{label}-ledger-restored",
                run_id,
                "finding-ledger",
                json.dumps(baseline_ledger, separators=(",", ":")),
            ]
        )
        if restored.get("status") != "completed":
            raise WorkSlotJourneyFailure(f"{label} ledger restore failed: {restored}")

    # A fresh actor can reuse the original pass judgment with one concise,
    # current applicability declaration. The declaration carries no command,
    # binding, path, digest, attempt, coverage, or changed-input inventory.
    applicability = {
        "origin": {
            "kind": "context-record",
            "id": "selected-attempt-evidence-pass",
        },
        "target": {
            "subject": subject,
            "revision": revision,
            "checkpoint": None,
        },
        "attesting_driver": {"name": "selected-driver", "kind": "agent"},
        "reason": "The original pass remains applicable to the current design.",
    }
    applicability_result = engine_call(
        [
            "append",
            "--record-id=selected-attempt-applicability",
            run_id,
            "evidence-applicability",
            json.dumps(applicability, separators=(",", ":")),
        ]
    )
    if applicability_result.get("status") != "completed":
        raise WorkSlotJourneyFailure(
            f"current applicability declaration was not appended: {applicability_result}"
        )
    resumed = engine_call(["show", run_id])
    contexts = resumed.get("result", {}).get("context", [])
    applicability_record = next(
        (
            record
            for record in contexts
            if isinstance(record, dict)
            and record.get("id") == "selected-attempt-applicability"
        ),
        None,
    )
    if applicability_record is None:
        raise WorkSlotJourneyFailure(
            "fresh show omitted the evidence-applicability declaration"
        )
    if applicability_record.get("data") != applicability:
        raise WorkSlotJourneyFailure(
            f"applicability declaration was rewritten: {applicability_record}"
        )
    if any(
        field in applicability_record.get("data", {})
        for field in (
            "command",
            "args",
            "binding",
            "selected_attempt",
            "selected_output_sha256",
            "selected_output_path",
            "capture_dir",
            "overridden_inputs",
            "changed_inputs",
        )
    ):
        raise WorkSlotJourneyFailure(
            f"applicability declaration duplicated mechanical provenance: {applicability_record}"
        )
    source_record = next(
        (
            record
            for record in contexts
            if isinstance(record, dict)
            and record.get("id") == "selected-attempt-evidence-pass"
        ),
        None,
    )
    if source_record is None or source_record.get("kind") != "review-evidence":
        raise WorkSlotJourneyFailure(
            "fresh show omitted the immutable evidence source for applicability"
        )
    approved = engine_call(["event", run_id, "approved"])
    if approved.get("status") != "completed" or approved.get("result", {}).get("run", {}).get("current_state") != "plan":
        raise WorkSlotJourneyFailure(
            f"selected-attempt ledger and applicability recovery did not advance: {approved}"
        )

    return [
        "show exposes rich engine-owned assignment, attempt, capture, and output identity",
        "concise invocation/assignment evidence links selected raw output without duplicated coordinates",
        "finding-ledger source uses an immutable context-record reference",
        "one current evidence-applicability declaration preserves the original judgment and driver attestation",
        "fresh show resumes the run with original failure, capture, context, and history visible",
        "no live model",
    ]


def prove_subset_applicability_checked(
    *,
    engine: Path,
    provider: Path,
    profile_source: Path,
    fixture_root: Path,
    work_dir: Path,
) -> list[str]:
    """Prove subset re-execution plus one current applicability declaration."""
    work_dir.mkdir(parents=True, exist_ok=True)
    worker_script = work_dir / "subset-applicability-worker.py"
    worker_script.write_text(
        "import argparse, json, sys\n"
        "from pathlib import Path\n"
        "parser = argparse.ArgumentParser()\n"
        "parser.add_argument('--author', required=True)\n"
        "parser.add_argument('--state', required=True)\n"
        "args = parser.parse_args()\n"
        "state = Path(args.state)\n"
        "count = int(state.read_text()) if state.exists() else 0\n"
        "state.write_text(str(count + 1))\n"
        "sys.stdin.buffer.read()\n"
        "print(json.dumps({'axis':'subset-applicability-axis','author':{'name':args.author,'kind':'script'},'result':'pass','findings':''}, separators=(',', ':')))\n",
        encoding="utf-8",
    )
    worker_script.chmod(0o755)
    custom_profile = work_dir / "subset-applicability-profile.json"
    profile = json.loads(profile_source.read_text(encoding="utf-8"))
    policies = profile["review_policies"]
    for gate in list(policies):
        policies[gate] = []
    policies["design-review"] = [
        {
            "id": "subset-applicability-axis",
            "description": "Subset applicability checked transition proof",
            "example_prompt": "Judge subset-applicability-axis only.",
            "required_authors": 2,
        }
    ]
    custom_profile.write_text(json.dumps(profile, indent=2) + "\n", encoding="utf-8")
    schema = {
        "type": "object",
        "required": ["axis", "author", "result", "findings"],
        "properties": {
            "axis": {"const": "subset-applicability-axis"},
            "author": {"type": "object"},
            "result": {"const": "pass"},
            "findings": {"const": ""},
        },
        "additionalProperties": False,
    }
    states = {index: work_dir / f"worker-{index}.count" for index in (0, 1)}
    binding = fan_out_binding(
        engine=engine,
        workers=[
            {
                "command": sys.executable,
                "args": [
                    str(worker_script),
                    "--author",
                    f"subset-worker-{index}",
                    "--state",
                    str(states[index]),
                ],
                "preamble": f"subset-applicability worker {index}",
                "full_output_schema": schema,
            }
            for index in (0, 1)
        ],
    )
    engine_call, artifact_root, frozen_profile = _start_isolated_software_change(
        engine=engine,
        provider=provider,
        profile_source=custom_profile,
        fixture_root=fixture_root,
        work_dir=work_dir / "run",
        run_id="subset-applicability-checked",
        extra_bindings={"design-review": binding},
    )
    run_id = "subset-applicability-checked"
    invoke_until_succeeded(engine_call, run_id, "intent-draft", timeout_s=20.0)
    _expect_event_state(engine_call, run_id, "intent-ready", "design")
    _expect_event_state(engine_call, run_id, "design-ready", "design-review")
    subject = "design.json"
    revision = json.loads((artifact_root / subject).read_text(encoding="utf-8"))["revision"]
    ledger = {
        "schema_version": "1",
        "gate": "design-review",
        "subject": subject,
        "subject_revision": revision,
        "author": {"name": "subset-driver", "kind": "agent"},
        "findings": [],
    }
    ledger_result = engine_call(
        [
            "append",
            "--record-id=subset-applicability-ledger",
            run_id,
            "finding-ledger",
            json.dumps(ledger, separators=(",", ":")),
        ]
    )
    if ledger_result.get("status") != "completed":
        raise WorkSlotJourneyFailure(f"clean finding-ledger append failed: {ledger_result}")

    full_overlay = invoke_until_succeeded(engine_call, run_id, "design-review", timeout_s=20.0)
    full_workers = full_overlay.get("inner_workers")
    if not isinstance(full_workers, list) or [worker.get("assignment_id") for worker in full_workers] != ["worker-0", "worker-1"]:
        raise WorkSlotJourneyFailure(f"full overlay did not enumerate both assignments: {full_overlay}")
    if any(states[index].read_text() != "1" for index in (0, 1)):
        raise WorkSlotJourneyFailure("full overlay did not start both dummy assignments exactly once")
    config_version = frozen_profile["config_version"]
    source_records: dict[str, str] = {}
    for worker in full_workers:
        assignment = worker["assignment_id"]
        output_path = Path(worker["selected_output_path"])
        output_digest = "sha256:" + hashlib.sha256(output_path.read_bytes()).hexdigest()
        author = f"subset-{assignment}"
        if worker.get("selected_attempt") != 1 or worker.get("selected_output_sha256") != output_digest:
            raise WorkSlotJourneyFailure(f"full overlay omitted selected output evidence: {worker}")
        evidence = {
            "gate": "design-review",
            "policy_id": "subset-applicability-axis",
            "result": "pass",
            "findings": "",
            "author": {"name": author, "kind": "script"},
            "subject": subject,
            "subject_revision": revision,
            "config_version": config_version,
            "origin": {
                "kind": "selected-assignment-output",
                "id": full_overlay["invocation_id"],
                "assignment_id": assignment,
            },
        }
        record_id = f"subset-applicability-source-{assignment}"
        appended = engine_call(
            [
                "append",
                f"--record-id={record_id}",
                run_id,
                "review-evidence",
                json.dumps(evidence, separators=(",", ":")),
            ]
        )
        if appended.get("status") != "completed":
            raise WorkSlotJourneyFailure(f"source evidence append failed for {assignment}: {appended}")
        source_records[assignment] = record_id

    subset = invoke_until_succeeded(
        engine_call,
        run_id,
        "design-review",
        timeout_s=20.0,
        invoke_args=("--assignment", "worker-0"),
    )
    subset_workers = subset.get("inner_workers")
    if not isinstance(subset_workers, list) or [worker.get("assignment_id") for worker in subset_workers] != ["worker-0"]:
        raise WorkSlotJourneyFailure(f"subset re-invoke did not select only worker-0: {subset}")
    counts = {index: int(states[index].read_text()) for index in (0, 1)}
    if counts != {0: 2, 1: 1}:
        raise WorkSlotJourneyFailure(f"subset re-invoke changed assignment counts unexpectedly: {counts}")

    subset_worker = subset_workers[0]
    subset_output_path = Path(subset_worker["selected_output_path"])
    subset_output_digest = "sha256:" + hashlib.sha256(subset_output_path.read_bytes()).hexdigest()
    if (
        subset_worker.get("selected_attempt") != 1
        or subset_worker.get("selected_output_sha256") != subset_output_digest
        or subset_output_path == Path(full_workers[0]["selected_output_path"])
    ):
        raise WorkSlotJourneyFailure(f"subset overlay omitted fresh worker-0 output evidence: {subset_worker}")
    subset_evidence = {
        "gate": "design-review",
        "policy_id": "subset-applicability-axis",
        "result": "pass",
        "findings": "",
        "author": {"name": "subset-worker-0", "kind": "script"},
        "subject": subject,
        "subject_revision": revision,
        "config_version": config_version,
        "origin": {
            "kind": "selected-assignment-output",
            "id": subset["invocation_id"],
            "assignment_id": subset_worker["assignment_id"],
        },
    }
    subset_evidence_result = engine_call(
        [
            "append",
            "--record-id=subset-applicability-fresh-worker-0",
            run_id,
            "review-evidence",
            json.dumps(subset_evidence, separators=(",", ":")),
        ]
    )
    if subset_evidence_result.get("status") != "completed":
        raise WorkSlotJourneyFailure(
            f"subset worker-0 evidence append failed: {subset_evidence_result}"
        )

    applicability = {
        "origin": {
            "kind": "context-record",
            "id": source_records["worker-1"],
        },
        "target": {"subject": subject, "revision": revision, "checkpoint": None},
        "attesting_driver": {"name": "subset-driver", "kind": "agent"},
        "reason": "The unchanged worker-1 judgment remains applicable to this review.",
    }
    applicability_result = engine_call(
        [
            "append",
            "--record-id=subset-applicability-declaration",
            run_id,
            "evidence-applicability",
            json.dumps(applicability, separators=(",", ":")),
        ]
    )
    if applicability_result.get("status") != "completed":
        raise WorkSlotJourneyFailure(f"applicability append failed: {applicability_result}")

    shown = engine_call(["show", run_id])
    context = shown.get("result", {}).get("context", [])
    declaration = next(
        (
            record
            for record in context
            if isinstance(record, dict)
            and record.get("id") == "subset-applicability-declaration"
        ),
        None,
    )
    if declaration is None or declaration.get("data") != applicability:
        raise WorkSlotJourneyFailure(f"show did not preserve applicability declaration: {shown}")
    if any(
        key in declaration.get("data", {})
        for key in (
            "command",
            "args",
            "binding",
            "selected_attempt",
            "selected_output_sha256",
            "selected_output_path",
            "capture_dir",
            "overridden_inputs",
            "changed_inputs",
        )
    ):
        raise WorkSlotJourneyFailure(f"applicability duplicated mechanical provenance: {declaration}")
    source_record = next(
        (
            record
            for record in context
            if isinstance(record, dict)
            and record.get("id") == source_records["worker-1"]
        ),
        None,
    )
    if source_record is None or source_record.get("kind") != "review-evidence":
        raise WorkSlotJourneyFailure("show omitted the immutable applicability source record")

    checked = engine_call(["event", run_id, "approved"])
    checked_run = (checked.get("result") or {}).get("run") or {}
    if checked.get("status") != "completed" or checked_run.get("current_state") != "plan":
        raise WorkSlotJourneyFailure(f"applicable evidence did not satisfy checked transition: {checked}")
    return [
        "full two-assignment overlay records selected outputs under engine-owned captures",
        "subset re-invoke starts worker-0 twice and never starts unselected worker-1",
        "fresh evidence names only invocation and assignment while engine resolves output metadata",
        "one evidence-applicability declaration preserves the worker-1 judgment for the current target",
        "checked approval counts fresh and applicable independent authors",
        "show keeps source evidence, applicability, and immutable history visible",
        "no live model",
    ]

def prove_stdin_exec(*, provider: Path, work_dir: Path) -> list[str]:
    """Drive the provider's hidden stdin-exec helper as a real CLI."""
    work_dir.mkdir(parents=True, exist_ok=True)
    stdin_file = work_dir / "duty.json"
    duty = b'{"duty":"bytes stay in the stdin file"}\n'
    stdin_file.write_bytes(duty)
    receipt = work_dir / "child.stdin"
    session_receipt = work_dir / "child.session-dir"
    sidecar = work_dir / "sidecar.json"
    child = (
        "import os,sys; from pathlib import Path; "
        f"Path({str(receipt)!r}).write_bytes(sys.stdin.buffer.read()); "
        f"Path({str(session_receipt)!r}).write_text(os.environ.get('PI_CODING_AGENT_SESSION_DIR', '')); "
        "raise SystemExit(7)"
    )
    clean_env = os.environ.copy()
    clean_env.pop("PI_CODING_AGENT_SESSION_DIR", None)
    sidecar_run = subprocess.run(
        [
            str(provider),
            "stdin-exec",
            "--stdin-file",
            str(stdin_file),
            "--exit-mode",
            "sidecar",
            "--sidecar-file",
            str(sidecar),
            "--",
            sys.executable,
            "-c",
            child,
        ],
        input=b"not-duty-argv-or-stdin",
        capture_output=True,
        check=False,
        cwd=work_dir,
        env=clean_env,
    )
    if sidecar_run.returncode != 0:
        raise WorkSlotJourneyFailure(
            f"stdin-exec sidecar failed: {sidecar_run.returncode}: "
            f"{sidecar_run.stderr!r}"
        )
    if json.loads(sidecar.read_text(encoding="utf-8")) != {"exit_code": 7}:
        raise WorkSlotJourneyFailure("stdin-exec sidecar did not preserve inner waitpid exit")
    if receipt.read_bytes() != duty:
        raise WorkSlotJourneyFailure("stdin-exec did not attach the requested stdin file")
    session_dir = Path(session_receipt.read_text(encoding="utf-8"))
    if session_dir != (stdin_file.parent / "sessions").resolve():
        raise WorkSlotJourneyFailure(
            f"stdin-exec did not colocate the child session directory: {session_dir}"
        )
    if duty.decode("utf-8") in sidecar_run.args:
        raise WorkSlotJourneyFailure("stdin-exec duty bytes appeared in argv")

    propagate = work_dir / "propagate.stdin"
    propagate_run = subprocess.run(
        [
            str(provider),
            "stdin-exec",
            "--stdin-file",
            str(stdin_file),
            "--exit-mode",
            "propagate",
            "--",
            sys.executable,
            "-c",
            child,
        ],
        capture_output=True,
        check=False,
        cwd=work_dir,
        env=clean_env,
    )
    if propagate_run.returncode != 7 or not receipt.is_file():
        raise WorkSlotJourneyFailure(
            f"stdin-exec propagate did not return inner exit 7: {propagate_run.returncode}"
        )
    bad_propagate = subprocess.run(
        [
            str(provider),
            "stdin-exec",
            "--stdin-file",
            str(stdin_file),
            "--exit-mode",
            "propagate",
            "--sidecar-file",
            str(work_dir / "bad-sidecar.json"),
            "--",
            sys.executable,
            "-c",
            "pass",
        ],
        capture_output=True,
        check=False,
        cwd=work_dir,
        env=clean_env,
    )
    if bad_propagate.returncode == 0:
        raise WorkSlotJourneyFailure("stdin-exec propagate accepted --sidecar-file")

    missing_sidecar = work_dir / "missing-sidecar.json"
    missing = subprocess.run(
        [
            str(provider),
            "stdin-exec",
            "--stdin-file",
            str(stdin_file),
            "--exit-mode",
            "sidecar",
            "--sidecar-file",
            str(missing_sidecar),
            "--",
            str(work_dir / "does-not-exist"),
        ],
        capture_output=True,
        check=False,
        cwd=work_dir,
        env=clean_env,
    )
    if missing.returncode == 0 or missing_sidecar.exists():
        raise WorkSlotJourneyFailure("stdin-exec spawn failure produced a successful sidecar")

    help_run = subprocess.run(
        [str(provider), "--help"], capture_output=True, text=True, check=False
    )
    if help_run.returncode != 0 or "stdin-exec" in help_run.stdout:
        raise WorkSlotJourneyFailure("software-change help exposed hidden stdin-exec")
    return [
        "stdin-exec sidecar records inner waitpid exit and file stdin",
        "stdin-exec propagate returns inner exit and rejects sidecar-file",
        "stdin-exec spawn failure has no successful sidecar",
        "stdin-exec colocates session directory without frozen --session-dir",
        "software-change help omits hidden stdin-exec",
        "no live model",
    ]


def invoke_until_status(
    engine_call: EngineCall,
    run_id: str,
    slot_id: str,
    *,
    expected: str,
    timeout_s: float = 15.0,
) -> dict[str, Any]:
    started = engine_call(["invoke", run_id, slot_id])
    if started.get("status") != "completed":
        raise WorkSlotJourneyFailure(f"invoke failed: {started}")
    result = started.get("result") or {}
    invocation_id = result.get("invocation_id")
    if not isinstance(invocation_id, str) or not invocation_id:
        raise WorkSlotJourneyFailure(f"invoke omitted invocation_id: {started}")

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
            last_status = match.get("status")
            if last_status == expected:
                if match.get("slot_id") != slot_id:
                    raise WorkSlotJourneyFailure(f"overlay has wrong slot: {match}")
                return match
            if last_status in ("succeeded", "failed", "overrun") and last_status != expected:
                raise WorkSlotJourneyFailure(
                    f"bound worker ended {last_status}, expected {expected}: {match}"
                )
        time.sleep(0.05)
    raise WorkSlotJourneyFailure(
        f"timed out waiting for {expected} overlay on {slot_id} (last={last_status})"
    )


def _engine_json(
    engine: Path,
    database: Path,
    arguments: Sequence[str],
) -> dict[str, Any]:
    completed = subprocess.run(
        [str(engine), "--database", str(database), "--json", *arguments],
        text=True,
        capture_output=True,
        check=False,
    )
    if not completed.stdout.strip():
        raise WorkSlotJourneyFailure(
            f"engine returned no JSON (exit={completed.returncode}): {completed.stderr}"
        )
    try:
        value = json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        raise WorkSlotJourneyFailure(
            f"engine returned non-JSON (exit={completed.returncode}): {error}"
        ) from error
    if not isinstance(value, dict):
        raise WorkSlotJourneyFailure(f"engine response is not an object: {value}")
    return value


def prove_preview_fail_closed(*, engine: Path, work_dir: Path) -> list[str]:
    """preview-bindings exits nonzero on zero-worker fan-out and creates no run."""
    work_dir.mkdir(parents=True, exist_ok=True)
    database = work_dir / "must-not-exist.sqlite"
    payload = {"design-review": {"command": "loop-engine", "args": ["fan-out"]}}
    before = {path.name for path in work_dir.iterdir()}
    code, report = run_preview_bindings(
        engine,
        payload,
        database=database,
        cwd=work_dir,
    )
    if code == 0:
        raise WorkSlotJourneyFailure(
            f"preview-bindings zero-worker fan-out unexpectedly exited 0: {report}"
        )
    errors = report.get("errors")
    rendered = json.dumps(report)
    if not (
        (isinstance(errors, list) and any("zero --worker" in str(item) for item in errors))
        or "zero --worker" in rendered
    ):
        raise WorkSlotJourneyFailure(
            f"preview-bindings zero-worker report omitted the fail-closed error: {report}"
        )
    if database.exists() or database.with_name(database.name + "-wal").exists():
        raise WorkSlotJourneyFailure(
            f"preview-bindings created a database at {database}"
        )
    created = {path.name for path in work_dir.iterdir()} - before
    if created:
        raise WorkSlotJourneyFailure(
            f"preview-bindings created unexpected paths {sorted(created)}"
        )
    return [
        "preview-bindings exits nonzero on zero-worker fan-out JSON",
        "preview-bindings created no database or run directory",
        "no live model",
    ]


def _preview_warning_text(report: Mapping[str, Any]) -> str:
    warnings = report.get("warnings")
    if isinstance(warnings, list):
        return "\n".join(str(item) for item in warnings)
    return json.dumps(report)


def _fan_out_pi_binding(args: Sequence[str]) -> dict[str, Any]:
    return {
        "design-review": {
            "command": PATH_LOOP_ENGINE,
            "args": [
                "fan-out",
                "--worker",
                json.dumps({"command": "pi", "args": list(args)}),
            ],
        }
    }


def prove_preview_pi_extension_warnings(*, engine: Path, work_dir: Path) -> list[str]:
    """preview-bindings warns on --no-extensions without -e; missing --no-extensions is not required."""
    work_dir.mkdir(parents=True, exist_ok=True)
    missing_e = _fan_out_pi_binding(
        [
            "--print",
            "--no-skills",
            "--no-extensions",
            "--tools",
            "read,grep,find,ls",
            "--model",
            "x",
        ]
    )
    with_e = _fan_out_pi_binding(
        [
            "--print",
            "--no-skills",
            "--no-extensions",
            "-e",
            "/tmp/cursor",
            "-e",
            "/tmp/claude-bridge",
            "--tools",
            "read,grep,find,ls",
            "--model",
            "x",
        ]
    )
    without_no_extensions = _fan_out_pi_binding(
        [
            "--print",
            "--no-skills",
            "--tools",
            "read,grep,find,ls",
            "--model",
            "x",
        ]
    )
    implement_with_e = {
        "implement": {
            "command": PATH_SOFTWARE_CHANGE,
            "args": [
                "run-plan-graph",
                "--working-directory",
                "/tmp",
                "--task-worker",
                json.dumps(
                    {
                        "command": "pi",
                        "args": [
                            "--print",
                            "--no-skills",
                            "--no-extensions",
                            "-e",
                            "/tmp/cursor",
                            "-e",
                            "/tmp/claude-bridge",
                            "--model",
                            "x",
                        ],
                    }
                ),
            ],
        }
    }

    code, report = run_preview_bindings(engine, missing_e, cwd=work_dir)
    if code != 0:
        raise WorkSlotJourneyFailure(
            f"preview-bindings missing -e unexpectedly exited {code}: {report}"
        )
    missing_text = _preview_warning_text(report)
    if PI_NO_EXTENSIONS_WITHOUT_E not in missing_text:
        raise WorkSlotJourneyFailure(
            f"preview-bindings omitted missing-`-e` warning: {report}"
        )

    code, report = run_preview_bindings(engine, with_e, cwd=work_dir)
    if code != 0:
        raise WorkSlotJourneyFailure(
            f"preview-bindings with -e unexpectedly exited {code}: {report}"
        )
    with_e_text = _preview_warning_text(report)
    if PI_NO_EXTENSIONS_WITHOUT_E in with_e_text:
        raise WorkSlotJourneyFailure(
            f"preview-bindings warned missing -e despite -e args: {report}"
        )
    if "lacks --no-extensions" in with_e_text:
        raise WorkSlotJourneyFailure(
            f"preview-bindings warned about missing --no-extensions: {report}"
        )

    code, report = run_preview_bindings(engine, without_no_extensions, cwd=work_dir)
    if code != 0:
        raise WorkSlotJourneyFailure(
            f"preview-bindings without --no-extensions unexpectedly exited {code}: {report}"
        )
    omitted_text = _preview_warning_text(report)
    if PI_NO_EXTENSIONS_WITHOUT_E in omitted_text:
        raise WorkSlotJourneyFailure(
            f"preview-bindings warned missing -e when --no-extensions was absent: {report}"
        )
    if "lacks --no-extensions" in omitted_text:
        raise WorkSlotJourneyFailure(
            f"missing --no-extensions was treated as a required warning: {report}"
        )

    code, report = run_preview_bindings(engine, implement_with_e, cwd=work_dir)
    if code != 0:
        raise WorkSlotJourneyFailure(
            f"preview-bindings opt-in implement with -e exited {code}: {report}"
        )
    implement_text = _preview_warning_text(report)
    if PI_NO_EXTENSIONS_WITHOUT_E in implement_text:
        raise WorkSlotJourneyFailure(
            f"opt-in implement with -e still warned missing -e: {report}"
        )

    return [
        "preview-bindings warns when pi has --no-extensions and no -e",
        "preview-bindings does not require a missing --no-extensions warning",
        "opt-in dummy implement/review bindings may include -e args",
        "no live model",
    ]


def prove_zero_worker_review_invoke(
    *,
    engine: Path,
    provider: Path,
    profile_source: Path,
    fixture_root: Path,
    work_dir: Path,
) -> list[str]:
    """Zero-worker fan-out is fail-closed at preview-bindings, not after start."""
    del provider, profile_source, fixture_root
    return prove_preview_fail_closed(engine=engine, work_dir=work_dir)


def prove_default_sandbox_argv(*, provider: Path, work_dir: Path) -> list[str]:
    """PATH stub named pi records default sandbox argv. No live model."""
    work_dir.mkdir(parents=True, exist_ok=True)
    _ensure_git_repository(work_dir)
    artifact_root = work_dir / "artifacts"
    capture_dir = work_dir / "captures" / "inv-default-pi"
    plan = _write_small_plan(artifact_root)
    task_ids = [task["id"] for task in plan["tasks"]]
    leftover = artifact_root / "implementation-report.json"
    leftover.write_text("{}\n", encoding="utf-8")
    stub_dir = work_dir / "bin"
    log_dir = work_dir / "pi-argv"
    stub = _write_pi_stub(stub_dir)
    env = os.environ.copy()
    env["PATH"] = str(stub_dir) + os.pathsep + env.get("PATH", "")
    env["PI_STUB_LOG_DIR"] = str(log_dir)
    which = shutil.which("pi", path=env["PATH"])
    if which is None or Path(which).resolve() != stub.resolve():
        raise WorkSlotJourneyFailure(
            f"PATH stub pi was not the resolved pi: {which!r} vs {stub}"
        )
    completed = _run_binding(
        {
            "command": str(provider),
            "args": [*SHIPPED_IMPLEMENT_ARGS, "--working-directory", str(work_dir)],
        },
        stdin=_invoke_packet(
            run_id="default-sandbox-argv",
            slot_id="implement",
            artifact_root=artifact_root,
            instruction_body="Use the default inner worker.",
            capture_dir=capture_dir,
        ),
        cwd=work_dir,
        env=env,
    )
    if completed.returncode != 0:
        raise WorkSlotJourneyFailure(
            "default-sandbox run-plan-graph exited "
            f"{completed.returncode}: {completed.stderr.decode('utf-8', 'replace')}"
        )
    logs = sorted(log_dir.glob("*.argv.json"))
    expected_invocations = len(task_ids) + 1  # plan tasks plus summarizer
    if len(logs) != expected_invocations:
        raise WorkSlotJourneyFailure(
            f"PATH stub pi invocations {len(logs)} != tasks+summarizer {expected_invocations}"
        )
    _assert_dummy_report(artifact_root, str(plan["revision"]))
    if leftover.read_text(encoding="utf-8") == "{}\n":
        raise WorkSlotJourneyFailure("default-sandbox leftover empty report still present")
    for path in logs:
        argv = json.loads(path.read_text(encoding="utf-8"))
        if argv != DEFAULT_PI_SANDBOX_ARGS:
            raise WorkSlotJourneyFailure(
                f"PATH stub pi argv {argv} != {DEFAULT_PI_SANDBOX_ARGS}"
            )
        for flag in FORBIDDEN_PI_FLAGS:
            if flag in argv:
                raise WorkSlotJourneyFailure(
                    f"PATH stub pi received forbidden flag {flag}: {argv}"
                )
    workers = _load_summary_workers(capture_dir)
    if [worker.get("command") for worker in workers] != ["pi"] * len(task_ids):
        raise WorkSlotJourneyFailure(f"summary command was not pi: {workers}")
    if any(worker.get("args") != DEFAULT_PI_SANDBOX_ARGS for worker in workers):
        raise WorkSlotJourneyFailure(
            f"summary args were not sandbox defaults: {workers}"
        )
    _assert_capture_files(capture_dir, task_ids)
    return [
        "omitted --task-worker uses PATH stub pi",
        "recorded argv [--print, --no-skills, --no-extensions]",
        "did not receive --no-context-files or --tools",
        "no live model",
    ]


def prove_bound_fan_out_heartbeat(
    *,
    engine: Path,
    provider: Path,
    profile_source: Path,
    fixture_root: Path,
    work_dir: Path,
) -> list[str]:
    """Bound fan-out invoke: show heartbeat, inner nonzero, capture isolation."""
    work_dir.mkdir(parents=True, exist_ok=True)
    run_id = "bound-fan-out-heartbeat"
    extra = {
        "design-review": fan_out_binding(
            engine=engine,
            workers=[
                stdin_worker_cli(work_dir / "inner-fail.stdin", ("--exit", "7")),
                stdin_worker_cli(
                    work_dir / "operating-context.stdin",
                    ("--inspect-operating-context",),
                ),
            ],
        )
    }
    engine_call, artifact_root, profile = _start_isolated_software_change(
        engine=engine,
        provider=provider,
        profile_source=profile_source,
        fixture_root=fixture_root,
        work_dir=work_dir / "run",
        run_id=run_id,
        extra_bindings=extra,
    )
    _advance_software_change_to(
        engine_call,
        run_id=run_id,
        profile=profile,
        artifact_root=artifact_root,
        target="design-review",
    )
    context_response = engine_call(
        [
            "append",
            "--record-id=forwarded-ledger-context",
            run_id,
            "finding-ledger",
            json.dumps(
                {"source": "driver", "proposal": "inert until disposition"},
                separators=(",", ":"),
            ),
        ]
    )
    if context_response.get("status") != "completed":
        raise WorkSlotJourneyFailure(
            f"finding-ledger context append failed before forwarding proof: {context_response}"
        )
    forwarded_record = (context_response.get("result") or {}).get("context")
    if not isinstance(forwarded_record, dict):
        raise WorkSlotJourneyFailure(
            f"append did not return forwarded context record: {context_response}"
        )
    # The fresh reviewer process receives the driver's immutable ledger record,
    # while the worker below reads the frozen operating context from artifact_root.
    first = invoke_until_succeeded(
        engine_call, run_id, "design-review", timeout_s=20.0
    )
    _assert_inner_exit_codes(first, (7, 0))
    _assert_capture_files(Path(first["capture_dir"]), ("0", "1"))
    for receipt in (
        work_dir / "inner-fail.stdin",
        work_dir / "operating-context.stdin",
    ):
        try:
            packet = json.loads(receipt.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as error:
            raise WorkSlotJourneyFailure(
                f"forwarding worker receipt was not JSON: {receipt}: {error}"
            ) from error
        forwarded_context = packet.get("context")
        if (
            not isinstance(forwarded_context, list)
            or forwarded_record not in forwarded_context
            or forwarded_context[-1] != forwarded_record
        ):
            raise WorkSlotJourneyFailure(
                f"worker {receipt} did not receive appended finding-ledger context in order: {packet}"
            )
    try:
        inspected_context = json.loads(
            (work_dir / "operating-context.stdin.operating-context.json").read_text(
                encoding="utf-8"
            )
        )
        intent_context = json.loads(
            (artifact_root / "intent.json").read_text(encoding="utf-8")
        )["operating_context"]
    except (KeyError, OSError, json.JSONDecodeError) as error:
        raise WorkSlotJourneyFailure(
            f"bound reviewer did not inspect frozen operating_context: {error}"
        ) from error
    # bookends:LE-91 — a fresh bound reviewer process reads the frozen operating context from the public artifact root rather than chat state.
    if inspected_context != intent_context:
        raise WorkSlotJourneyFailure(
            "bound reviewer observed operating_context different from intent.json"
        )
    second = invoke_until_succeeded(
        engine_call, run_id, "design-review", timeout_s=20.0
    )
    _assert_inner_exit_codes(second, (7, 0))
    _assert_capture_isolation(first, second, ("0", "1"))
    return [
        "show heartbeat overlay_meaning elapsed_ms remaining_allowed_ms capture_dir inner_workers",
        "finding-ledger context forwards unchanged to a fresh bound worker process",
        "fresh bound reviewer reads frozen operating_context from artifact_root",
        "inner nonzero with collector 0 yields overlay succeeded",
        "second invoke uses a new invocation-id capture_dir and leaves the first intact",
        "no live model",
    ]


def prove_bound_fan_out_overrun(
    *,
    engine: Path,
    provider: Path,
    profile_source: Path,
    fixture_root: Path,
    work_dir: Path,
) -> list[str]:
    """Prove reader overrun, immediate show, and retry admission via public CLI."""
    work_dir.mkdir(parents=True, exist_ok=True)
    run_id = "bound-fan-out-overrun"
    slot_id = "design-review"
    binding = fan_out_binding(
        engine=engine,
        workers=[stdin_worker_cli(work_dir / "overrun.stdin", ("--sleep", "0.5"))],
    )
    engine_call, artifact_root, profile = _start_isolated_software_change(
        engine=engine,
        provider=provider,
        profile_source=profile_source,
        fixture_root=fixture_root,
        work_dir=work_dir / "run",
        run_id=run_id,
        extra_bindings={slot_id: binding},
    )
    _advance_software_change_to(
        engine_call,
        run_id=run_id,
        profile=profile,
        artifact_root=artifact_root,
        target=slot_id,
    )
    database = work_dir / "run" / "loop.sqlite"

    def invoke_with_short_budget() -> dict[str, Any]:
        completed = subprocess.run(
            [
                str(engine),
                "--database",
                str(database),
                "--timeout-ms",
                "20",
                "--json",
                "invoke",
                run_id,
                slot_id,
            ],
            text=True,
            capture_output=True,
            check=False,
        )
        try:
            value = json.loads(completed.stdout)
        except json.JSONDecodeError as error:
            raise WorkSlotJourneyFailure(
                f"overrun invoke returned non-JSON: {error}; stderr={completed.stderr!r}"
            ) from error
        if value.get("status") != "completed":
            raise WorkSlotJourneyFailure(f"short-budget invoke failed: {value}")
        return value

    first_started = invoke_with_short_budget()
    first_id = first_started["result"]["invocation_id"]
    first = _wait_overlay_status(
        engine_call,
        run_id,
        first_id,
        expected="overrun",
        timeout_s=5.0,
    )
    immediately_shown = engine_call(["show", run_id])
    if immediately_shown.get("status") != "completed":
        raise WorkSlotJourneyFailure(f"show before overrun retry failed: {immediately_shown}")
    first_view = next(
        item
        for item in immediately_shown["result"]["work_slot_invocations"]
        if item.get("invocation_id") == first_id
    )
    retry_started = invoke_with_short_budget()
    second_id = retry_started["result"]["invocation_id"]
    second = _wait_overlay_status(
        engine_call,
        run_id,
        second_id,
        expected="overrun",
        timeout_s=5.0,
    )
    if (
        first_view.get("status") != "overrun"
        or first_view.get("overlay_meaning", "").find("run show immediately") < 0
        or second_id == first_id
        or second.get("status") != "overrun"
        or Path(first["capture_dir"]) == Path(second["capture_dir"])
    ):
        raise WorkSlotJourneyFailure(
            f"overrun retry was not inspectable or isolated: first={first_view}; "
            f"second={second}"
        )
    time.sleep(0.7)
    return [
        "overrun remains a reader overlay and is not already-running",
        "driver show occurs immediately before retry",
        "overrun and retry captures remain distinct and inspectable",
        "no live model",
    ]


def prove_bound_contracted_fan_out_failure(
    *,
    engine: Path,
    provider: Path,
    profile_source: Path,
    fixture_root: Path,
    work_dir: Path,
) -> Path:
    """Bound fan-out preserves exit 0 workers while failing nonconformance."""
    work_dir.mkdir(parents=True, exist_ok=True)
    run_id = "bound-contracted-fan-out-failure"
    slot_id = "design-review"
    required = ["axis", "author", "result", "findings"]
    preamble = "Read-only reviewer. Return only the contracted judgment."
    receipt_a = work_dir / "conforming.stdin"
    receipt_b = work_dir / "refusal.stdin"

    conforming = json.dumps(
        {
            "axis": "journey-mechanics",
            "author": {"name": "deterministic-worker", "kind": "script"},
            "result": "pass",
            "findings": "",
        },
        separators=(",", ":"),
    )
    refusal = "I refuse to produce the contracted review judgment."
    extra = {
        slot_id: fan_out_binding(
            engine=engine,
            workers=[
                contracted_stdin_worker_cli(
                    receipt_a, conforming, preamble=preamble, required=required
                ),
                contracted_stdin_worker_cli(
                    receipt_b, refusal, preamble=preamble, required=required
                ),
            ],
        )
    }
    engine_call, artifact_root, profile = _start_isolated_software_change(
        engine=engine,
        provider=provider,
        profile_source=profile_source,
        fixture_root=fixture_root,
        work_dir=work_dir / "run",
        run_id=run_id,
        extra_bindings=extra,
    )
    _advance_software_change_to(
        engine_call,
        run_id=run_id,
        profile=profile,
        artifact_root=artifact_root,
        target="design-review",
    )
    failed = invoke_until_status(
        engine_call,
        run_id,
        "design-review",
        expected="failed",
        timeout_s=20.0,
    )
    if failed.get("overlay_meaning") != OVERLAY_MEANING_FAILED:
        raise WorkSlotJourneyFailure(
            f"contracted fan-out failed overlay meaning mismatch: {failed}"
        )
    collector_exit = failed.get("exit_code")
    capture_raw = failed.get("capture_dir")
    if not isinstance(collector_exit, int) or collector_exit == 0:
        # A waiter that vanishes after the facade has persisted its summary
        # still has a fail-closed overlay. Accept that observable path when
        # the persisted summary proves the nonconforming worker; do not infer
        # success from the missing outer exit code.
        if not isinstance(capture_raw, str) or not (Path(capture_raw) / "summary.json").is_file():
            raise WorkSlotJourneyFailure(
                f"nonconforming collector must exit nonzero or persist failed summary: {failed}"
            )
        persisted = json.loads((Path(capture_raw) / "summary.json").read_text(encoding="utf-8"))
        if not any(worker.get("status") == "failed" for worker in persisted.get("workers", [])):
            raise WorkSlotJourneyFailure(
                f"nonconforming collector omitted persisted failed worker: {failed}"
            )
    if isinstance(collector_exit, int):
        _assert_inner_exit_codes(failed, (0, 0))
    capture_raw = failed.get("capture_dir")
    if not isinstance(capture_raw, str) or not capture_raw:
        raise WorkSlotJourneyFailure(
            f"contracted fan-out overlay omitted capture_dir: {failed}"
        )
    capture_dir = Path(capture_raw)
    _assert_capture_files(capture_dir, ("0", "1"))
    workers = _load_summary_workers(capture_dir)
    statuses = [worker.get("status") for worker in workers]
    if statuses != ["succeeded", "failed"]:
        raise WorkSlotJourneyFailure(
            f"contracted worker statuses {statuses} != ['succeeded', 'failed']"
        )
    if [worker.get("exit_code") for worker in workers] != [0, 0]:
        raise WorkSlotJourneyFailure(
            f"contracted summary did not preserve exit 0: {workers}"
        )
    if "conformance_error" in workers[0]:
        raise WorkSlotJourneyFailure(
            f"conforming worker unexpectedly has conformance_error: {workers[0]}"
        )
    error = workers[1].get("conformance_error")
    if not isinstance(error, str) or not error:
        raise WorkSlotJourneyFailure(
            f"nonconforming worker omitted conformance_error: {workers[1]}"
        )
    stdout_a = capture_dir / "0" / "stdout"
    stdout_b = capture_dir / "1" / "stdout"
    if stdout_a.read_text(encoding="utf-8") != conforming:
        raise WorkSlotJourneyFailure("conforming stdout capture changed")
    if stdout_b.read_text(encoding="utf-8") != refusal:
        raise WorkSlotJourneyFailure("refusal stdout capture changed")

    rejected = engine_call(["event", run_id, "approved"])
    expect_rejection(
        rejected,
        BOUND_SLOT_INVOCATION_REQUIRED,
        action="event after contracted collector failure",
    )
    persisted = engine_call(["show", run_id])
    if persisted.get("status") != "completed":
        raise WorkSlotJourneyFailure(
            f"show after contracted collector failure failed: {persisted}"
        )
    invocations = (persisted.get("result") or {}).get("work_slot_invocations")
    match = next(
        (
            item
            for item in invocations or []
            if isinstance(item, dict)
            and item.get("invocation_id") == failed.get("invocation_id")
        ),
        None,
    )
    if not isinstance(match, dict) or match.get("status") != "failed":
        raise WorkSlotJourneyFailure(
            f"failed contracted overlay was not persisted: {persisted}"
        )
    if match.get("capture_dir") != str(capture_dir):
        raise WorkSlotJourneyFailure(
            f"persisted capture_dir changed: {match}"
        )
    _assert_capture_files(capture_dir, ("0", "1"))
    persisted_workers = _load_summary_workers(capture_dir)
    if [worker.get("status") for worker in persisted_workers] != [
        "succeeded",
        "failed",
    ]:
        raise WorkSlotJourneyFailure(
            f"contracted summary did not persist: {persisted_workers}"
        )
    if not receipt_a.is_file() or not receipt_b.is_file():
        raise WorkSlotJourneyFailure(
            f"contracted workers did not capture stdin: {receipt_a} {receipt_b}"
        )
    recorded_a = receipt_a.read_bytes()
    recorded_b = receipt_b.read_bytes()
    if recorded_a != recorded_b:
        raise WorkSlotJourneyFailure(
            f"contracted workers recorded different stdin: {recorded_a!r} / {recorded_b!r}"
        )
    assert_bound_preamble_stdin(
        recorded_a,
        preamble=preamble,
        artifact_root=artifact_root,
        run_id=run_id,
        slot_id=slot_id,
    )
    return capture_dir


def prove_bound_graph_runner_heartbeat(
    *,
    engine: Path,
    provider: Path,
    profile_source: Path,
    fixture_root: Path,
    checkout_root: Path,
    work_dir: Path,
) -> list[str]:
    """Bound run-plan-graph invoke: inner workers in task order plus capture isolation."""
    work_dir.mkdir(parents=True, exist_ok=True)
    run_id = "bound-graph-runner-heartbeat"
    selected_checkout = work_dir / "selected-checkout"
    if not checkout_root.is_absolute() or not checkout_root.is_dir():
        raise WorkSlotJourneyFailure(
            f"bound graph checkout is not an existing absolute directory: {checkout_root}"
        )
    if not (checkout_root / ".git").exists():
        raise WorkSlotJourneyFailure(
            f"bound graph checkout omitted repository marker {checkout_root / '.git'}"
        )
    try:
        selected_checkout.symlink_to(checkout_root, target_is_directory=True)
    except OSError as error:
        raise WorkSlotJourneyFailure(
            f"could not create selected checkout symlink {selected_checkout}: {error}"
        ) from error
    if not selected_checkout.is_dir() or not (selected_checkout / ".git").exists():
        raise WorkSlotJourneyFailure(
            f"selected checkout alias cannot see repository marker: {selected_checkout}"
        )
    extra = {
        "implement": implement_graph_runner_binding(
            provider=provider,
            task_worker=stdin_worker_cli(
                work_dir / "task-receipts",
                ("--record-cwd", "--require-cwd-entry", ".git"),
            ),
            working_directory=selected_checkout,
        )
    }
    implement_args = extra["implement"]["args"]
    if implement_args[1:3] != ["--working-directory", str(selected_checkout)]:
        raise WorkSlotJourneyFailure(
            f"bound implement argv did not freeze selected checkout alias: {implement_args}"
        )
    engine_call, artifact_root, profile = _start_isolated_software_change(
        engine=engine,
        provider=provider,
        profile_source=profile_source,
        fixture_root=fixture_root,
        work_dir=work_dir / "run",
        run_id=run_id,
        extra_bindings=extra,
    )
    _advance_software_change_to(
        engine_call,
        run_id=run_id,
        profile=profile,
        artifact_root=artifact_root,
        target="implement",
    )
    plan = json.loads((artifact_root / "plan.json").read_text(encoding="utf-8"))
    if not isinstance(plan, dict) or not isinstance(plan.get("tasks"), list):
        raise WorkSlotJourneyFailure("isolated plan.json is not a plan document")
    task_ids = [task["id"] for task in plan["tasks"] if isinstance(task, dict)]
    if not task_ids or any(not isinstance(task_id, str) for task_id in task_ids):
        raise WorkSlotJourneyFailure(f"isolated plan.json omitted task ids: {plan}")
    first = invoke_until_succeeded(
        engine_call, run_id, "implement", timeout_s=45.0
    )
    inner = first.get("inner_workers")
    if not isinstance(inner, list) or len(inner) != len(task_ids):
        raise WorkSlotJourneyFailure(
            f"implement inner_workers length {inner} != plan tasks {task_ids}"
        )
    _assert_inner_exit_codes(first, [0] * len(task_ids))
    _assert_capture_files(Path(first["capture_dir"]), task_ids)
    _assert_capture_files(Path(first["capture_dir"]), ("summarizer",))
    _assert_yaml_working_directory(Path(first["capture_dir"]), selected_checkout)
    plan_revision = plan.get("revision")
    if not isinstance(plan_revision, str) or not plan_revision:
        raise WorkSlotJourneyFailure(f"isolated plan.json omitted revision: {plan}")
    _assert_dummy_report(artifact_root, plan_revision)
    report = json.loads((artifact_root / "implementation-report.json").read_text(encoding="utf-8"))
    author = report.get("author") if isinstance(report, dict) else None
    if not isinstance(author, dict) or author.get("name") != "dummy-stdin-worker":
        raise WorkSlotJourneyFailure(
            f"bound dummy summarizer did not author implementation-report.json: {author}"
        )
    receipts = work_dir / "task-receipts"
    for task_id in task_ids:
        path = receipts / f"{task_id}.stdin"
        if not path.is_file():
            raise WorkSlotJourneyFailure(f"bound dummy omitted task receipt {path}")
        recorded = path.read_text(encoding="utf-8")
        if SUMMARIZER_ASSIGNMENT_PREFIX in recorded:
            raise WorkSlotJourneyFailure(
                f"ordinary dummy task {task_id} received summarizer assignment"
            )
        if "instruction_body" in recorded.split(GRAPH_STDIN_SEPARATOR, 1)[0]:
            raise WorkSlotJourneyFailure(
                f"ordinary dummy task {task_id} stdin dumped instruction_body"
            )
    summarizer_receipt = receipts / "summarizer.stdin"
    if not summarizer_receipt.is_file():
        raise WorkSlotJourneyFailure("bound dummy omitted summarizer stdin receipt")
    if SUMMARIZER_ASSIGNMENT_PREFIX not in summarizer_receipt.read_text(encoding="utf-8"):
        raise WorkSlotJourneyFailure("bound dummy summarizer receipt omitted assignment")
    for worker_id in [*task_ids, "summarizer"]:
        _assert_cwd_receipt(
            receipts,
            worker_id,
            selected_directory=selected_checkout,
        )
    for worker in inner:
        if worker.get("command") != sys.executable:
            raise WorkSlotJourneyFailure(
                f"implement inner command was not dummy python: {worker}"
            )
        args = worker.get("args") or []
        if not any("dummy-stdin-worker.py" in item for item in args):
            raise WorkSlotJourneyFailure(
                f"implement inner args omitted dummy worker: {worker}"
            )
    second = invoke_until_succeeded(
        engine_call, run_id, "implement", timeout_s=45.0
    )
    _assert_capture_isolation(first, second, task_ids)
    return [
        "bound run-plan-graph show inner workers in plan task order",
        "graph-level working_dir inherited by every task and summarizer",
        "symlink-selected checkout cwd receipts are filesystem-equivalent and see .git",
        "capture_dir isolation on implement retry",
        "dummy summarizer writes implementation-report.json; ordinary dummy tasks do not",
        "no live model",
    ]


def _start_invocation(
    engine_call: EngineCall,
    run_id: str,
    slot_id: str,
) -> tuple[str, str]:
    started = engine_call(["invoke", run_id, slot_id])
    if started.get("status") != "completed":
        raise WorkSlotJourneyFailure(f"invoke failed: {started}")
    result = started.get("result") or {}
    invocation_id = result.get("invocation_id")
    if not isinstance(invocation_id, str) or not invocation_id:
        raise WorkSlotJourneyFailure(f"invoke omitted invocation_id: {started}")
    if result.get("slot_id") != slot_id:
        raise WorkSlotJourneyFailure(f"invoke returned wrong slot_id: {started}")
    capture_dir = result.get("capture_dir")
    if not isinstance(capture_dir, str) or not capture_dir:
        raise WorkSlotJourneyFailure(f"invoke omitted capture_dir: {started}")
    return invocation_id, capture_dir


def _show_invocation(
    engine_call: EngineCall,
    run_id: str,
    invocation_id: str,
) -> dict[str, Any] | None:
    shown = engine_call(["show", run_id])
    if shown.get("status") != "completed":
        raise WorkSlotJourneyFailure(f"show after invoke failed: {shown}")
    invocations = (shown.get("result") or {}).get("work_slot_invocations")
    if not isinstance(invocations, list):
        raise WorkSlotJourneyFailure("show omitted work_slot_invocations")
    return next(
        (
            item
            for item in invocations
            if isinstance(item, dict) and item.get("invocation_id") == invocation_id
        ),
        None,
    )


def _wait_overlay_status(
    engine_call: EngineCall,
    run_id: str,
    invocation_id: str,
    *,
    expected: str,
    timeout_s: float,
) -> dict[str, Any]:
    deadline = time.monotonic() + timeout_s
    last_status = None
    while time.monotonic() < deadline:
        match = _show_invocation(engine_call, run_id, invocation_id)
        if match is None:
            last_status = "missing"
        else:
            last_status = match.get("status")
            if last_status == expected:
                return match
            if (
                last_status in ("succeeded", "failed", "overrun")
                and last_status != expected
            ):
                raise WorkSlotJourneyFailure(
                    f"overlay ended {last_status}, expected {expected}: {match}"
                )
        time.sleep(0.05)
    raise WorkSlotJourneyFailure(
        f"timed out waiting for overlay {expected} (last={last_status})"
    )


def _assert_running_overlay(
    match: Mapping[str, Any],
    *,
    slot_id: str,
    capture_dir: str,
) -> None:
    if match.get("slot_id") != slot_id:
        raise WorkSlotJourneyFailure(f"running overlay has wrong slot: {match}")
    if match.get("status") != "running":
        raise WorkSlotJourneyFailure(f"overlay is not running: {match}")
    if match.get("overlay_meaning") != OVERLAY_MEANING_RUNNING:
        raise WorkSlotJourneyFailure(
            f"running overlay_meaning mismatch: {match.get('overlay_meaning')!r}"
        )
    elapsed = match.get("elapsed_ms")
    remaining = match.get("remaining_allowed_ms")
    if not isinstance(elapsed, int) or elapsed < 0:
        raise WorkSlotJourneyFailure(f"elapsed_ms invalid while running: {elapsed!r}")
    if not isinstance(remaining, int) or remaining < 0:
        raise WorkSlotJourneyFailure(
            f"remaining_allowed_ms invalid while running: {remaining!r}"
        )
    if match.get("capture_dir") != capture_dir:
        raise WorkSlotJourneyFailure(
            f"running capture_dir {match.get('capture_dir')!r} != {capture_dir!r}"
        )
    inner = match.get("inner_workers")
    if inner != []:
        raise WorkSlotJourneyFailure(
            f"running overlay inner_workers must be empty: {inner}"
        )
    if "waiter_pid" in match:
        raise WorkSlotJourneyFailure("show leaked waiter_pid")


def _assert_progress_snapshot(
    envelope: Mapping[str, Any],
    *,
    run_id: str,
    invocation_id: str,
    capture_dir: str,
) -> dict[str, Any]:
    if envelope.get("status") != "completed":
        raise WorkSlotJourneyFailure(
            f"invocation-progress was not a success envelope: {envelope}"
        )
    result = envelope.get("result")
    if not isinstance(result, dict):
        raise WorkSlotJourneyFailure(f"invocation-progress omitted result: {envelope}")
    leaked = sorted(PROGRESS_SNAPSHOT_FORBIDDEN.intersection(result))
    if leaked:
        raise WorkSlotJourneyFailure(
            f"progress snapshot leaked overlay fields {leaked}: {result}"
        )
    if result.get("run_id") != run_id:
        raise WorkSlotJourneyFailure(
            f"progress run_id {result.get('run_id')!r} != {run_id!r}"
        )
    if result.get("invocation_id") != invocation_id:
        raise WorkSlotJourneyFailure(
            f"progress invocation_id {result.get('invocation_id')!r} != {invocation_id!r}"
        )
    if result.get("capture_dir") != capture_dir:
        raise WorkSlotJourneyFailure(
            f"progress capture_dir {result.get('capture_dir')!r} != {capture_dir!r}"
        )
    slot_id = result.get("slot_id")
    if not isinstance(slot_id, str) or not slot_id:
        raise WorkSlotJourneyFailure(f"progress omitted slot_id: {result}")
    traces = result.get("traces")
    if not isinstance(traces, list):
        raise WorkSlotJourneyFailure(f"progress omitted traces list: {result}")
    return result


def _assert_graph_steps(
    result: Mapping[str, Any],
    expected_names: Sequence[str],
) -> list[dict[str, Any]]:
    graph = result.get("graph")
    if not isinstance(graph, dict):
        raise WorkSlotJourneyFailure(f"progress omitted graph after locator: {result}")
    locator = graph.get("locator")
    if not isinstance(locator, dict):
        raise WorkSlotJourneyFailure(f"progress graph omitted locator: {graph}")
    for key in ("dagu_home", "dag_name", "run_name"):
        value = locator.get(key)
        if not isinstance(value, str) or not value:
            raise WorkSlotJourneyFailure(f"progress locator omitted {key}: {locator}")
    steps = graph.get("steps")
    if not isinstance(steps, list) or not steps:
        raise WorkSlotJourneyFailure(f"progress graph omitted steps: {graph}")
    names: list[str] = []
    for step in steps:
        if not isinstance(step, dict):
            raise WorkSlotJourneyFailure(f"progress step is not an object: {step}")
        name = step.get("name")
        state = step.get("state")
        if not isinstance(name, str) or not name:
            raise WorkSlotJourneyFailure(f"progress step omitted name: {step}")
        if state not in GRAPH_STEP_STATES:
            raise WorkSlotJourneyFailure(
                f"progress step state {state!r} not in {sorted(GRAPH_STEP_STATES)}"
            )
        names.append(name)
    missing = [name for name in expected_names if name not in names]
    if missing:
        raise WorkSlotJourneyFailure(
            f"progress graph omitted steps {missing}: {names}"
        )
    return [step for step in steps if isinstance(step, dict)]


def _assert_progress_traces(
    result: Mapping[str, Any],
    *,
    require_sidecar: bool = False,
    require_session: bool = False,
) -> list[dict[str, Any]]:
    traces = result.get("traces")
    if not isinstance(traces, list):
        raise WorkSlotJourneyFailure(f"progress omitted traces: {result}")
    now_ms = int(time.time() * 1000)
    kinds: list[str] = []
    for trace in traces:
        if not isinstance(trace, dict):
            raise WorkSlotJourneyFailure(f"progress trace is not an object: {trace}")
        path = trace.get("path")
        kind = trace.get("kind")
        mtime = trace.get("last_modified_ms")
        if not isinstance(path, str) or not path:
            raise WorkSlotJourneyFailure(f"progress trace omitted path: {trace}")
        if kind not in ("sidecar", "session"):
            raise WorkSlotJourneyFailure(f"progress trace kind invalid: {trace}")
        if not isinstance(mtime, int) or mtime <= 0:
            raise WorkSlotJourneyFailure(
                f"progress trace omitted last_modified_ms: {trace}"
            )
        if mtime > now_ms + 5_000 or now_ms - mtime > 3_600_000:
            raise WorkSlotJourneyFailure(
                f"progress last_modified_ms {mtime} is not unix milliseconds: {trace}"
            )
        lowered = path.replace("\\", "/").lower()
        if lowered.endswith("/stdout") or lowered.endswith("/stderr"):
            raise WorkSlotJourneyFailure(
                f"progress named worker stdout/stderr as a trace: {trace}"
            )
        for forbidden in ("stdout", "stderr", "body", "contents"):
            if forbidden in trace:
                raise WorkSlotJourneyFailure(
                    f"progress trace parsed {forbidden}: {trace}"
                )
        if not Path(path).is_file():
            raise WorkSlotJourneyFailure(f"progress named missing trace file {path}")
        kinds.append(kind)
    if require_sidecar and "sidecar" not in kinds:
        raise WorkSlotJourneyFailure(f"progress omitted sidecar traces: {traces}")
    if require_session and "session" not in kinds:
        raise WorkSlotJourneyFailure(f"progress omitted session traces: {traces}")
    return [trace for trace in traces if isinstance(trace, dict)]


def _ordinary_running_count(
    steps: Sequence[Mapping[str, Any]],
    *,
    terminal_names: Sequence[str],
) -> int:
    terminals = set(terminal_names)
    return sum(
        1
        for step in steps
        if step.get("name") not in terminals and step.get("state") == "running"
    )


def _assert_terminal_after_ordinary(
    steps: Sequence[Mapping[str, Any]],
    *,
    terminal_name: str,
    ordinary_names: Sequence[str],
) -> None:
    by_name = {step.get("name"): step.get("state") for step in steps}
    terminal_state = by_name.get(terminal_name)
    if terminal_state not in ("running", "reaped"):
        return
    unfinished = [
        name
        for name in ordinary_names
        if by_name.get(name) != "reaped"
    ]
    if unfinished:
        raise WorkSlotJourneyFailure(
            f"{terminal_name} is {terminal_state} before ordinary steps reaped "
            f"{unfinished}: {steps}"
        )


def _locator_is_complete(locator: Path) -> bool:
    try:
        payload = json.loads(locator.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError):
        return False
    if not isinstance(payload, dict):
        return False
    return all(
        isinstance(payload.get(key), str) and payload.get(key)
        for key in ("dagu_home", "dag_name", "run_name")
    )


def _wait_for_locator(capture_dir: Path, timeout_s: float) -> Path:
    locator = capture_dir / "dagu-locator.json"
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        if locator.is_file() and _locator_is_complete(locator):
            return locator
        time.sleep(0.05)
    raise WorkSlotJourneyFailure(f"timed out waiting for locator {locator}")


def _query_progress(
    engine_call: EngineCall,
    run_id: str,
    invocation_id: str | None = None,
) -> dict[str, Any]:
    arguments = ["invocation-progress", run_id]
    if invocation_id is not None:
        arguments.append(invocation_id)
    return engine_call(arguments)


def _assert_progress_query_error(envelope: Mapping[str, Any], *, context: str) -> None:
    status = envelope.get("status")
    if status == "completed":
        raise WorkSlotJourneyFailure(
            f"{context}: invocation-progress unexpectedly succeeded: {envelope}"
        )
    if status not in ("error", "invalid-invocation"):
        raise WorkSlotJourneyFailure(
            f"{context}: expected a non-success envelope, got {envelope}"
        )


def _induce_progress_query_failure(
    engine_call: EngineCall,
    *,
    run_id: str,
    invocation_id: str,
    slot_id: str,
    capture_dir: str,
) -> None:
    unknown = _query_progress(engine_call, run_id, "missing-invocation-id")
    _assert_progress_query_error(unknown, context="unknown invocation id")
    match = _show_invocation(engine_call, run_id, invocation_id)
    if match is None:
        raise WorkSlotJourneyFailure("show omitted overlay after unknown-id query")
    status = match.get("status")
    if status == "failed":
        raise WorkSlotJourneyFailure(
            f"unknown-id progress query flipped overlay to failed: {match}"
        )
    if status == "running":
        _assert_running_overlay(match, slot_id=slot_id, capture_dir=capture_dir)

    locator = _wait_for_locator(Path(capture_dir), 8.0)
    original = locator.read_bytes()
    try:
        locator.write_text("{not-json", encoding="utf-8")
        malformed = _query_progress(engine_call, run_id, invocation_id)
        _assert_progress_query_error(malformed, context="malformed dagu-locator.json")
    finally:
        locator.write_bytes(original)
    match = _show_invocation(engine_call, run_id, invocation_id)
    if match is None:
        raise WorkSlotJourneyFailure("show omitted overlay after malformed-locator query")
    status = match.get("status")
    if status == "failed":
        raise WorkSlotJourneyFailure(
            f"malformed-locator progress query flipped overlay to failed: {match}"
        )
    if status == "running":
        _assert_running_overlay(match, slot_id=slot_id, capture_dir=capture_dir)
    if status not in ("running", "succeeded"):
        raise WorkSlotJourneyFailure(
            f"progress query left overlay {status}: {match}"
        )


def _poll_overlay_running_progress(
    engine_call: EngineCall,
    *,
    run_id: str,
    invocation_id: str,
    slot_id: str,
    capture_dir: str,
    expected_steps: Sequence[str],
    timeout_s: float,
    max_ordinary_running: int | None = None,
    ordinary_names: Sequence[str] | None = None,
    terminal_name: str | None = None,
    require_session: bool = False,
) -> dict[str, Any]:
    deadline = time.monotonic() + timeout_s
    saw_graph = False
    last_result: dict[str, Any] | None = None
    last_with_sessions: dict[str, Any] | None = None
    observed_ordinary_running = 0
    while time.monotonic() < deadline:
        match = _show_invocation(engine_call, run_id, invocation_id)
        if match is None:
            time.sleep(0.05)
            continue
        status = match.get("status")
        if status == "running":
            _assert_running_overlay(match, slot_id=slot_id, capture_dir=capture_dir)
            envelope = _query_progress(engine_call, run_id)
            if envelope.get("status") != "completed":
                time.sleep(0.05)
                continue
            result = _assert_progress_snapshot(
                envelope,
                run_id=run_id,
                invocation_id=invocation_id,
                capture_dir=capture_dir,
            )
            after = _show_invocation(engine_call, run_id, invocation_id)
            if after is None:
                raise WorkSlotJourneyFailure("show omitted overlay after progress query")
            if after.get("status") == "running":
                _assert_running_overlay(
                    after, slot_id=slot_id, capture_dir=capture_dir
                )
            graph = result.get("graph")
            if isinstance(graph, dict):
                steps = _assert_graph_steps(result, expected_steps)
                saw_graph = True
                last_result = result
                traces = _assert_progress_traces(result)
                if any(trace.get("kind") == "session" for trace in traces):
                    last_with_sessions = result
                if max_ordinary_running is not None:
                    running = _ordinary_running_count(
                        steps, terminal_names=(terminal_name,) if terminal_name else ()
                    )
                    if running > max_ordinary_running:
                        raise WorkSlotJourneyFailure(
                            f"invocation-progress showed {running} ordinary steps "
                            f"running (cap {max_ordinary_running}): {steps}"
                        )
                    observed_ordinary_running = max(
                        observed_ordinary_running, running
                    )
                if terminal_name and ordinary_names is not None:
                    _assert_terminal_after_ordinary(
                        steps,
                        terminal_name=terminal_name,
                        ordinary_names=ordinary_names,
                    )
        elif status == "succeeded":
            break
        elif status in ("failed", "overrun"):
            raise WorkSlotJourneyFailure(
                f"overlay ended {status} during progress poll: {match}"
            )
        time.sleep(0.05)
    if not saw_graph or last_result is None:
        raise WorkSlotJourneyFailure(
            "never observed a locator-backed invocation-progress graph while overlay running"
        )
    if max_ordinary_running is not None and observed_ordinary_running < 1:
        raise WorkSlotJourneyFailure(
            "never observed an ordinary worker step running under the concurrency cap"
        )
    if require_session:
        if last_with_sessions is None:
            raise WorkSlotJourneyFailure(
                "never observed session traces in invocation-progress while overlay running"
            )
        return last_with_sessions
    return last_result


def prove_overlay_running_bound_fan_out_progress(
    *,
    engine: Path,
    provider: Path,
    profile_source: Path,
    fixture_root: Path,
    work_dir: Path,
) -> list[str]:
    """Bound fan-out yields an overlay-running progress snapshot with dummy workers."""
    work_dir.mkdir(parents=True, exist_ok=True)
    run_id = "overlay-running-bound-fan-out"
    slot_id = "design-review"
    extra = {
        slot_id: fan_out_binding(
            engine=engine,
            workers=[
                stdin_worker_cli(work_dir / "w0.stdin", ("--sleep", "8")),
                stdin_worker_cli(work_dir / "w1.stdin", ("--sleep", "8")),
            ],
        )
    }
    engine_call, _artifact_root, profile = _start_isolated_software_change(
        engine=engine,
        provider=provider,
        profile_source=profile_source,
        fixture_root=fixture_root,
        work_dir=work_dir / "run",
        run_id=run_id,
        extra_bindings=extra,
    )
    _advance_software_change_to(
        engine_call,
        run_id=run_id,
        profile=profile,
        artifact_root=_artifact_root,
        target=slot_id,
    )
    invocation_id, capture_dir = _start_invocation(engine_call, run_id, slot_id)
    running = _wait_overlay_status(
        engine_call, run_id, invocation_id, expected="running", timeout_s=8.0
    )
    _assert_running_overlay(running, slot_id=slot_id, capture_dir=capture_dir)
    _induce_progress_query_failure(
        engine_call,
        run_id=run_id,
        invocation_id=invocation_id,
        slot_id=slot_id,
        capture_dir=capture_dir,
    )
    snapshot = _poll_overlay_running_progress(
        engine_call,
        run_id=run_id,
        invocation_id=invocation_id,
        slot_id=slot_id,
        capture_dir=capture_dir,
        expected_steps=("w0", "w1", "join"),
        timeout_s=30.0,
        terminal_name="join",
        ordinary_names=("w0", "w1"),
        require_session=True,
    )
    _assert_yaml_omits_max_active_steps(Path(capture_dir))
    _assert_progress_traces(snapshot, require_session=True)
    succeeded = _wait_overlay_status(
        engine_call, run_id, invocation_id, expected="succeeded", timeout_s=20.0
    )
    if succeeded.get("overlay_meaning") != OVERLAY_MEANING_SUCCEEDED:
        raise WorkSlotJourneyFailure(
            f"fan-out overlay meaning mismatch after progress queries: {succeeded}"
        )
    final = _query_progress(engine_call, run_id, invocation_id)
    final_result = _assert_progress_snapshot(
        final,
        run_id=run_id,
        invocation_id=invocation_id,
        capture_dir=capture_dir,
    )
    _assert_graph_steps(final_result, ("w0", "w1", "join"))
    _assert_progress_traces(final_result, require_sidecar=True, require_session=True)
    return [
        "overlay-running bound fan-out invocation-progress names invocation capture_dir graph steps",
        "show overlay elapsed remaining capture_dir unchanged; inner_workers empty while running",
        "progress-query failure leaves overlay running or succeeded from facade waitpid",
        "snapshot names sidecar and session traces with last_modified_ms",
        "no live model",
    ]


def prove_overlay_running_bound_graph_runner_progress(
    *,
    engine: Path,
    provider: Path,
    profile_source: Path,
    fixture_root: Path,
    work_dir: Path,
) -> list[str]:
    """Bound run-plan-graph yields an overlay-running progress snapshot with dummy workers."""
    work_dir.mkdir(parents=True, exist_ok=True)
    _ensure_git_repository(work_dir)
    run_id = "overlay-running-bound-graph-runner"
    slot_id = "implement"
    extra = {
        slot_id: implement_graph_runner_binding(
            provider=provider,
            task_worker=stdin_worker_cli(work_dir / "task-receipts", ("--sleep", "6")),
            working_directory=work_dir,
        )
    }
    engine_call, artifact_root, profile = _start_isolated_software_change(
        engine=engine,
        provider=provider,
        profile_source=profile_source,
        fixture_root=fixture_root,
        work_dir=work_dir / "run",
        run_id=run_id,
        extra_bindings=extra,
    )
    _advance_software_change_to(
        engine_call,
        run_id=run_id,
        profile=profile,
        artifact_root=artifact_root,
        target=slot_id,
    )
    plan = _write_small_plan(artifact_root)
    task_ids = [task["id"] for task in plan["tasks"]]
    expected_steps = [*task_ids, "summarizer"]
    invocation_id, capture_dir = _start_invocation(engine_call, run_id, slot_id)
    running = _wait_overlay_status(
        engine_call, run_id, invocation_id, expected="running", timeout_s=8.0
    )
    _assert_running_overlay(running, slot_id=slot_id, capture_dir=capture_dir)
    snapshot = _poll_overlay_running_progress(
        engine_call,
        run_id=run_id,
        invocation_id=invocation_id,
        slot_id=slot_id,
        capture_dir=capture_dir,
        expected_steps=expected_steps,
        timeout_s=35.0,
        terminal_name="summarizer",
        ordinary_names=task_ids,
        require_session=True,
    )
    _assert_yaml_max_active_steps(Path(capture_dir), 4)
    _assert_progress_traces(snapshot, require_session=True)
    succeeded = _wait_overlay_status(
        engine_call, run_id, invocation_id, expected="succeeded", timeout_s=35.0
    )
    if succeeded.get("overlay_meaning") != OVERLAY_MEANING_SUCCEEDED:
        raise WorkSlotJourneyFailure(
            f"run-plan-graph overlay meaning mismatch after progress poll: {succeeded}"
        )
    return [
        "overlay-running bound run-plan-graph invocation-progress names task ids plus summarizer",
        "omitted run-plan-graph yaml has max_active_steps: 4",
        "show inner_workers empty while overlay running",
        "no live model",
    ]


def prove_max_active_bound_fan_out(
    *,
    engine: Path,
    provider: Path,
    profile_source: Path,
    fixture_root: Path,
    work_dir: Path,
) -> list[str]:
    """Set --max-active N in bound argv and ad-hoc fan-out without a run."""
    work_dir.mkdir(parents=True, exist_ok=True)
    run_id = "max-active-bound-fan-out"
    slot_id = "design-review"
    extra = {
        slot_id: fan_out_binding(
            engine=engine,
            max_active=1,
            workers=[
                stdin_worker_cli(work_dir / "w0.stdin", ("--sleep", "2.5")),
                stdin_worker_cli(work_dir / "w1.stdin", ("--sleep", "2.5")),
                stdin_worker_cli(work_dir / "w2.stdin", ("--sleep", "2.5")),
            ],
        )
    }
    engine_call, artifact_root, profile = _start_isolated_software_change(
        engine=engine,
        provider=provider,
        profile_source=profile_source,
        fixture_root=fixture_root,
        work_dir=work_dir / "run",
        run_id=run_id,
        extra_bindings=extra,
    )
    _advance_software_change_to(
        engine_call,
        run_id=run_id,
        profile=profile,
        artifact_root=artifact_root,
        target=slot_id,
    )
    invocation_id, capture_dir = _start_invocation(engine_call, run_id, slot_id)
    _wait_overlay_status(
        engine_call, run_id, invocation_id, expected="running", timeout_s=8.0
    )
    _poll_overlay_running_progress(
        engine_call,
        run_id=run_id,
        invocation_id=invocation_id,
        slot_id=slot_id,
        capture_dir=capture_dir,
        expected_steps=("w0", "w1", "w2", "join"),
        timeout_s=25.0,
        max_ordinary_running=1,
        ordinary_names=("w0", "w1", "w2"),
        terminal_name="join",
    )
    _assert_yaml_max_active_steps(Path(capture_dir), 1)
    _wait_overlay_status(
        engine_call, run_id, invocation_id, expected="succeeded", timeout_s=20.0
    )

    adhoc_root = work_dir / "adhoc"
    adhoc_root.mkdir(parents=True, exist_ok=True)
    instructions = adhoc_root / "instructions.bin"
    instructions.write_bytes(b"ad-hoc-max-active-bytes")
    adhoc = _run_binding(
        fan_out_binding(
            engine=engine,
            max_active=3,
            workers=[
                stdin_worker_cli(adhoc_root / "a.stdin"),
                stdin_worker_cli(adhoc_root / "b.stdin"),
            ],
            instructions=instructions,
        ),
        stdin=b"not a packet",
        cwd=adhoc_root,
    )
    if adhoc.returncode != 0:
        raise WorkSlotJourneyFailure(
            "ad hoc --max-active fan-out exited "
            f"{adhoc.returncode}: {adhoc.stderr.decode('utf-8', 'replace')}"
        )
    adhoc_summary = _json_stdout(adhoc)
    adhoc_dir = adhoc_summary.get("output_dir")
    if not isinstance(adhoc_dir, str) or not adhoc_dir:
        raise WorkSlotJourneyFailure(
            f"ad hoc --max-active omitted output_dir: {adhoc_summary}"
        )
    _assert_yaml_max_active_steps(Path(adhoc_dir), 3)
    return [
        "bound fan-out --max-active 1 yaml has max_active_steps: 1",
        "invocation-progress never shows more than one ordinary fan-out step running",
        "ad-hoc fan-out without a run emits max_active_steps N",
        "no live model",
    ]


def prove_max_active_bound_graph_runner(
    *,
    engine: Path,
    provider: Path,
    profile_source: Path,
    fixture_root: Path,
    work_dir: Path,
) -> list[str]:
    """Set --max-active N on bound run-plan-graph with dummy ordinary tasks."""
    work_dir.mkdir(parents=True, exist_ok=True)
    _ensure_git_repository(work_dir)
    run_id = "max-active-bound-graph-runner"
    slot_id = "implement"
    extra = {
        slot_id: implement_graph_runner_binding(
            provider=provider,
            max_active=1,
            task_worker=stdin_worker_cli(
                work_dir / "task-receipts", ("--sleep", "2.5")
            ),
            working_directory=work_dir,
        )
    }
    engine_call, artifact_root, profile = _start_isolated_software_change(
        engine=engine,
        provider=provider,
        profile_source=profile_source,
        fixture_root=fixture_root,
        work_dir=work_dir / "run",
        run_id=run_id,
        extra_bindings=extra,
    )
    _advance_software_change_to(
        engine_call,
        run_id=run_id,
        profile=profile,
        artifact_root=artifact_root,
        target=slot_id,
    )
    plan = _write_plan_document(artifact_root, independent_plan_document(3))
    task_ids = [task["id"] for task in plan["tasks"]]
    invocation_id, capture_dir = _start_invocation(engine_call, run_id, slot_id)
    _wait_overlay_status(
        engine_call, run_id, invocation_id, expected="running", timeout_s=8.0
    )
    _poll_overlay_running_progress(
        engine_call,
        run_id=run_id,
        invocation_id=invocation_id,
        slot_id=slot_id,
        capture_dir=capture_dir,
        expected_steps=(*task_ids, "summarizer"),
        timeout_s=25.0,
        max_ordinary_running=1,
        ordinary_names=task_ids,
        terminal_name="summarizer",
    )
    _assert_yaml_max_active_steps(Path(capture_dir), 1)
    succeeded = _wait_overlay_status(
        engine_call, run_id, invocation_id, expected="succeeded", timeout_s=25.0
    )
    report = json.loads(
        (artifact_root / "implementation-report.json").read_text(encoding="utf-8")
    )
    author = report.get("author") if isinstance(report, dict) else None
    if not isinstance(author, dict) or author.get("name") != "dummy-stdin-worker":
        raise WorkSlotJourneyFailure(
            f"max-active summarizer did not write implementation-report.json: {author}"
        )
    if succeeded.get("overlay_meaning") != OVERLAY_MEANING_SUCCEEDED:
        raise WorkSlotJourneyFailure(
            f"max-active graph overlay meaning mismatch: {succeeded}"
        )
    return [
        "bound run-plan-graph --max-active 1 yaml has max_active_steps: 1",
        "invocation-progress never shows more than one ordinary plan task running",
        "summarizer still runs after ordinary tasks",
        "no live model",
    ]


def self_test_helpers() -> None:
    """Unit-test PATH rewrite and dummy-stdin-worker without spawning the engine."""
    engine = Path("/tmp/built/loop-engine")
    provider = Path("/tmp/built/software-change")
    assert_shipped_path_names(None)
    assert_shipped_path_names({})
    try:
        assert_shipped_path_names(
            {
                "implement": {
                    "command": PATH_SOFTWARE_CHANGE,
                    "args": list(SHIPPED_IMPLEMENT_ARGS),
                },
            }
        )
    except WorkSlotJourneyFailure as error:
        if "unexpectedly bound" not in str(error):
            raise WorkSlotJourneyFailure(
                f"self-test: shipped check rejected implement for the wrong reason: {error}"
            ) from error
    else:
        raise WorkSlotJourneyFailure(
            "self-test: shipped check accepted implement binding"
        )
    opt_in_implement = {
        "implement": {
            "command": PATH_SOFTWARE_CHANGE,
            "args": list(SHIPPED_IMPLEMENT_ARGS),
        },
    }
    rewritten_defaults = rewrite_path_commands(
        opt_in_implement, engine=engine, provider=provider
    )
    assert_rewritten_binaries(
        rewritten_defaults, engine=engine, provider=provider
    )
    assert_no_review_bindings(None, source="self-test-none")
    assert_no_review_bindings({}, source="self-test-empty")

    caller_supplied = {
        "implement": {
            "command": PATH_SOFTWARE_CHANGE,
            "args": list(SHIPPED_IMPLEMENT_ARGS),
        },
        "design-review": {
            "command": PATH_LOOP_ENGINE,
            "args": list(SHIPPED_FAN_OUT_ARGS),
        },
        "plan-review": {
            "command": PATH_LOOP_ENGINE,
            "args": list(SHIPPED_FAN_OUT_ARGS),
        },
        "implementation-review": {
            "command": PATH_LOOP_ENGINE,
            "args": list(SHIPPED_FAN_OUT_ARGS),
        },
        "intent-draft": {"command": sys.executable, "args": [str(WORKER_SCRIPT)]},
    }
    try:
        assert_shipped_path_names(
            {
                key: value
                for key, value in caller_supplied.items()
                if key != "intent-draft"
            }
        )
    except WorkSlotJourneyFailure as error:
        if "unexpectedly bound" not in str(error):
            raise WorkSlotJourneyFailure(
                f"self-test: shipped check rejected reviews for the wrong reason: {error}"
            ) from error
    else:
        raise WorkSlotJourneyFailure(
            "self-test: shipped check accepted review slot bindings"
        )
    rewritten = rewrite_path_commands(
        caller_supplied, engine=engine, provider=provider
    )
    if rewritten["implement"]["command"] != str(provider):
        raise WorkSlotJourneyFailure("self-test: implement PATH was not rewritten")
    if rewritten["design-review"]["command"] != str(engine):
        raise WorkSlotJourneyFailure("self-test: review PATH was not rewritten")
    if rewritten["intent-draft"]["command"] != sys.executable:
        raise WorkSlotJourneyFailure("self-test: non-PATH command was rewritten")
    if rewritten["implement"]["args"] != SHIPPED_IMPLEMENT_ARGS:
        raise WorkSlotJourneyFailure("self-test: rewrite mutated implement args")

    binding = implement_graph_runner_binding(
        provider=provider,
        task_worker=stdin_worker_cli(Path("/tmp/receipts")),
        working_directory=Path("/tmp"),
    )
    if binding["command"] != str(provider):
        raise WorkSlotJourneyFailure("self-test: graph-runner command not rewritten")
    if binding["args"][:3] != ["run-plan-graph", "--working-directory", "/tmp"]:
        raise WorkSlotJourneyFailure(f"self-test: unexpected graph-runner args {binding['args']}")
    if binding["args"][3] != "--task-worker":
        raise WorkSlotJourneyFailure(f"self-test: missing task-worker flag {binding['args']}")
    parsed_worker = json.loads(binding["args"][4])
    if parsed_worker["command"] != sys.executable:
        raise WorkSlotJourneyFailure("self-test: task-worker is not the dummy python")
    if "dummy-stdin-worker.py" not in parsed_worker["args"][0]:
        raise WorkSlotJourneyFailure("self-test: task-worker is not dummy-stdin-worker.py")

    zero = fan_out_binding(engine=engine, workers=())
    if zero != {"command": str(engine), "args": ["fan-out"]}:
        raise WorkSlotJourneyFailure(f"self-test: zero-worker binding mismatch: {zero}")
    capped = fan_out_binding(engine=engine, workers=(), max_active=2)
    if capped != {"command": str(engine), "args": ["fan-out", "--max-active", "2"]}:
        raise WorkSlotJourneyFailure(f"self-test: max-active fan-out binding mismatch: {capped}")
    capped_graph = implement_graph_runner_binding(
        provider=provider,
        task_worker=stdin_worker_cli(Path("/tmp/receipts")),
        working_directory=Path("/tmp"),
        max_active=1,
    )
    if capped_graph["args"][:5] != [
        "run-plan-graph",
        "--working-directory",
        "/tmp",
        "--max-active",
        "1",
    ]:
        raise WorkSlotJourneyFailure(
            f"self-test: max-active graph-runner args mismatch: {capped_graph['args']}"
        )

    layout = (
        '{"artifact_root":"/tmp/artifacts"}\n'
        "---\n\n"
        '{"id":"alpha"}'
    )
    parsed = parse_graph_runner_stdin(layout)
    if parsed["task"] != {"id": "alpha"} or parsed["artifact_root"] != "/tmp/artifacts":
        raise WorkSlotJourneyFailure(f"self-test: stdin layout parse mismatch: {parsed}")
    try:
        parse_graph_runner_stdin(
            '{"artifact_root":"/tmp/artifacts"}\n---\n\n'
            "Write artifact_root/implementation-report.json for this invocation only."
        )
    except WorkSlotJourneyFailure as error:
        if "summarizer" not in str(error):
            raise WorkSlotJourneyFailure(
                f"self-test: summarizer stdin failed for the wrong reason: {error}"
            ) from error
    else:
        raise WorkSlotJourneyFailure("self-test: summarizer stdin parsed as a task")

    with tempfile.TemporaryDirectory(prefix="dummy-stdin-worker-self-test-") as temp:
        receipt = Path(temp) / "raw.stdin"
        completed = subprocess.run(
            [
                sys.executable,
                str(STDIN_WORKER_SCRIPT),
                "--receipt",
                str(receipt),
                "--exit",
                "7",
            ],
            input=b"raw-bytes-not-a-model",
            capture_output=True,
            check=False,
        )
        if completed.returncode != 7:
            raise WorkSlotJourneyFailure(
                f"self-test: dummy-stdin-worker --exit 7 returned {completed.returncode}"
            )
        if receipt.read_bytes() != b"raw-bytes-not-a-model":
            raise WorkSlotJourneyFailure("self-test: dummy-stdin-worker did not record raw stdin")
        stdout_receipt = Path(temp) / "stdout.stdin"
        printed = subprocess.run(
            [
                sys.executable,
                str(STDIN_WORKER_SCRIPT),
                "--receipt",
                str(stdout_receipt),
                "--stdout",
                '{"axis":"a"}',
            ],
            input=b"record-me",
            capture_output=True,
            check=False,
        )
        if printed.returncode != 0 or printed.stdout != b'{"axis":"a"}':
            raise WorkSlotJourneyFailure(
                f"self-test: dummy-stdin-worker --stdout mismatch: {printed}"
            )
        if stdout_receipt.read_bytes() != b"record-me":
            raise WorkSlotJourneyFailure(
                "self-test: dummy-stdin-worker --stdout did not record stdin"
            )
        cwd_root = Path(temp) / "cwd-root"
        cwd_root.mkdir()
        (cwd_root / "checkout-marker").mkdir()
        cwd_receipt = Path(temp) / "cwd.stdin"
        cwd_run = subprocess.run(
            [
                sys.executable,
                str(STDIN_WORKER_SCRIPT),
                "--receipt",
                str(cwd_receipt),
                "--record-cwd",
                "--require-cwd-entry",
                "checkout-marker",
            ],
            input=b"cwd-me",
            capture_output=True,
            check=False,
            cwd=cwd_root,
        )
        if cwd_run.returncode != 0:
            raise WorkSlotJourneyFailure(
                f"self-test: dummy cwd worker exited {cwd_run.returncode}: "
                f"{cwd_run.stderr!r}"
            )
        reported_cwd = Path(
            (Path(str(cwd_receipt) + ".cwd")).read_text(encoding="utf-8").strip()
        )
        if not reported_cwd.samefile(cwd_root):
            raise WorkSlotJourneyFailure(
                f"self-test: dummy cwd receipt {reported_cwd} != {cwd_root}"
            )
        if not Path(str(cwd_receipt) + ".cwd-entry").is_file():
            raise WorkSlotJourneyFailure("self-test: dummy cwd entry receipt missing")
        session_dir = Path(temp) / "sessions"
        session_receipt = Path(temp) / "session.stdin"
        session_env = os.environ.copy()
        session_env["PI_CODING_AGENT_SESSION_DIR"] = str(session_dir)
        session_run = subprocess.run(
            [
                sys.executable,
                str(STDIN_WORKER_SCRIPT),
                "--receipt",
                str(session_receipt),
            ],
            input=b"session-me",
            capture_output=True,
            check=False,
            env=session_env,
        )
        if session_run.returncode != 0:
            raise WorkSlotJourneyFailure(
                f"self-test: dummy session worker exited {session_run.returncode}"
            )
        marker = session_dir / DUMMY_SESSION_MARKER
        if not marker.is_file():
            raise WorkSlotJourneyFailure(
                f"self-test: dummy worker omitted session marker {marker}"
            )
        receipts = Path(temp) / "graph-receipts"
        receipts.mkdir()
        artifacts = Path(temp) / "artifacts"
        artifacts.mkdir()
        (artifacts / "plan.json").write_text(
            '{"revision":"7","tasks":[],"dependency_graph":[]}\n', encoding="utf-8"
        )
        task_stdin = (
            '{"artifact_root":"' + str(artifacts) + '"}\n---\n\n'
            '{"id":"alpha","objective":"Independent A"}'
        ).encode("utf-8")
        task_run = subprocess.run(
            [sys.executable, str(STDIN_WORKER_SCRIPT), "--receipt", str(receipts)],
            input=task_stdin,
            capture_output=True,
            check=False,
        )
        if task_run.returncode != 0:
            raise WorkSlotJourneyFailure(
                f"self-test: dummy task worker exited {task_run.returncode}"
            )
        if (artifacts / "implementation-report.json").exists():
            raise WorkSlotJourneyFailure(
                "self-test: ordinary dummy task wrote implementation-report.json"
            )
        if not (receipts / "alpha.stdin").is_file():
            raise WorkSlotJourneyFailure("self-test: dummy task did not write alpha.stdin")
        summarizer_stdin = (
            '{"artifact_root":"' + str(artifacts) + '","capture_dir":"'
            + str(temp) + '","plan_path":"' + str(artifacts / "plan.json")
            + '"}\n---\n\n'
            "Write artifact_root/implementation-report.json for this invocation only. "
            "You are the sole writer of that filename."
        ).encode("utf-8")
        summarizer_run = subprocess.run(
            [sys.executable, str(STDIN_WORKER_SCRIPT), "--receipt", str(receipts)],
            input=summarizer_stdin,
            capture_output=True,
            check=False,
        )
        if summarizer_run.returncode != 0:
            raise WorkSlotJourneyFailure(
                f"self-test: dummy summarizer exited {summarizer_run.returncode}"
            )
        _assert_dummy_report(artifacts, "7")
        skipped = Path(temp) / "skipped-artifacts"
        skipped.mkdir()
        (skipped / "plan.json").write_text(
            '{"revision":"7","tasks":[],"dependency_graph":[]}\n', encoding="utf-8"
        )
        skipped_run = subprocess.run(
            [
                sys.executable,
                str(STDIN_WORKER_SCRIPT),
                "--receipt",
                str(Path(temp) / "skipped-receipts"),
                "--no-report",
            ],
            input=summarizer_stdin.replace(str(artifacts).encode(), str(skipped).encode()),
            capture_output=True,
            check=False,
        )
        if skipped_run.returncode != 0:
            raise WorkSlotJourneyFailure(
                f"self-test: dummy summarizer --no-report exited {skipped_run.returncode}"
            )
        if (skipped / "implementation-report.json").exists():
            raise WorkSlotJourneyFailure(
                "self-test: dummy summarizer --no-report still wrote the report"
            )

    preamble = "Read-only reviewer. Return only the contracted judgment."
    artifact_root = Path("/tmp/artifacts")
    context = b'{"artifact_root":"/tmp/artifacts"}\n'
    raw = preamble.encode("utf-8") + b"\n" + context + BOUND_PREAMBLE_SEPARATOR
    assert_bound_preamble_stdin(
        raw,
        preamble=preamble,
        artifact_root=artifact_root,
        run_id="run-1",
        slot_id="design-review",
    )
    dumped = raw + b"instruction_body dump\n"
    try:
        assert_bound_preamble_stdin(
            dumped,
            preamble=preamble,
            artifact_root=artifact_root,
            run_id="run-1",
            slot_id="design-review",
        )
    except WorkSlotJourneyFailure as error:
        if "instruction_body" not in str(error) and "extra bytes" not in str(error):
            raise WorkSlotJourneyFailure(
                f"self-test: instruction_body dump failed for the wrong reason: {error}"
            ) from error
    else:
        raise WorkSlotJourneyFailure("self-test: instruction_body dump unexpectedly accepted")
    duplicate = (
        preamble.encode("utf-8")
        + b"\n"
        + b'{"artifact_root":"/tmp/artifacts","capture_dir":"/tmp/cap"}\n'
        + BOUND_PREAMBLE_SEPARATOR
    )
    try:
        assert_bound_preamble_stdin(
            duplicate,
            preamble=preamble,
            artifact_root=artifact_root,
            run_id="run-1",
            slot_id="design-review",
        )
    except WorkSlotJourneyFailure as error:
        if "capture_dir" not in str(error) and "one-key" not in str(error) and "keys" not in str(error):
            raise WorkSlotJourneyFailure(
                f"self-test: duplicate-identity stdin failed for the wrong reason: {error}"
            ) from error
    else:
        raise WorkSlotJourneyFailure("self-test: capture_dir context unexpectedly accepted")

    if DEFAULT_PI_SANDBOX_ARGS != ["--print", "--no-skills", "--no-extensions"]:
        raise WorkSlotJourneyFailure(
            f"self-test: sandbox argv mismatch: {DEFAULT_PI_SANDBOX_ARGS}"
        )
    for flag in FORBIDDEN_PI_FLAGS:
        if flag in DEFAULT_PI_SANDBOX_ARGS:
            raise WorkSlotJourneyFailure(
                f"self-test: sandbox argv contains forbidden {flag}"
            )

    omitted_review = {
        "work_slots": [
            {
                "id": "deterministic-review",
                "state": "deterministic-review",
                "event": "passed",
            },
            {"id": "semantic-review", "state": "semantic-review", "event": "passed"},
        ]
    }
    assert_catalog(omitted_review, ["deterministic-review", "semantic-review"])
    try:
        assert_catalog(
            omitted_review,
            ["deterministic-review", "semantic-review"],
            stdin_context_kinds={"deterministic-review": ["finding-ledger"]},
        )
    except WorkSlotJourneyFailure as error:
        if "deterministic-review" not in str(error) or "stdin_context_kinds" not in str(
            error
        ):
            raise WorkSlotJourneyFailure(
                f"self-test: required kinds failed for the wrong reason: {error}"
            ) from error
    else:
        raise WorkSlotJourneyFailure(
            "self-test: omitted kinds unexpectedly accepted when required"
        )
    software_change_catalog = {
        "work_slots": [
            {
                "id": "intent-review",
                "state": "intent-review",
                "event": "approved",
                "stdin_context_kinds": ["finding-ledger"],
            },
            {"id": "intent-draft", "state": "explore", "event": "intent-ready"},
        ]
    }
    assert_catalog(
        software_change_catalog,
        ["intent-review", "intent-draft"],
        stdin_context_kinds={"intent-review": ["finding-ledger"]},
    )
    try:
        assert_catalog(
            {
                "work_slots": [
                    {
                        "id": "intent-draft",
                        "state": "explore",
                        "event": "intent-ready",
                        "stdin_context_kinds": ["finding-ledger"],
                    }
                ]
            },
            ["intent-draft"],
            stdin_context_kinds={"intent-review": ["finding-ledger"]},
        )
    except WorkSlotJourneyFailure as error:
        if "intent-draft" not in str(error) or "declared stdin_context_kinds" not in str(
            error
        ):
            raise WorkSlotJourneyFailure(
                f"self-test: draft kinds failed for the wrong reason: {error}"
            ) from error
    else:
        raise WorkSlotJourneyFailure(
            "self-test: draft stdin_context_kinds unexpectedly accepted"
        )
