#!/usr/bin/env python3
"""Check exact acceptance-to-proof mapping against supplied repository evidence."""

from __future__ import annotations

import re
import sys
from pathlib import Path

ACCEPTANCE_REQUIREMENTS = (
    "A caller can inspect the frozen policies and artifact schemas before evaluating a transition.",
    "A malformed subject artifact is denied with each violated structural rule named.",
    "A checked transition refuses stale, self-authored, duplicate-author, or configured-obligation-incomplete evidence.",
    "The reference workflow reaches its terminal state only after its configured validation obligations are satisfied.",
)
EXPECTED_PROOFS = (
    (
        "fictional-repo/provider/tests/a1.rs::standard_run_progresses_schema_deny_then_evidence_deny_then_allow",
        "fictional-repo/provider/tests/a15.rs::every_shipped_profile_starts_and_exposes_policies_and_schemas",
        "fictional-repo/provider/tests/describe_protocol.rs::describe_matches_committed_snapshot_byte_for_byte",
        "scripts/production-journey.py::frozen_profile_is_inspectable_before_transition",
    ),
    (
        "schema::tests::instance_collects_object_rules_and_nested_properties",
        "fictional-repo/provider/tests/evaluate.rs::schema_deny_reports_every_simultaneous_structural_violation",
        "absent_artifact_is_schema_deny",
        "unparseable_artifact_is_schema_deny",
        "revision_link_mismatch_is_schema_deny_naming_both_artifacts",
        "schema_deny_is_byte_identical_when_only_context_varies",
        "scripts/production-journey.py::malformed_artifact_denial_names_all_rules",
    ),
    (
        "fictional-repo/provider/src/evidence.rs::stale_revision_does_not_satisfy_and_names_both_revisions",
        "stale_config_does_not_satisfy_and_names_both_versions",
        "subject_author_identity_requires_exact_name_and_kind_pair",
        "n_two_requires_distinct_non_subject_authors_and_rejects_standing_fail",
        "fictional-repo/provider/tests/evaluate.rs::evidence_phase_denies_with_configured_axis_diagnostics",
        "scripts/production-journey.py::evidence_denial_reports_each_configured_reason",
    ),
    (
        "scripts/production-journey.py::terminal_validation_gate",
        "python3 fictional-repo/scripts/assert-doc-authority.py",
    ),
)
INVENTORY_PATH = Path("implementation-evidence/repo-state-2026-08-12.txt")
INVENTORY_HEADING = "## Executable proof inventory"
INVENTORY_PREFIX = "proof: "
BACKTICK = re.compile(r"`([^`]+)`")
NON_PROOF_INLINE_CODE = frozenset({"validation→end"})


def read_text(path: Path) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except (OSError, UnicodeError) as error:
        raise ValueError(f"cannot read {path}: {error}") from error


def matrix_proofs(matrix: Path, requirement: str) -> tuple[str, ...]:
    text = read_text(matrix)
    found = None
    for line in text.splitlines():
        cells = line.split("|")
        if len(cells) < 4 or cells[1].strip() != requirement:
            continue
        if found is not None:
            raise ValueError(f"duplicate acceptance requirement in proof matrix: {requirement}")
        found = tuple(
            token for token in BACKTICK.findall(cells[2]) if token not in NON_PROOF_INLINE_CODE
        )
    if found is None:
        raise ValueError(f"acceptance requirement missing from proof matrix: {requirement}")
    if not found:
        raise ValueError(f"proof mapping missing for requirement: {requirement}")
    return found


def proof_inventory(state: Path) -> tuple[str, ...]:
    text = read_text(state)
    if "HEAD: repo-state-2026-08-12" not in text:
        raise ValueError(f"repository-state evidence has unexpected HEAD: {state}")
    try:
        start = text.splitlines().index(INVENTORY_HEADING) + 1
    except ValueError as error:
        raise ValueError(f"repository-state evidence missing {INVENTORY_HEADING}") from error

    entries = []
    for line in text.splitlines()[start:]:
        if line.startswith("## "):
            break
        if not line.strip():
            continue
        if not line.startswith(INVENTORY_PREFIX):
            raise ValueError(f"malformed executable proof inventory line: {line}")
        identifier = line[len(INVENTORY_PREFIX) :]
        if not identifier:
            raise ValueError("executable proof inventory contains empty identifier")
        if identifier in entries:
            raise ValueError(f"duplicate executable proof inventory identifier: {identifier}")
        entries.append(identifier)
    if not entries:
        raise ValueError("executable proof inventory is empty")
    return tuple(entries)


def check(root: Path) -> None:
    matrix = root / "implementation-evidence" / "requirement-to-proof.md"
    state = root / INVENTORY_PATH
    actual_by_requirement = tuple(matrix_proofs(matrix, requirement) for requirement in ACCEPTANCE_REQUIREMENTS)
    for requirement, actual, expected in zip(ACCEPTANCE_REQUIREMENTS, actual_by_requirement, EXPECTED_PROOFS):
        if actual != expected:
            raise ValueError(
                f"proof mapping mismatch for {requirement}: expected {expected!r}, found {actual!r}"
            )

    actual_inventory = proof_inventory(state)
    expected_inventory = tuple(proof for proofs in EXPECTED_PROOFS for proof in proofs)
    if actual_inventory != expected_inventory:
        raise ValueError(
            "executable proof inventory mismatch: "
            f"expected {expected_inventory!r}, found {actual_inventory!r}"
        )


def main() -> int:
    root = Path(sys.argv[1]) if len(sys.argv) == 2 else Path(__file__).resolve().parents[1]
    if len(sys.argv) > 2:
        print("assert-requirement-proof: expected zero or one root argument", file=sys.stderr)
        return 1
    try:
        root = root.resolve()
        if not root.is_dir():
            raise ValueError(f"companion root is not a directory: {root}")
        check(root)
    except (OSError, UnicodeError, ValueError) as error:
        print(f"assert-requirement-proof: {error}", file=sys.stderr)
        return 1
    print("assert-requirement-proof: passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
