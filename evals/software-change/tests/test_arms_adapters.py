"""Adapter launch proofs: each arm's real role script through real Pi and the public bridge, in the arms image.

Real Pi (1.2.0) runs the arm launcher (arm-native, arm-superpowers, arm-openspec) or the unchanged sc-driver for the
Loop Engine arm, with the arm's own prompt from its descriptor. The pinned Superpowers v7.0.0 and OpenSpec 1.14.3
packages in the image are the ones loaded; there is no fixture package. The model is a scripted no-network backend
that records the skills and markers in the prompt it receives, asks the simulated owner through ask-owner, edits
labels.py through bash and exits; the test asserts the recorded setup. The harness scores each delivery with the
common completion and hidden-test rule and grades it with the independent grader through a scripted grader model.

Mechanics proof only. No paid model, no network, no real workflow behaviour. Retained output:
tests/proofs/arms-adapters.txt.
"""
import io
import json
from pathlib import Path
import re
import tarfile
import tempfile
import time

import anyio
from inspect_ai import Task, eval
from inspect_ai.agent import sandbox_agent_bridge
from inspect_ai.dataset import Sample
from inspect_ai.model import (ChatCompletionChoice, ChatMessageAssistant, ModelAPI, ModelOutput, ModelUsage,
                              get_model, modelapi)
from inspect_ai.solver import solver
from inspect_ai.tool import ToolCall
from inspect_ai.util import sandbox, sandbox_service

from scevals.arms import is_workflow_owned, load_arm, render_prompt
from scevals.cases import load_case
from scevals.grader import CLAIM_INSTRUCTION, FIDELITY_INSTRUCTION, build_evidence, grade
from scevals.owner import SimulatedOwner
from scevals.scoring_common import score_common
from scevals.scripted_arm_driver import CORRECT_LABELS
from scevals.target import (ARM_BASELINE_COMMAND, CAPTURE_TAR_COMMAND, TerminalTrial, captured_files,
                            repository_files, requirement_statuses, run_hidden_tests)

ROOT = Path(__file__).resolve().parents[1]
CASE = ROOT / "example-case"
CAPTURES = ROOT / "captures/T10/adapters"
PROOF = ROOT / "tests/proofs/arms-adapters.txt"
ARMS_IMAGE = "loop-engine-sc-evals:arms-test"
BRIDGE_PORT = 14142
QUESTION = "Which label rule applies to whitespace?"
FINAL_MESSAGE = "Label keys trim and collapse whitespace, casefold, and fall back to inbox."
GRADER = "arm-grader/gpt-scripted-judge"  # OpenAI family; the driver models are Anthropic-family names
PINS = {"pi": "1.2.0", "openspec": "1.14.3", "superpowers": "bb92a77741419a4ab5f06e711a283343f1ada0c3"}
REQUEST = (CASE / "public/request.md").read_text()
# Markers only Pi's own loading produces: the skill list in the system prompt (<name> tags) and the Superpowers bootstrap
# message the pinned extension injects. The request text never contains these tags.
SUPERPOWERS_BOOTSTRAP = "superpowers:using-superpowers bootstrap for pi"
SUPERPOWERS_SKILLS = ("<name>using-superpowers</name>", "<name>test-driven-development</name>")
OPENSPEC_SKILLS = ("<name>openspec-propose</name>", "<name>openspec-apply-change</name>")
LOOP_ENGINE_SKILLS = ("<name>using-loop-engine</name>", "<name>using-software-change-provider</name>")
SKILL_NAME = re.compile(r"<name>([^<]+)</name>")

