"""The campaign guard's marks, allowances and stop state."""
import time

import pytest

from evalcore.limits import CampaignGuard
from evalcore.settings import CampaignSettings

PRICE = {"input_per_million": 2, "output_per_million": 0}


def guard(started=None, **overrides):
    settings = CampaignSettings.model_validate({
        "models": ["mockllm/model"], "cases_path": "cases", "simulator_model": "mockllm/model",
        "judge_model": "mockllm/model", "prices": {"mockllm/model": PRICE}, **overrides})
    return CampaignGuard(settings, started)


def test_marks_follow_the_command_start():
    g = guard(100.0, campaign_wall_limit_seconds=1000, post_trial_reserve_seconds=300, publication_reserve_seconds=60)
    assert (g.deadline, g.stop_line, g.hard_end) == (1100.0, 800.0, 1040.0)


def test_no_limits_means_inactive_and_unbounded_allowance():
    g = guard()
    assert not g.active and g.allowance(600) == 600 and g.allowance(None) is None
    assert g.can_start_trial() and g.model_calls_allowed()


def test_allowance_is_bounded_by_time_left_before_the_hard_end():
    g = guard(time.monotonic() - 90, campaign_wall_limit_seconds=200, post_trial_reserve_seconds=100, publication_reserve_seconds=20)
    assert 85 < g.allowance(600) <= 90      # H is 180s after start; about 90s left
    assert g.allowance(10) == 10
    late = guard(time.monotonic() - 190, campaign_wall_limit_seconds=200, post_trial_reserve_seconds=100, publication_reserve_seconds=20)
    assert late.allowance(600) == 0 and not late.model_calls_allowed()


def test_wall_stop_line_blocks_new_trials_and_names_the_reason():
    g = guard(time.monotonic() - 150, campaign_wall_limit_seconds=200, post_trial_reserve_seconds=100, publication_reserve_seconds=20)
    assert not g.can_start_trial()
    assert g.stop_reason == "campaign_wall_time" and not g.model_calls_allowed()
    assert g.step_stop_reason() == "campaign_wall_time"


def test_dollar_stop_line_is_cap_minus_margin_and_counts_finished_and_live_spend():
    g = guard(campaign_budget_usd=5, dollar_margin_usd=0.5)
    roles = {"role-driver": "mockllm/model"}
    assert g.check(roles, {"role-driver": {"input_tokens": 2_000_000}}) is None   # $4 < 4.5
    assert g.check(roles, {"role-driver": {"input_tokens": 2_250_000}}) == "campaign_dollars"   # $4.5
    g.add_finished(roles, {"role-driver": {"input_tokens": 2_000_000}})
    assert g.check(roles, {"role-driver": {"input_tokens": 1_000_000}}) == "campaign_dollars"   # $4 + $2
    g.fire("campaign_dollars")
    assert not g.can_start_trial() and not g.model_calls_allowed()


def test_first_stop_wins_and_is_recorded_once():
    g = guard(campaign_budget_usd=5)
    g.fire("campaign_dollars")
    g.fire("campaign_wall_time")
    assert g.stop_reason == "campaign_dollars" and [e["reason"] for e in g.events] == ["campaign_dollars"]


def test_token_budget_stops_but_is_only_an_extra():
    g = guard(campaign_token_budget=100)
    assert g.check({}, {"role-driver": {"input_tokens": 99}}) is None
    assert g.check({}, {"role-driver": {"input_tokens": 60, "output_tokens": 40}}) == "campaign_tokens"
