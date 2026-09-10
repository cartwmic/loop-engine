import json,pathlib,subprocess,time,hashlib,os,sys
# Executed only by operational-ux-journey with its explicit fixture arguments.
from test_contract import ROOT as C
P=ux_args.released_root/'bin'
F=ux_args.attempt;repo=F/'repository';repo.mkdir()
E=os.environ.copy()
commands=[]
def run(argv,name,input=None,cwd=repo,expected=0):
 t=time.time();p=subprocess.run(list(map(str,argv)),cwd=cwd,input=b'' if input is None else json.dumps(input).encode(),capture_output=True,env=E)
 out=F/(name+'.stdout');err=F/(name+'.stderr');out.write_bytes(p.stdout);err.write_bytes(p.stderr)
 commands.append(dict(argv=list(map(str,argv)),cwd=str(cwd),exit=p.returncode,started_at=t,finished_at=time.time(),stdout=str(out),stderr=str(err)))
 (F/'commands.json').write_text(json.dumps(commands,indent=2));assert p.returncode==expected,commands[-1];return p
import shutil
(repo/'scripts').mkdir()
for name in ['assert-implementation-report.py','test_contract.py']:
 shutil.copyfile(C/'scripts'/name,repo/'scripts'/name)
E['PYTHONDONTWRITEBYTECODE']='1'
run(['git','init',repo],'init');(repo/'tracked.txt').write_text('fixture\n');run(['git','add','.'],'add');run(['git','-c','user.name=Fixture','-c','user.email=fixture@example.invalid','commit','-m','fixture'],'commit')
spec=dict(id='compatibility',command=sys.executable,args=['-c',"import sys;from pathlib import Path;p=Path("+repr(str(F/'launch-count'))+");p.write_text(str(int(p.read_text())+1) if p.exists() else '1');print('actual-proof-output');print('actual-proof-error',file=sys.stderr)"],owner='driver',obligation='actual retained execution')
instructions=F/'instructions.json';instructions.write_text(json.dumps(dict(artifact_root=str(F))))
worker=dict(command=str(ux_args.binary_dir/'software-change'),args=['validation-command',str(repo),json.dumps(spec,separators=(',',':')),'10000'])
p=run([P/'loop-engine','fan-out','--instructions',instructions,'--max-active','1','--worker',json.dumps(worker)],'fan-out',cwd=F)
summary_path=pathlib.Path(json.loads(p.stdout)['output_dir'])/'summary.json';summary=json.loads(summary_path.read_text());row=summary['workers'][0]
raw_path=pathlib.Path(row['stdout_path']);raw=raw_path.read_bytes();value=json.loads(raw)
assert row['selected_attempt']>0 and row['selected_output_sha256']=='sha256:'+hashlib.sha256(raw).hexdigest()
assert value['spec']==spec and value['cwd']==str(repo) and value['exit_code']==0
receipt_path=pathlib.Path(value['capture_receipt']);receipt=json.loads(receipt_path.read_text())
sys.path.insert(0,str(C/'scripts'));from test_contract import repository_proof_identity
assert receipt['repository_before']==receipt['repository_after']==repository_proof_identity(repo)
assert receipt['argv']==[spec['command']]+spec['args']
assert (receipt_path.parent/'stdout').read_text()==value['stdout']=='actual-proof-output\n'
assert (receipt_path.parent/'stderr').read_text()==value['stderr']=='actual-proof-error\n'
# Public report checker in the actual committed fixture checkout, unchanged code.
proof=F/'proof';proof.mkdir()
(proof/'receipts.json').write_text(json.dumps({'plan_revision':'1','target_directory':str(repo/'target'),'commands':[{'id':spec['id'],'receipt':str(receipt_path)}]}))
matrix=F/'matrix.json';matrix.write_text(json.dumps({'schema_version':1,'plan_revision':'1','local_final':[spec],'post_report':[],'after_separate_authorization':[]}))
head=run(['git','rev-parse','HEAD'],'head').stdout.decode().strip()
report_path=F/'checker-report.json';report_path.write_text(json.dumps({'revision':'1','plan_revision':'1','coverage':{'commit':head+'+uncommitted-worktree'},'changed_surface':[],'validation':[{'proof':spec['id']+': passed'}]}))
checker=[sys.executable,repo/'scripts/assert-implementation-report.py','--report',report_path,'--revision','1','--plan-revision','1','--matrix',matrix]
run(checker,'report-compatible')
art=F/'artifacts';art.mkdir()
author={'name':'implementer','kind':'script'}
schema={'type':'object','required':['revision','author'],'properties':{'revision':{'type':'string'},'author':{'type':'object'}}}
profile={'contract_version':2,'criterion_policy':{'required_authors':1,'goal_required_authors':1},'config_version':'fixture','artifact_root':str(F),'review_policies':{},'artifact_schemas':{n:schema for n in ['intent.json','design.json','plan.json','implementation-report.json','validation-report.json']}}
docs={'intent.json':{'revision':'1','author':author,'acceptance':[{'id':'AC-1','statement':'retained execution'}]},'design.json':{'revision':'1','author':author},'plan.json':{'revision':'1','author':author,'tasks':[],'proof_commands':[spec]},'implementation-report.json':{'revision':'1','author':author},'validation-report.json':{'revision':'1','author':{'name':'driver','kind':'script'},'implementation_revision':'1','command_evidence_ids':['cmd'],'criteria':[{'criterion_id':'AC-1','verdict_ids':['ac']}],'goal_verdict_ids':['goal']}}
for name,doc in docs.items():(F/name).write_text(json.dumps(doc))
run([P/'software-change','checkpoint','--phase','implementation','--artifact-root',F,'--working-directory',repo],'checkpoint-implementation')
run([P/'software-change','checkpoint','--phase','validation','--artifact-root',F,'--working-directory',repo],'checkpoint-validation')
workflow=json.loads(run([P/'software-change'],'describe',{'operation':'describe','initial_input':profile}).stdout)
impl_transition=next(t for t in workflow['transitions'] if t['event']=='implementation-ready')
impl_request={'operation':'evaluate','workflow':workflow,'initial_input':profile,'context':[],'transition':impl_transition,'prior_evaluations':[]}
impl_result=json.loads(run([P/'software-change'],'evaluate-implementation',impl_request).stdout)
assert impl_result['result']=='allow',impl_result
context=[]
def record(id,kind,data):context.append({'id':id,'kind':kind,'data':data,'created_at':1,'sequence':len(context)+1})
record('cmd','command-evidence',{'proof_id':spec['id'],'capture':{'summary':str(summary_path),'assignment_id':row['assignment_id']}})
for id,kind in [('ac','criterion-verdict'),('goal','goal-verdict')]:
 data={'subject':'validation-report.json','subject_revision':'1','checkpoint':'validation-checkpoint.json','author':{'name':'independent','kind':'script'},'result':'pass','findings':[],'evidence_context_ids':['cmd']}
 if id=='ac':data['criterion_id']='AC-1'
 record(id,kind,data)
