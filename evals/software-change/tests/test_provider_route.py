"""Provider access-route failures, not model content refusals, are infrastructure."""
import asyncio
from types import SimpleNamespace

import pytest
from inspect_ai.model import ModelOutput, ModelRefusalError

from evalcore.accounting import classify_exception
from evalcore.target import InfrastructureError, TrialExecution
from scevals import target

POLICY = "This request was blocked as it seems to violate Anthropic's Terms of Service restrictions on reverse engineering or duplicating model outputs"


@pytest.mark.parametrize("error,reason", [
    ("InfrastructureError: provider policy block", "provider policy block"),
    ("InfrastructureError: provider rejected bridged request", "provider rejected bridged request"),
    (f"BadRequestError('400 {POLICY}')", "provider policy block"),
    (f"Error code: 400 - {POLICY}", "provider policy block"),
    ("Error code: 400 - Invalid `signature` in `thinking` block", "provider rejected bridged request"),
    ("ModelGenerateError('400 Invalid `signature` in `thinking` block')", "provider rejected bridged request"),
    ("BadRequestError('400 malformed request')", "provider rejected bridged request"),
    ("ModelRefusalError('400 malformed request')", None),
    (f"ModelRefusalError('{POLICY}')", None),
    (f"Model refusal (pi/model): {POLICY}", None),
    ("ModelError('400 malformed request')", None),
    ("BadRequestError('400 content refused')", None),
    ("RuntimeError('provider policy block')", None),
])
def test_last_workflow_provider_error_only(monkeypatch, error, reason):
    import inspect_ai.log
    ModelEvent = type("ModelEvent", (), {})
    def event(role, error):
        e = ModelEvent(); e.role = role; e.error = error
        return e
    events = [event("role-reviewer", error), event("judge", "BadRequestError('400 malformed request')")]
    monkeypatch.setattr(inspect_ai.log, "transcript", lambda: SimpleNamespace(events=events))
    assert target.provider_route_failure() == reason
    events.append(event("role-driver", None))
    assert target.provider_route_failure() is None
    events[:] = [event("simulator", error)]
    assert target.provider_route_failure() is None


@pytest.mark.parametrize("reason", ["provider policy block", "provider rejected bridged request"])
def test_nonterminal_score_raises_fixed_infrastructure_before_replay(monkeypatch, reason):
    monkeypatch.setattr(target, "captured_lifecycle", lambda archive: "active")
    execution = TrialExecution(terminal=False, data=("unused", {"evidence": {"provider_route_failure": reason}}))
    with pytest.raises(InfrastructureError, match=f"^{reason}$") as error:
        asyncio.run(target.SoftwareChangeTarget().score(None, execution, None))
    assert classify_exception(error.value) == ("infrastructure", "infrastructure_error")


def test_content_refusal_stays_model_error():
    error = ModelRefusalError(ModelOutput.from_content("mockllm/model", POLICY), "mockllm/model")
    assert classify_exception(error) == ("failure", "model_error")
