"""Public checked/check-free advice-closure and exception journey."""
from __future__ import annotations

import json
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
        raise ValueError(f"advice transition work-root is not a directory: {parent}")
    if not parent.exists():
        parent.mkdir()
        root = parent
    else:
        root = parent / f"sol-advice-transition-{time.time_ns()}"
        root.mkdir()
    journey._dogfood_advice_transition_root = root
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
        raise ValueError(f"public advice-transition command did not complete: {argv!r}: {error}") from error
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
        raise ValueError(f"public CLI unexpectedly accepted the operation: {envelope!r}")
    return envelope, process


def _engine(journey, root: Path, database: Path, *args: str, expect: str = "completed", timeout: float = 30):
    return _capture(root, [str(journey.engine), "--database", str(database), "--json", *args], expect=expect, timeout=timeout)


def _show(journey, root: Path, database: Path, run_id: str, view: str = "action"):
    return _engine(journey, root, database, "show", "--view", view, run_id)


def _append(journey, root: Path, database: Path, run_id: str, record_id: str, kind: str, data: dict[str, Any]):
    path = root / "records" / f"{record_id}.json"
    path.parent.mkdir(exist_ok=True)
    path.write_text(json.dumps(data, separators=(",", ":")) + "\n")
    _show(journey, root, database, run_id)
    return _engine(journey, root, database, "--record-id", record_id, "append", run_id, kind, f"@{path}")[0]


def _write_fixtures(root: Path):
    provider_counter = root / "provider-count.txt"
    provider_mode = root / "provider-mode.txt"
    provider_mode.write_text("allow")
    provider = root / "fixture-provider.py"
    provider.write_text(
        "import json,pathlib,sys\n"
        "request=json.load(sys.stdin)\n"
        "if request.get('operation')=='describe':\n"
        " initial=request.get('initial_input') or {}\n"
        " slots=[{'id':'slow','state':'work','event':'approve'}] if initial.get('cancel_fixture') else []\n"
        " print(json.dumps({'id':'advice-transition-fixture-v1','initial_state':'work','states':[{'id':'work','title':'Work','instructions':'Inspect the current work.','final':False},{'id':'done','title':'Done','instructions':'Work complete.','final':True}],'transitions':[{'source':'work','event':'approve','target':'done','kind':'checked'},{'source':'work','event':'revise','target':'work','kind':'check-free'}],'work_slots':slots}))\n"
        "else:\n"
        f" count=pathlib.Path({str(provider_counter)!r}); count.write_text(str(int(count.read_text())+1) if count.exists() else '1')\n"
        f" mode=pathlib.Path({str(provider_mode)!r}).read_text().strip()\n"
        " print(json.dumps({'result':'deny','feedback':{'code':'scripted-denial','message':'independent provider gate denied'}}) if mode=='deny' else json.dumps({'result':'allow'}))\n",
        encoding="utf-8",
    )
    advice = root / "fixture-advisor.py"
    advice.write_text(
        "import json,sys,time\n"
        "request=json.load(sys.stdin); mode=request.get('state',{}).get('mode','valid')\n"
        "if mode=='timeout': time.sleep(3)\n"
        "if mode=='invalid': print(json.dumps({'answers':{'wrong':{'type':'noul','noul':0.5}}})); raise SystemExit(0)\n"
        "answers={qid:{'type':'noul','noul':(0.1 if mode=='bad' else 0.8)} for qid in request['questions']}\n"
        "print(json.dumps({'answers':answers}))\n",
        encoding="utf-8",
    )
    worker = root / "slow-worker.py"
    worker.write_text("import time\ntime.sleep(60)\n", encoding="utf-8")
    providers = root / "providers.toml"
    providers.write_text(
        "[providers.fixture]\n"
        f"command = {json.dumps(sys.executable)}\n"
        f"args = [{json.dumps(str(provider))}]\n",
        encoding="utf-8",
    )
    advice_config = {
        "command": sys.executable,
        "args": [str(advice)],
        "timeout_ms": 500,
        "max_request_bytes": 8192,
        "max_response_bytes": 8192,
    }
    return providers, advice_config, worker, provider_counter, provider_mode


