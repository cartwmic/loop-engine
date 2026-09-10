"""Actual policy-document selector plus isolated bound invocation proof."""
from __future__ import annotations
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import time


def policy_selection_case(args):
    root = args.attempt
    engine = args.binary_dir / "loop-engine"
    provider = args.binary_dir / "policy-document"
    counter = 0

    def run(argv, packet=None, expected=0):
        nonlocal counter
        counter += 1
        prefix = root / f"command-{counter:03}"
        p = subprocess.run([str(x) for x in argv], input=None if packet is None else json.dumps(packet),
                           text=True, capture_output=True)
        prefix.with_suffix(".stdout").write_text(p.stdout)
        prefix.with_suffix(".stderr").write_text(p.stderr)
        prefix.with_suffix(".json").write_text(json.dumps(dict(argv=[str(x) for x in argv],
            cwd=str(Path.cwd()), exit=p.returncode), indent=2))
        assert p.returncode == expected, (argv, p.returncode, p.stdout, p.stderr)
        return json.loads(p.stdout) if p.stdout.strip() else None

    target = root / "target.md"
    target.write_text("# Current target\n")
    profile = dict(schema_version=1, profile_version="selection-1", mode="audit",
                   target=dict(id="doc", path=str(target)),
                   deterministic_policies=[dict(id="present", type="non-empty")],
                   semantic_policies=[dict(id="quality", description="quality", example_prompt="judge quality")])
    frozen = json.dumps(profile)
    workflow = run([provider], dict(operation="describe", initial_input=profile))
    catalog = workflow["work_slots"]

    def record(id, kind, data, sequence):
        return dict(id=id, kind=kind, data=data, sequence=sequence, created_at=sequence)

    old = dict(gate="semantic-review", policy_id="quality", result="pass", findings="old finding",
               author=dict(name="old", kind="script"), target_id="doc", target_sha256="a"*64,
               profile_version="selection-1")
    records = [record("old", "review-evidence", old, 1),
               record("unrelated", "review-evidence", {**old, "findings":"OMIT ME"}, 2)]

    def inspect(context, expected=0):
        return run([provider, "commission", frozen], dict(status="completed", result=dict(
            initial_input=profile, context=context, work_slots=catalog)), expected)

    assert inspect(records)["context"] == []
    selection = dict(target_id="doc", slot_id="semantic-review", record_ids=["old"])
    records.append(record("selection-1", "review-context-selection", selection, 3))
    selected = inspect(records)
    assert selected["context"] == [records[0], records[2]]
    assert selected["diagnostics"][-1] == dict(record_id="old", role="historical-context-not-proof", stale=True)
    assert selected["current_target"]["target_sha256"] == hashlib.sha256(target.read_bytes()).hexdigest()
    filtered = run([provider, "commission", frozen], dict(slot_id="semantic-review", work_slots=catalog, context=records))
    assert filtered["record_ids"] == selected["record_ids"]
    bad = records + [record("bad", "review-context-selection", {**selection, "record_ids":["missing"]}, 4)]
    inspect(bad, 1)
    inspect(records + [record("bad-super", "review-context-selection", {**selection, "supersedes":"missing"}, 4)], 1)
    saved = json.dumps(selected)
    records.append(record("selection-2", "review-context-selection", {**selection, "record_ids":[], "supersedes":"selection-1"}, 4))
    cleared = inspect(records)
    assert cleared["record_ids"] == ["selection-2"]
    assert cleared["diagnostics"][0]["supersedes"] == "selection-1"
    assert json.dumps(selected) == saved
    # Exact target proof remains independent of historical packet selection.
    request = dict(operation="evaluate", workflow=workflow, initial_input=profile, context=records,
                   transition=dict(source="semantic-review", event="passed", target="end", kind="checked"), prior_evaluations=[])
    assert run([provider], request)["result"] == "deny"
    current = {**old, "target_sha256":hashlib.sha256(target.read_bytes()).hexdigest()}
    request["context"] = records + [record("fresh", "review-evidence", current, 5)]
    assert run([provider], request)["result"] == "allow"
    target.write_text("# Changed target\n")
    assert run([provider], request)["result"] == "deny"

    # Public engine start/show/append/event/invoke path with a dummy external worker.
    db = root / "isolated.sqlite"
    providers = root / "providers.toml"
    providers.write_text('[providers.policy-document]\ncommand = '+json.dumps(str(provider))+'\nargs = []\n')
    received = root / "received.json"
    worker = root / "worker.py"
    worker.write_text("import sys,pathlib\npathlib.Path(sys.argv[1]).write_text(sys.stdin.read())\nprint('{}')\n")
    profile["work_slot_bindings"] = {"semantic-review":dict(command=sys.executable,
        args=[str(worker), str(received)], context_filter=dict(command=str(provider), args=["commission", frozen]))}
    profile_path = root / "profile.json"
    profile_path.write_text(json.dumps(profile))

    def call(*argv, expected=0):
        return run([engine, "--json", "--database", db, "--config", providers, *argv], expected=expected)

    started = call("start", "--id", "selection-proof", "policy-document", "@"+str(profile_path), "selection proof")
    assert started["status"] == "completed"
    run_id = "selection-proof"
    def show():
        return call("show", "--view", "full", run_id)
    def append(id, kind, data):
        show()
        assert call("append", "--record-id", id, run_id, kind, json.dumps(data))["status"] == "completed"
    show()
    call("event", run_id, "ready")
    show()
    call("event", run_id, "passed")
    append("old", "review-evidence", old)
    append("unrelated", "review-evidence", {**old, "findings":"OMIT ME"})

    def invoke(label):
        show()
        result = call("invoke", run_id, "semantic-review")
        assert result["status"] == "completed", result
        deadline = time.monotonic()+30
        while time.monotonic() < deadline:
            shown = show()
            matches = [v for v in shown["result"]["work_slot_invocations"] if v["invocation_id"] == result["result"]["invocation_id"]]
            if matches and matches[0]["status"] == "succeeded":
                raw = received.read_text()
                (root / (label+".packet")).write_text(raw)
                return json.loads(raw)
            time.sleep(.1)
        raise AssertionError(("bound invocation did not finish", shown))

    empty_packet = invoke("absent")
    assert empty_packet["context"] == []
    def prepare_selection(data):
        packet = show()
        packet["selection_data"] = data
        return run([provider, "commission", frozen], packet)

    prepared = prepare_selection(selection)
    assert prepared["receipt"]["current_target"]["target_sha256"] == hashlib.sha256(target.read_bytes()).hexdigest()
    assert prepared["receipt"]["diagnostics"][-1]["stale"] is True
    append("selection-1", "review-context-selection", prepared)
    selected_packet = invoke("selected")
    assert "historical context, never current proof" in selected_packet["instruction_body"]
    unbound = run([provider, "commission", frozen], show())
    assert selected_packet["context"] == unbound["context"]
    assert [r["id"] for r in selected_packet["context"]] == ["old", "selection-1"]
    assert selected_packet["context"][-1]["data"]["receipt"] == prepared["receipt"]
    target.write_text("# Receipt invalidated by actual target edit\n")
    show()
    before_launch = received.read_bytes()
    assert call("invoke", run_id, "semantic-review", expected=20)["status"] == "error"
    assert received.read_bytes() == before_launch
    refreshed = prepare_selection({**selection, "supersedes":"selection-1"})
    assert refreshed["receipt"] != prepared["receipt"]
    append("refreshed", "review-context-selection", refreshed)
    refreshed_packet = invoke("refreshed")
    assert refreshed_packet["context"][-1]["data"]["receipt"] == refreshed["receipt"]
    assert refreshed_packet["context"] == run([provider, "commission", frozen], show())["context"]
    append("selection-2", "review-context-selection", prepare_selection({**selection, "record_ids":[], "supersedes":"refreshed"}))
    assert [r["id"] for r in invoke("cleared")["context"]] == ["selection-2"]
    assert json.loads((root / "selected.packet").read_text()) == selected_packet
    append("bad", "review-context-selection", {**selection, "record_ids":["unknown"], "supersedes":"selection-2"})
    show()
    before_launch = received.read_bytes()
    refused = call("invoke", run_id, "semantic-review", expected=20)
    assert refused["status"] == "error"
    assert received.read_bytes() == before_launch
    (root / "scenarios.json").write_text(json.dumps(dict(status="passed", database=str(db), run_id=run_id,
        scenarios=["absent attachment", "bound/unbound exact originals", "unknown refusal before launch",
                   "stale historical labels", "supersession and cleared selection", "unrelated omitted",
                   "launched packets preserved", "unchanged current digest evidence requirement",
                   "public prepare append invoke receipt delivery", "stale receipt refused before launch",
                   "refreshed receipt launch and bound/unbound equality"]), indent=2))