transition=next(t for t in workflow['transitions'] if t['source']=='validation' and t['event']=='passed')
request={'operation':'evaluate','workflow':workflow,'initial_input':profile,'context':context,'transition':transition,'prior_evaluations':[]}
(F/'request.json').write_text(json.dumps(request,indent=2))
result=json.loads(run([P/'software-change'],'evaluate',request).stdout);assert result['result']=='allow',result
import copy,shutil
negative=[]
for mutation in ['spec','identity','digest','path','attempt']:
 dest=F/('mutated-'+mutation);shutil.copytree(summary_path.parent,dest)
 changed=json.loads((dest/'summary.json').read_text());w=changed['workers'][0]
 # Explicit corruption copy: paths are rebased only within this copied fixture.
 for key in ['stdout_path','stderr_path','selected_output_path']:
  old=pathlib.Path(w[key]);old=old if old.is_absolute() else summary_path.parent/old
  w[key]=str(dest/old.relative_to(summary_path.parent))
 output=pathlib.Path(w['stdout_path']);v=json.loads(output.read_text())
 if mutation=='spec':v['spec']['args']=['mutated']
 if mutation=='identity':v['repository_after']='sha256:'+'0'*64
 if mutation in ['spec','identity']:
  output.write_text(json.dumps(v));w['selected_output_sha256']='sha256:'+hashlib.sha256(output.read_bytes()).hexdigest()
 if mutation=='digest':w['selected_output_sha256']='sha256:'+'0'*64
 if mutation=='path':w['selected_output_path']=str(dest/'absent')
 if mutation=='attempt':w['selected_attempt']=0
 (dest/'summary.json').write_text(json.dumps(changed))
 altered=copy.deepcopy(request);altered['context'][0]['data']['capture']['summary']=str(dest/'summary.json')
 result=json.loads(run([P/'software-change'],'deny-'+mutation,altered).stdout)
 assert result['result']=='deny',result
 negative.append({'mutation':mutation,'result':result})
