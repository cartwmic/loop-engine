"""Black-box evalctl campaigns: no mocks of the runner, CLI or report writer."""
import json
import os
import subprocess
import time
from pathlib import Path

import pytest
from inspect_ai.log import read_eval_log

REPO = Path(__file__).resolve().parents[3]


def invoke(root, *args, extra_env=None, timeout=90):
    env = {**os.environ, "EVAL_FAKE_ROOT": str(root),
           "PYTHONPATH": str(Path(__file__).parent), **(extra_env or {})}
    command = ["uv", "run", "--project", "evals", "evalctl", *args]
    result = subprocess.run(command, cwd=REPO, env=env, text=True, capture_output=True, timeout=timeout)
    # Retain actual command outputs; no real credential values are printed or saved.
    (root / "command.txt").write_text(" ".join(command) + f"\nexit={result.returncode}\n" + result.stdout + result.stderr)
    return result


def settings(root, **overrides):
    data = {"target": "fake_target:create_target", "campaign": "test",
            "models": ["fake/a", "fake/b"], "cases_path": str(root / "cases"),
            "simulator_model": "fake/a", "judge_model": "fake/a",
            "prices": {m: {"input_per_million": 2, "output_per_million": 4} for m in ("fake/a", "fake/b")},
            **overrides}
    path = root / "settings.json"
    path.write_text(json.dumps(data))
    return path


def run(root, **overrides):
    path = settings(root, **overrides)
    result = invoke(root, "campaign", "--settings", str(path))
    assert result.returncode == 0, result.stdout + result.stderr
    output = Path(result.stdout.split("Report written: ", 1)[1].strip())
    assert output.parent == root / "results" / "test"
    assert (output / "report.md").is_file()
    return json.loads((output / "summary.json").read_text())


def test_small_matched_campaign(campaign_root):
    summary = run(campaign_root)
    assert summary["settings"]["preset"] == "small"
    assert summary["settings"]["profile"] == "standard-16"
    assert summary["settings"]["setting_sources"]["preset"] == "default"
    assert summary["settings"]["setting_sources"]["models"] == "explicit"
    assert summary["pinned_models"] == {"simulator": "fake/a", "judge": "fake/a"}
    assert summary["versions"]["target"]["loop_engine"] == "v0.26.0"
    assert summary["versions"]["harness"]["inspect_ai"] == "0.3.276"
    assignments = summary["matched_assignment"]
    assert len(assignments) == len(summary["trials"]) == 18
    matched = [[(a["case_id"], a["case_version"], a["trial"]) for a in assignments if a["combination_id"] == model] for model in ("fake/a", "fake/b")]
    assert matched[0] == matched[1]
    for comparison in summary["comparisons"]:
        core = comparison["core"]
        assert core["success_rate"] == {"numerator": 5, "denominator": 8, "rate": 5 / 8}
        assert core["pass_k"] == {"k": 3, "numerator": 1, "denominator": 2, "rate": 0.5}
        assert core["incomplete_cases"] == [{"case_id": "case-1", "scorable_trials": 2, "required_trials": 3}]
        assert core["infrastructure_failure_rate"] == {"numerator": 1, "denominator": 9, "rate": 1 / 9}
        assert core["timeouts"] == core["model_errors"] == 1
        assert core["workflow_cost"]["total_tokens"] == 9 * 3 * 15
        assert core["evaluation_overhead"]["total_tokens"] == 9 * 2 * 15
        assert core["workflow_cost"]["estimated_usd"] == pytest.approx(9 * 3 * 40 / 1_000_000)
        assert core["wall_seconds"] > 0
    report = next((campaign_root / "results/test").glob("*/report.md")).read_text()
    for expected in ("Matched Assignment", "incomplete_cases", "numerator", "denominator", "evaluation_overhead", "timeout"):
        assert expected in report
    assert list((campaign_root / "logs").glob("*.eval"))


