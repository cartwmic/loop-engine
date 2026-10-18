"""Fixed-k accounting. Infrastructure is never silently retried or dropped."""
from dataclasses import asdict, dataclass, field
from typing import Literal

from inspect_ai.model import ModelRefusalError
from inspect_ai.util import SandboxUnavailableError

from .settings import CampaignSettings, OVERHEAD_ROLES, Price, WORKFLOW_ROLES
from .target import InfrastructureError, LimitReached, ModelError

Outcome = Literal["success", "failure", "infrastructure", "limit"]

CLASSIFICATION_RULES = (
    "Success requires a terminal trial and target success. Timeouts, hangs, limits and model errors "
    "are failures. Harness/container/bridge errors and provider-service failures (including exhausted "
    "429/5xx responses) are infrastructure. Unknown exceptions are failures unless the target marks "
    "them InfrastructureError. A trial or step stopped by a time, dollar or token budget is 'limit' with its "
    "reason; it counts as a non-success in the headline rates, is listed separately and is never retried. "
    "Infrastructure is excluded from success rate and fixed-k pass^k; "
    "cases with fewer than k scorable trials are listed as incomplete. No automatic trial retries."
)


def classify_exception(error: Exception) -> tuple[Outcome, str]:
    # Do not classify by error message: model text must not be able to spoof infrastructure.
    chain = []
    seen = set()
    current = error
    while current is not None and id(current) not in seen:
        seen.add(id(current))
        chain.append(current)
        current = current.__cause__ or current.__context__
    limit = next((e for e in chain if isinstance(e, LimitReached)), None)
    if limit is not None:
        return "limit", limit.reason
    if any(isinstance(e, TimeoutError) or type(e).__name__ in {
        "APITimeoutError", "AttemptTimeoutError", "StreamIdleTimeoutError"
    } for e in chain):
        return "failure", "timeout"
    if any(isinstance(e, (ModelError, ModelRefusalError)) for e in chain):
        return "failure", "model_error"
    if any(isinstance(e, (InfrastructureError, SandboxUnavailableError, ConnectionError)) for e in chain):
        return "infrastructure", "infrastructure_error"
    for e in chain:
        module = type(e).__module__
        inspect_provider_error = module.startswith("inspect_ai.") and type(e).__name__ == "ModelGenerateError"
        if inspect_provider_error or module.split(".")[0] in {
            "openai", "anthropic", "httpx", "aiohttp", "botocore", "mistralai", "groq"
        } or module.startswith(("google.genai.", "google.api_core.")):
            status = getattr(e, "status_code", None) or getattr(e, "code", None)
            if status == 408:
                return "failure", "timeout"
            if status in (401, 403, 429) or isinstance(status, int) and status >= 500:
                return "infrastructure", "provider_service_error"
            if type(e).__name__ in {"APIConnectionError", "ConnectError", "ClientConnectionError"}:
                return "infrastructure", "provider_service_error"
            if inspect_provider_error:
                # Inspect wraps provider errors, not bad model text. Inspect's wrapper
                # preserves status_code; do not mistake a 503 wrapper for model error.
                return ("infrastructure", "provider_service_error") if status is None else ("failure", "model_error")
    return "failure", "unclassified_error"


@dataclass
class TrialRecord:
    sample_id: str
    combination_id: str
    case_id: str
    case_version: str
    trial: int
    outcome: Outcome
    reason: str
    terminal: bool = False
    wall_seconds: float = 0
    # Inspect role_usage, not model_usage: one model can serve both workflow and overhead.
    usage_by_role: dict[str, dict] = field(default_factory=dict)
    target_metrics: dict = field(default_factory=dict)
    error_type: str | None = None
    resumed: bool = False
    segments: list[dict] = field(default_factory=list)
    carried_from: str | None = None
    usage_unknown_roles: list[str] = field(default_factory=list)
    # For outcome 'limit': the step that was cut off (e.g. capture, scoring, judge, not_started).
    limit_step: str | None = None

    def recorded(self) -> dict:
        return asdict(self)


def rate(numerator: int, denominator: int) -> dict:
    return {"numerator": numerator, "denominator": denominator,
            "rate": numerator / denominator if denominator else None}


def usage_cost_usd(usage: dict, price: Price | None) -> float | None:
    """The one price rule. None means unpriced, never zero. Used for live and final estimates."""
    if price is None:
        return None
    read_price = price.input_per_million if price.cache_read_per_million is None else price.cache_read_per_million
    write_price = price.input_per_million if price.cache_write_per_million is None else price.cache_write_per_million
    def get(key):
        return usage.get(key) or 0
    return (get("input_tokens") * price.input_per_million + get("output_tokens") * price.output_per_million
            + get("input_tokens_cache_read") * read_price + get("input_tokens_cache_write") * write_price) / 1_000_000


def usage_tokens(usage: dict) -> int:
    return sum(usage.get(k) or 0 for k in ("input_tokens", "output_tokens", "input_tokens_cache_read", "input_tokens_cache_write"))


