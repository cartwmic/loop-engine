#!/usr/bin/env python3
"""Verify the current run's narrow Bookends traceability reconciliation.

This is a read-only, post-integration assertion.  It compares the current
intent with the owner-approved revision-14 snapshot and permits exactly these
status-only changes::

    AC-3  candidate LE-143 -> linked-live LE-143
    AC-6  candidate LE-141 -> linked-live LE-141
    AC-7  candidate LE-141 -> linked-live LE-141
    AC-10 candidate LE-142 -> linked-live LE-142
    AC-15 candidate LE-144 -> linked-live LE-144
    AC-16 candidate LE-144 -> linked-live LE-144
    AC-17 candidate LE-142 -> linked-live LE-142
    AC-19 candidate LE-142 -> linked-live LE-142

The driver must supply a recorded commit identity, an explicit applicability
statement, an unchanged-semantic rationale, and independent inspection
references.  The checker also reads the exact committed ``docs/PRD.md`` bytes
and verifies the four accepted records there.  It never edits intent metadata,
accepts requirements, stages, commits, promotes a candidate, or advances a
workflow.

The promotion record format is intentionally small and driver-owned.  Its
required fields are ``plan_revision``, ``design_revision``,
``approved_snapshot`` (with ``path`` and ``sha256``), ``mappings`` (the eight
``criterion_id``/``from``/``to`` entries), ``applicability``,
``unchanged_semantic_rationale``, and ``proof_references``.  An explicit
``independent_inspection`` object or ``independently_inspected: true`` is also
required.  The commit record may use ``commit`` or ``head`` for the full Git
SHA; a full SHA must match the current ``HEAD``.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import sys
from pathlib import Path
from typing import Any, Mapping, Sequence

ROOT = Path(__file__).resolve().parents[1]
EXPECTED_INTENT_REVISION = "14"
EXPECTED_PLAN_REVISION = "8"
EXPECTED_DESIGN_REVISION = "4"

# Keep this ordered: it is the owner-approved order in the plan handoff and in
# the driver promotion record.  A different order is not a harmless rewrite.
EXPECTED_MAPPINGS: tuple[tuple[str, str], ...] = (
    ("AC-3", "LE-143"),
    ("AC-6", "LE-141"),
    ("AC-7", "LE-141"),
    ("AC-10", "LE-142"),
    ("AC-15", "LE-144"),
    ("AC-16", "LE-144"),
    ("AC-17", "LE-142"),
    ("AC-19", "LE-142"),
)
EXPECTED_CRITERIA = tuple(criterion for criterion, _requirement in EXPECTED_MAPPINGS)
EXPECTED_REQUIREMENTS = tuple(dict.fromkeys(requirement for _criterion, requirement in EXPECTED_MAPPINGS))
FULL_SHA_RE = re.compile(r"^[0-9a-fA-F]{40,64}$")
HEX_SHA256_RE = re.compile(r"^[0-9a-f]{64}$")


class TraceabilityError(RuntimeError):
    """The current-run traceability promotion is not mechanically safe."""


def fail(message: str) -> "NoReturn":
    raise TraceabilityError(message)


def read_bytes(path: Path, label: str) -> bytes:
    try:
        return path.read_bytes()
    except OSError as error:
        fail(f"could not read {label} {path}: {error}")


def read_json(path: Path, label: str) -> Any:
    try:
        return json.loads(path.read_bytes())
    except OSError as error:
        fail(f"could not read {label} {path}: {error}")
    except (UnicodeError, json.JSONDecodeError) as error:
        fail(f"invalid JSON in {label} {path}: {error}")


def require(condition: bool, message: str) -> None:
    if not condition:
        fail(message)


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def sha256_file(path: Path, label: str) -> str:
    return sha256_bytes(read_bytes(path, label))


def as_object(value: Any, label: str) -> dict[str, Any]:
    require(isinstance(value, dict), f"{label} must be a JSON object")
    return value


def nonempty_string(value: Any, label: str) -> str:
    require(isinstance(value, str) and bool(value.strip()), f"{label} must be a non-empty string")
    return value


def acceptance_map(value: Any, label: str) -> tuple[list[dict[str, Any]], dict[str, dict[str, Any]]]:
    document = as_object(value, label)
    acceptance = document.get("acceptance")
    require(isinstance(acceptance, list), f"{label}.acceptance must be an array")
    entries: list[dict[str, Any]] = []
    by_id: dict[str, dict[str, Any]] = {}
    for index, item in enumerate(acceptance):
        criterion = as_object(item, f"{label}.acceptance[{index}]")
        criterion_id = nonempty_string(criterion.get("id"), f"{label}.acceptance[{index}].id")
        require(criterion_id not in by_id, f"{label}.acceptance repeats {criterion_id}")
        by_id[criterion_id] = criterion
        entries.append(criterion)
    require(entries, f"{label}.acceptance must not be empty")
    return entries, by_id


def type_name(value: Any) -> str:
    if value is None:
        return "null"
    if isinstance(value, bool):
        return "boolean"
    if isinstance(value, dict):
        return "object"
    if isinstance(value, list):
        return "array"
    return type(value).__name__


def assert_unchanged(
    before: Any,
    after: Any,
    allowed_paths: set[tuple[object, ...]],
    path: tuple[object, ...] = (),
) -> None:
    """Fail unless differences occur at the eight named disposition objects."""
    if path in allowed_paths:
        return
    if type(before) is not type(after):
        fail(
            f"unexpected change at {format_path(path)}: "
            f"{type_name(before)} became {type_name(after)}"
        )
    if isinstance(before, dict):
        before_keys = set(before)
        after_keys = set(after)
        if before_keys != after_keys:
            missing = sorted(str(key) for key in before_keys - after_keys)
            added = sorted(str(key) for key in after_keys - before_keys)
            detail: list[str] = []
            if missing:
                detail.append("removed " + ", ".join(missing))
            if added:
                detail.append("added " + ", ".join(added))
            fail(f"unexpected object change at {format_path(path)}: {'; '.join(detail)}")
        for key in before:
            assert_unchanged(before[key], after[key], allowed_paths, path + (key,))
        return
    if isinstance(before, list):
        if len(before) != len(after):
            fail(
                f"unexpected array length change at {format_path(path)}: "
                f"{len(before)} became {len(after)}"
            )
        for index, (before_item, after_item) in enumerate(zip(before, after)):
            assert_unchanged(before_item, after_item, allowed_paths, path + (index,))
        return
    if before != after:
        fail(f"unexpected change at {format_path(path)}: {before!r} became {after!r}")


def format_path(path: Sequence[object]) -> str:
    result = "$"
    for item in path:
        if isinstance(item, int):
            result += f"[{item}]"
        else:
            result += f"[{item!r}]"
    return result


def exact_traceability_snapshot(
    approved: dict[str, Any], current: dict[str, Any]
) -> tuple[dict[str, dict[str, Any]], dict[str, dict[str, Any]]]:
    approved_entries, approved_by_id = acceptance_map(approved, "approved snapshot")
    current_entries, current_by_id = acceptance_map(current, "current intent")
    require(
        [entry["id"] for entry in approved_entries] == [entry["id"] for entry in current_entries],
        "current intent changed acceptance criterion order or membership",
    )
    require(approved.get("revision") == EXPECTED_INTENT_REVISION, "approved snapshot is not intent revision 14")
    require(current.get("revision") == EXPECTED_INTENT_REVISION, "current intent is not intent revision 14")

    approved_candidates: dict[str, dict[str, Any]] = {}
    for criterion_id, requirement_id in EXPECTED_MAPPINGS:
        criterion = approved_by_id.get(criterion_id)
        require(criterion is not None, f"approved snapshot lacks {criterion_id}")
        disposition = criterion.get("prd_traceability")
        require(isinstance(disposition, dict), f"approved {criterion_id} lacks prd_traceability")
        expected = {
            "type": "candidate",
            "proposed_id": requirement_id,
            "record_markdown": disposition.get("record_markdown"),
        }
        require(
            disposition == expected and isinstance(expected["record_markdown"], str)
            and bool(expected["record_markdown"].strip()),
            f"approved {criterion_id} is not the exact candidate for {requirement_id}",
        )
        approved_candidates[criterion_id] = disposition

    candidate_ids = {
        criterion_id
        for criterion_id, criterion in ((entry["id"], entry) for entry in approved_entries)
        if isinstance(criterion.get("prd_traceability"), dict)
        and criterion["prd_traceability"].get("type") == "candidate"
    }
    require(
        candidate_ids == set(EXPECTED_CRITERIA),
        "approved snapshot candidate set differs from the eight owner-approved mappings",
    )

    for criterion_id, requirement_id in EXPECTED_MAPPINGS:
        current_criterion = current_by_id[criterion_id]
        current_disposition = current_criterion.get("prd_traceability")
        expected_current = {"type": "linked-live", "live_ids": [requirement_id]}
        require(
            current_disposition == expected_current,
            f"current {criterion_id} must be exactly linked-live to {requirement_id}",
        )

    allowed_paths: set[tuple[object, ...]] = set()
    for index, entry in enumerate(approved_entries):
        if entry["id"] in EXPECTED_CRITERIA:
            allowed_paths.add(("acceptance", index, "prd_traceability"))
    assert_unchanged(approved, current, allowed_paths)

    current_candidates = {
        entry["id"]
        for entry in current_entries
        if isinstance(entry.get("prd_traceability"), dict)
        and entry["prd_traceability"].get("type") == "candidate"
    }
    require(not current_candidates, "current intent still contains a candidate disposition")
    return approved_candidates, current_by_id


def snapshot_reference(record: Mapping[str, Any], approved_path: Path) -> None:
    reference: Any = record.get("approved_snapshot", record.get("pre_promotion_snapshot"))
    if reference is None:
        digest = record.get("approved_snapshot_sha256", record.get("snapshot_sha256"))
        reference = {"sha256": digest}
    if isinstance(reference, str):
        reference = {"sha256": reference}
    reference = as_object(reference, "promotion record approved_snapshot")
    recorded_digest = nonempty_string(
        reference.get("sha256", reference.get("digest")),
        "promotion record approved_snapshot.sha256",
    ).lower()
    require(HEX_SHA256_RE.fullmatch(recorded_digest) is not None, "approved snapshot digest is not SHA-256")
    actual_digest = sha256_file(approved_path, "approved snapshot")
    require(recorded_digest == actual_digest, "promotion record does not identify the supplied approved snapshot bytes")
    recorded_path = reference.get("path")
    if recorded_path is not None:
        recorded_path = nonempty_string(recorded_path, "promotion record approved_snapshot.path")
        path = Path(recorded_path)
        if not path.is_absolute():
            path = (approved_path.parent / path).resolve()
        require(path.resolve() == approved_path.resolve(), "promotion record approved snapshot path differs")


def verify_mapping_record(
    record: Mapping[str, Any],
    approved_by_id: Mapping[str, dict[str, Any]],
    current_by_id: Mapping[str, dict[str, Any]],
) -> list[str]:
    raw = record.get("mappings", record.get("changed_mappings"))
    require(isinstance(raw, list), "promotion record mappings must be an array")
    require(len(raw) == len(EXPECTED_MAPPINGS), "promotion record must contain exactly eight mappings")
    proof_references: list[str] = []
    seen: set[str] = set()
    for index, item in enumerate(raw):
        mapping = as_object(item, f"promotion record mappings[{index}]")
        criterion_id = nonempty_string(
            mapping.get("criterion_id", mapping.get("id")),
            f"promotion record mappings[{index}].criterion_id",
        )
        require(criterion_id not in seen, f"promotion record repeats mapping {criterion_id}")
        seen.add(criterion_id)
        expected_criterion, expected_requirement = EXPECTED_MAPPINGS[index]
        require(
            criterion_id == expected_criterion,
            f"promotion record mapping {index} is {criterion_id}, expected {expected_criterion}",
        )
        before = mapping.get("from", mapping.get("before"))
        after = mapping.get("to", mapping.get("after"))
        require(
            before == approved_by_id[criterion_id].get("prd_traceability"),
            f"promotion record {criterion_id} does not retain the exact candidate before-state",
        )
        require(
            after == current_by_id[criterion_id].get("prd_traceability"),
            f"promotion record {criterion_id} does not identify the exact current live mapping",
        )
        require(
            after == {"type": "linked-live", "live_ids": [expected_requirement]},
            f"promotion record {criterion_id} maps to the wrong live requirement",
        )
        refs = mapping.get("proof_references")
        if refs is not None:
            proof_references.extend(string_array(refs, f"promotion record mappings[{index}].proof_references"))
    require(seen == set(EXPECTED_CRITERIA), "promotion record mapping set is not exactly the eight approved criteria")
    return proof_references


def string_array(value: Any, label: str) -> list[str]:
    require(isinstance(value, list), f"{label} must be an array")
    result: list[str] = []
    for index, item in enumerate(value):
        result.append(nonempty_string(item, f"{label}[{index}]"))
    return result


def verify_rationale_and_applicability(record: Mapping[str, Any]) -> list[str]:
    rationale_value = record.get(
        "unchanged_semantic_rationale",
        record.get("unchanged_semantics_rationale", record.get("rationale")),
    )
    if isinstance(rationale_value, dict):
        rationale_value = rationale_value.get("text", rationale_value.get("statement"))
    rationale = nonempty_string(rationale_value, "promotion record unchanged_semantic_rationale")
    require(
        len(rationale.strip()) >= 20,
        "unchanged-semantic rationale is too short to explain the status-only reconciliation",
    )
    lowered = rationale.lower()
    require("unchang" in lowered, "unchanged-semantic rationale must state semantic unchangedness")
    require(
        any(token in lowered for token in ("semantic", "meaning", "criterion")),
        "unchanged-semantic rationale must address criterion meaning",
    )
    require(
        any(token in lowered for token in ("traceability", "status", "metadata", "mapping")),
        "unchanged-semantic rationale must identify the traceability-only change",
    )

    applicability = record.get("applicability")
    require(applicability is not None, "promotion record must state applicability")
    if isinstance(applicability, str):
        require(bool(applicability.strip()), "promotion record applicability must not be empty")
    elif isinstance(applicability, dict):
        require(bool(applicability), "promotion record applicability must not be empty")
    elif isinstance(applicability, list):
        require(bool(applicability), "promotion record applicability must not be empty")
    else:
        fail("promotion record applicability must be a non-empty string, object, or array")

    refs = record.get("proof_references", record.get("public_proof"))
    return string_array(refs, "promotion record proof_references")


def recursive_objects(value: Mapping[str, Any]) -> list[Mapping[str, Any]]:
    result: list[Mapping[str, Any]] = [value]
    for key in (
        "independent_inspection",
        "inspection",
        "document_inspection",
        "authoritative_documents",
        "integration",
        "prd",
    ):
        child = value.get(key)
        if isinstance(child, dict):
            result.append(child)
    return result


def verify_independent_inspection(
    promotion: Mapping[str, Any], commit: Mapping[str, Any], mapping_proofs: Sequence[str], top_proofs: Sequence[str]
) -> list[str]:
    objects = [*recursive_objects(promotion), *recursive_objects(commit)]
    explicit = False
    exact_text = False
    references: list[str] = [*mapping_proofs, *top_proofs]
    for value in objects:
        for key in ("independently_inspected", "independent", "independent_inspection"):
            child = value.get(key)
            if child is True:
                explicit = True
            elif isinstance(child, str) and child.lower() in {"passed", "verified", "complete", "true"}:
                explicit = True
            elif isinstance(child, dict):
                status = child.get("status")
                if status in {"passed", "verified", "complete"}:
                    explicit = True
        for key in (
            "exact_prd_text",
            "exact_live_prd_text",
            "exact_prd_text_committed",
            "prd_text_committed",
            "exact_text_committed",
        ):
            if value.get(key) is True:
                exact_text = True
        for key in ("proof_references", "public_proof"):
            if key in value and value[key] is not None:
                references.extend(string_array(value[key], f"{key}"))
    require(explicit, "promotion/commit record lacks explicit independent document inspection")
    # The checker itself verifies the exact committed PRD record bytes below;
    # this boolean is still required in the record to preserve the separate
    # driver-owned inspection claim rather than silently turning this check into
    # the only semantic/document review.
    require(exact_text or any("committed" in ref.lower() or "prd" in ref.lower() for ref in references),
            "promotion/commit record does not record exact committed PRD-text inspection")
    unique = list(dict.fromkeys(references))
    require(unique, "promotion record has no associated public-proof references")
    return unique


def commit_identity(record: Mapping[str, Any]) -> str:
    values: list[str] = []
    containers: list[Mapping[str, Any]] = [record]
    for key in ("identity", "commit_identity", "git", "repository"):
        child = record.get(key)
        if isinstance(child, dict):
            containers.append(child)
    for container in containers:
        for key in ("commit", "head", "sha", "revision", "sha1", "sha256"):
            value = container.get(key)
            if isinstance(value, str) and value.strip():
                values.append(value.strip())
    require(values, "commit record lacks a recorded full Git commit identity")
    require(all(FULL_SHA_RE.fullmatch(value) for value in values), "commit record contains a non-full Git SHA identity")
    unique = set(value.lower() for value in values)
    require(len(unique) == 1, "commit record contains conflicting Git commit identities")
    return next(iter(unique))


def verify_commit_record(record: Mapping[str, Any], repository: Path) -> str:
    for key in ("committed", "commit_applied", "local_commit"):
        if key in record:
            require(record[key] is True, f"commit record {key} is not true")
    for key in ("status", "state"):
        value = record.get(key)
        if value is not None:
            require(value in {"committed", "verified", "complete", "passed"}, f"commit record {key} is not committed")
    recorded = commit_identity(record)
    result = subprocess.run(
        ["git", "rev-parse", "HEAD"],
        cwd=repository,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        check=False,
    )
    if result.returncode:
        diagnostic = result.stderr.strip() or "git rev-parse failed"
        fail(f"could not read current Git HEAD: {diagnostic}")
    head = result.stdout.strip().lower()
    require(FULL_SHA_RE.fullmatch(head) is not None, "git rev-parse HEAD did not return a full commit identity")
    require(recorded == head, f"commit record identifies {recorded}, but current HEAD is {head}")
    return head


def committed_bytes(repository: Path, path: str) -> bytes:
    result = subprocess.run(
        ["git", "show", f"HEAD:{path}"],
        cwd=repository,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if result.returncode:
        diagnostic = result.stderr.decode("utf-8", "replace").strip() or "git show failed"
        fail(f"cannot read committed HEAD:{path}: {diagnostic}")
    return result.stdout


def verify_committed_prd(repository: Path, approved_by_id: Mapping[str, dict[str, Any]]) -> None:
    prd = committed_bytes(repository, "docs/PRD.md")
    for requirement_id in EXPECTED_REQUIREMENTS:
        records = [
            criterion["prd_traceability"]["record_markdown"]
            for criterion_id, criterion in approved_by_id.items()
            if criterion_id in EXPECTED_CRITERIA
            and criterion["prd_traceability"]["proposed_id"] == requirement_id
        ]
        unique_records = list(dict.fromkeys(records))
        require(
            len(unique_records) == 1,
            f"approved snapshot does not provide one exact record for {requirement_id}",
        )
        record = unique_records[0].encode("utf-8")
        heading = f"### {requirement_id}:".encode("utf-8")
        require(prd.count(heading) == 1, f"committed docs/PRD.md must contain exactly one {requirement_id} heading")
        require(prd.count(record) == 1, f"committed docs/PRD.md lacks the exact accepted {requirement_id} text")


def verify_bounded_inputs(artifact_root: Path, plan_revision: str) -> dict[str, Any]:
    """Read only the named final-proof locations; never discover other artifacts."""
    required = {
        "receipt_index": artifact_root / "proof" / "receipts.json",
        "proof_matrix": artifact_root / "proof-matrix.json",
        "provider_readme": artifact_root / "proof" / "provider-readme-full.json",
        "provider_agents": artifact_root / "proof" / "provider-agents-full.json",
    }
    for label, path in required.items():
        require(path.is_file(), f"{label} is absent/pending at the exact required path {path}")

    matrix = as_object(read_json(required["proof_matrix"], "proof matrix"), "proof matrix")
    require(matrix.get("schema_version") == 1, "proof matrix schema_version must be 1")
    require(matrix.get("plan_revision") == plan_revision, "proof matrix plan revision differs")
    rows: list[dict[str, Any]] = []
    for section in ("local_final", "post_report", "after_separate_authorization"):
        section_rows = matrix.get(section)
        require(isinstance(section_rows, list), f"proof matrix {section} must be an array")
        rows.extend(item for item in section_rows if isinstance(item, dict))
    row_ids = {row.get("id") for row in rows}
    require("driver-current-traceability-check" in row_ids, "proof matrix omits driver-current-traceability-check")

    receipts = as_object(read_json(required["receipt_index"], "receipt index"), "receipt index")
    require(receipts.get("plan_revision") == plan_revision, "receipt index plan revision differs")
    receipt_rows = receipts.get("commands")
    require(isinstance(receipt_rows, list), "receipt index commands must be an array")
    receipt_ids = [item.get("id") for item in receipt_rows if isinstance(item, dict)]
    require(len(receipt_ids) == len(set(receipt_ids)), "receipt index contains duplicate command IDs")
    # The current check may not require its own receipt (that would be
    # circular), but the completed receipts index must include the prerequisite
    # route checks that make the promotion meaningful.
    for command_id in (
        "bookends-candidate-parse",
        "focused-software-provider-tests",
        "driver-committed-integration",
        "bookends-gate",
    ):
        require(command_id in receipt_ids, f"receipt index is missing prerequisite {command_id}")

    provider_states: dict[str, Any] = {}
    for label in ("provider_readme", "provider_agents"):
        envelope = as_object(read_json(required[label], label), label)
        if "status" in envelope:
            require(envelope["status"] == "completed", f"{label} envelope is not completed")
        provider_states[label] = envelope.get("status", "present")
    return {
        "artifact_root": str(artifact_root),
        "proof_matrix": str(required["proof_matrix"]),
        "receipt_index": str(required["receipt_index"]),
        "provider_states": provider_states,
        "prerequisite_receipts": [
            "bookends-candidate-parse",
            "focused-software-provider-tests",
            "driver-committed-integration",
            "bookends-gate",
        ],
    }


def parse_args(argv: Sequence[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--intent", type=Path, required=True)
    parser.add_argument("--approved-snapshot", type=Path, required=True)
    parser.add_argument("--promotion-record", type=Path, required=True)
    parser.add_argument("--commit", type=Path, required=True, dest="commit_record")
    parser.add_argument("--plan-revision", required=True)
    parser.add_argument("--design-revision", required=True)
    parser.add_argument(
        "--repository",
        type=Path,
        default=ROOT,
        help="repository to inspect (defaults to this script's checkout)",
    )
    return parser.parse_args(argv)


def verify(args: argparse.Namespace) -> dict[str, Any]:
    repository = args.repository.resolve()
    require(repository.is_dir(), f"repository is not a directory: {repository}")
    intent_path = args.intent.resolve()
    approved_path = args.approved_snapshot.resolve()
    promotion_path = args.promotion_record.resolve()
    commit_path = args.commit_record.resolve()

    current = as_object(read_json(intent_path, "current intent"), "current intent")
    approved = as_object(read_json(approved_path, "approved snapshot"), "approved snapshot")
    promotion = as_object(read_json(promotion_path, "promotion record"), "promotion record")
    commit = as_object(read_json(commit_path, "commit record"), "commit record")

    require(args.plan_revision == EXPECTED_PLAN_REVISION, "plan revision must be 8 for this frozen reconciliation")
    require(args.design_revision == EXPECTED_DESIGN_REVISION, "design revision must be 4 for this frozen reconciliation")
    require(promotion.get("plan_revision") == args.plan_revision, "promotion record plan revision differs")
    require(promotion.get("design_revision") == args.design_revision, "promotion record design revision differs")
    require(promotion.get("intent_revision", EXPECTED_INTENT_REVISION) == EXPECTED_INTENT_REVISION, "promotion record intent revision differs")

    _approved_entries, approved_by_id = acceptance_map(approved, "approved snapshot")
    _current_entries, current_by_id = acceptance_map(current, "current intent")
    exact_traceability_snapshot(approved, current)

    snapshot_reference(promotion, approved_path)
    mapping_proofs = verify_mapping_record(promotion, approved_by_id, current_by_id)
    top_proofs = verify_rationale_and_applicability(promotion)
    proof_references = verify_independent_inspection(promotion, commit, mapping_proofs, top_proofs)
    head = verify_commit_record(commit, repository)
    verify_committed_prd(repository, approved_by_id)

    bounded = verify_bounded_inputs(intent_path.parent, args.plan_revision)
    return {
        "status": "verified",
        "intent_revision": EXPECTED_INTENT_REVISION,
        "plan_revision": args.plan_revision,
        "design_revision": args.design_revision,
        "approved_snapshot": {
            "path": str(approved_path),
            "sha256": sha256_file(approved_path, "approved snapshot"),
        },
        "current_intent": str(intent_path),
        "committed_head": head,
        "changed_mappings": [
            {"criterion_id": criterion_id, "from": "candidate", "to": f"linked-live:{requirement_id}"}
            for criterion_id, requirement_id in EXPECTED_MAPPINGS
        ],
        "proof_references": proof_references,
        "bounded_inputs": bounded,
        "promotion": "verified only; no traceability mutation performed",
    }


def main(argv: Sequence[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    try:
        result = verify(args)
    except (TraceabilityError, OSError, subprocess.SubprocessError) as error:
        print(f"current-traceability assertion failed: {error}", file=sys.stderr)
        return 1
    print("current-traceability assertions passed: " + json.dumps(result, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
