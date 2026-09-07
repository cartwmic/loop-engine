"""Public cancellation proof. Real local processes/Dagu, isolated catalogs only.

A nested fixture graph creates two genuinely eligible scheduler frontiers in the
controller-free gap: its queued task and the outer graph's summarizer. Neither
is merely waiting on a still-running prerequisite. A PATH-local ps gate permits
precise controller interruption after durable stop admission, before signaling;
it does not simulate worker exits, Dagu, cleanup, or the resumed controller.
"""
import json
import os
from pathlib import Path
import platform
import shutil
import signal
import subprocess
import sys
import tempfile
import time


def wait_for(predicate, description, seconds=30):
    end = time.monotonic() + seconds
    while True:
        value = predicate()
        if value:
            return value
        assert time.monotonic() < end, description
        time.sleep(.01)


def process_table():
    raw = subprocess.check_output(["ps", "-axo", "pid=,ppid=,pgid=,stat="], text=True, timeout=10)
    return {int(f[0]): {"parent": int(f[1]), "group": int(f[2]), "state": f[3]}
            for line in raw.splitlines() if len(f := line.split()) == 4}


WORKER = r'''import json, os, pathlib, signal, subprocess, sys, time
mode, root, provider, checkout = sys.argv[1:]
root=pathlib.Path(root)
raw=sys.stdin.read()
if mode=='direct':
    packet=json.loads(raw); name='direct'
else:
    location, assignment=raw.split('\n---\n\n',1)
    packet=json.loads(location)
    name='summarizer' if 'plan_path' in packet else json.loads(assignment)['id']
with (root/'workers.jsonl').open('a') as f:
    f.write(json.dumps({'pid':os.getpid(),'name':name,'mode':mode,'at':time.monotonic()})+'\n')
print('partial stdout '+name, flush=True)
print('partial stderr '+name, file=sys.stderr, flush=True)
if name in ('B','summarizer'):
    (root/(mode+'-'+name+'-FORBIDDEN')).touch()
    sys.exit(91)
if mode=='gap-outer':
    nested=root/'nested'; nested.mkdir()
    (nested/'plan.json').write_text(json.dumps({'revision':'1','tasks':[{'id':'A'},{'id':'B'}], 'dependency_graph':[{'from':'A','to':'B'}]}))
    capture=nested/'capture'; capture.mkdir()
    packet={'run_id':'nested-fixture','slot_id':'implement','artifact_root':str(nested),
        'instruction_body':'deterministic nested frontier','capture_dir':str(capture)}
    binding={'command':sys.executable,'args':[__file__,'gap-nested',str(root),provider,checkout]}
    child=subprocess.Popen([provider,'run-plan-graph','--working-directory',checkout,'--max-active','1',
        '--task-worker',json.dumps(binding)],stdin=subprocess.PIPE,
        stdout=(nested/'stdout').open('w'),stderr=(nested/'stderr').open('w'))
    child.stdin.write(json.dumps(packet).encode());child.stdin.close()
    (root/'nested-facade.pid').write_text(str(child.pid))
if mode in ('graph','direct'):
    # A child in a new group exercises descendant discovery, not just killpg
    # of the facade. It ignores TERM; its real parent waits/reaps on shutdown.
    child=subprocess.Popen([sys.executable,'-c',
        'import os,pathlib,signal,sys,time; signal.signal(signal.SIGTERM,signal.SIG_IGN); pathlib.Path(sys.argv[1]).write_text(str(os.getpid())); print("resistant descendant",flush=True); time.sleep(120)',str(root/'resistant.pid')],start_new_session=True)
    def shutdown(*args):
        child.wait()
        sys.exit(7)
    signal.signal(signal.SIGTERM,shutdown)
(root/(mode+'-ready')).touch()
while not (root/'release').exists(): time.sleep(.01)
(root/(mode+'-prerequisite-finished')).write_text(str(time.monotonic()))
print('actual successful prerequisite '+name,flush=True)
'''