def estimate_usd(settings: CampaignSettings, role_models: dict[str, str], usage_by_role: dict) -> float | None:
    """Dollar estimate over roles; None when any used role's model is unpriced."""
    total = 0.0
    for role, usage in usage_by_role.items():
        if usage_tokens(usage) == 0:
            continue
        cost = usage_cost_usd(usage, settings.prices.get(role_models.get(role, "")))
        if cost is None:
            return None
        total += cost
    return total


def cost_by_role(records: list[TrialRecord], settings: CampaignSettings, role_models: dict[str, str], roles: tuple) -> dict:
    result = {}
    for role in roles:
        def total(key):
            return sum(r.usage_by_role.get(role, {}).get(key) or 0 for r in records)
        inputs, outputs = total("input_tokens"), total("output_tokens")
        cache_read, cache_write = total("input_tokens_cache_read"), total("input_tokens_cache_write")
        model = role_models[role]
        price = settings.prices.get(model)
        known = not any(role in r.usage_unknown_roles for r in records)
        usd = None
        if price and known:
            usd = usage_cost_usd({"input_tokens": inputs, "output_tokens": outputs, "input_tokens_cache_read": cache_read,
                                  "input_tokens_cache_write": cache_write}, price)
        result[role] = {
            "model": model, "input_tokens": inputs if known else None, "output_tokens": outputs if known else None,
            "cache_read_tokens": cache_read if known else None, "cache_write_tokens": cache_write if known else None,
            "total_tokens": inputs + outputs + cache_read + cache_write if known else None,
            "usage_available": known,
            "estimated_usd": usd, "price_available": price is not None,
        }
    dollars = [v["estimated_usd"] for v in result.values()]
    totals = [v["total_tokens"] for v in result.values()]
    return {"by_role": result, "total_tokens": sum(totals) if all(v is not None for v in totals) else None,
            "estimated_usd": sum(dollars) if all(v is not None for v in dollars) else None}


def cost_verdict(settings: CampaignSettings, models_in_use: list[str], total_usd: float | None) -> str:
    """not_budgeted, pass, fail_over_cap, fail_unpriced or fail_unknown_usage. Unpriced is never zero."""
    if settings.campaign_budget_usd is None:
        return "not_budgeted"
    if any(m not in settings.prices for m in models_in_use):
        return "fail_unpriced"
    if total_usd is None:
        return "fail_unknown_usage"
    return "pass" if total_usd <= settings.campaign_budget_usd else "fail_over_cap"


def campaign_totals(comparisons: list[dict]) -> dict:
    """Whole-campaign tokens and dollars from per-combination core metrics; None if any part is unknown."""
    parts = [c["core"][section] for c in comparisons for section in ("workflow_cost", "evaluation_overhead")]
    def add(key):
        values = [p[key] for p in parts]
        return None if any(v is None for v in values) else sum(values)
    return {"total_tokens": add("total_tokens"), "estimated_usd": add("estimated_usd")}


def core_metrics(records: list[TrialRecord], settings: CampaignSettings, role_models: dict[str, str]) -> dict:
    _, k = settings.counts
    scorable = [r for r in records if r.outcome != "infrastructure" and not r.resumed]
    case_ids = sorted({r.case_id for r in records})
    eligible = []
    incomplete = []
    for case_id in case_ids:
        trials = [r for r in scorable if r.case_id == case_id]
        if len(trials) == k:
            eligible.append(trials)
        else:
            incomplete.append({"case_id": case_id, "scorable_trials": len(trials), "required_trials": k})
    return {
        "trial_count": len(records),
        "resumed_trials": len([r for r in records if r.resumed]),
        "resumed_success_rate": rate(sum(r.outcome == "success" for r in records if r.resumed and r.outcome != "infrastructure"),
                                     sum(r.resumed and r.outcome != "infrastructure" for r in records)),
        "terminated_trials": sum(r.target_metrics.get("lifecycle") == "terminated" for r in records),
        "success_rate": rate(sum(r.outcome == "success" for r in scorable), len(scorable)),
        "pass_k": {"k": k, **rate(sum(all(r.outcome == "success" for r in ts) for ts in eligible), len(eligible))},
        "incomplete_cases": incomplete,
        "infrastructure_failure_rate": rate(sum(r.outcome == "infrastructure" for r in records), len(records)),
        "timeouts": sum(r.reason == "timeout" for r in records),
        "limits": {reason: sum(r.outcome == "limit" and r.reason == reason for r in records)
                   for reason in sorted({r.reason for r in records if r.outcome == "limit"})},
        "model_errors": sum(r.reason == "model_error" for r in records),
        "failures": sum(r.outcome == "failure" for r in records),
        "workflow_completion": rate(sum(r.terminal for r in records), len(records)),
        "workflow_cost": cost_by_role(records, settings, role_models, WORKFLOW_ROLES),
        "evaluation_overhead": cost_by_role(records, settings, role_models, OVERHEAD_ROLES),
        "wall_seconds": sum(r.wall_seconds for r in records),
    }
