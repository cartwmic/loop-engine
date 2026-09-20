"""Real constructors and independent scripted mixed-provider return/resume proof."""
from __future__ import annotations
import json
from pathlib import Path
import subprocess
import sys
from test_contract import ROOT


def assert_git_guidance(text):
    # Independent clause checks, not a copied complete expected instruction.
    text = text.replace('implementation capture triage', 'implementation triage')
    for clause in ['implementation triage', 'before independent review', 'owner',
                   'Git decision', 'staged names and diff', 'authorized', 'human/driver commit',
                   'verify its resulting identity', 'git rev-parse HEAD', 'pending/declined',
                   'claiming a created commit', 'Workers do not independently commit',
                   'invalidated', 'source/Git identity stable']:
        assert clause in text, clause


def guidance_case(args):
    root = args.attempt
    engine = args.binary_dir / 'loop-engine'
    software = args.binary_dir / 'software-change'
    counter = 0

    def run(argv, packet=None, expected=0):
        nonlocal counter
        counter += 1
        prefix = root / f'command-{counter:03}'
        p = subprocess.run([str(v) for v in argv], input=None if packet is None else json.dumps(packet),
                           text=True, capture_output=True, cwd=ROOT, timeout=120)
        prefix.with_suffix('.stdout').write_text(p.stdout)
        prefix.with_suffix('.stderr').write_text(p.stderr)
        prefix.with_suffix('.json').write_text(json.dumps(dict(argv=[str(v) for v in argv], cwd=str(ROOT), exit=p.returncode)))
        assert p.returncode == expected, (argv, p.returncode, p.stdout, p.stderr)
        return p.stdout

    # This entry point executes the actual software, policy and research skill constructors.
    text = run([sys.executable, ROOT / 'scripts/software-change-journey.py', '--self-test'])
    assert 'worker-data skill/root policy assertions passed' in text
    profile = json.loads((ROOT / 'crates/software-change-provider/data/configs/high-rigor.json').read_text())
    workflow = json.loads(run([software], dict(operation='describe', initial_input=profile)))
    for state in workflow['states']:
        if state['final']:
            continue
        guidance = state['action_guidance']
        for axis in guidance['review_axes'] or []:
            original = next(p for p in profile['review_policies'][state['id']] if p['id'] == axis['id'])
            assert axis['required_authors'] == original.get('required_authors', 1)
        assert 'no task' in guidance['repair']
        assert_git_guidance(guidance['git'])
    implement = next(state for state in workflow['states'] if state['id'] == 'implement')
    assert_git_guidance(implement['action_guidance']['git'])
    skill = (ROOT / 'crates/software-change-provider/skills/using-software-change-provider/SKILL.md').read_text()
    assert_git_guidance(skill)
    coordinator_path = ROOT / 'skills/coordinating-loop-engine/SKILL.md'
    coordinator = coordinator_path.read_text()
    (root / 'coordinator-skill-read.md').write_text(coordinator)
    for clause in ['owner to supply', 'role/model', 'serial budget', 'monitor and escalation owner',
                   'return destination', 'resume references', 'Keep successful peers independent']:
        assert clause in coordinator, clause
    engine_skill = ROOT / 'skills/using-loop-engine/SKILL.md'
    engine_guidance = engine_skill.read_text()
    for clause in ['workflow_lane', 'worker_lane', 'acceptance: unknown',
                   'active Pi conversation', 'observed change',
                   'needed action or owner decision', 'machine attention/completion']:
        assert clause in engine_guidance, clause
    assert '../using-loop-engine/SKILL.md' in coordinator
    for name in ['software-change', 'policy-document', 'research']:
        assert f'crates/{name}-provider/skills/using-{name}-provider/SKILL.md' in coordinator
    help_text = run([engine, 'fan-out', '--help'])
    assert 'Required in ad-hoc mode' in help_text
    run([engine, 'fan-out'], expected=2)
    # --json must reach the same ordinary input refusal, not an unknown-option error.
    plain = subprocess.run([str(software), 'checkpoint'], capture_output=True, text=True)
    marked = subprocess.run([str(software), 'checkpoint', '--json'], capture_output=True, text=True)
    for name, result in [('plain', plain), ('json', marked)]:
        (root / f'checkpoint-{name}.stdout').write_text(result.stdout)
        (root / f'checkpoint-{name}.stderr').write_text(result.stderr)
        (root / f'checkpoint-{name}.json').write_text(json.dumps(dict(argv=result.args,cwd=str(ROOT),exit=result.returncode)))
    assert plain.returncode == marked.returncode != 0 and plain.stderr == marked.stderr
    checkpoint_root = root / 'checkpoint-inputs'
    checkpoint_root.mkdir()
    for filename in ['intent.json', 'design.json', 'plan.json', 'implementation-report.json', 'validation-report.json']:
        (checkpoint_root / filename).write_text(json.dumps(dict(revision='scripted-1')))
    for phase in ['implementation', 'validation']:
        argv = [software, 'checkpoint', '--phase', phase, '--artifact-root', checkpoint_root, '--working-directory', ROOT]
        original = json.loads(run(argv))
        checkpoint = (checkpoint_root / (phase + '-checkpoint.json')).read_bytes()
        assert json.loads(run(argv + ['--json'])) == original
        assert (checkpoint_root / (phase + '-checkpoint.json')).read_bytes() == checkpoint

    # Only isolated scripted proof runs. Each new driver process receives its own durable handoff.
    providers = root / 'providers.toml'
    providers.write_text(''.join(f'[providers.{name}]\ncommand = {json.dumps(str(args.binary_dir / name))}\nargs = []\n' for name in ['research', 'policy-document']))
    target = root / 'target.md'
    target.write_text('# Scripted independent document\n')
    research = json.loads((ROOT / 'crates/research-provider/data/configs/standard.json').read_text())
    doc = dict(schema_version=1, profile_version='guidance-scripted-1', mode='audit', target=dict(id='doc',path=str(target)),
               deterministic_policies=[dict(id='present', type='non-empty')], semantic_policies=[dict(id='quality',description='scripted fixture quality',example_prompt='scripted fixture only')])
    for name, profile in [('research',research), ('policy-document',doc)]:
        artifacts = root / name
        artifacts.mkdir()
        profile['artifact_root'] = str(artifacts)
        path = root / f'{name}-profile.json'
        path.write_text(json.dumps(profile))
        handoff = dict(provider=name, run_id=name+'-guidance', database=str(root / (name+'.sqlite')),
                       engine=str(engine), providers=str(providers), profile=str(path), artifact_root=str(artifacts),
                       output=str(root / (name+'-return.json')),
                       outcome='Complete the scripted provider path; research first escalates its missing brief',
                       working_directory=str(ROOT), source_references=[str(path), str(target)],
                       ownership=dict(write=str(artifacts), coordination='fixture coordinator; no peer-run authority'),
                       model_binding_confirmation=dict(kind='scripted-only; no model launch',
                           reference=str(root / 'scripted-authority.json')),
                       serial_budget=dict(max_driver_launches=2 if name == 'research' else 1, timeout_seconds=120),
                       monitor_owner=name+'-driver', escalation_owner='fixture-source-owner',
                       next_observation='show --view full '+name+'-guidance',
                       resume_references=[str(path), str(artifacts / 'blocked-return.json')],
                       required_skills=[str(coordinator_path), str(engine_skill),
                           str(ROOT / f'crates/{name}-provider/skills/using-{name}-provider/SKILL.md')])
        (root / f'{name}-handoff.json').write_text(json.dumps(handoff))
    (root / 'scripted-authority.json').write_text(json.dumps(dict(
        scope='isolated deterministic fixture only', models=[], bindings='unbound scripted drivers',
        ownership='each driver writes only its own artifact directory and catalog; coordinator retains returns',
        budget='three serial driver processes, 120 seconds each; no live models or production operations')))
    driver = root / 'driver.py'
    driver.write_text(DRIVER)
    research_handoff = root / 'research-handoff.json'
    doc_handoff = root / 'policy-document-handoff.json'
    run([sys.executable, driver, research_handoff, 'block', ROOT])
    blocked = json.loads((root / 'research-return.json').read_text())
    assert blocked['status'] == 'blocked' and blocked['state'] == 'scope'
    run([sys.executable, driver, doc_handoff, 'complete', ROOT])
    done = (root / 'policy-document-return.json').read_bytes()
    assert json.loads(done)['state'] == 'end'
    peer_database = (root / 'policy-document.sqlite').read_bytes()
    assert blocked['required_owner_decision'] == 'supply missing brief'
    assert blocked['failed_captures']
    # The fixture source owner supplies durable permission/material, only to the blocked peer.
    (root / 'research' / 'source-owner-return.json').write_text(json.dumps(dict(
        decision='supply missing brief', source='scripts/research-journey.py scripted brief factory')))
    # New process resumes from original handoff plus retained blocked return, not parent memory.
    run([sys.executable, driver, research_handoff, 'resume', ROOT])
    assert json.loads((root / 'research-return.json').read_text())['state'] == 'end'
    assert (root / 'policy-document-return.json').read_bytes() == done
    assert (root / 'policy-document.sqlite').read_bytes() == peer_database
    for name, modes in [('research', ['block', 'resume']), ('policy-document', ['complete'])]:
        h = json.loads((root / f'{name}-handoff.json').read_text())
        result = json.loads(Path(h['output']).read_text())
        for key in ['completed_outcomes', 'failed_captures', 'unresolved_ownership_cleanup',
                    'required_owner_decision', 'resume_references', 'monitor_owner', 'escalation_owner']:
            assert key in result, key
        for mode in modes:
            reads = json.loads((root / name / (mode+'-skill-reads.json')).read_text())
            assert [entry['path'] for entry in reads] == h['required_skills']
            assert all(entry['bytes'] > 0 for entry in reads)
    (root / 'scenarios.json').write_text(json.dumps(dict(status='passed', scenarios=[
        'real three-provider skill constructors', 'frozen normalized action obligations',
        'checkpoint JSON parser parity', 'fan-out missing-input refusal',
        'research blocked while independent policy driver completes', 'fresh research driver resumes to end without peer mutation'],
        semantic_quality='not tested; explicitly scripted judgments only', handoffs=[str(research_handoff),str(doc_handoff)]), indent=2))


