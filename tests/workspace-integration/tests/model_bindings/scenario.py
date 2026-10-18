"""Black-box proof of amendable per-role model bindings through the built programs.

Runs on fresh private catalogs under --output-root with the built loop-engine and
software-change and scripted dummy reviewers (no model, no user catalog, no human review).
"""
import argparse
import importlib.util
import json
from pathlib import Path
import sys
import traceback

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from harness import Harness, configure_with, require  # noqa: E402
from capture_support import Capture  # noqa: E402

import amendment_cases  # noqa: E402
import declaration_cases  # noqa: E402
import eligibility_cases  # noqa: E402
import kind_cases  # noqa: E402
import profile_cases  # noqa: E402


def load(path, name):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def drive(args, journey, name, *, keep, hooks, bindings=None, stop_at=None):
    """One journey run on its own private catalog; `hooks` maps state -> callable(harness).

    Each hook runs once, on the first visit to its state, and then the journey continues
    (or stops at `stop_at`).  Returns (work directory, harness of the last hook).
    """
    root = args.output_root / name
    root.mkdir()
    log = Capture(root / 'commands', args)
    shipped = args.source_root / 'crates/software-change-provider/data/configs/high-rigor.json'
    state = {'done': set(), 'harness': None}

    def exercise(phase, ctx):
        handler = hooks.get(phase)
        if handler is None or phase in state['done']:
            return False
        state['done'].add(phase)
        harness = Harness(ctx, log, name, args.trail)
        state['harness'] = harness
        handler(harness)
        return True

    work = journey.run(args, shipped, root, configure=configure_with(
        args, keep_from_shipped=keep, bindings=bindings), exercise=exercise, stop_at=stop_at,
        command_log=log, judgment_reason='Scripted fixture judgments; no model or human review claim')
    missing = set(hooks) - state['done']
    require(not missing, f'hooks never reached: {sorted(missing)}')
    log.finalize()
    return work, state['harness']


def unbound_run(args, journey):
    return drive(args, journey, 'unbound-amendments',
                 keep={'intent-review': [('solution-agnostic', 'aggregate', 2)]},
                 hooks={'intent-review': amendment_cases.run}, stop_at='intent-review')


def declaration_run(args, journey):
    work, h = drive(args, journey, 'author-implementer-declarations', keep={},
                    bindings=declaration_cases.BINDINGS, hooks=declaration_cases.HOOKS)
    declaration_cases.finished(h)


def cold_reader_run(args, journey):
    drive(args, journey, 'cold-reader-return', bindings=kind_cases.BINDINGS, hooks=kind_cases.HOOKS_COLD,
          keep={'intent-review': [('intent-sufficient', 'aggregate', 1)]}, stop_at='intent-review')


def review_kinds_run(args, journey):
    drive(args, journey, 'review-and-verdict-returns', bindings=kind_cases.BINDINGS, hooks=kind_cases.HOOKS_REVIEW,
          keep={'design-review': [('intent-faithful', 'aggregate', 1)],
                'design-adversarial-review': [('intent-faithful', 'aggregate', 1)]})


def unmarked_run(args, journey):
    drive(args, journey, 'unmarked-amendment', keep={'intent-review': [('solution-agnostic', 'aggregate', 2)]},
          hooks={'intent-review': eligibility_cases.unmarked}, stop_at='intent-review')


def carried_run(args, journey):
    drive(args, journey, 'carried-after-amendment', keep={'design-review': [('intent-faithful', 'aggregate', 2)]},
          hooks={'design-review': eligibility_cases.carried}, stop_at='design-review')


def marked_run(args, journey):
    drive(args, journey, 'marked-amendment', keep={'intent-review': [('solution-agnostic', 'aggregate', 2)]},
          hooks={'intent-review': eligibility_cases.marked}, stop_at='intent-review')


def main():
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    for flag in ('source-root', 'output-root', 'engine', 'provider'):
        parser.add_argument('--' + flag, required=True, type=Path)
    parser.add_argument('--only', action='append', default=None)
    args = parser.parse_args()
    require(not sys.flags.optimize, 'Fixture assertions must be enabled')
    for key in ('source_root', 'output_root', 'engine', 'provider'):
        value = getattr(args, key)
        require(value.is_absolute(), key)
        setattr(args, key, value.resolve())
    require(not args.output_root.exists(), 'Fresh output directory required')
    require(args.source_root not in args.output_root.parents, 'Test catalog must be outside checkout')
    args.output_root.mkdir()
    sys.path.insert(0, str(args.source_root / 'scripts'))
    journey = load(args.source_root / 'scripts/software-change-journey.py', 'model_binding_journey')
    args.mode, args.data_root = 'source', str(args.source_root)
    args.trail = []
    cases = [('profile-identity', lambda a, j: profile_cases.run(a)),
             ('unbound-amendments', unbound_run),
             ('author-implementer-declarations', declaration_run),
             ('cold-reader-return', cold_reader_run),
             ('review-and-verdict-returns', review_kinds_run),
             ('unmarked-amendment', unmarked_run),
             ('carried-after-amendment', carried_run),
             ('marked-amendment', marked_run)]
    completed, failed = [], {}
    for name, case in cases:
        if args.only and name not in args.only:
            continue
        print('START ' + name, flush=True)
        try:
            case(args, journey)
        except Exception:  # every case is reported; one failing case does not hide the others
            failed[name] = traceback.format_exc()
            print('FAIL ' + name + '\n' + failed[name], file=sys.stderr, flush=True)
            continue
        completed.append(name)
        print('PASS ' + name, flush=True)
    (args.output_root / 'assertions.txt').write_text('\n'.join(args.trail) + '\n')
    print('EXECUTED ASSERTIONS:\n' + '\n'.join(args.trail), flush=True)
    print(json.dumps({'completed': completed, 'failed': sorted(failed), 'assertions': len(args.trail),
                      'semantic_review': False}))
    if failed:
        sys.exit(1)


if __name__ == '__main__':
    main()