class Fixture:
    def __init__(self, engine, provider, checkout, work_root, name, mode):
        self.root = Path(tempfile.mkdtemp(prefix=f"recovery-cancellation-{name}-", dir=work_root))
        print(f"cancellation fixture {name}: {self.root}", flush=True)
        self.engine, self.provider, self.checkout = engine, provider, str(checkout)
        self.db, self.artifacts = self.root / "loop.sqlite", self.root / "artifacts"
        self.artifacts.mkdir()
        self.name, self.mode, self.transcript = name, mode, []
        self.worker = self.root / "worker.py"
        self.worker.write_text(WORKER)
        self.config = self.root / "providers.toml"
        self.config.write_text('[providers.software-change]\ncommand = ' + json.dumps(provider) + '\n')
        worker = {"command": sys.executable, "args": [str(self.worker), mode, str(self.root), provider, str(checkout)]}
        self.slot = "intent-draft" if mode == "direct" else "implement"
        binding = worker if mode == "direct" else {"command": provider, "args": ["run-plan-graph", "--working-directory", str(checkout), "--max-active", "1", "--task-worker", json.dumps(worker)]}
        schema = {"type": "object", "required": ["revision", "author"], "properties": {
            "revision": {"type": "string"}, "author": {"type": "object", "required": ["name", "kind"],
                "properties": {"name": {"type": "string"}, "kind": {"type": "string", "enum": ["human", "agent", "script"]}}}}}
        self.profile = {"contract_version": 2, "criterion_policy": {"required_authors": 1, "goal_required_authors": 1}, "config_version": "recovery-cancellation-2", "artifact_root": str(self.artifacts),
            "review_policies": {}, "artifact_schemas": {n + ".json": schema for n in ("intent", "design", "plan")},
            "work_slot_bindings": {self.slot: binding}}
        for name in ("intent", "design", "plan"):
            value = {"revision": "1", "author": {"name": "fixture", "kind": "script"}}
            if name == "plan":
                value.update(tasks=[{"id": "A"}] if mode == "gap-outer" else [{"id": "A"}, {"id": "B"}],
                    dependency_graph=[] if mode == "gap-outer" else [{"from": "A", "to": "B"}])
            (self.artifacts / (name + ".json")).write_text(json.dumps(value))
        self.call(["--config", str(self.config), "start", "--id", self.name, "software-change", json.dumps(self.profile)])
        if self.slot == "implement":
            for event in ("intent-ready", "design-ready", "plan-ready"):
                self.show()
                self.call(["event", self.name, event])
        self.before = self.show()
        self.result = self.call(["invoke", self.name, self.slot, "--timeout-ms", "100"])
        self.capture = Path(self.result["capture_dir"])
        self.ownership = self.capture / "ownership"
        try:
            wait_for(lambda: (self.root / (mode + "-ready")).exists(), "primary worker ready")
            if mode == "gap-outer":
                wait_for(lambda: (self.root / "gap-nested-ready").exists(), "nested graph primary ready")
            else:
                wait_for(lambda: (self.root / "resistant.pid").exists(), "TERM-resistant descendant ready")
            self.initial = self.show()
        except BaseException:
            self.emergency_cleanup()
            raise

    def argv(self, args):
        return [self.engine, "--database", str(self.db), "--json", *args]

    def call(self, args, status="completed"):
        p = subprocess.run(self.argv(args), cwd=self.checkout, capture_output=True, timeout=40)
        value = json.loads(p.stdout)
        self.transcript.append({"argv": self.argv(args), "at": time.monotonic(), "exit": p.returncode,
            "envelope": value, "stderr": p.stderr.decode()})
        (self.root / "transcript.json").write_text(json.dumps(self.transcript, indent=2))
        assert value["status"] == status and p.returncode == {"completed": 0, "rejected": 10, "error": 20}[status], value
        return value.get("result", value)

    def show(self):
        return self.call(["show", self.name])

    def invocation(self):
        return next(r for r in self.show()["work_slot_invocations"] if r["invocation_id"] == self.result["invocation_id"])

    def cancel_args(self):
        return ["cancel-invocation", self.name, self.result["invocation_id"]]

    def cancel(self):
        began = time.monotonic()
        result = self.call(self.cancel_args())
        elapsed = time.monotonic() - began
        assert elapsed < 10, (elapsed, result)
        assert result["attempt"]["elapsed_ms"] < 10000, result
        assert result["status"] == "failed" and result["cancelled"]
        return result

    def assert_clean(self, result, expected_attempt=1):
        final = self.show()
        assert final["current_state"] == self.before["current_state"]
        assert final["state_visit"] == self.before["state_visit"]
        assert final["initial_input"] == self.before["initial_input"]
        row = self.invocation()
        assert row["status"] == "failed" and row["completed_at"] is not None, row
        owned = row["ownership"]
        assert not owned["live_owned_work"] and not owned["cleanup_pending"], owned
        progress = self.call(["invocation-progress", self.name, self.result["invocation_id"]])
        assert progress["ownership"]["cancellation"] == owned["cancellation"]
        assert owned["cancellation"]["acknowledgment"]["attempt"] == expected_attempt
        receipt = result["attempt"]["cleanup"]
        assert receipt["verified_no_survivors"]
        pids = set(receipt["verified_absent_pids"])
        pids.update(p["pid"] for p in receipt["observed"])
        pids.update(json.loads(line)["pid"] for line in (self.root / "workers.jsonl").read_text().splitlines())
        if (self.root / "resistant.pid").exists():
            resistant = int((self.root / "resistant.pid").read_text())
            pids.add(resistant)
            assert any(s["pid"] == resistant and s["signal"] == signal.SIGKILL and s["elapsed_ms"] >= 3000 for s in receipt["signals"]), receipt
        for path in self.root.rglob("ownership/*.json"):
            value = json.loads(path.read_text())
            if isinstance(value, dict) and "root_pid" in value:
                assert value["root_pid"] in pids, ("owned root omitted from receipt", path)
        table = process_table()
        assert not pids.intersection(table), ("owned PID not reaped", pids.intersection(table))
        assert not set(receipt["verified_empty_groups"]).intersection(r["group"] for r in table.values())
        assert not list(self.root.glob("*-FORBIDDEN"))
        assert not (self.artifacts / "implementation-report.json").exists()
        output = self.ownership / "stdout" if self.mode == "direct" else self.capture / "A" / "stdout"
        stderr = self.ownership / "stderr" if self.mode == "direct" else self.capture / "A" / "stderr"
        assert "partial stdout" in output.read_text()
        assert "partial stderr" in stderr.read_text()
        history = self.call(["history", self.name])
        self.call(self.cancel_args(), "rejected")
        assert self.call(["history", self.name]) == history
        (self.root / "proof.json").write_text(json.dumps({"status": "passed", "platform": platform.system(),
            "database": str(self.db), "run_id": self.name, "result": result, "no_survivor_pids": sorted(pids),
            "captures_preserved": [str(output), str(stderr)], "workflow_unchanged": True}, indent=2))

    def emergency_cleanup(self):
        # Only for a failing test: never used to make a cancellation pass.
        rows = process_table()
        known, groups = set(), set()
        for path in self.root.rglob("ownership/*.json"):
            try:
                value = json.loads(path.read_text())
                if "root_pid" in value: known.add(value["root_pid"])
                if "process_group_id" in value: groups.add(value["process_group_id"])
            except (ValueError, OSError):
                pass
        for _ in range(20):
            old = set(known)
            known.update(pid for pid, row in rows.items() if row["parent"] in known or row["group"] in groups)
            if old == known: break
        for _ in range(100):
            live = {pid: row for pid, row in process_table().items() if pid in known}
            if not live: return
            for pid in live:
                if not any(row["parent"] == pid and not row["state"].startswith("Z") for row in live.values()):
                    try: os.kill(pid, signal.SIGKILL)
                    except ProcessLookupError: pass
            time.sleep(.05)
        raise AssertionError(f"failed fixture emergency cleanup unverified: {live}")


