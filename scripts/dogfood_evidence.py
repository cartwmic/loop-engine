"""Public evidence-admission, exact-append, and recovery-integration fixtures.

Reviewers remain scripted; the configured-model path uses a deterministic
local command only. These cases prove mechanics and refusal behavior, not
reviewer/model quality or semantic approval.
"""
from __future__ import annotations

import copy
import hashlib
import json
import shutil
import subprocess
import sys
import time
from pathlib import Path
from typing import Any

import dogfood_observation
import dogfood_recovery


REVIEWER = {"name": "fixture-reviewer", "kind": "agent"}
DRIVER = {"name": "fixture-driver", "kind": "script"}


def _sha(data: bytes) -> str:
    return "sha256:" + hashlib.sha256(data).hexdigest()


def _json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


def _record(root: Path, argv: list[str], completed: subprocess.CompletedProcess[bytes]) -> None:
    dogfood_recovery._record(root, argv, completed)


def _record_show_metadata(root: Path, argv: list[str], completed: subprocess.CompletedProcess[bytes]) -> None:
    """Retain a full-show identity projection, never its stdout bytes."""
    commands = root / "commands"
    commands.mkdir(exist_ok=True)
    ordinal = len(list(commands.glob("*.argv.json")))
    stem = f"{ordinal:04d}"
    (commands / f"{stem}.argv.json").write_text(
        json.dumps({"argv": argv, "cwd": str(root)}) + "\n", encoding="utf-8"
    )
    (commands / f"{stem}.stdout-projection.json").write_text(
        json.dumps({"kind": "full-show-stdout-projection", "bytes": len(completed.stdout), "sha256": _sha(completed.stdout)}) + "\n",
        encoding="utf-8",
    )
    (commands / f"{stem}.stderr").write_bytes(completed.stderr)
    (commands / f"{stem}.exit.json").write_text(
        json.dumps({"returncode": completed.returncode}) + "\n", encoding="utf-8"
    )


