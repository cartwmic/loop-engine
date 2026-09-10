"""Public bound per-author batch proof. Scripted judgments prove mechanics only."""
import copy
import importlib
import json
import subprocess
import tempfile
import time
from pathlib import Path


def prove(journey):
    helpers = importlib.import_module("software-change-journey")
    from work_slot_journey import assert_projected_fan_out_capture
    journey.work_root.mkdir(parents=True, exist_ok=True)
    root = Path(tempfile.mkdtemp(prefix="recovery-batch-", dir=journey.work_root))
    print(f"batched-review captures: {root}", flush=True)
    data = journey.data_root / "crates/software-change-provider"
    skill = (data / "skills/using-software-change-provider/SKILL.md").read_text()
    constructor = helpers._extract_jq_after(skill, '--slurpfile roster "$ROSTER" ')
    roster = [{"author": "reviewer-a", "model": "scripted-a"},
              {"author": "reviewer-b", "model": "scripted-b"}]
    roster_path = root / "roster.json"
    roster_path.write_text(json.dumps(roster))
    transcript = []
    proof = {"status": "running", "constructor_comparisons": [], "runs": [], "cases": []}

    def write(path, value):
        path.write_text(json.dumps(value, indent=2) + "\n")

    def construct(profile, worker, reason=""):
        args = ["jq", "--arg", "slot", "intent-review", "--arg", "engine", str(journey.engine),
                "--arg", "provider", str(journey.provider), "--arg", "pi", str(worker), "--arg", "cursor", "/scripted/cursor",
                "--arg", "bridge", "/scripted/bridge", "--arg", "separate_axes_reason", reason,
                "--rawfile", "base_preamble", str(data / "data/review-worker-preamble.txt"),
                "--slurpfile", "output_schema", str(data / "data/review-worker-output-schema.json"),
                "--slurpfile", "roster", str(roster_path), constructor, str(profile)]
        return args

    # Execute the shipped constructor against every live gate of every exact
    # selected shipped profile, not a hand-built replacement binding algorithm.
    for label in ("minimal", "standard", "high-rigor"):
        source = json.loads((data / f"data/configs/{label}.json").read_text())
        assert not source.get("work_slot_bindings")
        for gate, policies in source["review_policies"].items():
            if not policies:
                continue
            path = root / f"constructor-{label}-{gate}.json"
            write(path, source)
            argv = construct(path, "/scripted/pi")
            argv[3] = gate
            result = subprocess.run(argv, capture_output=True, check=True)
            built = json.loads(result.stdout)
            write(path, built)
            assert built["review_policies"] == source["review_policies"]
            assert list(built["work_slot_bindings"]) == [gate]
            workers = helpers._fan_out_workers(built["work_slot_bindings"][gate], engine=str(journey.engine))
            expected = helpers._policy_author_batches(policies, roster)
            assert len(workers) == len(expected)
            for worker, (assigned, entry) in zip(workers, expected):
                helpers._assert_worker_assignment(worker, policy=assigned, roster_entry=entry,
                    base_preamble=(data / "data/review-worker-preamble.txt").read_text(),
                    schema=json.loads((data / "data/review-worker-output-schema.json").read_text()),
                    pi_command="/scripted/pi", fragments=(gate,), schema_field="full_output_schema",
                    criterion_contract=gate == "validation-review")
            proof["constructor_comparisons"].append({"profile": label, "gate": gate,
                "axes": [p["id"] for p in policies], "workers": len(workers),
                "prior_per_axis_workers": len(helpers._policy_author_pairs(policies, roster))})

    def call(run, args, expected="completed"):
        db = run["db"]
        proc = subprocess.run([str(journey.engine), "--database", str(db), "--json", *args],
                              capture_output=True, cwd=journey.data_root, timeout=60)
        value = json.loads(proc.stdout)
        prefix = root / f"{len(transcript):04d}-{args[0]}"
        prefix.with_suffix(".stdout.json").write_bytes(proc.stdout)
        prefix.with_suffix(".stderr").write_bytes(proc.stderr)
        transcript.append({"database": str(db),
                           "argv": [str(journey.engine), "--database", str(db), "--json", *args],
                           "cwd": str(journey.data_root), "exit": proc.returncode,
                           "stdout": str(prefix.with_suffix(".stdout.json")),
                           "stderr": str(prefix.with_suffix(".stderr"))})
        write(root / "transcript.json", transcript)
        assert value["status"] == expected, value
        assert proc.returncode == {"completed": 0, "rejected": 10, "error": 20}[expected], value
        return value

    def show(run):
        return call(run, ["show", "--view", "full", run["id"]])

    def append(run, kind, record, value):
        show(run)
        return call(run, ["append", run["id"], "--kind", kind, "--record-id", record, json.dumps(value)])

    def author(name="reviewer-a"):
        return {"name": name, "kind": "agent"}

    def ledger(run, findings=None, revision="1"):
        append(run, "finding-ledger", "ledger-" + str(len(transcript)), {
            "schema_version": "1", "gate": "intent-review", "subject": "intent.json",
            "subject_revision": revision, "author": author("driver"), "findings": findings or []})

    def event(run, allowed=False, needle=None):
        show(run)
        value = call(run, ["event", run["id"], "approved"], "completed" if allowed else "rejected")
        if needle:
            assert needle in json.dumps(value), value
        if allowed:
            assert show(run)["result"]["current_state"] == "design"
        return value

    worker_code = '''#!/usr/bin/env python3
import json, sys
from pathlib import Path
packet = sys.stdin.read()
settings = json.loads(Path(__file__).with_name("mode.json").read_text())
policies = json.loads(next((s.removeprefix("assigned_policies: ") for s in packet.splitlines() if s.startswith("assigned_policies: ")), '[{"id":"alpha"},{"id":"beta"}]'))
name = next((s.removeprefix("required_author_claim: ") for s in packet.splitlines() if s.startswith("required_author_claim: ")), "reviewer-a")
rows = [{"axis": p["id"], "result": "pass", "findings": ""} for p in policies]
mode = settings["mode"]
if mode in ("mixed", "retry-mixed"):
    for row in rows:
        if row["axis"] == "beta": row.update(result="fail", findings="beta needs focused repair")
if mode == "missing" or (mode == "retry-mixed" and "SCHEMA-CONFORMANCE RETRY" not in packet): rows.pop()
if mode == "duplicate": rows[-1] = rows[0].copy()
if mode == "unknown": rows[-1]["axis"] = "unknown-axis"
if mode == "wrong-author": name = "unassigned-author"
if mode == "reuse": rows[-1] = {"axis": rows[-1]["axis"], "reuse": settings["reuse"]}
# Deliberately reverse output order: candidates must use assignment order.
output = {"author": {"name": name, "kind": "agent"}, "judgments": list(reversed(rows))}
with Path(__file__).with_name("launches.jsonl").open("a") as f:
    f.write(json.dumps({"argv": sys.argv, "mode": mode, "stdin": packet, "retry": "SCHEMA-CONFORMANCE RETRY" in packet, "output": output}) + "\\n")
print(json.dumps(output))
'''

    def start(name, mode="pass", required=1, separate=False, plain=False):
        directory = root / name
        directory.mkdir()
        artifacts = directory / "artifacts"
        artifacts.mkdir()
        worker = directory / "worker.py"
        worker.write_text(worker_code)
        worker.chmod(0o755)
        write(directory / "mode.json", {"mode": mode})
        config = directory / "providers.toml"
        config.write_text('[providers.software-change]\ncommand = ' + json.dumps(str(journey.provider)) + '\n')
        schema = {"type": "object", "additionalProperties": False, "required": ["revision", "author"],
            "properties": {"revision": {"type": "string", "minLength": 1}, "author": {
                "type": "object", "additionalProperties": False, "required": ["name", "kind"],
                "properties": {"name": {"type": "string", "minLength": 1}, "kind": {"type": "string", "enum": ["human", "agent", "script"]}}}}}
        profile = {"contract_version": 2, "criterion_policy": {"required_authors": 1, "goal_required_authors": 1}, "config_version": "recovery-batch-2", "artifact_root": str(artifacts),
            "review_policies": {"intent-review": [{"id": axis, "description": axis,
                "example_prompt": "Judge " + axis + " independently using retained proof.",
                "required_authors": required} for axis in ("alpha", "beta")]},
            "artifact_schemas": {"intent.json": schema}}
        profile_path = directory / "profile.json"
        write(profile_path, profile)
        args = construct(profile_path, worker, "observed specialization requires one-axis review" if separate else "")
        built = subprocess.run(args, capture_output=True, check=True)
        profile_path.write_bytes(built.stdout)
        profile = json.loads(built.stdout)
        if plain:
            args = profile["work_slot_bindings"]["intent-review"]["args"]
            for index, arg in enumerate(args):
                if arg == "--worker":
                    worker_cli = json.loads(args[index + 1])
                    worker_cli.pop("preamble", None)
                    worker_cli["args"] = []
                    args[index + 1] = json.dumps(worker_cli)
            write(profile_path, profile)
        write(artifacts / "intent.json", {"revision": "1", "author": author("subject")})
        run = {"id": name, "db": directory / "loop.sqlite", "artifacts": artifacts,
               "directory": directory, "profile": profile, "binding": profile["work_slot_bindings"]["intent-review"]}
        call(run, ["--config", str(config), "start", "--id", name, "software-change", "@" + str(profile_path), name])
        show(run)
        call(run, ["event", name, "intent-ready"])
        ledger(run)
        proof["runs"].append({"id": name, "database": str(run["db"])})
        return run

    def invoke(run, expected="succeeded", controls=None, assignments=None):
        show(run)
        args = ["invoke", run["id"], "intent-review"]
        if controls:
            args += ["--controls", json.dumps(controls)]
        if assignments:
            args += ["--assignment", assignments]
        started = call(run, args)
        identity = started["result"]["invocation_id"]
        deadline = time.monotonic() + 60
        while True:
            shown = show(run)
            invocation = next(i for i in shown["result"]["work_slot_invocations"] if i["invocation_id"] == identity)
            if invocation.get("completed_at") is not None:
                assert invocation["status"] == expected, invocation
                measurement = assert_projected_fan_out_capture(invocation)
                launches = [json.loads(line) for line in (run["directory"] / "launches.jsonl").read_text().splitlines()]
                spec = json.loads((Path(invocation["capture_dir"]) / "fan-out-spec.json").read_text())
                for worker in spec["workers"]:
                    delivered = Path(worker["stdin_path"]).read_text()
                    assert any(launch["stdin"] == delivered for launch in launches)
                    location = json.loads(delivered.removesuffix("---\n\n").rstrip().splitlines()[-1])
                    assert location["artifact_root"] == str(run["artifacts"])
                proof.setdefault("projection_measurements", []).append(dict(
                    measurement, invocation_id=identity))
                write(root / "proof.json", proof)
                return shown, invocation
            assert time.monotonic() < deadline, invocation
            time.sleep(0.1)

    def project(shown, invocation):
        results = []
        for _ in range(2):
            proc = subprocess.run([str(journey.provider), "review-candidates"], input=json.dumps(shown).encode(),
                                  capture_output=True, check=True)
            results.append(proc.stdout)
        assert results[0] == results[1], "normalization must be deterministic and read-only"
        calls = proof.setdefault("candidate_calls", [])
        path = root / f"{invocation['invocation_id']}-candidates-{len(calls):04d}.json"
        path.write_bytes(results[0])
        input_path = path.with_suffix(".input.json")
        write(input_path, shown)
        stderr_path = path.with_suffix(".stderr")
        stderr_path.write_bytes(proc.stderr)
        calls.append({"argv": [str(journey.provider), "review-candidates"],
            "cwd": str(Path.cwd()), "exit": proc.returncode, "stdin": str(input_path),
            "stdout": str(path), "stderr": str(stderr_path), "identical_repetitions": 2})
        write(root / "proof.json", proof)
        return [r for r in json.loads(results[0])["candidates"] if r["origin"]["id"] == invocation["invocation_id"]]

    def evidence(run, row, revision="1", record=None):
        assert row["status"] == "ready"
        data = {"gate": "intent-review", "policy_id": row["axis"], "author": row["author"],
                "result": row["result"], "findings": row["findings"], "origin": row["origin"],
                "subject": "intent.json", "subject_revision": revision, "config_version": "recovery-batch-2"}
        record = record or f"evidence-{len(transcript)}-{row['axis']}"
        append(run, "review-evidence", record, data)
        return record

    def finish_fresh(run, revision="1"):
        write(run["directory"] / "mode.json", {"mode": "pass"})
        shown, invocation = invoke(run)
        for row in project(shown, invocation):
            evidence(run, row, revision)
        ledger(run, revision=revision)
        event(run, allowed=True)
        assert show(run)["result"]["initial_input"]["work_slot_bindings"]["intent-review"] == run["binding"]
        call(run, ["history", run["id"]])

    run = start("mixed-retry", "retry-mixed")
    shown, invocation = invoke(run)
    assert len(invocation["inner_workers"]) == 1
    worker = invocation["inner_workers"][0]
    assert worker["selected_attempt"] == 2
    capture = Path(invocation["capture_dir"])
    manifest = json.loads((capture / "0/attempts.json").read_text())
    assert len(manifest["attempts"]) == 2 and manifest["attempts"][0]["validation_errors"]
    launches = [json.loads(line) for line in (run["directory"] / "launches.jsonl").read_text().splitlines()]
    assert len(launches) == 2 and launches[0]["argv"] == launches[1]["argv"]
    assert not launches[0]["retry"] and launches[1]["retry"]
    raw = {p: p.read_bytes() for p in capture.rglob("*") if p.is_file()}
    rows = project(shown, invocation)
    assert [r["axis"] for r in rows] == ["alpha", "beta"]
    assert [r["result"] for r in rows] == ["pass", "fail"]
    assert rows[0]["origin"] == rows[1]["origin"]
    assert show(run)["result"]["context"] == shown["result"]["context"]
    event(run, needle="missing")
    forged = copy.deepcopy(rows[1])
    forged.update(result="pass", findings="")
    evidence(run, forged, record="forged")
    event(run, needle="disagrees")
    evidence(run, rows[0], record="alpha-1")
    evidence(run, rows[1], record="beta-1")
    event(run, needle="beta-1")
    finding = {"id": "F-beta", "source": {"kind": "context-record", "id": "beta-1"},
        "policy_id": "beta", "statement": rows[1]["findings"], "disposition": "accepted",
        "reason": "driver confirms focused repair is needed", "owner_phase": "intent",
        "task_ids": [], "review_axes": ["beta"], "status": "unresolved"}
    ledger(run, [finding])
    event(run, needle="accepted_unresolved")
    finding["status"] = "resolved"
    finding["reason"] = "driver inspected the focused correction"
    ledger(run, [finding])
    event(run, allowed=True)
    assert all(p.read_bytes() == b for p, b in raw.items())
    proof["cases"].append("mixed batch, selected same-worker retry, inert candidates, raw-row mismatch, exact-source disposition")

    for mode in ("missing", "duplicate", "unknown", "wrong-author"):
        run = start("exhausted-" + mode, mode)
        shown, invocation = invoke(run, expected="failed")
        rows = project(shown, invocation)
        assert [r["status"] for r in rows] == ["exhausted"], rows
        manifest = json.loads((Path(invocation["capture_dir"]) / "0/attempts.json").read_text())
        assert manifest["selected_attempt"] is None and manifest["exhausted"]
        assert len(manifest["attempts"]) == 2
        assert all(a["validation_errors"] for a in manifest["attempts"])
        event(run)
        finish_fresh(run)
        proof["cases"].append("reject and recover exhausted " + mode)

    # Confirmation keeps the original binding and exact full axis set. A late
    # applicability append cannot authorize an already-started batch retroactively.
    run = start("focused-confirmation", plain=True)
    append(run, "fixture-context", "meaningful-nested", {
        "judgments": [{"result": "fail", "findings": "retain this meaningful statement"}],
        "nested": {"loop_engine_origin": {"note": "ordinary nested data, not provenance"}},
        "ordered": [3, 1, 2]})
    shown, first = invoke(run)
    for row in project(shown, first):
        evidence(run, row, record=row["axis"] + "-old")
    write(run["artifacts"] / "intent.json", {"revision": "2", "author": author("subject")})
    ledger(run, revision="2")
    write(run["directory"] / "mode.json", {"mode": "reuse", "reuse": "carry-beta"})
    shown, unauthorized = invoke(run)
    assert project(shown, unauthorized)[0]["status"] == "malformed"
    carry = {"origin": {"kind": "context-record", "id": "beta-old"},
             "target": {"subject": "intent.json", "revision": "2"},
             "attesting_driver": author("driver"), "reason": "Only alpha changed; beta evidence is unaffected"}
    append(run, "evidence-applicability", "carry-beta", carry)
    assert project(show(run), unauthorized)[0]["status"] == "malformed"
    event(run)
    shown, forced = invoke(run, expected="failed", controls={"force_fresh": True})
    forced_rows = project(shown, forced)
    assert forced_rows[0]["status"] == "exhausted", forced_rows
    forced_worker = forced["inner_workers"][0]
    assert forced_worker["selected_attempt"] is None
    forced_schema = forced_worker["declared_output_contract"]
    assert forced_schema["allOf"][-1] == forced_schema["x-loop-engine-force-fresh"]
    manifest = json.loads((Path(forced["capture_dir"]) / "0/attempts.json").read_text())
    assert len(manifest["attempts"]) == 2 and all(a["validation_errors"] for a in manifest["attempts"])
    # The invalid full batch has no selected source, including its fresh sibling.
    show(run)
    call(run, ["append", run["id"], "--kind", "review-evidence", "--record-id", "forced-invalid",
        json.dumps({"origin": {"kind": "selected-assignment-output", "id": forced["invocation_id"], "assignment_id": "worker-0"}})], "rejected")
    event(run)
    shown, current = invoke(run)
    assert proof["projection_measurements"][-1]["engine_origins"] >= 2
    full_before = show(run)
    history_before = call(run, ["history", run["id"]])
    capture_spec_path = Path(current["capture_dir"]) / "fan-out-spec.json"
    original_spec = capture_spec_path.read_bytes()
    rows = project(shown, current)
    evidence(run, rows[0], "2", "alpha-before-corruption")
    for snapshot in (None, {}, [{"data": {}}]):
        corrupted = json.loads(original_spec)
        if snapshot is None:
            del corrupted["workers"][0]["routed_inputs"]
        else:
            corrupted["workers"][0]["routed_inputs"] = snapshot
        write(capture_spec_path, corrupted)
        try:
            assert project(show(run), current)[0]["status"] == "malformed"
            event(run)  # Real provider evaluation must refuse the captured evidence too.
        finally:
            capture_spec_path.write_bytes(original_spec)
    current_context = show(run)["result"]["context"]
    assert current_context[:len(full_before["result"]["context"])] == full_before["result"]["context"]
    current_history = call(run, ["history", run["id"]])["result"]
    assert current_history[:len(history_before["result"])] == history_before["result"]
    selected_output = Path(current["inner_workers"][0]["selected_output_path"])
    original_output = selected_output.read_bytes()
    selected_output.write_bytes(original_output + b" ")
    try:
        assert project(show(run), current)[0]["status"] != "ready"
        event(run)
    finally:
        selected_output.write_bytes(original_output)
    show(run)
    call(run, ["append", run["id"], "--kind", "review-evidence", "--record-id", "wrong-assignment",
        json.dumps({"origin": {"kind": "selected-assignment-output", "id": current["invocation_id"],
                              "assignment_id": "not-the-selected-worker"}})], "rejected")
    rows = project(show(run), current)
    assert [r["status"] for r in rows] == ["ready", "carried"]
    assert rows[1]["applicability_id"] == "carry-beta"
    assert rows[0]["origin"] == rows[1]["origin"]
    # A carried row is not a new worker judgment and cannot be laundered as one.
    fake = dict(rows[1], status="ready", result="pass", findings="")
    evidence(run, fake, "2", "fake-carried-fresh")
    event(run, needle="carried row")
    # Re-declare the same original reference after the invalid candidate so latest
    # per-author selection replaces the malformed attempted fresh row honestly.
    append(run, "evidence-applicability", "carry-beta-confirmed", carry)
    evidence(run, rows[0], "2", "alpha-current")
    event(run, allowed=True)
    assert show(run)["result"]["initial_input"]["work_slot_bindings"]["intent-review"] == run["binding"]
    proof["cases"].append("fresh affected alpha, exact beta applicability, captured authorization, force-fresh refusal, no binding amendment")

    for fault in ("stale", "wrong-axis", "wrong-author"):
        run = start("reuse-" + fault)
        shown, initial = invoke(run)
        for row in project(shown, initial):
            evidence(run, row, record=row["axis"] + "-old")
        source = "beta-old"
        if fault == "wrong-axis":
            source = "alpha-old"
        if fault == "wrong-author":
            append(run, "review-evidence", "other-author", {
                "gate": "intent-review", "policy_id": "beta", "author": author("other"),
                "result": "pass", "findings": "", "subject": "intent.json",
                "subject_revision": "1", "config_version": "recovery-batch-2"})
            source = "other-author"
        write(run["artifacts"] / "intent.json", {"revision": "2", "author": author("subject")})
        ledger(run, revision="2")
        append(run, "evidence-applicability", "bad-carry", {"origin": {"kind": "context-record", "id": source},
            "target": {"subject": "intent.json", "revision": "1" if fault == "stale" else "2"},
            "attesting_driver": author("driver"), "reason": "scripted negative carry"})
        write(run["directory"] / "mode.json", {"mode": "reuse", "reuse": "bad-carry"})
        shown, invalid = invoke(run)
        rows = project(shown, invalid)
        assert rows[0]["status"] == "malformed", rows
        fake = {"status": "ready", "axis": "alpha", "author": author(), "result": "pass", "findings": "",
            "origin": {"kind": "selected-assignment-output", "id": invalid["invocation_id"], "assignment_id": "worker-0"}}
        evidence(run, fake, "2")
        event(run, needle="reuse")
        finish_fresh(run, "2")
        proof["cases"].append("reject and recover " + fault + " reuse")

    run = start("distinct-authors", required=2)
    shown, initial = invoke(run, assignments="worker-0")
    for row in project(shown, initial):
        evidence(run, row)
    event(run)
    shown, replacement = invoke(run, assignments="worker-1")
    for row in project(shown, replacement):
        evidence(run, row)
    event(run, allowed=True)
    proof["cases"].append("first-N distinct author allocation remains independently required")

    run = start("justified-singletons", separate=True)
    workers = helpers._fan_out_workers(run["binding"], engine=str(journey.engine))
    assert len(workers) == 2 and all(w["full_output_schema"]["properties"]["judgments"]["maxItems"] == 1 for w in workers)
    finish_fresh(run)
    proof["cases"].append("justified one-axis constructor uses identical batch contract")
    proof["status"] = "passed"
    proof["launches"] = [json.loads(line) for p in root.rglob("launches.jsonl") for line in p.read_text().splitlines()]
    write(root / "proof.json", proof)
    print("recovery batched-review scenario passed: per-author construction, exact captured rows, inert candidates, disposition, focused carry and failures", flush=True)
