"""Temporary Git delivery fixture; never mutates the source repository."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys

from test_contract import ROOT


def delivery_case(args):
    root = args.attempt
    repo = root / 'repository'
    repo.mkdir()
    receipts = []
    outcomes = []

    def call(argv, expected=0, cwd=repo):
        process = subprocess.run(argv, cwd=cwd, capture_output=True, timeout=30)
        stem = root / f'command-{len(receipts):03}'
        stdout, stderr = Path(str(stem) + '.stdout'), Path(str(stem) + '.stderr')
        stdout.write_bytes(process.stdout)
        stderr.write_bytes(process.stderr)
        receipts.append({'argv': argv, 'cwd': str(cwd), 'exit_code': process.returncode,
                         'stdout': str(stdout), 'stderr': str(stderr)})
        (root / 'commands.json').write_text(json.dumps(receipts, indent=2))
        assert process.returncode == expected, (argv, process.returncode, process.stderr)
        return process.stdout.decode().strip()

    def git(*argv):
        return call(['git', *argv])

    def sha(data):
        return 'sha256:' + hashlib.sha256(data).hexdigest()

    def dump(path, value):
        path.write_text(json.dumps(value, indent=2) + '\n')

    git('init')
    git('config', 'user.email', 'fixture@example.invalid')
    git('config', 'user.name', 'Delivery fixture')
    (repo / 'source').write_text('before\n')
    (repo / 'unchanged').write_text('complete inventory\n')
    (repo / 'removed').write_text('delete me\n')
    (repo / 'link').symlink_to('source')
    (repo / 'executable').write_text('executable\n')
    (repo / 'executable').chmod(0o755)
    git('add', '.')
    git('commit', '-m', 'base')
    head = git('rev-parse', 'HEAD')
    (repo / 'source').write_text('reviewed\n')
    (repo / 'added').write_text('new\n')
    (repo / 'removed').unlink()
    # Reproduce the native provider's complete retained checkpoint entry shape.
    # Hash entries reconstruct content identity without needing the old worktree.
    entries = []
    for path in sorted(repo.iterdir()):
        if path.name == '.git':
            continue
        link = path.is_symlink()
        entries.append({'path': path.name, 'tracked': path.name != 'added',
                        'kind': 'symlink' if link else 'regular',
                        'mode': '120000' if link else '100755' if path.stat().st_mode & 0o111 else '100644',
                        'content_sha256': sha(str(path.readlink()).encode() if link else path.read_bytes())})
    entries.append({'path': 'removed', 'tracked': True, 'kind': 'missing', 'mode': '100644', 'content_sha256': None})
    entries.sort(key=lambda e: e['path'])
    report = root / 'implementation-report.json'
    checkpoint = root / 'implementation-checkpoint.json'
    reference = root / 'run-reference.json'
    output = root / 'delivery.json'
    dump(report, {'revision': 'fixture-r1', 'repository_state': head + '+uncommitted-worktree'})
    state = {'head': head, 'index_sha256': sha(git('ls-files', '--stage', '-z').encode()),
             'status_sha256': sha(git('status', '--porcelain=v2', '-z', '--untracked-files=all', '--ignored=no').encode()), 'entries': entries}
    state['state_sha256'] = sha(json.dumps(state, ensure_ascii=False, separators=(',', ':')).encode())
    retained = {'schema_version': '1', 'phase': 'implementation',
                'report': {'file': report.name, 'revision': 'fixture-r1', 'sha256': sha(report.read_bytes())},
                'documents': {'intent_revision': '1', 'design_revision': '1', 'plan_revision': '1'},
                'repository': state}
    dump(checkpoint, retained)
    dump(reference, {'run_id': 'synthetic-delivery', 'evidence_kind': 'fixture-not-production'})
    base = [sys.executable, str(ROOT / 'scripts/delivery-pointer.py'), '--report', str(report),
            '--checkpoint', str(checkpoint), '--run-reference', str(reference),
            '--repository', str(repo), '--output', str(output)]
    immutable = {p: p.read_bytes() for p in (report, checkpoint, reference)}
    pending = json.loads(call(base))
    assert pending['git']['status'] == pending['hosted']['status'] == 'pending'
    first = output.read_bytes()
    git('add', '-A')
    git('commit', '-m', 'land reviewed bytes with different metadata')
    landed = git('rev-parse', 'HEAD')
    assert landed != head
    matched = json.loads(call(base + ['--commit', landed]))
    assert matched['git']['status'] == 'matched' and matched['hosted']['status'] == 'pending'
    assert output.read_bytes() == first
    second = Path(matched['output'])
    assert json.loads(second.read_text())['previous']['sha256'] == sha(first)
    outcomes += ['pending without commit', 'equivalent commit despite changed HEAD/index/status', 'immutable linked generations', 'hosted remains pending']
    # Metadata-only additional commit must also match.
    git('commit', '--allow-empty', '-m', 'metadata only')
    call(base + ['--commit', git('rev-parse', 'HEAD')])
    for scenario in ('changed', 'extra', 'missing', 'mode', 'symlink'):
        git('reset', '--hard', landed)
        if scenario == 'changed':
            (repo / 'unchanged').write_text('not reviewed\n')
        elif scenario == 'extra':
            (repo / 'extra').write_text('not reviewed\n')
        elif scenario == 'missing':
            (repo / 'unchanged').unlink()
        elif scenario == 'mode':
            (repo / 'executable').chmod(0o644)
        else:
            (repo / 'link').unlink()
            (repo / 'link').symlink_to('added')
        git('add', '-A')
        git('commit', '-m', scenario)
        call(base + ['--commit', git('rev-parse', 'HEAD')], expected=1)
        outcomes.append(scenario + ' refused')
    bad = root / 'absent-reconstruction.json'
    dump(bad, {**retained, 'repository': {'head': head}})
    bad_args = base.copy()
    bad_args[bad_args.index('--checkpoint') + 1] = str(bad)
    call(bad_args + ['--commit', landed], expected=1)
    outcomes.append('absent reconstruction refused')
    bad_report = root / 'wrong-report.json'
    dump(bad_report, {'revision': 'fixture-r1', 'changed': True})
    bad_args = base.copy()
    bad_args[bad_args.index('--report') + 1] = str(bad_report)
    call(bad_args + ['--commit', landed], expected=1)
    missing_reference = base.copy()
    missing_reference[missing_reference.index('--run-reference') + 1] = str(root / 'not-present.json')
    call(missing_reference + ['--commit', landed], expected=1)
    outcomes.append('wrong report and absent run evidence refused')
    hosted = root / 'hosted.json'
    dump(hosted, {'status': 'pending'})
    assert json.loads(call(base + ['--commit', landed, '--hosted-evidence', str(hosted)]))['hosted']['status'] == 'pending'
    dump(hosted, {'status': 'success', 'commit': landed, 'url': 'https://example.invalid/fixture/1'})
    assert json.loads(call(base + ['--commit', landed, '--hosted-evidence', str(hosted)]))['hosted']['status'] == 'success'
    call(base + ['--hosted-evidence', str(hosted)], expected=1)
    dump(hosted, {'status': 'success', 'commit': head, 'url': 'https://example.invalid/fixture/2'})
    call(base + ['--commit', landed, '--hosted-evidence', str(hosted)], expected=1)
    outcomes.append('hosted observed only with exact matched commit; no-commit/mismatch refused')
    assert all(p.read_bytes() == data for p, data in immutable.items())
    (root / 'scenarios.json').write_text(json.dumps(outcomes, indent=2) + '\n')
