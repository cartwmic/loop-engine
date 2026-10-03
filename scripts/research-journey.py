#!/usr/bin/env python3
"""External-driver research journey through normal completion."""
import argparse
import json
import tempfile
import subprocess
from pathlib import Path
from external_work_driver import engine_call, record_reviews

def author(name: str) -> dict[str, str]:
    return {"name": name, "kind": "human"}


def brief() -> dict[str, Any]:
    return {
        "revision": "1",
        "author": author("owner"),
        "question": "What is the capital of France?",
        "scope": "One inspectable geography fact.",
        "acceptance": ["Name the capital city."],
        "constraints": ["Cite one source extract."],
        "non_goals": ["Travel advice."],
    }


def sources() -> dict[str, Any]:
    return {
        "revision": "1",
        "author": author("owner"),
        "brief_revision": "1",
        "sources": [
            {
                "id": "src-1",
                "title": "Atlas",
                "locator": "https://example.invalid/paris",
                "extract": "Paris is the capital of France.",
            }
        ],
    }


def verification() -> dict[str, Any]:
    return {
        "revision": "1",
        "author": author("owner"),
        "sources_revision": "1",
        "claims": [
            {
                "id": "claim-1",
                "statement": "Paris is the capital of France.",
                "source_ids": ["src-1"],
                "support": "The atlas extract names Paris as the capital.",
                "challenge": "No contrary extract was found after search.",
            }
        ],
    }


def report() -> dict[str, Any]:
    return {
        "revision": "1",
        "author": author("owner"),
        "verification_revision": "1",
        "conclusion": "Paris is the capital of France.",
        "citations": [{"claim_id": "claim-1", "source_id": "src-1"}],
    }



def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--mode', choices=['source', 'packaged'], default='source')
    parser.add_argument('--engine')
    parser.add_argument('--provider')
    parser.add_argument('--profile')
    parser.add_argument('--data-root')
    parser.add_argument('--self-test', action='store_true')
    args = parser.parse_args()
    if args.self_test:
        assert verification()['sources_revision'] == sources()['revision']
        assert report()['verification_revision'] == verification()['revision']
        print('research journey self-test passed')
        return
    for name in ['engine', 'provider', 'profile']:
        if not getattr(args, name):
            parser.error(f'--{name} is required')
    engine, provider = Path(args.engine).resolve(), Path(args.provider).resolve()
    profile_path = Path(args.profile).resolve()
    if args.mode == 'packaged':
        if not args.data_root:
            parser.error('packaged mode requires --data-root')
        dumped = Path(args.data_root).resolve()
        if dumped.exists() and any(dumped.iterdir()):
            parser.error('packaged --data-root must be empty')
        subprocess.run([str(provider), 'data-dump', str(dumped)],check=True,capture_output=True)
        profile_path = dumped / 'crates/research-provider/data/configs' / profile_path.name
    with tempfile.TemporaryDirectory(prefix='research-external-') as temporary:
        work = Path(temporary)
        artifacts = work / 'artifacts'
        artifacts.mkdir()
        profile = json.loads(profile_path.read_text())
        profile['artifact_root'] = str(artifacts)
        profile_path = work / 'profile.json'
        profile_path.write_text(json.dumps(profile))
        config = work / 'providers.toml'
        config.write_text(f'[providers.research]\ncommand = {json.dumps(str(provider))}\nargs = []\n')
        database = work / 'run.sqlite'
        run_id = 'research-external'
        def call(*args):
            value = engine_call(engine, database, config, list(args))
            if value['status'] != 'completed':
                raise RuntimeError(value)
            return value['result']
        call('start', '--id', run_id, 'research', '@' + str(profile_path), 'external research')
        for state, artifact, make, event in [
            ('scope', 'brief.json', brief, 'scoped'),
            ('gather', 'sources.json', sources, 'gathered'),
            ('verify', 'verification.json', verification, 'verified'),
            ('synthesize', 'report.json', report, 'completed')]:
            shown = call('show', '--view', 'full', run_id)
            assert shown['current_state'] == state, shown
            (artifacts / artifact).write_text(json.dumps(make()))
            if state in ['verify', 'synthesize']:
                record_reviews(engine, database, config, provider, run_id, state,
                               [p['id'] for p in profile['review_policies'][state]],
                               artifact, '1')
                call('show', '--view', 'full', run_id)
            call('event', run_id, event)
        terminal = call('show', '--view', 'full', run_id)
        assert terminal['current_state'] == 'end' and terminal['lifecycle'] == 'final', terminal
        print('research external-work normal completion passed')

if __name__ == '__main__':
    main()
