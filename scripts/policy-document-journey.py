#!/usr/bin/env python3
"""External-driver policy-document journey; synthetic reviews prove mechanics only."""
import argparse
import sys
import json
import tempfile
import subprocess
from pathlib import Path
from external_work_driver import engine_call, record_reviews, declare_target_authorship


def document(agents=False):
    if agents:
        return ('# Agent instructions\n\n## Scope\nWork in this fixture only.\n\n'
                '## Workflow\n```sh\ncargo test\n```\n\n'
                '## Completion\nRetain command results and report remaining blockers.\n')
    return ('# Product\n\n## Purpose\nUseful tool.\n\n## Installation\n'
            '```sh\ncargo build\n```\n\n## Usage\nRun it.\n\n## Validation\n'
            '```sh\ncargo test\n```\n')


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--engine')
    parser.add_argument('--provider')
    parser.add_argument('--profile')
    parser.add_argument('--mode', choices=['draft', 'audit'], default='draft')
    parser.add_argument('--self-test', action='store_true')
    args = parser.parse_args()
    # Bare invocation is the recognized CI collection form; full proof uses the parameterized run below.
    if args.self_test or len(sys.argv) == 1:
        assert '## Validation' in document()
        assert '## Completion' in document(True)
        print('policy-document journey self-test passed')
        return
    for name in ['engine', 'provider', 'profile']:
        if not getattr(args, name):
            parser.error(f'--{name} is required')
    engine, provider = Path(args.engine).resolve(), Path(args.provider).resolve()
    profile = json.loads(Path(args.profile).read_text())
    with tempfile.TemporaryDirectory(prefix='policy-external-') as temporary:
        work = Path(temporary)
        artifacts = work / 'artifacts'
        artifacts.mkdir()
        agents = profile['target']['id'] == 'AGENTS.md'
        target = work / ('AGENTS.md' if agents else 'README.md')
        target.write_text('')
        profile['mode'] = args.mode
        profile['target']['path'] = str(target)
        profile['artifact_root'] = str(artifacts)
        selected = work / 'profile.json'
        selected.write_text(json.dumps(profile))
        config = work / 'providers.toml'
        config.write_text(f'[providers.policy-document]\ncommand = {json.dumps(str(provider))}\nargs = []\n')
        database = work / 'run.sqlite'
        run_id = 'policy-external'
        def call(*args):
            value = engine_call(engine, database, config, list(args))
            if value['status'] != 'completed':
                raise RuntimeError(value)
            return value['result']
        # bookends:LE-55 bookends:LE-63 — draft/audit and README/AGENTS policy sets use the same unchanged engine boundary.
        call('start', '--id', run_id, 'policy-document', '@' + str(selected), 'external policy')
        call('show', '--view', 'full', run_id)
        # bookends:LE-86 — prepare has no checked work slot; external drafting precedes deterministic approval.
        call('event', run_id, 'ready')
        def denial():
            call('show', '--view', 'full', run_id)
            result = subprocess.run([str(engine),'--database',str(database),'--config',str(config),'--json','event',run_id,'passed'],text=True,capture_output=True)
            assert result.returncode != 0, result.stdout
            return json.loads(result.stdout)
        # bookends:LE-56 — absent required headings return actual deterministic violations.
        refused = denial()
        assert 'nonconforming' in json.dumps(refused) and 'violations' in json.dumps(refused), refused
        target.write_text(document(agents))
        call('show', '--view', 'full', run_id)
        call('event', run_id, 'passed')
        # bookends:LE-57 — external semantic evidence is required independently of deterministic conformance.
        missing = denial()
        assert 'review-incomplete' in json.dumps(missing), missing
        # bookends:LE-61 bookends:LE-62 — finalization rechecks externally changed bytes; engine state does not lock the document.
        conforming = target.read_text()
        target.write_text('# Broken after deterministic approval\n')
        changed = denial()
        assert 'nonconforming' in json.dumps(changed), changed
        target.write_text(conforming)
        # bookends:LE-58 bookends:LE-59 bookends:LE-60 — fresh reviewers can inspect retained prior findings through a repeated revision cycle.
        previous = call('show', '--view', 'full', run_id)
        assert len(previous['evaluation_history']) >= 4, previous
        call('event', run_id, 'revise')
        call('show', '--view', 'full', run_id)
        call('event', run_id, 'ready')
        call('show', '--view', 'full', run_id)
        call('event', run_id, 'passed')
        call('show', '--view', 'full', run_id)
        digest = declare_target_authorship(engine, database, config, run_id, target,
                                           {'name': 'document-drafter', 'kind': 'script'},
                                           'target-authorship-current')
        record_reviews(engine, database, config, provider, run_id, 'semantic-review',
                       [p['id'] for p in profile['semantic_policies']],
                       digest, digest)
        call('show', '--view', 'full', run_id)
        call('event', run_id, 'passed')
        terminal = call('show', '--view', 'full', run_id)
        assert terminal['current_state'] == 'end' and terminal['lifecycle'] == 'final', terminal
        print(f'policy-document {args.mode} external-work normal completion passed')

if __name__ == '__main__':
    main()
