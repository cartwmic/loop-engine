"""Seams in target.py: option-off golden behaviour, and option-on behaviour for arm seams (a)-(f).

The trial container and the replay containers are FakeSandbox objects: an in-memory file tree that unpacks and packs the
same tar archives the harness writes, and a log of every command. The sandbox service, the bridge, the Docker probes and
the replay containers are replaced at their boundary. The simulated owner, the scoring code, the arm descriptor, the
grader and the snapshot step are the real code paths. The grader's model is a scripted Inspect model. No Docker, network
or model credentials are used.
"""
import asyncio
import contextlib
import io
import json
from pathlib import Path
import shutil
import subprocess
import tarfile
from types import SimpleNamespace

import anyio
import pytest
from inspect_ai.model import ModelAPI, ModelOutput, ModelUsage, modelapi

from evalcore.settings import Assignment, CampaignSettings, Case
from evalcore.target import InfrastructureError, TrialContext
from scevals import grader, target
from scevals.arms import load_arm, render_prompt
from scevals.cases import load_case
from scevals.grader import CLAIM_INSTRUCTION, FIDELITY_INSTRUCTION, GRADER_CONTRACT
from scevals.image import DEFAULT_IMAGE
from scevals.judge import IntentJudgment
from scevals.target import ARM_BASELINE_COMMAND, FINAL_MESSAGE, PREAMBLE, SoftwareChangeTarget

CASE = Path(__file__).resolve().parents[1] / "example-case"
REFERENCE = CASE / "hidden/reference"
REQUEST = (CASE / "public/request.md").read_text()
DRIVER = "driver-scripted/claude-scripted-driver"  # Anthropic family
GRADER = "seam-grader/gpt-scripted-judge"  # OpenAI family, so the grader is independent of the driver
QUESTION = "Which label rule applies to whitespace?"
FINAL_STDOUT = "Trimmed whitespace and folded labels."
ARM_IMAGE = "loop-engine-sc-evals:arm-seams"
ARM_DRIVER = ["arm-driver", "--seam"]
NATIVE = {"id": "native", "driver_command": ARM_DRIVER, "prompt_template": "native",
          "workflow_owned_paths": [], "completion_rule": "driver-exit"}
NATIVE_IN_IMAGE = NATIVE | {"image": ARM_IMAGE}
LOOP_ENGINE = {"id": "loop-engine", "driver_command": ["sc-driver"], "prompt_template": "loop-engine",
               "workflow_owned_paths": [], "completion_rule": "driver-exit"}
VISIBLE_COMMAND = ["visible-tests", "--seam"]
GRADED = {"arm": NATIVE, "grade": True}


def engine_show(lifecycle, state="end"):
    return {"lifecycle": lifecycle, "current_state": state, "state_visit": 1, "context": [],
            "work_slot_invocations": [], "override_count": 0, "initial_input": {"config_version": "standard-16"}}


def engine(lifecycle="final"):
    return {"show": engine_show(lifecycle), "history": []}


def result(returncode=0, stdout=""):
    return SimpleNamespace(success=returncode == 0, returncode=returncode, stdout=stdout, stderr="")


class FakeSandbox:
    """One container: an in-memory file tree, a log of every exec, and the answers the harness's commands need."""

    def __init__(self, *, engine=None, hidden="passed", driver=None, visible=0, runner_returncode=0):
        self.files = {}
        self.calls = []  # (argv, timeout) in order
        self.engine = engine  # {"show": ..., "history": ...} when a Loop Engine ran, else None
        self.hidden = hidden  # status given to every hidden test
        self.driver = driver  # async (sandbox, prompt) -> (returncode, stdout), or raises TimeoutError
        self.visible = visible  # exit code of the visible test command, or TimeoutError
        self.runner_returncode = runner_returncode  # exit code of the hidden-test runner
        self.visible_cwd = None
        self.services = {}
        self.requirement_tags = load_case(CASE).test_requirements

    def commands(self):
        return [argv for argv, _ in self.calls]

    async def connection(self):
        return SimpleNamespace(container="fake-trial-container")

    async def read_file(self, path, text=True):
        if path not in self.files:
            raise FileNotFoundError(path)
        return self.files[path].decode() if text else self.files[path]

    async def write_file(self, path, content):
        self.files[path] = content.encode() if isinstance(content, str) else content

    async def exec(self, command, *, cwd=None, env=None, timeout=None):
        self.calls.append((list(command), timeout))
        if list(command) == VISIBLE_COMMAND:
            self.visible_cwd = cwd
            if self.visible is TimeoutError:
                raise TimeoutError
            return result(self.visible)
        if command[1] == "/opt/sc-evals/driver_process.py":
            self.driver_env = dict(env or {})
            returncode, stdout = await (self.driver or delivering())(self, command[-1])
            return result(returncode, stdout)
        if command[0] == "sh" and "loop-engine --database" in command[2]:
            return self.engine_answer(command)
        if command[0] == "python3" and command[1] == "-c" and "sqlite3" in command[2]:
            return result(0, "")  # no committed run row yet: the progress poll has no reading
        if command[0] == "sh" and "split -b" in command[2]:
            self.files["/tmp/capture-parts/part-000000"] = self.pack("/workspace", "/workspace")
            return result(0)
        if command[0] == "sh" and command[2] == "ls /tmp/capture-parts":
            names = sorted(path.rsplit("/", 1)[1] for path in self.files if path.startswith("/tmp/capture-parts/"))
            return result(0, "\n".join(names))
        if command[0] == "sh" and "tar -xf /tmp/restore-workspace.tar" in command[2]:
            self.unpack(self.files.pop("/tmp/restore-workspace.tar"), "/workspace")
            return result(0)
        if command[0] == "sh" and "tar -xf /tmp/copy-tree.tar" in command[2]:
            self.unpack(self.files.pop("/tmp/copy-tree.tar"), command[3])
            return result(0)
        if command[0] == "sh" and "/tmp/snapshot.tar" in command[2]:
            self.files["/tmp/snapshot.tar"] = self.pack("/workspace/public/repo", "/workspace")
            return result(0)
        if command[1] == "/opt/scoring/case_runtime.py":
            if self.runner_returncode:
                return result(self.runner_returncode)
            tests = [{"test_id": f"t{number}", "requirement_id": tag, "status": self.hidden}
                     for number, tag in enumerate(self.requirement_tags)]
            self.files["/workspace/trial/hidden-results.json"] = json.dumps(
                {"tests": tests, "collection_error": False}).encode()
            return result(0)
        return result(0)

    def engine_answer(self, command):
        path, arguments = command[3], command[4:]
        if self.engine is None:
            return result(1)
        self.files[path] = json.dumps({"result": self.engine["history" if arguments[0] == "history" else "show"]}).encode()
        return result(0)

    def pack(self, directory, base):
        buffer = io.BytesIO()
        with tarfile.open(fileobj=buffer, mode="w") as archive:
            for path, data in sorted(self.files.items()):
                if path.startswith(directory + "/"):
                    info = tarfile.TarInfo(path[len(base) + 1:])
                    info.size = len(data)
                    archive.addfile(info, io.BytesIO(data))
        return buffer.getvalue()

    def unpack(self, archive_bytes, destination):
        with tarfile.open(fileobj=io.BytesIO(archive_bytes)) as archive:
            for member in archive.getmembers():
                if member.isfile():
                    self.files[f"{destination}/{member.name}"] = archive.extractfile(member).read()


