"""Public captured-output and failed/cancelled fan-out recovery fixtures.

All workers and the model-shaped formatter are deterministic local commands;
this proves mechanics, not reviewer quality or authorization for a live model.
"""
from __future__ import annotations

import copy
import hashlib
import json
import os
import shutil
import signal
import sqlite3
import subprocess
import sys
import time
from pathlib import Path
from typing import Any

import dogfood_observation


def _fresh_root(journey) -> Path:
    return dogfood_observation._fresh_root(journey, "sol-recovery-core")


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
    if not isinstance(result, dict):
        return {"available": False}

    invocation_fields = ("invocation_id", "slot_id", "status", "exit_code", "capture_dir")
    invocations = []
    for row in result.get("work_slot_invocations", []):
        if not isinstance(row, dict):
            continue
        projected = {key: row[key] for key in invocation_fields if key in row}
        ownership = row.get("ownership")
        if isinstance(ownership, dict):
            projected["ownership"] = {
                key: ownership[key]
                for key in ("cleanup_pending", "live_owned_work")
                if key in ownership
            }
        invocations.append(projected)
    return {
        "available": True,
        "run_id": result.get("run_id"),
        "current_state": result.get("current_state"),
        "work_slot_invocations": invocations,
    }


def _record(
    root: Path,
    argv: list[str],
    completed: subprocess.CompletedProcess[bytes],
    *,
    stdin_payload: bytes | None = None,
) -> None:
    commands = root / "commands"
    commands.mkdir(exist_ok=True)
    ordinal = len(list(commands.glob("*.argv.json")))
    stem = f"{ordinal:04d}"
    argv_record: dict[str, Any] = {"argv": argv, "cwd": str(root)}
    if stdin_payload is not None:
        argv_record["stdin_capture"] = {
            "kind": "transient-payload-facts",
            "byte_length": len(stdin_payload),
            "sha256": _sha(stdin_payload),
            "raw_stdin_retained": False,
        }
    if _full_show_command(argv):
        projection_name = f"{stem}.stdout-projection.json"
        argv_record["stdout_capture"] = {
            "kind": "bounded-full-show-projection",
            "path": projection_name,
            "raw_stdout_retained": False,
        }
        _json(
            commands / projection_name,
            {
                "kind": "loop-engine-full-show-projection-v1",
                "raw_stdout_retained": False,
                "original_stdout": {
                    "byte_length": len(completed.stdout),
                    "sha256": _sha(completed.stdout),
                },
                "projection": _full_show_projection(completed.stdout),
            },
        )
    else:
        (commands / f"{stem}.stdout").write_bytes(completed.stdout)
    (commands / f"{stem}.argv.json").write_text(
        json.dumps(argv_record) + "\n", encoding="utf-8"
    )
    (commands / f"{stem}.stderr").write_bytes(completed.stderr)
    (commands / f"{stem}.exit.json").write_text(
        json.dumps({"returncode": completed.returncode}) + "\n", encoding="utf-8"
    )


def _call(root: Path, argv: list[str], *, expect: str = "completed", timeout: float = 40) -> tuple[dict[str, Any], subprocess.CompletedProcess[bytes]]:
    try:
        completed = subprocess.run(argv, cwd=root, capture_output=True, timeout=timeout, check=False)
    except (OSError, subprocess.TimeoutExpired) as error:
        raise ValueError(f"public command did not complete: {argv!r}: {error}") from error
    _record(root, argv, completed)
    try:
        value = json.loads(completed.stdout)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError(
            f"public command returned non-JSON (exit={completed.returncode}): {argv!r}; "
            f"stderr={completed.stderr[-1200:]!r}"
        ) from error
    if expect == "completed":
        if completed.returncode != 0 or value.get("status") != "completed":
            raise ValueError(f"public command failed: {argv!r}: {value!r} {completed.stderr[-1200:]!r}")
    elif expect == "rejected":
        if completed.returncode != 10 or value.get("status") != "rejected":
            raise ValueError(f"public command was not refused: {argv!r}: {value!r} {completed.stderr[-1200:]!r}")
    elif expect == "invalid":
        if completed.returncode != 2:
            raise ValueError(f"public helper did not refuse malformed input: {argv!r}: {completed.returncode}")
    else:
        raise ValueError(f"unknown expected command outcome {expect!r}")
    return value, completed


def _engine(journey, root: Path, database: Path, *args: str, expect: str = "completed", timeout: float = 40):
    return _call(
        root,
        [str(journey.engine), "--database", str(database), "--json", *args],
        expect=expect,
        timeout=timeout,
    )


def _json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


def _sha(data: bytes) -> str:
    return "sha256:" + hashlib.sha256(data).hexdigest()


def _wait_invocation(journey, root: Path, database: Path, run_id: str, invocation_id: str, *, timeout: float = 45):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        shown, _ = _engine(journey, root, database, "show", "--view", "full", run_id)
        result = shown["result"]
        row = next((item for item in result["work_slot_invocations"] if item["invocation_id"] == invocation_id), None)
        if row and row["status"] in ("succeeded", "failed", "overrun"):
            return shown, row
        time.sleep(0.05)
    raise ValueError(f"invocation {invocation_id} did not become terminal")


def _show_action(journey, root: Path, database: Path, run_id: str):
    return _engine(journey, root, database, "show", "--view", "action", run_id)[0]


def _write_software_run(
    journey,
    root: Path,
    label: str,
    slot_id: str,
    binding: dict[str, Any],
    *,
    review: bool = False,
) -> tuple[Path, Path, str, Path]:
    run_root = root / label
    run_root.mkdir(parents=True)
    artifact_root = run_root / "artifacts"
    artifact_root.mkdir()
    checkout = run_root / "fixture-checkout"
    checkout.mkdir()
    provider_config = run_root / "providers.toml"
    provider_config.write_text(
        "[providers.software-change]\n"
        f"command = {json.dumps(str(journey.provider))}\nargs = []\n",
        encoding="utf-8",
    )
    profile = json.loads((journey.data_root / "crates/software-change-provider/data/configs/minimal.json").read_text(encoding="utf-8"))
    profile["review_policies"] = {}
    if review:
        source_profile = json.loads((journey.data_root / "crates/software-change-provider/data/configs/minimal.json").read_text(encoding="utf-8"))
        profile["review_policies"] = {"intent-review": [copy.deepcopy(source_profile["review_policies"]["intent-review"][0])]}
    profile["artifact_root"] = str(artifact_root)
    profile["work_slot_bindings"] = {slot_id: binding}
    profile_path = run_root / "initial-input.json"
    _json(profile_path, profile)
    database = run_root / "loop.sqlite"
    run_id = f"p05-{label}"
    started, _ = _engine(
        journey,
        root,
        database,
        "--config",
        str(provider_config),
        "start",
        "--id",
        run_id,
        "software-change",
        "@" + str(profile_path),
    )
    stored_root = Path(started["result"]["run"]["initial_input"]["artifact_root"])
    if stored_root != artifact_root:
        raise ValueError(f"software-change fixture changed its configured artifact root: {stored_root}")
    fixtures = journey.data_root / "crates/software-change-provider/data/calibration/fixtures"
    names = ["intent-good.json"] if review else ["intent-good.json", "design-good.json", "plan-good.json"]
    for name in names:
        shutil.copyfile(fixtures / name, artifact_root / name.replace("-good", ""))
    for event in (("intent-ready",) if review else ("intent-ready", "design-ready", "plan-ready")):
        _show_action(journey, root, database, run_id)
        _engine(journey, root, database, "event", run_id, event)
    shown, _ = _engine(journey, root, database, "show", "--view", "full", run_id)
    expected_state = "intent-review" if review else "implement"
    if shown["result"]["current_state"] != expected_state:
        raise ValueError(f"software-change fixture did not reach {expected_state}: {shown['result']['current_state']}")
    return database, artifact_root, run_id, checkout


def _setup_review_run(
    journey,
    root: Path,
    label: str,
    reviewer: Path,
    launch_counter: Path,
    *,
    mode: str = "raw-review",
) -> tuple[Path, Path, str]:
    run_root = root / label
    run_root.mkdir(parents=True)
    artifact_root = run_root / "artifacts"
    artifact_root.mkdir()
    provider_config = run_root / "providers.toml"
    provider_config.write_text(
        chr(10).join([
            "[providers.software-change]",
            f"command = {json.dumps(str(journey.provider))}",
            "args = []",
            "",
        ]),
        encoding="utf-8",
    )
    source_profile = json.loads(
        (journey.data_root / "crates/software-change-provider/data/configs/minimal.json")
        .read_text(encoding="utf-8")
    )
    profile = copy.deepcopy(source_profile)
    profile["review_policies"] = {
        "intent-review": [copy.deepcopy(source_profile["review_policies"]["intent-review"][0])]
    }
    profile["artifact_root"] = str(artifact_root)
    profile_input = run_root / "setup-input.json"
    _json(profile_input, profile)
    evidence_locator = f"{label}/artifacts/intent.json#L1-L1"
    roster = [{
        "author": "fixture-reviewer",
        "command": sys.executable,
        "args": [str(reviewer), mode, str(launch_counter), evidence_locator],
        "token_budget": {
            "model_id": "scripted-fixture",
            "context_window_tokens": 100000,
            "system_tokens": 100,
            "framing_tokens": 100,
            "output_reserve_tokens": 100,
            "reasoning_reserve_tokens": 100,
        },
    }]
    roster_path = run_root / "roster.json"
    _json(roster_path, roster)
    profile_path = run_root / "generated-profile.json"
    setup_argv = [
        str(journey.provider), "setup", "--profile", str(profile_input),
        "--roster", str(roster_path), "--engine", str(journey.engine),
        "--provider", str(journey.provider), "--output", str(profile_path),
        "--decline-advice",
    ]
    setup = subprocess.run(setup_argv, cwd=root, capture_output=True, timeout=60, check=False)
    _record(root, setup_argv, setup)
    if setup.returncode != 0:
        raise ValueError(f"public setup failed for recovery fixture: {setup.stderr[-1200:]!r}")
    generated = json.loads(profile_path.read_text(encoding="utf-8"))
    binding = generated["work_slot_bindings"]["intent-review"]
    args = binding["args"]
    workers = [
        json.loads(args[index + 1])
        for index, value in enumerate(args[:-1])
        if value == "--worker"
    ]
    if len(workers) != 1 or not isinstance(workers[0].get("full_output_schema"), dict):
        raise ValueError("public setup did not generate one full-schema-v2 reviewer")
    if workers[0]["full_output_schema"].get("x-loop-engine-output-recovery") != "repair-first-v1":
        raise ValueError("new setup did not freeze repair-first output recovery")
    _json(run_root / "generated-review-worker.json", workers[0])

    shutil.copyfile(
        journey.data_root / "crates/software-change-provider/data/calibration/fixtures/intent-good.json",
        artifact_root / "intent.json",
    )
    database = run_root / "loop.sqlite"
    run_id = f"p05-{label}"
    _engine(
        journey,
        root,
        database,
        "--config", str(provider_config), "start", "--id", run_id,
        "software-change", "@" + str(profile_path),
    )
    _show_action(journey, root, database, run_id)
    _engine(journey, root, database, "event", run_id, "intent-ready")
    shown, _ = _engine(journey, root, database, "show", "--view", "full", run_id)
    if shown["result"]["current_state"] != "intent-review":
        raise ValueError("setup-generated recovery run did not reach intent-review")
    return database, artifact_root, run_id


def _fanout_binding(engine: Path, workers: list[dict[str, Any]], *, max_active: int = 1, split: int | None = None) -> dict[str, Any]:
    args = ["fan-out", "--max-active", str(max_active)]
    for index, worker in enumerate(workers):
        if split is not None and index == split:
            args.append("--then")
        args.extend(["--worker", json.dumps(worker, separators=(",", ":"))])
    return {"command": str(engine), "args": args}


