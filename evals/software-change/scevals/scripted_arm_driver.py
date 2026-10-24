"""Scripted arm driver for the scoring parity and snapshot proofs: no model, no network.

Its text is passed as an arm's driver_command (`python3 -c TEXT OPTIONS... PROMPT`), so it runs in the trial
container without a copy step. Each run does what its options name: ask the owner once through ask-owner, write the
public example's labels.py (correct, wrong, or staged), optionally start a Loop Engine run, then end as named.
It is harness mechanics only, never a model's behaviour and never a semantic review. It reads no hidden file.
"""
import argparse
import json
import math
import os
from pathlib import Path
import subprocess
import sys
import time
import urllib.request

REPOSITORY = Path("/workspace/public/repo")
WORK = Path("/workspace/trial")
RUN_ID = "eval-trial"
OWNER_QUESTION = "Which label rule applies to whitespace?"
CORRECT_LABELS = ('"""Grouping keys for task-board labels."""\n\n\n'
                  'def label_key(label: str) -> str:\n'
                  '    return " ".join(label.split()).casefold() or "inbox"\n')
# Without the empty-label fallback: only the REQ-3 hidden tests fail.
WRONG_LABELS = ('"""Grouping keys for task-board labels."""\n\n\n'
                'def label_key(label: str) -> str:\n'
                '    return " ".join(label.split()).casefold()\n')


def parse(argv):
    parser = argparse.ArgumentParser(prog="scripted-arm-driver")
    parser.add_argument("--engine", choices=("none", "terminated", "active", "crashed"), default="none",
                        help="Loop Engine run to start and the state to leave it in; crashed exits 137 while it is active")
    parser.add_argument("--code", choices=("correct", "wrong", "staged"), default="correct",
                        help="labels.py content; staged writes wrong, waits, writes correct, waits")
    parser.add_argument("--seconds", type=float, default=0.0, help="wait after each write")
    parser.add_argument("--keepalive", action="store_true",
                        help="during each wait, make max(1, ceil(seconds)) scripted model calls through the bridge, "
                             "spaced across the wait, which the progress poll sees")
    parser.add_argument("--checkin", action="store_true",
                        help="after the first ask-owner, write a design note and send it to the owner check-in twice "
                             "(the second call must be refused); the reply is written to /workspace/trial/checkin-replies.txt")
    parser.add_argument("--end", choices=("exit", "hang"), default="exit",
                        help="hang sleeps forever after the work, so an idle or hard stop must end it")
    parser.add_argument("prompt", nargs="?", default="", help="the harness prompt; not used by this driver")
    return parser.parse_args(argv)


def engine(*arguments):
    """One loop-engine JSON command against the trial's run database; a failed command raises."""
    return subprocess.run(["loop-engine", "--database", str(WORK / "run.sqlite"),
                           "--config", str(WORK / "providers.toml"), "--json", *arguments],
                          cwd=REPOSITORY, check=True, capture_output=True, text=True)


def start_engine():
    """Start the shipped software-change profile named by SC_PROFILE, with the public request as the run label."""
    data = WORK / "data"
    subprocess.run(["software-change", "data-dump", str(data)], check=True, capture_output=True)
    profiles = [json.loads(path.read_text()) for path in (data / "crates/software-change-provider/data/configs").glob("*.json")]
    profile_id = os.environ.get("SC_PROFILE", "standard-16")
    profile = next(p for p in profiles if p["config_version"] == profile_id)
    profile["artifact_root"] = str(WORK / "artifacts")
    (WORK / "artifacts").mkdir(parents=True, exist_ok=True)
    (WORK / "profile.json").write_text(json.dumps(profile))
    engine("start", "--id", RUN_ID, "software-change", "@" + str(WORK / "profile.json"),
           Path("/workspace/public/request.md").read_text())


def pause(seconds, keepalive):
    """Wait. With keepalive, make max(1, ceil(seconds)) scripted model calls, each followed by the rest of its seconds/n
    slot. The count is fixed: a slow call never drops one and a fast call never adds one, so the driver's token count
    does not depend on call latency."""
    if not keepalive:
        time.sleep(max(0.0, seconds))
        return
    calls = max(1, math.ceil(seconds))
    slot = seconds / calls
    for _ in range(calls):
        started = time.monotonic()
        model_call()
        time.sleep(max(0.0, slot - (time.monotonic() - started)))


def model_call():
    """One no-network completion through the bridge to the driver's scripted model; its tokens count as progress."""
    body = json.dumps({"model": "role-driver", "messages": [{"role": "user", "content": "Progress check."}]}).encode()
    request = urllib.request.Request(os.environ["SC_BRIDGE_URL"] + "/chat/completions", data=body, method="POST",
                                     headers={"content-type": "application/json", "authorization": "Bearer inspect-no-credential"})
    with urllib.request.urlopen(request, timeout=60) as response:
        response.read()


def ask_owner(question):
    answer = subprocess.run(["ask-owner", question], capture_output=True, text=True, check=True)
    return answer.stdout


def write_labels(content):
    (REPOSITORY / "labels.py").write_text(content)


def main(argv=None):
    options = parse(sys.argv[1:] if argv is None else argv)
    ask_owner(OWNER_QUESTION)
    if options.checkin:
        note = REPOSITORY / "design-note.md"
        note.write_text("Intent: label_key trims and casefolds labels.\n")
        replies = []
        for _ in range(2):
            reply = subprocess.run(["python3", "/opt/sc-evals/confirm_intent.py", str(note)], capture_output=True, text=True, check=True)
            replies.append(reply.stdout.strip())
        note.unlink()
        (WORK / "checkin-replies.txt").write_text("\n---\n".join(replies))
    if options.code == "staged":
        write_labels(WRONG_LABELS)
        pause(options.seconds, options.keepalive)
        write_labels(CORRECT_LABELS)
    else:
        write_labels(CORRECT_LABELS if options.code == "correct" else WRONG_LABELS)
    pause(options.seconds, options.keepalive)
    if options.engine != "none":
        start_engine()
    pause(options.seconds, options.keepalive)
    if options.engine == "terminated":
        engine("show", "--view", "full", RUN_ID)  # the engine refuses a mutation of a run it has not shown
        engine("terminate", RUN_ID)
    if options.engine == "crashed":
        return 137  # a crash: the driver exits nonzero while the engine run is still active
    if options.end == "hang":
        time.sleep(10 ** 6)
    print("Scripted arm driver finished.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
