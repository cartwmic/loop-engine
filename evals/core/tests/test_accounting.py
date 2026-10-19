import pytest
from inspect_ai.model import ModelOutput, ModelRefusalError
from inspect_ai.util import SandboxUnavailableError

from types import SimpleNamespace

from evalcore.accounting import (TrialRecord, call_cost_usd, campaign_totals, classify_exception, core_metrics, cost_by_role,
                                 cost_verdict, estimate_usd, events_spend, usage_cost_usd)
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


SOL = {"input_per_million": 2, "output_per_million": 10, "cache_read_per_million": 0.1, "cache_write_per_million": 2.5,
       "tiers": [{"input_tokens_above": 272000, "input_per_million": 4, "output_per_million": 15,
                  "cache_read_per_million": 0.2, "cache_write_per_million": 5}]}


def sol_price():
    return settings(prices={"mockllm/model": SOL}).prices["mockllm/model"]


def test_each_request_is_priced_by_its_own_size():
    price = sol_price()
    small = {"input_tokens": 1000, "output_tokens": 100, "input_tokens_cache_read": 99_000}      # 100k total input
    edge = {"input_tokens": 272_000, "output_tokens": 0}                                          # at the threshold: base
    big = {"input_tokens": 100_000, "output_tokens": 100, "input_tokens_cache_read": 200_000}     # 300k total: tier
    assert call_cost_usd(small, price) == pytest.approx((1000 * 2 + 100 * 10 + 99_000 * 0.1) / 1e6)
    assert call_cost_usd(edge, price) == pytest.approx(272_000 * 2 / 1e6)
    assert call_cost_usd({"input_tokens": 272_001}, price) == pytest.approx(272_001 * 4 / 1e6)
    assert call_cost_usd(big, price) == pytest.approx((100_000 * 4 + 100 * 15 + 200_000 * 0.2) / 1e6)
    # Two small calls plus one big call: exact is far below the aggregate upper bound.
    exact = sum(call_cost_usd(u, price) for u in (small, small, big))
    total = {k: sum(u.get(k, 0) for u in (small, small, big)) for k in ("input_tokens", "output_tokens", "input_tokens_cache_read")}
    assert exact < usage_cost_usd(total, price)
    assert usage_cost_usd(total, price) == pytest.approx((total["input_tokens"] * 4 + total["output_tokens"] * 15 + total["input_tokens_cache_read"] * 0.2) / 1e6)


def test_unpriced_call_and_no_tier_price_behave():
    flat = settings(prices={"mockllm/model": {"input_per_million": 1, "output_per_million": 2}}).prices["mockllm/model"]
    assert call_cost_usd({"input_tokens": 10_000_000}, flat) == pytest.approx(10.0)  # 10M tokens at 1 per million
    assert call_cost_usd({"input_tokens": 1}, None) is None


class ModelEvent:  # same class name as Inspect's event; events_spend matches on it
    def __init__(self, role, usage):
        self.role = role
        self.output = SimpleNamespace(usage=None if usage is None else SimpleNamespace(model_dump=lambda: usage))


def test_events_spend_prices_each_call_and_never_reads_unpriced_as_zero():
    s = settings(prices={"mockllm/model": SOL})
    roles = {"role-driver": "mockllm/model", "judge": "mockllm/model"}
    events = [ModelEvent("role-driver", {"input_tokens": 100_000, "output_tokens": 0}),
              ModelEvent("role-driver", {"input_tokens": 300_000, "output_tokens": 0}),
              ModelEvent("judge", {"input_tokens": 1_000_000, "output_tokens": 0}), ModelEvent(None, {"input_tokens": 9}),
              ModelEvent("role-driver", None), object()]
    spend = events_spend(events, roles, s.prices)
    assert spend == {"role-driver": pytest.approx(100_000 * 2 / 1e6 + 300_000 * 4 / 1e6), "judge": pytest.approx(4.0)}
    assert events_spend(events, {"role-driver": "mockllm/other", "judge": "mockllm/model"}, s.prices) is None
    assert events_spend([], roles, s.prices) == {}


def test_final_cost_uses_exact_per_call_dollars_when_every_record_has_them():
    s = settings(prices={"mockllm/model": SOL})
    usage = {"input_tokens": 600_000, "output_tokens": 0}
    a, b = record("c", 1, "success"), record("c", 2, "success")
    for r in (a, b):
        r.usage_by_role = {"role-driver": usage}
        r.priced_usd_by_role = {"role-driver": 1.2}      # two 300k calls at 2 per million each... exact, not the 4 per million bound
    exact = cost_by_role([a, b], s, {"role-driver": "mockllm/model"}, ("role-driver",))["by_role"]["role-driver"]["estimated_usd"]
    assert exact == pytest.approx(2.4)
    b.priced_usd_by_role = None                          # one record without per-call data: fall back to the upper bound
    bound = cost_by_role([a, b], s, {"role-driver": "mockllm/model"}, ("role-driver",))["by_role"]["role-driver"]["estimated_usd"]
    assert bound == pytest.approx(2 * 600_000 * 4 / 1e6) and bound > exact


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