DRIVER = r'''import importlib.util,json,sys,subprocess,hashlib
from pathlib import Path
h=json.loads(Path(sys.argv[1]).read_text()); mode=sys.argv[2]; repo=Path(sys.argv[3]); n=0
for key in ['outcome','working_directory','source_references','ownership','model_binding_confirmation',
            'serial_budget','monitor_owner','escalation_owner','next_observation','resume_references','required_skills']:
 assert h[key],key
assert h['working_directory']==str(repo)
assert h['ownership']['write']==h['artifact_root']
authority=json.loads(Path(h['model_binding_confirmation']['reference']).read_text())
assert authority['models']==[] and authority['bindings']=='unbound scripted drivers'
reads=[]
for path in h['required_skills']:
 data=Path(path).read_bytes();assert data
 text=data.decode()
 if 'coordinating-loop-engine' in path:
  assert 'durable handoff' in text and 'Keep successful peers independent' in text
 elif path.endswith('using-loop-engine/SKILL.md'):
  assert 'show --view full' in text
 else:
  assert 'review-evidence' in text and 'show' in text
 Path(h['artifact_root'],mode+'-skill-'+str(len(reads))+'.md').write_bytes(data)
 reads.append(dict(path=path,bytes=len(data),sha256=hashlib.sha256(data).hexdigest()))
Path(h['artifact_root'],mode+'-skill-reads.json').write_text(json.dumps(reads))
base=[h['engine'],'--json','--database',h['database'],'--config',h['providers']]
def call(*argv, expected=0):
 global n
 n+=1
 p=subprocess.run(base+list(argv),capture_output=True,text=True,cwd=h['working_directory'],timeout=h['serial_budget']['timeout_seconds'])
 prefix=Path(h['artifact_root'])/(mode+'-'+str(n))
 prefix.with_suffix('.stdout').write_text(p.stdout);prefix.with_suffix('.stderr').write_text(p.stderr)
 prefix.with_suffix('.json').write_text(json.dumps(dict(argv=p.args,cwd=str(Path.cwd()),exit=p.returncode)))
 assert p.returncode==expected,(argv,p.stdout,p.stderr)
 return json.loads(p.stdout)
def show(): return call('show','--view','full',h['run_id'])['result']
def event(e):
 show();return call('event',h['run_id'],e)
if mode!='resume': call('start','--id',h['run_id'],h['provider'],'@'+h['profile'],'independent scripted guidance')
if mode=='block':
 show(); denied=call('event',h['run_id'],'scoped',expected=10)
 assert denied['status']=='rejected'
 result=dict(status='blocked',state=show()['current_state'],reason='missing brief source; owner must supply it',handoff=sys.argv[1])
 result.update(required_owner_decision='supply missing brief')
elif h['provider']=='policy-document':
 event('ready');event('passed')
 profile=json.loads(Path(h['profile']).read_text())
 evidence=dict(gate='semantic-review',policy_id='quality',result='pass',findings='',author=dict(name='scripted-independent',kind='script'),target_id='doc',target_sha256=hashlib.sha256(Path(profile['target']['path']).read_bytes()).hexdigest(),profile_version=profile['profile_version'])
 show();call('append',h['run_id'],'review-evidence',json.dumps(evidence));event('passed')
 result=dict(status='completed',state=show()['current_state'],handoff=sys.argv[1])
else:
 prior=json.loads(Path(h['output']).read_text()); assert prior['status']=='blocked' and show()['current_state']=='scope'
 assert json.loads(Path(h['artifact_root'],'source-owner-return.json').read_text())['decision']=='supply missing brief'
 assert Path(h['artifact_root'],'blocked-return.json').is_file()
 spec=importlib.util.spec_from_file_location('research_journey',repo/'scripts/research-journey.py')
 m=importlib.util.module_from_spec(spec);sys.path.insert(0,str(repo/'scripts'));sys.modules[spec.name]=m;spec.loader.exec_module(m)
 for filename, factory,e in [('brief.json',m.brief,'scoped'),('sources.json',m.sources,'gathered'),('verification.json',m.verification,'verified'),('report.json',m.report,'completed')]:
  Path(h['artifact_root'],filename).write_text(json.dumps(factory()))
  if filename in ['verification.json','report.json']:
   # Follow the loaded research procedure: validate shape before commissioning evidence.
   show();denied=call('event',h['run_id'],e,expected=10);assert denied['status']=='rejected'
   gate='verify' if filename=='verification.json' else 'synthesize'
   profile=json.loads(Path(h['profile']).read_text())
   for axis in profile['review_policies'][gate]:
    show();call('append',h['run_id'],'review-evidence',json.dumps(m.evidence(gate,axis['id'],filename,'scripted-independent',profile['config_version'])))
  event(e)
 result=dict(status='completed',state=show()['current_state'],handoff=sys.argv[1],resumed_from=str(Path(h['artifact_root'],'blocked-return.json')))
result.update(completed_outcomes=['started and observed scope'] if mode=='block' else ['provider reached end'],
 failed_captures=[str(p) for p in Path(h['artifact_root']).glob('*.json')
                  if isinstance((record:=json.loads(p.read_text())),dict) and record.get('exit',0)!=0],
 unresolved_ownership_cleanup='none; peer ownership unchanged',
 resume_references=h['resume_references']+[h['database'],sys.argv[1]],
 monitor_owner=h['monitor_owner'],escalation_owner=h['escalation_owner'])
result.setdefault('required_owner_decision','none')
if mode=='block': Path(h['artifact_root'],'blocked-return.json').write_text(json.dumps(result))
Path(h['output']).write_text(json.dumps(result))
'''
