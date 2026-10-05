import pytest
from inspect_ai.model import ModelOutput, ModelRefusalError
from inspect_ai.util import SandboxUnavailableError

from evalcore.accounting import TrialRecord, classify_exception, core_metrics
from evalcore.settings import CampaignSettings
from evalcore.target import InfrastructureError, ModelError


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