def ordinary(engine, provider, checkout, work_root, mode="graph", lose_waiter=False):
    f = Fixture(engine, provider, checkout, work_root, "lost-waiter" if lose_waiter else mode, mode)
    try:
        # Wrong run and unknown/completed identities refuse without changing history.
        other = dict(f.profile, artifact_root=str(f.root / "other"))
        Path(other["artifact_root"]).mkdir()
        f.call(["--config", str(f.config), "start", "--id", "other", "software-change", json.dumps(other)])
        history = f.call(["history", f.name])
        other_history = f.call(["history", "other"])
        f.call(["cancel-invocation", "other", f.result["invocation_id"]], "rejected")
        f.call(["cancel-invocation", f.name, "not-an-invocation"], "rejected")
        assert f.call(["history", f.name]) == history
        assert f.call(["history", "other"]) == other_history
        if lose_waiter:
            root_pid = f.invocation()["ownership"]["execution"]["root_pid"]
            waiter = process_table()[root_pid]["parent"]
            os.kill(waiter, signal.SIGKILL)
            wait_for(lambda: waiter not in process_table(), "original waiter reaped")
            assert f.invocation()["ownership"]["live_owned_work"]
        time.sleep(.12)
        assert "cancel-invocation" in f.invocation()["overlay_meaning"] if not lose_waiter else True
        f.call(["invoke", f.name, f.slot], "rejected")
        result = f.cancel()
        f.assert_clean(result)
        if lose_waiter:
            assert f.invocation()["exit_code"] is None, "must not invent a lost waitpid result"
        return f.root
    except BaseException:
        f.emergency_cleanup()
        raise


