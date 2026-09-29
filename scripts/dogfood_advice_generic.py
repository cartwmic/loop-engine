"""Public generic-advice CLI journey with a scripted non-software provider."""
from __future__ import annotations

import hashlib
import json
import os
import subprocess
import sys
import time
from pathlib import Path
from typing import Any


def _fresh_root(journey) -> Path:
    journey.preflight()
    parent = journey.work_root
    parent.parent.mkdir(parents=True, exist_ok=True)
    if parent.exists() and not parent.is_dir():
        raise ValueError(f"advice scenario work-root is not a directory: {parent}")
    if not parent.exists():
        parent.mkdir()
        root = parent
    else:
        root = parent / f"sol-advice-generic-{time.time_ns()}"
        root.mkdir()
    journey._dogfood_case_root = root
    return root


def _capture(root: Path, argv: list[str], *, expect: str = "completed", timeout: float = 30):
    logs = root / "commands"
    logs.mkdir(exist_ok=True)
    ordinal = len(list(logs.glob("*.argv.json")))
    label = f"{ordinal:03d}"
    started = time.perf_counter_ns()
    try:
        process = subprocess.run(argv, cwd=root, capture_output=True, timeout=timeout, check=False)
    except (OSError, subprocess.TimeoutExpired) as error:
        raise ValueError(f"public advice command did not complete: {argv!r}: {error}") from error
    elapsed_ms = (time.perf_counter_ns() - started) / 1_000_000
    (logs / f"{label}.argv.json").write_text(json.dumps({"argv": argv, "cwd": str(root)}) + "\n")
    (logs / f"{label}.stdout").write_bytes(process.stdout)
    (logs / f"{label}.stderr").write_bytes(process.stderr)
    (logs / f"{label}.exit.json").write_text(json.dumps({"returncode": process.returncode, "elapsed_ms": elapsed_ms}) + "\n")
    try:
        envelope = json.loads(process.stdout)
    except Exception as error:
        raise ValueError(f"public CLI returned non-JSON: {argv!r}: {process.stderr[-1000:]!r}") from error
    if expect == "completed" and (process.returncode != 0 or envelope.get("status") != "completed"):
        raise ValueError(f"public CLI did not complete: {envelope!r} {process.stderr[-1000:]!r}")
    if expect == "error" and (process.returncode == 0 or envelope.get("status") not in ("error", "rejected")):
        raise ValueError(f"invalid advice response was accepted: {envelope!r}")
    return envelope, elapsed_ms, process


def _engine(journey, root: Path, database: Path, *args: str, expect: str = "completed", timeout: float = 30):
    argv = [str(journey.engine), "--database", str(database), "--json", *args]
    return _capture(root, argv, expect=expect, timeout=timeout)


def _request(occasion: str) -> dict[str, Any]:
    return {
        "version": 1,
        "admissibility": {"bounded_judgment": True, "evidence_sufficient": True},
        "state": {"evidence": "The selected observation says: the task finished and its output was inspected."},
        "target": {"revision": "fixture-r1", "evidence_ids": ["evidence-1"]},
        "occasion": occasion,
        "questions": {
            "route": {
                "type": "choice",
                "instructions": "Classify this one supplied sentence as an observation or an intention. The option keys are fixture labels, not actions.",
                "criteria": {"continue": "The sentence describes completed work.", "pause": "The sentence describes planned work."},
            },
            "quality": {
                "type": "score",
                "instructions": "Rate how directly the supplied sentence describes an actual observation, without inferring task quality.",
                "levels": [
                    {"description": "weak", "criteria": "The evidence is incomplete."},
                    {"description": "strong", "criteria": "The evidence is directly supported."},
                ],
            },
            "premise": {
                "type": "noul",
                "instructions": "Is the stated premise supported?",
                "proposition": "The supplied sentence describes completed work rather than planned work.",
            },
        },
    }


def _expected_response(request: dict[str, Any]) -> dict[str, Any]:
    return {
        "answers": {
            "route": {
                "type": "choice",
                "choice": "continue",
                "probabilities": {"continue": 0.75, "pause": 0.25},
                "confidence": 0.8,
            },
            "quality": {
                "type": "score",
                "score": 0.25,
                "legend": {"0": "weak", "1": "strong"},
                "probabilities": {"0": 0.75, "1": 0.25},
                "confidence": 0.7,
                "rationale": "The evidence names the supplied source.",
            },
            "premise": {"type": "noul", "noul": 0.6},
        }
    }


