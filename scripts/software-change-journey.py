#!/usr/bin/env python3
"""External-driver software-change journey using scripted judgments, not semantic proof."""
import argparse
import hashlib
import os
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from concurrent.futures import ThreadPoolExecutor

ROOT = Path(__file__).resolve().parents[1]
PREDECESSOR = '506d6cce47a68496afd842e3388044341a68b667'


class CommandLog:
    """Append metadata immediately; retain exact payload bytes in separate files."""
    def __init__(self, root):
        self.root = Path(root)
        self.root.mkdir(parents=True, exist_ok=True)
        self.rows = []
        self.index = self.root / 'commands.jsonl'
        if self.index.exists():
            raise ValueError('Command capture index already exists; use a fresh root')
        self.environment = {k: v for k, v in os.environ.items()
                            if not k.startswith(('LOOP_', 'XDG_'))}

    def execute(self, argv, packet=None, *, cwd=None, raw=False):
        argv = [str(value) for value in argv]
        stdin = None if packet is None else json.dumps(packet).encode('utf-8')
        captured = subprocess.run(argv, input=stdin, cwd=cwd, env=self.environment,
                                  capture_output=True)
        command_id = f'{len(self.rows) + 1:06d}'
        def retain(stream, content):
            if content is None:
                return None
            path = self.root / f'{command_id}.{stream}'
            with path.open('xb') as output:
                output.write(content)
                output.flush()
                os.fsync(output.fileno())
            return {'path':path.name, 'bytes':len(content),
                    'sha256':hashlib.sha256(content).hexdigest()}
        row = {'command_id':command_id,
               'fixture_label':'SCRIPTED DUMMY BACKENDS; actual external command outcome',
               'argv': argv, 'cwd': str(Path(cwd or Path.cwd()).resolve()),
               'stdin':retain('stdin', stdin), 'exit_code':captured.returncode,
               'stdout':retain('stdout', captured.stdout), 'stderr':retain('stderr', captured.stderr)}
        with self.index.open('a', encoding='utf-8') as index:
            index.write(json.dumps(row) + '\n')
            index.flush()
            os.fsync(index.fileno())
        self.rows.append(row)
        # Decode only for consumers; raw files above preserve even non-UTF8 bytes
        # and CRLF without subprocess universal-newline conversion. Invalid UTF8
        # still fails closed, but now its exact failure evidence is retained first.
        try:
            result = subprocess.CompletedProcess(argv, captured.returncode,
                        captured.stdout.decode('utf-8'), captured.stderr.decode('utf-8'))
        except UnicodeDecodeError as error:
            raise RuntimeError(f'Non-UTF8 command response: {row}') from error
        if raw:
            return result
        try:
            value = json.loads(result.stdout)
        except json.JSONDecodeError as error:
            raise RuntimeError(f'Non-JSON command response: {row}') from error
        if result.returncode and value.get('status') != 'rejected':
            raise RuntimeError(f'Unexpected failed command: {row}')
        return value

    def finalize(self):
        """Optional small metadata-only convenience index; JSONL is authoritative."""
        (self.root / 'commands.json').write_text(json.dumps(self.rows, indent=2))

    def identity(self, engine, provider, *extra_providers):
        binaries = {str(Path(p).resolve()): hashlib.sha256(Path(p).read_bytes()).hexdigest()
                    for p in (engine, provider, *extra_providers)}
        source = self.execute(['git', 'rev-parse', 'HEAD'], cwd=ROOT, raw=True)
        diff = self.execute(['git', 'diff', '--binary'], cwd=ROOT, raw=True)
        files = self.execute(['git', 'ls-files', '--modified', '--others', '--exclude-standard'],
                             cwd=ROOT, raw=True)
        hashes = {p: hashlib.sha256((ROOT / p).read_bytes()).hexdigest()
                  for p in files.stdout.splitlines() if (ROOT / p).is_file()}
        (self.root / 'identity.json').write_text(json.dumps(
            {'binaries': binaries, 'source_head': source.stdout.strip(),
             'source_diff': diff.stdout, 'modified_and_untracked_sha256': hashes}, indent=2))


SUBJECTS = {'intent.json':'intent-good.json', 'design.json':'design-good.json',
            'plan.json':'plan-good.json', 'implementation-report.json':'implementation-report-good.json',
            'validation-report.json':'validation-report-good.json'}


def append_scripted_intent_comparison(call, show, run_id, intent, cold):
    """Use the existing owner-authorized agent convention, never human approval."""
    steering = {'target':{'kind':'slots', 'ids':['intent-review']},
                'instruction':'SCRIPTED DUMMY BACKENDS: a simulated owner authorizes the '
                              'scripted agent driver to compare every current cold reconstruction '
                              'against this exact frozen intent baseline. Mechanical fixture only; '
                              'not real owner authorization or semantic approval.'}
    authorization = call('append', run_id, 'user-steering', json.dumps(steering))
    assert authorization['status'] == 'completed', authorization
    source = authorization['result']['context']
    assert source['kind'] == 'user-steering' and source['data'] == steering, source
    retained = next(r for r in show()['result']['context'] if r['id'] == source['id'])
    assert retained == source, retained
    comparison = {'subject_revision':intent['revision'], 'baseline':intent['baseline'],
                  'reconstruction_ids':[r['id'] for r in cold],
                  'actor':{'name':'scripted-owner-authorized-driver', 'kind':'agent'},
                  'authority':source['id'],
                  'dispositions':[{'reconstruction_id':r['id'],
                                   'alternative':'Scripted fixture reconstruction; no material alternative supplied.',
                                   'disposition':'implementation-freedom',
                                   'note':'Scripted mechanical comparison only, not semantic judgment.'} for r in cold]}
    appended = call('append', run_id, 'intent-comparison', json.dumps(comparison))
    assert appended['status'] == 'completed', appended
    assert appended['result']['context']['data'] == comparison, appended
    return comparison