def delivering(*, stdout=FINAL_STDOUT, returncode=0, sleep=0.0, timeout=False, intent=False, wait_for=None):
    """A driver that asks the owner once, delivers the case's reference overlay, then exits by itself or times out.

    With wait_for (an asyncio.Event), it exits only after that event is set.
    """
    async def driver(sandbox, prompt):
        await sandbox.services["ask_owner"](QUESTION)
        for file in sorted(REFERENCE.rglob("*")):
            if file.is_file():
                sandbox.files["/workspace/public/repo/" + file.relative_to(REFERENCE).as_posix()] = file.read_bytes()
        if intent:
            sandbox.files["/workspace/trial/artifacts/intent.json"] = b"{}"
        if wait_for is not None:
            await wait_for.wait()
        await anyio.sleep(sleep)
        if timeout:
            raise TimeoutError
        return returncode, stdout
    return driver


async def fake_service(name, methods, until, sandbox, started, instance):
    sandbox.services = methods
    started.set()


@contextlib.asynccontextmanager
async def fake_bridge(**kwargs):
    yield


async def fake_judge(intent, brief, **kwargs):
    return IntentJudgment(passed=True, model="judge-fake", usage={})


GRADER_STATE = {}


@modelapi(name="seam-grader")
class SeamGrader(ModelAPI):
    """Answers the grader's fidelity and claim calls from GRADER_STATE in call order and keeps each payload.

    A reply that is an exception is raised by that call, as a provider failure would be.
    """

    async def generate(self, input, tools, tool_choice, config):
        kind = {FIDELITY_INSTRUCTION: "fidelity", CLAIM_INSTRUCTION: "claim"}[input[0].text]
        GRADER_STATE["recorded"].append((kind, json.loads(input[1].text)))
        reply = GRADER_STATE["replies"][kind].pop(0)
        if isinstance(reply, BaseException):
            raise reply
        output = ModelOutput.from_content(model=self.model_name, content=json.dumps(reply))
        output.usage = ModelUsage(input_tokens=3, output_tokens=2, total_tokens=5)
        return output


def fidelity_reply(faithful=True):
    requirements = [{"id": tag, "verdict": "satisfied", "probability": 0.9, "pointer": None,
                     "null_reason": "judged from the whole change"} for tag in load_case(CASE).requirement_ids]
    return {"requirements": requirements, "unrequested_changes": [], "intent_faithful": faithful,
            "intent_probability": 0.9}


@pytest.fixture
def scripted_grader():
    GRADER_STATE.clear()
    GRADER_STATE.update(replies={"fidelity": [fidelity_reply(), fidelity_reply()],
                                 "claim": [{"claims": [{"id": "c1", "verdict": "supported"}]}] * 2},
                        recorded=[])
    yield GRADER_STATE
    GRADER_STATE.clear()


async def scripted_grade(**kwargs):
    return await grader.grade(**(kwargs | {"grader_model": GRADER}))


class Outcome(SimpleNamespace):
    @property
    def metrics(self):
        return self.scored.metrics

    def driver_exec(self):
        """(argv, timeout) of the driver process exec in the trial container."""
        return next((argv, timeout) for argv, timeout in self.trial.calls if argv[1:2] == ["/opt/sc-evals/driver_process.py"])


