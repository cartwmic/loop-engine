"""T4 public CLI + real Pi + real Docker/Inspect bridge, scripted models only."""
import asyncio
import json
import os
from pathlib import Path
import subprocess
import tomllib
import tarfile
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


def campaign(settings_name, driver=None, extra_env=None, **overrides):
    CAPTURES.mkdir(parents=True, exist_ok=True)
    settings = tomllib.loads((ROOT / "campaigns" / settings_name).read_text())
    if driver:
        settings["target_options"]["scripted_driver"] = driver
    for key, value in overrides.items():
        if key == "target_options":
            settings["target_options"] = value
        else:
            settings[key] = value
    name = "t4-" + uuid.uuid4().hex
    settings["campaign"] = name
    config = CAPTURES / (name + ".json")
    config.write_text(json.dumps(settings))
    cmd = ["uv", "run", "--project", "evals", "evalctl", "campaign", "--settings", str(config)]
    result = subprocess.run(cmd, cwd=REPO, env={**os.environ, **(extra_env or {})}, text=True, capture_output=True, timeout=650)
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


def test_real_settings_record_owner_selection_without_any_generation():
    data = tomllib.loads((ROOT / "campaigns/real-example.toml").read_text())
    assert data["models"] == ["codex/gpt-6.1-sol"]
    assert data["simulator_model"] == data["judge_model"] == "openrouter/openai/gpt-6-luna"
    assert data["model_args"][data["judge_model"]] == {"strict_tools": False}
    assert data["prices"][data["judge_model"]]["cache_read_per_million"] == 0.01
    # Selection is owner-authorized; tests must not construct credentialed models
    # or launch the real campaign to prove this settings contract.


def test_driver_stops_before_trial_limit_so_the_outcome_is_recorded():
    # A driver that never finishes is stopped DRIVER_LIMIT_MARGIN_SECONDS before
    # the trial limit (minimum 60 s), so the trial is scored, not lost.
    summary, markdown = campaign("scripted-example.toml", hard_limit_seconds=120,
                                 target_options={"driver_command": ["sh", "-c", "sleep 1000", "sh"]})
    trial = summary["trials"][0]
    assert (trial["outcome"], trial["reason"]) == ("limit", "trial_time") and trial["terminal"] is False
    metrics = trial["target_metrics"]
    assert metrics.get("driver_timed_out") is True, (trial["reason"], metrics)
    assert metrics["run_health"]["timeouts"] == 1
    assert metrics["run_health"]["timeout_kind"] == "hard"
    assert "driver_timed_out" in markdown
    assert_nonterminal_capture(summary, trial)


def assert_nonterminal_capture(summary, trial):
    metrics = trial["target_metrics"]
    capture = Path(summary["capture_run"]) / trial["sample_id"] / "workspace.tar"
    assert capture.is_file()
    with tarfile.open(capture) as archive:
        names = [name.removeprefix("./") for name in archive.getnames()]
        assert "trial/show.json" in names and "trial/history.json" in names
        if metrics.get("lifecycle"):
            assert "trial/run.sqlite" in names
            assert "trial/artifacts/intent.json" in names
    assert all(row["phase"] == "before_driver" for row in metrics["copy_log"])
    assert metrics["copy_out_log"][0]["phase"] == "after_driver"


def test_idle_real_pi_driver_is_stopped_and_captured():
    summary, _ = campaign("scripted-example.toml", idle_limit_seconds=4,
        target_options={"scripted_driver": "asks-and-builds", "driver_command": [
            "sh", "-c", 'export SC_SCRIPTED_HANG=1; exec sc-driver "$@"', "sh"]})
    trial = summary["trials"][0]
    assert trial["outcome"] == "failure" and trial["terminal"] is False
    assert trial["target_metrics"]["driver_timeout_kind"] == "idle"
    assert trial["target_metrics"]["run_health"]["timeout_kind"] == "idle"
    assert trial["target_metrics"]["run_health"]["timeouts"] == 1
    assert trial["usage_by_role"]["role-driver"]["total_tokens"] > 0
    assert_nonterminal_capture(summary, trial)


def test_real_pi_engine_progress_outlives_idle_limit():
    summary, _ = campaign("scripted-example.toml", idle_limit_seconds=10,
        target_options={"scripted_driver": "asks-and-builds", "driver_command": [
            "sh", "-c", 'export SC_SCRIPTED_STEP_DELAY=2; exec sc-driver "$@"', "sh"]})
    trial = summary["trials"][0]
    assert trial["wall_seconds"] > 10
    assert_lifecycle(trial, 3)
    assert trial["outcome"] == "success"
    assert not trial["target_metrics"].get("driver_timed_out")


def test_real_pi_role_token_progress_outlives_idle_limit():
    summary, _ = campaign("scripted-example.toml", idle_limit_seconds=6,
        target_options={"scripted_driver": "asks-and-builds", "driver_command": [
            "sh", "-c", 'for i in 1 2 3 4 5 6; do sc-review "Return a routing marker."; sleep 2; done; exec sc-driver "$@"', "sh"]})
    trial = summary["trials"][0]
    assert trial["wall_seconds"] > 6
    assert_lifecycle(trial, 3)
    assert trial["outcome"] == "success"
    assert not trial["target_metrics"].get("driver_timed_out")