def _write_fixtures(root: Path):
    provider = root / "generic-provider.py"
    provider.write_text(
        "import json,sys\n"
        "request=json.load(sys.stdin)\n"
        "if request.get('operation') == 'describe':\n"
        " print(json.dumps({'id':'generic-advice-fixture-v1','initial_state':'work','states':[{'id':'work','title':'Primary work','instructions':'Run the primary task.','final':False},{'id':'done','title':'Done','instructions':'Primary task completed.','final':True}],'transitions':[{'source':'work','event':'finish','target':'done','kind':'checked'}],'work_slots':[{'id':'primary','state':'work','event':'finish'}]}))\n"
        "else:\n"
        " print(json.dumps({'result':'allow'}))\n",
        encoding="utf-8",
    )
    advice = root / "scripted-advice.py"
    advice.write_text(
        "import json,sys,time,pathlib\n"
        "default_mode=sys.argv[1]; counter=pathlib.Path(sys.argv[2]); counter.write_text(str(int(counter.read_text() or '0')+1) if counter.exists() else '1')\n"
        "request=json.load(sys.stdin); mode=request.get('occasion',default_mode)\n"
        "answers={'route':{'type':'choice','choice':'continue','probabilities':{'continue':0.75,'pause':0.25},'confidence':0.8},'quality':{'type':'score','score':0.25,'legend':{'0':'weak','1':'strong'},'probabilities':{'0':0.75,'1':0.25},'confidence':0.7,'rationale':'The evidence names the supplied source.'},'premise':{'type':'noul','noul':0.6}}\n"
        "if mode=='wrong-keys': answers={'wrong':{'type':'noul','noul':0.5}}\n"
        "elif mode=='wrong-type': answers['route']={'type':'noul','noul':0.5}\n"
        "elif mode=='wrong-choice-key': answers['route']['choice']='unrequested'\n"
        "elif mode=='fake-probability': answers['route']['probabilities']={'continue':0.7,'pause':0.1}\n"
        "elif mode=='extra-answer': answers['unexpected']={'type':'noul','noul':0.5}\n"
        "elif mode=='malformed': sys.stdout.write('{not-json'); raise SystemExit(0)\n"
        "elif mode=='timeout': time.sleep(30)\n"
        "elif mode=='nonzero': print(json.dumps({'answers':answers})); raise SystemExit(7)\n"
        "response={'answers':answers}; encoded=json.dumps(response,separators=(',',':'))\n"
        "if mode=='large-response': encoded += ' '*5000\n"
        "print(encoded)\n",
        encoding="utf-8",
    )
    worker = root / "primary-worker.py"
    worker.write_text(
        "import pathlib,sys\n"
        "pathlib.Path(sys.argv[1]).write_text('primary-work-ran-once')\n"
        "print('primary work complete')\n",
        encoding="utf-8",
    )
    counter = root / "advice-invocation-count.txt"
    providers = root / "providers.toml"
    providers.write_text(
        "[providers.fixture]\n"
        f"command = {json.dumps(sys.executable)}\n"
        f"args = [{json.dumps(str(provider))}]\n",
        encoding="utf-8",
    )
    return advice, worker, counter, providers


