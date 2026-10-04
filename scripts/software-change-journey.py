#!/usr/bin/env python3
"""External-driver software-change journey using scripted judgments, not semantic proof."""
import argparse
import json
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from external_work_driver import process

ROOT = Path(__file__).resolve().parents[1]
SUBJECTS = {'intent.json':'intent-good.json', 'design.json':'design-good.json',
            'plan.json':'plan-good.json', 'implementation-report.json':'implementation-report-good.json',
            'validation-report.json':'validation-report-good.json'}


def self_test():
    paths = [ROOT / 'AGENTS.md', ROOT / 'skills/using-loop-engine/SKILL.md']
    paths += list((ROOT / 'crates').glob('*/skills/*/SKILL.md'))
    retired = re.compile(r'loop-engine\s+(?:invoke|fan-out|preview-bindings|amend-binding|capture-command|capture-matrix)\b|work_slot_bindings')
    violations = [str(path.relative_to(ROOT)) for path in paths if retired.search(path.read_text())]
    if violations:
        raise RuntimeError('external-work skill/root policy assertions failed: ' + ', '.join(violations))
    print('external-work skill/root policy assertions passed')


# bookends:LE-39 bookends:LE-40 bookends:LE-43 bookends:LE-53 bookends:LE-85 — externally driven fixture artifacts complete under selected, materially different policies without invocation receipts.
def run(args, profile_source, root, *, configure=None, exercise=None, judgment_reason=None):
    engine, provider = Path(args.engine).resolve(), Path(args.provider).resolve()
    work = Path(tempfile.mkdtemp(prefix='software-external-', dir=root))
    artifacts = work / 'artifacts'
    artifacts.mkdir()
    repository = work / 'repository'
    repository.mkdir()
    for command in [['git','init','-q'],['git','config','user.name','Journey'],
                    ['git','config','user.email','journey@example.invalid']]:
        subprocess.run(command,cwd=repository,check=True,capture_output=True)
    (repository / 'helper.py').write_text('def helper(value):\n    return value\n')
    subprocess.run(['git','add','helper.py'],cwd=repository,check=True)
    subprocess.run(['git','commit','-qm','fixture'],cwd=repository,check=True)
    data_root = Path(args.data_root or ROOT).resolve()
    if args.mode == 'packaged':
        if not args.data_root or (data_root.exists() and any(data_root.iterdir())):
            raise ValueError('Packaged mode requires an empty --data-root.')
        subprocess.run([str(provider),'data-dump',str(data_root)],check=True,capture_output=True)
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
                          'commit':subprocess.check_output(['git','rev-parse','HEAD'],cwd=repository,text=True).strip()}
    intent_path.write_text(json.dumps(intent))
    if configure:
        configure(profile, repository, artifacts)
    selected.write_text(json.dumps(profile))
    config = work / 'providers.toml'
    config.write_text(f'[providers.software-change]\ncommand = {json.dumps(str(provider))}\nargs = []\n')
    database = work / 'run.sqlite'
    run_id = 'software-external'
    def call(*args):
        return process([engine,'--database',database,'--config',config,'--json',*args],cwd=repository)
    def show():
        return call('show','--view','full',run_id)
    def event(name):
        show()
        response = call('event',run_id,name)
        assert response['status'] == 'completed', response
        return response
    def checkpoint(phase):
        before = [subprocess.check_output(['git',*argv],cwd=repository) for argv in [['rev-parse','HEAD'],['status','--porcelain']]]
        result = process([provider,'checkpoint','--phase',phase,'--artifact-root',artifacts,
                         '--working-directory',repository],cwd=repository)
        after = [subprocess.check_output(['git',*argv],cwd=repository) for argv in [['rev-parse','HEAD'],['status','--porcelain']]]
        assert before == after, 'checkpoint mutated Git state'
        return result
    call('start','--id',run_id,'software-change','@'+str(selected),'external software')
    while True:
        shown = show()
        # bookends:LE-41 bookends:LE-42 — fresh-process inspection recovers the exact frozen review policies.
        assert shown['result']['initial_input']['review_policies'] == profile['review_policies'], shown
        state = shown['result']['current_state']
        print('external software state:',state,flush=True)
        if exercise:
            handled = exercise(state, {'call':call,'show':show,'event':event,'checkpoint':checkpoint,
                                       'profile':profile,'artifacts':artifacts,'repository':repository,
                                       'provider':provider,'engine':engine,'work':work,'run_id':run_id,
                                       'database':database,'config':config,'shown':shown})
            if handled:
                continue
            shown = show()
        if state == 'end':
            assert shown['result']['lifecycle'] == 'final' and not shown['result']['has_overrides'], shown
            break
        if state.endswith('review'):
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
                           'result':'pass','findings':[],'reason':judgment_reason or 'Inspected actual driver command outcome.',
                           'evidence_context_ids':report['command_evidence_ids']}
                if batch['kind'] == 'criterion-verdict':
                    verdict['criterion_id'] = batch['criterion_id']
                call('append','--record-id',batch['record_id'],run_id,batch['kind'],json.dumps(verdict))
            if exercise:
                exercise('validation-prepared', {'call':call,'show':show,'event':event,'checkpoint':checkpoint,
                                                'profile':profile,'artifacts':artifacts,'repository':repository,
                                                'provider':provider,'engine':engine,'work':work,'run_id':run_id,
                                                'database':database,'config':config,'shown':show(),'report':report})
            event('validation-ready' if any(e['event'] == 'validation-ready' for e in shown['result']['requestable_events']) else 'passed')
        else:
            raise RuntimeError(f'Unexpected workflow state {state}')
    return work


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--mode',choices=['source','packaged'],default='source')
    parser.add_argument('--engine')
    parser.add_argument('--provider')
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
    root.mkdir(parents=True,exist_ok=True)
    run(args,args.profile,root)
    print('full software-change journey passed')
    if args.mode == 'source':
        run(args,ROOT / 'crates/software-change-provider/data/configs/minimal.json',root)
        print('stitched software-change journey passed')

if __name__ == '__main__':
    main()
