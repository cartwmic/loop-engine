#!/usr/bin/env python3
"""Record delivery facts separately from immutable reviewed evidence.

--output is an exclusive new record path. If it exists, append a numbered
sibling generation, linking its predecessor; never replace an earlier record.
Run-reference is an opaque nonempty JSON object. Hosted evidence is an object
with status (pending/success/failure), commit and URL for observed outcomes.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys


def digest(data):
    return 'sha256:' + hashlib.sha256(data).hexdigest()


def encoded(value):
    return json.dumps(value, ensure_ascii=False, separators=(',', ':')).encode()


def evidence(path):
    data = path.read_bytes()
    value = json.loads(data)
    if not isinstance(value, dict) or not value:
        raise ValueError(f'{path}: requires nonempty JSON object')
    return value, {'path': str(path.resolve()), 'sha256': digest(data)}


def inventory(checkpoint):
    if checkpoint.get('schema_version') != '1' or checkpoint.get('phase') != 'implementation':
        raise ValueError('requires implementation checkpoint schema 1')
    repo = checkpoint['repository']
    # This is the provider serialization order, not a driver-supplied inventory.
    state = {key: repo[key] for key in ('head', 'index_sha256', 'status_sha256', 'entries')}
    if digest(encoded(state)) != repo['state_sha256']:
        raise ValueError('checkpoint repository state digest mismatch')
    result = {}
    seen = set()
    for entry in repo['entries']:
        path, kind, mode, sha = (entry[k] for k in ('path', 'kind', 'mode', 'content_sha256'))
        if not path or path.startswith('/') or any(p in ('', '.', '..') for p in path.split('/')) or path in seen:
            raise ValueError('invalid or duplicate checkpoint path')
        seen.add(path)
        if kind == 'missing':
            if sha is not None:
                raise ValueError('missing entry has content')
            continue
        if kind not in ('regular', 'symlink', 'submodule') or not isinstance(sha, str) or not re.fullmatch(r'sha256:[0-9a-f]{64}', sha):
            raise ValueError('missing reconstructable reviewed content')
        expected_modes = {'regular': ('100644', '100755'), 'symlink': ('120000',), 'submodule': ('160000',)}
        if mode not in expected_modes[kind]:
            raise ValueError('checkpoint kind/mode disagreement cannot reconstruct Git tree')
        result[path] = {'mode': mode, 'content_sha256': sha}
    return dict(sorted(result.items()))


def git(repo, *args):
    return subprocess.check_output(['git', *args], cwd=repo, stderr=subprocess.PIPE)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('report', 'checkpoint', 'run-reference', 'repository', 'output'):
        parser.add_argument('--' + name, required=True, type=Path)
    parser.add_argument('--commit')
    parser.add_argument('--hosted-evidence', type=Path)
    args = parser.parse_args()
    try:
        if not args.repository.is_absolute() or not args.repository.is_dir():
            raise ValueError('--repository requires an existing absolute directory')
        report, report_ref = evidence(args.report)
        checkpoint, checkpoint_ref = evidence(args.checkpoint)
        _, run_ref = evidence(args.run_reference)
        identity = checkpoint['report']
        if identity['sha256'] != report_ref['sha256'] or identity['revision'] != report['revision'] or identity['file'] != args.report.name:
            raise ValueError('report/checkpoint identity mismatch')
        reviewed = inventory(checkpoint)
        pointer = {'schema_version': '1', 'report': report_ref, 'checkpoint': checkpoint_ref,
                   'run_reference': run_ref, 'repository': str(args.repository.resolve()),
                   'reviewed_content_sha256': digest(encoded(reviewed)),
                   'git': {'status': 'pending'}, 'hosted': {'status': 'pending'},
                   'semantic_approval': False}
        if args.commit:
            if not re.fullmatch(r'[0-9a-f]{40}|[0-9a-f]{64}', args.commit):
                raise ValueError('--commit requires a full commit SHA')
            commit = git(args.repository, 'rev-parse', '--verify', args.commit + '^{commit}').decode().strip()
            actual = {}
            for row in git(args.repository, 'ls-tree', '-rz', '--full-tree', commit).split(b'\0'):
                if not row:
                    continue
                header, name = row.split(b'\t', 1)
                mode, kind, oid = header.decode().split()
                data = (oid + '\n').encode() if kind == 'commit' else git(args.repository, 'cat-file', 'blob', oid)
                actual[name.decode()] = {'mode': mode, 'content_sha256': digest(data)}
            if actual != reviewed:
                changed = sorted(p for p in actual.keys() | reviewed.keys() if actual.get(p) != reviewed.get(p))
                raise ValueError('complete commit content differs: ' + ', '.join(changed))
            pointer['git'] = {'status': 'matched', 'commit': commit, 'tree': git(args.repository, 'rev-parse', commit + '^{tree}').decode().strip()}
        if args.hosted_evidence:
            hosted, ref = evidence(args.hosted_evidence)
            if hosted.get('status') not in ('pending', 'success', 'failure'):
                raise ValueError('hosted status must be pending, success or failure')
            if hosted['status'] != 'pending' and (not args.commit or hosted.get('commit') != args.commit or not hosted.get('url')):
                raise ValueError('observed hosted evidence requires matching commit and URL')
            pointer['hosted'] = {**hosted, 'evidence': ref}
        output = args.output
        previous = None
        generation = 1
        while output.exists():
            previous = output
            generation += 1
            output = args.output.with_name(args.output.name + f'.{generation}')
        if previous:
            old, ref = evidence(previous)
            if any(old.get(k) != pointer[k] for k in ('report', 'checkpoint', 'run_reference', 'repository', 'reviewed_content_sha256')):
                raise ValueError('pointer generation changes reviewed identity')
            pointer['previous'] = ref
        pointer['generation'] = generation
        with output.open('x') as stream:
            stream.write(json.dumps(pointer, indent=2) + '\n')
        print(json.dumps({'status': 'recorded', 'output': str(output.resolve()), 'git': pointer['git'], 'hosted': pointer['hosted']}))
        return 0
    except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as error:
        print(json.dumps({'status': 'error', 'message': str(error)}), file=sys.stderr)
        return 1


if __name__ == '__main__':
    sys.exit(main())