def interrupted(engine, provider, checkout, work_root):
    f = Fixture(engine, provider, checkout, work_root, "controller-gap", "gap-outer")
    controller = None
    try:
        gate = f.root / "path-gate"
        gate.mkdir()
        real_ps = shutil.which("ps")
        ps = gate / "ps"
        ps.write_text('#!' + sys.executable + '\n' + f'''import os,pathlib,time
root=pathlib.Path({str(f.root)!r})
(root/'controller-at-inventory').write_text(str(os.getpid()))
while not (root/'release-inventory').exists(): time.sleep(.005)
os.execv({real_ps!r},[{real_ps!r},'-axo','pid=,ppid=,pgid=,stat='])
''')
        ps.chmod(0o755)
        env = dict(os.environ, PATH=str(gate) + os.pathsep + os.environ["PATH"])
        controller = subprocess.Popen(f.argv(f.cancel_args()), env=env,
            stdout=(f.root / "interrupted-controller.stdout").open('w'),
            stderr=(f.root / "interrupted-controller.stderr").open('w'))
        wait_for(lambda: (f.root / "controller-at-inventory").exists(), "controller acquired and admitted stop")
        attempt_path = f.ownership / "attempt-1.json"
        first = json.loads(attempt_path.read_text())
        original_request = (f.ownership / "stop.json").read_bytes()
        assert first["outcome"] == "incomplete"
        assert not (f.ownership / "control-1-dagu-stop.stdout").exists(), "interruption must precede scheduler stop"
        controller.kill()
        assert controller.wait(timeout=5) == -signal.SIGKILL
        controller = None
        gap_start = time.monotonic()
        (f.root / "release-inventory").touch()
        gate_pid = int((f.root / "controller-at-inventory").read_text())
        wait_for(lambda: gate_pid not in process_table(), "interrupted controller inventory helper reaped")
        # Now both real prerequisite workers finish successfully. The nested
        # scheduler attempts B; the outer scheduler attempts its summarizer.
        (f.root / "release").touch()
        for mode in ("gap-outer", "gap-nested"):
            wait_for(lambda mode=mode: (f.root / (mode + "-prerequisite-finished")).exists(), "eligible prerequisite completed")
        nested_capture = f.root / "nested" / "capture"
        def inhibited(path):
            return path.exists() and "launch inhibited" in path.read_text()
        wait_for(lambda: inhibited(nested_capture / "B" / "stderr"), "queued B admission attempted and inhibited")
        wait_for(lambda: inhibited(f.capture / "summarizer" / "stderr"), "eligible summarizer admission attempted and inhibited")
        timeline = []
        while time.monotonic() - gap_start <= 11:
            timeline.append({"at": time.monotonic(), "since_interruption": time.monotonic() - gap_start,
                "stop_marker": (f.ownership / "stop.json").exists(),
                "queued_task_inhibited": inhibited(nested_capture / "B" / "stderr"),
                "summarizer_inhibited": inhibited(f.capture / "summarizer" / "stderr"),
                "forbidden_sentinels": [str(p) for p in f.root.glob("*-FORBIDDEN")]})
            assert not timeline[-1]["forbidden_sentinels"]
            time.sleep(.2)
        (f.root / "gap-timeline.json").write_text(json.dumps(timeline, indent=2))
        assert timeline[-1]["since_interruption"] > 10
        assert all(r["queued_task_inhibited"] and r["summarizer_inhibited"] and r["stop_marker"] for r in timeline)
        assert json.loads(attempt_path.read_text()) == first, "interrupted attempt rewritten"
        row = f.invocation()
        assert row["completed_at"] is None and row["ownership"]["cleanup_pending"], row
        f.call(["invoke", f.name, f.slot], "rejected")
        result = f.cancel()
        f.assert_clean(result, expected_attempt=2)
        assert json.loads(attempt_path.read_text()) == first
        assert (f.ownership / "stop.json").read_bytes() == original_request
        assert len(f.invocation()["ownership"]["cancellation"]["attempts"]) == 2
        assert result["attempt"]["attempt"] == 2
        return f.root
    except BaseException:
        if controller is not None:
            controller.kill(); controller.wait(timeout=5)
        (f.root / "release-inventory").touch()
        f.emergency_cleanup()
        raise


