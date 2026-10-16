#!/usr/bin/env python3
"""Root-owned v6 fictional proof: separate build + real engine, never paid calls."""
import argparse
import copy
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

ENGINE_SHA = "e1599f645f93d0b3a8d64403253f74e0d3e956145b80aa791f57cc3f752723bc"
SNAPSHOT_SHA = "2361843677d68b90775db42939f6333f16760138bfd36c96a71d5f01a65498e2"


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def inventory(root):
    return {str(p.relative_to(root)): sha(p) for p in sorted(root.rglob("*")) if p.is_file()}


def write(path, value):
    path.write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n")


def sample(schema):
    kind = schema["type"]
    if kind == "object":
        return {key: sample(schema["properties"][key]) for key in schema.get("required", [])}
    if kind == "array":
        return [sample(schema["items"]) for _ in range(max(1, schema.get("minItems", 0)))]
    return schema.get("enum", ["Scripted fictional proof value"])[0]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fixture", type=Path, required=True)
    parser.add_argument("--engine", type=Path, required=True)
    parser.add_argument("--output-root", type=Path, required=True)
    parser.add_argument("--reuse-proof", type=Path, required=True)
    parser.add_argument("--provider", type=Path, required=True)
    args = parser.parse_args()
    fixture = args.fixture.resolve(strict=True)
    engine = args.engine.resolve(strict=True)
    output = args.output_root.resolve()
    assert args.output_root.is_absolute() and not output.exists()
    assert not output.is_relative_to(fixture)
    assert sha(engine) == ENGINE_SHA
    output.mkdir(parents=True)
    env = {k: v for k, v in os.environ.items() if not k.startswith(("LOOP_", "XDG_", "BOOKENDS_"))}
    env.update(CARGO_TARGET_DIR=str(output / "build"), PYTHONDONTWRITEBYTECODE="1",
               XDG_DATA_HOME=str(output / "data"), XDG_STATE_HOME=str(output / "state"))
    before = inventory(fixture)
    sealed = json.loads((fixture.parent / "version-manifest.json").read_text())
    assert sealed["runtime_proof_ready"] and sealed["candidate_sha256"] == before
    receipt = {"status": "running", "fixture_before": before, "engine_sha256": ENGINE_SHA,
               "mechanical_only": True, "semantic_review": False, "calls": [], "journeys": []}
    write(output / "receipt.json", receipt)

    def call(argv, packet=None, cwd=fixture, raw_input=None):
        result = subprocess.run(list(map(str, argv)), cwd=cwd, env=env,
                                input=raw_input if raw_input is not None else (None if packet is None else json.dumps(packet)),
                                text=True, capture_output=True)
        row = {"argv": list(map(str, argv)), "cwd": str(cwd), "stdin": packet,
               "stdin_raw": raw_input,
               "exit_code": result.returncode, "stdout": result.stdout, "stderr": result.stderr}
        receipt["calls"].append(row)
        write(output / ("command-%03d.json" % len(receipt["calls"])), row)
        write(output / "receipt.json", receipt)
        return result

    try:
        # Reuse exact successful v6 components; the original overall attempt stays FAILED.
        reuse_path = args.reuse_proof.resolve(strict=True)
        reuse = json.loads(reuse_path.read_text())
        binding = json.loads((fixture.parent / "reuse-binding.json").read_text())
        assert sha(reuse_path) == binding["receipt_sha256"]
        assert reuse["status"] == "failed" and reuse["engine_sha256"] == ENGINE_SHA
        provider = args.provider.resolve(strict=True)
        assert sha(provider) == reuse["provider_sha256"] == binding["provider_sha256"]
        original_fixture = Path(binding["v6_fixture"]).resolve(strict=True)
        assert inventory(original_fixture) == reuse["fixture_before"] == binding["v6_candidate_sha256"]
        changed = {name for name in set(before) | set(reuse["fixture_before"])
                   if before.get(name) != reuse["fixture_before"].get(name)}
        assert changed == {"scripts/production-journey.py"}, changed
        expected_cargo = [
            ["cargo", "fmt", "--all", "--", "--check"],
            ["cargo", "test", "--workspace", "--all-targets", "--locked"],
            ["cargo", "test", "-p", "software-change-provider", "--lib", "--locked"],
            ["cargo", "clippy", "--workspace", "--all-targets", "--locked", "--", "-D", "warnings"],
            ["cargo", "build", "--workspace", "--locked"],
        ]
        for index, command in enumerate(expected_cargo):
            row = reuse["calls"][index]
            assert row["argv"] == command and row["exit_code"] == 0
        # Also retain the already successful protocol, documentation mechanics and
        # exact data-dump controls; provider and all their inputs are byte-identical.
        assert [row["exit_code"] for row in reuse["calls"][5:14]] == [0, 0, 0, 0, 2, 2, 2, 0, 0]
        snapshot = fixture / "provider/tests/snapshots/describe.json"
        assert sha(snapshot) == SNAPSHOT_SHA
        source_data = fixture / "provider/data"
        original_dump = reuse_path.parent / "references"
        assert inventory(original_dump) == inventory(source_data)
        receipt["provider_sha256"] = sha(provider)
        receipt["reused_components"] = {
            "receipt": str(reuse_path), "receipt_sha256": sha(reuse_path),
            "original_overall_status": "failed", "successful_cargo_call_indexes": [1, 2, 3, 4, 5],
            "protocol_doc_dump_call_indexes": list(range(6, 15)),
            "applicability": "Every Rust/Cargo/test/data/doc file unchanged; only external journey assertion and explicit reuse orchestration changed.",
            "original_provider": str(provider), "provider_sha256": sha(provider),
            "old_catalog": str(reuse_path.parent / "run.sqlite"), "old_catalog_not_opened": True}
        receipt["versioned_reference_exposure"] = reuse["versioned_reference_exposure"]
        receipt["versioned_reference_exposure_applicability"] = "Exact v6 data exposure reused unchanged, not relabeled as new execution."
        old_catalog_hashes = {str(path): sha(path) for path in reuse_path.parent.glob("run.sqlite*") if path.is_file()}
        preserved = json.loads((fixture.parent / "preserved-failure-bindings.json").read_text())
        failed_catalog_hashes = {
            str(path): sha(path) for directory in preserved["failed_proof_directories"]
            for path in Path(directory).glob("run.sqlite*") if path.is_file()}
        receipt["preserved_failed_catalog_hashes_before"] = failed_catalog_hashes
        receipt["old_catalog_hashes_before"] = old_catalog_hashes
        write(output / "receipt.json", receipt)
        # Frozen config unchanged; provider TOML points directly to NEW binary.
        config = output / "providers.toml"
        config.write_text("[providers.software-change]\ncommand = " + json.dumps(str(provider)) + "\nargs = []\n")
        base = [engine, "--json", "--database", output / "run.sqlite", "--config", config]

        def cli(*argv, status="completed"):
            result = call(base + list(argv))
            value = json.loads(result.stdout)
            assert value["status"] == status
            assert result.returncode == {"completed": 0, "rejected": 10, "error": 20}[status]
            return value

        for profile_name in ["minimal", "standard", "high-rigor"]:
            profile_path = source_data / ("configs/" + profile_name + ".json")
            submitted = json.loads(profile_path.read_text())
            rid = "fictional-v6-" + profile_name
            cli("start", "--id", rid, "software-change", "@" + str(profile_path),
                "NEW fictional deterministic proof; not historic observation")
            full = cli("show", rid, "--view", "full")["result"]
            frozen = full["initial_input"]
            assert {k: v for k, v in frozen.items() if k != "artifact_root"} == submitted
            assert set(frozen) == set(submitted) | {"artifact_root"}
            assert "templates" not in frozen and "reviewer_protocol" not in frozen
            root = Path(frozen["artifact_root"]).resolve()
            assert root.is_relative_to(output)
            root.mkdir(parents=True, exist_ok=True)
            documents = {}
            for name, schema in submitted["artifact_schemas"].items():
                doc = sample(schema)
                doc.update(revision="NEW-r1", author={"name": "fixture-subject-author", "kind": "human"})
                documents[name] = doc
            for link in submitted.get("revision_links", []):
                documents[link["from"]][link["field"]] = documents[link["to"]]["revision"]
            # Real missing/unparseable/exhaustive structural negative, not context records
            # pretending to materialize authored files.
            cli("event", rid, "intent-ready", status="rejected")
            (root / "intent.json").write_text("not JSON")
            cli("show", rid, "--view", "action")
            cli("event", rid, "intent-ready", status="rejected")
            malformed = copy.deepcopy(documents["intent.json"])
            del malformed["revision"]
            malformed["outcome"] = ""
            malformed["NEW-unexpected"] = "defect"
            write(root / "intent.json", malformed)
            cli("show", rid, "--view", "action")
            denied = cli("event", rid, "intent-ready", status="rejected")
            diagnostics = denied["details"]
            actual = {(d["path"], d["rule"]) for d in diagnostics}
            assert any(d["path"] == "" and d["rule"] == "required"
                       and d["subject"] == "intent.json"
                       and d["message"] == "required property `revision` is missing"
                       for d in diagnostics), denied
            assert ("/outcome", "minLength") in actual, denied
            assert ("/NEW-unexpected", "additionalProperties") in actual, denied
            for name, doc in documents.items():
                write(root / name, doc)
            transition_count = 0
            serial = 0
            negatives = []

            def append(data, kind="review-evidence"):
                nonlocal serial
                serial += 1
                cli("show", rid, "--view", "full")
                cli("append", "--record-id", "proof-" + str(serial), rid, kind, json.dumps(data))

            while True:
                full = cli("show", rid, "--view", "full")["result"]
                assert full["initial_input"] == frozen
                state = full["current_state"]
                if state == "end":
                    # Installed0.25.0 lifecycle is final; completed is the separate
                    # ordinary completion-mode projection, not lifecycle vocabulary.
                    assert full["lifecycle"] == "final"
                    assert full["requestable_events"] == []
                    assert full["completion_mode"] == "completed"
                    assert full["has_overrides"] is False and full["override_count"] == 0
                    assert any(row["id"] == state and row["final"] is True
                               for row in full["workflow_graph"]["states"])
                    history = cli("history", rid)["result"]
                    assert isinstance(history, list) and history
                    sequences = [row["sequence"] for row in history]
                    assert sequences == sorted(set(sequences))
                    by_sequence = {row["sequence"]: row for row in history}
                    denied_evaluations = [row for row in full["evaluation_history"]
                                          if row["result"]["result"] == "deny"]
                    assert {"software-change-schema-deny", "software-change-evidence-incomplete"} <= {
                        row["result"]["feedback"]["code"] for row in denied_evaluations}
                    for evaluation in denied_evaluations:
                        action = by_sequence[evaluation["sequence"]]["action"]
                        assert action["kind"] == "transition"
                        assert action["transition"] == evaluation["transition"]
                        assert action["outcome"]["outcome"] == "denied"
                        assert action["outcome"]["feedback"] == evaluation["result"]["feedback"]
                    assert any(row["action"]["kind"] == "transition"
                               and row["action"]["transition"]["target"] == "end"
                               and row["action"]["outcome"]["outcome"] == "committed"
                               for row in history)
                    receipt["journeys"].append({"profile": profile_name, "run_id": rid,
                        "state": state, "transitions": transition_count,
                        "negative_cases": negatives, "frozen_input": frozen,
                        "artifact_root": str(root), "history": history})
                    break
                edge = next(t for t in full["workflow_graph"]["transitions"]
                            if t["source"] == state and t["kind"] == "checked")
                axes = submitted["review_policies"].get(state, [])
                if axes:
                    before_state = full["current_state"]
                    cli("event", rid, edge["event"], status="rejected")
                    assert cli("show", rid, "--view", "full")["result"]["current_state"] == before_state
                    subject = ("validation-report.json" if state.startswith("validation") else
                               "implementation-report.json" if state.startswith("implementation") else
                               state.split("-")[0] + ".json")
                    good = [{"gate": state, "policy_id": axis["id"], "subject": subject,
                             "subject_revision": documents[subject]["revision"],
                             "config_version": submitted["config_version"],
                             "author": {"name": "reviewer-" + str(i), "kind": "script"},
                             "result": "pass", "findings": ""}
                            for axis in axes for i in range(axis.get("required_authors", 1))]
                    for record in good[:-1]:
                        append(record)
                    missing = good[-1]
                    # Decisive bad contribution; each repair adds only the missing judgment.
                    for defect in ["stale-revision", "stale-config", "subject-author"]:
                        bad = copy.deepcopy(missing)
                        if defect == "stale-revision":
                            bad["subject_revision"] = "OLD-r0"
                        elif defect == "stale-config":
                            bad["config_version"] = "OLD-profile"
                        else:
                            bad["author"] = documents[subject]["author"]
                        append(bad)
                        cli("show", rid, "--view", "full")
                        cli("event", rid, edge["event"], status="rejected")
                        negatives.append({"gate": state, "case": defect})
                    if axes[-1].get("required_authors", 1) > 1:
                        bad = copy.deepcopy(missing)
                        bad["author"] = good[-2]["author"]
                        append(bad)
                        cli("show", rid, "--view", "full")
                        cli("event", rid, edge["event"], status="rejected")
                        negatives.append({"gate": state, "case": "duplicate-author"})
                    if state == "intent-review":
                        failed = copy.deepcopy(missing)
                        failed.update(result="fail", findings="SCRIPTED standing failure witness")
                        append(failed)
                        other = copy.deepcopy(missing)
                        other["author"]["name"] = "additional-distinct-reviewer"
                        append(other)
                        cli("show", rid, "--view", "full")
                        denied = cli("event", rid, edge["event"], status="rejected")
                        assert any(d.get("standing_fails", 0) > 0 for d in denied["details"])
                        negatives.append({"gate": state, "case": "standing-fail-not-outvoted"})
                    append(missing)
                    cli("show", rid, "--view", "full")
                cli("event", rid, edge["event"])
                transition_count += 1
                assert transition_count <= 30, "unexpected topology"
        receipt["fixture_after"] = inventory(fixture)
        assert receipt["fixture_after"] == before
        assert sha(engine) == ENGINE_SHA and sha(provider) == receipt["provider_sha256"]
        assert len(receipt["journeys"]) == 3
        assert {str(path): sha(path) for path in reuse_path.parent.glob("run.sqlite*") if path.is_file()} == old_catalog_hashes
        assert {str(path): sha(path) for directory in preserved["failed_proof_directories"]
                for path in Path(directory).glob("run.sqlite*") if path.is_file()} == failed_catalog_hashes
        receipt["preserved_failed_catalogs_unchanged"] = True
        receipt["old_catalog_unchanged"] = True
        receipt["status"] = "passed"
    except BaseException as error:
        receipt.update(status="failed", failure=repr(error))
        raise
    finally:
        write(output / "receipt.json", receipt)


if __name__ == "__main__":
    main()