def _worker(path: Path, mode: str) -> None:
    path.write_text(
        "import hashlib,json,pathlib,sys,time\n"
        "mode=sys.argv[1]; counter=pathlib.Path(sys.argv[2]); extra=pathlib.Path(sys.argv[3]) if len(sys.argv)>3 else None\n"
        "raw=sys.stdin.buffer.read(); separator=bytes([45,45,45,10,10]); payload=raw.split(separator,1)[0] if separator in raw else raw; packet=json.loads(payload.splitlines()[-1] if payload != raw else raw)\n"
        "if not isinstance(packet,dict) or not isinstance(packet.get('artifact_root'),str) or not set(packet).issubset({'artifact_root','context'}): raise SystemExit('bad compact location packet')\n"
        "if packet.get('context',[]) != []: raise SystemExit('unexpected fixture context')\n"
        "count=int(counter.read_text() or '0') if counter.exists() else 0\n"
        "counter.write_text(str(count+1))\n"
        "if mode=='implementation':\n"
        " checkout=pathlib.Path(sys.argv[3]); target=checkout/'tracked.txt'; before=target.read_bytes(); target.write_bytes(b'after\\n')\n"
        " print(json.dumps({'implementation_result':'performed exactly one fixture edit','effect':{'path':'tracked.txt','before':before.decode(),'after':target.read_text()}}))\n"
        "elif mode=='raw-review':\n"
        " subject=pathlib.Path(packet['artifact_root'])/'intent.json'; locator=sys.argv[3]; digest='sha256:'+hashlib.sha256(subject.read_bytes()).hexdigest()\n"
        " print('FAIL: the selected fallback is not observable by an operator.')\n"
        " print('Finding: the fallback branch has no observable completion signal.')\n"
        " print('Reason: the selected fallback exposes no completion signal to the operator.')\n"
        " print('Evidence locator: '+locator)\n"
        " print('Evidence SHA: '+digest)\n"
        "elif mode=='full-schema-missing-constants':\n"
        " subject=pathlib.Path(packet['artifact_root'])/'intent.json'; locator=sys.argv[3]; digest='sha256:'+hashlib.sha256(subject.read_bytes()).hexdigest()\n"
        " judgment={'axis':'solution-agnostic','result':'fail','findings':'the fallback branch has no observable completion signal.','grounds':{'reason':'the selected fallback exposes no completion signal to the operator.','evidence':[{'locator':locator,'sha256':digest}]}}\n"
        " print(json.dumps({'judgments':[judgment]}))\n"
        "elif mode=='group-a':\n"
        " print(json.dumps({'result':'group-a-complete'}))\n"
        "elif mode=='group-b':\n"
        " if count==0: print(json.dumps({'wrong':'first attempt deliberately misses the output contract'}))\n"
        " else: print(json.dumps({'result':'group-b-complete'}))\n"
        "elif mode=='cancel-a':\n"
        " print(json.dumps({'result':'cancel-a-complete'}))\n"
        "elif mode=='cancel-b':\n"
        " if count==0:\n"
        "  extra.write_text('started\\n'); time.sleep(60)\n"
        " print(json.dumps({'result':'cancel-b-complete'}))\n"
        "elif mode=='group-c':\n"
        " print(json.dumps({'result':'group-c-complete'}))\n"
        "else: raise SystemExit('unknown fixture mode')\n",
        encoding="utf-8",
    )


def _adapter(path: Path) -> None:
    path.write_text(
        "import json,pathlib,sys\n"
        "mode=sys.argv[1]; counter=pathlib.Path(sys.argv[2]); request=json.load(sys.stdin)\n"
        "count=int(counter.read_text() or '0') if counter.exists() else 0; counter.write_text(str(count+1))\n"
        "raw=bytes.fromhex(request['raw_output_hex']).decode()\n"
        "fields={line.split(': ',1)[0]:line.split(': ',1)[1] for line in raw.splitlines() if ': ' in line}\n"
        "schema=request['full_output_schema']; properties=schema['properties']; review=properties['judgments']['items']['oneOf'][0]\n"
        "axis=review['properties']['axis']['enum'][0]; author=properties['author']['const']; stage=properties['review_stage']['const']; version=properties['review_contract_version']['const']\n"
        "finding=fields['Finding']; reason=fields['Reason']; locator=fields['Evidence locator']; source_sha=fields['Evidence SHA']\n"
        "if mode=='changed': finding='changed meaning: fallback is observable'\n"
        "judgment={'axis':axis,'result':'fail','findings':finding,'grounds':{'reason':reason,'evidence':[{'locator':locator,'sha256':source_sha}]}}\n"
        "if mode=='missing': judgment.pop('grounds')\n"
        "output={'review_contract_version':version,'review_stage':stage,'author':author,'judgments':[judgment]}\n"
        "if mode=='unmetered': print(json.dumps({'output':json.dumps(output)})); raise SystemExit(0)\n"
        "if mode not in ('faithful','changed','missing','overbudget'): raise SystemExit('unknown adapter mode')\n"
        "cost=7 if mode=='overbudget' else 2\n"
        "print(json.dumps({'output':json.dumps(output,separators=(',',':')),'usage':{'calls':1,'metered_cost_micros':cost}}))\n",
        encoding="utf-8",
    )


def _model_adapter(path: Path) -> None:
    """A local, deterministic representation adapter; it never calls a model service."""
    path.write_text(
        "import json,os,pathlib,sys,time\n"
        "counter=pathlib.Path(sys.argv[1]); behavior=json.loads(pathlib.Path(sys.argv[2]).read_text()); request=json.load(sys.stdin)\n"
        "selected=request.get('model_adapter'); budget=request.get('budget',{})\n"
        "if request.get('representation_only') is not True or not isinstance(selected,dict): raise SystemExit('missing model selection')\n"
        "if not selected.get('model_id') or not selected.get('command') or not isinstance(selected.get('args'),list): raise SystemExit('missing model/command identity')\n"
        "if any(not isinstance(budget.get(k),int) or budget[k]<=0 for k in ('max_calls','max_time_ms','max_cost_micros','remaining_calls','remaining_time_ms','remaining_cost_micros')): raise SystemExit('missing positive bounds')\n"
        "count=int(counter.read_text() or '0') if counter.exists() else 0; counter.write_text(str(count+1))\n"
        "if behavior.get('pid_path'): pathlib.Path(behavior['pid_path']).write_text(str(os.getpid()))\n"
        "if behavior.get('sleep_ms'): time.sleep(behavior['sleep_ms']/1000)\n"
        "raw=bytes.fromhex(request['raw_output_hex']).decode(); fields={line.split(': ',1)[0]:line.split(': ',1)[1] for line in raw.splitlines() if ': ' in line}\n"
        "if 'FAIL:' not in raw or not all(k in fields for k in ('Finding','Reason','Evidence locator','Evidence SHA')): raise SystemExit('raw prose did not contain explicit FAIL grounds')\n"
        "schema=request['full_output_schema']; properties=schema['properties']; review=properties['judgments']['items']['oneOf'][0]\n"
        "judgment={'axis':review['properties']['axis']['enum'][0],'result':'fail','findings':fields['Finding'],'grounds':{'reason':fields['Reason'],'evidence':[{'locator':fields['Evidence locator'],'sha256':fields['Evidence SHA']}]}}\n"
        "output={'review_contract_version':properties['review_contract_version']['const'],'review_stage':properties['review_stage']['const'],'author':properties['author']['const'],'judgments':[judgment]}\n"
        "response={'output':json.dumps(output,separators=(',',':'))}\n"
        "if behavior.get('usage') is not None: response['usage']=behavior['usage']\n"
        "print(json.dumps(response,separators=(',',':')))\n"
        "if behavior.get('exit_code'): raise SystemExit(behavior['exit_code'])\n",
        encoding="utf-8",
    )


def _model_adapter_config(adapter: Path, counter: Path, behavior: Path, *, model_id: str = "fixture-format-model-v1", max_calls: int = 1, max_time_ms: int = 8000, max_cost: int = 20) -> dict[str, Any]:
    return {
        "kind": "model",
        "model_id": model_id,
        "command": sys.executable,
        "args": [str(adapter), str(counter), str(behavior)],
        "max_calls": max_calls,
        "max_time_ms": max_time_ms,
        "max_cost_micros": max_cost,
    }


def _adapter_config(adapter: Path, counter: Path, mode: str, *, max_cost: int = 20, max_time: int = 8000, max_calls: int = 1) -> dict[str, Any]:
    return {
        "kind": "scripted",
        "command": sys.executable,
        "args": [str(adapter), mode, str(counter)],
        "max_calls": max_calls,
        "max_time_ms": max_time,
        "max_cost_micros": max_cost,
    }


