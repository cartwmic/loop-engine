"""No network/container calls: real Inspect samples and deterministic ModelAPI roles."""
import asyncio
import os
import subprocess
from pathlib import Path

from inspect_ai.dataset import Sample
from inspect_ai.model import ModelAPI, ModelCall, ModelOutput, ModelUsage, get_model, modelapi

from evalcore.settings import Case
from evalcore.target import InfrastructureError, LimitReached, ModelError, TrialExecution, TrialScore


@modelapi(name="fake")
class FakeModelAPI(ModelAPI):
    async def generate(self, input, tools, tool_choice, config):
        if self.model_name in ("service-error", "bad-request"):
            error = RuntimeError("synthetic provider error")
            error.status_code = 503 if self.model_name == "service-error" else 400
            return error, ModelCall(request={"model": self.model_name}, error=True)
        content = self.model_name + (f":{config.reasoning_effort}" if config.reasoning_effort else "")
        output = ModelOutput.from_content(model=self.model_name, content=content)
        if self.model_name == "costly":  # one call is a million input tokens
            output.usage = ModelUsage(input_tokens=1_000_000, output_tokens=0, total_tokens=1_000_000)
        else:
            output.usage = ModelUsage(input_tokens=10, output_tokens=5, total_tokens=15)
        return output


class FakeTarget:
    name = "fake"

    def __init__(self):
        root = Path(os.environ["EVAL_FAKE_ROOT"])
        self.results_dir = root / "results"
        self.logs_dir = root / "logs"

    def load_cases(self, path):
        return [Case(id=f"case-{i}", version=f"v{i}.0", path=path / f"case-{i}") for i in range(1, 7)]

    def prepare_sample(self, context):
        if context.settings.target_options.get("block_setup"):
            # A synchronous setup probe that never returns, bounded the way a real target bounds it.
            try:
                subprocess.run(["sleep", "600"], timeout=context.guard.allowance(30))
            except subprocess.TimeoutExpired:
                raise LimitReached(context.guard.step_stop_reason(), "image_inspection") from None
        return Sample(input=f"fake request {context.case.id}")

    async def run_trial(self, state, context):
        options = context.settings.target_options
        if options.get("sleep"):
            await asyncio.sleep(2)
        # Proof hooks: extra driver calls over time (for dollar stops) and a long wait (for time stops).
        for _ in range(options.get("late_costly_calls", 0)):
            await get_model(role="role-driver").generate("late costly call")  # lands in the trial's last moments
        for _ in range(options.get("extra_driver_calls", 0)):
            await get_model(role="role-driver").generate("extra call")
            await asyncio.sleep(options.get("call_gap_seconds", 0.5))
        if options.get("wait_seconds"):
            await asyncio.sleep(options["wait_seconds"])
        roles = {}
        for role in ("role-driver", "role-implementer", "role-reviewer", "simulator", "judge"):
            output = await get_model(role=role).generate("deterministic usage probe")
            roles[role] = output.completion
        if context.settings.target_options.get("raise_unknown"):
            raise KeyError("intent.json")
        if context.settings.target_options.get("inject_errors", True):
            if context.case.id == "case-1" and context.assignment.trial == 1:
                raise InfrastructureError("injected container failure")
            if context.case.id == "case-2" and context.assignment.trial == 1:
                raise TimeoutError("injected timeout")
            if context.case.id == "case-2" and context.assignment.trial == 2:
                raise ModelError("injected schema error")
        return TrialExecution(terminal=not context.settings.target_options.get("nonterminal", False), data=roles)

    async def score(self, state, execution, context):
        metrics = {"judge_model_calls_allowed": context.guard.model_calls_allowed() if context.guard else None,
                   "seen_roles": execution.data, "profile_used": context.settings.profile,
                   "case_version_used": context.case.version}
        if "score_text" in context.settings.target_options:
            metrics["scored_field"] = context.settings.target_options["score_text"]
        if context.settings.target_options.get("leak"):
            metrics["scored_field"] = os.environ["FAKE_EVAL_API_KEY"]
        return TrialScore(success=context.case.id != "case-2", metrics=metrics)

    def versions(self, settings):
        if settings.target_options.get("block_versions"):
            import time as _t
            _t.sleep(600)  # a version probe that never returns
        return {"loop_engine": settings.loop_engine_version, "provider": settings.provider_version, "fake_target": "1"}

    def report_metrics(self, scores):
        return {"scored_trials": len(scores)}


def create_target():
    return FakeTarget()


def register_subcommands(subparsers):
    # Demonstrate a target-owned command without coupling it to core.
    parser = subparsers.add_parser("fake-check")
    parser.set_defaults(handler=lambda args: print("fake target registration works"))