def unverified_deadline(engine, provider, checkout, work_root):
    f = Fixture(engine, provider, checkout, work_root, "unverified-deadline", "direct")
    try:
        gate = f.root / "path-gate"
        gate.mkdir()
        ps = gate / "ps"
        ps.write_text('#!' + sys.executable + '\nimport os,pathlib,time\npathlib.Path(' +
            repr(str(f.root / "stalled-inventory.pid")) + ').write_text(str(os.getpid()))\ntime.sleep(60)\n')
        ps.chmod(0o755)
        env = dict(os.environ, PATH=str(gate) + os.pathsep + os.environ["PATH"])
        began = time.monotonic()
        p = subprocess.run(f.argv(f.cancel_args()), env=env, capture_output=True, timeout=12)
        value = json.loads(p.stdout)
        assert p.returncode == 20 and value["code"] == "cancellation-cleanup-pending", value
        assert time.monotonic() - began < 11, "active attempt was not bounded"
        first_path = f.ownership / "attempt-1.json"
        first = first_path.read_bytes()
        attempt = json.loads(first)
        assert attempt["outcome"] == "unverified" and 9900 <= attempt["elapsed_ms"] < 11000
        assert not (f.ownership / "cleanup-acknowledged.json").exists()
        stalled_pid = int((f.root / "stalled-inventory.pid").read_text())
        assert stalled_pid not in process_table(), "timed-out inventory helper not reaped"
        row = f.invocation()
        assert row["completed_at"] is None and row["ownership"]["cleanup_pending"] and row["ownership"]["live_owned_work"], row
        f.call(["invoke", f.name, f.slot], "rejected")
        original = (f.ownership / "stop.json").read_bytes()
        result = f.cancel()
        f.assert_clean(result, expected_attempt=2)
        assert first_path.read_bytes() == first
        assert (f.ownership / "stop.json").read_bytes() == original
        (f.root / "unverified-deadline.json").write_text(json.dumps({"initial_error": value,
            "initial_attempt": attempt, "retry": result}, indent=2))
        return f.root
    except BaseException:
        f.emergency_cleanup()
        raise


def natural_races(engine, provider, checkout, work_root):
    # Real short worker completion races the same catalog/admission path. Either
    # ordinary completion wins without a stop mutation, or cancellation wins
    # and cannot be rewritten as successful invocation completion.
    roots = []
    for n in range(4):
        f = Fixture(engine, provider, checkout, work_root, f"natural-{n}", "gap-outer")
        try:
            # Avoid the deliberate fail-if-run graph frontiers: terminate these
            # prerequisite workers by ordinary nonzero exits, not fixture kill.
            # A direct completed command is installed for the next invocation.
            result = f.cancel()
            f.assert_clean(result)
            f.show()
            binding = {"command": sys.executable, "args": ["-c", "import sys,time;sys.stdin.read();time.sleep(.15);print('natural completion')"]}
            if n == 3:
                binding["args"] = ["-c", "import sys,time,pathlib;sys.stdin.read()\nwhile not pathlib.Path(" + repr(str(f.root / "release-natural")) + ").exists(): time.sleep(.01)\nprint('natural completion')"]
            f.call(["amend-binding", f.name, f.slot, json.dumps({"state_visit": f.before["state_visit"], "owner": "fixture", "reason": "natural completion race", "binding": binding})])
            f.show()
            launched = f.call(["invoke", f.name, f.slot])
            ownership = Path(launched["capture_dir"]) / "ownership"
            wait_for(lambda: (ownership / "ownership.json").exists(), "racing ownership")
            if n == 0:
                wait_for(lambda: any(r["invocation_id"] == launched["invocation_id"] and r["completed_at"] for r in f.show()["work_slot_invocations"]), "ordinary completion")
            elif n == 1:
                time.sleep(.15)
            elif n == 3:
                owned_root = json.loads((ownership / "ownership.json").read_text())["root_pid"]
                waiter = process_table()[owned_root]["parent"]
                os.kill(waiter, signal.SIGKILL)
                (f.root / "release-natural").touch()
                wait_for(lambda: waiter not in process_table() and owned_root not in process_table(), "non-running unwritten target disappeared")
            history = f.call(["history", f.name])
            p = subprocess.run(f.argv(["cancel-invocation", f.name, launched["invocation_id"]]), capture_output=True, timeout=12)
            value = json.loads(p.stdout)
            assert value["status"] in ("completed", "rejected"), value
            if value["status"] == "rejected":
                assert not (ownership / "stop.json").exists()
                # A natural waiter can finish concurrently with the read; after
                # the completed case (n=0), history must be byte-for-byte stable.
                if n in (0, 3): assert f.call(["history", f.name]) == history
            else:
                assert value["result"]["status"] == "failed"
                assert value["result"]["attempt"]["elapsed_ms"] < 10000
            rows = f.show()["work_slot_invocations"]
            row = next(r for r in rows if r["invocation_id"] == launched["invocation_id"])
            assert not row["ownership"]["live_owned_work"]
            if n == 3:
                assert value["status"] == "rejected" and row["completed_at"] is None and row["exit_code"] is None
            else:
                assert row["completed_at"]
                assert row["status"] == ("failed" if value["status"] == "completed" else "succeeded")
            (f.root / "natural-race.json").write_text(json.dumps({"cancel": value, "invocation": row}, indent=2))
            roots.append(f.root)
        except BaseException:
            f.emergency_cleanup()
            raise
    return roots


