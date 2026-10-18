import pytest
from inspect_ai.model import ModelOutput, ModelRefusalError
from inspect_ai.util import SandboxUnavailableError

from evalcore.accounting import (TrialRecord, campaign_totals, classify_exception, core_metrics, cost_by_role,
                                 cost_verdict, estimate_usd, usage_cost_usd)
from evalcore.settings import CampaignSettings
from evalcore.target import InfrastructureError, LimitReached, ModelError


def settings(**overrides):
    return CampaignSettings(models=("mockllm/model",), cases_path="cases",
                            simulator_model="mockllm/model", judge_model="mockllm/model", **overrides)


def record(case, trial, outcome):
    return TrialRecord(sample_id=f"{case}-{trial}", combination_id="mockllm/model", case_id=case,
                       case_version="v1", trial=trial, outcome=outcome, reason="test", terminal=True)


@pytest.mark.parametrize("error,outcome,reason", [
    (TimeoutError(), "failure", "timeout"),
    (ModelError(), "failure", "model_error"),
    (ModelRefusalError(ModelOutput.from_content("mockllm/model", "refused"), "mockllm/model"), "failure", "model_error"),
    (InfrastructureError(), "infrastructure", "infrastructure_error"),
    (SandboxUnavailableError("unavailable"), "infrastructure", "infrastructure_error"),
    (ConnectionError(), "infrastructure", "infrastructure_error"),
    (ValueError("InfrastructureError: spoofed model text"), "failure", "unclassified_error"),
])
def test_classification(error, outcome, reason):
    assert classify_exception(error) == (outcome, reason)


def test_nested_timeout_stays_failure():
    error = InfrastructureError()
    error.__cause__ = TimeoutError()
    assert classify_exception(error) == ("failure", "timeout")


def test_provider_status_rules():
    ProviderError = type("ProviderError", (Exception,), {"__module__": "openai.errors", "status_code": 503})
    assert classify_exception(ProviderError()) == ("infrastructure", "provider_service_error")
    ProviderError.status_code = 429
    assert classify_exception(ProviderError())[0] == "infrastructure"
    ProviderError.status_code = 400
    assert classify_exception(ProviderError())[0] == "failure"


@pytest.mark.parametrize("outcomes,success,pass_k,incomplete", [
    (["success", "success", "success"], (3, 3), (1, 1), False),
    (["success", "failure", "success"], (2, 3), (0, 1), False),
    (["success", "infrastructure", "success"], (2, 2), (0, 0), True),
    (["infrastructure"] * 3, (0, 0), (0, 0), True),
])
def test_fixed_k_and_empty_denominators(outcomes, success, pass_k, incomplete):
    configuration = settings()
    roles = {**configuration.resolved_combinations()[0].role_models(),
             "simulator": configuration.simulator_model, "judge": configuration.judge_model}
    result = core_metrics([record("a", i, outcome) for i, outcome in enumerate(outcomes)], configuration, roles)
    for name, counts in (("success_rate", success), ("pass_k", pass_k)):
        assert (result[name]["numerator"], result[name]["denominator"]) == counts
        assert result[name]["rate"] == (counts[0] / counts[1] if counts[1] else None)
    assert bool(result["incomplete_cases"]) is incomplete
    assert result["workflow_cost"]["estimated_usd"] is None  # missing price is not free


def test_cached_input_counts_toward_tokens_and_cost():
    s = settings(prices={"mockllm/model": {"input_per_million": 1, "output_per_million": 2,
                                           "cache_read_per_million": 0.1}})
    r = record("c", 1, "success")
    r.usage_by_role = {"role-driver": {"input_tokens": 10, "output_tokens": 5,
                                       "input_tokens_cache_read": 1_000_000, "input_tokens_cache_write": 100}}
    cost = cost_by_role([r], s, {"role-driver": "mockllm/model"}, ("role-driver",))
    row = cost["by_role"]["role-driver"]
    assert row["total_tokens"] == 10 + 5 + 1_000_000 + 100
    # cache write falls back to the input price
    assert row["estimated_usd"] == pytest.approx((10 * 1 + 5 * 2 + 1_000_000 * 0.1 + 100 * 1) / 1_000_000)


def test_limit_reached_is_limit_with_its_reason_even_when_nested():
    assert classify_exception(LimitReached("campaign_dollars", "judge")) == ("limit", "campaign_dollars")
    outer = RuntimeError()
    outer.__cause__ = LimitReached("campaign_wall_time", "capture")
    assert classify_exception(outer) == ("limit", "campaign_wall_time")


def test_limit_counts_as_scorable_non_success():
    configuration = settings()
    roles = {**configuration.resolved_combinations()[0].role_models(),
             "simulator": configuration.simulator_model, "judge": configuration.judge_model}
    limited = record("a", 1, "limit")
    limited.reason = "campaign_dollars"
    result = core_metrics([limited, record("a", 2, "success"), record("a", 3, "success")], configuration, roles)
    assert (result["success_rate"]["numerator"], result["success_rate"]["denominator"]) == (2, 3)
    assert result["pass_k"]["numerator"] == 0
    assert result["limits"] == {"campaign_dollars": 1}
    assert result["infrastructure_failure_rate"]["numerator"] == 0


def test_one_price_rule_for_live_and_final_estimates():
    s = settings(prices={"mockllm/model": {"input_per_million": 1, "output_per_million": 2, "cache_read_per_million": 0.1}})
    usage = {"input_tokens": 10, "output_tokens": 5, "input_tokens_cache_read": 1_000_000, "input_tokens_cache_write": 100}
    r = record("c", 1, "success")
    r.usage_by_role = {"role-driver": usage}
    final = cost_by_role([r], s, {"role-driver": "mockllm/model"}, ("role-driver",))["by_role"]["role-driver"]["estimated_usd"]
    assert usage_cost_usd(usage, s.prices["mockllm/model"]) == pytest.approx(final)
    assert estimate_usd(s, {"role-driver": "mockllm/model"}, {"role-driver": usage}) == pytest.approx(final)


def test_unpriced_model_never_estimates_as_zero():
    s = settings()
    usage = {"input_tokens": 10, "output_tokens": 5}
    assert usage_cost_usd(usage, None) is None
    assert estimate_usd(s, {"role-driver": "mockllm/model"}, {"role-driver": usage}) is None
    assert estimate_usd(s, {"role-driver": "mockllm/model"}, {"role-driver": {}}) == 0


def test_cost_verdicts():
    priced = settings(prices={"mockllm/model": {"input_per_million": 1, "output_per_million": 2}}, campaign_budget_usd=5)
    models = ["mockllm/model"]
    assert cost_verdict(priced, models, 4.99) == "pass"
    assert cost_verdict(priced, models, 5.0) == "pass"
    assert cost_verdict(priced, models, 5.01) == "fail_over_cap"
    assert cost_verdict(priced, models, None) == "fail_unknown_usage"
    assert cost_verdict(priced, ["mockllm/other"], 0.0) == "fail_unpriced"
    assert cost_verdict(settings(), models, None) == "not_budgeted"


def test_campaign_totals_are_none_when_any_part_is_unknown():
    part = {"total_tokens": 10, "estimated_usd": 1.0}
    unknown = {"total_tokens": 10, "estimated_usd": None}
    assert campaign_totals([{"core": {"workflow_cost": part, "evaluation_overhead": part}}]) == {"total_tokens": 20, "estimated_usd": 2.0}
    assert campaign_totals([{"core": {"workflow_cost": part, "evaluation_overhead": unknown}}])["estimated_usd"] is None