class Seams:
    """One software-change trial over fakes: prepare, run and score, with the harness's own code on each step."""

    def __init__(self, monkeypatch, tmp_path):
        self.tmp_path = tmp_path
        self.inspected = []  # trial images the harness asked Docker to inspect
        self.replays = []  # images of replay containers created for scoring
        self.trial = None
        self.target = SoftwareChangeTarget()
        self.target.run_directory = tmp_path / "captures" / "seams"
        self.target.run_directory.mkdir(parents=True)
        self.engine, self.hidden, self.driver, self.visible = None, "passed", None, 0
        self.runner_returncode = 0
        self.replay_envs = []  # every replay container's sandbox, in creation order
        original_run_process = anyio.run_process

        async def run_process(command, **kwargs):
            if command[:2] == ["docker", "inspect"]:
                config = {"Id": "fake-trial-container", "Image": "sha256:trial", "Mounts": [], "Platform": "linux/arm64",
                          "HostConfig": {"Binds": None}, "Config": {"WorkingDir": "/workspace"}}
                return SimpleNamespace(stdout=json.dumps([config]).encode())
            return await original_run_process(command, **kwargs)

        def docker_image_inspect(command, timeout=None):
            self.inspected.append(command[-1])
            identity = {"Id": "sha256:image", "Architecture": "arm64", "Os": "linux",
                        "Config": {"Labels": {"org.opencontainers.image.revision": "abc123"}}}
            return json.dumps([identity])

        class ReplayContainer:
            def __init__(sandbox, image, **kwargs):
                self.replays.append(image)
                sandbox.env = FakeSandbox(engine=self.engine, hidden=self.hidden, visible=self.visible,
                                          runner_returncode=self.runner_returncode)
                self.replay_envs.append(sandbox.env)

            async def __aenter__(sandbox):
                return sandbox.env

            async def __aexit__(sandbox, *exc):
                return None

        def sandbox():
            self.trial = FakeSandbox(engine=self.engine, hidden=self.hidden, driver=self.driver)
            return self.trial

        monkeypatch.setattr(target, "sandbox", sandbox)
        monkeypatch.setattr(target, "sandbox_service", fake_service)
        monkeypatch.setattr(target, "sandbox_agent_bridge", fake_bridge)
        monkeypatch.setattr(target, "get_model", lambda *args, **kwargs: None)
        monkeypatch.setattr(target, "resolve_ref", lambda ref: "abc123")
        monkeypatch.setattr(target, "output", docker_image_inspect)
        monkeypatch.setattr(target, "ReplayContainer", ReplayContainer)
        monkeypatch.setattr(target, "judge_intent", fake_judge)
        monkeypatch.setattr(target, "grade", scripted_grade)
        monkeypatch.setattr(anyio, "run_process", run_process)

    def configure(self, *, options=None, engine=None, hidden="passed", driver=None, idle_limit=1800, hard_limit=28800,
                  driver_model=DRIVER, judge_model=GRADER, visible=0, runner_returncode=0):
        self.engine, self.hidden, self.driver, self.visible = engine, hidden, driver, visible
        self.runner_returncode = runner_returncode
        settings = CampaignSettings.model_validate({
            "campaign": "seams", "cases_path": str(CASE), "simulator_model": "sc-scripted/simulator",
            "judge_model": judge_model, "idle_limit_seconds": idle_limit, "hard_limit_seconds": hard_limit,
            "combinations": [{"id": "combo", "driver": driver_model, "implementer": "sc-scripted/implementer",
                              "reviewer": "sc-scripted/reviewer"}],
            "target_options": options or {}})
        case = load_case(CASE)
        self.context = TrialContext(settings=settings,
                                    assignment=Assignment(sample_id="seam-1", combination_id="combo",
                                                          case_id=case.id, case_version=case.version, trial=1),
                                    case=Case(id=case.id, version=case.version, path=CASE),
                                    combination=settings.resolved_combinations()[0], guard=None)
        return self

    def prepare(self):
        return self.target.prepare_sample(self.context)

    def run(self):
        sample = self.prepare()
        state = SimpleNamespace(input=sample.input)

        async def trial():
            execution = await self.target.run_trial(state, self.context)
            scored = await self.target.score(state, execution, self.context)
            return execution, scored

        execution, scored = asyncio.run(trial())
        capture = self.target.capture_for(self.context)
        return Outcome(execution=execution, scored=scored, trial=self.trial, capture=capture,
                       meta=json.loads((capture / "meta.json").read_text()), compose=Path(sample.sandbox.config).read_text(),
                       replays=self.replays, inspected=self.inspected, run_directory=self.target.run_directory)


@pytest.fixture
def seams(monkeypatch, tmp_path):
    return Seams(monkeypatch, tmp_path)


# Option-off golden behaviour: no new option or arm, so every seam executes the statement it had on main.

def test_default_margin_is_300_seconds_and_sets_the_driver_stop(seams):
    run = seams.configure(hard_limit=1000).run()
    assert target.DRIVER_LIMIT_MARGIN_SECONDS == 300
    assert run.driver_exec()[1] == 700


def test_default_prompt_and_driver_command_and_image_are_unchanged(seams):
    run = seams.configure().run()
    argv, _ = run.driver_exec()
    assert argv == ["python3", "/opt/sc-evals/driver_process.py", "sc-driver",
                    PREAMBLE.format(profile="standard-16", run_id="eval-trial", scripted="", request=REQUEST)]
    assert run.inspected == [DEFAULT_IMAGE] and run.replays == [DEFAULT_IMAGE]
    assert f"image: {json.dumps(DEFAULT_IMAGE)}" in run.compose


