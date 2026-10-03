#!/usr/bin/env python3
"""Genuine Pi-authored scratch software change, with a durable fresh-driver handoff."""
import argparse
import hashlib
import json
import os
import re
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile

from external_work_driver import process

ROOT = Path(__file__).resolve().parents[1]
MODEL = 'openai-codex/gpt-6.1-sol'
SUBJECTS = {'explore': 'intent.json', 'design': 'design.json', 'plan': 'plan.json',
            'implement': 'implementation-report.json', 'validation': 'validation-report.json'}
EVENTS = {'explore': 'intent-ready', 'design': 'design-ready', 'plan': 'plan-ready',
          'implement': 'implementation-ready', 'reconciliation': 'reconciliation-ready',
          'validation': 'validation-ready'}


def save(path, value):
    path.write_text(json.dumps(value, indent=2))


def json_answer(text):
    text = text.strip()
    if text.startswith('```'):
        text = text.split('\n', 1)[1].rsplit('```', 1)[0].strip()
    return json.loads(text)


def valid_locator(root, locator):
    if '#' not in locator:
        return False
    path, selector = locator.split('#', 1)
    source = root / path
    if Path(path).is_absolute() or '..' in Path(path).parts or not source.is_file():
        return False
    lines = re.fullmatch(r'L(\d+)(?:-L?(\d+))?', selector)
    if lines:
        start, end = int(lines[1]), int(lines[2] or lines[1])
        return 1 <= start <= end <= len(source.read_text().splitlines())
    if selector == '' or selector.startswith('/'):
        try:
            value = json.loads(source.read_text())
            for key in selector.split('/')[1:]:
                key = key.replace('~1', '/').replace('~0', '~')
                value = value[int(key)] if isinstance(value, list) else value[key]
            return True
        except (ValueError, KeyError, IndexError, TypeError):
            return False
    return False


