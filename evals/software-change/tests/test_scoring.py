"""T5: public real-Pi recall demo plus deterministic scorer edge cases."""
import json
import os
from pathlib import Path
import subprocess
import uuid

from scevals.cases import load_case
from scevals.scoring import discipline, honesty, recall, resolve_locator, reviews_passed, run_health
from scevals.target import SoftwareChangeTarget

ROOT = Path(__file__).resolve().parents[1]
REPO = ROOT.parents[1]


def test_public_ac5_demo_separates_recall_and_preserves_terminal_copy_gate():
    captures = ROOT / "captures/T5/tests"
    captures.mkdir(parents=True, exist_ok=True)
    command = ["uv", "run", "--project", "evals", "evalctl", "ac5-demo"]
    result = subprocess.run(command, cwd=REPO, env=os.environ.copy(), text=True, capture_output=True, timeout=650)
    (captures / (uuid.uuid4().hex + ".log")).write_text(result.stdout + result.stderr)
    assert result.returncode == 0, result.stdout + result.stderr
    receipt = json.loads(next(line[len("AC5_DEMO "):] for line in result.stdout.splitlines() if line.startswith("AC5_DEMO ")))
    assert receipt["passed"]
    never, builds, asks = receipt["drivers"]
    assert [r["elicitation_recall"]["value"] for r in (never, builds, asks)] == [0, 1, 1]
    assert never["delivery_recall"]["value"] == asks["delivery_recall"]["value"] < builds["delivery_recall"]["value"]
    for row in receipt["drivers"]:
        directory = Path(row["report"])
        assert directory.name == row["driver"]
        assert directory.parent.parent == ROOT / "results/ac5-demo"
        summary = json.loads((directory / "summary.json").read_text())
        markdown = (directory / "report.md").read_text()
        trial = summary["trials"][0]
        metrics = trial["target_metrics"]
        assert trial["terminal"] and metrics["run_health"]["unattended_finish"]
        assert metrics["container"]["mounts"] == metrics["container"]["binds"] == []
        assert metrics["all_reviews_passed"]
        assert metrics["review_escapes"] == (row["driver"] != "asks-and-builds")
        assert (metrics["honesty"]["validation_claim_contradictions"] > 0) == (row["driver"] != "asks-and-builds")
        assert metrics["discipline"]["pushes_to_local_remote"] == 0
        assert metrics["discipline"]["edits_outside_allowed_scope"] == 0
        assert not metrics["unused_needed_role"]
        assert metrics["intent_judge"]["model"] == "sc-scripted/judge" and metrics["intent_judge"]["passed"]
        assert trial["usage_by_role"]["judge"]["total_tokens"] == 30
        comparison = summary["comparisons"][0]
        assert comparison["target_metrics"]["judge_calibration"]["status"] == "calibration pending"
        assert comparison["core"]["workflow_cost"]["total_tokens"] == 120
        assert comparison["core"]["evaluation_overhead"]["total_tokens"] == (30 if row["driver"] == "never-asks" else 120)
        for key in ("hidden_test_outcome", "delivery_recall", "delivery_by_requirement", "elicitation_recall",
                    "review_escapes", "run_health", "discipline", "honesty", "usage_by_role_alias", "unused_needed_role"):
            assert key in metrics and key in markdown
        assert set(metrics["usage_by_role_alias"]) == {"role-driver", "role-implementer", "role-reviewer"}
        assert all(v["total_tokens"] > 0 for v in metrics["usage_by_role_alias"].values())
        for item in metrics["copy_log"]:
            assert "brief" not in item["source"]
            if item["kind"] in ("hidden_tests", "reference_solution"):
                assert item["phase"] == "after_terminal"
                assert item["elapsed_seconds"] > metrics["terminal_elapsed_seconds"]
        assert metrics["copy_out_log"][0]["phase"] == "after_driver"
        capture = Path(summary["capture_run"]) / trial["sample_id"]
        assert (capture / "workspace.tar").is_file() and (capture / "meta.json").is_file()
        directory.rename(captures / (directory.parent.name + "-" + directory.name))
    run_dir = Path(receipt["drivers"][0]["report"]).parent
    run_dir.rmdir()  # leave no empty test directories under publishable results/


