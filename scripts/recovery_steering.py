"""Public steering/filter proof. All catalogs and workers are isolated fixtures."""
import json
import subprocess
import sys
import tempfile
import time
from pathlib import Path


def prove(journey):
    journey.work_root.mkdir(parents=True, exist_ok=True)
    root = Path(tempfile.mkdtemp(prefix="recovery-steering-", dir=journey.work_root))
    print(f"steering captures: {root}", flush=True)
    artifacts = root / "artifacts"
    artifacts.mkdir()
    db = root / "loop.sqlite"
    transcript = []

    def write(path, value):
        path.write_text(json.dumps(value, indent=2) + "\n")

    def call(args, expected="completed"):
        process = subprocess.run([str(journey.engine), "--database", str(db), "--json", *args],
                                 cwd=journey.data_root, capture_output=True, timeout=45)
        value = json.loads(process.stdout)
        transcript.append({"argv": args, "exit": process.returncode, "envelope": value,
                           "stderr": process.stderr.decode()})
        write(root / "transcript.json", transcript)
        assert value["status"] == expected, value
        assert process.returncode == (0 if expected == "completed" else 10 if expected == "rejected" else 20), value
        return value

    def show():
        return call(["show", "--view", "full", "steering"])

    def append(record_id, data, kind="user-steering"):
        show()
        call(["append", "steering", "--kind", kind, "--record-id", record_id, json.dumps(data)])

    worker = root / "worker.py"
    worker.write_text('''import json, pathlib, sys, time
p=json.load(sys.stdin)
root=pathlib.Path(p['artifact_root'])
(root/'started').write_text('started')
while (root/'hold').exists(): time.sleep(.02)
steering=[r for r in p.get('context',[]) if r['kind']=='user-steering']
output=json.dumps({'outcome':[r['data']['instruction'] for r in steering], 'context':p.get('context',[])})
(pathlib.Path(p['capture_dir'])/'stdout').write_text(output)
print(output)
''')
    filter_binding = {"command": str(journey.provider), "args": ["commission"]}
    binding = {"command": sys.executable, "args": [str(worker)], "context_filter": filter_binding}
    config = root / "providers.toml"
    config.write_text('[providers.software-change]\ncommand = ' + json.dumps(str(journey.provider)) + '\n')
    schema = {"type": "object", "required": ["revision", "author"], "properties": {
        "revision": {"type": "string"}, "author": {"type": "object", "required": ["name", "kind"],
        "properties": {"name": {"type": "string"}, "kind": {"type": "string", "enum": ["human", "agent", "script"]}}}}}
    profile = {"contract_version": 2, "criterion_policy": {"required_authors": 1, "goal_required_authors": 1}, "config_version": "recovery-steering-2", "artifact_root": str(artifacts),
               "review_policies": {"intent-review": [{"id": "axis", "description": "steering", "required_authors": 1}]},
               "artifact_schemas": {name: schema for name in ["intent.json", "design.json", "plan.json"]},
               "work_slot_bindings": {slot: binding for slot in ["intent-draft", "intent-review", "implement"]}}
    write(root / "profile.json", profile)
    for name in ["intent", "design"]:
        write(artifacts / (name + ".json"), {"revision": "1", "author": {"name": "subject", "kind": "script"}})
    plan = {"revision": "1", "author": {"name": "subject", "kind": "script"},
            "tasks": [{"id": "A"}, {"id": "B"}], "dependency_graph": [],
            "proof_commands": [{"id": "final", "owner": "driver", "command": "old", "args": [], "obligation": "complete proof"}]}
    write(artifacts / "plan.json", plan)
    original_plan = (artifacts / "plan.json").read_bytes()
    call(["--config", str(config), "start", "--id", "steering", "software-change", "@" + str(root / "profile.json"), "steering"])

    def inspect(slot, task=None):
        args = [str(journey.provider), "commission", "--slot", slot]
        if task:
            args += ["--task", task]
        p = subprocess.run(args, input=json.dumps(show()).encode(), capture_output=True, timeout=30)
        assert p.returncode == 0, p.stderr
        value = json.loads(p.stdout)
        write(root / (slot + ("-" + task if task else "") + "-commission.json"), value)
        return value

    def invoke(slot):
        expected = inspect(slot)["context"]
        result = call(["invoke", "steering", slot])["result"]
        return result, expected

    def completed(result, expected):
        deadline = time.monotonic() + 40
        while True:
            state = show()["result"]
            invocation = next(i for i in state["work_slot_invocations"] if i["invocation_id"] == result["invocation_id"])
            if invocation.get("completed_at") is not None:
                assert invocation["status"] == "succeeded", invocation
                break
            assert time.monotonic() < deadline, invocation
            time.sleep(.05)
        output = json.loads((Path(result["capture_dir"]) / "stdout").read_text())
        assert output["context"] == expected, (output, expected)
        # Retained invocation commission, not a reconstruction from current context.
        assert invocation["routed_inputs"] == expected, invocation
        return output

    append("red", {"target": {"kind": "all"}, "instruction": "red"})
    (artifacts / "hold").touch()
    first, expected = invoke("intent-draft")
    deadline = time.monotonic() + 20
    while not (artifacts / "started").exists():
        assert time.monotonic() < deadline
        time.sleep(.02)
    append("blue", {"target": {"kind": "all"}, "instruction": "blue", "supersedes": ["red"],
                    "proof_updates": [{"proof_id": "final", "owner": "proof-owner", "command": "new", "args": ["focused"], "reason": "same obligation, explicit executor"}]})
    (artifacts / "hold").unlink()
    red = completed(first, expected)
    second, expected = invoke("intent-draft")
    blue = completed(second, expected)
    assert red["outcome"] == ["red"] and blue["outcome"] == ["blue"]
    receipt = inspect("intent-draft")["commission"]
    assert receipt["proof_commands"][0] == {"id": "final", "owner": "proof-owner", "command": "new", "args": ["focused"], "obligation": "complete proof"}
    assert (artifacts / "plan.json").read_bytes() == original_plan
    append("receipt", {"steering_ids": ["blue"], "applied": "Changed execution ownership and command only; plan and proof obligation unchanged"}, "steering-incorporation")
    show()
    call(["event", "steering", "intent-ready"])
    review, expected = invoke("intent-review")
    assert completed(review, expected)["outcome"] == ["blue"]
    append("pass", {"gate": "intent-review", "policy_id": "axis", "result": "pass", "findings": "",
                    "author": {"name": "reviewer", "kind": "script"}, "subject": "intent.json", "subject_revision": "1", "config_version": "recovery-steering-2"}, "review-evidence")
    append("ledger", {"schema_version": "1", "gate": "intent-review", "subject": "intent.json", "subject_revision": "1",
                      "author": {"name": "driver", "kind": "agent"}, "findings": []}, "finding-ledger")
    show()
    call(["event", "steering", "approved"])
    for event in ["design-ready", "plan-ready"]:
        show()
        call(["event", "steering", event])
    append("task-a", {"target": {"kind": "tasks", "plan_revision": "1", "ids": ["A"]}, "instruction": "only-A"})
    append("stale", {"target": {"kind": "tasks", "plan_revision": "0", "ids": ["removed"]}, "instruction": "stale"})
    implementation, expected = invoke("implement")
    assert completed(implementation, expected)["outcome"] == ["blue", "only-A"]
    assert inspect("implement", "A")["commission"]["steering_ids"] == ["blue", "task-a"]
    assert inspect("implement", "B")["commission"]["steering_ids"] == ["blue"]
    assert inspect("implement", "summarizer")["commission"]["steering_ids"] == ["blue"]

    # The same public transport refuses bad selectors before spawning primary work.
    # Separate starts freeze each binding; no production state is modified.
    for name, returned, exit_code in [("unknown", ["missing"], 0), ("duplicate", ["one", "one"], 0),
                                      ("reordered", ["two", "one"], 0), ("altered", [{"id": "one", "data": "rewritten"}], 0), ("failed", [], 7)]:
        marker = root / (name + "-primary-started")
        bad_profile = dict(profile)
        bad_profile["work_slot_bindings"] = {"intent-draft": {
            "command": sys.executable, "args": ["-c", "from pathlib import Path; Path(" + repr(str(marker)) + ").touch()"],
            "context_filter": {"command": sys.executable, "args": ["-c", "import sys;sys.stdin.read();print(" + repr(json.dumps({"record_ids": returned})) + ");sys.exit(" + str(exit_code) + ")"]}}}
        write(root / (name + ".json"), bad_profile)
        call(["--config", str(config), "start", "--id", name, "software-change", "@" + str(root / (name + ".json")), name])
        for record_id in ["one", "two"]:
            call(["show", "--view", "full", name])
            call(["append", name, "--kind", "user-steering", "--record-id", record_id, json.dumps({"target": {"kind": "all"}, "instruction": record_id})])
        call(["show", "--view", "full", name])
        call(["invoke", name, "intent-draft"], "error")
        assert not marker.exists()
        assert not call(["show", "--view", "full", name])["result"]["work_slot_invocations"]
    append("invalid-target", {"target": {"kind": "slots", "ids": ["missing"]}, "instruction": "invalid"})
    before = len(show()["result"]["work_slot_invocations"])
    call(["invoke", "steering", "implement"], "error")
    assert len(show()["result"]["work_slot_invocations"]) == before
    # Actual bound plan graph, with exact task selection and an untargeted task.
    graph_artifacts = root / "graph-artifacts"
    graph_artifacts.mkdir()
    for name in ["intent.json", "design.json", "plan.json"]:
        (graph_artifacts / name).write_bytes((artifacts / name).read_bytes())
    graph_worker = root / "graph-worker.py"
    graph_worker.write_text('''import json, pathlib, sys
location, assignment=sys.stdin.read().split('\\n---\\n\\n',1)
p=json.loads(location); root=pathlib.Path(p['artifact_root'])
if 'plan_path' in p:
    report={'revision':'graph-1','author':{'name':'dummy','kind':'script'},'plan_revision':'1','coverage':['fixture'],'summary':'deterministic steering proof','changed_surface':['fixture'],'validation':['focused steering']}
    (root/'implementation-report.json').write_text(json.dumps(report))
    (root/'summarizer-receipt').write_text(assignment)
else:
    task=json.loads(assignment)
    (root/(task['id']+'-receipt.json')).write_text(json.dumps(task))
    print(json.dumps({'task':task['id'],'outcome':[r['data']['instruction'] for r in task.get('steering_context',[])]}))
''')
    graph_profile = dict(profile)
    graph_profile["artifact_root"] = str(graph_artifacts)
    graph_profile["review_policies"] = {}
    graph_profile["work_slot_bindings"] = {"implement": {"command": str(journey.provider),
        "args": ["run-plan-graph", "--working-directory", str(journey.data_root), "--max-active", "1", "--task-worker", json.dumps({"command": sys.executable, "args": [str(graph_worker)]})],
        "context_filter": filter_binding}}
    write(root / "graph-profile.json", graph_profile)
    call(["--config", str(config), "start", "--id", "graph-steering", "software-change", "@" + str(root / "graph-profile.json"), "graph"])
    for event in ["intent-ready", "design-ready", "plan-ready"]:
        call(["show", "--view", "full", "graph-steering"])
        call(["event", "graph-steering", event])
    for record_id, data in [("global", {"target": {"kind": "all"}, "instruction": "global"}),
                            ("exact", {"target": {"kind": "tasks", "plan_revision": "1", "ids": ["A"]}, "instruction": "exact-A"})]:
        call(["show", "--view", "full", "graph-steering"])
        call(["append", "graph-steering", "--kind", "user-steering", "--record-id", record_id, json.dumps(data)])
    call(["show", "--view", "full", "graph-steering"])
    graph = call(["--timeout-ms", "120000", "invoke", "graph-steering", "implement", "--input", json.dumps({"plan_revision": "1", "task_roots": ["A", "B"]})])["result"]
    deadline = time.monotonic() + 120
    while True:
        invocation = next(i for i in call(["show", "--view", "full", "graph-steering"])["result"]["work_slot_invocations"] if i["invocation_id"] == graph["invocation_id"])
        if invocation.get("completed_at") is not None:
            assert invocation["status"] == "succeeded", invocation
            break
        assert time.monotonic() < deadline, invocation
        time.sleep(.1)
    for task, expected in [("A", ["global", "exact"]), ("B", ["global"])]:
        packet = json.loads((graph_artifacts / (task + "-receipt.json")).read_text())
        assert [r["id"] for r in packet["steering_context"]] == expected, packet
    summary_receipt = (graph_artifacts / "summarizer-receipt").read_text()
    assert 'global' in summary_receipt and 'exact-A' not in summary_receipt
    assert (graph_artifacts / "implementation-checkpoint.json").is_file()
    write(root / "proof.json", {"status": "passed", "passed": ["changed-worker-outcome", "snapshot-after-append", "draft-review-implementation-delivery", "inspection-agreement", "selected-task-delivery-and-exclusion", "proof-owner-command-no-plan-edit", "filter-refusals"]})
    print("recovery steering passed: changed outcomes, immutable commission, later recipients, exact tasks, proof updates and fail-closed filters", flush=True)
