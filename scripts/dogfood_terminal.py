"""Read-only terminal navigator proof through isolated public CLI fixtures."""
from __future__ import annotations

import fcntl
import json
import os
import pty
import select
import shutil
import struct
import subprocess
import termios
import time
from pathlib import Path
from typing import Any


FOLDED = (34, 62)
UNFOLDED = (27, 114)


def _fresh_root(journey) -> Path:
    journey.preflight()
    parent = journey.work_root
    parent.parent.mkdir(parents=True, exist_ok=True)
    if not parent.exists():
        parent.mkdir()
        root = parent
    elif not parent.is_dir():
        raise ValueError(f"sol-terminal work-root is not a directory: {parent}")
    elif any(parent.iterdir()):
        root = parent / f"sol-terminal-{time.time_ns()}"
        root.mkdir()
    else:
        root = parent
    journey._dogfood_terminal_root = root
    return root


def _run(root: Path, argv: list[str], *, expected: str = "completed", timeout: float = 30) -> dict[str, Any]:
    commands = root / "commands"
    commands.mkdir(exist_ok=True)
    ordinal = len(list(commands.glob("*.argv.json")))
    stem = f"{ordinal:03d}"
    try:
        completed = subprocess.run(argv, cwd=root, capture_output=True, timeout=timeout, check=False)
    except (OSError, subprocess.TimeoutExpired) as error:
        raise ValueError(f"public command did not complete: {argv!r}: {error}") from error
    (commands / f"{stem}.argv.json").write_text(
        json.dumps({"argv": argv, "cwd": str(root)}) + "\n", encoding="utf-8"
    )
    (commands / f"{stem}.stdout").write_bytes(completed.stdout)
    (commands / f"{stem}.stderr").write_bytes(completed.stderr)
    (commands / f"{stem}.exit.json").write_text(
        json.dumps({"returncode": completed.returncode}) + "\n", encoding="utf-8"
    )
    try:
        packet = json.loads(completed.stdout)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError(
            f"public command returned non-JSON (exit={completed.returncode}): {argv!r}; "
            f"stderr={completed.stderr[-1000:]!r}"
        ) from error
    if packet.get("status") != expected:
        raise ValueError(
            f"public command status {packet.get('status')!r}, expected {expected!r}: "
            f"{argv!r}: {packet!r} stderr={completed.stderr[-1000:]!r}"
        )
    expected_code = 10 if expected == "rejected" else 0
    if completed.returncode != expected_code:
        raise ValueError(
            f"public command exit {completed.returncode}, expected {expected_code}: "
            f"{argv!r}: {packet!r}"
        )
    return packet


def _engine(journey, root: Path, database: Path, *args: str, expected: str = "completed", timeout: float = 30):
    return _run(
        root,
        [str(journey.engine), "--database", str(database), "--json", *args],
        expected=expected,
        timeout=timeout,
    )


def _binding(engine: Path, worker_script: Path, counter: Path, worker_names: list[str]) -> dict[str, Any]:
    args = ["fan-out", "--max-active", "1"]
    for name in worker_names:
        worker = {
            "command": os.environ.get("PYTHON", "python3"),
            "args": [str(worker_script), str(counter), name],
            "title": f"{name} task",
            "role": "fixture worker",
            "preamble": "Run only the assigned scripted fixture task. This is a supplied duty, not a semantic approval.",
            "full_output_schema": {"type": "object", "properties": {"result": {"type": "string", "enum": [name]}}, "required": ["result"]},
        }
        args.extend(["--worker", json.dumps(worker, separators=(",", ":"))])
    return {"command": str(engine), "args": args}


def _write_worker(path: Path) -> None:
    path.write_text(
        "import json,pathlib,sys\n"
        "sys.stdin.buffer.read()\n"
        "counter=pathlib.Path(sys.argv[1]); name=sys.argv[2]\n"
        "count=int(counter.read_text() or '0') if counter.exists() else 0\n"
        "counter.write_text(str(count+1))\n"
        "print(json.dumps({'result':name}))\n",
        encoding="utf-8",
    )


def _provider_config(path: Path, alias: str, command: str, args: list[str]) -> None:
    path.write_text(
        f"[providers.{json.dumps(alias)}]\ncommand = {json.dumps(command)}\n"
        f"args = {json.dumps(args)}\n",
        encoding="utf-8",
    )


