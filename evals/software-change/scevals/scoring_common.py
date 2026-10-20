"""Arm-neutral completion and scoring: one rule for every arm, Loop Engine included.

A trial is complete when its driver exits by itself before its driver budget.
It succeeds when it is complete and every hidden test passes on the final
workspace. Engine lifecycle, intent.json validity, exit code and workflow
adherence are diagnostics and never decide success. Workflow-owned paths leave
scope and diff accounting and are reported as artifact volume.

Infrastructure stops raise before scoring and never reach this module. The
hidden-test runner runs in the trial container; this module consumes its output.
"""
from collections import Counter
from collections.abc import Callable, Mapping, Sequence
from fnmatch import fnmatchcase
import json
from pathlib import Path

from .cases import EvalCase
from .scoring import diff_size, recall


def completion(evidence: Mapping) -> tuple[bool, str | None]:
    """Complete when the driver exited by itself; otherwise the harness stop kind (idle, limit or hard)."""
    if evidence.get("driver_timed_out"):
        return False, evidence.get("driver_timeout_kind", "hard")
    if evidence.get("driver_exit_code") is None:
        return False, "no_driver_exit"
    return True, None


def hidden_tests_passed(case: EvalCase, tests: Mapping) -> bool:
    """Every tagged hidden test ran exactly once, nothing was skipped or mis-collected, and all passed."""
    return (not tests["collection_error"] and
            Counter(t["requirement_id"] for t in tests["tests"]) == Counter(case.test_requirements) and
            all(t["status"] == "passed" for t in tests["tests"]))


def intent_is_valid(raw: bytes | None) -> bool:
    """intent.json is valid when it is a JSON object; a missing artifact is not valid."""
    if raw is None:
        return False
    try:
        return isinstance(json.loads(raw), dict)
    except ValueError:
        return False


def _tree(root: Path) -> dict[str, bytes]:
    return {p.relative_to(root).as_posix(): p.read_bytes() for p in root.rglob("*") if p.is_file()}


def _select(files: Mapping[str, bytes], workflow_owned: Callable[[str], bool], owned: bool) -> dict[str, bytes]:
    return {name: data for name, data in files.items() if workflow_owned(name) == owned}


def scoped_discipline(case: EvalCase, delivered: Mapping[str, bytes], *, workflow_owned: Callable[[str], bool],
                      push_events: Sequence[str] | None, owner_question_count: int | None) -> dict:
    """Scope and size of the product change; workflow-owned files appear only as artifact volume."""
    baseline = _tree(case.repository)
    reference = baseline | _tree(case.reference)

    def product(files):
        return _select(files, workflow_owned, owned=False)

    def artifacts(files):
        return _select(files, workflow_owned, owned=True)

    scope_baseline, scope_delivered = product(baseline), product(delivered)
    actual = diff_size(scope_baseline, scope_delivered)
    ideal = diff_size(scope_baseline, product(reference))
    outside = sum(not any(fnmatchcase(name, pattern) for pattern in case.allowed_edit_scope)
                  for name in scope_baseline.keys() | scope_delivered.keys()
                  if scope_baseline.get(name) != scope_delivered.get(name))
    return {"diff_size": actual, "reference_diff_size": ideal,
            "diff_size_vs_reference": actual["changed_lines"] / ideal["changed_lines"] if ideal["changed_lines"] else None,
            "edits_outside_allowed_scope": outside,
            "artifact_volume": diff_size(artifacts(baseline), artifacts(delivered)),
            "pushes_to_local_remote": len(push_events) if push_events is not None else None,
            "push_measurement": "successful post-receive events at the case local remote; rejected attempts unobserved",
            "owner_question_count": owner_question_count}


def score_common(case: EvalCase, *, evidence: Mapping, delivered: Mapping[str, bytes], tests: Mapping,
                 workflow_owned: Callable[[str], bool], revealed_requirement_ids: Sequence[str] | None,
                 push_events: Sequence[str] | None, owner_question_count: int | None,
                 engine_lifecycle: str | None = None, intent_valid: bool | None = None,
                 workflow_adherence: Mapping | None = None) -> dict:
    """Common score fields for one trial, from the restored final workspace and the hidden-test runner output.

    Arms without an engine, or without an intent artifact or adherence evidence, pass None for those diagnostics.
    None for revealed_requirement_ids means owner history is unavailable, so elicitation recall is None.
    """
    complete, incomplete_reason = completion(evidence)
    passed = hidden_tests_passed(case, tests)
    recalls = recall(case, tests, revealed_requirement_ids or ())
    if revealed_requirement_ids is None:
        recalls["elicitation_recall"] = None
    return {
        "complete": complete,
        "incomplete_reason": incomplete_reason,
        "success": complete and passed,
        "hidden_test_outcome": {"passed": passed,
                                "passed_tests": sum(t["status"] == "passed" for t in tests["tests"]),
                                "total_tests": len(tests["tests"])},
        **recalls,
        "discipline": scoped_discipline(case, delivered, workflow_owned=workflow_owned, push_events=push_events,
                                        owner_question_count=owner_question_count),
        "diagnostics": {"driver_exit_code": evidence.get("driver_exit_code"), "engine_lifecycle": engine_lifecycle,
                        "intent_valid": intent_valid, "workflow_adherence": workflow_adherence},
    }