def self_test():
    paths = [ROOT / 'AGENTS.md', ROOT / 'skills/using-loop-engine/SKILL.md']
    paths += list((ROOT / 'crates').glob('*/skills/*/SKILL.md'))
    retired = re.compile(r'loop-engine\s+(?:invoke|fan-out|preview-bindings|amend-binding|capture-command|capture-matrix)\b|work_slot_bindings')
    violations = [str(path.relative_to(ROOT)) for path in paths if retired.search(path.read_text())]
    if violations:
        raise RuntimeError('external-work skill/root policy assertions failed: ' + ', '.join(violations))
    # Regression for the validator's permitted agent + explicit authorization
    # convention. IDs below are emitted by the stub append, not chosen by helper.
    records, calls = [], []
    intent = {'revision':'fixture-r1', 'baseline':{'kind':'git','repository_scope':'primary','commit':'exact-fixture-baseline'}}
    cold = [{'id':f'stub-retained-cold-{i}', 'author':{'name':f'reviewer-{i}'}} for i in range(2)]
    def append_stub(*argv):
        calls.append(argv)
        assert argv[:2] == ('append','stub-run')
        record = {'id':f'stub-append-emitted-{len(records)+1}', 'kind':argv[2],
                  'data':json.loads(argv[3]), 'sequence':len(records)+1}
        records.append(record)
        return {'status':'completed', 'result':{'context':record}}
    comparison = append_scripted_intent_comparison(append_stub,
        lambda:{'result':{'context':records}}, 'stub-run', intent, cold)
    assert [r['kind'] for r in records] == ['user-steering','intent-comparison']
    assert records[0]['data']['target'] == {'kind':'slots','ids':['intent-review']}
    assert set(records[0]['data']) == {'target','instruction'}
    assert 'SCRIPTED DUMMY BACKENDS' in records[0]['data']['instruction']
    assert comparison['actor']['kind'] == 'agent'
    assert comparison['authority'] == records[0]['id'] and comparison['authority'] != 'owner'
    assert set(comparison) == {'subject_revision','baseline','reconstruction_ids','actor','authority','dispositions'}
    assert comparison['baseline'] == intent['baseline'] and comparison['subject_revision'] == intent['revision']
    assert comparison['reconstruction_ids'] == [r['id'] for r in cold]
    assert [d['reconstruction_id'] for d in comparison['dispositions']] == [r['id'] for r in cold]
    # Exact bytes (including CRLF/non-UTF8 stderr) and append-only metadata; no
    # repeated final-index serialization, even with large command responses.
    from unittest.mock import patch
    with tempfile.TemporaryDirectory(prefix='software-recorder-interface-') as directory:
        log = CommandLog(directory)
        stdout = b'{"status":"completed","padding":"' + b'x'*65536 + b'"}\r\n'
        stderr = b'scripted stderr\r\n\x00'
        completed = subprocess.CompletedProcess(['stub'], 0, stdout, stderr)
        packet = {'label':'SCRIPTED fixture', 'non_ascii':'\u2014'}
        with patch.object(subprocess, 'run', return_value=completed) as stub:
            log.execute(['stub'], packet, cwd=directory)
            assert stub.call_args.kwargs['input'] == json.dumps(packet).encode('utf-8')
            assert 'text' not in stub.call_args.kwargs
            prefix = log.index.read_bytes()
            first = log.rows[0]
            for _ in range(19):
                log.execute(['stub'], packet, cwd=directory)
        assert log.index.read_bytes().startswith(prefix) and log.rows[0] == first
        assert (log.root / first['stdout']['path']).read_bytes() == stdout
        assert (log.root / first['stderr']['path']).read_bytes() == stderr
        assert (log.root / first['stdin']['path']).read_bytes() == json.dumps(packet).encode('utf-8')
        assert len(log.rows) == 20 and len(log.index.read_bytes()) < len(stdout)
        assert not (log.root / 'commands.json').exists()
        error = subprocess.CompletedProcess(['stub'], 1, b'{"status":"error"}', b'failed\r\n')
        with patch.object(subprocess, 'run', return_value=error):
            try:
                log.execute(['stub-error'])
            except RuntimeError:
                pass
            else:
                raise AssertionError('Unexpected command error accepted')
        rows = [json.loads(line) for line in log.index.read_text().splitlines()]
        assert rows == log.rows and rows[-1]['exit_code'] == 1
        assert (log.root / rows[-1]['stderr']['path']).read_bytes() == b'failed\r\n'
        invalid_utf8 = subprocess.CompletedProcess(['stub'], 0, b'{"status":"completed"}', b'\xff\r\n')
        with patch.object(subprocess, 'run', return_value=invalid_utf8):
            try:
                log.execute(['stub-invalid-utf8'])
            except RuntimeError as error:
                assert 'Non-UTF8' in str(error)
            else:
                raise AssertionError('Invalid UTF8 command response accepted')
        rows = [json.loads(line) for line in log.index.read_text().splitlines()]
        assert rows == log.rows and (log.root / rows[-1]['stderr']['path']).read_bytes() == b'\xff\r\n'
        log.finalize()
        assert json.loads((log.root / 'commands.json').read_text()) == rows
        assert (log.root / 'commands.json').stat().st_size < len(stdout)
        payload_bytes = sum(row[stream]['bytes'] for row in rows
                            for stream in ('stdin','stdout','stderr') if row[stream] is not None)
        print(f'SCRIPTED recorder stub: {len(rows)} commands; payload_bytes={payload_bytes}; '
              f'append_only_index_bytes={log.index.stat().st_size}; '
              f'final_metadata_index_bytes={(log.root / "commands.json").stat().st_size}')
    print('external-work skill/root policy and scripted comparison/append-only recorder assertions passed')


