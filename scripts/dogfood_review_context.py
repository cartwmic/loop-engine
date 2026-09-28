"""Public compact-commission, grounded-output, and intent-source fixture.

All reviewers are local scripted workers. Their outputs prove capture and
mechanical contracts only; they are not semantic review or owner approval.
"""
from __future__ import annotations

import hashlib
import json
import subprocess
import sys
import time
from pathlib import Path
from typing import Any

import dogfood_observation


def _json(path: Path, value: Any) -> None:
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


def _review_worker() -> str:
    return r'''import hashlib,json,os,sys,time
from pathlib import Path

log_path=Path(sys.argv[1]); retrieved_root=Path(sys.argv[2]); citation_mode=sys.argv[3]
raw=sys.stdin.buffer.read(); text=raw.decode("utf-8")
def line(prefix, default=""):
    return next((item[len(prefix):] for item in text.splitlines() if item.startswith(prefix)),default)
location_line=next(item for item in reversed(text.splitlines()) if item.startswith('{"artifact_root"'))
location=json.loads(location_line); artifact_root=Path(location["artifact_root"])
slot=line("slot_id: "); stage=line("review_stage: "); author=line("required_author_claim: ")
policies=json.loads(line("assigned_policies: ","[]")); axes=[item["id"] for item in policies]
subject={"intent-review":"intent.json","intent-adversarial-review":"intent.json"}[slot]
source_bytes=(artifact_root/subject).read_bytes(); source=json.loads(source_bytes)
source_hash="sha256:"+hashlib.sha256(source_bytes).hexdigest()
locator=subject+"#/acceptance/0"
owner_records=[row for row in location.get("context",[]) if row.get("kind")=="user-steering"]
baseline_revisions=[row.get("data",{}).get("intent_baseline",{}).get("revision") for row in owner_records]
owner_qualification=any("reviewer judgment, driver inference, and owner approval" in row.get("data",{}).get("instruction","") for row in owner_records)
material_later=any(value and value!=source.get("revision") for value in baseline_revisions)
lost_owner_qualification=material_later and owner_qualification and "owner" not in source.get("outcome","").lower()
owner_source_check={"material_later_revision":material_later,"owner_qualification_present":owner_qualification,"qualification_missing_from_current_outcome":lost_owner_qualification,"source_ids":[row.get("id") for row in owner_records]}
if citation_mode=="false-pass" and lost_owner_qualification:
    locator=subject+"#/outcome"
if citation_mode=="bad-citation": source_hash="sha256:"+"0"*64
retrieved_root.mkdir(parents=True,exist_ok=True)
retrieved=retrieved_root/(str(time.time_ns())+"-"+author+"-"+stage+"-"+"-".join(axes)+".json")
retrieved.write_bytes(source_bytes)
entry={"author":author,"stage":stage,"slot":slot,"axes":axes,"stdin":text,"stdin_bytes":len(raw),
       "owner_source_check":owner_source_check,
       "retrieved":{"locator":subject,"sha256":"sha256:"+hashlib.sha256(source_bytes).hexdigest(),"bytes":len(source_bytes),"capture":str(retrieved)}}
with log_path.open("a",encoding="utf-8") as stream: stream.write(json.dumps(entry)+"\n")
output={"review_contract_version":2,"review_stage":stage,"author":{"name":author,"kind":"agent"},"judgments":[]}
for policy in policies:
    result="pass"; findings=""; reason="The scripted fixture inspected the current assigned subject."
    if policy["id"]=="owner-comprehensible" and lost_owner_qualification:
        if citation_mode=="false-pass":
            # Deliberately false but mechanically well-formed negative probe:
            # only driver triage can reject a semantically unsupported pass.
            result="pass"; findings=""; reason="The current intent preserves the owner's approval boundary."
        else:
            result="fail"; findings="The later intent outcome drops the qualified owner distinction present in the retained source."
            reason="The exact retained owner statement requires the reviewer/driver/owner distinction, while intent.json#/outcome no longer says owner."
    output["judgments"].append({"axis":policy["id"],"result":result,"findings":findings,
      "grounds":{"reason":reason,"evidence":[{"locator":locator,"sha256":source_hash}]}})
print(json.dumps(output,separators=(",",":")))
'''


def _token_budget(model_id: str) -> dict[str, Any]:
    return {
        "model_id": model_id,
        "context_window_tokens": 272000,
        "system_tokens": 4000,
        "framing_tokens": 4000,
        "output_reserve_tokens": 4000,
        "reasoning_reserve_tokens": 16000,
    }


def _append(journey, root: Path, database: Path, run_id: str, record_id: str, kind: str, data: dict[str, Any]) -> None:
    _call(journey, root, database, "show", "--view", "full", run_id)
    path = root / f"append-{record_id}.json"
    _json(path, data)
    result, _, _ = dogfood_observation._engine(
        journey, database, "append", "--record-id", record_id, run_id, kind, "@" + str(path)
    )
    if result.get("status") != "completed":
        raise ValueError(f"append {record_id} did not complete: {result}")


def _call(journey, root: Path, database: Path, *args: str, expect: str = "completed"):
    return dogfood_observation._engine(journey, database, *args, expect=expect)


def _wait_invocation(journey, root: Path, database: Path, run_id: str, invocation_id: str) -> tuple[dict[str, Any], dict[str, Any]]:
    deadline = time.monotonic() + 60
    while time.monotonic() < deadline:
        shown, _, _ = _call(journey, root, database, "show", "--view", "full", run_id)
        projection = shown.get("result", {})
        rows = projection.get("work_slot_invocations", [])
        row = next((item for item in rows if item.get("invocation_id") == invocation_id), None)
        if row and row.get("status") in ("succeeded", "failed", "overrun"):
            return shown, row
        time.sleep(0.05)
    raise ValueError(f"bound review invocation {invocation_id} did not finish")


