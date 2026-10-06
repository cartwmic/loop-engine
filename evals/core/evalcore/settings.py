"""Validated settings and a deterministic, matched assignment."""
from pathlib import Path
from typing import Literal

from pydantic import BaseModel, ConfigDict, Field, model_validator

WORKFLOW_ROLES = ("role-driver", "role-implementer", "role-reviewer")
OVERHEAD_ROLES = ("simulator", "judge")
PRESETS = {"small": (3, 3), "medium": (6, 3)}


class StrictModel(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True)


class Price(StrictModel):
    input_per_million: float = Field(ge=0, allow_inf_nan=False)
    output_per_million: float = Field(ge=0, allow_inf_nan=False)
    # Cached input is billed too; defaults to the input price when unset.
    cache_read_per_million: float | None = Field(default=None, ge=0, allow_inf_nan=False)
    cache_write_per_million: float | None = Field(default=None, ge=0, allow_inf_nan=False)


class Combination(StrictModel):
    id: str = Field(min_length=1)
    driver: str = Field(pattern=r"^[^/\s]+/\S+$")
    implementer: str = Field(pattern=r"^[^/\s]+/\S+$")
    reviewer: str = Field(pattern=r"^[^/\s]+/\S+$")

    def role_models(self) -> dict[str, str]:
        return dict(zip(WORKFLOW_ROLES, (self.driver, self.implementer, self.reviewer)))


class ModelGenerationSettings(StrictModel):
    # Keep the first per-model surface deliberately small; add knobs as needed.
    reasoning_effort: Literal["none", "minimal", "low", "medium", "high", "xhigh", "max"]


class CampaignSettings(StrictModel):
    target: str = "scevals.target:create_target"
    campaign: str = Field(default="campaign", pattern=r"^[A-Za-z0-9][A-Za-z0-9_.-]*$")
    models: tuple[str, ...] = ()
    combinations: tuple[Combination, ...] = ()
    preset: Literal["small", "medium"] = "small"
    case_count: int | None = Field(default=None, ge=1)
    trials_per_case: int | None = Field(default=None, ge=1)
    profile: str = "standard-14"
    cases_path: Path
    simulator_model: str = Field(pattern=r"^[^/\s]+/\S+$")
    judge_model: str = Field(pattern=r"^[^/\s]+/\S+$")
    idle_limit_seconds: int = Field(default=1800, ge=1)
    hard_limit_seconds: int = Field(default=28800, ge=1)
    prices: dict[str, Price] = Field(default_factory=dict)
    # Per-model Inspect provider arguments, e.g. {"openrouter/x" = {strict_tools = false}}.
    model_args: dict[str, dict] = Field(default_factory=dict)
    # Pydantic reserves model_config for class configuration; TOML/report alias.
    model_generation: dict[str, ModelGenerationSettings] = Field(default_factory=dict, alias="model_config")
    # Read on the host for the pre-write scan; contents are never serialized.
    provider_config_paths: tuple[Path, ...] = ()
    loop_engine_version: str = "v0.24.0"
    provider_version: str = "v0.24.0"
    # Target-specific options (e.g. a driver command) are not interpreted by core.
    target_options: dict = Field(default_factory=dict)

    @model_validator(mode="after")
    def validate_models(self):
        if bool(self.models) == bool(self.combinations):
            raise ValueError("supply models OR per-role combinations (not both)")
        combos = self.resolved_combinations()
        if len({c.id for c in combos}) != len(combos):
            raise ValueError("combination IDs must be unique")
        models = [self.simulator_model, self.judge_model,
                  *(model for combo in combos for model in combo.role_models().values())]
        if any(key not in models for key in self.model_generation):
            raise ValueError("model_config keys must name a configured role model")
        if any(model.startswith("pi/") for model in models):
            from .pi import pi_config_paths
            paths = tuple(dict.fromkeys((*self.provider_config_paths, *pi_config_paths())))
            object.__setattr__(self, "provider_config_paths", paths)
        if any(model.startswith("codex/") for model in models):
            from .codex import codex_auth_path
            paths = tuple(dict.fromkeys((*self.provider_config_paths, codex_auth_path())))
            object.__setattr__(self, "provider_config_paths", paths)
        return self

    @property
    def counts(self) -> tuple[int, int]:
        cases, trials = PRESETS[self.preset]
        return self.case_count or cases, self.trials_per_case or trials

    def resolved_combinations(self) -> tuple[Combination, ...]:
        if self.combinations:
            return self.combinations
        return tuple(Combination(id=m, driver=m, implementer=m, reviewer=m) for m in self.models)

    def recorded(self) -> dict:
        data = self.model_dump(mode="json", by_alias=True)
        data["effective_case_count"], data["effective_trials_per_case"] = self.counts
        data["resolved_cases_path"] = str(self.cases_path.resolve())
        data["resolved_combinations"] = [c.model_dump() for c in self.resolved_combinations()]
        data["setting_sources"] = {
            field.alias or key: "explicit" if key in self.model_fields_set else "default"
            for key, field in type(self).model_fields.items()
        }
        return data


class Case(StrictModel):
    id: str = Field(min_length=1)
    version: str = Field(min_length=1)
    path: Path


class Assignment(StrictModel):
    sample_id: str
    combination_id: str
    case_id: str
    case_version: str
    trial: int


def matched_assignment(settings: CampaignSettings, cases: list[Case]) -> tuple[list[Case], list[Assignment]]:
    count, k = settings.counts
    if len({c.id for c in cases}) != len(cases):
        raise ValueError("case IDs must be unique")
    if len(cases) < count:
        raise ValueError(f"requested {count} cases but target supplied {len(cases)}")
    selected = sorted(cases, key=lambda c: c.id)[:count]
    assignments = []
    for combo_index, combo in enumerate(settings.resolved_combinations()):
        for case_index, case in enumerate(selected):
            for trial in range(1, k + 1):
                assignments.append(Assignment(
                    sample_id=f"m{combo_index}-c{case_index}-t{trial}",
                    combination_id=combo.id, case_id=case.id, case_version=case.version, trial=trial,
                ))
    return selected, assignments