def _write_software_wrapper(path: Path, provider: Path, request_log: Path) -> None:
    path.write_text(
        "import json,pathlib,subprocess,sys\n"
        "real=pathlib.Path(sys.argv[1]); log=pathlib.Path(sys.argv[2])\n"
        "request=json.load(sys.stdin)\n"
        "with log.open('a',encoding='utf-8') as stream: stream.write(json.dumps(request,separators=(',',':'))+'\\n')\n"
        "if request.get('operation')=='describe':\n"
        " result=subprocess.run([str(real)],input=json.dumps(request),text=True,capture_output=True,check=False)\n"
        " if result.returncode: sys.stderr.write(result.stderr); raise SystemExit(result.returncode)\n"
        " sys.stdout.write(result.stdout)\n"
        "elif request.get('operation')=='evaluate':\n"
        " edge=request.get('transition',{})\n"
        " if edge.get('source') in {'design','intent-review'}:\n"
        "  print(json.dumps({'result':'deny','feedback':{'code':'fixture-not-allowed','message':'scripted denial for terminal display'}}))\n"
        " else: print(json.dumps({'result':'allow'}))\n"
        "else: raise SystemExit('unexpected software-change operation')\n",
        encoding="utf-8",
    )
    path.chmod(0o755)


def _write_worklist_provider(path: Path, request_log: Path) -> None:
    path.write_text(
        "import json,pathlib,sys\n"
        "log=pathlib.Path(sys.argv[1]); request=json.load(sys.stdin)\n"
        "with log.open('a',encoding='utf-8') as stream: stream.write(json.dumps(request,separators=(',',':'))+'\\n')\n"
        "if request.get('operation')=='describe':\n"
        " print(json.dumps({'id':'work-list-only-fixture-v1','initial_state':'work','states':["
        "{'id':'work','title':'Available work','instructions':'Inspect assignments.','final':False}],"
        "'transitions':[],'work_slots':[{'id':'work-list','state':'work','event':'perform'}]}))\n"
        "elif request.get('operation')=='evaluate': print(json.dumps({'result':'allow'}))\n"
        "else: raise SystemExit('unexpected work-list fixture operation')\n",
        encoding='utf-8',
    )
    path.chmod(0o755)


def _write_fixture_provider(path: Path, request_log: Path) -> None:
    path.write_text(
        "import json,pathlib,sys\n"
        "log=pathlib.Path(sys.argv[1]); request=json.load(sys.stdin)\n"
        "with log.open('a',encoding='utf-8') as stream: stream.write(json.dumps(request,separators=(',',':'))+'\\n')\n"
        "if request.get('operation')=='describe':\n"
        " print(json.dumps({'id':'generic-terminal-fixture-v1','initial_state':'current','states':["
        "{'id':'graph-head','title':'Graph head','instructions':'Not current.','final':False},"
        "{'id':'current','title':'Current fixture state','instructions':'Observe only.','final':False},"
        "{'id':'done','title':'Done','instructions':'Finished.','final':True}],"
        "'transitions':[{'source':'graph-head','event':'advance','target':'done','kind':'checked'},"
        "{'source':'current','event':'demonstrate','target':'current','kind':'check-free'},"
        "{'source':'current','event':'advance','target':'done','kind':'checked'}],"
        "'work_slots':[{'id':'fixture-work','state':'current','event':'demonstrate'}]}))\n"
        "elif request.get('operation')=='evaluate':\n"
        " print(json.dumps({'result':'deny','feedback':{'code':'fixture-denied','message':'scripted denial'}}))\n"
        "else: raise SystemExit('unexpected fixture-provider operation')\n",
        encoding="utf-8",
    )
    path.chmod(0o755)


def _geometry(master: int, rows: int, columns: int) -> None:
    fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", rows, columns, 0, 0))


def _read_until(master: int, transcript: bytearray, start: int, marker: str) -> None:
    marker_bytes = marker.encode()
    deadline = time.monotonic() + 5
    while marker_bytes not in transcript[start:]:
        if time.monotonic() >= deadline:
            raise ValueError(
                f"PTY did not render {marker!r}; transcript={transcript[-6000:]!r}"
            )
        readable, _, _ = select.select([master], [], [], 0.1)
        if readable:
            try:
                chunk = os.read(master, 8192)
            except OSError as error:
                if error.errno in (5, 11):
                    continue
                raise
            if chunk:
                transcript.extend(chunk)