def _owner_sources(journey, root: Path, database: Path, run_id: str, intent: dict[str, Any]) -> None:
    _append(journey, root, database, run_id, "owner-source-old", "user-steering", {
        "target": {"kind": "all"},
        "instruction": "Preserve independent external judgment and every material owner qualification; a driver paraphrase is not an owner decision.",
    })
    _append(journey, root, database, run_id, "owner-source-current", "user-steering", {
        "target": {"kind": "all"},
        "instruction": "The intent must retain the distinction between reviewer judgment, driver inference, and owner approval.",
        "supersedes": ["owner-source-old"],
    })
    _append(journey, root, database, run_id, "driver-incorporation", "steering-incorporation", {
        "steering_ids": ["owner-source-current"],
        "applied": "Driver paraphrase: owner approved automatic review completion.",
    })


def _seed_initial_history(
    journey, root: Path, database: Path, run_id: str, intent: dict[str, Any], *, mature: bool
) -> list[str]:
    seeded_ids = ["seed-individual-a", "seed-individual-b"]
    for record_id, author, text in [
        (seeded_ids[0], "reviewer-a", "Seeded individual finding from author A"),
        (seeded_ids[1], "reviewer-b", "Seeded individual finding from author B"),
    ]:
        _append(journey, root, database, run_id, record_id, "review-evidence", {
            "gate": "intent-review", "policy_id": "solution-agnostic", "review_stage": "individual",
            "result": "fail", "findings": text,
            "author": {"name": author, "kind": "agent"},
            "subject": "intent.json", "subject_revision": intent["revision"],
            "config_version": "high-rigor-12",
        })
    findings = []
    for record_id, author, finding_id, statement in [
        (seeded_ids[0], "reviewer-a", "F-seed-a", "Seeded individual finding from author A"),
        (seeded_ids[1], "reviewer-b", "F-seed-b", "Seeded individual finding from author B"),
    ]:
        findings.append({
            "id": finding_id,
            "source": {"kind": "context-record", "id": record_id},
            "policy_id": "solution-agnostic",
            "statement": statement,
            "disposition": "accepted",
            "reason": "Seeded solely to test the first-aggregate projection.",
            "owner_phase": "intent",
            "task_ids": [],
            "review_axes": ["solution-agnostic"],
            "status": "unresolved",
        })
    _append(journey, root, database, run_id, "seed-finding-ledger", "finding-ledger", {
        "schema_version": "1", "gate": "intent-review", "subject": "intent.json",
        "subject_revision": intent["revision"],
        "author": {"name": "fixture-driver", "kind": "human"},
        "findings": findings,
    })
    if mature:
        seeded_ids.append("seed-finding-ledger")
        for index in range(32):
            history_id = f"irrelevant-history-{index:02d}"
            seeded_ids.append(history_id)
            _append(journey, root, database, run_id, history_id, "review-evidence", {
                "gate": "plan-review", "policy_id": "task-sized", "review_stage": "aggregate",
                "result": "fail", "findings": f"unrelated accumulated history {index}: " + "z" * 1800,
                "author": {"name": "historical-reviewer", "kind": "agent"},
                "subject": "plan.json", "subject_revision": f"old-{index}",
                "config_version": "high-rigor-12",
            })
    else:
        seeded_ids.append("seed-finding-ledger")
    return seeded_ids


def _start_run(journey, root: Path, label: str, citation_modes: tuple[str, str], *, mature: bool) -> dict[str, Any]:
    run_root = root / label
    run_root.mkdir(parents=True)
    reviewer = run_root / "reviewer.py"
    reviewer.write_text(_review_worker(), encoding="utf-8")
    log_a = run_root / "reviewer-a.jsonl"
    log_b = run_root / "reviewer-b.jsonl"
    retrieved = run_root / "retrieved"
    roster = run_root / "roster.json"
    _json(roster, [
        {"author": "reviewer-a", "command": sys.executable,
         "args": [str(reviewer), str(log_a), str(retrieved), citation_modes[0]],
         "token_budget": _token_budget("fixture/reviewer-a")},
        {"author": "reviewer-b", "command": sys.executable,
         "args": [str(reviewer), str(log_b), str(retrieved), citation_modes[1]],
         "token_budget": _token_budget("fixture/reviewer-b")},
    ])
    profile = run_root / "profile.json"
    setup_argv = [str(journey.provider), "setup", "--rigor", "high", "--roster", str(roster),
                  "--engine", str(journey.engine), "--provider", str(journey.provider), "--output", str(profile),
                  "--decline-advice"]
    setup, _, process = dogfood_observation._capture(root, setup_argv, expect="setup", timeout=60)
    if process.returncode != 0 or setup.get("status") != "ready":
        raise ValueError(f"public setup failed for {label}: {setup} {process.stderr[-1000:]!r}")
    provider_config = run_root / "providers.toml"
    provider_config.write_text(
        "[providers.software-change]\n"
        f"command = {json.dumps(str(journey.provider))}\nargs = []\n",
        encoding="utf-8",
    )
    database = run_root / "loop.sqlite"
    run_id = f"review-context-{label}"
    artifact_root = run_root / "artifacts"
    profile_value = json.loads(profile.read_text(encoding="utf-8"))
    profile_value["artifact_root"] = str(artifact_root)
    initial_path = run_root / "initial-input.json"
    _json(initial_path, profile_value)
    started, _, _ = dogfood_observation._engine(
        journey, database, "--config", str(provider_config), "start", "--id", run_id,
        "software-change", "@" + str(initial_path),
    )
    if started.get("status") != "completed":
        raise ValueError(f"public run start failed for {label}: {started}")
    initial_input = started["result"]["run"]["initial_input"]
    artifact_root = Path(initial_input["artifact_root"])
    try:
        artifact_root.resolve().relative_to(Path(journey.data_root).resolve())
    except ValueError:
        pass
    else:
        raise ValueError("review-context fixture artifact root unexpectedly lies inside the checkout")
    fixture = Path(journey.data_root) / "crates/software-change-provider/data/calibration/fixtures/intent-good.json"
    intent_bytes = fixture.read_bytes()
    intent = json.loads(intent_bytes)
    artifact_root.mkdir(parents=True, exist_ok=True)
    (artifact_root / "intent.json").write_bytes(intent_bytes)
    shown, _, _ = dogfood_observation._engine(journey, database, "show", "--view", "full", run_id)
    _call(journey, root, database, "event", run_id, "intent-ready")
    _call(journey, root, database, "show", "--view", "action", run_id)
    _owner_sources(journey, root, database, run_id, intent)
    seeded = _seed_initial_history(journey, root, database, run_id, intent, mature=mature)
    full_before, _, _ = dogfood_observation._engine(journey, database, "show", "--view", "full", run_id)
    raw_context_bytes = len(json.dumps(full_before["result"].get("context", []), separators=(",", ":")).encode())
    owner_view = _inspect_owner_view(journey, root, database, run_id, roster)
    _call(journey, root, database, "show", "--view", "action", run_id)
    started_invocation, _, _ = dogfood_observation._engine(
        journey, database, "--timeout-ms", "120000", "invoke", run_id, "intent-review"
    )
    if started_invocation.get("status") != "completed":
        raise ValueError(f"bound public intent commission failed to start: {started_invocation}")
    invocation_id = started_invocation["result"]["invocation_id"]
    shown, invocation = _wait_invocation(journey, root, database, run_id, invocation_id)
    if invocation.get("status") != "succeeded":
        raise ValueError(f"scripted review invocation did not complete: {invocation}")
    candidates = _review_candidates(journey, root, shown)
    capture = Path(invocation["capture_dir"])
    observations = _inspect_captured_workers(capture, [log_a, log_b], seeded, retrieved)
    return {
        "root": run_root, "database": database, "run_id": run_id, "artifact_root": artifact_root,
        "intent_bytes": intent_bytes, "intent": intent, "owner_view": owner_view, "invocation": invocation,
        "capture": capture, "candidates": candidates, "observations": observations,
        "raw_context_bytes": raw_context_bytes, "log_a": log_a, "log_b": log_b,
        "retrieved": retrieved, "seeded": seeded, "profile": profile, "roster": roster,
        "bad_citation": "bad-citation" in citation_modes, "invocation": invocation,
    }


