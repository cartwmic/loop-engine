"""Arm-neutral completion and scoring. Pure Python: no Docker, no model, no hidden test execution on the host.

Hidden-test results are supplied in the runner's output shape (case_runtime.run_tests), because hidden tests run only
in the trial container.
"""
from pathlib import Path

import pytest

from scevals.cases import load_case
from scevals.scoring import discipline
from scevals.scoring_common import completion, hidden_tests_passed, intent_is_valid, score_common

ROOT = Path(__file__).resolve().parents[1]
CASE = load_case(ROOT / "example-case")


def workspace(*, reference=True):
    """Final workspace of the example case: the public repository, overlaid with the reference solution if asked."""
    files = {p.relative_to(CASE.repository).as_posix(): p.read_bytes() for p in CASE.repository.rglob("*") if p.is_file()}
    if reference:
        files |= {p.relative_to(CASE.reference).as_posix(): p.read_bytes() for p in CASE.reference.rglob("*") if p.is_file()}
    return files


def runner_output(failing=()):
    return {"tests": [{"test_id": f"test_{index}", "requirement_id": tag,
                       "status": "failed" if index in failing else "passed"}
                      for index, tag in enumerate(CASE.test_requirements)],
            "collection_error": False}


def no_workflow_paths(name):
    return False


def score_example(**changes):
    arguments = {"evidence": {"driver_exit_code": 0}, "delivered": workspace(), "tests": runner_output(),
                 "workflow_owned": no_workflow_paths, "revealed_requirement_ids": list(CASE.requirement_ids),
                 "push_events": [], "owner_question_count": 0}
    arguments.update(changes)
    return score_common(CASE, **arguments)


def without_diagnostics(result):
    return {key: value for key, value in result.items() if key != "diagnostics"}


@pytest.mark.parametrize("exit_code", [0, 1, 137])
def test_driver_exit_by_itself_is_complete_whatever_its_exit_code(exit_code):
    assert completion({"driver_exit_code": exit_code}) == (True, None)
    result = score_example(evidence={"driver_exit_code": exit_code})
    assert result["complete"] and result["incomplete_reason"] is None and result["success"]


@pytest.mark.parametrize("kind", ["idle", "limit", "hard"])
def test_harness_stop_is_incomplete_and_never_success_even_with_correct_delivery(kind):
    evidence = {"driver_exit_code": 137, "driver_timed_out": True, "driver_timeout_kind": kind}
    result = score_example(evidence=evidence)
    assert not result["complete"] and result["incomplete_reason"] == kind
    assert not result["success"]
    assert result["hidden_test_outcome"]["passed"]
    assert result["delivery_recall"]["value"] == 1.0


def test_hard_stop_without_recorded_kind_is_hard():
    assert completion({"driver_timed_out": True}) == (False, "hard")


def test_driver_without_exit_code_is_incomplete():
    result = score_example(evidence={})
    assert not result["complete"] and result["incomplete_reason"] == "no_driver_exit"
    assert not result["success"]


@pytest.mark.parametrize(("tests", "passed"), [
    (runner_output(), True),
    (runner_output(failing=(0,)), False),
    ({"tests": runner_output()["tests"], "collection_error": True}, False),
    ({"tests": runner_output()["tests"][:-1], "collection_error": False}, False),
    ({"tests": [dict(t, status="skipped") if i == 0 else t for i, t in enumerate(runner_output()["tests"])],
      "collection_error": False}, False),
])
def test_success_needs_every_hidden_test_to_pass_on_a_clean_collection(tests, passed):
    assert hidden_tests_passed(CASE, tests) is passed
    result = score_example(tests=tests)
    assert result["complete"]
    assert result["success"] is passed
    assert result["hidden_test_outcome"]["passed"] is passed


def test_complete_run_with_a_failing_test_keeps_partial_delivery_recall():
    result = score_example(tests=runner_output(failing=(0,)))
    assert result["complete"] and not result["success"]
    assert result["delivery_recall"]["numerator"] == 5 and result["delivery_recall"]["denominator"] == 6
    assert result["hidden_test_outcome"] == {"passed": False, "passed_tests": 5, "total_tests": 6}


@pytest.mark.parametrize(("lifecycle", "exit_code"),
                         [("final", 0), ("terminated", 0), ("active", 0), ("active", 1)],
                         ids=["completed", "terminated", "active", "crashed"])
