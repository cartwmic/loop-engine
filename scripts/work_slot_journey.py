#!/usr/bin/env python3
"""Shared black-box helpers for frozen work-slot bindings in public journeys."""

from __future__ import annotations

import json
import os
import shutil
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
    "validate",
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
    "intent": "intent.json",
    "design-review": "design.json",
    "plan-review": "plan.json",
    "implementation-review": "implementation-report.json",
    "validation": "validation-report.json",
}
SOFTWARE_CHANGE_ADVANCE_STEPS = (
    ("explore", "intent", "intent-ready", "design"),
    ("design", None, "design-ready", "design-review"),
    ("design-review", "design-review", "approved", "plan"),
    ("plan", None, "plan-ready", "plan-review"),
    ("plan-review", "plan-review", "approved", "implement"),
)
PACKET_KEYS = frozenset(
    {"run_id", "slot_id", "artifact_root", "instruction_body", "capture_dir"}
)
OVERLAY_MEANING_SUCCEEDED = (
    "Overlay succeeded means the bound CLI exited 0, not that the provider accepted the work."
)
BOUND_PREAMBLE_SEPARATOR = b"---\n\n"
OVERLAY_MEANING_FAILED = (
    "Overlay failed means the bound CLI exited nonzero or the waiter vanished."
)
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
        f"Legal start: loop-engine invoke {run_id} {slot_id}. "
        f"{OVERLAY_MEANING_SUCCEEDED} "
        "Captures are at the named capture directory on the invocation view and invoke result. "
        "The driver triages worker output, appends provider-shaped records, then requests the shown event. "
        "On overrun run show immediately before re-invoking the same slot. "
        "On failed inspect capture_dir/summary.json and captured stdout before stderr."
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
    allowed = {"command", "args", "exit_code"}
    for worker in inner_workers:
        assert_inner_worker(worker)
        extra = set(worker) - allowed
        if extra:
            raise WorkSlotJourneyFailure(
                f"show inner_workers leaked extra fields {sorted(extra)}: {worker}"
            )
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


def assert_bound_preamble_stdin(
    raw: bytes,
    *,
    preamble: str,
    artifact_root: Path,
    run_id: str,
    slot_id: str,
) -> None:
    """Assert bound opted-in stdin: preamble + one-key context + separator + legacy body."""
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
    if list(parsed_context.keys()) != ["artifact_root"]:
        raise WorkSlotJourneyFailure(
            f"artifact_root context keys {list(parsed_context)} != ['artifact_root']"
        )
    expected_context = json.dumps(
        {"artifact_root": str(artifact_root)}, separators=(",", ":")
    ).encode("utf-8") + b"\n"
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
    if BOUND_PREAMBLE_SEPARATOR in body:
        raise WorkSlotJourneyFailure("legacy body/trailer unexpectedly contains the separator")
    trailer = (
        f"run_id: {run_id}\n"
        f"slot_id: {slot_id}\n"
        f"artifact_root: {artifact_root}\n"
    ).encode("utf-8")
    if not body.endswith(trailer):
        raise WorkSlotJourneyFailure(
            f"legacy body/trailer mismatch: expected suffix {trailer!r} in {body!r}"
        )


def append_task_worker(binding: Mapping[str, Any], task_worker: Mapping[str, Any]) -> dict[str, Any]:
    args = list(binding["args"]) + ["--task-worker", worker_cli_json(task_worker)]
    return {"command": binding["command"], "args": args}


def append_fan_out_workers(
    binding: Mapping[str, Any],
    workers: Sequence[Mapping[str, Any]],
    *,
    instructions: Path | None = None,
) -> dict[str, Any]:
    args = list(binding["args"])
    if instructions is not None:
        args.extend(["--instructions", str(instructions)])
    for worker in workers:
        args.extend(["--worker", fan_out_worker_json(worker)])
    return {"command": binding["command"], "args": args}


