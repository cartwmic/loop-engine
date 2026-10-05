"""Container-only scripted drivers: real public engine/provider CLI, mechanical reviews.

This is controlled harness proof, not a model's semantic review. No hidden files
are read. The asks-and-builds fixture implements the public example independently.
"""
import json
from pathlib import Path
import shutil
import subprocess
import sys


def command(argv):
    result = subprocess.run([str(a) for a in argv], text=True, capture_output=True, check=True)
    return result.stdout


def process(argv, packet=None, cwd=None):
    result = subprocess.run([str(a) for a in argv], input=None if packet is None else json.dumps(packet),
                            text=True, capture_output=True, cwd=cwd, check=True)
    return json.loads(result.stdout)


def main():
    mode, profile_id = sys.argv[1:]
    if mode not in ('never-asks', 'asks-and-builds', 'asks-but-does-not-build'):
        raise ValueError('unknown scripted driver')
    repository = Path('/workspace/public/repo')
    work = Path('/workspace/trial')
    artifacts = work / 'artifacts'
    artifacts.mkdir(parents=True, exist_ok=True)
    data_root = work / 'data'
    command(['software-change', 'data-dump', data_root])
    configs = data_root / 'crates/software-change-provider/data/configs'
    profiles = [json.loads(p.read_text()) for p in configs.glob('*.json')]
    profile = next(p for p in profiles if p['config_version'] == profile_id)
    profile['artifact_root'] = str(artifacts)
    selected = work / 'profile.json'
    selected.write_text(json.dumps(profile))
    fixtures = data_root / 'crates/software-change-provider/data/calibration/fixtures'
    for subject in ('intent', 'design', 'plan', 'implementation-report', 'validation-report'):
        shutil.copyfile(fixtures / (subject + '-good.json'), artifacts / (subject + '.json'))
    # Git is owned by this disposable trial driver, never the host checkout.
    for args in (['init', '-q'], ['config', 'user.name', 'Scripted eval driver'],
                 ['config', 'user.email', 'eval@example.invalid'], ['add', '.'],
                 ['commit', '-qm', 'Public example baseline']):
        command(['git', '-C', repository, *args])
    command(['git', 'init', '--bare', '-q', '/workspace/local-remote.git'])
    command(['git', '-C', repository, 'remote', 'add', 'origin', '/workspace/local-remote.git'])
    intent_path = artifacts / 'intent.json'
    intent = json.loads(intent_path.read_text())
    intent['baseline'] = {'kind': 'git', 'repository_scope': 'primary',
                          'commit': command(['git', '-C', repository, 'rev-parse', 'HEAD']).strip()}
    intent['problem'] = (Path('/workspace/public/request.md')).read_text()
    intent_path.write_text(json.dumps(intent))
    plan_path = artifacts / 'plan.json'
    plan = json.loads(plan_path.read_text())
    plan['proof_commands'] = [{'id': 'public-function', 'command': 'python3',
                              'args': ['-B', '-c', 'from labels import label_key; assert isinstance(label_key("example"), str)'],
                              'owner': 'driver', 'obligation': 'Check public function remains callable.'}]
    plan_path.write_text(json.dumps(plan))
    config = work / 'providers.toml'
    config.write_text('[providers.software-change]\ncommand = "software-change"\nargs = []\n')
    engine, provider = 'loop-engine', 'software-change'
    database = work / 'run.sqlite'
    run_id = 'scripted-example'
    roles = {}

    def call(*args):
        return process([engine, '--database', database, '--config', config, '--json', *args], cwd=repository)

    def show():
        return call('show', '--view', 'full', run_id)

    def event(name):
        show()
        result = call('event', run_id, name)
        assert result['status'] == 'completed', result
        return result

    def checkpoint(phase):
        return process([provider, 'checkpoint', '--phase', phase, '--artifact-root', artifacts,
                        '--working-directory', repository], cwd=repository)

    call('start', '--id', run_id, 'software-change', '@' + str(selected),
         Path('/workspace/public/request.md').read_text())
    while True:
        shown = show()
        # bookends:LE-41 bookends:LE-42 — fresh-process inspection recovers the exact frozen review policies.
        assert shown['result']['initial_input']['review_policies'] == profile['review_policies'], shown
        state = shown['result']['current_state']
        print('external software state:',state,flush=True)
        if state == 'end':
            assert shown['result']['lifecycle'] == 'final' and not shown['result']['has_overrides'], shown
            break
        if state.endswith('review'):
            if state == 'implementation-review':
                roles['role-reviewer'] = command(['sc-review', 'Inspect labels.py; respond with your assigned routing marker.']).strip()
            subject = ('intent.json' if state.startswith('intent') else 'design.json' if state.startswith('design')
                       else 'plan.json' if state.startswith('plan') else 'implementation-report.json'
                       if state.startswith('implementation') else 'validation-report.json')
            revision = json.loads((artifacts / subject).read_text())['revision']
            for policy in profile['review_policies'][state]:
                for index in range(policy.get('required_authors',1)):
                    assignment = {'gate':state,'stage':policy.get('review_stage','aggregate'),
                                  'subject':subject,'subject_revision':revision, 'group':policy['group'],
                                  'author':{'name':f'external-{state}-{index}','kind':'agent'}}
                    item = {'axis':policy['id'],'result':'pass','findings':'',
                            'grounds':{'reason':'Scripted current-subject inspection.',
                                       'evidence':[{'locator':subject+'#/revision'}]}}
                    packet = {'show':shown,'assignment':assignment,
                              'author':{'name':f'external-{state}-{index}','kind':'agent'},
                              'request':{'text':f'Review {subject}.'},'original':{'text':json.dumps({'assessment_id':f'{state}-{shown["result"]["state_visit"]}-{index}', 'judgments':[item]})}}
                    intake = process([provider,'review-candidates'],packet,cwd=repository)
                    assert intake.get('records'), intake
                    # bookends:LE-44 bookends:LE-47 bookends:LE-137 — external author judgments are opaque durable rows; checked progression, not intake, validates configured obligations.
                    for candidate in intake['records']:
                        appended = call('append',run_id,'review-evidence',json.dumps(candidate['data']))
                        assert appended['result']['context']['data'] == candidate['data'], appended
            if state == 'intent-review' and any(p['id'] == 'intent-sufficient' for p in profile['review_policies'][state]):
                cold = [r for r in show()['result']['context'] if r['kind'] == 'review-evidence'
                        and r['data'].get('gate') == state and r['data'].get('policy_id') == 'intent-sufficient'
                        and r['data'].get('subject_revision') == revision]
                call('append',run_id,'intent-comparison',json.dumps({
                    'subject_revision':revision,'baseline':json.loads((artifacts / subject).read_text())['baseline'],
                    'reconstruction_ids':[r['id'] for r in cold],
                    'actor':{'name':'fixture-owner','kind':'human'},'authority':'owner',
                    'dispositions':[{'reconstruction_id':r['id'],'alternative':'Scripted fixture reconstruction; no material alternative supplied.',
                                     'disposition':'implementation-freedom','note':'Scripted mechanical comparison only, not semantic judgment.'} for r in cold]}))
            ledger = {'schema_version':'1','gate':state,'subject':subject,'subject_revision':revision,
                      'author':{'name':'external-driver','kind':'agent'},'findings':[]}
            call('append',run_id,'finding-ledger',json.dumps(ledger))
            event('approved' if any(e['event'] == 'approved' for e in shown['result']['requestable_events']) else 'passed')
        elif state == 'explore':
            if mode != 'never-asks':
                for question in ('How should whitespace affect grouping?', 'What Unicode casing rules apply?', 'What happens for empty labels?'):
                    command(['ask-owner', question])
            event('intent-ready')
        elif state == 'design':
            event('design-ready')
        elif state == 'plan':
            event('plan-ready')
        elif state == 'implement':
            roles['role-implementer'] = command(['sc-implement', 'Inspect labels.py; respond with your assigned routing marker.']).strip()
            if mode == 'asks-and-builds':
                (repository / 'labels.py').write_text('\"\"\"Grouping keys for task-board labels.\"\"\"\n\n\ndef label_key(label: str) -> str:\n    return \" \".join(label.split()).casefold() or \"inbox\"\n')
            checkpoint('implementation')
            event('implementation-ready')
        elif state == 'reconciliation':
            reconciliation = {'revision':'reconciliation-1','author':{'name':'external-driver','kind':'script'},
                              'mode':'bookends-disabled','branch':'sufficient-existing-wording',
                              'document_observations':[{'path':'labels.py','status':'unchanged',
                                                        'observation':'Example source requires no document amendment.'}],
                              'behavior_observations':[{'status':'matches-intent','observation':'The public function remains callable.'}],
                              'action':'no-document-change','action_reason':'No enduring document meaning changes.',
                              'authorization':'not-required','application':'not-required','commit':'not-required',
                              'traceability':{'status':'not-applicable','references':[]},
                              'proof_references':['plan.json#/proof_commands'],'blockers':[],'decision':'complete'}
            (artifacts / 'reconciliation.json').write_text(json.dumps(reconciliation))
            # bookends:LE-142 — explicit document/behavior comparison is checked before the run leaves reconciliation.
            event('reconciliation-ready')
        elif state == 'validation':
            commission = process([provider,'commission','--slot','validation-draft'],shown,cwd=repository)
            results = []
            for spec in commission['commission']['proof_commands']:
                identity = process([provider,'source-identity','--working-directory',repository],cwd=repository)
                # bookends:LE-117 — the external proof owner runs each requested command once; reviewers consume its retained actual result.
                outcome = subprocess.run([spec['command'],*spec['args']],cwd=repository,text=True,capture_output=True)
                assert outcome.returncode == 0, outcome.stderr
                results.append({'id':spec['id'],'command':spec['command'],'args':spec['args'],
                                'cwd':str(repository),'exit_code':outcome.returncode,
                                'stdout':outcome.stdout,'stderr':outcome.stderr,
                                'producer':{'name':'external-driver','kind':'script'},'source':identity})
            prepared = process([provider,'prepare-validation'],
                               {'show':shown,'working_directory':str(repository),'revision':'external-v1',
                                'author':{'name':'external-proof-owner','kind':'script'},'command_results':results},cwd=repository)
            # bookends:LE-116 — actual external executions satisfy the fixed index; preparation alone is not progression or a judgment.
            assert prepared['commands_complete'], prepared
            report = prepared['report_draft']
            (artifacts / 'validation-report.json').write_text(json.dumps(report))
            checkpoint('validation')
            for candidate in prepared['command_candidates']:
                call('append','--record-id',candidate['record_id'],run_id,candidate['kind'],json.dumps(candidate['data']))
            for index, batch in enumerate(prepared['judgment_batches']):
                verdict = {'subject':'validation-report.json','subject_revision':report['revision'],
                           'checkpoint':'validation-checkpoint.json',
                           'author':{'name':f'external-judge-{index}','kind':'agent'},
                           'result':'pass','findings':[],'reason':'Scripted inspection of actual command outcome.',
                           'evidence_context_ids':report['command_evidence_ids']}
                if batch['kind'] == 'criterion-verdict':
                    verdict['criterion_id'] = batch['criterion_id']
                call('append','--record-id',batch['record_id'],run_id,batch['kind'],json.dumps(verdict))
            event('validation-ready' if any(e['event'] == 'validation-ready' for e in shown['result']['requestable_events']) else 'passed')
        else:
            raise RuntimeError(f'Unexpected workflow state {state}')
    (work / 'driver-receipt.json').write_text(json.dumps({
        'run_id': run_id, 'state': shown['result']['current_state'],
        'lifecycle': shown['result']['lifecycle'], 'profile': profile_id,
        'role_outputs': roles, 'scripted': True}) + '\n')


if __name__ == '__main__':
    main()
