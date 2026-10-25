"""S2: profile identity, bound start, frozen input.

Uses the real `software-change setup`, then `loop-engine start`/`show` on a private
catalog; no journey is needed because nothing is reviewed here.
"""
import json
from pathlib import Path

from harness import NEVER_BOUND, binding, require
from capture_support import Capture, sha

REVIEWER = binding('provider/reviewer-model', 'high')
DRIVER = binding('provider/driver-model', 'high')


def setup(log, args, output, *settings, expect_ok=True):
    argv = [args.provider, 'setup', '--rigor', 'minimal', '--decline-advice', '--set', 'principles.enabled=false', '--output', output]
    for key, value in settings:
        argv += ['--set', f'{key}={value}']
    result = log.execute(argv, cwd=args.source_root, raw=True)
    if expect_ok:
        require(result.returncode == 0, result)
    return result


def run(args):
    root = args.output_root / 'profile-identity'
    root.mkdir()
    log = Capture(root / 'commands', args)
    work = root / 'work'
    work.mkdir()
    artifacts = work / 'artifacts'
    artifacts.mkdir()
    config = work / 'providers.toml'
    config.write_text(f'[providers.software-change]\ncommand = {json.dumps(str(args.provider))}\nargs = []\n')
    database = work / 'run.sqlite'
    engine = [args.engine, '--database', database, '--config', config, '--json']

    unbound, bound, other = (work / name for name in ('unbound.json', 'bound.json', 'other.json'))
    setup(log, args, unbound)
    setup(log, args, bound, ('binding.ordinary-reviewer', 'provider/reviewer-model@high'))
    setup(log, args, other, ('binding.ordinary-reviewer', 'provider/reviewer-model@xhigh'))
    hashes = [sha(path.read_bytes()) for path in (unbound, bound, other)]
    require(len(set(hashes)) == 3, hashes)
    require('model_bindings' not in json.loads(unbound.read_bytes()), 'setup without a binding adds no key')
    require(json.loads(bound.read_bytes())['model_bindings'] == {'roles': {'ordinary-reviewer': REVIEWER}},
            json.loads(bound.read_bytes()).get('model_bindings'))
    explain = json.loads(Path(str(bound) + '.explain.json').read_text())
    require(any(row['key'] == 'binding.ordinary-reviewer' and row['value'] == REVIEWER
                for row in explain['overrides']), explain['overrides'])
    print('ASSERT [profile-identity] setup with and without --set binding.* gives different file SHA-256 '
          + ', '.join(hashes), flush=True)
    args.trail.append('[profile-identity] setup without a binding, with one, and with a thinking-level variant give '
                      'three different profile file SHA-256 values; the unbound profile has no model_bindings key; '
                      'the override is recorded in explain overrides')

    # Setup refuses a driver binding without its session.
    refused = setup(log, args, work / 'driver-only.json', ('binding.driver', 'provider/driver-model@high'),
                    expect_ok=False)
    require(refused.returncode != 0 and 'driver' in refused.stdout + refused.stderr and
            not (work / 'driver-only.json').exists(), refused)
    args.trail.append('[profile-identity] setup refuses a driver binding without driver-session and writes no profile')

    # Start the bound and the unbound profiles; frozen input is visible; later edits do not reach the run.
    for path, name in ((unbound, 'unbound-run'), (bound, 'bound-run')):
        data = json.loads(path.read_bytes())
        data['artifact_root'] = str(artifacts)
        selected = work / f'{name}.profile.json'
        selected.write_text(json.dumps(data))
        started = log.execute([*engine, 'start', '--id', name, 'software-change', '@' + str(selected), name],
                              cwd=args.source_root)
        require(started['status'] == 'completed', started)
    bound_run = log.execute([*engine, 'show', '--view', 'full', 'bound-run'], cwd=args.source_root)['result']
    require(bound_run['initial_input']['model_bindings'] == {'roles': {'ordinary-reviewer': REVIEWER}}, bound_run)
    view = bound_run['model_bindings']
    require(view['state'] == 'bound' and view['history'] == [] and
            {row['role']: row['in_force'] for row in view['roles']}['ordinary-reviewer'] == REVIEWER, view)
    unbound_run = log.execute([*engine, 'show', '--view', 'full', 'unbound-run'], cwd=args.source_root)['result']
    require('model_bindings' not in unbound_run['initial_input'] and
            unbound_run['model_bindings'] == {'state': 'never-bound', 'statement': NEVER_BOUND}, unbound_run)
    args.trail.append('[profile-identity] bound start works; the frozen model_bindings input and its view are in '
                      'show full; the unbound run states never-bound')

    # A later edit of the profile file leaves the started run exactly as frozen.
    edited = json.loads((work / 'bound-run.profile.json').read_bytes())
    edited['model_bindings']['roles']['ordinary-reviewer'] = binding('provider/edited', 'low')
    (work / 'bound-run.profile.json').write_text(json.dumps(edited))
    again = log.execute([*engine, 'show', '--view', 'full', 'bound-run'], cwd=args.source_root)['result']
    require(again['initial_input'] == bound_run['initial_input'] and again['model_bindings'] == view, again)
    status = log.execute([*engine, 'show', '--view', 'status', 'bound-run'], cwd=args.source_root)['result']
    require(status['model_bindings'] == view, status)
    args.trail.append('[profile-identity] editing the profile file after start changes neither initial_input nor '
                      'the full or status model_bindings view of the started run')

    # Start refuses a driver binding with no matching driver_session, naming the cause.
    broken = json.loads((work / 'bound-run.profile.json').read_bytes())
    broken['model_bindings'] = {'roles': {'driver': binding('provider/driver-model')}}
    (work / 'broken.profile.json').write_text(json.dumps(broken))
    def start_driver(name, session):
        """Start from the bound profile with the driver bound to DRIVER and `session` declared (or not)."""
        data = json.loads(bound.read_bytes())
        data['artifact_root'] = str(artifacts)
        data['model_bindings'] = {'roles': {'driver': DRIVER}}
        if session is not None:
            data['model_bindings']['driver_session'] = session
        path = work / f'{name}.profile.json'
        path.write_text(json.dumps(data))
        return data, log.execute([*engine, 'start', '--id', name, 'software-change', '@' + str(path), name],
                                 cwd=args.source_root, raw=True)

    def no_such_run(name):
        shown = log.execute([*engine, 'show', '--view', 'status', name], cwd=args.source_root, raw=True)
        result = json.loads(shown.stdout)['result'] if shown.returncode == 0 else None
        require(shown.returncode != 0 or (result['uncertainty']['state'] == 'unavailable' and
                                          'was not found' in result['uncertainty']['reason']), shown)

    expected = 'expected model `provider/driver-model` at thinking `high`'
    refusals = (
        ('driver-missing', None, ['driver_session', 'role `driver`', expected,
                                  'actual: no session declared (statement missing)']),
        ('driver-model', binding('provider/other-model', 'high'),
         ['role `driver`', expected, 'actual model `provider/other-model` at thinking `high`',
          'differs in model only']),
        ('driver-thinking', binding('provider/driver-model', 'low'),
         ['role `driver`', expected, 'actual model `provider/driver-model` at thinking `low`',
          'differs in thinking level only']),
        ('driver-both', binding('provider/other-model', 'low'),
         ['role `driver`', expected, 'actual model `provider/other-model` at thinking `low`',
          'differs in model and thinking level']),
    )
    for name, session, needles in refusals:
        _, result = start_driver(name, session)
        text = result.stdout + result.stderr
        require(result.returncode != 0, result)
        for needle in needles:
            require(needle in text, (name, needle, text))
        no_such_run(name)
    args.trail.append('[profile-identity] start refuses a driver binding with a missing, model-only, thinking-only '
                      'and model-and-thinking driver_session, naming role, expected, actual and the differing part, '
                      'and creates no run')

    # A matching driver_session starts the run and is stored with it.
    data, started = start_driver('driver-run', DRIVER)
    require(started.returncode == 0, started)
    run = log.execute([*engine, 'show', '--view', 'full', 'driver-run'], cwd=args.source_root)['result']
    require(run['initial_input']['model_bindings'] == data['model_bindings'] ==
            {'roles': {'driver': DRIVER}, 'driver_session': DRIVER}, run['initial_input'])
    require({row['role']: row['in_force'] for row in run['model_bindings']['roles']}['driver'] == DRIVER and
            run['model_bindings']['state'] == 'bound', run['model_bindings'])
    args.trail.append('[profile-identity] start with a driver binding and a matching driver_session succeeds; '
                      'both are frozen in initial_input and the driver is in force in the view')
    log.finalize()
