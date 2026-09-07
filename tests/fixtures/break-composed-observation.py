"""Mutation proof: run the actual outer journey with a missing terminal observation.

Does not edit source, provider state, catalogs or captures. Only the observer's
copy loses lifecycle; the original required assertion must fail the CLI process.
All remaining argv are the ordinary software-change journey public arguments.
"""
import copy
import hashlib
import json
from pathlib import Path
import runpy
import sys

root = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(root / "scripts"))
import recovery_composed

original = recovery_composed.assert_terminal


def missing_observation(shown, *args):
    observation = copy.deepcopy(shown)
    observation["lifecycle"] = "observation-deliberately-missing"
    print("deliberately removed required terminal observation", flush=True)
    try:
        original(observation, *args)
    except AssertionError:
        # The actual observation and real failed bound outputs must survive the
        # observer mutation. Retain them before propagating the outer failure.
        assert shown['lifecycle'] == 'final'
        failures = [r for r in shown['context'] if r['kind'] == 'criterion-verdict'
                    and r['data']['result'] == 'fail']
        assert failures
        invocation_id = failures[0]['data']['origin']['id']
        invocation = next(i for i in shown['work_slot_invocations'] if i['invocation_id'] == invocation_id)
        capture = Path(invocation['capture_dir'])
        raw_outputs = list(capture.glob('*/attempts/*/stdout'))
        assert any('"fail"' in p.read_text() for p in raw_outputs)
        root = Path(shown['initial_input']['artifact_root'])
        receipt = {'actual_terminal': shown, 'history': args[0], 'failures': failures,
                   'raw_captures': {str(p): hashlib.sha256(p.read_bytes()).hexdigest() for p in raw_outputs}}
        (root / 'broken-observation-receipt.json').write_text(json.dumps(receipt, indent=2))
        print('outer failure retained actual terminal, failed verdicts, history and raw captures', flush=True)
        raise
    raise AssertionError("missing terminal observation was accepted")


recovery_composed.assert_terminal = missing_observation
sys.argv[0] = str(root / "scripts/software-change-journey.py")
runpy.run_path(sys.argv[0], run_name="__main__")
