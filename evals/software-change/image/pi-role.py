#!/usr/bin/env python3
"""One-shot Pi commands: callers supply prompts, never a provider/model option."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

ROLES = {"sc-driver": "role-driver", "sc-implement": "role-implementer", "sc-review": "role-reviewer"}
ROOT = Path("/opt/sc-evals")


def configuration(role):
    return {"providers": {"inspect": {
        "baseUrl": os.environ.get("SC_BRIDGE_URL", "http://localhost:13131/v1"),
        "api": "openai-completions", "apiKey": "inspect-no-credential",
        "models": [{"id": role, "name": role, "reasoning": False,
                    "input": ["text"], "contextWindow": 128000, "maxTokens": 16384,
                    "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0},
                    "compat": {"supportsDeveloperRole": False, "supportsStore": False}}]
    }}}


def session_arguments():
    """Keep the session record in SC_PI_SESSION_DIR when it is set; unset or empty keeps the ephemeral session."""
    directory = os.environ.get("SC_PI_SESSION_DIR")
    return ["--session-dir", directory] if directory else ["--no-session"]


def main():
    if sys.argv[1:] == ["--write-templates"] and Path(sys.argv[0]).name == "pi-role.py":
        for role in ROLES.values():
            directory = ROOT / "pi" / role
            directory.mkdir(parents=True, exist_ok=True)
            (directory / "models.json").write_text(json.dumps(configuration(role), indent=2) + "\n")
        return 0
    role = ROLES[Path(sys.argv[0]).name]
    prompts = sys.argv[1:]
    if prompts and prompts[0] == "--":
        prompts = prompts[1:]
    elif any(prompt.startswith("-") for prompt in prompts):
        print("Supply prompts or @files only; model and runtime options are harness-owned.", file=sys.stderr)
        return 2
    # Separate per-invocation state prevents simultaneous reviewers sharing sessions.
    with tempfile.TemporaryDirectory(prefix=f"pi-{role}-") as directory:
        Path(directory, "models.json").write_text(json.dumps(configuration(role)))
        env = dict(os.environ, PI_CODING_AGENT_DIR=directory)
        command = ["pi", "--print", *session_arguments(), "--no-extensions", "--no-skills",
                   "--no-prompt-templates", "--no-themes", "--provider", "inspect",
                   "--model", role, "--models", role,
                   "--skill", str(ROOT / "skills/using-loop-engine"),
                   "--skill", str(ROOT / "skills/using-software-change-provider")]
        if role != "role-driver":
            command += ["--append-system-prompt",
                        "You are a delegated worker. Return evidence to the driver; do not mutate "
                        "Loop Engine workflow state. " +
                        ("Implement the assigned scoped change." if role == "role-implementer" else
                         "Review the assigned subject independently; do not edit source files.")]
        # -- prevents prompt text from overriding pinned options.
        return subprocess.call([*command, "--", *prompts], env=env)


if __name__ == "__main__":
    sys.exit(main())