class Driver:
    def __init__(self, root):
        self.root = Path(root).resolve()
        self.meta = json.loads((self.root / 'driver.json').read_text())
        self.repo = Path(self.meta['repository'])
        self.artifacts = Path(self.meta['artifacts'])
        self.calls = self.root / 'calls'
        self.calls.mkdir(exist_ok=True)

    def engine(self, *args):
        argv = [self.meta['engine'], '--database', self.meta['catalog'], '--config', self.meta['config'], '--json', *args]
        with (self.root / 'engine-commands.jsonl').open('a') as log:
            log.write(json.dumps({'argv': argv, 'pid': os.getpid()}) + '\n')
        return process(argv, cwd=self.repo)

    def show(self):
        return self.engine('show', '--view', 'full', self.meta['run_id'])

    def provider(self, args, packet=None):
        return process([self.meta['software'], *args], packet, cwd=self.repo)

    def append(self, kind, data, record_id=None):
        self.show()
        args = ['append'] + (['--record-id', record_id] if record_id else [])
        return self.engine(*args, self.meta['run_id'], kind, json.dumps(data))

    def event(self, name):
        self.show()
        result = self.engine('event', self.meta['run_id'], name)
        if result['status'] != 'completed':
            raise RuntimeError(f'Checked transition refused: {result}')

    def model(self, purpose, prompt, *, implementation=False):
        index = len(list(self.calls.glob('*.request.txt')))
        stem = self.calls / f'{index:03}-{purpose}'
        request = Path(str(stem) + '.request.txt')
        stdout = Path(str(stem) + '.jsonl')
        stderr = Path(str(stem) + '.stderr')
        request.write_text(prompt)
        argv = ['pi', '--no-extensions', '--no-skills', '--no-context-files', '--no-session',
                '--model', MODEL, '--thinking', 'low' if implementation else 'high', '--mode', 'json',
                '--print', '--tools', 'read,grep,find,ls,edit,write' if implementation else 'read,grep,find,ls',
                '@' + str(request)]
        with stdout.open('w') as out, stderr.open('w') as err:
            result = subprocess.run(argv, cwd=self.repo, stdout=out, stderr=err)
        messages = []
        for line in stdout.read_text().splitlines():
            event = json.loads(line)
            if event.get('type') == 'message_end' and event.get('message', {}).get('role') == 'assistant':
                messages.append(event['message'])
        final = next((m for m in reversed(messages) if m.get('stopReason') == 'stop'), None)
        entry = {'id': stem.name, 'purpose': purpose, 'argv': argv, 'model': MODEL,
                 'exit_code': result.returncode, 'stdout': str(stdout), 'stderr': str(stderr),
                 'request': str(request), 'cwd': str(self.repo), 'pid': os.getpid()}
        with (self.root / 'model-calls.jsonl').open('a') as log:
            log.write(json.dumps(entry) + '\n')
        if result.returncode or final is None or final.get('model') != 'gpt-6.1-sol':
            raise RuntimeError(f'Model failed: {entry}')
        answer = ''.join(c['text'] for c in final['content'] if c['type'] == 'text')
        parsed = json_answer(answer)
        save(Path(str(stem) + '.answer.json'), parsed)
        return entry, parsed

    def author(self, slot):
        commissioned = self.provider(['commission', '--slot', slot], self.show())
        target = {'intent-draft':'intent.json', 'design-draft':'design.json', 'plan-draft':'plan.json',
                  'implement':'implementation-report.json', 'reconciliation-draft':'reconciliation.json'}[slot]
        schema = (json.loads((self.root / 'data/crates/software-change-provider/data/reconciliation-schema.json').read_text())
                  if target == 'reconciliation.json' else self.show()['result']['initial_input']['artifact_schemas'][target])
        commissioned.pop('context', None)
        prompt = ('You are the external author for a tiny fictional software change. Inspect calculator.py and test_calculator.py. '
                  'Fix add(a,b), which currently subtracts. Acceptance: add integers correctly, including negative operands. '
                  'No docs, release or hosted proof needed. Acceptance includes delivered runnable regression coverage, not just passing a zero-test suite. '
                  'Only for plan authorship: exactly three tasks, function fix, positive test, negative test. This is driver grouping, NOT an intent constraint. '
                  'The proof command is python3 -B -m unittest -v test_calculator. Use absolute artifact paths below. '
                  'Read existing artifacts and retained reviews in ' + str(self.calls) + '. Correct genuine findings from previous attempts explicitly, without hiding any issue. '
                  'Write no files. Return ONLY the artifact JSON required by this commission, with a fresh revision for any correction. '
                  'For implement, return implementation-report.json recording ACTUAL implemented tasks, not a proposed implementation. Inspect task-proof.json for current actual task checks and their ordering. '
                  'Each criterion_id appears at most once in each local array, notably validation; consolidate multiple proof observations into one entry per criterion. '
                  'Omit optional source_coverage unless you use a declared scope ID. Source identity: ' + json.dumps(self.provider(['source-identity', '--working-directory', str(self.repo)])) + '. '
                  'For reconciliation-draft return reconciliation.json. Inspect early-proof.json for actual driver command results. '
                  'Reconciliation compares enduring documents with behavior, NOT final validation approval; required proof will also be recorded at validation. '
                  'Bookends is disabled: traceability must be {status: not-applicable, references: []}, with no PRD or Bookends citations. '
                  'Do not copy fixture judgments or assert checks have run before they do. Return the ' + target + ' artifact, conforming EXACTLY to this schema: ' + json.dumps(schema) + '\nCommission:\n' + json.dumps(commissioned))
        entry, artifact = self.model(slot, prompt)
        save(self.artifacts / target, artifact)
        self.append('driver-note', {'model_call': entry['id'], 'artifact': str(self.artifacts / target)})

    def reviews(self, state):
        snapshot = self.show()
        save(self.artifacts / 'review-context.json', snapshot)
        policies = snapshot['result']['initial_input']['review_policies'][state]
        subject = next(name for prefix, name in [('intent','intent.json'), ('design','design.json'),
                       ('plan','plan.json'), ('implementation','implementation-report.json'), ('validation','validation-report.json')]
                       if state.startswith(prefix))
        revision = json.loads((self.artifacts / subject).read_text())['revision']
        grouped = {}
        for policy in policies:
            grouped.setdefault((policy.get('review_stage', 'aggregate'), policy['group']), []).append(policy)
        for (stage, group), axes in grouped.items():
            author = {'name': f'pi-{state}', 'kind': 'agent'}
            assignment = {'gate': state, 'stage': stage, 'group': group, 'subject': subject,
                          'subject_revision': revision, 'author': author}
            prompt = ('Independently inspect the actual current artifacts and scratch source. Inspect review-context.json, the actual full durable engine show snapshot, to resolve context-record IDs in validation-report.json; they are not file paths. Give an honest judgment for EVERY supplied axis. '
                      'Return ONLY JSON {"assessment_id":"native-review","judgments":[{"axis":"ID","result":"pass or fail",'
                      '"findings":"explanation of failures or empty string","grounds":{"reason":"specific reasoning",'
                      '"evidence":[{"locator":"artifact-path#/field"}]}}]}. Never pass a defect to progress the run. '
                      'Evidence locators MUST refer ONLY to actual files beneath ' + str(self.artifacts) + ' using relative paths with JSON pointers or line selectors. Never cite calls, prompts, or previous reviews as evidence; never use .. or absolute paths. '
                      'Artifacts: ' + str(self.artifacts) + '\nAssignment: ' + json.dumps(assignment) + '\nPolicies: ' + json.dumps(axes))
            for attempt in range(5):
                entry, judgment = self.model('review-' + state + '-' + stage + '-' + group, prompt)
                locators = [e['locator'] for j in judgment.get('judgments', []) for e in j.get('grounds', {}).get('evidence', [])]
                if locators and all(valid_locator(self.artifacts, loc) for loc in locators):
                    break
                prompt += '\nYour previous response had mechanically invalid evidence locators. Reinspect source and return corrected honest JSON; preserve genuine findings. Previous response: ' + json.dumps(judgment)
            else:
                raise RuntimeError('Model did not correct invalid evidence locators')
            packet = {'show': self.show(), 'assignment': assignment, 'author': author,
                      'request': {'text': prompt}, 'original': {'text': json.dumps(judgment)}}
            intake = self.provider(['review-candidates'], packet)
            if not intake.get('records'):
                raise RuntimeError(f'No review records: {intake}')
            for candidate in intake['records']:
                self.append(candidate['kind'], candidate['data'])
            self.append('driver-note', {'review_author': author['name'], 'model_call': entry['id'], 'group': group})
            if any(j['result'] != 'pass' for j in judgment['judgments']):
                # Preserve genuine refusals. Do not synthesize a correction or a clearance ledger.
                self.append('driver-note', {'triage': 'retry author to correct genuine findings', 'review': entry['id'], 'findings': judgment})
                events = [e['event'] for e in self.show()['result']['requestable_events']]
                self.event('revise' if 'revise' in events else 'revise-' + ('implementation' if state.startswith('implementation') else state.split('-')[0]))
                return
        _, ledger = self.model('ledger-' + state,
            'Inspect these genuine reviews and author a finding-ledger. Return only JSON with schema_version "1", '
            'gate, subject, subject_revision, author {name:"pi-ledger",kind:"agent"}, findings []. '
            'Use findings [] only if the actual reviews contain no findings. Reviews in ' + str(self.calls) +
            '\nGate ' + state + '\nSubject ' + subject + '\nRevision ' + revision)
        self.append('finding-ledger', ledger)
        events = [e['event'] for e in self.show()['result']['requestable_events']]
        self.event('approved' if 'approved' in events else 'passed')

    def checkpoint(self, phase):
        return self.provider(['checkpoint', '--phase', phase, '--artifact-root', str(self.artifacts),
                              '--working-directory', str(self.repo)])

    def validation(self):
        commission = self.provider(['commission', '--slot', 'validation-draft'], self.show())
        results = []
        for spec in commission['commission']['proof_commands']:
            identity = self.provider(['source-identity', '--working-directory', str(self.repo)])
            outcome = subprocess.run([spec['command'], *spec['args']], cwd=self.repo, text=True, capture_output=True)
            results.append({'id': spec['id'], 'command': spec['command'], 'args': spec['args'], 'cwd': str(self.repo),
                            'exit_code': outcome.returncode, 'stdout': outcome.stdout, 'stderr': outcome.stderr,
                            'producer': {'name':'native-driver','kind':'script'}, 'source': identity})
        prepared = self.provider(['prepare-validation'], {'show': self.show(), 'working_directory': str(self.repo),
            'revision': 'native-validation-' + str(len(list(self.calls.glob('*.request.txt')))), 'author': {'name':'native-driver','kind':'script'}, 'command_results': results})
        save(self.root / 'validation-prepared.json', prepared)
        if not prepared['commands_complete']:
            raise RuntimeError('External proof commands incomplete')
        report = prepared['report_draft']
        save(self.artifacts / 'validation-report.json', report)
        self.checkpoint('validation')
        checkpoint = json.loads((self.artifacts / 'validation-checkpoint.json').read_text())
        current_identity = self.provider(['source-identity', '--working-directory', str(self.repo)])
        correspondence = {'command_identity': results[0]['source'], 'current_identity': current_identity,
                          'checkpoint_repository': checkpoint['repository'],
                          'checkpoint_entry_bytes_match': all(e['content_sha256'] == 'sha256:' + hashlib.sha256((self.repo / e['path']).read_bytes()).hexdigest() for e in checkpoint['repository']['entries'] if e['kind'] == 'regular')}
        save(self.artifacts / 'source-correspondence.json', correspondence)
        assert correspondence['command_identity'] == correspondence['current_identity'] and correspondence['checkpoint_entry_bytes_match']
        for candidate in prepared['command_candidates']:
            self.append(candidate['kind'], candidate['data'], candidate['record_id'])
        for batch in prepared['judgment_batches']:
            entry, verdict = self.model('verdict-' + batch['record_id'],
                'Inspect actual source, artifacts and external proof output. Inspect source-correspondence.json: it mechanically compares current source-identity with command source identity and checks every checkpoint source-entry hash against actual bytes. The checkpoint repository full-state hash and source-identity content-state hash are different representations, not interchangeable. Return ONLY honest verdict JSON for this batch. '
                'Required fields: subject "validation-report.json", subject_revision ' + json.dumps(report['revision']) +
                ', checkpoint "validation-checkpoint.json", author {name:"pi-verdict",kind:"agent"}, result "pass" or "fail", '
                'findings [], reason, evidence_context_ids. Include criterion_id for criterion-verdict. '
                '\nBatch: ' + json.dumps(batch) + '\nPrepared report and actual outputs: ' + json.dumps(prepared))
            self.append(batch['kind'], verdict, batch['record_id'])
            self.append('driver-note', {'review_author': verdict['author']['name'], 'model_call': entry['id']})
            if verdict['result'] != 'pass':
                raise RuntimeError('Genuine verdict requires correction')
        _, ledger = self.model('ledger-validation',
            'Inspect genuine current verdicts and current report. Return ONLY an honest finding-ledger JSON with schema_version \"1\", gate \"validation-review\", subject \"validation-report.json\", subject_revision ' + json.dumps(report['revision']) +
            ', author {name:\"pi-ledger\",kind:\"agent\"}, findings []. Use [] only if current verdicts contain no findings. Current proof and report: ' + json.dumps(prepared))
        self.append('finding-ledger', ledger)
        self.event('validation-ready')

    def drive(self, stop_after_plan=False):
        self.append('driver-note', {'driver_pid': os.getpid(), 'resumed_from': 'catalog and durable driver.json'})
        for _ in range(80):
            shown = self.show()
            state = shown['result']['current_state']
            print('native state:', state, flush=True)
            if state == 'end':
                return
            if state.endswith('review'):
                self.reviews(state)
                if stop_after_plan and state == 'plan-adversarial-review' and self.show()['result']['current_state'] == 'implement':
                    save(self.root / 'continuation.json', {'first_pid': os.getpid(), 'state': self.show()})
                    return
            elif state in ('explore', 'design', 'plan'):
                self.author('intent-draft' if state == 'explore' else state + '-draft')
                self.event(EVENTS[state])
            elif state == 'implement':
                entry, response = self.model('grouped-implementation',
                    'Implement exactly the first TWO tasks from the plan: fix add and add the positive test. '
                    'Leave the negative test to the driver. Use edit/write tools. Do not run commands. '
                    'Return ONLY JSON {"tasks":["actual first task id","actual second task id"],"summary":"actual edits"}. '
                    'Plan: ' + (self.artifacts / 'plan.json').read_text(), implementation=True)
                save(self.root / 'grouped-implementation.json', {'model_call': entry['id'], 'response': response})
                checks = [['python3', '-B', '-c', 'from calculator import add; cases=[(2,3,5),(0,3,3),(3,0,3),(0,0,0),(-2,-3,-5),(-2,3,1),(2,-3,-1),(-3,0,-3),(0,-3,-3)]; assert all(add(a,b)==expected for a,b,expected in cases); print("nine direct checks passed")'],
                          ['python3', '-B', '-m', 'unittest', '-v', 'test_calculator.AddTests.test_add_positive']]
                outcomes = []
                for argv in checks:
                    check = subprocess.run(argv, cwd=self.repo, text=True, capture_output=True)
                    outcomes.append({'argv': argv, 'exit_code': check.returncode, 'stdout': check.stdout, 'stderr': check.stderr,
                                     'ordering': 'after grouped first-two-task implementation and before direct third-task handoff'})
                save(self.artifacts / 'task-proof.json', outcomes)
                self.append('driver-note', {'task_command_results': outcomes})
                if any(outcome['exit_code'] for outcome in outcomes):
                    raise RuntimeError('Genuine task proof failed; author correction required')
                source = self.repo / 'test_calculator.py'
                prefix = source.read_text().split('\nclass NegativeOperands')[0].split('\n    def test_add_negative')[0]
                source.write_text(prefix.rstrip() + '\n\n    def test_add_negative(self):\n'
                    '        cases = [(-2, -3, -5), (-2, 3, 1), (2, -3, -1), (-3, 0, -3), (0, -3, -3)]\n'
                    '        for a, b, expected in cases:\n'
                    '            with self.subTest(a=a, b=b):\n'
                    '                self.assertEqual(add(a, b), expected)\n')
                plan = json.loads((self.artifacts / 'plan.json').read_text())
                save(self.root / 'direct-implementation.json', {'pid': os.getpid(), 'plan': plan,
                    'path': str(self.repo / 'test_calculator.py'), 'action': 'implemented AddTests.test_add_negative directly'})
                self.author('implement')
                self.checkpoint('implementation')
                self.event(EVENTS[state])
            elif state == 'reconciliation':
                plan = json.loads((self.artifacts / 'plan.json').read_text())
                outcomes = []
                for spec in plan['proof_commands']:
                    result = subprocess.run([spec['command'], *spec['args']], cwd=self.repo, text=True, capture_output=True)
                    outcomes.append({'spec': spec, 'exit_code': result.returncode, 'stdout': result.stdout, 'stderr': result.stderr})
                save(self.artifacts / 'early-proof.json', outcomes)
                self.append('driver-note', {'external_command_results': outcomes})
                self.author('reconciliation-draft')
                reconciliation = json.loads((self.artifacts / 'reconciliation.json').read_text())
                if reconciliation['decision'] == 'blocked':
                    self.append('driver-note', {'triage': 'correct genuine reconciliation blockers', 'blockers': reconciliation['blockers']})
                    self.event('revise-implementation')
                else:
                    self.checkpoint('implementation')
                    self.event(EVENTS[state])
            elif state == 'validation':
                self.validation()
            else:
                raise RuntimeError(f'Unhandled state: {state}')
        raise RuntimeError('Driver correction budget exhausted before completion')


