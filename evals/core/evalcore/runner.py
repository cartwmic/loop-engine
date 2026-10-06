"""One Inspect sample per matched (combination, case, trial)."""
import asyncio
import importlib.metadata
import platform
import subprocess
import time
import traceback
from pathlib import Path

from inspect_ai import Task, eval
from inspect_ai.model import GenerateConfig, get_model

from . import pi  # register the host SDK provider even before wheel metadata refresh
from inspect_ai.scorer import Score, scorer
from inspect_ai.solver import solver

from .accounting import CLASSIFICATION_RULES, TrialRecord, classify_exception, core_metrics
from .report import protected_values, write_report
from .settings import CampaignSettings, OVERHEAD_ROLES, matched_assignment
from .target import Target, TrialContext


def harness_versions() -> dict:
    repo = Path(__file__).resolve().parents[3]
    def git(*args):
        result = subprocess.run(["git", "-C", str(repo), *args], capture_output=True, text=True)
        return result.stdout.strip() if result.returncode == 0 else "unknown"
    return {"inspect_ai": importlib.metadata.version("inspect-ai"),
            "evalcore": importlib.metadata.version("loop-engine-evals"),
            "python": platform.python_version(), "checkout_commit": git("rev-parse", "HEAD"),
            "checkout_dirty": bool(git("status", "--porcelain"))}


@solver
def trial_solver(target: Target, contexts: dict[str, TrialContext], records: dict[str, TrialRecord]):
    async def solve(state, generate):
        context = contexts[str(state.sample_id)]
        assignment = context.assignment
        record = TrialRecord(**assignment.model_dump(), outcome="failure", reason="not_completed")
        records[assignment.sample_id] = record
        started = time.monotonic()
        try:
            # Include scoring in the same limit. Target subprocesses must also honour it.
            async with asyncio.timeout(context.settings.hard_limit_seconds):
                execution = await target.run_trial(state, context)
                record.terminal = execution.terminal
                scored = await target.score(state, execution, context)
                record.target_metrics = scored.metrics
                record.outcome = "success" if execution.terminal and scored.success else "failure"
                record.reason = "passed" if record.outcome == "success" else "target_failure"
        except Exception as error:
            record.outcome, record.reason = classify_exception(error)
            # The exception class name is safe to report; the full traceback may
            # contain private case text, so it stays in the ignored local logs.
            record.error_type = type(error).__name__
            errors = Path(target.logs_dir) / "errors"
            errors.mkdir(parents=True, exist_ok=True)
            (errors / f"{context.settings.campaign}-{assignment.sample_id}.txt").write_text(
                "".join(traceback.format_exception(error)))
        finally:
            record.wall_seconds = time.monotonic() - started
        state.completed = True
        return state
    return solve


@scorer(metrics=[])
def trial_scorer(records: dict[str, TrialRecord]):
    async def score(state, target):
        record = records[str(state.sample_id)]
        return Score(value=int(record.outcome == "success"), metadata={"outcome": record.outcome, "reason": record.reason})
    return score


