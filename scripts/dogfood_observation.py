"""Public CLI fixtures for bounded observation and completion persistence."""
from __future__ import annotations

import hashlib
import json
import shutil
import sqlite3
import subprocess
import sys
import time
from pathlib import Path
from typing import Any


def _fresh_root(journey, name: str) -> Path:
    journey.preflight()
    parent = journey.work_root
    parent.parent.mkdir(parents=True, exist_ok=True)
    if parent.exists() and not parent.is_dir():
        raise ValueError(f"{name} scenario work-root is not a directory: {parent}")
    if not parent.exists():
        parent.mkdir()
        root = parent
    elif any(parent.iterdir()):
        # Keep earlier failed captures intact; every attempt gets a fresh leaf.
        root = parent / f"{name}-{time.time_ns()}"
        root.mkdir()
    else:
        root = parent
    journey._dogfood_case_root = root
    return root


def _case_root(journey) -> Path:
    return getattr(journey, "_dogfood_case_root", journey.work_root)


def _full_show_command(argv: list[str]) -> bool:
    try:
        show_index = argv.index("show", 1)
    except ValueError:
        return False
    return any(
        argument == "--view=full"
        or (argument == "--view" and index + 1 < len(argv) and argv[index + 1] == "full")
        for index, argument in enumerate(argv[show_index + 1 :], start=show_index + 1)
    )


def _full_show_projection(stdout: bytes) -> dict[str, Any]:
    try:
        envelope = json.loads(stdout)
    except (UnicodeDecodeError, json.JSONDecodeError):
        return {"available": False}
    result = envelope.get("result") if isinstance(envelope, dict) else None
    if not isinstance(result, dict) or not isinstance(result.get("work_slot_invocations", []), list):
        return {"available": False}
    fields = ("invocation_id", "slot_id", "status", "exit_code", "capture_dir")
    invocations = []
    for row in result.get("work_slot_invocations", []):
        if isinstance(row, dict):
            invocations.append({key: row[key] for key in fields if key in row})
    return {
        "available": True,
        "run_id": result.get("run_id"),
        "current_state": result.get("current_state"),
        "work_slot_invocations": invocations,
    }


def _capture(root: Path, argv: list[str], *, timeout: float = 30, expect: str = "completed") -> tuple[dict[str, Any], float, subprocess.CompletedProcess[bytes]]:
    logs = root / "commands"
    logs.mkdir(exist_ok=True)
    ordinal = len(list(logs.glob("*.argv.json")))
    label = f"{ordinal:03d}"
    started = time.perf_counter_ns()
    try:
        result = subprocess.run(argv, cwd=root, capture_output=True, timeout=timeout, check=False)
    except (OSError, subprocess.TimeoutExpired) as error:
        raise ValueError(f"public command did not complete: {argv!r}: {error}") from error
    elapsed_ms = (time.perf_counter_ns() - started) / 1_000_000
    argv_record: dict[str, Any] = {"argv": argv, "cwd": str(root)}
    if _full_show_command(argv):
        projection_name = f"{label}.stdout-projection.json"
        argv_record["stdout_capture"] = {
            "kind": "bounded-full-show-projection",
            "path": projection_name,
            "raw_stdout_retained": False,
        }
        projection = {
            "kind": "loop-engine-full-show-projection-v1",
            "raw_stdout_retained": False,
            "original_stdout": {
                "byte_length": len(result.stdout),
                "sha256": "sha256:" + hashlib.sha256(result.stdout).hexdigest(),
            },
            "projection": _full_show_projection(result.stdout),
        }
        (logs / projection_name).write_text(json.dumps(projection, indent=2) + "\n", encoding="utf-8")
    else:
        (logs / f"{label}.stdout").write_bytes(result.stdout)
    (logs / f"{label}.argv.json").write_text(json.dumps(argv_record) + "\n")
    (logs / f"{label}.stderr").write_bytes(result.stderr)
    (logs / f"{label}.exit.json").write_text(json.dumps({"returncode": result.returncode, "elapsed_ms": elapsed_ms}) + "\n")
    try:
        envelope = json.loads(result.stdout)
    except Exception as error:
        raise ValueError(f"public command returned non-JSON: {argv!r}: {result.stderr[-1000:]!r}") from error
    if expect == "completed" and (result.returncode != 0 or envelope.get("status") != "completed"):
        raise ValueError(f"public command did not complete: {argv!r}: {envelope!r} {result.stderr[-1000:]!r}")
    return envelope, elapsed_ms, result


def _engine(journey, database: Path, *args: str, timeout: float = 30, expect: str = "completed"):
    return _capture(_case_root(journey), [str(journey.engine), "--database", str(database), "--json", *args], timeout=timeout, expect=expect)


def _read(journey, database: Path, run_id: str, kind: str, *options: str, expect: str = "completed"):
    return _capture(_case_root(journey), [str(journey.engine), "read", "--database", str(database), "--json",
        run_id, "--kind", kind, *options], timeout=4, expect=expect)