def _wait_invocation(journey, root: Path, database: Path, run_id: str, invocation_id: str):
    deadline = time.monotonic() + 90
    while time.monotonic() < deadline:
        shown, _, _ = dogfood_observation._engine(journey, database, "show", "--view", "full", run_id)
        rows = shown["result"].get("work_slot_invocations", [])
        invocation = next((row for row in rows if row.get("invocation_id") == invocation_id), None)
        if invocation and invocation.get("status") in ("succeeded", "failed", "overrun"):
            return shown, invocation
        time.sleep(0.05)
    raise ValueError(f"review invocation {invocation_id} did not finish")


def _review_candidates(journey, root: Path, show: dict[str, Any]) -> dict[str, Any]:
    process = subprocess.run([str(journey.provider), "review-candidates"], cwd=root,
                             input=json.dumps(show, separators=(",", ":")).encode(),
                             capture_output=True, timeout=30, check=False)
    (root / "review-candidates.stdout").write_bytes(process.stdout)
    (root / "review-candidates.stderr").write_bytes(process.stderr)
    if process.returncode != 0:
        raise ValueError(f"public review-candidates refused valid captured output: {process.stderr[-1200:]!r}")
    return json.loads(process.stdout)


def _preamble_fields(stdin: bytes) -> tuple[str, str, tuple[str, ...]]:
    text = stdin.decode("utf-8")
    def line(prefix: str) -> str:
        return next(value[len(prefix):] for value in text.splitlines() if value.startswith(prefix))
    policies = json.loads(line("assigned_policies: "))
    return line("required_author_claim: "), line("review_stage: "), tuple(row["id"] for row in policies)


def _inspect_captured_workers(capture: Path, logs: list[Path], hidden_ids: list[str], retrieved_root: Path) -> list[dict[str, Any]]:
    spec = json.loads((capture / "fan-out-spec.json").read_text(encoding="utf-8"))
    log_rows = []
    for path in logs:
        if path.exists():
            log_rows.extend(json.loads(line) for line in path.read_text(encoding="utf-8").splitlines())
    keyed = {(row["author"], row["stage"], tuple(row["axes"])): row for row in log_rows}
    if len(keyed) != len(log_rows):
        raise ValueError("reviewer fixture emitted duplicate author/stage/axis capture keys")
    observed = []
    for worker in spec.get("workers", []):
        path = Path(worker["stdin_path"])
        if not path.is_absolute():
            path = capture / path
        raw = path.read_bytes()
        author, stage, axes = _preamble_fields(raw)
        row = keyed.get((author, stage, axes))
        if row is None:
            raise ValueError(f"captured stdin has no matching scripted reader row: {author}/{stage}/{axes}")
        if row["stdin"].encode() != raw or row["stdin_bytes"] != len(raw):
            raise ValueError("scripted reader bytes differ from the exact captured fan-out stdin")
        location = json.loads(next(line for line in raw.decode().splitlines() if line.startswith('{"artifact_root"')))
        delivered = location.get("context", [])
        delivered_ids = {record.get("id") for record in delivered}
        leaked = delivered_ids.intersection(hidden_ids)
        if leaked:
            raise ValueError(f"first aggregate/independent packet leaked individual or unrelated IDs: {sorted(leaked)}")
        if "driver-incorporation" in delivered_ids:
            raise ValueError("driver paraphrase leaked as owner instruction")
        if "owner-source-old" not in delivered_ids or "owner-source-current" not in delivered_ids:
            raise ValueError("qualified/superseded owner sources were not both retained")
        owner_current = next(record for record in delivered if record.get("id") == "owner-source-current")
        if "owner-source-old" not in owner_current.get("data", {}).get("supersedes", []):
            raise ValueError("owner-source supersession edge was not preserved")
        if "reviewer judgment" not in owner_current.get("data", {}).get("instruction", ""):
            raise ValueError("current qualified owner statement was changed in the delivered packet")
        if stage == "aggregate" and "FIRST AGGREGATE REVIEW" not in raw.decode():
            raise ValueError("aggregate capture lacks truthful first-stage preamble")
        retrieved = row["retrieved"]
        retrieved_path = Path(retrieved["capture"])
        exact = retrieved_path.read_bytes()
        digest = "sha256:" + hashlib.sha256(exact).hexdigest()
        if digest != retrieved["sha256"] or retrieved["bytes"] != len(exact):
            raise ValueError("retrieved original did not retain exact bytes/hash/length")
        if exact != Path(location["artifact_root"], retrieved["locator"]).read_bytes():
            raise ValueError("retrieved original bytes differ from their original locator")
        observed.append({
            "author": author, "stage": stage, "axes": list(axes), "stdin_bytes": len(raw),
            "delivered_record_ids": sorted(delivered_ids), "retrieved": retrieved,
        })
    if len(observed) != len(log_rows):
        raise ValueError(f"captured assignments/read logs disagree: {len(observed)} != {len(log_rows)}")
    return observed