def _send_and_wait(master: int, transcript: bytearray, key: bytes, marker: str) -> None:
    start = len(transcript)
    os.write(master, key)
    _read_until(master, transcript, start, marker)


def _quit_and_drain(master: int, process, transcript: bytearray) -> int:
    # Enter now expands a tree and repaints. A PTY consumer must keep reading
    # while awaiting q; otherwise queued frames fill the PTY and block stdout.
    deadline = time.monotonic() + 5
    while process.poll() is None:
        if time.monotonic() >= deadline:
            raise ValueError("terminal navigator did not quit on q while its output was drained")
        readable, _, _ = select.select([master], [], [], 0.1)
        if readable:
            try:
                transcript.extend(os.read(master, 8192))
            except OSError as error:
                if error.errno not in (5, 11):
                    raise
    return process.wait(timeout=1)


def _selection_marker(key: str) -> str:
    if key.startswith("assignment:"):
        _, slot_id, assignment_id = key.split(":", 2)
        return f"selected_assignment_id={assignment_id} slot={slot_id}"
    return f"selected={key}"


def _pty_navigation(journey, root: Path, database: Path, run_id: str, show: dict[str, Any], counter: Path, provider_log: Path) -> dict[str, Any]:
    result = show["result"]
    workflow = result.get("workflow_graph")
    if not isinstance(workflow, dict):
        raise ValueError("full show did not expose the exact stored workflow graph")
    states = workflow.get("states", [])
    if not states or states[0]["id"] == result.get("current_state"):
        raise ValueError(
            "terminal fixture must make current state differ from the graph's first displayed state"
        )
    current = result["current_state"]
    if not any(state.get("id") == current for state in states):
        raise ValueError("stored current state is absent from the frozen graph")
    labels_by_slot: dict[str, list[str]] = {}
    for invocation in result.get("work_slot_invocations", []):
        slot_id = invocation.get("slot_id")
        for label in invocation.get("assignment_labels", []):
            assignment_id = label.get("assignment_id")
            if slot_id and assignment_id and assignment_id not in labels_by_slot.setdefault(slot_id, []):
                labels_by_slot[slot_id].append(assignment_id)
    expected_keys: list[str] = []
    for state in states:
        state_id = state["id"]
        expected_keys.append(f"state:{state_id}")
        for slot in workflow.get("work_slots", []):
            if slot.get("state") == state_id:
                expected_keys.extend(
                    f"assignment:{slot['id']}:{assignment_id}"
                    for assignment_id in labels_by_slot.get(slot["id"], [])
                )
    if not any(key.startswith("assignment:") for key in expected_keys):
        raise ValueError("frozen workflow proof fixture contains no actual assignment labels")
    current_index = expected_keys.index(f"state:{current}")
    assignment_key = next(key for key in expected_keys if key.startswith("assignment:"))

    master, slave = pty.openpty()
    transcript = bytearray()
    process = None
    try:
        _geometry(slave, *FOLDED)
        process = subprocess.Popen(
            [str(journey.engine), "--database", str(database), "explore", run_id],
            cwd=root,
            stdin=slave,
            stdout=slave,
            stderr=slave,
            close_fds=True,
        )
        os.close(slave)
        slave = -1
        os.set_blocking(master, False)
        _read_until(master, transcript, 0, f"terminal-size={FOLDED[0]}x{FOLDED[1]}")
        _read_until(master, transcript, 0, f"selected=state:{current}")
        if "checked-not-allowed" not in transcript.decode(errors="replace"):
            raise ValueError("current state's checked denial is not shown as checked-not-allowed")
        if "requestable now" not in transcript.decode(errors="replace"):
            raise ValueError("requestability is not separately displayed from checked permission")
        rendered = transcript.decode(errors="replace")
        if "[VISITED]" not in rendered or "visited does not mean passed" not in rendered:
            raise ValueError("visited state was not kept distinct from semantic pass")
        if "acceptance: passed" in rendered:
            raise ValueError("terminal navigator promoted a visited state to pass")

        delta = expected_keys.index(assignment_key) - current_index
        for step in range(abs(delta)):
            key = b"j" if delta > 0 else b"k"
            next_key = expected_keys[current_index + (step + 1) * (1 if delta > 0 else -1)]
            _send_and_wait(master, transcript, key, _selection_marker(next_key))
        if _selection_marker(assignment_key) not in transcript.decode(errors="replace"):
            raise ValueError("selected assignment ID was not reachable by basic letter navigation")

        if b"\x1b[1;36m" not in transcript or b"\x1b[1;36;7m" not in transcript:
            raise ValueError("production viewer omitted terminal-palette hierarchy/selection colors")
        _send_and_wait(master, transcript, b"\t", "DETAILS [focused]")
        _send_and_wait(master, transcript, b"l", "v Stored data:")
        for _ in range(5):
            _send_and_wait(master, transcript, b"j", "detail-offset=")
        _send_and_wait(master, transcript, b"l", "v worker_definition:")
        _read_until(master, transcript, 0, "command:")
        # Cursor identity and branch openness survive both phone geometries.
        for dimensions in (UNFOLDED, FOLDED):
            before = len(transcript)
            _geometry(master, *dimensions)
            _read_until(master, transcript, before, f"terminal-size={dimensions[0]}x{dimensions[1]}")
            _read_until(master, transcript, before, "tree-node=/worker_definition")
            _read_until(master, transcript, before, "v worker_definition:")
            resized = transcript[before:].decode(errors="replace")
            if _selection_marker(assignment_key) not in resized:
                raise ValueError(f"selection changed during terminal reflow to {dimensions}: {resized[-2000:]}")
        _send_and_wait(master, transcript, b"h", "> worker_definition:")
        _send_and_wait(master, transcript, b"l", "v worker_definition:")
        _send_and_wait(master, transcript, b"]", "detail-offset=")
        _send_and_wait(master, transcript, b"[", "detail-offset=")
        _send_and_wait(master, transcript, b"\t", "DETAILS [Tab to focus]")
        _send_and_wait(master, transcript, b"g", _selection_marker(expected_keys[0]))
        for index, expected_key in enumerate(expected_keys[1:], 1):
            key = b"\x1b[B" if index % 2 else b"j"
            before = len(transcript)
            _send_and_wait(master, transcript, key, _selection_marker(expected_key))
            if expected_key.startswith("state:") and expected_key != f"state:{current}":
                frame_start = transcript.index(_selection_marker(expected_key).encode(), before)
                _read_until(master, transcript, frame_start, "detail-offset=")
                frame = bytes(transcript[frame_start:]).split(b"\x1b[?25l\x1b[H\x1b[2J", 1)[0]
                if "requestable now" in " ".join(frame.decode(errors="replace").split()):
                    raise ValueError(f"noncurrent state {expected_key} marks an edge requestable now")
        _send_and_wait(master, transcript, b"G", _selection_marker(expected_keys[-1]))

        worker_count_before = counter.read_text(encoding="utf-8")
        provider_log_before = provider_log.read_bytes()
        os.write(master, b"aei\r")
        os.write(master, b"q")
        return_code = _quit_and_drain(master, process, transcript)
        if return_code != 0:
            raise ValueError(f"terminal navigator exited {return_code}: {transcript[-4000:]!r}")
        if counter.read_text(encoding="utf-8") != worker_count_before:
            raise ValueError("navigator input started a worker assignment")
        if provider_log.read_bytes() != provider_log_before:
            raise ValueError("navigator invoked the provider or advanced a checked route")
    finally:
        if process is not None and process.poll() is None:
            process.kill()
            process.wait(timeout=3)
        if slave >= 0:
            os.close(slave)
        os.close(master)
        # Retain the actual PTY bytes even when an assertion fails.
        (root / "terminal-transcript.bin").write_bytes(transcript)

    transcript_path = root / "terminal-transcript.bin"
    transcript_path.write_bytes(transcript)
    return {
        "run_id": run_id,
        "geometry": [list(FOLDED), list(UNFOLDED), list(FOLDED)],
        "initial_selection": f"state:{current}",
        "assignment_selection": assignment_key,
        "reachable_items": expected_keys,
        "read_only_keys": "aei\r",
        "production_palette": True,
        "tree_expand_collapse_and_cursor_reflow": True,
        "transcript": str(transcript_path),
    }