def test_real_pi_nonterminal_driver_exit_retains_evidence():
    summary, _ = campaign("scripted-example.toml",
        target_options={"scripted_driver": "asks-and-builds", "driver_command": [
            "sh", "-c", 'export SC_SCRIPTED_EXIT_AFTER_START=1; exec sc-driver "$@"', "sh"]})
    trial = summary["trials"][0]
    assert trial["outcome"] == "failure" and trial["terminal"] is False
    assert trial["target_metrics"]["driver_exit_code"] == 0
    assert not trial["target_metrics"].get("driver_timed_out")
    assert_nonterminal_capture(summary, trial)


def test_real_pi_hang_after_terminal_is_still_a_failure():
    summary, _ = campaign("scripted-example.toml", idle_limit_seconds=10,
        target_options={"scripted_driver": "asks-and-builds", "driver_command": [
            "sh", "-c", 'sc-driver "$@"; sleep 1000', "sh"]})
    trial = summary["trials"][0]
    assert trial["terminal"] is True and trial["outcome"] == "failure"
    assert trial["target_metrics"]["hidden_test_outcome"]["passed"] is True
    assert trial["target_metrics"]["run_health"]["timeout_kind"] == "idle"
    assert trial["target_metrics"]["run_health"]["unattended_finish"] is False


def test_judge_failure_does_not_fail_the_trial():
    summary, _ = campaign("scripted-example.toml", "asks-and-builds", judge_model="sc-scripted/broken-judge")
    trial = summary["trials"][0]
    assert trial["outcome"] == "success", trial
    assert trial["target_metrics"]["intent_judge"] == {"status": "judge_error", "model": "sc-scripted/broken-judge"}


def test_capture_is_read_in_parts():
    # Real captures exceed Inspect's 100 MiB single-read limit; tiny parts prove reassembly.
    summary, _ = campaign("scripted-example.toml", "asks-and-builds",
                          target_options={"scripted_driver": "asks-and-builds", "capture_chunk_bytes": 65536})
    trial = summary["trials"][0]
    assert trial["outcome"] == "success", trial


def test_progress_reading_tolerates_non_json():
    from types import SimpleNamespace
    from scevals.target import SoftwareChangeTarget

    class Env:
        async def exec(self, *args, **kwargs):
            return SimpleNamespace(success=True, stdout="ready" if args[0][0] == "python3" else "")

        async def read_file(self, path):
            return "db busy\n{"

    assert asyncio.run(SoftwareChangeTarget().progress_state(Env(), "eval-trial")) is None


def test_progress_does_not_initialize_a_database_before_driver_start():
    from types import SimpleNamespace
    from scevals.target import SoftwareChangeTarget

    calls = []
    class Env:
        async def exec(self, command, **kwargs):
            calls.append(command)
            return SimpleNamespace(success=True, stdout="")
        async def read_file(self, path):
            pytest.fail("engine views must not run before a committed run exists")
    assert asyncio.run(SoftwareChangeTarget().progress_state(Env(), "eval-trial")) is None
    assert len(calls) == 1 and calls[0][0] == "python3"
    assert "mode=ro" in calls[0][2]


def test_dropped_provider_connection_is_infrastructure(monkeypatch):
    from types import SimpleNamespace
    import inspect_ai.log
    from scevals.target import provider_connection_failed

    def events(*errors):
        ModelEvent = type("ModelEvent", (), {})
        out = []
        for role, error in errors:
            e = ModelEvent(); e.role = role; e.error = error; out.append(e)
        return SimpleNamespace(events=out)

    monkeypatch.setattr(inspect_ai.log, "transcript", lambda: events(("role-driver", "RemoteProtocolError('peer closed')")))
    assert provider_connection_failed()
    monkeypatch.setattr(inspect_ai.log, "transcript", lambda: events(("role-driver", "RemoteProtocolError('x')"), ("role-driver", None)))
    assert not provider_connection_failed()
    monkeypatch.setattr(inspect_ai.log, "transcript", lambda: events(("simulator", "RemoteProtocolError('x')")))
    assert not provider_connection_failed()


def test_engine_output_larger_than_exec_limit_is_read_from_files():
    # Real runs' full views exceed Inspect's exec-output limit; shrink the limit
    # so even the scripted run exceeds it, and the trial must still be scored.
    summary, _ = campaign("scripted-example.toml", "asks-and-builds",
                          extra_env={"INSPECT_SANDBOX_MAX_EXEC_OUTPUT_SIZE": "20000"})
    trial = summary["trials"][0]
    assert trial["outcome"] == "success", trial
    assert any(item["kind"] == "public" and item["source"] == "public/" for item in trial["target_metrics"]["copy_log"])