def _inspect_owner_view(journey, root: Path, database: Path, run_id: str, roster: Path) -> dict[str, Any]:
    shown, _, _ = dogfood_observation._engine(journey, database, "show", "--view", "full", run_id)
    budgets = json.loads(roster.read_text(encoding="utf-8"))
    call_budgets = [{"author": row["author"], **row["token_budget"]} for row in budgets]
    command = subprocess.run(
        [str(journey.provider), "commission", "--slot", "intent-review", "--stage", "aggregate",
         "--call-budgets", json.dumps(call_budgets, separators=(",", ":"))],
        cwd=root, input=json.dumps(shown, separators=(",", ":")).encode(),
        capture_output=True, timeout=30, check=False,
    )
    (root / "intent-owner-view.stdout").write_bytes(command.stdout)
    (root / "intent-owner-view.stderr").write_bytes(command.stderr)
    if command.returncode != 0:
        raise ValueError(f"public intent owner view failed: {command.stderr[-1200:]!r}")
    return json.loads(command.stdout)


def _append_review_candidates(journey, root: Path, database: Path, run_id: str, candidates: dict[str, Any], invocation_id: str, revision: str) -> list[str]:
    record_ids = []
    index = 0
    for row in candidates.get("candidates", []):
        if row.get("status") != "ready" or row.get("origin", {}).get("id") != invocation_id:
            continue
        data = {
            "gate": "intent-review", "policy_id": row["axis"], "review_stage": row["review_stage"],
            "result": row["result"], "findings": row["findings"],
            "review_contract_version": row["review_contract_version"], "grounds": row["grounds"],
            "author": row["author"], "subject": "intent.json", "subject_revision": revision,
            "config_version": "high-rigor-12", "origin": row["origin"],
        }
        record_id = f"fixture-review-{index:03d}"
        _append(journey, root, database, run_id, record_id, "review-evidence", data)
        record_ids.append(record_id)
        index += 1
    return record_ids


def _record_missing_source_negative(journey, root: Path, database: Path, run_id: str, intent: dict[str, Any]) -> None:
    _append(journey, root, database, run_id, "confirmation-anchor", "review-evidence", {
        "gate": "intent-review", "policy_id": "solution-agnostic", "review_stage": "aggregate",
        "result": "pass", "findings": "", "author": {"name": "reviewer-a", "kind": "agent"},
        "subject": "intent.json", "subject_revision": intent["revision"], "config_version": "high-rigor-12",
    })
    _append(journey, root, database, run_id, "missing-confirmation-ledger", "finding-ledger", {
        "schema_version": "1", "gate": "intent-review", "subject": "intent.json",
        "subject_revision": intent["revision"], "author": {"name": "fixture-driver", "kind": "human"},
        "findings": [{
            "id": "F-missing-confirmation", "source": {"kind": "context-record", "id": "absent-source"},
            "policy_id": "solution-agnostic", "statement": "mandatory confirmation finding",
            "disposition": "accepted", "reason": "seeded missing-source negative",
            "owner_phase": "intent", "task_ids": [], "review_axes": ["solution-agnostic"], "status": "unresolved",
        }],
    })
    _call(journey, root, database, "show", "--view", "action", run_id)
    response, _, _ = _call(
        journey, root, database, "--timeout-ms", "120000", "invoke", run_id, "intent-review", expect="error"
    )
    if response.get("status") not in ("error", "rejected"):
        raise ValueError(f"missing mandatory confirmation source was not refused: {response}")
    detail = json.dumps(response).lower()
    if "absent-source" not in detail and "mandatory confirmation" not in detail:
        # The prepare-context diagnostic is emitted on stderr; retained command
        # capture is the source of truth if the envelope omits the detail.
        captures = sorted((root / "commands").glob("*.stderr"))
        if not any("absent-source" in path.read_text(encoding="utf-8", errors="replace") for path in captures):
            raise ValueError(f"missing source refusal did not name its original locator: {response}")