# Every arm: its driver argv, its descriptor fields, the model name the bridge reaches, the markers it must show and
# must not show, and where its skills come from on disk (the pinned package, the init output or the shipped skills).
ARMS = {
    "native": {"driver": ["arm-native"], "workflow_owned_paths": [], "model": "claude-native", "skills_dir": None,
               "loaded": (), "absent": (SUPERPOWERS_BOOTSTRAP, *SUPERPOWERS_SKILLS, *OPENSPEC_SKILLS, *LOOP_ENGINE_SKILLS)},
    "superpowers": {"driver": ["arm-superpowers"], "workflow_owned_paths": ["docs/superpowers/**", ".superpowers/**"],
                    "model": "claude-superpowers", "skills_dir": "/opt/sc-evals/arms/superpowers/skills",
                    "loaded": (SUPERPOWERS_BOOTSTRAP, *SUPERPOWERS_SKILLS),
                    "absent": (*OPENSPEC_SKILLS, *LOOP_ENGINE_SKILLS)},
    "openspec": {"driver": ["arm-openspec"], "workflow_owned_paths": ["openspec/**", ".pi/**"],
                 "model": "claude-openspec", "skills_dir": "/workspace/public/repo/.pi/skills",
                 "loaded": OPENSPEC_SKILLS, "absent": (SUPERPOWERS_BOOTSTRAP, *SUPERPOWERS_SKILLS, *LOOP_ENGINE_SKILLS)},
    "loop-engine": {"driver": ["sc-driver"], "workflow_owned_paths": [], "model": "claude-loop-engine",
                    "skills_dir": "/opt/sc-evals/skills", "loaded": LOOP_ENGINE_SKILLS,
                    "absent": (SUPERPOWERS_BOOTSTRAP, *SUPERPOWERS_SKILLS, *OPENSPEC_SKILLS)},
}
MODEL_ARM = {spec["model"]: arm_id for arm_id, spec in ARMS.items()}
CALLS = []


def descriptor(arm_id):
    spec = ARMS[arm_id]
    return load_arm({"arm": {"id": arm_id, "driver_command": spec["driver"], "prompt_template": arm_id,
                             "workflow_owned_paths": spec["workflow_owned_paths"], "completion_rule": "driver-exit"}})


def bash(command, call_id):
    return ModelOutput(model="arm", choices=[ChatCompletionChoice(
        message=ChatMessageAssistant(content="", tool_calls=[ToolCall(
            id=call_id, function="bash", arguments={"command": command})]), stop_reason="tool_calls")],
        usage=ModelUsage(input_tokens=10, output_tokens=5, total_tokens=15))


@modelapi(name="arm-scripted")
class ArmScriptedAPI(ModelAPI):
    """One scripted driver per arm: checks its setup, asks the owner, edits labels.py, then ends with a final message."""

    async def generate(self, input, tools, tool_choice, config):
        arm_id = MODEL_ARM[self.model_name]
        results = [message.text for message in input if message.role == "tool"]
        CALLS.append({"arm": arm_id, "tool_results": len(results)})
        if not results:
            assert any(tool.name == "bash" for tool in tools), "real Pi must expose bash"
            prompt = "\n".join(message.text for message in input)
            system = "\n".join(message.text for message in input if message.role == "system")
            CALLS[-1]["setup_seen"] = {marker: marker in prompt for marker in ARMS[arm_id]["loaded"] + ARMS[arm_id]["absent"]}
            CALLS[-1]["skills_seen"] = sorted(set(SKILL_NAME.findall(system)))
            return bash(f'ask-owner "{QUESTION}"', "t10_ask")
        if len(results) == 1:
            assert "Trim whitespace" in results[0], "the owner answer must reach Pi's tool result"
            return bash("cat > /workspace/public/repo/labels.py <<'EOF'\n" + CORRECT_LABELS + "EOF\n", "t10_edit")
        assert len(results) == 2, "the scripted driver ends after its edit"
        output = ModelOutput.from_content(model=self.model_name, content=FINAL_MESSAGE)
        output.usage = ModelUsage(input_tokens=12, output_tokens=6, total_tokens=18)
        return output