def _public_graphless_worklist(journey, root: Path, worker: Path) -> dict[str, Any]:
    fixture = root / "graphless-work-list"
    fixture.mkdir()
    database = fixture / "loop.sqlite"
    artifacts = fixture / "artifacts"
    artifacts.mkdir()
    counter = fixture / "worker-count"
    counter.write_text("0", encoding="utf-8")
    provider_log = fixture / "provider-requests.jsonl"
    provider_log.touch()
    provider = fixture / "work-list-provider.py"
    _write_worklist_provider(provider, provider_log)
    config = fixture / "providers.toml"
    _provider_config(config, "work-list", os.environ.get("PYTHON", "python3"), [str(provider), str(provider_log)])
    initial = {
        "artifact_root": str(artifacts),
        "work_slot_bindings": {
            "work-list": _binding(journey.engine, worker, counter, ["queued-task"])
        },
    }
    run_id = "terminal-graphless-work-list"
    _run(
        fixture,
        [str(journey.engine), "--database", str(database), "--config", str(config), "--json",
         "start", "--id", run_id, "work-list", json.dumps(initial, separators=(",", ":"))],
    )
    before = _semantic_snapshot(journey, fixture, database, run_id)
    show = _engine(journey, fixture, database, "show", "--view", "full", run_id)["result"]
    if show["workflow_graph"].get("transitions") != []:
        raise ValueError("graphless fallback fixture unexpectedly contains workflow edges")
    provider_before = provider_log.read_bytes()

    master, slave = pty.openpty()
    transcript = bytearray()
    process = None
    try:
        _geometry(slave, *FOLDED)
        process = subprocess.Popen(
            [str(journey.engine), "--database", str(database), "explore", run_id],
            cwd=fixture,
            stdin=slave,
            stdout=slave,
            stderr=slave,
            close_fds=True,
        )
        os.close(slave)
        slave = -1
        os.set_blocking(master, False)
        _read_until(master, transcript, 0, f"terminal-size={FOLDED[0]}x{FOLDED[1]}")
        _read_until(master, transcript, 0, "Work list — no stored workflow graph")
        _read_until(master, transcript, 0, "selected_assignment_id=worker-0 slot=work-list")
        text = transcript.decode(errors="replace")
        if "assignment:queued-task" not in text and "queued-task task" not in text:
            raise ValueError("graphless work list omitted its configured assignment")
        if "barrier" in text.lower() or "event " in text:
            raise ValueError("graphless work list fabricated an edge or invocation barrier")
        os.write(master, b"aei\r")
        os.write(master, b"q")
        return_code = _quit_and_drain(master, process, transcript)
        if return_code != 0:
            raise ValueError(f"graphless navigator exited {return_code}: {transcript[-3000:]!r}")
    finally:
        if process is not None and process.poll() is None:
            process.kill()
            process.wait(timeout=3)
        if slave >= 0:
            os.close(slave)
        os.close(master)

    after = _semantic_snapshot(journey, fixture, database, run_id)
    if after != before:
        raise ValueError(f"graphless navigation changed durable run semantics: {before} -> {after}")
    if counter.read_text(encoding="utf-8") != "0" or provider_log.read_bytes() != provider_before:
        raise ValueError("graphless list input invoked work or called the provider")
    path = fixture / "terminal-transcript.bin"
    path.write_bytes(transcript)
    return {
        "run_id": run_id,
        "display": "work list without stored transitions",
        "selected_assignment": "worker-0",
        "fabricated_barrier": False,
        "transcript": str(path),
    }


