"""One fresh v2 run: recovery composes to checked terminal completion.

Only public engine processes write catalogs. Fixture workers write product and
judgment artifacts; only the graph summarizer writes implementation reports.
Synthetic judgments establish mechanics, not model quality or owner acceptance.
"""
import copy
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time

from recovery_override import Fixture, software as exceptional
from recovery_cancellation import Fixture as CleanupFixture, process_table, wait_for


GRAPH_WORKER = r'''import json,os,sys,subprocess
from pathlib import Path
location, assignment=sys.stdin.read().split('\n---\n\n',1)
p=json.loads(location); root=Path(p['artifact_root']); repo=Path.cwd()
if 'plan_path' in p:
    previous=list((root/'summaries').glob('*.json')) if (root/'summaries').exists() else []
    revision=str(len(previous)+1)
    report={'revision':revision,'author':{'name':'implementer','kind':'script'},'plan_revision':'2',
            'coverage':{'commit':subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip()+'+uncommitted-worktree',
                        'documents':[{'path':n,'revision':json.loads((root/n).read_text())['revision']} for n in ['intent.json','design.json','plan.json']]},
            'summary':'scripted product change',
            'changed_surface':['product.txt','stable.txt'],'validation':['task-local product assertion']}
    (root/'summaries').mkdir(exist_ok=True)
    (root/'summaries'/('report-'+revision+'.json')).write_text(json.dumps(report))
    (root/'implementation-report.json').write_text(json.dumps(report))
    assert 'set-product:' not in assignment, 'task steering leaked to summarizer'
else:
    task=json.loads(assignment); name=task['id']
    if name=='B' and (root/'unaffected-task.py').exists():
        os.execv(sys.executable,[sys.executable,str(root/'unaffected-task.py')])
    if name=='A':
        instructions=[r['data']['instruction'] for r in task.get('steering_context',[])]
        selected=[s.split(':',1)[1] for s in instructions if s.startswith('set-product:')]
        value=selected[-1] if selected else 'broken'
        (repo/'product.txt').write_text(value+'\n')
        assert (repo/'product.txt').read_text()==value+'\n'
    else:
        (repo/'stable.txt').write_text('retained\n')
        assert (repo/'stable.txt').read_text()=='retained\n'
    with (root/'task-launches.jsonl').open('a') as f:f.write(json.dumps(task)+'\n')
print('focused worker completed',flush=True)
'''

REVIEW_WORKER = r'''import json,sys,subprocess
from pathlib import Path
p=json.loads(sys.stdin.read().split('---',1)[0]); root=Path(p['artifact_root'])
report=json.loads((root/'validation-report.json').read_text()); context={r['id']:r for r in p['context']}
repo=Path(sys.argv[1]); author={'name':'criterion-reviewer','kind':'script'}; rows=[]
correct=(repo/'product.txt').read_text()=='fixed\n'
for criterion in report['criteria']:
    id=criterion['verdict_ids'][0]; ac=criterion['criterion_id']
    if context.get(id,{}).get('kind')=='evidence-applicability':continue
    if ac=='AC-2' and (root/'unaffected-reviewer.py').exists():
        subprocess.run([sys.executable,str(root/'unaffected-reviewer.py')],check=True)
    ok=correct if ac=='AC-1' else (repo/'stable.txt').read_text()=='retained\n'
    value={'subject':'validation-report.json','subject_revision':report['revision'],'checkpoint':'validation-checkpoint.json',
           'author':author,'result':'pass' if ok else 'fail','findings':[] if ok else ['product is not fixed'],
           'evidence_context_ids':report['command_evidence_ids'],'criterion_id':ac}
    rows.append({'kind':'criterion-verdict','record_id':id,'data':value})
value={'subject':'validation-report.json','subject_revision':report['revision'],'checkpoint':'validation-checkpoint.json',
       'author':author,'result':'pass' if correct else 'fail','findings':[] if correct else ['product is not fixed'],
       'evidence_context_ids':report['command_evidence_ids']}
rows.append({'kind':'goal-verdict','record_id':report['goal_verdict_ids'][0],'data':value})
print(json.dumps({'author':author,'judgments':[{'axis':'delivery','result':'pass' if correct else 'fail',
    'findings':'' if correct else 'product is not fixed'}],'validation_verdicts':rows}))
'''

