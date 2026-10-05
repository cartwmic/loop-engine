"""Black-box evalctl campaigns: no mocks of the runner, CLI or report writer."""
import json
import os
import subprocess
from pathlib import Path

import pytest
from inspect_ai.log import read_eval_log

REPO = Path(__file__).resolve().parents[3]


def invoke(root, *args, extra_env=None):
    env = {**os.environ, "EVAL_FAKE_ROOT": str(root),
           "PYTHONPATH": str(Path(__file__).parent), **(extra_env or {})}
    command = ["uv", "run", "--project", "evals", "evalctl", *args]
    result = subprocess.run(command, cwd=REPO, env=env, text=True, capture_output=True, timeout=90)
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
    assert summary["settings"]["profile"] == "standard-14"
    assert summary["settings"]["setting_sources"]["preset"] == "default"
    assert summary["settings"]["setting_sources"]["models"] == "explicit"
    assert summary["pinned_models"] == {"simulator": "fake/a", "judge": "fake/a"}
    assert summary["versions"]["target"]["loop_engine"] == "v0.24.0"
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


def test_actual_time_limit_is_failure(campaign_root):
    summary = run(campaign_root, models=["fake/a"], case_count=1, trials_per_case=1,
                  time_limit_seconds=1, target_options={"sleep": True})
    assert summary["trials"][0]["outcome"] == "failure"
    assert summary["comparisons"][0]["core"]["timeouts"] == 1
    assert summary["comparisons"][0]["core"]["success_rate"]["denominator"] == 1


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
