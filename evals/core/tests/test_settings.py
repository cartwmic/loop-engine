import pytest
from pydantic import ValidationError

from evalcore.settings import CampaignSettings, Case, matched_assignment


def configuration(**overrides):
    return CampaignSettings.model_validate({"models": ["mockllm/model"], "cases_path": "cases",
                                           "simulator_model": "mockllm/model", "judge_model": "mockllm/model", **overrides})


def test_defaults_and_deterministic_assignment():
    settings = configuration(models=["mockllm/a", "mockllm/b"])
    assert settings.counts == (3, 3)
    assert settings.profile == "standard-16"
    assert settings.resolved_combinations()[0].role_models()["role-reviewer"] == "mockllm/a"
    cases = [Case(id=str(i), version="v1", path="cases") for i in (3, 1, 2, 4)]
    selected, assignments = matched_assignment(settings, cases)
    assert [case.id for case in selected] == ["1", "2", "3"]
    assert len(assignments) == 18
    assert len({a.sample_id for a in assignments}) == 18


@pytest.mark.parametrize("overrides", [
    {"model_config": {"mockllm/model": {"reasoning_effort": "invalid"}}},
    {"model_config": {"mockllm/absent": {"reasoning_effort": "high"}}},
    {"model_config": {"mockllm/model": {"reasoning_effort": "high", "unsupported": True}}},
    {"models": []}, {"models": ["missing-provider"]}, {"models": ["mockllm/model"] * 2},
    {"trials_per_case": 0}, {"case_count": -1}, {"hard_limit_seconds": 0}, {"idle_limit_seconds": 0},
    {"preset": "large"}, {"campaign": "../escape"}, {"simulator_model": ""},
    {"prices": {"mockllm/model": {"input_per_million": -1, "output_per_million": 0}}},
    {"prices": {"mockllm/model": {"input_per_million": float("nan"), "output_per_million": 0}}},
    {"models": ["mockllm/a"], "combinations": [{"id": "x", "driver": "mockllm/a", "implementer": "mockllm/a", "reviewer": "mockllm/a"}]},
])
def test_invalid_settings(overrides):
    with pytest.raises(ValidationError):
        configuration(**overrides)


def test_case_shortage_and_duplicates_are_not_silently_unmatched():
    settings = configuration()
    with pytest.raises(ValueError, match="requested 3"):
        matched_assignment(settings, [Case(id="one", version="v1", path="cases")])
    with pytest.raises(ValueError, match="unique"):
        matched_assignment(settings, [Case(id="one", version="v1", path="cases")] * 3)


PRICE = {"input_per_million": 1, "output_per_million": 2}


def test_dollar_budget_refuses_unpriced_model_before_any_call():
    with pytest.raises(ValidationError, match="unpriced: mockllm/model"):
        configuration(campaign_budget_usd=5)
    # Every role model counts: pinned simulator and judge too.
    with pytest.raises(ValidationError, match="mockllm/judge"):
        configuration(campaign_budget_usd=5, judge_model="mockllm/judge", prices={"mockllm/model": PRICE})
    ok = configuration(campaign_budget_usd=5, prices={"mockllm/model": PRICE})
    assert ok.campaign_budget_usd == 5


def test_unbudgeted_campaign_may_be_unpriced_and_token_budget_is_not_a_substitute():
    assert configuration().campaign_budget_usd is None
    with pytest.raises(ValidationError):
        configuration(campaign_budget_usd=5, campaign_token_budget=1000)


@pytest.mark.parametrize("overrides", [
    {"campaign_budget_usd": 1, "dollar_margin_usd": 1, "prices": {"mockllm/model": PRICE}},
    {"campaign_budget_usd": 0, "prices": {"mockllm/model": PRICE}},
    {"campaign_wall_limit_seconds": 100, "post_trial_reserve_seconds": 100},
    {"campaign_wall_limit_seconds": 100, "post_trial_reserve_seconds": 50, "publication_reserve_seconds": 50},
    {"campaign_token_budget": 0},
])
def test_invalid_budget_settings(overrides):
    with pytest.raises(ValidationError):
        configuration(**overrides)


def test_price_tiers_load_and_reject_bad_thresholds():
    price = {**PRICE, "tiers": [{"input_tokens_above": 272000, "input_per_million": 4, "output_per_million": 15}]}
    assert configuration(prices={"mockllm/model": price}).prices["mockllm/model"].tiers[0].input_tokens_above == 272000
    with pytest.raises(ValidationError):
        configuration(prices={"mockllm/model": {**PRICE, "tiers": [{"input_tokens_above": 0, "input_per_million": 4, "output_per_million": 15}]}})
    with pytest.raises(ValidationError):
        configuration(prices={"mockllm/model": {**PRICE, "tiers": [{"input_tokens_above": 5, "input_per_million": 4}]}})


def test_budget_defaults_are_off_and_recorded():
    settings = configuration()
    assert (settings.campaign_wall_limit_seconds, settings.campaign_budget_usd, settings.campaign_token_budget) == (None, None, None)
    recorded = settings.recorded()
    assert recorded["setting_sources"]["campaign_budget_usd"] == "default"
