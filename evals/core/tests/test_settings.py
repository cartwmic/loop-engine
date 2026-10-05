import pytest
from pydantic import ValidationError

from evalcore.settings import CampaignSettings, Case, matched_assignment


def configuration(**overrides):
    return CampaignSettings.model_validate({"models": ["mockllm/model"], "cases_path": "cases",
                                           "simulator_model": "mockllm/model", "judge_model": "mockllm/model", **overrides})


def test_defaults_and_deterministic_assignment():
    settings = configuration(models=["mockllm/a", "mockllm/b"])
    assert settings.counts == (3, 3)
    assert settings.profile == "standard-14"
    assert settings.resolved_combinations()[0].role_models()["role-reviewer"] == "mockllm/a"
    cases = [Case(id=str(i), version="v1", path="cases") for i in (3, 1, 2, 4)]
    selected, assignments = matched_assignment(settings, cases)
    assert [case.id for case in selected] == ["1", "2", "3"]
    assert len(assignments) == 18
    assert len({a.sample_id for a in assignments}) == 18


@pytest.mark.parametrize("overrides", [
    {"models": []}, {"models": ["missing-provider"]}, {"models": ["mockllm/model"] * 2},
    {"trials_per_case": 0}, {"case_count": -1}, {"time_limit_seconds": 0},
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