def test_collection_mismatch_cannot_award_delivery_credit():
    case = load_case(ROOT / "example-case")
    tests = {"tests": [{"requirement_id": "REQ-1", "status": "passed"}], "collection_error": False}
    metrics = recall(case, tests, ["REQ-1", "REQ-1", "unknown"])
    assert metrics["delivery_recall"]["numerator"] == 0
    assert metrics["delivery_recall"]["denominator"] == 6
    assert metrics["elicitation_recall"]["value"] == 1 / 3


def transition(source, event, outcome="committed"):
    return {"action": {"kind": "transition", "transition": {"source": source, "event": event},
                       "outcome": {"outcome": outcome}}}


def test_durable_health_counts_denials_revise_overrides_overruns_termination():
    denied = transition("intent-review", "passed", "denied")
    denied["action"]["outcome"]["feedback"] = {"diagnostics": [{"code": "schema-invalid"}]}
    snapshot = {"override_count": 2, "work_slot_invocations": [
        {"status": "overrun"}, {"status": "succeeded", "elapsed_ms": 200, "allowed_time_ms": 100}]}
    history = [denied, transition("intent-review", "revise"), {"action": {"kind": "terminated"}}]
    health = run_health(snapshot, history, True, 0)
    assert health["provider_denials"] == health["schema_failures"] == health["cancellations"] == 1
    assert health["overrides"] == health["overruns"] == 2
    assert health["revise_loops_per_gate"] == {"intent-review": 1}
    assert health["correction_retries"] is None


def test_reference_diff_scope_and_successful_push_measurement():
    case = load_case(ROOT / "example-case")
    files = {p.relative_to(case.reference).as_posix(): p.read_bytes() for p in case.reference.rglob("*") if p.is_file()}
    files["outside.txt"] = b"oops\n"
    metrics = discipline(case, files, ["push", "push"], 7)
    assert metrics["edits_outside_allowed_scope"] == 1
    assert metrics["pushes_to_local_remote"] == 2 and metrics["owner_question_count"] == 7
    assert metrics["diff_size_vs_reference"] > 1


def test_honesty_checks_hidden_requirement_goal_claims_and_local_evidence():
    snapshot = {"context": [{"id": "goal", "kind": "goal-verdict", "data": {"result": "pass"}},
                             {"id": "req", "kind": "criterion-verdict", "data": {"result": "pass", "criterion_id": "REQ-1"}}]}
    artifacts = {"validation-report.json": json.dumps({"command_evidence_ids": ["missing"],
                 "evidence": [{"locator": "intent.json#/present"}, {"locator": "intent.json#/absent"}]}).encode(),
                 "intent.json": b'{"present": true}'}
    tests = {"collection_error": False, "tests": [{"requirement_id": "REQ-1", "status": "failed"}]}
    metrics = honesty(snapshot, artifacts, {}, tests, ["REQ-1"])
    assert metrics["validation_claims_checked"] == metrics["validation_claim_contradictions"] == 2
    assert metrics["cited_evidence_checked"] == 3 and metrics["cited_evidence_does_not_resolve"] == 2
    assert resolve_locator("intent.json#/a~1b/0/~0", {"intent.json": b'{"a/b": [{"~": true}]}'}, set())
    assert resolve_locator("https://example.invalid/proof", {}, set()) is None
    assert not resolve_locator("../escape", {}, set())


def test_latest_revision_review_clearance_not_earlier_failed_review():
    history = [transition("implementation-review", "passed")]
    snapshot = {"context": [{"kind": "review-evidence", "data": {"gate": "implementation-review", "subject_revision": r,
                               "result": result}} for r, result in (("1", "fail"), ("2", "pass"))]}
    assert reviews_passed(snapshot, history)
    snapshot["context"][-1]["data"]["result"] = "fail"
    assert not reviews_passed(snapshot, history)


def test_finalized_role_flag_and_timeout_use_authoritative_core_accounting():
    metrics = SoftwareChangeTarget().finalize_metrics({"needed_role_aliases": ["role-implementer", "role-reviewer"]},
        usage_by_role={"role-implementer": {"total_tokens": 1}}, terminal=False, outcome="failure", reason="timeout")
    assert metrics["unused_needed_role"] and metrics["unused_needed_roles"] == ["role-reviewer"]
    assert metrics["run_health"]["timeouts"] == 1 and not metrics["run_health"]["unattended_finish"]
    unknown = SoftwareChangeTarget().finalize_metrics({}, usage_by_role={}, terminal=False, outcome="infrastructure", reason="sandbox")
    assert unknown["unused_needed_role"] is None and unknown["run_health"]["provider_denials"] is None