@modelapi(name="arm-grader")
class ArmGraderAPI(ModelAPI):
    """Scripted independent grader: a fixed fidelity and claim reply, chosen by instruction, never by the arm."""

    async def generate(self, input, tools, tool_choice, config):
        payload = json.loads(input[1].text)
        if input[0].text == FIDELITY_INSTRUCTION:
            reply = {"requirements": [{"id": rid, "verdict": "satisfied", "probability": 0.9, "pointer": None,
                                       "null_reason": "scripted proof judges the changed file as a whole"}
                                      for rid in payload["requirement_ids"]],
                     "unrequested_changes": [], "intent_faithful": True, "intent_probability": 0.9}
        else:
            assert input[0].text == CLAIM_INSTRUCTION, "unknown grader instruction"
            reply = {"claims": [{"id": sentence["id"], "verdict": "supported"} for sentence in payload["sentences"]]}
        output = ModelOutput.from_content(model=self.model_name, content=json.dumps(reply))
        output.usage = ModelUsage(input_tokens=3, output_tokens=2, total_tokens=5)
        return output


async def copy_case_public(env):
    buffer = io.BytesIO()
    with tarfile.open(fileobj=buffer, mode="w") as archive:
        for file in sorted((CASE / "public").rglob("*")):
            if file.is_file():
                archive.add(file, arcname=file.relative_to(CASE / "public").as_posix())
    await env.write_file("/tmp/t10-public.tar", buffer.getvalue())
    result = await env.exec(["sh", "-c", 'mkdir -p /workspace/public && tar -xf /tmp/t10-public.tar -C /workspace/public'],
                            timeout=120)
    assert result.success, "copying the public case failed"


async def reset_workspace(env):
    """The harness's own starting state for an arm: the case, an origin remote and the baseline commit."""
    await env.exec(["sh", "-c", "rm -rf /workspace/public /workspace/trial /workspace/local-remote.git"], timeout=60)
    await copy_case_public(env)
    await env.exec(["mkdir", "-p", "/workspace/trial"], timeout=30)
    remote = await env.exec(["git", "init", "--bare", "-q", "/workspace/local-remote.git"], timeout=60)
    assert remote.success, "local remote setup failed"
    baseline = await env.exec(["sh", "-c", ARM_BASELINE_COMMAND], timeout=600)
    assert baseline.success, "arm baseline commit failed"


async def final_workspace(env, case):
    """The delivered workspace, read back through the same capture command the trial uses."""
    result = await env.exec(["sh", "-c", CAPTURE_TAR_COMMAND], cwd="/workspace", timeout=600)
    assert result.success, "workspace capture failed"
    archive = await env.read_file("/tmp/workspace.tar", text=False)
    path = Path(tempfile.mkdtemp(prefix="t10-adapter-")) / "workspace.tar"
    path.write_bytes(archive)
    return captured_files(path)


def delivered_files(captured):
    """The public repository as delivered: every captured file under public/repo except git's own store."""
    return {path[len("public/repo/"):]: data for path, data in captured.items()
            if path.startswith("public/repo/") and ".git" not in Path(path).parts}


