"""Owner-attested event exceptions through fresh public CLI processes only.

The software-change case retains actual review/missing-proof denials and real
Bookends RED/BYPASS output. A scripted provider supplies the same two-edge
normal/exceptional terminal comparison. No catalog edits or bootstrap access.
"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time

from recovery_cancellation import Fixture as ExecutionFixture, process_table, wait_for


class Fixture:
    def __init__(self, engine, provider, checkout, work_root, name):
        self.root = Path(tempfile.mkdtemp(prefix=f"recovery-override-{name}-", dir=work_root))
        print(f"override fixture {name}: {self.root}", flush=True)
        self.engine, self.provider, self.checkout = str(engine), str(provider), str(checkout)
        self.name, self.transcript = name, []
        self.db, self.artifacts = self.root / "loop.sqlite", self.root / "artifacts"
        self.artifacts.mkdir()
        self.config = self.root / "providers.toml"
        self.config.write_text('[providers.software-change]\ncommand = ' + json.dumps(self.provider) + '\n')

    def call(self, args, status="completed", human=False):
        argv = [self.engine, "--database", str(self.db), *([] if human else ["--json"]), *args]
        p = subprocess.run(argv, cwd=self.checkout, capture_output=True, text=True, timeout=45)
        value = p.stdout if human else json.loads(p.stdout)
        self.transcript.append({"argv": argv, "cwd": self.checkout, "exit": p.returncode,
                                "stdout": p.stdout, "stderr": p.stderr})
        (self.root / "transcript.json").write_text(json.dumps(self.transcript, indent=2))
        assert p.returncode == {"completed": 0, "rejected": 10, "error": 20, "invalid-invocation": 2}[status], value
        if human:
            return value
        assert value["status"] == status, value
        return value

    def result(self, args, status="completed"):
        value = self.call(args, status)
        return value.get("result", value)

    def show(self):
        return self.result(["show", "--view", "full", self.name])

    def history(self):
        return self.call(["history", self.name])

    def start(self, profile):
        self.result(["--config", str(self.config), "start", "--id", self.name,
                     "software-change", json.dumps(profile)])

    def attestation(self):
        return {"state_visit": self.show()["state_visit"], "owner": "fixture-owner",
                "reason": "Explicit test-only exception; retained failure is not a pass"}

    def override(self, event):
        request = self.attestation()
        before = self.history()["result"]
        value = self.result(["event", self.name, event, "--override", json.dumps(request)])
        action = value["history"]["action"]
        assert action["outcome"]["outcome"] == "overridden", action
        assert action["outcome"]["exception"]["attestation"] == request
        assert action["outcome"]["exception"]["provider_evaluation"] == "not-performed"
        assert self.history()["result"][:-1] == before
        return value

    def append(self, kind, record_id, value):
        self.show()
        self.result(["append", self.name, "--kind", kind, "--record-id", record_id, json.dumps(value)])

    def summary(self, count, terminal=True):
        expected = {"has_overrides": count > 0, "override_count": count,
                    "completion_mode": ("completed-with-overrides" if count else "completed") if terminal else None}
        show = self.show()
        listed = next(r for r in self.result(["list"]) if r["id"] == self.name)
        history = self.history()
        for surface in (show, listed, history):
            assert {key: surface[key] for key in expected} == expected, surface
        if terminal:
            assert show["lifecycle"] == "final" and not show["requestable_events"]
        for args in (["show", "--view", "full", self.name], ["show", self.name, "--compact"], ["history", self.name], ["list"]):
            human = self.call(args, human=True)
            assert "override_count" in human and "has_overrides" in human, human
            if terminal:
                assert expected["completion_mode"] in human, human
        return expected

    def refusal(self, event, request, status="rejected", code=None):
        before = self.history()
        value = self.call(["event", self.name, event, "--override", json.dumps(request)], status)
        if code:
            assert value["code"] == code, value
        assert self.history() == before, "refused override changed history"
        return value

    def proof(self, **details):
        (self.root / "proof.json").write_text(json.dumps({"status": "passed", "database": str(self.db),
            "run_id": self.name, **details}, indent=2))


def software(engine, provider, checker, checkout, work_root):
    f = Fixture(engine, provider, checkout, work_root, "software-exception")
    author = {"name": "subject", "kind": "script"}
    schema = {"type": "object", "required": ["revision", "author"], "properties": {
        "revision": {"type": "string"}, "author": {"type": "object", "required": ["name", "kind"],
        "properties": {"name": {"type": "string"}, "kind": {"type": "string", "enum": ["human", "agent", "script"]}}}}}
    profile = {"contract_version": 2, "criterion_policy": {"required_authors": 1, "goal_required_authors": 1}, "config_version": "recovery-override-2", "artifact_root": str(f.artifacts),
        "artifact_schemas": {name + ".json": schema for name in ("intent", "design", "plan")},
        "review_policies": {"intent-review": [{"id": "axis", "description": "retained failure", "required_authors": 1}]},
        "work_slot_bindings": {"implement": {"command": sys.executable, "args": ["-c", "raise SystemExit('MUST NOT RUN')"]}}}
    (f.artifacts / "intent.json").write_text(json.dumps({"revision": "1", "author": author}))
    f.start(profile)
    # No observation may be manufactured by the attestation itself.
    f.refusal("intent-ready", {"state_visit": 0, "owner": "owner", "reason": "reason"}, code="run-not-observed")
    f.show()
    f.result(["event", f.name, "intent-ready"])
    request = f.attestation()
    for malformed in ({}, {**request, "extra": True}, {**request, "state_visit": -1},
                      {**request, "state_visit": "1"}, {**request, "owner": {}}, None):
        f.refusal("approved", malformed, "invalid-invocation")
    for key in ("owner", "reason"):
        f.refusal("approved", {**request, key: " \t"}, code="invalid-override")
    f.refusal("approved", {**request, "state_visit": request["state_visit"] - 1}, code="stale-state-visit")
    f.refusal("not-an-edge", request, code="event-unavailable")
    review = {"gate": "intent-review", "policy_id": "axis", "result": "fail", "findings": "required outcome absent",
        "author": {"name": "reviewer", "kind": "script"}, "subject": "intent.json", "subject_revision": "1",
        "config_version": profile["config_version"]}
    raw_review = subprocess.run([sys.executable, "-c", "import sys;sys.stdout.write(sys.stdin.read())"],
        input=json.dumps(review), text=True, capture_output=True, check=True, timeout=20).stdout
    (f.root / "failed-review.stdout").write_text(raw_review)
    f.append("review-evidence", "failed-review", json.loads(raw_review))
    # Invoke the actual repository checker on a deliberately incomplete fixture.
    # Its RED and explicit test-only BYPASS are evidence, never GREEN.
    bookends = []
    for args, label, exit_code in (([], "RED", 1), (["--bypass", "fixture:retain non-green evidence"], "BYPASS", 0)):
        argv = [str(checker), "--repo", str(f.root), *args]
        p = subprocess.run(argv, capture_output=True, text=True, timeout=30)
        assert p.returncode == exit_code and p.stdout.splitlines()[0] == label, p
        (f.root / ("bookends-" + label + ".stdout")).write_text(p.stdout)
        value = {"argv": argv, "exit": p.returncode, "stdout": p.stdout, "status": label}
        bookends.append(value)
        f.append("repository-check", "bookends-" + label, value)
    f.append("finding-ledger", "ledger", {"schema_version": "1", "gate": "intent-review",
        "subject": "intent.json", "subject_revision": "1", "author": {"name": "driver", "kind": "agent"}, "findings": []})
    denied = f.result(["event", f.name, "approved"], "rejected")
    assert "failed-review" in json.dumps(denied), denied
    original = f.history()["result"]
    f.override("approved")
    f.refusal("design-ready", request, code="stale-state-visit")
    assert not (f.artifacts / "design.json").exists()
    f.show()
    missing = f.result(["event", f.name, "design-ready"], "rejected")
    assert "design.json" in json.dumps(missing), missing
    f.override("design-ready")
    # A later ordinary edge remains ordinary, with its own schema obligation.
    f.show()
    f.result(["event", f.name, "plan-ready"], "rejected")
    (f.artifacts / "plan.json").write_text(json.dumps({"revision": "1", "author": author}))
    f.result(["event", f.name, "plan-ready"])
    f.show()
    denied_bound = f.result(["event", f.name, "implementation-ready"], "rejected")
    assert denied_bound["code"] == "bound-slot-invocation-required", denied_bound
    skipped = f.override("implementation-ready")["history"]["action"]["outcome"]["exception"]
    assert [row["slot_id"] for row in skipped["skipped_bound_checks"]] == ["implement"]
    assert skipped["skipped_bound_checks"][0]["subject"]
    f.show()
    f.result(["event", f.name, "passed"], "rejected")
    f.override("passed")
    summary = f.summary(4)
    terminal = f.show()
    records = {r["id"]: r["data"] for r in terminal["context"]}
    assert records["failed-review"] == review
    assert [records["bookends-" + label] for label in ("RED", "BYPASS")] == bookends
    assert f.history()["result"][:len(original)] == original
    assert all(e["result"]["result"] != "allow" for e in terminal["latest_evaluations"]
               if e["transition"]["event"] in ("approved", "design-ready", "implementation-ready", "passed"))
    assert not terminal["work_slot_invocations"]
    for name in ("design.json", "implementation-report.json", "validation-report.json"):
        assert not (f.artifacts / name).exists(), name
    f.refusal("passed", f.attestation(), code="run-not-active")
    f.proof(summary=summary, skipped=skipped, failed_review_retained=True, bookends=bookends,
            missing_artifacts_retained=True, later_ordinary_edge_checked=True)
    return f.root


PROVIDER = '''#!/usr/bin/env python3
import json,pathlib,sys
r=json.load(sys.stdin)
if r['operation']=='describe':
 print(json.dumps({'id':'override-comparison','initial_state':'first','states':[
  {'id':s,'title':s,'instructions':'external proof '+s,'final':s=='end'} for s in ['first','second','end']],
  'transitions':[{'source':s,'event':'next','target':t,'kind':'checked'} for s,t in [('first','second'),('second','end')]]}))
else:
 root=pathlib.Path(r['initial_input']['artifact_root']); source=r['transition']['source']
 with (root/'evaluated.jsonl').open('a') as f: f.write(json.dumps(r)+'\\n')
 exists=(root/(source+'.proof')).exists()
 print(json.dumps({'result':'allow'} if exists else {'result':'deny','feedback':{'code':'missing-proof','message':source+' proof is missing'}}))
'''


def comparison(engine, provider, checkout, work_root):
    roots, summaries = [], []
    for exceptional in (False, True):
        f = Fixture(engine, provider, checkout, work_root, "exceptional" if exceptional else "ordinary")
        script = f.root / "provider.py"
        script.write_text(PROVIDER)
        script.chmod(0o755)
        f.config.write_text('[providers.software-change]\ncommand = ' + json.dumps(str(script)) + '\n')
        f.start({"artifact_root": str(f.artifacts)})
        for state in ("first", "second"):
            f.show()
            f.result(["event", f.name, "next"], "rejected")
            if exceptional:
                f.override("next")
            else:
                (f.artifacts / (state + ".proof")).write_text("actual fixture proof supplied\n")
                f.result(["event", f.name, "next"])
        evaluated = (f.artifacts / "evaluated.jsonl").read_text().splitlines()
        assert len(evaluated) == (2 if exceptional else 4), "override executed provider"
        summary = f.summary(2 if exceptional else 0)
        # Read-only discovery must not run even this deliberately unavailable provider.
        script.unlink()
        assert f.summary(2 if exceptional else 0) == summary
        f.proof(summary=summary, provider_evaluation_count=len(evaluated), provider_free_reads=True)
        summaries.append(summary)
        roots.append(f.root)
    assert summaries[0]["completion_mode"] != summaries[1]["completion_mode"]
    return roots


class WorkFixture(ExecutionFixture):
    """T06's generic invocation/cancel receipts, without a T07 route dependency."""
    def __init__(self, engine, provider, checkout, work_root):
        self.root = Path(tempfile.mkdtemp(prefix="recovery-override-live-", dir=work_root))
        print(f"override live fixture: {self.root}", flush=True)
        self.engine, self.provider, self.checkout = str(engine), str(provider), str(checkout)
        self.name, self.slot, self.mode, self.transcript = "override-live", "intent-draft", "direct", []
        self.db, self.artifacts = self.root / "loop.sqlite", self.root / "artifacts"
        self.artifacts.mkdir()
        worker = self.root / "worker.py"
        worker.write_text('import json,os,pathlib,sys,time\n'
            + 'packet=json.load(sys.stdin);root=pathlib.Path(sys.argv[1])\n'
            + "(root/'workers.jsonl').write_text(json.dumps({'pid':os.getpid()})+'\\n')\n"
            + "print('retained cancelled work',flush=True)\n(root/'ready').touch()\n"
            + "while not (root/'release').exists(): time.sleep(.01)\n")
        config = self.root / "providers.toml"
        config.write_text('[providers.software-change]\ncommand = ' + json.dumps(self.provider) + '\n')
        profile = {"contract_version": 2, "criterion_policy": {"required_authors": 1, "goal_required_authors": 1}, "config_version": "recovery-override-live-2", "artifact_root": str(self.artifacts),
            "review_policies": {}, "work_slot_bindings": {self.slot: {
                "command": sys.executable, "args": [str(worker), str(self.root)]}}}
        self.call(["--config", str(config), "start", "--id", self.name, "software-change", json.dumps(profile)])
        self.before = self.show()
        self.result = self.call(["invoke", self.name, self.slot, "--timeout-ms", "2000"])
        self.capture = Path(self.result["capture_dir"])
        self.ownership = self.capture / "ownership"
        try:
            wait_for(lambda: (self.root / "ready").exists(), "override worker ready")
        except BaseException:
            self.emergency_cleanup()
            raise


