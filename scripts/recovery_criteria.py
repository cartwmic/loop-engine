"""Checkpoint-bound criterion proof through public CLI; no semantic quality claim."""
import copy
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
from recovery_override import Fixture


def prove(journey):
    journey.work_root.mkdir(parents=True, exist_ok=True)
    root = Path(tempfile.mkdtemp(prefix="recovery-criterion-", dir=journey.work_root))
    print(f"criterion captures: {root}", flush=True)
    repo = root / "repository"
    repo.mkdir()
    # Disposable fixture repository only, never the maintained checkout.
    for args in (["init", "-q"], ["config", "user.email", "fixture@example.invalid"],
                 ["config", "user.name", "fixture"]):
        subprocess.run(["git", "-C", str(repo), *args], check=True, capture_output=True)
    (repo / "product.txt").write_text("broken\n")
    subprocess.run(["git", "-C", str(repo), "add", "."], check=True)
    subprocess.run(["git", "-C", str(repo), "commit", "-qm", "isolated fixture"], check=True)
    captures = []
    def external(args, input=None, expected=0, cwd=repo):
        p = subprocess.run([str(a) for a in args], input=None if input is None else json.dumps(input),
                           text=True, capture_output=True, cwd=cwd, timeout=1200)
        captures.append({"argv": [str(a) for a in args], "cwd": str(cwd), "exit": p.returncode,
                         "stdout": p.stdout, "stderr": p.stderr})
        (root / "external.json").write_text(json.dumps(captures, indent=2))
        if p.returncode != expected:
            executions = list(root.glob("**/validation-execution-*"))
            if executions:
                latest = max(executions, key=lambda path: path.stat().st_mtime_ns)
                for path in sorted(latest.rglob("*")):
                    if path.is_file() and path.name in ("stdout", "stderr", "inner_exit.json", "summary.json"):
                        detail = path.read_text(errors="replace")
                        if detail:
                            print(f"capture diagnostic {path}:\n{detail[:12000]}", file=sys.stderr)
        assert p.returncode == expected, captures[-1]
        return json.loads(p.stdout) if p.stdout.strip().startswith("{") else p.stdout
    author = {"name": "implementer", "kind": "agent"}
    reviewer = {"name": "independent", "kind": "agent"}
    schema = {"type":"object", "required":["revision","author"], "properties":{
        "revision":{"type":"string","minLength":1}, "author":{"type":"object","required":["name","kind"],
        "properties":{"name":{"type":"string","minLength":1},"kind":{"type":"string","enum":["human","agent","script"]}}}}}
    report_schema = json.loads((journey.data_root / "crates/software-change-provider/data/validation-report-schema.json").read_text())
    proof = {"status":"running", "runs":[], "cases":[]}
    def start(name, axes=True, floor=1, command=None):
        f = Fixture(journey.engine, journey.provider, repo, root, name)
        profile = {"contract_version":3,"criterion_policy":{"required_authors":floor,"goal_required_authors":1},
            "config_version":"recovery-criterion-3", "artifact_root":str(f.artifacts),
            "review_policies": {"validation-review":[{"id":"delivery","description":"consume criterion collection","review_stage":"aggregate","required_authors":1}]} if axes else {},
            "revision_links": [{"from":"design.json","field":"intent_revision","to":"intent.json"},
                               {"from":"plan.json","field":"design_revision","to":"design.json"}],
            "artifact_schemas": {**{n:copy.deepcopy(schema) for n in ("intent.json","design.json","plan.json","implementation-report.json")},
                                 "validation-report.json":report_schema}}
        if name=="repair-carry":
            profile["review_policies"]["validation-adversarial-review"]=[{"id":"delivery","description":"challenge consumes existing criterion collection","review_stage":"aggregate","required_authors":1}]
        f.start(profile)
        docs = {"intent.json":{"revision":"1","author":author,"acceptance":[{"id":"AC-1","statement":"product corrected"},{"id":"AC-2","statement":"proof output is retained"}]},
            "design.json":{"revision":"1","author":author,"intent_revision":"1"},
            "plan.json":{"revision":"1","author":author,"design_revision":"1","tasks":[],"proof_commands":[{"id":"product","command":command or sys.executable,
                "args":["-c","from pathlib import Path; print(Path('product.txt').read_text())"],"owner":"driver","obligation":"inspect product"}]},
            "implementation-report.json":{"revision":"1","author":author}}
        for name, value in docs.items(): (f.artifacts/name).write_text(json.dumps(value))
        for event in ("intent-ready","design-ready","plan-ready"):
            f.show(); f.result(["event",f.name,event])
        checkpoint(f,"implementation")
        f.show(); f.result(["event",f.name,"implementation-ready"])
        proof["runs"].append({"id":f.name,"database":str(f.db),"artifacts":str(f.artifacts)})
        return f
    def checkpoint(f, phase):
        external([journey.provider,"checkpoint","--phase",phase,"--artifact-root",f.artifacts,"--working-directory",repo])
    def run(f, revision, failed=False, timeout=None):
        # The full show arms this unchanged validation visit and is also the
        # exact packet supplied to run-validation.  Reuse that observation
        # instead of reparsing the same durable projection immediately.
        observed = f.show()
        before = observed["context"]
        result = external([journey.provider,"run-validation","--engine",journey.engine,"--working-directory",repo,"--revision",revision,
                           *([] if timeout is None else ["--timeout-ms",str(timeout)])],
                          {"status":"completed","operation":"show","result":observed}, expected=1 if failed else 0)
        assert result["commands_passed"] == (not failed), result
        assert f.show()["context"] == before, "helper appended catalog records"
        for row in result["command_candidates"]: f.append(row["kind"], row["record_id"], row["data"])
        return result["report"]
    def ledger(f, revision, findings=None):
        f.append("finding-ledger", "ledger-"+str(len(f.transcript)), {"schema_version":"1","gate":"validation-review","subject":"validation-report.json",
            "subject_revision":revision,"author":{"name":"driver","kind":"agent"},"findings":findings or []})
    def verdict(report, criterion=None, result="pass", who=reviewer):
        v={"subject":"validation-report.json","subject_revision":report["revision"],"checkpoint":"validation-checkpoint.json","author":who,
           "result":result,"findings":["product remains broken"] if result=="fail" else [],"evidence_context_ids":report["command_evidence_ids"]}
        if criterion: v["criterion_id"]=criterion
        return v
    def genuine(f, record_id, kind, value, sentinel=False):
        # A real scripted external judgment capture, never a placeholder record.
        code = "import json,sys; v=json.load(sys.stdin); print(json.dumps(v))"
        if sentinel: code = "raise SystemExit('unaffected reviewer MUST NOT RUN')"
        output = external([sys.executable,"-c",code],value)
        f.append(kind,record_id,output)
    def event(f, name, status="completed", needle=None):
        shown=f.show()
        if f.name=="repair-carry" and shown["current_state"]=="validation-review" and name=="passed": name="approved"
        result=f.result(["event",f.name,name],status)
        if needle: assert needle in json.dumps(result), result
        return result
    def complete_rows(f, report):
        for row in report["criteria"]:
            for n, id in enumerate(row["verdict_ids"]):
                genuine(f,id,"criterion-verdict",verdict(report,row["criterion_id"],who={"name":"independent-"+str(n),"kind":"agent"}))
        genuine(f,report["goal_verdict_ids"][0],"goal-verdict",verdict(report))
    # Fixed index/checkpoint before external review, two different results.
    f=start("repair-carry")
    report=run(f,"v1")
    before_captures=list(f.artifacts.glob("validation-execution-*"))
    collision=external([journey.provider,"run-validation","--engine",journey.engine,"--working-directory",repo,"--revision","v1"],{"status":"completed","operation":"show","result":f.show()},expected=2)
    assert list(f.artifacts.glob("validation-execution-*"))==before_captures
    checkpoint(f,"validation")
    original_report=(f.artifacts/"validation-report.json").read_bytes()
    original_checkpoint=(f.artifacts/"validation-checkpoint.json").read_bytes()
    (root/"frozen-v1-report.json").write_bytes(original_report)
    (root/"frozen-v1-checkpoint.json").write_bytes(original_checkpoint)
    event(f,"validation-ready")
    ledger(f,"v1")
    event(f,"passed","rejected","missing")
    for row in report["criteria"]:
        genuine(f,row["verdict_ids"][0],"criterion-verdict",verdict(report,row["criterion_id"],"fail" if row["criterion_id"]=="AC-1" else "pass"))
    genuine(f,report["goal_verdict_ids"][0],"goal-verdict",verdict(report))
    assert (f.artifacts/"validation-report.json").read_bytes()==original_report
    assert (f.artifacts/"validation-checkpoint.json").read_bytes()==original_checkpoint
    event(f,"passed","rejected","unresolved failing verdict")
    finding={"id":"F-product","source":{"kind":"context-record","id":report["criteria"][0]["verdict_ids"][0]},"policy_id":"AC-1",
        "statement":"product remains broken","disposition":"accepted","reason":"observed product output","owner_phase":"implementation","task_ids":[],"review_axes":[],"status":"unresolved"}
    ledger(f,"v1",[finding]); event(f,"passed","rejected","finding ledger")
    event(f,"revise-implementation")
    (repo/"product.txt").write_text("fixed\n")
    (f.artifacts/"implementation-report.json").write_text(json.dumps({"revision":"2","author":author}))
    checkpoint(f,"implementation"); event(f,"implementation-ready")
    new=run(f,"v2")
    # Prechoose one genuine applicability ID in the new immutable index.
    new["criteria"][1]["verdict_ids"]=["carry-ac2"]
    (f.artifacts/"validation-report.json").write_text(json.dumps(new))
    checkpoint(f,"validation")
    f.append("criterion-revalidation","affected",{"subject_revision":"v2","affected_criteria":["AC-1"],"change_kind":"material","reason":"focused product correction"})
    carry={"origin":{"kind":"context-record","id":report["criteria"][1]["verdict_ids"][0]},
           "target":{"subject":"validation-report.json","subject_revision":"v2","checkpoint":{"phase":"validation","report_revision":"v2"}},
           "attesting_driver":{"name":"driver","kind":"agent"},"reason":"AC-2 retained capture mechanics unchanged"}
    f.append("evidence-applicability","carry-ac2",carry)
    finding.update(status="resolved",reason="focused correction with fresh proof")
    ledger(f,"v2",[finding])
    event(f,"validation-ready")
    # The unaffected reviewer is replaced by a fail-if-invoked sentinel. Only
    # fresh selected IDs are commissioned; the sentinel branch must not execute.
    sentinel=root/"unaffected-sentinel.py"
    sentinel.write_text("from pathlib import Path\nPath(__file__).with_suffix('.invoked').touch()\nraise SystemExit('MUST NOT RUN unaffected review')\n")
    # The actual bound ordinary commission handles criterion + goal + axis once.
    # Its explicit carry selection dispatches no reviewer for AC-2. Installing
    # a fail-if-executed backend there makes accidental reruns observable.
    worker=root/"combined-review.py"
    worker.write_text('''import json,sys,subprocess
from pathlib import Path
packet=json.loads(sys.stdin.read().split("---",1)[0])
root=Path(packet["artifact_root"])
report=json.loads((root/"validation-report.json").read_text())
context={r["id"]:r for r in packet["context"]}
author={"name":"independent","kind":"agent"}
rows=[]
for row in report["criteria"]:
    id=row["verdict_ids"][0]
    if context.get(id,{}).get("kind")=="evidence-applicability": continue
    if row["criterion_id"]=="AC-2": subprocess.run([sys.executable,sys.argv[1]],check=True)
    v={"criterion_id":row["criterion_id"],"subject":"validation-report.json","subject_revision":report["revision"],"checkpoint":"validation-checkpoint.json","author":author,"result":"pass","findings":[],"evidence_context_ids":report["command_evidence_ids"]}
    rows.append({"record_id":id,"kind":"criterion-verdict","data":v})
v={"subject":"validation-report.json","subject_revision":report["revision"],"checkpoint":"validation-checkpoint.json","author":author,"result":"pass","findings":[],"evidence_context_ids":report["command_evidence_ids"]}
rows.append({"record_id":report["goal_verdict_ids"][0],"kind":"goal-verdict","data":v})
print(json.dumps({"review_stage":"aggregate","author":author,"judgments":[{"axis":"delivery","result":"pass","findings":""}],"validation_verdicts":rows}))
''')
    output_schema=json.loads((journey.data_root/"crates/software-change-provider/data/review-worker-output-schema.json").read_text())
    output_schema["properties"]["author"]["const"]=reviewer
    judgments=output_schema["properties"]["judgments"]
    judgments.update(minItems=1,maxItems=1,allOf=[{"contains":{"type":"object","required":["axis"],"properties":{"axis":{"const":"delivery"}}}}])
    for variant in judgments["items"]["oneOf"]:variant["properties"]["axis"]["enum"]=["delivery"]
    output_schema["properties"]["validation_verdicts"]={"type":"array","items":{"type":"object"}}
    binding={"command":str(journey.engine),"args":["fan-out","--worker",json.dumps({"command":sys.executable,"args":[str(worker),str(sentinel)],"full_output_schema":output_schema})],
             "context_filter":{"command":str(journey.provider),"args":["commission"]}}
    f.result(["amend-binding",f.name,"validation-review",json.dumps({"state_visit":f.show()["state_visit"],"owner":"fixture-owner","reason":"combine affected criterion, goal and axis commission","binding":binding})])
    f.show(); invocation=f.result(["invoke",f.name,"validation-review"])["invocation_id"]
    import time
    deadline=time.monotonic()+60
    while True:
        shown=f.show()
        selected=next(i for i in shown["work_slot_invocations"] if i["invocation_id"]==invocation)
        if selected.get("completed_at") is not None:break
        assert time.monotonic()<deadline,selected
        time.sleep(.1)
    assert selected["status"]=="succeeded",selected
    candidates=external([journey.provider,"review-candidates"],{"status":"completed","operation":"show","result":shown})["candidates"]
    assert len(candidates)==3,candidates
    before=f.show()["context"]
    assert not any(r["id"] in [new["criteria"][0]["verdict_ids"][0],new["goal_verdict_ids"][0]] for r in before)
    for row in candidates:
        if row["status"]=="verdict-ready":
            f.append(row["kind"],row["record_id"],{**row["data"],"origin":row["origin"]})
        else:
            assert row["status"]=="ready",row
            f.append("review-evidence","axis",{"gate":"validation-review","policy_id":row["axis"],"review_stage":"aggregate","subject":"validation-report.json","subject_revision":"v2",
                "config_version":"recovery-criterion-3","author":row["author"],"result":row["result"],"findings":row["findings"],"origin":row["origin"]})
    projection=external([journey.provider,"commission","--slot","validation-review"],{"status":"completed","result":f.show()})
    assert [r["mode"] for r in projection["validation_collection"]]==["fresh","carried","fresh"],projection
    assert projection["validation_collection"][1]["source"]["data"]==verdict(report,"AC-2")
    event(f,"passed")
    assert f.show()["current_state"]=="validation-adversarial-review"
    challenge=external([journey.provider,"commission","--slot","validation-adversarial-review"],{"status":"completed","operation":"show","result":f.show()})
    assert challenge["validation_collection"]==projection["validation_collection"]
    challenge_output=external([sys.executable,"-c","import json,sys; c=json.load(sys.stdin); assert len(c['validation_collection'])==3; print(json.dumps({'result':'pass','findings':''}))"],challenge)
    f.append("review-evidence","challenge-axis",{"gate":"validation-adversarial-review","policy_id":"delivery","review_stage":"aggregate","subject":"validation-report.json","subject_revision":"v2",
        "config_version":"recovery-criterion-3","author":{"name":"challenge","kind":"agent"},**challenge_output})
    f.append("finding-ledger","challenge-ledger",{"schema_version":"1","gate":"validation-adversarial-review","subject":"validation-report.json","subject_revision":"v2",
        "author":{"name":"driver","kind":"agent"},"findings":[]})
    event(f,"passed")
    assert f.show()["lifecycle"]=="final" and not f.show()["has_overrides"]
    assert not sentinel.with_suffix('.invoked').exists()
    proof["terminal_collection"]=projection["validation_collection"]
    proof["terminal_run"]={"id":f.name,"database":str(f.db),"invocation_id":invocation,"capture_dir":selected["capture_dir"],"checkpoint":str(f.artifacts/"validation-checkpoint.json"),"report":str(f.artifacts/"validation-report.json")}
    proof["cases"].append("two-results-unresolved-repair-explicit-carry-goal-terminal")
    # No review topology still requires all genuine criterion/goal records.
    f=start("reviewless",False,2)
    report=run(f,"v1"); checkpoint(f,"validation")
    event(f,"passed","rejected","missing")
    complete_rows(f,report)
    event(f,"passed")
    proof["cases"].append("reviewless-independent-policy-terminal")
    # Every negative gets a fresh immutable report revision and names. Failures
    # stay in context/history; correction never overwrites an appended record.
    negatives = {
        "omitted":lambda r:r["criteria"].pop(),
        "duplicate":lambda r:r["criteria"].append(copy.deepcopy(r["criteria"][0])),
        "unknown":lambda r:r["criteria"][0].update(criterion_id="AC-999"),
        "missing-command":lambda r:r.update(command_evidence_ids=["absent-command"]),
        "duplicate-verdict-id":lambda r:r["criteria"][1].update(verdict_ids=r["criteria"][0]["verdict_ids"]),
        "duplicate-command-id":lambda r:r["command_evidence_ids"].append(r["command_evidence_ids"][0]),
    }
    for label, mutate in negatives.items():
        f=start(label,False); report=run(f,"v1"); mutate(report)
        (f.artifacts/"validation-report.json").write_text(json.dumps(report)); checkpoint(f,"validation")
        event(f,"passed","rejected")
        proof["cases"].append(label)
    for label in ("stale","stale-checkpoint","self-authored","self-report-author","insufficient-authors","missing-evidence","missing-capture","incomplete-capture","wrong-repository"):
        f=start(label,False,2 if label=="insufficient-authors" else 1); report=run(f,"v1")
        if label=="insufficient-authors":
            for row in report["criteria"]: row["verdict_ids"]=row["verdict_ids"][:1]
            (f.artifacts/"validation-report.json").write_text(json.dumps(report))
        checkpoint(f,"validation")
        for row in report["criteria"]:
            v=verdict(report,row["criterion_id"])
            if label=="stale":v["subject_revision"]="old"
            if label=="self-authored":v["author"]=author
            if label=="self-report-author":v["author"]=report["author"]
            if label=="stale-checkpoint":v["checkpoint"]="stale-checkpoint.json"
            if label=="missing-evidence":v["evidence_context_ids"]=["absent"]
            genuine(f,row["verdict_ids"][0],"criterion-verdict",v)
        genuine(f,report["goal_verdict_ids"][0],"goal-verdict",verdict(report))
        if label in ("missing-capture","incomplete-capture"):
            command=next(r for r in f.show()["context"] if r["kind"]=="command-evidence")
            summary=json.loads(Path(command["data"]["capture"]["summary"]).read_text())
            path=Path(summary["workers"][0]["stdout_path"])
            path.rename(root/("preserved-"+label))
            if label=="incomplete-capture":path.write_text('{"incomplete":')
        if label=="wrong-repository":
            (repo/"product.txt").write_text("changed after commands\n")
            (f.artifacts/"implementation-report.json").write_text(json.dumps({"revision":"2","author":author}))
            # Honest new implementation admission first; command capture still old.
            event(f,"revise-implementation"); checkpoint(f,"implementation"); event(f,"implementation-ready")
            report["implementation_revision"]="2"; (f.artifacts/"validation-report.json").write_text(json.dumps(report)); checkpoint(f,"validation")
        event(f,"passed","rejected")
        proof["cases"].append(label)
    for label, command in (("missing-executable","/missing-proof-executable"),("nonzero",sys.executable)):
        f=start(label,False,command=command)
        if label=="nonzero":
            # Explicit steering replaces execution, retaining named obligation.
            f.append("user-steering","command-update",{"target":{"kind":"slots","ids":["validation-draft"]},"instruction":"capture failure",
                "proof_updates":[{"proof_id":"product","reason":"negative execution proof","command":sys.executable,"args":["-c","raise SystemExit(7)"]}]})
        report=run(f,"v1",failed=True); checkpoint(f,"validation")
        event(f,"passed","rejected","command failed")
        if label=="nonzero":
            failed_id=report["criteria"][0]["verdict_ids"][0]
            genuine(f,failed_id,"criterion-verdict",verdict(report,"AC-1","fail"))
            finding={"id":"F-command","source":{"kind":"context-record","id":failed_id},"policy_id":"AC-1","statement":"product remains broken",
                "disposition":"accepted","status":"unresolved","owner_phase":"validation","task_ids":[],"review_axes":[],"reason":"retain actual nonzero execution"}
            ledger(f,"v1",[finding])
            f.append("user-steering","correct-command",{"target":{"kind":"slots","ids":["validation-draft"]},"instruction":"correct execution without changing obligation","supersedes":["command-update"],
                "proof_updates":[{"proof_id":"product","reason":"fix mistaken execution arguments","command":sys.executable,"args":["-c","from pathlib import Path; print(Path('product.txt').read_text())"]}]})
            report=run(f,"v2"); checkpoint(f,"validation")
            finding.update(status="resolved",reason="fresh named execution now succeeds; original failure retained")
            ledger(f,"v2",[finding]); complete_rows(f,report); event(f,"passed")
            proof["cases"].append("nonzero-failing-criterion-source-resolves-with-history-retained")
        proof["cases"].append(label)
    f=start("carry-refusals",False)
    old=run(f,"v1"); checkpoint(f,"validation"); complete_rows(f,old)
    new=run(f,"v2")
    for n,row in enumerate(new["criteria"]): row["verdict_ids"]=["carry-"+str(n)]
    new["goal_verdict_ids"]=["carry-goal"]
    (f.artifacts/"validation-report.json").write_text(json.dumps(new)); checkpoint(f,"validation")
    for n,original in enumerate([old["criteria"][0]["verdict_ids"][0],old["criteria"][1]["verdict_ids"][0],old["goal_verdict_ids"][0]]):
        f.append("evidence-applicability","carry-"+str(n) if n<2 else "carry-goal",{
            "origin":{"kind":"context-record","id":original},"target":{"subject":"validation-report.json","subject_revision":"v2","checkpoint":{"phase":"validation","report_revision":"v2"}},
            "attesting_driver":{"name":"driver","kind":"agent"},"reason":"original proof still applies to unchanged tree"})
    for record_id,affected,kind in (("affected-carry",["AC-1"],"material"),("goal-material",[],"material")):
        f.append("criterion-revalidation",record_id,{"subject_revision":"v2","affected_criteria":affected,"change_kind":kind,"reason":"explicit negative declaration"})
        event(f,"passed","rejected","require fresh verdicts")
    f.append("criterion-revalidation","index-only",{"subject_revision":"v2","affected_criteria":[],"change_kind":"report-index-only","reason":"only index changed; outcome and implementation unchanged"})
    event(f,"passed")
    proof["cases"].append("affected-carry-and-material-goal-refuse-index-only-goal-carry-terminal")
    f=start("criterion-goal-disposition",False)
    report=run(f,"v1"); checkpoint(f,"validation")
    failures=[]
    for criterion_id,record_id,kind in [(r["criterion_id"],r["verdict_ids"][0],"criterion-verdict") for r in report["criteria"]]+[(None,report["goal_verdict_ids"][0],"goal-verdict")]:
        result="pass" if criterion_id=="AC-2" else "fail"
        genuine(f,record_id,kind,verdict(report,criterion_id,result))
        if result=="fail":failures.append({"id":"F-"+(criterion_id.lower() if criterion_id else "goal"),"source":{"kind":"context-record","id":record_id},
            "policy_id":criterion_id or "goal","statement":"product remains broken","disposition":"accepted","status":"unresolved","owner_phase":"implementation",
            "task_ids":[],"review_axes":[],"reason":"negative disposition proof"})
    ledger(f,"v1",failures); event(f,"passed","rejected","finding ledger")
    for finding in failures:finding.update(disposition="rejected",status="recorded",owner_phase=None,reason="driver explicitly rejects exact claim; original fail retained")
    ledger(f,"v1",failures); event(f,"passed")
    assert sum(r["data"].get("result")=="fail" for r in f.show()["context"] if r["kind"] in ("criterion-verdict","goal-verdict"))==2
    proof["cases"].append("criterion-and-goal-rejection-counts-with-original-failures-retained")
    f=start("timeout",False)
    timeout_pid=root/"timeout.pid"
    f.append("user-steering","timeout-command",{"target":{"kind":"slots","ids":["validation-draft"]},"instruction":"exercise bounded command failure",
        "proof_updates":[{"proof_id":"product","reason":"timeout proof","command":sys.executable,"args":["-c",f"import os,time; from pathlib import Path; Path({str(timeout_pid)!r}).write_text(str(os.getpid())); time.sleep(30)"]}]})
    run(f,"v1",failed=True,timeout=100); checkpoint(f,"validation")
    event(f,"passed","rejected","command failed")
    pid=int(timeout_pid.read_text())
    try:os.kill(pid,0)
    except ProcessLookupError:pass
    else:raise AssertionError(f"timed-out command {pid} survived")
    proof["cases"].append("timeout-fails-and-reaps-command")
    f=start("capture-start-failure",False)
    report=run(f,"v1"); checkpoint(f,"validation"); complete_rows(f,report)
    before=f.show()["context"]
    failed_engine=root/"failed-capture-engine"
    failed_engine.write_text("#!/bin/sh\nprintf 'capture-backend-sentinel\\n' >&2\nexit 20\n")
    failed_engine.chmod(0o755)
    external([journey.provider,"run-validation","--engine",failed_engine,"--working-directory",repo,"--revision","v2"],
             {"status":"completed","operation":"show","result":f.show()},expected=2)
    assert "capture-backend-sentinel" in captures[-1]["stderr"], captures[-1]
    assert f.show()["context"]==before
    assert json.loads((f.artifacts/"validation-report.json").read_text())["revision"]=="v2"
    event(f,"passed","rejected","checkpoint")
    checkpoint(f,"validation"); event(f,"passed","rejected","missing")
    proof["cases"].append("capture-start-failure-invalidates-old-passing-index-without-placeholders")
    proof["status"]="passed"
    (root/"proof.json").write_text(json.dumps(proof,indent=2))
    print(f"recovery criterion proof passed: {root/'proof.json'}",flush=True)
    return proof