def test_no_options_campaign_uses_defaults_and_one_model_in_every_role(campaign_root):
    """AC-3 defaults. Only the required pinned simulator/judge identities and one model are named;
    no preset, profile, counts or per-role options are given."""
    summary = run(campaign_root, models=["fake/a"])
    recorded = summary["settings"]
    assert (recorded["preset"], recorded["effective_case_count"], recorded["effective_trials_per_case"]) == ("small", 3, 3)
    assert recorded["profile"] == "standard-16"
    for name in ("preset", "profile", "case_count", "trials_per_case", "combinations"):
        assert recorded["setting_sources"][name] == "default", name
    assert recorded["setting_sources"]["models"] == "explicit"
    assert recorded["resolved_combinations"] == [{"id": "fake/a", "driver": "fake/a", "implementer": "fake/a", "reviewer": "fake/a"}]
    assert len(summary["trials"]) == 9
    assert {a["combination_id"] for a in summary["matched_assignment"]} == {"fake/a"}
    roles = summary["comparisons"][0]["role_models"]
    workflow_models = {model for role, model in roles.items() if role not in ("simulator", "judge")}
    assert len(roles) - 2 == 3 and workflow_models == {"fake/a"}


@pytest.mark.parametrize("override,count,k", [({"preset": "medium"}, 36, 3), ({"case_count": 2, "trials_per_case": 2}, 8, 2)])
def test_preset_and_explicit_counts(campaign_root, override, count, k):
    summary = run(campaign_root, **override)
    assert len(summary["trials"]) == count
    assert summary["settings"]["effective_trials_per_case"] == k
    for key, value in override.items():
        assert summary["settings"][key] == value
        assert summary["settings"]["setting_sources"][key] == "explicit"


def test_per_role_combination_changes_actual_calls(campaign_root):
    summary = run(campaign_root, models=[], combinations=[
        {"id": "roles", "driver": "fake/a", "implementer": "fake/b", "reviewer": "fake/a"},
        {"id": "swapped", "driver": "fake/a", "implementer": "fake/a", "reviewer": "fake/b"},
    ], profile="high-rigor", case_count=1, trials_per_case=1, target_options={"inject_errors": False})
    assert len(summary["trials"]) == 2
    first, second = summary["trials"]
    assert first["target_metrics"]["seen_roles"]["role-implementer"] == "b"
    assert second["target_metrics"]["seen_roles"]["role-implementer"] == "a"
    assert second["target_metrics"]["seen_roles"]["role-reviewer"] == "b"
    assert first["target_metrics"]["profile_used"] == "high-rigor"
    assert first["target_metrics"]["case_version_used"] == "v1.0"


def test_per_model_generation_settings_reach_every_role_and_report(campaign_root):
    summary = run(campaign_root, models=[], combinations=[
        {"id": "roles", "driver": "fake/a", "implementer": "fake/b", "reviewer": "fake/a"}
    ], simulator_model="fake/b", judge_model="fake/a", case_count=1, trials_per_case=1,
        model_config={"fake/a": {"reasoning_effort": "high"}, "fake/b": {"reasoning_effort": "medium"}},
        target_options={"inject_errors": False})
    seen = summary["trials"][0]["target_metrics"]["seen_roles"]
    assert seen == {"role-driver": "a:high", "role-implementer": "b:medium", "role-reviewer": "a:high",
                    "simulator": "b:medium", "judge": "a:high"}
    assert summary["settings"]["model_config"] == {
        "fake/a": {"reasoning_effort": "high"}, "fake/b": {"reasoning_effort": "medium"}}
    assert summary["settings"]["setting_sources"]["model_config"] == "explicit"


