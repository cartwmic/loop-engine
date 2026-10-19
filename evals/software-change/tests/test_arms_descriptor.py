"""T1: arm descriptor validation, per-arm prompt rendering, workflow-owned path matcher, default prompt golden."""
import pytest

from scevals import arms
from scevals.target import PREAMBLE

WORKFLOW_PATHS = ["openspec/**", ".pi/**", "docs/superpowers/**", ".superpowers/**"]
ROLE_SCRIPTS = {"native": ["arm-native"], "superpowers": ["arm-superpowers"],
                "openspec": ["arm-openspec"], "loop-engine": ["sc-driver"]}
ARM_PATHS = {"native": [], "superpowers": WORKFLOW_PATHS, "openspec": WORKFLOW_PATHS, "loop-engine": []}
DEFAULT_PROMPT_GOLDEN = """Run this software-change request unattended to a terminal engine state.
Profile: standard-17
Use the shipped software-change profile with exactly that config_version. Read the
installed using-loop-engine and using-software-change-provider skills. Implementation
and review work MUST use sc-implement and sc-review. Reach the owner ONLY through
ask-owner; do not ask a human or bypass the service. The case repository is
/workspace/public/repo. Use /workspace/trial/run.sqlite, providers.toml at
/workspace/trial/providers.toml, run ID run-7, artifacts /workspace/trial/artifacts.
The owner has preapproved this exact profile and launch for this disposable trial.
You may initialize Git and create the starting public-repository baseline commit
inside this disposable container only. Do not commit delivered changes, push or
edit outside the request's scope. Create an initially empty bare local remote at
/workspace/local-remote.git and configure it as origin; pushing there is forbidden
and measured (this is not a hosted remote). Leave final intent.json
and reports under the artifact directory. Finish by reaching a terminal engine
state; merely saying you are done does not count.

Owner request:
Add retries.
"""


def arm_options(arm_id, **changes):
    arm = {"id": arm_id, "driver_command": list(ROLE_SCRIPTS[arm_id]), "prompt_template": arm_id,
           "workflow_owned_paths": list(ARM_PATHS[arm_id]), "completion_rule": "driver-exit"}
    arm.update(changes)
    return {"arm": arm}


def render(arm, scripted=""):
    return arms.render_prompt(arm, profile="standard-17", run_id="run-7", scripted=scripted, request="Add retries.")


def test_arm_unset_has_no_descriptor_and_owns_no_paths():
    assert arms.load_arm({}) is None
    assert arms.is_workflow_owned("openspec/changes/a.md", None) is False


def test_arm_unset_prompt_is_today_preamble_byte_for_byte():
    prompt = render(None)
    assert prompt == DEFAULT_PROMPT_GOLDEN
    assert prompt == PREAMBLE.format(profile="standard-17", run_id="run-7", scripted="", request="Add retries.")


def test_arm_unset_scripted_prompt_is_today_preamble_byte_for_byte():
    scripted = "Scripted driver: never-asks"
    prompt = render(None, scripted=scripted)
    assert prompt == PREAMBLE.format(profile="standard-17", run_id="run-7", scripted=scripted, request="Add retries.")
    assert "\n" + scripted + "\nOwner request:\n" in prompt


@pytest.mark.parametrize("arm_id", arms.ARM_IDS)
def test_each_arm_parses_to_its_descriptor(arm_id):
    assert arms.load_arm(arm_options(arm_id)) == arms.ArmDescriptor(
        id=arm_id, driver_command=tuple(ROLE_SCRIPTS[arm_id]), prompt_template=arm_id,
        workflow_owned_paths=tuple(ARM_PATHS[arm_id]), completion_rule="driver-exit", image=None)


def test_optional_image_is_kept():
    arm = arms.load_arm(arm_options("superpowers", image="example-arms-image:test"))
    assert arm.image == "example-arms-image:test"


def without(arm_id, key):
    options = arm_options(arm_id)
    del options["arm"][key]
    return options


