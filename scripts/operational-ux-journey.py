#!/usr/bin/env python3
"""Public operational UX fixture entry point; case owners add public-path proofs.

This is not a test runner: existing commands remain the proof implementations.
Unimplemented cases fail closed, never produce a passing scenario receipt.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import sys
import subprocess

from test_contract import ContractError, ROOT, repository_proof_identity

CASES = ("show", "capture", "monitor", "summary", "validation", "policy-selection",
         "guidance", "packaged", "delivery", "bookends")


def absolute_directory(value: str) -> Path:
    path = Path(value)
    if not path.is_absolute() or not path.is_dir():
        raise argparse.ArgumentTypeError("requires an existing absolute directory")
    return path.resolve()


def output_directory(value: str) -> Path:
    path = Path(value)
    if not path.is_absolute():
        raise argparse.ArgumentTypeError("requires an absolute output directory")
    path = path.resolve()
    if path == ROOT or ROOT in path.parents:
        raise argparse.ArgumentTypeError("fixture output must be outside the checkout")
    if path.exists() and not path.is_dir():
        raise argparse.ArgumentTypeError("output is not a directory")
    return path


def show_case(args: argparse.Namespace) -> None:
    """Drive only isolated candidate catalogs and a scripted external provider."""
    import time

    root = args.attempt
    engine = args.binary_dir / "loop-engine"
    provider = root / "provider.py"
    provider.write_text('''#!/usr/bin/env python3
import json, pathlib, sys
r = json.load(sys.stdin)
root = pathlib.Path(__file__).parent
if r['operation'] == 'describe':
    state = {'id':'work','title':'Work','instructions':'Perform external work, append evidence, then request check. Preserve valid evidence; inspect full context.','final':False}
    if not r['initial_input'].get('legacy'):
        state['action_guidance'] = {'authors': {'ordinary':7, 'challenge':3, 'criterion':2, 'goal':4}, 'repair':'Correct validation locally; select owning tasks for implementation defects; honest no-task repair only when no task owns it. Upstream revision invalidates affected downstream proof, not all work.'}
    print(json.dumps({'id':'show-proof','initial_state':'work','states':[state],
       'work_slots':[{'id':'worker','state':'work','event':'check'}],
       'transitions':[{'source':'work','event':'check','target':'work','kind':'checked'}, {'source':'work','event':'loop','target':'work','kind':'check-free'}]}))
else:
    with (root/'provider-calls').open('a') as f: f.write('evaluate\\n')
    print(json.dumps({'result':'allow'} if (root/'allow').exists() else {'result':'deny','feedback':{'code':'missing-proof','message':'Retained fixture denial, not current approval'}}))
''')
    provider.chmod(0o755)
    config = root / "providers.toml"
    config.write_text('[providers.fixture]\ncommand = ' + json.dumps(str(provider)) + '\n')
    receipts = []

    def call(database, arguments, expected="completed", human=False):
        argv = [str(engine), "--database", str(database), "--config", str(config)]
        if not human:
            argv.append("--json")
        argv += arguments
        index = len(receipts)
        start = time.time()
        process = subprocess.run(argv, cwd=root, capture_output=True, text=True, timeout=30)
        stdout = root / f"call-{index:03}.stdout"
        stderr = root / f"call-{index:03}.stderr"
        stdout.write_text(process.stdout)
        stderr.write_text(process.stderr)
        receipts.append({"argv":argv, "cwd":str(root), "exit_code":process.returncode,
                         "started_at":start, "finished_at":time.time(), "stdout":str(stdout), "stderr":str(stderr)})
        (root / "commands.json").write_text(json.dumps(receipts, indent=2))
        if human:
            if process.returncode != 0:
                raise ContractError(f"human show failed: {stderr}")
            return process.stdout
        value = json.loads(process.stdout)
        if value.get("status") != expected:
            raise ContractError(f"expected {expected}, got {value}: {stdout}")
        return value

    outcomes = []
    for legacy, bound in [(False, False), (False, True), (True, False), (True, True)]:
        name = f"show-{legacy}-{bound}"
        database = root / (name + ".sqlite")
        initial = {"legacy":legacy}
        if bound:
            initial["work_slot_bindings"] = {"worker":{"command":sys.executable,
                "args":["-c", "import sys,time;sys.stdin.read();time.sleep(0.5)"]}}
        call(database, ["start", "--id", name, "fixture", json.dumps(initial)])
        for view in [["--view","status"], ["--compact"]]:
            status = call(database, ["show",name,*view])["result"]
            assert status["mutation_armed"] is False and "current_state_instructions" not in status
            call(database, ["show",name,*view], human=True)
            for mutation in [["append",name,"note","{}"], ["event",name,"loop"],
                             ["terminate",name]] + ([["invoke",name,"worker"]] if bound else []):
                denied = call(database, mutation, "rejected")
                assert denied["code"] == "run-not-observed", denied
        action = call(database, ["show",name])["result"]
        assert action["mutation_armed"] and action["current_state_instructions"]
        assert action["execution_paths"][0]["mode"] == ("bound" if bound else "unbound")
        assert "context" not in action and "initial_input" not in action
        if legacy:
            assert "unknown" in action["guidance_status"]
        else:
            assert action["action_guidance"]["authors"] == {"ordinary":7,"challenge":3,"criterion":2,"goal":4}
        for i in range(12):
            call(database, ["append",name,"unrelated",json.dumps({"payload":"history"*1000})])
        later = call(database, ["show",name])["result"]
        assert {k:v for k,v in action.items() if k != "observed_at"} == {k:v for k,v in later.items() if k != "observed_at"}
        full = call(database, ["show",name,"--view","full"])["result"]
        assert len(full["context"]) == 12
        if bound:
            from work_slot_journey import assert_bound_redaction
            assert_bound_redaction(full, run_id=name, slot_id="worker",
                                   command=sys.executable,
                                   args=initial["work_slot_bindings"]["worker"]["args"])
            for _ in range(3):
                call(database, ["invoke",name,"worker"])
                active = call(database, ["show",name])["result"]
                assert active["work_slot_invocations"] and active["work_slot_invocations"][0]["capture_dir"]
                assert "binding" not in active["work_slot_invocations"][0]
                deadline = time.monotonic() + 20
                while call(database, ["show",name,"--view","status"])["result"]["work_slot_invocations"]:
                    if time.monotonic() > deadline:
                        raise ContractError("bound fixture did not become quiescent")
                    time.sleep(0.05)
            after = call(database, ["show",name])["result"]
            assert not after["work_slot_invocations"]
            assert len(after["latest_current_slot_execution"]) == 1
            assert "triage" in after["next_action"]
            dynamic = {"observed_at", "next_action", "latest_current_slot_execution", "visibility",
                       "workflow", "execution", "worker", "conformance", "acceptance", "evidence",
                       "freshness", "uncertainty", "owner_update_guidance"}
            assert {k:v for k,v in action.items() if k not in dynamic} == {k:v for k,v in after.items() if k not in dynamic}
            assert len(call(database, ["show",name,"--view","full"])["result"]["work_slot_invocations"]) == 3
        # Provider disappears: persisted instructions/guidance and full remain usable.
        unavailable = root / "provider-unavailable.py"
        provider.rename(unavailable)
        try:
            offline = call(database, ["show",name])["result"]
            assert offline["current_state_instructions"] == action["current_state_instructions"]
            call(database, ["show",name,"--view","full"])
        finally:
            unavailable.rename(provider)
        if not bound:
            call(database, ["event",name,"check"], "rejected")
            (root / "allow").write_text("fixture decision")
            call(database, ["event",name,"check"])
            (root / "allow").unlink()
            # Full arms the new visit after a self-loop, status cannot.
            call(database, ["show",name,"--view","status"])
            assert call(database, ["append",name,"note","{}"], "rejected")["code"] == "run-not-observed"
            call(database, ["show",name,"--view","full"])
            call(database, ["append",name,"note","{}"])
            call(database, ["event",name,"check"], "rejected")
            full = call(database, ["show",name,"--view","full"])["result"]
            history = full["evaluation_history"]
            assert [row["result"]["result"] for row in history] == ["deny","allow","deny"], history
            assert len(full["latest_evaluations"]) == 1
            assert [row["sequence"] for row in history] == sorted(row["sequence"] for row in history)
            assert all(row["occurred_at"] is not None and row["transition"]["event"] == "check" for row in history)
            call(database, ["event",name,"check","--override",json.dumps({"state_visit":full["state_visit"],"owner":"fixture-owner","reason":"Scripted exception; denial remains denial"})])
            exceptional = call(database, ["show",name,"--view","full"])["result"]
            assert exceptional["evaluation_history"] == history and exceptional["override_count"] == 1
            assert "overridden" in json.dumps(call(database, ["history",name])["result"])
            blocker = call(database, ["show",name])["result"]["blocker_assessment"]
            assert blocker["freshness"] == "unknown" and blocker["sources"]
        outcomes.append({"legacy":legacy,"bound":bound,"status":"passed","database":str(database)})
    (root / "scenarios.json").write_text(json.dumps(outcomes, indent=2))


def run_case(args: argparse.Namespace) -> None:
    if args.case == "delivery":
        from operational_ux_delivery import delivery_case
        delivery_case(args)
        return
    if args.case == "show":
        show_case(args)
        return
    if args.case == "policy-selection":
        from operational_ux_policy_selection import policy_selection_case
        policy_selection_case(args)
        return
    if args.case == "bookends":
        from operational_ux_bookends import bookends_case
        bookends_case(args)
        return
    if args.case == "packaged":
        from operational_ux_packaged import packaged_case
        packaged_case(args)
        return
    if args.case == "summary":
        from operational_ux_summary import summary_case
        summary_case(args)
        return
    if args.case == "monitor":
        from operational_ux_monitor import monitor_case
        monitor_case(args)
        return
    if args.case == "guidance":
        from operational_ux_guidance import guidance_case
        guidance_case(args)
        return
    if args.case == "validation":
        import runpy
        runpy.run_path(str(ROOT / "scripts/operational_ux_validation.py"), init_globals={"ux_args": args})
        return
    if args.case == "capture":
        from operational_ux_capture import capture_case
        capture_case(args)
        return
    raise ContractError(f"case {args.case!r} is not implemented; owning task must supply public-path assertions")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--case", choices=CASES, required=True)
    parser.add_argument("--binary-dir", type=absolute_directory, required=True)
    parser.add_argument("--released-root", type=absolute_directory, required=True)
    parser.add_argument("--output-root", type=output_directory, required=True)
    args = parser.parse_args()
    args.output_root.mkdir(parents=True, exist_ok=True)
    # Never overwrite an earlier failed or successful attempt.
    import tempfile
    attempt = Path(tempfile.mkdtemp(prefix=f"{args.case}-", dir=args.output_root))
    args.attempt = attempt
    before = repository_proof_identity(ROOT)
    try:
        run_case(args)
    except (ContractError, OSError, AssertionError, ValueError, subprocess.SubprocessError) as error:
        outcome = {"case": args.case, "status": "failed", "diagnostic": str(error)}
        exit_code = 1
    else:
        outcome = {"case": args.case, "status": "passed"}
        exit_code = 0
    outcome.update(repository_before=before, repository_after=repository_proof_identity(ROOT))
    (attempt / "outcome.json").write_text(json.dumps(outcome, indent=2) + "\n")
    print(json.dumps({**outcome, "artifact_root": str(attempt)}))
    return exit_code


if __name__ == "__main__":
    sys.exit(main())
