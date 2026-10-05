"""T4 public CLI + real Pi + real Docker/Inspect bridge, scripted models only."""
import asyncio
import json
import os
from pathlib import Path
import subprocess
import tomllib
import uuid

import pytest
from inspect_ai.model import get_model

from evalcore.target import ModelError
from scevals.cases import load_case
from scevals.owner import SimulatedOwner
from scevals.target import TerminalTrial

REPO = Path(__file__).resolve().parents[3]
ROOT = REPO / "evals/software-change"
CAPTURES = ROOT / "captures/T4/tests"


def campaign(settings_name, driver=None):
    CAPTURES.mkdir(parents=True, exist_ok=True)
    settings = tomllib.loads((ROOT / "campaigns" / settings_name).read_text())
    if driver:
        settings["target_options"]["scripted_driver"] = driver
    name = "t4-" + uuid.uuid4().hex
    settings["campaign"] = name
    config = CAPTURES / (name + ".json")
    config.write_text(json.dumps(settings))
    cmd = ["uv", "run", "--project", "evals", "evalctl", "campaign", "--settings", str(config)]
    result = subprocess.run(cmd, cwd=REPO, env=os.environ.copy(), text=True, capture_output=True, timeout=650)
    (CAPTURES / (name + ".log")).write_text(result.stdout + result.stderr)
    assert result.returncode == 0, result.stdout + result.stderr
    result_dir = Path(result.stdout.split("Report written: ", 1)[1].strip())
    assert result_dir.parent == ROOT / "results" / name
    summary = json.loads((result_dir / "summary.json").read_text())
    markdown = (result_dir / "report.md").read_text()
    # Keep test-generated reports as local receipts, not publishable extra results.
    result_dir.rename(CAPTURES / name)
    result_dir.parent.rmdir()
    return summary, markdown


def assert_lifecycle(trial, expected_answers):
    assert trial["terminal"] is True
    evidence = trial["target_metrics"]
    assert evidence["terminal_state"] == "end"
    assert evidence["lifecycle"] == "final"
    assert evidence["container"]["mounts"] == evidence["container"]["binds"] == []
    assert evidence["simulated_owner"]["answer_count"] == expected_answers
    for copy in evidence["copy_log"]:
        assert "brief" not in copy["source"]
        if copy["source"].startswith("hidden/"):
            assert copy["phase"] == "after_terminal"
            assert copy["elapsed_seconds"] > evidence["terminal_elapsed_seconds"]
        else:
            assert copy["source"].startswith(("public/", "harness/"))
    assert all(trial["usage_by_role"][role]["total_tokens"] > 0
               for role in ("role-driver", "role-implementer", "role-reviewer"))


def test_real_pi_role_routing_swaps_models_in_reports():
    summary, markdown = campaign("role-routing.toml")
    assert len(summary["trials"]) == 2
    assert len({t["target_metrics"]["container"]["id"] for t in summary["trials"]}) == 2
    for trial, implementer, reviewer in zip(summary["trials"], ("model-a", "model-b"), ("model-b", "model-a")):
        assert_lifecycle(trial, 3)
        assert trial["outcome"] == "success"
        assert trial["target_metrics"]["role_observations"] == {
            "role-driver": "routing-marker-driver",
            "role-implementer": "routing-marker-" + implementer,
            "role-reviewer": "routing-marker-" + reviewer}
        assert len(trial["target_metrics"]["hidden_tests"]["tests"]) == 6
        assert trial["usage_by_role"]["simulator"]["total_tokens"] == 90
    assert summary["comparisons"][0]["core"]["evaluation_overhead"]["total_tokens"] == 120  # owner 90 + judge 30
    assignments = summary["matched_assignment"]
    assert {(a["case_id"], a["case_version"], a["trial"]) for a in assignments} == {("label-keys", "1", 1)}
    assert "routing-marker-model-a" in markdown and "routing-marker-model-b" in markdown


@pytest.mark.parametrize("driver,answers", [("never-asks", 0), ("asks-but-does-not-build", 3)])
def test_controlled_drivers_reach_terminal_without_building(driver, answers):
    summary, _ = campaign("scripted-example.toml", driver)
    trial = summary["trials"][0]
    assert_lifecycle(trial, answers)
    assert trial["outcome"] == "failure"
    assert any(t["status"] == "failed" for t in trial["target_metrics"]["hidden_tests"]["tests"])


def test_owner_reveals_only_asked_requirement_and_keeps_text_host_side():
    case = load_case(ROOT / "example-case")
    owner = SimulatedOwner(case, "sc-scripted/simulator")
    answer = asyncio.run(owner.ask_owner("What should happen with empty labels?"))
    assert "inbox" in answer and "REQ-" not in answer and "casefold" not in answer
    assert owner.receipt()["revealed_requirement_ids"] == ["REQ-3"]
    assert "question" not in json.dumps(owner.receipt())
    assert "answer\"" not in json.dumps(owner.receipt())
    assert owner.answers[0]["answer"] == answer


def test_hidden_copy_requires_terminal():
    trial = TerminalTrial(None, None, None, {}, False, None)
    with pytest.raises(ModelError, match="terminal"):
        asyncio.run(trial.copy_scoring_material())


def test_owner_gate_cli_message_is_clear_without_launch(monkeypatch, capsys):
    from evalcore import cli
    from evalcore.target import OwnerApprovalRequired

    def blocked(args):
        raise OwnerApprovalRequired("private text must not be printed")

    monkeypatch.setattr(cli, "campaign", blocked)
    assert cli.main(["campaign"]) == 2
    error = capsys.readouterr().err
    assert "Owner-pending" in error and "paid-call approval" in error
    assert "private text" not in error


def test_real_settings_require_owner_selection_before_any_generation():
    data = tomllib.loads((ROOT / "campaigns/real-example.toml").read_text())
    for model in (*data["models"], data["simulator_model"], data["judge_model"]):
        with pytest.raises(ValueError, match="Owner-pending.*paid-call approval"):
            get_model(model, memoize=False)