(F/'negative-outcomes.json').write_text(json.dumps(negative,indent=2))
# Candidate-only additive/preparation behavior, independently of frozen acceptance.
candidate=ux_args.binary_dir/'software-change'
candidate_workflow=json.loads(run([candidate],'candidate-describe',{'operation':'describe','initial_input':profile}).stdout)
show={'operation':'show','status':'completed','result':{'current_state':'validation','initial_input':profile,'context':[],'work_slots':candidate_workflow['work_slots']}}
prep={'show':show,'working_directory':str(repo),'revision':'candidate-1','author':{'name':'driver','kind':'script'},'capture_indexes':[value['capture_index']],'execution_settings':{'timeout_ms':10000},'additions':[]}
launch_count=(F/'launch-count').read_text()
first=json.loads(run([candidate,'prepare-validation'],'prepare-1',prep).stdout)
original={str(p):p.read_bytes() for p in pathlib.Path(value['capture_index']).parent.rglob('*') if p.is_file()}
second=json.loads(run([candidate,'prepare-validation'],'prepare-2',prep).stdout)
assert first==second and first['commands_complete']
assert original=={str(p):p.read_bytes() for p in pathlib.Path(value['capture_index']).parent.rglob('*') if p.is_file()}
assert (F/'launch-count').read_text()==launch_count
assert {b['kind'] for b in first['judgment_batches']}=={'criterion-verdict','goal-verdict'}
extra=copy.deepcopy(spec);extra['id']='extra'
extra_worker=copy.deepcopy(worker);extra_worker['args'][2]=json.dumps(extra)
p=run([P/'loop-engine','fan-out','--instructions',instructions,'--max-active','1','--worker',json.dumps(extra_worker)],'extra-fan-out',cwd=F)
extra_summary=json.loads((pathlib.Path(json.loads(p.stdout)['output_dir'])/'summary.json').read_text());extra_raw=json.loads(pathlib.Path(extra_summary['workers'][0]['stdout_path']).read_text())
prep['additions']=[extra];prep['capture_indexes'].append(extra_raw['capture_index'])
prepared=json.loads(run([candidate,'prepare-validation'],'prepare-addition',prep).stdout)
assert prepared['commands_complete'] and len(prepared['report_draft']['command_evidence_ids'])==2
for name,alter in [('duplicate',lambda p:p['additions'].append(extra)),('replacement',lambda p:p['additions'].append(spec))]:
 bad=copy.deepcopy(prep);alter(bad);run([candidate,'prepare-validation'],'prepare-deny-'+name,bad,expected=2)
missing=copy.deepcopy(prep);missing['capture_indexes']=[]
assert not json.loads(run([candidate,'prepare-validation'],'prepare-missing',missing).stdout)['commands_complete']
# Integrity specimens are copies, never altered retained originals.
for kind in ['missing-stream','corrupt-stream','stale-tree']:
 capture_root=pathlib.Path(value['capture_index']).parent
 dest=F/('capture-'+kind);shutil.copytree(capture_root,dest)
 idx=json.loads((dest/'index.json').read_text());old=pathlib.Path(idx['receipts'][0]['receipt']);new=dest/old.relative_to(capture_root);idx['receipts'][0]['receipt']=str(new)
 (dest/'index.json').write_text(json.dumps(idx))
 if kind=='missing-stream':(new.parent/'stdout').unlink()
 if kind=='corrupt-stream':(new.parent/'stdout').write_text('corruption')
 if kind=='stale-tree':
  finished=json.loads(new.read_text());finished['repository_after']='sha256:'+'0'*64;new.write_text(json.dumps(finished))
 bad=copy.deepcopy(prep);bad['capture_indexes']=[str(dest/'index.json'),extra_raw['capture_index']]
 result=json.loads(run([candidate,'prepare-validation'],'prepare-'+kind,bad).stdout);assert not result['commands_complete'],result
