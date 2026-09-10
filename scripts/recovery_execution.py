"""Public execution-controls proof with isolated catalogs and deterministic workers."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import time


def prove_bindings(engine, provider):
    root = Path(tempfile.mkdtemp(prefix="recovery-execution-bindings-"))
    artifacts = root / "artifacts"
    artifacts.mkdir()
    db = root / "loop.sqlite"
    transcript = []

    def call(args, status="completed"):
        p = subprocess.run([engine, "--database", str(db), "--json", *args],
                           capture_output=True, timeout=40)
        value = json.loads(p.stdout)
        transcript.append({"argv": args, "exit": p.returncode, "envelope": value,
                           "stderr": p.stderr.decode()})
        (root / "transcript.json").write_text(json.dumps(transcript, indent=2))
        assert value["status"] == status, value
        assert p.returncode == {"completed": 0, "rejected": 10, "error": 20, "invalid-invocation": 2}[status], value
        return value.get("result", value)

    def show():
        return call(["show", "--view", "full", "binding-correction"])

    worker = root / "worker.py"
    worker.write_text('''import json, pathlib, sys, time
packet=json.load(sys.stdin)
root=pathlib.Path(packet['artifact_root'])
(root/'started').touch()
while (root/'hold').exists(): time.sleep(.02)
output=json.dumps({'argv':sys.argv[1:], 'packet':packet})
(pathlib.Path(packet['capture_dir'])/'stdout').write_text(output)
print(output)
''')
    selector = root / "selector.py"
    selector.write_text('''import json, pathlib, sys
p=json.load(sys.stdin)
(pathlib.Path(p['artifact_root'])/'filter-packet.json').write_text(json.dumps(p))
print(json.dumps({'record_ids':[r['id'] for r in p['context']]}))
''')
    original = {"command": sys.executable, "args": [str(worker), "--model", "typo", "--extension", "old"]}
    corrected = {"command": sys.executable, "args": [str(worker), "--model", "correct", "--extension", "new"]}
    for binding in (original, corrected):
        binding["context_filter"] = {"command": sys.executable, "args": [str(selector)]}
    profile = {"contract_version": 2, "criterion_policy": {"required_authors": 1, "goal_required_authors": 1}, "config_version": "recovery-execution-2", "artifact_root": str(artifacts),
               "review_policies": {}, "work_slot_bindings": {"intent-draft": original}}
    config = root / "providers.toml"
    config.write_text('[providers.software-change]\ncommand = ' + json.dumps(provider) + '\n')
    call(["--config", str(config), "start", "--id", "binding-correction", "software-change", json.dumps(profile)])
    before = show()
    frozen = before["initial_input"]
    visit = before["state_visit"]
    assert before["binding_amendments"] == []
    preview = call(["invoke", "binding-correction", "intent-draft", "--preview", "--timeout-ms", "250"])
    assert preview["binding"] == original and preview["allowed_time_ms"] == 250
    assert preview["controls"] == {"force_fresh": False}
    assert not (artifacts / "started").exists()
    assert not (artifacts / "work-slot-captures").exists()
    assert not show()["work_slot_invocations"]
    assert json.loads((artifacts / "filter-packet.json").read_text())["controls"]["timeout_ms"] == 250
    for controls in [{"max_active": 2}, {"force_fresh": True}]:
        call(["invoke", "binding-correction", "intent-draft", "--controls", json.dumps(controls)], "error")
    for controls in [{"max_active": 0}, {"force_fresh": "yes"}, {"policy": {}}]:
        call(["invoke", "binding-correction", "intent-draft", "--controls", json.dumps(controls)], "invalid-invocation")
    (artifacts / "hold").touch()
    first = call(["invoke", "binding-correction", "intent-draft", "--timeout-ms", "250"])
    deadline = time.monotonic() + 20
    while not (artifacts / "started").exists():
        assert time.monotonic() < deadline
        time.sleep(.02)
    request = {"state_visit": visit, "owner": "fixture-owner", "reason": "correct model and extension arguments", "binding": corrected}
    # Timeout is the retained attempt allowance, not an invented successful stop.
    time.sleep(.3)
    elapsed = show()["work_slot_invocations"][0]
    assert elapsed["allowed_time_ms"] == 250 and elapsed["status"] == "overrun", elapsed
    assert elapsed["controls"] == preview["controls"]
    amendment = call(["amend-binding", "binding-correction", "intent-draft", json.dumps(request)])
    assert amendment["action"]["kind"] == "binding_amended"
    payload = amendment["action"]["amendment"]
    assert payload["original"] == original and payload["effective"] == corrected
    during = show()
    assert during["initial_input"] == frozen
    assert during["effective_bindings"]["intent-draft"] == corrected
    assert '"correct"' in during["current_state_instructions"]
    assert '"typo"' not in during["current_state_instructions"]
    assert during["work_slot_invocations"][0]["binding"] == original
    call(["append", "binding-correction", "--kind", "user-steering", "--record-id", "future-steering", json.dumps({"target": {"kind": "all"}, "instruction": "later commission only"})])
    (artifacts / "hold").unlink()

    def wait(result, binding):
        deadline = time.monotonic() + 25
        while True:
            invocation = next(i for i in show()["work_slot_invocations"] if i["invocation_id"] == result["invocation_id"])
            if invocation["completed_at"] is not None:
                assert invocation["status"] == "succeeded", invocation
                break
            assert time.monotonic() < deadline, invocation
            time.sleep(.03)
        output = json.loads((Path(result["capture_dir"]) / "stdout").read_text())
        assert output["argv"] == binding["args"][1:], output
        assert invocation["binding"] == binding
        return output

    old_output = wait(first, original)
    second = call(["invoke", "binding-correction", "intent-draft"])
    new_output = wait(second, corrected)
    assert old_output["packet"]["instruction_body"] == new_output["packet"]["instruction_body"]
    assert old_output["packet"].get("context") == preview["context"]
    assert [row["id"] for row in new_output["packet"]["context"]] == ["future-steering"]
    assert show()["work_slot_invocations"][0]["controls"] == preview["controls"]
    for invalid in [dict(request, policy={}), dict(request, topology={}),
                    dict(request, binding=dict(corrected, required_authors=1))]:
        call(["amend-binding", "binding-correction", "intent-draft", json.dumps(invalid)], "invalid-invocation")
    call(["amend-binding", "binding-correction", "unknown", json.dumps(request)], "rejected")
    call(["amend-binding", "binding-correction", "intent-draft", json.dumps(dict(request, state_visit=visit + 1))], "error")
    call(["amend-binding", "binding-correction", "intent-draft", json.dumps(dict(request, owner=" "))], "rejected")
    final = show()
    assert final["initial_input"] == frozen and final["state_visit"] == visit
    assert len(final["binding_amendments"]) == 1
    history = call(["history", "binding-correction"])
    assert sum(row["action"]["kind"] == "binding_amended" for row in history) == 1
    call(["--config", str(config), "start", "--id", "unobserved", "software-change", json.dumps(profile)])
    call(["amend-binding", "unobserved", "intent-draft", json.dumps(request)], "rejected")
    call(["show", "--view", "full", "unobserved"])
    call(["terminate", "unobserved"])
    call(["terminate", "binding-correction"])
    # Bounded preparation kills/reaps a stalled filter, with no primary launch.
    stalled = dict(profile, work_slot_bindings={"intent-draft": {
        "command": sys.executable, "args": [str(worker)],
        "context_filter": {"command": sys.executable, "args": ["-c", "import os,pathlib,sys,time;sys.stdin.read();pathlib.Path(" + repr(str(root / "filter.pid")) + ").write_text(str(os.getpid()));time.sleep(90)"]}}})
    call(["--config", str(config), "start", "--id", "stalled-filter", "software-change", json.dumps(stalled)])
    call(["show", "--view", "full", "stalled-filter"])
    began = time.monotonic()
    refusal = call(["invoke", "stalled-filter", "intent-draft", "--preview"], "error")
    assert "timeout" in json.dumps(refusal) and time.monotonic() - began < 40
    assert not call(["show", "--view", "full", "stalled-filter"])["work_slot_invocations"]
    import os
    try:
        os.kill(int((root / "filter.pid").read_text()), 0)
    except ProcessLookupError:
        pass
    else:
        raise AssertionError("timed-out filter was not reaped")
    call(["terminate", "stalled-filter"])
    call(["amend-binding", "binding-correction", "intent-draft", json.dumps(request)], "rejected")
    print(f"binding amendment subproof passed; captures: {root}")
    return root


def prove_facades(engine, provider, checkout, work_root=None):
    root = Path(tempfile.mkdtemp(prefix="recovery-execution-facades-", dir=work_root))
    print(f"execution facade captures: {root}", flush=True)
    artifacts = root / "artifacts"
    artifacts.mkdir()
    db = root / "loop.sqlite"
    transcript = []

    def call(args, status="completed"):
        p = subprocess.run([engine, "--database", str(db), "--json", *args], cwd=checkout,
                           capture_output=True, timeout=120)
        value = json.loads(p.stdout)
        transcript.append({"argv": args, "exit": p.returncode, "envelope": value, "stderr": p.stderr.decode()})
        (root / "transcript.json").write_text(json.dumps(transcript, indent=2))
        assert value["status"] == status, value
        assert p.returncode == {"completed": 0, "rejected": 10, "error": 20, "invalid-invocation": 2}[status], value
        return value.get("result", value)

    def show(run="graph-controls"):
        return call(["show", "--view", "full", run])

    def wait(result, run="graph-controls"):
        deadline = time.monotonic() + 120
        while True:
            record = next(i for i in show(run)["work_slot_invocations"] if i["invocation_id"] == result["invocation_id"])
            if record["completed_at"] is not None:
                assert record["status"] == "succeeded", record
                ownership = record["ownership"]
                assert not ownership["cleanup_pending"], ownership
                assert not ownership["live_owned_work"], ownership
                owned = ownership["execution"]
                assert owned["root_pid"] == owned["process_group_id"], owned
                locator = json.loads(Path(owned["graph_locator"]).read_text())
                assert set(locator) == {"dagu_home", "dag_name", "run_name"}, locator
                assert list(Path(owned["admission_directory"]).glob("helper-*.json"))
                return record
            assert time.monotonic() < deadline, record
            time.sleep(.1)

    worker = root / "graph-worker.py"
    worker.write_text('''import fcntl, json, os, pathlib, sys, time
ownership=pathlib.Path(os.environ['LOOP_ENGINE_OWNERSHIP_DIRECTORY'])
owned=json.loads((ownership/'ownership.json').read_text())
assert pathlib.Path(owned['graph_locator']).is_file()
assert json.loads((ownership/('helper-%s.json' % os.getppid())).read_text())['root_pid']==os.getppid()
location, assignment=sys.stdin.read().split('\\n---\\n\\n',1)
p=json.loads(location); root=pathlib.Path(p['artifact_root'])
name='summarizer' if 'plan_path' in p else json.loads(assignment)['id']
def event(kind):
    with (root/'activity.jsonl').open('a') as f:
        fcntl.flock(f, fcntl.LOCK_EX)
        f.write(json.dumps({'task':name,'event':kind,'at':time.monotonic(),'pid':os.getpid(),'argv':sys.argv[1:],'input':assignment})+'\\n')
        f.flush()
event('start')
time.sleep(.7)
if name=='summarizer':
    report={'revision':str(time.time_ns()),'author':{'name':'dummy','kind':'script'},'plan_revision':'1','coverage':['fixture'],'summary':'deterministic controls proof','changed_surface':['fixture'],'validation':['focused controls']}
    (root/'implementation-report.json').write_text(json.dumps(report))
event('end')
print(json.dumps({'task':name,'fresh':True,'repository_effect':{'kind':'fixture-receipt-only'}}))
''')
    binding = {"command": provider, "args": ["run-plan-graph", "--working-directory", str(checkout),
        "--max-active", "4", "--task-worker", json.dumps({"command": sys.executable, "args": [str(worker), "actual-graph-worker"]})],
        "context_filter": {"command": provider, "args": ["commission"]}}
    schema = {"type": "object", "required": ["revision", "author"], "properties": {
        "revision": {"type": "string"}, "author": {"type": "object", "required": ["name", "kind"],
        "properties": {"name": {"type": "string"}, "kind": {"type": "string", "enum": ["human", "agent", "script"]}}}}}
    profile = {"contract_version": 2, "criterion_policy": {"required_authors": 1, "goal_required_authors": 1}, "config_version": "recovery-execution-2", "artifact_root": str(artifacts), "review_policies": {},
        "artifact_schemas": {name: schema for name in ["intent.json", "design.json", "plan.json"]},
        "work_slot_bindings": {"implement": binding}}
    for name in ["intent", "design"]:
        (artifacts / (name + ".json")).write_text(json.dumps({"revision": "1", "author": {"name": "subject", "kind": "script"}}))
    plan = {"revision": "1", "author": {"name": "subject", "kind": "script"},
            "tasks": [{"id": n} for n in ["A", "B", "C"]], "dependency_graph": [{"from": "A", "to": "B"}]}
    (artifacts / "plan.json").write_text(json.dumps(plan))
    plan_bytes = (artifacts / "plan.json").read_bytes()
    config = root / "providers.toml"
    config.write_text('[providers.software-change]\ncommand = ' + json.dumps(provider) + '\n')
    call(["--config", str(config), "start", "--id", "graph-controls", "software-change", json.dumps(profile)])
    for event in ["intent-ready", "design-ready", "plan-ready"]:
        show()
        call(["event", "graph-controls", event])
    show()
    selected = ["--input", json.dumps({"plan_revision": "1", "task_roots": ["B"]})]
    call(["invoke", "graph-controls", "implement", "--preview", *selected], "error")
    call(["invoke", "graph-controls", "implement", *selected], "error")
    assert not show()["work_slot_invocations"]
    assert not (artifacts / "work-slot-captures").exists()
    assert not (artifacts / "activity.jsonl").exists()

    def launch(controls, extra=()):
        args = ["invoke", "graph-controls", "implement", "--timeout-ms", "120000", "--controls", json.dumps(controls), *extra]
        before = len(show()["work_slot_invocations"])
        old_activity = (artifacts / "activity.jsonl").read_bytes() if (artifacts / "activity.jsonl").exists() else b""
        preview = call([*args, "--preview"])
        assert len(show()["work_slot_invocations"]) == before
        assert ((artifacts / "activity.jsonl").read_bytes() if (artifacts / "activity.jsonl").exists() else b"") == old_activity
        result = call(args)
        record = wait(result)
        assert record["controls"] == preview["controls"] == controls
        assert record["binding"] == preview["binding"] == binding
        assert record["routed_inputs"] == preview["context"]
        assert record["allowed_time_ms"] == preview["allowed_time_ms"] == 120000
        assert record["invocation_input"] == preview["invocation_input"] if "invocation_input" in record else preview["invocation_input"] is None
        activity = [json.loads(line) for line in (artifacts / "activity.jsonl").read_bytes()[len(old_activity):].splitlines()]
        active, peak = set(), 0
        starts = []
        for item in activity:
            assert item["argv"] == ["actual-graph-worker"]
            if item["event"] == "start":
                if item["task"] == "summarizer":
                    assert not active
                active.add(item["task"])
                starts.append(item["task"])
                peak = max(peak, len(active))
            else:
                active.remove(item["task"])
        assert not active and peak <= controls["max_active"]
        assert set(preview["facade_preparation"]["selected_tasks"]) == set(starts) - {"summarizer"}
        return result, starts, peak

    first, starts, peak = launch({"max_active": 1, "force_fresh": False})
    assert set(starts) == {"A", "B", "C", "summarizer"} and peak == 1
    assert (artifacts / "implementation-checkpoint.json").exists()
    # A fresh subset cannot count A's old successful execution as fresh.
    refusal = call(["invoke", "graph-controls", "implement", "--preview", *selected, "--controls", '{"force_fresh":true}'], "error")
    assert "full execution" in json.dumps(refusal), refusal
    call(["invoke", "graph-controls", "implement", *selected, "--controls", '{"force_fresh":true}'], "error")
    second, starts, peak = launch({"max_active": 2, "force_fresh": True})
    assert set(starts) == {"A", "B", "C", "summarizer"} and peak == 2
    assert first["capture_dir"] != second["capture_dir"]
    third, starts, peak = launch({"max_active": 1, "force_fresh": False}, selected)
    assert starts == ["B", "summarizer"], starts
    state = show()
    assert not state["binding_amendments"]
    assert state["work_slot_invocations"][0]["controls"] == {"max_active": 1, "force_fresh": False}
    assert (artifacts / "plan.json").read_bytes() == plan_bytes

    # Actual engine fan-out is the other supported facade, not an argv lookalike.
    fan_worker = root / "fan-worker.py"
    fan_worker.write_text('''import json, os, pathlib, sys, time
ownership=pathlib.Path(os.environ['LOOP_ENGINE_OWNERSHIP_DIRECTORY'])
owned=json.loads((ownership/'ownership.json').read_text())
assert pathlib.Path(owned['graph_locator']).is_file()
assert json.loads((ownership/('helper-%s.json' % os.getppid())).read_text())['root_pid']==os.getppid()
root=pathlib.Path(sys.argv[1]); index=sys.argv[2]; raw=sys.stdin.read()
start=time.monotonic(); time.sleep(.7); end=time.monotonic()
with (root/'fan-activity.jsonl').open('a') as f:
    f.write(json.dumps({'id':index,'start':start,'end':end,'fresh':json.loads(raw).get('controls',{}).get('force_fresh',False),'input':raw})+'\\n')
print(json.dumps({'id':index,'fresh':True}))
''')
    fan_binding = {"command": engine, "args": ["fan-out", "--max-active", "4"]}
    for n in range(2):
        fan_binding["args"] += ["--worker", json.dumps({"command": sys.executable, "args": [str(fan_worker), str(root), str(n)], "output_schema": {"required": ["id", "fresh"]}})]
    fan_selector = root / "fan-selector.py"
    fan_selector.write_text("import json,pathlib,sys;p=json.load(sys.stdin);pathlib.Path(" + repr(str(root / "fan-filter.json")) + ").write_text(json.dumps(p));print(json.dumps({'record_ids':[r['id'] for r in p['context']]}))")
    fan_binding["context_filter"] = {"command": sys.executable, "args": [str(fan_selector)]}
    fan_profile = dict(profile, work_slot_bindings={"intent-draft": fan_binding})
    call(["--config", str(config), "start", "--id", "fan-controls", "software-change", json.dumps(fan_profile)])
    for maximum in [1, 2]:
        show("fan-controls")
        args = ["invoke", "fan-controls", "intent-draft", "--timeout-ms", "120000", "--controls", json.dumps({"max_active": maximum, "force_fresh": True})]
        preview = call([*args, "--preview"])
        filtered = json.loads((root / "fan-filter.json").read_text())
        assert filtered["controls"] == {"max_active": maximum, "force_fresh": True, "timeout_ms": 120000}
        result = call(args)
        record = wait(result, "fan-controls")
        assert record["controls"] == preview["controls"]
        activity = [json.loads(line) for line in (root / "fan-activity.jsonl").read_text().splitlines()][-2:]
        assert all(row["fresh"] for row in activity)
        overlap = max(row["start"] for row in activity) < min(row["end"] for row in activity)
        assert overlap == (maximum == 2), activity
    show("fan-controls")
    selected_fan = call(["invoke", "fan-controls", "intent-draft", "--assignment", "worker-0", "--controls", '{"force_fresh":true}'])
    record = wait(selected_fan, "fan-controls")
    assert record["assignment_selection"] == ["worker-0"]
    assert not show("fan-controls")["binding_amendments"]
    call(["invoke", "fan-controls", "intent-draft", "--assignment", "worker-0", "--input", '{}'], "invalid-invocation")
    call(["invoke", "fan-controls", "intent-draft", "--assignment", "unknown", "--preview"], "rejected")
    call(["terminate", "fan-controls"])
    show()
    call(["terminate", "graph-controls"])
    (root / "proof.json").write_text(json.dumps({"status": "passed", "checks": ["preview-no-primary-or-invocation", "real-plan-and-fanout-concurrency", "force-fresh-execution", "selected-prerequisites", "immutable-controls", "selection-no-amendment"]}, indent=2))
    return root


def prove(journey):
    journey.work_root.mkdir(parents=True, exist_ok=True)
    prove_bindings(str(journey.engine), str(journey.provider))
    prove_facades(str(journey.engine), str(journey.provider), journey.data_root, journey.work_root)
    print("recovery execution-controls scenario passed", flush=True)


if __name__ == "__main__":
    engine, provider = (str(Path(arg).resolve()) for arg in sys.argv[1:3])
    prove_bindings(engine, provider)
    prove_facades(engine, provider, Path.cwd())
    print("recovery execution-controls scenario passed", flush=True)
