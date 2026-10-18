"""The small boundary between campaign mechanics and a workflow target."""
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Protocol

from inspect_ai.dataset import Sample
from inspect_ai.solver import TaskState

from .settings import Assignment, CampaignSettings, Case, Combination


@dataclass(frozen=True)
class TrialContext:
    settings: CampaignSettings
    assignment: Assignment
    case: Case
    combination: Combination
    # Campaign guard (evalcore.limits.CampaignGuard) or None; targets use it for post-trial allowances.
    guard: Any = None


@dataclass
class TrialExecution:
    terminal: bool
    # Set by a target whose own driver was stopped by a budget (e.g. trial_time, campaign_dollars).
    # Core still scores what was captured, then records the trial as outcome 'limit'.
    stop_reason: str | None = None
    # Opaque host-side state, never exported by core. May hold sandbox handles.
    data: Any = None


@dataclass
class TrialScore:
    success: bool
    # Export only IDs, metrics and safe evidence; never private case text.
    metrics: dict = field(default_factory=dict)


class InfrastructureError(RuntimeError):
    """Harness/container/bridge/provider-service failure, not model behaviour."""


class LimitReached(RuntimeError):
    """A budget or deadline ended this step. Core records outcome 'limit' with this reason and step."""

    def __init__(self, reason: str, step: str):
        super().__init__(f"{reason}:{step}")
        self.reason, self.step = reason, step


class OwnerApprovalRequired(ValueError):
    """A model placeholder requires owner selection and paid-call approval."""


class ModelError(RuntimeError):
    """Invalid model output, refusal, schema error, etc. Counts as failure."""


class Target(Protocol):
    name: str
    results_dir: Path  # campaign reports are written under results_dir/<campaign>
    logs_dir: Path     # raw Inspect logs must be local and gitignored
    # Optional attribute: provider_config_paths = (Path(...), ...), used by the scan.

    def load_cases(self, path: Path) -> list[Case]: ...

    def prepare_sample(self, context: TrialContext) -> Sample:
        """Set input and optional sandbox; core pins the sample ID."""
        ...

    async def run_trial(self, state: TaskState, context: TrialContext) -> TrialExecution: ...

    async def score(self, state: TaskState, execution: TrialExecution, context: TrialContext) -> TrialScore: ...

    def versions(self, settings: CampaignSettings) -> dict[str, str]:
        """Actual/pinned target, engine and provider identities, not config contents."""
        ...

    def report_metrics(self, scores: list[dict]) -> dict:
        """Target-specific per-combination aggregation, including calibration if applicable."""
        ...