@pytest.mark.parametrize(("options", "message"), [
    ({"arm": "superpowers"}, "arm must be a table"),
    ({"arm": {**arm_options("native")["arm"], "extra": 1}}, "unknown keys"),
    (without("openspec", "completion_rule"), "missing keys"),
    (arm_options("native", id="codex"), "arm id must be one of"),
    (arm_options("native", driver_command=[]), "driver_command"),
    (arm_options("native", driver_command=["arm-native", ""]), "driver_command"),
    (arm_options("native", driver_command="arm-native"), "driver_command"),
    (arm_options("superpowers", prompt_template="native"), "prompt_template"),
    (arm_options("native", completion_rule="exit-code"), "completion_rule must be driver-exit"),
    (arm_options("superpowers", workflow_owned_paths=["/etc/**"]), "workflow_owned_paths"),
    (arm_options("superpowers", workflow_owned_paths=["../openspec/**"]), "workflow_owned_paths"),
    (arm_options("superpowers", workflow_owned_paths=["openspec\\**"]), "workflow_owned_paths"),
    (arm_options("superpowers", workflow_owned_paths=[]), "must declare its workflow_owned_paths"),
    (arm_options("native", workflow_owned_paths=["openspec/**"]), "owns no workflow_owned_paths"),
    (arm_options("loop-engine", image=""), "image must be"),
    (arm_options("loop-engine", image=3), "image must be"),
    ({**arm_options("native"), "driver_command": ["sc-driver"]}, "must be set inside arm"),
    ({**arm_options("native"), "image": "example-arms-image:test"}, "must be set inside arm"),
])
def test_invalid_arm_is_refused(options, message):
    with pytest.raises(ValueError, match=message):
        arms.load_arm(options)


@pytest.mark.parametrize("arm_id", arms.ARM_IDS)
def test_each_arm_renders_neutral_base_plus_its_addition(arm_id):
    prompt = render(arms.load_arm(arm_options(arm_id)))
    assert prompt.startswith("Run this software-change request unattended and exit by yourself")
    assert "Reach the owner ONLY through ask-owner" in prompt
    assert "/workspace/local-remote.git, configured as origin" in prompt
    assert "You may initialize Git" not in prompt
    assert prompt.endswith("\nOwner request:\nAdd retries.\n")


def test_native_prompt_is_the_base_alone_and_names_no_workflow():
    prompt = render(arms.load_arm(arm_options("native")))
    assert prompt == arms.BASE_PROMPT.format(workflow="", scripted="", request="Add retries.")
    for name in ("Superpowers", "OpenSpec", "Loop Engine", "sc-implement", "intent.json", "Profile:"):
        assert name not in prompt


def test_superpowers_prompt_names_only_superpowers():
    prompt = render(arms.load_arm(arm_options("superpowers")))
    assert "Use the Superpowers workflow installed in this Pi session for this request." in prompt
    assert "OpenSpec" not in prompt and "sc-implement" not in prompt


def test_openspec_prompt_names_only_openspec():
    prompt = render(arms.load_arm(arm_options("openspec")))
    assert "Use the OpenSpec workflow initialised in this repository for this request." in prompt
    assert "Superpowers" not in prompt and "sc-implement" not in prompt


def test_loop_engine_prompt_carries_profile_run_and_engine_steps():
    prompt = render(arms.load_arm(arm_options("loop-engine")))
    assert "Profile: standard-17\n" in prompt
    assert "run ID run-7" in prompt
    assert "sc-implement and sc-review" in prompt
    assert "reaching a terminal engine state; merely saying you are done does not count." in prompt
    assert "Leave final intent.json" in prompt


def test_arm_prompt_passes_scripted_marker_through():
    prompt = render(arms.load_arm(arm_options("superpowers")), scripted="Scripted driver: never-asks")
    assert "\nScripted driver: never-asks\nOwner request:\n" in prompt


def test_workflow_owned_matcher_for_superpowers_arm():
    arm = arms.load_arm(arm_options("superpowers"))
    cases = {
        "openspec/changes/add-retry/proposal.md": True,
        ".pi/skills/opsx/apply.md": True,
        "docs/superpowers/plans/retry.md": True,
        ".superpowers/state.json": True,
        "src/openspec/lib.rs": False,
        "openspec.md": False,
        "docs/superpowers.md": False,
        "src/retry.rs": False,
    }
    for path, owned in cases.items():
        assert arms.is_workflow_owned(path, arm) is owned, path


def test_workflow_owned_matcher_glob_rules():
    arm = arms.ArmDescriptor(id="openspec", driver_command=("arm-openspec",), prompt_template="openspec",
                             workflow_owned_paths=("src/*.rs", "**/gen/**"), completion_rule="driver-exit")
    cases = {
        "src/a.rs": True,
        "src/x/a.rs": False,
        "a/gen/b.txt": True,
        "gen/b.txt": True,
        "a/genx/b.txt": False,
    }
    for path, owned in cases.items():
        assert arms.is_workflow_owned(path, arm) is owned, path


def test_native_and_loop_engine_own_no_repo_paths():
    for arm_id in ("native", "loop-engine"):
        arm = arms.load_arm(arm_options(arm_id))
        assert arms.is_workflow_owned("openspec/changes/a.md", arm) is False
