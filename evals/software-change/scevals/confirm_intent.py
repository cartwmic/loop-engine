#!/usr/bin/env python3
"""Owner check-in client: send the agent's intent/proposal/plan files to the simulated owner, print the reply.

Copied into the trial container by the harness. It calls Inspect's generated sc_owner sandbox-service client and
never loads a brief. Usage: python3 /opt/sc-evals/confirm_intent.py FILE [FILE...]  (or pipe the text on stdin).
"""
import importlib.util
import os
from pathlib import Path
import signal
import sys


def timed_out(*_):
    raise TimeoutError()


def read_artifact(paths):
    if not paths:
        return sys.stdin.read()
    parts = []
    for name in paths:
        parts.append("## " + name + "\n" + Path(name).read_text())
    return "\n\n".join(parts)


def main():
    try:
        artifact = read_artifact(sys.argv[1:])
    except OSError as error:
        print("Cannot read the file to confirm: " + str(error), file=sys.stderr)
        return 2
    if not artifact.strip():
        print("Usage: confirm_intent.py FILE [FILE...] (or pipe the intent text on stdin)", file=sys.stderr)
        return 2
    directory = Path("/var/tmp/sandbox-services/sc_owner")
    instance = os.environ.get("SC_OWNER_INSTANCE")
    if instance:
        directory /= instance
    try:
        timeout = int(os.environ.get("SC_OWNER_TIMEOUT_SECONDS", "300"))
        signal.signal(signal.SIGALRM, timed_out)
        signal.alarm(timeout)
        spec = importlib.util.spec_from_file_location("sc_owner", directory / "sc_owner.py")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        answer = module.call_sc_owner("confirm_intent", artifact=artifact)
        if not isinstance(answer, str):
            raise TypeError("owner answer must be text")
        print(answer)
        return 0
    except Exception:
        print("Owner service unavailable, timed out, or returned an invalid answer.", file=sys.stderr)
        return 1
    finally:
        signal.alarm(0)


if __name__ == "__main__":
    sys.exit(main())
