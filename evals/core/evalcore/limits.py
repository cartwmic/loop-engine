"""Campaign guard: one clock, three deadline marks, and the first stop that fires. Target neutral.

D is the cap (campaign_wall_limit_seconds from command start). S = D - post_trial_reserve is where trial
work stops. H = D - publication_reserve is where capture, scoring and the judge must be done.
The dollar cap is best effort: spend is polled, so a campaign can overshoot by about one model call.
"""
import time

import anyio

from .accounting import campaign_totals, cost_verdict, estimate_usd, usage_tokens
from .settings import CampaignSettings


class CampaignGuard:
    def __init__(self, settings: CampaignSettings, started: float | None = None):
        self.settings = settings
        self.started = time.monotonic() if started is None else started
        limit = settings.campaign_wall_limit_seconds
        self.deadline = None if limit is None else self.started + limit                      # D
        self.stop_line = None if limit is None else self.deadline - settings.post_trial_reserve_seconds   # S
        self.hard_end = None if limit is None else self.deadline - settings.publication_reserve_seconds   # H
        self.stop_reason: str | None = None
        self.events: list[dict] = []
        self.cut_off_steps: list[dict] = []
        self.finished_usd = 0.0
        self.finished_tokens = 0
        self._event = None
        self.active = any(v is not None for v in (limit, settings.campaign_budget_usd, settings.campaign_token_budget))

    # --- stop state -------------------------------------------------------------------------
    def event(self) -> anyio.Event:
        """Set when any stop fires. Created lazily because anyio events need a running loop."""
        if self._event is None:
            self._event = anyio.Event()
            if self.stop_reason:
                self._event.set()
        return self._event

    def fire(self, reason: str) -> None:
        if self.stop_reason is None:
            self.stop_reason = reason
            self.events.append({"reason": reason, "elapsed_seconds": round(self.elapsed(), 3)})
            if self._event is not None:
                self._event.set()

    def elapsed(self) -> float:
        return time.monotonic() - self.started

    def note_cut_off(self, step: str, reason: str, sample_id: str | None = None) -> None:
        self.cut_off_steps.append({"step": step, "reason": reason, "sample_id": sample_id})

    # --- time -------------------------------------------------------------------------------
    def remaining_to_hard_end(self) -> float | None:
        return None if self.hard_end is None else max(0.0, self.hard_end - time.monotonic())

    def remaining_to_deadline(self) -> float | None:
        """Time left before the cap D (publication and summary preparation live between H and D)."""
        return None if self.deadline is None else max(0.0, self.deadline - time.monotonic())

    def allowance(self, own_limit: float | None) -> float | None:
        """A post-trial step's time: its own limit, bounded by what is left before H."""
        remaining = self.remaining_to_hard_end()
        if remaining is None:
            return own_limit
        return remaining if own_limit is None else min(own_limit, remaining)

    def step_stop_reason(self) -> str:
        """Why a step was cut off: the first stop if one fired, else the deadline."""
        return self.stop_reason or "campaign_wall_time"

    def model_calls_allowed(self) -> bool:
        """No new model call after any campaign stop, or once the post-trial window has closed."""
        remaining = self.remaining_to_hard_end()
        return self.stop_reason is None and (remaining is None or remaining > 0)

    # --- spend ------------------------------------------------------------------------------
    def add_finished(self, role_models: dict[str, str], usage_by_role: dict, exact_usd_by_role: dict | None = None) -> None:
        """Bank a finished trial's spend: exact per-call dollars when known, else the aggregate upper bound."""
        if exact_usd_by_role is not None:
            cost = sum(exact_usd_by_role.values())
        else:
            cost = estimate_usd(self.settings, role_models, usage_by_role)
        self.finished_usd += cost or 0.0
        self.finished_tokens += sum(usage_tokens(u) for u in usage_by_role.values())

    def check(self, role_models: dict[str, str], live_usage_by_role: dict, live_exact_usd: float | None = None) -> str | None:
        """The first stop reason that applies now, or None. Does not fire it."""
        if self.stop_reason:
            return self.stop_reason
        if self.stop_line is not None and time.monotonic() >= self.stop_line:
            return "campaign_wall_time"
        budget = self.settings.campaign_budget_usd
        if budget is not None:
            live = live_exact_usd if live_exact_usd is not None else (estimate_usd(self.settings, role_models, live_usage_by_role) or 0.0)
            spent = self.finished_usd + live
            if spent >= budget - self.settings.dollar_margin_usd:
                return "campaign_dollars"
        tokens = self.settings.campaign_token_budget
        if tokens is not None and self.finished_tokens + sum(usage_tokens(u) for u in live_usage_by_role.values()) >= tokens:
            return "campaign_tokens"
        return None

    def can_start_trial(self) -> bool:
        reason = self.check({}, {})
        if reason:
            self.fire(reason)
        return reason is None


async def watch_trial(guard: CampaignGuard, role_models: dict[str, str], live_usage, trial_scope: anyio.CancelScope,
                      live_spend=None) -> None:
    """Poll while a trial runs. On the first stop, fire it; a target that does not stop itself is cancelled after the grace."""
    while True:
        await anyio.sleep(guard.settings.guard_poll_seconds)
        reason = guard.check(role_models, live_usage(), live_spend() if live_spend else None)
        if reason:
            guard.fire(reason)
            break
    await anyio.sleep(guard.settings.stop_grace_seconds)
    trial_scope.cancel()


DOLLAR_CAP_BASIS = ("best effort: the stop line is the cap minus a margin and spend is polled, so a campaign can very rarely "
                    "overshoot the cap by about one model call; an overshoot fails the cost verdict")


def budget_summary(guard: CampaignGuard, comparisons: list[dict], models_in_use: list[str]) -> dict:
    """The report's budget block. Wall seconds are finalized when the report is written."""
    s = guard.settings
    totals = campaign_totals(comparisons)
    return {
        "tier": s.tier, "note": s.report_note,
        "caps": {"wall_seconds": s.campaign_wall_limit_seconds, "usd": s.campaign_budget_usd, "tokens": s.campaign_token_budget,
                 "dollar_margin_usd": s.dollar_margin_usd, "post_trial_reserve_seconds": s.post_trial_reserve_seconds,
                 "publication_reserve_seconds": s.publication_reserve_seconds},
        "actual": {"usd": totals["estimated_usd"], "tokens": totals["total_tokens"], "wall_seconds": None},
        "wall_within_cap": None,
        "cost_verdict": cost_verdict(s, models_in_use, totals["estimated_usd"]),
        "dollar_cap_basis": DOLLAR_CAP_BASIS,
        "stop_events": guard.events, "cut_off_steps": guard.cut_off_steps,
    }