def test_top_level_driver_command_and_image_still_apply_without_an_arm(seams):
    run = seams.configure(options={"driver_command": ["sh", "-c", 'exec sc-driver "$@"', "sh"],
                                   "image": "loop-engine-sc-evals:other"}).run()
    argv, _ = run.driver_exec()
    assert argv[2:6] == ["sh", "-c", 'exec sc-driver "$@"', "sh"]
    assert run.inspected == ["loop-engine-sc-evals:other"] and run.replays == ["loop-engine-sc-evals:other"]


def test_without_an_arm_the_harness_makes_no_baseline_and_retains_no_arm_evidence(seams):
    run = seams.configure(engine=engine("final"), driver=delivering(intent=True)).run()
    assert ["sh", "-c", ARM_BASELINE_COMMAND] not in run.trial.commands()
    assert not (run.capture / FINAL_MESSAGE).exists()
    assert not (run.run_directory / "seam-1-snapshots").exists()
    assert not {"driver_elapsed_seconds", "owner_interaction", "outcome_grade", "outcome_grade_detail"} & run.metrics.keys()
    assert "run_health" in run.metrics and "intent_judge" in run.metrics


def test_without_an_arm_a_terminated_engine_run_still_fails_the_existing_rule(seams):
    run = seams.configure(engine=engine("terminated")).run()
    assert run.scored.success is False
    assert run.execution.terminal is True


def test_without_an_arm_an_active_engine_run_is_not_terminal_and_is_not_scored_by_tests(seams):
    run = seams.configure(engine=engine("active")).run()
    assert run.execution.terminal is False and run.scored.success is False
    assert "hidden_tests" not in run.metrics


def test_without_an_arm_a_final_engine_run_with_valid_intent_still_succeeds(seams):
    run = seams.configure(engine=engine("final"), driver=delivering(intent=True)).run()
    assert run.scored.success is True and run.execution.terminal is True


def test_without_an_arm_the_original_scoring_branch_is_the_one_reached(seams, monkeypatch):
    def no_arm_scoring(self, trial, archive, context, arm):
        raise AssertionError("the arm scoring branch must not run without an arm")

    monkeypatch.setattr(SoftwareChangeTarget, "score_arm_capture", no_arm_scoring)
    run = seams.configure(engine=engine("final"), driver=delivering(intent=True)).run()
    assert run.scored.success is True


# Option-on seams (a)-(f) and the grader, with an arm set.

def test_arm_driver_command_and_image_reach_the_container_exec_and_the_replay(seams):
    run = seams.configure(options={"arm": NATIVE_IN_IMAGE}).run()
    argv, _ = run.driver_exec()
    assert argv[2:-1] == ARM_DRIVER
    assert argv[-1] == render_prompt(load_arm({"arm": NATIVE_IN_IMAGE}), profile="standard-16", run_id="eval-trial",
                                     scripted="", request=REQUEST)
    assert run.inspected == [ARM_IMAGE] and run.replays == [ARM_IMAGE]
    assert f"image: {json.dumps(ARM_IMAGE)}" in run.compose


def test_arm_runs_keep_the_pi_session_record_under_the_trial_area(seams):
    run = seams.configure(options={"arm": NATIVE_IN_IMAGE}).run()
    assert run.trial.driver_env["SC_PI_SESSION_DIR"] == "/workspace/trial/pi-sessions"


def test_runs_without_an_arm_do_not_set_the_pi_session_directory(seams):
    run = seams.configure().run()
    assert "SC_PI_SESSION_DIR" not in run.trial.driver_env


def test_arm_without_an_image_uses_the_default_image_for_the_trial_and_the_replay(seams):
    run = seams.configure(options={"arm": NATIVE}).run()
    assert run.inspected == [DEFAULT_IMAGE] and run.replays == [DEFAULT_IMAGE]


def test_arm_prompt_is_rendered_from_the_descriptor_not_the_preamble(seams):
    run = seams.configure(options={"arm": NATIVE}).run()
    prompt = run.driver_exec()[0][-1]
    assert prompt.startswith("Run this software-change request unattended")
    assert "starting baseline commit" in prompt
    assert prompt != PREAMBLE.format(profile="standard-16", run_id="eval-trial", scripted="", request=REQUEST)


def test_arm_refuses_a_top_level_driver_command_or_image(seams):
    with pytest.raises(ValueError, match="inside arm"):
        seams.configure(options={"arm": NATIVE, "driver_command": ["sc-driver"]}).prepare()


def test_arm_harness_makes_the_baseline_commit_and_origin_before_the_driver(seams):
    run = seams.configure(options={"arm": NATIVE}).run()
    commands = run.trial.commands()
    remote = commands.index(["git", "init", "--bare", "-q", "/workspace/local-remote.git"])
    baseline = commands.index(["sh", "-c", ARM_BASELINE_COMMAND])
    driver = next(i for i, argv in enumerate(commands) if argv[1:2] == ["/opt/sc-evals/driver_process.py"])
    assert remote < baseline < driver


def test_baseline_command_makes_a_real_baseline_commit_and_origin(tmp_path):
    root = tmp_path / "workspace"
    shutil.copytree(CASE / "public", root / "public")
    subprocess.run(["git", "init", "--bare", "-q", str(root / "local-remote.git")], check=True)
    subprocess.run(["sh", "-c", ARM_BASELINE_COMMAND.replace("/workspace", str(root))], check=True, capture_output=True)
    repo = root / "public/repo"

    def git(*args):
        return subprocess.run(["git", "-C", str(repo), *args], check=True, capture_output=True, text=True).stdout.strip()

    assert git("log", "--format=%s") == "Starting baseline"
    assert git("remote", "get-url", "origin") == str(root / "local-remote.git")
    assert git("status", "--porcelain") == ""