# bookends:LE-39 bookends:LE-40 bookends:LE-43 bookends:LE-53 bookends:LE-85 — externally driven fixture artifacts complete under selected, materially different policies without invocation receipts.
def run(args, profile_source, root, *, configure=None, exercise=None, judgment_reason=None,
        stop_at=None, capture=False, command_log=None):
    engine, provider = Path(args.engine).resolve(), Path(args.provider).resolve()
    work = Path(tempfile.mkdtemp(prefix='software-external-', dir=root))
    # Callers may run several traversals under one root; an unshared log is owned by this traversal.
    log = command_log or CommandLog(work / 'commands')
    invoke = log.execute
    def raw(argv, *, cwd=None, check=True):
        result = invoke(argv, cwd=cwd, raw=True)
        if check and result.returncode:
            raise RuntimeError(f'Command failed: {argv}: {result.stderr}')
        return result
    artifacts = work / 'artifacts'
    artifacts.mkdir()
    repository = work / 'repository'
    repository.mkdir()
    for command in [['git','init','-q'],['git','config','user.name','Journey'],
                    ['git','config','user.email','journey@example.invalid']]:
        raw(command, cwd=repository)
    (repository / 'helper.py').write_text('def helper(value):\n    return value\n')
    raw(['git','add','helper.py'], cwd=repository)
    raw(['git','commit','-qm','fixture'], cwd=repository)
    data_root = Path(args.data_root or ROOT).resolve()
    if args.mode == 'packaged':
        if not args.data_root or (data_root.exists() and any(data_root.iterdir())):
            raise ValueError('Packaged mode requires an empty --data-root.')
        raw([provider,'data-dump',data_root])
        # bookends:LE-106 — packaged execution uses only data materialized by the compiled provider, not the source checkout.
        profile_source = data_root / 'crates/software-change-provider/data/configs' / Path(profile_source).name
        assert profile_source.is_file(), profile_source
    profile = json.loads(Path(profile_source).read_text())
    profile['artifact_root'] = str(artifacts)
    selected = work / 'profile.json'
    fixtures = data_root / 'crates/software-change-provider/data/calibration/fixtures'
    for target, source in SUBJECTS.items():
        shutil.copyfile(fixtures / source, artifacts / target)
    plan = json.loads((artifacts / 'plan.json').read_text())
    plan['proof_commands'] = [{'id':'helper-test','command':sys.executable,
                              'args':['-B','-c','from helper import helper; assert helper(7) == 7'],
                              'owner':'driver','obligation':'Assert the fixture helper returns its input.'}]
    (artifacts / 'plan.json').write_text(json.dumps(plan))
    implementation = json.loads((artifacts / 'implementation-report.json').read_text())
    implementation['validation'] = [{'proof':'External helper-test is required before normal completion.'}]
    (artifacts / 'implementation-report.json').write_text(json.dumps(implementation))
    intent_path = artifacts / 'intent.json'
    intent = json.loads(intent_path.read_text())
    intent['baseline'] = {'kind':'git','repository_scope':'primary',
                          'commit':raw(['git','rev-parse','HEAD'],cwd=repository).stdout.strip()}
    intent_path.write_text(json.dumps(intent))
    if configure:
        configure(profile, repository, artifacts)
    selected.write_text(json.dumps(profile))
    (work / 'fixture-declaration.json').write_text(json.dumps({
        'label':'SCRIPTED DUMMY BACKENDS: mechanical proof only; no semantic review or human approval',
        'artifact_sources':{target:str(fixtures / source) for target, source in SUBJECTS.items()},
        'changes':'Scratch helper repository; real helper-test proof; current Git baseline; prepared full validation index',
        'actual_command_outcomes':str(log.index), 'command_payload_root':str(log.root),
        'frozen_profile':profile}, indent=2))
    config = work / 'providers.toml'
    config.write_text(f'[providers.software-change]\ncommand = {json.dumps(str(provider))}\nargs = []\n')
    database = work / 'run.sqlite'
    run_id = 'software-external'
    def call(*args, allow_rejected=False):
        # CommandLog retains every command and returns rejected envelopes as values;
        # allow_rejected is accepted for parallelism callers that expect a denial.
        if args[0] in ('append', 'event'):
            show()  # One driver: EACH mutation gets a new arming observation.
        return invoke([engine,'--database',database,'--config',config,'--json',*args],cwd=repository)
    def invoke_here(argv, packet=None, *, cwd=repository, allow_rejected=False):
        return invoke(argv, packet, cwd=cwd)
    def append(kind, data, record_id=None):
        argv = ['append']
        if record_id:
            argv += ['--record-id',record_id]
        # Keep growing evidence out of argv without changing its serialized bytes.
        with tempfile.NamedTemporaryFile(dir=work, suffix='.json') as source:
            source.write(json.dumps(data).encode('utf-8'))
            source.flush()
            return call(*argv,run_id,kind,'@'+source.name)
    def show():
        return call('show','--view','full',run_id)
    def event(name):
        show()
        response = call('event',run_id,name)
        assert response['status'] == 'completed', response
        return response
    def checkpoint(phase):
        before = [raw(['git',*argv],cwd=repository).stdout for argv in [['rev-parse','HEAD'],['status','--porcelain']]]
        result = invoke([provider,'checkpoint','--phase',phase,'--artifact-root',artifacts,
                         '--working-directory',repository],cwd=repository)
        after = [raw(['git',*argv],cwd=repository).stdout for argv in [['rev-parse','HEAD'],['status','--porcelain']]]
        assert before == after, 'checkpoint mutated Git state'
        return result
    call('start','--id',run_id,'software-change','@'+str(selected),'external software')
    visited = []
    for _ in range(1000):
        shown = show()
        # bookends:LE-41 bookends:LE-42 — fresh-process inspection recovers the exact frozen review policies.
        assert shown['result']['initial_input']['review_policies'] == profile['review_policies'], shown
        state = shown['result']['current_state']
        visited.append(state)
        (work / 'states.json').write_text(json.dumps(visited))
        print('external software state:',state,flush=True)
        if exercise:
            handled = exercise(state, {'call':call,'show':show,'event':event,'checkpoint':checkpoint,
                                       'profile':profile,'artifacts':artifacts,'repository':repository,
                                       'provider':provider,'engine':engine,'work':work,'run_id':run_id,
                                       'database':database,'config':config,'shown':shown,'invoke':invoke_here,'append':append})
            if handled:
                continue
            shown = show()
        if state == stop_at:
            return work
        if state == 'end':
            assert shown['result']['lifecycle'] == 'final' and not shown['result']['has_overrides'], shown
            break
        if state.endswith('review'):
            subject = ('intent.json' if state.startswith('intent') else 'design.json' if state.startswith('design')
                       else 'plan.json' if state.startswith('plan') else 'implementation-report.json'
                       if state.startswith('implementation') else 'validation-report.json')
            revision = json.loads((artifacts / subject).read_text())['revision']
            append_reviews({'call':call,'append':append,'invoke':invoke_here,'shown':shown,'profile':profile,
                            'repository':repository,'provider':provider,'run_id':run_id},
                           state,subject,revision,profile['review_policies'][state])
            if state == 'intent-review' and any(p['id'] == 'intent-sufficient' for p in profile['review_policies'][state]):
                cold = [r for r in show()['result']['context'] if r['kind'] == 'review-evidence'
                        and r['data'].get('gate') == state and r['data'].get('policy_id') == 'intent-sufficient'
                        and r['data'].get('subject_revision') == revision]
                append_scripted_intent_comparison(call, show, run_id,
                    json.loads((artifacts / subject).read_text()), cold)
            ledger = {'schema_version':'1','gate':state,'subject':subject,'subject_revision':revision,
                      'author':{'name':'external-driver','kind':'agent'},'findings':[]}
            call('append',run_id,'finding-ledger',json.dumps(ledger))
            event('approved' if any(e['event'] == 'approved' for e in shown['result']['requestable_events']) else 'passed')
        elif state == 'explore':
            event('intent-ready')
        elif state == 'design':
            event('design-ready')
        elif state == 'plan':
            event('plan-ready')
        elif state == 'implement':
            checkpoint('implementation')
            event('implementation-ready')
        elif state == 'reconciliation':
            reconciliation = {'revision':'reconciliation-1','author':{'name':'external-driver','kind':'script'},
                              'mode':'bookends-disabled','branch':'sufficient-existing-wording',
                              'document_observations':[{'path':'helper.py','status':'unchanged',
                                                        'observation':'Fixture source requires no document amendment.'}],
                              'behavior_observations':[{'status':'matches-intent','observation':'The helper returns its input.'}],
                              'action':'no-document-change','action_reason':'No enduring document meaning changes.',
                              'authorization':'not-required','application':'not-required','commit':'not-required',
                              'traceability':{'status':'not-applicable','references':[]},
                              'proof_references':['plan.json#/proof_commands'],'blockers':[],'decision':'complete'}
            (artifacts / 'reconciliation.json').write_text(json.dumps(reconciliation))
            # bookends:LE-142 — explicit document/behavior comparison is checked before the run leaves reconciliation.
            event('reconciliation-ready')
        elif state == 'validation':
            commission = invoke([provider,'commission','--slot','validation-draft'],shown,cwd=repository)
            results = []
            for spec in commission['commission']['proof_commands']:
                identity = invoke([provider,'source-identity','--working-directory',repository],cwd=repository)
                # bookends:LE-117 — the external proof owner runs each requested command once; reviewers consume its retained actual result.
                outcome = raw([spec['command'],*spec['args']],cwd=repository)
                assert outcome.returncode == 0, outcome.stderr
                results.append({'id':spec['id'],'command':spec['command'],'args':spec['args'],
                                'cwd':str(repository),'exit_code':outcome.returncode,
                                'stdout':outcome.stdout,'stderr':outcome.stderr,
                                'producer':{'name':'external-driver','kind':'script'},'source':identity})
            prepared = invoke([provider,'prepare-validation'],
                               {'show':shown,'working_directory':str(repository),'revision':'external-v1',
                                'author':{'name':'external-proof-owner','kind':'script'},'command_results':results},cwd=repository)
            # bookends:LE-116 — actual external executions satisfy the fixed index; preparation alone is not progression or a judgment.
            assert prepared['commands_complete'], prepared
            report = prepared['report_draft']
            (artifacts / 'validation-report.json').write_text(json.dumps(report))
            checkpoint('validation')
            for candidate in prepared['command_candidates']:
                append(candidate['kind'],candidate['data'],candidate['record_id'])
            judgments_handled = exercise and exercise('validation-judgments', {
                'call':call,'show':show,'event':event,'checkpoint':checkpoint,
                'profile':profile,'artifacts':artifacts,'repository':repository,
                'provider':provider,'engine':engine,'work':work,'run_id':run_id,
                'database':database,'config':config,'shown':show(),'report':report,
                'prepared':prepared,'invoke':invoke})
            for index, batch in enumerate([] if judgments_handled else prepared['judgment_batches']):
                verdict = {'subject':'validation-report.json','subject_revision':report['revision'],
                           'checkpoint':'validation-checkpoint.json',
                           'author':{'name':f'external-judge-{index}','kind':'agent'},
                           'result':'pass','findings':[],'reason':judgment_reason or 'Inspected actual driver command outcome.',
                           'evidence_context_ids':report['command_evidence_ids']}
                if batch['kind'] == 'criterion-verdict':
                    verdict['criterion_id'] = batch['criterion_id']
                append(batch['kind'],verdict,batch['record_id'])
            if exercise:
                exercise('validation-prepared', {'call':call,'show':show,'event':event,'checkpoint':checkpoint,
                                                'profile':profile,'artifacts':artifacts,'repository':repository,
                                                'provider':provider,'engine':engine,'work':work,'run_id':run_id,
                                                'database':database,'config':config,'shown':show(),'report':report,'invoke':invoke})
            event('validation-ready' if any(e['event'] == 'validation-ready' for e in shown['result']['requestable_events']) else 'passed')
        else:
            raise RuntimeError(f'Unexpected workflow state {state}')
    else:
        raise RuntimeError('Journey exceeded transition budget without reaching end')
    history = call('history', run_id)
    assert history['status'] == 'completed', history
    (work / 'history.json').write_text(json.dumps(history, indent=2))
    (work / 'terminal.json').write_text(json.dumps(shown, indent=2))
    log.finalize()
    return work


