"""One Inspect sample per matched (combination, case, trial)."""
import asyncio
from contextlib import nullcontext
import importlib.metadata
import platform
import subprocess
import threading
import time
import traceback
from pathlib import Path

from inspect_ai import Task, eval
from inspect_ai.model import GenerateConfig, get_model

from . import pi  # register the host SDK provider even before wheel metadata refresh
from inspect_ai.model._model import sample_role_usage  # live per-sample usage, used for the dollar stop
from inspect_ai.scorer import Score, scorer
from inspect_ai.solver import solver
import anyio

from .accounting import CLASSIFICATION_RULES, TrialRecord, classify_exception, core_metrics, events_spend
from .limits import CampaignGuard, budget_summary, watch_trial
from .provenance import cohort
from .publish import publish
from .report import UnsafeReportError, protected_values, utc_timestamp
from .settings import CampaignSettings, OVERHEAD_ROLES, matched_assignment
from .target import LimitReached, Target, TrialContext


CAMPAIGN_STOP_REASONS = ("trial_time", "campaign_wall_time", "campaign_dollars", "campaign_tokens")
PROBE_TIMEOUT_SECONDS = 10  # version probes must never hold the command past the cap


INSPECT_BACKSTOP_SECONDS = 10
SUMMARY_SAFETY_SECONDS = 5  # kept free after summary preparation so the limit-only record can still be written


def harness_versions(budget: float | None = None) -> dict:
    """Version facts. With a budget (seconds) the git probes share it and give up when it is spent."""
    repo = Path(__file__).resolve().parents[3]
    end = None if budget is None else time.monotonic() + budget
    def git(*args):
        timeout = PROBE_TIMEOUT_SECONDS if end is None else min(PROBE_TIMEOUT_SECONDS, end - time.monotonic())
        if timeout <= 0:
            return "unknown"
        try:
            result = subprocess.run(["git", "-C", str(repo), *args], capture_output=True, text=True, timeout=timeout)
        except subprocess.TimeoutExpired:
            return "unknown"
        return result.stdout.strip() if result.returncode == 0 else "unknown"
    return {"inspect_ai": importlib.metadata.version("inspect-ai"),
            "evalcore": importlib.metadata.version("loop-engine-evals"),
            "python": platform.python_version(), "checkout_commit": git("rev-parse", "HEAD"),
            "checkout_dirty": bool(git("status", "--porcelain"))}


def prepare_versions(target, settings: CampaignSettings, guard: CampaignGuard) -> tuple[dict, dict]:
    """(harness versions, target versions) inside the time left before the cap, never holding the command past it.
    A step that runs out is named in the report as cut off."""
    remaining = guard.remaining_to_deadline()
    budget = None if remaining is None else max(0.0, remaining - SUMMARY_SAFETY_SECONDS)
    harness = harness_versions(budget)
    box: dict = {}
    def work():
        try:
            box["value"] = target.versions(settings)
        except BaseException as error:  # reported as unavailable, never raised into the summary
            box["error"] = type(error).__name__
    worker = threading.Thread(target=work, daemon=True)
    worker.start()
    left = guard.remaining_to_deadline()
    worker.join(timeout=None if left is None else max(0.0, left - SUMMARY_SAFETY_SECONDS))
    if "value" in box:
        return harness, box["value"]
    guard.note_cut_off("version_probe", guard.step_stop_reason())
    return harness, {"unavailable": "version probe " + ("failed (" + box["error"] + ")" if "error" in box else "ran out of time")}


def live_spend_usd(settings: CampaignSettings, role_models: dict) -> float | None:
    """This trial's spend so far, from its own model-call events (tiered by request size). None if unavailable."""
    from inspect_ai.log import transcript
    spend = events_spend(transcript().events, role_models, settings.prices)
    return None if spend is None else sum(spend.values())


def limit_step_of(error: BaseException) -> str | None:
    seen = set()
    while error is not None and id(error) not in seen:
        seen.add(id(error))
        if isinstance(error, LimitReached):
            return error.step
        error = error.__cause__ or error.__context__
    return None


async def run_trial_stoppable(target: Target, state, context: TrialContext, guard: CampaignGuard, role_models: dict):
    """Run the model-driven part with the campaign watcher beside it. A target that ignores a stop is cancelled after the grace."""
    execution = None
    try:
        with anyio.CancelScope() as trial_scope:
            async with anyio.create_task_group() as group:
                if guard.active:
                    group.start_soon(watch_trial, guard, role_models,
                                     lambda: {role: usage.model_dump() for role, usage in sample_role_usage().items()}, trial_scope,
                                     lambda: live_spend_usd(context.settings, role_models))
                try:
                    execution = await target.run_trial(state, context)
                finally:
                    group.cancel_scope.cancel()
    except BaseExceptionGroup as errors:
        # anyio wraps even a single error; classification needs the original exception.
        error = errors
        while isinstance(error, BaseExceptionGroup) and len(error.exceptions) == 1:
            error = error.exceptions[0]
        raise error
    if execution is None:
        raise LimitReached(guard.step_stop_reason(), "trial_work")
    return execution