def test_actual_time_limit_is_a_limit_not_a_failure(campaign_root):
    summary = run(campaign_root, models=["fake/a"], case_count=1, trials_per_case=1,
                  hard_limit_seconds=1, target_options={"sleep": True})
    trial = summary["trials"][0]
    assert (trial["outcome"], trial["reason"]) == ("limit", "trial_time")
    core = summary["comparisons"][0]["core"]
    assert core["limits"] == {"trial_time": 1}
    assert core["success_rate"]["denominator"] == 1  # a limit is a scorable non-success


def test_credential_in_scored_field_refuses_all_results(campaign_root):
    path = settings(campaign_root, case_count=1, trials_per_case=1, target_options={"leak": True, "inject_errors": False})
    value = "synthetic-eval-credential-ONLY-for-test"
    result = invoke(campaign_root, "campaign", "--settings", str(path), extra_env={"FAKE_EVAL_API_KEY": value})
    assert result.returncode == 2
    assert "Result refused" in result.stderr
    assert value not in result.stdout + result.stderr
    assert not (campaign_root / "results").exists()
    assert all(value not in read_eval_log(str(log)).model_dump_json()
               for log in (campaign_root / "logs").glob("*.eval"))


def test_subcommand_registration(campaign_root):
    result = invoke(campaign_root, "--plugin", "fake_target", "fake-check")
    assert result.returncode == 0
    assert "fake target registration works" in result.stdout


def test_nonterminal_is_failure_even_when_score_passes(campaign_root):
    summary = run(campaign_root, case_count=1, trials_per_case=1,
                  target_options={"nonterminal": True, "inject_errors": False})
    assert all(r["outcome"] == "failure" and not r["terminal"] for r in summary["trials"])
    assert summary["comparisons"][0]["core"]["success_rate"]["numerator"] == 0


def test_provider_config_from_settings_refuses_all_results(campaign_root):
    config = campaign_root / "providers.toml"
    config.write_text('[providers.synthetic]\nbase_url = "https://synthetic-provider.invalid"\n')
    path = settings(campaign_root, case_count=1, trials_per_case=1,
                    provider_config_paths=[str(config)],
                    target_options={"score_text": config.read_text(), "inject_errors": False})
    result = invoke(campaign_root, "campaign", "--settings", str(path))
    assert result.returncode == 2
    assert "Result refused" in result.stderr
    assert "synthetic-provider.invalid" not in result.stdout + result.stderr
    assert not (campaign_root / "results").exists()


def test_missing_models_and_pinned_ids_do_not_launch(campaign_root):
    result = invoke(campaign_root, "campaign", "--target", "fake_target:create_target", "--cases-path", str(campaign_root))
    assert result.returncode == 2
    assert "simulator_model and judge_model" in result.stderr
    assert not (campaign_root / "logs").exists()


@pytest.mark.parametrize("model,outcome,reason", [
    ("fake/service-error", "infrastructure", "provider_service_error"),
    ("fake/bad-request", "failure", "model_error"),
])
def test_inspect_wrapped_provider_errors(campaign_root, model, outcome, reason):
    summary = run(campaign_root, models=[model], case_count=1, trials_per_case=1,
                  target_options={"inject_errors": False})
    trial = summary["trials"][0]
    assert (trial["outcome"], trial["reason"]) == (outcome, reason)
    core = summary["comparisons"][0]["core"]
    assert core["success_rate"]["denominator"] == int(outcome == "failure")
    assert bool(core["incomplete_cases"]) == (outcome == "infrastructure")


def test_unclassified_error_reports_type_and_keeps_traceback_local(campaign_root):
    summary = run(campaign_root, models=["fake/a"], case_count=1, trials_per_case=1,
                  target_options={"raise_unknown": True})
    trial = summary["trials"][0]
    assert (trial["outcome"], trial["reason"], trial["error_type"]) == ("failure", "unclassified_error", "KeyError")
    tracebacks = list((campaign_root / "logs" / "errors").glob("test-*.txt"))
    assert len(tracebacks) == 1 and "KeyError" in tracebacks[0].read_text()


