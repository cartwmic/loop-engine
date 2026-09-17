"""Deterministic public-CLI proof of exact-source driver dispositions."""
import copy
import json
import subprocess
import sys
import tempfile
import time
from pathlib import Path


def prove(journey):
    # Every catalog and artifact is private to this deterministic fixture.
    journey.work_root.mkdir(parents=True, exist_ok=True)
    root = Path(tempfile.mkdtemp(prefix="recovery-dispositions-", dir=journey.work_root))
    print(f"dispositions captures: {root}", flush=True)
    captures = []
    transcripts = []
    counter = 0

    def write(path, value):
        path.write_text(json.dumps(value, indent=2) + "\n")

    def call(db, args, expected="completed"):
        nonlocal counter
        process = subprocess.run([str(journey.engine), "--database", str(db), "--json", *args],
                                 capture_output=True, cwd=journey.data_root, timeout=45)
        counter += 1
        prefix = root / f"{counter:03d}-{args[0]}"
        prefix.with_suffix(".stdout.json").write_bytes(process.stdout)
        prefix.with_suffix(".stderr").write_bytes(process.stderr)
        value = json.loads(process.stdout)
        transcripts.append({"argv": args, "exit": process.returncode, "envelope": value})
        write(root / "transcript.json", transcripts)
        assert value["status"] == expected, value
        assert process.returncode == (0 if expected == "completed" else 10), value
        return value

    author = lambda name: {"name": name, "kind": "script"}
    schema = {"type": "object", "properties": {
        "revision": {"type": "string", "minLength": 1},
        "author": {"type": "object", "properties": {
            "name": {"type": "string", "minLength": 1},
            "kind": {"type": "string", "enum": ["human", "agent", "script"]}},
            "required": ["name", "kind"], "additionalProperties": False}},
        "required": ["revision", "author"], "additionalProperties": False}

    def start(name, required=2, binding=None):
        directory = root / name
        directory.mkdir()
        artifacts = directory / "artifacts"
        artifacts.mkdir()
        db = directory / "loop.sqlite"
        config = directory / "providers.toml"
        config.write_text('[providers.software-change]\ncommand = ' + json.dumps(str(journey.provider)) + '\n')
        profile = {"contract_version": 3, "criterion_policy": {"required_authors": 1, "goal_required_authors": 1}, "config_version": "recovery-dispositions-3",
                   "artifact_root": str(artifacts),
                   "review_policies": {"intent-review": [{"id": "axis", "description": "disposition proof", "review_stage": "aggregate", "required_authors": required}]},
                   "artifact_schemas": {"intent.json": schema}}
        if binding is not None:
            profile["work_slot_bindings"] = {"intent-review": binding}
        write(directory / "profile.json", profile)
        write(artifacts / "intent.json", {"revision": "1", "author": author("subject")})
        call(db, ["--config", str(config), "start", "--id", name, "software-change", "@" + str(directory / "profile.json"), name])
        call(db, ["show", "--view", "full", name])
        call(db, ["event", name, "intent-ready"])
        call(db, ["show", "--view", "full", name])
        return db, name, artifacts

    def append(run, kind, record_id, data):
        db, name, _ = run
        call(db, ["show", "--view", "full", name])
        call(db, ["append", name, "--kind", kind, "--record-id", record_id, json.dumps(data)])

    def review(run, record_id, reviewer, result="fail", revision="1"):
        # Scripted external reviewer: its actual stdout is retained verbatim.
        verdict = {"gate": "intent-review", "policy_id": "axis", "review_stage": "aggregate", "result": result,
                   "findings": "same finding text" if result == "fail" else "",
                   "author": author(reviewer), "subject": "intent.json",
                   "subject_revision": revision, "config_version": "recovery-dispositions-3"}
        raw = subprocess.run([sys.executable, "-c", "import sys; sys.stdout.buffer.write(sys.stdin.buffer.read())"],
                             input=json.dumps(verdict).encode(), capture_output=True, check=True).stdout
        path = root / f"{run[1]}-{record_id}-raw-verdict.json"
        path.write_bytes(raw)
        captures.append((path, raw))
        append(run, "review-evidence", record_id, json.loads(raw))
        return verdict

    def finding(record_id, disposition="rejected", status="recorded", **fields):
        value = {"id": "F-" + record_id, "source": {"kind": "context-record", "id": record_id},
                 "policy_id": "axis", "statement": "same finding text", "disposition": disposition,
                 "reason": "driver checked the retained evidence and explicitly dispositions this source",
                 "owner_phase": "implementation" if disposition == "accepted" else None,
                 "task_ids": [], "review_axes": [], "status": status}
        value.update(fields)
        return value

    def ledger(run, findings, revision="1"):
        append(run, "finding-ledger", f"ledger-{counter}", {"schema_version": "1", "gate": "intent-review",
               "subject": "intent.json", "subject_revision": revision, "author": {"name": "driver", "kind": "agent"}, "findings": findings})

    def event(run, allowed=False, contains=None):
        db, name, _ = run
        call(db, ["show", "--view", "full", name])
        result = call(db, ["event", name, "approved"], "completed" if allowed else "rejected")
        if contains:
            assert contains in json.dumps(result), result
        if allowed:
            show = call(db, ["show", "--view", "full", name])
            assert show["result"]["current_state"] == "design", show
        return result

    run = start("exact-source")
    original = review(run, "a", "alice")
    ledger(run, [])
    event(run, contains="a")
    ledger(run, [finding("a", reason="   ")])
    event(run, contains="reason")
    ledger(run, [finding("a")])
    # A discharged failure is one judgment, not two independent authors.
    denial = event(run, contains="independence")
    assert denial["details"]["satisfied_by_disposition"] == ["a"], denial
    review(run, "b", "bob")
    event(run, contains="bob")
    ledger(run, [finding("a"), finding("b", "accepted", "unresolved")])
    event(run, contains="accepted_unresolved")
    ledger(run, [finding("a"), finding("b", "accepted", "resolved")])
    # Latest-per-author still selects the new source, even with identical text.
    review(run, "a2", "alice")
    event(run, contains="a2")
    ledger(run, [finding("a"), finding("a2"), finding("b", "accepted", "resolved")])
    event(run, allowed=True)
    show = call(run[0], ["show", "--view", "full", run[1]])
    records = show["result"]["context"]
    assert next(r["data"] for r in records if r["id"] == "a") == original
    call(run[0], ["history", run[1]])

    run = start("retirement")
    review(run, "retired", "alice")
    review(run, "remaining", "bob", "pass")
    retired = finding("retired", "retired-author")
    ledger(run, [retired])
    event(run, contains="reviewer-manifest")

    def roster(names):
        append(run, "reviewer-manifest", f"roster-{counter}", {"gate": "intent-review", "authors": [author(n) for n in names], "reason": "owner changed the review roster"})

    roster(["alice", "bob"])
    roster(["alice", "bob"])
    ledger(run, [retired])
    event(run, contains="current absence")
    roster(["bob", "bob"])
    ledger(run, [retired])
    event(run, contains="duplicate authors")
    roster(["bob", "carol"])
    ledger(run, [retired])
    event(run, contains="independence")
    review(run, "departed-pass", "alice", "pass")
    event(run, contains="independence")
    review(run, "replacement", "carol", "pass")
    roster(["alice", "bob", "carol"])
    event(run, contains="current absence")
    roster(["bob", "carol"])
    ledger(run, [retired])
    # Reconfirm replacement coverage after the final roster/ledger snapshot;
    # v3 stage-aware aggregation must see two current non-retired authors.
    review(run, "replacement-final", "carol", "pass")
    event(run, allowed=True)
    call(run[0], ["history", run[1]])

    run = start("historical-resolution")
    write(run[2] / "plan.json", {"tasks": [{"id": "old-task"}]})
    review(run, "historical", "alice")
    old = finding("historical", "accepted", "unresolved", task_ids=["old-task"], review_axes=["axis"])
    ledger(run, [old])
    event(run, contains="accepted_unresolved")
    write(run[2] / "intent.json", {"revision": "3", "author": author("subject")})
    write(run[2] / "plan.json", {"tasks": [{"id": "current-task"}]})
    review(run, "current-a", "alice", "pass", "3")
    review(run, "current-b", "bob", "pass", "3")
    ledger(run, [], "3")
    event(run, contains="omitted")
    ledger(run, [old], "3")
    event(run, contains="unknown plan task")
    unresolved = copy.deepcopy(old)
    unresolved["task_ids"] = ["current-task"]
    ledger(run, [unresolved], "3")
    event(run, contains="accepted_unresolved")
    resolved = copy.deepcopy(old)
    resolved["status"] = "resolved"
    resolved["review_axes"] = ["historical-axis"]
    ledger(run, [resolved], "3")
    event(run, allowed=True)
    show = call(run[0], ["show", "--view", "full", run[1]])
    records = show["result"]["context"]
    assert not any(r["kind"] == "evidence-applicability" for r in records)
    assert next(r for r in records if r["id"] == "historical")["data"]["subject_revision"] == "1"
    call(run[0], ["history", run[1]])
    # Bound raw capture uses the real selected-assignment origin, not copied
    # coordinates or a manufactured pass. Disposition must leave every byte intact.
    import work_slot_journey
    judgment = {"review_stage": "aggregate", "axis": "axis", "author": author("bound-reviewer"), "result": "fail", "findings": "same finding text"}
    binding = work_slot_journey.fan_out_binding(engine=journey.engine, workers=[{
        "command": sys.executable,
        "args": ["-c", "import sys; sys.stdin.read(); print(" + repr(json.dumps(judgment)) + ")"],
        "preamble": "Return the deterministic failing judgment.",
        "full_output_schema": {"type": "object", "properties": {
            "review_stage": {"const": "aggregate"},
            "axis": {"const": "axis"}, "author": {"const": author("bound-reviewer")},
            "result": {"const": "fail"}, "findings": {"const": "same finding text"}},
            "required": ["review_stage", "axis", "author", "result", "findings"], "additionalProperties": False}}])
    run = start("bound-disposition", required=1, binding=binding)
    invoked = call(run[0], ["invoke", run[1], "intent-review"])
    invocation_id = invoked["result"]["invocation_id"]
    capture = Path(invoked["result"]["capture_dir"])
    deadline = time.monotonic() + 30
    while True:
        shown = call(run[0], ["show", "--view", "full", run[1]])
        invocation = next(i for i in shown["result"]["work_slot_invocations"] if i["invocation_id"] == invocation_id)
        if invocation.get("completed_at") is not None:
            assert invocation["status"] == "succeeded", invocation
            break
        assert time.monotonic() < deadline, invocation
        time.sleep(0.05)
    raw_before = {str(p.relative_to(capture)): p.read_bytes() for p in capture.rglob("*") if p.is_file()}
    selected_raw = capture / "0" / "attempts" / "1" / "stdout"
    assert json.loads(selected_raw.read_bytes()) == judgment
    evidence = {"gate": "intent-review", "policy_id": "axis", "review_stage": "aggregate", "author": author("bound-reviewer"),
                "result": "fail", "findings": "same finding text", "subject": "intent.json",
                "subject_revision": "1", "config_version": "recovery-dispositions-3",
                "origin": {"kind": "selected-assignment-output", "id": invocation_id, "assignment_id": "worker-0"}}
    append(run, "review-evidence", "bound-fail", evidence)
    ledger(run, [])
    event(run, contains="bound-fail")
    ledger(run, [finding("bound-fail")])
    event(run, allowed=True)
    assert {str(p.relative_to(capture)): p.read_bytes() for p in capture.rglob("*") if p.is_file()} == raw_before
    call(run[0], ["history", run[1]])
    for path, raw in captures:
        assert path.read_bytes() == raw
    write(root / "proof.json", {"status": "passed", "raw_verdicts_unchanged": len(captures),
          "bound_capture_unchanged": str(capture),
          "cases": ["exact-source", "retirement", "historical-resolution", "bound-disposition"], "database_paths": [str(root / name / "loop.sqlite") for name in ("exact-source", "retirement", "historical-resolution", "bound-disposition")]})
    print("recovery dispositions scenario passed: exact sources, distinct authors, roster retirement, historical resolution, unchanged raw verdicts")
