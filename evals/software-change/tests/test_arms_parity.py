"""Scoring parity through the public campaign path (evalctl campaign), scripted drivers only, no model and no network.

Every case is one real single-trial campaign in the real trial container. The arm's driver_command is
scevals/scripted_arm_driver.py (its text is passed to `python3 -c`), or the shipped Loop Engine driver for the
completed case. The drivers vary only what the spec names: a Loop Engine run left terminated, active or crashed,
a wrong delivery, an idle stop and a hard stop. Assertions read the exported campaign outcome.

Mechanics proof only: nothing here is a model's behaviour or a real workflow run. The retained output is
tests/proofs/arms-parity.txt, with the campaign reports and logs under tests/proofs/campaigns/parity-*/.
"""
from concurrent.futures import ThreadPoolExecutor
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import tomllib
import uuid

import pytest

from scevals import scripted_driver

REPO = Path(__file__).resolve().parents[3]
ROOT = REPO / "evals/software-change"
PROOFS = ROOT / "tests/proofs"
PARITY_OUTPUT = PROOFS / "arms-parity.txt"
DRIVER_SOURCE = (ROOT / "scevals/scripted_arm_driver.py").read_text()
# Fields that describe the delivered code and its completion. Elicitation and owner counts are left out on purpose:
# they follow the driver's own owner questions (the shipped driver asks for all three requirements, the scripted one asks once).
DISCIPLINE_FIELDS = ("diff_size", "diff_size_vs_reference", "edits_outside_allowed_scope", "artifact_volume")


def arm(arm_id, *driver_options):
    """An arm descriptor whose driver is the scripted arm driver, or the shipped Loop Engine driver with no options."""
    command = ["python3", "-c", DRIVER_SOURCE, *driver_options] if driver_options else ["sc-driver"]
    return {"id": arm_id, "driver_command": command, "prompt_template": arm_id,
            "workflow_owned_paths": [], "completion_rule": "driver-exit"}


# case id -> (target options, campaign overrides, expected outcome, expected reason)
CASES = {
    "native-complete": ({"arm": arm("native", "--code", "correct")}, {}, "success", "passed"),
    "loop-engine-terminated": ({"arm": arm("loop-engine", "--engine", "terminated", "--code", "correct")}, {},
                               "success", "passed"),
    "loop-engine-active": ({"arm": arm("loop-engine", "--engine", "active", "--code", "correct")}, {},
                           "success", "passed"),
    "loop-engine-crashed": ({"arm": arm("loop-engine", "--engine", "crashed", "--code", "correct")}, {},
                            "success", "passed"),
    "loop-engine-completed": ({"arm": arm("loop-engine"), "scripted_driver": "asks-and-builds"}, {},
                              "success", "passed"),
    "native-wrong": ({"arm": arm("native", "--code", "wrong")}, {}, "failure", "target_failure"),
    "loop-engine-terminated-wrong": ({"arm": arm("loop-engine", "--engine", "terminated", "--code", "wrong")}, {},
                                     "failure", "target_failure"),
    "native-idle": ({"arm": arm("native", "--code", "correct", "--end", "hang")}, {"idle_limit_seconds": 4},
                    "failure", "target_failure"),
    "loop-engine-idle": ({"arm": arm("loop-engine", "--engine", "active", "--end", "hang")},
                         {"idle_limit_seconds": 4}, "failure", "target_failure"),
    "native-hard": ({"arm": arm("native", "--code", "correct", "--end", "hang")}, {"hard_limit_seconds": 120},
                    "limit", "trial_time"),
}
# Loop Engine lifecycle the engine must report for each correct-delivery case.
ENGINE_STATE = {"loop-engine-terminated": "terminated", "loop-engine-active": "active",
                "loop-engine-crashed": "active", "loop-engine-completed": "final"}

# Every case's campaign starts at its first request; each test reads its own case's future. At most four run at once:
# each sandbox takes a Docker network from a small predefined pool that the other proof units share.
_CAMPAIGNS = {}
_POOL = ThreadPoolExecutor(max_workers=4)