# A fresh process consumes only this small location packet plus public show and
# named artifacts. No in-memory fixture state or previous driver conversation.
RESUME_DRIVER = r'''import json,subprocess,sys
from pathlib import Path
p=json.load(open(sys.argv[1]))
def call(*args):
    r=subprocess.run([p['engine'],'--database',p['database'],'--json',*args],text=True,capture_output=True,check=True)
    return json.loads(r.stdout)
show=call('show',p['run_id']); s=show['result']; history=call('history',p['run_id'])
root=Path(s['initial_input']['artifact_root'])
files={n:json.loads((root/n).read_text()) for n in ['intent.json','plan.json','implementation-report.json','implementation-checkpoint.json']}
for n in ['validation-report.json','validation-checkpoint.json']:
    if (root/n).exists():files[n]=json.loads((root/n).read_text())
assert s['effective_bindings'] and s['binding_amendments']
assert s['initial_input']['contract_version']==2
assert s['current_state_instructions']
assert any(r['kind']=='finding-ledger' for r in s['context'])
assert any(i['status']=='failed' and i['ownership']['cancellation'] for i in s['work_slot_invocations'])
resumption=None
if s['current_state']=='validation-review':
    ledgers=[r for r in s['context'] if r['kind']=='finding-ledger' and r['data']['gate']=='validation-review']
    unresolved=[f for f in ledgers[-1]['data']['findings'] if f['disposition']=='accepted' and f['status']=='unresolved']
    assert unresolved and all(f['owner_phase']=='implementation' and f['task_ids']==['A'] for f in unresolved)
    assert any(e['event']=='revise-implementation' for e in s['requestable_events'])
    resumption=call('event',p['run_id'],'revise-implementation')
print(json.dumps({'show':show,'history':history,'artifacts':files,'remaining_events':s['requestable_events'],
                  'effective_settings':s['effective_bindings'],'exceptions':s['override_count'],'resumption':resumption}))
'''