def initialize(args):
    args.work_root.resolve().mkdir(parents=True, exist_ok=True)
    root = Path(tempfile.mkdtemp(prefix="attempt-", dir=args.work_root.resolve()))
    repo = root / 'repository'
    repo.mkdir()
    artifacts = repo
    for argv in [['git','init','-q'], ['git','config','user.name','Native Proof'],
                 ['git','config','user.email','native@example.invalid']]:
        subprocess.run(argv, cwd=repo, check=True)
    (repo / '.git/info/exclude').write_text('*.json\n__pycache__/\n')
    (repo / 'calculator.py').write_text('def add(a, b):\n    return a - b\n')
    (repo / 'test_calculator.py').write_text('import unittest\nfrom calculator import add\n')
    subprocess.run(['git','add','.'], cwd=repo, check=True)
    subprocess.run(['git','commit','-qm','fictional bug'], cwd=repo, check=True)
    software = args.software.resolve()
    data = root / 'data'
    subprocess.run([software, 'data-dump', data], check=True, capture_output=True)
    base = data / 'crates/software-change-provider/data/configs/high-rigor.json'
    profile = json.loads(base.read_text())
    settings = ['--set','criterion.required_authors=1','--set','goal.required_authors=1']
    for gate, policies in profile['review_policies'].items():
        for policy in policies:
            key = f'review.{gate}.{policy.get("review_stage", "aggregate")}.{policy["id"]}'
            settings += ['--set', key + '.required_authors=1', '--set', key + '.group=native-' + policy.get('review_stage', 'aggregate')]
    selected = root / 'profile.json'
    profile = process([software, 'setup', '--profile', base, '--output', selected, '--decline-advice', *settings])
    profile['artifact_root'] = str(artifacts)
    save(selected, profile)
    config = root / 'providers.toml'
    config.write_text('[providers.software-change]\ncommand = ' + json.dumps(str(software)) + '\nargs = []\n')
    meta = {'engine': str(args.engine.resolve()), 'software': str(software), 'catalog': str(root / 'catalog.sqlite'),
            'config': str(config), 'run_id': 'native-software', 'repository': str(repo), 'artifacts': str(artifacts)}
    save(root / 'driver.json', meta)
    driver = Driver(root)
    driver.engine('start', '--id', meta['run_id'], 'software-change', '@' + str(selected),
                  'Fix fictional integer addition and deliver runnable regression coverage for positive and negative operands. No release or docs changes.')
    return driver