def _semantic_snapshot(journey, root: Path, database: Path, run_id: str) -> dict[str, Any]:
    shown = _engine(journey, root, database, "show", "--view", "full", run_id)
    history = _engine(journey, root, database, "history", run_id)
    return {
        "current_state": shown["result"]["current_state"],
        "state_visit": shown["result"]["state_visit"],
        "context_ids": [row["id"] for row in shown["result"].get("context", [])],
        "invocation_ids": [row["invocation_id"] for row in shown["result"].get("work_slot_invocations", [])],
        "history": history["result"],
    }


def _deny_current_checked_edge(journey, root: Path, database: Path, run_id: str, provider_log: Path) -> dict[str, Any]:
    before = _engine(journey, root, database, "show", "--view", "full", run_id)["result"]
    current = before["current_state"]
    workflow = before["workflow_graph"]
    edge = next(
        (edge for edge in workflow["transitions"] if edge["source"] == current and edge["kind"] == "checked"),
        None,
    )
    if edge is None:
        raise ValueError(f"fixture current state {current!r} has no checked route to exercise")
    denied = _engine(
        journey,
        root,
        database,
        "event",
        run_id,
        edge["event"],
        expected="rejected",
    )
    if denied.get("code") not in {"fixture-not-allowed", "fixture-denied"}:
        raise ValueError(f"scripted provider denial was not retained: {denied}")
    after = _engine(journey, root, database, "show", "--view", "full", run_id)["result"]
    if after["current_state"] != current:
        raise ValueError("denied checked request changed current state")
    evaluation = next(
        (row for row in after["latest_evaluations"] if row["transition"]["event"] == edge["event"]),
        None,
    )
    if not evaluation or evaluation.get("result", {}).get("result") != "deny":
        raise ValueError("checked denial was not present in latest recorded evaluations")
    request_lines = provider_log.read_text(encoding="utf-8").splitlines()
    if not any(json.loads(line).get("operation") == "evaluate" for line in request_lines):
        raise ValueError("fixture provider did not receive the real public checked evaluation")
    return after