def test_driver_margin_option_moves_the_driver_stop(seams):
    run = seams.configure(hard_limit=1000, options={"driver_margin_seconds": 400}).run()
    assert run.driver_exec()[1] == 600


@pytest.mark.parametrize("margin", [0, -5, True, "300"])
def test_invalid_driver_margin_is_refused_before_any_trial(seams, margin):
    with pytest.raises(ValueError, match="driver_margin_seconds"):
        seams.configure(options={"driver_margin_seconds": margin}).prepare()


def test_snapshots_are_taken_from_the_progress_poll_beside_the_capture(seams):
    run = seams.configure(options={"arm": NATIVE, "snapshot_interval_seconds": 1}, idle_limit=4,
                          driver=delivering(sleep=2.5)).run()
    index = json.loads((run.run_directory / "seam-1-snapshots" / "snapshots.json").read_text())
    assert [entry["file"] for entry in index] == ["snapshot-000001.tar"]
    assert index[0]["elapsed_seconds"] >= 1.0
    assert not any(path.name.startswith("snapshot") for path in run.capture.iterdir())
    assert run.scored.success is True


def test_a_driver_that_exits_during_a_snapshot_longer_than_the_idle_limit_is_complete_not_idle_stopped(seams, monkeypatch):
    """The snapshot runs in the progress poll; a driver that exits during it is complete, never idle-stopped.

    Real time, about four seconds: the poll wakes after its idle/2 interval, the snapshot starts, the driver exits at
    once, and the snapshot then lasts longer than the idle limit. The idle clock counts from the last progress, so
    without the exit check the poll would stop a driver that had already finished.
    """
    idle_limit = 2
    snapshot_started = asyncio.Event()
    real_snapshot_step = target.snapshot_step

    async def slow_snapshot_step(*args, **kwargs):
        snapshot_started.set()
        await anyio.sleep(idle_limit + 1)
        return await real_snapshot_step(*args, **kwargs)

    monkeypatch.setattr(target, "snapshot_step", slow_snapshot_step)
    run = seams.configure(options={"arm": NATIVE, "snapshot_interval_seconds": 0.5}, idle_limit=idle_limit,
                          driver=delivering(wait_for=snapshot_started)).run()
    index = json.loads((run.run_directory / "seam-1-snapshots" / "snapshots.json").read_text())
    assert len(index) == 1
    assert run.meta["driver_stop_kind"] == "exit"
    assert run.metrics["complete"] is True and run.metrics["incomplete_reason"] is None
    assert run.execution.terminal is True and run.scored.success is True


@pytest.mark.parametrize("options,idle_limit,message", [
    ({"arm": NATIVE, "snapshot_interval_seconds": 0}, 1800, "positive number"),
    ({"arm": NATIVE, "snapshot_interval_seconds": True}, 1800, "positive number"),
    ({"arm": NATIVE, "snapshot_interval_seconds": 60}, None, "needs idle_limit_seconds"),
])
def test_snapshot_options_are_refused_before_any_driver_runs(seams, options, idle_limit, message):
    with pytest.raises(ValueError, match=message):
        seams.configure(options=options, idle_limit=idle_limit).prepare()


def test_snapshots_are_off_by_default_and_leave_no_directory(seams):
    run = seams.configure(options={"arm": NATIVE}, idle_limit=4, driver=delivering(sleep=2.5)).run()
    assert not (run.run_directory / "seam-1-snapshots").exists()


def test_arm_retains_the_final_message_privately_and_records_numbers_only(seams):
    run = seams.configure(options={"arm": NATIVE}, driver=delivering(sleep=0.2)).run()
    answer = run.meta["simulated_owner"]["answers"][0]["answer"]
    assert (run.capture / FINAL_MESSAGE).read_text() == FINAL_STDOUT
    assert run.metrics["owner_interaction"] == {"question_words": len(QUESTION.split()), "answer_words": len(answer.split())}
    assert run.metrics["discipline"]["owner_question_count"] == 1
    assert 0.2 <= run.metrics["driver_elapsed_seconds"] < 5
    assert FINAL_STDOUT not in json.dumps(run.metrics)


@pytest.mark.parametrize("options,engine_state,expect_success", [
    ({"arm": NATIVE}, None, True),  # naturally completed native run
    ({"arm": LOOP_ENGINE}, engine("terminated"), True),  # correct code left terminated by the engine
    ({"arm": LOOP_ENGINE}, engine("active"), True),  # correct code left active
    ({"arm": LOOP_ENGINE}, engine("final"), True),  # naturally completed Loop Engine run
    ({"arm": LOOP_ENGINE}, engine("terminated"), False),  # same code, one hidden test failing: not a success
])
def test_arm_completion_is_the_driver_exit_and_success_is_complete_with_all_tests(seams, options, engine_state, expect_success):
    hidden = "passed" if expect_success else "failed"
    run = seams.configure(options=options, engine=engine_state, hidden=hidden, driver=delivering(intent=True)).run()
    assert run.scored.success is expect_success
    assert run.execution.terminal is True
    assert run.metrics["hidden_test_outcome"]["passed"] is expect_success