def write(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def preserved(directory):
    return {str(p): hashlib.sha256(p.read_bytes()).hexdigest() for p in directory.rglob('*') if p.is_file()}


def assert_terminal(shown, history, initial, sentinels):
    assert shown['lifecycle'] == 'final' and shown['current_state'] == 'end'
    assert shown['completion_mode'] == 'completed' and not shown['has_overrides']
    assert shown['override_count'] == 0 and not shown['requestable_events']
    assert shown['initial_input'] == initial, 'frozen profile changed'
    assert all(not p.with_suffix('.invoked').exists() for p in sentinels)
    assert all(not i['ownership']['live_owned_work'] and not i['ownership']['cleanup_pending']
               for i in shown['work_slot_invocations'])
    outcomes = [r['action'].get('outcome', {}).get('outcome') for r in history]
    assert 'overridden' not in outcomes and 'denied' in outcomes
    assert any(r['kind'] == 'criterion-verdict' and r['data']['result'] == 'fail' for r in shown['context'])


def prove(journey):
    journey.work_root.mkdir(parents=True, exist_ok=True)
    root = Path(tempfile.mkdtemp(prefix='recovery-composed-', dir=journey.work_root))
    print(f'composed recovery captures: {root}', flush=True)
    external_log = []

    def external(argv, value=None, expected=0, cwd=None):
        p = subprocess.run([str(a) for a in argv], input=None if value is None else json.dumps(value),
                           text=True, capture_output=True, cwd=cwd, timeout=1200)
        external_log.append({'argv': [str(a) for a in argv], 'cwd': str(cwd or Path.cwd()),
                             'exit': p.returncode, 'stdout': p.stdout, 'stderr': p.stderr})
        write(root/'external.json', external_log)
        assert p.returncode == expected, external_log[-1]
        return json.loads(p.stdout) if p.stdout.strip().startswith('{') else p.stdout

    repo = root/'repository'
    repo.mkdir()
    # Git identity is local to this disposable product fixture, never checkout Git.
    for args in (['init','-q'], ['config','user.email','fixture@example.invalid'],
                 ['config','user.name','fixture']):
        external(['git','-C',repo,*args])
    (repo/'product.txt').write_text('unimplemented\n')
    external(['git','-C',repo,'add','.'])
    external(['git','-C',repo,'commit','-qm','isolated product fixture'])
    f = Fixture(journey.engine, journey.provider, repo, root, 'composed-normal')
    sentinels = []
    captures = {}
    snapshots = []
    author = {'name':'implementer','kind':'script'}
    config_version = 'composed-recovery-2'
    schema = {'type':'object','required':['revision','author'],'properties':{
        'revision':{'type':'string','minLength':1},'author':{'type':'object','required':['name','kind'],
        'properties':{'name':{'type':'string'},'kind':{'type':'string','enum':['script','agent','human']}}}}}
    graph_worker = root/'graph-worker.py'; graph_worker.write_text(GRAPH_WORKER)
    waiting = root/'waiting-worker.py'
    waiting.write_text("import json,os,time\nfrom pathlib import Path\np=json.load(__import__('sys').stdin)\nr=Path(p['artifact_root'])\n(r/'waiting.pid').write_text(str(os.getpid()))\nprint('cancelled unfinished product work',flush=True)\nwhile True:time.sleep(.05)\n")
    filter_binding = {'command':str(journey.provider),'args':['commission']}
    initial_binding = {'command':sys.executable,'args':[str(waiting)],'context_filter':filter_binding}
    graph_binding = {'command':str(journey.provider),'args':['run-plan-graph','--working-directory',str(repo),
        '--max-active','1','--task-worker',json.dumps({'command':sys.executable,'args':[str(graph_worker)]})],
        'context_filter':filter_binding}
    profile = {'contract_version':2,'criterion_policy':{'required_authors':1,'goal_required_authors':1},
        'config_version':config_version,'artifact_root':str(f.artifacts),
        'artifact_schemas':{**{name:copy.deepcopy(schema) for name in ['intent.json','design.json','plan.json','implementation-report.json']},
            'validation-report.json':json.loads((journey.data_root/'crates/software-change-provider/data/validation-report-schema.json').read_text())},
        'review_policies':{gate:[{'id':'delivery','description':'scripted independent product judgment','required_authors':1}]
                           for gate in ['implementation-review','validation-review']},
        'work_slot_bindings':{'implement':initial_binding}}
    f.start(profile)
    initial = f.show()['initial_input']
    plan = {'revision':'1','author':author,'tasks':[{'id':'A'},{'id':'B'}],'dependency_graph':[],
            'proof_commands':[{'id':'product','owner':'driver','command':sys.executable,
                'args':['-c',"from pathlib import Path; print(Path('product.txt').read_text()); assert Path('stable.txt').read_text()=='retained\\n'"],
                'obligation':'retain product output and verify unaffected product'}]}
    for name, value in {'intent.json':{'revision':'1','author':author,'acceptance':[
            {'id':'AC-1','statement':'product is fixed'},{'id':'AC-2','statement':'stable product and command evidence remain intact'}]},
            'design.json':{'revision':'1','author':author},'plan.json':plan}.items():write(f.artifacts/name,value)

    def event(name, status='completed', needle=None):
        f.show(); value=f.result(['event',f.name,name],status)
        if needle:assert needle in json.dumps(value),value
        return value

    def wait(invocation):
        def finished():
            return next(i for i in f.show()['work_slot_invocations'] if i['invocation_id']==invocation['invocation_id'])
        wait_for(lambda: finished()['completed_at'] is not None, 'composed invocation completion', seconds=120)
        row=finished(); assert row['status']=='succeeded',row
        assert not row['ownership']['live_owned_work'] and not row['ownership']['cleanup_pending'],row
        captures.update(preserved(Path(row['capture_dir'])))
        return row

    def amend(slot, binding):
        request={'state_visit':f.show()['state_visit'],'owner':'fixture-owner','reason':'future execution correction, same obligations','binding':binding}
        f.result(['amend-binding',f.name,slot,json.dumps(request)])

    def ledger(gate, revision, findings):
        f.append('finding-ledger','ledger-'+str(len(f.transcript)),{'schema_version':'1','gate':gate,
            'subject':'implementation-report.json' if gate=='implementation-review' else 'validation-report.json',
            'subject_revision':revision,'author':{'name':'driver','kind':'agent'},'findings':copy.deepcopy(findings)})

    def finding(id, source, statement, policy='delivery', rejected=False):
        return {'id':id,'source':{'kind':'context-record','id':source},'policy_id':policy,'statement':statement,
                'disposition':'rejected' if rejected else 'accepted','reason':'explicit driver triage of this exact source',
                'owner_phase':None if rejected else 'implementation','task_ids':[] if rejected else ['A'],
                'review_axes':[],'status':'recorded' if rejected else 'unresolved'}

    def review(id, revision, reviewer, mode='product'):
        # Real external deterministic reviewer observes the product, not a prefilled pass.
        if reviewer=='unaffected-reviewer' and (f.artifacts/'unaffected-implementation-reviewer.py').exists():
            external([sys.executable,f.artifacts/'unaffected-implementation-reviewer.py'])
        output=external([sys.executable,'-c',
            "import json,pathlib,sys; value=pathlib.Path('product.txt').read_text(); "
            "ok=value!='broken\\n' if sys.argv[1]=='product' else not value.endswith('\\n'); "
            "print(json.dumps({'result':'pass' if ok else 'fail','findings':'' if ok else ('product is broken' if sys.argv[1]=='product' else 'trailing newline')}))",mode],cwd=repo)
        f.append('review-evidence',id,{'gate':'implementation-review','policy_id':'delivery','subject':'implementation-report.json',
            'subject_revision':revision,'config_version':config_version,'author':{'name':reviewer,'kind':'script'},**output})
        return output

    def sentinel(name):
        path=f.artifacts/(name+'.py')
        path.write_text("from pathlib import Path\nPath(__file__).with_suffix('.invoked').touch()\nraise SystemExit('unaffected work MUST NOT RUN')\n")
        sentinels.append(path)

    def graph(selected=False):
        args=['invoke',f.name,'implement','--controls',json.dumps({'max_active':1}), '--timeout-ms','120000']
        if selected:args += ['--input',json.dumps({'plan_revision':'2','task_roots':['A']})]
        preview=f.result([*args,'--preview'])
        assert preview['binding']==graph_binding and preview['controls']['max_active']==1
        f.show(); row=wait(f.result(args))
        summary=json.loads((Path(row['capture_dir'])/'summary.json').read_text())
        write(root/('graph-'+str(len(snapshots))+'.json'),summary)
        snapshot={n:json.loads((f.artifacts/n).read_text()) for n in ['implementation-report.json','implementation-checkpoint.json']}
        snapshots.append(snapshot)
        assert snapshot['implementation-checkpoint.json']['repository']['head']==external(['git','-C',repo,'rev-parse','HEAD']).strip()
        return snapshot['implementation-report.json']['revision']

    def steering(id, value, supersedes=None):
        data={'target':{'kind':'tasks','plan_revision':'2','ids':['A']},'instruction':'set-product:'+value}
        if supersedes:data['supersedes']=[supersedes]
        f.append('user-steering',id,data)
        shown={'status':'completed','operation':'show','result':f.show()}
        commission=external([journey.provider,'commission','--slot','implement','--task','A'],shown)
        assert id in commission['commission']['steering_ids']
        other=external([journey.provider,'commission','--slot','implement','--task','B'],shown)
        assert id not in other['commission']['steering_ids']

    def resume(label):
        packet={'engine':str(journey.engine),'database':str(f.db),'run_id':f.name}
        write(root/'resume-location.json',packet)
        script=root/'fresh-driver.py'; script.write_text(RESUME_DRIVER)
        result=external([sys.executable,script,root/'resume-location.json'])
        write(root/('fresh-driver-'+label+'.json'),result)
        return result

    try:
        for edge in ['intent-ready','design-ready','plan-ready']:event(edge)
        # Cancel rejected work before any report, then choose the true owning phase.
        f.show(); cancelled=f.result(['invoke',f.name,'implement'])
        wait_for(lambda:(f.artifacts/'waiting.pid').exists(),'cancel target started')
        event('revise-plan','rejected','live-owned-work')
        f.result(['invoke',f.name,'implement'],'rejected')
        began=time.monotonic()
        cancellation=f.result(['cancel-invocation',f.name,cancelled['invocation_id']])
        assert time.monotonic()-began<10 and cancellation['attempt']['elapsed_ms']<10000
        assert cancellation['attempt']['cleanup']['verified_no_survivors']
        assert int((f.artifacts/'waiting.pid').read_text()) not in process_table()
        assert not (f.artifacts/'implementation-report.json').exists()
        captures.update(preserved(Path(cancelled['capture_dir'])))
        partial=Path(cancelled['capture_dir'])/'ownership/stdout'
        assert 'cancelled unfinished product work' in partial.read_text()
        event('revise-plan')
        assert f.show()['current_state']=='plan'
        plan['revision']='2'; plan['tasks'][0]['objective']='repair only the product; preserve B'
        write(f.artifacts/'plan.json',plan)
        event('plan-ready')
        # Malformed/stale/unknown public amendments must not change history.
        before=f.history()
        request={'state_visit':f.show()['state_visit'],'owner':'owner','reason':'correct binding','binding':graph_binding}
        f.result(['amend-binding',f.name,'implement',json.dumps({**request,'policy':{}})],'invalid-invocation')
        f.result(['amend-binding',f.name,'missing',json.dumps(request)],'rejected')
        f.result(['amend-binding',f.name,'implement',json.dumps({**request,'state_visit':0})],'error')
        assert f.history()==before
        amend('implement',graph_binding)
        revision=graph()
        assert (repo/'product.txt').read_text()=='broken\n'
        event('implementation-ready')
        review('product-fail',revision,'product-reviewer')
        review('newline-fail',revision,'unaffected-reviewer','newline')
        findings=[finding('F-product','product-fail','product is broken'),
                  finding('F-newline','newline-fail','trailing newline',rejected=True)]
        ledger('implementation-review',revision,[])
        event('approved','rejected','product-fail')
        ledger('implementation-review',revision,findings)
        event('approved','rejected','accepted_unresolved')
        resume('ordinary-failure')
        event('revise')
        sentinel('unaffected-task')
        sentinel('unaffected-implementation-reviewer')
        steering('focused-first','almost')
        plan_bytes=(f.artifacts/'plan.json').read_bytes()
        revision=graph(True)
        assert (repo/'product.txt').read_text()=='almost\n'
        findings[0].update(status='resolved',reason='task A removed the ordinary broken output; focused assertion retained')
        event('implementation-ready')
        review('product-repaired',revision,'product-reviewer')
        ledger('implementation-review',revision,findings)
        event('approved')
        # Actual named commands freeze an index; one bound commission produces
        # criterion/goal and axis judgments, including a real underlying fail.
        reviewer=root/'review-worker.py'; reviewer.write_text(REVIEW_WORKER)
        output_schema=json.loads((journey.data_root/'crates/software-change-provider/data/review-worker-output-schema.json').read_text())
        output_schema['properties']['author']['const']={'name':'criterion-reviewer','kind':'script'}
        judgments=output_schema['properties']['judgments']
        judgments.update(minItems=1,maxItems=1,allOf=[{'contains':{'type':'object','required':['axis'],'properties':{'axis':{'const':'delivery'}}}}])
        for variant in judgments['items']['oneOf']:variant['properties']['axis']['enum']=['delivery']
        output_schema['properties']['validation_verdicts']={'type':'array','items':{'type':'object'}}
        review_binding={'command':str(journey.engine),'args':['fan-out','--worker',json.dumps({'command':sys.executable,
            'args':[str(reviewer),str(repo)],'full_output_schema':output_schema})],'context_filter':filter_binding}
        amend('validation-review',review_binding)

        def validation(revision, carry=None):
            result=external([journey.provider,'run-validation','--engine',journey.engine,'--working-directory',repo,'--revision',revision],
                            {'status':'completed','operation':'show','result':f.show()})
            assert result['commands_passed']
            for row in result['command_candidates']:f.append(row['kind'],row['record_id'],row['data'])
            report=result['report']
            if carry:report['criteria'][1]['verdict_ids']=['carry-ac2']; write(f.artifacts/'validation-report.json',report)
            external([journey.provider,'checkpoint','--phase','validation','--artifact-root',f.artifacts,'--working-directory',repo])
            write(root/('validation-'+revision+'.json'),report)
            write(root/('checkpoint-'+revision+'.json'),json.loads((f.artifacts/'validation-checkpoint.json').read_text()))
            if carry:
                f.append('criterion-revalidation','affected-ac1',{'subject_revision':revision,'affected_criteria':['AC-1'],
                    'change_kind':'material','reason':'task A repairs product only'})
                f.append('evidence-applicability','carry-ac2',{'origin':{'kind':'context-record','id':carry['criteria'][1]['verdict_ids'][0]},
                    'target':{'subject':'validation-report.json','subject_revision':revision,'checkpoint':{'phase':'validation','report_revision':revision}},
                    'attesting_driver':{'name':'driver','kind':'agent'},'reason':'B and its retained command evidence mechanics unchanged'})
                ledger('validation-review',revision,criterion_findings)
            event('validation-ready')
            before=preserved(f.artifacts) # compare only immutable index/checkpoint below
            f.show(); row=wait(f.result(['invoke',f.name,'validation-review']))
            candidates=external([journey.provider,'review-candidates'],{'status':'completed','operation':'show','result':f.show()})['candidates']
            current=[c for c in candidates if c['origin']['id']==row['invocation_id']]
            assert len(current)==(3 if carry else 4),candidates
            for historical in [c for c in candidates if c not in current]:
                assert carry and historical['status']=='malformed' and 'stale' in historical['diagnostic'],historical
            for candidate in current:
                if candidate['status']=='verdict-ready':
                    f.append(candidate['kind'],candidate['record_id'],{**candidate['data'],'origin':candidate['origin']})
                else:
                    assert candidate['status']=='ready',candidate
                    f.append('review-evidence','validation-axis-'+revision,{'gate':'validation-review','policy_id':'delivery','subject':'validation-report.json',
                        'subject_revision':revision,'config_version':config_version,'author':candidate['author'],
                        'result':candidate['result'],'findings':candidate['findings'],'origin':candidate['origin']})
            for name in ['validation-report.json','validation-checkpoint.json']:
                assert preserved(f.artifacts)[str(f.artifacts/name)]==before[str(f.artifacts/name)]
            return report,row

        old,failed_review=validation('v1')
        ledger('validation-review','v1',[])
        event('passed','rejected')
        criterion_findings=[finding('F-criterion',old['criteria'][0]['verdict_ids'][0],'product is not fixed','AC-1'),
            finding('F-goal',old['goal_verdict_ids'][0],'product is not fixed','goal'),
            finding('F-validation-axis','validation-axis-v1','product is not fixed')]
        ledger('validation-review','v1',criterion_findings)
        event('passed','rejected','finding ledger')
        fresh=resume('criterion-failure')
        assert fresh['show']['result']['current_state']=='validation-review' and fresh['remaining_events']
        assert fresh['resumption']['status']=='completed' and f.show()['current_state']=='implement'
        sentinel('unaffected-reviewer')
        steering('focused-final','fixed','focused-first')
        revision=graph(True)
        assert (repo/'product.txt').read_text()=='fixed\n'
        assert (f.artifacts/'plan.json').read_bytes()==plan_bytes
        for item in criterion_findings:item.update(status='resolved',reason='task A fixed product; fresh criterion and goal proof follow')
        event('implementation-ready')
        review('product-final',revision,'product-reviewer')
        ledger('implementation-review',revision,findings)
        event('approved')
        new,_=validation('v2',old)
        ledger('validation-review','v2',criterion_findings)
        projection=external([journey.provider,'commission','--slot','validation-review'],{'status':'completed','result':f.show()})
        assert [r['mode'] for r in projection['validation_collection']]==['fresh','carried','fresh'],projection
        assert projection['validation_collection'][1]['source']['id']==old['criteria'][1]['verdict_ids'][0]
        event('passed')
        terminal=f.show(); history=f.history()['result']
        assert_terminal(terminal,history,initial,sentinels)
        accepted_proofs=list((f.artifacts/'implementation-proof-history').glob('*.json'))
        assert accepted_proofs
        final_checkpoint=json.loads((f.artifacts/'validation-checkpoint.json').read_text())
        assert final_checkpoint['repository']==snapshots[-1]['implementation-checkpoint.json']['repository']
        assert all(hashlib.sha256(Path(path).read_bytes()).hexdigest()==digest for path,digest in captures.items())
        launches=[json.loads(line) for line in (f.artifacts/'task-launches.jsonl').read_text().splitlines()]
        assert [t['id'] for t in launches].count('B')==1 and [t['id'] for t in launches].count('A')==3
        assert len(terminal['binding_amendments'])==2
        assert terminal['work_slot_invocations'][0]['binding']==initial_binding
        assert terminal['effective_bindings']['implement']==graph_binding
        # Match every durable transition to its actual public command envelope,
        # including the fresh driver's resumed event. No injected transition or
        # replacement run can satisfy this equality; no override is admitted.
        public_actions=[]
        for command in f.transcript:
            assert command['argv'][:4]==[str(journey.engine),'--database',str(f.db),'--json']
            assert '--override' not in command['argv']
            envelope=json.loads(command['stdout'])
            action=envelope.get('result',{}).get('history',{}).get('action') if isinstance(envelope.get('result'),dict) else None
            if action and action['kind']=='transition':public_actions.append(action)
        for command in external_log:
            if command['argv'][0]==sys.executable and command['argv'][1].endswith('fresh-driver.py'):
                resumed=json.loads(command['stdout']).get('resumption')
                if resumed:public_actions.append(resumed['result']['history']['action'])
        committed=[r['action'] for r in history if r['action']['kind']=='transition' and r['action']['outcome']['outcome']=='committed']
        # Compare multisets: fresh-driver captures are logged separately.
        assert sorted(map(lambda v:json.dumps(v,sort_keys=True),public_actions))==sorted(map(lambda v:json.dumps(v,sort_keys=True),committed))
        assert sum(r['action']['kind']=='run_created' for r in history)==1
        assert all(r['action']['kind'] in {'run_created','invocation_started','transition','binding_amended','context_appended','invocation_status_changed'} for r in history)
        records={r['id']:r for r in terminal['context']}
        assert all(records[id]['data']['result']=='fail' for id in ['product-fail','newline-fail','validation-axis-v1',old['criteria'][0]['verdict_ids'][0]])
        external_judgments=[json.loads(c['stdout']) for c in external_log if c['argv'][:2]==[sys.executable,'-c'] and c['stdout'].startswith('{')]
        for id in ['product-fail','newline-fail','product-repaired','product-final']:
            value=records[id]['data']
            assert {'result':value['result'],'findings':value['findings']} in external_judgments
        for record in records.values():
            if record['kind'] in ('criterion-verdict','goal-verdict'):
                assert record['data']['origin']['kind']=='selected-assignment-output'
        # The earlier command output still says almost; repair never rewrites it.
        old_command=records[old['command_evidence_ids'][0]]['data']
        old_summary=json.loads(Path(old_command['capture']['summary']).read_text())
        assert 'almost' in Path(old_summary['workers'][0]['stdout_path']).read_text()
        # All ledger snapshots after acceptance retain every accepted source;
        # no bootstrap current-blocker-only projection ever erases old findings.
        accepted={}
        for r in terminal['context']:
            if r['kind']!='finding-ledger':continue
            gate=r['data']['gate']; prior=accepted.setdefault(gate,set())
            entries=r['data']['findings']; ids={e['id'] for e in entries}
            assert prior<=ids, (gate,prior,ids)
            prior.update(e['id'] for e in entries if e['disposition']=='accepted')
        terminal_resume=resume('terminal')
        assert terminal_resume['show']['result']['completion_mode']=='completed'
        write(root/'normal-terminal.json',{'status':'completed','operation':'show','result':terminal})
        write(root/'normal-history.json',history)
        write(root/'validation-collection.json',projection['validation_collection'])
        write(root/'implementation-snapshots.json',snapshots)
        # Deliberately broken required observations must fail the same final
        # assertion, with actual terminal/history/captures remaining untouched.
        negative=[]
        for label, mutate in [('missing-terminal',lambda s:s.update(lifecycle='active')),
                              ('false-normal',lambda s:s.update(completion_mode='completed-with-overrides')),
                              ('lost-failure',lambda s:s.update(context=[r for r in s['context'] if r['kind']!='criterion-verdict']))]:
            damaged=copy.deepcopy(terminal); mutate(damaged)
            try:assert_terminal(damaged,history,initial,sentinels)
            except AssertionError:negative.append(label)
            else:raise AssertionError('broken terminal observation passed: '+label)
        assert preserved(Path(failed_review['capture_dir'])).items()<=captures.items()
        write(root/'negative-observations.json',negative)
    except BaseException:
        # Failure cleanup never contributes to a passing cancellation receipt.
        CleanupFixture.emergency_cleanup(f)
        raise

    exception_root=exceptional(journey.engine,journey.provider,journey.engine.parent/'bookends-check',repo,root)
    exception_proof=json.loads((exception_root/'proof.json').read_text())
    assert exception_proof['summary']['completion_mode']=='completed-with-overrides'
    assert exception_proof['failed_review_retained'] and exception_proof['missing_artifacts_retained']
    exceptional_transcript=json.loads((exception_root/'transcript.json').read_text())
    exceptional_shows=[json.loads(r['stdout']) for r in exceptional_transcript
                       if '--json' in r['argv'] and 'show' in r['argv']]
    exceptional_terminal=exceptional_shows[-1]
    assert exceptional_terminal['result']['completion_mode']=='completed-with-overrides'
    write(root/'exceptional-terminal.json',exceptional_terminal)
    proof={'status':'passed','scenario':'composed-recovery','normal':{'run_id':f.name,'database':str(f.db),'artifacts':str(f.artifacts),
           'terminal':str(root/'normal-terminal.json'),'history':str(root/'normal-history.json')},
           'exceptional':exception_proof,'exceptional_root':str(exception_root),
           'sentinels':[str(p) for p in sentinels],'preserved_capture_files':captures,
           'observables':{'AC-11':['normal-terminal.json','normal-history.json','fresh-driver-criterion-failure.json',
               'validation-collection.json','negative-observations.json','external.json'],
               'AC-12':['fresh-driver-terminal.json',str(exception_root/'proof.json'),'implementation-snapshots.json']},
           'scope':'synthetic mechanics only; no production edits, injected state, fake reviewer success, bootstrap ledger projections or off-engine completion'}
    write(root/'proof.json',proof)
    print(f'composed recovery journey passed: normal completed; exceptional completed-with-overrides; {root / "proof.json"}',flush=True)
    return proof
