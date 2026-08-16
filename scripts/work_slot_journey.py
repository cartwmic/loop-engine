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


def assert_shipped_path_names(bindings: Mapping[str, Any]) -> None:
    implement = bindings.get("implement")
    if not isinstance(implement, dict):
        raise WorkSlotJourneyFailure("shipped bindings omitted implement")
    if implement.get("command") != PATH_SOFTWARE_CHANGE or implement.get("args") != SHIPPED_IMPLEMENT_ARGS:
        raise WorkSlotJourneyFailure(f"shipped implement binding mismatch: {implement}")
    for slot_id in SHIPPED_REVIEW_SLOT_IDS:
        review = bindings.get(slot_id)
        if not isinstance(review, dict):
            raise WorkSlotJourneyFailure(f"shipped bindings omitted {slot_id}")
        if review.get("command") != PATH_LOOP_ENGINE or review.get("args") != SHIPPED_FAN_OUT_ARGS:
            raise WorkSlotJourneyFailure(f"shipped {slot_id} binding mismatch: {review}")
    if "validate" in bindings:
        raise WorkSlotJourneyFailure("shipped bindings unexpectedly bound validate")


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
        review = bindings.get(slot_id)
        if not isinstance(review, dict) or review.get("command") != str(engine):
            raise WorkSlotJourneyFailure(
                f"rewritten {slot_id} command {review} != {engine}"
            )
        if review.get("args") != SHIPPED_FAN_OUT_ARGS:
            raise WorkSlotJourneyFailure(
                f"PATH rewrite must not change shipped {slot_id} args: {review.get('args')}"
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
        args.extend(["--worker", worker_cli_json(worker)])
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
    )