@pytest.mark.parametrize("arm", [NATIVE, LOOP_ENGINE])
@pytest.mark.parametrize("state", ["terminated", "active", "crashed", "final"])
def test_correct_code_scores_the_same_under_native_and_loop_engine_arms(seams, arm, state):
    lifecycle = {"crashed": "active"}.get(state, state)
    driver = delivering(intent=True, returncode=137 if state == "crashed" else 0)
    run = seams.configure(options={"arm": arm}, engine=engine(lifecycle), driver=driver).run()
    assert run.scored.success is True and run.execution.terminal is True


def test_a_driver_stopped_at_the_hard_limit_is_incomplete_but_its_delivery_is_still_scored(seams):
    run = seams.configure(options={"arm": NATIVE}, driver=delivering(timeout=True)).run()
    assert run.scored.success is False and run.execution.terminal is False
    assert run.metrics["complete"] is False and run.metrics["incomplete_reason"] == "hard"
    assert run.metrics["hidden_test_outcome"]["passed"] is True
    assert run.metrics["delivery_recall"]["value"] == 1.0


def test_arm_diagnostics_are_null_for_an_arm_without_an_engine(seams):
    run = seams.configure(options={"arm": NATIVE}).run()
    diagnostics = run.metrics["diagnostics"]
    assert diagnostics["engine_lifecycle"] is None and diagnostics["intent_valid"] is None
    assert diagnostics["workflow_adherence"] is None
    assert "run_health" not in run.metrics


def test_loop_engine_lifecycle_and_intent_validity_are_diagnostics_only(seams):
    run = seams.configure(options={"arm": LOOP_ENGINE}, engine=engine("terminated"),
                          driver=delivering(intent=True)).run()
    diagnostics = run.metrics["diagnostics"]
    assert diagnostics["engine_lifecycle"] == "terminated" and diagnostics["intent_valid"] is True
    assert run.scored.success is True
    assert run.metrics["run_health"]["unattended_finish"] is False  # a process diagnostic; it decides nothing


# A trusted provider route failure is infrastructure for an arm, whatever the driver exit; without an arm the rule is unchanged.

ROUTE_FAILURES = ("provider policy block", "provider rejected bridged request")


@pytest.mark.parametrize("returncode", [0, 137])
@pytest.mark.parametrize("reason", ROUTE_FAILURES)
def test_arm_route_failure_is_infrastructure_at_finalization_whatever_the_driver_exit(seams, monkeypatch, reason, returncode):
    monkeypatch.setattr(target, "provider_route_failure", lambda: reason)
    with pytest.raises(InfrastructureError, match=f"^{reason}$"):
        seams.configure(options={"arm": NATIVE}, driver=delivering(returncode=returncode)).run()


@pytest.mark.parametrize("returncode", [0, 137])
@pytest.mark.parametrize("reason", ROUTE_FAILURES)
def test_arm_route_failure_is_infrastructure_at_scoring_whatever_the_driver_exit(seams, monkeypatch, reason, returncode):
    monkeypatch.setattr(target, "provider_route_failure", lambda: None)
    run = seams.configure(options={"arm": NATIVE}, driver=delivering(returncode=returncode)).run()
    run.execution.data[1]["evidence"]["provider_route_failure"] = reason  # the capture as a rescore would read it
    with pytest.raises(InfrastructureError, match=f"^{reason}$"):
        asyncio.run(seams.target.score(None, run.execution, seams.context))


def test_arm_without_a_route_failure_is_unchanged_by_the_route_rule(seams, monkeypatch):
    monkeypatch.setattr(target, "provider_route_failure", lambda: None)
    run = seams.configure(options={"arm": NATIVE}, driver=delivering(returncode=137, intent=True)).run()
    assert run.scored.success is True and run.execution.terminal is True
    assert "provider_route_failure" not in run.meta["evidence"]


@pytest.mark.parametrize("lifecycle", ["active", "terminated", "final"])
def test_without_an_arm_a_route_failure_is_infrastructure_only_for_an_unfinished_engine_run(seams, monkeypatch, lifecycle):
    monkeypatch.setattr(target, "provider_route_failure", lambda: "provider policy block")
    if lifecycle == "active":
        with pytest.raises(InfrastructureError, match="^provider policy block$"):
            seams.configure(engine=engine(lifecycle)).run()
    else:
        run = seams.configure(engine=engine(lifecycle), driver=delivering(intent=True)).run()
        assert run.scored.success is (lifecycle == "final")


@pytest.mark.parametrize("lifecycle,raises", [("active", True), ("final", False)])
def test_without_an_arm_a_recorded_route_failure_is_raised_at_scoring_only_when_not_terminal(seams, lifecycle, raises):
    run = seams.configure(engine=engine(lifecycle), driver=delivering(intent=True)).run()
    run.execution.data[1]["evidence"]["provider_route_failure"] = "provider rejected bridged request"
    if raises:
        with pytest.raises(InfrastructureError, match="^provider rejected bridged request$"):
            asyncio.run(seams.target.score(None, run.execution, seams.context))
    else:
        assert asyncio.run(seams.target.score(None, run.execution, seams.context)).success is True