async def run_arm(env, arm_id, case, driver_process):
    spec = ARMS[arm_id]
    arm = descriptor(arm_id)
    await reset_workspace(env)
    await env.write_file("/opt/sc-evals/driver_process.py", driver_process)
    await env.write_file("/workspace/trial/providers.toml",
                         '[providers.software-change]\ncommand = "software-change"\nargs = []\n')
    prompt = render_prompt(arm, profile="standard-16", run_id="eval-trial", scripted="", request=REQUEST)
    owner = SimulatedOwner(case, "sc-scripted/simulator")
    done, started = anyio.Event(), anyio.Event()
    instance = f"t10-{arm_id}"

    async def service():
        await sandbox_service("sc_owner", {"ask_owner": owner.ask_owner}, until=done.is_set,
                              sandbox=env, started=started, instance=instance)

    model = get_model(f"arm-scripted/{spec['model']}", role="role-driver", memoize=False)
    CALLS.clear()
    async with anyio.create_task_group() as group:
        group.start_soon(service)
        await started.wait()
        try:
            async with sandbox_agent_bridge(model_aliases={"role-driver": model}, port=BRIDGE_PORT):
                launch = await env.exec(
                    ["python3", "/opt/sc-evals/driver_process.py", *spec["driver"], prompt],
                    cwd="/workspace/public/repo",
                    env={"SC_BRIDGE_URL": f"http://localhost:{BRIDGE_PORT}/v1", "SC_OWNER_INSTANCE": instance,
                         "SC_PROFILE": "standard-16", "SC_PI_SESSION_DIR": "/workspace/trial/pi-sessions"},
                    timeout=900)
        finally:
            done.set()
    sessions = await env.exec(["sh", "-c", "find /workspace/trial/pi-sessions -name '*.jsonl' | wc -l"], timeout=30)
    skills_on_disk = []
    if spec["skills_dir"]:
        listing = await env.exec(["sh", "-c", f"ls {spec['skills_dir']}"], timeout=30)
        assert listing.success, listing.stderr
        skills_on_disk = sorted(listing.stdout.split())
    files = delivered_files(await final_workspace(env, case))
    evidence = {"started": time.monotonic(), "copy_log": [], "copy_out_log": []}
    trial = TerminalTrial(env, case, owner, evidence, True, None)
    tests = await run_hidden_tests(trial)
    scored = score_common(
        case, evidence={"driver_exit_code": launch.returncode}, delivered=files,
        tests=tests, workflow_owned=lambda path: is_workflow_owned(path, arm),
        revealed_requirement_ids=owner.receipt()["revealed_requirement_ids"], push_events=[],
        owner_question_count=len(owner.answers))
    graded = await grade(brief=case.brief.read_text(), requirement_ids=case.requirement_ids,
                         baseline=repository_files(case), final=files, final_message=launch.stdout,
                         hidden_tests=requirement_statuses(case, tests), visible_tests=None, arm=arm,
                         driver_model=f"arm-scripted/{spec['model']}", grader_model=GRADER)
    grader_input = build_evidence(brief=case.brief.read_text(), requirement_ids=case.requirement_ids,
                                  baseline=repository_files(case), final=files, final_message=launch.stdout, arm=arm)
    labels = files.get("labels.py")
    return {
        "arm": arm_id, "driver": spec["driver"], "exit_code": launch.returncode,
        "final_message": launch.stdout.strip(), "model_calls": len(CALLS),
        "setup_seen": next((call["setup_seen"] for call in CALLS if "setup_seen" in call), {}),
        "skills_seen": next((call["skills_seen"] for call in CALLS if "skills_seen" in call), []),
        "skills_on_disk": skills_on_disk,
        "owner_questions": len(owner.answers), "owner_revealed_requirement_ids": owner.receipt()["revealed_requirement_ids"],
        "labels_edited": labels == CORRECT_LABELS.encode(),
        "hidden_tests": {"passed": scored["hidden_test_outcome"]["passed"],
                         "total": scored["hidden_test_outcome"]["total_tests"]},
        "complete": scored["complete"], "success": scored["success"],
        "artifact_volume_changed_files": scored["discipline"]["artifact_volume"]["changed_files"],
        "grader": {"status": graded["status"], "intent_faithful": graded.get("intent", {}).get("counted_faithful"),
                   "grader_family": graded.get("grader_family"), "driver_family": graded.get("driver_family"),
                   "files_in_grader_input": sorted(grader_input.final.keys() | grader_input.baseline.keys()),
                   "leakage_residual_terms": graded.get("leakage", {}).get("residual_terms")},
        "pi_session_records": int(sessions.stdout.strip() or 0),
    }