def _assert_native_boot_session_identity(
    journey, database: Path, run_id: str, invocation_id: str, root: Path
) -> dict[str, Any] | None:
    if sys.platform != "darwin":
        return None

    command = ["sysctl", "-n", "kern.bootsessionuuid"]
    started = time.perf_counter_ns()
    result = subprocess.run(command, cwd=root, capture_output=True, timeout=4, check=False)
    elapsed_ms = (time.perf_counter_ns() - started) / 1_000_000
    logs = root / "commands"
    logs.mkdir(exist_ok=True)
    ordinal = len(list(logs.glob("*.argv.json")))
    label = f"{ordinal:03d}"
    (logs / f"{label}.argv.json").write_text(json.dumps({"argv": command, "cwd": str(root)}) + "\n")
    (logs / f"{label}.stdout").write_bytes(result.stdout)
    (logs / f"{label}.stderr").write_bytes(result.stderr)
    (logs / f"{label}.exit.json").write_text(json.dumps({"returncode": result.returncode, "elapsed_ms": elapsed_ms}) + "\n")
    if result.returncode != 0:
        raise ValueError(f"native boot-session query failed: {result.stderr[-1000:]!r}")
    native_boot_id = result.stdout.decode("ascii").strip().lower()
    if not native_boot_id:
        raise ValueError("native boot-session UUID was empty")

    live_row = None
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        status, elapsed, _ = _show_status(journey, database, run_id)
        if elapsed > 2000:
            raise ValueError(f"boot-identity status exceeded two seconds: {elapsed:.1f}ms")
        live_row = next((row for row in status["result"]["invocations"]["items"]
                         if row["invocation_id"] == invocation_id), None)
        if live_row and live_row["execution"]["state"] == "running":
            break
        time.sleep(.05)
    if not live_row or live_row["execution"]["state"] != "running":
        raise ValueError(f"public status did not retain the live bound invocation: {live_row}")

    progress, elapsed, _ = _engine(journey, database, "invocation-progress", run_id, invocation_id)
    if elapsed > 2000:
        raise ValueError(f"live invocation progress exceeded two seconds: {elapsed:.1f}ms")
    result = progress["result"]
    ownership = result.get("ownership") or {}
    execution = ownership.get("execution") or {}
    identity = execution.get("root_identity") or {}
    if identity.get("boot_id", "").lower() != native_boot_id:
        raise ValueError(f"persisted ownership boot identity differs from native session UUID: {identity}")
    if (
        identity.get("pid") != execution.get("root_pid")
        or not isinstance(identity.get("start_time"), int)
        or identity["start_time"] <= 0
    ):
        raise ValueError(f"persisted root PID/start identity is incomplete: {identity}")
    if (
        ownership.get("live_owned_work") is not True
        or result.get("visibility", {}).get("execution", {}).get("state") != "running"
    ):
        raise ValueError(f"native identity did not retain genuinely live owned work: {result}")
    return {
        "boot_session_uuid": native_boot_id,
        "root_pid": identity["pid"],
        "root_start_time": identity["start_time"],
        "live_owned_work": True,
        "public_execution_state": result["visibility"]["execution"]["state"],
    }


def _assert_full_show_retention(root: Path) -> int:
    logs = root / "commands"
    count = 0
    for argv_path in sorted(logs.glob("*.argv.json")):
        record = json.loads(argv_path.read_text(encoding="utf-8"))
        argv = record.get("argv")
        if not isinstance(argv, list) or not _full_show_command(argv):
            continue
        count += 1
        label = argv_path.name.removesuffix(".argv.json")
        if (logs / f"{label}.stdout").exists():
            raise ValueError("full show stdout was retained as an unbounded original stream")
        if record.get("stdout_capture") != {
            "kind": "bounded-full-show-projection",
            "path": f"{label}.stdout-projection.json",
            "raw_stdout_retained": False,
        }:
            raise ValueError("full show stdout was not labeled as a bounded projection")
        projection = json.loads((logs / f"{label}.stdout-projection.json").read_text(encoding="utf-8"))
        original = projection.get("original_stdout", {})
        facts = projection.get("projection", {})
        if (
            projection.get("kind") != "loop-engine-full-show-projection-v1"
            or projection.get("raw_stdout_retained") is not False
            or not isinstance(original.get("byte_length"), int)
            or original["byte_length"] <= 0
            or not isinstance(original.get("sha256"), str)
            or len(original["sha256"]) != 71
            or not original["sha256"].startswith("sha256:")
            or set(facts) != {"available", "run_id", "current_state", "work_slot_invocations"}
            or facts.get("available") is not True
        ):
            raise ValueError("full show retention omitted original byte facts or bounded projection")
        if not (logs / f"{label}.stderr").is_file() or not (logs / f"{label}.exit.json").is_file():
            raise ValueError("full show retention omitted stderr or actual command exit")
    if count == 0:
        raise ValueError("persistence fixture did not exercise full-show retention")
    if list(root.rglob("show-*.json")):
        raise ValueError("persistence fixture persisted a full show envelope")
    return count