def _finish_fixture_findings(journey, root: Path, database: Path, run_id: str, intent: dict[str, Any]) -> None:
    entries = [
        ("F-seed-a", "seed-individual-a", "Seeded individual finding from author A"),
        ("F-seed-b", "seed-individual-b", "Seeded individual finding from author B"),
    ]
    _append(journey, root, database, run_id, "triaged-fixture-ledger", "finding-ledger", {
        "schema_version": "1", "gate": "intent-review", "subject": "intent.json",
        "subject_revision": intent["revision"], "author": {"name": "fixture-driver", "kind": "human"},
        "findings": [{
            "id": finding_id, "source": {"kind": "context-record", "id": source_id},
            "policy_id": "solution-agnostic", "statement": statement,
            "disposition": "rejected", "reason": "Synthetic fixture triage; this is not semantic review approval.",
            "owner_phase": None, "task_ids": [], "review_axes": [], "status": "recorded",
        } for finding_id, source_id, statement in entries],
    })


def _candidate_reference_cases(journey, root: Path, label: str, run: dict[str, Any]) -> None:
    candidates = run["candidates"].get("candidates", [])
    ready = [row for row in candidates if row.get("status") == "ready"]
    malformed = [row for row in candidates if row.get("status") == "malformed"]
    if not ready:
        raise ValueError(f"{label}: no sound grounded pass candidate was mechanically ready")
    if run["bad_citation"]:
        if len(malformed) < 6:
            raise ValueError(f"{label}: false source citations were not refused per reviewer assignment: {candidates}")
    else:
        if malformed or len(ready) != len(candidates):
            raise ValueError(f"{label}: sound exact citations did not remain ready: {candidates}")


def _inspect_owner_delta(journey, root: Path, run: dict[str, Any]) -> dict[str, Any]:
    artifact = run["artifact_root"] / "intent.json"
    previous_bytes = artifact.read_bytes()
    previous = json.loads(previous_bytes)
    current = json.loads(json.dumps(previous))
    current["revision"] = "r16"
    current["outcome"] = "A checked transition may pass on a syntactically valid review record."
    artifact.write_text(json.dumps(current, indent=2) + "\n", encoding="utf-8")
    baseline = {
        "revision": previous["revision"], "locator": "intent.json captured before material revision",
        "sha256": "sha256:" + hashlib.sha256(previous_bytes).hexdigest(),
        "json_bytes": previous_bytes.decode("utf-8"),
    }
    _append(journey, root, run["database"], run["run_id"], "owner-source-revision", "user-steering", {
        "target": {"kind": "all"},
        "instruction": "Preserve the qualification that external reviewers judge and the owner alone approves material intent.",
        "supersedes": ["owner-source-current"], "intent_baseline": baseline,
    })
    _append(journey, root, run["database"], run["run_id"], "revision-applicability", "evidence-applicability", {
        "origin":{"kind":"context-record","id":"seed-individual-a"},
        "target":{"subject":"intent.json","revision":current["revision"],"checkpoint":None},
        "attesting_driver":{"name":"fixture-driver","kind":"human"},
        "reason":"The seeded finding remains applicable to the materially revised intent."
    })
    _call(journey, root, run["database"], "show", "--view", "action", run["run_id"])
    show, _, _ = dogfood_observation._engine(journey, run["database"], "show", "--view", "full", run["run_id"])
    roster = json.loads(run["roster"].read_text(encoding="utf-8"))
    budgets = [{"author": item["author"], **item["token_budget"]} for item in roster]
    result = subprocess.run(
        [str(journey.provider), "commission", "--slot", "intent-review", "--stage", "aggregate",
         "--call-budgets", json.dumps(budgets, separators=(",", ":"))],
        cwd=root, input=json.dumps(show, separators=(",", ":")).encode(), capture_output=True, timeout=30, check=False,
    )
    (root / "owner-delta-commission.stdout").write_bytes(result.stdout)
    (root / "owner-delta-commission.stderr").write_bytes(result.stderr)
    if result.returncode != 0:
        raise ValueError(f"material later intent commission failed: {result.stderr[-1200:]!r}")
    view = json.loads(result.stdout)["owner_approval_view"]
    if view["exact_substantive_wording_delta"]["status"] != "available":
        raise ValueError(f"owner view omitted an exact prior-intent diff: {view}")
    baseline_ref = view["exact_substantive_wording_delta"].get("baseline_source", {})
    if not baseline_ref.get("locator") or not baseline_ref.get("sha256", "").startswith("sha256:") or not baseline_ref.get("bytes"):
        raise ValueError("owner view omitted the exact prior-intent source locator/digest/length")
    if not any(row.get("instruction", "").startswith("Preserve the qualification") for row in view["owner_sources"]):
        raise ValueError("owner view lost the qualified current owner statement")
    if any(row.get("record_id") == "driver-incorporation" for row in view["owner_sources"]):
        raise ValueError("driver incorporation was laundered into owner-source instructions")
    changes = view["exact_substantive_wording_delta"].get("changes", [])
    if not any(row.get("pointer") == "/outcome" and row.get("before") != row.get("after") for row in changes):
        raise ValueError("owner view did not preserve the exact changed outcome text")
    if view["actors"]["owner_approval"].get("status") != "not-performed":
        raise ValueError("commission fabricated owner approval")
    if not {"reviewer-a", "reviewer-b"}.issubset(set(view["actors"]["reviewers"])):
        raise ValueError(f"owner view did not separate non-drafter reviewer identities: {view['actors']}")
    if "fixture-driver" not in view["actors"]["driver"]:
        raise ValueError(f"owner view did not separate driver identity: {view['actors']}")
    if view["current_intent"]["value"] != current:
        raise ValueError("owner approval view omitted the full current intent")
    return view


