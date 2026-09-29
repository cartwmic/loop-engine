"""Public software-change advice preparation and future-run completion proof.

All reviewers/advisors are deterministic local fixtures. The script exercises
public provider/engine processes and never treats their output as semantic
approval.
"""
from __future__ import annotations

import copy
import hashlib
import importlib.util
import json
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path
from typing import Any

import dogfood_observation


OCCASIONS = [
    "review-candidates",
    "accepted-defect",
    "implementation-correction",
    "execution-or-authority-issue",
    "evidence-applicability",
    "requirements-reconciliation",
    "review-round-departure",
    "final-completion",
]
SUBJECT_BY_GATE = {
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


def _root(journey) -> Path:
    return dogfood_observation._fresh_root(journey, "sol-advice-provider")


def _capture(
    root: Path,
    argv: list[str],
    *,
    input_value: Any = None,
    cwd: Path | None = None,
    expected_code: int = 0,
    timeout: float = 90,
) -> tuple[dict[str, Any] | None, subprocess.CompletedProcess[bytes]]:
    logs = root / "commands"
    logs.mkdir(exist_ok=True)
    ordinal = len(list(logs.glob("*.argv.json")))
    stem = f"{ordinal:05d}"
    started = time.perf_counter_ns()
    payload = None if input_value is None else json.dumps(input_value, separators=(",", ":")).encode()
    try:
        completed = subprocess.run(argv, cwd=cwd or root, input=payload, capture_output=True, timeout=timeout, check=False)
    except (OSError, subprocess.TimeoutExpired) as error:
        raise ValueError(f"public advice-provider command did not complete: {argv!r}: {error}") from error
    elapsed_ms = (time.perf_counter_ns() - started) / 1_000_000
    (logs / f"{stem}.argv.json").write_text(json.dumps({"argv": argv, "cwd": str(cwd or root)}) + "\n")
    (logs / f"{stem}.stdin").write_bytes(payload or b"")
    (logs / f"{stem}.stdout").write_bytes(completed.stdout)
    (logs / f"{stem}.stderr").write_bytes(completed.stderr)
    (logs / f"{stem}.exit.json").write_text(json.dumps({"returncode": completed.returncode, "elapsed_ms": elapsed_ms}) + "\n")
    if completed.returncode != expected_code:
        raise ValueError(
            f"public command exit mismatch: expected {expected_code}, got {completed.returncode}: "
            f"{argv!r}; stderr={completed.stderr[-1400:]!r}"
        )
    if not completed.stdout:
        return None, completed
    try:
        value = json.loads(completed.stdout)
    except (UnicodeDecodeError, json.JSONDecodeError):
        if expected_code != 0:
            return None, completed
        raise ValueError(f"public command returned non-JSON: {argv!r}: {completed.stdout[-1200:]!r}")
    if not isinstance(value, dict):
        raise ValueError(f"public command returned a non-object: {argv!r}: {value!r}")
    return value, completed


def _write_json(path: Path, value: Any) -> None:
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


def _setup_profiles(journey, root: Path, advice_config_path: Path) -> dict[str, dict[str, Any]]:
    roster_path = root / "review-roster.json"
    token_budget = {
        "model_id": "scripted-reviewer",
        "context_window_tokens": 64000,
        "system_tokens": 1000,
        "framing_tokens": 1000,
        "output_reserve_tokens": 1000,
        "reasoning_reserve_tokens": 1000,
    }
    roster = [
        {"author": f"fixture-reviewer-{n}", "command": "/bin/true", "args": [], "token_budget": token_budget}
        for n in ("a", "b")
    ]
    _write_json(roster_path, roster)
    selected: dict[str, dict[str, Any]] = {}
    for name in ("minimal", "standard", "high-rigor"):
        source = journey.data_root / f"crates/software-change-provider/data/configs/{name}.json"
        original = json.loads(source.read_text(encoding="utf-8"))
        advice_defaults = original.get("extra", {}).get("advice", {})
        occasions = advice_defaults.get("occasion_map")
        if occasions != OCCASIONS:
            raise ValueError(f"{name} does not ship the common explicit eight-family advice map: {occasions!r}")
        if (set(advice_defaults) != {"occasion_map"}
                or "advice_command" in original or "advice_departures" in original):
            raise ValueError(f"{name} bundled advice defaults must not select a command or frozen departure gate")
        configured_path = root / f"setup-{name}-configured.json"
        configured, _ = _capture(
            root,
            [str(journey.provider), "setup", "--profile", str(source), "--roster", str(roster_path),
             "--engine", str(journey.engine), "--provider", str(journey.provider), "--output", str(configured_path),
             "--advice-config", str(advice_config_path)],
        )
        generated = json.loads(configured_path.read_text(encoding="utf-8"))
        advice = configured["enablement"]["advice"]
        if (advice.get("decision") != "configure" or advice.get("enabled") is not True
                or advice.get("configured") is not True or advice.get("command") != json.loads(advice_config_path.read_text())
                or advice.get("occasion_map") != OCCASIONS
                or configured["effective_policy"].get("advice_enabled") is not True
                or configured["effective_bindings"] != generated.get("work_slot_bindings")):
            raise ValueError(f"{name} setup did not freeze and expose the configured command, bounds, map, and bindings")
        departures = advice.get("departure_map", {}).get("occasions", [])
        families = {str(row.get("occasion_id", "")).split(":", 1)[0] for row in departures}
        if families != set(OCCASIONS):
            raise ValueError(f"{name} departure map omitted advice families: {families!r}")
        if not advice.get("occasion_descriptions") or set(advice["occasion_descriptions"]) != set(OCCASIONS):
            raise ValueError(f"{name} setup did not explain every default advice occasion")
        if configured.get("started") is not False or configured.get("preview", {}).get("errors"):
            raise ValueError(f"{name} configured setup started a run or failed binding preview")

        declined_path = root / f"setup-{name}-declined.json"
        declined, _ = _capture(
            root,
            [str(journey.provider), "setup", "--profile", str(source), "--roster", str(roster_path),
             "--engine", str(journey.engine), "--provider", str(journey.provider), "--output", str(declined_path),
             "--decline-advice"],
        )
        declined_profile = json.loads(declined_path.read_text(encoding="utf-8"))
        disabled = declined["enablement"]["advice"]
        if (disabled.get("decision") != "decline" or disabled.get("enabled") is not False
                or disabled.get("configured") is not False or "advice_command" in declined_profile
                or "advice_departures" in declined_profile
                or disabled.get("occasion_map") != OCCASIONS):
            raise ValueError(f"{name} explicit decline selected a backend or retained active advice gates")
        selected[name] = {"configured": configured, "configured_profile": generated,
                          "declined": declined, "declined_profile": declined_profile}

    omitted = root / "setup-no-choice.json"
    _, no_choice = _capture(
        root,
        [str(journey.provider), "setup", "--profile", str(journey.data_root / "crates/software-change-provider/data/configs/high-rigor.json"),
         "--roster", str(roster_path), "--engine", str(journey.engine), "--provider", str(journey.provider),
         "--output", str(omitted)],
        expected_code=2,
    )
    if omitted.exists() or b"choose exactly one" not in no_choice.stderr:
        raise ValueError("setup silently treated an omitted advice choice as decline")
    both = root / "setup-both-choices.json"
    _, both_result = _capture(
        root,
        [str(journey.provider), "setup", "--profile", str(journey.data_root / "crates/software-change-provider/data/configs/high-rigor.json"),
         "--roster", str(roster_path), "--engine", str(journey.engine), "--provider", str(journey.provider),
         "--output", str(both), "--advice-config", str(advice_config_path), "--decline-advice"],
        expected_code=2,
    )
    if both.exists() or b"choose exactly one" not in both_result.stderr:
        raise ValueError("setup accepted simultaneous configure and decline choices")
    return selected


def _profile_map(profile: dict[str, Any], *, bookends: bool = False) -> dict[str, Any]:
    configured = copy.deepcopy(profile)
    configured.pop("work_slot_bindings", None)
    configured.pop("artifact_root", None)
    configured["extra"] = copy.deepcopy(configured.get("extra", {}))
    if bookends:
        configured["extra"]["bookends"] = {"enabled": True}
    return configured


def _append_record(journey, run_id: str, state: str, record_id: str, kind: str, data: dict[str, Any]) -> dict[str, Any]:
    journey._assert_show_for(run_id, state, f"p11-before-append-{record_id}")
    result = journey._engine_for(
        run_id,
        ["append", f"--record-id={record_id}", run_id, kind, json.dumps(data, separators=(",", ":"))],
        state=state,
        event="append",
        axis=kind,
    )
    journey._expect_status(result, "completed", event="append", state=state, axis=kind)
    if result.get("result", {}).get("context", {}).get("id") != record_id:
        raise ValueError(f"public append changed record identity {record_id!r}: {result!r}")
    return result


def _fixture_judgments(show, occasion_id, source_ids):
    """Explicit scripted driver claims; not provider-generated reasoning tasks."""
    family = occasion_id.split(":", 1)[0]
    selected = [r for r in show["context"] if r["id"] in source_ids]
    ids = []
    if family == "review-candidates":
        ids = [f"finding.{r['id']}.{axis}" for r in selected
               if r["kind"] == "review-evidence" and r["data"].get("result") == "fail"
               for axis in ("support", "materiality", "scope")]
    elif family in ("accepted-defect", "implementation-correction"):
        suffix = "owner" if family == "accepted-defect" else "route"
        ids = [f"defect.{f['id']}.{suffix}" for r in selected if r["kind"] == "finding-ledger"
               for f in r["data"]["findings"] if f.get("disposition") == "accepted" and f.get("status") == "unresolved"]
    elif family == "execution-or-authority-issue":
        ids = ["blocker.authority"]
    elif family == "evidence-applicability":
        ids = [f"source.{r['id']}.applicability" for r in selected if r["kind"] == "evidence-applicability"]
        ids += [f"source.{r['id']}.same-assertion" for r in selected if "before_assertion" in r["data"] and "after_assertion" in r["data"]]
    elif family == "requirements-reconciliation":
        ids = ["outcome.branch"]
    elif family == "review-round-departure":
        ids = ["gate.departure"]
    elif family == "final-completion":
        intent = json.loads((Path(show["initial_input"]["artifact_root"]) / "intent.json").read_text())
        ids = [f"criterion.{r['id']}.{axis}" for r in intent["acceptance"] for axis in ("fulfillment", "checks-could-miss")]
        ids += ["goal.fulfillment", "goal.checks-could-miss"]
    claim = "The selected excerpt describes an actual observation rather than a future plan."
    if family == "review-candidates":
        claim = "The selected finding's cited revision alone does not describe an observable failure."
    elif family == "evidence-applicability":
        claim = "The supplied before and after assertions describe the same observable behavior."
    return {id: claim for id in ids}


def _provider_request(
    journey,
    root: Path,
    show: dict[str, Any],
    occasion_id: str,
    source_ids: list[str],
    artifacts: list[str],
    documents: list[dict[str, str]] | None = None,
) -> dict[str, Any]:
    packet = {
        "show": {"status": "completed", "result": show},
        "occasion_id": occasion_id,
        "admissibility": {"bounded_judgment": True, "evidence_sufficient": True},
        "judgments": _fixture_judgments(show, occasion_id, source_ids),
        "source_context_ids": source_ids,
        "artifact_names": artifacts,
        "documents": documents or [],
    }
    value, _ = _capture(root, [str(journey.provider), "advice-request"], input_value=packet)
    request = value
    if (not isinstance(request, dict) or request.get("occasion") != occasion_id
            or not request.get("questions") or request.get("target", {}).get("run_id") != show.get("run_id")):
        raise ValueError(f"provider did not prepare a selected evidence request: {request!r}")
    return request


def _answer(request: dict[str, Any], mode: str) -> dict[str, Any]:
    answers: dict[str, Any] = {}
    for ordinal, (question_id, question) in enumerate(request["questions"].items()):
        kind = question["type"]
        rationale = "The answer is limited to the supplied evidence." if ordinal % 2 == 0 else None
        if kind == "choice":
            choices = list(question["criteria"])
            if "supported" in choices:
                choice = "contradicted" if mode.startswith("wrong") else "supported"
            elif mode.startswith("wrong") and question_id.endswith((".fulfillment", ".materiality")):
                choice = "delivery-failed" if "delivery-failed" in choices else "material"
                if choice not in choices:
                    choice = choices[-1]
            elif question_id.endswith(".owner") and "implementation" in choices:
                choice = "implementation"
            elif question_id.endswith(".route") and "task-and-dependants" in choices:
                choice = "task-and-dependants"
            elif question_id == "outcome.branch" and "sufficient-existing-wording" in choices:
                choice = "sufficient-existing-wording"
            elif question_id == "gate.departure" and "repair-needed" in choices:
                choice = "repair-needed" if mode == "correct-with-finding" else "supports-departure"
            elif question_id.endswith(".materiality") and "nonmaterial" in choices:
                choice = "nonmaterial"
            else:
                choice = choices[0]
            selected_probability = 0.99 if mode.startswith("wrong") else 0.8
            rest = (1.0 - selected_probability) / max(1, len(choices) - 1)
            probabilities = {key: (selected_probability if key == choice else rest) for key in choices}
            answer = {"type": "choice", "choice": choice, "probabilities": probabilities,
                      "confidence": 0.99 if mode.startswith("wrong") else 0.82}
            if rationale is not None:
                answer["rationale"] = rationale
            answers[question_id] = answer
        elif kind == "score":
            levels = question["levels"]
            chosen = 0 if mode.startswith("wrong") else len(levels) - 1
            probabilities = {str(index): (0.9 if index == chosen else 0.1 / max(1, len(levels) - 1))
                             for index in range(len(levels))}
            score = sum(int(index) * probability for index, probability in probabilities.items())
            answer = {
                "type": "score", "score": score,
                "legend": {str(index): level["description"] for index, level in enumerate(levels)},
                "probabilities": probabilities, "confidence": 0.99 if mode.startswith("wrong") else 0.83,
            }
            if rationale is not None:
                answer["rationale"] = rationale
            answers[question_id] = answer
        elif kind == "noul":
            negative = "checks-could-miss" in question_id and not mode.startswith("wrong")
            value = 0.95 if mode.startswith("wrong") else (0.15 if negative else 0.85)
            answer = {"type": "noul", "noul": value}
            if rationale is not None:
                answer["rationale"] = rationale
            answers[question_id] = answer
        else:
            raise ValueError(f"fixture advisor received an unsupported question: {question!r}")
    return {"answers": answers}


def _advise(journey, root: Path, run_id: str, request: dict[str, Any], mode: str) -> dict[str, Any]:
    altered = copy.deepcopy(request)
    altered["state"]["fixture_response_mode"] = mode
    path = root / f"advice-request-{mode}-{time.time_ns()}.json"
    _write_json(path, altered)
    result = journey._engine_for(run_id, ["advise", run_id, f"@{path}"], state=journey.state, event="advise")
    journey._expect_status(result, "completed", event="advise", state=journey.state)
    payload = result.get("result", {})
    expected = _answer(altered, mode)
    actual = payload.get("response", {}).get("answers", {})
    if not payload.get("attempt_id") or set(actual) != set(expected["answers"]):
        raise ValueError(f"configured scripted command returned different answer IDs: {payload!r}")
    for answer_id, expected_answer in expected["answers"].items():
        actual_answer = actual[answer_id]
        if actual_answer.get("type") != expected_answer["type"]:
            raise ValueError(f"configured scripted command changed answer type for {answer_id}: {actual_answer!r}")
        if expected_answer["type"] == "choice" and actual_answer.get("choice") != expected_answer["choice"]:
            raise ValueError(f"configured scripted command changed Choice for {answer_id}: {actual_answer!r}")
        if expected_answer["type"] == "score" and actual_answer.get("legend") != expected_answer["legend"]:
            raise ValueError(f"configured scripted command changed Score legend for {answer_id}: {actual_answer!r}")
    return {"attempt_id": payload["attempt_id"], "response": payload["response"], "request": altered}


def _dispositions(
    journey,
    run_id: str,
    state: str,
    response: dict[str, Any],
    *,
    status: str,
    reason: str,
    applicability_id: str | None = None,
    target: dict[str, Any] | None = None,
    omit_answer: str | None = None,
) -> None:
    request = response["request"]
    for index, answer_id in enumerate(response["response"]["answers"]):
        if answer_id == omit_answer:
            continue
        data: dict[str, Any] = {
            "response_id": response["attempt_id"],
            "answer_id": answer_id,
            "occasion_id": request["occasion"],
            "state_visit": request["target"]["state_visit"],
            "target": target or request["target"],
            "disposition": status,
            "reason": reason,
        }
        if applicability_id is not None:
            data["applicability_id"] = applicability_id
        _append_record(journey, run_id, state,
                       f"disp-{response['attempt_id']}-{index}-{time.time_ns()}",
                       "advice-disposition", data)


def _append_one_disposition(
    journey,
    run_id: str,
    state: str,
    response: dict[str, Any],
    answer_id: str,
    *,
    status: str,
    reason: str,
    target: dict[str, Any],
    applicability_id: str | None = None,
) -> None:
    request = response["request"]
    data: dict[str, Any] = {
        "response_id": response["attempt_id"],
        "answer_id": answer_id,
        "occasion_id": request["occasion"],
        "state_visit": request["target"]["state_visit"],
        "target": target,
        "disposition": status,
        "reason": reason,
    }
    if applicability_id is not None:
        data["applicability_id"] = applicability_id
    _append_record(journey, run_id, state, f"disp-{response['attempt_id']}-{answer_id}-{time.time_ns()}",
                   "advice-disposition", data)


def _prepare_family(
    journey,
    root: Path,
    run_id: str,
    family: str,
    source_ids: list[str],
    artifacts: list[str],
    *,
    documents: list[dict[str, str]] | None = None,
) -> dict[str, Any]:
    shown = journey._show_for(run_id, state=journey.state, event=f"p11-advice-show-{family}")
    departures = shown["initial_input"].get("advice_departures", {}).get("occasions", [])
    state = shown["current_state"]
    row = next((item for item in departures
                if item.get("state") == state and item.get("occasion_id", "").startswith(family + ":")), None)
    if row is None:
        raise ValueError(f"no actual {family} departure is mapped for current state {state}")
    return _provider_request(journey, root, shown, row["occasion_id"], source_ids, artifacts, documents)


def _record_occasion(
    journey,
    run_id: str,
    state: str,
    event: str,
    occasion: dict[str, Any],
    *,
    triggered: bool,
    request: dict[str, Any] | None = None,
    source_ids: list[str] | None = None,
    response_ids: list[str] | None = None,
    reason: str | None = None,
) -> None:
    visit = occasion["state_visit"]
    data: dict[str, Any] = {
        "state_visit": visit,
        "source_state": state,
        "event": event,
        "occasion_id": occasion["occasion_id"],
        "target": request["target"] if request else occasion["target"],
        "triggered": triggered,
    }
    if triggered:
        data["trigger_source_ids"] = source_ids or []
        data["response_ids"] = response_ids or []
    else:
        data["reason"] = reason or "No event-specific decision of this family is due on this observed departure."
    identity = f"{occasion['occasion_id']}:{visit}"
    _append_record(journey, run_id, state,
                   "occasion-" + hashlib.sha256(identity.encode()).hexdigest()[:16],
                   "advice-occasion", data)


def _simple_target(show: dict[str, Any], source_ids: list[str]) -> dict[str, Any]:
    return {"run_id": show["run_id"], "state": show["current_state"],
            "state_visit": show["state_visit"], "source_context_ids": source_ids}


def _map_families(profile: dict[str, Any], state: str, event: str) -> list[dict[str, Any]]:
    return [row for row in profile["advice_departures"]["occasions"]
            if row["state"] == state and row["event"] == event]


def _add_driver_note(journey, run_id: str, state: str, record_id: str, data: dict[str, Any]) -> str:
    _append_record(journey, run_id, state, record_id, "driver-note", data)
    return record_id


def _append_review_gate(
    journey,
    run_id: str,
    gate: str,
    *,
    failing: tuple[str, str] | None = None,
) -> dict[str, Any]:
    subject = SUBJECT_BY_GATE[gate]
    artifact_path = journey.artifact_root / subject
    raw = artifact_path.read_bytes()
    document = json.loads(raw)
    revision = document["revision"]
    digest = "sha256:" + hashlib.sha256(raw).hexdigest()
    evidence_ids: list[str] = []
    selected_failure: dict[str, Any] | None = None
    for axis in journey._overlay_axes(gate):
        axis_id = axis["id"]
        stage = axis.get("review_stage", "aggregate")
        required = int(axis.get("required_authors", 1))
        for author_number in range(required):
            is_failure = failing == (axis_id, f"reviewer-{author_number}")
            record_id = f"evidence-{gate}-{stage}-{axis_id}-reviewer-{author_number}"
            data = {
                "gate": gate,
                "policy_id": axis_id,
                "review_stage": stage,
                "result": "fail" if is_failure else "pass",
                "findings": "The supplied check gap is not established by the actual selected excerpt." if is_failure else "",
                "review_contract_version": 2,
                "grounds": {
                    "reason": "The scripted external reviewer inspected the exact current artifact excerpt.",
                    "evidence": [{"locator": f"{subject}#/revision", "sha256": digest}],
                },
                "author": {"name": f"scripted-{gate}-{axis_id}-{author_number}", "kind": "script"},
                "subject": subject,
                "subject_revision": revision,
                "config_version": journey.profile["config_version"],
            }
            _append_record(journey, run_id, journey.state, record_id, "review-evidence", data)
            evidence_ids.append(record_id)
            if is_failure:
                selected_failure = {"id": record_id, "data": data}
    return {"subject": subject, "revision": revision, "evidence_ids": evidence_ids,
            "failure": selected_failure}


def _append_ledger(
    journey,
    run_id: str,
    gate: str,
    subject: str,
    revision: str,
    *,
    finding: dict[str, Any] | None = None,
    record_id: str | None = None,
) -> str:
    data = {
        "schema_version": "1",
        "gate": gate,
        "subject": subject,
        "subject_revision": revision,
        "author": {"name": "scripted-driver", "kind": "agent"},
        "findings": [finding] if finding else [],
    }
    record_id = record_id or f"ledger-{gate}-{time.time_ns()}"
    _append_record(journey, run_id, journey.state, record_id, "finding-ledger", data)
    return record_id


def _finding(
    source_id: str,
    policy_id: str,
    *,
    disposition: str,
    owner_phase: str | None,
    task_ids: list[str],
    status: str,
) -> dict[str, Any]:
    return {
        "id": "F-advice-" + hashlib.sha256(source_id.encode()).hexdigest()[:12],
        "source": {"kind": "context-record", "id": source_id},
        "policy_id": policy_id,
        "statement": "The supplied check gap is not established by the actual selected excerpt.",
        "disposition": disposition,
        "reason": "The fixture driver independently triaged this exact selected source.",
        "owner_phase": owner_phase,
        "task_ids": task_ids,
        "review_axes": [policy_id] if disposition == "accepted" else [],
        "status": status,
    }


def _trigger_family(
    journey,
    root: Path,
    run_id: str,
    family: str,
    current: dict[str, Any],
) -> tuple[dict[str, Any], list[str], list[dict[str, Any]]]:
    context = {row["id"]: row for row in current["context"]}
    gate = current["current_state"]
    if family == "review-candidates":
        source_ids = [journey._p11_sources["intent_failure"]]
        artifacts = ["intent.json"]
        docs = None
    elif family == "accepted-defect":
        source_ids = [journey._p11_sources["implementation_ledger"]]
        artifacts = ["intent.json", "implementation-report.json"]
        docs = None
    elif family == "implementation-correction":
        source_ids = [journey._p11_sources["implementation_ledger"],
                      journey._p11_sources["implementation_owner_note"]]
        artifacts = ["plan.json"]
        docs = None
    elif family == "execution-or-authority-issue":
        source_ids = [journey._p11_sources["execution_issue"]]
        artifacts = []
        docs = None
    elif family == "evidence-applicability":
        source_ids = [journey._p11_sources["applicability"], journey._p11_sources["before-after"]]
        artifacts = ["intent.json"]
        docs = None
    elif family == "requirements-reconciliation":
        source_ids = [journey._p11_sources["reconciliation-observation"]]
        artifacts = ["reconciliation.json", "intent.json"]
        prd = journey.repository_root / "docs" / "PRD.md"
        docs = [{"path": str(prd), "sha256": "sha256:" + hashlib.sha256(prd.read_bytes()).hexdigest()}]
    elif family == "review-round-departure":
        rows = [row for row in current["context"] if row["kind"] == "review-evidence"
                and row["data"].get("gate") == gate]
        ledgers = [row for row in current["context"] if row["kind"] == "finding-ledger"
                   and row["data"].get("gate") == gate]
        if gate == "implementation-review" and journey._p11_sources.get("implementation_ledger_resolved"):
            ledgers.extend([context[journey._p11_sources["implementation_ledger"]],
                            context[journey._p11_sources["implementation_ledger_resolved"]]])
        source_ids = [row["id"] for row in rows + ledgers]
        artifacts = []
        docs = None
    elif family == "final-completion":
        report = json.loads((journey.artifact_root / "validation-report.json").read_text())
        source_ids = list(report["command_evidence_ids"])
        for row in report["criteria"]:
            source_ids.extend(row["verdict_ids"])
        source_ids.extend(report["goal_verdict_ids"])
        artifacts = ["intent.json", "plan.json", "validation-report.json"]
        docs = None
    else:
        raise ValueError(f"no source selector is defined for {family} in {gate}")
    if any(source_id not in context for source_id in source_ids):
        raise ValueError(f"{family} selected a source absent from current full show: {source_ids!r}")
    rows = [row for row in current["initial_input"]["advice_departures"]["occasions"]
            if row["state"] == gate and row["event"] == journey._p11_event]
    occasion = next(row for row in rows if row["occasion_id"].startswith(family + ":"))
    request = _provider_request(
        journey, root, current, occasion["occasion_id"], source_ids, artifacts, docs,
    )
    return request, source_ids, [occasion]


def _advice_response(journey, root: Path, run_id: str, request: dict[str, Any], mode: str) -> dict[str, Any]:
    return _advise(journey, root, run_id, request, mode)


def _final_completion_departure(journey, root: Path, run_id: str, state: str, event: str,
                                row: dict[str, Any], shown: dict[str, Any]) -> None:
    request, selected_ids, _ = _trigger_family(journey, root, run_id, "final-completion", shown)
    correct = _advice_response(journey, root, run_id, request, "correct")
    wrong = _advice_response(journey, root, run_id, request, "wrong-high-confidence")
    rationales = [answer.get("rationale") for answer in correct["response"]["answers"].values()]
    if not any(rationale for rationale in rationales) or not any(rationale is None for rationale in rationales):
        raise ValueError("software-change scripted answers did not preserve optional rationale absent and present")
    first_ac = json.loads((journey.artifact_root / "intent.json").read_text())["acceptance"][0]["id"]
    wrong_answer = wrong["response"]["answers"][f"criterion.{first_ac}.fulfillment"]
    if wrong_answer.get("choice") != "contradicted" or wrong_answer.get("confidence", 0) < 0.99:
        raise ValueError("future-run scripted advisor did not return the planned confidently wrong fulfillment answer")
    for response, status in ((correct, "accept"), (wrong, "reject")):
        _dispositions(journey, run_id, state, response, status=status,
                      reason=("The current captured checks support the answer; driver decision remains separate."
                              if status == "accept" else "The passing captured assertion contradicts this high-confidence wrong answer."))

    # The new selected driver observation changes the target identity without
    # changing the underlying proof. An old response cannot silently carry to it.
    changed_target_note = "final-target-revision-note"
    _append_record(journey, run_id, state, changed_target_note, "driver-note", {
        "observation":"A new target-linked driver note was selected after the original answer.",
        "actual_check_assertion":"The captured validation command remains successful and unchanged."
    })
    fresh_show = journey._show_for(run_id, state=state, event="final-target-re-show")
    fresh_request = _provider_request(
        journey, root, fresh_show, row["occasion_id"], selected_ids + [changed_target_note],
        ["intent.json", "plan.json", "validation-report.json"],
    )
    if fresh_request["target"] == request["target"]:
        raise ValueError("changed selected target did not produce a fresh advice target identity")
    occasion = {**row, "state_visit": shown["state_visit"], "target": fresh_request["target"]}
    _record_occasion(journey, run_id, state, event, occasion, triggered=True,
                     request=fresh_request, source_ids=selected_ids + [changed_target_note],
                     response_ids=[correct["attempt_id"], wrong["attempt_id"]])

    # Freshness refusal: the selected responses refer to the old target and no
    # applicability declaration has yet been supplied.
    journey._assert_show_for(run_id, state, "final-stale-target-observation")
    stale = journey._engine_for(run_id, ["event", run_id, event], state=state, event=event)
    if stale.get("status") not in {"error", "rejected"} or "unanswered due advice" not in json.dumps(stale).lower():
        raise ValueError(f"changed-target advice was not blocked before applicability: {stale!r}")
    journey._assert_show_for(run_id, state, "final-after-stale-denial")

    for response in (correct, wrong):
        applicability_id = f"final-applicability-{response['attempt_id']}"
        _append_record(journey, run_id, state, applicability_id, "advice-applicability", {
            "response_id":response["attempt_id"],"occasion_id":row["occasion_id"],
            "state_visit":shown["state_visit"],"target":fresh_request["target"],
            "attesting_driver":"scripted-owner-driver",
            "reason":"The driver compared the exact current check assertions and found the response remains applicable to this target."
        })
        _dispositions(journey, run_id, state, response,
                      status="reject" if response is wrong else "partial",
                      reason=("Rejected wrong advice on the refreshed target after inspecting the actual passing check."
                              if response is wrong else "The driver recorded partial applicability on the refreshed target."),
                      applicability_id=applicability_id, target=fresh_request["target"])
    journey._assert_show_for(run_id, state, "final-advice-closure-ready")


def _close_departure(journey, root: Path, run_id: str, event: str, target_state: str) -> None:
    journey._p11_event = event
    state = journey.state
    shown = journey._show_for(run_id, state=state, event=f"advice-before-{event}")
    guidance = shown.get("action_guidance", {}).get("advice", {})
    if guidance.get("enabled") is not True:
        raise ValueError(f"enabled software-change state omitted actionable advice guidance: {guidance!r}")
    for field in ("request_path", "ad_hoc", "disposition", "occasion_record", "authority", "limits"):
        if not guidance.get(field):
            raise ValueError(f"action guidance omitted {field}: {guidance!r}")
    due = [row for row in shown["initial_input"]["advice_departures"]["occasions"]
           if row["state"] == state and row["event"] == event]
    listed = {row.get("occasion_id") for row in guidance.get("due_departures", [])
              if row.get("event") == event}
    if {row["occasion_id"] for row in due} != listed:
        raise ValueError(f"state action guidance did not name exact due occasions for {state}/{event}")
    for row in due:
        family = row["occasion_id"].split(":", 1)[0]
        triggered = False
        answer_response: list[dict[str, Any]] = []
        selected_ids: list[str] = []
        request = None
        if family == "final-completion" and (state, event, family) in journey._p11_trigger_families:
            _final_completion_departure(journey, root, run_id, state, event, row, shown)
            continue
        if (state, event, family) in journey._p11_trigger_families:
            current = journey._show_for(run_id, state=state, event=f"advice-source-{family}")
            request, selected_ids, _ = _trigger_family(journey, root, run_id, family, current)
            if family == "review-candidates":
                answer_response = [
                    _advice_response(journey, root, run_id, request, "correct-with-finding"),
                    _advice_response(journey, root, run_id, request, "wrong-finding"),
                ]
            elif family == "final-completion":
                answer_response = [
                    _advice_response(journey, root, run_id, request, "correct"),
                    _advice_response(journey, root, run_id, request, "wrong-high-confidence"),
                ]
            else:
                answer_response = [_advice_response(journey, root, run_id, request, "correct")]
            triggered = True
            if family == "requirements-reconciliation":
                journey._p11_sources["reconciliation-question_request"] = request
            if family == "final-completion":
                journey._p11_sources["final_responses"] = answer_response
                journey._p11_sources["final_sources"] = selected_ids
        if triggered:
            target = request["target"]
            response_ids = [response["attempt_id"] for response in answer_response]
            _record_occasion(journey, run_id, state, event,
                             {**row, "state_visit": shown["state_visit"], "target": target},
                             triggered=True, request=request, source_ids=selected_ids,
                             response_ids=response_ids)
            for response_index, response in enumerate(answer_response):
                status = "reject" if "wrong" in response["request"]["state"].get("fixture_response_mode", "") else (
                    "partial" if response_index == 0 and len(answer_response) > 1 else "accept")
                _dispositions(
                    journey, run_id, state, response, status=status,
                    reason=("Rejected the confidently wrong answer against the actual retained assertion; advice does not alter the driver ledger."
                            if status == "reject" else "Driver recorded a reasoned advisory disposition after inspecting selected evidence."),
                )
        else:
            target = _simple_target(shown, [])
            _record_occasion(journey, run_id, state, event,
                             {**row, "state_visit": shown["state_visit"], "target": target},
                             triggered=False, reason=f"No {family} trigger was present for this actual {state}/{event} departure.")
    journey._assert_show_for(run_id, state, f"advice-closure-before-{event}")


def _write_advisor(root: Path) -> tuple[Path, Path]:
    counter = root / "advisor-calls.txt"
    script = root / "scripted-advisor.py"
    script.write_text(
        "import json,pathlib,sys,time\n"
        "counter=pathlib.Path(sys.argv[1]); counter.write_text(str(int(counter.read_text())+1) if counter.exists() else '1')\n"
        "request=json.load(sys.stdin); mode=request.get('state',{}).get('fixture_response_mode','correct')\n"
        "if mode=='timeout': time.sleep(5)\n"
        "answers={}\n"
        "for i,(qid,q) in enumerate(request['questions'].items()):\n"
        " t=q['type']; wrong=mode.startswith('wrong')\n"
        " rationale='Only the selected evidence was considered.' if i%2==0 else None\n"
        " if t=='choice':\n"
        "  keys=list(q['criteria'])\n"
        "  if 'supported' in keys: choice='contradicted' if wrong else 'supported'\n"
        "  elif wrong and qid.endswith(('.fulfillment','.materiality')): choice='delivery-failed' if 'delivery-failed' in keys else 'material'\n"
        "  elif qid.endswith('.owner') and 'implementation' in keys: choice='implementation'\n"
        "  elif qid.endswith('.route') and 'task-and-dependants' in keys: choice='task-and-dependants'\n"
        "  elif qid=='outcome.branch' and 'sufficient-existing-wording' in keys: choice='sufficient-existing-wording'\n"
        "  elif qid.endswith('.materiality') and 'nonmaterial' in keys and not wrong: choice='nonmaterial'\n"
        "  elif qid=='gate.departure' and 'repair-needed' in keys: choice='repair-needed' if mode=='correct-with-finding' else 'supports-departure'\n"
        "  else: choice=keys[0]\n"
        "  peak=.99 if wrong else .8; rest=(1-peak)/max(1,len(keys)-1); probs={k:(peak if k==choice else rest) for k in keys}\n"
        "  a={'type':'choice','choice':choice,'probabilities':probs,'confidence':.99 if wrong else .82}\n"
        "  if rationale is not None: a['rationale']=rationale\n"
        " elif t=='score':\n"
        "  levels=q['levels']; chosen=0 if wrong else len(levels)-1; probs={str(n):(.9 if n==chosen else .1/max(1,len(levels)-1)) for n in range(len(levels))}\n"
        "  a={'type':'score','score':sum(int(n)*p for n,p in probs.items()),'legend':{str(n):v['description'] for n,v in enumerate(levels)},'probabilities':probs,'confidence':.99 if wrong else .83}\n"
        "  if rationale is not None: a['rationale']=rationale\n"
        " elif t=='noul':\n"
        "  a={'type':'noul','noul':(.99 if wrong else (.15 if 'checks-could-miss' in qid else .85))}\n"
        "  if rationale is not None: a['rationale']=rationale\n"
        " else: raise SystemExit(3)\n"
        " answers[qid]=a\n"
        "print(json.dumps({'answers':answers},separators=(',',':')))\n",
        encoding="utf-8",
    )
    return script, counter


def _selected_advice_config(script: Path, counter: Path) -> dict[str, Any]:
    return {
        "command": sys.executable,
        "args": [str(script), str(counter)],
        "timeout_ms": 2000,
        "max_request_bytes": 262144,
        "max_response_bytes": 262144,
    }


def _public_setup_and_question_families(journey, root: Path, setup_results: dict[str, dict[str, Any]],
                                        advice_config: dict[str, Any], advisor_script: Path,
                                        advisor_counter: Path) -> None:
    """Prepare every family from actual selected rows in a real engine full show."""
    profile = copy.deepcopy(setup_results["high-rigor"]["configured_profile"])
    artifact_root = root / "question-run-artifacts"
    artifact_root.mkdir()
    profile["artifact_root"] = str(artifact_root)
    intent = json.loads((journey.fixture_root / "intent-good.json").read_text(encoding="utf-8"))
    intent["revision"] = "intent-advice-question-r1"
    _write_json(artifact_root / "intent.json", intent)
    _write_json(artifact_root / "plan.json", {"revision":"plan-advice-r1","tasks":[{"id":"P11","dependencies":[]}]})
    implementation = json.loads((journey.fixture_root / "implementation-report-good.json").read_text(encoding="utf-8"))
    implementation["revision"] = "implementation-question-r1"
    _write_json(artifact_root / "implementation-report.json", implementation)
    _write_json(artifact_root / "reconciliation.json", {"revision":"reconciliation-r1","branch":"sufficient-existing-wording"})
    criterion_rows = [{"criterion_id":row["id"],"verdict_ids":[f"advice-criterion-{index}"]}
                      for index,row in enumerate(intent["acceptance"])]
    validation = {"revision":"validation-r1","command_evidence_ids":["advice-check"],
                  "criteria":criterion_rows,
                  "goal_verdict_ids":["advice-goal"]}
    _write_json(artifact_root / "validation-report.json", validation)
    docs = root / "question-documents"
    docs.mkdir()
    prd = docs / "PRD.md"
    prd.write_text("### LE-1: Selected operator outcome\n\nThe selected proof is inspectable.\n", encoding="utf-8")
    database = root / "question-run.sqlite"
    providers = root / "question-providers.toml"
    journey._write_provider_config_at(providers)
    profile_path = root / "question-profile.json"
    _write_json(profile_path, profile)
    journey.run_dir = root
    journey.database = database
    journey.provider_config = providers
    journey.profile_path = profile_path
    journey.artifact_root = artifact_root
    journey.repository_root = root
    journey.command_cwd = root
    journey.command_env = {}
    journey.profile = profile
    journey.work_slot_bindings = profile.get("work_slot_bindings", {})
    journey.state = "not-started"
    journey.run_id = "sol-advice-question-source"
    journey._start()
    # The ACTIVE full-show fixture uses real artifact files and public context
    # append operations below; it advances only after the intent is in place.
    journey._assert_show("explore", "p11-before-intent-ready")
    journey._expect_allow("intent-ready", "intent-review")
    intent_path = artifact_root / "intent.json"
    intent_hash = "sha256:" + hashlib.sha256(intent_path.read_bytes()).hexdigest()
    report_path = artifact_root / "implementation-report.json"
    report_revision = json.loads(report_path.read_text())["revision"]
    report_hash = "sha256:" + hashlib.sha256(report_path.read_bytes()).hexdigest()
    plan = json.loads((artifact_root / "plan.json").read_text())
    task_id = plan["tasks"][0]["id"]
    intent_failure_data = {
        "gate":"intent-review","policy_id":"acceptance-granularity","review_stage":"aggregate",
        "result":"fail","findings":"The selected excerpt may not prove the named operator outcome.",
        "review_contract_version":2,"grounds":{"reason":"The script inspected the exact current intent excerpt.",
            "evidence":[{"locator":"intent.json#/acceptance/0","sha256":intent_hash}]},
        "author":{"name":"question-reviewer","kind":"script"},"subject":"intent.json",
        "subject_revision":intent["revision"],"config_version":profile["config_version"]
    }
    implementation_axis = profile["review_policies"]["implementation-review"][0]
    implementation_failure_data = {
        "gate":"implementation-review","policy_id":implementation_axis["id"],
        "review_stage":implementation_axis.get("review_stage","aggregate"),
        "result":"fail","findings":"The implementation report may omit the selected operator assertion.",
        "review_contract_version":2,"grounds":{"reason":"The script inspected the current implementation report excerpt.",
            "evidence":[{"locator":"implementation-report.json#/revision","sha256":report_hash}]},
        "author":{"name":"question-reviewer-implementation","kind":"script"},
        "subject":"implementation-report.json","subject_revision":report_revision,
        "config_version":profile["config_version"]
    }
    intent_fail_id = "question-intent-fail"
    implementation_fail_id = "question-implementation-fail"
    _append_record(journey, journey.run_id, "intent-review", intent_fail_id, "review-evidence", intent_failure_data)
    _append_record(journey, journey.run_id, "intent-review", implementation_fail_id, "review-evidence", implementation_failure_data)
    rejected_finding = _finding(intent_fail_id, intent_failure_data["policy_id"], disposition="rejected",
                                owner_phase=None, task_ids=[], status="recorded")
    accepted_finding = _finding(implementation_fail_id, implementation_failure_data["policy_id"],
                                disposition="accepted", owner_phase="implementation",
                                task_ids=[task_id], status="unresolved")
    intent_ledger_data = {
        "schema_version":"1","gate":"intent-review","subject":"intent.json",
        "subject_revision":intent["revision"],"author":{"name":"question-driver","kind":"script"},
        "findings":[rejected_finding]
    }
    intent_ledger_id = "question-intent-ledger"
    _append_record(journey, journey.run_id, "intent-review", intent_ledger_id, "finding-ledger", intent_ledger_data)
    _append_record(journey, journey.run_id, "intent-review", "question-intent-ledger-round-2",
                   "finding-ledger", intent_ledger_data)
    implementation_ledger_data = {
        "schema_version":"1","gate":"implementation-review","subject":"implementation-report.json",
        "subject_revision":report_revision,"author":{"name":"question-driver","kind":"script"},
        "findings":[accepted_finding]
    }
    implementation_ledger_id = "question-implementation-ledger"
    _append_record(journey, journey.run_id, "intent-review", implementation_ledger_id,
                   "finding-ledger", implementation_ledger_data)
    driver_note_id = "question-driver-observation"
    _append_record(journey, journey.run_id, "intent-review", driver_note_id, "driver-note", {
        "observation":"The driver compared the before/after assertion and actual selected evidence.",
        "correction_owner":"implementation",
        "before_assertion":"The operator could not inspect the selected evidence.",
        "after_assertion":"The operator can inspect the selected evidence."
    })
    applicability_id = "question-applicability"
    _append_record(journey, journey.run_id, "intent-review", applicability_id, "evidence-applicability", {
        "origin":{"kind":"context-record","id":intent_fail_id},
        "target":{"subject":"intent.json","revision":intent["revision"],"checkpoint":None},
        "attesting_driver":{"name":"question-driver","kind":"script"},
        "reason":"The current selected assertion was compared with this original evidence."
    })
    failed = root / "selected-failing-command.py"
    failed.write_text("import sys; print('scripted selected prerequisite failed'); raise SystemExit(7)\n", encoding="utf-8")
    failure_process = subprocess.run([sys.executable, str(failed)], cwd=root, capture_output=True, timeout=15, check=False)
    (root / "selected-failing-command.stdout").write_bytes(failure_process.stdout)
    (root / "selected-failing-command.stderr").write_bytes(failure_process.stderr)
    if failure_process.returncode != 7:
        raise ValueError("execution-issue fixture did not preserve the actual nonzero process result")
    execution_issue_id = "question-execution-issue"
    _append_record(journey, journey.run_id, "intent-review", execution_issue_id, "execution-issue", {
        "argv":[sys.executable,str(failed)],"exit_code":failure_process.returncode,
        "stdout":failure_process.stdout.decode(errors="replace"),
        "stderr":failure_process.stderr.decode(errors="replace"),
        "capture_sha256":"sha256:" + hashlib.sha256(failure_process.stdout + failure_process.stderr).hexdigest(),
        "observation":"The selected command actually exited 7; the isolated fixture has no live owned work."
    })
    check = root / "selected-proof.py"
    check.write_text(
        "import json,sys; d=json.load(open(sys.argv[1])); assert any(x.get('id')=='" + intent["acceptance"][0]["id"] + "' for x in d['acceptance']); print('assertion: current criterion " + intent["acceptance"][0]["id"] + " is present in the selected source')\n",
        encoding="utf-8",
    )
    proof_run = subprocess.run([sys.executable, str(check), str(intent_path)], cwd=root, capture_output=True, check=False)
    if proof_run.returncode != 0 or b"assertion:" not in proof_run.stdout:
        raise ValueError("selected current proof command did not emit its actual distinguishing assertion")
    assertion = proof_run.stdout.decode().strip()
    check_id = "advice-check"
    _append_record(journey, journey.run_id, "intent-review", check_id, "command-evidence", {
        "proof_id":check_id,"assertion":assertion,
        "spec":{"id":check_id,"command":sys.executable,"args":[str(check),str(intent_path)],
                "owner":"scripted-driver","obligation":assertion},
        "capture":{"stdout":proof_run.stdout.decode(),"stderr":proof_run.stderr.decode(),
                   "exit_code":proof_run.returncode}
    })
    verdict_ids = []
    for index, criterion in enumerate(intent["acceptance"]):
        record_id = f"advice-criterion-{index}"
        verdict_ids.append(record_id)
        _append_record(journey, journey.run_id, "intent-review", record_id, "criterion-verdict", {
            "criterion_id":criterion["id"],"result":"pass",
            "reason":"The scripted public check inspected this current acceptance source.",
            "evidence_context_ids":[check_id]
        })
    goal_id = "advice-goal"
    _append_record(journey, journey.run_id, "intent-review", goal_id, "goal-verdict", {
        "result":"pass","reason":"The scripted public check inspected the supplied whole-goal source.",
        "evidence_context_ids":[check_id]
    })
    # The helper prepares from a real full-show projection. It receives only
    # explicitly selected IDs and fixed artifact names; no repository search.
    full = journey._show_for(journey.run_id, state="intent-review", event="p11-question-full-show")
    departure_rows = full["initial_input"]["advice_departures"]["occasions"]
    def occasion(family: str) -> str:
        return next(row["occasion_id"] for row in departure_rows if row["occasion_id"].startswith(family + ":"))
    cases = {
        "review-candidates": ([intent_fail_id], ["intent.json"], None),
        "accepted-defect": ([implementation_ledger_id], ["implementation-report.json"], None),
        "implementation-correction": ([implementation_ledger_id, driver_note_id], ["plan.json","implementation-report.json"], None),
        "execution-or-authority-issue": ([execution_issue_id], [], None),
        "evidence-applicability": ([applicability_id, driver_note_id], ["intent.json"], None),
        "requirements-reconciliation": ([driver_note_id], ["reconciliation.json","intent.json"],
            [{"path":str(prd),"sha256":"sha256:" + hashlib.sha256(prd.read_bytes()).hexdigest()}]),
        "review-round-departure": ([intent_fail_id,intent_ledger_id,"question-intent-ledger-round-2"], [], None),
        "final-completion": ([check_id,*verdict_ids,goal_id], ["intent.json","plan.json","validation-report.json"], None)
    }
    requests: dict[str, dict[str, Any]] = {}
    for family, (ids, artifacts, documents) in cases.items():
        packet_show = journey._show_for(journey.run_id, state="intent-review", event=f"p11-selected-{family}")
        request = _provider_request(journey, root, packet_show, occasion(family), ids, artifacts, documents)
        requests[family] = request
        _write_json(root / f"prepared-{family}.json", request)
    if not {"support", "materiality", "scope"}.issubset(
        {key.rsplit(".",1)[-1] for key in requests["review-candidates"]["questions"]}
    ):
        raise ValueError("finding advice did not keep support/materiality/scope separate")
    for prepared in requests.values():
        if set(prepared["questions"]) != set(prepared["state"]["agent_judgments"]):
            raise ValueError("provider added unrequested broad questions")
        if not prepared["state"]["selected_evidence"]:
            raise ValueError("tool-selected evidence was replaced with driver-only claims")
        for question in prepared["questions"].values():
            if set(question.get("criteria", {})) != {"supported", "contradicted", "not-established"}:
                raise ValueError("provider emitted a workflow decision rather than bounded claim support")
    if "LE-1: Selected operator outcome" not in json.dumps(requests["requirements-reconciliation"]):
        raise ValueError("reconciliation question did not include the exact selected authoritative excerpt")
    for row in intent["acceptance"]:
        if f"criterion.{row['id']}.fulfillment" not in requests["final-completion"]["questions"] or f"criterion.{row['id']}.checks-could-miss" not in requests["final-completion"]["questions"]:
            raise ValueError(f"final advice omitted current AC/check-gap pair for {row['id']}")
    if not {"goal.fulfillment","goal.checks-could-miss"}.issubset(requests["final-completion"]["questions"]):
        raise ValueError("final advice omitted whole-goal and check-gap questions")
    if "assertion: current criterion" not in json.dumps(requests["final-completion"]):
        raise ValueError("final advice omitted the actual selected check assertion")

    # Dependent routing refuses before the driver supplies actual owner triage.
    missing_owner = copy.deepcopy(requests["implementation-correction"])
    no_owner_packet = {"show":{"status":"completed","result":full},
        "admissibility":{"bounded_judgment":True,"evidence_sufficient":True},
        "judgments":requests["implementation-correction"]["state"]["agent_judgments"],
        "occasion_id":occasion("implementation-correction"),
        "source_context_ids":[implementation_ledger_id],"artifact_names":["plan.json","implementation-report.json"]}
    _, refused = _capture(root, [str(journey.provider),"advice-request"], input_value=no_owner_packet, expected_code=2)
    if b"explicit driver correction-owner triage" not in refused.stderr:
        raise ValueError("implementation routing did not wait for actual owner triage")
    no_accepted_packet = {"show":{"status":"completed","result":full},
        "admissibility":{"bounded_judgment":True,"evidence_sufficient":True},
        "judgments":requests["accepted-defect"]["state"]["agent_judgments"],
        "occasion_id":occasion("accepted-defect"),"source_context_ids":[intent_fail_id],"artifact_names":["intent.json"]}
    _, refused = _capture(root, [str(journey.provider),"advice-request"], input_value=no_accepted_packet, expected_code=2)
    if b"driver-triaged accepted unresolved finding" not in refused.stderr:
        raise ValueError("correction ownership question did not wait for real driver finding triage")

    # Scripted correct and confidently wrong answers are captured separately;
    # each answer, including the superseded response, receives a reasoned disposition.
    correct_f = _advise(journey, root, journey.run_id, requests["review-candidates"], "correct-with-finding")
    wrong_f = _advise(journey, root, journey.run_id, requests["review-candidates"], "wrong-finding")
    wrong_materiality = wrong_f["response"]["answers"]["finding." + intent_fail_id + ".materiality"]
    if wrong_materiality["confidence"] < 0.99 or wrong_materiality["choice"] != "contradicted":
        raise ValueError("scripted wrong finding advice was not confidently wrong for the rejected nonmaterial fixture")
    _dispositions(journey, journey.run_id, journey.state, correct_f, status="partial",
                  reason="The driver considered but did not delegate the finding judgment.")
    _dispositions(journey, journey.run_id, journey.state, wrong_f, status="reject",
                  reason="The high-confidence materiality advice contradicts the supplied rejected finding and is rejected normally.")
    correct_w = _advise(journey, root, journey.run_id, requests["final-completion"], "correct")
    wrong_w = _advise(journey, root, journey.run_id, requests["final-completion"], "wrong-high-confidence")
    bad_goal = wrong_w["response"]["answers"]["criterion." + intent["acceptance"][0]["id"] + ".fulfillment"]
    if bad_goal["choice"] != "contradicted" or bad_goal["confidence"] < 0.99:
        raise ValueError("scripted final response did not provide the confidently wrong negative")
    _dispositions(journey, journey.run_id, journey.state, correct_w, status="accept",
                  reason="The driver compared this answer with the selected passing assertion.")
    _dispositions(journey, journey.run_id, journey.state, wrong_w, status="reject",
                  reason="The actual selected check passed; this confidently wrong advice is rejected without an exception.")
    ad_hoc = {
        "version":1,"state":{"admissibility":{"bounded_judgment":True,"evidence_sufficient":True},"selected_source":"driver-note/question-driver-observation"},
        "target":{"run_id":journey.run_id,"state":"intent-review","state_visit":full["state_visit"],"source_context_ids":[driver_note_id]},
        "occasion":"ad-hoc:p11-active-operator-question",
        "questions":{"operator-note":{"type":"noul","instructions":"Is this selected note relevant to the operator's current question?",
                                       "proposition":"The selected driver note states the current operator observation."}}
    }
    adhoc_response = _advise(journey, root, journey.run_id, ad_hoc, "correct")
    _dispositions(journey, journey.run_id, journey.state, adhoc_response, status="partial",
                  reason="The driver dispositioned the ad-hoc answer while the run remained ACTIVE.")
    timeout_request = {**ad_hoc,"occasion":"ad-hoc:p11-timeout","state":{**ad_hoc["state"],"fixture_response_mode":"timeout","selected_source":"timeout fixture"}}
    timeout_path = root / "timeout-request.json"
    _write_json(timeout_path, timeout_request)
    timeout_result = journey._engine_for(journey.run_id,["advise",journey.run_id,f"@{timeout_path}"],state=journey.state,event="advise-timeout")
    if timeout_result.get("status") not in {"error","rejected"} or timeout_result.get("code") != "advice-command-failed":
        raise ValueError(f"configured software-change advice timeout did not remain unanswered: {timeout_result!r}")
    after = journey._show_for(journey.run_id,state=journey.state,event="p11-advice-after-dispositions")
    ledgers = [row for row in after["context"] if row["kind"]=="finding-ledger"]
    if len(ledgers) != 3 or ledgers[0]["data"] != intent_ledger_data:
        raise ValueError("advice answers mutated the driver-owned finding ledger")
    successful = [row for row in after["context"] if row["kind"]=="advice-attempt" and row["data"].get("status")=="completed"]
    disposition_keys = {(row["data"].get("response_id"),row["data"].get("answer_id"))
                        for row in after["context"] if row["kind"]=="advice-disposition"}
    for result in successful:
        attempt_id = result["id"]
        answer_ids = result["data"]["typed_result"]["answers"].keys()
        if any((attempt_id,answer_id) not in disposition_keys for answer_id in answer_ids):
            raise ValueError(f"successful or superseded advice answer was left undispositioned: {attempt_id}")
    timeout_rows = [row for row in after["context"] if row["kind"]=="advice-attempt" and row["data"].get("status")=="failed"]
    if not timeout_rows or not any(row["data"].get("timed_out") for row in timeout_rows):
        raise ValueError("timed-out scripted advice was not retained as an unanswered attempt")
    (root / "question-family-proof.json").write_text(json.dumps({
        "families":{family:sorted(request["questions"]) for family,request in requests.items()},
        "correct_wrong_and_adhoc_attempts":[correct_f["attempt_id"],wrong_f["attempt_id"],correct_w["attempt_id"],wrong_w["attempt_id"],adhoc_response["attempt_id"]],
        "timeout_code":timeout_result.get("code"),"ledger_ids":[row["id"] for row in ledgers],
        "selected_assertion":assertion,"answer_dispositions":len(disposition_keys),
        "synthetic_semantic_quality_claim":False
    },indent=2)+"\n",encoding="utf-8")


def _prepare_real_phase_coverage(journey, run_root: Path, checkout: Path) -> None:
    """Explain this completed fixture's *actual* live scope in its phase artifacts.

    The calibration inputs remain untouched. This fixture's accepted AC spine is
    linked to the committed requirements that its scripted operator paths can
    actually expose, not the unrelated LE-1/LE-2 placeholder links.
    """
    artifacts = run_root / "artifacts"
    intent_path = artifacts / "intent.json"
    intent = json.loads(intent_path.read_text())
    live = {"AC-1":"LE-132", "AC-2":"LE-137", "AC-4":"LE-116"}
    for criterion in intent["acceptance"]:
        if criterion["id"] in live:
            criterion["prd_traceability"] = {"type":"linked-live","live_ids":[live[criterion["id"]]]}
    intent["operating_context"]["outside_obligations"].extend([
        {"source":"docs/PRD.md#LE-161",
         "obligation":"Account for accepted in-scope requirements through intent, design, plan, implementation and validation in each phase's own terms, with named authoritative obligations and real public GREEN."},
        {"source":"docs/PRD.md#LE-97 and docs/PRD.md#LE-134",
         "obligation":"Distinguish actual public assertions from schema, ID or exit-only activity; real eligibility and Bookends GREEN are separate."},
        {"source":"docs/PRD.md#LE-116 and crates/software-change-provider/docs/prd.md#11",
         "obligation":"Keep the fixed AC-N/goal command index and independent evidence at one current checkpoint."},
        {"source":"crates/software-change-provider/data/reviewer-protocol.md",
         "obligation":"Scripted reviewer output is candidate evidence, not semantic approval."},
    ])
    _write_json(intent_path,intent)

    design_path = artifacts / "design.json"
    design = json.loads(design_path.read_text())
    design["author"] = {"name":"scripted-bookends-design","kind":"script"}
    design["approach"] = (
        "Use real public setup, malformed-subject denial, source-linked review and revised-plan "
        "completion commands for the four accepted fixture criteria. Keep Bookends-enabled phase "
        "artifacts on their existing AC/command/verdict spine; retain actual bytes for external inspection."
    )
    design["elements"] = [{"name":"selected public proof paths",
        "responsibility":"Deliver distinct captured operator observations for AC-1..AC-4; a scripted verdict is not semantic approval."}]
    design["decisions"] = [{"choice":"Use named public command captures and the fixed validation index",
        "rationale":"LE-116 and LE-161 need actual selected sources and phase-appropriate explanations, not a second ID ledger or local phrase classifier."}]
    design["risks"] = [{"risk":"A schema-valid synthetic PASS could be mistaken for semantic coverage",
        "mitigation":"Retain exact artifact and command bytes, distinct criterion citations and explicit semantic limits for independent inspection."}]
    descriptions = {
        "AC-1":"LE-132: keep selected setup policy and immutable initial_input visible before transitions; the setup and shown frozen bytes, not a describe token, establish the operator observation.",
        "AC-2":"LE-137: deterministic checked evaluation reports each malformed subject path/rule without letting the script decide semantic sufficiency; the public negative must be inspected separately.",
        "AC-3":"Change-specific independence and complete configured axes: sol-evidence checks stale, subject-author, duplicate-author and missing-axis real high-rigor gate refusals with a valid ledger, then repairs each before checked progress. No unrelated PRD ID supplies this criterion's meaning.",
        "AC-4":"LE-116 and LE-161: sol-reuse attempts the terminal edge with one named configured validation-adversarial axis missing and a valid ledger/index, then repairs it and completes; captured commands, independent AC/goal records and the current implementation checkpoint supply the fifth phase's links. docs/PRD.md#LE-97/134 still require public assertion inspection.",
    }
    for row, criterion in zip(design["coverage"],intent["acceptance"]):
        row["criterion_id"] = criterion["id"]
        row["delivered_by"] = descriptions[criterion["id"]]
    _write_json(design_path,design)

    plan_path = artifacts / "plan.json"
    plan = json.loads(plan_path.read_text())
    plan["author"] = {"name":"scripted-bookends-plan","kind":"script"}
    plan["objective"] = (
        "Capture four distinguishing public paths and retain five actual Bookends-enabled phase "
        "artifacts with the fixed current-criterion and whole-goal validation index."
    )
    subjects = {
        "AC-1":("phase-setup","LE-132","selected copied profile bytes, effective policy and frozen floor"),
        "AC-2":("phase-schema","LE-137","two required-field denials from a real malformed intent and unchanged state"),
        "AC-3":("phase-evidence",None,"high-rigor stale, self-authored, duplicate-author and missing-axis checked refusals with valid ledger and repaired gate"),
        "AC-4":("phase-completion","LE-116","missing configured validation-axis terminal refusal, repair, checked completion and distinct criterion/goal evidence"),
    }
    plan["tasks"] = [{
        "id":task,"objective":f"Exercise the {criterion} operator path through a selected public command.",
        "dependencies":[],"source_of_truth":["intent.json#/acceptance/"+str(index),
            "design.json#/coverage/"+str(index),
            *([f"docs/PRD.md#{requirement}"] if requirement else
              ["crates/software-change-provider/data/configs/high-rigor.json#/review_policies/intent-review"]),
            "docs/PRD.md#LE-161","crates/software-change-provider/docs/prd.md#11"],
        "deliverables":[f"Retained {observation} with actual stdout/stderr and command identity."],
        "out_of_scope":["Scripted pass and exit zero are not semantic approval; an independent actor inspects exact assertions."],
        "validation":[f"The selected public command asserts {observation}; expose its negative or an explicit limitation rather than matching an ID."],
        "handoff":"The current artifact and captured assertion are read by validation; verdicts name only this selected source.",
        "criterion_ids":[criterion],"proof_command_ids":["phase-"+criterion.lower()],
    } for index,(criterion,(task,requirement,observation)) in enumerate(subjects.items())]
    plan["dependency_graph"] = []
    plan["proof_commands"] = list(plan["proof_commands"])
    proof_cases = {
        "AC-1":"sol-profiles", "AC-3":"sol-evidence", "AC-4":"sol-reuse",
    }
    for criterion_id, scenario in proof_cases.items():
        command_id = "phase-" + criterion_id.lower()
        plan["proof_commands"].append({
            "id":command_id,"command":sys.executable,
            "args":[str(checkout / "scripts/software-change-journey.py"),"--mode","source",
                    "--engine",str(journey.engine),"--provider",str(journey.provider),
                    "--data-root",str(checkout),"--work-root",str(run_root / f"proof-{criterion_id.lower()}"),
                    "--profile","crates/software-change-provider/data/configs/high-rigor.json",
                    "--scenario",scenario],
            "owner":"fixture-proof-owner",
            "obligation":f"AC {criterion_id}: execute the {scenario} public CLI path and inspect its retained distinct assertions and negatives.",
        })
    denial_script = run_root / "phase-ac2-schema-denial.py"
    denial_script.write_text(
        "import json,pathlib,subprocess,sys\n"
        "engine,provider,profile_path,fixture_path,root=map(pathlib.Path,sys.argv[1:])\n"
        "root.mkdir(); artifacts=root/'artifacts'; artifacts.mkdir()\n"
        "profile=json.loads(profile_path.read_text()); profile['artifact_root']=str(artifacts); profile['review_policies']={}\n"
        "(root/'profile.json').write_text(json.dumps(profile))\n"
        "(root/'providers.toml').write_text('[providers.software-change]\\ncommand = '+json.dumps(str(provider))+'\\nargs = []\\n')\n"
        "db=root/'loop.sqlite'; run='phase-ac2-malformed-subject'\n"
        "def call(*args):\n"
        " p=subprocess.run([str(engine),'--database',str(db),'--json',*args],cwd=root,capture_output=True,text=True,check=False)\n"
        " v=json.loads(p.stdout); return p,v\n"
        "p,v=call('--config',str(root/'providers.toml'),'start','--id',run,'software-change','@'+str(root/'profile.json'))\n"
        "assert p.returncode==0 and v['status']=='completed',v\n"
        "intent=json.loads(fixture_path.read_text()); intent.pop('revision'); intent.pop('problem')\n"
        "(artifacts/'intent.json').write_text(json.dumps(intent))\n"
        "p,v=call('show','--view','action',run); assert p.returncode==0 and v['status']=='completed',v\n"
        "p,v=call('event',run,'intent-ready'); violations=v.get('details',{}).get('violations',[])\n"
        "missing={x['message'] for x in violations if x.get('path')=='' and x.get('rule')=='required'}\n"
        "assert v.get('status')=='rejected' and v.get('code')=='software-change-schema-invalid' and {'required property `revision` is missing','required property `problem` is missing'}<=missing,v\n"
        "p,after=call('show','--view','full',run); assert p.returncode==0 and after['result']['current_state']=='explore',after\n"
        "print('assertion: real malformed intent rejects both absent required fields without transition; '+json.dumps(sorted(missing)))\n",
        encoding="utf-8",
    )
    plan["proof_commands"].append({"id":"phase-ac-2","command":sys.executable,
        "args":[str(denial_script),str(journey.engine),str(journey.provider),
                str(checkout / "crates/software-change-provider/data/configs/minimal.json"),
                str(journey.data_root / "crates/software-change-provider/data/calibration/fixtures/intent-good.json"),
                str(run_root / "proof-ac2")],
        "owner":"fixture-proof-owner",
        "obligation":"AC-2: public start/show/event refuses two malformed intent fields with distinct paths and unchanged state."})
    for task in plan["tasks"]:
        for criterion_id in task.get("criterion_ids",[]):
            proof_id = "phase-" + criterion_id.lower()
            if proof_id not in task.get("proof_command_ids",[]):
                task.setdefault("proof_command_ids",[]).append(proof_id)
    _write_json(plan_path,plan)

    report_path = artifacts / "implementation-report.json"
    report = json.loads(report_path.read_text())
    report["author"] = {"name":"scripted-bookends-implementation","kind":"script"}
    documents = ("docs/PRD.md","crates/software-change-provider/docs/prd.md",
                 "crates/software-change-provider/data/reviewer-protocol.md",
                 "scripts/software-change-journey.py")
    head = subprocess.run(["git","rev-parse","HEAD"],cwd=checkout,capture_output=True,text=True,check=False)
    if head.returncode != 0:
        raise ValueError(f"fixture implementation report has no actual Git HEAD: {head.stderr}")
    report["coverage"] = {"commit":head.stdout.strip(),"documents":[
        {"path":name,"revision":"sha256:"+hashlib.sha256((checkout/name).read_bytes()).hexdigest()}
        for name in documents]}
    report["changed_surface"] = ["scripted external fixture artifacts and captures; no checkout source edit"]
    report["summary"] = (
        "Scripted Bookends-enabled fixture, not a claim of implementing the old calibration's fictional repository. "
        "Its selected live LE-132/137/116 scope uses the existing AC-N spine, actual public command captures, "
        "and the provider PRD/reviewer protocol obligations under LE-161; semantic coverage and advice quality remain external judgments."
    )
    report["validation"] = [
        {"criterion_id":"AC-1","proof":"phase-ac-1: sol-profiles checks selected bytes/effective policy and frozen floor through setup/start/show (LE-132)."},
        {"criterion_id":"AC-2","proof":"phase-ac-2: public start/show/event denies two malformed intent fields and retains explore (LE-137)."},
        {"criterion_id":"AC-3","proof":"phase-ac-3: sol-evidence checks four ledger-present high-rigor independence/axis deficits, their specific checked refusals, repair and approval (change-specific)."},
        {"criterion_id":"AC-4","proof":"phase-ac-4: sol-reuse requests a real terminal edge missing one configured validation-adversarial axis, retains its denial and then completes after the missing review is appended (LE-116)."},
    ]
    _write_json(report_path,report)


def _prepare_phase_validation(journey, run_id: str, checkout: Path) -> dict[str, Any]:
    """Capture actual named public checks, then give each synthetic AC its own source."""
    helper = Path(__file__).resolve().parents[1] / "tests/fixtures/prepare-validation.py"
    spec = importlib.util.spec_from_file_location("phase_validation_fixture",helper)
    if spec is None or spec.loader is None:
        raise ValueError("validation capture fixture could not be loaded")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    shown = journey._engine(["show","--view","full",run_id],state="validation",event="phase-validation-show")
    journey._expect_status(shown,"completed",event="show",state="validation")
    prepared = module.prepare(journey.provider,journey.engine,checkout,shown,"p11-validation-r1")
    evidence = {row["data"]["proof_id"]:row["record_id"] for row in prepared["records"]
                if row["kind"] == "command-evidence"}
    for criterion_id in ("AC-1","AC-2","AC-3","AC-4"):
        if "phase-" + criterion_id.lower() not in evidence:
            raise ValueError(f"validation capture omitted the distinct {criterion_id} public proof")
    for row in prepared["records"]:
        if row["kind"] == "criterion-verdict":
            criterion = row["data"]["criterion_id"]
            command_id = "phase-" + criterion.lower()
            row["data"]["evidence_context_ids"] = [evidence[command_id]]
            row["data"]["reason"] = (
                f"Scripted fixture selected the retained {command_id} public command and its "
                "actual assertion/negative for this AC under LE-161's phase-specific coverage duty; separate independent semantic review remains required."
            )
        elif row["kind"] == "goal-verdict":
            row["data"]["evidence_context_ids"] = [evidence["phase-ac-1"],evidence["phase-ac-2"],
                                                       evidence["phase-ac-3"],evidence["phase-ac-4"]]
            row["data"]["reason"] = (
                "Scripted fixture selected all four distinct captured public paths; "
                "the final Bookends-enabled goal and LE-161's five-phase semantic sufficiency remain external."
            )
        _append_record(journey,run_id,"validation",row["record_id"],row["kind"],row["data"])
    return prepared


def _retain_phase_inspection(journey, run_root: Path, final: dict[str, Any], prd_identity: dict[str, Any]) -> dict[str, Any]:
    """Retain exact resulting bytes and phase-specific facts for human review."""
    artifacts = run_root / "artifacts"
    names = ("intent","design","plan","implementation-report","validation-report")
    bytes_by_phase = {name:(artifacts / f"{name}.json").read_bytes() for name in names}
    documents = {name:json.loads(raw) for name,raw in bytes_by_phase.items()}
    acceptance = {row["id"]:row for row in documents["intent"]["acceptance"]}
    explanations = {
        "intent": {"acceptance":documents["intent"]["acceptance"],
                   "outside_obligations":documents["intent"]["operating_context"]["outside_obligations"]},
        "design": {"coverage":documents["design"]["coverage"]},
        "plan": {"tasks":[{"id":row["id"],"criterion_ids":row.get("criterion_ids"),
                             "source_of_truth":row["source_of_truth"],"validation":row["validation"],
                             "out_of_scope":row["out_of_scope"]} for row in documents["plan"]["tasks"]],
                 "proof_commands":documents["plan"]["proof_commands"]},
        "implementation-report": {"coverage":documents["implementation-report"]["coverage"],
                                  "summary":documents["implementation-report"]["summary"],
                                  "validation":documents["implementation-report"]["validation"]},
        "validation-report": {"fixed_index":documents["validation-report"]},
    }
    index = documents["validation-report"]
    context = {row["id"]:row for row in final["context"]}
    commands = [context[record_id] for record_id in index["command_evidence_ids"]]
    verdicts = {row["criterion_id"]:[context[record_id] for record_id in row["verdict_ids"]]
                for row in index["criteria"]}
    goals = [context[record_id] for record_id in index["goal_verdict_ids"]]
    if (set(acceptance) != set(verdicts)
            or any(record["kind"] != "command-evidence" for record in commands)
            or any(record["kind"] != "criterion-verdict" for rows in verdicts.values() for record in rows)
            or any(record["kind"] != "goal-verdict" for record in goals)):
        raise ValueError("completed five-artifact inspection lost actual AC, command, or goal sources")
    if {row["criterion_id"] for row in documents["design"]["coverage"]} != set(acceptance):
        raise ValueError("actual design omitted a current AC coverage relationship")
    if not all(any(row["criterion_id"] in task.get("criterion_ids",[]) for task in documents["plan"]["tasks"])
                   for row in index["criteria"]):
        raise ValueError("actual plan omitted an AC task relationship")
    if {row["criterion_id"] for row in documents["implementation-report"]["validation"]
            if "criterion_id" in row} != set(acceptance):
        raise ValueError("actual implementation report omitted a current AC proof relationship")
    inspection = {
        "kind":"completed-five-phase-artifact-inspection-v1", "run_id":final["run_id"],
        "terminal_state":final["current_state"],"lifecycle":final["lifecycle"],
        "normative_source":prd_identity,
        "normative_obligations": ["docs/PRD.md#LE-161","docs/PRD.md#LE-132","docs/PRD.md#LE-137",
            "docs/PRD.md#LE-116","docs/PRD.md#LE-97","docs/PRD.md#LE-134",
            "crates/software-change-provider/docs/prd.md#11",
            "crates/software-change-provider/data/reviewer-protocol.md"],
        "artifacts":{name:{"path":str(artifacts / f"{name}.json"),
                           "sha256":"sha256:"+hashlib.sha256(raw).hexdigest(),
                           "byte_length":len(raw),"phase_explanation":explanations[name]}
                     for name,raw in bytes_by_phase.items()},
        "validation_sources":{"command_evidence":commands,"criterion_verdicts":verdicts,"goal_verdicts":goals},
        "semantic_limit":"Exact real artifact bytes and source relationships are retained for independent judgment; structural relationships and scripted PASS do not prove semantic coverage or advice usefulness. The selected command negatives exercise provider mechanics, not independent semantic quality.",
    }
    path = run_root / "phase-coverage-inspection.json"
    _write_json(path,inspection)
    return {"path":str(path),"sha256":"sha256:"+hashlib.sha256(path.read_bytes()).hexdigest(),
            "artifact_sha256":{name:inspection["artifacts"][name]["sha256"] for name in names}}


def _completed_bookends_run(
    journey, root: Path, setup_results: dict[str, dict[str, Any]], advice_config: dict[str, Any]
) -> dict[str, Any]:
    """Complete a new high-rigor Bookends-enabled run through ordinary gates."""
    run_root = root / "future-high-rigor-bookends"
    run_root.mkdir()
    checkout = run_root / "checkout"
    shutil.copytree(
        journey.data_root,
        checkout,
        ignore=shutil.ignore_patterns(".git", "target", "__pycache__", "*.pyc", ".pi-subagents", ".loop-engine"),
    )
    # The fixture creates a disposable Git baseline for Bookends. Use the
    # repository's actual committed PRD bytes so an owner-accepted but
    # uncommitted proposal in the working tree cannot become synthetic live
    # policy merely because this fixture commits its copied files.
    committed_prd = subprocess.run(
        ["git", "show", "HEAD:docs/PRD.md"],
        cwd=journey.data_root,
        capture_output=True,
        check=False,
    )
    if committed_prd.returncode != 0:
        raise ValueError(
            "Bookends fixture could not read committed HEAD:docs/PRD.md: "
            + committed_prd.stderr.decode("utf-8", "replace")
        )
    source_head = subprocess.run(
        ["git", "rev-parse", "HEAD"],
        cwd=journey.data_root,
        text=True,
        capture_output=True,
        check=False,
    )
    if source_head.returncode != 0:
        raise ValueError("Bookends fixture could not identify the source HEAD commit")
    fixture_prd = checkout / "docs" / "PRD.md"
    fixture_prd.write_bytes(committed_prd.stdout)
    if fixture_prd.read_bytes() != committed_prd.stdout:
        raise ValueError("Bookends fixture PRD differs from committed source bytes")
    committed_prd_identity = {
        "source_head": source_head.stdout.strip(),
        "source": "git show HEAD:docs/PRD.md",
        "byte_length": len(committed_prd.stdout),
        "sha256": "sha256:" + hashlib.sha256(committed_prd.stdout).hexdigest(),
    }
    journey._initialize_overlay_checkout(checkout)
    artifacts = run_root / "artifacts"
    artifacts.mkdir()
    profile = json.loads((journey.data_root / "crates/software-change-provider/data/configs/high-rigor.json").read_text())
    profile.pop("work_slot_bindings", None)  # scripted external reviewers use the supported unbound path
    profile["artifact_root"] = str(artifacts)
    profile["advice_command"] = advice_config
    profile["advice_departures"] = setup_results["high-rigor"]["configured_profile"]["advice_departures"]
    profile["extra"] = copy.deepcopy(profile.get("extra", {}))
    profile["extra"]["bookends"] = {"enabled": True}

    journey.run_dir = run_root
    journey.database = run_root / "loop.sqlite"
    journey.provider_config = run_root / "providers.toml"
    journey.profile_path = run_root / "high-rigor-bookends.json"
    journey.profile_source = journey.data_root / "crates/software-change-provider/data/configs/high-rigor.json"
    journey.artifact_root = artifacts
    journey.repository_root = checkout
    journey.command_cwd = checkout
    journey.command_env = {"BOOKENDS_BYPASS": ""}
    journey.profile = profile
    journey.work_slot_bindings = {}
    journey.run_id = "sol-advice-bookends-completed"
    journey.state = "not-started"
    journey._p11_sources = {}
    journey._p11_trigger_families = {
        ("intent-review", "approved", "review-candidates"),
        ("intent-review", "approved", "evidence-applicability"),
        ("intent-review", "approved", "review-round-departure"),
        ("implement", "implementation-ready", "execution-or-authority-issue"),
        ("reconciliation", "reconciliation-ready", "requirements-reconciliation"),
        ("validation-review", "approved", "review-round-departure"),
        ("validation-adversarial-review", "passed", "final-completion"),
    }
    _write_json(journey.profile_path, profile)
    journey._write_provider_config_at(journey.provider_config)
    journey._write_overlay_artifacts(artifacts, candidate=False, unfulfilled=False)
    journey._assert_overlay_artifacts(artifacts, candidate=False, unfulfilled=False)
    _prepare_real_phase_coverage(journey, run_root, checkout)

    original_expect = journey._expect_allow

    def expect_with_advice(event: str, target: str):
        _close_departure(journey, root, journey.run_id, event, target)
        result = original_expect(event, target)
        journey._assert_show(target, f"p11-after-{event}")
        return result

    journey._expect_allow = expect_with_advice
    journey._start()
    journey._assert_show("explore", "p11-enabled-start")
    failed_command = root / "completed-run-execution-issue.py"
    failed_command.write_text("import sys; print('known isolated failure'); raise SystemExit(7)\n", encoding="utf-8")
    failed_process = subprocess.run([sys.executable,str(failed_command)],cwd=root,capture_output=True,check=False)
    if failed_process.returncode != 7:
        raise ValueError("completed-run execution/authority source did not retain a real nonzero exit")
    journey._p11_sources["execution_issue"] = "p11-completed-run-execution-issue"
    _append_record(journey,journey.run_id,"explore",journey._p11_sources["execution_issue"],"execution-issue",{
        "argv":[sys.executable,str(failed_command)],"exit_code":failed_process.returncode,
        "stdout":failed_process.stdout.decode(errors="replace"),"stderr":failed_process.stderr.decode(errors="replace"),
        "capture_sha256":"sha256:"+hashlib.sha256(failed_process.stdout+failed_process.stderr).hexdigest(),
        "observation":"This scripted prerequisite failed, has no owned process, and does not authorize a retry."
    })

    # ACTIVE ad-hoc advice remains available; a timeout is retained but has no
    # answer or primary-work effect.
    ad_hoc_source = _add_driver_note(journey, journey.run_id, "explore", "p11-adhoc-source", {
        "observation":"The operator selected this exact active-run note for an ad-hoc question."
    })
    ad_hoc_show = journey._show_for(journey.run_id, state="explore", event="p11-adhoc-show")
    ad_hoc_request = {
        "version":1,"state":{"admissibility":{"bounded_judgment":True,"evidence_sufficient":True},"selected_source_id":ad_hoc_source},
        "target":{"run_id":journey.run_id,"state":"explore","state_visit":ad_hoc_show["state_visit"],
                  "source_context_ids":[ad_hoc_source]},
        "occasion":"ad-hoc:p11-active-driver-question",
        "questions":{"active-note":{"type":"noul","instructions":"Judge only the supplied active-run note.",
                                      "proposition":"The selected note records the current operator observation."}}
    }
    ad_hoc_answer = _advise(journey, root, journey.run_id, ad_hoc_request, "correct")
    _dispositions(journey, journey.run_id, "explore", ad_hoc_answer, status="partial",
                  reason="The driver recorded a partial disposition for this ad-hoc answer.")
    timeout_request = copy.deepcopy(ad_hoc_request)
    timeout_request["occasion"] = "ad-hoc:p11-timeout"
    timeout_request["state"]["fixture_response_mode"] = "timeout"
    timeout_path = root / "p11-timeout-request.json"
    _write_json(timeout_path, timeout_request)
    timed_out = journey._engine_for(journey.run_id,["advise",journey.run_id,f"@{timeout_path}"],
                                    state="explore",event="timeout")
    if timed_out.get("code") != "advice-command-failed" or not timed_out.get("details",{}).get("timed_out"):
        raise ValueError(f"required advice timeout did not remain an unanswered captured attempt: {timed_out!r}")
    journey._assert_show("explore", "p11-after-timeout")

    journey._expect_allow("intent-ready", "intent-review")
    intent_result = _append_review_gate(
        journey, journey.run_id, "intent-review", failing=("acceptance-granularity", "reviewer-0")
    )
    failure = intent_result["failure"]
    if failure is None:
        raise ValueError("scripted intent reviewer did not emit the planned selected candidate")
    journey._p11_sources["intent_failure"] = failure["id"]
    rejected = _finding(failure["id"], failure["data"]["policy_id"], disposition="rejected",
                        owner_phase=None, task_ids=[], status="recorded")
    intent_ledger = _append_ledger(
        journey, journey.run_id, "intent-review", "intent.json", intent_result["revision"],
        finding=rejected, record_id="p11-intent-ledger",
    )
    journey._p11_sources["intent_ledger"] = intent_ledger
    applicability_source = next(record_id for record_id in intent_result["evidence_ids"] if record_id != failure["id"])
    journey._p11_sources["applicability"] = "p11-intent-applicability"
    _append_record(journey,journey.run_id,"intent-review",journey._p11_sources["applicability"],"evidence-applicability",{
        "origin":{"kind":"context-record","id":applicability_source},
        "target":{"subject":"intent.json","revision":intent_result["revision"],"checkpoint":None},
        "attesting_driver":{"name":"scripted-driver","kind":"script"},
        "reason":"The driver compared this exact prior reviewer source with the current intent target."
    })
    journey._p11_sources["before-after"] = _add_driver_note(journey,journey.run_id,"intent-review","p11-before-after",{
        "observation":"The same selected assertion was compared at both targets.",
        "before_assertion":"The selected proof did not expose the source ID.",
        "after_assertion":"The selected proof exposes the source ID."
    })
    journey._expect_allow("approved", "intent-adversarial-review")
    for gate,event,target in [
        ("intent-adversarial-review","approved","design"),
    ]:
        _append_review_gate(journey,journey.run_id,gate)
        _append_ledger(journey,journey.run_id,gate,SUBJECT_BY_GATE[gate],journey._fixture_revision(SUBJECT_BY_GATE[gate]))
        journey._expect_allow(event,target)
    journey._expect_allow("design-ready", "design-review")
    for gate,event,target in [
        ("design-review","approved","design-adversarial-review"),
        ("design-adversarial-review","approved","plan"),
    ]:
        _append_review_gate(journey,journey.run_id,gate)
        _append_ledger(journey,journey.run_id,gate,SUBJECT_BY_GATE[gate],journey._fixture_revision(SUBJECT_BY_GATE[gate]))
        journey._expect_allow(event,target)
    journey._expect_allow("plan-ready", "plan-review")
    for gate,event,target in [
        ("plan-review","approved","plan-adversarial-review"),
        ("plan-adversarial-review","approved","implement"),
    ]:
        _append_review_gate(journey,journey.run_id,gate)
        _append_ledger(journey,journey.run_id,gate,SUBJECT_BY_GATE[gate],journey._fixture_revision(SUBJECT_BY_GATE[gate]))
        journey._expect_allow(event,target)

    failed_command = root / "owned-work-failure.py"
    failed_command.write_text("import sys; print('isolated cleanup fixture'); raise SystemExit(7)\n",encoding="utf-8")
    failed_process = subprocess.run([sys.executable,str(failed_command)],cwd=root,capture_output=True,check=False)
    if failed_process.returncode != 7:
        raise ValueError("execution/authority question source did not capture the intended nonzero child")
    journey._p11_sources["execution_issue"] = "p11-execution-issue"
    _append_record(journey,journey.run_id,"implement","p11-execution-issue","execution-issue",{
        "argv":[sys.executable,str(failed_command)],"exit_code":failed_process.returncode,
        "stdout":failed_process.stdout.decode(errors="replace"),"stderr":failed_process.stderr.decode(errors="replace"),
        "capture_sha256":"sha256:"+hashlib.sha256(failed_process.stdout+failed_process.stderr).hexdigest(),
        "observation":"The isolated child actually exited nonzero; no process remains owned."
    })
    journey._create_checkpoint("implementation")
    journey._expect_allow("implementation-ready", "reconciliation")
    citation = journey._committed_reconciliation_reference(checkout)
    recon = journey._write_reconciliation_result(
        revision="p11-reconciliation-r1", mode="bookends-enabled",
        branch="sufficient-existing-wording",
        document_observations=[{"path":"docs/PRD.md","status":"sufficient","observation":"The exact selected live requirement text was read."}],
        behavior_observations=[{"status":"matches-intent","observation":"The selected public assertion matches the current AC spine."}],
        action="no-document-change", action_reason="The selected accepted wording already covers this outcome.",
        authorization="not-required", application="not-required", commit="not-required",
        traceability={"status":"retained","references":[citation["reference"]]},
        proof_references=[citation["reference"],"journey:p11-advice-reconciliation"],
        blockers=[], decision="complete",
    )
    journey._p11_sources["reconciliation-observation"] = _add_driver_note(
        journey,journey.run_id,"reconciliation","p11-reconciliation-observation",{
            "observation":"The delivered behavior and actual PRD bytes were inspected before this reconciliation decision.",
            "branch":"sufficient-existing-wording"
        }
    )
    journey._expect_allow("reconciliation-ready", "implementation-review")
    journey._create_checkpoint("implementation")
    gate="implementation-review"
    result=_append_review_gate(journey,journey.run_id,gate)
    ledger=_append_ledger(journey,journey.run_id,gate,result["subject"],result["revision"])
    journey._p11_sources["implementation_ledger"] = ledger
    journey._expect_allow("approved","implementation-adversarial-review")
    gate="implementation-adversarial-review"
    result=_append_review_gate(journey,journey.run_id,gate)
    _append_ledger(journey,journey.run_id,gate,result["subject"],result["revision"])
    journey._expect_allow("approved","validation")

    validation = _prepare_phase_validation(journey,journey.run_id,checkout)
    if not validation.get("report",{}).get("command_evidence_ids") or not validation.get("records"):
        raise ValueError("future-run path omitted real proof captures, criterion records, or goal records")
    journey._expect_allow("validation-ready","validation-review")
    gate="validation-review"
    result=_append_review_gate(journey,journey.run_id,gate)
    _append_ledger(journey,journey.run_id,gate,result["subject"],result["revision"])
    journey._expect_allow("approved","validation-adversarial-review")
    gate="validation-adversarial-review"
    result=_append_review_gate(journey,journey.run_id,gate)
    _append_ledger(journey,journey.run_id,gate,result["subject"],result["revision"])
    journey._expect_allow("passed","end")
    final = journey._show_for(journey.run_id,state="end",event="p11-final-full-show")
    if final.get("current_state")!="end" or final.get("lifecycle")!="final" or final["initial_input"].get("extra",{}).get("bookends",{}).get("enabled") is not True:
        raise ValueError("scripted high-rigor Bookends-enabled path did not reach its frozen terminal outcome")
    review_rows=[row for row in final["context"] if row["kind"]=="review-evidence"]
    ledgers=[row for row in final["context"] if row["kind"]=="finding-ledger"]
    command_rows=[row for row in final["context"] if row["kind"]=="command-evidence"]
    verdict_rows=[row for row in final["context"] if row["kind"] in {"criterion-verdict","goal-verdict"}]
    if not review_rows or len(ledgers)<10 or not command_rows or not verdict_rows:
        raise ValueError("completed path did not retain independent reviewer, ledger, proof, and criterion/goal evidence")
    final_occasions=[row for row in final["context"] if row["kind"]=="advice-occasion"
                     and row["data"].get("occasion_id","").startswith("final-completion:")]
    if len(final_occasions)!=1 or len(final_occasions[0]["data"].get("response_ids",[]))!=2:
        raise ValueError("final occasion did not retain both original and superseding scripted responses")
    final_response_ids=final_occasions[0]["data"]["response_ids"]
    captured={row["id"]:row["data"] for row in final["context"] if row["kind"]=="advice-attempt"}
    final_disposition_counts={}
    for response_id in final_response_ids:
        response=captured[response_id]
        answer_ids=response["typed_result"]["answers"].keys()
        count=sum(1 for answer_id in answer_ids if any(
            row["kind"]=="advice-disposition" and row["data"].get("response_id")==response_id
            and row["data"].get("answer_id")==answer_id
            and row["data"].get("target")==final_occasions[0]["data"]["target"]
            for row in final["context"]))
        if count!=len(response["typed_result"]["answers"]):
            raise ValueError(f"final response {response_id} has undispositioned or stale-target answers")
        final_disposition_counts[response_id]=count
    phase_inspection = _retain_phase_inspection(journey,run_root,final,committed_prd_identity)
    journey._expect_allow = original_expect
    proof={"status":"passed","run_id":journey.run_id,"terminal_state":final["current_state"],
           "bookends_enabled":True,"bookends_source_prd":committed_prd_identity,
           "five_phase_inspection":phase_inspection,
           "review_evidence_count":len(review_rows),"ledger_count":len(ledgers),
           "command_evidence_count":len(command_rows),"criterion_goal_count":len(verdict_rows),
           "reconciliation":str(recon),"validation_capture_count":len(validation["records"]),
           "advice_attempt_ids":[row["id"] for row in final["context"] if row["kind"]=="advice-attempt"],
           "final_response_ids":final_response_ids,"superseded_response_ids":final_response_ids[1:],
           "final_answer_disposition_counts":final_disposition_counts,
           "semantic_advice_quality_claim":False}
    _write_json(run_root / "completed-path-proof.json",proof)
    return proof


def _disabled_and_exception_variants(journey, root: Path, setup_results: dict[str, dict[str, Any]]) -> None:
    # Declined setup freezes no command/map, show says disabled, and advise does
    # not execute the previously configured scripted command.
    profile = copy.deepcopy(setup_results["high-rigor"]["declined_profile"])
    run_root = root / "disabled-run"
    run_root.mkdir()
    artifacts = run_root / "artifacts"
    artifacts.mkdir()
    intent = json.loads((journey.fixture_root / "intent-good.json").read_text(encoding="utf-8"))
    _write_json(artifacts / "intent.json", intent)
    profile["artifact_root"] = str(artifacts)
    profile_path = run_root / "profile.json"
    _write_json(profile_path, profile)
    providers = run_root / "providers.toml"
    journey._write_provider_config_at(providers)
    journey.run_dir, journey.database, journey.provider_config = run_root, run_root / "loop.sqlite", providers
    journey.profile_path, journey.artifact_root = profile_path, artifacts
    journey.repository_root = run_root
    journey.command_cwd = run_root
    journey.command_env = {}
    journey.profile = profile
    journey.work_slot_bindings = profile.get("work_slot_bindings", {})
    journey.run_id, journey.state = "sol-advice-disabled", "not-started"
    journey._start()
    show = journey._assert_show("explore","disabled-advice-guidance")
    advice_guidance = show.get("action_guidance",{}).get("advice",{})
    if advice_guidance.get("enabled") is not False:
        raise ValueError("declined future run did not expose effective advice-disabled state")
    config = json.loads((root / "advice-config.json").read_text())
    disabled_request = {"version":1,"state":{"admissibility":{"bounded_judgment":True,"evidence_sufficient":True},"fact":"fixture"},"target":{"revision":"r1"},
                        "occasion":"ad-hoc:disabled","questions":{"q":{"type":"noul","instructions":"Judge the supplied fact.","proposition":"The fact is present."}}}
    request_path = run_root / "disabled-request.json"
    _write_json(request_path, disabled_request)
    disabled = journey._engine_for(journey.run_id,["advise",journey.run_id,f"@{request_path}"],state=journey.state,event="advise-disabled")
    if disabled.get("code") != "advice-disabled":
        raise ValueError(f"declined run selected a default advice backend: {disabled!r}")
    after = journey._show_for(journey.run_id,state="explore",event="disabled-show")
    if any(row["kind"]=="advice-attempt" for row in after["context"]):
        raise ValueError("disabled advice call created an attempt")
    journey._assert_show("explore","disabled-before-transition")
    journey._expect_allow("intent-ready","intent-review")

    # A scoped owner exception discharges one unanswered mapped occasion on a
    # check-free review revision. It is recorded separately from whole-edge
    # override and does not create a reviewer answer or finding disposition.
    enabled_profile = copy.deepcopy(setup_results["high-rigor"]["configured_profile"])
    exception_root = root / "owner-exception-run"
    exception_root.mkdir()
    exception_artifacts = exception_root / "artifacts"
    exception_artifacts.mkdir()
    _write_json(exception_artifacts / "intent.json", intent)
    enabled_profile["artifact_root"] = str(exception_artifacts)
    exception_profile_path = exception_root / "profile.json"
    _write_json(exception_profile_path, enabled_profile)
    exception_providers = exception_root / "providers.toml"
    journey._write_provider_config_at(exception_providers)
    journey.run_dir, journey.database, journey.provider_config = exception_root, exception_root / "loop.sqlite", exception_providers
    journey.profile_path, journey.artifact_root = exception_profile_path, exception_artifacts
    journey.repository_root = exception_root
    journey.command_cwd = exception_root
    journey.command_env = {}
    journey.profile = enabled_profile
    journey.work_slot_bindings = enabled_profile.get("work_slot_bindings", {})
    journey.run_id, journey.state = "sol-advice-owner-exception", "not-started"
    journey._start()
    journey._assert_show("explore","owner-exception-start")
    journey._expect_allow("intent-ready","intent-review")
    shown = journey._show_for(journey.run_id,state="intent-review",event="owner-exception-map")
    rows = [row for row in shown["initial_input"]["advice_departures"]["occasions"]
            if row["state"]=="intent-review" and row["event"]=="revise"]
    due = next(row for row in rows if row["occasion_id"].startswith("review-candidates:"))
    source_id = "owner-exception-source"
    _append_record(journey,journey.run_id,"intent-review",source_id,"driver-note",
                   {"observation":"A selected active-run note supplies this mapped review-candidate question."})
    visit = journey._show_for(journey.run_id,state="intent-review",event="owner-exception-visit")["state_visit"]
    target = {"run_id":journey.run_id,"state":"intent-review","state_visit":visit,"source_context_ids":[source_id]}
    request = {"version":1,"state":{"admissibility":{"bounded_judgment":True,"evidence_sufficient":True},"selected_source_id":source_id},"target":target,
               "occasion":due["occasion_id"],
               "questions":{"selected-note":{"type":"noul","instructions":"Judge the supplied note only.",
                                                "proposition":"The note records the current observation."}}}
    answer = _advise(journey,root,journey.run_id,request,"correct")
    for row in rows:
        row_target = request["target"] if row["occasion_id"]==due["occasion_id"] else target
        _record_occasion(journey,journey.run_id,"intent-review","revise",
                         {**row,"state_visit":visit,"target":row_target},
                         triggered=row["occasion_id"]==due["occasion_id"],
                         request=request if row["occasion_id"]==due["occasion_id"] else None,
                         source_ids=[source_id] if row["occasion_id"]==due["occasion_id"] else None,
                         response_ids=[answer["attempt_id"]] if row["occasion_id"]==due["occasion_id"] else None,
                         reason="No other mapped decision is triggered on this check-free revision.")
    bad_disposition = {"response_id":answer["attempt_id"],"answer_id":"selected-note",
                       "occasion_id":request["occasion"],"state_visit":visit,"target":target,
                       "disposition":"accept","reason":""}
    _append_record(journey,journey.run_id,"intent-review","missing-reason-disposition",
                   "advice-disposition",bad_disposition)
    exception_for_answer = {"state_visit":visit,"owner":"fixture owner",
                            "reason":"Owner exception cannot replace a successful answer disposition.",
                            "occasion_ids":[due["occasion_id"]]}
    journey._assert_show("intent-review","missing-reason-before-exception")
    missing_reason = journey._engine(["event",journey.run_id,"revise","--advice-exception",json.dumps(exception_for_answer)],
                                     state="intent-review",event="revise")
    if missing_reason.get("status") not in {"error","rejected"} or "reasoned disposition" not in json.dumps(missing_reason).lower():
        raise ValueError(f"missing answer disposition/reason was waived by owner exception: {missing_reason!r}")
    journey._assert_show("intent-review","missing-reason-denial-observed")
    _append_one_disposition(journey,journey.run_id,"intent-review",answer,"selected-note",status="accept",
                            reason="The driver inspected and accounted for the selected answer.",target=target)
    journey._assert_show("intent-review","answer-disposed-before-revision")
    normal = journey._engine(["event",journey.run_id,"revise"],state="intent-review",event="revise")
    journey._expect_status(normal,"completed",event="revise",state="intent-review")
    journey.state="explore"
    journey._assert_show("explore","re-enter-after-reasoned-disposition")
    journey._expect_allow("intent-ready","intent-review")

    # A separate unanswered due occasion is excused only by its explicit owner
    # attestation, which remains distinct from event override and reviewer pass.
    shown = journey._show_for(journey.run_id,state="intent-review",event="owner-exception-map")
    rows = [row for row in shown["initial_input"]["advice_departures"]["occasions"]
            if row["state"]=="intent-review" and row["event"]=="revise"]
    due = next(row for row in rows if row["occasion_id"].startswith("review-candidates:"))
    source_id = "owner-exception-source-2"
    _append_record(journey,journey.run_id,"intent-review",source_id,"driver-note",
                   {"observation":"The owner attests this exact unanswered review-candidate occasion for the fixture."})
    intent_bytes=(exception_artifacts / "intent.json").read_bytes()
    owner_review_id="owner-exception-review-candidate"
    _append_record(journey,journey.run_id,"intent-review",owner_review_id,"review-evidence",{
        "gate":"intent-review","policy_id":"acceptance-granularity","review_stage":"aggregate",
        "result":"fail","findings":"The selected source lacks an observed outcome.",
        "review_contract_version":2,"grounds":{"reason":"The owner-exception fixture retains the exact current intent source.",
            "evidence":[{"locator":"intent.json#/revision","sha256":"sha256:"+hashlib.sha256(intent_bytes).hexdigest()}]},
        "author":{"name":"exception-fixture-reviewer","kind":"script"},"subject":"intent.json",
        "subject_revision":json.loads(intent_bytes)["revision"],"config_version":enabled_profile["config_version"]
    })
    shown=journey._show_for(journey.run_id,state="intent-review",event="owner-exception-timeout-show")
    request=_provider_request(journey,root,shown,due["occasion_id"],[owner_review_id],["intent.json"])
    request["state"]["fixture_response_mode"]="timeout"
    timeout_path=exception_root/"due-timeout-request.json"
    _write_json(timeout_path,request)
    timed_out=journey._engine_for(journey.run_id,["advise",journey.run_id,f"@{timeout_path}"],
                                  state="intent-review",event="due-timeout")
    if timed_out.get("code")!="advice-command-failed" or not timed_out.get("details",{}).get("timed_out"):
        raise ValueError(f"due software-change advice timeout did not remain unanswered: {timed_out!r}")
    response_id=timed_out["details"]["attempt_id"]
    visit = journey._show_for(journey.run_id,state="intent-review",event="owner-exception-visit")["state_visit"]
    target = request["target"]
    for row in rows:
        if row["occasion_id"]==due["occasion_id"]:
            _record_occasion(journey,journey.run_id,"intent-review","revise",
                             {**row,"state_visit":visit,"target":target},triggered=True,
                             source_ids=[owner_review_id,source_id],response_ids=[response_id])
        else:
            _record_occasion(journey,journey.run_id,"intent-review","revise",
                             {**row,"state_visit":visit,"target":target},triggered=False,
                             reason="No other mapped decision is triggered in this scoped exception fixture.")
    exception = {"state_visit":visit,"owner":"fixture owner","reason":"The owner explicitly accepts this one unanswered scripted occasion.",
                 "occasion_ids":[due["occasion_id"]]}
    journey._assert_show("intent-review","timeout-before-owner-exception")
    refused=journey._engine(["event",journey.run_id,"revise"],state="intent-review",event="revise")
    if refused.get("status") not in {"error","rejected"} or "unanswered due advice" not in json.dumps(refused).lower():
        raise ValueError(f"timed-out due advice did not block ordinary revision: {refused!r}")
    journey._assert_show("intent-review","owner-exception-before-event")
    result = journey._engine(["event",journey.run_id,"revise","--advice-exception",json.dumps(exception)],
                             state="intent-review",event="revise")
    journey._expect_status(result,"completed",event="revise",state="intent-review")
    outcome = result["result"]["history"]["action"]["outcome"]
    if outcome.get("outcome")!="advice-exception" or outcome.get("exception",{}).get("occasion_ids")!=[due["occasion_id"]]:
        raise ValueError(f"scoped owner exception was not visible in ordinary software-change history: {outcome!r}")
    if result["result"]["run"].get("has_overrides") or result["result"]["run"].get("override_count",0):
        raise ValueError("advice-only owner exception was conflated with whole-edge override")
    print("software-change advice setup, selected questions, dispositions, disabled, timeout, and scoped exception assertions passed")


def advice_provider_case(journey) -> None:
    root = _root(journey)
    advisor_script, advisor_counter = _write_advisor(root)
    advice_config = _selected_advice_config(advisor_script, advisor_counter)
    advice_config_path = root / "advice-config.json"
    _write_json(advice_config_path, advice_config)
    setup_results = _setup_profiles(journey, root, advice_config_path)
    _public_setup_and_question_families(journey, root, setup_results, advice_config, advisor_script, advisor_counter)
    completed = _completed_bookends_run(journey, root, setup_results, advice_config)
    _disabled_and_exception_variants(journey, root, setup_results)
    _write_json(root / "sol-advice-provider-proof.json", {
        "setup_profiles": sorted(setup_results),
        "eight_question_families": OCCASIONS,
        "completed_bookends_run": completed,
        "disabled_and_owner_exception": "public future-run variants passed",
        "semantic_advice_quality_claim": False,
    })