def termination(engine, provider, checkout, work_root):
    """Public refusal -> verified cancellation -> termination, without stranded work."""
    f = Fixture(engine, provider, checkout, work_root, "termination", "direct")
    try:
        before = f.show()
        assert before["work_slot_invocations"][0]["ownership"]["live_owned_work"]
        history = f.call(["history", f.name])
        refusal = f.call(["terminate", f.name], "rejected")
        assert refusal["code"] == "live-owned-work", refusal
        assert f.call(["history", f.name]) == history
        after = f.show()
        for key in ("lifecycle", "current_state", "state_visit", "initial_input"):
            assert after[key] == before[key], key
        result = f.cancel()
        f.assert_clean(result)
        f.show()
        terminated = f.call(["terminate", f.name])
        assert terminated["run"]["lifecycle"] == "terminated", terminated
        history = f.call(["history", f.name])
        assert f.call(f.cancel_args(), "rejected")["code"] == "run-not-active"
        assert f.call(["terminate", f.name], "rejected")["code"] == "run-not-active"
        assert f.call(["history", f.name]) == history
        proof = json.loads((f.root / "proof.json").read_text())
        proof.update(termination_refusal=refusal, terminal=terminated,
                     inactive_run_unchanged=True)
        (f.root / "proof.json").write_text(json.dumps(proof, indent=2))
        return f.root
    except BaseException:
        f.emergency_cleanup()
        raise


def run(engine, provider, checkout, work_root, case="all"):
    Path(work_root).mkdir(parents=True, exist_ok=True)
    roots = []
    if case in ("all", "graph"):
        roots.append(ordinary(engine, provider, checkout, work_root))
    if case in ("all", "waiter"):
        roots.append(ordinary(engine, provider, checkout, work_root, mode="direct", lose_waiter=True))
    if case in ("all", "interrupted"):
        roots.append(interrupted(engine, provider, checkout, work_root))
    if case in ("all", "races"):
        roots.extend(natural_races(engine, provider, checkout, work_root))
    if case in ("all", "deadline"):
        roots.append(unverified_deadline(engine, provider, checkout, work_root))
    if case in ("all", "termination"):
        roots.append(termination(engine, provider, checkout, work_root))
    assert roots, case
    print(json.dumps({"scenario": "cancellation", "case": case, "status": "passed", "platform": platform.system(),
        "proof_roots": [str(p) for p in roots], "hosted_linux": "pending" if platform.system() != "Linux" else "local Linux observed; hosted execution not inferred"}), flush=True)
    return roots


def prove(journey):
    run(str(journey.engine), str(journey.provider), journey.data_root, journey.work_root)
    print("recovery cancellation scenario passed", flush=True)


if __name__ == "__main__":
    run(str(Path(sys.argv[1]).resolve()), str(Path(sys.argv[2]).resolve()), Path.cwd(),
        sys.argv[3] if len(sys.argv) > 3 else tempfile.mkdtemp(prefix="recovery-cancellation-"),
        sys.argv[4] if len(sys.argv) > 4 else "all")