def _wait_for_invocation(journey, root: Path, database: Path, run_id: str, invocation_id: str) -> None:
    deadline = time.monotonic() + 25
    while time.monotonic() < deadline:
        status = _engine(journey, root, database, "show", "--view", "status", run_id)["result"]
        row = next(
            (item for item in status.get("invocations", {}).get("items", [])
             if item.get("invocation_id") == invocation_id),
            None,
        )
        state = row.get("execution", {}).get("state") if row else None
        if state == "succeeded":
            return
        if state in {"failed", "cancelled", "overrun"}:
            raise ValueError(f"fixture bound invocation ended as {state}: {row}")
        time.sleep(0.05)
    raise ValueError(f"fixture invocation did not finish before its proof deadline: {invocation_id}")


def _exercise_fixture(journey, root: Path, database: Path, provider_config: Path, alias: str, run_id: str, initial_input: dict[str, Any], current_event: str, provider_log: Path, counter: Path) -> dict[str, Any]:
    profile_input = root / "initial-input.json"
    profile_input.write_text(json.dumps(initial_input, separators=(",", ":")), encoding="utf-8")
    packet = _engine(
        journey,
        root,
        database,
        "--config",
        str(provider_config),
        "start",
        "--id",
        run_id,
        alias,
        "@" + str(profile_input.resolve()),
    )
    if packet["result"]["run"]["id"] != run_id:
        raise ValueError("public start returned a different fixture run")
    started = _engine(journey, root, database, "show", "--view", "full", run_id)["result"]
    workflow = started.get("workflow_graph")
    if not workflow:
        raise ValueError("public start omitted the frozen workflow graph")
    slot = next(
        (slot for slot in workflow["work_slots"] if slot["state"] == started["current_state"] and slot["event"] == current_event),
        None,
    )
    if slot is None:
        raise ValueError(f"fixture has no work slot for current event {current_event!r}")
    invoked = _engine(journey, root, database, "--timeout-ms", "120000", "invoke", run_id, slot["id"], timeout=130)
    invocation_id = invoked["result"]["invocation_id"]
    _wait_for_invocation(journey, root, database, run_id, invocation_id)
    _engine(journey, root, database, "show", "--view", "full", run_id)
    moved = _engine(journey, root, database, "event", run_id, current_event)
    if moved.get("status") != "completed":
        raise ValueError(f"fixture could not move to its current demonstration state: {moved}")
    current_show = _engine(journey, root, database, "show", "--view", "full", run_id)["result"]
    if workflow["states"][0]["id"] == current_show["current_state"]:
        raise ValueError("fixture current state must differ from the first displayed graph state")
    after_denial = _deny_current_checked_edge(journey, root, database, run_id, provider_log)
    before = _semantic_snapshot(journey, root, database, run_id)
    workers_before = counter.read_text(encoding="utf-8")
    requests_before = provider_log.read_bytes()
    report = _pty_navigation(
        journey,
        root,
        database,
        run_id,
        {"result": after_denial},
        counter,
        provider_log,
    )
    after = _semantic_snapshot(journey, root, database, run_id)
    if after != before:
        raise ValueError(f"terminal navigation changed durable run semantics: before={before} after={after}")
    if counter.read_text(encoding="utf-8") != workers_before or provider_log.read_bytes() != requests_before:
        raise ValueError("terminal key navigation invoked work or provider behavior")
    report["state_before"] = before["current_state"]
    report["state_visit_before"] = before["state_visit"]
    report["invocations_before"] = before["invocation_ids"]
    report["history_count_before"] = len(before["history"])
    report["provider_requests_before"] = len(provider_log.read_text(encoding="utf-8").splitlines())
    (root / "terminal-assertions.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    return report


def terminal_case(journey) -> None:
    if os.name != "posix":
        raise ValueError("sol-terminal PTY proof requires a POSIX platform")
    if not shutil.which("dagu"):
        raise ValueError("sol-terminal requires the operator-provided Dagu binary on PATH")
    root = _fresh_root(journey)
    worker = root / "worker.py"
    _write_worker(worker)

    # Software-change uses the real provider's frozen graph description; only
    # transition answers are scripted so no reviewer or semantic approval is claimed.
    software_root = root / "software-change"
    software_root.mkdir()
    software_artifacts = software_root / "artifacts"
    software_artifacts.mkdir()
    software_counter = software_root / "worker-count"
    software_counter.write_text("0", encoding="utf-8")
    software_log = software_root / "provider-requests.jsonl"
    software_log.touch()
    wrapper = software_root / "provider-wrapper.py"
    _write_software_wrapper(wrapper, journey.provider, software_log)
    software_config = software_root / "providers.toml"
    _provider_config(software_config, "software-change", os.environ.get("PYTHON", "python3"), [str(wrapper), str(journey.provider), str(software_log)])
    software_input = json.loads(journey.profile_source.read_text(encoding="utf-8"))
    software_input["artifact_root"] = str(software_artifacts)
    software_input["work_slot_bindings"] = {
        "intent-draft": _binding(journey.engine, worker, software_counter, ["draft-one", "draft-two"])
    }
    software_database = software_root / "loop.sqlite"
    software_report = _exercise_fixture(
        journey,
        software_root,
        software_database,
        software_config,
        "software-change",
        "terminal-software-change",
        software_input,
        "intent-ready",
        software_log,
        software_counter,
    )

    # A separately scripted non-software provider exercises the same CLI on a
    # different frozen graph, with its initial state deliberately not the list head.
    generic_root = root / "generic-provider"
    generic_root.mkdir()
    generic_artifacts = generic_root / "artifacts"
    generic_artifacts.mkdir()
    generic_counter = generic_root / "worker-count"
    generic_counter.write_text("0", encoding="utf-8")
    generic_log = generic_root / "provider-requests.jsonl"
    generic_log.touch()
    generic_provider = generic_root / "fixture-provider.py"
    _write_fixture_provider(generic_provider, generic_log)
    generic_config = generic_root / "providers.toml"
    _provider_config(generic_config, "fixture", os.environ.get("PYTHON", "python3"), [str(generic_provider), str(generic_log)])
    generic_input = {
        "artifact_root": str(generic_artifacts),
        "work_slot_bindings": {
            "fixture-work": _binding(journey.engine, worker, generic_counter, ["task-one", "task-two"])
        },
    }
    generic_database = generic_root / "loop.sqlite"
    generic_report = _exercise_fixture(
        journey,
        generic_root,
        generic_database,
        generic_config,
        "fixture",
        "terminal-generic-provider",
        generic_input,
        "demonstrate",
        generic_log,
        generic_counter,
    )

    _fresh_before = software_report["reachable_items"]
    if not any(item.startswith("assignment:") for item in _fresh_before):
        raise ValueError("software-change PTY did not reach a frozen assignment ID")
    if not any(item.startswith("assignment:") for item in generic_report["reachable_items"]):
        raise ValueError("non-software PTY did not reach a frozen assignment ID")
    graphless_report = _public_graphless_worklist(journey, root, worker)
    summary = {
        "software_change": software_report,
        "non_software_provider": generic_report,
        "graphless_work_list": graphless_report,
        "owner_reported_geometry_only": [list(FOLDED), list(UNFOLDED)],
        "owner_fold_demonstration_and_usability_acceptance": "pending external owner demonstration",
    }
    (root / "outcome.json").write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")
    print("sol-terminal public PTY passed: both frozen graphs, current-state focus, stable assignment reflow, navigation and read-only checks")
    print("Fold7 owner demonstration and usability acceptance remain external")