def _start(journey, root, providers, advice_config, run_id, database, occasions, *, cancel_fixture=False):
    artifacts = root / "artifacts" / run_id
    artifacts.mkdir(parents=True, exist_ok=True)
    initial: dict[str, Any] = {
        "artifact_root": str(artifacts),
        "advice_command": advice_config,
        "advice_departures": {"version": 1, "occasions": occasions},
    }
    if cancel_fixture:
        initial["cancel_fixture"] = True
        initial["work_slot_bindings"] = {
            "slow": {"command": sys.executable, "args": [str(root / "slow-worker.py")]}
        }
    _capture(root, [str(journey.engine), "--database", str(database), "--config", str(providers), "--json",
                    "start", "--id", run_id, "fixture", json.dumps(initial)])
    _show(journey, root, database, run_id)


def _map(event: str, occasion_id: str):
    return [{"state": "work", "event": event, "occasion_id": occasion_id}]


def _request(occasion: str, target: dict[str, Any], *, mode="valid"):
    return {
        "version": 1,
        "state": {"mode": mode,
                  "admissibility": {"bounded_judgment": True, "evidence_sufficient": True},
                  "evidence": ["Observed: the label was clipped after resizing. Expected: the label remains readable. Proposed check: resize and inspect the label."]},
        "target": target,
        "occasion": occasion,
        "questions": {
            "decision": {"type": "noul", "instructions": "Classify only the supplied observation.", "proposition": "The observation describes an actual failure rather than a proposed improvement."},
            "coverage": {"type": "noul", "instructions": "Compare only the supplied observation and proposed check.", "proposition": "They describe the same observable behavior."},
            "risk": {"type": "noul", "instructions": "Judge only the supplied observation and explicit expectation.", "proposition": "The observation contradicts that expectation."},
        },
    }


def _advise(journey, root, database, run_id, request, *, expect="completed"):
    path = root / "requests" / f"{run_id}-{time.time_ns()}.json"
    path.parent.mkdir(exist_ok=True)
    path.write_text(json.dumps(request, separators=(",", ":")))
    result, _ = _engine(journey, root, database, "advise", run_id, f"@{path}", expect=expect)
    return result


def _occasion_record(state_visit, event, occasion_id, target, source_id, response_ids, *, triggered=True, reason=None):
    record: dict[str, Any] = {
        "state_visit": state_visit,
        "source_state": "work",
        "event": event,
        "occasion_id": occasion_id,
        "target": target,
        "triggered": triggered,
    }
    if triggered:
        record["trigger_source_ids"] = [source_id]
        record["response_ids"] = response_ids
    else:
        record["reason"] = reason
    return record


def _dispositions(journey, root, database, run_id, response_id, occasion_id, target, *, statuses=None, applicability_id=None):
    statuses = statuses or {}
    for answer_id in ("decision", "coverage", "risk"):
        data: dict[str, Any] = {
            "response_id": response_id,
            "answer_id": answer_id,
            "occasion_id": occasion_id,
            "state_visit": 0,
            "target": target,
            "disposition": statuses.get(answer_id, "accept"),
            "reason": statuses.get(f"{answer_id}_reason", f"Driver disposition for {answer_id} at this target."),
        }
        if applicability_id:
            data["applicability_id"] = applicability_id
        _append(journey, root, database, run_id, f"disp-{response_id}-{answer_id}-{len(list((root / 'records').glob('disp-*.json')))}", "advice-disposition", data)


def _source(journey, root, database, run_id, name):
    record_id = f"source-{name}"
    _append(journey, root, database, run_id, record_id, "driver-note", {"observation": name})
    return record_id