@solver
def trial_solver(target: Target, contexts: dict[str, TrialContext], records: dict[str, TrialRecord], guard: CampaignGuard):
    async def solve(state, generate):
        context = contexts[str(state.sample_id)]
        assignment = context.assignment
        record = TrialRecord(**assignment.model_dump(), outcome="failure", reason="not_completed")
        records[assignment.sample_id] = record
        started = time.monotonic()
        execution = None
        role_models = {**context.combination.role_models(),
                       **dict(zip(OVERHEAD_ROLES, (context.settings.simulator_model, context.settings.judge_model)))}
        deadline = (asyncio.timeout(context.settings.hard_limit_seconds)
                    if context.settings.hard_limit_seconds is not None else None)
        try:
            if not guard.can_start_trial():
                raise LimitReached(guard.stop_reason, "not_started")
            # Include scoring in the same limit. Target subprocesses must also honour it.
            async with deadline or nullcontext():
                execution = await run_trial_stoppable(target, state, context, guard, role_models)
                # The watcher ended with the trial; look once more now, so spend or time reached in its last seconds stops
                # every later model call (the judge) and trial.
                reason = guard.check(role_models, {role: usage.model_dump() for role, usage in sample_role_usage().items()},
                                     live_spend_usd(context.settings, role_models))
                if reason:
                    guard.fire(reason)
                record.terminal = execution.terminal
                scored = await target.score(state, execution, context)
                record.terminal = execution.terminal
                record.target_metrics = scored.metrics
                record.outcome = "success" if execution.terminal and scored.success else "failure"
                record.reason = "passed" if record.outcome == "success" else "target_failure"
                if execution.stop_reason:
                    record.outcome, record.reason = "limit", execution.stop_reason
        except Exception as error:
            if execution is not None:
                record.terminal = execution.terminal
            if hasattr(target, "failure_metrics"):
                record.target_metrics = target.failure_metrics(context)
            record.outcome, record.reason = classify_exception(error)
            record.limit_step = limit_step_of(error)
            if isinstance(error, TimeoutError) and deadline is not None and deadline.expired():
                # The per-trial hard limit is a budget stop, not a model or harness failure.
                record.outcome, record.reason = "limit", "trial_time"
            if record.outcome == "limit":
                guard.note_cut_off(record.limit_step or "trial", record.reason, assignment.sample_id)
            # The exception class name is safe to report; the full traceback may
            # contain private case text, so it stays in the ignored local logs.
            record.error_type = type(error).__name__
            errors = Path(target.logs_dir) / "errors"
            errors.mkdir(parents=True, exist_ok=True)
            (errors / f"{context.settings.campaign}-{assignment.sample_id}.txt").write_text(
                "".join(traceback.format_exception(error)))
        finally:
            record.wall_seconds = time.monotonic() - started
            if record.outcome == "limit" and record.reason in CAMPAIGN_STOP_REASONS:
                guard.fire(record.reason)  # a per-trial time stop also stops later trials and model calls
            from inspect_ai.log import transcript
            record.priced_usd_by_role = events_spend(transcript().events, role_models, context.settings.prices)
            guard.add_finished(role_models, {role: usage.model_dump() for role, usage in sample_role_usage().items()},
                               record.priced_usd_by_role)
            if hasattr(target, "checkpoint_record"):
                target.checkpoint_record(context, record)
        state.completed = True
        return state
    return solve


@scorer(metrics=[])
def trial_scorer(records: dict[str, TrialRecord]):
    async def score(state, target):
        record = records[str(state.sample_id)]
        return Score(value=int(record.outcome == "success"), metadata={"outcome": record.outcome, "reason": record.reason})
    return score


