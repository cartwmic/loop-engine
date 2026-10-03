#!/usr/bin/env python3
"""External-driver policy-document journey; synthetic reviews prove mechanics only."""
import argparse
import hashlib
import json
import tempfile
from pathlib import Path
from external_work_driver import engine_call, record_reviews


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
    if args.self_test:
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
        agents = profile['profile_version'] == 'agents-3'
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
        call('start', '--id', run_id, 'policy-document', '@' + str(selected), 'external policy')
        call('show', '--view', 'full', run_id)
        call('event', run_id, 'ready')
        call('show', '--view', 'full', run_id)
        target.write_text(document(agents))
        call('event', run_id, 'passed')
        call('show', '--view', 'full', run_id)
        digest = hashlib.sha256(target.read_bytes()).hexdigest()
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