def test_engine_state_and_exit_code_never_decide_success(lifecycle, exit_code):
    reference = score_example(evidence={"driver_exit_code": 0}, engine_lifecycle="final", intent_valid=True)
    result = score_example(evidence={"driver_exit_code": exit_code}, engine_lifecycle=lifecycle, intent_valid=False)
    assert result["complete"] and result["success"]
    assert result["diagnostics"] == {"driver_exit_code": exit_code, "engine_lifecycle": lifecycle,
                                     "intent_valid": False, "workflow_adherence": None}
    assert without_diagnostics(result) == without_diagnostics(reference)


def test_arm_without_engine_reports_null_engine_diagnostics():
    result = score_example()
    assert result["diagnostics"] == {"driver_exit_code": 0, "engine_lifecycle": None,
                                     "intent_valid": None, "workflow_adherence": None}


def test_workflow_adherence_is_reported_as_given_and_never_decides_success():
    adherence = {"provider_denials": 2, "revise_loops_per_gate": {"intent-review": 1}}
    result = score_example(evidence={"driver_exit_code": 1}, workflow_adherence=adherence)
    assert result["success"] and result["diagnostics"]["workflow_adherence"] == adherence


@pytest.mark.parametrize(("raw", "valid"), [
    (None, False),
    (b'{"requirements": []}', True),
    (b"[]", False),
    (b"not json", False),
    (b"\xff\xfe", False),
])
def test_intent_validity_is_a_json_object(raw, valid):
    assert intent_is_valid(raw) is valid


def test_elicitation_counts_revealed_requirements_and_is_null_without_owner_history():
    partial = score_example(revealed_requirement_ids=[CASE.requirement_ids[0]])
    assert partial["elicitation_recall"]["numerator"] == 1
    assert partial["elicitation_recall"]["denominator"] == len(CASE.requirement_ids)
    unavailable = score_example(revealed_requirement_ids=None)
    assert unavailable["elicitation_recall"] is None
    assert unavailable["delivery_recall"]["value"] == 1.0


def test_unmeasured_pushes_and_owner_questions_stay_null():
    result = score_example(push_events=None, owner_question_count=None)
    assert result["discipline"]["pushes_to_local_remote"] is None
    assert result["discipline"]["owner_question_count"] is None


def test_workflow_owned_files_leave_scope_and_diff_and_count_as_artifact_volume():
    delivered = workspace()
    delivered["notes/outside.txt"] = b"oops\n"
    without_workflow = score_example(delivered=delivered)["discipline"]
    delivered["openspec/changes/plan/proposal.md"] = b"plan\n"
    delivered[".pi/skills/plan.md"] = b"step one\nstep two\n"

    def workflow_owned(name):
        return name.startswith(("openspec/", ".pi/"))

    scored = score_example(delivered=delivered, workflow_owned=workflow_owned)["discipline"]
    assert scored["diff_size"] == without_workflow["diff_size"]
    assert scored["diff_size_vs_reference"] == without_workflow["diff_size_vs_reference"]
    assert scored["edits_outside_allowed_scope"] == 1 == without_workflow["edits_outside_allowed_scope"]
    assert scored["artifact_volume"] == {"changed_files": 2, "added_lines": 3, "deleted_lines": 0, "changed_lines": 3}
    assert without_workflow["artifact_volume"]["changed_files"] == 0

    unexcluded = score_example(delivered=delivered)["discipline"]
    assert unexcluded["edits_outside_allowed_scope"] == 3
    assert unexcluded["diff_size"]["changed_files"] == scored["diff_size"]["changed_files"] + 2


def test_scope_and_diff_match_existing_scoring_when_no_path_is_workflow_owned():
    delivered = workspace()
    delivered["notes/outside.txt"] = b"oops\n"
    existing = discipline(CASE, delivered, ["push"], 2)
    scored = score_example(delivered=delivered, push_events=["push"], owner_question_count=2)["discipline"]
    for key in ("diff_size", "reference_diff_size", "diff_size_vs_reference", "edits_outside_allowed_scope",
                "pushes_to_local_remote", "owner_question_count"):
        assert scored[key] == existing[key], key
    assert scored["artifact_volume"]["changed_files"] == 0