def _run_later_intent_revision(journey, root: Path, run: dict[str, Any]) -> str:
    _call(journey, root, run["database"], "show", "--view", "action", run["run_id"])
    started, _, _ = dogfood_observation._engine(
        journey, run["database"], "--timeout-ms", "120000", "invoke", run["run_id"], "intent-review"
    )
    if started.get("status") != "completed":
        raise ValueError(f"non-drafter later-intent commission did not start: {started}")
    invocation_id = started["result"]["invocation_id"]
    later_show, invocation = _wait_invocation(journey, root, run["database"], run["run_id"], invocation_id)
    if invocation.get("status") != "succeeded":
        raise ValueError(f"later-intent commission failed: {invocation}")
    later_capture = Path(invocation["capture_dir"])
    spec = json.loads((later_capture / "fan-out-spec.json").read_text(encoding="utf-8"))
    for worker in spec.get("workers", []):
        stdin_path = Path(worker["stdin_path"])
        if not stdin_path.is_absolute():
            stdin_path = later_capture / stdin_path
        raw = stdin_path.read_bytes()
        location = json.loads(next(line for line in raw.decode("utf-8").splitlines() if line.startswith('{"artifact_root"')))
        context = location.get("context", [])
        ledger = next((record for record in context if record.get("id") == "seed-finding-ledger"), None)
        if ledger is None:
            raise ValueError("material confirmation commission omitted the retained current finding ledger")
        retained = {row.get("source", {}).get("id"): row for row in ledger.get("data", {}).get("findings", [])}
        if retained.get("seed-individual-a", {}).get("statement") != "Seeded individual finding from author A":
            raise ValueError("material confirmation commission lost the actual accepted finding")
        if not any(record.get("id") == "seed-individual-a"
                   and record.get("data", {}).get("findings") == "Seeded individual finding from author A"
                   for record in context):
            raise ValueError("material confirmation commission omitted the original finding evidence")
        if not any(record.get("id") == "revision-applicability" for record in context):
            raise ValueError("material confirmation commission omitted current evidence-applicability")
        owner_sources = {record.get("id"): record for record in context if record.get("kind") == "user-steering"}
        for owner_id in ("owner-source-old", "owner-source-current", "owner-source-revision"):
            if owner_id not in owner_sources:
                raise ValueError(f"material intent commission omitted qualified/superseded owner source {owner_id}")
        if "driver-incorporation" in {record.get("id") for record in context}:
            raise ValueError("driver paraphrase leaked into material intent review")
        revision_source = owner_sources["owner-source-revision"]["data"]
        baseline = revision_source.get("intent_baseline", {})
        if baseline.get("json_bytes", "").encode("utf-8") != run["intent_bytes"]:
            raise ValueError("material intent commission did not retain the exact prior owner-review source bytes")
        if revision_source.get("supersedes") != ["owner-source-current"]:
            raise ValueError("material intent commission lost owner-source supersession")
    selected = []
    for worker in invocation.get("inner_workers", []):
        author = worker.get("author") or worker.get("name")
        output_path = worker.get("selected_output_path")
        if not output_path:
            continue
        output_path = Path(output_path)
        if not output_path.is_absolute():
            output_path = Path(invocation["capture_dir"]) / output_path
        raw = output_path.read_bytes()
        judgment = json.loads(raw)
        if judgment.get("author", {}).get("name") not in ("reviewer-a", "reviewer-b"):
            continue
        row = next((item for item in judgment.get("judgments", []) if item.get("axis") == "owner-comprehensible"), None)
        if row is not None:
            selected.append((judgment["author"]["name"], row))
    if {author for author, _ in selected} != {"reviewer-a", "reviewer-b"}:
        raise ValueError(f"later intent did not receive both non-drafter aggregate reviewers: {selected}")
    by_author = {author: row for author, row in selected}
    if set(by_author) != {"reviewer-a", "reviewer-b"}:
        raise ValueError(f"later intent review omitted a configured author: {selected}")
    if by_author["reviewer-a"].get("result") != "pass" or by_author["reviewer-a"].get("findings") != "":
        raise ValueError(f"scripted false-pass negative changed its declared judgment: {selected}")
    if by_author["reviewer-b"].get("result") != "fail" or "qualif" not in by_author["reviewer-b"].get("findings", "").lower():
        raise ValueError(f"scripted grounded defect judgment was not retained: {selected}")
    candidates = _review_candidates(journey, root, later_show)
    false_pass = next((row for row in candidates.get("candidates", [])
                       if row.get("origin", {}).get("id") == invocation_id
                       and row.get("author", {}).get("name") == "reviewer-a"
                       and row.get("axis") == "owner-comprehensible"), None)
    if not false_pass or false_pass.get("status") != "ready" or false_pass.get("result") != "pass":
        raise ValueError(f"mechanically valid but intentionally false pass was not retained as a candidate: {false_pass}")
    evidence = false_pass.get("grounds", {}).get("evidence", [])
    expected_hash = "sha256:" + hashlib.sha256((run["artifact_root"] / "intent.json").read_bytes()).hexdigest()
    if not any(item.get("locator") == "intent.json#/outcome" and item.get("sha256") == expected_hash for item in evidence):
        raise ValueError(f"false-pass candidate did not retain its exact, mechanically valid source citation: {false_pass}")
    logs = [run["log_a"], run["log_b"]]
    checks = [json.loads(line) for path in logs for line in path.read_text(encoding="utf-8").splitlines()]
    later = [row for row in checks if row.get("owner_source_check", {}).get("material_later_revision")]
    if len(later) < 2 or not all(row["owner_source_check"].get("qualification_missing_from_current_outcome") for row in later):
        raise ValueError("later reviewer sessions did not inspect qualified source plus changed intent")
    current_bytes = (run["artifact_root"] / "intent.json").read_bytes()
    for row in later:
        retrieved = row["retrieved"]
        retrieved_bytes = Path(retrieved["capture"]).read_bytes()
        if retrieved_bytes != current_bytes or retrieved["bytes"] != len(current_bytes):
            raise ValueError("later reviewer did not retain exact current intent bytes and source locator")
        if retrieved["sha256"] != "sha256:" + hashlib.sha256(current_bytes).hexdigest():
            raise ValueError("later reviewer source digest did not match the exact current intent bytes")
    return invocation_id, false_pass