def implement_graph_runner_binding(
    *,
    provider: Path,
    task_worker: Mapping[str, Any],
) -> dict[str, Any]:
    shipped = {
        "implement": {"command": PATH_SOFTWARE_CHANGE, "args": list(SHIPPED_IMPLEMENT_ARGS)}
    }
    rewritten = rewrite_path_commands(shipped, provider=provider)
    return append_task_worker(rewritten["implement"], task_worker)


def fan_out_binding(
    *,
    engine: Path,
    workers: Sequence[Mapping[str, Any]] = (),
    instructions: Path | None = None,
) -> dict[str, Any]:
    shipped = {
        "design-review": {"command": PATH_LOOP_ENGINE, "args": list(SHIPPED_FAN_OUT_ARGS)}
    }
    rewritten = rewrite_path_commands(shipped, engine=engine)
    return append_fan_out_workers(
        rewritten["design-review"], workers, instructions=instructions
    )


def small_plan_document() -> dict[str, Any]:
    """Two independent tasks plus one dependent task. Not the calibration fixture."""
    return {
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


def parse_graph_runner_stdin(text: str) -> dict[str, Any]:
    prefix = "run_id: "
    if not text.startswith(prefix):
        raise WorkSlotJourneyFailure(f"inner stdin missing run_id line: {text!r}")
    try:
        header, rest = text.split("\n\n## instruction_body\n", 1)
        instruction_body, task_raw = rest.split("\n\n## task\n", 1)
    except ValueError as error:
        raise WorkSlotJourneyFailure(
            f"inner stdin is not the locked layout: {text!r}"
        ) from error
    fields: dict[str, str] = {}
    for line in header.splitlines():
        key, sep, value = line.partition(": ")
        if not sep:
            raise WorkSlotJourneyFailure(f"inner stdin header line missing ': ': {line!r}")
        fields[key] = value
    if set(fields) != {"run_id", "slot_id", "artifact_root"}:
        raise WorkSlotJourneyFailure(f"inner stdin header keys mismatch: {fields}")
    task_json = task_raw.strip()
    try:
        task = json.loads(task_json)
    except json.JSONDecodeError as error:
        raise WorkSlotJourneyFailure(f"inner stdin task is not JSON: {error}") from error
    artifact_root = fields["artifact_root"]
    if not Path(artifact_root).is_absolute():
        raise WorkSlotJourneyFailure(
            f"inner stdin artifact_root is not absolute: {artifact_root!r}"
        )
    return {
        "run_id": fields["run_id"],
        "slot_id": fields["slot_id"],
        "artifact_root": artifact_root,
        "instruction_body": instruction_body,
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
    """Write a PATH stub named pi that records argv and exits 0."""
    directory.mkdir(parents=True, exist_ok=True)
    stub = directory / "pi"
    stub.write_text(
        "#!/usr/bin/env python3\n"
        "import json\n"
        "import os\n"
        "import sys\n"
        "\n"
        "log_dir = os.environ['PI_STUB_LOG_DIR']\n"
        "os.makedirs(log_dir, exist_ok=True)\n"
        "path = os.path.join(log_dir, f'{os.getpid()}.argv.json')\n"
        "with open(path, 'w', encoding='utf-8') as handle:\n"
        "    json.dump(sys.argv[1:], handle)\n"
        "sys.stdin.buffer.read()\n",
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
) -> bytes:
    return json.dumps(
        {
            "run_id": run_id,
            "slot_id": slot_id,
            "artifact_root": str(artifact_root),
            "instruction_body": instruction_body,
            "capture_dir": str(capture_dir),
        },
        separators=(",", ":"),
    ).encode("utf-8")


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
    invoke_until_succeeded(engine_call, run_id, "explore-intent", timeout_s=20.0)
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
        _expect_event_state(engine_call, run_id, event, nxt)
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
        **bindings_for(["explore-intent"]),
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
        return _engine_json(engine, database, operation)

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
    plan = small_plan_document()
    (artifact_root / "plan.json").write_text(
        json.dumps(plan, indent=2) + "\n", encoding="utf-8"
    )
    return plan


def _assert_task_receipt(
    receipt_dir: Path,
    task: Mapping[str, Any],
    *,
    run_id: str,
    slot_id: str,
    artifact_root: Path,
    instruction_body: str,
) -> None:
    task_id = task["id"]
    path = receipt_dir / f"{task_id}.stdin"
    try:
        recorded = path.read_text(encoding="utf-8")
    except OSError as error:
        raise WorkSlotJourneyFailure(f"missing graph-runner receipt {path}: {error}") from error
    parsed = parse_graph_runner_stdin(recorded)
    if parsed["run_id"] != run_id or parsed["slot_id"] != slot_id:
        raise WorkSlotJourneyFailure(f"receipt identity mismatch for {task_id}: {parsed}")
    if parsed["artifact_root"] != str(artifact_root):
        raise WorkSlotJourneyFailure(
            f"receipt artifact_root {parsed['artifact_root']!r} != {str(artifact_root)!r}"
        )
    if parsed["instruction_body"] != instruction_body:
        raise WorkSlotJourneyFailure(
            f"receipt instruction_body mismatch for {task_id}: {parsed['instruction_body']!r}"
        )
    if parsed["task"] != task:
        raise WorkSlotJourneyFailure(
            f"receipt task record mismatch for {task_id}: {parsed['task']}"
        )


def prove_graph_runner(*, provider: Path, work_dir: Path) -> list[str]:
    """Prove run-plan-graph with dummy --task-worker. Never calls a live model."""
    work_dir.mkdir(parents=True, exist_ok=True)
    plan = small_plan_document()
    task_ids = [task["id"] for task in plan["tasks"]]
    tasks_by_id = {task["id"]: task for task in plan["tasks"]}
    run_id = "graph-runner-proof"
    slot_id = "implement"
    instruction_body = "Implement the small fixture plan."

    success_root = work_dir / "success"
    success_receipts = success_root / "receipts"
    success_capture = success_root / "captures" / "inv-success"
    _write_small_plan(success_root)
    (success_root / "implementation-report.json").write_text("{}\n", encoding="utf-8")
    success_binding = implement_graph_runner_binding(
        provider=provider,
        task_worker=stdin_worker_cli(success_receipts),
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
            run_id=run_id,
            slot_id=slot_id,
            artifact_root=success_root,
            instruction_body=instruction_body,
        )

    missing_root = work_dir / "missing-report"
    missing_receipts = missing_root / "receipts"
    missing_capture = missing_root / "captures" / "inv-missing"
    _write_small_plan(missing_root)
    missing = _run_binding(
        implement_graph_runner_binding(
            provider=provider,
            task_worker=stdin_worker_cli(missing_receipts),
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
            run_id=run_id,
            slot_id=slot_id,
            artifact_root=missing_root,
            instruction_body=instruction_body,
        )
    if (missing_root / "implementation-report.json").is_file():
        raise WorkSlotJourneyFailure("missing-report path found a report file")

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
    _assert_task_receipt(
        reap_receipts,
        tasks_by_id["alpha"],
        run_id=run_id,
        slot_id=slot_id,
        artifact_root=reap_root,
        instruction_body=instruction_body,
    )
    _assert_task_receipt(
        reap_receipts,
        tasks_by_id["beta"],
        run_id=run_id,
        slot_id=slot_id,
        artifact_root=reap_root,
        instruction_body=instruction_body,
    )
    reap_workers = _load_summary_workers(reap_capture)
    reap_ids = ["alpha", "beta"]
    _assert_capture_files(reap_capture, reap_ids)
    if [worker.get("exit_code") for worker in reap_workers] != [1, 0]:
        raise WorkSlotJourneyFailure(
            f"reap summary must list alpha then beta exits: {reap_workers}"
        )
    if (reap_capture / "gamma").exists():
        raise WorkSlotJourneyFailure("reap path captured unstarted dependent task")

    return [
        "PATH rewrite software-change -> built provider",
        "implement args [run-plan-graph, --task-worker, dummy-json]",
        "small plan two independent plus one dependent",
        "success path exits 0 when implementation-report.json exists",
        "dummy receipts match locked inner stdin layout",
        "capture_dir summary.json and per-task stdout/stderr",
        "missing implementation-report.json exits nonzero",
        "failing sibling reaped before runner exits",
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
    expected_bound = (
        f"{instruction_body}\n\n"
        f"run_id: {run_id}\n"
        f"slot_id: {slot_id}\n"
        f"artifact_root: {artifact_root}\n"
    ).encode("utf-8")
    recorded_a = receipt_a.read_bytes()
    recorded_b = receipt_b.read_bytes()
    if recorded_a != expected_bound or recorded_b != expected_bound:
        raise WorkSlotJourneyFailure(
            f"bound fan-out stdin mismatch: {recorded_a!r} / {recorded_b!r}"
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

    return [
        "PATH rewrite loop-engine -> built engine",
        "bound mode dummy workers record locked shared stdin",
        "ad hoc mode dummy workers record exact --instructions bytes",
        "fan-out reaps every worker before exit",
        "bound outputs under packet.capture_dir",
        "inner nonzero exit with collector exit 0",
        "ad hoc summary.json present",
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
    artifact_root = work_dir / "artifacts"
    capture_dir = work_dir / "captures" / "inv-default-pi"
    plan = _write_small_plan(artifact_root)
    task_ids = [task["id"] for task in plan["tasks"]]
    (artifact_root / "implementation-report.json").write_text("{}\n", encoding="utf-8")
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
        {"command": str(provider), "args": list(SHIPPED_IMPLEMENT_ARGS)},
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
    if len(logs) != len(task_ids):
        raise WorkSlotJourneyFailure(
            f"PATH stub pi invocations {len(logs)} != tasks {len(task_ids)}"
        )
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
                stdin_worker_cli(work_dir / "inner-ok.stdin"),
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
    first = invoke_until_succeeded(
        engine_call, run_id, "design-review", timeout_s=20.0
    )
    _assert_inner_exit_codes(first, (7, 0))
    _assert_capture_files(Path(first["capture_dir"]), ("0", "1"))
    second = invoke_until_succeeded(
        engine_call, run_id, "design-review", timeout_s=20.0
    )
    _assert_inner_exit_codes(second, (7, 0))
    _assert_capture_isolation(first, second, ("0", "1"))
    return [
        "show heartbeat overlay_meaning elapsed_ms remaining_allowed_ms capture_dir inner_workers",
        "inner nonzero with collector 0 yields overlay succeeded",
        "second invoke uses a new invocation-id capture_dir and leaves the first intact",
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
    refusal = "I refuse to produce the contracted review judgment.\n"
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
    if not isinstance(collector_exit, int) or collector_exit == 0:
        raise WorkSlotJourneyFailure(
            f"nonconforming collector must exit nonzero: {failed}"
        )
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
    work_dir: Path,
) -> list[str]:
    """Bound run-plan-graph invoke: inner workers in task order plus capture isolation."""
    work_dir.mkdir(parents=True, exist_ok=True)
    run_id = "bound-graph-runner-heartbeat"
    extra = {
        "implement": implement_graph_runner_binding(
            provider=provider,
            task_worker=stdin_worker_cli(work_dir / "task-receipts"),
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
        target="implement",
    )
    plan = json.loads((artifact_root / "plan.json").read_text(encoding="utf-8"))
    if not isinstance(plan, dict) or not isinstance(plan.get("tasks"), list):
        raise WorkSlotJourneyFailure("isolated plan.json is not a plan document")
    task_ids = [task["id"] for task in plan["tasks"] if isinstance(task, dict)]
    if not task_ids or any(not isinstance(task_id, str) for task_id in task_ids):
        raise WorkSlotJourneyFailure(f"isolated plan.json omitted task ids: {plan}")
    first = invoke_until_succeeded(
        engine_call, run_id, "implement", timeout_s=30.0
    )
    inner = first.get("inner_workers")
    if not isinstance(inner, list) or len(inner) != len(task_ids):
        raise WorkSlotJourneyFailure(
            f"implement inner_workers length {inner} != plan tasks {task_ids}"
        )
    _assert_inner_exit_codes(first, [0] * len(task_ids))
    _assert_capture_files(Path(first["capture_dir"]), task_ids)
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
        engine_call, run_id, "implement", timeout_s=30.0
    )
    _assert_capture_isolation(first, second, task_ids)
    return [
        "bound run-plan-graph show inner workers in plan task order",
        "capture_dir isolation on implement retry",
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
        "explore-intent": {"command": sys.executable, "args": [str(WORKER_SCRIPT)]},
    }
    try:
        assert_shipped_path_names(
            {
                key: value
                for key, value in caller_supplied.items()
                if key != "explore-intent"
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
    if rewritten["explore-intent"]["command"] != sys.executable:
        raise WorkSlotJourneyFailure("self-test: non-PATH command was rewritten")
    if rewritten["implement"]["args"] != SHIPPED_IMPLEMENT_ARGS:
        raise WorkSlotJourneyFailure("self-test: rewrite mutated implement args")

    binding = implement_graph_runner_binding(
        provider=provider,
        task_worker=stdin_worker_cli(Path("/tmp/receipts")),
    )
    if binding["command"] != str(provider):
        raise WorkSlotJourneyFailure("self-test: graph-runner command not rewritten")
    if binding["args"][0] != "run-plan-graph" or binding["args"][1] != "--task-worker":
        raise WorkSlotJourneyFailure(f"self-test: unexpected graph-runner args {binding['args']}")
    parsed_worker = json.loads(binding["args"][2])
    if parsed_worker["command"] != sys.executable:
        raise WorkSlotJourneyFailure("self-test: task-worker is not the dummy python")
    if "dummy-stdin-worker.py" not in parsed_worker["args"][0]:
        raise WorkSlotJourneyFailure("self-test: task-worker is not dummy-stdin-worker.py")

    zero = fan_out_binding(engine=engine, workers=())
    if zero != {"command": str(engine), "args": ["fan-out"]}:
        raise WorkSlotJourneyFailure(f"self-test: zero-worker binding mismatch: {zero}")

    layout = (
        "run_id: run-1\n"
        "slot_id: implement\n"
        "artifact_root: /tmp/artifacts\n"
        "\n## instruction_body\n"
        "Do the work\n"
        "\n## task\n"
        '{"id":"alpha"}\n'
    )
    parsed = parse_graph_runner_stdin(layout)
    if parsed["task"] != {"id": "alpha"} or parsed["instruction_body"] != "Do the work":
        raise WorkSlotJourneyFailure(f"self-test: stdin layout parse mismatch: {parsed}")

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

    preamble = "Read-only reviewer. Return only the contracted judgment."
    artifact_root = Path("/tmp/artifacts")
    body = (
        "Judge the assigned axis.\n\n"
        "run_id: run-1\n"
        "slot_id: design-review\n"
        f"artifact_root: {artifact_root}\n"
    )
    context = b'{"artifact_root":"/tmp/artifacts"}\n'
    raw = preamble.encode("utf-8") + b"\n" + context + BOUND_PREAMBLE_SEPARATOR + body.encode("utf-8")
    assert_bound_preamble_stdin(
        raw,
        preamble=preamble,
        artifact_root=artifact_root,
        run_id="run-1",
        slot_id="design-review",
    )
    duplicate = (
        preamble.encode("utf-8")
        + b"\n"
        + b'{"artifact_root":"/tmp/artifacts","capture_dir":"/tmp/cap"}\n'
        + BOUND_PREAMBLE_SEPARATOR
        + body.encode("utf-8")
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
