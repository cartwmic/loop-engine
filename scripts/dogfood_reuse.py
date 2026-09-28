"""Focused public proof for future same-run reuse and honest completion paths."""
from __future__ import annotations

import collections
import hashlib
import json
import shutil
import subprocess
import sys
import time
from pathlib import Path
from typing import Any

import dogfood_observation


def _write_json(path: Path, value: Any) -> None:
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


def _checked(response: dict[str, Any], operation: str) -> dict[str, Any]:
    if response.get("status") != "completed":
        raise ValueError(f"{operation} did not complete: {response}")
    result = response.get("result")
    if not isinstance(result, dict):
        raise ValueError(f"{operation} omitted its result object: {response}")
    return result


def _git(root: Path, *args: str) -> str:
    completed = subprocess.run(
        ["git", *args], cwd=root, capture_output=True, text=True, check=False
    )
    if completed.returncode != 0:
        raise ValueError(
            f"fixture git command failed: git {' '.join(args)}: "
            f"{completed.stderr.strip() or completed.stdout.strip()}"
        )
    return completed.stdout.strip()


def _fixture_plan(revision: str, *, reshaped: bool = False) -> dict[str, Any]:
    if not reshaped:
        tasks = [
            {"id": "alpha", "title": "Prepare alpha", "role": "implementer", "dependencies": []},
            {"id": "beta", "title": "Prepare beta", "role": "implementer", "dependencies": []},
            {"id": "gamma", "title": "Prepare gamma", "role": "implementer", "dependencies": []},
            {"id": "merge", "title": "Join alpha beta and gamma", "role": "integrator", "dependencies": ["alpha", "beta", "gamma"]},
        ]
        edges = [
            {"from": "alpha", "to": "merge"},
            {"from": "beta", "to": "merge"},
            {"from": "gamma", "to": "merge"},
        ]
    else:
        tasks = [
            {"id": "alpha", "title": "Prepare changed alpha", "role": "implementer", "dependencies": []},
            {"id": "beta-renamed", "title": "Prepare beta schema", "role": "implementer", "dependencies": []},
            {"id": "beta-split", "title": "Prepare beta index", "role": "implementer", "dependencies": []},
            {"id": "beta-merge", "title": "Merge beta and gamma obligations", "role": "integrator", "dependencies": []},
            {"id": "upstream-new", "title": "New upstream prerequisite", "role": "implementer", "dependencies": []},
            {"id": "merged-new", "title": "Merge revised alpha and beta work", "role": "integrator", "dependencies": ["alpha", "beta-renamed", "beta-split", "beta-merge", "upstream-new"]},
        ]
        edges = [
            {"from": "alpha", "to": "merged-new"},
            {"from": "beta-renamed", "to": "merged-new"},
            {"from": "beta-split", "to": "merged-new"},
            {"from": "beta-merge", "to": "merged-new"},
            {"from": "upstream-new", "to": "merged-new"},
        ]
    return {"revision": revision, "tasks": tasks, "dependency_graph": edges}


