#!/usr/bin/env python3
"""NEW four-acceptance exact map/inventory + hash-bound genuine proof checker."""
import argparse
import ast
import hashlib
import json
from pathlib import Path
import re

REQUIREMENTS = (
    "A caller can inspect the frozen policies and artifact schemas before evaluating a transition.",
    "A malformed subject artifact is denied with each violated structural rule named.",
    "A checked transition refuses stale, self-authored, duplicate-author, or configured-obligation-incomplete evidence.",
    "The reference workflow reaches its terminal state only after its configured validation obligations are satisfied.",
)


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def check(root, receipt_path, binding_path, repair_path, evidence_path):
    inventory = json.loads((root / "implementation-evidence/source-inventory.json").read_text())
    mapping = json.loads((root / "implementation-evidence/four-acceptance-proof-map.json").read_text())
    assert [row["requirement"] for row in mapping] == list(REQUIREMENTS), "four exact acceptance rows"
    matrix = (root / "implementation-evidence/requirement-to-proof.md").read_text()
    references = set()
    for expected in mapping:
        rows = [line.split("|") for line in matrix.splitlines()
                if line.startswith("| " + expected["requirement"] + " |")]
        assert len(rows) == 1, "missing/duplicate exact matrix requirement"
        actual = re.findall(r"`([^`]+)`", rows[0][2])
        assert actual == expected["proofs"], "matrix proof names disagree"
        references.update(actual)
    assert references == set(inventory["proof_inventory"]), "proof inventory differs"
    for reference in references:
        filename, name = reference.split("::", 1)
        path = root / filename
        assert path.is_file(), "proof source absent"
        if path.suffix == ".rs":
            assert re.search(r"fn\s+" + re.escape(name) + r"\s*\(", path.read_text()), "Rust proof name absent"
        else:
            tree = ast.parse(path.read_text())
            assert any(isinstance(node, ast.FunctionDef) and node.name == name
                       for node in ast.walk(tree)), "Python proof name absent"
    actual_files = {str(p.relative_to(root)): sha(p) for p in sorted(root.rglob("*"))
                    if p.is_file() and p.name != "source-inventory.json"}
    assert actual_files == inventory["files_sha256"], "covered source inventory differs"
    journey = (root / "scripts/production-journey.py").read_text()
    prefix = "for record in good[:-1]:"
    repair = "append(missing)"
    assert prefix in journey and repair in journey, "public journey lacks actual deficit/repair"
    assert journey.index(prefix) < journey.index("for defect in") < journey.index(repair), "public journey preloads coverage"

    binding = json.loads(binding_path.read_text())
    assert sha(receipt_path) == binding["runtime_receipt_sha256"], "runtime receipt identity mismatch"
    receipt = json.loads(receipt_path.read_text())
    assert receipt["status"] == "passed" and len(receipt["journeys"]) == 3
    assert all(j["state"] == "end" and j["history"] for j in receipt["journeys"])
    assert receipt["fixture_before"] == receipt["fixture_after"] == binding["v8_source_sha256"]
    assert receipt["provider_sha256"] == binding["provider_sha256"]
    for filename, expected in binding["unchanged_runtime_files"].items():
        assert sha(root / filename) == expected, "runtime input changed"
    for filename, entry in binding["runtime_prefixes"].items():
        if filename in binding["focused_evidence_runtime_changes"]:
            continue  # Original v8 prefix remains historical; actual v13 proof binds current bytes below.
        content = (root / filename).read_bytes()
        assert hashlib.sha256(content[:entry["bytes"]]).hexdigest() == entry["sha256"], "runtime body changed"
        assert content[entry["bytes"]:].decode().lstrip().startswith(entry["test_only_marker"])
    assert (root / "scripts/production-journey.py").read_bytes() == Path(binding["v8_journey"]).read_bytes()
    repair = json.loads(repair_path.read_text())
    assert repair["status"] == "passed" and len(repair["cases"]) == 2
    assert repair["cases"][0]["label"] == "old-red" and repair["cases"][0]["contract"]["contract_pass"] is False
    assert repair["cases"][1]["label"] == "new-green" and repair["cases"][1]["contract"]["contract_pass"] is True
    assert repair["cases"][1]["corrected_continuation"] == "design-review"
    assert repair["current_source_sha256"] == binding["focused_proof_source_sha256"], "original repair execution source differs"
    current_source = {str(p.relative_to(root)): sha(p) for p in sorted(root.rglob("*")) if p.is_file()}
    changed_since_repair = {name for name in set(current_source) | set(repair["current_source_sha256"])
                            if current_source.get(name) != repair["current_source_sha256"].get(name)}
    allowed_docs = {"implementation-evidence/four-acceptance-proof-map.json",
                    "implementation-evidence/requirement-to-proof.md",
                    "implementation-evidence/source-inventory.json", "scripts/assert-requirement-proof.py"}
    evidence = json.loads(evidence_path.read_text())
    assert sha(evidence_path) == binding["evidence_proof_receipt_sha256"], "evidence receipt identity mismatch"
    assert evidence["status"] == "passed" and evidence["contract_pass"] is True
    assert evidence["source_before"] == evidence["source_after"] == binding["evidence_proof_source_sha256"], "original evidence execution source differs"
    expected_cases = [("valid-control", "completed", False),
                      ("missing-config-restoration", "rejected", True),
                      ("stale-with-sufficient", "completed", False),
                      ("stale-noncounting", "rejected", False),
                      ("stale-malformed-order-unchanged", "completed", False),
                      ("standing-fail-and-own-restoration", "rejected", True)]
    assert [(case["case"], case["event_status"], case["same_author_restoration"])
            for case in evidence["cases"]] == expected_cases, "evidence scenario outcomes differ"
    assert all(case["context_unchanged"] is True for case in evidence["cases"])
    # Only this documented successor is additional to the original four paths.
    # Bind its actual bytes at both genuine proof sources and the current source;
    # do not edit either historical receipt or infer its absent source fields.
    current_documents = binding.get("current_document_bindings", {})
    assert isinstance(current_documents, dict), "current document bindings must be an object"
    assert set(current_documents) <= {"loop-engine-software-change-provider-task-packets.md"}, "unsupported current document path"
    for filename, entry in current_documents.items():
        assert entry == {
            "repair_source_sha256": repair["current_source_sha256"][filename],
            "evidence_source_sha256": evidence["source_before"][filename],
            "current_source_sha256": current_source[filename],
        }, "current document binding differs from genuine proof or current source"
        allowed_docs.add(filename)
    changed_since_evidence = {name for name in set(current_source) | set(evidence["source_before"])
                             if current_source.get(name) != evidence["source_before"].get(name)}
    assert changed_since_evidence <= allowed_docs, "runtime/test/data changed after evidence proof"
    assert set(binding["document_only_successor_paths"]) == allowed_docs
    assert set(binding["focused_evidence_runtime_changes"]) == {"provider/src/evidence.rs"}
    assert changed_since_repair <= allowed_docs | {"provider/src/evidence.rs"}, "unproved runtime changes after original repair"
    for filename, entry in binding["focused_evidence_runtime_changes"].items():
        assert repair["current_source_sha256"][filename] == entry["before_sha256"]
        assert sha(root / filename) == evidence["source_before"][filename] == entry["after_sha256"]
        assert entry["scope"] == "required revision/config shape guard only, original stale author/verdict order preserved"
    for filename, entry in binding["focused_runtime_changes"].items():
        assert sha(root / filename) == entry["after_sha256"]
        assert entry["scope"] == "propagate linked-parent containment error; ordinary parent mismatch/missing unchanged"
    print("four exact acceptance rows, actual named source inventory, deficit/repair, and genuine runtime receipt agree; not semantic review")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--receipt", type=Path, required=True)
    parser.add_argument("--binding", type=Path, required=True)
    parser.add_argument("--repair-proof", type=Path, required=True)
    parser.add_argument("--evidence-proof", type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    if json.loads(args.binding.read_bytes()).get("generation") == "new-current-01":
        from assert_new_current_proof import validate
        print(json.dumps(validate(root, args.receipt, args.binding, args.repair_proof, args.evidence_proof)))
    else:
        check(root, args.receipt, args.binding, args.repair_proof, args.evidence_proof)


if __name__ == "__main__":
    main()
