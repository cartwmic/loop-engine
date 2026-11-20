"""Scripted lean-change arm driver for the harness proof: no model, no network. Its text is passed as an arm's
driver_command (`python3 -c TEXT --profile b|c PROMPT`), so it runs in the trial container without a copy step.

It drives the real loop-engine and lean-change binaries through the public example case: ask the owner once, draft an
intent with executable checks, (profile b) hand the intent packet to a scripted "fresh reviewer" and dispose of its one
finding, run the owner check-in, deliberately request a transition that must be denied, write labels.py, verify, and
(profile b) do the same with the review packet. Each scripted reviewer record passes `lean-change review-check` first.
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
REVIEW_QUESTIONS = ("intent", "scope", "edge-cases", "checks", "docs", "principles", "smaller-design")
CHECK_QUESTIONS = ("use", "edge", "control", "terms", "criteria-gap", "principles-gap")
REVIEWER = {"name": "scripted-reviewer", "kind": "agent"}
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


def reviewed(kind, questions, packet, base):
    """The scripted reviewer's record: it checks the packet holds each fixed question, fails the last one with one
    blocking finding and pipes the record through the provider's own review-check, as a real reviewer must."""
    assert all(q in packet for q in questions), "packet lacks a fixed question"
    answers = [{"question": q, "verdict": "pass", "note": "scripted"} for q in questions]
    answers[-1]["verdict"] = "fail"
    finding = {"id": "f1", "severity": "blocking", "question": questions[-1], "text": "Scripted finding.",
               "obligation": "None; harness proof only.", "evidence": "labels.py", "consequence": "None; harness proof only."}
    record = {"findings_schema": 1, "intent": "intent-1", "reviewer": REVIEWER, "fresh_context": True, "answers": answers,
              "findings": [finding]} | base
    run(["lean-change", "review-check", "--kind", kind], input=json.dumps(record))
    return record


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
    # The example case's labels.py at the base commit returns the label unchanged, so the check fails on the old tree: old "fails".
    intent = {"author": WORKER, "goal": "label_key groups different spellings of one label.", "constraints": ["Edit labels.py only."],
              "hard": False, "criteria": [{"id": "keys", "statement": "whitespace, casefold and blank fallback",
                                           "check": {"argv": ["env", "PYTHONDONTWRITEBYTECODE=1", "python3", "-c", CHECK],
                                                     "timeout_ms": 60000}, "old": {"expect": "fails"}}]}
    (WORK / "intent.json").write_text(json.dumps(intent))
    append("intent-1", "intent", intent)
    if options.profile == "b":
        packet = run(["lean-change", "intent-packet"], input=observe(), cwd=REPOSITORY)
        append("chk-1", "intent-check", reviewed("intent-check", CHECK_QUESTIONS, packet, {}))
        append("chk-disp-1", "finding-disposition", {"review": "chk-1", "finding": "f1", "disposition": "rejected",
                                                     "reason": "Scripted proof, no change needed."})
    event("intent-ready")
    reply = run(["python3", "/opt/sc-evals/confirm_intent.py", str(WORK / "intent.json")]).strip() or "Yes, go ahead."
    append("d-1", "owner-decision", {"checkpoint": "intent", "subject": "intent", "subject_revision": "intent-1",
                                     "decision": "approve", "text": reply, "actor": OWNER})
    event("approved")
    (REPOSITORY / "labels.py").write_text(LABELS)
    event("ready")
    denied("verified")  # no verification record yet: the one denial this driver provokes
    verification = json.loads(run(["lean-change", "verify"], input=observe(), cwd=REPOSITORY))
    append("v-1", "verification", verification)
    event("verified")
    if options.profile == "b":
        packet = run(["lean-change", "review-packet"], input=observe(), cwd=REPOSITORY)
        append("rev-1", "review", reviewed("review", REVIEW_QUESTIONS, packet, {"tree": verification["tree"]}))
        append("disp-1", "finding-disposition", {"review": "rev-1", "finding": "f1", "disposition": "rejected",
                                                 "reason": "Scripted proof, no change needed."})
        event("reviewed")
    print("Scripted lean driver finished.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
