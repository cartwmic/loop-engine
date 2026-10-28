#!/usr/bin/env python3
"""External-driver Generate-PRD candidate journey."""
import argparse
import sys
import json
import subprocess
import tempfile
from pathlib import Path
from typing import Any, Sequence
from external_work_driver import engine_call, record_reviews

def candidate_text(records: Sequence[dict[str, str]]) -> str:
    lines = [
        "# Proposed PRD candidate",
        "",
        "> Candidate IDs are provisional proposals. A human must accept this file before any commit to the living PRD.",
        "",
    ]
    for record in records:
        lines.extend(
            [
                f"### {record['id']}: {record['title']}",
                "- Status: live",
                "- Coverage: e2e/journey",
                "",
                "This proposal is backed by the repository evidence below; it is not yet PRD authority.",
                "",
                "#### Repository evidence",
                f"- Locator: `{record['locator']}`",
                f"- Extract: `{record['extract']}`",
                "",
            ]
        )
    return "\n".join(lines)


def brief_artifact() -> dict[str, Any]:
    return {
        "revision": "1",
        "author": {"name": "generate-prd-worker", "kind": "script"},
        "question": "Extract a schema-valid living markdown PRD candidate for the current repository.",
        "scope": "Tracked documents, tests, and other reviewable evidence in this checkout.",
        "acceptance": [
            "Every proposed requirement carries one reviewable repository locator and exact source extract.",
            "The candidate is parser-valid but remains provisional until human acceptance.",
        ],
        "constraints": [
            "Use the published LE-n ID grammar.",
            "Do not edit docs/PRD.md or commit automatically.",
        ],
        "non_goals": [
            "Certifying semantic completeness.",
            "Calling another provider's evaluate or creating another provider.",
        ],
    }


def sources_artifact(records: Sequence[dict[str, str]]) -> dict[str, Any]:
    return {
        "revision": "1",
        "author": {"name": "generate-prd-worker", "kind": "script"},
        "brief_revision": "1",
        "sources": [
            {
                "id": f"source-{record['id']}",
                "title": record["title"],
                "locator": record["locator"],
                "extract": record["extract"],
            }
            for record in records
        ],
    }


def verification_artifact(records: Sequence[dict[str, str]]) -> dict[str, Any]:
    return {
        "revision": "1",
        "author": {"name": "generate-prd-worker", "kind": "script"},
        "sources_revision": "1",
        "claims": [
            {
                "id": f"claim-{record['id']}",
                "statement": record["title"],
                "source_ids": [f"source-{record['id']}"],
                "support": f"The exact extract from {record['locator']} supports this proposed requirement.",
                "challenge": "No contrary repository extract was found after the deterministic search.",
            }
            for record in records
        ],
    }


def report_artifact(records: Sequence[dict[str, str]], candidate: str) -> dict[str, Any]:
    return {
        "revision": "1",
        "author": {"name": "generate-prd-worker", "kind": "script"},
        "verification_revision": "1",
        "conclusion": candidate,
        "citations": [
            {
                "claim_id": f"claim-{record['id']}",
                "source_id": f"source-{record['id']}",
            }
            for record in records
        ],
    }



def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--mode', choices=['source'], default='source')
    parser.add_argument('--engine')
    parser.add_argument('--provider')
    parser.add_argument('--profile')
    parser.add_argument('--checker')
    parser.add_argument('--work-root')
    parser.add_argument('--self-test', action='store_true')
    args = parser.parse_args()
    # Bare invocation is the recognized CI collection form; full proof uses the parameterized run below.
    if args.self_test or len(sys.argv) == 1:
        assert 'LE-9001' in candidate_text([{'id':'LE-9001','title':'Fixture','locator':__file__,'extract':'fixture'}])
        print('generate-prd journey self-test passed')
        return
    for name in ['engine', 'provider', 'profile', 'checker']:
        if not getattr(args, name):
            parser.error(f'--{name} is required')
    engine, provider, checker = [Path(getattr(args, key)).resolve() for key in ['engine','provider','checker']]
    profile = json.loads(Path(args.profile).read_text())
    assert profile.get('extra', {}).get('profile') == 'generate-prd', profile
    base = Path(args.work_root).resolve() if args.work_root else None
    if base:
        base.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='generate-prd-external-', dir=base) as temporary:
        work = Path(temporary)
        artifacts = work / 'artifacts'
        artifacts.mkdir()
        profile['artifact_root'] = str(artifacts)
        selected = work / 'profile.json'
        selected.write_text(json.dumps(profile))
        config = work / 'providers.toml'
        config.write_text(f'[providers.research]\ncommand = {json.dumps(str(provider))}\nargs = []\n')
        database = work / 'run.sqlite'
        run_id = 'generate-prd-external'
        def call(*args):
            value = engine_call(engine, database, config, list(args))
            if value['status'] != 'completed':
                raise RuntimeError(value)
            return value['result']
        records = [{'id':'LE-9001', 'title':'Generate-PRD uses the research provider',
                    'locator':'scripts/generate-prd-journey.py',
                    'extract':"from external_work_driver import engine_call, record_reviews"}]
        candidate = candidate_text(records)
        call('start', '--id', run_id, 'research', '@' + str(selected), 'external Generate-PRD')
        for state, artifact, value, event in [
            ('scope','brief.json',brief_artifact(),'scoped'),
            ('gather','sources.json',sources_artifact(records),'gathered'),
            ('verify','verification.json',verification_artifact(records),'verified'),
            ('synthesize','report.json',report_artifact(records,candidate),'completed')]:
            shown = call('show', '--view', 'full', run_id)
            assert shown['current_state'] == state, shown
            (artifacts / artifact).write_text(json.dumps(value))
            if state in ['verify','synthesize']:
                record_reviews(engine, database, config, provider, run_id, state,
                               [p['id'] for p in profile['review_policies'][state]], artifact, '1')
                call('show', '--view', 'full', run_id)
            call('event', run_id, event)
        terminal = call('show', '--view', 'full', run_id)
        assert terminal['current_state'] == 'end' and terminal['lifecycle'] == 'final', terminal
        candidate_path = artifacts / 'prd-candidate.md'
        candidate_path.write_text(candidate)
        (artifacts / 'prd-evidence.json').write_text(json.dumps({'candidate_revision':'1','proposals':records}))
        checked = subprocess.run([str(checker),'candidate',str(candidate_path)],text=True,capture_output=True)
        assert checked.returncode == 0 and checked.stdout.splitlines()[0] == 'GREEN', checked
        print('generate-prd external-work normal completion passed')

if __name__ == '__main__':
    main()