FAST_GUARD = {"guard_poll_seconds": 0.2, "stop_grace_seconds": 0.5}


def test_campaign_wall_time_stops_the_running_trial_and_starts_no_more(campaign_root):
    """D=14s, S=8s: the first trial is stopped at S, later trials are never started, the command still reports."""
    summary = run(campaign_root, models=["fake/a"], case_count=3, trials_per_case=1, **FAST_GUARD,
                  campaign_wall_limit_seconds=14, post_trial_reserve_seconds=6, publication_reserve_seconds=2,
                  target_options={"inject_errors": False, "wait_seconds": 120})
    first, *rest = summary["trials"]
    assert (first["outcome"], first["reason"], first["limit_step"]) == ("limit", "campaign_wall_time", "trial_work")
    assert [(t["outcome"], t["reason"], t["limit_step"]) for t in rest] == [("limit", "campaign_wall_time", "not_started")] * 2
    assert summary["comparisons"][0]["core"]["limits"] == {"campaign_wall_time": 3}
    assert summary["comparisons"][0]["core"]["infrastructure_failure_rate"]["numerator"] == 0


def test_dollar_stop_fires_at_the_stop_line_and_later_trials_never_start(campaign_root):
    """Cap 5, margin 0.5: the stop line is 4.5. A call costs 2 dollars; spend is polled, so the stop comes shortly after."""
    summary = run(campaign_root, models=[], combinations=[
        {"id": "pricey", "driver": "fake/costly", "implementer": "fake/a", "reviewer": "fake/a"}],
        prices={"fake/costly": {"input_per_million": 2, "output_per_million": 0}, "fake/a": {"input_per_million": 0.001, "output_per_million": 0.001}},
        case_count=2, trials_per_case=1, **FAST_GUARD, campaign_budget_usd=5, dollar_margin_usd=0.5,
        target_options={"inject_errors": False, "extra_driver_calls": 20, "call_gap_seconds": 0.4})
    first, second = summary["trials"]
    assert (first["outcome"], first["reason"]) == ("limit", "campaign_dollars")
    assert (second["outcome"], second["reason"], second["limit_step"]) == ("limit", "campaign_dollars", "not_started")
    spent = first["usage_by_role"]["role-driver"]["input_tokens"] * 2 / 1_000_000
    assert 4.5 <= spent <= 8  # stopped at the line, overshoot by at most about a call or two


def test_tiered_price_charges_each_request_by_its_size_not_the_aggregate_bound(campaign_root):
    """Every fake request is far below the tier threshold, so the base rate applies even though a tier is configured."""
    tiered = {"input_per_million": 2, "output_per_million": 0, "tiers": [
        {"input_tokens_above": 5_000_000, "input_per_million": 4, "output_per_million": 0}]}
    summary = run(campaign_root, models=[], combinations=[
        {"id": "pricey", "driver": "fake/costly", "implementer": "fake/a", "reviewer": "fake/a"}],
        prices={"fake/costly": tiered, "fake/a": {"input_per_million": 0, "output_per_million": 0}},
        case_count=1, trials_per_case=1, campaign_budget_usd=100, dollar_margin_usd=1,
        target_options={"inject_errors": False, "extra_driver_calls": 2, "call_gap_seconds": 0.1})
    trial = summary["trials"][0]
    driver_tokens = trial["usage_by_role"]["role-driver"]["input_tokens"]
    assert driver_tokens >= 3_000_000 and trial["priced_usd_by_role"]["role-driver"] == pytest.approx(driver_tokens * 2 / 1e6)
    actual = summary["budget"]["actual"]["usd"]
    assert actual == pytest.approx(driver_tokens * 2 / 1e6, rel=0.01) and actual < driver_tokens * 4 / 1e6 / 1.5   # not the 4/M bound
    assert summary["budget"]["cost_verdict"] == "pass"