def append_reviews(ctx, gate, subject, revision, policies, *, wrong_group=False):
    """Return scripted external judgments through public intake, preserving group batches."""
    shown = ctx['shown']
    for policy in policies:
        for index in range(policy.get('required_authors',1)):
            author = {'name':f'external-{gate}-{index}','kind':'agent'}
            assignment = {'gate':gate,'stage':policy.get('review_stage','aggregate'),
                          'subject':subject,'subject_revision':revision,'group':policy['group'],
                          'author':author}
            item = {'axis':policy['id'],'result':'pass','findings':'',
                    'grounds':{'reason':'Scripted current-subject inspection.',
                               'evidence':[{'locator':subject+'#/revision'}]}}
            packet = {'show':shown,'assignment':assignment,'author':author,
                      'request':{'text':f'Review {subject}.'},
                      'original':{'text':json.dumps({
                          'assessment_id':f'{gate}-{policy.get("review_stage","aggregate")}-{shown["result"]["state_visit"]}-{index}',
                          'judgments':[item]})}}
            intake = ctx['invoke']([ctx['provider'],'review-candidates'],packet)
            assert intake.get('records'), intake
            # bookends:LE-44 bookends:LE-47 bookends:LE-137 — intake supplies durable rows, not progression authority.
            for candidate in intake['records']:
                data = candidate['data']
                if wrong_group:
                    # Deliberately mis-tag an otherwise valid individual judgment at append,
                    # so evaluate (not intake's assignment check) must enforce membership.
                    data['group'] = gate+'.individual.dependencies-honest'
                appended = ctx['append']('review-evidence',data)
                assert appended['result']['context']['data'] == data, appended