def run_campaign(label, target_options, **overrides):
    """One single-trial campaign through `evalctl campaign`; the report and log are retained under tests/proofs."""
    settings = tomllib.loads((ROOT / "campaigns/scripted-example.toml").read_text())
    settings["target_options"] = target_options
    settings.update(overrides)
    name = f"t10-{label}-{uuid.uuid4().hex[:8]}"
    settings["campaign"] = name
    with tempfile.TemporaryDirectory(prefix="t10-parity-") as scratch:
        config = Path(scratch) / (name + ".json")
        config.write_text(json.dumps(settings))
        result = subprocess.run(["uv", "run", "--project", "evals", "evalctl", "campaign", "--settings", str(config)],
                                cwd=REPO, text=True, capture_output=True, timeout=650)
    assert result.returncode == 0, result.stdout + result.stderr
    result_dir = Path(result.stdout.split("Report written: ", 1)[1].strip())
    assert result_dir.parent == ROOT / "results" / name
    summary = json.loads((result_dir / "summary.json").read_text())
    # The report is a local receipt under tests/proofs, never an extra published result.
    retained = PROOFS / "campaigns" / label
    shutil.rmtree(retained, ignore_errors=True)
    retained.parent.mkdir(parents=True, exist_ok=True)
    result_dir.rename(retained)
    result_dir.parent.rmdir()
    (retained / "evalctl.log").write_text(result.stdout + result.stderr)
    assert len(summary["trials"]) == 1, summary
    return summary["trials"][0]


def trial_for(case_id):
    """Each case runs one campaign per session; every assertion reads that same exported trial."""
    if not _CAMPAIGNS:
        for case, (options, overrides, _, _) in CASES.items():
            _CAMPAIGNS[case] = _POOL.submit(run_campaign, f"parity-{case}", options, **overrides)
    return _CAMPAIGNS[case_id].result()


def score_view(trial):
    metrics = trial["target_metrics"]
    return {"complete": metrics["complete"], "incomplete_reason": metrics["incomplete_reason"],
            "hidden_test_outcome": metrics["hidden_test_outcome"], "delivery_recall": metrics["delivery_recall"],
            "discipline": {field: metrics["discipline"][field] for field in DISCIPLINE_FIELDS}}


@pytest.mark.parametrize("case_id", list(CASES))
def test_campaign_exports_the_expected_outcome(case_id):
    _, _, expected_outcome, expected_reason = CASES[case_id]
    trial = trial_for(case_id)
    assert (trial["outcome"], trial["reason"]) == (expected_outcome, expected_reason), trial["target_metrics"]
    assert trial["target_metrics"]["owner_interaction"]["question_words"] > 0  # the owner was asked through ask-owner


@pytest.mark.parametrize("case_id", list(ENGINE_STATE))
def test_loop_engine_states_with_a_correct_driver_score_like_the_native_arm(case_id):
    native = trial_for("native-complete")
    trial = trial_for(case_id)
    assert trial["outcome"] == "success" and trial["terminal"] is True, trial["target_metrics"]
    assert score_view(trial) == score_view(native), case_id
    assert trial["target_metrics"]["diagnostics"]["engine_lifecycle"] == ENGINE_STATE[case_id]


def test_crashed_driver_is_complete_and_its_exit_code_is_a_diagnostic_only():
    trial = trial_for("loop-engine-crashed")
    assert trial["target_metrics"]["driver_exit_code"] == 137
    assert trial["target_metrics"]["complete"] is True and trial["outcome"] == "success"


@pytest.mark.parametrize("case_id,expected", [("native-idle", "idle"), ("loop-engine-idle", "idle"),
                                              ("native-hard", "hard")])
def test_incomplete_driver_is_not_a_success(case_id, expected):
    trial = trial_for(case_id)
    assert trial["outcome"] != "success" and trial["terminal"] is False
    assert trial["target_metrics"]["complete"] is False
    assert trial["target_metrics"]["incomplete_reason"] == expected