def _write_provider(root: Path) -> Path:
    provider = root / "fixture-provider.py"
    provider.write_text(
        "import hashlib,json,sys\n"
        "from pathlib import Path\n"
        "request=json.load(sys.stdin)\n"
        "if request.get('operation')=='describe':\n"
        " initial=request.get('initial_input') or {}\n"
        " enabled='implement' in initial.get('driver_act_slots',[])\n"
        " print(json.dumps({'id':'dogfood-reuse-v2','initial_state':'implement','states':["
        "{'id':'implement','title':'Implement','instructions':'Complete the implementation.','final':False},"
        "{'id':'reconciliation','title':'Reconcile','instructions':'Reconcile current documents.','final':False},"
        "{'id':'post-reconciliation','title':'Post-reconciliation','instructions':'Continue through ordinary review and validation gates.','final':False},"
        "{'id':'done','title':'Done','instructions':'Finished.','final':True}],"
        "'transitions':["
        "{'source':'implement','event':'implementation-ready','target':'reconciliation','kind':'checked'},"
        "{'source':'reconciliation','event':'reconciliation-ready','target':'post-reconciliation','kind':'checked'},"
        "{'source':'reconciliation','event':'revise-implementation','target':'implement','kind':'check-free'},"
        "{'source':'post-reconciliation','event':'revise-implementation','target':'implement','kind':'check-free'},"
        "{'source':'post-reconciliation','event':'finish','target':'done','kind':'checked'}],"
        "'work_slots':[{'id':'implement','state':'implement','event':'implementation-ready',"
        "'driver_act_allowed':enabled,'stdin_context_kinds':['reuse-fixture','reconciliation-decision']}]}))\n"
        "elif request.get('operation')=='evaluate':\n"
        " transition=request['transition']; act=request.get('driver_act')\n"
        " if act:\n"
        "  root=Path(request['initial_input']['artifact_root']); expected=act['request']['unchanged_documents']\n"
        "  for name,key in [('intent.json','intent_revision'),('design.json','design_revision'),('plan.json','plan_revision')]:\n"
        "   if json.loads((root/name).read_text()).get('revision') != expected.get(key):\n"
        "    print(json.dumps({'result':'deny','feedback':{'code':'fixture-driver-act-stale','message':'unchanged accepted document revision differs'}})); raise SystemExit(0)\n"
        " if transition['event']=='reconciliation-ready':\n"
        "  root=Path(request['initial_input']['artifact_root']); raw=(root/'reconciliation.json').read_bytes(); doc=json.loads(raw)\n"
        "  revisions={name:json.loads((root/(name+'.json')).read_text())['revision'] for name in ['intent','design','plan']}\n"
        "  print(json.dumps({'result':'allow','context_append':{'kind':'reconciliation-decision','data':{'revision':doc['revision'],'sha256':'sha256:'+hashlib.sha256(raw).hexdigest(),'documents':revisions}}}))\n"
        " else: print(json.dumps({'result':'allow'}))\n",
        encoding="utf-8",
    )
    return provider


def _write_task_worker(root: Path, counter: Path) -> Path:
    worker = root / "graph-worker.py"
    worker.write_text(
        "import json,sys,time\n"
        "from pathlib import Path\n"
        "counter=Path(sys.argv[1]); raw=sys.stdin.buffer.read().decode()\n"
        "location,body=raw.split('\\n---\\n\\n',1); loc=json.loads(location)\n"
        "if body.startswith('Write artifact_root/implementation-report.json'):\n"
        " plan=json.loads((Path(loc['artifact_root'])/'plan.json').read_text())\n"
        " cap=Path(loc['capture_dir']).name\n"
        " report={'revision':'report-'+cap,'author':{'name':'fixture-summarizer','kind':'script'},"
        "'plan_revision':plan['revision'],'coverage':{'commit':'fixture','documents':[]},"
        "'summary':'fixture report from the bound graph summarizer',"
        "'changed_surface':sorted(p.name for p in Path.cwd().iterdir()),"
        "'validation':[{'proof':'public bound graph fixture'}]}\n"
        " (Path(loc['artifact_root'])/'implementation-report.json').write_text(json.dumps(report)+'\\n')\n"
        "else:\n"
        " task=json.loads(body); task_id=task['id']\n"
        " with counter.open('a') as stream: stream.write(task_id+'\\n')\n"
        " marker=counter.with_suffix('.alpha-delay-used')\n"
        " if task_id=='alpha' and not marker.exists(): time.sleep(1.5); marker.write_text('used')\n"
        " effect=Path.cwd()/('effect-'+task_id+'.txt'); effect.write_text('effect for '+task_id+'\\n')\n"
        " print(json.dumps({'task':task_id,'repository_effect':{'files':[effect.name],"
        "'dependencies':task.get('dependencies',[])}}))\n",
        encoding="utf-8",
    )
    return worker


def _start_invoke(journey, database: Path, run_id: str, *, invocation_input=None) -> dict[str, Any]:
    dogfood_observation._engine(journey, database, "show", "--view", "action", run_id)
    args = ["--timeout-ms", "120000", "invoke", run_id, "implement"]
    if invocation_input is not None:
        args.extend(["--input", json.dumps(invocation_input, separators=(",", ":"))])
    started, _, _ = dogfood_observation._engine(journey, database, *args)
    result = _checked(started, "invoke")
    if not isinstance(result.get("invocation_id"), str):
        raise ValueError(f"invoke omitted its invocation identity: {result}")
    return result