def _call(
    root: Path,
    argv: list[str],
    *,
    input_bytes: bytes | None = None,
    expect: str = "completed",
    timeout: float = 45,
    save_stdout: bool = True,
) -> tuple[dict[str, Any], subprocess.CompletedProcess[bytes]]:
    try:
        completed = subprocess.run(
            argv, cwd=root, input=input_bytes, capture_output=True, timeout=timeout, check=False
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise ValueError(f"public command did not complete: {argv!r}: {error}") from error
    if save_stdout:
        _record(root, argv, completed)
    else:
        _record_show_metadata(root, argv, completed)
    try:
        value = json.loads(completed.stdout)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError(
            f"public command returned non-JSON (exit={completed.returncode}): {argv!r}; "
            f"stderr={completed.stderr[-1200:]!r}"
        ) from error
    if expect == "completed":
        if completed.returncode != 0 or value.get("status") != "completed":
            raise ValueError(f"public command did not complete: {argv!r}: {value!r}")
    elif expect == "rejected":
        if completed.returncode != 10 or value.get("status") != "rejected":
            raise ValueError(f"public command was not refused: {argv!r}: {value!r}")
    elif expect == "invalid":
        if completed.returncode != 2:
            raise ValueError(f"public helper did not refuse malformed input: {argv!r}: {completed.returncode}")
    else:
        raise ValueError(f"unknown expected command outcome {expect!r}")
    return value, completed


def _engine(journey, root: Path, database: Path, *args: str, expect: str = "completed", timeout: float = 45):
    return _call(
        root,
        [str(journey.engine), "--database", str(database), "--json", *args],
        expect=expect,
        timeout=timeout,
    )


def _show_action(journey, root: Path, database: Path, run_id: str) -> dict[str, Any]:
    return _engine(journey, root, database, "show", "--view", "action", run_id)[0]


def _show_full(journey, root: Path, database: Path, run_id: str) -> dict[str, Any]:
    argv = [str(journey.engine), "--database", str(database), "--json", "show", "--view", "full", run_id]
    packet, _ = _call(root, argv, save_stdout=False)
    return packet


def _wait(journey, root: Path, database: Path, run_id: str, invocation_id: str, timeout: float = 60):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        show = _show_full(journey, root, database, run_id)
        result = show["result"]
        row = next((item for item in result["work_slot_invocations"] if item["invocation_id"] == invocation_id), None)
        if row and row["status"] in ("succeeded", "failed", "overrun"):
            return show, row
        time.sleep(0.05)
    raise ValueError(f"invocation did not become terminal: {invocation_id}")


def _invoke(journey, root: Path, database: Path, run_id: str, slot: str, invocation_input: dict[str, Any] | None = None):
    _show_action(journey, root, database, run_id)
    args = ["--timeout-ms", "120000", "invoke", run_id, slot]
    if invocation_input is not None:
        args.extend(["--input", json.dumps(invocation_input, separators=(",", ":"))])
    started, _ = _engine(journey, root, database, *args, timeout=60)
    invocation_id = started["result"]["invocation_id"]
    return started["result"], *_wait(journey, root, database, run_id, invocation_id)


def _append(journey, root: Path, database: Path, run_id: str, record_id: str, kind: str, data: dict[str, Any]):
    _show_action(journey, root, database, run_id)
    result, _ = _engine(
        journey,
        root,
        database,
        "append",
        run_id,
        kind,
        json.dumps(data, separators=(",", ":")),
        f"--record-id={record_id}",
    )
    return result["result"]


def _event(journey, root: Path, database: Path, run_id: str, event: str, *, expect: str = "completed"):
    _show_action(journey, root, database, run_id)
    return _engine(journey, root, database, "event", run_id, event, expect=expect)[0]


def _candidate_doc(journey, root: Path, checkout: Path, show: dict[str, Any]) -> dict[str, Any]:
    argv = [str(journey.provider), "review-candidates"]
    completed = subprocess.run(
        argv,
        cwd=checkout,
        input=json.dumps(show, separators=(",", ":")).encode(),
        capture_output=True,
        timeout=30,
        check=False,
    )
    _record(root, argv, completed)
    if completed.returncode != 0:
        raise ValueError(f"review-candidates refused an authoritative show: {completed.stderr[-1200:]!r}")
    try:
        return json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        raise ValueError(f"review-candidates returned malformed JSON: {completed.stdout!r}") from error


def _review_schema(axes: list[str], *, author: dict[str, str] = REVIEWER, batch: bool = True) -> dict[str, Any]:
    if not batch:
        fields = ["review_contract_version", "review_stage", "author", "axis", "result", "findings", "grounds"]
        return {
            "type": "object",
            "additionalProperties": False,
            "required": fields,
            "properties": {
                "review_contract_version": {"type": "integer", "const": 2},
                "review_stage": {"type": "string", "const": "aggregate"},
                "author": {
                    "type": "object", "additionalProperties": False, "required": ["name", "kind"],
                    "properties": {
                        "name": {"type": "string", "minLength": 1},
                        "kind": {"type": "string", "enum": ["human", "agent", "script"]},
                    },
                    "const": author,
                },
                "axis": {"type": "string", "const": axes[0]},
                "result": {"type": "string", "enum": ["pass", "fail"]},
                "findings": {"type": "string"},
                "grounds": {
                    "type": "object", "additionalProperties": False, "required": ["reason", "evidence"],
                    "properties": {
                        "reason": {"type": "string", "minLength": 1, "maxLength": 1200},
                        "evidence": {
                            "type": "array", "minItems": 1, "maxItems": 8,
                            "items": {
                                "type": "object", "additionalProperties": False, "required": ["locator", "sha256"],
                                "properties": {
                                    "locator": {"type": "string", "minLength": 1, "maxLength": 1024},
                                    "sha256": {"type": "string", "pattern": "^sha256:[0-9a-f]{64}$"},
                                },
                            },
                        },
                    },
                },
            },
            "oneOf": [
                {"properties": {"result": {"const": "pass"}, "findings": {"const": ""}}},
                {"properties": {"result": {"const": "fail"}, "findings": {"type": "string", "minLength": 1}}},
            ],
        }

    schema = json.loads(
        (Path(__file__).resolve().parents[1] / "crates/software-change-provider/data/review-worker-output-schema-v2.json").read_text(encoding="utf-8")
    )
    schema["properties"]["review_stage"]["const"] = "aggregate"
    schema["properties"]["author"]["const"] = author
    judgments = schema["properties"]["judgments"]
    judgments["minItems"] = len(axes)
    judgments["maxItems"] = len(axes)
    for branch in judgments["items"]["oneOf"]:
        axis_schema = branch.get("properties", {}).get("axis")
        if axis_schema is not None:
            axis_schema["enum"] = axes
    judgments["allOf"] = [
        {"contains": {"type": "object", "required": ["axis"], "properties": {"axis": {"const": axis}}}}
        for axis in axes
    ]
    return schema


def _worker(path: Path) -> None:
    path.write_text(
        "import hashlib,json,pathlib,sys\n"
        "mode=sys.argv[1]; counter=pathlib.Path(sys.argv[2]); spec=json.loads(pathlib.Path(sys.argv[3]).read_text()); packet=json.loads(sys.stdin.buffer.read())\n"
        "count=int(counter.read_text()) if counter.exists() else 0; counter.write_text(str(count+1))\n"
        "root=pathlib.Path(packet['artifact_root']); intent=(root/'intent.json').read_bytes(); digest='sha256:'+hashlib.sha256(intent).hexdigest()\n"
        "if mode=='batch':\n"
        " rows=[]\n"
        " for row in spec['judgments']:\n"
        "  if 'reuse' in row: rows.append({'axis':row['axis'],'reuse':row['reuse']}); continue\n"
        "  result=row['result']; findings=row.get('findings','')\n"
        "  rows.append({'axis':row['axis'],'result':result,'findings':findings,'grounds':{'reason':'I inspected the exact current intent source for this fixture judgment.','evidence':[{'locator':'intent.json#/revision','sha256':digest}]}})\n"
        " print(json.dumps({'review_contract_version':2,'review_stage':'aggregate','author':spec['author'],'judgments':rows},separators=(',',':')))\n"
        "elif mode=='prose':\n"
        " print('Representation-only explicit judgment')\n"
        " print('Review stage: aggregate')\n"
        " print('Author name: '+spec['author']['name'])\n"
        " print('Author kind: '+spec['author']['kind'])\n"
        " print('Axis: '+spec['axis'])\n"
        " print('Result: fail')\n"
        " print('Finding: '+spec['finding'])\n"
        " print('Reason: '+spec['reason'])\n"
        " print('Evidence locator: intent.json#/revision')\n"
        " print('Evidence sha256: '+digest)\n"
        "else: raise SystemExit('unknown fixture mode')\n",
        encoding="utf-8",
    )


def _adapter(path: Path) -> None:
    path.write_text(
        "import json,pathlib,sys\n"
        "counter=pathlib.Path(sys.argv[1]); request=json.load(sys.stdin); raw=bytes.fromhex(request['raw_output_hex']).decode('utf-8')\n"
        "counter.write_text(str((int(counter.read_text()) if counter.exists() else 0)+1))\n"
        "fields={}\n"
        "for line in raw.splitlines():\n"
        " if ': ' in line:\n"
        "  key,value=line.split(': ',1); fields[key]=value\n"
        "required=['Review stage','Author name','Author kind','Axis','Result','Finding','Reason','Evidence locator','Evidence sha256']\n"
        "if any(key not in fields for key in required) or fields['Result']!='fail': raise SystemExit('raw prose did not contain the complete explicit FAIL meaning')\n"
        "output={'review_contract_version':2,'review_stage':fields['Review stage'],'author':{'name':fields['Author name'],'kind':fields['Author kind']},'axis':fields['Axis'],'result':'fail','findings':fields['Finding'],'grounds':{'reason':fields['Reason'],'evidence':[{'locator':fields['Evidence locator'],'sha256':fields['Evidence sha256']}]}}\n"
        "print(json.dumps({'output':json.dumps(output,separators=(',',':')),'usage':{'calls':1,'metered_cost_micros':2}}))\n",
        encoding="utf-8",
    )


def _write_run(journey, root: Path, label: str, axes: list[str], worker: dict[str, Any], *, schema: dict[str, Any] | None = None, binding: dict[str, Any] | None = None):
    run_root = root / label
    run_root.mkdir(parents=True)
    artifacts = run_root / "artifacts"
    artifacts.mkdir()
    checkout = run_root / "checkout"
    checkout.mkdir()
    provider_config = run_root / "providers.toml"
    provider_config.write_text(
        "[providers.software-change]\n"
        f"command = {json.dumps(str(journey.provider))}\nargs = []\n",
        encoding="utf-8",
    )
    source_profile = json.loads(
        (journey.data_root / "crates/software-change-provider/data/configs/minimal.json").read_text(encoding="utf-8")
    )
    templates = {entry["id"]: entry for entry in source_profile["review_policies"]["intent-review"]}
    template = next(iter(templates.values()))
    entries = []
    for axis in axes:
        entry = copy.deepcopy(templates.get(axis, template))
        entry["id"] = axis
        entries.append(entry)
    profile = copy.deepcopy(source_profile)
    profile["review_policies"] = {"intent-review": entries}
    profile["artifact_root"] = str(artifacts)
    binding_worker = dict(worker)
    if schema is not None:
        binding_worker["full_output_schema"] = schema
    profile["work_slot_bindings"] = {
        "intent-review": binding or dogfood_recovery._fanout_binding(journey.engine, [binding_worker], max_active=1)
    }
    profile_path = run_root / "initial-input.json"
    _json(profile_path, profile)
    database = run_root / "loop.sqlite"
    run_id = f"p07-{label}"
    started, _ = dogfood_recovery._call(
        root,
        [
            str(journey.engine), "--database", str(database), "--json", "--config", str(provider_config),
            "start", "--id", run_id, "software-change", "@" + str(profile_path),
        ],
    )
    if Path(started["result"]["run"]["initial_input"]["artifact_root"]) != artifacts:
        raise ValueError("fixture changed its configured artifact_root")
    fixtures = journey.data_root / "crates/software-change-provider/data/calibration/fixtures"
    shutil.copyfile(fixtures / "intent-good.json", artifacts / "intent.json")
    _show_action(journey, root, database, run_id)
    dogfood_recovery._engine(journey, root, database, "event", run_id, "intent-ready")
    shown = _show_full(journey, root, database, run_id)
    if shown["result"]["current_state"] != "intent-review":
        raise ValueError(f"fixture did not reach intent-review: {shown['result']['current_state']}")
    return run_root, artifacts, checkout, database, run_id


def _minimal_config_version(journey) -> str:
    profile = journey.data_root / "crates/software-change-provider/data/configs/minimal.json"
    return json.loads(profile.read_text(encoding="utf-8"))["config_version"]


def _review_evidence(axis: str, gate: str, revision: str, config: str) -> dict[str, Any]:
    return {
        "gate": gate,
        "policy_id": axis,
        "review_stage": "aggregate",
        "result": "pass",
        "findings": "",
        "author": dict(REVIEWER),
        "subject": "intent.json",
        "subject_revision": revision,
        "config_version": config,
    }


def _append_applicability(journey, root, database, run_id, record_id, source_id, target_revision):
    _append(
        journey,
        root,
        database,
        run_id,
        record_id,
        "evidence-applicability",
        {
            "origin": {"kind": "context-record", "id": source_id},
            "target": {"subject": "intent.json", "revision": target_revision, "checkpoint": None},
            "attesting_driver": dict(DRIVER),
            "reason": "Fixture driver explicitly tested captured row-reference mechanics.",
        },
    )


def _axes_from_records(records: list[dict[str, Any]], status: str) -> list[str]:
    return sorted(record["axis"] for record in records if record["status"] == status and record.get("axis"))


def high_rigor_refusal_case(journey, root: Path) -> dict[str, Any]:
    """Prove four distinct real high-rigor review refusals with a valid ledger."""
    profile_source = journey.data_root / "crates/software-change-provider/data/configs/high-rigor.json"
    fixture = journey.data_root / "crates/software-change-provider/data/calibration/fixtures/intent-good.json"
    selected_stage, selected_axis = "aggregate", "acceptance-granularity"
    proof: dict[str, Any] = {}
    for deficit in ("stale", "self-authored", "duplicate-author", "incomplete-axis"):
        case = root / f"high-rigor-{deficit}"
        case.mkdir()
        artifacts = case / "artifacts"
        artifacts.mkdir()
        intent_bytes = fixture.read_bytes()
        (artifacts / "intent.json").write_bytes(intent_bytes)
        intent = json.loads(intent_bytes)
        profile = json.loads(profile_source.read_text())
        policies = profile["review_policies"]["intent-review"]
        selected = [axis for axis in policies if axis["review_stage"] == selected_stage
                    and axis["id"] == selected_axis]
        if len(selected) != 1 or selected[0]["required_authors"] != 2:
            raise ValueError("selected high-rigor aggregate axis lost its two-author floor")
        profile["artifact_root"] = str(artifacts)
        profile["work_slot_bindings"] = {}
        _json(case / "profile.json", profile)
        (case / "providers.toml").write_text(
            "[providers.software-change]\n"
            f"command = {json.dumps(str(journey.provider))}\nargs = []\n", encoding="utf-8")
        database = case / "loop.sqlite"
        run_id = f"sol-evidence-{deficit}"
        _engine(journey, case, database, "--config", str(case / "providers.toml"),
                "start", "--id", run_id, "software-change", "@" + str(case / "profile.json"))
        _event(journey, case, database, run_id, "intent-ready")
        digest = _sha(intent_bytes)
        authors = [{"name": f"independent-{index}", "kind": "script"} for index in (0, 1)]
        repair: list[tuple[str, dict[str, Any]]] = []
        populated = 0
        for policy in policies:
            stage, axis = policy["review_stage"], policy["id"]
            if policy["required_authors"] != 2:
                raise ValueError(f"high-rigor selected policy changed its floor: {policy}")
            for index, author in enumerate(authors):
                record_id = f"evidence-{stage}-{axis}-{index}"
                data = {
                    "gate": "intent-review", "policy_id": axis, "review_stage": stage,
                    "result": "pass", "findings": "", "review_contract_version": 2,
                    "grounds": {"reason": "Scripted external judgment refers to the exact current fixture intent.",
                                "evidence": [{"locator": "intent.json#/revision", "sha256": digest}]},
                    "author": author, "subject": "intent.json",
                    "subject_revision": intent["revision"], "config_version": profile["config_version"],
                }
                if (stage, axis) == (selected_stage, selected_axis):
                    if deficit == "incomplete-axis":
                        repair.append((record_id, data))
                        continue
                    if index == 1:
                        if deficit == "stale":
                            stale = {**data, "subject_revision": "previous-intent-revision"}
                            _append(journey, case, database, run_id, "stale-" + record_id,
                                    "review-evidence", stale)
                        elif deficit == "self-authored":
                            self_row = {**data, "author": intent["author"]}
                            _append(journey, case, database, run_id, "self-" + record_id,
                                    "review-evidence", self_row)
                        else:
                            duplicate = {**data, "author": authors[0]}
                            _append(journey, case, database, run_id, "duplicate-" + record_id,
                                    "review-evidence", duplicate)
                        repair.append((record_id, data))
                        continue
                _append(journey, case, database, run_id, record_id, "review-evidence", data)
                populated += 1
        _append(journey, case, database, run_id, "driver-ledger", "finding-ledger", {
            "schema_version": "1", "gate": "intent-review", "subject": "intent.json",
            "subject_revision": intent["revision"],
            "author": {"name": "fixture-driver", "kind": "agent"}, "findings": [],
        })
        denied = _event(journey, case, database, run_id, "approved", expect="rejected")
        details = denied.get("details", {})
        diagnostics = details.get("diagnostics", [])
        expected = [item for item in diagnostics
                    if item.get("review_stage") == selected_stage and item.get("axis") == selected_axis]
        if (denied.get("code") != "software-change-review-incomplete"
                or details.get("phase") != "evidence" or len(diagnostics) != 1
                or len(expected) != 1):
            raise ValueError(f"{deficit} was not isolated to the selected configured axis: {denied}")
        categories = {item.get("category") for item in expected[0].get("diagnostics", [])}
        if deficit == "incomplete-axis":
            if not {"missing", "independence"} <= categories:
                raise ValueError(f"missing configured axis had the wrong cause: {denied}")
        elif "independence" not in categories or expected[0]["diagnostics"][-1].get("distinct_present") != 1:
            raise ValueError(f"{deficit} was not refused for insufficient distinct non-subject authors: {denied}")
        if deficit == "stale" and not any(
            item.get("axis") == selected_axis and item.get("review_stage") == selected_stage
            and any(d.get("category") == "stale" for d in item.get("diagnostics", []))
            for item in details.get("informational", [])
        ):
            raise ValueError(f"stale source did not yield a real stale diagnostic: {denied}")
        after_denial = _show_full(journey, case, database, run_id)["result"]
        if after_denial["current_state"] != "intent-review" or after_denial["lifecycle"] != "active":
            raise ValueError(f"{deficit} refusal changed workflow state")
        _json(case / "specific-refusal.json", denied)
        for record_id, data in repair:
            _append(journey, case, database, run_id, record_id, "review-evidence", data)
        _event(journey, case, database, run_id, "approved")
        after_repair = _show_full(journey, case, database, run_id)["result"]
        if after_repair["current_state"] != "intent-adversarial-review":
            raise ValueError(f"{deficit} repaired records did not pass the checked gate")
        proof[deficit] = {"database": str(database), "run_id": run_id,
                          "refusal": str(case / "specific-refusal.json"),
                          "axis": f"{selected_stage}/{selected_axis}",
                          "configured_floor": 2, "valid_rows_before_denial": populated,
                          "repair_record_ids": [id for id, _ in repair],
                          "checked_state": after_repair["current_state"]}
    _json(root / "high-rigor-refusals-proof.json", proof)
    return proof


def evidence_case(journey) -> None:
    root = dogfood_observation._fresh_root(journey, "sol-evidence")
    if not shutil.which("dagu"):
        raise ValueError("sol-evidence requires the operator-provided Dagu binary on PATH")
    counter = root / "batch-worker-count"
    worker_script = root / "review-worker.py"
    _worker(worker_script)
    config_version = _minimal_config_version(journey)
    stale_config_version = config_version.rsplit("-", 1)[0] + "-11"
    axes = ["fresh-fail", "fresh-pass", "reuse-wrong-gate", "reuse-wrong-kind", "reuse-stale", "reuse-stale-config", "reuse-late", "reuse-ambiguous"]
    judgments = [
        {"axis": "fresh-fail", "result": "fail", "findings": "fixture failure remains a failure"},
        {"axis": "fresh-pass", "result": "pass", "findings": ""},
        {"axis": "reuse-wrong-gate", "reuse": "app-wrong-gate"},
        {"axis": "reuse-wrong-kind", "reuse": "evidence-not-applicability"},
        {"axis": "reuse-stale", "reuse": "app-stale-target"},
        {"axis": "reuse-stale-config", "reuse": "app-stale-config"},
        {"axis": "reuse-late", "reuse": "app-added-after-launch"},
        {"axis": "reuse-ambiguous", "reuse": "app-ambiguous"},
    ]
    spec = root / "batch-spec.json"
    _json(spec, {"author": REVIEWER, "judgments": judgments})
    binding_worker = {
        "command": sys.executable,
        "args": [str(worker_script), "batch", str(counter), str(spec)],
        "title": "P07 review batch with independent fresh and carried rows",
        "role": "reviewer",
    }
    schema = _review_schema(axes)
    run_root, artifacts, checkout, database, run_id = _write_run(
        journey, root, "row-specific-admission", axes, binding_worker, schema=schema
    )
    revision = json.loads((artifacts / "intent.json").read_text(encoding="utf-8"))["revision"]

    _append(journey, root, database, run_id, "wrong-gate-source", "review-evidence",
            _review_evidence("reuse-wrong-gate", "design-review", "old-revision", config_version))
    _append_applicability(journey, root, database, run_id, "app-wrong-gate", "wrong-gate-source", revision)
    _append(journey, root, database, run_id, "evidence-not-applicability", "review-evidence",
            _review_evidence("reuse-wrong-kind", "intent-review", "old-revision", config_version))
    _append(journey, root, database, run_id, "stale-target-source", "review-evidence",
            _review_evidence("reuse-stale", "intent-review", "old-revision", config_version))
    _append_applicability(journey, root, database, run_id, "app-stale-target", "stale-target-source", "stale-revision")
    _append(journey, root, database, run_id, "stale-config-source", "review-evidence",
            _review_evidence("reuse-stale-config", "intent-review", "old-revision", stale_config_version))
    _append_applicability(journey, root, database, run_id, "app-stale-config", "stale-config-source", revision)
    _append(journey, root, database, run_id, "late-target-source", "review-evidence",
            _review_evidence("reuse-late", "intent-review", "old-revision", config_version))
    _append(journey, root, database, run_id, "ambiguous-source", "review-evidence",
            _review_evidence("reuse-ambiguous", "intent-review", revision, config_version))
    _append_applicability(journey, root, database, run_id, "app-ambiguous", "ambiguous-source", revision)
    prelaunch_context = _show_full(journey, root, database, run_id)["result"]["context"]
    external_claim = next(record for record in prelaunch_context if record["id"] == "evidence-not-applicability")
    if external_claim["data"]["author"] != REVIEWER or "origin" in external_claim["data"] or "loop_engine_origin" in external_claim["data"]:
        raise ValueError("external authorship was promoted from a declared claim to bound-source provenance")

    started, show, invocation = _invoke(journey, root, database, run_id, "intent-review")
    if invocation["status"] != "succeeded":
        raise ValueError(f"conforming mixed review batch did not complete: {invocation}")
    capture = Path(invocation["capture_dir"])
    worker = invocation["inner_workers"][0]
    selected_path = Path(worker["selected_output_path"])
    if not selected_path.is_absolute():
        selected_path = capture / selected_path
    raw_bytes = selected_path.read_bytes()
    raw_digest = _sha(raw_bytes)
    if raw_digest != worker["selected_output_sha256"]:
        raise ValueError("selected raw review bytes changed before preview")

    _append_applicability(journey, root, database, run_id, "app-added-after-launch", "late-target-source", "stale-revision")
    intent_path = artifacts / "intent.json"
    original_intent_bytes = intent_path.read_bytes()
    drifted_intent = json.loads(original_intent_bytes)
    drifted_intent["revision"] = str(drifted_intent["revision"]) + "-before-preview"
    _json(intent_path, drifted_intent)
    stale_target_document = _candidate_doc(journey, root, checkout, _show_full(journey, root, database, run_id))
    if any(row.get("status") == "ready" for row in stale_target_document.get("records", [])):
        raise ValueError("selected judgments remained append-ready after pre-apply target drift")
    intent_path.write_bytes(original_intent_bytes)
    before_preview = _show_full(journey, root, database, run_id)
    document = _candidate_doc(journey, root, checkout, before_preview)
    after_preview = _show_full(journey, root, database, run_id)
    if before_preview["result"]["context"] != after_preview["result"]["context"]:
        raise ValueError("review-candidates preview mutated run context")
    if raw_digest != _sha(selected_path.read_bytes()):
        raise ValueError("read-only candidate preview changed immutable raw review bytes")
    ready = [row for row in document["records"] if row["status"] == "ready"]
    invalid = [row for row in document["records"] if row["status"] == "invalid-reuse"]
    if len(ready) != 2 or {row["axis"] for row in ready} != {"fresh-fail", "fresh-pass"}:
        raise ValueError(f"valid fresh siblings were lost: {document}")
    diagnostics = {row["axis"]: row.get("diagnostic", "") for row in invalid}
    expected_bad = {"reuse-wrong-gate", "reuse-wrong-kind", "reuse-stale", "reuse-stale-config", "reuse-late"}
    if set(diagnostics) != expected_bad:
        raise ValueError(f"invalid reuse rows were not independently projected: {document}")
    carried = [row for row in document["candidates"] if row.get("status") == "carried"]
    if len(carried) != 1 or carried[0].get("axis") != "reuse-ambiguous":
        raise ValueError(f"the originally unique earlier applicability was not delivered as a separate carry: {document}")
    if "wrong gate" not in diagnostics["reuse-wrong-gate"] or "not an authorized captured applicability" not in diagnostics["reuse-wrong-kind"] or "stale" not in diagnostics["reuse-stale"] or "stale config version" not in diagnostics["reuse-stale-config"] or "not an authorized captured applicability" not in diagnostics["reuse-late"]:
        raise ValueError(f"row-specific reuse diagnostics lost their causes: {diagnostics}")
    if any(row["data"].get("result") not in ("pass", "fail") for row in ready):
        raise ValueError("fresh candidate omitted the original binary result")
    by_axis = {row["axis"]: row for row in ready}
    if by_axis["fresh-fail"]["data"]["findings"] != "fixture failure remains a failure" or by_axis["fresh-pass"]["data"]["findings"] != "":
        raise ValueError("preview changed the original fail/pass findings")
    if any(row["data"]["author"] != REVIEWER or row["gate"] != "intent-review" or row["review_stage"] != "aggregate" or row["config_version"] != config_version for row in ready):
        raise ValueError("preview omitted exact gate/stage/config/author facts")

    # Duplicate a delivered applicability only in a disposable capture copy.
    # The raw source and the actual invocation capture remain unchanged.
    original_spec_path = capture / "fan-out-spec.json"
    original_spec_digest = _sha(original_spec_path.read_bytes())
    ambiguous_capture = root / "ambiguous-capture-copy"
    shutil.copytree(capture, ambiguous_capture)
    copied_spec_path = ambiguous_capture / "fan-out-spec.json"
    copied_spec = json.loads(copied_spec_path.read_text(encoding="utf-8"))
    copied_worker = next(row for row in copied_spec["workers"] if row["assignment_id"] == worker["assignment_id"])
    captured_stdin = Path(copied_worker["stdin_path"])
    if captured_stdin.is_absolute():
        captured_stdin = captured_stdin.relative_to(capture)
    copied_worker["stdin_path"] = str(ambiguous_capture / captured_stdin)
    delivered_applicability = next(record for record in copied_worker["routed_inputs"] if record["id"] == "app-ambiguous")
    copied_worker["routed_inputs"].append(copy.deepcopy(delivered_applicability))
    _json(copied_spec_path, copied_spec)
    copied_show = copy.deepcopy(before_preview)
    copied_invocation = next(
        row for row in copied_show["result"]["work_slot_invocations"]
        if row["invocation_id"] == invocation["invocation_id"]
    )
    copied_invocation["capture_dir"] = str(ambiguous_capture)
    copied_worker_show = copied_invocation["inner_workers"][0]
    copied_selected_path = Path(copied_worker_show["selected_output_path"])
    if copied_selected_path.is_absolute():
        copied_selected_path = copied_selected_path.relative_to(capture)
    copied_worker_show["selected_output_path"] = str(copied_selected_path)
    ambiguous_document = _candidate_doc(journey, root, checkout, copied_show)
    ambiguous_rows = [
        row for row in ambiguous_document["records"]
        if row.get("status") == "invalid-reuse" and row.get("axis") == "reuse-ambiguous"
    ]
    ambiguous_fresh = [row for row in ambiguous_document["records"] if row.get("status") == "ready"]
    copied_selected_path = Path(worker["selected_output_path"])
    if copied_selected_path.is_absolute():
        copied_selected_path = copied_selected_path.relative_to(capture)
    if (
        len(ambiguous_rows) != 1
        or "not an authorized captured applicability" not in (ambiguous_rows[0].get("diagnostic") or "")
        or {row.get("axis") for row in ambiguous_fresh} != {"fresh-fail", "fresh-pass"}
        or raw_digest != _sha(selected_path.read_bytes())
        or original_spec_digest != _sha(original_spec_path.read_bytes())
        or raw_digest != _sha((ambiguous_capture / copied_selected_path).read_bytes())
    ):
        raise ValueError(f"ambiguous reuse identity was guessed or erased its fresh siblings: {ambiguous_document}")

    # Explicitly apply one source-checked ID, then resume from a fresh full show.
    first = by_axis["fresh-fail"]
    _append(journey, root, database, run_id, first["record_id"], first["kind"], first["data"])
    resumed_show = _show_full(journey, root, database, run_id)
    resumed = _candidate_doc(journey, root, checkout, resumed_show)
    resumed_rows = {row.get("record_id"): row for row in resumed["records"] if row.get("record_id")}
    if resumed_rows[first["record_id"]]["status"] != "already-applied":
        raise ValueError(f"identical source/bytes/target did not resume by exact ID: {resumed_rows[first['record_id']]}")
    second = next(row for row in resumed["records"] if row.get("axis") == "fresh-pass")
    if second["status"] != "ready":
        raise ValueError(f"interrupted sibling was not left ready: {second}")
    _append(journey, root, database, run_id, second["record_id"], second["kind"], second["data"])
    denied = _event(journey, root, database, run_id, "approved", expect="rejected")
    detail = json.dumps(denied)
    if "failed" not in detail.lower() and "missing" not in detail.lower() and "unverified" not in detail.lower():
        raise ValueError(f"checked gate denial omitted the unresolved source obligations: {denied}")

    # A target revision change cannot silently rebind prior selected bytes.
    intent = json.loads(intent_path.read_text(encoding="utf-8"))
    intent["revision"] = str(intent["revision"]) + "-drift"
    _json(intent_path, intent)
    drift_doc = _candidate_doc(journey, root, checkout, _show_full(journey, root, database, run_id))
    if any(record["status"] == "ready" for record in drift_doc.get("records", [])):
        raise ValueError("target drift left stale source candidates ready")

    # A separate public append fixture proves manually altered rows are retained
    # but cannot satisfy their own linked-source obligation.
    forged_axes = ["manual-result", "manual-findings", "manual-author", "manual-gate", "manual-stage", "manual-config", "manual-bytes"]
    forged_spec = root / "forged-batch-spec.json"
    _json(forged_spec, {"author": REVIEWER, "judgments": [{"axis": axis, "result": "pass", "findings": ""} for axis in forged_axes]})
    forged_counter = root / "forged-worker-count"
    forged_run_root, forged_artifacts, forged_checkout, forged_db, forged_run = _write_run(
        journey,
        root,
        "manual-source-forgeries",
        forged_axes,
        {"command": sys.executable, "args": [str(worker_script), "batch", str(forged_counter), str(forged_spec)], "title": "Forged source checks", "role": "reviewer"},
        schema=_review_schema(forged_axes),
    )
    _, forged_show, forged_invocation = _invoke(journey, root, forged_db, forged_run, "intent-review")
    forged_doc = _candidate_doc(journey, root, forged_checkout, forged_show)
    candidates = {row["axis"]: row for row in forged_doc["records"] if row["status"] == "ready"}
    tamper_fields = {
        "manual-result": ("result", "fail"),
        "manual-findings": ("findings", "fabricated finding"),
        "manual-author": ("author", {"name": "different-reviewer", "kind": "agent"}),
        "manual-gate": ("gate", "intent-adversarial-review"),
        "manual-stage": ("review_stage", "individual"),
        "manual-config": ("config_version", "stale-config"),
        "manual-bytes": (None, None),
    }
    for axis, (field, value) in tamper_fields.items():
        candidate = candidates[axis]
        data = copy.deepcopy(candidate["data"])
        if field is not None:
            data[field] = value
            if axis == "manual-result":
                data["findings"] = "forged result does not match raw pass"
        _append(journey, root, forged_db, forged_run, candidate["record_id"], candidate["kind"], data)
    forged_after_append = _candidate_doc(journey, root, forged_checkout, _show_full(journey, root, forged_db, forged_run))
    statuses = {row.get("record_id"): row["status"] for row in forged_after_append["records"] if row.get("record_id")}
    for axis, candidate in candidates.items():
        expected_status = "already-applied" if axis == "manual-bytes" else "conflict"
        if statuses.get(candidate["record_id"]) != expected_status:
            raise ValueError(f"manual append ID had {statuses.get(candidate['record_id'])}, expected {expected_status}: {statuses}")
    forged_worker = forged_invocation["inner_workers"][0]
    forged_capture = Path(forged_invocation["capture_dir"])
    raw_path = Path(forged_worker["selected_output_path"])
    if not raw_path.is_absolute():
        raw_path = forged_capture / raw_path
    raw_path.write_bytes(raw_path.read_bytes() + b"\n")
    changed_source_doc = _candidate_doc(
        journey,
        root,
        forged_checkout,
        _show_full(journey, root, forged_db, forged_run),
    )
    if any(record["status"] in ("ready", "already-applied") for record in changed_source_doc.get("records", [])):
        raise ValueError("changed selected bytes remained resumable after source re-preview")
    forged_denial = _event(journey, root, forged_db, forged_run, "approved", expect="rejected")
    if "unverified" not in json.dumps(forged_denial).lower() and "missing" not in json.dumps(forged_denial).lower():
        raise ValueError(f"changed selected bytes or contradictory manual records did not block gate: {forged_denial}")

    validation_preview = validation_preview_case(journey, root)
    high_rigor_refusals = high_rigor_refusal_case(journey, root)
    proof = {
        "status": "passed",
        "fresh_siblings": sorted(by_axis),
        "independent_invalid_reuse_rows": diagnostics,
        "ambiguous_reuse_refused": True,
        "ambiguous_reuse_diagnostic": ambiguous_rows[0]["diagnostic"],
        "resume_record_id": first["record_id"],
        "stale_target_refused": True,
        "manual_forgery_axes": sorted(tamper_fields),
        "manual_append_gate_refused": True,
        "external_authorship": "declared claim without bound-source origin",
        "changed_source_not_resumable": True,
        "criterion_goal_preview": validation_preview,
        "high_rigor_review_refusals": high_rigor_refusals,
        "full_show_stdout": "not persisted; only byte-count/hash projections retained",
    }
    _json(root / "sol-evidence-proof.json", proof)
    print("sol-evidence passed: row-local reuse, source forgery, and high-rigor stale/self/duplicate/missing-axis checked refusals followed by repair")


def _adapter_config(path: Path, counter: Path) -> dict[str, Any]:
    return {
        "kind": "scripted",
        "command": sys.executable,
        "args": [str(path), str(counter)],
        "max_calls": 1,
        "max_time_ms": 8000,
        "max_cost_micros": 20,
    }


def _preview_in_memory(journey, root: Path, invocation_id: str, show: dict[str, Any]) -> dict[str, Any]:
    argv = [str(journey.engine), "recovery-preview", invocation_id]
    completed = subprocess.run(
        argv,
        cwd=root,
        input=json.dumps(show, separators=(",", ":")).encode(),
        capture_output=True,
        timeout=30,
        check=False,
    )
    _record(root, argv, completed)
    if completed.returncode != 0:
        raise ValueError(f"recovery-preview refused quiescent selected source: {completed.stderr[-1000:]!r}")
    value = json.loads(completed.stdout)
    if value.get("ready") is not True:
        raise ValueError(f"recovery-preview did not retain a ready source: {value}")
    return value


def mixed_review_recovery_case(journey, root: Path, label: str, *, cancel: bool) -> dict[str, Any]:
    """Admit a preserved failed/cancelled sibling and fresh barrier work at a real gate."""
    case = root / label
    case.mkdir()
    script = case / "mixed-reviewer.py"
    script.write_text(
        "import hashlib,json,pathlib,sys,time\n"
        "role=sys.argv[1]; counter=pathlib.Path(sys.argv[2]); marker=pathlib.Path(sys.argv[3]); axes=json.loads(sys.argv[4])\n"
        "packet=json.loads(sys.stdin.buffer.read()); subject=pathlib.Path(packet['artifact_root'])/'intent.json'\n"
        "count=int(counter.read_text()) if counter.exists() else 0; counter.write_text(str(count+1))\n"
        "if role=='b' and count==0:\n"
        " if sys.argv[5]=='cancel': marker.write_text('started\\n'); time.sleep(60)\n"
        " print(json.dumps({'wrong':'first b output lacks the required review envelope'})); raise SystemExit(0)\n"
        "digest='sha256:'+hashlib.sha256(subject.read_bytes()).hexdigest()\n"
        "rows=[{'axis':axis,'result':'pass','findings':'','grounds':{'reason':'Scripted independent review of the current intent source.','evidence':[{'locator':'intent.json#/revision','sha256':digest}]}} for axis in axes]\n"
        "print(json.dumps({'review_contract_version':2,'review_stage':'aggregate','author':{'name':'fixture-'+role,'kind':'script'},'judgments':rows}))\n",
        encoding="utf-8",
    )
    source = json.loads((journey.data_root / "crates/software-change-provider/data/configs/minimal.json").read_text())
    axes = [row["id"] for row in source["review_policies"]["intent-review"]]
    groups = [axes[:2], axes[2:5], axes[5:]]
    counters = {role: case / f"{role}-launches" for role in "abc"}
    marker = case / "b-started"
    workers = []
    for role, assigned in zip("abc", groups):
        schema = _review_schema(assigned, author={"name": f"fixture-{role}", "kind": "script"})
        schema["x-loop-engine-output-recovery"] = "repair-first-v1"
        workers.append({
            "command": sys.executable,
            "args": [str(script), role, str(counters[role]), str(marker), json.dumps(assigned), "cancel" if cancel else "failure"],
            "title": f"Independent scripted review {role}", "role": "reviewer",
            "full_output_schema": schema,
        })
    binding = dogfood_recovery._fanout_binding(journey.engine, workers, max_active=1, split=2)
    _, artifacts, checkout, database, run_id = _write_run(
        journey, case, "software-mixed", axes, workers[0], binding=binding,
    )
    _show_action(journey, case, database, run_id)
    started, _ = _engine(journey, case, database, "--timeout-ms", "120000", "invoke", run_id, "intent-review")
    original_id = started["result"]["invocation_id"]
    if cancel:
        deadline = time.monotonic() + 20
        while not marker.exists() and time.monotonic() < deadline:
            time.sleep(0.05)
        if not marker.exists():
            raise ValueError("mixed software-change cancellation never started the second reviewer")
        _show_action(journey, case, database, run_id)
        overlap, _ = _engine(journey, case, database, "invoke", run_id, "intent-review", expect="rejected")
        if "already-running" not in overlap.get("code", ""):
            raise ValueError(f"mixed software-change live work permitted overlap: {overlap}")
        _show_action(journey, case, database, run_id)
        _engine(journey, case, database, "cancel-invocation", run_id, original_id)
    original_show, original = _wait(journey, case, database, run_id, original_id)
    if original["status"] != "failed" or (cancel and original.get("ownership", {}).get("cleanup_pending") is not False):
        raise ValueError(f"mixed software-change origin was not quiescent and failed: {original}")
    capture = Path(original["capture_dir"])
    original_bytes = dogfood_recovery._capture_manifest(capture)
    original_summary = json.loads((capture / "summary.json").read_text())
    if original_summary["workers"][0].get("status") != "succeeded" or original_summary["workers"][2].get("started") is not False:
        raise ValueError(f"mixed original lost completed sibling or unstarted barrier: {original_summary}")
    preview = dogfood_recovery._preview(journey, case, journey.engine, original_id, original_show)
    by_id = {row["assignment_id"]: row for row in preview["assignments"]}
    if (by_id["worker-0"]["classification"] != "conforming-completed"
            or by_id["worker-2"]["classification"] != "never-started"
            or preview["barriers"]["second_group"] != ["worker-2"]
            or preview["recovery_input"]["pending_assignment_ids"] != ["worker-1", "worker-2"]):
        raise ValueError(f"mixed software-change preview lost source, pending selection or barrier: {preview}")
    if not cancel:
        bad = by_id["worker-1"]
        if (bad["classification"] != "invalid" or bad["exit_code"] != 0
                or bad["conformance_status"] != "failed" or not bad["raw_output"]["available"]):
            raise ValueError(f"exit-zero invalid full-schema review was incorrectly eligible: {bad}")
        forged = copy.deepcopy(preview["recovery_input"])
        forged["pending_assignment_ids"].remove("worker-1")
        forged["sources"].append({
            "assignment_id": "worker-1", "source_class": "original-raw",
            "raw_attempt": bad["raw_output"]["attempt"],
            "raw_stdout_sha256": bad["raw_output"]["sha256"],
        })
        dogfood_recovery._reject_recovery(
            journey, case, database, run_id, "intent-review", forged,
            "not the conforming selected origin output", len(original_show["result"]["work_slot_invocations"]),
        )
    # Outer success alone is not admission: the checked provider gate still
    # needs exact-source append and the new judgments from both pending workers.
    _show_action(journey, case, database, run_id)
    joined_start, _ = _engine(
        journey, case, database, "--timeout-ms", "120000", "invoke", run_id, "intent-review",
        "--input", json.dumps(preview["recovery_input"], separators=(",", ":")),
    )
    joined_show, joined = _wait(journey, case, database, run_id, joined_start["result"]["invocation_id"])
    recovered = {row["assignment_id"]: row for row in joined["inner_workers"]}
    preserved = recovered["worker-0"].get("recovery_source", {})
    if (joined["status"] != "succeeded" or preserved.get("source_class") != "original-raw"
            or preserved.get("origin", {}).get("invocation_id") != original_id
            or recovered["worker-0"].get("started") is not None
            or any(recovered[f"worker-{n}"].get("started") is not True for n in (1, 2))
            or [counters[role].read_text() for role in "abc"] != ["1", "2", "1"]):
        raise ValueError(f"mixed software-change join did not retain original/fresh assignments: {joined}")
    selected = Path(joined["capture_dir"]) / recovered["worker-0"]["selected_output_path"]
    if _sha(selected.read_bytes()) != recovered["worker-0"]["selected_output_sha256"]:
        raise ValueError("preserved sibling's selected bytes differ from the joined capture")
    old = next(row for row in joined_show["result"]["work_slot_invocations"] if row["invocation_id"] == original_id)
    if old["status"] != "failed" or dogfood_recovery._capture_manifest(capture) != original_bytes:
        raise ValueError("mixed join rewrote original failed/cancelled status or raw capture")
    before = _event(journey, case, database, run_id, "approved", expect="rejected")
    if "missing" not in json.dumps(before).lower() and "review" not in json.dumps(before).lower():
        raise ValueError(f"outer success incorrectly passed a gate without evidence: {before}")
    document = _candidate_doc(journey, case, checkout, joined_show)
    rows = [row for row in document["records"] if row.get("status") == "ready"
            and row.get("origin", {}).get("id") == joined["invocation_id"]]
    if (len(rows) != len(axes) or {row["axis"] for row in rows} != set(axes)
            or {row["origin"]["assignment_id"] for row in rows} != {"worker-0", "worker-1", "worker-2"}):
        raise ValueError(f"mixed original/fresh review candidates were not all source-ready: {document}")
    for row in rows:
        _append(journey, case, database, run_id, row["record_id"], row["kind"], row["data"])
    _append(journey, case, database, run_id, "mixed-driver-ledger", "finding-ledger", {
        "schema_version": "1", "gate": "intent-review", "subject": "intent.json",
        "subject_revision": json.loads((artifacts / "intent.json").read_text())["revision"],
        "author": {"name": "fixture-driver", "kind": "agent"}, "findings": [],
    })
    approved = _event(journey, case, database, run_id, "approved")
    final = _show_full(journey, case, database, run_id)["result"]
    if approved["status"] != "completed" or final["current_state"] != "design":
        raise ValueError(f"selected original plus fresh reviews did not pass the real checked gate: {approved}")
    if dogfood_recovery._capture_manifest(capture) != original_bytes:
        raise ValueError("checked admission changed the original failed/cancelled capture")
    return {
        "run_id": run_id, "database": str(database), "origin_invocation": original_id,
        "selected_invocation": joined["invocation_id"], "origin_status": old["status"],
        "cancelled": cancel, "origin_capture": str(capture), "origin_capture_manifest": original_bytes,
        "candidate_record_ids": [row["record_id"] for row in rows],
        "original_assignment_axes": groups[0], "fresh_assignment_axes": groups[1:],
        "gate_denied_without_append": before["code"], "checked_state": final["current_state"],
        "counters": {role: counters[role].read_text() for role in "abc"},
    }


def recovery_case(journey) -> None:
    root = dogfood_observation._fresh_root(journey, "sol-recovery")
    if not shutil.which("dagu"):
        raise ValueError("sol-recovery requires the operator-provided Dagu binary on PATH")
    counter = root / "prose-worker-count"
    worker_script = root / "review-worker.py"
    adapter_script = root / "scripted-format-adapter.py"
    adapter_counter = root / "adapter-call-count"
    _worker(worker_script)
    _adapter(adapter_script)
    axis = "solution-agnostic"
    spec = root / "prose-review-spec.json"
    expected_finding = "the fallback branch has no observable completion signal."
    _json(spec, {
        "author": REVIEWER,
        "axis": axis,
        "finding": expected_finding,
        "reason": "The explicit prose says the fallback result cannot be observed, which blocks the stated operator outcome.",
    })
    required = ["review_contract_version", "review_stage", "author", "axis", "result", "findings", "grounds"]
    run_root, artifacts, checkout, database, run_id = _write_run(
        journey,
        root,
        "scripted-derived-fail",
        [axis],
        {
            "command": sys.executable,
            "args": [str(worker_script), "prose", str(counter), str(spec)],
            "title": "Explicit prose failure requiring representation-only formatting",
            "role": "reviewer",
            "output_schema": {"required": required},
        },
    )
    started, failed_show, failed = _invoke(journey, root, database, run_id, "intent-review")
    if failed["status"] != "failed" or counter.read_text(encoding="utf-8") != "1":
        raise ValueError(f"explicit prose review did not fail once before repair: {failed}")
    original_capture = Path(failed["capture_dir"])
    raw_path = original_capture / "0/stdout"
    raw = raw_path.read_bytes()
    raw_text = raw.decode("utf-8")
    if "Result: fail" not in raw_text or f"Finding: {expected_finding}" not in raw_text:
        raise ValueError("raw review prose did not contain the explicit FAIL and exact finding")
    preview = _preview_in_memory(journey, root, started["invocation_id"], failed_show)
    repair_request = {
        "version": 1,
        "preview": preview,
        "assignment_id": "worker-0",
        "scripted_adapter": _adapter_config(adapter_script, adapter_counter),
    }
    repair_path = root / "recover-output-request.json"
    _json(repair_path, repair_request)
    repaired = dogfood_recovery._repair(journey, root, journey.engine, repair_request, root / "repair-envelope.json")
    derived_bytes = Path(repaired["selected"]["path"]).read_bytes()
    derived = json.loads(derived_bytes)
    if adapter_counter.read_text(encoding="utf-8") != "1" or derived["result"] != "fail" or derived["findings"] != expected_finding:
        raise ValueError(f"scripted formatter did not retain explicit FAIL meaning: {derived}")
    if repaired["origin"]["raw_stdout_sha256"] != _sha(raw) or repaired["selected"]["sha256"] != _sha(derived_bytes):
        raise ValueError("repair did not preserve distinct immutable raw and selected-derived identities")
    recovered_input = dogfood_recovery._attach_derived(preview, repaired)
    same_human_actor = "fixture-human-owner-driver"
    recovered_source = recovered_input["sources"][-1]
    recovered_source["owner_approval"]["name"] = same_human_actor
    recovered_source["owner_approval"]["reason"] = "This fixture human owner authorized scripted mechanics only."
    recovered_source["fidelity_approval"]["name"] = same_human_actor
    recovered_source["fidelity_approval"]["reason"] = "The same fixture human driver compared derived bytes with the original explicit FAIL."
    before_context = failed_show["result"]["context"]
    joined_start, joined_show, joined = _invoke(
        journey, root, database, run_id, "intent-review", recovered_input
    )
    if joined["status"] != "succeeded" or counter.read_text(encoding="utf-8") != "1":
        raise ValueError(f"same-binding recovered join relaunched or failed the reviewer: {joined}")
    joined_worker = joined["inner_workers"][0]
    recovery_source = joined_worker.get("recovery_source") or {}
    if joined_worker.get("selected_attempt") is not None or recovery_source.get("source_class") != "eligible-derived" or recovery_source.get("origin", {}).get("raw_stdout_sha256") != _sha(raw):
        raise ValueError(f"join lost distinct raw/derived identities or source class: {recovery_source}")
    if recovery_source.get("selected_output_sha256") != joined_worker.get("selected_output_sha256") or recovery_source.get("selected_output_path") != joined_worker.get("selected_output_path"):
        raise ValueError("joined derived source does not identify the exact selected bytes")
    if joined_show["result"]["context"] != before_context:
        raise ValueError("mechanical join fabricated or changed run context")
    if raw_path.read_bytes() != raw:
        raise ValueError("output-only repair changed the immutable original raw attempt")

    for label, edit_source, diagnostic_fragment in [
        ("missing-fidelity", lambda source: source.pop("fidelity_approval", None), "fidelity_approval"),
        ("wrong-assignment", lambda source: source["origin"].__setitem__("assignment_id", "worker-forged"), "assignment"),
        ("wrong-binding", lambda source: source["origin"].__setitem__("binding_sha256", "sha256:" + "0" * 64), "binding"),
        ("wrong-subject", lambda source: source["origin"].__setitem__("subject", "design.json"), "subject"),
        ("reviewer-self-triage", lambda source: source["fidelity_approval"].__setitem__("name", REVIEWER["name"]), "declared reviewer"),
    ]:
        forged_show = copy.deepcopy(joined_show)
        forged_invocations = [row for row in forged_show["result"]["work_slot_invocations"] if row["invocation_id"] == joined_start["invocation_id"]]
        if len(forged_invocations) != 1:
            raise ValueError("joined show omitted its unique selected invocation")
        edit_source(forged_invocations[0]["inner_workers"][0]["recovery_source"])
        forged_projection = _candidate_doc(journey, root, checkout, forged_show)
        malformed = [row for row in forged_projection["candidates"] if row.get("status") == "malformed"]
        if len(malformed) != 1 or diagnostic_fragment not in malformed[0].get("diagnostic", ""):
            raise ValueError(f"{label} derived-source metadata was not refused: {forged_projection}")

    document = _candidate_doc(journey, root, checkout, joined_show)
    candidates = [row for row in document["candidates"] if row.get("status") == "ready"]
    records = [row for row in document["records"] if row.get("status") == "ready"]
    if len(candidates) != 1 or candidates[0].get("result") != "fail" or candidates[0].get("findings") != expected_finding:
        raise ValueError(f"derived candidate did not preserve factual FAIL/findings: {document}")
    if len(records) != 1 or records[0]["data"].get("result") != "fail" or records[0]["data"].get("author") != REVIEWER:
        raise ValueError(f"derived factual append preview omitted judgment/source metadata: {document}")
    if records[0]["data"].get("origin") != {
        "kind": "selected-assignment-output",
        "id": joined_start["invocation_id"],
        "assignment_id": "worker-0",
    }:
        raise ValueError(f"candidate source is not the joined selected derived output: {records[0]}")

    current_revision = json.loads((artifacts / "intent.json").read_text(encoding="utf-8"))["revision"]
    _append(journey, root, database, run_id, "derived-initial-ledger", "finding-ledger", {
        "schema_version": "1",
        "gate": "intent-review",
        "subject": "intent.json",
        "subject_revision": current_revision,
        "author": {"name": "fixture-driver", "kind": "agent"},
        "findings": [],
    })
    forged_result = copy.deepcopy(records[0]["data"])
    forged_result["result"] = "pass"
    forged_result["findings"] = ""
    _append(journey, root, database, run_id, "derived-forged-result", "review-evidence", forged_result)
    forged_denial = _event(journey, root, database, run_id, "approved", expect="rejected")
    if "unverified" not in json.dumps(forged_denial).lower():
        raise ValueError(f"manual source-linked derived PASS did not fail as unverified: {forged_denial}")

    _append(journey, root, database, run_id, records[0]["record_id"], "review-evidence", records[0]["data"])
    denied = _event(journey, root, database, run_id, "approved", expect="rejected")
    if "failed" not in json.dumps(denied).lower() and "finding" not in json.dumps(denied).lower():
        raise ValueError(f"successful outer join or formatter success hid the admitted FAIL: {denied}")

    # A driver-authored exact-source rejection discharges the failed judgment;
    # it never rewrites the retained fail to pass.
    source_id = records[0]["record_id"]
    ledger = {
        "schema_version": "1",
        "gate": "intent-review",
        "subject": "intent.json",
        "subject_revision": json.loads((artifacts / "intent.json").read_text(encoding="utf-8"))["revision"],
        "author": {"name": "fixture-driver", "kind": "agent"},
        "findings": [{
            "id": "F-derived-fixture",
            "source": {"kind": "context-record", "id": source_id},
            "policy_id": axis,
            "statement": expected_finding,
            "disposition": "rejected",
            "reason": "The driver inspected this isolated fixture and rejected its finding as non-blocking; the original fail remains unchanged.",
            "owner_phase": None,
            "task_ids": [],
            "review_axes": [],
            "status": "recorded",
        }],
    }
    _append(journey, root, database, run_id, "driver-disposition", "finding-ledger", ledger)
    approved = _event(journey, root, database, run_id, "approved")
    final_show = _show_full(journey, root, database, run_id)
    if approved.get("status") != "completed" or final_show["result"]["current_state"] != "design":
        raise ValueError(f"checked gate did not approve after exact driver disposition: {approved}")
    retained = next(record for record in final_show["result"]["context"] if record["id"] == source_id)
    if retained["data"].get("result") != "fail" or retained["data"].get("findings") != expected_finding:
        raise ValueError("driver disposition rewrote original derived failure evidence")

    original_attach_derived = dogfood_recovery._attach_derived
    full_schema_actor_calls = 0

    def attach_same_human(preview, derived, **options):
        nonlocal full_schema_actor_calls
        value = original_attach_derived(preview, derived, **options)
        source = value["sources"][-1]
        if "owner_approval" in source and "fidelity_approval" in source:
            source["owner_approval"]["name"] = same_human_actor
            source["owner_approval"]["reason"] = "This fixture human owner authorized scripted mechanics only."
            source["fidelity_approval"]["name"] = same_human_actor
            source["fidelity_approval"]["reason"] = "The same fixture human driver checked derived bytes against the original explicit FAIL."
            full_schema_actor_calls += 1
        return value

    dogfood_recovery._attach_derived = attach_same_human
    try:
        full_schema_recovery = dogfood_recovery._software_change_scripted_review_case(journey, root)
    finally:
        dogfood_recovery._attach_derived = original_attach_derived
    if (
        full_schema_actor_calls == 0
        or full_schema_recovery.get("candidate_result") != "fail"
        or full_schema_recovery.get("gate_blocked_before_disposition") is not True
        or full_schema_recovery.get("gate_approved_after_disposition") is not True
        or full_schema_recovery.get("reviewer_launches") != "1"
        or full_schema_recovery.get("adapter_calls", {}).get("faithful") != "1"
    ):
        raise ValueError(f"same-human setup-generated full-schema recovery did not reach factual admission and checked disposition: {full_schema_recovery}")

    model_recovery = dogfood_recovery._software_change_model_repair_case(journey, root)
    model_budget_negatives = dogfood_recovery._software_change_model_budget_negatives(journey, root)
    model_cross_origin_budget = dogfood_recovery._software_change_model_cross_origin_budget_case(journey, root)
    mixed_failed = mixed_review_recovery_case(journey, root, "software-mixed-failed", cancel=False)
    mixed_cancelled = mixed_review_recovery_case(journey, root, "software-mixed-cancelled", cancel=True)

    proof = {
        "status": "passed",
        "raw_attempt_sha256": _sha(raw),
        "derived_output_sha256": repaired["selected"]["sha256"],
        "raw_result": "explicit prose fail",
        "derived_result": derived["result"],
        "joined_selected_source_class": recovery_source["source_class"],
        "same_owner_and_fidelity_human_name": same_human_actor,
        "fidelity_reviewer_independence": same_human_actor != REVIEWER["name"],
        "source_checked_review_candidate": records[0]["record_id"],
        "formatter_success_did_not_approve": True,
        "gate_blocked_before_driver_disposition": True,
        "driver_disposition": "rejected; raw fail retained",
        "gate_approved_after_disposition": True,
        "setup_generated_full_schema_recovery": full_schema_recovery,
        "configured_model_recovery": model_recovery,
        "model_budget_negatives": model_budget_negatives,
        "model_cross_origin_budget": model_cross_origin_budget,
        "software_mixed_failed": mixed_failed,
        "software_mixed_cancelled": mixed_cancelled,
    }
    _json(root / "sol-recovery-proof.json", proof)
    print("sol-recovery passed: scripted and configured-model explicit-prose FAIL retained as derived evidence; model identity/bounds/usage, repeated-budget refusal, and checked-gate denials verified")


def validation_preview_case(journey, root: Path) -> dict[str, Any]:
    """Drive real captured criterion/goal rows through public review-candidates."""
    run_root = root / "validation-verdict-preview"
    run_root.mkdir()
    artifacts = run_root / "artifacts"
    artifacts.mkdir()
    checkout = run_root / "checkout"
    checkout.mkdir()
    subprocess.run(["git", "init", "-q"], cwd=checkout, check=True)
    subprocess.run(["git", "config", "user.email", "fixture@example.test"], cwd=checkout, check=True)
    subprocess.run(["git", "config", "user.name", "Fixture"], cwd=checkout, check=True)
    (checkout / "tracked.txt").write_text("fixture\n", encoding="utf-8")
    subprocess.run(["git", "add", "tracked.txt"], cwd=checkout, check=True)
    subprocess.run(["git", "commit", "-qm", "fixture baseline"], cwd=checkout, check=True)

    for name, document in {
        "intent.json": {"revision": "intent-v1", "acceptance": [{"id": "AC-1", "statement": "The intended outcome is achieved."}]},
        "design.json": {"revision": "design-v1"},
        "plan.json": {"revision": "plan-v1"},
        "implementation-report.json": {"revision": "implementation-v1"},
        "validation-report.json": {
            "revision": "validation-v1",
            "author": {"name": "fixture-driver", "kind": "script"},
            "implementation_revision": "implementation-v1",
            "command_evidence_ids": [],
            "criteria": [{"criterion_id": "AC-1", "verdict_ids": ["criterion-candidate-1"]}],
            "goal_verdict_ids": ["goal-candidate-1"],
        },
    }.items():
        _json(artifacts / name, document)

    author = dict(REVIEWER)
    schema = _review_schema(["validation-axis"], author=author)
    schema["required"].remove("review_contract_version")
    del schema["properties"]["review_contract_version"]
    validation_verdict_schema = {
        "type": "array", "minItems": 2, "maxItems": 2,
        "items": {
            "type": "object", "additionalProperties": False,
            "required": ["record_id", "kind", "data"],
            "properties": {
                "record_id": {"type": "string"},
                "kind": {"type": "string", "enum": ["criterion-verdict", "goal-verdict"]},
                "data": {"type": "object"},
            },
        },
    }
    schema["properties"]["validation_verdicts"] = validation_verdict_schema

    command_spec = {"id": "fixture-proof", "command": "true", "args": [], "owner": "driver", "obligation": "retained fixture outcome"}
    command_capture = artifacts / "proof-capture"
    command_stdout = command_capture / "worker-0/attempts/1/stdout"
    command_stderr = command_capture / "worker-0/attempts/1/stderr"
    command_stdout.parent.mkdir(parents=True)
    command_stderr.write_bytes(b"")
    command_raw = {
        "cwd": str(checkout),
        "spec": command_spec,
        "stdout": "fixture proof passed",
        "stderr": "",
        "elapsed_ms": 1,
        "timed_out": False,
        "spawn_error": None,
        "exit_code": 0,
        "repository_before": "sha256:" + "0" * 64,
        "repository_after": "sha256:" + "0" * 64,
    }
    command_bytes = json.dumps(command_raw, separators=(",", ":")).encode()
    command_stdout.write_bytes(command_bytes)
    _json(command_capture / "summary.json", {"workers": [{
        "assignment_id": "command-0", "exit_code": 0,
        "stdout_path": str(command_stdout),
        "stderr_path": str(command_stderr),
        "selected_output_path": str(command_stdout),
        "selected_output_sha256": _sha(command_bytes), "selected_attempt": 1,
        "args": ["validation-command", str(checkout), json.dumps(command_spec, separators=(",", ":")), "fixture"],
    }]})

    worker = run_root / "validation-reviewer.py"
    worker.write_text(
        "import hashlib,json,pathlib,sys\n"
        "packet=json.loads(sys.stdin.buffer.read()); root=pathlib.Path(packet['artifact_root']); intent=(root/'intent.json').read_bytes(); revision=json.loads((root/'validation-report.json').read_text())['revision']; author={'name':'fixture-reviewer','kind':'agent'}\n"
        "common={'subject':'validation-report.json','subject_revision':revision,'checkpoint':'validation-checkpoint.json','author':author,'result':'pass','findings':[],'reason':'Inspected the retained fixture command evidence.','evidence_context_ids':['fixture-command']}\n"
        "rows=[{'record_id':'criterion-candidate-1','kind':'criterion-verdict','data':{'criterion_id':'AC-1',**common}}, {'record_id':'goal-candidate-1','kind':'goal-verdict','data':dict(common)}]\n"
        "print(json.dumps({'review_stage':'aggregate','author':author,'judgments':[{'axis':'validation-axis','result':'pass','findings':'','grounds':{'reason':'Inspected exact validation fixture.','evidence':[{'locator':'validation-report.json#/revision','sha256':'sha256:'+hashlib.sha256((root/'validation-report.json').read_bytes()).hexdigest()}]}}], 'validation_verdicts':rows},separators=(',',':')))\n",
        encoding="utf-8",
    )
    profile = {
        "artifact_root": str(artifacts),
        "config_version": _minimal_config_version(journey),
        "contract_version": 3,
        "review_policies": {"validation-review": [{"id": "validation-axis", "review_stage": "aggregate", "required_authors": 1}]},
        "work_slot_bindings": {
            "validation-review": dogfood_recovery._fanout_binding(journey.engine, [{
                "command": sys.executable,
                "args": [str(worker)],
                "title": "Captured criterion and goal judgments",
                "role": "reviewer",
                "full_output_schema": schema,
            }], max_active=1)
        },
    }
    initial_path = run_root / "initial-input.json"
    _json(initial_path, profile)
    fake_provider = run_root / "fixture-provider.py"
    fake_provider.write_text(
        "import json,sys\n"
        "q=json.load(sys.stdin)\n"
        "if q.get('operation')=='describe': print(json.dumps({'id':'software-change','initial_state':'validation-review','states':[{'id':'validation-review','title':'Validation review','instructions':'Inspect selected judgments.','final':False},{'id':'done','title':'Done','instructions':'Complete.','final':True}],'transitions':[{'source':'validation-review','event':'approved','target':'done','kind':'checked'}],'work_slots':[{'id':'validation-review','state':'validation-review','event':'approved','stdin_context_kinds':['command-evidence']}]}))\n"
        "else: print(json.dumps({'result':'allow'}))\n",
        encoding="utf-8",
    )
    provider_config = run_root / "providers.toml"
    provider_config.write_text(
        "[providers.software-change]\n"
        f"command = {json.dumps(sys.executable)}\nargs = [{json.dumps(str(fake_provider))}]\n",
        encoding="utf-8",
    )
    database = run_root / "loop.sqlite"
    run_id = "p07-validation-record-preview"
    dogfood_recovery._call(root, [str(journey.engine), "--database", str(database), "--json", "--config", str(provider_config), "start", "--id", run_id, "software-change", "@" + str(initial_path)])
    _append(journey, root, database, run_id, "fixture-command", "command-evidence", {
        "proof_id": "fixture-proof",
        "capture": {"summary": str(command_capture / "summary.json"), "assignment_id": "command-0"},
    })
    for phase in ("implementation", "validation"):
        cp_argv = [
            str(journey.provider), "checkpoint", "--phase", phase,
            "--artifact-root", str(artifacts), "--working-directory", str(checkout),
        ]
        cp = subprocess.run(cp_argv, cwd=checkout, capture_output=True, timeout=30, check=False)
        _record(root, cp_argv, cp)
        if cp.returncode != 0:
            raise ValueError(f"{phase} checkpoint fixture failed: {cp.stderr[-1000:]!r}")
    _show_action(journey, root, database, run_id)
    started, _ = _engine(journey, root, database, "--timeout-ms", "120000", "invoke", run_id, "validation-review")
    show, invocation = _wait(journey, root, database, run_id, started["result"]["invocation_id"])
    if invocation["status"] != "succeeded":
        raise ValueError(f"criterion/goal reviewer did not complete: {invocation}")
    document = _candidate_doc(journey, root, checkout, show)
    verdicts = [row for row in document["records"] if row.get("kind") in ("criterion-verdict", "goal-verdict")]
    if {row["record_id"] for row in verdicts} != {"criterion-candidate-1", "goal-candidate-1"}:
        raise ValueError(f"exact selected criterion/goal candidates were missing: {document}")
    if any(row["status"] != "ready" or row["data"]["result"] != "pass" or row["data"]["author"] != author for row in verdicts):
        raise ValueError(f"criterion/goal preview lost exact source facts: {verdicts}")
    criterion = next(row for row in verdicts if row["kind"] == "criterion-verdict")
    _append(journey, root, database, run_id, criterion["record_id"], criterion["kind"], criterion["data"])
    resumed = _candidate_doc(journey, root, checkout, _show_full(journey, root, database, run_id))
    statuses = {row["record_id"]: row["status"] for row in resumed["records"] if row.get("record_id")}
    if statuses.get(criterion["record_id"]) != "already-applied" or statuses.get("goal-candidate-1") != "ready":
        raise ValueError(f"criterion append did not resume exactly alongside the untouched goal: {statuses}")
    return {"criterion": criterion["record_id"], "goal": "goal-candidate-1", "resumed": True}