def run_campaign(target: Target, settings: CampaignSettings, *, provider_config_paths: tuple[Path, ...] = (), report_directory: Path | None = None,
                 carried_records: list[TrialRecord] = (), assignments_override=None, clock_start: float | None = None) -> Path:
    # The campaign clock starts when the evalctl command starts (the CLI passes it); a direct call starts it here.
    guard = CampaignGuard(settings, clock_start)
    if report_directory is not None and report_directory.exists():
        raise FileExistsError("report directory already exists")
    provider_config_paths = (*settings.provider_config_paths, *provider_config_paths,
                             *getattr(target, "provider_config_paths", ()))
    # Validate readability before executing. Re-scan values at write time as well.
    protected_values(provider_config_paths=provider_config_paths)
    # Strict label validation before any execution; unlabelled settings are the explicit baseline cohort.
    projection_cohort = cohort(settings)
    cases, assignments = matched_assignment(settings, target.load_cases(settings.cases_path))
    if assignments_override is not None:
        assignments = assignments_override
    case_lookup = {c.id: c for c in cases}
    if hasattr(target, "begin_run"):
        target.begin_run(settings, utc_timestamp())
        print(f"Captures retained: {target.run_directory}", flush=True)
        import json
        manifest = {"settings": settings.recorded(), "matched_assignment": [a.model_dump() for a in assignments],
                    "sources": {k: str(v) for k, v in getattr(target, "sources", {}).items()}}
        if getattr(target, "execution_continuation", None):
            manifest["execution_continuation"] = target.execution_continuation
        (target.run_directory / "run.json").write_text(json.dumps(manifest, indent=2) + "\n")
    records: dict[str, TrialRecord] = {r.sample_id: r for r in carried_records}
    carried_ids = set(records)
    tasks = []
    setup_limit = None
    try:
        # Resolve all model providers before executing any trial. No generation here.
        for combo in settings.resolved_combinations():
            model_ids = {**combo.role_models(), **dict(zip(OVERHEAD_ROLES, (settings.simulator_model, settings.judge_model)))}
            contexts = {a.sample_id: TrialContext(settings, a, case_lookup[a.case_id], combo, guard)
                        for a in assignments if a.combination_id == combo.id and a.sample_id not in carried_ids}
            if not contexts:
                continue
            role_models = {role: get_model(
                model_id, role=role, memoize=False,
                config=GenerateConfig(**settings.model_generation[model_id].model_dump())
                if model_id in settings.model_generation else GenerateConfig(),
                **settings.model_args.get(model_id, {})) for role, model_id in model_ids.items()}
            samples = []
            for context in contexts.values():
                sample = target.prepare_sample(context)
                sample.id = context.assignment.sample_id
                samples.append(sample)
            tasks.append(Task(
                name=f"{target.name}-{len(tasks)}", dataset=samples,
                solver=trial_solver(target, contexts, records, guard), scorer=trial_scorer(records),
                model=role_models["role-driver"], model_roles=role_models,
                fail_on_error=False,
            # The solver enforces the hard limit itself (so it can stop later trials); Inspect's own limit is only the backstop.
            time_limit=None if settings.hard_limit_seconds is None else settings.hard_limit_seconds + INSPECT_BACKSTOP_SECONDS,
            ))
    except LimitReached as error:
        # A synchronous setup step ran out of time: no trial starts and the campaign ends as a limit, step named.
        guard.fire(error.reason)
        guard.note_cut_off(error.step, error.reason)
        setup_limit = {"reason": error.reason, "step": error.step}
        tasks = []
    started = time.monotonic()
    logs = eval(tasks, log_dir=str(target.logs_dir), display="none", retry_on_error=0,
                log_model_api=False, max_tasks=1,
                max_samples=1) if tasks else []  # sequential: the guard sees one trial's live spend at a time
    sample_logs = {str(s.id): s for log in logs for s in log.samples or []}
    for assignment in assignments:
        if assignment.sample_id in carried_ids:
            continue
        sample = sample_logs.get(assignment.sample_id)
        record = records.get(assignment.sample_id)
        if record is None:
            if setup_limit:
                record = TrialRecord(**assignment.model_dump(), outcome="limit", reason=setup_limit["reason"], limit_step=setup_limit["step"])
            else:
                record = TrialRecord(**assignment.model_dump(), outcome="infrastructure", reason="inspect_sample_unavailable")
            records[assignment.sample_id] = record
        if sample is not None:
            record.wall_seconds = sample.total_time or record.wall_seconds
            record.usage_by_role = {role: usage.model_dump() for role, usage in sample.role_usage.items()}
            if sample.limit:
                record.outcome = "limit"
                record.reason = "trial_time" if sample.limit.type in ("time", "working") else sample.limit.type
            elif sample.error:
                # All target/model exceptions were caught inside our solver. An error
                # outside it is Inspect setup/scorer/cleanup infrastructure. EvalError
                # has no typed category; never infer one from untrusted traceback text.
                record.outcome = "infrastructure"
                record.reason = "inspect_lifecycle_error"
    # Replay retains original workflow/simulator usage from host meta. Only the
    # newly run judge replaces prior judge overhead; no fictitious workflow calls.
    if hasattr(target, "sources"):
        import json
        for sample_id, source in target.sources.items():
            path = Path(source) / "meta.json"
            if path.is_file() and sample_id in records:
                meta = json.loads(path.read_text())
                record = records[sample_id]
                previous = meta.get("usage_by_role", {})
                if meta.get("original_usage_available") is False:
                    record.usage_unknown_roles = ["role-driver", "role-implementer", "role-reviewer", "simulator"]
                if getattr(target, "operation", "campaign") == "rescore":
                    record.usage_by_role = {**previous, **record.usage_by_role}
                    record.wall_seconds = meta.get("wall_seconds", record.wall_seconds)
                else:
                    for role, usage in previous.items():
                        current = record.usage_by_role.setdefault(role, {})
                        for key, value in usage.items():
                            if isinstance(value, (int, float)):
                                current[key] = (current.get(key) or 0) + value
                record.resumed = meta.get("resumed", False) or getattr(target, "operation", "campaign") != "rescore"
                record.priced_usd_by_role = None  # earlier segments have no per-call figures; use the upper bound
                record.segments = meta.get("segments", [])
    ordered_records = [records[a.sample_id] for a in assignments]
    # Targets that need finalized alias usage/failure classification cannot obtain
    # it during sandbox scoring. Keep this optional and target-neutral.
    finalize = getattr(target, "finalize_metrics", None)
    if finalize is not None:
        for record in ordered_records:
            if record.sample_id in carried_ids:
                continue
            record.target_metrics = finalize(record.target_metrics, usage_by_role=record.usage_by_role,
                                             terminal=record.terminal, outcome=record.outcome, reason=record.reason)
    if hasattr(target, "persist_record"):
        for record in ordered_records:
            if record.sample_id not in carried_ids:
                target.persist_record(record)
    comparisons = []
    for combo in settings.resolved_combinations():
        trials = [r for r in ordered_records if r.combination_id == combo.id]
        roles = {**combo.role_models(), "simulator": settings.simulator_model, "judge": settings.judge_model}
        comparisons.append({"id": combo.id, "role_models": roles, "core": core_metrics(trials, settings, roles),
                            "target_metrics": target.report_metrics([r.target_metrics for r in trials if r.target_metrics])})
    harness_info, target_versions = prepare_versions(target, settings, guard)
    models_in_use = sorted({m for c in comparisons for m in c["role_models"].values()})
    summary = {
        "schema_version": 1, "campaign": settings.campaign, "target": target.name,
        "projection_cohort": projection_cohort,
        "settings": settings.recorded(),
        "pinned_models": {"simulator": settings.simulator_model, "judge": settings.judge_model},
        "versions": {"harness": harness_info, "target": target_versions,
                     "eval_cases": {c.id: c.version for c in cases}},
        "matched_assignment": [a.model_dump() for a in assignments],
        "classification_rules": CLASSIFICATION_RULES,
        "campaign_wall_seconds": None,  # finalized when the report is written (command start to report written)
        "budget": budget_summary(guard, comparisons, models_in_use),
        "setup_limit": setup_limit,
        "comparisons": comparisons, "trials": [r.recorded() for r in ordered_records],
        "capture_run": str(target.run_directory) if hasattr(target, "run_directory") else None,
        "sources": {**{k: str(v) for k, v in getattr(target, "carried_sources", {}).items()},
                    **{k: str(v) for k, v in getattr(target, "sources", {}).items()}},
        "usage_basis": "captured host meta.json (Inspect role_usage); replay uses current judge usage; external snapshots have unknown original usage",
        "operation": getattr(target, "operation", "campaign"),
        "resumed_trials": [r.recorded() for r in ordered_records if r.resumed],
        "carried_trials": [{"sample_id": r.sample_id,
                           "from_run": str(getattr(target, "carried_sources", {}).get(r.sample_id, r.carried_from))}
                          for r in ordered_records if r.sample_id in carried_ids],
        "execution_continuation": getattr(target, "execution_continuation", None),
    }
    if hasattr(target, "run_directory"):
        import json
        # This is private, never a report-export bypass.
        (target.run_directory / "records.json").write_text(json.dumps(summary, indent=2) + "\n")
    try:
        return publish(target.results_dir, summary, provider_config_paths=provider_config_paths,
                       report_directory=report_directory, deadline=guard.deadline, command_start=guard.started,
                       minimal_only=setup_limit is not None)
    except UnsafeReportError:
        if hasattr(target, "run_directory"):
            print(f"Private captures kept. Rescore with: evalctl rescore {target.run_directory}", flush=True)
        raise