PLAN_GUIDANCE = ('Split work into tasks that do not share writable files or state unless one '
                 'depends on the other, and declare a dependency only where a real contract, '
                 'context or hard-input need exists.')
IMPLEMENT_GUIDANCE = ('Tasks with no dependency path between them (no chain of declared dependencies '
                      'in either direction) are dispatched concurrently by default; state a reason '
                      'when running them serially.')
PARALLELISM_AXIS = 'parallelism-honest'
PLAN_GATES = ('plan-review','plan-adversarial-review')


# bookends:LE-184 bookends:LE-130 — every successor profile exposes guidance and denies missing plan-axis coverage at every stage.
def parallelism_traversal(args, profile_source, root):
    receipts = []
    def configure(profile, repository, artifacts):
        assert profile['config_version'] == profile_source.stem+'-15', profile['config_version']
        for gate in PLAN_GATES:
            policies = profile['review_policies'][gate]
            siblings = [p for p in policies if p['id'] == 'dependencies-honest']
            parallel = [p for p in policies if p['id'] == PARALLELISM_AXIS]
            assert siblings and len(parallel) == len(siblings), (gate,policies)
            for sibling in siblings:
                matching = [p for p in parallel if p['review_stage'] == sibling['review_stage']]
                assert len(matching) == 1, (gate,sibling,parallel)
                policy = matching[0]
                assert policy['required_authors'] == sibling['required_authors'], (gate,policy,sibling)
                expected_group = (gate+'.individual.'+PARALLELISM_AXIS
                                  if sibling['review_stage'] == 'individual' else sibling['group'])
                assert policy['group'] == expected_group, (gate,policy)

    def exercise(state, ctx):
        shown = ctx['shown']
        work = ctx['work']
        if state in ('plan','implement'):
            guidance = PLAN_GUIDANCE if state == 'plan' else IMPLEMENT_GUIDANCE
            # bookends:LE-184 — AC-1 observes instructions returned by the real engine/provider, not source literals.
            (work / f'{state}-show.json').write_text(json.dumps(shown,indent=2))
            assert guidance in shown['result']['current_state_instructions'], (state,shown)
        if state not in PLAN_GATES:
            return False
        profile = ctx['profile']
        policies = profile['review_policies'][state]
        parallel = [p for p in policies if p['id'] == PARALLELISM_AXIS]
        stages = {p['review_stage'] for p in parallel}
        revision = json.loads((ctx['artifacts'] / 'plan.json').read_text())['revision']
        event = 'approved' if any(e['event'] == 'approved' for e in shown['result']['requestable_events']) else 'passed'
        append_reviews(ctx,state,'plan.json',revision,[p for p in policies if p['id'] != PARALLELISM_AXIS])
        ctx['call']('append',ctx['run_id'],'finding-ledger',json.dumps({
            'schema_version':'1','gate':state,'subject':'plan.json','subject_revision':revision,
            'author':{'name':'external-driver','kind':'agent'},'findings':[]}))

        def refuse(label, expected_stages):
            ctx['show']()
            response = ctx['call']('event',ctx['run_id'],event,allow_rejected=True)
            (work / f'{state}-{label}.json').write_text(json.dumps(response,indent=2))
            # bookends:LE-130 bookends:LE-184 — rejected envelopes name the missing axis at each configured stage.
            assert response['status'] == 'rejected', response
            def diagnostics(value):
                if isinstance(value,dict):
                    if value.get('axis') == PARALLELISM_AXIS and 'review_stage' in value:
                        yield value
                    for child in value.values():
                        yield from diagnostics(child)
                elif isinstance(value,list):
                    for child in value:
                        yield from diagnostics(child)
            axis_diagnostics = list(diagnostics(response))
            assert {d['review_stage'] for d in axis_diagnostics} == expected_stages, response
            assert all(any(d.get('category') in ('missing','independence')
                           for d in axis['diagnostics']) for axis in axis_diagnostics), response
            after = ctx['show']()
            assert after['result']['current_state'] == state, after
            assert after['result']['state_visit'] == shown['result']['state_visit'], after
            return axis_diagnostics

        missing = refuse('missing-parallelism',stages)
        if profile['config_version'] == 'high-rigor-15' and state == 'plan-review':
            aggregate = [p for p in parallel if p['review_stage'] == 'aggregate']
            individual = [p for p in parallel if p['review_stage'] == 'individual']
            append_reviews(ctx,state,'plan.json',revision,aggregate)
            append_reviews(ctx,state,'plan.json',revision,individual,wrong_group=True)
            # All other evidence, including aggregate parallelism, is now complete.
            wrong = refuse('wrong-group',{'individual'})
            assert all(d.get('category') not in ('malformed','unverified')
                       for axis in wrong for d in axis['diagnostics']), wrong
            append_reviews(ctx,state,'plan.json',revision,individual)
            # Finish in the normal individual-then-aggregate order: challenge
            # clearance re-evaluates each parent group using its latest batch.
            append_reviews(ctx,state,'plan.json',revision,aggregate)
        else:
            append_reviews(ctx,state,'plan.json',revision,parallel)
        accepted = ctx['event'](event)
        (work / f'{state}-accepted.json').write_text(json.dumps(accepted,indent=2))
        after = ctx['show']()
        assert after['result']['current_state'] != state, after
        receipts.append({'gate':state,'missing_stages':sorted(stages),'missing_diagnostics':missing,
                         'wrong_group_refused':profile['config_version'] == 'high-rigor-15' and state == 'plan-review',
                         'accepted':accepted['status']})
        return True

    work = run(args,profile_source,root,configure=configure,exercise=exercise,
               stop_at='implement',capture=True,
               command_log=CommandLog(Path(root) / f'commands-parallelism-{profile_source.stem}'))
    assert {r['gate'] for r in receipts} == set(PLAN_GATES), receipts
    (work / 'parallelism-receipts.json').write_text(json.dumps({
        'profile':profile_source.stem+'-15','bookends':['LE-184','LE-130'],
        'catalog':str(work / 'run.sqlite'),'receipts':receipts,
        'guidance_captures':['plan-show.json','implement-show.json'],
        'stopped_at':'implement'},indent=2))
    print(f'parallelism traversal passed: {profile_source.stem}-15; receipts: {work}',flush=True)
    return work


