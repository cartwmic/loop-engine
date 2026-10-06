"""Settings-only validation; never resolve/generate live models or run pilot."""
from pathlib import Path

from evalcore.cli import read_settings
from evalcore.settings import CampaignSettings


def test_owner_pilot_loads_with_pinned_roles_thinking_and_default_limits():
    path = Path(__file__).resolve().parents[2] / "software-change/campaigns/pilot.toml"
    settings = CampaignSettings.model_validate(read_settings(str(path)))
    assert settings.cases_path == Path("/Volumes/Workshop/macbook/git/eval-cases")
    assert settings.counts == (3, 1)
    assert settings.profile == "standard-14"
    assert settings.models == ("codex/gpt-6.1-sol", "pi/claude-compat/claude-opus-5-5")
    assert settings.simulator_model == settings.judge_model == "codex/gpt-6.1-sol"
    assert settings.idle_limit_seconds == 1800 and settings.hard_limit_seconds == 28800
    assert settings.recorded()["model_config"] == {
        "codex/gpt-6.1-sol": {"reasoning_effort": "high"},
        "pi/claude-compat/claude-opus-5-5": {"reasoning_effort": "medium"}}
    assert settings.recorded()["setting_sources"]["idle_limit_seconds"] == "default"