def advice_case(journey) -> None:
    root = _fresh_root(journey)
    advice_script, worker, counter, providers = _write_fixtures(root)
    database = root / "generic-run.sqlite"
    artifact_root = root / "run-artifacts"
    artifact_root.mkdir()
    primary_marker = root / "primary-work.txt"
    advice_config = {
        "command": sys.executable,
        "args": [str(advice_script), "valid", str(counter)],
        "timeout_ms": 2000,
        "max_request_bytes": 4096,
        "max_response_bytes": 4096,
    }
    initial = {
        "artifact_root": str(artifact_root),
        "advice_command": advice_config,
        "work_slot_bindings": {
            "primary": {"command": sys.executable, "args": [str(worker), str(primary_marker)]}
        },
    }
    _capture(root, [str(journey.engine), "--database", str(database), "--config", str(providers), "--json",
                    "start", "--id", "generic-advice-run", "fixture", json.dumps(initial)])

    valid_request = _request("valid")
    valid_path = root / "request-valid.json"
    valid_bytes = json.dumps(valid_request, separators=(",", ":")).encode()
    valid_path.write_bytes(valid_bytes)
    completed, _, _ = _engine(journey, root, database, "advise", "generic-advice-run", f"@{valid_path}")
    result = completed["result"]
    response = result["response"]
    if response != _expected_response(valid_request):
        raise ValueError(f"generic typed response did not preserve exact Choice/Score/Noul meanings: {response!r}")
    if "confidence" in response["answers"]["premise"]:
        raise ValueError("Noul incorrectly required/emitted separate confidence")
    if "rationale" in response["answers"]["route"] or "rationale" in response["answers"]["premise"]:
        raise ValueError("absent optional per-answer rationale was invented")
    if response["answers"]["quality"].get("rationale") != "The evidence names the supplied source.":
        raise ValueError("present optional per-answer rationale was lost")
    origin = result["origin"]
    capture_dir = Path(origin["capture_dir"])
    if origin["kind"] != "advice-attempt" or origin["run_id"] != "generic-advice-run" or origin["context_record_id"] != result["attempt_id"]:
        raise ValueError(f"typed result origin is incomplete or not run-owned: {origin!r}")
    if (capture_dir / "request.json").read_bytes() != valid_bytes:
        raise ValueError("run-owned advice input capture differs from the exact attempted bytes")
    attempt = capture_dir / "attempts" / "1"
    receipt = json.loads((attempt / "receipt.json").read_bytes())
    if (attempt / "stdin").read_bytes() != valid_bytes:
        raise ValueError("configured command did not receive the exact captured request bytes")
    if receipt["argv"] != [sys.executable, *advice_config["args"]] or receipt["exit_code"] != 0 or receipt["timed_out"]:
        raise ValueError(f"captured command/exit/timeout do not match the configured call: {receipt!r}")
    captured_response = (attempt / "stdout").read_bytes()
    if json.loads(captured_response) != response:
        raise ValueError("typed result differs from exact captured stdout")
    if not (capture_dir / "typed-result.json").is_file():
        raise ValueError("typed result capture was not retained beside the command capture")

    full, _, _ = _engine(journey, root, database, "show", "--view", "full", "generic-advice-run")
    full_result = full["result"]
    advice_records = [record for record in full_result["context"] if record["kind"] == "advice-attempt"]
    if len(advice_records) != 1 or advice_records[0]["id"] != result["attempt_id"]:
        raise ValueError("successful advice is not separately referenced in immutable run context")
    if full_result["work_slot_invocations"] or full_result["current_state"] != "work":
        raise ValueError("advice masqueraded as primary work or advanced the generic provider run")
    if primary_marker.exists():
        raise ValueError("primary work ran during the independent advice call")

    negative_modes = [
        ("wrong-keys", "invalid-advice-response"),
        ("wrong-type", "invalid-advice-response"),
        ("wrong-choice-key", "invalid-advice-response"),
        ("fake-probability", "invalid-advice-response"),
        ("extra-answer", "invalid-advice-response"),
        ("malformed", "invalid-advice-response"),
        ("timeout", "advice-command-failed"),
        ("nonzero", "advice-command-failed"),
        ("large-response", "advice-command-failed"),
    ]
    retained_attempts = [result["attempt_id"]]
    # Public admission refuses before spawning any configured backend. The
    # declaration is the driver's assessment, not a semantic engine classifier.
    for field in ("missing", "bounded_judgment", "evidence_sufficient"):
        inadmissible = _request("inadmissible-" + field)
        if field == "missing":
            del inadmissible["admissibility"]
        else:
            inadmissible["admissibility"][field] = False
        path = root / f"request-inadmissible-{field}.json"
        path.write_text(json.dumps(inadmissible), encoding="utf-8")
        before_count = counter.read_text()
        refused, _, _ = _engine(journey, root, database, "advise", "generic-advice-run", f"@{path}", expect="error")
        if refused.get("code") != "invalid-advice-request" or counter.read_text() != before_count:
            raise ValueError(f"inadmissible {field} request reached the advisor or had the wrong refusal: {refused}")
        retained_attempts.append(refused["details"]["attempt_id"])
    for mode, expected_code in negative_modes:
        request_path = root / f"request-{mode}.json"
        request_bytes = json.dumps(_request(mode), separators=(",", ":")).encode()
        request_path.write_bytes(request_bytes)
        failed, _, _ = _engine(
            journey,
            root,
            database,
            "advise",
            "generic-advice-run",
            f"@{request_path}",
            expect="error",
            timeout=15,
        )
        if failed.get("code") != expected_code:
            raise ValueError(f"{mode} returned the wrong refusal: {failed!r}")
        attempt_id = failed.get("details", {}).get("attempt_id")
        if not attempt_id:
            raise ValueError(f"{mode} refusal omitted its immutable capture origin: {failed!r}")
        retained_attempts.append(attempt_id)
        failed_root = Path(failed["details"]["capture_dir"])
        if not (failed_root / "request.json").is_file() or not (failed_root / "typed-result.json").is_file():
            raise ValueError(f"{mode} failed attempt is not fully inspectable under run artifacts")
        if failed["details"].get("timed_out") is not (mode == "timeout"):
            raise ValueError(f"{mode} timeout fact was not retained: {failed!r}")
        current, _, _ = _engine(journey, root, database, "show", "--view", "full", "generic-advice-run")
        if current["result"]["work_slot_invocations"] or current["result"]["current_state"] != "work":
            raise ValueError(f"failed/invalid {mode} advice changed primary work or run state")

    # Requests over the positive input bound are retained but never sent to the
    # configured process. This is not a run-lifetime call quota.
    oversized = root / "request-over-limit.json"
    oversized.write_bytes(b"{" + b" " * 4096 + b"}")
    before_count = counter.read_text()
    refused, _, _ = _engine(
        journey,
        root,
        database,
        "advise",
        "generic-advice-run",
        f"@{oversized}",
        expect="error",
    )
    if refused.get("code") != "advice-request-too-large":
        raise ValueError(f"positive request byte bound did not fail closed: {refused!r}")
    if counter.read_text() != before_count:
        raise ValueError("oversized request reached the external advice command")
    over_id = refused["details"]["attempt_id"]
    retained_attempts.append(over_id)

    final_show, _, _ = _engine(journey, root, database, "show", "--view", "full", "generic-advice-run")
    stored = [record for record in final_show["result"]["context"] if record["kind"] == "advice-attempt"]
    if {record["id"] for record in stored} != set(retained_attempts):
        raise ValueError("successful and failed advice captures were not all retained by stable context origin")
    if any(record["data"].get("typed_result") is not None for record in stored if record["data"]["status"] == "failed"):
        raise ValueError("failed/invalid attempt was admitted as a typed advice answer")

    # Only the separately bound primary command plus the checked provider event
    # completes the generic non-software workflow.
    _engine(journey, root, database, "show", "--view", "action", "generic-advice-run")
    invoked, _, _ = _engine(journey, root, database, "--timeout-ms", "120000", "invoke", "generic-advice-run", "primary")
    invocation_id = invoked["result"].get("invocation_id")
    if invocation_id is None:
        raise ValueError("separate primary worker invocation was not recorded")
    deadline = time.monotonic() + 10
    primary_invocation = None
    while time.monotonic() < deadline:
        observed, _, _ = _engine(journey, root, database, "show", "--view", "full", "generic-advice-run")
        primary_invocation = next(
            (row for row in observed["result"]["work_slot_invocations"] if row["invocation_id"] == invocation_id),
            None,
        )
        if primary_invocation and primary_invocation.get("status") in ("succeeded", "failed"):
            break
        time.sleep(0.05)
    if not primary_invocation or primary_invocation.get("status") != "succeeded" or primary_invocation.get("exit_code") != 0:
        raise ValueError(f"separate primary work did not complete successfully: {primary_invocation!r}")
    if not primary_marker.is_file() or primary_marker.read_text() != "primary-work-ran-once":
        raise ValueError("primary work did not execute exactly once after advice inspection")
    _engine(journey, root, database, "show", "--view", "action", "generic-advice-run")
    finished, _, _ = _engine(journey, root, database, "event", "generic-advice-run", "finish")
    if finished["status"] != "completed" or finished["result"]["run"]["current_state"] != "done":
        raise ValueError(f"primary checked outcome did not reach its completed state: {finished!r}")
    if counter.read_text() != str(len(negative_modes) + 1):
        raise ValueError("advice invocation count does not match actual configured calls")

    # Missing frozen configuration is disabled, not a default backend lookup.
    disabled_database = root / "generic-disabled.sqlite"
    disabled_artifacts = root / "disabled-artifacts"
    disabled_artifacts.mkdir()
    disabled_input = {
        "artifact_root": str(disabled_artifacts),
        "work_slot_bindings": initial["work_slot_bindings"],
    }
    _capture(root, [str(journey.engine), "--database", str(disabled_database), "--config", str(providers), "--json",
                    "start", "--id", "generic-advice-disabled", "fixture", json.dumps(disabled_input)])
    before_disabled = counter.read_text()
    disabled, _, _ = _engine(
        journey,
        root,
        disabled_database,
        "advise",
        "generic-advice-disabled",
        f"@{valid_path}",
        expect="error",
    )
    if disabled.get("code") != "advice-disabled" or counter.read_text() != before_disabled:
        raise ValueError(f"missing frozen advice command selected or ran a default backend: {disabled!r}")
    disabled_show, _, _ = _engine(
        journey, root, disabled_database, "show", "--view", "full", "generic-advice-disabled"
    )
    if disabled_show["result"]["context"] or disabled_show["result"]["work_slot_invocations"]:
        raise ValueError("disabled advice created an attempt or primary worker record")

    (root / "sol-advice-generic-proof.json").write_text(json.dumps({
        "case": "sol-advice-generic",
        "status": "passed-scripted-assertions",
        "successful_and_failed_attempt_ids": retained_attempts,
        "primary_invocation_id": invocation_id,
        "completed_state": finished["result"]["run"]["current_state"],
        "disabled_mode_did_not_call_advisor": True,
        "semantic_advice_quality": "not-judged-by-scripted-fixture",
    }, indent=2) + "\n", encoding="utf-8")
    print("generic advice typed capture, refusal, and separate primary completion assertions passed")