# bookends:LE-184 — source proof also checks the template actually shipped by the compiled provider.
def parallelism_journeys(args, root):
    data_root = root / 'parallelism-data'
    outcome = subprocess.run([str(Path(args.provider).resolve()),'data-dump',str(data_root)],
                             text=True,capture_output=True)
    (root / 'parallelism-data-dump.json').write_text(json.dumps({
        'argv':[str(Path(args.provider).resolve()),'data-dump',str(data_root)],
        'exit_code':outcome.returncode,'stdout':outcome.stdout,'stderr':outcome.stderr},indent=2))
    assert outcome.returncode == 0, outcome.stderr
    template = data_root / 'crates/software-change-provider/data/templates/task-packet.md'
    text = template.read_text()
    assert PLAN_GUIDANCE in text, template
    assert IMPLEMENT_GUIDANCE in text, template
    profiles = [ROOT / f'crates/software-change-provider/data/configs/{name}.json'
                for name in ('minimal','standard','high-rigor')]
    # Independent catalogs/repositories; bounded by the existing external-driver run budget.
    with ThreadPoolExecutor(max_workers=args.jobs) as pool:
        works = list(pool.map(lambda profile: parallelism_traversal(args,profile,root),profiles))
    (root / 'parallelism-receipts.json').write_text(json.dumps({
        'template':str(template),'profiles':{profile.stem:str(work) for profile,work in zip(profiles,works)}},indent=2))