def _test_missing_intent_baseline_refusal(journey, root: Path, run: dict[str, Any]) -> None:
    artifact = run["artifact_root"] / "intent.json"
    original = artifact.read_bytes()
    changed = json.loads(original)
    changed["revision"] = "r16"
    artifact.write_text(json.dumps(changed, indent=2) + "\n", encoding="utf-8")
    shown, _, _ = dogfood_observation._engine(journey, run["database"], "show", "--view", "full", run["run_id"])
    roster = json.loads(run["roster"].read_text(encoding="utf-8"))
    budgets = [{"author": item["author"], **item["token_budget"]} for item in roster]
    result = subprocess.run(
        [str(journey.provider), "commission", "--slot", "intent-review", "--stage", "aggregate",
         "--call-budgets", json.dumps(budgets, separators=(",", ":"))],
        cwd=root, input=json.dumps(shown, separators=(",", ":")).encode(),
        capture_output=True, timeout=30, check=False,
    )
    (root / "missing-intent-baseline.stdout").write_bytes(result.stdout)
    (root / "missing-intent-baseline.stderr").write_bytes(result.stderr)
    artifact.write_bytes(original)
    if result.returncode == 0 or "intent_baseline" not in result.stderr.decode(errors="replace"):
        raise ValueError("material later intent commission did not refuse absent exact owner baseline")


def _test_budget_refusal(journey, root: Path, run: dict[str, Any]) -> None:
    shown, _, _ = dogfood_observation._engine(journey, run["database"], "show", "--view", "full", run["run_id"])
    tiny = [{"author":"reviewer-a","model_id":"fixture/tiny","context_window_tokens":5000,
             "system_tokens":1000,"framing_tokens":1000,"output_reserve_tokens":1000,"reasoning_reserve_tokens":1000}]
    result = subprocess.run(
        [str(journey.provider), "commission", "--slot", "intent-review", "--stage", "aggregate",
         "--call-budgets", json.dumps(tiny, separators=(",", ":"))],
        cwd=root, input=json.dumps(shown, separators=(",", ":")).encode(), capture_output=True, timeout=30, check=False,
    )
    (root / "tiny-budget.stdout").write_bytes(result.stdout)
    (root / "tiny-budget.stderr").write_bytes(result.stderr)
    if result.returncode == 0 or "intent.json" not in result.stderr.decode(errors="replace") or "fixture/tiny" not in result.stderr.decode(errors="replace"):
        raise ValueError("oversized mandatory source did not refuse with its original locator and model identity")