def test_a_per_trial_time_stop_also_stops_the_later_trials(campaign_root):
    summary = run(campaign_root, models=["fake/a"], case_count=2, trials_per_case=1, hard_limit_seconds=1,
                  target_options={"inject_errors": False, "sleep": True})
    first, second = summary["trials"]
    assert (first["outcome"], first["reason"]) == ("limit", "trial_time")
    assert (second["outcome"], second["reason"], second["limit_step"]) == ("limit", "trial_time", "not_started")
    assert [e["reason"] for e in summary["budget"]["stop_events"]] == ["trial_time"]


def test_spend_reached_in_the_trials_last_moments_stops_later_model_calls(campaign_root):
    """The watcher never polls (60 s); the cost lands just before run_trial returns. The look right after it must catch it."""
    summary = run(campaign_root, models=[], combinations=[
        {"id": "pricey", "driver": "fake/costly", "implementer": "fake/a", "reviewer": "fake/a"}],
        prices={"fake/costly": {"input_per_million": 2, "output_per_million": 0}, "fake/a": {"input_per_million": 0, "output_per_million": 0}},
        case_count=2, trials_per_case=1, guard_poll_seconds=60, campaign_budget_usd=5, dollar_margin_usd=0.5,
        target_options={"inject_errors": False, "late_costly_calls": 3})
    first, second = summary["trials"]
    assert first["target_metrics"]["judge_model_calls_allowed"] is False    # score() sees the stop, so a judge call would be skipped
    assert [e["reason"] for e in summary["budget"]["stop_events"]] == ["campaign_dollars"]
    assert (second["outcome"], second["limit_step"]) == ("limit", "not_started")


def test_a_blocked_version_probe_is_cut_off_and_the_command_still_ends_by_the_cap(campaign_root):
    path = settings(campaign_root, models=["fake/a"], case_count=1, trials_per_case=1, campaign_wall_limit_seconds=30,
                    post_trial_reserve_seconds=14, publication_reserve_seconds=10, target_options={"inject_errors": False, "block_versions": True})
    begun = time.monotonic()
    result = invoke(campaign_root, "campaign", "--settings", str(path))
    assert result.returncode == 0, result.stdout + result.stderr
    assert time.monotonic() - begun < 30 + 6
    summary = json.loads((only_report(campaign_root) / "summary.json").read_text())
    assert summary["versions"]["target"] == {"unavailable": "version probe ran out of time"}
    assert {"step": "version_probe", "reason": "campaign_wall_time", "sample_id": None} in summary["budget"]["cut_off_steps"]


def test_unpriced_model_with_a_dollar_budget_is_refused_before_any_call(campaign_root):
    path = settings(campaign_root, campaign_budget_usd=5, prices={"fake/a": {"input_per_million": 1, "output_per_million": 1}})
    result = invoke(campaign_root, "campaign", "--settings", str(path))
    assert result.returncode == 2
    assert "needs a price for every model" in result.stderr
    assert not (campaign_root / "logs").exists() and not (campaign_root / "results").exists()


def only_report(root):
    (directory,) = (root / "results" / "test").iterdir()
    return directory


def test_report_is_written_inside_the_cap_and_records_the_command_to_report_time(campaign_root):
    summary = run(campaign_root, models=["fake/a"], case_count=1, trials_per_case=1,
                  campaign_wall_limit_seconds=60, post_trial_reserve_seconds=20, publication_reserve_seconds=10,
                  tier="tier-0", report_note="Tier 0 is a cost and time floor, not a ranking.",
                  target_options={"inject_errors": False})
    directory = only_report(campaign_root)
    assert summary["campaign_wall_seconds"] > 0
    assert summary["budget"]["wall_within_cap"] is True and summary["budget"]["actual"]["wall_seconds"] == summary["campaign_wall_seconds"]
    report = (directory / "report.md").read_text()
    assert "{CAMPAIGN_WALL_SECONDS}" not in report and "Tier 0 is a cost and time floor" in report
    publication = json.loads((directory / "publication.json").read_text())
    assert publication["within_cap"] is True and publication["command_start_to_report_written_seconds"] <= 60