def _preview(journey, root: Path, engine: Path, origin_id: str, show: dict[str, Any]) -> dict[str, Any]:
    command = [str(engine), "recovery-preview", origin_id]
    show_payload = json.dumps(show, separators=(",", ":")).encode("utf-8")
    try:
        completed = subprocess.run(
            command,
            cwd=root,
            input=show_payload,
            capture_output=True,
            timeout=30,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise ValueError(f"recovery-preview did not complete: {error}") from error
    _record(root, command, completed, stdin_payload=show_payload)
    if completed.returncode != 0:
        raise ValueError(f"recovery-preview refused the expected quiescent source: {completed.stderr.decode(errors='replace')}")
    try:
        result = json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        raise ValueError(f"recovery-preview returned invalid JSON: {completed.stdout!r}") from error
    if result.get("ready") is not True:
        raise ValueError(f"recovery preview did not declare a ready, quiescent source: {result}")
    return result


def _preview_refused(journey, root: Path, engine: Path, origin_id: str, show: dict[str, Any], marker: str) -> None:
    command = [str(engine), "recovery-preview", origin_id]
    payload = json.dumps(show, separators=(",", ":")).encode("utf-8")
    completed = subprocess.run(command, cwd=root, input=payload, capture_output=True, timeout=30, check=False)
    _record(root, command, completed, stdin_payload=payload)
    detail = (completed.stdout + completed.stderr).decode(errors="replace").lower()
    if completed.returncode == 0 or marker.lower() not in detail:
        raise ValueError(f"recovery-preview did not refuse the {marker} negative: {detail[-1200:]}")


def _repair(journey, root: Path, engine: Path, request: dict[str, Any], path: Path, *, ok: bool = True) -> dict[str, Any]:
    _json(path, request)
    command = [str(engine), "recover-output", "@" + str(path)]
    try:
        completed = subprocess.run(command, cwd=root, capture_output=True, timeout=30, check=False)
    except (OSError, subprocess.TimeoutExpired) as error:
        raise ValueError(f"recover-output did not complete: {error}") from error
    _record(root, command, completed)
    if not ok:
        if completed.returncode != 2:
            raise ValueError(f"recover-output did not refuse: exit={completed.returncode} stderr={completed.stderr!r}")
        return {"refused": completed.stderr.decode(errors="replace")}
    if completed.returncode != 0:
        raise ValueError(f"recover-output failed: {completed.stderr.decode(errors='replace')}")
    try:
        return json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        raise ValueError(f"recover-output returned invalid JSON: {completed.stdout!r}") from error


def _process_identity(pid: int) -> dict[str, Any]:
    result = subprocess.run(
        ["ps", "-p", str(pid), "-o", "pid=", "-o", "lstart=", "-o", "command="],
        capture_output=True, text=True, check=False,
    )
    return {
        "returncode": result.returncode,
        "stdout": result.stdout.strip(),
        "stderr": result.stderr.strip(),
    }


def _repair_with_watchdog(root: Path, engine: Path, path: Path, *, timeout: float, observe) -> tuple[subprocess.CompletedProcess[bytes], int, bool, dict[str, int]]:
    command = [str(engine), "recover-output", "@" + str(path)]
    started = time.monotonic()
    process = subprocess.Popen(
        command, cwd=root, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
        stderr=subprocess.PIPE, start_new_session=True,
    )
    identities = {"pid": process.pid, "process_group_id": process.pid}
    watchdog_killed_group = False
    while process.poll() is None and time.monotonic() - started < timeout:
        observe()
        time.sleep(0.005)
    if process.poll() is None:
        # This group was created by the still-live Popen handle. Marker PIDs
        # are observed only and are never signaled by the fixture.
        os.killpg(process.pid, signal.SIGKILL)
        watchdog_killed_group = True
    try:
        stdout, stderr = process.communicate(timeout=3)
    except subprocess.TimeoutExpired:
        os.killpg(process.pid, signal.SIGKILL)
        watchdog_killed_group = True
        stdout, stderr = process.communicate(timeout=3)
    elapsed_ms = round((time.monotonic() - started) * 1000)
    completed = subprocess.CompletedProcess(command, process.returncode, stdout, stderr)
    _record(root, command, completed)
    return completed, elapsed_ms, watchdog_killed_group, identities


def _candidate_doc(journey, root: Path, show: dict[str, Any]) -> dict[str, Any]:
    command = [str(journey.provider), "review-candidates"]
    payload = json.dumps(show, separators=(",", ":")).encode("utf-8")
    completed = subprocess.run(command, cwd=root, input=payload, capture_output=True, timeout=30, check=False)
    _record(root, command, completed, stdin_payload=payload)
    if completed.returncode != 0:
        raise ValueError(f"public review-candidates failed: {completed.stderr[-1200:]!r}")
    try:
        return json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        raise ValueError(f"review-candidates returned invalid JSON: {error}") from error


def _append_record(journey, root: Path, database: Path, run_id: str, record_id: str, kind: str, data: dict[str, Any]):
    _show_action(journey, root, database, run_id)
    return _engine(
        journey, root, database, "append", "--record-id", record_id,
        run_id, kind, json.dumps(data, separators=(",", ":")),
    )[0]


def _event(journey, root: Path, database: Path, run_id: str, event_id: str, *, expect: str = "completed"):
    _show_action(journey, root, database, run_id)
    return _engine(journey, root, database, "event", run_id, event_id, expect=expect)[0]


def _snapshot_tree(checkout: Path, counter: Path) -> dict[str, Any]:
    def git(*args: str) -> bytes:
        result = subprocess.run(["git", *args], cwd=checkout, capture_output=True, check=False)
        if result.returncode != 0:
            raise ValueError(f"fixture git {' '.join(args)} failed: {result.stderr!r}")
        return result.stdout
    names = git("ls-files", "-co", "--exclude-standard", "-z").split(b"\0")
    files = {}
    for raw in names:
        if raw:
            name = raw.decode("utf-8")
            files[name] = (checkout / name).read_bytes()
    return {
        "head": git("rev-parse", "HEAD").decode().strip(),
        "index": git("ls-files", "--stage", "-z"),
        "status": git("status", "--porcelain=v1", "-z", "--untracked-files=all"),
        "files": files,
        "counter": counter.read_text(encoding="utf-8") if counter.exists() else "0",
    }


def _capture_manifest(root: Path) -> dict[str, str]:
    manifest = {}
    for path in sorted(root.rglob("*")):
        if path.is_file():
            manifest[str(path.relative_to(root))] = _sha(path.read_bytes())
    return manifest


def _assert_full_show_retention(root: Path) -> dict[str, int]:
    commands = root / "commands"
    full_show_count = 0
    preview_stdin_count = 0
    for argv_path in sorted(commands.glob("*.argv.json")):
        record = json.loads(argv_path.read_text(encoding="utf-8"))
        argv = record.get("argv")
        if not isinstance(argv, list) or not _full_show_command(argv):
            if isinstance(argv, list) and "recovery-preview" in argv:
                stdin_capture = record.get("stdin_capture", {})
                if (
                    stdin_capture.get("kind") != "transient-payload-facts"
                    or stdin_capture.get("raw_stdin_retained") is not False
                    or stdin_capture.get("byte_length", 0) <= 0
                    or not isinstance(stdin_capture.get("sha256"), str)
                ):
                    raise ValueError("recovery-preview did not record transient full-envelope stdin facts")
                preview_stdin_count += 1
            continue
        full_show_count += 1
        stem = argv_path.name.removesuffix(".argv.json")
        if (commands / f"{stem}.stdout").exists():
            raise ValueError("full show stdout was retained as an unbounded original stream")
        if record.get("stdout_capture") != {
            "kind": "bounded-full-show-projection",
            "path": f"{stem}.stdout-projection.json",
            "raw_stdout_retained": False,
        }:
            raise ValueError("full show stdout retention was not labeled as a projection")
        projection_path = commands / f"{stem}.stdout-projection.json"
        projection = json.loads(projection_path.read_text(encoding="utf-8"))
        original = projection.get("original_stdout", {})
        if (
            projection.get("kind") != "loop-engine-full-show-projection-v1"
            or projection.get("raw_stdout_retained") is not False
            or not isinstance(original.get("byte_length"), int)
            or original["byte_length"] <= 0
            or not isinstance(original.get("sha256"), str)
            or len(original["sha256"]) != 71
            or not original["sha256"].startswith("sha256:")
        ):
            raise ValueError("full show original stream facts or projection identity were not retained")
        facts = projection.get("projection", {})
        if not facts.get("available") or set(facts) != {"available", "run_id", "current_state", "work_slot_invocations"}:
            raise ValueError("full show projection did not retain only its bounded recovery facts")
        allowed_invocation_fields = {
            "invocation_id", "slot_id", "status", "exit_code", "capture_dir", "ownership"
        }
        for row in facts["work_slot_invocations"]:
            if not isinstance(row, dict) or not set(row).issubset(allowed_invocation_fields):
                raise ValueError("full show projection retained an unexpected invocation field")
        if not (commands / f"{stem}.stderr").is_file() or not (commands / f"{stem}.exit.json").is_file():
            raise ValueError("full show command lost its unaltered stderr or actual exit record")
    if full_show_count == 0 or preview_stdin_count == 0:
        raise ValueError("recovery fixture did not exercise full-show projection and transient preview stdin")
    if list(root.rglob("show-*.json")):
        raise ValueError("recovery fixture persisted a full show envelope for recovery-preview")
    return {"full_show_commands": full_show_count, "transient_preview_stdin_commands": preview_stdin_count}


def _invoke_started(journey, root: Path, database: Path, run_id: str, slot_id: str, invocation_input: dict[str, Any] | None = None, *, expect: str = "completed") -> dict[str, Any]:
    _show_action(journey, root, database, run_id)
    args = ["--timeout-ms", "120000", "invoke", run_id, slot_id]
    if invocation_input is not None:
        args.extend(["--input", json.dumps(invocation_input, separators=(",", ":"))])
    response, _ = _engine(journey, root, database, *args, expect=expect, timeout=30)
    if expect == "completed":
        return response["result"]
    return response


def _attach_derived(preview: dict[str, Any], derived: dict[str, Any], *, owner: bool = True, fidelity: bool = True, meaning: bool = True) -> dict[str, Any]:
    selected = derived["selected"]
    origin = derived["origin"]
    source = {
        "assignment_id": origin["assignment_id"],
        "source_class": "eligible-derived",
        "raw_attempt": origin["raw_attempt"],
        "raw_stdout_sha256": origin["raw_stdout_sha256"],
        "selected_output_path": selected["path"],
        "selected_output_sha256": selected["sha256"],
        "derivation": derived["derivation"],
    }
    if owner:
        source["owner_approval"] = {
            "name": "P05-isolated-fixture-authorization",
            "reason": "Owner-authorized scripted mechanics fixture only; this does not enable live repair in ordinary runs.",
        }
    if fidelity and meaning:
        source["fidelity_approval"] = {
            "name": "fixture-driver",
            "reason": "Compared the explicit result and effect/finding values to the original raw meaning.",
        }
    recovery_input = copy.deepcopy(preview["recovery_input"])
    recovery_input["pending_assignment_ids"] = [
        item for item in recovery_input["pending_assignment_ids"] if item != source["assignment_id"]
    ]
    recovery_input["sources"].append(source)
    return recovery_input


def _reject_recovery(journey, root: Path, database: Path, run_id: str, slot_id: str, value: dict[str, Any], marker: str, expected_rows: int) -> None:
    response = _invoke_started(journey, root, database, run_id, slot_id, value, expect="rejected")
    if response.get("code") != "invalid-fan-out-recovery" or marker.lower() not in response.get("message", "").lower():
        raise ValueError(f"recovery negative did not reject for {marker!r}: {response}")
    show, _ = _engine(journey, root, database, "show", "--view", "full", run_id)
    if len(show["result"]["work_slot_invocations"]) != expected_rows:
        raise ValueError("invalid recovery input spawned a waiter or wrote an invocation")


def _software_change_mechanical_case(journey, root: Path) -> dict[str, Any]:
    worker = root / "implementation-worker.py"
    counter = root / "implementation-launch-count"
    _worker(worker, "implementation")
    checkout = root / "implementation-checkout"
    checkout.mkdir()
    subprocess.run(["git", "init", "-q"], cwd=checkout, check=True)
    subprocess.run(["git", "config", "user.email", "fixture@example.test"], cwd=checkout, check=True)
    subprocess.run(["git", "config", "user.name", "Recovery Fixture"], cwd=checkout, check=True)
    (checkout / "tracked.txt").write_bytes(b"before\n")
    subprocess.run(["git", "add", "tracked.txt"], cwd=checkout, check=True)
    subprocess.run(["git", "commit", "-qm", "baseline"], cwd=checkout, check=True)
    worker_command = {
        "command": sys.executable,
        "args": [str(worker), "implementation", str(counter), str(checkout)],
        "title": "One ordinary implementation assignment",
        "role": "implementer",
        "output_schema": {"required": ["implementation_result", "repository_effect"]},
    }
    binding = _fanout_binding(journey.engine, [worker_command])
    database, artifacts, run_id, _ = _write_software_run(journey, root, "mechanical-implementation", "implement", binding)
    invoked = _invoke_started(journey, root, database, run_id, "implement")
    _, failed = _wait_invocation(journey, root, database, run_id, invoked["invocation_id"])
    if failed.get("status") != "failed":
        raise ValueError(f"missing required repository_effect did not fail the public outer invocation: {failed}")
    original_capture = Path(failed["capture_dir"])
    summary_bytes = (original_capture / "summary.json").read_bytes()
    summary = json.loads(summary_bytes)
    row = summary["workers"][0]
    raw_bytes = (original_capture / "0/stdout").read_bytes()
    raw_digest = _sha(raw_bytes)
    if row["exit_code"] != 0 or row["status"] != "failed" or row["raw_output_sha256"] != raw_digest:
        raise ValueError(f"implementation raw exit/conformance/digest facts were not retained: {row}")
    if row["declared_output_contract"] != {"required": ["implementation_result", "repository_effect"]}:
        raise ValueError(f"frozen legacy required-key contract was not retained: {row}")
    if (original_capture / "0/attempts.json").exists():
        raise ValueError("legacy output_schema implementation fixture fabricated attempts.json")
    raw_value = json.loads(raw_bytes)
    if set(raw_value) != {"implementation_result", "effect"}:
        raise ValueError(f"fixture did not emit the required substantive keys: {raw_value}")
    if counter.read_text() != "1":
        raise ValueError("implementation command did not launch exactly once")
    after_worker = _snapshot_tree(checkout, counter)
    before_repair_capture = _capture_manifest(original_capture)
    shown, _ = _engine(journey, root, database, "show", "--view", "full", run_id)
    preview = _preview(journey, root, journey.engine, invoked["invocation_id"], shown)
    assignments = {row["assignment_id"]: row for row in preview["assignments"]}
    if assignments["worker-0"]["classification"] != "invalid" or preview["recovery_input"]["pending_assignment_ids"] != ["worker-0"]:
        raise ValueError(f"implementation conformance failure was not previewed as pending: {preview}")
    repair_request = {"version": 1, "preview": preview, "assignment_id": "worker-0"}
    derived = _repair(journey, root, journey.engine, repair_request, root / "implementation-repair-request.json")
    derived_bytes = Path(derived["selected"]["path"]).read_bytes()
    normalized = json.loads(derived_bytes)
    expected_effect = raw_value["effect"]
    if normalized.get("implementation_result") != raw_value["implementation_result"] or normalized.get("repository_effect") != expected_effect or "effect" in normalized:
        raise ValueError(f"mechanical key restoration changed explicit implementation meaning: {normalized}")
    if derived["derivation"]["kind"] != "mechanical" or derived["derivation"]["difference"] != {"kind": "rename-key", "from": "effect", "to": "repository_effect"}:
        raise ValueError(f"mechanical correction was not separately attributed: {derived}")
    if _snapshot_tree(checkout, counter) != after_worker:
        raise ValueError("output-only mechanical repair changed the checkout or implementation counter")
    if _capture_manifest(original_capture) != before_repair_capture:
        raise ValueError("output repair modified the immutable failed parent capture")

    row_count = len(shown["result"]["work_slot_invocations"])
    base_input = _attach_derived(preview, derived)
    invalids = []
    wrong_target = copy.deepcopy(base_input); wrong_target["slot_id"] = "review"
    invalids.append((wrong_target, "target"))
    wrong_binding = copy.deepcopy(base_input); wrong_binding["binding_sha256"] = "sha256:" + "0" * 64
    invalids.append((wrong_binding, "binding"))
    wrong_origin = copy.deepcopy(base_input); wrong_origin["origin_invocation_id"] = "missing-origin"
    invalids.append((wrong_origin, "origin"))
    bad_digest = copy.deepcopy(base_input); bad_digest["sources"][0]["raw_stdout_sha256"] = "sha256:" + "0" * 64
    invalids.append((bad_digest, "raw attempt or digest"))
    incomplete = copy.deepcopy(base_input); incomplete["sources"] = []; incomplete["pending_assignment_ids"] = []
    invalids.append((incomplete, "cover"))
    missing_owner = copy.deepcopy(base_input); missing_owner["sources"][0].pop("owner_approval")
    invalids.append((missing_owner, "owner approval"))
    missing_fidelity = copy.deepcopy(base_input); missing_fidelity["sources"][0].pop("fidelity_approval")
    invalids.append((missing_fidelity, "fidelity approval"))
    for invalid, marker in invalids:
        _reject_recovery(journey, root, database, run_id, "implement", invalid, marker, row_count)

    # A changed result/effect cannot receive driver fidelity approval. The
    # public invoke path must refuse that unapproved derivative.
    changed_value = copy.deepcopy(normalized)
    changed_value["implementation_result"] = "different implementation meaning"
    changed_path = artifacts / "changed-derived-output.json"
    changed_path.write_text(json.dumps(changed_value, separators=(",", ":")), encoding="utf-8")
    changed_input = copy.deepcopy(preview["recovery_input"])
    changed_input["pending_assignment_ids"] = []
    changed_input["sources"] = [{
        "assignment_id": "worker-0", "source_class": "eligible-derived",
        "raw_attempt": derived["origin"]["raw_attempt"], "raw_stdout_sha256": raw_digest,
        "selected_output_path": str(changed_path), "selected_output_sha256": _sha(changed_path.read_bytes()),
        "derivation": {"kind": "mechanical", "difference": {"kind": "changed-meaning"}},
        "owner_approval": {"name": "fixture-owner", "reason": "No model permission is implied."},
    }]
    _reject_recovery(journey, root, database, run_id, "implement", changed_input, "fidelity approval", row_count)

    _show_action(journey, root, database, run_id)
    recovery_result, _ = _engine(
        journey, root, database, "--timeout-ms", "120000", "invoke", run_id, "implement",
        "--input", json.dumps(base_input, separators=(",", ":")), timeout=30,
    )
    if recovery_result["status"] != "completed":
        raise ValueError(f"public recovery join was not admitted: {recovery_result}")
    joined_show, joined = _wait_invocation(journey, root, database, run_id, recovery_result["result"]["invocation_id"])
    if joined.get("status") != "succeeded" or joined.get("exit_code") != 0:
        raise ValueError(f"recovery join did not produce a succeeded outer invocation: {joined}")
    if len(joined_show["result"]["work_slot_invocations"]) != row_count + 1:
        raise ValueError("recovery join did not create exactly one real outer invocation")
    joined_inner = joined["inner_workers"][0]
    source = joined_inner.get("recovery_source")
    if not isinstance(source, dict) or source.get("source_class") != "eligible-derived" or source.get("origin", {}).get("invocation_id") != invoked["invocation_id"]:
        raise ValueError(f"show did not expose derived selection and true raw origin: {joined_inner}")
    output_path = Path(joined["capture_dir"]) / joined_inner["selected_output_path"]
    if _sha(output_path.read_bytes()) != joined_inner["selected_output_sha256"]:
        raise ValueError("selected repaired bytes are not contained in the new invocation capture")
    if joined.get("invocation_input", {}).get("sources", [])[0].get("raw_stdout_sha256") != raw_digest:
        raise ValueError("public show did not retain the original raw stdout digest")
    if counter.read_text() != "1" or _snapshot_tree(checkout, counter) != after_worker:
        raise ValueError("recovery join reran implementation or changed checkout bytes/status/HEAD")
    if _capture_manifest(original_capture) != before_repair_capture or (original_capture / "summary.json").read_bytes() != summary_bytes:
        raise ValueError("recovery join mutated the immutable failed parent capture")
    return {
        "run_id": run_id,
        "failed_invocation": invoked["invocation_id"],
        "joined_invocation": joined["invocation_id"],
        "raw_sha256": raw_digest,
        "derived_sha256": derived["selected"]["sha256"],
        "launch_counter": counter.read_text(),
        "tree_status": (checkout / "tracked.txt").read_text(),
        "negative_refusals": len(invalids) + 1,
    }


def _software_change_full_schema_mechanical_case(journey, root: Path) -> dict[str, Any]:
    reviewer = root / "constant-reviewer.py"
    launch_counter = root / "constant-reviewer-launches"
    _worker(reviewer, "full-schema-missing-constants")
    database, artifacts, run_id = _setup_review_run(
        journey, root, "mechanical-fullschema", reviewer, launch_counter,
        mode="full-schema-missing-constants",
    )
    started = _invoke_started(journey, root, database, run_id, "intent-review")
    shown, failed = _wait_invocation(journey, root, database, run_id, started["invocation_id"])
    if failed.get("status") != "failed" or launch_counter.read_text() != "1":
        raise ValueError(f"full-schema mechanical fixture did not preserve one failed raw attempt: {failed}")
    capture = Path(failed["capture_dir"])
    raw_path = capture / "0/attempts/1/stdout"
    raw = raw_path.read_bytes()
    raw_value = json.loads(raw)
    if set(raw_value) != {"judgments"} or raw_value["judgments"][0].get("result") != "fail":
        raise ValueError(f"constant-restoration fixture did not retain explicit raw FAIL meaning: {raw_value}")
    original_capture = _capture_manifest(capture)
    preview = _preview(journey, root, journey.engine, started["invocation_id"], shown)
    derived = _repair(
        journey,
        root,
        journey.engine,
        {"version": 1, "preview": preview, "assignment_id": "worker-0"},
        root / "repair-full-schema-constants.json",
    )
    output_path = Path(derived["selected"]["path"])
    output = json.loads(output_path.read_text(encoding="utf-8"))
    if (
        output.get("review_contract_version") != 2
        or output.get("review_stage") != "aggregate"
        or output.get("author") != {"name": "fixture-reviewer", "kind": "agent"}
        or output.get("judgments") != raw_value["judgments"]
        or derived.get("derivation", {}).get("kind") != "mechanical"
        or derived.get("derivation", {}).get("difference", {}).get("kind")
        != "restore-frozen-constants"
    ):
        raise ValueError(f"mechanical repair did not restore only frozen constants: {derived}")
    if launch_counter.read_text() != "1" or _capture_manifest(capture) != original_capture:
        raise ValueError("full-schema mechanical repair relaunched the reviewer or changed raw capture")
    recovery_input = _attach_derived(preview, derived)
    _show_action(journey, root, database, run_id)
    joined_result, _ = _engine(
        journey, root, database, "--timeout-ms", "120000", "invoke", run_id,
        "intent-review", "--input", json.dumps(recovery_input, separators=(",", ":")),
    )
    joined_show, joined = _wait_invocation(
        journey, root, database, run_id, joined_result["result"]["invocation_id"]
    )
    worker = joined["inner_workers"][0]
    source = worker.get("recovery_source") or {}
    selected_path = Path(joined["capture_dir"]) / worker["selected_output_path"]
    if (
        joined.get("status") != "succeeded"
        or source.get("source_class") != "eligible-derived"
        or source.get("origin", {}).get("raw_stdout_sha256") != _sha(raw)
        or _sha(selected_path.read_bytes()) != derived["selected"]["sha256"]
        or launch_counter.read_text() != "1"
        or _capture_manifest(capture) != original_capture
    ):
        raise ValueError(f"mechanical output-only join lost raw origin or relaunched reviewer: {joined}")
    return {
        "run_id": run_id,
        "failed_invocation": started["invocation_id"],
        "joined_invocation": joined["invocation_id"],
        "raw_sha256": _sha(raw),
        "derived_sha256": derived["selected"]["sha256"],
        "reviewer_launches": launch_counter.read_text(),
        "selected_result": output["judgments"][0]["result"],
        "outer_status": joined["status"],
    }


def _software_change_model_repair_case(journey, root: Path) -> dict[str, Any]:
    case_root = root / "model-repair"
    case_root.mkdir(parents=True)
    reviewer = case_root / "prose-reviewer.py"
    reviewer_counter = case_root / "reviewer-launch-count"
    _worker(reviewer, "raw-review")
    database, artifacts, run_id = _setup_review_run(
        journey, case_root, "configured-model-review", reviewer, reviewer_counter
    )
    started = _invoke_started(journey, case_root, database, run_id, "intent-review")
    shown, failed = _wait_invocation(journey, case_root, database, run_id, started["invocation_id"])
    if failed.get("status") != "failed" or reviewer_counter.read_text() != "1":
        raise ValueError("normal setup/invoke did not preserve exactly one malformed explicit prose FAIL")
    capture = Path(failed["capture_dir"])
    raw_path = capture / "0/attempts/1/stdout"
    raw = raw_path.read_bytes()
    raw_text = raw.decode("utf-8")
    if "FAIL:" not in raw_text or "Finding: " not in raw_text or "Reason: " not in raw_text:
        raise ValueError("the configured model fixture did not start from explicit raw FAIL grounds")
    original_manifest = _capture_manifest(capture)
    preview = _preview(journey, case_root, journey.engine, started["invocation_id"], shown)
    adapter_script = case_root / "local-model-command.py"
    adapter_counter = case_root / "model-command-launch-count"
    behavior = case_root / "model-behavior.json"
    _model_adapter(adapter_script)
    _json(behavior, {"usage": {"calls": 1, "metered_cost_micros": 2}})

    disabled = _repair(
        journey, case_root, journey.engine,
        {"version": 1, "preview": preview, "assignment_id": "worker-0"},
        case_root / "model-disabled.json", ok=False,
    )
    if "no output-only adapter" not in disabled["refused"] or adapter_counter.exists():
        raise ValueError("missing model configuration did not refuse before execution")
    missing_identity = _model_adapter_config(
        adapter_script, adapter_counter, behavior
    )
    missing_identity.pop("model_id")
    identity_refusal = _repair(
        journey, case_root, journey.engine,
        {"version": 1, "preview": preview, "assignment_id": "worker-0", "adapter": missing_identity},
        case_root / "model-missing-identity.json", ok=False,
    )
    if "model_id" not in identity_refusal["refused"] or adapter_counter.exists():
        raise ValueError("missing model identity did not refuse before execution")
    missing_caps = _model_adapter_config(
        adapter_script, adapter_counter, behavior, max_cost=0
    )
    cap_refusal = _repair(
        journey, case_root, journey.engine,
        {"version": 1, "preview": preview, "assignment_id": "worker-0", "adapter": missing_caps},
        case_root / "model-missing-cap.json", ok=False,
    )
    if "positive" not in cap_refusal["refused"] or adapter_counter.exists():
        raise ValueError("missing positive model bound did not refuse before execution")

    config = _model_adapter_config(adapter_script, adapter_counter, behavior, max_calls=1, max_time_ms=8000, max_cost=2)
    request = {
        "version": 1,
        "preview": preview,
        "assignment_id": "worker-0",
        "adapter": config,
    }
    repaired = _repair(
        journey, case_root, journey.engine, request,
        case_root / "model-repair-request.json",
    )
    if adapter_counter.read_text() != "1" or reviewer_counter.read_text() != "1":
        raise ValueError("the explicitly selected local model command or original reviewer ran the wrong number of times")
    if repaired.get("derivation", {}).get("kind") != "model":
        raise ValueError("configured model command was not honestly attributed as a model derivation")
    usage = repaired.get("derivation", {}).get("adapter", {})
    if (
        usage.get("model_id") != config["model_id"]
        or usage.get("command") != config["command"]
        or usage.get("args") != config["args"]
        or usage.get("usage_accounted") is not True
        or usage.get("calls") != 1
        or usage.get("metered_cost_micros") != 2
        or not (0 < usage.get("elapsed_ms", 0) <= config["max_time_ms"])
        or usage.get("max_calls") != config["max_calls"]
        or usage.get("max_cost_micros") != config["max_cost_micros"]
    ):
        raise ValueError(f"configured model identity/bounds/metering were not retained: {usage}")
    adapter_capture = usage.get("capture", {})
    adapter_dir = Path(adapter_capture.get("directory", ""))
    if not adapter_dir.is_dir():
        raise ValueError("model invocation capture was not persisted")
    packet = json.loads((adapter_dir / "request.json").read_text(encoding="utf-8"))
    if (
        packet.get("representation_only") is not True
        or packet.get("raw_attempt") != 1
        or packet.get("raw_stdout_path") != "0/attempts/1/stdout"
        or packet.get("model_adapter") != {"model_id": config["model_id"], "command": config["command"], "args": config["args"]}
        or packet.get("budget", {}).get("max_calls") != 1
        or packet.get("budget", {}).get("max_time_ms") != 8000
        or packet.get("budget", {}).get("max_cost_micros") != 2
        or packet.get("budget", {}).get("remaining_calls") != 1
        or packet.get("budget", {}).get("remaining_time_ms") != 8000
        or packet.get("budget", {}).get("remaining_cost_micros") != 2
        or bytes.fromhex(packet.get("raw_output_hex", "")) != raw
    ):
        raise ValueError("local model command did not receive exact raw bytes and remaining identity/bounds")
    derived_path = Path(repaired["selected"]["path"])
    derived = json.loads(derived_path.read_text(encoding="utf-8"))
    judgment = derived.get("judgments", [{}])[0]
    if (
        judgment.get("result") != "fail"
        or judgment.get("findings") != "the fallback branch has no observable completion signal."
        or judgment.get("grounds", {}).get("reason")
        != "the selected fallback exposes no completion signal to the operator."
        or not repaired.get("derivation", {}).get("difference", {}).get("mechanical_decline")
        or repaired.get("origin", {}).get("raw_stdout_sha256") != _sha(raw)
        or repaired.get("selected", {}).get("sha256") != _sha(derived_path.read_bytes())
    ):
        raise ValueError(f"model representation changed the original FAIL or lost separate raw/derived identities: {derived}")
    if raw_path.read_bytes() != raw or _capture_manifest(capture) != original_manifest:
        raise ValueError("model output repair changed the immutable original raw capture")

    repeated = _repair(
        journey, case_root, journey.engine, request,
        case_root / "model-repeat-exhausted.json", ok=False,
    )
    if "call cap is exhausted" not in repeated["refused"] or adapter_counter.read_text() != "1":
        raise ValueError("repeated same-assignment model use reset its call budget or launched again")
    widened = copy.deepcopy(request)
    widened["adapter"]["max_calls"] = 2
    widened_refusal = _repair(
        journey, case_root, journey.engine, widened,
        case_root / "model-repeat-widened-cap.json", ok=False,
    )
    if "bounds differ" not in widened_refusal["refused"] or adapter_counter.read_text() != "1":
        raise ValueError("a repeated model request widened the same assignment budget")
    relabelled = copy.deepcopy(request)
    relabelled["adapter"]["model_id"] = "fixture-different-model-v1"
    identity_refusal = _repair(
        journey, case_root, journey.engine, relabelled,
        case_root / "model-repeat-changed-identity.json", ok=False,
    )
    if "identity or per-assignment bounds differ" not in identity_refusal["refused"] or adapter_counter.read_text() != "1":
        raise ValueError("a repeated model request changed its selected model identity to reset usage")

    recovery_input = _attach_derived(preview, repaired)
    same_owner_driver = "fixture-owner-driver"
    source = recovery_input["sources"][-1]
    source["owner_approval"] = {"name": same_owner_driver, "reason": "Authorized this deterministic local model adapter fixture only."}
    source["fidelity_approval"] = {"name": same_owner_driver, "reason": "Compared derived FAIL/findings/grounds with the immutable original prose."}
    joined = _invoke_started(
        journey, case_root, database, run_id, "intent-review", recovery_input
    )
    joined_show, joined_row = _wait_invocation(
        journey, case_root, database, run_id, joined["invocation_id"]
    )
    selected = joined_row["inner_workers"][0]
    recovery_source = selected.get("recovery_source") or {}
    if (
        joined_row.get("status") != "succeeded"
        or selected.get("selected_attempt") is not None
        or recovery_source.get("source_class") != "eligible-derived"
        or recovery_source.get("derivation", {}).get("kind") != "model"
        or recovery_source.get("derivation", {}).get("adapter", {}).get("model_id") != config["model_id"]
        or recovery_source.get("origin", {}).get("raw_stdout_sha256") != _sha(raw)
        or recovery_source.get("selected_output_sha256") != selected.get("selected_output_sha256")
        or reviewer_counter.read_text() != "1"
    ):
        raise ValueError(f"same-binding join lost truthful model attribution or relaunched the reviewer: {selected}")
    candidates = _candidate_doc(journey, case_root, joined_show)
    records = [row for row in candidates.get("records", []) if row.get("status") == "ready"]
    ready = [row for row in candidates.get("candidates", []) if row.get("status") == "ready"]
    if (
        len(ready) != 1
        or ready[0].get("result") != "fail"
        or ready[0].get("findings") != judgment["findings"]
        or len(records) != 1
        or records[0].get("data", {}).get("result") != "fail"
        or records[0].get("data", {}).get("origin", {}).get("id") != joined["invocation_id"]
    ):
        raise ValueError(f"model-derived candidate/source-linked append preview lost explicit FAIL: {candidates}")
    revision = json.loads((artifacts / "intent.json").read_text(encoding="utf-8"))["revision"]
    _append_record(journey, case_root, database, run_id, "model-fixture-initial-ledger", "finding-ledger", {
        "schema_version": "1", "gate": "intent-review", "subject": "intent.json",
        "subject_revision": revision, "author": {"name": "fixture-driver", "kind": "agent"},
        "findings": [],
    })
    _append_record(journey, case_root, database, run_id, records[0]["record_id"], "review-evidence", records[0]["data"])
    denied = _event(journey, case_root, database, run_id, "approved", expect="rejected")
    if "failed" not in json.dumps(denied).lower() and "finding" not in json.dumps(denied).lower():
        raise ValueError(f"checked gate accepted derived model FAIL: {denied}")
    if reviewer_counter.read_text() != "1" or adapter_counter.read_text() != "1":
        raise ValueError("recovery join or checked gate restarted substantive or representation work")
    return {
        "run_id": run_id,
        "failed_invocation": started["invocation_id"],
        "joined_invocation": joined["invocation_id"],
        "raw_sha256": _sha(raw),
        "derived_sha256": repaired["selected"]["sha256"],
        "model_id": usage["model_id"],
        "model_command": usage["command"],
        "model_calls": adapter_counter.read_text(),
        "original_reviewer_calls": reviewer_counter.read_text(),
        "candidate_result": ready[0]["result"],
        "checked_gate_denied_fail": True,
        "repeated_budget_refused": True,
        "model_identity_switch_refused": True,
    }


def _software_change_model_budget_negatives(journey, root: Path) -> dict[str, Any]:
    case_root = root / "model-budget-negatives"
    case_root.mkdir(parents=True)
    adapter_script = case_root / "local-model-command.py"
    _model_adapter(adapter_script)

    def source(label: str):
        scenario = case_root / label
        scenario.mkdir()
        reviewer = scenario / "prose-reviewer.py"
        reviewer_counter = scenario / "reviewer-launch-count"
        _worker(reviewer, "raw-review")
        database, artifacts, run_id = _setup_review_run(
            journey, scenario, label, reviewer, reviewer_counter
        )
        started = _invoke_started(journey, scenario, database, run_id, "intent-review")
        shown, failed = _wait_invocation(journey, scenario, database, run_id, started["invocation_id"])
        if failed.get("status") != "failed" or reviewer_counter.read_text() != "1":
            raise ValueError(f"{label} did not start from one ordinary malformed reviewer attempt")
        preview = _preview(journey, scenario, journey.engine, started["invocation_id"], shown)
        return scenario, artifacts, database, run_id, started, preview, reviewer_counter

    # Missing metering is retained and poisons the remaining assignment budget;
    # a repeated request cannot hide the unknown cost by starting a fresh call.
    scenario, artifacts, _database, _run_id, _started, preview, reviewer_counter = source("model-missing-usage")
    counter = scenario / "model-command-count"
    behavior = scenario / "behavior.json"
    _json(behavior, {"usage": None})
    request = {
        "version": 1, "preview": preview, "assignment_id": "worker-0",
        "adapter": _model_adapter_config(adapter_script, counter, behavior, max_calls=2, max_time_ms=8000, max_cost=20),
    }
    missing = _repair(journey, scenario, journey.engine, request, scenario / "missing-usage-request.json", ok=False)
    if "usage/response is missing or malformed" not in missing["refused"] or counter.read_text() != "1" or reviewer_counter.read_text() != "1":
        raise ValueError(f"missing model metering did not fail closed after exactly one command: {missing}")
    attempt_dirs = list((artifacts / "recovery-adapter-attempts/model").glob("*/*"))
    if len(attempt_dirs) != 1:
        raise ValueError("missing model usage attempt did not leave one durable attempt capture")
    usage_capture = json.loads((attempt_dirs[0] / "usage.json").read_text(encoding="utf-8"))
    if usage_capture.get("usage_accounted") is not False or usage_capture.get("metered_cost_micros") is not None:
        raise ValueError("missing model metering was not retained as explicitly unaccounted")
    repeated = _repair(journey, scenario, journey.engine, request, scenario / "missing-usage-repeat.json", ok=False)
    if "missing or unverifiable usage" not in repeated["refused"] or counter.read_text() != "1":
        raise ValueError("repeated request silently reset an unmetered assignment budget")

    # One exactly metered call may consume the entire configured cost cap; the
    # next request is refused before the dummy command starts again.
    scenario, artifacts, _database, _run_id, _started, preview, reviewer_counter = source("model-cost-exhaustion")
    counter = scenario / "model-command-count"
    behavior = scenario / "behavior.json"
    _json(behavior, {"usage": {"calls": 1, "metered_cost_micros": 2}})
    request = {
        "version": 1, "preview": preview, "assignment_id": "worker-0",
        "adapter": _model_adapter_config(adapter_script, counter, behavior, max_calls=2, max_time_ms=8000, max_cost=2),
    }
    _repair(journey, scenario, journey.engine, request, scenario / "cost-used.json")
    cost_exhausted = _repair(journey, scenario, journey.engine, request, scenario / "cost-exhausted.json", ok=False)
    if "metered-cost cap is exhausted" not in cost_exhausted["refused"] or counter.read_text() != "1" or reviewer_counter.read_text() != "1":
        raise ValueError(f"repeated model use reset exhausted metered cost: {cost_exhausted}")

    # A second attempt receives only remaining aggregate time. The fixture sleeps
    # beyond that bound; the child is killed/reaped and no fallback follows.
    scenario, artifacts, _database, _run_id, _started, preview, reviewer_counter = source("model-time-exhaustion")
    counter = scenario / "model-command-count"
    behavior = scenario / "behavior.json"
    pid_file = scenario / "timed-adapter-pid"
    _json(behavior, {"sleep_ms": 220, "pid_path": str(pid_file), "usage": {"calls": 1, "metered_cost_micros": 2}})
    request = {
        "version": 1, "preview": preview, "assignment_id": "worker-0",
        "adapter": _model_adapter_config(adapter_script, counter, behavior, max_calls=3, max_time_ms=350, max_cost=20),
    }
    _repair(journey, scenario, journey.engine, request, scenario / "time-first-call.json")
    time_exhausted = _repair(journey, scenario, journey.engine, request, scenario / "time-exhausted.json", ok=False)
    if "remaining per-assignment time cap" not in time_exhausted["refused"] or counter.read_text() != "2" or reviewer_counter.read_text() != "1":
        raise ValueError(f"model time bound did not stop/reap the bounded second call: {time_exhausted}")
    timed_pid = int(pid_file.read_text(encoding="utf-8"))
    process_probe = subprocess.run(["ps", "-p", str(timed_pid), "-o", "pid="], capture_output=True, text=True, check=False)
    budget_dir = next((artifacts / "recovery-adapter-attempts/model").glob("*"))
    if process_probe.returncode == 0 and process_probe.stdout.strip() or (budget_dir / "budget.lock").exists():
        raise ValueError("timed-out model adapter or assignment budget lock survived refusal")
    timed_attempts = list((artifacts / "recovery-adapter-attempts/model").glob("*/*"))
    if len(timed_attempts) != 2 or not all((attempt / name).is_file() for attempt in timed_attempts for name in ("request.json", "configuration.json", "stdout", "stderr", "usage.json")):
        raise ValueError("timed-out model attempt did not retain its complete refusal capture")
    timed_usage = [json.loads((attempt / "usage.json").read_text(encoding="utf-8")) for attempt in timed_attempts]
    if not any(item.get("usage_accounted") is False for item in timed_usage):
        raise ValueError("timed-out model usage was not marked unaccounted")
    retry_after_timeout = _repair(journey, scenario, journey.engine, request, scenario / "time-timeout-repeat.json", ok=False)
    if "missing or unverifiable usage" not in retry_after_timeout["refused"] or counter.read_text() != "2":
        raise ValueError("timeout accounting permitted an unmetered model retry")

    # A normal trusted wrapper can spawn a finite backend that inherits both
    # output pipes. The outer watchdog owns only the public command's process
    # group; the engine must itself terminate its adapter group at the short cap.
    scenario, artifacts, _database, _run_id, _started, preview, reviewer_counter = source(
        "model-wrapper-subprocess-timeout"
    )
    wrapper_script = scenario / "trusted-wrapper.py"
    backend_script = scenario / "finite-slow-backend.py"
    wrapper_pid_file = scenario / "wrapper-process.json"
    backend_pid_file = scenario / "backend-process.json"
    launch_count = scenario / "wrapper-launch-count"
    backend_sleep_ms = 2500
    wrapper_script.write_text(
        "import hashlib,json,os,pathlib,subprocess,sys\n"
        "raw=sys.stdin.buffer.read(); request=json.loads(raw)\n"
        "pathlib.Path(sys.argv[2]).write_text(json.dumps({'pid':os.getpid(),'ppid':os.getppid(),'request_bytes':len(raw),'request_sha256':hashlib.sha256(raw).hexdigest()}))\n"
        "counter=pathlib.Path(sys.argv[4]); counter.write_text(str((int(counter.read_text()) if counter.exists() else 0)+1))\n"
        "subprocess.run([sys.executable,sys.argv[1],sys.argv[3]],check=True)\n"
        "print(json.dumps({'output':'{}','usage':{'calls':1,'metered_cost_micros':1}}),flush=True)\n",
        encoding="utf-8",
    )
    backend_script.write_text(
        "import json,os,pathlib,sys,time\n"
        "pathlib.Path(sys.argv[1]).write_text(json.dumps({'pid':os.getpid(),'ppid':os.getppid(),'monotonic_ns':time.monotonic_ns()}))\n"
        "print('h1-backend-stdout-start',flush=True); print('h1-backend-stderr-start',file=sys.stderr,flush=True)\n"
        f"time.sleep({backend_sleep_ms}/1000)\n"
        "print('h1-backend-stdout-finish',flush=True); print('h1-backend-stderr-finish',file=sys.stderr,flush=True)\n",
        encoding="utf-8",
    )
    request = {
        "version": 1, "preview": preview, "assignment_id": "worker-0",
        "adapter": {
            "kind": "model", "model_id": "h1-wrapper-subprocess-fixture-v1",
            "command": sys.executable,
            "args": [str(wrapper_script), str(backend_script), str(wrapper_pid_file),
                     str(backend_pid_file), str(launch_count)],
            "max_calls": 2, "max_time_ms": 300, "max_cost_micros": 20,
        },
    }
    request_path = scenario / "wrapper-timeout-request.json"
    _json(request_path, request)
    observed_processes: dict[str, dict[str, Any]] = {}

    def observe_wrapper_tree() -> None:
        for name, path, expected_script in (
            ("wrapper", wrapper_pid_file, wrapper_script),
            ("backend", backend_pid_file, backend_script),
        ):
            if name in observed_processes or not path.is_file():
                continue
            marker = json.loads(path.read_text(encoding="utf-8"))
            identity = _process_identity(marker["pid"])
            if identity["returncode"] != 0 or str(expected_script) not in identity["stdout"]:
                raise ValueError(f"could not observe the live test-owned {name} process identity: {identity}")
            observed_processes[name] = {"marker": marker, "native_process_identity": identity}

    public_timeout, public_elapsed_ms, watchdog_killed_group, public_handle = _repair_with_watchdog(
        scenario, journey.engine, request_path, timeout=7.0, observe=observe_wrapper_tree
    )
    if watchdog_killed_group or public_timeout.returncode != 2 or public_elapsed_ms >= 1500:
        raise ValueError(
            "bounded model refusal did not promptly finish before the inherited-pipe backend: "
            f"exit={public_timeout.returncode} elapsed_ms={public_elapsed_ms} "
            f"watchdog_killed_group={watchdog_killed_group}"
        )
    refusal_text = public_timeout.stderr.decode(errors="replace")
    if "remaining per-assignment time cap" not in refusal_text:
        raise ValueError(f"wrapper subprocess timeout was not refused with the assignment bound: {refusal_text}")
    if set(observed_processes) != {"wrapper", "backend"}:
        raise ValueError(f"public timeout did not observe both live process identities: {observed_processes}")
    process_cleanup = {}
    for name, marker_path, expected_script in (
        ("wrapper", wrapper_pid_file, wrapper_script),
        ("backend", backend_pid_file, backend_script),
    ):
        marker = json.loads(marker_path.read_text(encoding="utf-8"))
        after = _process_identity(marker["pid"])
        if after["returncode"] == 0 and str(expected_script) in after["stdout"]:
            raise ValueError(f"timed-out test-owned {name} process remained alive: {after}")
        process_cleanup[name] = {"pid": marker["pid"], "before": observed_processes[name]["native_process_identity"], "after": after}
    if launch_count.read_text(encoding="utf-8") != "1" or reviewer_counter.read_text(encoding="utf-8") != "1":
        raise ValueError("wrapper timeout changed adapter or reviewer launch counts")

    target = preview["target"]
    scope_key = hashlib.sha256(
        f"{target['run_id']}:{target['slot_id']}:{target['state_visit']}:{target['binding_sha256']}:worker-0".encode()
    ).hexdigest()
    budget_dir = artifacts / "recovery-adapter-attempts" / "model" / scope_key
    attempts = sorted(path for path in budget_dir.iterdir() if path.is_dir())
    if len(attempts) != 1:
        raise ValueError(f"wrapper timeout did not retain exactly one immutable attempt: {attempts}")
    attempt = attempts[0]
    required_capture = ("request.json", "configuration.json", "stdout", "stderr", "usage.json")
    if not all((attempt / name).is_file() for name in required_capture):
        raise ValueError("wrapper timeout capture omitted request/configuration/stream/usage evidence")
    captured_stdout = (attempt / "stdout").read_bytes()
    captured_stderr = (attempt / "stderr").read_bytes()
    captured_request = (attempt / "request.json").read_bytes()
    immutable_capture_hashes = {
        name: _sha((attempt / name).read_bytes()) for name in required_capture
    }
    usage = json.loads((attempt / "usage.json").read_text(encoding="utf-8"))
    wrapper_request = observed_processes["wrapper"]["marker"]
    if (
        wrapper_request.get("request_bytes") != len(captured_request)
        or wrapper_request.get("request_sha256") != hashlib.sha256(captured_request).hexdigest()
        or captured_stdout != b"h1-backend-stdout-start\n"
        or captured_stderr != b"h1-backend-stderr-start\n"
        or usage.get("usage_accounted") is not False
        or usage.get("metered_cost_micros") is not None
        or not 0 < usage.get("elapsed_ms", 0) < 1500
        or len(captured_stdout) > 1024 * 1024
        or len(captured_stderr) > 1024 * 1024
        or (budget_dir / "budget.lock").exists()
    ):
        raise ValueError("timeout streams/unknown-usage capture or lock did not complete within bounds")
    retry_after_wrapper_timeout = _repair(
        journey, scenario, journey.engine, request, scenario / "wrapper-timeout-repeat.json", ok=False
    )
    if (
        "missing or unverifiable usage" not in retry_after_wrapper_timeout["refused"]
        or launch_count.read_text(encoding="utf-8") != "1"
        or reviewer_counter.read_text(encoding="utf-8") != "1"
        or len([path for path in budget_dir.iterdir() if path.is_dir()]) != 1
    ):
        raise ValueError("same-scope retry bypassed the immutable unknown-usage timeout refusal")
    after_retry_hashes = {
        name: _sha((attempt / name).read_bytes()) for name in required_capture
    }
    if after_retry_hashes != immutable_capture_hashes:
        raise ValueError("same-scope refusal modified the immutable timeout request/stream/usage capture")
    wrapper_subprocess_timeout = {
        "elapsed_bound_refused_promptly": True,
        "public_recover_output_elapsed_ms": public_elapsed_ms,
        "configured_time_cap_ms": 300,
        "finite_backend_sleep_ms": backend_sleep_ms,
        "outer_watchdog_seconds": 7.0,
        "outer_watchdog_killed_test_group": watchdog_killed_group,
        "test_owned_public_process_group": public_handle,
        "adapter_launch_count": launch_count.read_text(encoding="utf-8"),
        "reviewer_launch_count": reviewer_counter.read_text(encoding="utf-8"),
        "wrapper_backend_process_cleanup": process_cleanup,
        "capture": {
            "attempt_dir": str(attempt), "files": list(required_capture),
            "stdout_sha256": _sha(captured_stdout), "stdout_bytes": len(captured_stdout),
            "stderr_sha256": _sha(captured_stderr), "stderr_bytes": len(captured_stderr),
            "usage": usage, "budget_lock_released": True,
            "immutable_capture_hashes_before_and_after_retry": immutable_capture_hashes,
        },
        "same_scope_retry_refused_without_launch": True,
    }
    return {
        "missing_usage_refused": True,
        "missing_usage_repeated_without_launch": True,
        "cost_cap_exhausted_without_launch": True,
        "cumulative_time_exhausted_and_child_reaped": True,
        "retry_after_timeout_refused": True,
        "wrapper_subprocess_timeout": wrapper_subprocess_timeout,
    }


def _software_change_model_cross_origin_budget_case(journey, root: Path) -> dict[str, Any]:
    case_root = root / "model-cross-origin-budget"
    case_root.mkdir(parents=True)
    reviewer = case_root / "prose-reviewer.py"
    reviewer_counter = case_root / "reviewer-launch-count"
    _worker(reviewer, "raw-review")
    database, artifacts, run_id = _setup_review_run(
        journey, case_root, "cross-origin-review", reviewer, reviewer_counter
    )
    model_script = case_root / "local-model-command.py"
    _model_adapter(model_script)
    model_counter = case_root / "model-command-count"
    behavior = case_root / "behavior.json"
    _json(behavior, {"usage": {"calls": 1, "metered_cost_micros": 2}})
    config = _model_adapter_config(
        model_script, model_counter, behavior,
        model_id="fixture-cross-origin-model-v1",
        max_calls=2,
        max_time_ms=12000,
        max_cost=20,
    )
    origin_ids = []
    raw_digests = []
    scope_identity = None
    refusal = None
    selected_preview = None
    selected_derived = None
    for index in range(3):
        started = _invoke_started(journey, case_root, database, run_id, "intent-review")
        shown, failed = _wait_invocation(journey, case_root, database, run_id, started["invocation_id"])
        if failed.get("status") != "failed":
            raise ValueError("new raw origin did not preserve a failed original review")
        origin_ids.append(started["invocation_id"])
        raw = (Path(failed["capture_dir"]) / "0/attempts/1/stdout").read_bytes()
        raw_digests.append(_sha(raw))
        preview = _preview(journey, case_root, journey.engine, started["invocation_id"], shown)
        identity = preview["target"]
        current_scope = (identity["run_id"], identity["slot_id"], identity["state_visit"], identity["binding_sha256"], "worker-0")
        if scope_identity is None:
            scope_identity = current_scope
        elif current_scope != scope_identity:
            raise ValueError("a new failed origin changed the engine-generated assignment budget scope")
        request = {"version": 1, "preview": preview, "assignment_id": "worker-0", "adapter": config}
        if index < 2:
            derived = _repair(journey, case_root, journey.engine, request, case_root / f"model-call-{index + 1}.json")
            if model_counter.read_text() != str(index + 1):
                raise ValueError("the configured model command did not consume one call for its explicit origin")
            if index == 1:
                selected_preview = preview
                selected_derived = derived
        else:
            refusal = _repair(
                journey, case_root, journey.engine, request,
                case_root / "model-new-origin-budget-refusal.json", ok=False,
            )
            if "call cap is exhausted" not in refusal["refused"] or model_counter.read_text() != "2":
                raise ValueError("a new failed origin reset the same assignment's model-call budget")
    if len(set(origin_ids)) != 3 or reviewer_counter.read_text() != "3":
        raise ValueError("the cross-origin budget proof did not retain three distinct substantive attempts")
    if len(raw_digests) != 3 or any(not digest.startswith("sha256:") for digest in raw_digests):
        raise ValueError("the cross-origin budget proof omitted a raw attempt identity")
    run_id_key, slot_id, visit, binding_sha, assignment = scope_identity
    key = hashlib.sha256(f"{run_id_key}:{slot_id}:{visit}:{binding_sha}:{assignment}".encode()).hexdigest()
    budget_dir = artifacts / "recovery-adapter-attempts/model" / key
    attempt_dirs = [path for path in budget_dir.iterdir() if path.is_dir()]
    if len(attempt_dirs) != 2:
        raise ValueError("different failed origins did not reuse one durable assignment budget directory")
    captured_origins = sorted(
        json.loads((path / "configuration.json").read_text(encoding="utf-8"))["origin_invocation_id"]
        for path in attempt_dirs
    )
    if captured_origins != sorted(origin_ids[:2]):
        raise ValueError("model budget captures lost the distinct raw-origin invocation identities")
    if selected_preview is None or selected_derived is None:
        raise ValueError("cross-origin proof did not retain the second origin's derived output")
    recovery_input = _attach_derived(selected_preview, selected_derived)
    joined = _invoke_started(journey, case_root, database, run_id, "intent-review", recovery_input)
    joined_show, joined_row = _wait_invocation(journey, case_root, database, run_id, joined["invocation_id"])
    selected = joined_row["inner_workers"][0]
    source_record = selected.get("recovery_source") or {}
    if (
        joined_row.get("status") != "succeeded"
        or source_record.get("derivation", {}).get("kind") != "model"
        or source_record.get("origin", {}).get("invocation_id") != origin_ids[1]
        or source_record.get("origin", {}).get("raw_stdout_sha256") != raw_digests[1]
        or source_record.get("derivation", {}).get("adapter", {}).get("model_id") != config["model_id"]
        or model_counter.read_text() != "2"
        or reviewer_counter.read_text() != "3"
    ):
        raise ValueError("same-binding join did not verify aggregate usage and the selected distinct raw origin")
    candidates = _candidate_doc(journey, case_root, joined_show)
    ready = [row for row in candidates.get("candidates", []) if row.get("status") == "ready"]
    records = [row for row in candidates.get("records", []) if row.get("status") == "ready"]
    if len(ready) != 1 or ready[0].get("result") != "fail" or len(records) != 1 or records[0].get("data", {}).get("result") != "fail":
        raise ValueError("provider admission did not preserve the cross-origin model-derived FAIL")
    _append_record(journey, case_root, database, run_id, "cross-origin-initial-ledger", "finding-ledger", {
        "schema_version": "1", "gate": "intent-review", "subject": "intent.json",
        "subject_revision": json.loads((artifacts / "intent.json").read_text(encoding="utf-8"))["revision"],
        "author": {"name": "fixture-driver", "kind": "agent"}, "findings": [],
    })
    _append_record(journey, case_root, database, run_id, records[0]["record_id"], "review-evidence", records[0]["data"])
    denied = _event(journey, case_root, database, run_id, "approved", expect="rejected")
    if "failed" not in json.dumps(denied).lower() and "finding" not in json.dumps(denied).lower():
        raise ValueError("checked gate did not block the cross-origin model-derived FAIL")
    return {
        "assignment_scope": {"run_id": run_id_key, "slot_id": slot_id, "state_visit": visit, "binding_sha256": binding_sha, "assignment_id": assignment},
        "distinct_failed_origins": origin_ids,
        "raw_attempt_sha256": raw_digests,
        "accepted_model_calls_across_origins": model_counter.read_text(),
        "third_origin_refused_without_launch": True,
        "second_origin_joined_without_review_restart": True,
        "cross_origin_candidate_result": ready[0]["result"],
        "cross_origin_checked_gate_denied_fail": True,
        "reviewer_attempts_are_explicitly_observable": reviewer_counter.read_text(),
    }


def _software_change_scripted_review_case(journey, root: Path) -> dict[str, Any]:
    reviewer = root / "prose-reviewer.py"
    review_counter = root / "reviewer-launch-count"
    _worker(reviewer, "raw-review")
    database, artifacts, run_id = _setup_review_run(
        journey, root, "scripted-review", reviewer, review_counter
    )
    started = _invoke_started(journey, root, database, run_id, "intent-review")
    shown, failed = _wait_invocation(journey, root, database, run_id, started["invocation_id"])
    if failed.get("status") != "failed":
        raise ValueError(f"explicit prose FAIL did not fail output conformance: {failed}")
    cleanup_pending_show = copy.deepcopy(shown)
    cleanup_origin = next(
        row for row in cleanup_pending_show["result"]["work_slot_invocations"]
        if row["invocation_id"] == started["invocation_id"]
    )
    cleanup_origin.setdefault("ownership", {})["cleanup_pending"] = True
    _preview_refused(
        journey, root, journey.engine, started["invocation_id"],
        cleanup_pending_show, "cleanup pending",
    )
    capture = Path(failed["capture_dir"])
    raw = (capture / "0/stdout").read_bytes()
    prose = raw.decode("utf-8")
    expected_finding = "the fallback branch has no observable completion signal."
    if (
        "FAIL:" not in prose
        or "Finding: " + expected_finding not in prose
        or "Reason: the selected fallback exposes no completion signal to the operator." not in prose
        or "Evidence locator: scripted-review/artifacts/intent.json#L1-L1" not in prose
    ):
        raise ValueError(f"review fixture was not a grounded explicit substantive prose FAIL: {prose!r}")
    manifest = json.loads((capture / "0/attempts.json").read_text(encoding="utf-8"))
    if (
        manifest.get("schema_version") != "2"
        or manifest.get("recovery_state") != "awaiting-output-repair"
        or manifest.get("selected_attempt") is not None
        or manifest.get("exhausted") is not False
        or len(manifest.get("attempts", [])) != 1
        or review_counter.read_text() != "1"
    ):
        raise ValueError(f"full-schema first-invalid state was not retained without substantive retry: {manifest}")
    attempt_page, _ = _engine(
        journey, root, database, "read", run_id, "--kind", "attempt",
        "--assignment", "worker-0", "--invocation", started["invocation_id"],
    )
    attempts = attempt_page["result"]["items"]
    if (
        attempt_page["result"].get("total") != 1
        or len(attempts) != 1
        or attempts[0].get("attempt") != 1
        or attempts[0].get("failed") is not True
        or attempts[0].get("stdout_path") != "0/attempts/1/stdout"
    ):
        raise ValueError(f"public attempt read did not expose the single failed raw output: {attempt_page}")
    initial_candidates = _candidate_doc(journey, root, shown)
    if (
        len(initial_candidates.get("candidates", [])) != 1
        or initial_candidates["candidates"][0].get("status") != "missing-selection"
        or "result" in initial_candidates["candidates"][0]
    ):
        raise ValueError(f"candidate admission treated repair-available raw prose as selected/exhausted: {initial_candidates}")
    original_manifest = _capture_manifest(capture)
    preview = _preview(journey, root, journey.engine, started["invocation_id"], shown)
    preview_row = preview["assignments"][0]
    if (
        preview_row.get("classification") != "invalid"
        or preview_row.get("attempt_manifest", {}).get("schema_version") != "2"
        or preview_row.get("attempt_manifest", {}).get("recovery_state") != "awaiting-output-repair"
        or preview["recovery_input"]["pending_assignment_ids"] != ["worker-0"]
    ):
        raise ValueError(f"recovery preview misclassified repair-first full-schema output: {preview_row}")
    adapter_script = root / "scripted-output-only-adapter.py"
    _adapter(adapter_script)
    good_calls = root / "good-adapter-count"
    faithful = _repair(journey, root, journey.engine, {
        "version": 1, "preview": preview, "assignment_id": "worker-0",
        "scripted_adapter": _adapter_config(adapter_script, good_calls, "faithful"),
    }, root / "repair-review-faithful.json")
    if good_calls.read_text() != "1" or faithful["derivation"]["kind"] != "scripted":
        raise ValueError("bounded scripted representation-only adapter was not actually called exactly once")
    usage = faithful["derivation"].get("adapter")
    if not isinstance(usage, dict) or usage.get("usage_accounted") is not True or usage.get("calls") != 1 or not (0 < usage.get("elapsed_ms", 0) <= usage.get("max_time_ms", 0)) or usage.get("metered_cost_micros", 0) > usage.get("max_cost_micros", 0):
        raise ValueError(f"scripted repair omitted enforceable positive usage/bounds: {faithful}")
    derived = json.loads(Path(faithful["selected"]["path"]).read_text(encoding="utf-8"))
    derived_judgment = derived.get("judgments", [{}])[0]
    if (
        derived.get("review_contract_version") != 2
        or derived.get("review_stage") != "aggregate"
        or derived.get("author") != {"name": "fixture-reviewer", "kind": "agent"}
        or derived_judgment.get("axis") != "solution-agnostic"
        or derived_judgment.get("result") != "fail"
        or derived_judgment.get("findings") != expected_finding
        or derived_judgment.get("grounds", {}).get("reason")
        != "the selected fallback exposes no completion signal to the operator."
        or derived_judgment.get("grounds", {}).get("evidence", [{}])[0].get("locator")
        != "scripted-review/artifacts/intent.json#L1-L1"
    ):
        raise ValueError(f"scripted adapter changed explicit raw FAIL meaning: {derived}")
    mechanical_decline = faithful["derivation"].get("difference", {}).get("mechanical_decline")
    if not isinstance(mechanical_decline, str) or not mechanical_decline:
        raise ValueError("mechanical normalization did not explicitly decline prose before adapter call")
    if faithful["origin"]["raw_stdout_sha256"] != _sha(raw) or faithful["selected"]["sha256"] != _sha(Path(faithful["selected"]["path"]).read_bytes()):
        raise ValueError("scripted repair did not retain raw and separate derived digests")
    if not faithful.get("adapter_capture", {}).get("capture", {}).get("directory"):
        raise ValueError("scripted adapter stdout/stderr/request capture was omitted")

    # The driver refuses changed meaning by withholding fidelity approval. A
    # conforming shape alone cannot authorize a selected source.
    changed_calls = root / "changed-adapter-count"
    changed = _repair(journey, root, journey.engine, {
        "version": 1, "preview": preview, "assignment_id": "worker-0",
        "scripted_adapter": _adapter_config(adapter_script, changed_calls, "changed"),
    }, root / "repair-review-changed.json")
    changed_output = json.loads(Path(changed["selected"]["path"]).read_text(encoding="utf-8"))
    if changed_output.get("judgments", [{}])[0].get("findings") == expected_finding:
        raise ValueError("changed-meaning negative fixture did not change the original finding")
    changed_input = _attach_derived(preview, changed, owner=True, fidelity=False, meaning=False)
    initial_count = len(shown["result"]["work_slot_invocations"])
    _reject_recovery(journey, root, database, run_id, "intent-review", changed_input, "fidelity approval", initial_count)

    # Missing required meaning cannot be completed by formatting.
    missing_calls = root / "missing-adapter-count"
    _repair(journey, root, journey.engine, {
        "version": 1, "preview": preview, "assignment_id": "worker-0",
        "scripted_adapter": _adapter_config(adapter_script, missing_calls, "missing"),
    }, root / "repair-review-missing.json", ok=False)
    if missing_calls.read_text() != "1":
        raise ValueError("missing-meaning adapter was not actually exercised")

    # A real scripted call whose meter exceeds the declared positive cap is
    # refused, with the raw adapter call retained outside the failed capture.
    over_calls = root / "overbudget-adapter-count"
    refused = _repair(journey, root, journey.engine, {
        "version": 1, "preview": preview, "assignment_id": "worker-0",
        "scripted_adapter": _adapter_config(adapter_script, over_calls, "overbudget", max_cost=1),
    }, root / "repair-review-overbudget.json", ok=False)
    if over_calls.read_text() != "1" or "metered-cost cap" not in refused["refused"]:
        raise ValueError(f"exceeded-cost scripted repair did not refuse after a real bounded call: {refused}")
    attempt_root = artifacts / "recovery-adapter-attempts"
    overbudget_receipts = []
    for usage_path in attempt_root.glob("*/usage.json"):
        usage = json.loads(usage_path.read_text(encoding="utf-8"))
        config = json.loads(usage_path.with_name("configuration.json").read_text(encoding="utf-8"))
        if config.get("max_cost_micros") == 1:
            overbudget_receipts.append((usage_path, usage))
    if len(overbudget_receipts) != 1 or overbudget_receipts[0][1].get("metered_cost_micros", 0) <= 1:
        raise ValueError("over-budget adapter call/usage/cap was not retained truthfully")
    if not overbudget_receipts[0][0].with_name("stdout").is_file():
        raise ValueError("over-budget scripted adapter raw output was not retained")

    unmetered_calls = root / "unmetered-adapter-count"
    unmetered_refusal = _repair(journey, root, journey.engine, {
        "version": 1, "preview": preview, "assignment_id": "worker-0",
        "scripted_adapter": _adapter_config(adapter_script, unmetered_calls, "unmetered"),
    }, root / "repair-review-unmetered.json", ok=False)
    if unmetered_calls.read_text() != "1" or "retained capture" not in unmetered_refusal["refused"]:
        raise ValueError("unmetered adapter response was not refused and captured")
    model_calls = root / "model-adapter-must-not-run"
    model_request = {"version": 1, "preview": preview, "assignment_id": "worker-0",
        "scripted_adapter": {"kind":"model","command":sys.executable,"args":[str(adapter_script),"faithful",str(model_calls)],
                              "max_calls":1,"max_time_ms":1000,"max_cost_micros":1}}
    model_refusal = _repair(journey, root, journey.engine, model_request, root / "repair-review-model-disabled.json", ok=False)
    if "model_id" not in model_refusal["refused"] or model_calls.exists():
        raise ValueError("legacy model-kind request without selected identity launched or failed to refuse")
    disabled_caps = _repair(journey, root, journey.engine, {
        "version": 1, "preview": preview, "assignment_id": "worker-0",
        "scripted_adapter": _adapter_config(adapter_script, root / "zero-cap-must-not-run", "faithful", max_cost=0),
    }, root / "repair-review-zero-cap.json", ok=False)
    if "positive" not in disabled_caps["refused"]:
        raise ValueError("zero scripted-adapter bound was not refused")

    base_input = _attach_derived(preview, faithful)
    if review_counter.read_text() != "1":
        raise ValueError("output repair reran the original reviewer")
    _show_action(journey, root, database, run_id)
    invoked, _ = _engine(
        journey, root, database, "--timeout-ms", "120000", "invoke", run_id, "intent-review",
        "--input", json.dumps(base_input, separators=(",", ":")), timeout=30,
    )
    joined_show, joined = _wait_invocation(journey, root, database, run_id, invoked["result"]["invocation_id"])
    if joined.get("status") != "succeeded" or joined.get("exit_code") != 0:
        raise ValueError(f"scripted derived-source join did not succeed: {joined}")
    selected = joined["inner_workers"][0]
    source = selected.get("recovery_source") or {}
    if source.get("source_class") != "eligible-derived" or source.get("origin", {}).get("raw_stdout_sha256") != _sha(raw):
        raise ValueError(f"join did not retain original raw prose FAIL as derived-source origin: {source}")
    delivery = source.get("origin", {})
    original_input = capture / delivery.get("delivered_input_path", "")
    joined_input = Path(joined["capture_dir"]) / "0/stdin"
    if (
        not original_input.is_file()
        or not joined_input.is_file()
        or original_input.read_bytes() != joined_input.read_bytes()
        or _sha(joined_input.read_bytes()) != delivery.get("delivered_input_sha256")
    ):
        raise ValueError("recovery join did not retain the true original worker delivery bytes/digest")
    if review_counter.read_text() != "1":
        raise ValueError("mechanical join silently relaunched the reviewer")
    if joined_show["result"]["context"] != shown["result"]["context"]:
        raise ValueError("join fabricated review evidence or changed provider context")
    if _capture_manifest(capture) != original_manifest:
        raise ValueError("scripted output repair or join modified the original failed review capture")

    document = _candidate_doc(journey, root, joined_show)
    candidates = [row for row in document["candidates"] if row.get("status") == "ready"]
    records = [row for row in document["records"] if row.get("status") == "ready"]
    if (
        len(candidates) != 1
        or candidates[0].get("result") != "fail"
        or candidates[0].get("findings") != expected_finding
        or len(records) != 1
        or records[0].get("kind") != "review-evidence"
        or records[0].get("data", {}).get("result") != "fail"
        or records[0].get("origin", {}).get("id") != invoked["result"]["invocation_id"]
    ):
        raise ValueError(f"derived full-schema candidate/append source lost the explicit FAIL: {document}")
    subject = json.loads((artifacts / "intent.json").read_text(encoding="utf-8"))
    revision = subject["revision"]
    _append_record(journey, root, database, run_id, "repair-first-initial-ledger", "finding-ledger", {
        "schema_version": "1", "gate": "intent-review", "subject": "intent.json",
        "subject_revision": revision, "author": {"name": "fixture-driver", "kind": "agent"},
        "findings": [],
    })
    source_id = records[0]["record_id"]
    _append_record(journey, root, database, run_id, source_id, "review-evidence", records[0]["data"])
    denied = _event(journey, root, database, run_id, "approved", expect="rejected")
    if "failed" not in json.dumps(denied).lower() and "finding" not in json.dumps(denied).lower():
        raise ValueError(f"checked provider gate did not retain and block derived FAIL: {denied}")
    _append_record(journey, root, database, run_id, "repair-first-driver-disposition", "finding-ledger", {
        "schema_version": "1", "gate": "intent-review", "subject": "intent.json",
        "subject_revision": revision, "author": {"name": "fixture-driver", "kind": "agent"},
        "findings": [{
            "id": "F-repair-first-fixture", "source": {"kind": "context-record", "id": source_id},
            "policy_id": "solution-agnostic", "statement": expected_finding,
            "disposition": "rejected",
            "reason": "The driver inspected the isolated scripted fixture; its explicit failure remains unchanged.",
            "owner_phase": None, "task_ids": [], "review_axes": [], "status": "recorded",
        }],
    })
    approved = _event(journey, root, database, run_id, "approved")
    final_show, _ = _engine(journey, root, database, "show", "--view", "full", run_id)
    retained = next(record for record in final_show["result"]["context"] if record["id"] == source_id)
    if (
        approved.get("status") != "completed"
        or final_show["result"]["current_state"] != "design"
        or retained["data"].get("result") != "fail"
        or retained["data"].get("findings") != expected_finding
    ):
        raise ValueError("driver disposition did not preserve raw derived FAIL through the checked provider gate")
    return {
        "run_id": run_id,
        "failed_invocation": started["invocation_id"],
        "joined_invocation": joined["invocation_id"],
        "raw_result": "fail",
        "derived_result": derived_judgment["result"],
        "candidate_result": candidates[0]["result"],
        "gate_blocked_before_disposition": True,
        "gate_approved_after_disposition": True,
        "raw_sha256": _sha(raw),
        "derived_sha256": faithful["selected"]["sha256"],
        "reviewer_launches": review_counter.read_text(),
        "adapter_calls": {"faithful": good_calls.read_text(), "changed": changed_calls.read_text(), "missing": missing_calls.read_text(), "overbudget": over_calls.read_text()},
    }


def _generic_recovery_case(journey, root: Path, label: str, *, cancel: bool) -> dict[str, Any]:
    case_root = root / label
    case_root.mkdir()
    worker = case_root / "review-worker.py"
    counters = {role: case_root / f"{role}-count" for role in ("a", "b", "c")}
    markers = case_root / "b-started"
    worker.write_text(
        "import json,pathlib,sys,time\n"
        "role=sys.argv[1]; counter=pathlib.Path(sys.argv[2]); marker=pathlib.Path(sys.argv[3])\n"
        "raw=sys.stdin.buffer.read(); packet=json.loads(raw); count=int(counter.read_text() or '0') if counter.exists() else 0; counter.write_text(str(count+1))\n"
        "if role=='a': print(json.dumps({'result':'a-complete'}))\n"
        "elif role=='b' and sys.argv[4]=='failure' and count==0: print(json.dumps({'wrong':'first failure'}))\n"
        "elif role=='b' and sys.argv[4]=='cancel' and count==0: marker.write_text('started\\n'); time.sleep(60)\n"
        "else: print(json.dumps({'result':role+'-complete'}))\n",
        encoding="utf-8",
    )
    mode = "cancel" if cancel else "failure"
    workers = []
    for role, title in [("a", "First-group completed reviewer"), ("b", "First-group failed or cancelled reviewer"), ("c", "Second-group dependent reviewer")]:
        workers.append({
            "command": sys.executable,
            "args": [str(worker), role, str(counters[role]), str(markers), mode],
            "title": title,
            "role": "reviewer",
            "output_schema": {"required": ["result"]},
        })
    binding = _fanout_binding(journey.engine, workers, max_active=1, split=2)
    database, artifacts = dogfood_observation._provider_and_run(journey, case_root, ["review"], {"review": binding}, f"p05-{label}")
    run_id = f"p05-{label}"
    _show_action(journey, root, database, run_id)
    started, _ = _engine(journey, root, database, "--timeout-ms", "120000", "invoke", run_id, "review")
    invocation_id = started["result"]["invocation_id"]
    if cancel:
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline and not markers.exists():
            time.sleep(0.05)
        if not markers.exists():
            raise ValueError("second-group cancellation worker never started")
        # The same slot must refuse overlapping work while the original graph owns children.
        _show_action(journey, root, database, run_id)
        overlap, _ = _engine(journey, root, database, "invoke", run_id, "review", expect="rejected")
        if "already-running" not in overlap.get("code", ""):
            raise ValueError(f"live owned fan-out was not protected from overlap: {overlap}")
        _show_action(journey, root, database, run_id)
        _show_action(journey, root, database, run_id)
        _engine(journey, root, database, "cancel-invocation", run_id, invocation_id, timeout=20)
        shown, original = _wait_invocation(journey, root, database, run_id, invocation_id, timeout=25)
        if original.get("status") != "failed" or not original.get("ownership") or original["ownership"].get("cleanup_pending") is not False or original["ownership"].get("live_owned_work") is not False:
            raise ValueError(f"cancelled origin did not expose verified quiescence: {original}")
    else:
        shown, original = _wait_invocation(journey, root, database, run_id, invocation_id)
        if original.get("status") != "failed":
            raise ValueError(f"two-group contract failure did not fail its original invocation: {original}")
    origin_capture = Path(original["capture_dir"])
    origin_capture_snapshot = _capture_manifest(origin_capture)
    preview = _preview(journey, root, journey.engine, invocation_id, shown)
    rows = {row["assignment_id"]: row for row in preview["assignments"]}
    if rows["worker-0"]["classification"] != "conforming-completed":
        raise ValueError(f"genuine completed sibling was not retained: {rows['worker-0']}")
    if rows["worker-2"]["classification"] != "never-started":
        raise ValueError(f"downstream barrier member was not retained as never-started: {rows['worker-2']}")
    if preview["barriers"]["second_group"] != ["worker-2"]:
        raise ValueError(f"original barrier identities were not retained: {preview['barriers']}")
    recovery_input = copy.deepcopy(preview["recovery_input"])
    if recovery_input["pending_assignment_ids"] != ["worker-1", "worker-2"]:
        raise ValueError(f"preview selected wrong pending assignments: {recovery_input}")
    _show_action(journey, root, database, run_id)
    recovery, _ = _engine(
        journey, root, database, "--timeout-ms", "120000", "invoke", run_id, "review",
        "--input", json.dumps(recovery_input, separators=(",", ":")), timeout=30,
    )
    joined_show, joined = _wait_invocation(journey, root, database, run_id, recovery["result"]["invocation_id"])
    if joined.get("status") != "succeeded" or joined.get("exit_code") != 0:
        raise ValueError(f"pending-only two-group recovery did not succeed: {joined}")
    recovered = {row["assignment_id"]: row for row in joined["inner_workers"]}
    if recovered["worker-0"].get("started") is not None or recovered["worker-0"].get("recovery_source", {}).get("source_class") != "original-raw":
        raise ValueError(f"mechanical join invented a reviewer attempt for the successful sibling: {recovered['worker-0']}")
    if recovered["worker-1"].get("started") is not True or recovered["worker-2"].get("started") is not True:
        raise ValueError(f"recovery did not execute exactly pending assignments: {recovered}")
    if counters["a"].read_text() != "1" or counters["b"].read_text() != "2" or counters["c"].read_text() != "1":
        raise ValueError(f"reviewer launch counts show replay or skipped pending work: {counters}")
    summary = json.loads((Path(joined["capture_dir"]) / "summary.json").read_text(encoding="utf-8"))
    if summary.get("recovery", {}).get("pending_assignment_ids") != ["worker-1", "worker-2"]:
        raise ValueError("new capture omitted pending/source selection")
    if _capture_manifest(origin_capture) != origin_capture_snapshot:
        raise ValueError("pending recovery mutated the immutable failed/cancelled origin capture")
    old = next(row for row in joined_show["result"]["work_slot_invocations"] if row["invocation_id"] == invocation_id)
    if old.get("status") != "failed":
        raise ValueError("recovery join rewrote the original failed/cancelled invocation status")
    return {
        "run_id": run_id,
        "origin_invocation": invocation_id,
        "recovery_invocation": joined["invocation_id"],
        "origin_status": original["status"],
        "quiescent": True,
        "source_assignments": ["worker-0"],
        "pending_assignments": ["worker-1", "worker-2"],
        "review_launches": {name: path.read_text() for name, path in counters.items()},
        "barrier": summary["recovery"]["barriers"],
    }


def _scaled_completion_case(journey) -> dict[str, Any]:
    if not shutil.which("dagu"):
        raise ValueError("sol-persistence requires the operator-provided Dagu binary on PATH")
    parent = journey.work_root
    parent.mkdir(parents=True, exist_ok=True)
    root = parent / f"sol-persistence-scaled-{time.time_ns()}"
    root.mkdir()
    journey._dogfood_case_root = root
    artifacts = root / "artifacts"
    artifacts.mkdir()
    worker = root / "worker.py"
    worker.write_text(
        "import json,sys\n"
        "location=json.load(sys.stdin); context=location.get('context',[])\n"
        "if len(context)!=1 or len(context[0].get('data',{}).get('text',''))!=48000: raise SystemExit('large context was not forwarded exactly')\n"
        "print(json.dumps({'result':sys.argv[1]}),flush=True)\n",
        encoding="utf-8",
    )
    workers = [{
        "command": sys.executable,
        "args": [str(worker), f"worker-{index}"],
        "title": f"Scaled completion {index}",
        "role": "scripted worker",
        "output_schema": {"required": ["result"]},
    } for index in range(14)]
    binding = _fanout_binding(journey.engine, workers, max_active=4)
    provider = root / "fixture-provider.py"
    provider.write_text(
        "import json,sys\n"
        "request=json.load(sys.stdin)\n"
        "if request.get('operation')=='describe':\n"
        " initial=request.get('initial_input') or {};\n"
        " print(json.dumps({'id':'scaled-completion-fixture','initial_state':'work','states':[{'id':'work','title':'Work','instructions':'Complete captured work.','final':False},{'id':'done','title':'Done','instructions':'Done.','final':True}], 'transitions':[{'source':'work','event':'finish','target':'done','kind':'check-free'}], 'work_slots':[{'id':'fanout','state':'work','event':'finish','stdin_context_kinds':['large-evidence']}]}))\n"
        "else: print(json.dumps({'result':'allow'}))\n",
        encoding="utf-8",
    )
    config = root / "providers.toml"
    config.write_text(f"[providers.fixture]\ncommand = {json.dumps(sys.executable)}\nargs = [{json.dumps(str(provider))}]\n", encoding="utf-8")
    initial = {"artifact_root": str(artifacts), "work_slot_bindings": {"fanout": binding}}
    initial_path = root / "initial.json"
    _json(initial_path, initial)
    database = root / "loop.sqlite"
    run_id = "scaled-completion-run"
    _engine(journey, root, database, "--config", str(config), "start", "--id", run_id, "fixture", "@" + str(initial_path))
    large_context = {"text": "x" * 48_000}
    _show_action(journey, root, database, run_id)
    _engine(journey, root, database, "--record-id", "large-evidence", "append", run_id, "large-evidence", json.dumps(large_context))
    _show_action(journey, root, database, run_id)
    invoked, _ = _engine(journey, root, database, "--timeout-ms", "120000", "invoke", run_id, "fanout")
    invocation_id = invoked["result"]["invocation_id"]
    shown, row = _wait_invocation(journey, root, database, run_id, invocation_id, timeout=60)
    if row.get("status") != "succeeded" or len(row.get("inner_workers", [])) != 14:
        raise ValueError(f"public scaled 14-worker completion failed: {row}")
    uri = f"file:{database}?mode=ro"
    connection = sqlite3.connect(uri, uri=True, timeout=2)
    try:
        inner_length, snapshot_length = connection.execute(
            "SELECT length(inner_workers_json), length(completion_snapshot_json) FROM work_slot_invocations WHERE run_id=? AND invocation_id=?",
            (run_id, invocation_id),
        ).fetchone()
        history_count = connection.execute("SELECT count(*) FROM history_entries WHERE run_id=?", (run_id,)).fetchone()[0]
    finally:
        connection.close()
    if inner_length <= 75_000 or snapshot_length >= 128:
        raise ValueError(f"scaled public completion did not retain one large payload plus small digest reference: {inner_length}, {snapshot_length}")
    if history_count < 4:
        raise ValueError(f"completion compacting dropped semantic run history: {history_count}")
    # The real captured worker payload is larger than this scaled row limit;
    # duplicating it into the adjacent completion column would fail.
    probe = sqlite3.connect(":memory:")
    try:
        probe.setlimit(sqlite3.SQLITE_LIMIT_LENGTH, 75_000)
        probe.execute("CREATE TABLE payload_probe(value TEXT)")
        payload = (Path(row["capture_dir"]) / "summary.json").read_bytes()
        if len(payload) <= 75_000:
            raise ValueError("scaled completion fixture summary was not large enough")
        try:
            probe.execute("INSERT INTO payload_probe VALUES (?)", (payload.decode("utf-8"),))
        except sqlite3.DataError:
            pass
        else:
            raise ValueError("SQLite scaled length limit did not reject a duplicate-sized payload")
    finally:
        probe.close()
    outcome = {"status":"passed","run_id":run_id,"invocation_id":invocation_id,"workers":len(row["inner_workers"]),
               "inner_workers_json_bytes":inner_length,"completion_snapshot_json_bytes":snapshot_length,
               "history_entries":history_count,"scaled_length_limit":75_000}
    _json(root / "outcome.json", outcome)
    return outcome


def persistence_case(journey) -> None:
    dogfood_observation.persistence_case(journey)
    scaled = _scaled_completion_case(journey)
    print(f"sol-persistence passed: actual 14-worker completion and scaled double-payload refusal ({scaled['inner_workers_json_bytes']}/{scaled['completion_snapshot_json_bytes']} bytes)")


def recovery_case(journey) -> None:
    root = _fresh_root(journey)
    if not shutil.which("dagu"):
        raise ValueError("sol-recovery-core requires the operator-provided Dagu binary on PATH")
    mechanical = _software_change_mechanical_case(journey, root)
    full_schema_mechanical = _software_change_full_schema_mechanical_case(journey, root)
    prose = _software_change_scripted_review_case(journey, root)
    failed = _generic_recovery_case(journey, root, "failed-two-group", cancel=False)
    cancelled = _generic_recovery_case(journey, root, "cancelled-two-group", cancel=True)
    full_show_retention = _assert_full_show_retention(root)
    report = {
        "schema_version": 1,
        "status": "passed",
        "mechanical_implementation": mechanical,
        "full_schema_mechanical": full_schema_mechanical,
        "scripted_explicit_review_fail": prose,
        "failed_two_group": failed,
        "cancelled_two_group": cancelled,
        "full_show_retention": full_show_retention,
        "semantic_review": "not performed; scripted output proves mechanics only",
        "live_repair_model": "disabled",
    }
    _json(root / "sol-recovery-core-proof.json", report)
    print("sol-recovery-core passed: legacy implementation and full-schema mechanical repairs, separate scripted derived FAIL join, and failed/cancelled recovery/refusal cases")