def _show_status(journey, database: Path, run_id: str):
    return _engine(journey, database, "show", "--view", "status", run_id)


def _provider_and_run(journey, root: Path, slots: list[str], bindings: dict[str, Any], run_id: str):
    provider = root / "fixture-provider.py"
    provider.write_text(
        "import json,sys\n"
        "request=json.load(sys.stdin)\n"
        "if request.get('operation')=='describe':\n"
        "  initial=request.get('initial_input') or {}; names=initial.get('fixture_slots',[])\n"
        "  states=[{'id':'work','title':'Fixture work','instructions':'Inspect captured assignments.','final':False},{'id':'done','title':'Done','instructions':'Finished.','final':True}]\n"
        "  transitions=[{'source':'work','event':'revisit','target':'work','kind':'check-free'},{'source':'work','event':'finish','target':'done','kind':'check-free'}]\n"
        "  slots=[{'id':name,'state':'work','event':'revisit'} for name in names]\n"
        "  print(json.dumps({'id':'dogfood-observation-v1','initial_state':'work','states':states,'transitions':transitions,'work_slots':slots}))\n"
        "else: print(json.dumps({'result':'allow'}))\n",
        encoding="utf-8",
    )
    config = root / "providers.toml"
    config.write_text(
        "[providers.fixture]\n"
        f"command = {json.dumps(sys.executable)}\n"
        f"args = [{json.dumps(str(provider))}]\n",
        encoding="utf-8",
    )
    database = root / "loop.sqlite"
    artifacts = root / "artifacts"
    artifacts.mkdir(exist_ok=True)
    initial = {"artifact_root": str(artifacts), "fixture_slots": slots, "work_slot_bindings": bindings}
    _capture(_case_root(journey), [str(journey.engine), "--database", str(database), "--config", str(config), "--json",
        "start", "--id", run_id, "fixture", json.dumps(initial)], timeout=30)
    return database, artifacts


def _worker_command(script: Path, mode: str) -> dict[str, Any]:
    return {"command": sys.executable, "args": [str(script), mode]}


def _fanout_binding(engine: Path, workers: list[dict[str, Any]], *, max_active: int = 1) -> dict[str, Any]:
    args = ["fan-out", "--max-active", str(max_active)]
    for worker in workers:
        args.extend(["--worker", json.dumps(worker, separators=(",", ":"))])
    return {"command": str(engine), "args": args}


def _plan_graph_label_case(journey, root: Path) -> None:
    fixture = root / "plan-graph-labels"
    fixture.mkdir()
    wrapper = fixture / "graph-wrapper.py"
    wrapper.write_text(
        "#!/usr/bin/env python3\n"
        "import json,pathlib,sys,time\n"
        "labels=[{'assignment_id':'task-17','title':'Scoped implementation task','role':'implementer'},{'assignment_id':'summarizer','title':'Implementation report','role':'summarizer'}]\n"
        "if len(sys.argv)>1 and sys.argv[1]=='run-plan-graph':\n"
        " request=json.load(sys.stdin)\n"
        " if request.get('preview'): print(json.dumps({'prepared':True,'assignment_labels':labels}))\n"
        " else:\n"
        "  time.sleep(1.0); capture=pathlib.Path(request['capture_dir']); capture.mkdir(parents=True,exist_ok=True)\n"
        "  (capture/'summary.json').write_text(json.dumps({'workers':[{'assignment_id':x['assignment_id'],'command':'scripted-task','args':[],'exit_code':0,'task_definition':{'id':x['assignment_id']}} for x in labels]}))\n"
        "  print('{}')\n"
        "else:\n"
        " request=json.load(sys.stdin)\n"
        " if request.get('operation')=='describe':\n"
        "  print(json.dumps({'id':'plan-label-provider','initial_state':'work','states':[{'id':'work','title':'Work','instructions':'Run the plan graph.','final':False},{'id':'done','title':'Done','instructions':'Done.','final':True}],'transitions':[{'source':'work','event':'finish','target':'done','kind':'check-free'}],'work_slots':[{'id':'implement','state':'work','event':'finish'}]}))\n"
        " else: print(json.dumps({'result':'allow'}))\n",
        encoding="utf-8",
    )
    wrapper.chmod(0o755)
    config = fixture / "providers.toml"
    config.write_text(f"[providers.fixture]\ncommand = {json.dumps(str(wrapper))}\nargs = []\n")
    database = fixture / "loop.sqlite"
    artifacts = fixture / "artifacts"
    artifacts.mkdir()
    binding = {"command": str(wrapper), "args": ["run-plan-graph", "--working-directory", str(fixture)]}
    initial = {"artifact_root": str(artifacts), "work_slot_bindings": {"implement": binding}}
    _capture(_case_root(journey), [str(journey.engine), "--database", str(database), "--config", str(config), "--json",
        "start", "--id", "plan-label-run", "fixture", json.dumps(initial)])
    _engine(journey, database, "show", "--view", "action", "plan-label-run")
    preview, _, _ = _engine(journey, database, "invoke", "plan-label-run", "implement", "--preview")
    prepared = preview["result"]["facade_preparation"].get("assignment_labels")
    if prepared != [
        {"assignment_id": "task-17", "title": "Scoped implementation task", "role": "implementer"},
        {"assignment_id": "summarizer", "title": "Implementation report", "role": "summarizer"},
    ]:
        raise ValueError(f"plan graph did not supply frozen descriptors before output: {prepared}")
    invoked, _, _ = _engine(journey, database, "--timeout-ms", "120000", "invoke", "plan-label-run", "implement")
    invocation_id = invoked["result"]["invocation_id"]
    sampled = None
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        sampled, elapsed, _ = _show_status(journey, database, "plan-label-run")
        if elapsed > 2000:
            raise ValueError(f"plan graph label status exceeded two seconds: {elapsed}ms")
        items = sampled["result"].get("assignments", {}).get("items", [])
        if len(items) == 2:
            break
        time.sleep(.03)
    labels = {(item["assignment_id"], item["title"], item["role"]) for item in sampled["result"]["assignments"]["items"]}
    expected = {("task-17", "Scoped implementation task", "implementer"), ("summarizer", "Implementation report", "summarizer")}
    if labels != expected:
        raise ValueError(f"plan graph pre-output labels were not persisted by stable ID: {labels}")
    if any(item["state"] != "unknown" or item["acceptance"]["state"] != "unknown" for item in sampled["result"]["assignments"]["items"]):
        raise ValueError("configured plan labels were promoted without execution evidence")
    full, _, _ = _engine(journey, database, "show", "--view", "full", "plan-label-run")
    stored = next(row for row in full["result"]["work_slot_invocations"] if row["invocation_id"] == invocation_id)
    if stored.get("assignment_labels") != prepared:
        raise ValueError("full show did not retain frozen pre-output plan labels")


