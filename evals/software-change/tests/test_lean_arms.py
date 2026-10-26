"""Lean-change arms: descriptor, prompts, check-in text and the metrics read from engine records."""
import pytest

from scevals import arms
from scevals.lean_metrics import lean_metrics


def descriptor(arm_id):
    return arms.load_arm({"arm": {"id": arm_id, "driver_command": ["env", "SC_PROVIDER_SKILL=using-lean-change-provider", "sc-driver"],
                                  "prompt_template": arm_id, "workflow_owned_paths": [], "completion_rule": "driver-exit"}})


def prompt(arm_id, checkin=False):
    return arms.render_prompt(descriptor(arm_id), profile=arm_id, run_id="eval-trial", scripted="", request="REQ", checkin=checkin)


@pytest.mark.parametrize("arm_id", arms.LEAN_ARM_IDS)
def test_lean_arms_are_arms_that_own_no_workflow_paths(arm_id):
    arm = descriptor(arm_id)
    assert arm.id == arm_id and arm.workflow_owned_paths == ()
    with pytest.raises(ValueError):
        arms.load_arm({"arm": {"id": arm_id, "driver_command": ["x"], "prompt_template": arm_id,
                               "workflow_owned_paths": ["a/**"], "completion_rule": "driver-exit"}})


def test_b_requires_the_review_and_c_forbids_it():
    assert "MUST be done by sc-review" in prompt("lean-b")
    assert "no review step" in prompt("lean-c") and "sc-review" not in prompt("lean-c")


@pytest.mark.parametrize("arm_id", arms.LEAN_ARM_IDS)
def test_checkin_text_names_the_intent_record(arm_id):
    text = prompt(arm_id, checkin=True)
    assert "Owner check-in (once)" in text and "/workspace/trial/work/intent.json" in text


def record(kind, rid, data, seq):
    return {"kind": kind, "id": rid, "data": data, "sequence": seq, "created_at": 0}


def test_metrics_count_denials_findings_and_dispositions():
    context = [
        record("intent", "intent-1", {"hard": False, "criteria": [{"id": "c1"}, {"id": "c2"}]}, 1),
        record("verification", "v1", {}, 2),
        record("review", "r1", {"fresh_context": True, "findings": [
            {"id": "f1", "question": "principles"}, {"id": "f2", "question": "scope"}, {"id": "f3", "question": "docs"}]}, 3),
        record("review", "r2", {"fresh_context": True, "findings": []}, 4),
        record("finding-disposition", "d1", {"review": "r1", "finding": "f1", "disposition": "fixed"}, 5),
        record("finding-disposition", "d2", {"review": "r1", "finding": "f2", "disposition": "rejected"}, 6),
    ]
    history = [{"action": {"kind": "transition", "outcome": {"outcome": "committed"}}},
               {"action": {"kind": "transition", "outcome": {"outcome": "denied", "feedback": {"code": "lean-change-x"}}}},
               {"action": {"kind": "context_appended"}}]
    metrics = lean_metrics({"context": context, "lifecycle": "final", "current_state": "end", "initial_input": {"review": True}}, history)
    assert (metrics["transitions"], metrics["denials"], metrics["denials_by_code"]) == (2, 1, {"lean-change-x": 1})
    assert (metrics["criteria"], metrics["hard"], metrics["verifications"], metrics["reviews_appended"]) == (2, False, 1, 2)
    assert (metrics["review_findings"], metrics["principles_findings"]) == (3, 1)
    assert (metrics["findings_fixed"], metrics["findings_rejected"], metrics["findings_undisposed"]) == (1, 1, 1)


def test_metrics_without_a_review_have_no_findings():
    metrics = lean_metrics({"context": [], "lifecycle": "final", "current_state": "end", "initial_input": {"review": False}}, [])
    assert metrics["review_enabled"] is False and metrics["review_findings"] == 0 and metrics["reviews_appended"] == 0
