"""Public implement owning-route proof, with retained isolated catalogs/captures.

The historical fixture is an actual pre-change reviewless describe response.
Its scripted provider is replaced after start: only the stored graph remains
old. No database edits, report fabrication, or repository rollback is used.
"""
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import time

from recovery_cancellation import Fixture as ExecutionFixture, process_table, wait_for

ROUTES = {"revise-plan": "plan", "revise-design": "design", "revise-intent": "explore"}
WORKER = '''import json,os,pathlib,sys,time
root=pathlib.Path(sys.argv[1]); packet=json.load(sys.stdin)
assert packet['slot_id']=='implement'
assert not (pathlib.Path(packet['artifact_root'])/'implementation-report.json').exists()
(root/'workers.jsonl').write_text(json.dumps({'pid':os.getpid()})+'\\n')
print('rejected work capture retained',flush=True)
print('rejected work stderr retained',file=sys.stderr,flush=True)
(root/'ready').touch()
while not (root/'release').exists(): time.sleep(.01)
print('work finished without a report',flush=True)
'''


class Fixture(ExecutionFixture):
    def __init__(self, engine, provider, checkout, work_root, name, historical=False):
        self.root = Path(tempfile.mkdtemp(prefix=f"recovery-backtracking-{name}-", dir=work_root))
        print(f"backtracking fixture {name}: {self.root}", flush=True)
        self.engine, self.provider, self.checkout = engine, provider, str(checkout)
        self.name, self.slot, self.transcript = name, "implement", []
        self.db, self.artifacts = self.root / "loop.sqlite", self.root / "artifacts"
        self.artifacts.mkdir()
        self.worker = self.root / "worker.py"
        self.worker.write_text(WORKER)
        self.config = self.root / "providers.toml"
        self.config.write_text('[providers.software-change]\ncommand = ' + json.dumps(provider) + '\n')
        if historical:
            self.proxy = self.root / "historical-provider.py"
            frozen = Path(checkout) / "tests/fixtures/software-change-historical-reviewless.json"
            self.proxy.write_text('#!' + sys.executable + '\nimport json,sys\nr=json.load(sys.stdin)\n'
                + f"print(open({str(frozen)!r}).read() if r['operation']=='describe' else json.dumps({{'result':'allow'}}))\n")
            self.proxy.chmod(0o755)
            self.config.write_text('[providers.software-change]\ncommand = ' + json.dumps(str(self.proxy)) + '\n')
        schema = {"type": "object", "required": ["revision", "author"], "properties": {
            "revision": {"type": "string"}, "author": {"type": "object", "required": ["name", "kind"],
                "properties": {"name": {"type": "string"}, "kind": {"type": "string", "enum": ["script"]}}},
            "acceptance": {"type": "array", "items": {"type": "object", "required": ["id", "statement"],
                "properties": {"id": {"type": "string"}, "statement": {"type": "string"}}}}}}
        profile = {"contract_version": 3, "criterion_policy": {"required_authors": 1, "goal_required_authors": 1}, "config_version": "recovery-backtracking-3",
            "artifact_root": str(self.artifacts), "review_policies": {},
            "artifact_schemas": {n + ".json": schema for n in ("intent", "design", "plan")},
            "revision_links": [{"from": "design.json", "field": "intent_revision", "to": "intent.json"},
                               {"from": "plan.json", "field": "design_revision", "to": "design.json"}],
            "work_slot_bindings": {"implement": {"command": sys.executable, "args": [str(self.worker), str(self.root)]}}}
        (self.artifacts / "intent.json").write_text(json.dumps({"revision": "1", "author": {"name": "fixture", "kind": "script"},
            "acceptance": [{"id": "AC-1", "statement": "backtracking keeps rejected work observable"}]}))
        (self.artifacts / "design.json").write_text(json.dumps({"revision": "1", "author": {"name": "fixture", "kind": "script"}, "intent_revision": "1"}))
        self.call(["--config", str(self.config), "start", "--id", name, "software-change", json.dumps(profile)])
        for event in ("intent-ready", "design-ready"):
            self.show()
            self.call(["event", name, event])
        if not historical:
            self.show()
            # A genuine provider denial is durable before the later rescope.
            self.call(["event", name, "plan-ready"], "rejected")
        (self.artifacts / "plan.json").write_text(json.dumps({"revision": "1", "author": {"name": "fixture", "kind": "script"},
            "design_revision": "1", "tasks": [{"id": "A", "criterion_ids": ["AC-1"]}], "dependency_graph": []}))
        self.show()
        self.call(["event", name, "plan-ready"])
        self.before = self.show()
        assert self.before["current_state"] == "implement"
        self.preserved_artifacts = {p: p.read_bytes() for p in self.artifacts.glob("*.json")}
        self.repository_edit = self.root / "rejected-repository-edit.txt"
        self.repository_edit.write_text("driver-owned rejected work must not be rolled back\n")
        self.no_report()

    def no_report(self):
        assert not (self.artifacts / "implementation-report.json").exists()
        assert not (self.artifacts / "implementation-checkpoint.json").exists()

    def launch(self):
        self.show()
        self.result = self.call(["invoke", self.name, self.slot, "--timeout-ms", "2000"])
        self.capture = Path(self.result["capture_dir"])
        self.ownership = self.capture / "ownership"
        wait_for(lambda: (self.root / "ready").exists(), "bound implement worker ready")
        self.no_report()

    def refusals(self):
        before = self.show()
        rows = len(before["work_slot_invocations"])
        for event in ROUTES:
            value = self.call(["event", self.name, event], "rejected")
            assert value["code"] == "live-owned-work", value
            assert self.result["invocation_id"] in json.dumps(value)
            assert "cancel-invocation" in json.dumps(value)
            self.no_report()
        value = self.call(["invoke", self.name, self.slot], "rejected")
        assert self.result["invocation_id"] in json.dumps(value), value
        after = self.show()
        assert after["current_state"] == "implement" and after["state_visit"] == before["state_visit"]
        assert len(after["work_slot_invocations"]) == rows

    def depart(self, event):
        self.no_report()
        before = self.show()
        rows = before["work_slot_invocations"]
        assert not rows[0]["ownership"]["live_owned_work"] and not rows[0]["ownership"]["cleanup_pending"]
        captures = {p: p.read_bytes() for p in self.capture.rglob("*") if p.is_file()}
        assert any(b"rejected work capture retained" in value for value in captures.values())
        history = self.call(["history", self.name])
        self.call(["event", self.name, event])
        after = self.show()
        assert after["current_state"] == ROUTES[event]
        assert after["initial_input"] == before["initial_input"]
        assert after["context"] == before["context"]
        # Projections may change with the visit, but durable attempts may not.
        for key in ("invocation_id", "capture_dir", "binding", "completed_at", "exit_code", "controls", "instruction_digest", "routed_inputs", "started_at", "subject"):
            assert [r.get(key) for r in after["work_slot_invocations"]] == [r.get(key) for r in rows], key
        for path, value in {**self.preserved_artifacts, **captures}.items():
            assert path.read_bytes() == value, path
        assert self.repository_edit.read_text() == "driver-owned rejected work must not be rolled back\n"
        later = self.call(["history", self.name])
        # History response is an append-only list of entries, including denial.
        assert later[:len(history)] == history
        assert any("denied" in json.dumps(entry) for entry in history), history
        self.no_report()
        (self.root / "proof.json").write_text(json.dumps({"status": "passed", "run_id": self.name,
            "database": str(self.db), "event": event, "target": after["current_state"],
            "no_report": True, "retained_capture_files": [str(p) for p in captures],
            "retained_history_entries": len(history)}, indent=2))