@solver
def arm_adapter_proof():
    async def solve(state, generate):
        env = sandbox()
        case = load_case(CASE)
        driver_process = (ROOT / "scevals/driver_process.py").read_text()
        pins = {}
        for name, command in (("pi", ["pi", "--version"]), ("openspec", ["openspec", "--version"]),
                              ("superpowers", ["git", "-C", "/opt/sc-evals/arms/superpowers", "rev-parse", "HEAD"])):
            pins[name] = (await env.exec(command, timeout=120)).stdout.strip()
        arms = {}
        for arm_id in ARMS:
            arms[arm_id] = await run_arm(env, arm_id, case, driver_process)
        state.metadata["pins"] = pins
        state.metadata["arms"] = arms
        state.completed = True
        return state
    return solve


def test_four_arms_launch_through_real_pi_and_score_through_the_common_rule(tmp_path):
    compose = tmp_path / "compose.yaml"
    compose.write_text(f"services:\n  default:\n    image: {ARMS_IMAGE}\n    platform: linux/arm64\n"
                       "    working_dir: /workspace\n    command: [sleep, infinity]\n")
    CAPTURES.mkdir(parents=True, exist_ok=True)
    task = Task(name="t10-arms-adapters", dataset=[Sample(input=REQUEST)], solver=arm_adapter_proof(),
                sandbox=("docker", str(compose)), model=f"arm-scripted/{ARMS['native']['model']}", time_limit=3600)
    logs = eval(task, log_dir=str(CAPTURES / "logs"), display="none", retry_on_error=0,
                log_model_api=False, sandbox_cleanup=True)
    assert len(logs) == 1
    assert logs[0].status == "success", logs[0].error
    sample = logs[0].samples[0]
    assert sample.error is None, sample.error
    arms = sample.metadata["arms"]
    # The pinned packages in the image are the ones that ran (no fixture substitute).
    assert sample.metadata["pins"] == PINS, sample.metadata["pins"]
    for arm_id, arm in arms.items():
        assert arm["exit_code"] == 0 and arm["model_calls"] == 3, arm
        assert arm["owner_questions"] == 1 and arm["owner_revealed_requirement_ids"] == ["REQ-1"], arm
        assert arm["labels_edited"] is True, arm
        assert arm["hidden_tests"]["passed"] is True, arm
        assert arm["complete"] is True and arm["success"] is True, arm
        assert arm["grader"]["status"] == "graded" and arm["grader"]["intent_faithful"] is True, arm
        assert arm["grader"]["grader_family"] != arm["grader"]["driver_family"], arm
        assert arm["pi_session_records"] >= 1, arm
        assert arm["final_message"] == FINAL_MESSAGE, arm
        # Pi lists exactly the skills the arm's own setup put on disk: the pinned package, the init output or the shipped set.
        assert arm["skills_seen"] == arm["skills_on_disk"], arm
        assert bool(arm["skills_seen"]) == (arm_id != "native"), arm
        for marker in ARMS[arm_id]["loaded"]:
            assert arm["setup_seen"][marker] is True, (arm_id, marker, arm["setup_seen"])
        for marker in ARMS[arm_id]["absent"]:
            assert arm["setup_seen"][marker] is False, (arm_id, marker, arm["setup_seen"])
        for path in arm["grader"]["files_in_grader_input"]:
            assert not is_workflow_owned(path, descriptor(arm_id)), (arm_id, path)
    assert arms["superpowers"]["setup_seen"][SUPERPOWERS_BOOTSTRAP] is True
    assert arms["openspec"]["artifact_volume_changed_files"] > 0
    lines = [
        "Arms adapter proof: real Pi 1.2.0 through the public Inspect bridge, scripted no-network models, arms image",
        "Mechanics only. No paid model, no real workflow behaviour. The scripted driver asks the simulated owner,",
        "edits labels.py and exits; the grader is a scripted OpenAI-family model. Real pinned packages, no fixtures.",
        "Pins reported by the image: " + json.dumps(PINS, sort_keys=True), "Per arm:",
        *(json.dumps(arms[arm_id], sort_keys=True) for arm_id in ARMS),
    ]
    PROOF.parent.mkdir(parents=True, exist_ok=True)
    PROOF.write_text("\n".join(lines) + "\n")