def _invoke_packet(
    *,
    run_id: str,
    slot_id: str,
    artifact_root: Path,
    instruction_body: str,
) -> bytes:
    return json.dumps(
        {
            "run_id": run_id,
            "slot_id": slot_id,
            "artifact_root": str(artifact_root),
            "instruction_body": instruction_body,
        },
        separators=(",", ":"),
    ).encode("utf-8")


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
    tasks_by_id = {task["id"]: task for task in plan["tasks"]}
    run_id = "graph-runner-proof"
    slot_id = "implement"
    instruction_body = "Implement the small fixture plan."

    success_root = work_dir / "success"
    success_receipts = success_root / "receipts"
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
        ),
    )
    if success.returncode != 0:
        raise WorkSlotJourneyFailure(
            "graph-runner success path exited "
            f"{success.returncode}: {success.stderr.decode('utf-8', 'replace')}"
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
        ),
    )
    if missing.returncode == 0:
        raise WorkSlotJourneyFailure(
            "graph-runner missing implementation-report.json unexpectedly exited 0"
        )
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

    return [
        "PATH rewrite software-change -> built provider",
        "implement args [run-plan-graph, --task-worker, dummy-json]",
        "small plan two independent plus one dependent",
        "success path exits 0 when implementation-report.json exists",
        "dummy receipts match locked inner stdin layout",
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
    output_dir = artifact_root / "fan-out" / slot_id
    for index in ("0", "1"):
        if not (output_dir / index / "stdout").is_file():
            raise WorkSlotJourneyFailure(
                f"missing bound stdout under {output_dir / index}"
            )
        if not (output_dir / index / "stderr").is_file():
            raise WorkSlotJourneyFailure(
                f"missing bound stderr under {output_dir / index}"
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

    return [
        "PATH rewrite loop-engine -> built engine",
        "bound mode dummy workers record locked shared stdin",
        "ad hoc mode dummy workers record exact --instructions bytes",
        "fan-out reaps every worker before exit",
        "bound outputs under {artifact_root}/fan-out/{slot_id}/",
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


def prove_zero_worker_review_invoke(
    *,
    engine: Path,
    provider: Path,
    profile_source: Path,
    fixture_root: Path,
    work_dir: Path,
) -> list[str]:
    """Isolated engine run: bound review fan-out with zero --worker fails closed."""
    work_dir.mkdir(parents=True, exist_ok=True)
    artifact_root = work_dir / "artifacts"
    artifact_root.mkdir()
    for subject, fixture in (
        ("intent.json", "intent-good.json"),
        ("design.json", "design-good.json"),
        ("plan.json", "plan-good.json"),
        ("implementation-report.json", "implementation-report-good.json"),
        ("validation-report.json", "validation-report-good.json"),
    ):
        shutil.copy2(fixture_root / fixture, artifact_root / subject)

    profile = json.loads(profile_source.read_text(encoding="utf-8"))
    if not isinstance(profile, dict):
        raise WorkSlotJourneyFailure("zero-worker profile is not an object")
    profile["artifact_root"] = str(artifact_root)
    explore = bindings_for(["explore-intent"])
    review = fan_out_binding(engine=engine, workers=())
    if review["args"] != SHIPPED_FAN_OUT_ARGS:
        raise WorkSlotJourneyFailure(
            f"zero-worker review args must stay [fan-out], got {review['args']}"
        )
    profile["work_slot_bindings"] = {
        **explore,
        "design-review": review,
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
    run_id = "zero-worker-review-invoke"

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
            "zero-worker review invoke",
        ],
    )
    if started.get("status") != "completed":
        raise WorkSlotJourneyFailure(f"zero-worker start failed: {started}")

    invoke_until_succeeded(engine_call, run_id, "explore-intent", timeout_s=15.0)

    intent = json.loads((artifact_root / "intent.json").read_text(encoding="utf-8"))
    revision = intent.get("revision")
    if not isinstance(revision, str) or not revision:
        raise WorkSlotJourneyFailure("intent fixture omitted revision")
    axes = profile.get("review_policies", {}).get("intent")
    if not isinstance(axes, list):
        raise WorkSlotJourneyFailure("profile omitted intent review_policies")
    for entry in axes:
        if not isinstance(entry, dict) or not isinstance(entry.get("id"), str):
            raise WorkSlotJourneyFailure(f"malformed intent axis: {entry}")
        axis = entry["id"]
        for suffix in ("a", "b"):
            record_id = f"zero-worker-intent-{axis}-{suffix}"
            data = {
                "gate": "intent",
                "policy_id": axis,
                "result": "pass",
                "findings": "",
                "author": {
                    "name": f"synthetic-intent-{axis}-{suffix}",
                    "kind": "script",
                },
                "subject": "intent.json",
                "subject_revision": revision,
                "config_version": profile["config_version"],
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
                    f"zero-worker evidence append failed: {appended}"
                )

    intent_ready = engine_call(["event", run_id, "intent-ready"])
    if intent_ready.get("status") != "completed":
        raise WorkSlotJourneyFailure(f"intent-ready failed: {intent_ready}")
    design_ready = engine_call(["event", run_id, "design-ready"])
    if design_ready.get("status") != "completed":
        raise WorkSlotJourneyFailure(f"design-ready failed: {design_ready}")

    shown = engine_call(["show", run_id])
    if shown.get("status") != "completed":
        raise WorkSlotJourneyFailure(f"show before zero-worker invoke failed: {shown}")
    current = (shown.get("result") or {}).get("current_state")
    if current != "design-review":
        raise WorkSlotJourneyFailure(
            f"expected design-review before zero-worker invoke, got {current}"
        )

    overlay = invoke_until_status(
        engine_call,
        run_id,
        "design-review",
        expected="failed",
        timeout_s=15.0,
    )
    if overlay.get("status") == "succeeded":
        raise WorkSlotJourneyFailure(
            "zero-worker review invoke fail-opened with a succeeded overlay"
        )
    return [
        "isolated engine run froze design-review to built loop-engine fan-out",
        "zero --worker entries",
        "invoke failed closed (failed overlay, not succeeded)",
        "no live model",
    ]


def self_test_helpers() -> None:
    """Unit-test PATH rewrite and dummy-stdin-worker without spawning the engine."""
    engine = Path("/tmp/built/loop-engine")
    provider = Path("/tmp/built/software-change")
    shipped = {
        "implement": {"command": PATH_SOFTWARE_CHANGE, "args": list(SHIPPED_IMPLEMENT_ARGS)},
        "design-review": {"command": PATH_LOOP_ENGINE, "args": list(SHIPPED_FAN_OUT_ARGS)},
        "plan-review": {"command": PATH_LOOP_ENGINE, "args": list(SHIPPED_FAN_OUT_ARGS)},
        "implementation-review": {
            "command": PATH_LOOP_ENGINE,
            "args": list(SHIPPED_FAN_OUT_ARGS),
        },
        "explore-intent": {"command": sys.executable, "args": [str(WORKER_SCRIPT)]},
    }
    assert_shipped_path_names(
        {key: value for key, value in shipped.items() if key != "explore-intent"}
    )
    rewritten = rewrite_path_commands(shipped, engine=engine, provider=provider)
    if rewritten["implement"]["command"] != str(provider):
        raise WorkSlotJourneyFailure("self-test: implement PATH was not rewritten")
    if rewritten["design-review"]["command"] != str(engine):
        raise WorkSlotJourneyFailure("self-test: review PATH was not rewritten")
    if rewritten["explore-intent"]["command"] != sys.executable:
        raise WorkSlotJourneyFailure("self-test: non-PATH command was rewritten")
    if rewritten["implement"]["args"] != SHIPPED_IMPLEMENT_ARGS:
        raise WorkSlotJourneyFailure("self-test: rewrite mutated implement args")
    assert_rewritten_binaries(
        {key: value for key, value in rewritten.items() if key != "explore-intent"},
        engine=engine,
        provider=provider,
    )

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