def run_campaign(target: Target, settings: CampaignSettings, *, provider_config_paths: tuple[Path, ...] = (), report_directory: Path | None = None) -> Path:
    if report_directory is not None and report_directory.exists():
        raise FileExistsError("report directory already exists")
    provider_config_paths = (*settings.provider_config_paths, *provider_config_paths,
                             *getattr(target, "provider_config_paths", ()))
    # Validate readability before executing. Re-scan values at write time as well.
    protected_values(provider_config_paths=provider_config_paths)
    cases, assignments = matched_assignment(settings, target.load_cases(settings.cases_path))
    case_lookup = {c.id: c for c in cases}
    records: dict[str, TrialRecord] = {}
    tasks = []
    # Resolve all model providers before executing any trial. No generation here.
    for combo in settings.resolved_combinations():
        model_ids = {**combo.role_models(), **dict(zip(OVERHEAD_ROLES, (settings.simulator_model, settings.judge_model)))}
        role_models = {role: get_model(
            model_id, role=role, memoize=False,
            config=GenerateConfig(**settings.model_generation[model_id].model_dump())
            if model_id in settings.model_generation else GenerateConfig(),
            **settings.model_args.get(model_id, {})) for role, model_id in model_ids.items()}
        contexts = {a.sample_id: TrialContext(settings, a, case_lookup[a.case_id], combo)
                    for a in assignments if a.combination_id == combo.id}
        samples = []
        for context in contexts.values():
            sample = target.prepare_sample(context)
            sample.id = context.assignment.sample_id
            samples.append(sample)
        tasks.append(Task(
            name=f"{target.name}-{len(tasks)}", dataset=samples,
            solver=trial_solver(target, contexts, records), scorer=trial_scorer(records),
            model=role_models["role-driver"], model_roles=role_models,
            fail_on_error=False, time_limit=settings.hard_limit_seconds,
        ))
    started = time.monotonic()
    logs = eval(tasks, log_dir=str(target.logs_dir), display="none", retry_on_error=0,
                log_model_api=False, max_tasks=1)
    sample_logs = {str(s.id): s for log in logs for s in log.samples or []}
    for assignment in assignments:
        sample = sample_logs.get(assignment.sample_id)
        record = records.get(assignment.sample_id)
        if record is None:
            record = TrialRecord(**assignment.model_dump(), outcome="infrastructure", reason="inspect_sample_unavailable")
            records[assignment.sample_id] = record
        if sample is not None:
            record.wall_seconds = sample.total_time or record.wall_seconds
            record.usage_by_role = {role: usage.model_dump() for role, usage in sample.role_usage.items()}
            if sample.limit:
                record.outcome = "failure"
                record.reason = "timeout" if sample.limit.type in ("time", "working") else "limit"
            elif sample.error:
                # All target/model exceptions were caught inside our solver. An error
                # outside it is Inspect setup/scorer/cleanup infrastructure. EvalError
                # has no typed category; never infer one from untrusted traceback text.
                record.outcome = "infrastructure"
                record.reason = "inspect_lifecycle_error"
    ordered_records = [records[a.sample_id] for a in assignments]
    # Targets that need finalized alias usage/failure classification cannot obtain
    # it during sandbox scoring. Keep this optional and target-neutral.
    finalize = getattr(target, "finalize_metrics", None)
    if finalize is not None:
        for record in ordered_records:
            record.target_metrics = finalize(record.target_metrics, usage_by_role=record.usage_by_role,
                                             terminal=record.terminal, outcome=record.outcome, reason=record.reason)
    comparisons = []
    for combo in settings.resolved_combinations():
        trials = [r for r in ordered_records if r.combination_id == combo.id]
        roles = {**combo.role_models(), "simulator": settings.simulator_model, "judge": settings.judge_model}
        comparisons.append({"id": combo.id, "role_models": roles, "core": core_metrics(trials, settings, roles),
                            "target_metrics": target.report_metrics([r.target_metrics for r in trials if r.target_metrics])})
    summary = {
        "schema_version": 1, "campaign": settings.campaign, "target": target.name,
        "settings": settings.recorded(),
        "pinned_models": {"simulator": settings.simulator_model, "judge": settings.judge_model},
        "versions": {"harness": harness_versions(), "target": target.versions(settings),
                     "eval_cases": {c.id: c.version for c in cases}},
        "matched_assignment": [a.model_dump() for a in assignments],
        "classification_rules": CLASSIFICATION_RULES,
        "campaign_wall_seconds": time.monotonic() - started,
        "comparisons": comparisons, "trials": [r.recorded() for r in ordered_records],
    }
    return write_report(target.results_dir, summary, provider_config_paths=provider_config_paths,
                        report_directory=report_directory)