def test_grading_output_reaches_the_trial_metrics_and_success_is_unchanged(seams, scripted_grader):
    run = seams.configure(options={"arm": NATIVE, "grade": True}).run()
    assert run.metrics["outcome_grade"] == {"intent_faithful": True, "unstable": False}
    assert run.metrics["outcome_grade_status"] == "graded"
    assert run.scored.success is True
    fidelity = [payload for kind, payload in scripted_grader["recorded"] if kind == "fidelity"]
    claims = [payload for kind, payload in scripted_grader["recorded"] if kind == "claim"]
    assert fidelity and all(payload["final_message"] == FINAL_STDOUT for payload in fidelity)
    assert all("hidden_tests" not in payload for payload in fidelity)
    assert claims[0]["receipts"]["hidden_tests"] == {tag: "passed" for tag in load_case(CASE).requirement_ids}


def test_unstable_grade_counts_as_not_faithful_and_does_not_change_success(seams, scripted_grader):
    scripted_grader["replies"]["fidelity"] = [fidelity_reply(True), fidelity_reply(False)]
    run = seams.configure(options={"arm": NATIVE, "grade": True}).run()
    assert run.metrics["outcome_grade"] == {"intent_faithful": False, "unstable": True}
    assert run.scored.success is True


def test_grade_is_absent_when_the_option_is_off(seams, scripted_grader):
    run = seams.configure(options={"arm": NATIVE}).run()
    assert "outcome_grade" not in run.metrics and "outcome_grade_detail" not in run.metrics
    assert scripted_grader["recorded"] == []


def test_grade_needs_an_arm_and_a_judge_from_another_family_than_the_driver(seams):
    with pytest.raises(ValueError, match="needs an arm"):
        seams.configure(options={"grade": True}).prepare()
    with pytest.raises(ValueError, match="refused before any trial"):
        seams.configure(options={"arm": NATIVE, "grade": True}, driver_model="driver-scripted/gpt-scripted-driver").prepare()


def test_block_label_is_accepted_and_recorded_without_acting(seams):
    block = {"task": "label-keys", "repetition": 1, "arm": "native", "order": 1, "attempt": 1}
    context = seams.configure(options={"arm": NATIVE, "block": block}).context
    seams.prepare()
    assert context.settings.recorded()["target_options"]["block"] == block


# Grade detail (the grade() export without its overhead) and the visible-test receipt, with an arm and grade set.

DETAIL_KEYS = {"contract", "status", "samples", "grader_family", "driver_family", "intent", "requirements",
               "unrequested_changes", "pointers", "claims", "leakage"}
VERDICT_VALUES = {GRADER_CONTRACT, "graded", "anthropic", "openai", "satisfied", "partial", "violated", "not_addressed",
                  "faithful", "unfaithful", "unstable", "supported", "contradicted", "not_checkable"}


def strings(value):
    """Every string value of a nested export. Dict keys are requirement ids and are not values."""
    if isinstance(value, dict):
        return [s for item in value.values() for s in strings(item)]
    if isinstance(value, list):
        return [s for item in value for s in strings(item)]
    return [value] if isinstance(value, str) else []


def record_grade_arguments(monkeypatch):
    """Keeps the keyword arguments the target passes to grade(), and grades as the seams do."""
    received = []
    grade_before = target.grade

    async def recording(**kwargs):
        received.append(kwargs)
        return await grade_before(**kwargs)

    monkeypatch.setattr(target, "grade", recording)
    return received


def visible_calls(seams):
    """(argv, timeout) of each visible test command run in a replay container."""
    return [(argv, timeout) for env in seams.replay_envs for argv, timeout in env.calls if argv == VISIBLE_COMMAND]


def test_grade_detail_keeps_the_export_without_its_overhead(seams, scripted_grader):
    run = seams.configure(options=GRADED).run()
    detail = run.metrics["outcome_grade_detail"]
    requirement_ids = load_case(CASE).requirement_ids
    assert set(detail) == DETAIL_KEYS and "overhead" not in detail
    assert run.metrics["outcome_grade"] == {"intent_faithful": True, "unstable": False}
    assert detail["status"] == "graded"
    assert detail["intent"] == {"verdict": "faithful", "probability": 0.9, "counted_faithful": True}
    assert detail["requirements"] == {tag: {"verdict": "satisfied", "probability": 0.9} for tag in requirement_ids}
    assert detail["pointers"] == {"resolved": 0, "unresolved": 0, "null_with_reason": 2 * len(requirement_ids)}
    assert detail["claims"]["checked"] == 1 and detail["claims"]["supported"] == 1


def test_grade_detail_stores_no_final_message_diff_or_quote_text(seams, scripted_grader):
    quote = 'return " ".join(label.split()).casefold() or "inbox"'  # line 5 of the reference overlay, which is delivered
    reply = fidelity_reply()
    reply["requirements"][0] |= {"pointer": {"source": "final", "file": "labels.py", "start_line": 5, "end_line": 5,
                                             "quote": quote}, "null_reason": None}
    scripted_grader["replies"]["fidelity"] = [reply, reply]
    run = seams.configure(options=GRADED).run()
    detail = run.metrics["outcome_grade_detail"]
    stored = json.dumps(detail, ensure_ascii=False)
    assert detail["pointers"]["resolved"] == 2  # the host did check the quote, and kept only the count
    assert quote not in stored and FINAL_STDOUT not in stored and "return label" not in stored
    assert set(strings(detail)) <= VERDICT_VALUES