def cleanup_pending(f):
    # Interrupt after public cancellation admission, before any signal. Then
    # finish the worker normally: absence of live PIDs must NOT release the
    # barrier while the controller's cleanup acknowledgment is outstanding.
    gate = f.root / "path-gate"
    gate.mkdir()
    real_ps = shutil.which("ps")
    ps = gate / "ps"
    ps.write_text('#!' + sys.executable + '\nimport os,pathlib,time\n'
        + f"root=pathlib.Path({str(f.root)!r})\n(root/'controller-ready').write_text(str(os.getpid()))\n"
        + "while not (root/'release-inventory').exists(): time.sleep(.005)\n"
        + f"os.execv({real_ps!r},[{real_ps!r},'-axo','pid=,ppid=,pgid=,stat='])\n")
    ps.chmod(0o755)
    controller = subprocess.Popen(f.argv(f.cancel_args()), env=dict(os.environ, PATH=str(gate) + os.pathsep + os.environ["PATH"]),
        stdout=(f.root / "interrupted.stdout").open("w"), stderr=(f.root / "interrupted.stderr").open("w"))
    try:
        wait_for(lambda: (f.root / "controller-ready").exists(), "public cancellation admitted")
        original = (f.ownership / "attempt-1.json").read_bytes()
        assert json.loads(original)["outcome"] == "incomplete"
    finally:
        controller.kill()
        controller.wait(timeout=5)
        (f.root / "release-inventory").touch()
    gate_pid = int((f.root / "controller-ready").read_text())
    wait_for(lambda: gate_pid not in process_table(), "inventory helper reaped")
    (f.root / "release").touch()
    wait_for(lambda: not f.invocation()["ownership"]["live_owned_work"], "owned work finished")
    assert f.invocation()["ownership"]["cleanup_pending"]
    f.refusals()
    f.cancel()
    assert (f.ownership / "attempt-1.json").read_bytes() == original