def advice_transition_case(journey) -> None:
    root = _fresh_root(journey)
    providers, advice_config, worker, provider_counter, provider_mode = _write_fixtures(root)
    target = {"revision": "r1", "checkpoint": "cp-a"}

    # A semantically bad answer is still closed by explicit reasoned rejection;
    # all three disposition kinds remain ordinary driver decisions.
    checkfree_db = root / "checkfree.sqlite"
    _start(journey, root, providers, advice_config, "advice-checkfree", checkfree_db, _map("revise", "revise-review"))
    source_id = _source(journey, root, checkfree_db, "advice-checkfree", "bad-answer")
    advised = _advise(journey, root, checkfree_db, "advice-checkfree", _request("revise-review", target, mode="bad"))
    response_id = advised["result"]["attempt_id"]
    _append(journey, root, checkfree_db, "advice-checkfree", "occ-checkfree", "advice-occasion",
            _occasion_record(0, "revise", "revise-review", target, source_id, [response_id]))
    _dispositions(journey, root, checkfree_db, "advice-checkfree", response_id, "revise-review", target,
                  statuses={"decision": "reject", "decision_reason": "The answer contradicts the captured evidence.",
                            "coverage": "partial", "coverage_reason": "Only the named subset is covered.",
                            "risk": "accept", "risk_reason": "The narrow risk is recorded."})
    provider_calls_before = int(provider_counter.read_text()) if provider_counter.exists() else 0
    _show(journey, root, checkfree_db, "advice-checkfree")
    completed, _ = _engine(journey, root, checkfree_db, "event", "advice-checkfree", "revise")
    if completed["result"]["run"]["current_state"] != "work":
        raise ValueError("check-free advice departure did not commit to its stored target")
    provider_calls_after = int(provider_counter.read_text()) if provider_counter.exists() else 0
    if provider_calls_after != provider_calls_before:
        raise ValueError("check-free transition incorrectly invoked provider evaluation")

    # A checked transition still executes the independent provider gate after
    # complete, target-matched advice closure.
    checked_db = root / "checked.sqlite"
    _start(journey, root, providers, advice_config, "advice-checked", checked_db, _map("approve", "checked-review"))
    checked_source = _source(journey, root, checked_db, "advice-checked", "checked")
    checked_advice = _advise(journey, root, checked_db, "advice-checked", _request("checked-review", target))
    checked_response = checked_advice["result"]["attempt_id"]
    _append(journey, root, checked_db, "advice-checked", "occ-checked", "advice-occasion",
            _occasion_record(0, "approve", "checked-review", target, checked_source, [checked_response]))
    _dispositions(journey, root, checked_db, "advice-checked", checked_response, "checked-review", target,
                  statuses={"decision": "accept", "coverage": "partial", "risk": "reject"})
    checked_before = int(provider_counter.read_text()) if provider_counter.exists() else 0
    _show(journey, root, checked_db, "advice-checked")
    checked_event, _ = _engine(journey, root, checked_db, "--config", str(providers), "event", "advice-checked", "approve")
    if checked_event["result"]["run"]["current_state"] != "done" or int(provider_counter.read_text()) != checked_before + 1:
        raise ValueError("checked departure bypassed the independent provider evaluation")

    # Ad hoc successful answers are still dispositioned, even when their
    # result is not one of the frozen map's due occasions.
    adhoc_db = root / "ad-hoc.sqlite"
    _start(journey, root, providers, advice_config, "advice-adhoc", adhoc_db, _map("revise", "mapped-review"))
    adhoc_source = _source(journey, root, adhoc_db, "advice-adhoc", "adhoc")
    adhoc = _advise(journey, root, adhoc_db, "advice-adhoc", _request("unsolicited-question", target))
    adhoc_id = adhoc["result"]["attempt_id"]
    _append(journey, root, adhoc_db, "advice-adhoc", "occ-adhoc", "advice-occasion",
            _occasion_record(0, "revise", "mapped-review", target, adhoc_source, [], triggered=False,
                             reason="No admissible bounded question: resolving this issue requires investigation by the driver, not an advisor."))
    _show(journey, root, adhoc_db, "advice-adhoc")
    missing_adhoc = _engine(journey, root, adhoc_db, "event", "advice-adhoc", "revise", expect="error")[0]
    if "advice answer" not in missing_adhoc.get("message", ""):
        raise ValueError(f"undispositioned ad hoc answer did not block departure: {missing_adhoc!r}")
    _dispositions(journey, root, adhoc_db, "advice-adhoc", adhoc_id, "unsolicited-question", target,
                  statuses={"decision": "reject", "decision_reason": "Not relevant to this task.",
                            "coverage": "partial", "risk": "accept"})
    _show(journey, root, adhoc_db, "advice-adhoc")
    _engine(journey, root, adhoc_db, "event", "advice-adhoc", "revise")

    # Superseded successful results remain individually dispositioned.
    superseded_db = root / "superseded.sqlite"
    _start(journey, root, providers, advice_config, "advice-superseded", superseded_db, _map("revise", "superseded-review"))
    superseded_source = _source(journey, root, superseded_db, "advice-superseded", "superseded")
    old = _advise(journey, root, superseded_db, "advice-superseded", _request("superseded-review", target))
    new = _advise(journey, root, superseded_db, "advice-superseded", _request("superseded-review", target))
    old_id, new_id = old["result"]["attempt_id"], new["result"]["attempt_id"]
    _append(journey, root, superseded_db, "advice-superseded", "occ-superseded", "advice-occasion",
            _occasion_record(0, "revise", "superseded-review", target, superseded_source, [old_id, new_id]))
    _dispositions(journey, root, superseded_db, "advice-superseded", old_id, "superseded-review", target,
                  statuses={"decision": "reject", "decision_reason": f"Superseded by response {new_id}.",
                            "coverage": "reject", "coverage_reason": f"Superseded by response {new_id}.",
                            "risk": "reject", "risk_reason": f"Superseded by response {new_id}."})
    _dispositions(journey, root, superseded_db, "advice-superseded", new_id, "superseded-review", target,
                  statuses={"decision": "accept", "coverage": "partial", "risk": "reject"})
    _show(journey, root, superseded_db, "advice-superseded")
    _engine(journey, root, superseded_db, "event", "advice-superseded", "revise")

    # A response for a changed target is stale until the driver records exact
    # applicability and target-specific answer dispositions.
    stale_db = root / "stale-target.sqlite"
    _start(journey, root, providers, advice_config, "advice-stale", stale_db, _map("revise", "stale-review"))
    stale_source = _source(journey, root, stale_db, "advice-stale", "stale")
    old_target = {"revision": "r1", "checkpoint": "cp-old"}
    new_target = {"revision": "r2", "checkpoint": "cp-new"}
    stale = _advise(journey, root, stale_db, "advice-stale", _request("stale-review", old_target))
    stale_id = stale["result"]["attempt_id"]
    _append(journey, root, stale_db, "advice-stale", "occ-stale", "advice-occasion",
            _occasion_record(0, "revise", "stale-review", new_target, stale_source, [stale_id]))
    _dispositions(journey, root, stale_db, "advice-stale", stale_id, "stale-review", old_target)
    _show(journey, root, stale_db, "advice-stale")
    stale_refused = _engine(journey, root, stale_db, "event", "advice-stale", "revise", expect="error")[0]
    if "unanswered due advice occasions" not in stale_refused.get("message", ""):
        raise ValueError(f"stale-target advice did not block normally: {stale_refused!r}")
    applicability_id = "app-stale-target"
    _append(journey, root, stale_db, "advice-stale", applicability_id, "advice-applicability",
            {"response_id": stale_id, "occasion_id": "stale-review", "state_visit": 0,
             "target": new_target, "attesting_driver": "fixture-driver",
             "reason": "The accepted source and requested change are unchanged despite the checkpoint update."})
    _dispositions(journey, root, stale_db, "advice-stale", stale_id, "stale-review", new_target,
                  applicability_id=applicability_id)
    _show(journey, root, stale_db, "advice-stale")
    _engine(journey, root, stale_db, "event", "advice-stale", "revise")

    # Missing advice is blocked; the exact exception is visible, and it cannot
    # excuse an undispositioned successful ad hoc answer.
    exception_db = root / "exception.sqlite"
    _start(journey, root, providers, advice_config, "advice-exception", exception_db, _map("revise", "exception-review"))
    exception_source = _source(journey, root, exception_db, "advice-exception", "exception")
    _append(journey, root, exception_db, "advice-exception", "occ-exception", "advice-occasion",
            _occasion_record(0, "revise", "exception-review", target, exception_source, []))
    ad_hoc = _advise(journey, root, exception_db, "advice-exception", _request("unrelated-ad-hoc", target))
    ad_hoc_id = ad_hoc["result"]["attempt_id"]
    exception = {"state_visit": 0, "owner": "Fixture owner", "reason": "The advisor is unavailable and this due occasion cannot wait.",
                 "occasion_ids": ["exception-review"]}
    _show(journey, root, exception_db, "advice-exception")
    no_answers_excused = _engine(journey, root, exception_db, "event", "advice-exception", "revise",
                                 "--advice-exception", json.dumps(exception), expect="error")[0]
    if "no valid reasoned disposition" not in no_answers_excused.get("message", ""):
        raise ValueError(f"owner exception excused an undispositioned successful answer: {no_answers_excused!r}")
    _dispositions(journey, root, exception_db, "advice-exception", ad_hoc_id, "unrelated-ad-hoc", target,
                  statuses={"decision": "reject", "coverage": "reject", "risk": "partial"})
    wrong_scope = {**exception, "occasion_ids": ["not-this-occasion"]}
    _show(journey, root, exception_db, "advice-exception")
    wrong_exception = _engine(journey, root, exception_db, "event", "advice-exception", "revise",
                               "--advice-exception", json.dumps(wrong_scope), expect="error")[0]
    if wrong_exception.get("code") != "invalid-advice-exception":
        raise ValueError(f"advice exception accepted an unrelated occasion: {wrong_exception!r}")
    _show(journey, root, exception_db, "advice-exception")
    exceptional, _ = _engine(journey, root, exception_db, "event", "advice-exception", "revise",
                             "--advice-exception", json.dumps(exception))
    outcome = exceptional["result"]["history"]["action"]["outcome"]
    if (outcome.get("outcome") != "advice-exception"
            or outcome.get("exception", {}).get("occasion_ids") != ["exception-review"]
            or outcome.get("exception", {}).get("owner") != "Fixture owner"
            or outcome.get("exception", {}).get("reason") != exception["reason"]):
        raise ValueError(f"scoped advice exception was not visibly retained: {outcome!r}")
    if exceptional["result"]["run"]["has_overrides"] or exceptional["result"]["run"]["override_count"] != 0:
        raise ValueError("advice-only exception was conflated with the broader event override")

    # A check-free exception never calls evaluate; a checked exception does
    # not make a provider denial pass.
    denied_db = root / "exception-checked.sqlite"
    _start(journey, root, providers, advice_config, "advice-exception-checked", denied_db, _map("approve", "exception-checked"))
    denied_source = _source(journey, root, denied_db, "advice-exception-checked", "denied")
    _append(journey, root, denied_db, "advice-exception-checked", "occ-denied", "advice-occasion",
            _occasion_record(0, "approve", "exception-checked", target, denied_source, []))
    provider_mode.write_text("deny")
    denied_exception = {"state_visit": 0, "owner": "Fixture owner", "reason": "The due response is unavailable.",
                        "occasion_ids": ["exception-checked"]}
    provider_before = int(provider_counter.read_text())
    _show(journey, root, denied_db, "advice-exception-checked")
    denied, _ = _engine(journey, root, denied_db, "--config", str(providers), "event", "advice-exception-checked", "approve",
                        "--advice-exception", json.dumps(denied_exception), expect="error")
    if denied.get("code") != "scripted-denial" or int(provider_counter.read_text()) != provider_before + 1:
        raise ValueError(f"scoped advice exception bypassed the checked provider gate: {denied!r}")
    denied_show, _ = _show(journey, root, denied_db, "advice-exception-checked", "full")
    if denied_show["result"]["current_state"] != "work":
        raise ValueError("provider denial after advice exception changed the run state")
    provider_mode.write_text("allow")

    # Failed/invalid advice attempts are not answers and do not satisfy an
    # occurrence. The owner exception is not needed to observe normal refusal.
    for mode in ("invalid", "timeout"):
        failed_db = root / f"{mode}.sqlite"
        run_id = f"advice-{mode}"
        occasion_id = f"{mode}-review"
        _start(journey, root, providers, advice_config, run_id, failed_db, _map("revise", occasion_id))
        source_id = _source(journey, root, failed_db, run_id, mode)
        failed = _advise(journey, root, failed_db, run_id, _request(occasion_id, target, mode=mode), expect="error")
        failed_attempt = failed.get("details", {}).get("attempt_id")
        if not failed_attempt:
            raise ValueError(f"{mode} advice failure omitted its capture ID: {failed!r}")
        _append(journey, root, failed_db, run_id, f"occ-{mode}", "advice-occasion",
                _occasion_record(0, "revise", occasion_id, target, source_id, [failed_attempt]))
        _show(journey, root, failed_db, run_id)
        refused = _engine(journey, root, failed_db, "event", run_id, "revise", expect="error")[0]
        if "unanswered due advice occasions" not in refused.get("message", ""):
            raise ValueError(f"{mode} attempt incorrectly satisfied advice closure: {refused!r}")

    # Advice does not interlock ordinary observation, invocation, cancellation
    # or verified cleanup. Departure still refuses afterwards until closure.
    cancel_db = root / "cancel.sqlite"
    cancel_map = _map("revise", "cancel-review") + _map("approve", "cancel-approve-review")
    _start(journey, root, providers, advice_config, "advice-cancel", cancel_db, cancel_map, cancel_fixture=True)
    cancel_source = _source(journey, root, cancel_db, "advice-cancel", "cancel-approve")
    _append(journey, root, cancel_db, "advice-cancel", "occ-cancel-approve", "advice-occasion",
            _occasion_record(0, "approve", "cancel-approve-review", target, cancel_source, []))
    _show(journey, root, cancel_db, "advice-cancel")
    invoked, _ = _engine(journey, root, cancel_db, "--timeout-ms", "30000", "invoke", "advice-cancel", "slow")
    invocation_id = invoked["result"]["invocation_id"]
    # The public cancellation operation requires published local ownership;
    # give the spawned waiter time to persist that identity before requesting it.
    time.sleep(0.5)
    _show(journey, root, cancel_db, "advice-cancel")
    _engine(journey, root, cancel_db, "cancel-invocation", "advice-cancel", invocation_id)
    cancelled, _ = _show(journey, root, cancel_db, "advice-cancel", "status")
    status = next(row for row in cancelled["result"]["invocations"]["items"] if row["invocation_id"] == invocation_id)
    if status.get("execution", {}).get("state") != "cancelled":
        raise ValueError(f"advice closure blocked cancellation or verified cleanup: {status!r}")
    _show(journey, root, cancel_db, "advice-cancel")
    bound_exception = {"state_visit": 0, "owner": "Fixture owner", "reason": "The advice command is unavailable.",
                       "occasion_ids": ["cancel-approve-review"]}
    bound_refused = _engine(journey, root, cancel_db, "event", "advice-cancel", "approve",
                            "--advice-exception", json.dumps(bound_exception), expect="error")[0]
    if bound_refused.get("code") != "bound-slot-invocation-required":
        raise ValueError(f"advice exception bypassed bound completion: {bound_refused!r}")
    _show(journey, root, cancel_db, "advice-cancel")
    _engine(journey, root, cancel_db, "event", "advice-cancel", "revise", expect="error")

    # No map means a historical/disabled run receives no new departure guard.
    disabled_db = root / "disabled.sqlite"
    artifacts = root / "artifacts" / "advice-disabled"
    artifacts.mkdir(parents=True, exist_ok=True)
    disabled_input = {"artifact_root": str(artifacts)}
    _capture(root, [str(journey.engine), "--database", str(disabled_db), "--config", str(providers), "--json",
                    "start", "--id", "advice-disabled", "fixture", json.dumps(disabled_input)])
    _show(journey, root, disabled_db, "advice-disabled")
    disabled, _ = _engine(journey, root, disabled_db, "event", "advice-disabled", "revise")
    if disabled["result"]["run"]["current_state"] != "work":
        raise ValueError("a disabled/historical run acquired the new advice guard")

    (root / "sol-advice-transition-proof.json").write_text(json.dumps({
        "case": "sol-advice-transition",
        "status": "passed-scripted-assertions",
        "observed_paths": [
            "checked-and-check-free-departures",
            "ad-hoc-and-superseded-answer-dispositions",
            "changed-target-freshness",
            "scoped-owner-exception-and-independent-checked-gate",
            "invalid-timeout-cancellation-cleanup-and-disabled-mode",
        ],
        "public_command_captures": len(list((root / "commands").glob("*.argv.json"))),
        "semantic_advice_quality": "not-judged-by-scripted-fixture",
        "owner_exception": "fixture mechanics only; no real owner attestation",
    }, indent=2) + "\n", encoding="utf-8")
    print("advice transition closure passed: checked/check-free, dispositions, changed targets, scoped exception, and unrestricted cleanup")