LE184_AXIS = 'parallelism-honest'


def strip_le184_policies(policies):
    """Remove exactly the accepted LE-184 parallelism-honest entries; each must mirror dependencies-honest."""
    stripped = {}
    for gate, rows in policies.items():
        added = [row for row in rows if row['id'] == LE184_AXIS]
        mirror = [row for row in rows if row['id'] == 'dependencies-honest']
        shape = lambda row, axis: (row['review_stage'], row['required_authors'],
                                   row['group'].replace(axis, '{axis}'))
        if added:
            assert gate in ('plan-review', 'plan-adversarial-review'), gate
            assert sorted(shape(r, LE184_AXIS) for r in added) == sorted(shape(r, 'dependencies-honest') for r in mirror), gate
        stripped[gate] = [row for row in rows if row['id'] != LE184_AXIS]
    return stripped


def strip_le184_workflow(graph, instructions=True):
    """Inverse of the exact LE-184 workflow delta: two literal instruction sentences and listed axes.

    A current binary emits the instruction sentences for any profile, so a same-binary
    (profile-only) comparison removes only the profile-derived axes."""
    graph = json.loads(json.dumps(graph))
    for sentence in (PLAN_GUIDANCE, IMPLEMENT_GUIDANCE) if instructions else ():
        holders = [s for s in graph['states'] if sentence in (s.get('instructions') or '')]
        assert len(holders) == 1 and holders[0]['instructions'].count(sentence) == 1, sentence
        holders[0]['instructions'] = holders[0]['instructions'].replace(' ' + sentence, '', 1)
    for state in graph['states']:
        guidance = state.get('action_guidance')
        if isinstance(guidance, dict) and isinstance(guidance.get('review_axes'), list):
            guidance['review_axes'] = [a for a in guidance['review_axes'] if a['id'] != LE184_AXIS]
    return graph