def assert_receipt(path):
    receipt = json.loads(path.read_text())
    driver = Driver(receipt['work_root'])
    shown = driver.show()
    assert shown['result']['current_state'] == 'end' and shown['result']['lifecycle'] == 'final'
    assert not shown['result']['has_overrides']
    calls = {c['id']: c for c in receipt['model_calls']}
    assert calls
    actual_answers = {}
    for call in calls.values():
        assert call['model'] == MODEL and call['exit_code'] == 0
        assert Path(call['stdout']).is_file() and Path(call['request']).is_file()
        assert '--no-extensions' in call['argv'] and '--no-context-files' in call['argv']
        messages = [json.loads(line) for line in Path(call['stdout']).read_text().splitlines()]
        final = [m['message'] for m in messages if m.get('type') == 'message_end' and m.get('message', {}).get('role') == 'assistant' and m['message'].get('stopReason') == 'stop'][-1]
        assert final['model'] == 'gpt-6.1-sol' and final['provider'] == 'openai-codex'
        actual_answers[call['id']] = json_answer(''.join(c['text'] for c in final['content'] if c['type'] == 'text'))
        if call['purpose'].startswith(('review-', 'verdict-')):
            assert call['argv'][call['argv'].index('--thinking') + 1] == 'high'
            assert call['argv'][call['argv'].index('--tools') + 1] == 'read,grep,find,ls'
    assert any(c['purpose'] == 'grouped-implementation' for c in calls.values())
    direct = json.loads((driver.root / 'direct-implementation.json').read_text())
    grouped = json.loads((driver.root / 'grouped-implementation.json').read_text())
    task_ids = [task['id'] for task in direct['plan']['tasks']]
    assert len(task_ids) == 3 and grouped['response']['tasks'] == task_ids[:2]
    assert direct['action'] == 'implemented AddTests.test_add_negative directly'
    assert direct['path'] == str(driver.repo / 'test_calculator.py')
    assert actual_answers[grouped['model_call']] == grouped['response']
    with sqlite3.connect('file:' + receipt['catalog'] + '?mode=ro', uri=True) as catalog:
        rows = [(kind, json.loads(data)) for kind, data in catalog.execute('SELECT kind,data_json FROM context_records WHERE run_id=?', (receipt['run_id'],))]
    links = [(data['review_author'], data['model_call']) for kind, data in rows if kind == 'driver-note' and 'review_author' in data]
    for kind, data in rows:
        if kind == 'review-evidence':
            original = data['external_review']['original']['structured']
            assert any(author == data['author']['name'] and actual_answers.get(call_id) == original for author, call_id in links)
        elif kind in ('criterion-verdict', 'goal-verdict'):
            assert any(author == data['author']['name'] and actual_answers.get(call_id) == data for author, call_id in links)
        elif kind == 'finding-ledger':
            assert data in actual_answers.values()
    assert receipt['continuation']['first_pid'] != receipt['continuation']['second_pid']
    assert receipt['continuation']['state']['result']['current_state'] == 'implement'
    assert any(p['group'].startswith('native-') for ps in shown['result']['initial_input']['review_policies'].values() for p in ps)
    for line in (driver.root / 'engine-commands.jsonl').read_text().splitlines():
        argv = json.loads(line)['argv']
        assert argv[6] in ['start', 'show', 'append', 'event']
    assert receipt['final_show']['result']['current_state'] == shown['result']['current_state']
    assert receipt['source_identity'] == driver.provider(['source-identity', '--working-directory', str(driver.repo)])
    print('genuine native completed receipt verified:', path)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--mode', required=True, choices=['run', 'assert', 'continue'])
    parser.add_argument('--engine', type=Path, default=ROOT / 'target/debug/loop-engine')
    parser.add_argument('--software', type=Path, default=ROOT / 'target/debug/software-change')
    parser.add_argument('--work-root', type=Path)
    parser.add_argument('--receipt', type=Path)
    parser.add_argument('--resume', type=Path, help='Retry a retained failed attempt without discarding genuine calls or judgments')
    args = parser.parse_args()
    if args.mode == 'assert':
        assert_receipt(args.receipt)
        return
    if args.mode == 'continue':
        driver = Driver(args.work_root)
        continuation = json.loads((driver.root / 'continuation.json').read_text())
        continuation['second_pid'] = os.getpid()
        save(driver.root / 'continuation.json', continuation)
        driver.drive()
        return
    driver = Driver(args.resume) if args.resume else initialize(args)
    if not (driver.root / 'continuation.json').exists():
        driver.drive(stop_after_plan=True)
    subprocess.run([sys.executable, str(Path(__file__).resolve()), '--mode', 'continue',
                    '--work-root', str(driver.root)], check=True)
    receipt = {'work_root': str(driver.root), 'run_id': driver.meta['run_id'], 'catalog': driver.meta['catalog'],
               'model_calls': [json.loads(line) for line in (driver.root / 'model-calls.jsonl').read_text().splitlines()],
               'final_show': driver.show(), 'source_identity': driver.provider(['source-identity','--working-directory',str(driver.repo)]),
               'continuation': json.loads((driver.root / 'continuation.json').read_text())}
    save(driver.root / 'receipt.json', receipt)
    if args.receipt:
        save(args.receipt, receipt)
    assert_receipt(args.receipt or driver.root / 'receipt.json')


if __name__ == '__main__':
    main()
