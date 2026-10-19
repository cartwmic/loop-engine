#!/usr/bin/env python3
"""One-shot Pi driver for the arms: arm-native, arm-superpowers and arm-openspec (prompts only, like pi-role.py)."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT = Path("/opt/sc-evals")
ROLE = "role-driver"


def load_pi_role():
    """pi-role.py owns the model configuration and the session hook; the arm launchers reuse them unchanged."""
    spec = importlib.util.spec_from_file_location("pi_role", ROOT / "pi-role.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


PI_ROLE = load_pi_role()
ARMS = ("arm-native", "arm-superpowers", "arm-openspec")


def run_openspec_init():
    """Pinned OpenSpec init in the case repository. Its output goes to stderr, so stdout stays the final message."""
    env = dict(os.environ, OPENSPEC_TELEMETRY="0", OPENSPEC_NO_UPDATE_CHECK="1")
    return subprocess.call(["openspec", "init", "--tools", "pi"], env=env, stdout=sys.stderr)


def arm_arguments(name):
    """Extra Pi arguments that load one arm's setup; native loads nothing beyond the base command."""
    if name == "arm-superpowers":
        # The pinned package declares its extension and skills, so one path loads both.
        return ["--extension", str(ROOT / "arms" / "superpowers")]
    if name == "arm-openspec":
        # Explicit paths load the skills and prompt templates that init wrote into the repository.
        return ["--skill", str(Path.cwd() / ".pi" / "skills"),
                "--prompt-template", str(Path.cwd() / ".pi" / "prompts")]
    return []


def main():
    name = Path(sys.argv[0]).name
    if name not in ARMS:
        print("Run pi-arm.py through one arm launcher name: " + ", ".join(ARMS), file=sys.stderr)
        return 2
    prompts = sys.argv[1:]
    if prompts and prompts[0] == "--":
        prompts = prompts[1:]
    elif any(prompt.startswith("-") for prompt in prompts):
        print("Supply prompts or @files only; model and runtime options are harness-owned.", file=sys.stderr)
        return 2
    if name == "arm-openspec":
        status = run_openspec_init()
        if status:
            return status
    with tempfile.TemporaryDirectory(prefix="pi-arm-") as directory:
        Path(directory, "models.json").write_text(json.dumps(PI_ROLE.configuration(ROLE)))
        env = dict(os.environ, PI_CODING_AGENT_DIR=directory)
        command = ["pi", "--print", *PI_ROLE.session_arguments(), "--no-extensions", "--no-skills",
                   "--no-prompt-templates", "--no-themes", "--provider", "inspect",
                   "--model", ROLE, "--models", ROLE, *arm_arguments(name)]
        # -- prevents prompt text from overriding pinned options.
        return subprocess.call([*command, "--", *prompts], env=env)


if __name__ == "__main__":
    sys.exit(main())