def observation_case(journey) -> None:
    root = _fresh_root(journey, "sol-observation")
    _plan_graph_label_case(journey, root)
    if not shutil.which("dagu"):
        raise ValueError("sol-observation requires the operator-provided Dagu binary on PATH")
    worker = root / "scripted-worker.py"
    worker.write_text(
        "import json,sys,time\n"
        "mode=sys.argv[1]; raw=sys.stdin.buffer.read()\n"
        "if mode=='large':\n"
        "  time.sleep(.25); print(json.dumps({'result':'x'*90000}),flush=True)\n"
        "elif mode=='invalid':\n"
        "  time.sleep(1.5 if b'SCHEMA-CONFORMANCE RETRY' in raw else .35); sys.stderr.write('scripted validation failure\\n'); sys.stderr.flush(); print(json.dumps({'wrong':'value'}),flush=True)\n"
        "elif mode=='quiet':\n"
        "  time.sleep(4.0)\n"
        "elif mode=='long':\n"
        "  time.sleep(30)\n"
        "elif mode=='fail':\n"
        "  sys.stderr.write('scripted inner failure diagnostic\\n'); sys.stderr.flush(); raise SystemExit(7)\n"
        "else: print(json.dumps({'result':'ok'}),flush=True)\n",
        encoding="utf-8",
    )
    required = {"required": ["result"]}
    full_schema = {"type": "object", "required": ["result"], "properties": {"result": {"type": "string"}}}
    review_workers = [
        {**_worker_command(worker, "large"), "title": "Axis A", "role": "reviewer Sol", "output_schema": required},
        {**_worker_command(worker, "invalid"), "title": "Axis B", "role": "reviewer Astra", "full_output_schema": full_schema},
        {**_worker_command(worker, "quiet"), "title": "Silent review", "role": "quiet reviewer"},
    ]
    quiet_workers = [{**_worker_command(worker, "quiet"), "title": "Quiet task", "role": "implementer"}]
    cancel_workers = [{**_worker_command(worker, "long"), "title": "Cancelled task", "role": "implementer"}]
    bindings = {
        "review": _fanout_binding(journey.engine, review_workers, max_active=1),
        "quiet": _fanout_binding(journey.engine, quiet_workers),
        "cancel": _fanout_binding(journey.engine, cancel_workers),
    }
    database, _ = _provider_and_run(journey, root, ["review", "quiet", "cancel"], bindings, "observation-run")
    configured, _, _ = _show_status(journey, database, "observation-run")
    configured_items = configured["result"].get("assignments", {}).get("items", [])
    if len(configured_items) != 5 or any(item.get("state") != "configured" for item in configured_items):
        raise ValueError(f"pre-invocation status invented queued work or omitted the configured list: {configured_items}")
    action, _, _ = _engine(journey, database, "show", "--view", "action", "observation-run")
    action_result = action["result"]
    if action_result.get("mutation_armed") is not True or action_result.get("next_action", {}).get("slot_id") != "review":
        raise ValueError(f"bounded action view did not expose the correct permitted slot route: {action_result}")
    if not action_result.get("locators", {}).get("full") or "loop-engine invoke observation-run review" not in action_result.get("current_state_instructions", ""):
        raise ValueError("bounded action view omitted bound-work instructions or full locator")
    review_invoke, _, _ = _engine(journey, database, "--timeout-ms", "120000", "invoke", "observation-run", "review")
    review_id = review_invoke["result"]["invocation_id"]
    _engine(journey, database, "show", "--view", "action", "observation-run")
    quiet_invoke, _, _ = _engine(journey, database, "--timeout-ms", "120000", "invoke", "observation-run", "quiet")
    quiet_id = quiet_invoke["result"]["invocation_id"]
    _engine(journey, database, "show", "--view", "action", "observation-run")
    cancel_invoke, _, _ = _engine(journey, database, "--timeout-ms", "120000", "invoke", "observation-run", "cancel")
    cancel_id = cancel_invoke["result"]["invocation_id"]
    native_boot_observation = _assert_native_boot_session_identity(
        journey, database, "observation-run", cancel_id, root
    )

    monitor_argv = [str(journey.engine), "monitor", "--run", "observation-run", "--database", str(database),
                    "--json", "--poll-seconds", "0.05"]
    (root / "monitor.argv.json").write_text(json.dumps({"argv": monitor_argv, "cwd": str(root)}) + "\n")
    monitor = subprocess.Popen(monitor_argv, cwd=_case_root(journey), stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    try:
        time.sleep(.5)
    finally:
        monitor.terminate()
    monitor_stdout, monitor_stderr = monitor.communicate(timeout=4)
    (root / "monitor.stdout.jsonl").write_bytes(monitor_stdout)
    (root / "monitor.stderr").write_bytes(monitor_stderr)
    monitor_lines = [line for line in monitor_stdout.splitlines() if line.strip()]
    if not monitor_lines:
        raise ValueError(f"passive monitor emitted no bounded status sample: {monitor_stderr[-1000:]!r}")
    monitor_packet = json.loads(monitor_lines[0])
    if monitor_packet.get("source") != "run:observation-run" or not isinstance(monitor_packet.get("workflow"), dict):
        raise ValueError(f"monitor did not consume the versioned public status view: {monitor_packet}")

    observed_progress = []
    quiet_unknown_seen = False
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        status, elapsed_ms, _ = _show_status(journey, database, "observation-run")
        if elapsed_ms > 2000:
            raise ValueError(f"public status exceeded two seconds: {elapsed_ms:.1f}ms")
        result = status["result"]
        if result.get("completeness", {}).get("assignments") == "available":
            items = result.get("assignments", {}).get("items", [])
            observed_progress.extend(item.get("state") for item in items)
            quiet_item = next((item for item in items if item.get("slot_id") == "quiet"), None)
            quiet_unknown_seen |= bool(quiet_item and quiet_item["execution"]["state"] in ("running", "unknown")
                                       and quiet_item["conformance"]["state"] == "unknown"
                                       and quiet_item["acceptance"]["state"] == "unknown")
            correcting_worker = next((item for item in items if item.get("slot_id") == "review" and item.get("assignment_id") == "worker-1"), None)
            if correcting_worker and correcting_worker["state"] in ("queued", "running", "correcting") and correcting_worker["conformance"].get("attempt_count") is not None:
                raise ValueError("status invented an exact attempt count before completion facts were committed")
            if "queued" in observed_progress and "running" in observed_progress:
                break
        time.sleep(.05)
    if "queued" not in observed_progress or "running" not in observed_progress:
        raise ValueError(f"public status did not expose sampled queued/running assignment facts: {observed_progress}")
    if not quiet_unknown_seen:
        raise ValueError("quiet assignment did not remain semantically unknown during its live sample")

    correction_seen = False
    review_final = None
    deadline = time.monotonic() + 12
    while time.monotonic() < deadline:
        status, elapsed_ms, _ = _show_status(journey, database, "observation-run")
        if elapsed_ms > 2000:
            raise ValueError(f"public status exceeded two seconds during correction: {elapsed_ms:.1f}ms")
        items = status["result"].get("assignments", {}).get("items", [])
        observed_progress.extend(item.get("state") for item in items)
        correction_seen |= any(item.get("assignment_id") == "worker-1" and item.get("state") == "correcting" for item in items)
        invocation_rows = status["result"].get("invocations", {}).get("items", [])
        review_final = next((row for row in invocation_rows if row.get("invocation_id") == review_id), None)
        if review_final and review_final.get("execution", {}).get("state") in ("failed", "succeeded"):
            break
        time.sleep(.05)
    if not review_final or review_final.get("execution", {}).get("state") != "failed":
        raise ValueError("scripted invalid review did not retain a failed outer invocation")
    if not correction_seen:
        raise ValueError("bounded status did not distinguish the active schema-correction attempt")
    if "finished" not in observed_progress:
        raise ValueError("bounded status did not retain a sampled completed helper state")

    # A silent but live worker is not called dead or semantically failed.
    quiet_status, _, _ = _show_status(journey, database, "observation-run")
    quiet_assignment = next(row for row in quiet_status["result"]["assignments"]["items"] if row["slot_id"] == "quiet")
    if quiet_assignment["conformance"]["state"] != "not-checked" or quiet_assignment["acceptance"]["state"] != "unknown":
        raise ValueError("silent worker completion/conformance was confused with semantic acceptance")

    _engine(journey, database, "show", "--view", "action", "observation-run")
    _engine(journey, database, "cancel-invocation", "observation-run", cancel_id, timeout=15)
    cancelled, _, _ = _show_status(journey, database, "observation-run")
    cancelled_row = next(row for row in cancelled["result"]["invocations"]["items"] if row["invocation_id"] == cancel_id)
    if cancelled_row["execution"]["state"] != "cancelled":
        raise ValueError(f"public status omitted verified cancellation: {cancelled_row}")

    _engine(journey, database, "show", "--view", "action", "observation-run")
    _engine(journey, database, "event", "observation-run", "revisit")
    _engine(journey, database, "show", "--view", "action", "observation-run")
    second, _, _ = _engine(journey, database, "--timeout-ms", "120000", "invoke", "observation-run", "review")
    second_id = second["result"]["invocation_id"]
    final = None
    deadline = time.monotonic() + 12
    while time.monotonic() < deadline:
        status, elapsed_ms, _ = _show_status(journey, database, "observation-run")
        if elapsed_ms > 2000:
            raise ValueError(f"public status exceeded two seconds after revision: {elapsed_ms:.1f}ms")
        final = status["result"]
        row = next((item for item in final.get("invocations", {}).get("items", []) if item.get("invocation_id") == second_id), None)
        if row and row.get("execution", {}).get("state") == "failed":
            break
        time.sleep(.05)
    if final is None:
        raise ValueError("current subject status was not sampled")
    current = next(item for item in final["assignments"]["items"] if item["slot_id"] == "review" and item["assignment_id"] == "worker-1")
    if current["conformance"]["attempt_count"] != 2 or current["conformance"]["repeated_failure_count"] != 2:
        raise ValueError(f"current revision counts include old attempts or lost real failures: {current}")
    if current["state"] != "exhausted" or current["acceptance"]["state"] != "unknown":
        raise ValueError(f"conformance was confused with acceptance or exhaustion: {current}")

    human_argv = [str(journey.engine), "--database", str(database), "show", "--view", "status", "observation-run"]
    human_start = time.perf_counter_ns()
    human_process = subprocess.run(human_argv, cwd=_case_root(journey), capture_output=True, timeout=4, check=False)
    human_elapsed_ms = (time.perf_counter_ns() - human_start) / 1_000_000
    if human_process.returncode != 0 or human_elapsed_ms > 2000:
        raise ValueError(f"human status failed or exceeded its bound: {human_elapsed_ms:.1f}ms {human_process.stderr[-1000:]!r}")
    human_text = human_process.stdout.decode(errors="replace")
    human_machine, _, _ = _show_status(journey, database, "observation-run")
    if "Axis B [reviewer Astra] id=worker-1" not in human_text or "attempts=2 failures=2 acceptance=unknown" not in human_text:
        raise ValueError("human status lost frozen assignment identity, exact counts, or separate acceptance")
    machine_item = next(item for item in human_machine["result"]["assignments"]["items"] if item["assignment_id"] == "worker-1")
    if (machine_item["title"], machine_item["role"], machine_item["assignment_id"]) != ("Axis B", "reviewer Astra", "worker-1"):
        raise ValueError("machine status identity differs from human drill-down")

    attempts, _, _ = _read(journey, database, "observation-run", "attempt", "--invocation", second_id,
                           "--assignment", "worker-1", "--limit", "10")
    if attempts["result"]["total"] != 2 or not all(item["failed"] for item in attempts["result"]["items"]):
        raise ValueError(f"targeted current attempt history lost real failures: {attempts}")
    errors, _, _ = _read(journey, database, "observation-run", "error", "--invocation", second_id,
                         "--assignment", "worker-1", "--limit", "10")
    if errors["result"]["total"] != 2:
        raise ValueError(f"targeted error page total is wrong: {errors}")
    all_assignment, _, _ = _read(journey, database, "observation-run", "assignment", "--assignment", "worker-1", "--limit", "10")
    assignment_rows = all_assignment["result"]["items"]
    if all_assignment["result"].get("state") != "available" or all_assignment["result"].get("total") != 2:
        raise ValueError(f"targeted assignment page omitted its exact indexed total: {all_assignment}")
    if any((item["title"], item["role"], item["assignment_id"]) != ("Axis B", "reviewer Astra", "worker-1") for item in assignment_rows):
        raise ValueError("targeted assignment drill-down changed the frozen human/machine identity")
    revisions = {item["subject_revision"] for item in assignment_rows}
    if len(revisions) != 2 or current["subject_revision"] not in revisions:
        raise ValueError(f"older revision assignment history is not separately inspectable: {revisions}")

    full, _, _ = _engine(journey, database, "show", "--view", "full", "observation-run")
    current_invocation = next(row for row in full["result"]["work_slot_invocations"] if row["invocation_id"] == second_id)
    large_invocation = next(row for row in full["result"]["work_slot_invocations"] if row["invocation_id"] == review_id)
    large_worker = next(worker_row for worker_row in large_invocation["inner_workers"] if worker_row["assignment_id"] == "worker-0")
    original_path = Path(large_invocation["capture_dir"]) / large_worker["selected_output_path"]
    original_bytes = original_path.read_bytes()
    chunks = []
    offset = 0
    while True:
        chunk, _, _ = _read(journey, database, "observation-run", "stdout", "--invocation", review_id,
                            "--assignment", "worker-0", "--attempt", "1", "--offset", str(offset), "--limit", "65536")
        page = chunk["result"]
        if page["schema_version"] != 1 or page.get("delta_sequence") is None:
            raise ValueError("stream page omitted its output version or delta sequence")
        if page["offset"] != offset or page["total_bytes"] != len(original_bytes):
            raise ValueError("stdout page omitted exact offset/total")
        chunks.append(bytes.fromhex(page["data"]))
        if not page["truncated"]:
            break
        offset = page["next_offset"]
    if b"".join(chunks) != original_bytes:
        raise ValueError("paged stdout did not reconstruct exact original captured bytes")
    bad_path = Path(current_invocation["capture_dir"]) / "1/attempts/2/stderr"
    expected_stderr = bad_path.read_bytes()
    stderr, _, _ = _read(journey, database, "observation-run", "stderr", "--invocation", second_id,
                          "--assignment", "worker-1", "--attempt", "2", "--offset", "0", "--limit", "256")
    if bytes.fromhex(stderr["result"]["data"]) != expected_stderr:
        raise ValueError("stderr byte-offset page differs from original captured bytes")
    invalid, _, _ = _read(journey, database, "observation-run", "stdout", "--invocation", review_id,
                          "--assignment", "worker-0", "--attempt", "1", "--offset", str(len(original_bytes)+1),
                          "--limit", "10", expect="error")
    if invalid.get("status") != "error" or invalid.get("code") != "invalid-offset":
        raise ValueError("invalid stream offset was not flagged")

    first, _, _ = _read(journey, database, "observation-run", "history", "--cursor", "0", "--limit", "2")
    first_result = first["result"]
    if first_result["total"] <= 2 or not first_result["truncated"] or first_result["next_cursor"] is None:
        raise ValueError(f"semantic history page was not explicitly truncated: {first_result}")
    second_page, _, _ = _read(journey, database, "observation-run", "delta", "--cursor", str(first_result["next_cursor"]), "--limit", "2")
    if second_page["result"]["delta_sequence"] < first_result["delta_sequence"]:
        raise ValueError("delta sequence moved backwards")
    if second_page["result"]["sampled_execution"].get("missed_sample") != "unknown":
        raise ValueError("delta read erased the uncertainty about unsampled execution changes")
    if [item["sequence"] for item in second_page["result"]["items"]] and second_page["result"]["items"][0]["sequence"] <= first_result["next_cursor"]:
        raise ValueError("history cursor did not advance monotonically")

    lock = sqlite3.connect(database, timeout=1)
    lock.execute("PRAGMA journal_mode=DELETE")
    lock.execute("BEGIN EXCLUSIVE")
    try:
        locked, elapsed_ms, _ = _show_status(journey, database, "observation-run")
        if elapsed_ms > 2000 or locked.get("status") != "completed":
            raise ValueError(f"locked public status did not fail closed before two seconds: {elapsed_ms}ms {locked}")
        if locked["result"].get("uncertainty", {}).get("state") != "unavailable":
            raise ValueError(f"locked public status hid unavailable evidence: {locked}")
        if locked["result"].get("mutation_armed") is not False:
            raise ValueError("status read armed mutation while locked")
    finally:
        lock.rollback()
        lock.close()

    (root / "outcome.json").write_text(json.dumps({
        "scenario":"sol-observation","status":"passed","run_id":"observation-run",
        "review_invocations":[review_id,second_id],"quiet_invocation":quiet_id,"cancelled_invocation":cancel_id,
        "current_subject_attempts":current["conformance"]["attempt_count"],
        "current_subject_failures":current["conformance"]["repeated_failure_count"],
        "older_subject_revisions":sorted(revisions),"status_deadline_ms":2000,
        "locked_status_elapsed_ms":elapsed_ms,"original_stdout_bytes":len(original_bytes),
        "monitor_jsonl_samples":len(monitor_lines),
        "native_boot_observation":native_boot_observation,
        "historical_false_failure_diagnostic":"/Volumes/Workshop/macbook/loop-engine/launches/dogfood-backlog-20260924-072525/implementation-boot-identity-diagnostic.json",
        "acceptance":"unknown throughout; no semantic judgment was inferred"
    },indent=2)+"\n")


def persistence_case(journey) -> None:
    root = _fresh_root(journey, "sol-persistence")
    if not shutil.which("dagu"):
        raise ValueError("sol-persistence requires the operator-provided Dagu binary on PATH")
    worker = root / "worker.py"
    worker.write_text(
        "import json,sys\n"
        "worker_id=sys.argv[1]; sys.stdin.buffer.read()\n"
        "if worker_id=='fail':\n"
        "  sys.stderr.write('scripted inner failure diagnostic\\n'); sys.stderr.flush(); raise SystemExit(7)\n"
        "print(json.dumps({'result':worker_id}),flush=True)\n",
        encoding="utf-8",
    )
    workers = [{**_worker_command(worker, str(index)), "title": f"Task {index}", "role": "scripted worker",
                "output_schema": {"required": ["result"]}} for index in range(14)]
    diagnostic_worker = [{**_worker_command(worker, "fail"), "title":"Failing worker", "role":"scripted worker"}]
    bindings = {
        "fanout": _fanout_binding(journey.engine, workers, max_active=4),
        "diagnostic": _fanout_binding(journey.engine, diagnostic_worker),
    }
    database, _ = _provider_and_run(journey, root, ["fanout", "diagnostic"], bindings, "persistence-run")
    _engine(journey, database, "show", "--view", "action", "persistence-run")
    invoke, _, _ = _engine(journey, database, "--timeout-ms", "120000", "invoke", "persistence-run", "fanout")
    invocation_id = invoke["result"]["invocation_id"]
    deadline = time.monotonic() + 30
    final = None
    while time.monotonic() < deadline:
        final, _, _ = _engine(journey, database, "show", "--view", "full", "persistence-run")
        row = next(row for row in final["result"]["work_slot_invocations"] if row["invocation_id"] == invocation_id)
        if row["status"] in ("succeeded", "failed"):
            break
        time.sleep(.05)
    if final is None or row["status"] != "succeeded" or row["exit_code"] != 0 or len(row["inner_workers"]) != 14:
        raise ValueError(f"public 14-worker completion did not preserve succeeded/inner_workers: {row if final else None}")
    if any(worker_row["exit_code"] != 0 or worker_row.get("conformance_status") != "succeeded" for worker_row in row["inner_workers"]):
        raise ValueError("public 14-worker completion lost a worker success/conformance fact")
    status, _, _ = _show_status(journey, database, "persistence-run")
    assignments = [item for item in status["result"].get("assignments", {}).get("items", []) if item["slot_id"] == "fanout"]
    if len(assignments) != 14 or any(item["execution"]["state"] != "succeeded" for item in assignments):
        raise ValueError(f"bounded public assignment projection lost 14 worker facts: {assignments}")

    _engine(journey, database, "show", "--view", "action", "persistence-run")
    diagnostic_started, _, _ = _engine(journey, database, "--timeout-ms", "120000", "invoke", "persistence-run", "diagnostic")
    diagnostic_id = diagnostic_started["result"]["invocation_id"]
    diagnostic_show = None
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        diagnostic_show, _, _ = _engine(journey, database, "show", "--view", "full", "persistence-run")
        diagnostic_row = next(row for row in diagnostic_show["result"]["work_slot_invocations"] if row["invocation_id"] == diagnostic_id)
        if diagnostic_row["status"] in ("succeeded", "failed"):
            break
        time.sleep(.05)
    diagnostic_worker_row = diagnostic_row["inner_workers"][0]
    if diagnostic_row["status"] != "succeeded" or diagnostic_worker_row["exit_code"] != 7:
        raise ValueError(f"public nonzero inner worker diagnostic was not preserved distinctly: {diagnostic_row}")
    stderr, _, _ = _read(journey, database, "persistence-run", "stderr", "--invocation", diagnostic_id,
                          "--assignment", "worker-0", "--attempt", "1", "--offset", "0", "--limit", "128")
    if bytes.fromhex(stderr["result"]["data"]) != b"scripted inner failure diagnostic\n":
        raise ValueError("nonzero inner worker stderr diagnostic was not preserved byte-for-byte")
    full_show_count = _assert_full_show_retention(root)
    (root / "outcome.json").write_text(json.dumps({
        "scenario":"sol-persistence","status":"passed","run_id":"persistence-run",
        "invocation_id":invocation_id,"outer_status":row["status"],"inner_workers":len(row["inner_workers"]),
        "assignment_facts":len(assignments),"diagnostic_invocation":diagnostic_id,
        "inner_failure_exit":diagnostic_worker_row["exit_code"],
        "diagnostic":"outer succeeded remains distinct from preserved inner exit 7 and original stderr bytes",
        "full_show_retention":{"commands":full_show_count,"raw_stdout_retained":False}
    },indent=2)+"\n")