def _invocation_record(journey, database: Path, run_id: str, invocation_id: str) -> dict[str, Any]:
    shown, _, _ = dogfood_observation._engine(journey, database, "show", "--view", "full", run_id)
    projection = _checked(shown, "full show")
    rows = projection.get("work_slot_invocations", [])
    row = next((item for item in rows if item.get("invocation_id") == invocation_id), None)
    if not isinstance(row, dict):
        raise ValueError(f"full show omitted invocation {invocation_id}")
    return row


def _wait_invocation(journey, database: Path, run_id: str, invocation_id: str) -> dict[str, Any]:
    deadline = time.monotonic() + 90
    while time.monotonic() < deadline:
        row = _invocation_record(journey, database, run_id, invocation_id)
        if row.get("status") in ("succeeded", "failed", "overrun"):
            return row
        time.sleep(0.05)
    raise ValueError(f"bound implementation invocation did not reach a terminal status: {invocation_id}")


def _invoke(journey, database: Path, run_id: str, *, invocation_input=None) -> dict[str, Any]:
    started = _start_invoke(journey, database, run_id, invocation_input=invocation_input)
    row = _wait_invocation(journey, database, run_id, started["invocation_id"])
    if row.get("status") != "succeeded":
        raise ValueError(f"bound plan graph did not succeed: {row}")
    return row


def _invoke_refused(journey, database: Path, run_id: str, invocation_input: dict[str, Any], expected: str) -> str:
    dogfood_observation._engine(journey,database,"show","--view","action",run_id)
    response, _, process = dogfood_observation._engine(
        journey,database,"--timeout-ms","120000","invoke",run_id,"implement",
        "--input",json.dumps(invocation_input,separators=(",",":")),expect="any",
    )
    if response.get("status") in ("error","rejected"):
        detail = json.dumps(response) + process.stderr.decode(errors="replace")
    else:
        result = response.get("result") or {}
        invocation_id = result.get("invocation_id")
        if not invocation_id:
            raise ValueError(f"refused invoke returned an unexpected response: {response}")
        row = _wait_invocation(journey,database,run_id,invocation_id)
        if row.get("status") != "failed":
            raise ValueError(f"refused selection unexpectedly reached {row.get('status')}: {row}")
        detail = _capture_error_text(Path(row["capture_dir"]))
    if expected.lower() not in detail.lower():
        raise ValueError(f"invoke refusal omitted `{expected}`: {detail}")
    return detail


def _event(journey, database: Path, run_id: str, event: str, *options: str, expect="completed"):
    dogfood_observation._engine(journey,database,"show","--view","action",run_id)
    return dogfood_observation._engine(journey,database,"event",run_id,event,*options,expect=expect)


def _assert_capture_source(capture_root: Path, expected_ids: list[str]) -> dict[str, Any]:
    summary_bytes = (capture_root / "summary.json").read_bytes()
    summary = json.loads(summary_bytes)
    workers = summary.get("workers")
    if not isinstance(workers, list):
        raise ValueError(f"graph summary omitted task workers: {summary}")
    actual_ids = [row.get("assignment_id") for row in workers]
    if actual_ids != expected_ids:
        raise ValueError(f"graph selected unexpected tasks: {actual_ids} != {expected_ids}")
    proof = summary.get("completion_proof")
    if not isinstance(proof, dict) or not proof.get("checkpoint_sha256") or not proof.get("repository_state_sha256"):
        raise ValueError("completed task source omitted its report/checkpoint/tree identity")
    for row in workers:
        output = Path(row["selected_output_path"])
        try:
            output.resolve().relative_to(capture_root.resolve())
        except ValueError as error:
            raise ValueError(f"selected task source escaped its capture: {output}") from error
        raw = output.read_bytes()
        digest = "sha256:" + hashlib.sha256(raw).hexdigest()
        if row.get("selected_output_sha256") != digest:
            raise ValueError(f"selected task output identity did not match captured bytes: {row}")
        if not row.get("result_id") or row.get("proof") != proof or not row.get("effect_snapshot"):
            raise ValueError(f"task result omitted verifiable source/effect/proof identity: {row}")
        if not isinstance(row.get("dependencies"), list) or not isinstance(row.get("repository_effect"), dict):
            raise ValueError(f"task result omitted dependency/effect dimensions: {row}")
    return {"summary_sha256":"sha256:" + hashlib.sha256(summary_bytes).hexdigest(),"workers":workers,"proof":proof}


