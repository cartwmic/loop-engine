#!/usr/bin/env python3
"""Call Inspect's generated sc_owner sandbox-service client, never load a brief."""
import importlib.util
import os
from pathlib import Path
import signal
import sys


def timed_out(*_):
    raise TimeoutError()


def main():
    question = " ".join(sys.argv[1:]) if len(sys.argv) > 1 else sys.stdin.read()
    if not question.strip():
        print("Usage: ask-owner QUESTION (or pipe a question on stdin)", file=sys.stderr)
        return 2
    instance = os.environ.get("SC_OWNER_INSTANCE")
    directory = Path("/var/tmp/sandbox-services/sc_owner")
    if instance:
        directory /= instance
        # The host chooses a safe Inspect service-instance token.
        os.environ["SC_OWNER_INSTANCE"] = instance
    client = directory / "sc_owner.py"
    try:
        timeout = int(os.environ.get("SC_OWNER_TIMEOUT_SECONDS", "120"))
        if timeout <= 0:
            raise ValueError("nonpositive timeout")
        signal.signal(signal.SIGALRM, timed_out)
        signal.alarm(timeout)
        spec = importlib.util.spec_from_file_location("sc_owner", client)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        answer = module.call_sc_owner("ask_owner", question=question)
        if not isinstance(answer, str):
            raise TypeError("owner answer must be text")
        print(answer)
        return 0
    except Exception:
        # Host service errors can contain private text; never echo their contents.
        print("Owner service unavailable, timed out, or returned an invalid answer.", file=sys.stderr)
        return 1
    finally:
        signal.alarm(0)


if __name__ == "__main__":
    sys.exit(main())