def cleanup_pending(f, refuse):
    # Pause this isolated controller after stop admission, before signaling.
    # Natural worker exit cannot waive the outstanding cleanup acknowledgment.
    gate = f.root / "path-gate"
    gate.mkdir()
    real_ps = shutil.which("ps")
    ps = gate / "ps"
    ps.write_text('#!' + sys.executable + '\nimport os,pathlib,time\n'
        + f"root=pathlib.Path({str(f.root)!r})\n(root/'controller-ready').write_text(str(os.getpid()))\n"
        + "while not (root/'release-inventory').exists(): time.sleep(.005)\n"
        + f"os.execv({real_ps!r},[{real_ps!r},'-axo','pid=,ppid=,pgid=,stat='])\n")
    ps.chmod(0o755)
    controller = subprocess.Popen(f.argv(f.cancel_args()),
        env=dict(os.environ, PATH=str(gate) + os.pathsep + os.environ["PATH"]),
        stdout=(f.root / "interrupted.stdout").open("w"), stderr=(f.root / "interrupted.stderr").open("w"))
    try:
        wait_for(lambda: (f.root / "controller-ready").exists(), "cancellation admitted")
        original = (f.ownership / "attempt-1.json").read_bytes()
        assert json.loads(original)["outcome"] == "incomplete"
    finally:
        controller.kill()
        controller.wait(timeout=5)
        (f.root / "release-inventory").touch()
    helper = int((f.root / "controller-ready").read_text())
    wait_for(lambda: helper not in process_table(), "controller helper reaped")
    (f.root / "release").touch()
    wait_for(lambda: not f.invocation()["ownership"]["live_owned_work"], "worker exited")
    assert f.invocation()["ownership"]["cleanup_pending"]
    refuse()
    result = f.cancel()
    assert result["attempt"]["cleanup"]["verified_no_survivors"]
    assert (f.ownership / "attempt-1.json").read_bytes() == original
    return result