def current(engine, provider, checkout, work_root, mode):
    roots = []
    for event in ROUTES:
        f = Fixture(engine, provider, checkout, work_root, mode + "-" + event)
        try:
            f.launch()
            assert f.invocation()["status"] == "running"
            f.refusals()
            time.sleep(2.1)
            assert f.invocation()["status"] == "overrun"
            f.refusals()
            if mode == "complete":
                (f.root / "release").touch()
                wait_for(lambda: f.invocation()["completed_at"], "ordinary work completed")
                assert f.invocation()["status"] == "succeeded"
            elif mode == "cancel":
                f.cancel()
            else:
                cleanup_pending(f)
            f.depart(event)
            pids = {json.loads(line)["pid"] for line in (f.root / "workers.jsonl").read_text().splitlines()}
            assert not pids.intersection(process_table()), "worker not reaped"
            roots.append(f.root)
        except BaseException:
            f.emergency_cleanup()
            raise
    return roots


def historical(engine, provider, checkout, work_root):
    f = Fixture(engine, provider, checkout, work_root, "historical", historical=True)
    initial = f.show()
    # Replace the executable at its stored association, not the association,
    # profile, database, or stored workflow. It now forwards to the new binary.
    f.proxy.write_text('#!' + sys.executable + '\nimport os\n' + f"os.execv({provider!r},[{provider!r}])\n")
    described = subprocess.run([str(f.proxy)], input=json.dumps({"operation": "describe", "initial_input": {"review_policies": {}}}),
        text=True, capture_output=True, check=True, timeout=40)
    (f.root / "upgraded-describe.json").write_text(described.stdout)
    new_routes = {edge["event"]: edge["target"] for edge in json.loads(described.stdout)["transitions"]
        if edge["source"] == "implement" and edge["kind"] == "check-free"}
    assert new_routes == ROUTES, new_routes
    history = f.call(["history", f.name])
    after = f.show()
    assert after["requestable_events"] == initial["requestable_events"]
    assert [e["event"] for e in after["requestable_events"]] == ["implementation-ready"]
    for event in ROUTES:
        value = f.call(["event", f.name, event], "rejected")
        assert value["code"] == "event-unavailable", value
    assert f.call(["history", f.name]) == history
    assert f.show()["current_state"] == "implement"
    f.no_report()
    (f.root / "proof.json").write_text(json.dumps({"status": "passed", "run_id": f.name,
        "database": str(f.db), "historical_capability_absent": True,
        "available_events": after["requestable_events"]}, indent=2))
    return f.root


def run(engine, provider, checkout, work_root, case="all"):
    Path(work_root).mkdir(parents=True, exist_ok=True)
    roots = []
    for mode in ("complete", "cancel", "pending"):
        if case in ("all", mode):
            roots.extend(current(engine, provider, checkout, work_root, mode))
    if case in ("all", "historical"):
        roots.append(historical(engine, provider, checkout, work_root))
    assert roots, case
    print(json.dumps({"scenario": "backtracking", "case": case, "status": "passed",
        "proof_roots": [str(p) for p in roots]}), flush=True)


def prove(journey):
    run(str(journey.engine), str(journey.provider), journey.data_root, journey.work_root)
    print("recovery backtracking scenario passed: three owning routes, live/overrun/cleanup refusals, no report, retained history and old graph", flush=True)


if __name__ == "__main__":
    run(str(Path(sys.argv[1]).resolve()), str(Path(sys.argv[2]).resolve()), Path.cwd(),
        sys.argv[3] if len(sys.argv) > 3 else tempfile.mkdtemp(prefix="recovery-backtracking-"),
        sys.argv[4] if len(sys.argv) > 4 else "all")
