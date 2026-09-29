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


def _write_task_worker(root: Path, counter: Path, *, software: bool = False) -> Path:
    worker = root / "graph-worker.py"
    report_line = (
        " report={'revision':'report-'+cap,'author':{'name':'fixture-summarizer','kind':'agent'},"
        "'plan_revision':plan['revision'],'coverage':{'commit':subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(),"
        "'documents':[{'path':'.github/workflows/test.yml','revision':plan['revision']}]},"
        "'summary':'Scripted graph completion for the current plan and selected repository tree.',"
        "'changed_surface':['effect-alpha.txt','behavior.py','.github/workflows/test.yml'],"
        "'validation':[{'criterion_id':'AC-1','proof':'The named captured check observes the current effect and workflow command.'}]}\n"
        if software else
        " report={'revision':'report-'+cap,'author':{'name':'fixture-summarizer','kind':'script'},"
        "'plan_revision':plan['revision'],'coverage':{'commit':'fixture','documents':[]},"
        "'summary':'fixture report from the bound graph summarizer',"
        "'changed_surface':sorted(p.name for p in Path.cwd().iterdir()),"
        "'validation':[{'proof':'public bound graph fixture'}]}\n"
    )
    effect_line = (
        " effect=Path.cwd()/('effect-'+task_id+'.txt')\n"
        " if not (task_id=='alpha' and effect.exists() and effect.read_text()=='narrow driver correction\\n'): effect.write_text('effect for '+task_id+'\\n')\n"
        if software else
        " effect=Path.cwd()/('effect-'+task_id+'.txt'); effect.write_text('effect for '+task_id+'\\n')\n"
    )
    worker.write_text(
        "import json,sys,time,subprocess\n"
        "from pathlib import Path\n"
        "counter=Path(sys.argv[1]); raw=sys.stdin.buffer.read().decode()\n"
        "location,body=raw.split('\\n---\\n\\n',1); loc=json.loads(location)\n"
        "if body.startswith('Write artifact_root/implementation-report.json'):\n"
        " plan=json.loads((Path(loc['artifact_root'])/'plan.json').read_text())\n"
        " cap=Path(loc['capture_dir']).name\n"
        + report_line
        + " (Path(loc['artifact_root'])/'implementation-report.json').write_text(json.dumps(report)+'\\n')\n"
        "else:\n"
        " task=json.loads(body); task_id=task['id']\n"
        " with counter.open('a') as stream: stream.write(task_id+'\\n')\n"
        " marker=counter.with_suffix('.alpha-delay-used')\n"
        " if task_id=='alpha' and not marker.exists(): time.sleep(1.5); marker.write_text('used')\n"
        + effect_line
        + " print(json.dumps({'task':task_id,'repository_effect':{'files':[effect.name],"
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


def _software_change_completion(journey, root: Path) -> dict[str, Any]:
    """Compose real revised-plan reuse, direct correction and fresh checked review."""
    import dogfood_evidence
    import dogfood_recovery
    import importlib.util

    case = root / "software-change-completion"
    case.mkdir()
    checkout = case / "checkout"
    checkout.mkdir()
    _git(checkout, "init", "-q")
    _git(checkout, "config", "user.name", "Reuse Fixture")
    _git(checkout, "config", "user.email", "reuse@example.invalid")
    workflow = checkout / ".github/workflows/test.yml"
    workflow.parent.mkdir(parents=True)
    workflow.write_text("run: false\n", encoding="utf-8")
    (checkout / "tracked.txt").write_text("fixture\n", encoding="utf-8")
    behavior = checkout / "behavior.py"
    behavior.write_text(
        "def alpha_effect():\n"
        "    return 'changed upstream effect\\n'\n"
        "\n"
        "if __name__ == '__main__':\n"
        "    print(alpha_effect(), end='')\n",
        encoding="utf-8",
    )
    (checkout / "proof.py").write_text(
        "from pathlib import Path\n"
        "from behavior import alpha_effect\n"
        "assert alpha_effect() == 'narrow driver correction\\n'\n"
        "assert Path('effect-alpha.txt').read_text() == alpha_effect()\n"
        "assert Path('effect-merged-new.txt').is_file()\n"
        "print('assertion: corrected code effect and revised dependant are current')\n",
        encoding="utf-8",
    )
    (checkout / ".gitignore").write_text("commands/\n", encoding="utf-8")
    _git(checkout, "add", ".")
    _git(checkout, "commit", "-q", "-m", "fixture baseline")
    artifacts = case / "artifacts"
    artifacts.mkdir()
    fixtures = journey.data_root / "crates/software-change-provider/data/calibration/fixtures"
    intent = json.loads((fixtures / "intent-good.json").read_text())
    intent.update(revision="intent-r1", author={"name":"fixture-intent-author","kind":"agent"},
                  problem="The operator needs to retain applicable work after revising a plan.",
                  outcome="A revised plan completes with a selected code and workflow correction, current proof, and fresh review.",
                  acceptance=[{"id":"AC-1", "statement":"The operator sees the revised selected tasks, corrected code and workflow command in a captured current check and reviewed terminal state."}],
                  constraints=["Preserve old captures and do not treat a scripted judgment as semantic approval."],
                  non_goals=["No cross-run reuse or migration of the frozen main run."])
    intent["operating_context"]["outside_obligations"] = [{
        "source":"Fixture checkout and the selected production software-change graph",
        "obligation":"Keep selected result lineage, captured checks, and independent review current."}]
    design = json.loads((fixtures / "design-good.json").read_text())
    design.update(revision="design-r1", intent_revision="intent-r1",
                  author={"name":"fixture-design-author","kind":"agent"},
                  approach="Reuse only explicitly mapped applicable task effects; refresh affected graph, report, checkpoint, captured proof and independent review.",
                  coverage=[{"criterion_id":"AC-1","acceptance":intent["acceptance"][0]["statement"],
                             "delivered_by":"The bound graph verifies mapped standing tasks and reruns changed effects; fixture workflow command and code are checked through retained proof."}])
    for name, document in (("intent", intent), ("design", design)):
        _write_json(artifacts / f"{name}.json", document)
    check = (
        "from pathlib import Path; import shlex,subprocess; "
        f"root=Path({str(checkout)!r}); "
        "lines=(root/'.github/workflows/test.yml').read_text().splitlines(); "
        "assert len(lines)==1 and lines[0].startswith('run: '),lines; "
        "argv=shlex.split(lines[0][5:]); "
        "p=subprocess.run(argv,cwd=root,capture_output=True,text=True); "
        "assert p.returncode==0 and 'assertion: corrected code effect' in p.stdout,(argv,p.stdout,p.stderr); "
        "print('assertion: revised alpha/dependant and corrected workflow command run successfully: '+repr(argv))"
    )
    proof_command = {"id":"selected-reuse-check","command":sys.executable,"args":["-c",check],
                     "owner":"fixture-driver","obligation":"Assert revised task effect and corrected workflow command on the final repository tree."}

    def plan(revision: str, *, reshaped: bool = False, design_revision: str = "design-r1") -> None:
        graph = _fixture_plan(revision, reshaped=reshaped)
        tasks = [{"id":item["id"], "objective":item["title"],
                  "dependencies":item["dependencies"],
                  "source_of_truth":["intent.json#/acceptance/0", "design.json#/coverage/0"],
                  "deliverables":["A selected fixture effect on the current checkout"],
                  "out_of_scope":["No external repository or human semantic approval"],
                  "validation":["selected-reuse-check asserts current code and workflow effects"],
                  "handoff":"Retain the actual task result and its repository-effect/dependency dimensions.",
                  "criterion_ids":["AC-1"], "proof_command_ids":["selected-reuse-check"]}
                 for item in graph["tasks"]]
        _write_json(artifacts / "plan.json", {"revision":revision,
            "author":{"name":"fixture-plan-author","kind":"agent"},
            "design_revision":design_revision,
            "objective":"Complete only affected task roots after explicit same-run standing mappings.",
            "tasks":tasks,"dependency_graph":graph["dependency_graph"],
            "proof_commands":[proof_command]})

    plan("plan-r1")
    counter = case / "task-launches.txt"
    worker = _write_task_worker(case, counter, software=True)
    review_counter = case / "review-launches.txt"
    reviewer = case / "scripted-reviewer.py"
    reviewer.write_text(
        "import hashlib,json,pathlib,sys\n"
        "gate,subject,axes,counter=sys.argv[1],sys.argv[2],json.loads(sys.argv[3]),pathlib.Path(sys.argv[4])\n"
        "raw=sys.stdin.buffer.read().decode(); location=json.loads(raw.split('\\n---\\n\\n',1)[0].splitlines()[-1])\n"
        "path=pathlib.Path(location['artifact_root'])/subject; doc=json.loads(path.read_text()); digest='sha256:'+hashlib.sha256(path.read_bytes()).hexdigest()\n"
        "with counter.open('a') as stream: stream.write(gate+':'+doc['revision']+'\\n')\n"
        "judgments=[{'axis':axis,'result':'pass','findings':'','grounds':{'reason':'Scripted reviewer inspected the selected current artifact; semantic quality is not claimed.','evidence':[{'locator':subject+'#/revision','sha256':digest}]}} for axis in axes]\n"
        "print(json.dumps({'review_contract_version':2,'review_stage':'aggregate','author':{'name':'fixture-'+gate,'kind':'script'},'judgments':judgments}))\n",
        encoding="utf-8",
    )
    profile = json.loads((journey.data_root / "crates/software-change-provider/data/configs/minimal.json").read_text())
    bindings = {}
    for gate, policies in profile["review_policies"].items():
        if not policies:
            continue
        subject = dogfood_advice_subject(gate)
        axes = [row["id"] for row in policies]
        author = {"name":f"fixture-{gate}","kind":"script"}
        nested = {"command":sys.executable,
                  "args":[str(reviewer),gate,subject,json.dumps(axes),str(review_counter)],
                  "title":f"Fresh scripted {gate} review", "role":"reviewer",
                  "full_output_schema":dogfood_evidence._review_schema(axes,author=author)}
        bindings[gate] = dogfood_recovery._fanout_binding(journey.engine,[nested],max_active=1)
    bindings["implement"] = {"command":str(journey.provider),
        "args":["run-plan-graph","--working-directory",str(checkout),"--max-active","1",
                "--task-worker",json.dumps({"command":sys.executable,"args":[str(worker),str(counter)]},separators=(",",":"))]}
    profile.update(artifact_root=str(artifacts),work_slot_bindings=bindings,driver_act_slots=["implement"])
    profile_path = case / "profile.json"
    _write_json(profile_path,profile)
    config = case / "providers.toml"
    config.write_text("[providers.software-change]\n"
        f"command = {json.dumps(str(journey.provider))}\nargs = []\n",encoding="utf-8")
    database = case / "loop.sqlite"
    run_id = "sol-reuse-software-completed"
    # Provider evaluation resolves checkpoint Git identity from engine CWD.
    # Keep command captures ignored inside this disposable checkout.
    original_case_root = journey._dogfood_case_root
    journey._dogfood_case_root = checkout
    _checked(dogfood_observation._engine(
        journey,database,"--config",str(config),"start","--id",run_id,
        "software-change","@"+str(profile_path))[0],"production software-change start")

    def event(name: str, expected: str) -> dict[str, Any]:
        response, _, _ = _event(journey,database,run_id,name)
        result = _checked(response,name)
        if result.get("run",{}).get("current_state") != expected:
            raise ValueError(f"production {name} did not enter {expected}: {response}")
        return result

    terminal_denial: dict[str, Any] | None = None

    def review(gate: str, target: str, *, terminal_missing_axis: str | None = None) -> str:
        nonlocal terminal_denial
        invoked = _start_review(journey,database,run_id,gate)
        row = _wait_invocation(journey,database,run_id,invoked)
        if row["status"] != "succeeded":
            raise ValueError(f"scripted {gate} worker did not complete: {row}")
        full, _, _ = dogfood_observation._engine(journey,database,"show","--view","full",run_id)
        document = dogfood_evidence._candidate_doc(journey,case,checkout,full)
        ready = [item for item in document["records"] if item.get("status") == "ready"
                 and item.get("gate") == gate and item.get("origin",{}).get("id") == invoked]
        axes = [policy["id"] for policy in profile["review_policies"][gate]]
        if len(ready) != len(axes) or {item["axis"] for item in ready} != set(axes):
            raise ValueError(f"real {gate} review lacked exact fresh captured axes: {document}")
        if terminal_missing_axis is not None and (gate != "validation-adversarial-review"
                or terminal_missing_axis not in axes):
            raise ValueError("terminal deficit must name a configured validation-adversarial axis")
        for item in ready:
            if item["axis"] == terminal_missing_axis:
                continue
            dogfood_observation._engine(journey,database,"show","--view","action",run_id)
            _checked(dogfood_observation._engine(journey,database,"append","--record-id",item["record_id"],
                run_id,item["kind"],json.dumps(item["data"],separators=(",",":")))[0],"review append")
        subject = dogfood_advice_subject(gate)
        revision = json.loads((artifacts / subject).read_text())["revision"]
        ledger = {"schema_version":"1","gate":gate,"subject":subject,
                  "subject_revision":revision,"author":{"name":"fixture-driver","kind":"agent"},
                  "findings":[]}
        dogfood_observation._engine(journey,database,"show","--view","action",run_id)
        _checked(dogfood_observation._engine(journey,database,"append","--record-id",
            f"ledger-{gate}-{invoked}",run_id,"finding-ledger",
            json.dumps(ledger,separators=(",",":")))[0],"driver ledger append")
        if terminal_missing_axis is not None:
            # All command, criterion, goal and ledger prerequisites are already
            # present. Refuse the actual final edge for just this missing axis.
            denied, _, _ = _event(journey,database,run_id,"passed",expect="any")
            detail = json.dumps(denied)
            state = _checked(dogfood_observation._engine(
                journey,database,"show","--view","action",run_id)[0],"terminal denial show")
            if (denied.get("status") != "rejected"
                    or denied.get("code") != "software-change-review-incomplete"
                    or terminal_missing_axis not in detail
                    or state.get("current_state") != gate):
                raise ValueError(f"missing configured terminal review axis did not cause a specific unchanged-state refusal: {denied}")
            terminal_denial = denied
            _write_json(case / "terminal-missing-axis-denial.json",denied)
            missing = next(item for item in ready if item["axis"] == terminal_missing_axis)
            _checked(dogfood_observation._engine(journey,database,"append","--record-id",missing["record_id"],
                run_id,missing["kind"],json.dumps(missing["data"],separators=(",",":")))[0],"missing terminal review repair")
        event("approved" if gate != "validation-adversarial-review" else "passed",target)
        return invoked

    def early_reviews() -> None:
        event("intent-ready","intent-review")
        review("intent-review","intent-adversarial-review")
        review("intent-adversarial-review","design")
        event("design-ready","design-review")
        review("design-review","design-adversarial-review")
        review("design-adversarial-review","plan")
        event("plan-ready","plan-review")
        review("plan-review","plan-adversarial-review")
        review("plan-adversarial-review","implement")

    def reconciliation(revision: str, *, corrected: bool = False, write: bool = True) -> None:
        document = {
            "revision":revision,"author":{"name":"fixture-driver","kind":"script"},
            "mode":"bookends-disabled","branch":"change-specific-proof",
            "document_observations":[{"path":".github/workflows/test.yml","status":"unrelated",
                "observation":"The fixture workflow command is change-specific; no normative PRD edit is claimed."}],
            "behavior_observations":[{"status":"change-specific",
                "observation":("The corrected code effect and repository workflow command are now named in the selected public check."
                               if corrected else "Selected graph effects and captured workflow command have an operator-visible check.")}],
            "action":"no-document-change","action_reason":(
                "Corrected the prior fixture reconciliation explanation to name the actual code and workflow checks; no enduring requirement changed."
                if corrected else "The fixture adds no enduring requirement."),
            "authorization":"not-required","application":"not-required","commit":"not-required",
            "traceability":{"status":"not-applicable","references":[]},
            "proof_references":["selected-reuse-check"],"blockers":[],"decision":"complete"}
        if write:
            _write_json(artifacts / "reconciliation.json",document)
        elif json.loads((artifacts / "reconciliation.json").read_text()) != document:
            raise ValueError("driver-corrected reconciliation artifact changed before its checked decision")
        event("reconciliation-ready","implementation-review")

    early_reviews()
    first = _invoke(journey,database,run_id)
    old_capture = Path(first["capture_dir"])
    old_source = _assert_capture_source(old_capture,["alpha","beta","gamma","merge"])
    old_bytes = (old_capture / "summary.json").read_bytes()
    sources = _row_sources(artifacts / "plan-task-results.json")
    event("implementation-ready","reconciliation")
    reconciliation("recon-r1")
    initial_impl_review = review("implementation-review","implementation-adversarial-review")
    event("revise-intent","explore")
    intent["revision"] = "intent-r2"
    design.update(revision="design-r2",intent_revision="intent-r2")
    _write_json(artifacts / "intent.json",intent)
    _write_json(artifacts / "design.json",design)
    plan("plan-r2",reshaped=True,design_revision="design-r2")
    early_reviews()
    (checkout / "effect-alpha.txt").write_text("changed upstream effect\n",encoding="utf-8")
    mappings = [
        {**sources["beta"],"current_obligations":["beta-renamed","beta-split","beta-merge"],
         "reason":"Checked matching beta result effects and retained split/rename obligations."},
        {**sources["gamma"],"current_obligations":["beta-merge"],
         "reason":"Checked matching gamma result as one part of merged obligations."},
        {**sources["alpha"],"current_obligations":["alpha"],
         "reason":"Test changed upstream effect; this mapping must stay pending."},
        {**sources["merge"],"current_obligations":["merged-new"],
         "reason":"Check changed dependency dimension against the new upstream task."},
    ]
    revised = _invoke(journey,database,run_id,invocation_input={
        "plan_revision":"plan-r2","task_roots":["alpha","upstream-new"],
        "standing_results":mappings})
    revised_capture = Path(revised["capture_dir"])
    _assert_capture_source(revised_capture,["alpha","upstream-new","merged-new"])
    selection = json.loads((revised_capture / "selection.json").read_text())
    if {row["task_id"] for row in selection["standing_results"]} != {"beta-renamed","beta-split","beta-merge"}:
        raise ValueError("production revised graph lost mapped standing obligations")
    pending = {row["task_id"] for row in selection["pending_mappings"]}
    if pending != {"alpha","merged-new"} or (old_capture / "summary.json").read_bytes() != old_bytes:
        raise ValueError(f"revised graph relabeled changed effects/dependencies or rewrote original: {pending}")
    event("implementation-ready","reconciliation")
    reconciliation("recon-r2")
    event("revise","implement")
    # Run the defective executable itself before changing either its code or
    # the workflow. The proof's expected alpha value stays fixed throughout.
    defective_argv = [sys.executable, "behavior.py"]
    defective = subprocess.run(defective_argv,cwd=checkout,capture_output=True,check=False)
    (case / "before-code.stdout").write_bytes(defective.stdout)
    (case / "before-code.stderr").write_bytes(defective.stderr)
    _write_json(case / "before-code.command.json",{
        "argv":defective_argv,"cwd":str(checkout),"exit_code":defective.returncode,
        "stdout_sha256":"sha256:"+hashlib.sha256(defective.stdout).hexdigest()})
    if defective.returncode != 0 or defective.stdout != b"changed upstream effect\n":
        raise ValueError("defective executable did not exhibit its wrong alpha behavior")
    oracle_argv = [sys.executable, "-B", "proof.py"]
    oracle_failure = subprocess.run(oracle_argv,cwd=checkout,capture_output=True,check=False)
    (case / "before-oracle.stdout").write_bytes(oracle_failure.stdout)
    (case / "before-oracle.stderr").write_bytes(oracle_failure.stderr)
    _write_json(case / "before-oracle.command.json",{
        "argv":oracle_argv,"cwd":str(checkout),"exit_code":oracle_failure.returncode,
        "stderr_sha256":"sha256:"+hashlib.sha256(oracle_failure.stderr).hexdigest()})
    if oracle_failure.returncode == 0 or b"AssertionError" not in oracle_failure.stderr:
        raise ValueError("defective executable did not fail the unchanged code oracle")
    negative_argv = [sys.executable,"-c",check]
    negative = subprocess.run(negative_argv,cwd=checkout,capture_output=True,check=False)
    (case / "before-correction.stdout").write_bytes(negative.stdout)
    (case / "before-correction.stderr").write_bytes(negative.stderr)
    _write_json(case / "before-correction.command.json",{
        "argv":negative_argv,"cwd":str(checkout),"exit_code":negative.returncode})
    if negative.returncode == 0:
        raise ValueError("uncorrected code/workflow unexpectedly passed the selected final proof")
    # The plan/decomposition and proof oracle stay fixed. Correct the actual
    # executable, effect and workflow command in this disposable checkout.
    behavior.write_text(behavior.read_text().replace(
        "return 'changed upstream effect\\n'", "return 'narrow driver correction\\n'"),encoding="utf-8")
    (checkout / "effect-alpha.txt").write_text("narrow driver correction\n",encoding="utf-8")
    workflow.write_text("run: python3 -B proof.py\n",encoding="utf-8")
    # This is the driver-owned Loop reconciliation artifact (not merely CI
    # configuration). The prior checked r2 decision stays in engine history;
    # the new r3 decision will be checked on the re-entered reconciliation visit.
    corrected_reconciliation = artifacts / "reconciliation.json"
    previous_reconciliation = corrected_reconciliation.read_bytes()
    (case / "reconciliation-before-driver-act.json").write_bytes(previous_reconciliation)
    revised_decision = json.loads(previous_reconciliation)
    revised_decision.update(revision="recon-r3",
        behavior_observations=[{"status":"change-specific",
            "observation":"The corrected code effect and repository workflow command are now named in the selected public check."}],
        action_reason="Corrected the prior fixture reconciliation explanation to name the actual code and workflow checks; no enduring requirement changed.")
    _write_json(corrected_reconciliation,revised_decision)
    (case / "reconciliation-after-driver-act.json").write_bytes(corrected_reconciliation.read_bytes())
    if corrected_reconciliation.read_bytes() == previous_reconciliation:
        raise ValueError("driver act did not correct the actual Loop reconciliation artifact")
    corrected = _invoke(journey,database,run_id,invocation_input={
        "plan_revision":"plan-r2","task_roots":["alpha"]})
    _assert_capture_source(Path(corrected["capture_dir"]),["alpha","merged-new"])
    launch_bytes = counter.read_bytes()
    act = _driver_act(intent="intent-r2",design="design-r2",plan="plan-r2")
    act["changed_artifacts"] = ["behavior.py", "effect-alpha.txt", ".github/workflows/test.yml", "reconciliation.json"]
    act["reason"] = "Correct the executable alpha behavior, its effect and the driver-owned Loop reconciliation decision plus the fixture workflow command, without altering accepted intent, design, or task decomposition."
    intent_path = artifacts / "intent.json"
    accepted_intent = intent_path.read_bytes()
    changed_intent = dict(intent, revision="intent-r3")
    _write_json(intent_path,changed_intent)
    stale_act, _, _ = _event(journey,database,run_id,"implementation-ready","--driver-act",
                                json.dumps(act,separators=(",",":")),expect="any")
    intent_path.write_bytes(accepted_intent)
    if (stale_act.get("status") != "rejected"
            or stale_act.get("code") != "software-change-driver-act-invalid"
            or "intent.json revision changed" not in json.dumps(stale_act)):
        raise ValueError(f"production provider did not refuse stale accepted intent on driver act: {stale_act}")
    direct, _, _ = _event(journey,database,run_id,"implementation-ready","--driver-act",
                          json.dumps(act,separators=(",",":")),expect="any")
    direct_history = _checked(direct,"production driver-act implementation-ready").get("history",{}).get("action",{}).get("outcome",{})
    if (direct_history.get("outcome") != "driver-act" or counter.read_bytes() != launch_bytes
            or intent_path.read_bytes() != accepted_intent):
        raise ValueError("production driver act was not distinct from a bound task execution on unchanged documents")
    reconciliation("recon-r3",corrected=True,write=False)
    review("implementation-review","implementation-adversarial-review")
    review("implementation-adversarial-review","validation")
    (checkout / "fixture-docs.md").write_text("Authorized fixture documentation refresh\n",encoding="utf-8")
    _git(checkout,"add","fixture-docs.md")
    _git(checkout,"commit","-q","-m","fixture documentation refresh")
    current_head = _git(checkout,"rev-parse","HEAD")
    event("revise-implementation","implement")
    before_report = (artifacts / "implementation-report.json").read_bytes()
    before_results = (artifacts / "plan-task-results.json").read_bytes()
    before_launches = counter.read_bytes()
    report_only = _invoke(journey,database,run_id,invocation_input={
        "plan_revision":"plan-r2","report_only":True})
    report_capture = Path(report_only["capture_dir"])
    report_summary = json.loads((report_capture / "summary.json").read_text())
    if (report_summary["workers"] != [] or report_summary["expected_assignment_ids"] != ["summarizer"]
            or counter.read_bytes() != before_launches
            or (artifacts / "plan-task-results.json").read_bytes() != before_results
            or (artifacts / "implementation-report.json").read_bytes() == before_report):
        raise ValueError("real post-reconciliation report-only path replayed tasks or did not refresh proof")
    checkpoint = json.loads((artifacts / "implementation-checkpoint.json").read_text())
    if checkpoint["repository"]["head"] != current_head:
        raise ValueError("real report-only checkpoint omitted current fixture Git identity")
    event("implementation-ready","reconciliation")
    reconciliation("recon-r4",corrected=True)
    final_impl_review = review("implementation-review","implementation-adversarial-review")
    review("implementation-adversarial-review","validation")
    # The fixture helper executes the plan's real selected command through
    # common capture, then supplies a fixed index and scripted independent
    # criterion/goal records. No semantic verdict is inferred from that shape.
    helper = Path(__file__).resolve().parents[1] / "tests/fixtures/prepare-validation.py"
    spec = importlib.util.spec_from_file_location("reuse_validation_fixture", helper)
    module = importlib.util.module_from_spec(spec)
    assert spec and spec.loader
    spec.loader.exec_module(module)
    full, _, _ = dogfood_observation._engine(journey,database,"show","--view","full",run_id)
    prepared = module.prepare(journey.provider,journey.engine,checkout,full,"reuse-validation-r2")
    for item in prepared["records"]:
        dogfood_observation._engine(journey,database,"show","--view","action",run_id)
        _checked(dogfood_observation._engine(journey,database,"append","--record-id",item["record_id"],
            run_id,item["kind"],json.dumps(item["data"],separators=(",",":")))[0],"validation record append")
    event("validation-ready","validation-review")
    review("validation-review","validation-adversarial-review")
    missing_terminal_axis = profile["review_policies"]["validation-adversarial-review"][-1]["id"]
    review("validation-adversarial-review","end",terminal_missing_axis=missing_terminal_axis)
    completed, _, _ = dogfood_observation._engine(journey,database,"show","--view","full",run_id)
    if _checked(completed,"production terminal show").get("lifecycle") != "final":
        raise ValueError("production revised plan did not reach normal reviewed completion")
    if (old_capture / "summary.json").read_bytes() != old_bytes:
        raise ValueError("production completion changed the original task evidence")
    proof = {"status":"completed-scripted-production-software-change","run_id":run_id,
        "database":str(database),"artifact_root":str(artifacts),
        "original_capture":str(old_capture),"original_capture_summary_sha256":old_source["summary_sha256"],
        "revised_invocation":revised["invocation_id"],"corrected_invocation":corrected["invocation_id"],
        "report_only_invocation":report_only["invocation_id"],"initial_implementation_review":initial_impl_review,
        "fresh_implementation_review":final_impl_review,"review_launches":review_counter.read_text().splitlines(),
        "mapped_standing_tasks":["beta-renamed","beta-split","beta-merge"],
        "pending_effect_and_dependency_tasks":sorted(pending),
        "direct_act":direct_history,
        "reconciliation_before":str(case / "reconciliation-before-driver-act.json"),
        "reconciliation_after":str(case / "reconciliation-after-driver-act.json"),
        "previous_reconciliation_sha256":"sha256:"+hashlib.sha256(previous_reconciliation).hexdigest(),
        "stale_intent_driver_act_refusal":stale_act.get("code"),
        "before_code_command":str(case / "before-code.command.json"),
        "before_oracle_command":str(case / "before-oracle.command.json"),
        "before_correction_command":str(case / "before-correction.command.json"),
        "final_report_revision":json.loads((artifacts / "implementation-report.json").read_text())["revision"],
        "final_head":current_head,"validation_capture_index":prepared["capture_index"],
        "terminal_missing_validation_axis":missing_terminal_axis,
        "terminal_denial":str(case / "terminal-missing-axis-denial.json"),
        "terminal_denial_code":terminal_denial["code"] if terminal_denial else None,
        "terminal_state":"end","semantic_review":"scripted mechanics only; no independent human judgment"}
    _write_json(case / "completed-software-proof.json",proof)
    journey._dogfood_case_root = original_case_root
    return proof


def dogfood_advice_subject(gate: str) -> str:
    if gate.startswith("intent-"):
        return "intent.json"
    if gate.startswith("design-"):
        return "design.json"
    if gate.startswith("plan-"):
        return "plan.json"
    if gate.startswith("implementation-"):
        return "implementation-report.json"
    return "validation-report.json"


def _start_review(journey, database: Path, run_id: str, gate: str) -> str:
    dogfood_observation._engine(journey,database,"show","--view","action",run_id)
    started, _, _ = dogfood_observation._engine(journey,database,"--timeout-ms","120000","invoke",run_id,gate)
    return _checked(started,"invoke scripted independent review")["invocation_id"]


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
    outcome["production_software_change"] = _software_change_completion(journey,root)
    _write_json(root / "outcome.json",outcome)
    print("sol-reuse public mapped reuse, production revised-plan reviewed completion, driver act, and post-reconciliation report-only paths passed")