def test_blocked_rendering_cannot_hold_the_command_past_the_cap(campaign_root):
    """Rendering blocks outright. The limit-only record is already on disk, the child is killed, the command ends by D."""
    path = settings(campaign_root, models=["fake/a"], case_count=1, trials_per_case=1, campaign_wall_limit_seconds=14,
                    post_trial_reserve_seconds=6, publication_reserve_seconds=3, target_options={"inject_errors": False})
    begun = time.monotonic()
    result = invoke(campaign_root, "campaign", "--settings", str(path), extra_env={"EVALCORE_PROOF_RENDER_DELAY_SECONDS": "3600"})
    elapsed = time.monotonic() - begun
    assert result.returncode == 0, result.stdout + result.stderr
    assert elapsed < 14 + 6  # process start and teardown allowed; the point is it is nowhere near the blocked hour
    assert "limit-only record kept" in result.stdout
    record = json.loads((only_report(campaign_root) / "summary.json").read_text())
    assert record["record"] == "limit-only" and record["complete"] is False
    assert [t["outcome"] for t in record["trials"]] == ["success"]
    assert record["publication_cut_off"] is True
    assert {"step": "publication", "reason": "campaign_wall_time", "sample_id": None} in record["cut_off_steps"]
    assert "Cut off: publication" in (only_report(campaign_root) / "report.md").read_text()
    leftover = subprocess.run(["pgrep", "-f", "evalcore.publish"], capture_output=True, text=True)
    assert leftover.stdout.strip() == ""


def test_blocked_setup_step_ends_the_command_with_a_limit_and_the_limit_only_record(campaign_root):
    path = settings(campaign_root, models=["fake/a"], case_count=2, trials_per_case=1, campaign_wall_limit_seconds=18,
                    post_trial_reserve_seconds=8, publication_reserve_seconds=4, target_options={"block_setup": True})
    begun = time.monotonic()
    result = invoke(campaign_root, "campaign", "--settings", str(path))
    assert result.returncode == 0, result.stdout + result.stderr
    assert time.monotonic() - begun < 18 + 4
    record = json.loads((only_report(campaign_root) / "summary.json").read_text())
    assert record["record"] == "limit-only" and record["setup_limit"]["step"] == "image_inspection"
    assert {(t["outcome"], t["limit_step"]) for t in record["trials"]} == {("limit", "image_inspection")}
    assert not (campaign_root / "logs").exists()  # no trial, no model call


def test_limit_only_record_carries_no_private_text():
    from evalcore.publish import minimal_record
    summary = {"campaign": "c", "target": "t", "setup_limit": None, "budget": {"tier": "x", "caps": {}, "stop_events": [], "cut_off_steps": []},
               "trials": [{"sample_id": "s", "combination_id": "m", "case_id": "case", "trial": 1, "outcome": "limit", "reason": "r",
                           "limit_step": "capture", "target_metrics": {"hidden": "PRIVATE-CASE-TEXT"}, "error_type": "PRIVATE-ERROR"}]}
    text = json.dumps(minimal_record(summary, 1.0))
    assert "PRIVATE" not in text and "capture" in text


def test_one_command_runs_one_campaign_and_starts_no_second(campaign_root):
    run(campaign_root, case_count=1, trials_per_case=1, target_options={"inject_errors": False})
    assert len(list((campaign_root / "results").iterdir())) == 1 and len(list((campaign_root / "results" / "test").iterdir())) == 1
    assert len(list((campaign_root / "logs").glob("*.eval"))) == 2  # one log per model, nothing more
    leftover = subprocess.run(["pgrep", "-f", "evalctl campaign"], capture_output=True, text=True)
    assert leftover.stdout.strip() == ""
