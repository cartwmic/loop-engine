"""Scripted lean-change arm driver for the harness proof: no model, no network. Its text is passed as an arm's
driver_command (`python3 -c TEXT --profile b|c PROMPT`), so it runs in the trial container without a copy step.

It drives the real loop-engine and lean-change binaries through the public example case: ask the owner once, draft an
intent with executable checks, run the owner check-in, deliberately request a transition that must be denied, write
labels.py, verify, and (profile b) hand the packet to a scripted "fresh reviewer" and dispose of its one finding.
Harness mechanics only: never a model's behaviour and never a semantic review. It reads no hidden file."""
import argparse
import json
from pathlib import Path
import subprocess
import sys

REPOSITORY = Path("/workspace/public/repo")
TRIAL = Path("/workspace/trial")
WORK = TRIAL / "work"
RUN_ID = "eval-trial"
PRINCIPLES = "/opt/sc-evals/owner-context/design-principles.md"
QUESTIONS = ("intent", "scope", "edge-cases", "checks", "docs", "principles")
OWNER = {"name": "owner", "kind": "human"}
WORKER = {"name": "scripted-driver", "kind": "script"}
LABELS = ('"""Grouping keys for task-board labels."""\n\n\n'
          'def label_key(label: str) -> str:\n'
          '    return " ".join(label.split()).casefold() or "inbox"\n')
CHECK = ("from labels import label_key; assert label_key('  Straße   X ') == 'strasse x'; "
         "assert label_key('   ') == 'inbox'; assert label_key('A') == label_key('a')")


def run(argv, **kwargs):
    return subprocess.run(argv, check=True, capture_output=True, text=True, **kwargs).stdout


def engine(*arguments):
    return run(["loop-engine", "--database", str(TRIAL / "run.sqlite"), "--config", str(TRIAL / "providers.toml"),
                "--json", *arguments], cwd=REPOSITORY)


def observe():
    return engine("show", "--view", "full", RUN_ID)  # the engine refuses a mutation of a run it has not just shown


def append(record_id, kind, data):
    observe()
    engine("append", "--record-id", record_id, RUN_ID, kind, json.dumps(data))


def event(name):
    observe()
    engine("event", RUN_ID, name)


def denied(name):
    observe()
    result = subprocess.run(["loop-engine", "--database", str(TRIAL / "run.sqlite"), "--config", str(TRIAL / "providers.toml"),
                             "--json", "event", RUN_ID, name], cwd=REPOSITORY, capture_output=True, text=True)
    assert result.returncode != 0, "the transition should have been denied"


def main(argv=None):
    parser = argparse.ArgumentParser(prog="scripted-lean-driver")
    parser.add_argument("--profile", choices=("b", "c"), required=True)
    parser.add_argument("prompt", nargs="?", default="")
    options = parser.parse_args(sys.argv[1:] if argv is None else argv)
    run(["ask-owner", "Which label rule applies to whitespace?"])
    WORK.mkdir(parents=True, exist_ok=True)
    setup = ["lean-change", "setup", "--profile", options.profile, "--repo", str(REPOSITORY)]
    if options.profile == "b":
        setup += ["--principles", PRINCIPLES]
    (WORK / "profile.json").write_text(run(setup))
    engine("start", "--id", RUN_ID, "lean-change", "@" + str(WORK / "profile.json"), "scripted lean run")
    intent = {"author": WORKER, "goal": "label_key groups different spellings of one label.", "constraints": ["Edit labels.py only."],
              "hard": False, "criteria": [{"id": "keys", "statement": "whitespace, casefold and blank fallback",
                                           "check": {"argv": ["env", "PYTHONDONTWRITEBYTECODE=1", "python3", "-c", CHECK],
                                                     "timeout_ms": 60000}}]}
    (WORK / "intent.json").write_text(json.dumps(intent))
    append("intent-1", "intent", intent)
    event("intent-ready")
    reply = run(["python3", "/opt/sc-evals/confirm_intent.py", str(WORK / "intent.json")]).strip() or "Yes, go ahead."
    append("d-1", "owner-decision", {"checkpoint": "intent", "subject": "intent", "subject_revision": "intent-1",
                                     "decision": "approve", "text": reply, "actor": OWNER})
    event("approved")
    (REPOSITORY / "labels.py").write_text(LABELS)
    event("ready")
    denied("verified")  # no verification record yet: the one denial this driver provokes
    show = observe()
    verification = json.loads(run(["lean-change", "verify"], input=show, cwd=REPOSITORY))
    append("v-1", "verification", verification)
    event("verified")
    if options.profile == "b":
        packet = run(["lean-change", "review-packet"], input=observe(), cwd=REPOSITORY)
        assert all(q in packet for q in QUESTIONS), "packet lacks a fixed question"
        answers = [{"question": q, "verdict": "pass", "note": "scripted"} for q in QUESTIONS]
        answers[-1]["verdict"] = "fail"
        finding = {"id": "f1", "question": "principles", "text": "Scripted finding.", "evidence": "labels.py",
                   "consequence": "None; harness proof only."}
        append("rev-1", "review", {"intent": "intent-1", "tree": verification["tree"], "reviewer": {"name": "scripted-reviewer", "kind": "agent"},
                                   "fresh_context": True, "answers": answers, "findings": [finding]})
        append("disp-1", "finding-disposition", {"review": "rev-1", "finding": "f1", "disposition": "rejected",
                                                 "reason": "Scripted proof, no change needed."})
        event("reviewed")
    print("Scripted lean driver finished.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
