"""Arm descriptor and prompt rendering for the Stage 0 arm comparison.

`target_options.arm` declares one arm: id, driver argv, prompt template, workflow-owned repo
paths, completion rule and optional image. With no arm set, `render_prompt` returns the
existing PREAMBLE rendering unchanged.
"""
from collections.abc import Mapping
from dataclasses import dataclass
from pathlib import PurePosixPath
import re

ARM_IDS = ("native", "superpowers", "openspec", "loop-engine")
COMPLETION_RULE = "driver-exit"
ARM_KEYS = frozenset({"id", "driver_command", "prompt_template", "workflow_owned_paths", "completion_rule", "image"})
REQUIRED_ARM_KEYS = ARM_KEYS - {"image"}
# Set beside an arm, these would leave the driver or image ambiguous, so they belong inside the arm only.
ARM_OWNED_OPTIONS = ("driver_command", "image")
# Spec section 2.1: S and O own repo paths; native and Loop Engine keep no workflow files in the repo.
ARMS_WITH_WORKFLOW_PATHS = ("superpowers", "openspec")

BASE_PROMPT = """Run this software-change request unattended and exit by yourself when the request is complete.
The case repository is /workspace/public/repo. The harness has already created its starting baseline commit
and an empty bare local remote at /workspace/local-remote.git, configured as origin. Do not commit delivered
changes, do not push (pushing there is forbidden and measured), and keep edits within the request's scope.
Reach the owner ONLY through ask-owner; do not ask a human or bypass the service.
The owner has preapproved this exact launch for this disposable trial.
{workflow}{scripted}
Owner request:
{request}
"""

ARM_ADDITIONS = {
    "native": "",
    "superpowers": "Use the Superpowers workflow installed in this Pi session for this request.\n",
    "openspec": "Use the OpenSpec workflow initialised in this repository for this request.\n",
    "loop-engine": """Profile: {profile}
Use the shipped software-change profile with exactly that config_version. Read the installed
using-loop-engine and using-software-change-provider skills. Implementation and review work MUST use
sc-implement and sc-review. Use /workspace/trial/run.sqlite, providers.toml at
/workspace/trial/providers.toml, run ID {run_id}, artifacts /workspace/trial/artifacts. The owner has
preapproved this exact profile. Leave final intent.json and reports under the artifact directory. Finish by
reaching a terminal engine state; merely saying you are done does not count.
""",
}


@dataclass(frozen=True)
class ArmDescriptor:
    id: str
    driver_command: tuple[str, ...]
    prompt_template: str
    workflow_owned_paths: tuple[str, ...]
    completion_rule: str
    image: str | None = None


def load_arm(target_options: Mapping[str, object]) -> ArmDescriptor | None:
    """Validate `target_options.arm`; None means no arm is set and today's behavior applies."""
    if "arm" not in target_options:
        return None
    raw = target_options["arm"]
    if not isinstance(raw, Mapping):
        raise ValueError("arm must be a table with id, driver_command, prompt_template, workflow_owned_paths and completion_rule")
    unknown = set(raw) - ARM_KEYS
    if unknown:
        raise ValueError(f"arm has unknown keys: {sorted(unknown)}")
    missing = REQUIRED_ARM_KEYS - set(raw)
    if missing:
        raise ValueError(f"arm is missing keys: {sorted(missing)}")
    owned = [key for key in ARM_OWNED_OPTIONS if key in target_options]
    if owned:
        raise ValueError(f"{owned} must be set inside arm, not beside it, when arm is set")
    arm_id = raw["id"]
    if arm_id not in ARM_IDS:
        raise ValueError("arm id must be one of native, superpowers, openspec, loop-engine")
    command = raw["driver_command"]
    if not isinstance(command, list) or not command or not all(isinstance(s, str) and s for s in command):
        raise ValueError("arm driver_command must be a nonempty argv array (prompt is appended)")
    if raw["prompt_template"] != arm_id:
        raise ValueError(f"arm prompt_template must be the arm's own template: {arm_id}")
    paths = raw["workflow_owned_paths"]
    if not isinstance(paths, list) or not all(isinstance(p, str) and _is_repo_relative(p) for p in paths):
        raise ValueError("arm workflow_owned_paths must be a list of repo-relative glob strings")
    if arm_id in ARMS_WITH_WORKFLOW_PATHS and not paths:
        raise ValueError(f"arm {arm_id} must declare its workflow_owned_paths")
    if arm_id not in ARMS_WITH_WORKFLOW_PATHS and paths:
        raise ValueError(f"arm {arm_id} owns no workflow_owned_paths")
    if raw["completion_rule"] != COMPLETION_RULE:
        raise ValueError(f"arm completion_rule must be {COMPLETION_RULE}")
    image = raw.get("image")
    if image is not None and (not isinstance(image, str) or not image):
        raise ValueError("arm image must be a nonempty image tag")
    return ArmDescriptor(id=arm_id, driver_command=tuple(command), prompt_template=arm_id,
                         workflow_owned_paths=tuple(paths), completion_rule=COMPLETION_RULE, image=image)


def render_prompt(arm: ArmDescriptor | None, *, profile: str, run_id: str, scripted: str, request: str) -> str:
    if arm is None:
        # PREAMBLE lives in target.py, which imports this module; the import stays here to avoid a cycle.
        from .target import PREAMBLE
        return PREAMBLE.format(profile=profile, run_id=run_id, scripted=scripted, request=request)
    addition = ARM_ADDITIONS[arm.prompt_template].format(profile=profile, run_id=run_id)
    return BASE_PROMPT.format(workflow=addition, scripted=scripted, request=request)


def is_workflow_owned(path: str, arm: ArmDescriptor | None) -> bool:
    """True when a repo-relative POSIX path matches one of the arm's workflow-owned globs."""
    return arm is not None and any(_glob(pattern).fullmatch(path) for pattern in arm.workflow_owned_paths)


def _is_repo_relative(path: str) -> bool:
    pure = PurePosixPath(path)
    return bool(path) and not pure.is_absolute() and ".." not in pure.parts and "\\" not in path


def _glob(pattern: str) -> re.Pattern[str]:
    """`**` crosses path segments (and matches none), `*` and `?` stay inside one segment."""
    parts = []
    index = 0
    while index < len(pattern):
        if pattern.startswith("**/", index):
            parts.append("(?:.*/)?")
            index += 3
        elif pattern.startswith("**", index):
            parts.append(".*")
            index += 2
        elif pattern[index] == "*":
            parts.append("[^/]*")
            index += 1
        elif pattern[index] == "?":
            parts.append("[^/]")
            index += 1
        else:
            parts.append(re.escape(pattern[index]))
            index += 1
    return re.compile("".join(parts))
