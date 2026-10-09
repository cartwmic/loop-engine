#!/usr/bin/env python3
"""Black-box external-work outcomes. Scripted reviews establish mechanics only."""
import argparse
import hashlib
import copy
import importlib.util
import json
import os
import subprocess
import sys
import sqlite3
import tempfile
import time
from contextlib import contextmanager
from pathlib import Path
from types import SimpleNamespace
from external_work_driver import process

ROOT = Path(__file__).resolve().parents[1]
RETIRED_ENGINE = ['invoke','fan-out','amend-binding','cancel-invocation','invocation-progress',
                  'preview-bindings','capture-command','capture-matrix','capture-abort','advise']
RETIRED_PROVIDER = ['bindings','invoke','run-plan-graph','run-validation','recover-review','repair-review']


def software_module():
    spec = importlib.util.spec_from_file_location('software_journey', ROOT / 'scripts/software-change-journey.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def raw(argv, packet=None, cwd=None):
    return subprocess.run([str(a) for a in argv],input=None if packet is None else json.dumps(packet),
                          cwd=cwd,text=True,capture_output=True)


def refused(argv, packet=None, cwd=None, needle=None):
    result = raw(argv,packet,cwd)
    assert result.returncode != 0, result.stdout
    if needle:
        assert needle in result.stdout + result.stderr, result
    return result


@contextmanager
def json_source(work, data):
    # Keep growing evidence out of argv without changing its serialized bytes.
    with tempfile.NamedTemporaryFile(dir=work, suffix='.json') as source:
        source.write(json.dumps(data).encode('utf-8'))
        source.flush()
        yield '@'+source.name


def append(ctx, kind, data, record_id=None):
    args = ['append']
    if record_id:
        args += ['--record-id',record_id]
    with json_source(ctx['work'], data) as source:
        return ctx['call'](*args,ctx['run_id'],kind,source)


def provider(ctx, args, packet):
    return process([ctx['provider'],*args],packet,cwd=ctx['repository'])


def evaluate(ctx, source, event, target, *, context=None):
    shown = ctx['show']()
    workflow = provider(ctx,[],{'operation':'describe','initial_input':shown['result']['initial_input']})
    return provider(ctx,[],{'operation':'evaluate','workflow':workflow,
                           'initial_input':shown['result']['initial_input'],
                           'context':shown['result']['context'] if context is None else context,
                           'transition':{'source':source,'event':event,'target':target,'kind':'checked'},
                           'prior_evaluations':[]})


def ledger(ctx, gate, subject, revision):
    append(ctx,'finding-ledger',{'schema_version':'1','gate':gate,'subject':subject,
                               'subject_revision':revision,'author':{'name':'driver','kind':'agent'},'findings':[]})


# bookends:LE-176 — retired public commands are rejected; sentinel never runs.
def retirement(args, root):
    for binary, retired in [(args.engine,RETIRED_ENGINE),(args.software,RETIRED_PROVIDER),
                            (args.policy,RETIRED_PROVIDER),(args.research,RETIRED_PROVIDER)]:
        result = raw([binary,'--help'])
        if binary == args.engine:
            assert result.returncode == 0, result.stderr
        elif result.returncode:
            assert 'unsupported command' in result.stderr, result.stderr
        words = result.stdout.replace(',',' ').split()
        assert not set(words).intersection(retired), result.stdout
        if binary != args.engine:
            for command in retired:
                refused([binary,command],{})
    profile = json.loads((ROOT / 'crates/software-change-provider/data/configs/minimal.json').read_text())
    profile['artifact_root'] = str(root / 'artifacts')
    (root / 'artifacts').mkdir()
    profile['work_slot_bindings'] = {'intent-draft':{'command':'pi','args':[]}}
    selected = root / 'profile.json'
    selected.write_text(json.dumps(profile))
    config = root / 'providers.toml'
    config.write_text(f'[providers.software-change]\ncommand = {json.dumps(args.software)}\nargs = []\n')
    refused([args.engine,'--database',root / 'run.sqlite','--config',config,'--json','start',
             '--id','retired-bindings','software-change','@'+str(selected)],needle='binding')
    print('external-work retirement passed')


# bookends:LE-168 bookends:LE-170 bookends:LE-172 bookends:LE-177 bookends:LE-178 bookends:LE-182 — grouped external returns, typed external advice closure, early commissions/evidence, explicit counts and ordinary-first challenges.
def intake_counts_advice(args, root):
    software = software_module()
    state = {'early':False,'post':False,'answered':False}
    def configure(profile, repository, artifacts):
        selected = copy.deepcopy([p for p in profile['review_policies']['intent-review'] if p['review_stage'] == 'aggregate'][:2])
        for gate in profile['review_policies']:
            profile['review_policies'][gate] = []
        for gate in ['intent-review','intent-adversarial-review']:
            axes = copy.deepcopy(selected)
            for axis in axes:
                axis.update(group=gate+'.aggregate',required_authors=2,self_review=True)
            profile['review_policies'][gate] = axes
        configured = artifacts.parent / 'advice-profile.json'
        base = artifacts.parent / 'base.json'
        base.write_text(json.dumps(profile))
        process([args.software,'setup','--profile',base,'--output',configured,'--enable-advice'])
        profile.clear()
        profile.update(json.loads(configured.read_text()))
        profile['advice_departures'] = {'version':1,'occasions':[{'state':'explore','event':'intent-ready',
                                                               'occasion_id':'fixture-intent:explore:intent-ready'}]}
    def amend(ctx, after):
        policy = ctx['profile']['review_policies']['intent-review'][0]
        target = {'gate':'intent-review','stage':'aggregate','axis':policy['id']}
        data = {'targets':[target],'after':after,'actor':{'name':'owner','kind':'human'},
                'authority':{'kind':'owner','reference':'scripted owner instruction'},'reason':'Exercise external author budgeting.'}
        candidate = provider(ctx,['author-counts','--propose',json.dumps(data)],ctx['show']())
        append(ctx,candidate['kind'],candidate['data'])
        return candidate
    def hook(phase, ctx):
        shown = ctx['show']()
        if phase == 'explore' and not state['early']:
            state['early'] = True
            path = ctx['artifacts'] / 'intent.json'
            saved = path.read_bytes()
            path.unlink()
            early = provider(ctx,['commission','--slot','intent-review'],shown)
            assert early['missing_inputs'], early
            path.write_bytes(saved)
            subject = json.loads(saved)
            policies = ctx['profile']['review_policies']['intent-review']
            axes = [p['id'] for p in policies]
            ground_reason = 'The problem and outcome name observable frozen-policy inspection and refusal without choosing an unforced implementation; acceptance lists those operator observations.'
            judgments = [{'axis':axis,'result':'pass','findings':'','grounds':{'reason':ground_reason,
                           'evidence':[{'locator':'intent.json#/acceptance'}]}} for axis in axes]
            batch = {'assessment_id':'combined-early','judgments':judgments+[False,{'axis':axes[0],'result':'pass'}]}
            for index in range(2):
                packet = {'show':ctx['show'](),'assignment':{'gate':'intent-adversarial-review','stage':'aggregate',
                          'group':'intent-adversarial-review.aggregate','author':{'name':f'early-challenger-{index}','kind':'agent'},
                          'subject':'intent.json','subject_revision':subject['revision']},
                          'request':{'text':'Challenge the current intent.'},'original':{'text':json.dumps(batch)}}
                for candidate in provider(ctx,['review-candidates'],packet)['records']:
                    append(ctx,candidate['kind'],candidate['data'])
            ledger(ctx,'intent-adversarial-review','intent.json',subject['revision'])
            challenge = evaluate(ctx,'intent-adversarial-review','approved','design')
            assert challenge['result'] == 'deny' and challenge['feedback']['message'] == 'challenge-before-ordinary-clearance', challenge
            print('external-work ordinary-before-challenge refusal passed')
            # bookends:LE-91 bookends:LE-127 bookends:LE-149 — first ordinary review retains actual frozen operating context but excludes peers; challenge receives real parent grounds.
            blind = provider(ctx,['commission','--slot','intent-review','--stage','aggregate'],ctx['show']())
            assert blind['owner_approval_view']['current_intent']['value']['operating_context'] == subject['operating_context']
            assert all(row['kind'] not in ['review-evidence','finding-ledger'] for row in blind['context']), blind
            for index, form in enumerate(['prose','yaml','fenced','json','self']):
                if form in ['prose','self']:
                    original = {'text':'Assessment combined-early: pass with no findings for '+', '.join(axes)+
                                '. Grounds reason: '+ground_reason+' Evidence locator: intent.json#/acceptance. '
                                'Additional malformed output members: false; '+json.dumps({'axis':axes[0],'result':'pass'})+'.'}
                elif form == 'yaml':
                    original = {'text':'assessment_id: combined-early\njudgments:\n' + ''.join(
                        '- axis: '+j['axis']+'\n  result: pass\n  findings: ""\n  grounds:\n    reason: '+ground_reason+'\n    evidence:\n    - locator: intent.json#/acceptance\n' for j in judgments) +
                        '- false\n- axis: '+axes[0]+'\n  result: pass\n'}
                else:
                    text = json.dumps(batch)
                    original = {'text':'```json\n'+text+'\n```' if form == 'fenced' else text}
                assignment = {'gate':'intent-review','stage':'aggregate','group':policies[0]['group'],
                              'author':subject['author'] if form == 'self' else {'name':f'early-{index}','kind':'agent'},
                              'subject':'intent.json','subject_revision':subject['revision']}
                packet = {'show':shown,'assignment':assignment,'request':{'text':'Review both axes as one assessment.'},'original':original}
                if form in ['prose','yaml','self']:
                    packet['projection'] = batch
                # bookends:LE-93 bookends:LE-99 bookends:LE-105 bookends:LE-109 bookends:LE-151 — source-faithful projections retain valid siblings beside malformed members and remain inert until explicit append.
                if form == 'json':
                    corrected = copy.deepcopy(packet)
                    corrected['projection'] = copy.deepcopy(batch)
                    corrected['projection']['judgments'][0]['grounds']['evidence'][0]['locator'] = 'intent.json#/goal'
                    assert len(provider(ctx,['review-candidates'],corrected)['records']) == len(axes)
                    corrected['projection']['judgments'][0]['result'] = 'fail'
                    refused([args.software,'review-candidates'],corrected,needle='contradicts decoded result')
                    corrected['projection']['judgments'][0]['result'] = 'pass'
                    corrected['projection']['judgments'][0]['findings'] = 'new substantive finding'
                    refused([args.software,'review-candidates'],corrected,needle='contradicts decoded findings')
                    swapped = copy.deepcopy(packet)
                    decoded = copy.deepcopy(batch)
                    decoded['judgments'][0]['result'] = 'fail'
                    decoded['judgments'][0]['findings'] = 'axis-specific defect'
                    swapped['original'] = {'text':json.dumps(decoded)}
                    swapped['projection'] = copy.deepcopy(decoded)
                    swapped['projection']['judgments'][0]['axis'], swapped['projection']['judgments'][1]['axis'] = (
                        decoded['judgments'][1]['axis'], decoded['judgments'][0]['axis'])
                    refused([args.software,'review-candidates'],swapped,needle='contradicts decoded result')
                    rationale_swap = copy.deepcopy(packet)
                    decoded = copy.deepcopy(batch)
                    decoded['judgments'][1]['grounds']['reason'] = 'Distinct rationale for the second axis.'
                    rationale_swap['original'] = {'text':json.dumps(decoded)}
                    rationale_swap['projection'] = copy.deepcopy(decoded)
                    rationale_swap['projection']['judgments'][0]['grounds']['reason'], rationale_swap['projection']['judgments'][1]['grounds']['reason'] = (
                        decoded['judgments'][1]['grounds']['reason'], decoded['judgments'][0]['grounds']['reason'])
                    refused([args.software,'review-candidates'],rationale_swap,needle='contradicts decoded reason')
                before_intake = ctx['show']()['result']['context']
                intake = provider(ctx,['review-candidates'],packet)
                assert ctx['show']()['result']['context'] == before_intake
                assert len(intake['records']) == len(axes), intake
                assert sum(item['candidate'] is None for item in intake['items']) == 2, intake
                for candidate in intake['records']:
                    # bookends:LE-158 — exact preview rows can be resumed under their caller ID, while changed bytes conflict and leave the original intact.
                    if form == 'prose' and candidate == intake['records'][0]:
                        append(ctx,candidate['kind'],candidate['data'],'preview-resume')
                        resumed = next(row for row in ctx['show']()['result']['context'] if row['id'] == 'preview-resume')
                        assert resumed['kind'] == candidate['kind'] and resumed['data'] == candidate['data']
                        # The external driver skips this already-equal row rather than duplicating it.
                        changed = copy.deepcopy(candidate['data'])
                        changed['grounds']['reason'] = 'Conflicting replacement reason.'
                        ctx['show']()
                        with json_source(ctx['work'], changed) as source:
                            refused([ctx['engine'],'--database',ctx['database'],'--config',ctx['config'],'--json','append','--record-id','preview-resume',
                                    ctx['run_id'],candidate['kind'],source],cwd=ctx['repository'])
                    assert candidate['data']['external_review']['original']['original'] == original['text'], candidate
                    assert candidate['data']['grounds']['reason'] == ground_reason, candidate
                    assert candidate['data']['author'] == assignment['author'], candidate
                    assert candidate['data']['result'] == 'pass' and candidate['data']['findings'] == '', candidate
                    if form == 'self':
                        assert candidate['data']['external_review']['direct'] is True
                    append(ctx,candidate['kind'],candidate['data'])
            ledger(ctx,'intent-review','intent.json',subject['revision'])
            assert evaluate(ctx,'intent-review','approved','intent-adversarial-review')['result'] == 'allow'
            assert evaluate(ctx,'intent-adversarial-review','approved','design')['result'] == 'deny'
            challenge_packet = provider(ctx,['commission','--slot','intent-adversarial-review','--stage','aggregate'],ctx['show']())
            # bookends:LE-155 — challenge grounds are the actual retained parent reasons, not a fabricated pass summary.
            assert ground_reason in json.dumps(challenge_packet), challenge_packet
            denial = evaluate(ctx,'intent-adversarial-review','approved','design')
            assert 'challenge-before-ordinary-clearance' in json.dumps(denial), denial
            early_challenges = [r for r in ctx['show']()['result']['context']
                                if r['kind'] == 'review-evidence' and r['data'].get('gate') == 'intent-adversarial-review']
            for index, record in enumerate(early_challenges):
                append(ctx,'review-evidence',copy.deepcopy(record['data']),
                       'challenge-after-clearance-'+str(index))
            denial = evaluate(ctx,'intent-adversarial-review','approved','design')
            assert denial['result'] == 'deny' and 'challenge-before-ordinary-clearance' in json.dumps(denial), denial
            for index in range(2):
                fresh_batch = {'assessment_id':f'cleared-challenge-{index}','judgments':judgments}
                packet = {'show':ctx['show'](),'assignment':{'gate':'intent-adversarial-review','stage':'aggregate',
                          'group':'intent-adversarial-review.aggregate','author':{'name':f'cleared-challenger-{index}','kind':'agent'},
                          'subject':'intent.json','subject_revision':subject['revision']},
                          'request':{'text':json.dumps(challenge_packet)},'original':{'text':json.dumps(fresh_batch)}}
                for candidate in provider(ctx,['review-candidates'],packet)['records']:
                    append(ctx,candidate['kind'],candidate['data'])
            assert evaluate(ctx,'intent-adversarial-review','approved','design')['result'] == 'allow'
            print('external-work ordinary-before-challenge success passed')
            print('external-work grouped intake and early evidence qualification passed')
            decreased = amend(ctx,1)
            assert decreased['data']['before'] == 2
            increased = amend(ctx,2)
            assert increased['data']['before'] == 1
            bad = {'targets':[{'gate':'intent-review','stage':'aggregate','axis':'unknown'}],'after':1,
                   'actor':{'name':'owner','kind':'human'},'authority':{'kind':'owner','reference':'instruction'},'reason':'Invalid target.'}
            refused([ctx['provider'],'author-counts','--propose',json.dumps(bad)],ctx['show'](),ctx['repository'])
        if phase == 'intent-adversarial-review' and not state['post']:
            state['post'] = True
            amend(ctx,1)
            amend(ctx,2)
            print('external-work count decrease increase post-evidence and invalid-target refusal passed')
        # All setup-frozen occasions are declared by the driver, never inferred.
        if phase not in ['end','validation-prepared']:
            current = shown['result']['current_state']
            visit = shown['result']['state_visit']
            for occasion in shown['result']['initial_input'].get('advice_departures',{}).get('occasions',[]):
                if occasion['state'] != current:
                    continue
                triggered = not state['answered'] and phase == 'explore'
                target = {'subject':'intent.json','revision':json.loads((ctx['artifacts'] / 'intent.json').read_text())['revision']}
                response_id = 'external-answer'
                if triggered:
                    request = {'version':1,'state':{'admissibility':{'bounded_judgment':True,'evidence_sufficient':True},
                                                   'intent':json.loads((ctx['artifacts'] / 'intent.json').read_text())},
                               'target':target,'occasion':occasion['occasion_id'],
                               'questions':{'retain':{'type':'noul','instructions':'Judge this one supplied intent claim.',
                                                        'proposition':'The current intent describes the fixture need.'},
                                            'review-boundary':{'type':'noul','instructions':'Judge only the supplied intent constraint.',
                                                               'proposition':'The supplied intent places semantic review with external actors.'},
                                            'explicitness':{'type':'score','instructions':'Grade only whether the supplied problem and outcome are explicit.',
                                                'levels':[{'description':'Missing fields','criteria':'A problem or outcome is absent.'},
                                                          {'description':'Both fields explicit','criteria':'Both problem and outcome are stated.'}]}}}
                    response = {'answers':{'retain':{'type':'noul','noul':0.9,'rationale':'The explicit fixture intent states this need.'},
                                           'review-boundary':{'type':'noul','noul':0.9,'confidence':0.8},
                                           'explicitness':{'type':'score','score':0.9,'legend':{'0':'Missing fields','1':'Both fields explicit'},
                                                           'probabilities':{'0':0.1,'1':0.9},'confidence':0.9}}}
                    append(ctx,'advice-answer',{'occasion':occasion['occasion_id'],'request':request,'response':response,
                                              'author':{'name':'external-advisor','kind':'agent'},'original':{'text':json.dumps(response)}},response_id)
                    for answer_id in response['answers']:
                        append(ctx,'advice-disposition',{'response_id':response_id,'answer_id':answer_id,'occasion_id':occasion['occasion_id'],
                                                        'state_visit':visit,'target':target,'disposition':'accept',
                                                        'reason':'The supplied intent explicitly states the fixture problem and outcome; the bounded answer matches those fields.'})
                    state['answered'] = True
                    print('external-work typed advice answer supplied passed')
                append(ctx,'advice-occasion',{'state_visit':visit,'source_state':current,'event':occasion['event'],
                                            'occasion_id':occasion['occasion_id'],'target':target,'triggered':triggered,
                                            'response_ids':[response_id] if triggered else [],
                                            'trigger_source_ids':[next(r['id'] for r in ctx['show']()['result']['context'] if r['kind'] == 'review-evidence')] if triggered else [],
                                            'reason':None if triggered else 'Explicit driver judgment of applicability.'})
        if phase == 'explore':
            # bookends:LE-163 bookends:LE-165 bookends:LE-166 — externally supplied typed answers require reasoned dispositions at normal departure, not an engine advisor call.
            driver = '''import json, pathlib, subprocess, sys
engine, run_id, database, artifact_root = sys.argv[1:]
base = [engine, '--database', database, '--config', str(pathlib.Path(database).parent / 'providers.toml'), '--json']
def call(*args):
    result = subprocess.run(base + list(args), text=True, capture_output=True, check=True)
    return json.loads(result.stdout)
shown = call('show', '--view', 'full', run_id)['result']
assert shown['current_state'] == 'explore'
assert pathlib.Path(artifact_root, 'intent.json').is_file()
call('append', '--record-id', 'fresh-driver', run_id, 'driver-note', json.dumps({'resumed_from':'durable run and artifact references'}))
call('show', '--view', 'full', run_id)
call('event', run_id, 'intent-ready')
assert call('show', '--view', 'full', run_id)['result']['current_state'] == 'intent-review'
'''
            continuation = raw([sys.executable,'-c',driver,args.engine,ctx['run_id'],ctx['database'],ctx['artifacts']])
            assert continuation.returncode == 0, continuation.stderr
            print('external-work fresh-driver active continuation passed')
            return True
        if phase == 'end':
            assert state['answered'], state
            assert ctx['show']()['result']['lifecycle'] == 'final'
            print('external-work advice occasion closure normal completion passed')
    work = software.run(SimpleNamespace(engine=args.engine,provider=args.software,data_root=str(ROOT),mode='source'),
                        ROOT / 'crates/software-change-provider/data/configs/high-rigor.json',root,
                        configure=configure,exercise=hook)
    return work


# bookends:LE-179 bookends:LE-181 bookends:LE-183 — two source scopes, stale/wrong-source refusal and truthful final residuals.
# bookends:LE-94 bookends:LE-95 — provider checkpoints bind the actual scoped tree; a changed secondary tree cannot inherit primary acceptance.
def paired_and_command_proof(args, root):
    state = {}
    def configure(profile, repository, artifacts):
        profile['extra']['advice']['enabled'] = True
        profile['advice_departures'] = {'version':1,'occasions':[{'state':'validation','event':'validation-ready',
                                      'occasion_id':'final-completion:validation:validation-ready'}]}
        secondary = artifacts.parent / 'secondary'
        subprocess.run(['git','clone','-q',str(repository),str(secondary)],check=True)
        (secondary / 'scoped.json').write_text('{"secondary":true}')
        subprocess.run(['git','add','scoped.json'],cwd=secondary,check=True)
        subprocess.run(['git','-c','user.name=Journey','-c','user.email=journey@example.invalid',
                        'commit','-qm','secondary fixture'],cwd=secondary,check=True)
        intent_path = artifacts / 'intent.json'
        intent = json.loads(intent_path.read_text())
        intent['source_scopes'] = {'primary':{'root':str(repository)},'secondary':{'root':str(secondary)}}
        intent_path.write_text(json.dumps(intent))
        state['secondary'] = secondary
        state['residual'] = 'Unresolved proof gap: the helper assertion does not test malformed-artifact refusal (AC-2).'
        intent['operating_context']['accepted_risks'].append({
            'risk':'AC-1 inspection is demonstrated only by scripted fixture inspection.',
            'rationale':'This exception is scoped to AC-1 and does not waive AC-2 malformed-artifact refusal.'})
        intent['acceptance'][3]['statement'] += ' The secondary repository helper returns its input.'
        intent_path.write_text(json.dumps(intent))
        implementation_path = artifacts / 'implementation-report.json'
        implementation = json.loads(implementation_path.read_text())
        implementation['summary'] += ' ' + state['residual']
        implementation_path.write_text(json.dumps(implementation))
    def hook(phase, ctx):
        if phase == 'implement':
            # bookends:LE-123 — the public guidance requires the owner Git decision, and checkpoint creation is not a commit.
            guidance = ctx['show']()['result']['action_guidance']['git']
            assert 'ask the owner' in guidance and 'Workers do not independently commit' in guidance
        if phase == 'validation' and not state.get('recovered'):
            # bookends:LE-96 — validation exposes and takes the stored check-free implementation recovery route; both checkpoints are subsequently refreshed on the changed scoped tree.
            state['recovered'] = True
            (state['secondary'] / 'scoped.json').write_text('{"secondary":false}')
            report_path = ctx['artifacts'] / 'implementation-report.json'
            implementation = json.loads(report_path.read_text())
            implementation['revision'] += '-refreshed'
            report_path.write_text(json.dumps(implementation))
            shown = ctx['show']()['result']
            assert any(edge['event'] == 'revise-implementation' for edge in shown['requestable_events']), shown
            ctx['event']('revise-implementation')
            assert ctx['show']()['result']['current_state'] == 'implement'
            return True
        if phase == 'implementation-review':
            path = state['secondary'] / 'scoped.json'
            saved = path.read_bytes()
            path.write_bytes(saved + b'\n')
            stale = evaluate(ctx,'implementation-review','approved','implementation-adversarial-review')
            assert stale['result'] == 'deny' and 'checkpoint' in json.dumps(stale).lower(), stale
            path.write_bytes(saved)
            print('external-work paired-repo stale-secondary refusal passed')
        if phase == 'validation-prepared':
            shown = ctx['show']()
            command = next(r for r in shown['result']['context'] if r['kind'] == 'command-evidence')
            original = copy.deepcopy(ctx['report'])
            secondary_spec = {'id':'secondary-helper','command':sys.executable,
                              'args':['-B','-c','from helper import helper; assert helper(7) == 7'],
                              'owner':'driver','obligation':'Assert the secondary repository helper returns its input.'}
            plan_path = ctx['artifacts'] / 'plan.json'
            plan = json.loads(plan_path.read_text())
            plan['proof_commands'].append(secondary_spec)
            plan_path.write_text(json.dumps(plan))
            identity = provider(ctx,['source-identity','--working-directory',str(state['secondary'])],{})
            identity['scope'] = 'secondary'
            outcome = raw([secondary_spec['command'],*secondary_spec['args']],cwd=state['secondary'])
            assert outcome.returncode == 0, outcome.stderr
            result = {'id':secondary_spec['id'],'command':secondary_spec['command'],'args':secondary_spec['args'],
                      'cwd':str(state['secondary']),'exit_code':outcome.returncode,'stdout':outcome.stdout,
                      'stderr':outcome.stderr,'producer':{'name':'external-driver','kind':'script'},'source':identity}
            prepared = provider(ctx,['prepare-validation'],{'show':ctx['show'](),'working_directory':str(ctx['repository']),
                                'revision':original['revision']+'-secondary','author':{'name':'external-proof-owner','kind':'script'},
                                'command_results':[dict(command['data']['external_result'],id=command['data']['proof_id']),result]})
            assert prepared['commands_complete'], prepared
            original = prepared['report_draft']
            for candidate in prepared['command_candidates']:
                append(ctx,candidate['kind'],candidate['data'],candidate['record_id'])
            for index, batch in enumerate(prepared['judgment_batches']):
                append(ctx,batch['kind'],{'subject':'validation-report.json','subject_revision':original['revision'],
                           'checkpoint':'validation-checkpoint.json','author':{'name':f'secondary-judge-{index}','kind':'agent'},
                           **({'criterion_id':batch['criterion_id']} if batch['kind'] == 'criterion-verdict' else {}),'result':'pass','findings':[],
                           'reason':'The secondary-scoped command succeeded in the secondary repository.',
                           'evidence_context_ids':original['command_evidence_ids']},batch['record_id'])
            (ctx['artifacts'] / 'validation-report.json').write_text(json.dumps(original))
            ctx['checkpoint']('validation')
            print('external-work secondary-scoped successful command accepted passed')
            # bookends:LE-164 — final-completion bounded questions consume actual successor external command outputs and indexed judgments, not retired engine captures.
            report = original
            ids = report['command_evidence_ids'] + report['goal_verdict_ids'] + [id for row in report['criteria'] for id in row['verdict_ids']]
            def request(selected_ids):
                return provider(ctx,['advice-request'],{'show':ctx['show'](),'occasion_id':'final-completion:validation:validation-ready',
                       'admissibility':{'bounded_judgment':True,'evidence_sufficient':True},
                       'judgments':{'goal.checks-could-miss':'The helper assertion alone does not establish every fixture intent obligation.'},
                       'source_context_ids':selected_ids,'artifact_names':['intent.json','plan.json','validation-report.json'],'documents':[]})
            prepared_advice = request(ids)
            checks = prepared_advice['state']['selected_check_assertions']
            assert checks[0]['capture_excerpt']['format'] == 'external-command-result', checks
            assert checks[0]['capture_excerpt']['producer'] == command['data']['external_result']['producer']
            # Actual output references are readable; an unavailable reference is not invented output.
            saved_report = (ctx['artifacts'] / 'validation-report.json').read_bytes()
            referenced = copy.deepcopy(command['data'])
            stdout_file = ctx['work'] / 'external-stdout.txt'
            stdout_file.write_text(referenced['external_result'].pop('stdout'))
            referenced['external_result']['stdout_ref'] = str(stdout_file)
            append(ctx,'command-evidence',referenced,'advice-reference')
            reference_report = copy.deepcopy(report)
            reference_report['command_evidence_ids'] = ['advice-reference']
            (ctx['artifacts'] / 'validation-report.json').write_text(json.dumps(reference_report))
            reference_ids = ['advice-reference'] + [id for id in ids if id not in report['command_evidence_ids']]
            assert request(reference_ids)['state']['selected_check_assertions'][0]['capture_excerpt']['streams']['stdout']['excerpt'] == stdout_file.read_text()
            hidden = stdout_file.with_suffix('.hidden')
            stdout_file.rename(hidden)
            refused([ctx['provider'],'advice-request'],{'show':ctx['show'](),'occasion_id':'final-completion:validation:validation-ready',
                    'admissibility':{'bounded_judgment':True,'evidence_sufficient':True},'judgments':{'goal.checks-could-miss':'The supplied check can miss a fixture obligation.'},
                    'source_context_ids':reference_ids,'artifact_names':['intent.json','plan.json','validation-report.json'],'documents':[]},ctx['repository'],needle='reference is unavailable')
            hidden.rename(stdout_file)
            (ctx['artifacts'] / 'validation-report.json').write_bytes(saved_report)
            response = {'answers':{'goal.checks-could-miss':{'type':'choice','choice':'supported',
                        'probabilities':{'supported':0.9,'contradicted':0.05,'not-established':0.05},'confidence':0.9,
                        'rationale':'The one helper assertion cannot prove every fixture acceptance claim.'}}}
            external_answer = raw([sys.executable,'-c','print('+repr(json.dumps(response))+')'])
            assert external_answer.returncode == 0
            append(ctx,'advice-answer',{'occasion':prepared_advice['occasion'],'request':prepared_advice,'response':json.loads(external_answer.stdout),
                   'author':{'name':'scripted-external-advisor','kind':'agent'},'original':{'text':external_answer.stdout}},'final-advice-answer')
            append(ctx,'advice-disposition',{'response_id':'final-advice-answer','answer_id':'goal.checks-could-miss',
                   'occasion_id':prepared_advice['occasion'],'state_visit':ctx['show']()['result']['state_visit'],
                   'target':prepared_advice['target'],'disposition':'accept','reason':'The bounded fixture assertion does not establish all the fixture review obligations.'})
            append(ctx,'advice-occasion',{'state_visit':ctx['show']()['result']['state_visit'],'source_state':'validation','event':'validation-ready',
                   'occasion_id':prepared_advice['occasion'],'target':prepared_advice['target'],'triggered':True,
                   'response_ids':['final-advice-answer'],'trigger_source_ids':report['command_evidence_ids']})
            (ctx['artifacts'] / 'validation-report.json').write_text(json.dumps(reference_report))
            stdout_file.rename(hidden)
            ctx['checkpoint']('validation')
            refusal = refused([ctx['engine'],'--database',ctx['database'],'--config',ctx['config'],'--json',
                               'event',ctx['run_id'],'validation-ready'],cwd=ctx['repository'],needle='criterion-incomplete')
            assert 'unreadable stdout_ref' in refusal.stdout, refusal.stdout
            print('external-work missing external proof output reference refusal passed')
            hidden.rename(stdout_file)
            (ctx['artifacts'] / 'validation-report.json').write_bytes(saved_report)
            for label in ['stale','wrong-source']:
                data = copy.deepcopy(command['data'])
                if label == 'stale':
                    data['external_result']['source']['head'] = '0'*40
                else:
                    data['external_result']['cwd'] = str(state['secondary'])
                    data['external_result']['source'] = provider(ctx,['source-identity','--working-directory',str(state['secondary'])],{})
                record_id = 'refused-'+label
                append(ctx,'command-evidence',data,record_id)
                report = copy.deepcopy(original)
                report['command_evidence_ids'] = [record_id]
                (ctx['artifacts'] / 'validation-report.json').write_text(json.dumps(report))
                ctx['checkpoint']('validation')
                ctx['show']()
                refusal = refused([ctx['engine'],'--database',ctx['database'],'--config',ctx['config'],'--json',
                                   'event',ctx['run_id'],'validation-ready'],cwd=ctx['repository'],needle='criterion-incomplete')
                assert ('stale-source' if label == 'stale' else 'outside declared source scope') in refusal.stdout, refusal.stdout
            (ctx['artifacts'] / 'validation-report.json').write_text(json.dumps(original))
            ctx['checkpoint']('validation')
            print('external-work command proof stale and wrong-source refusal passed')
            original['residuals'] = [{'kind':'proof-gap','statement':state['residual'],'references':['intent.json#/acceptance/1']}]
            report_path = ctx['artifacts'] / 'validation-report.json'
            report_path.write_text(json.dumps(original))
            ctx['checkpoint']('validation')
            brief = provider(ctx,['commission','--slot','validation-review'],ctx['show']())
            sources = {row['locator']:(ctx['artifacts'] / row['locator']).read_text() for row in brief['review_sources']}
            assert state['residual'] in sources['validation-report.json'], sources
            goal = next(row for row in ctx['show']()['result']['context']
                        if row['id'] in original['goal_verdict_ids'])
            judgment = copy.deepcopy(goal['data'])
            judgment.update(result='fail',findings=[state['residual']],
                            reason='The AC-1 exception does not waive the unresolved AC-2 proof gap.')
            append(ctx,'goal-verdict',judgment,'residual-goal-refusal')
            original['goal_verdict_ids'] = ['residual-goal-refusal', original['goal_verdict_ids'][1]]
            report_path.write_text(json.dumps(original))
            ctx['checkpoint']('validation')
            refused([ctx['engine'],'--database',ctx['database'],'--config',ctx['config'],'--json',
                     'event',ctx['run_id'],'validation-ready'],cwd=ctx['repository'],needle='goal')
            print('external-work visible residual and outcome-scoped exception refusal passed')
            # The driver supplies the missing AC-2 proof before any passing reassessment.
            shown = ctx['show']()
            workflow = provider(ctx,[],{'operation':'describe','initial_input':shown['result']['initial_input']})
            packet = {'operation':'evaluate','workflow':workflow,
                      'initial_input':shown['result']['initial_input'],'context':shown['result']['context'],
                      'transition':{'source':'explore','event':'intent-ready','target':'intent-review','kind':'checked'},
                      'prior_evaluations':[]}
            packet_path = ctx['work'] / 'malformed-proof-request.json'
            packet_path.write_text(json.dumps(packet))
            proof_code = '\n'.join([
                'import json, pathlib, subprocess',
                'path = pathlib.Path(' + repr(str(ctx['artifacts'] / 'intent.json')) + ')',
                'saved = path.read_bytes()',
                'try:',
                '    artifact = json.loads(saved); artifact.pop("revision")',
                '    path.write_text(json.dumps(artifact))',
                '    result = subprocess.run([' + repr(str(ctx['provider'])) + '], input=pathlib.Path(' + repr(str(packet_path)) + ').read_text(), text=True, capture_output=True, check=True)',
                '    response = json.loads(result.stdout)',
                '    assert response["result"] == "deny", response',
                '    assert "revision" in json.dumps(response), response',
                '    print("AC-2 malformed intent denied: missing revision rule named")',
                'finally:',
                '    path.write_bytes(saved)',
            ])
            spec = {'id':'malformed-artifact-refusal','command':sys.executable,
                    'args':['-B','-c',proof_code],'owner':'driver',
                    'obligation':'AC-2: deny a malformed subject and name its missing revision rule.'}
            plan = json.loads(plan_path.read_text())
            plan['proof_commands'].append(spec)
            plan_path.write_text(json.dumps(plan))
            identity = provider(ctx,['source-identity','--working-directory',str(ctx['repository'])],{})
            outcome = raw([spec['command'],*spec['args']],cwd=ctx['repository'])
            assert outcome.returncode == 0, outcome.stdout + outcome.stderr
            results = [dict(row['data']['external_result'],id=row['data']['proof_id'])
                       for row in ctx['show']()['result']['context'] if row['id'] in original['command_evidence_ids']]
            results.append({'id':spec['id'],'command':spec['command'],'args':spec['args'],
                            'cwd':str(ctx['repository']),'exit_code':outcome.returncode,
                            'stdout':outcome.stdout,'stderr':outcome.stderr,
                            'producer':{'name':'external-driver','kind':'script'},'source':identity})
            prepared = provider(ctx,['prepare-validation'],{'show':ctx['show'](),
                                'working_directory':str(ctx['repository']),
                                'revision':original['revision']+'-resolved',
                                'author':{'name':'external-proof-owner','kind':'script'},'command_results':results})
            assert prepared['commands_complete'], prepared
            original = prepared['report_draft']
            intent = json.loads((ctx['artifacts'] / 'intent.json').read_text())
            state['accepted_limitation'] = intent['operating_context']['accepted_risks'][0]['risk']
            original['residuals'] = [{'kind':'accepted-boundary','statement':state['accepted_limitation'],
                                     'references':['intent.json#/operating_context/accepted_risks/0']}]
            append(ctx,'criterion-revalidation',{'subject_revision':original['revision'],
                   'affected_criteria':['AC-2'],'change_kind':'material',
                   'reason':'The missing malformed-artifact refusal command now succeeds.'})
            for candidate in prepared['command_candidates']:
                append(ctx,candidate['kind'],candidate['data'],candidate['record_id'])
            proof_id = next(candidate['record_id'] for candidate in prepared['command_candidates']
                            if candidate['data']['proof_id'] == spec['id'])
            for index, batch in enumerate(prepared['judgment_batches']):
                fresh = {'subject':'validation-report.json','subject_revision':original['revision'],
                         'checkpoint':'validation-checkpoint.json',
                         'author':{'name':f'residual-revalidation-{index}','kind':'agent'},
                         'result':'pass','findings':[],
                         'reason':'Actual AC-2 malformed-artifact refusal proof succeeded; the remaining accepted limitation does not waive any outcome.',
                         'evidence_context_ids':original['command_evidence_ids']}
                if batch['kind'] == 'criterion-verdict':
                    fresh['criterion_id'] = batch['criterion_id']
                assert proof_id in fresh['evidence_context_ids']
                append(ctx,batch['kind'],fresh,batch['record_id'])
            report_path.write_text(json.dumps(original))
            ctx['checkpoint']('validation')
            final_goals = [row for row in ctx['show']()['result']['context'] if row['id'] in original['goal_verdict_ids']]
            assert final_goals and all(row['data']['result'] == 'pass' and
                                       proof_id in row['data']['evidence_context_ids'] for row in final_goals)
            assert state['residual'] not in json.dumps(original['residuals'])
            assert original['residuals'][0]['kind'] == 'accepted-boundary'
            assert original['residuals'][0]['statement'] in [risk['risk'] for risk in intent['operating_context']['accepted_risks']]
            state['resolved_proof_id'] = proof_id
            print('external-work residual resolved before completion passed')
        if phase == 'end':
            assert not ctx['show']()['result']['has_overrides']
            final_report = ctx['work'] / 'final-report.md'
            final_report.write_text('# External-work final report\n\nNormal completion, no override.\n\n'
                                    'AC-2 gap resolved by successful command evidence: ' + state['resolved_proof_id'] + '\n'
                                    'Residual risks (declared accepted limitation): ' + state['accepted_limitation'] + '\n'
                                    'The external driver owns proof commands and source applicability decisions.\n')
            assert state['residual'] not in final_report.read_text()
            assert state['accepted_limitation'] in final_report.read_text()
            assert 'declared accepted limitation' in final_report.read_text()
            print('external-work paired-repo and external-command normal completion passed')
            print('external-work residual final-report reporting passed')
    return software_module().run(SimpleNamespace(engine=args.engine,provider=args.software,data_root=str(ROOT),mode='source'),
                                 ROOT / 'crates/software-change-provider/data/configs/high-rigor.json',root,
                                 configure=configure,exercise=hook)


# bookends:LE-171 — retained suite and whole-goal judgments remain applicable after affected lint checks.
# bookends:LE-107 — explicit affected checks preserve applicable original siblings, authors and command results without ceremonial suite replay.
def lint_only_reuse(args, root):
    state = {'reused':False}
    acceptance = [
        {'id':'AC-1','statement':'parse_retained_return recognizes review returns (P): kind review-evidence or both result and gate; ignores malformed JSON and unrelated objects; rejects missing or ambiguous judgments.'},
        {'id':'AC-2','statement':'parse_retained_return excludes duplicate recognized values (Q), returning one unique judgment even when repeated; behavior is unchanged by collapsing nested P/Q conditions to P and Q.'},
    ]
    suite_reason = ('test_recognition_and_duplicate_exclusion asserts equal returned values for both kind=review-evidence '
                    'and result/pass plus gate/validation-review, each repeated amid malformed JSON and an unrelated object. '
                    'test_missing_and_ambiguous_judgments asserts ValueError for empty input, result-only, gate-only, '
                    'unrelated objects and two distinct recognized judgments. These assertions cover the entire parser-only '
                    'intent and both criteria. In the retained nested and collapsed variants, the only difference is '
                    'if P: if Q: append versus if P and Q: append; the predicates, append and error handling are identical, '
                    'so the collapse preserves the behavior established by these assertions.')
    # Match a11.rs::parse_retained_return: recognize review returns, then exclude duplicates.
    nested = """import json


def parse_retained_return(output):
    values = []
    offset = 0
    decoder = json.JSONDecoder()
    while (start := output.find('{', offset)) != -1:
        try:
            value, consumed = decoder.raw_decode(output[start:])
            offset = start + consumed
            if (value.get("kind") == "review-evidence" or ("result" in value and "gate" in value)):
                if value not in values:
                    values.append(value)
        except json.JSONDecodeError:
            offset = start + 1
    if len(values) != 1:
        raise ValueError("prior raw return has missing or ambiguous judgments")
    return values[0]
"""
    collapsed = nested.replace(
        'if (value.get("kind") == "review-evidence" or ("result" in value and "gate" in value)):\n                if value not in values:\n                    values.append(value)',
        'if (value.get("kind") == "review-evidence" or ("result" in value and "gate" in value)) and value not in values:\n                values.append(value)')
    helper_tests = """import json
import unittest
from helper import parse_retained_return


class RetainedReturnTests(unittest.TestCase):
    def test_recognition_and_duplicate_exclusion(self):
        for value in [{"kind": "review-evidence", "data": {}}, {"result": "pass", "gate": "validation-review"}]:
            with self.subTest(value=value):
                text = json.dumps(value)
                self.assertEqual(parse_retained_return('noise {invalid ' + text + text + '{"other": true}'), value)

    def test_missing_and_ambiguous_judgments(self):
        for text in ['', '{"result": "pass"}', '{"gate": "validation-review"}', '{"other": true}',
                     '{"kind": "review-evidence"}{"result": "pass", "gate": "validation-review"}']:
            with self.subTest(text=text):
                with self.assertRaisesRegex(ValueError, "missing or ambiguous judgments"):
                    parse_retained_return(text)


if __name__ == '__main__':
    unittest.main()
"""
    def configure(profile, repository, artifacts):
        policy = copy.deepcopy(next(p for p in profile['review_policies']['validation-review'] if p['review_stage'] == 'aggregate'))
        for gate in profile['review_policies']:
            profile['review_policies'][gate] = []
        policy.update(required_authors=1,group='validation-review.aggregate')
        profile['review_policies']['validation-review'] = [policy]
        (repository / 'helper.py').write_text(nested)
        (repository / 'test_helper.py').write_text(helper_tests)
        # Keep both exact variants accessible as retained controlled-repository fixtures.
        (repository / 'helper-nested.txt').write_text(nested)
        (repository / 'helper-collapsed.txt').write_text(collapsed)
        subprocess.run(['git','add','helper.py','test_helper.py','helper-nested.txt','helper-collapsed.txt'],cwd=repository,check=True)
        subprocess.run(['git','commit','-qm','nested retained-return test helper'],cwd=repository,check=True)
        counter = artifacts.parent / 'suite-runs'
        assertion = 'import runpy; runpy.run_path("test_helper.py", run_name="__main__")'
        # unittest exits normally; count the suite before executing its assertions.
        suite_assertion = f'open({str(counter)!r}, "a").write("suite\\n"); ' + assertion
        state['assertion'] = assertion
        state['counter'] = counter
        intent_path = artifacts / 'intent.json'
        intent = json.loads(intent_path.read_text())
        intent.update(problem='The retained-return parser needs regression coverage during a behavior-equivalent lint collapse.',
                      outcome='parse_retained_return recognizes review returns (P) and excludes duplicates (Q), with parsing and missing/ambiguous rejection behavior unchanged by the lint collapse.',
                      acceptance=copy.deepcopy(acceptance),
                      constraints=['Preserve parser behavior when collapsing nested P/Q conditions to P and Q.'],
                      non_goals=['Policy/schema inspection, artifact diagnostics and workflow evidence refusals.'])
        intent['baseline']['commit'] = subprocess.check_output(['git','rev-parse','HEAD'],cwd=repository,text=True).strip()
        intent_path.write_text(json.dumps(intent))
        assert json.loads(intent_path.read_text())['acceptance'] == acceptance, 'Lint fixture acceptance must name only suite-covered parser behavior.'
        plan_path = artifacts / 'plan.json'
        plan = json.loads(plan_path.read_text())
        task = copy.deepcopy(plan['tasks'][0])
        task.update(id='parser-lint-collapse', objective=intent['outcome'], dependencies=[],
                    source_of_truth=['intent.json#/acceptance','helper-nested.txt','helper-collapsed.txt'],
                    deliverables=['helper.py with behavior-equivalent P/Q conjunction collapse'],
                    out_of_scope=intent['non_goals'], validation=[suite_reason],
                    handoff='Retain original parser suite proof; run current lint and affected helper tests after the collapse.',
                    criterion_ids=[item['id'] for item in acceptance])
        plan.update(objective=intent['outcome'], tasks=[task])
        plan['proof_commands'] = [{'id':'prior-suite','command':sys.executable,
                                  'args':['-B','-c',suite_assertion],
                                  'owner':'driver','obligation':'Assert review-return recognition, duplicate exclusion, and missing/ambiguous rejection.'}]
        plan_path.write_text(json.dumps(plan))
    def hook(phase, ctx):
        if phase == 'validation-review' and not state['reused']:
            state['reused'] = True
            artifacts = ctx['artifacts']
            original_report = json.loads((artifacts / 'validation-report.json').read_text())
            policy = ctx['profile']['review_policies']['validation-review'][0]
            packet = {'show':ctx['show'](),'assignment':{'gate':'validation-review','stage':'aggregate','group':policy['group'],
                      'author':{'name':'prior-reviewer','kind':'agent'},'subject':'validation-report.json',
                      'subject_revision':original_report['revision']},'request':{'text':'Assess the complete parser-only intent and both criteria using test_helper.py assertions and retained suite and goal judgments.'},
                      'original':{'text':json.dumps({'judgments':[{'axis':policy['id'],'result':'pass','findings':'',
                                  'grounds':{'reason':suite_reason,
                                             'evidence':[{'locator':'validation-report.json#/command_evidence_ids'}]}}]})}}
            for candidate in provider(ctx,['review-candidates'],packet)['records']:
                append(ctx,candidate['kind'],candidate['data'])
            ledger(ctx,'validation-review','validation-report.json',original_report['revision'])
            prior = evaluate(ctx,'validation-review','passed','end')
            assert prior['result'] == 'allow', prior
            originals = ctx['show']()['result']['context']
            ctx['event']('revise-implementation')
            (ctx['repository'] / 'helper.py').write_text(collapsed)
            subprocess.run(['git','add','helper.py'],cwd=ctx['repository'],check=True)
            subprocess.run(['git','commit','-qm','collapse lint-only nested condition'],cwd=ctx['repository'],check=True)
            implementation = json.loads((artifacts / 'implementation-report.json').read_text())
            implementation['revision'] = 'lint-i2'
            implementation['summary'] += ' Lint-only conjunction collapse preserves review-return recognition and unchanged duplicate exclusion.'
            (artifacts / 'implementation-report.json').write_text(json.dumps(implementation))
            ctx['checkpoint']('implementation')
            ctx['event']('implementation-ready')
            ctx['event']('reconciliation-ready')
            assert ctx['show']()['result']['current_state'] == 'validation'
            additions = [{'id':'helper-rerun','command':sys.executable,'args':['-B','-c',state['assertion']],
                          'owner':'driver','obligation':'Rerun the affected parse_retained_return recognition and deduplication tests.'},
                         {'id':'lint-rerun','command':sys.executable,
                          'args':['-B','-c','import ast, pathlib; s = pathlib.Path("helper.py").read_text(); ast.parse(s); tree = ast.parse(s); conditions = [n for n in ast.walk(tree) if isinstance(n, ast.If) and any(isinstance(v, ast.Call) and isinstance(v.func, ast.Attribute) and v.func.attr == "append" for v in ast.walk(n))]; assert len(conditions) == 1; assert isinstance(conditions[0].test, ast.BoolOp) and isinstance(conditions[0].test.op, ast.And); assert "and value not in values:" in s'],
                          'owner':'driver','obligation':'Check syntax and the exact requested conjunction collapse.'}]
            previous_command = next(row for row in originals if row['kind'] == 'command-evidence')
            results = [copy.deepcopy(previous_command['data']['external_result'])]
            for spec in additions:
                identity = provider(ctx,['source-identity','--working-directory',str(ctx['repository'])],{})
                outcome = raw([spec['command'],*spec['args']],cwd=ctx['repository'])
                assert outcome.returncode == 0, outcome.stderr
                results.append({'id':spec['id'],'command':spec['command'],'args':spec['args'],'cwd':str(ctx['repository']),
                                'exit_code':outcome.returncode,'stdout':outcome.stdout,'stderr':outcome.stderr,
                                'producer':{'name':'lint-driver','kind':'script'},'source':identity})
            prepared = provider(ctx,['prepare-validation'],{'show':ctx['show'](),'working_directory':str(ctx['repository']),
                                'revision':'lint-v2','author':{'name':'lint-proof-owner','kind':'script'},
                                'command_results':results,'additions':additions})
            assert prepared['diagnostics'] == [{'proof_id':'prior-suite','status':'stale-source'}], prepared
            for candidate in prepared['addition_candidates']:
                append(ctx,candidate['kind'],candidate['data'],candidate['record_id'])
            for candidate in prepared['command_candidates']:
                if candidate['data']['proof_id'] != 'prior-suite':
                    append(ctx,candidate['kind'],candidate['data'],candidate['record_id'])
            report = copy.deepcopy(original_report)
            report.update(revision='lint-v2',implementation_revision='lint-i2')
            for original in originals:
                kind = original['kind']
                if kind not in ['command-evidence','criterion-verdict','goal-verdict','review-evidence']:
                    continue
                if kind == 'review-evidence' and original['data']['gate'] != 'validation-review':
                    continue
                source_id = original['id']
                carry_id = 'carry-'+source_id
                target = {'subject':'validation-report.json','checkpoint':{'phase':'validation','report_revision':'lint-v2'}}
                target['subject_revision' if kind in ['criterion-verdict','goal-verdict'] else 'revision'] = 'lint-v2'
                append(ctx,'evidence-applicability',{'origin':{'kind':'context-record','id':source_id},'target':target,
                       'attesting_driver':{'name':'lint-driver','kind':'agent'},
                       'reason':'Only parse_retained_return review-return recognition (P) and unchanged duplicate exclusion (Q) were collapsed from nested ifs to P and Q. Both retained variants have identical parsing, recognition, deduplication and error behavior; current helper tests and lint pass. The earlier suite, complete aggregate review, criterion and whole-goal judgments keep their original identities and remain applicable because their complete scope and judging meaning are unchanged.'},carry_id)
                report['command_evidence_ids'] = [carry_id if value == source_id else value for value in report['command_evidence_ids']]
                for criterion in report['criteria']:
                    criterion['verdict_ids'] = [carry_id if value == source_id else value for value in criterion['verdict_ids']]
                report['goal_verdict_ids'] = [carry_id if value == source_id else value for value in report['goal_verdict_ids']]
            report['command_evidence_ids'] += [c['record_id'] for c in prepared['command_candidates'] if c['data']['proof_id'] != 'prior-suite']
            (artifacts / 'validation-report.json').write_text(json.dumps(report))
            ctx['checkpoint']('validation')
            ledger(ctx,'validation-review','validation-report.json','lint-v2')
            ctx['event']('validation-ready')
            ctx['event']('passed')
            assert state['counter'].read_text() == 'suite\n', 'Prior suite was rerun instead of reused.'
            terminal = ctx['show']()['result']
            assert terminal['lifecycle'] == 'final' and not terminal['has_overrides'], terminal
            print('external-work lint-only applicability normal completion passed')
            return True
    return software_module().run(SimpleNamespace(engine=args.engine,provider=args.software,data_root=str(ROOT),mode='source'),
                                 ROOT / 'crates/software-change-provider/data/configs/high-rigor.json',root,
                                 configure=configure,exercise=hook,judgment_reason=suite_reason)


# bookends:LE-154 — invalid confirmation reuse is row-local; a fresh sibling survives and the corrected carry completes the gate.
def invalid_reuse_fresh_sibling(args, root):
    state = {'done':False}
    def configure(profile, repository, artifacts):
        axes = copy.deepcopy([p for p in profile['review_policies']['intent-review'] if p['review_stage'] == 'aggregate'][:2])
        assert len(axes) == 2
        for gate in profile['review_policies']:
            profile['review_policies'][gate] = []
        for axis in axes:
            axis.update(group='confirmation-pair',required_authors=1,self_review=True)
        profile['review_policies']['intent-review'] = axes
    def hook(phase, ctx):
        if phase != 'intent-review' or state['done']:
            return
        state['done'] = True
        axes = ctx['profile']['review_policies'][phase]
        revision = json.loads((ctx['artifacts'] / 'intent.json').read_text())['revision']
        assignment = {'gate':phase,'stage':'aggregate','group':'confirmation-pair',
                      'author':{'name':'confirmation-author','kind':'agent'},
                      'subject':'intent.json','subject_revision':revision}
        def intake(rows):
            return provider(ctx,['review-candidates'],{'show':ctx['show'](),'assignment':assignment,
                'request':{'text':'Confirm applicable prior intent judgment and independently inspect its sibling.'},
                'original':{'text':json.dumps({'assessment_id':'confirmation-assessment','judgments':rows})}})
        def fresh(axis):
            return {'axis':axis,'result':'pass','findings':'','grounds':{
                'reason':'Inspected the current intent acceptance and operating boundary.',
                'evidence':[{'locator':'intent.json#/acceptance'}]}}
        historical_assignment = dict(assignment,subject_revision='prior-intent')
        original = provider(ctx,['review-candidates'],{'show':ctx['show'](),'assignment':historical_assignment,
            'request':{'text':'Inspect the original intent before confirmation.'},
            'original':{'text':json.dumps({'assessment_id':'confirmation-assessment','judgments':[fresh(axes[0]['id'])]})}})['records'][0]
        # The exact attributed original remains stale until explicitly mapped.
        append(ctx,'review-evidence',original['data'],'prior-confirmation')
        carry = {'origin':{'kind':'context-record','id':'prior-confirmation'},
                 'target':{'subject':'intent.json','revision':revision,'checkpoint':None},
                 'attesting_driver':{'name':'confirmation-driver','kind':'agent'},
                 'reason':'The complete original intent axis scope is unchanged; current acceptance and operating context were inspected.'}
        negative_show = ctx['show']()
        rows = [{'axis':axes[0]['id'],'reuse':'wrong-origin'},fresh(axes[1]['id'])]
        packet = {'show':negative_show,'assignment':assignment,'request':{'text':'Confirm both axes.'},
                  'original':{'text':json.dumps({'assessment_id':'confirmation-assessment','judgments':rows})}}
        admitted = provider(ctx,['review-candidates'],packet)
        assert len(admitted['records']) == 1 and admitted['records'][0]['data']['axis'] == axes[1]['id'], admitted
        assert admitted['items'][0]['candidate'] is None, admitted
        assert 'wrong-origin' in json.dumps(admitted['items'][0]['diagnostics']), admitted
        for candidate in admitted['records']:
            append(ctx,candidate['kind'],candidate['data'],'fresh-confirmation-sibling')
        ledger(ctx,phase,'intent.json',revision)
        denied = evaluate(ctx,phase,'approved','design')
        assert denied['result'] == 'deny', denied
        assert axes[0]['id'] in json.dumps(denied), denied
        sibling_diagnostics = next(d for d in denied['feedback']['details']['diagnostics'] if d['axis'] == axes[1]['id'])
        assert all(d['category'] == 'independence' for d in sibling_diagnostics['diagnostics']), denied
        append(ctx,'evidence-applicability',carry,'valid-confirmation')
        fixed = intake([{'axis':axes[0]['id'],'reuse':'valid-confirmation'}])
        assert len(fixed['records']) == 1 and fixed['records'][0]['kind'] == 'evidence-applicability', fixed
        assert fixed['records'][0]['reuse_record_id'] == 'valid-confirmation', fixed
        assert fixed['records'][0]['data'] == carry, fixed
        for candidate in fixed['records']:
            append(ctx,candidate['kind'],candidate['data'])
        ctx['event']('approved')
        retained = ctx['show']()['result']['context']
        assert next(r for r in retained if r['id'] == 'fresh-confirmation-sibling')['data'] == admitted['records'][0]['data']
        return True
    return software_module().run(SimpleNamespace(engine=args.engine,provider=args.software,data_root=str(ROOT),mode='source'),
                                 ROOT / 'crates/software-change-provider/data/configs/high-rigor.json',root,
                                 configure=configure,exercise=hook)


# bookends:LE-150 — real plan rename/split maps original fulfilled obligations explicitly; an unmapped obligation denies until freshly fulfilled.
def reshaped_plan_applicability(args, root):
    state = {'reshaped':False}
    def configure(profile, repository, artifacts):
        for gate in profile['review_policies']:
            profile['review_policies'][gate] = []
        # Keep a review state available for the plan-owned re-entry.
        axis = json.loads((ROOT / 'crates/software-change-provider/data/configs/high-rigor.json').read_text())['review_policies']['validation-review']
        profile['review_policies']['validation-review'] = [copy.deepcopy(next(p for p in axis if p['review_stage'] == 'aggregate'))]
        profile['review_policies']['validation-review'][0]['required_authors'] = 1
    def hook(phase, ctx):
        if phase == 'validation-review' and not state['reshaped']:
            state['reshaped'] = True
            state['originals'] = copy.deepcopy(ctx['show']()['result']['context'])
            state['report'] = json.loads((ctx['artifacts'] / 'validation-report.json').read_text())
            plan_path = ctx['artifacts'] / 'plan.json'
            plan = json.loads(plan_path.read_text())
            state['prior_plan'] = copy.deepcopy(plan)
            ctx['event']('revise-plan')
            first = plan['tasks'][0]
            old_id = first['id']
            first['id'] = 'renamed-contract'
            split = copy.deepcopy(first)
            split.update(id='split-contract-proof',dependencies=['renamed-contract'])
            split['objective'] = 'Independently confirm the original contract proof after plan reshaping.'
            plan['tasks'].append(split)
            for task in plan['tasks']:
                task['dependencies'] = ['renamed-contract' if d == old_id else d for d in task['dependencies']]
            plan['revision'] = 'reshaped-plan-v2'
            plan_path.write_text(json.dumps(plan))
            implementation_path = ctx['artifacts'] / 'implementation-report.json'
            implementation = json.loads(implementation_path.read_text())
            implementation.update(revision='reshaped-implementation-v2',plan_revision=plan['revision'])
            implementation['summary'] += ' Original contract effects now satisfy renamed-contract and split-contract-proof; remaining task dependencies are preserved.'
            implementation_path.write_text(json.dumps(implementation))
            state['mapping'] = [
                {'original_task':task['id'],
                 'revised_tasks':['renamed-contract','split-contract-proof'] if task['id'] == old_id else [task['id']],
                 'criterion_ids':task['criterion_ids']}
                for task in state['prior_plan']['tasks']
            ]
            return True
        if phase == 'validation' and state['reshaped']:
            results = []
            plan = json.loads((ctx['artifacts'] / 'plan.json').read_text())
            for spec in plan['proof_commands']:
                identity = provider(ctx,['source-identity','--working-directory',str(ctx['repository'])],{})
                outcome = raw([spec['command'],*spec['args']],cwd=ctx['repository'])
                assert outcome.returncode == 0, outcome.stderr
                results.append({'id':spec['id'],'command':spec['command'],'args':spec['args'],
                    'cwd':str(ctx['repository']),'exit_code':outcome.returncode,'stdout':outcome.stdout,'stderr':outcome.stderr,
                    'producer':{'name':'reshape-proof-driver','kind':'script'},'source':identity})
            prepared = provider(ctx,['prepare-validation'],{'show':ctx['show'](),'working_directory':str(ctx['repository']),
                'revision':'reshaped-validation-v2','author':{'name':'reshape-proof-owner','kind':'script'},'command_results':results})
            report = prepared['report_draft']
            (ctx['artifacts'] / 'validation-report.json').write_text(json.dumps(report))
            ctx['checkpoint']('validation')
            for candidate in prepared['command_candidates']:
                append(ctx,candidate['kind'],candidate['data'],candidate['record_id'])
            for index, batch in enumerate(prepared['judgment_batches']):
                verdict = {'subject':'validation-report.json','subject_revision':report['revision'],
                    'checkpoint':'validation-checkpoint.json','author':{'name':f'reshape-independent-{index}','kind':'agent'},
                    'result':'pass','findings':[],'reason':'Fresh inspection of current helper command result.',
                    'evidence_context_ids':report['command_evidence_ids']}
                if batch['kind'] == 'criterion-verdict':
                    verdict['criterion_id'] = batch['criterion_id']
                append(ctx,batch['kind'],verdict,batch['record_id'])
            hook('validation-prepared',dict(ctx,report=report))
            ctx['event']('validation-ready')
            return True
        if phase == 'validation-prepared' and state['reshaped']:
            report = ctx['report']
            report['revision'] = 'reshaped-validation-v2'
            current = copy.deepcopy(ctx['show']()['result']['context'])
            pending = report['criteria'][-1]
            fresh_ids = pending['verdict_ids'][:]
            for criterion in report['criteria'][:-1]:
                original_ids = next(c for c in state['report']['criteria'] if c['criterion_id'] == criterion['criterion_id'])['verdict_ids']
                mapped_ids = []
                for original_id in original_ids:
                    carry_id = 'reshaped-'+original_id
                    reason = ('Explicit task mapping ' + json.dumps(state['mapping']) +
                              ': inspected actual helper(7)==7 effect, current tree, preserved dependencies and fresh helper-test proof; original ' +
                              criterion['criterion_id'] + ' judgment scope and meaning are unchanged. No standing is inferred from names or file equality.')
                    append(ctx,'evidence-applicability',{'origin':{'kind':'context-record','id':original_id},
                        'target':{'subject':'validation-report.json','subject_revision':report['revision'],
                                  'checkpoint':{'phase':'validation','report_revision':report['revision']}},
                        'attesting_driver':{'name':'plan-reshape-driver','kind':'agent'},'reason':reason},carry_id)
                    mapped_ids.append(carry_id)
                criterion['verdict_ids'] = mapped_ids
            pending['verdict_ids'] = ['unmapped-plan-obligation']
            # Whole-goal judgment is freshly supplied on the post-reshape tree.
            for source_id in report['goal_verdict_ids']:
                original = next(r for r in current if r['id'] == source_id)
                data = copy.deepcopy(original['data'])
                data['subject_revision'] = report['revision']
                append(ctx,'goal-verdict',data,'reshaped-'+source_id)
            report['goal_verdict_ids'] = ['reshaped-'+i for i in report['goal_verdict_ids']]
            (ctx['artifacts'] / 'validation-report.json').write_text(json.dumps(report))
            ctx['checkpoint']('validation')
            ledger(ctx,'validation-review','validation-report.json',report['revision'])
            policy = ctx['profile']['review_policies']['validation-review'][0]
            packet = {'show':ctx['show'](),'assignment':{'gate':'validation-review','stage':'aggregate','group':policy['group'],
                'author':{'name':'reshape-reviewer','kind':'agent'},'subject':'validation-report.json','subject_revision':report['revision']},
                'request':{'text':'Inspect the revised task mapping and current proof.'},
                'original':{'text':json.dumps({'judgments':[{'axis':policy['id'],'result':'pass','findings':'',
                    'grounds':{'reason':'Inspected explicit original-to-revised task scopes and the fresh helper proof.',
                              'evidence':[{'locator':'plan.json#/tasks'},{'locator':'validation-report.json#/criteria'}]}}]})}}
            for candidate in provider(ctx,['review-candidates'],packet)['records']:
                append(ctx,candidate['kind'],candidate['data'])
            denied = evaluate(ctx,'validation-review','passed','end')
            assert denied['result'] == 'deny' and 'unmapped-plan-obligation' in json.dumps(denied), denied
            fulfilled_ids = []
            for fresh_id in fresh_ids:
                fresh = next(r for r in current if r['id'] == fresh_id)
                data = copy.deepcopy(fresh['data'])
                data['subject_revision'] = report['revision']
                data['reason'] = 'Fresh independent inspection of the unmapped obligation and current helper-test result after plan reshaping.'
                append(ctx,'criterion-verdict',data,'fresh-reshaped-'+fresh_id)
                fulfilled_ids.append('fresh-reshaped-'+fresh_id)
            pending['verdict_ids'] = fulfilled_ids
            (ctx['artifacts'] / 'validation-report.json').write_text(json.dumps(report))
            ctx['checkpoint']('validation')
            allowed = evaluate(ctx,'validation-review','passed','end')
            assert allowed['result'] == 'allow', allowed
            retained = ctx['show']()['result']['context']
            for criterion in report['criteria'][:-1]:
                mapping = next(r for r in retained if r['id'] == criterion['verdict_ids'][0])
                assert json.dumps(state['mapping']) in mapping['data']['reason'], mapping
                origin = next(r for r in state['originals'] if r['id'] == mapping['data']['origin']['id'])
                assert next(r for r in retained if r['id'] == origin['id']) == origin
    return software_module().run(SimpleNamespace(engine=args.engine,provider=args.software,data_root=str(ROOT),mode='source'),
                                 ROOT / 'crates/software-change-provider/data/configs/high-rigor.json',root,
                                 configure=configure,exercise=hook)


# bookends:LE-167 — external policy/research judgments complete normally without an engine executor.
def provider_specific(args, root):
    import hashlib
    from external_work_driver import engine_call, record_reviews, declare_target_authorship
    spec = importlib.util.spec_from_file_location('research_fixture', ROOT / 'scripts/research-journey.py')
    research = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(research)
    spec = importlib.util.spec_from_file_location('policy_fixture', ROOT / 'scripts/policy-document-journey.py')
    policy_fixture = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(policy_fixture)
    for kind, binary in [('policy-document',args.policy),('research',args.research)]:
        work = root / (kind+'-specific')
        work.mkdir()
        artifacts = work / 'artifacts'
        artifacts.mkdir()
        owner = {'name':'owner','kind':'human'}
        if kind == 'policy-document':
            profile = json.loads((ROOT / 'crates/policy-document-provider/data/readme.json').read_text())
            target = work / 'README.md'
            target.write_text(policy_fixture.document())
            profile.update(mode='audit')
            profile['target']['path'] = str(target)
            profile['semantic_policies'] = profile['semantic_policies'][:2]
            policy = profile['semantic_policies'][0]
            gate, event, subject = 'semantic-review','passed',profile['target']['id']
            revision = hashlib.sha256(target.read_bytes()).hexdigest()
        else:
            profile = json.loads((ROOT / 'crates/research-provider/data/configs/standard.json').read_text())
            for gate in ['verify','synthesize']:
                profile['review_policies'][gate] = profile['review_policies'][gate][:2] if gate == 'verify' else [profile['review_policies'][gate][0]]
            policy = profile['review_policies']['verify'][0]
            gate, event, subject, revision = 'verify','verified','verification.json','1'
            for name, make in [('brief.json',research.brief),('sources.json',research.sources),('verification.json',research.verification)]:
                (artifacts / name).write_text(json.dumps(make()))
            target = artifacts / 'verification.json'
        grouped = profile['semantic_policies'] if kind == 'policy-document' else profile['review_policies']['verify']
        for member in grouped:
            member.update(required_authors=1,self_review=False,group='combined-public')
        profile['artifact_root'] = str(artifacts)
        selected = work / 'profile.json'
        selected.write_text(json.dumps(profile))
        config = work / 'providers.toml'
        config.write_text(f'[providers.{kind}]\ncommand = {json.dumps(binary)}\nargs = []\n')
        database = work / 'run.sqlite'
        run_id = kind+'-external-specific'
        def call(*argv):
            return engine_call(args.engine,database,config,list(argv))
        def show():
            return call('show','--view','full',run_id)
        def move(event):
            show()
            return call('event',run_id,event)
        def record(kind,data,id):
            with json_source(work, data) as source:
                result = call('append','--record-id',id,run_id,kind,source)
            if kind == 'review-evidence':
                retained = next(row for row in show()['result']['context'] if row['id'] == id)
                assert retained['data']['external_review'] == data['external_review'], retained
            return result
        call('start','--id',run_id,kind,'@'+str(selected),'provider-specific external evidence')
        if kind == 'policy-document':
            move('ready')
            move('passed')
        else:
            move('scoped')
            move('gathered')
        def intake(author):
            assignment_subject = revision if kind == 'policy-document' else subject
            packet = {'show':show(),'assignment':{'gate':gate,'review_stage':'ordinary','subject':assignment_subject,
                      'subject_revision':revision,'policy_ids':[p['id'] for p in grouped],'assessment_id':'combined-'+author['name']},'author':author,
                      'request':{'text':'Review the current target.'},'result':{'text':json.dumps({'items':[{'policy_id':p['id'],'result':'pass','findings':''} for p in grouped]})}}
            result = process([binary,'review-candidates'],packet)
            # bookends:LE-168 bookends:LE-170 — public combined assignment refusal and faithful correction.
            subset = copy.deepcopy(packet)
            subset['assignment']['policy_ids'] = [policy['id']]
            refused([binary,'review-candidates'],subset,needle='whole review group')
            corrected = copy.deepcopy(packet)
            corrected['projection'] = json.loads(packet['result']['text'])
            # bookends:LE-93 — canonical policy identity cannot contradict the retained original axis.
            alias_conflict = copy.deepcopy(packet)
            axis_original = json.loads(packet['result']['text'])
            for row in axis_original['items']:
                row['axis'] = row.pop('policy_id')
            alias_conflict['result']['text'] = json.dumps(axis_original)
            alias_conflict['projection'] = copy.deepcopy(axis_original)
            alias_conflict['projection']['items'][0]['policy_id'] = grouped[1]['id']
            before_alias = show()['result']['context']
            refused([binary,'review-candidates'],alias_conflict,needle='projection axis and policy_id disagree')
            # A prose original has no decoded rows; its projection still cannot name two obligations.
            prose_conflict = copy.deepcopy(packet)
            prose_conflict['result']['text'] = 'Prose review: every grouped obligation passes.'
            prose_conflict['projection'] = {'items':[{'axis':grouped[0]['id'],'policy_id':grouped[1]['id'],'result':'pass','findings':''}]
                                            +[{'policy_id':p['id'],'result':'pass','findings':''} for p in grouped[1:]]}
            refused([binary,'review-candidates'],prose_conflict,needle='projection axis and policy_id disagree')
            assert show()['result']['context'] == before_alias
            corrected['projection']['items'][0]['locator'] = 'corrected-locator'
            assert len(process([binary,'review-candidates'],corrected)['candidates']) == len(grouped)
            corrected['projection']['items'][0]['result'] = 'fail'
            refused([binary,'review-candidates'],corrected,needle='contradicts decoded result')
            corrected['projection']['items'][0]['result'] = 'pass'
            corrected['projection']['items'][0]['findings'] = 'invented finding'
            refused([binary,'review-candidates'],corrected,needle='contradicts decoded findings')
            failed_original = copy.deepcopy(packet)
            decoded = json.loads(failed_original['result']['text'])
            decoded['items'][0].update(result='fail',findings='material defect')
            failed_original['result']['text'] = json.dumps(decoded)
            failed_original['projection'] = json.loads(packet['result']['text'])
            refused([binary,'review-candidates'],failed_original,needle='contradicts decoded result')
            assert len(result['candidates']) == len(grouped), result
            for candidate in result['candidates']:
                exchange = candidate['data']['external_review']
                assert exchange['request']['original'] == packet['request']['text'], candidate
                assert exchange['original']['original'] == packet['result']['text'], candidate
            return [c['data'] for c in result['candidates']]
        if kind == 'policy-document':
            missing = process([binary, 'commission', json.dumps(profile)], show())
            assert missing['missing_inputs'][0]['input'] == 'target-authorship', missing
            # Complete independent reviews cannot bypass a missing authorship declaration.
            for i, data in enumerate(intake({'name':'external-independent','kind':'agent'})):
                record('review-evidence', data, 'missing-authorship-'+str(i))
            show()
            refused([args.engine,'--database',database,'--config',config,'--json','event',run_id,event],
                    needle='missing-target-authorship')
            declare_target_authorship(Path(args.engine), database, config, run_id, target,
                                      owner, 'authorship-original')
            commissioned = process([binary, 'commission', json.dumps(profile)], show())
            assert commissioned['missing_inputs'] == [], commissioned
            # A different actor rewrites the bytes; the former declaration is now stale.
            target.write_text(target.read_text()+'\n<!-- authored revision -->\n')
            revision = hashlib.sha256(target.read_bytes()).hexdigest()
            owner = {'name':'revision-author','kind':'agent'}
            for i, data in enumerate(intake(owner)):
                record('review-evidence', data, 'stale-authorship-'+str(i))
            show()
            refused([args.engine,'--database',database,'--config',config,'--json','event',run_id,event],
                    needle='missing-target-authorship')
            stale = process([binary, 'commission', json.dumps(profile)], show())
            assert stale['missing_inputs'][0]['input'] == 'target-authorship', stale
            declare_target_authorship(Path(args.engine), database, config, run_id, target,
                                      owner, 'authorship-revision')
            show()
            refused([args.engine,'--database',database,'--config',config,'--json','event',run_id,event],
                    needle='self-excluded')
        for i, data in enumerate(intake(owner)):
            record('review-evidence',data,'self-evidence-'+str(i))
        show()
        refused([args.engine,'--database',database,'--config',config,'--json','event',run_id,event],needle='review-incomplete')
        independent = intake({'name':'external-independent','kind':'agent'})
        for i, data in enumerate(independent):
            singleton = copy.deepcopy(data)
            singleton['assessment_id'] = 'isolated-'+str(i)
            record('review-evidence',singleton,'isolated-'+str(i))
        show()
        refused([args.engine,'--database',database,'--config',config,'--json','event',run_id,event],needle='review-incomplete')
        for i, data in enumerate(independent):
            record('review-evidence',data,'independent-evidence-'+str(i))
        def amend(after,id):
            data = {'targets':[{'gate':gate,'axis':policy['id']}],'after':after,
                    'actor':owner,'authority':{'kind':'owner','reference':'scripted instruction'},'reason':'External review budget.'}
            candidate = process([binary,'author-counts','--propose',json.dumps(data)],show())
            record(candidate['kind'],candidate['data'],id)
        amend(2,'increase')
        show()
        refused([args.engine,'--database',database,'--config',config,'--json','event',run_id,event],needle='review-incomplete')
        amend(1,'decrease')
        if kind == 'policy-document':
            target.write_text(target.read_text()+'\n<!-- immaterial fixture comment -->\n')
            revision = hashlib.sha256(target.read_bytes()).hexdigest()
            declare_target_authorship(Path(args.engine), database, config, run_id, target,
                                      owner, 'authorship-final-revision')
        else:
            value = json.loads(target.read_text())
            value['revision'] = '2'
            target.write_text(json.dumps(value))
            revision = '2'
        show()
        refused([args.engine,'--database',database,'--config',config,'--json','event',run_id,event],needle='review-incomplete')
        # bookends:LE-171 — an older carried PASS cannot erase this author's current FAIL.
        failure_packet = {'show':show(), 'assignment':{'gate':gate,'review_stage':'ordinary',
                          'subject':revision if kind == 'policy-document' else subject,
                          'subject_revision':revision,'policy_ids':[p['id'] for p in grouped],
                          'assessment_id':'current-failure'},
                          'author':{'name':'external-independent','kind':'agent'},
                          'request':{'text':'Review the current revision for remaining defects.'},
                          'result':{'text':json.dumps({'items':[{'policy_id':p['id'],'result':'fail',
                          'findings':'standing current-revision defect'} for p in grouped]})}}
        for i, candidate in enumerate(process([binary,'review-candidates'],failure_packet)['candidates']):
            record(candidate['kind'],candidate['data'],'current-failure-'+str(i))
        for i in range(len(grouped)):
            record('evidence-applicability',{'origin':{'kind':'context-record','id':'independent-evidence-'+str(i)},
                   'target':{'subject':subject,'revision':revision},'attesting_driver':owner,
                   'reason':'Only immaterial fixture metadata changed; judgment scope remains unchanged.'},'carry-independent-'+str(i))
        show()
        refused([args.engine,'--database',database,'--config',config,'--json','event',run_id,event],
                needle='standing current-revision defect')
        # Correct the defect and obtain fresh confirmation, preserving all historical records.
        if kind == 'policy-document':
            target.write_text(target.read_text()+'\n<!-- corrected current-revision fixture defect -->\n')
            revision = hashlib.sha256(target.read_bytes()).hexdigest()
            declare_target_authorship(Path(args.engine), database, config, run_id, target,
                                      owner, 'authorship-corrected')
        else:
            value = json.loads(target.read_text())
            value['revision'] = '3'
            target.write_text(json.dumps(value))
            revision = '3'
        for i, data in enumerate(intake({'name':'external-independent','kind':'agent'})):
            record('review-evidence',data,'fresh-confirmation-'+str(i))
        move(event)
        if kind == 'research':
            value = research.report()
            value['verification_revision'] = revision
            (artifacts / 'report.json').write_text(json.dumps(value))
            record_reviews(Path(args.engine),database,config,Path(binary),run_id,'synthesize',
                           [p['id'] for p in profile['review_policies']['synthesize']],'report.json','1')
            move('completed')
        terminal = show()['result']
        assert terminal['current_state'] == 'end' and terminal['lifecycle'] == 'final' and not terminal['has_overrides'], terminal
        print(f'external-work {kind} self-exclusion amendment applicability normal completion passed')



def decode_retained(context):
    """Every retained artifact entry, in order, with its exact bytes (None when absent)."""
    import base64
    by_id = {record['id']: record for record in context}
    entries = []
    for record in sorted((r for r in context if r['kind'] == 'retained-artifacts'), key=lambda r: r['sequence']):
        assert record['data']['schema_version'] == 1, record
        for entry in record['data']['artifacts']:
            data = None
            if not entry.get('absent'):
                source = entry
                if 'same_as' in entry:
                    holder = by_id[entry['same_as']]
                    source = next(x for x in holder['data']['artifacts'] if x['name'] == entry['name'])
                    assert 'content' in source, source
                    assert 'content' not in entry, entry
                data = source['content'].encode() if source['encoding'] == 'utf-8' else base64.b64decode(source['content'])
                assert 'sha256:'+hashlib.sha256(data).hexdigest() == entry['sha256'], entry
                assert len(data) == entry['size'], entry
            entries.append({'record': record['id'], 'sequence': record['sequence'], 'created_at': record['created_at'],
                            'transition': record['data']['transition'], 'exit': record['data']['exit'],
                            'name': entry['name'], 'bytes': data, 'same_as': entry.get('same_as')})
    return entries


def assert_retained_history(call, run_id, entries):
    """Each retained record is a context entry immediately followed by the transition that retained it."""
    history = call('history', run_id)['result']
    by_sequence = {entry['sequence']: entry for entry in history}
    for entry in entries:
        appended = by_sequence[entry['sequence']]['action']
        assert appended == {'kind': 'context_appended', 'context_record_id': entry['record']}, appended
        following = by_sequence[entry['sequence'] + 1]['action']
        assert following['kind'] == 'transition', following
        assert {k: following['transition'][k] for k in ('source', 'event', 'target')} == entry['transition'], following
        expected = 'overridden' if entry['exit'] == 'override' else 'committed'
        assert following['outcome']['outcome'] == expected, following
        assert by_sequence[entry['sequence']]['occurred_at'] == entry['created_at']


def retained_view(call, run_id):
    shown = call('show', '--view', 'full', run_id)
    entries = decode_retained(shown['result']['context'])
    assert_retained_history(call, run_id, entries)
    return entries


# bookends:LE-184 — research and policy-document runs retain each owning phase's exact artifact bytes, readable after the files are deleted.
def retention_provider_runs(args, root):
    import shutil
    from external_work_driver import engine_call
    expectations = {
        'research': [('scope', 'scoped', 'brief.json'), ('gather', 'gathered', 'sources.json'),
                     ('verify', 'verified', 'verification.json'), ('synthesize', 'completed', 'report.json')],
        'policy-document': [('semantic-review', 'passed', 'README.md')],
    }
    for kind, exits in expectations.items():
        work = root / (kind + '-specific')
        database, config, run_id = work / 'run.sqlite', work / 'providers.toml', kind + '-external-specific'
        def call(*argv):
            return engine_call(args.engine, database, config, list(argv))
        files = {'README.md': work / 'README.md'} if kind == 'policy-document' else {
            name: work / 'artifacts' / name for _, _, name in exits}
        before = retained_view(call, run_id)
        assert [(e['transition']['source'], e['transition']['event'], e['name']) for e in before] == exits, before
        for entry in before:
            assert entry['exit'] == 'checked' and entry['same_as'] is None, entry
            assert entry['bytes'] == files[entry['name']].read_bytes(), entry['name']
        shutil.rmtree(work / 'artifacts')
        if kind == 'policy-document':
            files['README.md'].unlink()
        after = retained_view(call, run_id)
        assert [(e['name'], e['bytes'], e['transition']) for e in after] == [(e['name'], e['bytes'], e['transition']) for e in before]
        print(f'external-work {kind} retained artifacts readable after file deletion passed')


class _StopRun(Exception):
    pass


def _empty_review_policies(profile, repository, artifacts):
    for gate in profile['review_policies']:
        profile['review_policies'][gate] = []


# bookends:LE-184 — software-change retains only phase-exit bytes: drafts excluded, revise-loop re-exits appended, unchanged re-exits stored once and still readable after deletion.
def retention_software_revisions(args, root):
    import shutil
    visits = {'implement': 0}
    snapshots = {}
    drafts = []
    def hook(phase, ctx):
        plan_path = ctx['artifacts'] / 'plan.json'
        if phase == 'plan':
            if visits['implement'] == 2:
                value = json.loads(plan_path.read_text())
                for draft in ['draft one', 'draft two']:
                    value['objective'] = 'Retention journey ' + draft
                    plan_path.write_text(json.dumps(value))
                    drafts.append(plan_path.read_bytes())
                value['objective'] = 'Retention journey final plan'
                plan_path.write_text(json.dumps(value))
            snapshots.setdefault('plan', []).append(plan_path.read_bytes())
            return False
        if phase == 'implement':
            visits['implement'] += 1
            if visits['implement'] <= 2:
                ctx['event']('revise-plan')
                return True
        return False
    work = software_module().run(SimpleNamespace(engine=args.engine, provider=args.software, mode='source', data_root=None),
                                 ROOT / 'crates/software-change-provider/data/configs/minimal.json', root,
                                 configure=_empty_review_policies, exercise=hook)
    database, config, run_id = work / 'run.sqlite', work / 'providers.toml', 'software-external'
    from external_work_driver import engine_call
    def call(*argv):
        return engine_call(args.engine, database, config, list(argv))
    artifacts = work / 'artifacts'
    entries = retained_view(call, run_id)
    names = [e['name'] for e in entries]
    assert names == ['intent.json', 'design.json', 'plan.json', 'plan.json', 'plan.json',
                     'implementation-report.json', 'reconciliation.json', 'validation-report.json'], names
    plans = [e for e in entries if e['name'] == 'plan.json']
    assert [e['bytes'] for e in plans] == snapshots['plan'], 'plan exits must retain the bytes present at each exit'
    assert plans[1]['same_as'] == plans[0]['record'], plans[1]
    assert plans[2]['same_as'] is None and plans[2]['bytes'] != plans[0]['bytes']
    retained_bytes = {e['bytes'] for e in entries}
    assert drafts and not any(draft in retained_bytes for draft in drafts), 'intermediate drafts must not be retained'
    for entry in entries:
        assert entry['exit'] == 'checked'
        if entry['name'] != 'plan.json':
            assert entry['bytes'] == (artifacts / entry['name']).read_bytes(), entry['name']
    assert entries[-1]['transition']['target'] == 'end'
    shutil.rmtree(artifacts)
    after = retained_view(call, run_id)
    assert [(e['name'], e['bytes']) for e in after] == [(e['name'], e['bytes']) for e in entries]
    print('external-work software-change retained exits, drafts, revise loop and unchanged re-exit passed')
    return work


# bookends:LE-184 — owner overrides retain what the run leaves with, visibly marked, and record a missing artifact as absent.
def retention_software_overrides(args, root):
    captured = {}
    def override(ctx, event, reason):
        shown = ctx['show']()
        attestation = json.dumps({'state_visit': shown['result']['state_visit'], 'owner': 'journey-owner', 'reason': reason})
        response = ctx['call']('event', '--override', attestation, ctx['run_id'], event)
        assert response['status'] == 'completed', response
    def hook(phase, ctx):
        if phase == 'explore':
            captured['intent'] = (ctx['artifacts'] / 'intent.json').read_bytes()
            override(ctx, 'intent-ready', 'Journey override with the artifact present.')
            return True
        if phase == 'design':
            design = ctx['artifacts'] / 'design.json'
            saved = design.read_bytes()
            design.unlink()
            override(ctx, 'design-ready', 'Journey override with the artifact missing.')
            design.write_bytes(saved)
            captured['ctx'] = ctx
            raise _StopRun()
        return False
    try:
        software_module().run(SimpleNamespace(engine=args.engine, provider=args.software, mode='source', data_root=None),
                              ROOT / 'crates/software-change-provider/data/configs/minimal.json', root,
                              configure=_empty_review_policies, exercise=hook)
    except _StopRun:
        pass
    ctx = captured['ctx']
    entries = retained_view(ctx['call'], ctx['run_id'])
    assert [(e['name'], e['exit']) for e in entries] == [('intent.json', 'override'), ('design.json', 'override')], entries
    assert entries[0]['bytes'] == captured['intent']
    assert entries[1]['bytes'] is None
    shown = ctx['show']()['result']
    assert shown['has_overrides'] and shown['override_count'] == 2, shown
    print('external-work software-change override retention and absent artifact passed')


# bookends:LE-184 — a shipped minimal profile with review states retains each phase exactly once at its real exit.
def retention_minimal_profile(args, work):
    from external_work_driver import engine_call
    def call(*argv):
        return engine_call(args.engine, work / 'run.sqlite', work / 'providers.toml', list(argv))
    entries = retained_view(call, 'software-external')
    assert [e['name'] for e in entries] == ['intent.json', 'design.json', 'plan.json', 'implementation-report.json',
                                            'reconciliation.json', 'validation-report.json'], entries
    for entry in entries:
        assert entry['exit'] == 'checked' and entry['bytes'] == (work / 'artifacts' / entry['name']).read_bytes(), entry['name']
    print('external-work software-change minimal profile retains each phase exit passed')


def _seed_legacy_invocations(database, run_id, rows, payload_bytes):
    """Insert legacy-format work-slot invocation rows with bulky payloads, as earlier releases wrote them."""
    worker = {'assignment_id': 'worker-0', 'command': '/legacy/bin/pi',
              'args': ['--print', 'x' * payload_bytes], 'exit_code': 0, 'attempts': []}
    connection = sqlite3.connect(database, timeout=30)
    with connection:
        for index in range(rows):
            inner = json.dumps([worker])
            connection.execute(
                """INSERT INTO work_slot_invocations (run_id, invocation_id, slot_id, binding_json,
                       instruction_digest, subject, state_visit, waiter_pid, started_at, allowed_time_ms,
                       status, exit_code, completed_at, capture_dir, inner_workers_json, assignment_selection_json,
                       invocation_input_json, routed_inputs_json, frozen_run_identity_json, completion_snapshot_json)
                   VALUES (?, ?, 'legacy-slot', ?, 'sha256:legacy', 'legacy-subject', 0, 0, 1, 1000,
                           'succeeded', 0, 2, '/legacy/capture', ?, NULL, ?, ?, NULL, ?)""",
                (run_id, f'legacy-{run_id}-{index}-{time.time_ns()}',
                 json.dumps({'command': '/legacy/bin/pi', 'args': ['--print']}),
                 inner, json.dumps({'prompt': 'p' * 1000}), '[]', inner))
    connection.close()


def _invocation_columns(database, run_id):
    connection = sqlite3.connect(database, timeout=30)
    try:
        return connection.execute(
            """SELECT invocation_id, binding_json, inner_workers_json, completion_snapshot_json, routed_inputs_json,
                      invocation_input_json, assignment_selection_json, frozen_run_identity_json
               FROM work_slot_invocations WHERE run_id = ? ORDER BY invocation_id""", (run_id,)).fetchall()
    finally:
        connection.close()


def _file_digest(path):
    # A missing write-ahead log and an empty one hold the same catalog content.
    return hashlib.sha256(path.read_bytes() if path.exists() else b'').hexdigest()


# bookends:LE-185 — explicit reclaim previews without writing, clears only finished runs' legacy payloads, keeps cleaned runs listed and showable, leaves active runs advancing, and shrinks the catalog file.
def reclaim_legacy_catalog(args, root, retained_work):
    import shutil, signal
    from external_work_driver import engine_call
    work = root / 'reclaim'
    work.mkdir()
    database = work / 'loop.db'
    shutil.copyfile(retained_work / 'run.sqlite', database)
    config = retained_work / 'providers.toml'
    finished = 'software-external'
    def call(*argv):
        return engine_call(args.engine, database, config, list(argv))
    def engine(*argv):
        return raw([args.engine, '--database', database, '--config', config, '--json', *argv])
    profile = json.loads((retained_work / 'profile.json').read_text())
    active_artifacts = work / 'active-artifacts'
    active_artifacts.mkdir()
    shutil.copyfile(retained_work / 'profile.json', work / 'active-profile.json')
    fixtures = ROOT / 'crates/software-change-provider/data/calibration/fixtures'
    for target, source in software_module().SUBJECTS.items():
        shutil.copyfile(fixtures / source, active_artifacts / target)
    profile['artifact_root'] = str(active_artifacts)
    (work / 'active-profile.json').write_text(json.dumps(profile))
    call('start', '--id', 'active-run', 'software-change', '@'+str(work / 'active-profile.json'), 'active during reclaim')
    call('start', '--id', 'terminated-run', 'software-change', '@'+str(work / 'active-profile.json'), 'terminated before reclaim')
    call('show', 'terminated-run', '--view', 'action')
    assert call('terminate', 'terminated-run')['status'] == 'completed'
    for run_id in [finished, 'terminated-run', 'active-run']:
        _seed_legacy_invocations(database, run_id, 4, 400_000)
    retained_before = retained_view(call, finished)
    for run_id in [finished, 'terminated-run', 'active-run']:
        assert call('show', '--view', 'full', run_id)['status'] == 'completed'
    active_before = _invocation_columns(database, 'active-run')
    digest_before = (_file_digest(database), _file_digest(Path(str(database)+'-wal')))
    preview = call('reclaim')
    assert preview['status'] == 'completed' and preview['result']['mode'] == 'preview', preview
    listed = {run['run_id']: run for run in preview['result']['preview']['runs']}
    assert set(listed) == {finished, 'terminated-run'}, listed
    assert all(run['invocation_rows'] == 4 and run['reclaimable_bytes'] > 3_000_000 for run in listed.values()), listed
    assert (_file_digest(database), _file_digest(Path(str(database)+'-wal'))) == digest_before, 'preview must not write'
    human = raw([args.engine, '--database', database, 'reclaim'])
    assert human.returncode == 0 and 'nothing written' in human.stdout, human
    print('external-work reclaim preview lists only finished runs and writes nothing passed')
    applied = call('reclaim', '--apply')
    assert applied['status'] == 'completed', applied
    result = applied['result']['applied']
    assert result['invocation_rows_cleared'] == 8, result
    assert result['after']['file_bytes'] < result['before']['file_bytes'] - 3_000_000, result
    assert database.stat().st_size == result['after']['file_bytes']
    for run_id in [finished, 'terminated-run']:
        for row in _invocation_columns(database, run_id):
            assert row[2:] == ('[]', '[]', '[]', None, None, None), row
            assert json.loads(row[1])['command'], row
    assert _invocation_columns(database, 'active-run') == active_before, 'active run rows must be unchanged'
    listed_runs = {run['id'] for run in call('list')['result']}
    assert {finished, 'terminated-run', 'active-run'} <= listed_runs, listed_runs
    retained_after = retained_view(call, finished)
    assert [(e['name'], e['bytes']) for e in retained_after] == [(e['name'], e['bytes']) for e in retained_before]
    call('show', 'active-run', '--view', 'action')
    assert call('event', 'active-run', 'intent-ready')['status'] == 'completed'
    assert sqlite3.connect(database).execute('PRAGMA integrity_check').fetchone() == ('ok',)
    print('external-work reclaim apply clears finished runs only, keeps them showable and shrinks the file passed')

    # Concurrent driver command while reclaim rewrites the file: it completes or fails with a retryable busy error.
    _seed_legacy_invocations(database, 'terminated-run', 6, 2_000_000)
    reclaim_process = subprocess.Popen([args.engine, '--database', str(database), '--json', 'reclaim', '--apply'],
                                       stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    concurrent = engine('show', 'active-run', '--view', 'full')
    reclaim_out, _ = reclaim_process.communicate(timeout=300)
    assert reclaim_process.returncode == 0, reclaim_out
    if concurrent.returncode != 0:
        assert any(word in concurrent.stdout + concurrent.stderr for word in ['locked', 'busy']), concurrent
    assert call('show', '--view', 'full', 'active-run')['status'] == 'completed'
    print('external-work reclaim alongside a concurrent driver command passed')

    # Another process holds the catalog's write lock: reclaim stops with a clear busy error and changes nothing.
    _seed_legacy_invocations(database, 'terminated-run', 1, 100_000)
    holder = sqlite3.connect(database, timeout=30, isolation_level=None)
    holder.execute('BEGIN IMMEDIATE')
    busy = raw([args.engine, '--database', database, '--json', 'reclaim', '--apply'])
    holder.execute('ROLLBACK')
    holder.close()
    assert busy.returncode != 0, busy
    busy_issue = json.loads(busy.stdout)
    assert 'catalog-busy' in json.dumps(busy_issue) and 'retry' in json.dumps(busy_issue), busy_issue
    assert any(row[2] != '[]' for row in _invocation_columns(database, 'terminated-run')), 'busy reclaim must not clear rows'
    print('external-work reclaim reports a held catalog as busy passed')

    # Killed mid-apply: the catalog stays consistent and usable, and reclaim can simply be run again.
    _seed_legacy_invocations(database, 'terminated-run', 6, 2_000_000)
    victim = subprocess.Popen([args.engine, '--database', str(database), '--json', 'reclaim', '--apply'],
                              stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(0.3)
    victim.send_signal(signal.SIGKILL)
    victim.wait()
    assert sqlite3.connect(database).execute('PRAGMA integrity_check').fetchone() == ('ok',)
    assert {finished, 'terminated-run', 'active-run'} <= {run['id'] for run in call('list')['result']}
    assert call('show', '--view', 'full', 'active-run')['status'] == 'completed'
    rerun = call('reclaim', '--apply')
    assert rerun['status'] == 'completed', rerun
    assert all(row[2] == '[]' for row in _invocation_columns(database, 'terminated-run'))
    print('external-work reclaim interrupted by SIGKILL leaves a consistent catalog passed')

    # Storage runs out while the catalog is rewritten. A POSIX file-size limit makes the real SQLite write path
    # report full storage deterministically; filling an actual volume needs root or platform disk images.
    _seed_legacy_invocations(database, 'active-run', 4, 2_000_000)
    _seed_legacy_invocations(database, 'terminated-run', 1, 50_000)
    call('show', 'active-run', '--view', 'action')
    sqlite3.connect(database).execute('PRAGMA wal_checkpoint(TRUNCATE)').fetchone()
    limit = database.stat().st_size + 2048
    tmp = work / 'tmp'
    tmp.mkdir()
    def constrain():
        import resource
        signal.signal(signal.SIGXFSZ, signal.SIG_IGN)
        resource.setrlimit(resource.RLIMIT_FSIZE, (limit, limit))
    environment = dict(os.environ, TMPDIR=str(tmp), SQLITE_TMPDIR=str(tmp))
    full = subprocess.run([str(args.engine), '--database', str(database), '--json', 'reclaim', '--apply'],
                          text=True, capture_output=True, env=environment, preexec_fn=constrain)
    assert full.returncode != 0, full.stdout
    full_issue = json.dumps(json.loads(full.stdout))
    assert 'catalog-storage-full' in full_issue and 'storage' in full_issue, full_issue
    assert sqlite3.connect(database).execute('PRAGMA integrity_check').fetchone() == ('ok',)
    assert {finished, 'terminated-run', 'active-run'} <= {run['id'] for run in call('list')['result']}
    call('show', 'active-run', '--view', 'action')
    assert call('append', 'active-run', 'user-steering', json.dumps({'target': {'kind': 'all'}, 'instruction': 'after full storage'}))['status'] == 'completed'
    assert call('reclaim', '--apply')['status'] == 'completed'
    print('external-work reclaim reports full storage clearly and leaves the catalog usable passed')


# bookends:LE-46 bookends:LE-48 bookends:LE-49 bookends:LE-50 bookends:LE-52 bookends:LE-54 — durable denial, provider-unavailable revision and later superseding approval.
def revision_and_observation(args, root):
    state = {'revised':False, 'observed':False}
    def hook(phase, ctx):
        if phase == 'explore' and not state['observed']:
            state['observed'] = True
            before = ctx['show']()['result']
            # bookends:LE-146 — a locked source reports unavailability, never arms mutation, and completes in less than two seconds.
            lock = sqlite3.connect(ctx['database'])
            lock.execute('PRAGMA journal_mode=DELETE')
            lock.execute('BEGIN EXCLUSIVE')
            try:
                started = time.monotonic()
                observed = raw([ctx['engine'],'--database',ctx['database'],'--json','show','--view','status',ctx['run_id']])
                assert time.monotonic() - started < 2, observed
                assert observed.returncode == 0, observed.stderr
                status = json.loads(observed.stdout)['result']
                assert status['uncertainty']['state'] == 'unavailable' and status['mutation_armed'] is False, status
            finally:
                lock.rollback()
                lock.close()
            assert ctx['show']()['result']['current_state'] == before['current_state']
        if phase == 'intent-review' and not state['revised']:
            state['revised'] = True
            refusal = refused([ctx['engine'],'--database',ctx['database'],'--config',ctx['config'],'--json',
                               'event',ctx['run_id'],'approved'],cwd=ctx['repository'])
            denied = ctx['show']()['result']
            assert denied['current_state'] == 'intent-review'
            assert 'deny' in json.dumps(denied['latest_evaluations']), denied
            assert 'missing' in refusal.stdout.lower(), refusal.stdout
            passive_views(ctx)
            policies = denied['initial_input']['review_policies']
            append(ctx,'driver-note',{'reason':'Retain denial for the next external reviewer.'},'denial-handoff')
            unavailable = ctx['config'].with_suffix('.unavailable')
            ctx['config'].rename(unavailable)
            try:
                ctx['event']('revise')
                resumed = ctx['show']()['result']
                assert resumed['current_state'] == 'explore'
                assert resumed['initial_input']['review_policies'] == policies
                assert resumed['latest_evaluations'] == denied['latest_evaluations']
                assert any(r['id'] == 'denial-handoff' for r in resumed['context'])
            finally:
                unavailable.rename(ctx['config'])
            return True
        if phase == 'end':
            shown = ctx['show']()['result']
            latest = [row for row in shown['latest_evaluations'] if row['transition']['source'] == 'intent-review']
            assert len(latest) == 1 and latest[0]['result']['result'] == 'allow', latest
            assert any(row['result']['result'] == 'deny' for row in shown['evaluation_history']), shown
            assert state['revised'] and not shown['has_overrides']
            print('external-work durable review revision and bounded observation passed')
    return software_module().run(SimpleNamespace(engine=args.engine,provider=args.software,data_root=str(ROOT),mode='source'),
                                 ROOT / 'crates/software-change-provider/data/configs/minimal.json',root,exercise=hook)


# bookends:LE-124 — separate immutable pointer generations match complete reviewed content, preserve pending hosted facts, and refuse changed/missing content.
def delivery_proof(args, output_root):
    root = output_root / "delivery"
    root.mkdir()
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



# bookends:LE-132 bookends:LE-160 bookends:LE-180 — compiled setup exports inspectable standalone effective policy with small explicit base-derived overrides and visible newer-base drift.
def profile_preparation(args, root):
    work = root / 'profile-preparation'
    work.mkdir()
    for rigor in ['minimal','standard','high']:
        output = work / (rigor+'.json')
        profile = process([args.software,'setup','--rigor',rigor,'--output',output,'--decline-advice'])
        assert profile == json.loads(output.read_text())
        explain = json.loads(Path(str(output)+'.explain.json').read_text())
        assert explain['effective']['review_policies'] == profile['review_policies']
        assert explain['base']['sha256'].startswith('sha256:') and explain['base']['id'] == profile['config_version']
        assert not profile.get('work_slot_bindings') and not profile.get('advice_command')
        workflow = process([args.software],{'operation':'describe','initial_input':profile})
        # bookends:LE-119 bookends:LE-169 — new semantic identity is explicit, and useful plan/departure instructions do not require an engine executor or per-task isolation.
        assert profile['contract_version'] == 4
        implement = next(state for state in workflow['states'] if state['id'] == 'implement')
        assert 'driver/harness owns any external work' in implement['instructions']
        assert 'cancel-invocation' not in implement['instructions']
        plan = next(state for state in workflow['states'] if state['id'] == 'plan')
        assert 'group and execute work freely' in plan['instructions']
        # bookends:LE-141 bookends:LE-144 bookends:LE-175 — each shipped ordinary/challenge profile retains distinct externally judged granularity, comprehension, final-state and promised-outcome questions, not mechanical proxies.
        for gate in ['intent-review','intent-adversarial-review']:
            axes = {a['id']:a for a in profile['review_policies'][gate]}
            assert {'acceptance-granularity','owner-comprehensible','intent-sufficient','acceptance-sufficient'} <= axes.keys(), axes
            assert 'final state' in axes['intent-sufficient']['description'].replace('-', ' ')
            assert 'counterexample' in axes['acceptance-sufficient']['description']
    base_path = work / 'standard.json'
    base = json.loads(base_path.read_text())
    axis = base['review_policies']['intent-review'][0]['id']
    output = work / 'custom.json'
    options = ['--set',f'review.intent-review.aggregate.{axis}.required_authors=1',
               '--set',f'review.intent-review.aggregate.{axis}.self_review=true',
               '--set',f'review.intent-review.aggregate.{axis}.group=custom-ordinary',
               '--set','criterion.required_authors=1','--set','criterion.self_review=true',
               '--set','goal.required_authors=1','--set','goal.self_review=true',
               '--set','bookends.enabled=true',
               '--add-axis',f'intent-review.individual.{axis}=intent-review.aggregate.{axis}',
               '--set',f'review.intent-review.individual.{axis}.group=custom-individual']
    custom = process([args.software,'setup','--profile',base_path,'--output',output,'--enable-advice',*options])
    rows = [a for a in custom['review_policies']['intent-review'] if a['id'] == axis]
    assert {a['review_stage'] for a in rows} == {'individual','aggregate'}
    assert {a['group'] for a in rows} == {'custom-ordinary','custom-individual'}
    assert all(a['required_authors'] == 1 and a['self_review'] for a in rows)
    assert custom['criterion_policy']['required_authors'] == custom['criterion_policy']['goal_required_authors'] == 1
    assert custom['extra']['advice']['enabled'] and custom['extra']['bookends']['enabled']
    explain_path = Path(str(output)+'.explain.json')
    explain = json.loads(explain_path.read_text())
    assert explain['base']['sha256'] == 'sha256:'+hashlib.sha256(base_path.read_bytes()).hexdigest()
    changed = copy.deepcopy(base)
    changed['review_policies']['intent-review'][0]['required_authors'] = 3
    changed['review_policies']['design-review'][0]['description'] = 'New selected base default'
    newer = work / 'newer-base.json'
    newer.write_text(json.dumps(changed))
    refreshed = process([args.software,'setup','--profile',newer,'--output',work / 'refreshed.json','--enable-advice',
                         '--previous-explain',explain_path,*options])
    drift = json.loads((work / 'refreshed.json.explain.json').read_text())['drift']
    assert any(d['key'].endswith('.required_authors') and d['base_value'] == 3 for d in drift), drift
    assert refreshed['review_policies']['design-review'][0]['description'] == 'New selected base default'
    refused([args.software,'setup','--profile',newer,'--output',work / 'bad.json','--decline-advice',
             '--set','review.intent-review.aggregate.no-longer-present.required_authors=1'])
    for provider, semantic, bases in [(args.policy, True, ['readme','agents']),
                                       (args.research, False, ['standard','generate-prd'])]:
        for base_name in bases:
            prefix = ('policy-' if semantic else 'research-') + base_name
            selected = work / (prefix+'.json')
            base = process([provider,'setup','--rigor',base_name,'--output',selected])
            gate = 'semantic-review' if semantic else 'verify'
            def rows(profile):
                return profile['semantic_policies'] if semantic else profile['review_policies'][gate]
            axis = rows(base)[0]['id']
            options = ['--set',f'review.{gate}.{axis}.required_authors=2',
                       '--set',f'review.{gate}.{axis}.self_review=true',
                       '--set',f'review.{gate}.{axis}.group=custom',
                       '--add-axis',f'{gate}.added={gate}.{axis}']
            custom_path = work / (prefix+'-custom.json')
            custom = process([provider,'setup','--profile',selected,'--output',custom_path,*options])
            assert custom == json.loads(custom_path.read_text())
            assert rows(custom)[0]['required_authors'] == 2 and rows(custom)[0]['self_review']
            assert rows(custom)[0]['group'] == 'custom' and rows(custom)[-1]['id'] == 'added'
            explain_path = Path(str(custom_path)+'.explain.json')
            explain = json.loads(explain_path.read_text())
            assert explain['effective'] == custom
            assert explain['base']['sha256'] == 'sha256:'+hashlib.sha256(selected.read_bytes()).hexdigest()
            changed = copy.deepcopy(base)
            rows(changed)[0]['required_authors'] = 3
            rows(changed)[1]['description'] = 'New selected base default'
            newer = work / (prefix+'-newer.json')
            newer.write_text(json.dumps(changed))
            refreshed_path = work / (prefix+'-refreshed.json')
            refreshed = process([provider,'setup','--profile',newer,'--output',refreshed_path,
                                 '--previous-explain',explain_path,*options])
            drift = json.loads(Path(str(refreshed_path)+'.explain.json').read_text())['drift']
            assert any(d['key'].endswith('.required_authors') and d['base_value'] == 3 for d in drift)
            assert rows(refreshed)[1]['description'] == 'New selected base default'
            removed = process([provider,'setup','--profile',custom_path,'--output',work/(prefix+'-removed.json'),
                               '--remove-axis',f'{gate}.added'])
            assert all(row['id'] != 'added' for row in rows(removed))
            for setting in ['required_authors=0','self_review=maybe','group=""']:
                refused([provider,'setup','--profile',selected,'--output',work/(prefix+'-bad.json'),
                         '--set',f'review.{gate}.{axis}.{setting}'])
            refused([provider,'setup','--profile',newer,'--output',work/(prefix+'-missing.json'),
                     '--set',f'review.{gate}.no-longer-present.required_authors=1'])
    print('external-work base-derived profile preparation and drift passed')



# bookends:LE-92 bookends:LE-115 bookends:LE-157 — inert proposals, exact-source dispositions, unresolved refusal and unchanged original failing judgments.
def findings_and_steering(args, root):
    state = {}
    def configure(profile, repository, artifacts):
        axis = copy.deepcopy(profile['review_policies']['intent-review'][0])
        axis['required_authors'] = 2
        for gate in profile['review_policies']:
            profile['review_policies'][gate] = []
        profile['review_policies']['intent-review'] = [axis]
    def hook(phase, ctx):
        if phase == 'explore':
            baseline = (ctx['artifacts'] / 'intent.json').read_text()
            revision = json.loads(baseline)['revision']
            # The actual prior intent bytes and digest remain alongside driver instructions.
            append(ctx,'user-steering',{'target':{'kind':'slots','ids':['design-draft']},
                   'instruction':'Include the externally requested design marker: owner-directed-observation.',
                   'intent_baseline':{'revision':revision,'locator':str(ctx['artifacts'] / 'intent.json'),
                                      'sha256':'sha256:'+hashlib.sha256(baseline.encode()).hexdigest(),'json_bytes':baseline}},'design-only')
            state['baseline'] = baseline
            unrelated = provider(ctx,['commission','--slot','intent-draft'],ctx['show']())
            assert 'design-only' not in unrelated['commission']['steering_ids'], unrelated
        if phase == 'intent-review':
            policy = ctx['profile']['review_policies']['intent-review'][0]
            subject = json.loads((ctx['artifacts'] / 'intent.json').read_text())
            originals = {}
            findings = []
            def snapshot():
                append(ctx,'finding-ledger',{'schema_version':'1','gate':'intent-review','subject':'intent.json',
                       'subject_revision':subject['revision'],'author':{'name':'driver','kind':'agent'},'findings':findings})
            def denied(needle):
                ctx['show']()
                result = refused([ctx['engine'],'--database',ctx['database'],'--config',ctx['config'],'--json','event',ctx['run_id'],'approved'],cwd=ctx['repository'])
                assert needle in result.stdout, result.stdout
                return result
            for index, author in enumerate(['alice','bob']):
                packet = {'show':ctx['show'](),'assignment':{'gate':'intent-review','stage':'aggregate','group':policy['group'],
                          'author':{'name':author,'kind':'agent'},'subject':'intent.json','subject_revision':subject['revision']},
                          'request':{'text':'Inspect the current named obligation.'},'original':{'text':json.dumps({'judgments':[
                          {'axis':policy['id'],'result':'fail','findings':'Scripted concern about the exact current outcome.',
                           'grounds':{'reason':'The reviewer supplied this bounded concern.','evidence':[{'locator':'intent.json#/outcome'}]}}]})}}
                candidate = provider(ctx,['review-candidates'],packet)['records'][0]
                record_id = 'failed-'+author
                append(ctx,candidate['kind'],candidate['data'],record_id)
                originals[record_id] = candidate['data']
                finding = {'id':'F-'+author,'source':{'kind':'context-record','id':record_id},'policy_id':policy['id'],
                           'statement':candidate['data']['findings'],'disposition':'rejected' if index == 0 else 'accepted',
                           'reason':'Driver inspected the supplied source and consequence.','owner_phase':None if index == 0 else 'implementation',
                           'task_ids':[],'review_axes':[],'status':'recorded' if index == 0 else 'unresolved'}
                if index == 0:
                    append(ctx,'advisory-finding-proposal',finding)
                    snapshot()
                    denied('failed-alice')
                    findings.append(finding)
                    snapshot()
                    denied('independence')
                else:
                    findings.append(finding)
                    snapshot()
                    denied('accepted_unresolved')
                    findings[-1]['status'] = 'resolved'
                    findings[-1]['reason'] = 'Driver checked the exact concern against the unchanged observable outcome; no defect remains.'
                    snapshot()
            allowed = evaluate(ctx,'intent-review','approved','design')
            assert allowed['result'] == 'allow', allowed
            ctx['event']('approved')
            # bookends:LE-100 — original external records remain visible with their identity/content after driver dispositions, without an execution registry.
            for row in ctx['show']()['result']['context']:
                if row['id'] in originals:
                    assert row['data'] == originals[row['id']] and row['data']['result'] == 'fail'
            return True
        if phase == 'design':
            # bookends:LE-114 — only the named later commission receives steering, and the external actor changes the authored work accordingly.
            commission = provider(ctx,['commission','--slot','design-draft'],ctx['show']())
            assert commission['commission']['steering_ids'] == ['design-only'], commission
            original = next(row for row in ctx['show']()['result']['context'] if row['id'] == 'design-only')
            assert original['data']['intent_baseline']['json_bytes'] == state['baseline']
            design_path = ctx['artifacts'] / 'design.json'
            design = json.loads(design_path.read_text())
            design['approach'] += ' owner-directed-observation.'
            design_path.write_text(json.dumps(design))
            assert 'owner-directed-observation' in json.loads(design_path.read_text())['approach']
        if phase == 'end':
            assert not ctx['show']()['result']['has_overrides']
            print('external-work exact-source findings and recipient steering passed')
    return software_module().run(SimpleNamespace(engine=args.engine,provider=args.software,data_root=str(ROOT),mode='source'),
                                 ROOT / 'crates/software-change-provider/data/configs/minimal.json',root,configure=configure,exercise=hook)



# bookends:LE-143 — real changed packets and unchanged stdout silence describe the assistant-owned posting checkpoint; conversation delivery remains external evidence.
def owner_update_proof(ctx):
    import queue
    import threading
    skill = (ROOT / 'skills/using-loop-engine/SKILL.md').read_text()
    for instruction in ['coordinating assistant', 'concise source-backed owner update',
                        'active Pi conversation', 'meaningful observed change',
                        'before the next wait, inspect, or owner-help decision',
                        'stay quiet for unchanged observations']:
        assert instruction in skill, instruction
    before = ctx['call']('history',ctx['run_id'])
    packets = queue.Queue()
    child = subprocess.Popen([ctx['engine'],'monitor','--run',ctx['run_id'],
                              '--database',str(ctx['database']),'--json','--poll-seconds','0.05'],
                             stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
    def collect():
        for line in child.stdout:
            packets.put(json.loads(line))
    reader = threading.Thread(target=collect,daemon=True)
    reader.start()
    def receive():
        try:
            return packets.get(timeout=5)
        except queue.Empty:
            raise AssertionError('monitor did not emit expected packet')
    def quiet():
        try:
            unexpected = packets.get(timeout=0.4)
        except queue.Empty:
            assert child.poll() is None
            return
        raise AssertionError(('unchanged monitor repeated a packet',unexpected))
    try:
        initial = receive()
        assert initial['event'] == 'snapshot', initial
        quiet()
        assert ctx['call']('history',ctx['run_id']) == before
        append(ctx,'driver-note',{'reason':'Non-meaningful owner-update proof annotation.'},'owner-update-note')
        noted = ctx['call']('history',ctx['run_id'])
        unchanged = receive()['owner_update']
        assert unchanged['required_before_next_decision'] is False, unchanged
        assert unchanged['unchanged_observation'] == 'suppress the heartbeat', unchanged
        quiet()
        assert ctx['call']('history',ctx['run_id']) == noted
        ctx['event']('intent-ready')
        changed_history = ctx['call']('history',ctx['run_id'])
        current = ctx['show']()['result']
        changed = receive()
        deadline = time.monotonic() + 5
        while not changed['owner_update']['required_before_next_decision']:
            assert time.monotonic() < deadline, changed
            changed = receive()
        update = changed['owner_update']
        assert update['required_before_next_decision'] is True, changed
        assert 'workflow' in update['observed_change'], update
        assert changed['workflow_lane']['current_state'] == current['current_state'] == 'intent-review', changed
        assert update['needed_action_or_decision'] == changed['next_action'], changed
        assert changed['workflow']['requestable_events'] == current['requestable_events'], changed
        assert changed['workflow']['current_state_instructions'] == current['current_state_instructions'], changed
        assert update['channel'] == 'active Pi conversation', update
        assert update['machine_notification_is_not_owner_update'] is True, update
        quiet()
    finally:
        child.terminate()
        child.wait(timeout=5)
        reader.join(timeout=5)
        child.stdout.close()
        child.stderr.close()
    assert ctx['call']('history',ctx['run_id']) == changed_history
    print('external-work owner-update changed packets, unchanged silence and shipped assistant guidance passed')


# bookends:LE-120 bookends:LE-145 — passive snapshots/attention expose workflow and source-backed owner-update guidance while external execution stays unknown and observer shutdown changes no run state.
# bookends:LE-148 — focused action is smaller than retained full detail; public history paging returns explicit cursors/totals without mutation.
def passive_views(ctx):
    import fcntl, pty, select, struct, termios
    append(ctx,'retained-detail',{'text':'original external detail '*3000},'large-retained-detail')
    before = ctx['call']('history',ctx['run_id'])
    full = ctx['show']()
    action = ctx['call']('show','--view','action',ctx['run_id'])
    assert len(json.dumps(action)) < len(json.dumps(full))
    assert 'current_state_instructions' in action['result'] and 'locators' in action['result']
    assert next(row['data']['text'] for row in full['result']['context'] if row['id'] == 'large-retained-detail') == 'original external detail '*3000
    page = ctx['call']('read',ctx['run_id'],'--kind','history','--cursor','0','--limit','2')['result']
    assert len(page['items']) == 2 and page['truncated'] and page['total'] > 2, page
    following = ctx['call']('read',ctx['run_id'],'--kind','history','--cursor',str(page['next_cursor']),'--limit','2')['result']
    assert following['items'][0]['sequence'] > page['items'][-1]['sequence'], following
    assert ctx['call']('history',ctx['run_id']) == before
    try:
        subprocess.run([ctx['engine'],'monitor','--run',ctx['run_id'],'--database',ctx['database'],'--json',
                        '--poll-seconds','0.05','--attention-seconds','0.1'],text=True,capture_output=True,timeout=2)
        raise AssertionError('monitor unexpectedly exited')
    except subprocess.TimeoutExpired as stopped:
        output = stopped.stdout.decode() if isinstance(stopped.stdout,bytes) else stopped.stdout
    packets = [json.loads(line) for line in output.splitlines()]
    assert any(p['event'] == 'snapshot' for p in packets) and any(p['event'] == 'attention' for p in packets), packets
    assert all(p['execution']['state'] == 'unknown' for p in packets), packets
    assert any(p['owner_update'].get('machine_notification_is_not_owner_update') for p in packets), packets
    assert ctx['call']('history',ctx['run_id']) == before
    # bookends:LE-147 — a real PTY navigates the frozen provider graph, scrolls details and preserves selection across portrait folded/unfolded resize; q only stops observation.
    master, slave = pty.openpty()
    transcript = bytearray()
    child = None
    def size(fd, rows, columns):
        fcntl.ioctl(fd,termios.TIOCSWINSZ,struct.pack('HHHH',rows,columns,0,0))
    def until(marker, offset=0):
        deadline = time.monotonic()+5
        while marker.encode() not in transcript[offset:]:
            assert time.monotonic() < deadline, (marker,bytes(transcript[-4000:]))
            if select.select([master],[],[],0.1)[0]:
                try:
                    transcript.extend(os.read(master,8192))
                except OSError:
                    assert child.poll() is None, bytes(transcript[-4000:])
    def key(value, marker):
        offset = len(transcript)
        os.write(master,value)
        until(marker,offset)
    try:
        size(slave,34,62)
        child = subprocess.Popen([ctx['engine'],'--database',ctx['database'],'explore',ctx['run_id']],
                                 stdin=slave,stdout=slave,stderr=slave,close_fds=True)
        os.close(slave)
        slave = -1
        until('selected=state:intent-review')
        until('requestable now')
        until('checked-not-allowed')
        key(b'g','selected=state:explore')
        key(b'j','selected=state:intent-review')
        key(b']','detail-offset=')
        key(b'[','detail-offset=')
        offset = len(transcript)
        size(master,27,114)
        until('terminal-size=27x114',offset)
        until('selected=state:intent-review',offset)
        key(b'G','selected=state:end')
        os.write(master,b'q')
        deadline = time.monotonic()+5
        while child.poll() is None:
            assert time.monotonic() < deadline, 'explorer failed to quit'
            if select.select([master],[],[],0.1)[0]:
                try:
                    transcript.extend(os.read(master,8192))
                except OSError:
                    pass
        assert child.wait() == 0, bytes(transcript[-4000:])
    finally:
        if child is not None and child.poll() is None:
            child.kill()
            child.wait()
        os.close(master)
        if slave != -1:
            os.close(slave)
    (ctx['work'] / 'explorer.transcript').write_bytes(transcript)
    assert ctx['call']('history',ctx['run_id']) == before
    print('external-work passive monitor, targeted reads and PTY navigation passed')



# bookends:LE-97 bookends:LE-159 bookends:LE-161 bookends:LE-162 — one outcome spine carries phase-specific live wording/cross-reference coverage, an executed distinguishing before-fix negative and current public proof, read-only prerequisites, and actual final Bookends GREEN.
def bookends_outcome_spine(args, root):
    def configure(profile, repository, artifacts):
        profile['extra']['bookends'] = {'enabled':True}
        for gate in profile['review_policies']:
            profile['review_policies'][gate] = []
        absent = process([args.software,'bookends-preview','--working-directory',repository])
        assert absent['missing_prerequisites'] and not absent['gate_executed'] and not absent['tests_executed'], absent
        (repository / 'docs').mkdir()
        (repository / 'scripts').mkdir()
        (repository / '.github/workflows').mkdir(parents=True)
        (repository / 'docs/PRD.md').write_text('### LE-1: Public helper outcome\n- Status: live\n- Coverage: e2e/journey\n\nAn external caller imports helper and observes helper(7) return 7. The applicable boundary is in [contract](contract.md).\n')
        (repository / 'docs/contract.md').write_text('# Public contract\n\nThe supported helper import returns the caller-supplied value; engine workflow status is not this outcome.\n')
        (repository / 'bookends.toml').write_text('prd = "docs/PRD.md"\n[classes.e2e_journey]\npathspecs = ["scripts/helper-journey.py"]\nrequired_ci_jobs = ["journey"]\n')
        (repository / '.github/workflows/ci.yml').write_text('jobs:\n  journey:\n    steps:\n      - run: python3 scripts/helper-journey.py\n')
        (repository / 'scripts/helper-journey.py').write_text('# bookends:LE-1\nimport sys\nfrom pathlib import Path\nsys.path.insert(0,str(Path(__file__).resolve().parents[1]))\nfrom helper import helper\nassert helper(7) == 7, "public helper must return the supplied value"\n')
        subprocess.run(['git','add','docs','scripts','.github','bookends.toml'],cwd=repository,check=True)
        subprocess.run(['git','commit','-qm','public outcome proof fixture'],cwd=repository,check=True)
        preview = process([args.software,'bookends-preview','--working-directory',repository])
        assert not preview['gate_executed'] and not preview['tests_executed'] and preview['evaluation_cwd'] == str(repository)
        wording = preview['prd']['live_requirements'][0]
        assert wording['id'] == 'LE-1' and 'helper(7)' in wording['normative_text'] and wording['explicit_references'], wording
        surface = preview['proof_classes'][0]
        assert 'scripts/helper-journey.py' in surface['eligible_public_locations'], surface
        intent_path = artifacts / 'intent.json'
        intent = json.loads(intent_path.read_text())
        intent.update(problem='The public helper must return its supplied value.',outcome='An external importing caller observes helper(7) return 7.',
                      constraints=['Use the existing public helper import and actual external proof output.'],
                      non_goals=['Changing engine semantics or claiming semantic review quality.'])
        intent['operating_context'].update(operators=['trusted local fixture caller'],environment=['isolated repository','public Python import'],
                      threat_boundary={'in_scope':['public helper return value'],'excluded':['production delivery','model quality']},accepted_risks=[])
        intent['acceptance'] = [{'id':'AC-1','statement':'The public helper journey imports helper and asserts helper(7) == 7.',
                                 'prd_traceability':{'type':'linked-live','live_ids':['LE-1']}}]
        intent['operating_context']['outside_obligations'] = [{'source':'docs/contract.md','obligation':'The supported import returns the caller-supplied value.'}]
        intent_path.write_text(json.dumps(intent))
        design_path = artifacts / 'design.json'
        design = json.loads(design_path.read_text())
        design['approach'] = 'Keep the public helper import returning the supplied argument, and use the public script to observe it.'
        design['elements'] = [{'name':'public helper import','responsibility':'Return the caller-supplied value.'},
                              {'name':'public journey','responsibility':'Import helper and distinguish an incorrect return value.'}]
        design['decisions'] = [{'choice':'Assert the actual return through the public import.',
                                'rationale':'Workflow status or a citation token cannot establish the return value.',
                                'rejected':['Checking only command exit or artifact presence.']}]
        design['risks'] = [{'risk':'A token-only test could pass with the wrong return value.',
                            'mitigation':'Execute a faulty-return negative against the same public assertion.'}]
        design['coverage'] = [{'criterion_id':'AC-1','acceptance':intent['acceptance'][0]['statement'],
                              'delivered_by':'The public helper import and operator-visible scripts/helper-journey.py assertion implement docs/contract.md.'}]
        design_path.write_text(json.dumps(design))
        plan_path = artifacts / 'plan.json'
        plan = json.loads(plan_path.read_text())
        task = copy.deepcopy(plan['tasks'][0])
        task.update(id='public-helper',objective='Preserve the observable public helper return value.',dependencies=[],
                    source_of_truth=['intent.json#/acceptance/0','docs/contract.md'],deliverables=['helper.py public import'],
                    out_of_scope=['Other outcomes or engine execution'],validation=['AC-1: python3 scripts/helper-journey.py imports helper and asserts helper(7) == 7; returning None fails. Retain the executed faulty-helper negative separately from current passing proof.'],
                    handoff='The external driver runs the public script and retains its actual output and source identity.',
                    criterion_ids=['AC-1'],proof_command_ids=['helper-test'])
        plan.update(objective='Preserve the public helper outcome with a distinguishing external assertion and phase-specific requirement coverage.',
                    tasks=[task],dependency_graph=[],proof_commands=[{'id':'helper-test','command':sys.executable,
                    'args':['-B','scripts/helper-journey.py'],'owner':'external-proof-owner','obligation':'AC-1: public import assertion helper(7) == 7 under docs/contract.md.'}])
        plan_path.write_text(json.dumps(plan))
        report_path = artifacts / 'implementation-report.json'
        report = json.loads(report_path.read_text())
        report['summary'] = 'The public helper import returns the input; the operator-visible script distinguishes a faulty return.'
        report['changed_surface'] = ['scripts/helper-journey.py public assertion','docs/PRD.md fixture requirement and docs/contract.md boundary']
        report['coverage']['commit'] = subprocess.check_output(['git','rev-parse','HEAD'],cwd=repository,text=True).strip()
        report['coverage']['documents'] = [{'path':'docs/contract.md','revision':'current'}]
        report['validation'] = [{'criterion_id':'AC-1','proof':'The operator-visible scripts/helper-journey.py assertion imports helper and checks helper(7) == 7 under docs/contract.md.'}]
        report_path.write_text(json.dumps(report))
    def hook(phase, ctx):
        if phase == 'plan':
            plan = json.loads((ctx['artifacts'] / 'plan.json').read_text())
            assert plan['tasks'][0]['criterion_ids'] == ['AC-1'] and plan['tasks'][0]['proof_command_ids'] == ['helper-test']
            saved = (ctx['repository'] / 'helper.py').read_bytes()
            (ctx['repository'] / 'helper.py').write_text('def helper(value):\n    return None\n')
            identity = provider(ctx,['source-identity','--working-directory',ctx['repository']],{})
            negative = raw([sys.executable,'-B','scripts/helper-journey.py'],cwd=ctx['repository'])
            assert negative.returncode != 0 and 'public helper must return the supplied value' in negative.stderr, negative
            (ctx['work'] / 'before-fix.stderr').write_text(negative.stderr)
            append(ctx,'historical-negative',{'criterion_id':'AC-1','assertion':'helper(7) == 7','source':identity,
                   'exit_code':negative.returncode,'stdout':negative.stdout,'stderr':negative.stderr,
                   'applicability':'Deliberately faulty before-fix fixture, never current passing evidence.'})
            (ctx['repository'] / 'helper.py').write_bytes(saved)
            positive = raw([sys.executable,'-B','scripts/helper-journey.py'],cwd=ctx['repository'])
            assert positive.returncode == 0, positive.stderr
        if phase == 'reconciliation':
            reconciliation = {'revision':'bookends-reconciliation','author':{'name':'external-driver','kind':'agent'},
               'mode':'bookends-enabled','branch':'sufficient-existing-wording',
               'document_observations':[{'path':'docs/contract.md','status':'unchanged','observation':'The existing public-import promise matches helper(7) returning 7.'}],
               'behavior_observations':[{'status':'matches-intent','observation':'The actual public script distinguishes a faulty return and passes on restored source.'}],
               'action':'no-document-change','action_reason':'Existing LE-1 and docs/contract.md require this exact outcome.',
               'authorization':'not-required','application':'not-required','commit':'not-required',
               'traceability':{'status':'retained','references':['LE-1','docs/contract.md']},'proof_references':['plan.json#/proof_commands'],
               'blockers':[],'decision':'complete'}
            (ctx['artifacts'] / 'reconciliation.json').write_text(json.dumps(reconciliation))
            ctx['event']('reconciliation-ready')
            return True
        if phase == 'validation-prepared':
            report = ctx['report']
            assert report['criteria'][0]['criterion_id'] == 'AC-1' and report['command_evidence_ids'] and report['goal_verdict_ids']
            proof = ctx['repository'] / 'scripts/helper-journey.py'
            saved = proof.read_bytes()
            proof.write_bytes(saved.replace(b'# bookends:LE-1\n',b''))
            red = raw([args.checker,'--repo',ctx['repository']],cwd=ctx['repository'])
            assert red.returncode != 0 and 'no eligible e2e/journey citation' in red.stdout, red
            proof.write_bytes(saved)
            green = raw([args.checker,'--repo',ctx['repository']],cwd=ctx['repository'])
            assert green.returncode == 0 and green.stdout.splitlines()[0] == 'GREEN', green
            (ctx['work'] / 'bookends-green.stdout').write_text(green.stdout)
        if phase == 'end':
            assert ctx['show']()['result']['lifecycle'] == 'final' and not ctx['show']()['result']['has_overrides']
            print('external-work Bookends five-artifact public outcome spine passed')
    return software_module().run(SimpleNamespace(engine=args.engine,provider=args.software,data_root=str(ROOT),mode='source'),
                                 ROOT / 'crates/software-change-provider/data/configs/minimal.json',root,configure=configure,exercise=hook)


# bookends:LE-174 — real early briefs expose partial parent claims and missing provider inputs without changing state or evidence.
def early_review_briefs(args, root):
    from external_work_driver import engine_call, declare_target_authorship
    def durable(show):
        result = copy.deepcopy(show['result'])
        # Read timestamps are observation metadata, not workflow state or evidence.
        def strip(value):
            if isinstance(value, dict):
                value.pop('observed_at', None)
                value.pop('sampled_at_ms', None)
                for child in value.values():
                    strip(child)
            elif isinstance(value, list):
                for child in value:
                    strip(child)
        strip(result)
        return result
    def configure(profile, repository, artifacts):
        axes = copy.deepcopy([p for p in profile['review_policies']['intent-review'] if p['review_stage'] == 'aggregate'][:2])
        for gate in profile['review_policies']:
            profile['review_policies'][gate] = []
        for gate in ['intent-review', 'intent-adversarial-review']:
            profile['review_policies'][gate] = copy.deepcopy(axes)
            for axis in profile['review_policies'][gate]:
                axis.update(group=gate+'.aggregate', required_authors=1)
    def hook(phase, ctx):
        if phase != 'explore':
            return
        subject = json.loads((ctx['artifacts'] / 'intent.json').read_text())
        axes = ctx['profile']['review_policies']['intent-review']
        reason = 'The supplied intent names the observable outcome and acceptance observations.'
        packet = {'show':ctx['show'](), 'assignment':{'gate':'intent-review','stage':'aggregate',
                  'group':axes[0]['group'],'author':{'name':'early-parent','kind':'agent'},
                  'subject':'intent.json','subject_revision':subject['revision']},
                  'request':{'text':'Review the supplied intent axes.'},'original':{'text':json.dumps({'judgments':[
                  {'axis':a['id'],'result':'pass','findings':'','grounds':{'reason':reason,
                   'evidence':[{'locator':'intent.json#/acceptance'}]}} for a in axes]})}}
        candidates = provider(ctx,['review-candidates'],packet)['records']
        first = next(c for c in candidates if c['data']['axis'] == axes[0]['id'])
        append(ctx,first['kind'],first['data'])
        before = ctx['show']()
        assert before['result']['current_state'] == 'explore'
        brief = provider(ctx,['commission','--slot','intent-adversarial-review','--stage','aggregate'],before)
        assert any(axes[1]['id'] in missing for missing in brief['missing_inputs']), brief
        assert not any('claim `'+axes[0]['id']+'`' in missing for missing in brief['missing_inputs']), brief
        assert any(row['data'] == first['data'] for row in brief['context']), brief
        assert reason in json.dumps(brief), brief
        assert durable(ctx['show']()) == durable(before), 'Early software preparation mutated the run.'
    software_module().run(SimpleNamespace(engine=args.engine,provider=args.software,data_root=str(ROOT),mode='source'),
                          ROOT / 'crates/software-change-provider/data/configs/minimal.json',root,
                          configure=configure,exercise=hook)
    for kind, binary in [('policy-document',args.policy),('research',args.research)]:
        work = root / ('early-'+kind)
        work.mkdir()
        artifacts = work / 'artifacts'
        artifacts.mkdir()
        if kind == 'policy-document':
            spec = importlib.util.spec_from_file_location('early_policy_fixture', ROOT / 'scripts/policy-document-journey.py')
            fixture = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(fixture)
            profile = json.loads((ROOT / 'crates/policy-document-provider/data/readme.json').read_text())
            target = work / 'README.md'
            target.write_text(fixture.document())
            profile.update(mode='audit')
            profile['target']['path'] = str(target)
            command = ['commission',json.dumps(profile)]
            review_state = 'semantic-review'
        else:
            spec = importlib.util.spec_from_file_location('early_research_fixture', ROOT / 'scripts/research-journey.py')
            fixture = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(fixture)
            profile = json.loads((ROOT / 'crates/research-provider/data/configs/standard.json').read_text())
            for name, make in [('brief.json',fixture.brief),('sources.json',fixture.sources),
                               ('verification.json',fixture.verification),('report.json',fixture.report)]:
                (artifacts / name).write_text(json.dumps(make()))
            target = artifacts / 'verification.json'
            command = ['commission','verify']
            review_state = 'verify'
        profile['artifact_root'] = str(artifacts)
        selected = work / 'profile.json'
        selected.write_text(json.dumps(profile))
        config = work / 'providers.toml'
        config.write_text(f'[providers.{kind}]\ncommand = {json.dumps(binary)}\nargs = []\n')
        def call(*argv):
            return engine_call(args.engine,work / 'run.sqlite',config,list(argv))
        run_id = 'early-'+kind
        call('start','--id',run_id,kind,'@'+str(selected),'Early preparation public proof')
        if kind == 'policy-document':
            declare_target_authorship(Path(args.engine),work / 'run.sqlite',config,run_id,target,
                                      {'name':'owner','kind':'human'},'early-target-author')
        before = call('show','--view','full',run_id)
        assert before['result']['current_state'] != review_state, before
        complete = process([binary,*command],before)
        assert complete['missing_inputs'] == [], complete
        assert durable(call('show','--view','full',run_id)) == durable(before)
        saved = target.read_bytes()
        target.unlink()
        missing = process([binary,*command],before)
        assert any(row['input'] == ('target' if kind == 'policy-document' else 'verification.json')
                   for row in missing['missing_inputs']), missing
        assert durable(call('show','--view','full',run_id)) == durable(before)
        target.write_bytes(saved)
    print('external-work early review briefs and partial-parent public proof passed')


def self_test():
    import ast
    for filename in ['external-work-journey.py','external_work_driver.py','software-change-journey.py',
                     'policy-document-journey.py','research-journey.py','generate-prd-journey.py']:
        ast.parse((ROOT / 'scripts' / filename).read_text(),filename=filename)
    retired_modules = {'work_slot_journey','recovery_execution','recovery_cancellation',
                       'operational_ux_capture','operational_ux_summary','operational_ux_monitor'}
    for path in (ROOT / 'scripts').glob('*.py'):
        tree = ast.parse(path.read_text(),filename=str(path))
        for node in ast.walk(tree):
            imported = ([name.name for name in node.names] if isinstance(node,ast.Import)
                        else [node.module] if isinstance(node,ast.ImportFrom) else [])
            assert not retired_modules.intersection(imported), path
    assert all(not (ROOT / 'scripts' / (name+'.py')).exists() for name in retired_modules)
    assert len(RETIRED_ENGINE) == len(set(RETIRED_ENGINE))
    assert 'invoke' in RETIRED_ENGINE and 'run-validation' in RETIRED_PROVIDER
    assert software_module().SUBJECTS['plan.json'] == 'plan-good.json'
    print('external-work journey self-test passed')



def qualified_intent_revision(args, root):
    def configure(profile, repository, artifacts):
        axis = copy.deepcopy(profile['review_policies']['intent-review'][0])
        axis['required_authors'] = 1
        for gate in profile['review_policies']:
            profile['review_policies'][gate] = []
        profile['review_policies']['intent-review'] = [axis]
    def hook(phase, ctx):
        if phase != 'explore':
            return
        path = ctx['artifacts'] / 'intent.json'
        baseline = path.read_text()
        prior = json.loads(baseline)
        append(ctx,'review-evidence',{'gate':'intent-review','subject':'intent.json',
               'subject_revision':prior['revision'],'author':{'name':'prior-reviewer','kind':'agent'}},'prior-intent-review')
        revised = copy.deepcopy(prior)
        revised['revision'] = prior['revision'] + '-qualified'
        revised['outcome'] += ' Preserve status inspection during ordinary delivery outages.'
        path.write_text(json.dumps(revised))
        for artifact in ctx['artifacts'].glob('*.json'):
            if artifact == path:
                continue
            value = json.loads(artifact.read_text())
            if value.get('intent_revision') == prior['revision']:
                value['intent_revision'] = revised['revision']
                artifact.write_text(json.dumps(value))
        # bookends:LE-156 — a materially revised approval without qualified exact prior bytes is refused.
        refused([args.software,'commission','--slot','intent-review'],ctx['show'](),needle='exact owner baseline')
        instruction = 'Owner qualification: status inspection must remain available during ordinary delivery outages; delivery retry is separate.'
        append(ctx,'user-steering',{'target':{'kind':'all'},'instruction':instruction,
               'intent_baseline':{'revision':prior['revision'],'locator':str(path)+'@prior',
                                  'sha256':'sha256:'+hashlib.sha256(baseline.encode()).hexdigest(),
                                  'json_bytes':baseline}},'qualified-owner-source')
        brief = provider(ctx,['commission','--slot','intent-review'],ctx['show']())
        view = brief['owner_approval_view']
        # bookends:LE-156 — exact owner qualification, full current wording, exact delta and independent approval actor reach the public commission.
        assert view['current_intent']['value'] == revised
        assert any(source['record_id'] == 'qualified-owner-source' and source['instruction'] == instruction for source in view['owner_sources'])
        delta = view['exact_substantive_wording_delta']
        assert delta['status'] == 'available' and delta['from_revision'] == prior['revision']
        assert delta['baseline_source']['sha256'] == 'sha256:'+hashlib.sha256(baseline.encode()).hexdigest()
        assert {'pointer':'/outcome','change':'replace','before':prior['outcome'],'after':revised['outcome']} in delta['changes']
        assert view['actors']['draft_author'] == revised['author']
        assert view['actors']['owner_approval'] == {'kind':'human','status':'not-performed'}
        assert 'prior-reviewer' in view['actors']['reviewers']
    return software_module().run(SimpleNamespace(engine=args.engine,provider=args.software,mode='source',data_root=None),
                                 ROOT / 'crates/software-change-provider/data/configs/minimal.json',root,configure=configure,exercise=hook)


def intent_sufficient_comparison(args, root):
    def configure(profile, repository, artifacts):
        for gate in profile['review_policies']:
            axes = [copy.deepcopy(p) for p in profile['review_policies'][gate] if p['id'] == 'intent-sufficient'] if gate.startswith('intent') else []
            for axis in axes:
                axis['required_authors'] = 2
            profile['review_policies'][gate] = axes
    def hook(phase, ctx):
        if phase != 'intent-review':
            return
        intent = json.loads((ctx['artifacts'] / 'intent.json').read_text())
        policy = ctx['profile']['review_policies'][phase][0]
        ids = []
        for index in range(2):
            reason = ('Anticipated final state: approvals are durably recorded and inspectable during delivery outages; delivery retries are separate. '
                      'Alternative tested: varying internal retry queues does not change the stated operator outcome; this is implementation freedom, not an unwanted permitted state.')
            packet = {'show':ctx['show'](),'assignment':{'gate':phase,'stage':'aggregate','group':policy['group'],
                      'author':{'name':f'cold-reader-{index}','kind':'agent'},'subject':'intent.json','subject_revision':intent['revision']},
                      'request':{'text':'Cold reconstruction using only current intent and declared baseline; then search for material alternatives.'},
                      'original':{'text':json.dumps({'judgments':[{'axis':'intent-sufficient','result':'pass','findings':'',
                                  'grounds':{'reason':reason,'evidence':[{'locator':'intent.json#/baseline'},{'locator':'intent.json#/outcome'}]}}]})}}
            candidate = provider(ctx,['review-candidates'],packet)['records'][0]
            assert candidate['data']['baseline'] == intent['baseline']
            rid = f'cold-reconstruction-{index}'
            append(ctx,candidate['kind'],candidate['data'],rid)
            ids.append(rid)
        ledger(ctx,phase,'intent.json',intent['revision'])
        # bookends:LE-173 — cold returns alone cannot clear alignment; missing and incomplete owner comparisons deny public progression.
        missing = evaluate(ctx,phase,'approved','intent-adversarial-review')
        assert missing['result'] == 'deny' and 'missing current intent-comparison' in json.dumps(missing), missing
        challenge = provider(ctx,['commission','--slot','intent-adversarial-review','--stage','aggregate'],ctx['show']())
        assert 'intent-comparison' in json.dumps(challenge['missing_inputs'])
        comparison = {'subject_revision':intent['revision'],'baseline':intent['baseline'],'reconstruction_ids':ids,
                      'actor':{'name':'owner-authorized-driver','kind':'agent'},'authority':'owner authorization: fixture comparison instruction',
                      'dispositions':[{'reconstruction_id':rid,'alternative':'Internal retry queue variation',
                                       'disposition':'implementation-freedom','note':'Both counted reconstructions preserve the named outcome.'} for rid in ids]}
        partial = copy.deepcopy(comparison)
        partial['reconstruction_ids'] = ids[:1]
        partial['dispositions'] = partial['dispositions'][:1]
        append(ctx,'intent-comparison',partial)
        denied = evaluate(ctx,phase,'approved','intent-adversarial-review')
        assert denied['result'] == 'deny' and 'every counted reconstruction' in json.dumps(denied)
        append(ctx,'intent-comparison',comparison,'complete-owner-comparison')
        # bookends:LE-173 — all counted cold returns and their owner-authorized comparison clear the axis before informed challenge preparation.
        assert evaluate(ctx,phase,'approved','intent-adversarial-review')['result'] == 'allow'
        challenge = provider(ctx,['commission','--slot','intent-adversarial-review','--stage','aggregate'],ctx['show']())
        assert not challenge['missing_inputs'], challenge
        assert challenge['intent_comparisons'][-1]['data'] == comparison
        delivered = {r['id']:r for r in challenge['context']}
        assert set(ids) <= delivered.keys()
        assert all(delivered[rid]['data']['baseline'] == intent['baseline'] for rid in ids)
        path = ctx['artifacts'] / 'intent.json'
        saved = path.read_bytes()
        changed = copy.deepcopy(intent)
        changed['baseline']['commit'] = '0' * 40
        path.write_text(json.dumps(changed))
        # bookends:LE-173 — changed source baseline cannot reuse the recorded reconstruction or comparison.
        denied = evaluate(ctx,phase,'approved','intent-adversarial-review')
        assert denied['result'] == 'deny' and 'baseline changed' in json.dumps(denied)
        path.write_bytes(saved)
        ctx['event']('approved')
        return True
    return software_module().run(SimpleNamespace(engine=args.engine,provider=args.software,mode='source',data_root=None),
                                 ROOT / 'crates/software-change-provider/data/configs/minimal.json',root,configure=configure,exercise=hook)


def main():
    parser = argparse.ArgumentParser()
    for key in ['engine','software','policy','research','checker','output-root']:
        parser.add_argument('--'+key)
    parser.add_argument('--self-test',action='store_true')
    args = parser.parse_args()
    # Bare invocation is the recognized CI collection form; full proof uses the parameterized run below.
    if args.self_test or len(sys.argv) == 1:
        self_test()
        return
    for key in ['engine','software','policy','research','checker','output_root']:
        if not getattr(args,key):
            parser.error('--'+key.replace('_','-')+' is required')
        setattr(args,key,str(Path(getattr(args,key)).resolve()))
    root = Path(args.output_root)
    root.mkdir(parents=True,exist_ok=True)
    trap = root / 'trap-bin'
    trap.mkdir(exist_ok=True)
    sentinel = root / 'forbidden-execution'
    assert not sentinel.exists(), 'Use a fresh output root: prior execution sentinel exists.'
    for command in ['pi','dagu','cargo']:
        script = trap / command
        script.write_text('#!/bin/sh\nprintf "%s\\n" "$0" >> '+json.dumps(str(sentinel))+'\nexit 99\n')
        script.chmod(0o755)
    os.environ['PATH'] = str(trap)+os.pathsep+os.environ['PATH']
    retirement_root = root / 'retirement'
    retirement_root.mkdir()
    retirement(args,retirement_root)
    delivery_proof(args,root)
    profile_preparation(args,root)
    qualified_intent_revision(args,root)
    intent_sufficient_comparison(args,root)
    software_runs = [bookends_outcome_spine(args,root),findings_and_steering(args,root),revision_and_observation(args,root),intake_counts_advice(args,root),paired_and_command_proof(args,root),lint_only_reuse(args,root)]
    software_runs.append(reshaped_plan_applicability(args,root))
    software_runs.append(invalid_reuse_fresh_sibling(args,root))
    owner_update_done = False
    def owner_update_hook(phase, ctx):
        nonlocal owner_update_done
        if phase == 'explore' and not owner_update_done:
            owner_update_done = True
            owner_update_proof(ctx)
            return True
    software_runs.append(software_module().run(
        SimpleNamespace(engine=args.engine,provider=args.software,data_root=str(ROOT),mode='source'),
        ROOT / 'crates/software-change-provider/data/configs/minimal.json',root,
        exercise=owner_update_hook))
    retention_minimal_profile(args,software_runs[-1])
    provider_specific(args,root)
    retention_provider_runs(args,root)
    retained_work = retention_software_revisions(args,root)
    retention_software_overrides(args,root)
    reclaim_legacy_catalog(args,root,retained_work)
    early_review_briefs(args,root)
    generated = raw([sys.executable,ROOT / 'scripts/generate-prd-journey.py','--mode','source',
                     '--engine',args.engine,'--provider',args.research,'--checker',args.checker,
                     '--profile',ROOT / 'crates/research-provider/data/configs/generate-prd.json',
                     '--work-root',root / 'generate-prd'])
    (root / 'generate-prd.stdout').write_text(generated.stdout)
    (root / 'generate-prd.stderr').write_text(generated.stderr)
    assert generated.returncode == 0, generated.stderr
    assert 'generate-prd external-work normal completion passed' in generated.stdout
    print('external-work generate-prd checker-backed normal completion passed')
    # bookends:LE-139 — meaningful workflows and changed provider boundaries complete through real binaries with scripted judgments and no model/executor launches.
    assert not sentinel.exists(), sentinel.read_text() if sentinel.exists() else ''
    (root / 'outcome.json').write_text(json.dumps({'status':'passed','software_runs':[str(path) for path in software_runs],
                                                'residual_risks':['Scripted reviews prove mechanics, not semantic verdict quality.'],
                                                'execution_sentinel_absent':True},indent=2))
    print('external-work execution traps stayed unused passed')
    print('external-work journey passed')

if __name__ == '__main__':
    main()