def compare_off(args, profile_source, root, log):
    """Change-specific AC-8 proof (intent revision 5): options-off -15 against 0.24.1 -14 and the released slot-1 provider."""
    source = Path(profile_source).resolve()
    current = json.loads(source.read_text())
    assert current['extra'].get('narratives', []) == []
    assert current['extra'].get('checkpoints', []) == []
    assert current['config_version'].endswith('-15'), current['config_version']
    predecessor_args = argparse.Namespace(**vars(args))
    historical = bool(args.predecessor_provider)
    if historical:
        predecessor_args.provider = str(Path(args.predecessor_provider).resolve())
        version = log.execute([predecessor_args.provider, '--version'], cwd=ROOT, raw=True)
        assert version.returncode == 0 and version.stdout.strip().split()[-1] == '0.24.1', version.stdout
        dumped = Path(root) / f'{source.stem}-predecessor-data'
        assert not dumped.exists(), 'Predecessor dump destination already exists'
        response = log.execute([predecessor_args.provider, 'data-dump', dumped], cwd=ROOT, raw=True)
        assert response.returncode == 0, response.stderr
        old_bytes = (dumped / 'crates/software-change-provider/data/configs' / source.name).read_bytes()
    else:
        # Explicit profile-only mechanics, NOT full historical AC-8 proof.
        relative = source.relative_to(ROOT)
        original = log.execute(['git', 'show', f'{PREDECESSOR}:{relative}'], cwd=ROOT, raw=True)
        assert original.returncode == 0, original.stderr
        old_bytes = original.stdout.encode()
    predecessor = json.loads(old_bytes)
    assert predecessor['config_version'] == current['config_version'].replace('-15', '-14')
    # AC-8 (b): review obligations differ from 0.24.1 by exactly the LE-184 entries.
    assert strip_le184_policies(current['review_policies']) == predecessor['review_policies']
    old = Path(root) / f'{source.stem}-14.json'
    old.write_bytes(old_bytes)
    graphs = [log.execute([binary], {'operation':'describe', 'initial_input':p}, cwd=ROOT)
              for binary, p in ((predecessor_args.provider, predecessor), (args.provider, current))]
    # AC-8 (a): states and transitions equal; everything else equal except the exact LE-184 delta.
    edges = lambda g: [(t['source'], t['event'], t['target'], t.get('kind')) for t in g['transitions']]
    assert [s['id'] for s in graphs[0]['states']] == [s['id'] for s in graphs[1]['states']]
    assert edges(graphs[0]) == edges(graphs[1]), 'Options-off transitions changed'
    assert graphs[0] == strip_le184_workflow(graphs[1], instructions=historical), 'Options-off workflow differs beyond the LE-184 delta'
    released = None
    if args.released_provider:
        # AC-8 (c): workflow and review policies equal the released slot-1 provider.
        released = str(Path(args.released_provider).resolve())
        dumped = Path(root) / f'{source.stem}-released-data'
        assert not dumped.exists(), 'Released dump destination already exists'
        response = log.execute([released, 'data-dump', dumped], cwd=ROOT, raw=True)
        assert response.returncode == 0, response.stderr
        shipped = json.loads((dumped / 'crates/software-change-provider/data/configs' / source.name).read_bytes())
        assert shipped['config_version'] == current['config_version']
        assert shipped['review_policies'] == current['review_policies'], 'review_policies differ from released slot-1'
        released_graph = log.execute([released], {'operation':'describe', 'initial_input':shipped}, cwd=ROOT)
        assert released_graph == graphs[1], 'Options-off workflow differs from released slot-1'
    old_work = run(predecessor_args, old, root, command_log=log)
    new_work = run(args, source, root, command_log=log)
    states = [json.loads((w / 'states.json').read_text()) for w in (old_work, new_work)]
    assert states[0] == states[1], 'Options-off public progression changed'
    assert not any('narrative' in s or s.endswith('-checkpoint') for s in states[1])
    (Path(root) / f'{source.stem}-off-comparison.json').write_text(json.dumps(
        {'scripted_dummy_backends':True, 'historical_binary_comparison':historical,
         'ac8_historical_mechanical_proof':historical, 'ac8_released_comparison':released is not None,
         'predecessor_provider':predecessor_args.provider, 'released_provider':released,
         'predecessor_ref':None if historical else PREDECESSOR,
         'limitation':None if historical and released else 'Incomplete AC-8 proof: needs --predecessor-provider and --released-provider',
         'states_and_transitions_equal':True, 'workflow_equal_modulo_exact_le184_delta':True,
         'review_policies_equal_modulo_exact_le184_entries':True, 'released_workflow_and_policies_equal':released is not None,
         'states':states[1], 'actual_command_outcomes':str(log.index), 'command_payload_root':str(log.root),
         'terminal_runs':[str(old_work),str(new_work)]}, indent=2))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--mode',choices=['source','packaged'],default='source')
    parser.add_argument('--engine')
    parser.add_argument('--provider')
    parser.add_argument('--predecessor-provider', help='Explicit installed 0.24.1 provider for historical AC-8 parity; no guessing/fallback')
    parser.add_argument('--released-provider', help='Released slot-1 provider built from main 126a4fb for AC-8 (c); no guessing/fallback')
    parser.add_argument('--profile')
    parser.add_argument('--data-root')
    parser.add_argument('--work-root')
    parser.add_argument('--traversal-depth',choices=['checked-prefix','full'],default='full')
    parser.add_argument('--jobs',type=int,default=2,help='Budget for independent driver runs')
    parser.add_argument('--self-test',action='store_true')
    args = parser.parse_args()
    # Bare invocation is the recognized CI collection form; full proof uses the parameterized run below.
    if args.self_test or len(sys.argv) == 1:
        self_test()
        return
    for key in ['engine','provider','profile']:
        if not getattr(args,key):
            parser.error(f'--{key} is required')
    if args.jobs < 1:
        parser.error('--jobs must be positive')
    root = Path(args.work_root).resolve() if args.work_root else Path(tempfile.mkdtemp(prefix='software-external-root-'))
    if root == ROOT or root.is_relative_to(ROOT):
        parser.error('--work-root must be outside the checkout')
    if args.work_root and root.exists() and any(root.iterdir()):
        parser.error('Use a new/empty external work root; receipts are never overwritten')
    root.mkdir(parents=True,exist_ok=True)
    print(f'Retained software-change proof root: {root}; SCRIPTED DUMMY BACKENDS', flush=True)
    log = CommandLog(root / 'captures')
    log.identity(args.engine, args.provider, *[p for p in (args.predecessor_provider, args.released_provider) if p])
    try:
        if args.mode == 'source' and Path(args.profile).resolve().parent == ROOT / 'crates/software-change-provider/data/configs':
            compare_off(args, args.profile, root, log)
            scope = ('historical and released-binary AC-8 mechanics' if args.predecessor_provider and args.released_provider
                     else 'partial parity; NOT complete AC-8 proof')
            print(f'options-off predecessor/successor journeys reached end ({scope}; SCRIPTED DUMMY BACKENDS)')
        elif args.mode == 'source':
            run(args,args.profile,root,command_log=log)
            print('full software-change journey passed (SCRIPTED DUMMY BACKENDS)')
        else:
            run(args,args.profile,root,command_log=log)
            print('full packaged software-change journey passed (SCRIPTED DUMMY BACKENDS)')
        if args.mode == 'source':
            parallelism_journeys(args,root)
    finally:
        log.finalize()

if __name__ == '__main__':
    main()