@pytest.mark.parametrize("case_id", ["native-wrong", "loop-engine-terminated-wrong"])
def test_failing_hidden_test_is_not_a_success_whatever_the_engine_state(case_id):
    trial = trial_for(case_id)
    assert trial["outcome"] == "failure" and trial["terminal"] is True
    assert trial["target_metrics"]["hidden_test_outcome"]["passed"] is False


def test_wrong_delivery_scores_the_same_under_both_arms():
    native = trial_for("native-wrong")
    loop = trial_for("loop-engine-terminated-wrong")
    assert score_view(native) == score_view(loop)


def test_retained_parity_output_states_that_it_proves_mechanics_only():
    lines = []
    for case_id in CASES:
        trial = trial_for(case_id)
        metrics = trial["target_metrics"]
        lines.append({"case": case_id, "outcome": trial["outcome"], "reason": trial["reason"],
                      "terminal": trial["terminal"], "complete": metrics.get("complete"),
                      "incomplete_reason": metrics.get("incomplete_reason"),
                      "driver_exit_code": metrics.get("driver_exit_code"),
                      "engine_lifecycle": metrics.get("diagnostics", {}).get("engine_lifecycle"),
                      "hidden_tests_passed": metrics.get("hidden_test_outcome", {}).get("passed"),
                      "owner_question_words": metrics.get("owner_interaction", {}).get("question_words"),
                      "retained": f"tests/proofs/campaigns/parity-{case_id}"})
    PARITY_OUTPUT.parent.mkdir(parents=True, exist_ok=True)
    PARITY_OUTPUT.write_text(
        "Arms parity proof: scripted drivers through evalctl campaign in the real trial container.\n"
        "No model and no network were used. These results prove scoring and completion mechanics only; they are not\n"
        "real-model, workflow-behaviour or paid evidence. 'crashed' is a driver exit code 137 while the Loop Engine\n"
        "run stays active (the engine has no crashed lifecycle). loop-engine-completed runs the shipped scripted driver,\n"
        "whose baseline step skips work when the harness already made HEAD (scripted_driver.prepare_baseline).\n"
        "Exported outcome per case:\n"
        + "\n".join(json.dumps(line, sort_keys=True) for line in lines) + "\n")


def test_scripted_driver_without_a_harness_head_runs_the_original_baseline_sequence(tmp_path, monkeypatch):
    """Non-arm trials: no HEAD yet, so the original git sequence and the bare remote run in their original order."""
    calls = []
    monkeypatch.setattr(scripted_driver, "command", lambda argv: calls.append(argv) or "")
    repository = tmp_path / "repo"
    repository.mkdir()
    scripted_driver.prepare_baseline(repository)
    assert calls == [
        ["git", "-C", repository, "init", "-q"],
        ["git", "-C", repository, "config", "user.name", "Scripted eval driver"],
        ["git", "-C", repository, "config", "user.email", "eval@example.invalid"],
        ["git", "-C", repository, "add", "."],
        ["git", "-C", repository, "commit", "-qm", "Public example baseline"],
        ["git", "init", "--bare", "-q", "/workspace/local-remote.git"],
        ["git", "-C", repository, "remote", "add", "origin", "/workspace/local-remote.git"],
    ]


def test_scripted_driver_with_a_harness_head_keeps_the_harness_baseline_and_remote(tmp_path, monkeypatch):
    """Arm trials: the harness already committed the baseline and added origin, so the driver creates nothing."""
    calls = []
    monkeypatch.setattr(scripted_driver, "command", lambda argv: calls.append(argv) or "")
    repository = tmp_path / "repo"
    repository.mkdir()
    subprocess.run(["git", "-C", str(repository), "init", "-q"], check=True)
    subprocess.run(["git", "-C", str(repository), "-c", "user.name=h", "-c", "user.email=h@invalid",
                    "commit", "-q", "--allow-empty", "-m", "Starting baseline"], check=True)
    scripted_driver.prepare_baseline(repository)
    assert calls == []