def _row_sources(results_path: Path) -> dict[str, dict[str, str]]:
    value = json.loads(results_path.read_text(encoding="utf-8"))
    rows = value.get("results")
    if not isinstance(rows, list):
        raise ValueError("plan-task-results.json omitted its existing task result rows")
    output = {}
    for row in rows:
        task = row.get("assignment_id")
        if task in output or not row.get("result_id") or not row.get("invocation_id"):
            raise ValueError(f"task result lacks a unique original source identity: {row}")
        output[task] = {
            "source_result_id": row["result_id"],
            "source_invocation_id": row["invocation_id"],
            "source_assignment_id": task,
        }
    return output


def _driver_act(*, intent="intent-r1", design="design-r1", plan="plan-r1") -> dict[str, Any]:
    return {
        "author":{"name":"fixture-driver","kind":"script"},
        "reason":"Apply one understood local correction without changing accepted intent, design, or decomposition.",
        "changed_artifacts":["effect-alpha.txt"],
        "unchanged_documents":{"intent_revision":intent,"design_revision":design,"plan_revision":plan},
    }


def _write_reconciliation(artifacts: Path, revision: str) -> None:
    _write_json(artifacts / "reconciliation.json", {
        "revision":revision,
        "author":{"name":"fixture-driver","kind":"script"},
        "mode":"bookends-disabled",
        "branch":"sufficient-existing-wording",
        "document_observations":[{"path":"docs/PRD.md","status":"sufficient","observation":"Fixture current wording is sufficient; no normative edit is claimed."}],
        "behavior_observations":[{"status":"matches-intent","observation":"Scripted fixture only."}],
        "action":"no-document-change",
        "action_reason":"The disposable fixture takes the sufficient-existing-wording branch.",
        "authorization":"not-required",
        "application":"not-required",
        "commit":"not-required",
        "traceability":{"status":"not-applicable","references":[]},
        "proof_references":["scripted-fixture"],
        "blockers":[],
        "decision":"complete",
    })


def _capture_error_text(capture_root: Path) -> str:
    parts = []
    for path in sorted(capture_root.rglob("stderr")):
        try:
            parts.append(path.read_text(encoding="utf-8", errors="replace"))
        except OSError:
            continue
    for path in (capture_root / "summary.json", capture_root / "error.json"):
        try:
            parts.append(path.read_text(encoding="utf-8", errors="replace"))
        except OSError:
            continue
    return "\n".join(parts)