failed=copy.deepcopy(extra);failed['id']='failed';failed['args']=['-c','raise SystemExit(7)']
failed_worker=copy.deepcopy(worker);failed_worker['args'][2]=json.dumps(failed)
p=run([P/'loop-engine','fan-out','--instructions',instructions,'--max-active','1','--worker',json.dumps(failed_worker)],'failed-fan-out',cwd=F)
failed_summary=json.loads((pathlib.Path(json.loads(p.stdout)['output_dir'])/'summary.json').read_text());failed_raw=json.loads(pathlib.Path(failed_summary['workers'][0]['stdout_path']).read_text())
assert failed_raw['exit_code']==7
bad=copy.deepcopy(prep);bad['additions'].append(failed);bad['capture_indexes'].append(failed_raw['capture_index'])
assert not json.loads(run([candidate,'prepare-validation'],'prepare-failed',bad).stdout)['commands_complete']
for name,change in [('missing-index',lambda p:p['capture_indexes'].append(str(F/'absent-index.json'))),('stale-settings',lambda p:p['execution_settings'].update(timeout_ms=9999)),('duplicate-selection',lambda p:p['capture_indexes'].append(value['capture_index']))]:
 bad=copy.deepcopy(prep);change(bad)
 assert not json.loads(run([candidate,'prepare-validation'],'prepare-'+name,bad).stdout)['commands_complete']
(repo/'tracked.txt').write_text('changed tree\n')
try:
 assert not json.loads(run([candidate,'prepare-validation'],'prepare-actual-stale-tree',prep).stdout)['commands_complete']
finally:(repo/'tracked.txt').write_text('fixture\n')
report=prepared['report_draft'];(F/'validation-report.json').write_text(json.dumps(report))
run([candidate,'checkpoint','--phase','validation','--artifact-root',F,'--working-directory',repo],'candidate-checkpoint')
candidate_context=[]
for r in prepared['addition_candidates']+prepared['command_candidates']:
 candidate_context.append({'id':r['record_id'],'kind':r['kind'],'data':r['data'],'created_at':1,'sequence':len(candidate_context)+1})
for batch in prepared['judgment_batches']:
 data={'subject':'validation-report.json','subject_revision':report['revision'],'checkpoint':'validation-checkpoint.json','author':{'name':'independent','kind':'script'},'result':'pass','findings':[],'evidence_context_ids':report['command_evidence_ids']}
 if batch['kind']=='criterion-verdict':data['criterion_id']=batch['criterion_id']
 packet=F/(batch['record_id']+'.json');packet.write_text(json.dumps(data))
 scripted={'command':sys.executable,'args':['-c','import json,sys; print(json.dumps(json.load(sys.stdin)))']}
 output=run([P/'loop-engine','fan-out','--instructions',packet,'--max-active','1','--worker',json.dumps(scripted)],'author-'+batch['record_id'],cwd=F)
 selected=json.loads((pathlib.Path(json.loads(output.stdout)['output_dir'])/'summary.json').read_text())['workers'][0]
 author_bytes=pathlib.Path(selected['stdout_path']).read_bytes()
 assert selected['selected_output_sha256']=='sha256:'+hashlib.sha256(author_bytes).hexdigest() and selected['selected_attempt']>0
 actual=json.loads(author_bytes);assert actual==data
 candidate_context.append({'id':batch['record_id'],'kind':batch['kind'],'data':actual,'created_at':1,'sequence':len(candidate_context)+1})
candidate_request={'operation':'evaluate','workflow':candidate_workflow,'initial_input':profile,'context':candidate_context,'transition':next(t for t in candidate_workflow['transitions'] if t['source']=='validation' and t['event']=='passed'),'prior_evaluations':[]}
allowed=json.loads(run([candidate],'candidate-evaluate',candidate_request).stdout);assert allowed['result']=='allow',allowed
for name in ['self-author','implementation-author','missing-goal','missing-ac','duplicate-addition','replacement-addition']:
 bad=copy.deepcopy(candidate_request)
 if name in ['self-author','implementation-author']:
  for r in bad['context']:
   if r['kind'].endswith('verdict'):r['data']['author']=report['author'] if name=='self-author' else author
 elif name in ['duplicate-addition','replacement-addition']:
  duplicate=copy.deepcopy(bad['context'][0]);duplicate['id']='another-addition';duplicate['sequence']=len(bad['context'])+1
  if name=='replacement-addition':duplicate['data']=spec
  bad['context'].append(duplicate)
 else:bad['context']=[r for r in bad['context'] if r['kind']!=('goal-verdict' if name=='missing-goal' else 'criterion-verdict')]
 denied=json.loads(run([candidate],'candidate-deny-'+name,bad).stdout);assert denied['result']=='deny',denied

(F/'compatibility-outcome.json').write_text(json.dumps({'result':'allow','summary':str(summary_path),'raw':str(raw_path),'receipt':str(receipt_path),'selected_attempt':row['selected_attempt'],'digest':row['selected_output_sha256']},indent=2));print(F)
