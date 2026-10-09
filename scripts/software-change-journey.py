#!/usr/bin/env python3
"""External-driver software-change journey using scripted judgments, not semantic proof."""
import argparse
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
def run(args, profile_source, root, *, configure=None, exercise=None, judgment_reason=None,
        stop_at=None, capture=False):
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
    environment = {key:value for key,value in os.environ.items()
                   if not key.startswith(('LOOP_', 'XDG_'))}
    captures = work / 'captures'
    if capture:
        captures.mkdir()
    sequence = 0
    def invoke(argv, packet=None, *, cwd=repository, allow_rejected=False):
        nonlocal sequence
        outcome = subprocess.run([str(arg) for arg in argv],
                                 input=None if packet is None else json.dumps(packet),
                                 text=True,capture_output=True,cwd=cwd,env=environment)
        if capture:
            sequence += 1
            (captures / f'{sequence:04d}.json').write_text(json.dumps({
                'argv':[str(arg) for arg in argv],'cwd':str(cwd),'input':packet,
                'exit_code':outcome.returncode,'stdout':outcome.stdout,'stderr':outcome.stderr},indent=2))
        response = json.loads(outcome.stdout)
        if outcome.returncode and not (allow_rejected and response.get('status') == 'rejected'):
            raise RuntimeError(f'{argv}: {outcome.returncode}: {outcome.stdout}\n{outcome.stderr}')
        return response
    def call(*args, allow_rejected=False):
        return invoke([engine,'--database',database,'--config',config,'--json',*args],
                      allow_rejected=allow_rejected)
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
        before = [subprocess.check_output(['git',*argv],cwd=repository) for argv in [['rev-parse','HEAD'],['status','--porcelain']]]
        result = invoke([provider,'checkpoint','--phase',phase,'--artifact-root',artifacts,
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
                                       'database':database,'config':config,'shown':shown,'invoke':invoke,'append':append})
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
            append_reviews({'call':call,'append':append,'invoke':invoke,'shown':shown,'profile':profile,
                            'repository':repository,'provider':provider,'run_id':run_id},
                           state,subject,revision,profile['review_policies'][state])
            if state == 'intent-review' and any(p['id'] == 'intent-sufficient' for p in profile['review_policies'][state]):
                cold = [r for r in show()['result']['context'] if r['kind'] == 'review-evidence'
                        and r['data'].get('gate') == state and r['data'].get('policy_id') == 'intent-sufficient'
                        and r['data'].get('subject_revision') == revision]
                append('intent-comparison',{
                    'subject_revision':revision,'baseline':json.loads((artifacts / subject).read_text())['baseline'],
                    'reconstruction_ids':[r['id'] for r in cold],
                    'actor':{'name':'fixture-owner','kind':'human'},'authority':'owner',
                    'dispositions':[{'reconstruction_id':r['id'],'alternative':'Scripted fixture reconstruction; no material alternative supplied.',
                                     'disposition':'implementation-freedom','note':'Scripted mechanical comparison only, not semantic judgment.'} for r in cold]})
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
                outcome = subprocess.run([spec['command'],*spec['args']],cwd=repository,text=True,capture_output=True)
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
            for index, batch in enumerate(prepared['judgment_batches']):
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
                                                'database':database,'config':config,'shown':show(),'report':report})
            event('validation-ready' if any(e['event'] == 'validation-ready' for e in shown['result']['requestable_events']) else 'passed')
        else:
            raise RuntimeError(f'Unexpected workflow state {state}')
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
                          'assessment_id':f'{gate}-{shown["result"]["state_visit"]}-{index}',
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
               stop_at='implement',capture=True)
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
        parallelism_journeys(args,root)
    print(f'software-change journey captures retained: {root}')

if __name__ == '__main__':
    main()