def test_a_grader_error_stores_the_same_detail_shape_with_nulls(seams, scripted_grader):
    scripted_grader["replies"]["fidelity"] = ["not an object", "not an object"]
    run = seams.configure(options=GRADED).run()
    assert run.metrics["outcome_grade_status"] == "grader_error"
    assert run.metrics["outcome_grade"] == {"intent_faithful": None, "unstable": False}
    detail = run.metrics["outcome_grade_detail"]
    assert set(detail) == DETAIL_KEYS
    assert detail["status"] == "grader_error" and detail["requirements"] == {}
    assert detail["intent"] == {"verdict": None, "probability": None, "counted_faithful": None}
    assert detail["claims"]["checked"] is None and detail["pointers"]["resolved"] is None
    assert run.scored.success is True  # a grader error never fails the trial


# Evaluation-only failures: a failed grader or Loop Engine judge call is unavailable; the hidden tests already scored.

EVALUATION_FAILURES = [
    pytest.param(lambda: InfrastructureError("provider policy block"), id="infrastructure"),
    pytest.param(TimeoutError, id="timeout"),
    pytest.param(lambda: RuntimeError("provider wrapper error"), id="provider-wrapper"),
]


@pytest.mark.parametrize("failure", EVALUATION_FAILURES)
def test_a_failed_grader_call_is_unavailable_and_never_replaces_the_scored_outcome(seams, scripted_grader, failure):
    scripted_grader["replies"]["fidelity"] = [failure(), failure()]
    run = seams.configure(options=GRADED).run()
    total = len(load_case(CASE).test_requirements)
    assert run.metrics["outcome_grade_status"] == "grader_error"
    assert run.metrics["outcome_grade"] == {"intent_faithful": None, "unstable": False}
    detail = run.metrics["outcome_grade_detail"]
    assert set(detail) == DETAIL_KEYS and detail["status"] == "grader_error" and detail["requirements"] == {}
    assert run.metrics["hidden_test_outcome"] == {"passed": True, "passed_tests": total, "total_tests": total}
    assert run.scored.success is True


@pytest.mark.parametrize("failure", EVALUATION_FAILURES)
def test_a_failed_loop_engine_judge_call_is_unavailable_and_never_replaces_the_scored_outcome(seams, monkeypatch, failure):
    async def failing_judge(intent, brief, **kwargs):
        raise failure()

    monkeypatch.setattr(target, "judge_intent", failing_judge)
    run = seams.configure(options={"arm": LOOP_ENGINE}, engine=engine("final"), driver=delivering(intent=True)).run()
    total = len(load_case(CASE).test_requirements)
    assert run.metrics["intent_judge"] == {"status": "judge_error", "model": GRADER}
    assert run.metrics["hidden_test_outcome"] == {"passed": True, "passed_tests": total, "total_tests": total}
    assert run.scored.success is True


def test_a_failing_hidden_test_run_still_raises_and_the_grader_is_never_called(seams, scripted_grader):
    with pytest.raises(InfrastructureError, match="^hidden-test runner failed$"):
        seams.configure(options=GRADED, runner_returncode=1).run()
    assert scripted_grader["recorded"] == []


def test_visible_tests_receipt_is_none_when_the_option_is_absent(seams, scripted_grader, monkeypatch):
    received = record_grade_arguments(monkeypatch)
    seams.configure(options=GRADED).run()
    assert received[0]["visible_tests"] is None
    assert visible_calls(seams) == []


@pytest.mark.parametrize(("visible", "receipt"), [
    pytest.param(0, "passed", id="exit-zero"),
    pytest.param(1, "failed", id="exit-nonzero"),
    pytest.param(127, "error", id="cannot-run"),
    pytest.param(TimeoutError, "failed", id="timeout"),
])
def test_visible_tests_receipt_is_the_command_status_and_reaches_the_grader(seams, scripted_grader, monkeypatch, visible, receipt):
    received = record_grade_arguments(monkeypatch)
    run = seams.configure(options=GRADED | {"visible_test_command": VISIBLE_COMMAND}, visible=visible).run()
    assert received[0]["visible_tests"] == receipt
    claims = [payload["receipts"]["visible_tests"] for kind, payload in scripted_grader["recorded"] if kind == "claim"]
    assert claims and set(claims) == {receipt}
    assert visible_calls(seams) == [(VISIBLE_COMMAND, load_case(CASE).test_timeout_seconds)]
    assert run.scored.success is True  # the receipt is a grader input; hidden tests still decide success


def test_visible_tests_run_once_after_the_hidden_tests_in_the_replay_container(seams, scripted_grader):
    seams.configure(options=GRADED | {"visible_test_command": VISIBLE_COMMAND}).run()
    (replay,) = seams.replay_envs
    argvs = [argv for argv, _ in replay.calls]
    assert argvs.count(VISIBLE_COMMAND) == 1
    hidden = next(i for i, argv in enumerate(argvs) if argv[1:2] == ["/opt/scoring/case_runtime.py"])
    assert hidden < argvs.index(VISIBLE_COMMAND)
    assert replay.visible_cwd == "/workspace/public/repo"


@pytest.mark.parametrize("options", [
    {"arm": NATIVE, "visible_test_command": VISIBLE_COMMAND},
    GRADED | {"visible_test_command": []},
    GRADED | {"visible_test_command": "visible-tests --seam"},
    GRADED | {"visible_test_command": ["visible-tests", ""]},
])
def test_visible_test_command_needs_grade_and_a_nonempty_argv_and_is_refused_before_any_trial(seams, options):
    with pytest.raises(ValueError, match="visible_test_command"):
        seams.configure(options=options).prepare()
    assert seams.trial is None