def live(engine, provider, checkout, work_root):
    f = WorkFixture(engine, provider, checkout, work_root)
    try:

        def refuse():
            shown = f.show()
            before = f.call(["history", f.name])
            request = {"state_visit": shown["state_visit"], "owner": "owner", "reason": "cannot waive live work"}
            result = f.call(["event", f.name, "intent-ready", "--override", json.dumps(request)], "rejected")
            assert result["code"] == "live-owned-work", result
            assert f.call(["history", f.name]) == before
            assert f.show()["override_count"] == 0

        refuse()
        time.sleep(2.1)
        assert f.invocation()["status"] == "overrun"
        refuse()
        cancellation = cleanup_pending(f, refuse)
        assert not f.invocation()["ownership"]["cleanup_pending"]
        shown = f.show()
        request = {"state_visit": shown["state_visit"], "owner": "owner", "reason": "cancelled failed work waived"}
        result = f.call(["event", f.name, "intent-ready", "--override", json.dumps(request)])
        assert result["history"]["action"]["outcome"]["outcome"] == "overridden"
        assert f.show()["override_count"] == 1
        assert f.invocation()["status"] == "failed"
        assert not (f.artifacts / "implementation-report.json").exists()
        pids = {json.loads(line)["pid"] for line in (f.root / "workers.jsonl").read_text().splitlines()}
        assert not pids.intersection(process_table())
        (f.root / "proof.json").write_text(json.dumps({"status": "passed", "run_id": f.name,
            "database": str(f.db), "running_overrun_and_cleanup_pending_refused": True,
            "cancelled_invocation_retained": f.result["invocation_id"], "cancellation": cancellation,
            "no_survivors": sorted(pids)}, indent=2))
        return f.root
    except BaseException:
        f.emergency_cleanup()
        raise


def run(engine, provider, checker, checkout, work_root, case="all"):
    Path(work_root).mkdir(parents=True, exist_ok=True)
    roots = []
    if case in ("all", "software"):
        roots.append(software(engine, provider, checker, checkout, work_root))
    if case in ("all", "comparison"):
        roots.extend(comparison(engine, provider, checkout, work_root))
    if case in ("all", "live"):
        roots.append(live(engine, provider, checkout, work_root))
    assert roots, case
    print(json.dumps({"scenario": "override", "case": case, "status": "passed", "proof_roots": [str(p) for p in roots]}), flush=True)


def prove(journey):
    run(journey.engine, journey.provider, journey.engine.parent / "bookends-check", journey.data_root, journey.work_root)
    print("recovery override scenario passed: retained denials/fails/RED/BYPASS, live-work refusal, exceptional and ordinary terminal labels", flush=True)


if __name__ == "__main__":
    run(*[str(Path(p).resolve()) for p in sys.argv[1:4]], Path.cwd(), sys.argv[4], sys.argv[5] if len(sys.argv) > 5 else "all")
