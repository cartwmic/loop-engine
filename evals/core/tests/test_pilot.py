"""Settings-only validation; never resolve/generate live models or run pilot."""
from pathlib import Path

from evalcore.cli import read_settings
from evalcore.settings import CampaignSettings


def test_owner_pilot_loads_with_pinned_roles_thinking_and_default_limits():
    path = Path(__file__).resolve().parents[2] / "software-change/campaigns/pilot.toml"
    settings = CampaignSettings.model_validate(read_settings(str(path)))
    assert settings.cases_path == Path("/Volumes/Workshop/macbook/git/eval-cases")
    assert settings.counts == (3, 1)
    assert settings.profile == "standard-16"
    assert settings.models == ("codex/gpt-6.1-sol", "pi/claude-compat/claude-opus-5-5")
    assert settings.simulator_model == settings.judge_model == "codex/gpt-6.1-sol"
    assert settings.idle_limit_seconds == 1800 and settings.hard_limit_seconds == 28800
    assert settings.recorded()["model_config"] == {
        "codex/gpt-6.1-sol": {"reasoning_effort": "high"},
        "pi/claude-compat/claude-opus-5-5": {"reasoning_effort": "medium"}}
    assert settings.recorded()["setting_sources"]["idle_limit_seconds"] == "default"


def test_tier0_campaign_file_is_priced_budgeted_and_names_the_two_models():
    path = Path(__file__).resolve().parents[2] / "software-change/campaigns/tier0.toml"
    settings = CampaignSettings.model_validate(read_settings(str(path)))
    assert settings.models == ("codex/gpt-6.1-sol", "pi/claude-compat/claude-sonnet-5-5")
    assert settings.profile == "minimal-16" and settings.counts == (1, 1) and settings.tier == "tier-0"
    assert (settings.campaign_wall_limit_seconds, settings.campaign_budget_usd) == (1800, 5.0)
    combos = settings.resolved_combinations()
    assert {c.id: set(c.role_models().values()) for c in combos} == {m: {m} for m in settings.models}  # whole workflow each
    assert {settings.simulator_model, settings.judge_model, *settings.models} <= set(settings.prices)  # every role model priced
    assert "not a ranking" in settings.report_note
    sol = settings.prices["codex/gpt-6.1-sol"]
    assert (sol.input_per_million, sol.tiers[0].input_tokens_above, sol.tiers[0].input_per_million) == (2, 272000, 4)