def reuse_case(journey) -> None:
    """Drive mapped reuse, an honest direct act, and actual report-only work."""
    root = dogfood_observation._fresh_root(journey, "sol-reuse")
    if not shutil.which("dagu"):
        raise ValueError("sol-reuse requires the operator-provided Dagu executable on PATH")

    checkout = root / "checkout"
    checkout.mkdir()
    _git(checkout, "init", "-q")
    _git(checkout, "config", "user.name", "Dogfood Fixture")
    _git(checkout, "config", "user.email", "dogfood@example.invalid")
    (checkout / "tracked.txt").write_text("fixture checkout\n", encoding="utf-8")
    _git(checkout, "add", "tracked.txt")
    _git(checkout, "commit", "-q", "-m", "fixture baseline")

    artifacts = root / "artifacts"
    artifacts.mkdir()
    _write_json(artifacts / "intent.json", {"revision":"intent-r1"})
    _write_json(artifacts / "design.json", {"revision":"design-r1"})
    plan_path = artifacts / "plan.json"
    _write_json(plan_path, _fixture_plan("plan-r1"))
    counter = root / "task-launches.txt"
    worker = _write_task_worker(root, counter)
    provider = _write_provider(root)
    provider_config = root / "providers.toml"
    provider_config.write_text(
        "[providers.fixture]\n"
        f"command = {json.dumps(sys.executable)}\n"
        f"args = [{json.dumps(str(provider))}]\n",
        encoding="utf-8",
    )
    worker_cli = {"command":sys.executable,"args":[str(worker),str(counter)]}
    binding = {
        "command":str(journey.provider),
        "args":["run-plan-graph","--working-directory",str(checkout),"--max-active","1",
                "--task-worker",json.dumps(worker_cli,separators=(",",":"))],
    }
    initial = {
        "artifact_root":str(artifacts),
        "work_slot_bindings":{"implement":binding},
        "driver_act_slots":["implement"],
    }
    database = root / "loop.sqlite"
    run_id = "sol-reuse-run"
    started, _, _ = dogfood_observation._engine(
        journey,database,"--config",str(provider_config),"start","--id",run_id,"fixture",
        json.dumps(initial,separators=(",",":")),
    )
    _checked(started,"start public bound-plan fixture")

    first_started = _start_invoke(journey,database,run_id)
    conflicting, _, _ = _event(
        journey,database,run_id,"implementation-ready","--driver-act",
        json.dumps(_driver_act(),separators=(",",":")),expect="any",
    )
    if conflicting.get("status") != "rejected" or conflicting.get("code") != "live-owned-work":
        raise ValueError(f"driver act did not block conflicting owned work: {conflicting}")
    first = _wait_invocation(journey,database,run_id,first_started["invocation_id"])
    if first.get("status") != "succeeded":
        raise ValueError(f"omitted-input full plan did not succeed: {first}")
    first_id = first["invocation_id"]
    first_capture = Path(first["capture_dir"])
    first_source = _assert_capture_source(first_capture,["alpha","beta","gamma","merge"])
    results_path = artifacts / "plan-task-results.json"
    report_path = artifacts / "implementation-report.json"
    checkpoint_path = artifacts / "implementation-checkpoint.json"
    initial_report_revision = json.loads(report_path.read_text())["revision"]
    initial_checkpoint = json.loads(checkpoint_path.read_text())
    if initial_checkpoint["report"]["sha256"] != "sha256:" + hashlib.sha256(report_path.read_bytes()).hexdigest():
        raise ValueError("full-plan report and checkpoint identities differ")

    selected = _invoke(journey,database,run_id,invocation_input={"plan_revision":"plan-r1","task_roots":["beta"]})
    selected_id = selected["invocation_id"]
    selected_capture = Path(selected["capture_dir"])
    selected_source = _assert_capture_source(selected_capture,["beta","merge"])
    worker_packet = json.loads((selected_capture / "ownership" / "worker-packet.json").read_text(encoding="utf-8"))
    if "alpha" not in worker_packet.get("standing_assignment_ids",[]):
        raise ValueError("same-revision selection did not retain its genuine standing prerequisite")
    launches = collections.Counter(counter.read_text(encoding="utf-8").splitlines())
    if launches != collections.Counter({"alpha":1,"beta":2,"gamma":1,"merge":2}):
        raise ValueError(f"same-revision selection ran the wrong tasks: {launches}")

    # The legacy no-task repair mode remains a different route and must not be
    # reinterpreted as report-only work.
    repair_error = _invoke_refused(
        journey,database,run_id,{"repair_finding_ids":["fixture-finding"]},"finding"
    )
    if "repair" not in repair_error.lower() and "finding" not in repair_error.lower():
        raise ValueError("legacy repair mode was not separately refused without real finding context")
    _invoke_refused(
        journey,database,run_id,{"plan_revision":"plan-r1","report_only":True},"reconciliation"
    )
    if counter.read_text(encoding="utf-8").splitlines() != ["alpha","beta","gamma","merge","beta","merge"]:
        raise ValueError("rejected repair/report-only input started a plan task")

    # Resolve the exact existing task-result identities for a later revision.
    old_sources = _row_sources(results_path)
    original_summary_bytes = (first_capture / "summary.json").read_bytes()
    selected_summary_bytes = (selected_capture / "summary.json").read_bytes()
    (checkout / "effect-alpha.txt").write_text("changed upstream effect\n",encoding="utf-8")
    _write_json(artifacts / "intent.json", {"revision":"intent-r2"})
    _write_json(artifacts / "design.json", {"revision":"design-r2"})
    _write_json(plan_path,_fixture_plan("plan-r2",reshaped=True))
    mappings = [
        {**old_sources["beta"],"current_obligations":["beta-renamed","beta-split","beta-merge"],"reason":"The completed beta output still covers these split and renamed obligations."},
        {**old_sources["gamma"],"current_obligations":["beta-merge"],"reason":"The completed gamma output remains one part of the merged obligation."},
        {**old_sources["alpha"],"current_obligations":["alpha"],"reason":"Attempt same-name reuse despite its changed repository effect."},
        {**old_sources["merge"],"current_obligations":["merged-new"],"reason":"Check the merged dependency dimensions against the new upstream task."},
    ]
    before_reuse_launches = counter.read_text(encoding="utf-8").splitlines()
    reshaped = _invoke(journey,database,run_id,invocation_input={
        "plan_revision":"plan-r2","task_roots":["alpha","upstream-new"],"standing_results":mappings,
    })
    reshaped_capture = Path(reshaped["capture_dir"])
    reshaped_source = _assert_capture_source(reshaped_capture,["alpha","upstream-new","merged-new"])
    selection = json.loads((reshaped_capture / "selection.json").read_text(encoding="utf-8"))
    mapped_ids = {row["task_id"] for row in selection.get("standing_results",[])}
    if mapped_ids != {"beta-renamed","beta-split","beta-merge"}:
        raise ValueError(f"valid split/merge/rename mappings were not preserved: {mapped_ids}")
    pending = selection.get("pending_mappings",[])
    pending_by_task = {row.get("task_id"):row for row in pending}
    if "alpha" not in pending_by_task or "repository-effect-changed" not in json.dumps(pending_by_task["alpha"]):
        raise ValueError(f"changed same-name effect was not left pending: {pending}")
    if "merged-new" not in pending_by_task or "dependency-dimensions-changed" not in json.dumps(pending_by_task["merged-new"]):
        raise ValueError(f"changed dependency dimension was not left pending: {pending}")
    launches_after_reuse = collections.Counter(counter.read_text(encoding="utf-8").splitlines())
    if launches_after_reuse != collections.Counter({"alpha":2,"beta":2,"gamma":1,"merge":2,"upstream-new":1,"merged-new":1}):
        raise ValueError(f"mapped replan ran the wrong task set: {launches_after_reuse}")
    if (first_capture / "summary.json").read_bytes() != original_summary_bytes:
        raise ValueError("original full-plan source capture changed during reuse")
    if (selected_capture / "summary.json").read_bytes() != selected_summary_bytes:
        raise ValueError("same-revision source capture changed during reuse")
    mapped_result_file = json.loads(results_path.read_text(encoding="utf-8"))
    if mapped_result_file.get("plan_revision") != "plan-r2" or len(mapped_result_file.get("standing_results",[])) != 3:
        raise ValueError("mapped standing observations were not retained in the existing plan-task-results file")

    # Empty roots stay rejected. A report-only selector is a disjoint future
    # mode, and it still refuses until a current checked decision and re-entry.
    report_before_refusal = report_path.read_bytes()
    checkpoint_before_refusal = checkpoint_path.read_bytes()
    results_before_refusal = results_path.read_bytes()
    for label, value, expected in (
        ("empty roots",{"plan_revision":"plan-r2","task_roots":[]},"task_roots"),
        ("pre-decision report-only",{"plan_revision":"plan-r2","report_only":True},"reconciliation"),
    ):
        _invoke_refused(journey,database,run_id,value,expected)
    if report_path.read_bytes() != report_before_refusal or checkpoint_path.read_bytes() != checkpoint_before_refusal:
        raise ValueError("preflight refusal changed the earlier report/checkpoint")
    if results_path.read_bytes() != results_before_refusal:
        raise ValueError("preflight refusal rewrote the existing task results")

    # Exercise a checked reconciliation, check-free implementation re-entry,
    # explicit non-worker act, and current artifact identity checks.
    _write_reconciliation(artifacts,"reconciliation-r1")
    ordinary_ready, _, _ = _event(journey,database,run_id,"implementation-ready")
    _checked(ordinary_ready,"implementation-ready before act fixture")
    reconciled, _, _ = _event(journey,database,run_id,"reconciliation-ready")
    _checked(reconciled,"first reconciliation-ready")
    revised, _, _ = _event(journey,database,run_id,"revise-implementation")
    _checked(revised,"revise-implementation")
    (checkout / "effect-alpha.txt").write_text("narrow driver correction\n",encoding="utf-8")
    intent_path = artifacts / "intent.json"
    accepted_intent_bytes = intent_path.read_bytes()
    _write_json(intent_path,{"revision":"intent-r3"})
    current_documents = {"intent":"intent-r2","design":"design-r2","plan":"plan-r2"}
    stale_act, _, _ = _event(
        journey,database,run_id,"implementation-ready","--driver-act",
        json.dumps(_driver_act(intent=current_documents["intent"],design=current_documents["design"],plan=current_documents["plan"]),separators=(",",":")),expect="any",
    )
    intent_path.write_bytes(accepted_intent_bytes)
    if stale_act.get("status") != "rejected" or stale_act.get("code") != "fixture-driver-act-stale":
        raise ValueError(f"driver act did not block changed accepted intent identity: {stale_act}")
    launches_before_act = counter.read_bytes()
    direct, _, _ = _event(
        journey,database,run_id,"implementation-ready","--driver-act",
        json.dumps(_driver_act(intent=current_documents["intent"],design=current_documents["design"],plan=current_documents["plan"]),separators=(",",":")),expect="any",
    )
    direct_result = _checked(direct,"driver-act implementation-ready")
    outcome = direct_result.get("history",{}).get("action",{}).get("outcome",{})
    if outcome.get("outcome") != "driver-act":
        raise ValueError(f"driver act was not retained as distinct non-worker history: {direct_result}")
    act = outcome.get("act",{})
    if act.get("request",{}).get("author") != {"name":"fixture-driver","kind":"script"} or not act.get("binding_sha256") or not act.get("instruction_digest"):
        raise ValueError(f"driver act omitted honest authorship/current binding facts: {act}")
    if counter.read_bytes() != launches_before_act:
        raise ValueError("driver act started or forged a bound worker")
    _checked(_event(journey,database,run_id,"reconciliation-ready")[0],"reconciliation-ready after act")
    _checked(_event(journey,database,run_id,"revise-implementation")[0],"second implementation re-entry")
    post_act_roots = _invoke(
        journey,database,run_id,
        invocation_input={"plan_revision":"plan-r2","task_roots":["alpha"]},
    )
    post_act_source = _assert_capture_source(Path(post_act_roots["capture_dir"]),["alpha","merged-new"])
    launches_after_post_act = collections.Counter(counter.read_text(encoding="utf-8").splitlines())
    if launches_after_post_act["alpha"] != launches_after_reuse["alpha"] + 1 or launches_after_post_act["merged-new"] != launches_after_reuse["merged-new"] + 1:
        raise ValueError(f"post-act task-root re-entry did not rerun the affected root and dependant: {launches_after_post_act}")

    # Reconcile the reshaped plan and a disposable authorized docs/Git edit,
    # then run only the frozen summarizer on the same bound graph.
    _write_reconciliation(artifacts,"reconciliation-r2")
    _checked(_event(journey,database,run_id,"implementation-ready")[0],"implementation-ready for final reconciliation")
    _checked(_event(journey,database,run_id,"reconciliation-ready")[0],"final reconciliation-ready")
    docs = checkout / "fixture-docs.md"
    docs.write_text("authorized fixture documentation update\n",encoding="utf-8")
    _git(checkout,"add","fixture-docs.md")
    _git(checkout,"commit","-q","-m","fixture authorized documentation update")
    current_head = _git(checkout,"rev-parse","HEAD")
    _checked(_event(journey,database,run_id,"revise-implementation")[0],"post-decision revise-implementation")
    pre_report_revision = json.loads(report_path.read_text(encoding="utf-8"))["revision"]
    pre_results = results_path.read_bytes()
    before_report_only_launches = counter.read_bytes()
    report_only = _invoke(journey,database,run_id,invocation_input={"plan_revision":"plan-r2","report_only":True})
    report_only_capture = Path(report_only["capture_dir"])
    report_only_summary = json.loads((report_only_capture / "summary.json").read_text(encoding="utf-8"))
    selection = json.loads((report_only_capture / "selection.json").read_text(encoding="utf-8"))
    if selection.get("mode") != "report-only" or selection.get("tasks") != []:
        raise ValueError(f"report-only did not select only the existing summarizer: {selection}")
    if report_only_summary.get("workers") != [] or report_only_summary.get("expected_assignment_ids") != ["summarizer"]:
        raise ValueError(f"report-only launched or recorded plan tasks: {report_only_summary}")
    if counter.read_bytes() != before_report_only_launches:
        raise ValueError("report-only launched a task worker")
    if results_path.read_bytes() != pre_results:
        raise ValueError("report-only rewrote plan-task-results.json")
    report_after = json.loads(report_path.read_text(encoding="utf-8"))
    checkpoint_after = json.loads(checkpoint_path.read_text(encoding="utf-8"))
    if report_after.get("revision") == pre_report_revision or report_after.get("plan_revision") != "plan-r2":
        raise ValueError("report-only did not write a fresh current-plan report")
    if checkpoint_after.get("repository",{}).get("head") != current_head:
        raise ValueError("post-reconciliation checkpoint does not identify the authorized fixture Git change")
    if report_only_summary.get("completion_proof",{}).get("checkpoint_sha256") != "sha256:" + hashlib.sha256(checkpoint_path.read_bytes()).hexdigest():
        raise ValueError("report-only capture omitted the fresh checkpoint identity")

    _checked(_event(journey,database,run_id,"implementation-ready")[0],"post-report implementation-ready")
    reaffirmed, _, _ = _event(journey,database,run_id,"reconciliation-ready")
    _checked(reaffirmed,"post-report reconciliation reaffirmation")
    final_show, _, _ = dogfood_observation._engine(journey,database,"show","--view","full",run_id)
    final_projection = _checked(final_show,"final full show")
    if final_projection.get("current_state") != "post-reconciliation":
        raise ValueError("report-only path did not re-enter the post-reconciliation checked state")

    outcome = {
        "schema_version":1,
        "case":"sol-reuse",
        "status":"completed-scripted-future-paths",
        "run_id":run_id,
        "database":str(database),
        "first_invocation":first_id,
        "same_revision_selection_invocation":selected_id,
        "reshaped_task_roots_invocation":reshaped["invocation_id"],
        "post_act_task_roots_invocation":post_act_roots["invocation_id"],
        "report_only_invocation":report_only["invocation_id"],
        "observed":[
            "original task results retained invocation, stdout, result, repository-effect, report, checkpoint, and tree identities",
            "same-revision task-root selection reused real standing prerequisites and ran only the selected task and dependants",
            "explicit source-result mappings preserved sound renamed/split/merged obligations while changed same-name effects and changed dependencies remained pending and their affected roots/dependants ran",
            "opt-in driver act retained driver authorship, current visit/instruction/binding identity, rejected changed accepted intent, and did not start a worker",
            "live owned invocation blocked the driver act before provider progression",
            "post-act task-root re-entry reran only the affected task and dependant before the post-decision report-only case",
            "post-reconciliation report-only verified current standing tasks, ran only the same bound summarizer, wrote a fresh report/checkpoint after the fixture Git change, preserved task results, and re-entered the next checked post-reconciliation state",
            "empty task_roots remained rejected and legacy repair_finding_ids remained a separate refusal path",
            "no independent reviewer or semantic approval was simulated; ordinary review remains downstream and pending",
        ],
        "assertions":{
            "mapped_standing_tasks":sorted(mapped_ids),
            "pending_tasks":sorted(pending_by_task),
            "task_launches_after_mapped_reuse":dict(launches_after_reuse),
            "task_launches_after_driver_act_reentry":dict(launches_after_post_act),
            "final_tree_head":current_head,
            "report_revision_before_report_only":pre_report_revision,
            "report_revision_after_report_only":report_after["revision"],
        },
        "original_capture_summary_sha256":first_source["summary_sha256"],
        "same_revision_capture_summary_sha256":selected_source["summary_sha256"],
        "reshaped_capture_summary_sha256":reshaped_source["summary_sha256"],
        "semantic_review":"not simulated; current independent review and final semantic AC-6/AC-9 judgment remain driver-owned",
    }
    _write_json(root / "outcome.json",outcome)
    print("sol-reuse public mapped reuse, driver act, and post-reconciliation report-only paths passed")