def review_context_case(journey) -> None:
    root = dogfood_observation._fresh_root(journey, "sol-review-context")
    small = _start_run(journey, root, "small", ("good", "good"), mature=False)
    mature = _start_run(journey, root, "mature", ("good", "bad-citation"), mature=True)

    for label, run in [("small", small), ("mature", mature)]:
        if run["candidates"].get("schema_version") != "3":
            raise ValueError(f"{label}: review-candidates did not expose grounded projection schema version 3")
        _candidate_reference_cases(journey, root, label, run)
        if run["raw_context_bytes"] < 32_768 and label == "mature":
            raise ValueError("mature fixture did not contain enough unrelated accumulated history")
        if max(row["stdin_bytes"] for row in run["observations"]) >= 32_768:
            raise ValueError(f"{label} compact review stdin exceeded the starting limit")
        budget = run["owner_view"].get("review_budget", {})
        context_bytes = budget.get("context_bytes")
        original_bytes = budget.get("retrieved_original_bytes")
        if context_bytes is None or original_bytes is None:
            raise ValueError(f"{label} commission omitted actual byte accounting: {budget}")
        if budget.get("initial_evidence_bytes") != context_bytes + original_bytes:
            raise ValueError(f"{label} initial byte bound did not include mandatory originals: {budget}")
        calls = {row.get("author"): row for row in budget.get("calls", [])}
        roster = json.loads(run["roster"].read_text(encoding="utf-8"))
        if set(calls) != {row["author"] for row in roster}:
            raise ValueError(f"{label} budget did not account for each commissioned reviewer: {calls}")
        for entry in roster:
            call = calls[entry["author"]]
            for field in ("model_id", "context_window_tokens", "system_tokens", "framing_tokens", "output_reserve_tokens", "reasoning_reserve_tokens"):
                if call.get(field) != entry["token_budget"][field]:
                    raise ValueError(f"{label} per-call {field} was not preserved: {call}")
            if budget.get("total_evidence_bytes", 0) > call.get("available_input_tokens", 0):
                raise ValueError(f"{label} evidence did not fit after per-call reserves: {call}")

    small_bytes = max(row["stdin_bytes"] for row in small["observations"])
    mature_bytes = max(row["stdin_bytes"] for row in mature["observations"])
    if mature_bytes > small_bytes + 2048:
        raise ValueError(f"irrelevant mature history inflated delivered stdin: {mature_bytes} > {small_bytes}")
    source_lengths = {row["retrieved"]["bytes"] for row in small["observations"] + mature["observations"]}
    if len(source_lengths) != 1:
        raise ValueError("same small/mature target did not retain identical original-source byte counts")

    _test_budget_refusal(journey, root, mature)
    _test_missing_intent_baseline_refusal(journey, root, mature)

    delta = _start_run(journey, root, "intent-delta", ("false-pass", "good"), mature=False)
    owner_view = _inspect_owner_delta(journey, root, delta)
    owner_revision_invocation, false_pass_probe = _run_later_intent_revision(journey, root, delta)
    delta_intent = json.loads((delta["artifact_root"] / "intent.json").read_text(encoding="utf-8"))
    _record_missing_source_negative(journey, root, delta["database"], delta["run_id"], delta_intent)

    # Triage all synthetic failures as rejected fixture rows, then append every
    # first-run candidate with its true selected-assignment origin.
    _finish_fixture_findings(journey, root, small["database"], small["run_id"], small["intent"])
    appended = _append_review_candidates(
        journey, root, small["database"], small["run_id"], small["candidates"],
        small["invocation"]["invocation_id"], small["intent"]["revision"],
    )
    if len(appended) < 24:
        raise ValueError(f"expected grounded candidates for all high-rigor stages/axes, got {len(appended)}")
    _call(journey, root, small["database"], "show", "--view", "action", small["run_id"])
    parent_approved, _, _ = dogfood_observation._engine(
        journey, small["database"], "event", small["run_id"], "approved"
    )
    if parent_approved.get("status") != "completed":
        raise ValueError(f"synthetic source-complete parent fixture did not reach challenge: {parent_approved}")
    _call(journey, root, small["database"], "show", "--view", "action", small["run_id"])
    challenge, _, _ = dogfood_observation._engine(
        journey, small["database"], "--timeout-ms", "120000", "invoke", small["run_id"], "intent-adversarial-review"
    )
    if challenge.get("status") != "completed":
        raise ValueError(f"challenge with captured parent grounds did not launch: {challenge}")
    challenge_id = challenge["result"]["invocation_id"]
    challenge_show, challenge_row = _wait_invocation(journey, root, small["database"], small["run_id"], challenge_id)
    if challenge_row.get("status") != "succeeded":
        raise ValueError("grounded challenge fixture did not complete")
    challenge_spec = json.loads((Path(challenge_row["capture_dir"]) / "fan-out-spec.json").read_text(encoding="utf-8"))
    parent_grounds = {
        (row["axis"], row["author"]["name"]): row["grounds"]
        for row in small["candidates"].get("candidates", [])
        if row.get("status") == "ready"
        and row.get("origin", {}).get("id") == small["invocation"]["invocation_id"]
        and row.get("review_stage") == "aggregate"
    }
    challenge_text = []
    for worker in challenge_spec["workers"]:
        stdin_path = Path(worker["stdin_path"])
        if not stdin_path.is_absolute():
            stdin_path = Path(challenge_row["capture_dir"]) / stdin_path
        raw = stdin_path.read_text(encoding="utf-8")
        challenge_text.append(raw)
        location = json.loads(next(line for line in raw.splitlines() if line.startswith('{"artifact_root"')))
        delivered = [record for record in location.get("context", [])
                     if record.get("kind") == "review-evidence"
                     and record.get("data", {}).get("gate") == "intent-review"]
        delivered_keys = {(record["data"]["policy_id"], record["data"]["author"]["name"]): record["data"]["grounds"]
                          for record in delivered if record["data"].get("review_stage") == "aggregate"}
        if delivered_keys != parent_grounds:
            raise ValueError(f"challenge did not receive the exact parent aggregate grounds: {delivered_keys} != {parent_grounds}")
        if any(record["data"].get("review_stage") == "individual" for record in delivered):
            raise ValueError("challenge context leaked parent individual-stage judgments")
    if not challenge_text:
        raise ValueError("challenge fan-out had no captured worker stdin")

    # A newer ungrounded parent row must refuse a later challenge attempt; an
    # earlier grounded row cannot mask the missing current parent grounds.
    bad_parent = {
        "gate":"intent-review","policy_id":"solution-agnostic","review_stage":"aggregate",
        "result":"pass","findings":"","author":{"name":"reviewer-a","kind":"agent"},
        "subject":"intent.json","subject_revision":small["intent"]["revision"],"config_version":"high-rigor-12",
    }
    _append(journey,root,small["database"],small["run_id"],"parent-ground-missing","review-evidence",bad_parent)
    _call(journey,root,small["database"],"show","--view","action",small["run_id"])
    refused,_,_ = dogfood_observation._engine(
        journey,small["database"],"--timeout-ms","120000","invoke",small["run_id"],"intent-adversarial-review",expect="error"
    )
    if refused.get("status") not in ("error","rejected"):
        raise ValueError(f"challenge did not refuse absent parent grounds: {refused}")
    grounds_error = json.dumps(refused).lower()
    if "parent" not in grounds_error and "grounds" not in grounds_error:
        logs = sorted((root / "commands").glob("*.stderr"))
        if not any("parent" in path.read_text(encoding="utf-8",errors="replace").lower() for path in logs):
            raise ValueError("absent parent ground refusal did not identify its source")

    # Owner approval view is a proposal to inspect, never owner approval.
    result = {
        "schema_version": 1,
        "small": {"raw_history_bytes": small["raw_context_bytes"], "max_delivered_stdin_bytes": small_bytes,
                  "assignments": small["observations"], "candidate_schema_version": small["candidates"].get("schema_version")},
        "mature": {"raw_history_bytes": mature["raw_context_bytes"], "max_delivered_stdin_bytes": mature_bytes,
                    "assignments": mature["observations"], "candidate_schema_version": mature["candidates"].get("schema_version")},
        "runs": {
            "small": {"run_id": small["run_id"], "database": str(small["database"]),
                      "initial_review_invocation_id": small["invocation"]["invocation_id"]},
            "mature": {"run_id": mature["run_id"], "database": str(mature["database"]),
                       "initial_review_invocation_id": mature["invocation"]["invocation_id"]},
            "intent_delta": {"run_id": delta["run_id"], "database": str(delta["database"]),
                             "initial_review_invocation_id": delta["invocation"]["invocation_id"]},
        },
        "owner_approval_view": owner_view,
        "owner_revision_invocation_id": owner_revision_invocation,
        "false_pass_probe": false_pass_probe,
        "confirmation_applicability_id": "revision-applicability",
        "parent_evidence_ids": appended,
        "challenge_invocation_id": challenge_id,
        "semantic_status": "scripted transport/mechanical assertions only; no model judgment or owner approval",
    }
    _json(root / "outcome.json", result)
