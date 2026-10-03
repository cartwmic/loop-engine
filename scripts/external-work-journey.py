#!/usr/bin/env python3
"""Black-box external-work outcomes. Scripted reviews establish mechanics only."""
import argparse
import copy
import importlib.util
import json
import os
import subprocess
import sys
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


def append(ctx, kind, data, record_id=None):
    args = ['append']
    if record_id:
        args += ['--record-id',record_id]
    return ctx['call'](*args,ctx['run_id'],kind,json.dumps(data))


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


# bookends:LE-168 bookends:LE-170 bookends:LE-174 bookends:LE-177 bookends:LE-178 bookends:LE-182 — grouped external returns, early commissions/evidence, explicit counts and ordinary-first challenges.
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
                                                               'occasion_id':'review-candidates:explore:intent-ready'}]}
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
            judgments = [{'axis':axis,'result':'pass','findings':'','grounds':{'reason':'Current intent inspected.',
                           'evidence':[{'locator':'intent.json#/revision'}]}} for axis in axes]
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
            for index, form in enumerate(['prose','yaml','fenced','json','self']):
                if form in ['prose','self']:
                    original = {'text':'The two intent obligations are satisfied; see the exact revision.'}
                elif form == 'yaml':
                    original = {'text':'assessment_id: combined-early\njudgments:\n' + ''.join(
                        '- axis: '+j['axis']+'\n  result: pass\n  findings: ""\n  grounds:\n    reason: Current intent inspected.\n    evidence:\n    - locator: intent.json#/revision\n' for j in judgments)}
                else:
                    text = json.dumps(batch)
                    original = {'text':'```json\n'+text+'\n```' if form == 'fenced' else text}
                assignment = {'gate':'intent-review','stage':'aggregate','group':policies[0]['group'],
                              'author':subject['author'] if form == 'self' else {'name':f'early-{index}','kind':'agent'},
                              'subject':'intent.json','subject_revision':subject['revision']}
                packet = {'show':shown,'assignment':assignment,'request':{'text':'Review both axes as one assessment.'},'original':original}
                if form in ['prose','yaml','self']:
                    packet['projection'] = batch
                intake = provider(ctx,['review-candidates'],packet)
                assert len(intake['records']) == len(axes), intake
                assert sum(item['candidate'] is None for item in intake['items']) == 2, intake
                for candidate in intake['records']:
                    if form == 'self':
                        assert candidate['data']['external_review']['direct'] is True
                    append(ctx,candidate['kind'],candidate['data'])
            ledger(ctx,'intent-review','intent.json',subject['revision'])
            assert evaluate(ctx,'intent-review','approved','intent-adversarial-review')['result'] == 'allow'
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
                    request = {'version':1,'state':{'admissibility':{'bounded_judgment':True,'evidence_sufficient':True}},
                               'target':target,'occasion':occasion['occasion_id'],
                               'questions':{'retain':{'type':'noul','instructions':'Judge this one supplied intent claim.',
                                                        'proposition':'The current intent describes the fixture need.'}}}
                    response = {'answers':{'retain':{'type':'noul','noul':0.9,'rationale':'The explicit fixture intent states this need.'}}}
                    append(ctx,'advice-answer',{'occasion':occasion['occasion_id'],'request':request,'response':response,
                                              'author':{'name':'external-advisor','kind':'agent'},'original':{'text':json.dumps(response)}},response_id)
                    append(ctx,'advice-disposition',{'response_id':response_id,'answer_id':'retain','occasion_id':occasion['occasion_id'],
                                                    'state_visit':visit,'target':target,'disposition':'accept','reason':'Bounded answer agrees with inspected intent.'})
                    state['answered'] = True
                    print('external-work typed advice answer supplied passed')
                append(ctx,'advice-occasion',{'state_visit':visit,'source_state':current,'event':occasion['event'],
                                            'occasion_id':occasion['occasion_id'],'target':target,'triggered':triggered,
                                            'response_ids':[response_id] if triggered else [],
                                            'trigger_source_ids':[next(r['id'] for r in ctx['show']()['result']['context'] if r['kind'] == 'review-evidence')] if triggered else [],
                                            'reason':None if triggered else 'Explicit driver judgment of applicability.'})
        if phase == 'explore':
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
def paired_and_command_proof(args, root):
    state = {}
    def configure(profile, repository, artifacts):
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
    def hook(phase, ctx):
        if phase == 'implementation-review':
            path = state['secondary'] / 'scoped.json'
            saved = path.read_bytes()
            path.write_text('{"secondary":false}')
            stale = evaluate(ctx,'implementation-review','approved','implementation-adversarial-review')
            assert stale['result'] == 'deny' and 'checkpoint' in json.dumps(stale).lower(), stale
            path.write_bytes(saved)
            print('external-work paired-repo stale-secondary refusal passed')
        if phase == 'validation-prepared':
            shown = ctx['show']()
            command = next(r for r in shown['result']['context'] if r['kind'] == 'command-evidence')
            original = copy.deepcopy(ctx['report'])
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
        if phase == 'end':
            assert not ctx['show']()['result']['has_overrides']
            final_report = ctx['work'] / 'final-report.md'
            final_report.write_text('# External-work final report\n\nNormal completion, no override.\n\n'
                                    'Residual risks: scripted judgments prove recording and qualification mechanics, not semantic review quality.\n'
                                    'The external driver owns proof commands and source applicability decisions.\n')
            assert 'Residual risks:' in final_report.read_text()
            print('external-work paired-repo and external-command normal completion passed')
            print('external-work residual final-report reporting passed')
    return software_module().run(SimpleNamespace(engine=args.engine,provider=args.software,data_root=str(ROOT),mode='source'),
                                 ROOT / 'crates/software-change-provider/data/configs/high-rigor.json',root,
                                 configure=configure,exercise=hook)


# bookends:LE-171 — retained suite and whole-goal judgments remain applicable after affected lint checks.
def lint_only_reuse(args, root):
    state = {'reused':False}
    def configure(profile, repository, artifacts):
        policy = copy.deepcopy(next(p for p in profile['review_policies']['validation-review'] if p['review_stage'] == 'aggregate'))
        for gate in profile['review_policies']:
            profile['review_policies'][gate] = []
        policy.update(required_authors=1,group='validation-review.aggregate')
        profile['review_policies']['validation-review'] = [policy]
        (repository / 'helper.py').write_text('def helper(p, q, value):\n    values = []\n    if p:\n        if q:\n            values.append(value)\n    return values\n')
        subprocess.run(['git','add','helper.py'],cwd=repository,check=True)
        subprocess.run(['git','commit','-qm','nested test helper'],cwd=repository,check=True)
        counter = artifacts.parent / 'suite-runs'
        assertion = 'from helper import helper; ' + '; '.join(
            f'assert helper({p}, {q}, 7) == {expected}' for p,q,expected in
            [(False,False,[]),(False,True,[]),(True,False,[]),(True,True,[7])])
        state['assertion'] = assertion
        state['counter'] = counter
        plan_path = artifacts / 'plan.json'
        plan = json.loads(plan_path.read_text())
        plan['proof_commands'] = [{'id':'prior-suite','command':sys.executable,
                                  'args':['-B','-c',assertion+f'; open({str(counter)!r}, "a").write("suite\\n")'],
                                  'owner':'driver','obligation':'Assert the complete helper truth table.'}]
        plan_path.write_text(json.dumps(plan))
    def hook(phase, ctx):
        if phase == 'validation-review' and not state['reused']:
            state['reused'] = True
            artifacts = ctx['artifacts']
            original_report = json.loads((artifacts / 'validation-report.json').read_text())
            policy = ctx['profile']['review_policies']['validation-review'][0]
            packet = {'show':ctx['show'](),'assignment':{'gate':'validation-review','stage':'aggregate','group':policy['group'],
                      'author':{'name':'prior-reviewer','kind':'agent'},'subject':'validation-report.json',
                      'subject_revision':original_report['revision']},'request':{'text':'Inspect the prior suite and goal judgments.'},
                      'original':{'text':json.dumps({'judgments':[{'axis':policy['id'],'result':'pass','findings':'',
                                  'grounds':{'reason':'The actual helper truth table passed.',
                                             'evidence':[{'locator':'validation-report.json#/command_evidence_ids'}]}}]})}}
            for candidate in provider(ctx,['review-candidates'],packet)['records']:
                append(ctx,candidate['kind'],candidate['data'])
            ledger(ctx,'validation-review','validation-report.json',original_report['revision'])
            prior = evaluate(ctx,'validation-review','passed','end')
            assert prior['result'] == 'allow', prior
            originals = ctx['show']()['result']['context']
            ctx['event']('revise-implementation')
            (ctx['repository'] / 'helper.py').write_text('def helper(p, q, value):\n    values = []\n    if p and q:\n        values.append(value)\n    return values\n')
            subprocess.run(['git','add','helper.py'],cwd=ctx['repository'],check=True)
            subprocess.run(['git','commit','-qm','collapse lint-only nested condition'],cwd=ctx['repository'],check=True)
            implementation = json.loads((artifacts / 'implementation-report.json').read_text())
            implementation['revision'] = 'lint-i2'
            implementation['summary'] += ' Lint-only conjunction collapse preserves all four truth-table outcomes.'
            (artifacts / 'implementation-report.json').write_text(json.dumps(implementation))
            ctx['checkpoint']('implementation')
            ctx['event']('implementation-ready')
            ctx['event']('reconciliation-ready')
            assert ctx['show']()['result']['current_state'] == 'validation'
            additions = [{'id':'helper-rerun','command':sys.executable,'args':['-B','-c',state['assertion']],
                          'owner':'driver','obligation':'Rerun the affected helper truth table.'},
                         {'id':'lint-rerun','command':sys.executable,
                          'args':['-B','-c','import ast, pathlib; s = pathlib.Path("helper.py").read_text(); ast.parse(s); assert "if p and q:" in s; assert "if q:" not in s'],
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
                       'reason':'Only the equivalent nested helper condition was collapsed; retained suite and judgments remain applicable.'},carry_id)
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
                                 configure=configure,exercise=hook)


# bookends:LE-167 bookends:LE-172 — external policy/research judgments and external typed advice complete normally.
def provider_specific(args, root):
    import hashlib
    from external_work_driver import engine_call, record_reviews
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
            profile.update(mode='audit',target_author=owner)
            profile['target']['path'] = str(target)
            profile['semantic_policies'] = [profile['semantic_policies'][0]]
            policy = profile['semantic_policies'][0]
            gate, event, subject = 'semantic-review','passed',profile['target']['id']
            revision = hashlib.sha256(target.read_bytes()).hexdigest()
        else:
            profile = json.loads((ROOT / 'crates/research-provider/data/configs/standard.json').read_text())
            for gate in ['verify','synthesize']:
                profile['review_policies'][gate] = [profile['review_policies'][gate][0]]
            policy = profile['review_policies']['verify'][0]
            gate, event, subject, revision = 'verify','verified','verification.json','1'
            for name, make in [('brief.json',research.brief),('sources.json',research.sources),('verification.json',research.verification)]:
                (artifacts / name).write_text(json.dumps(make()))
            target = artifacts / 'verification.json'
        policy.update(required_authors=1,self_review=False)
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
            return call('append','--record-id',id,run_id,kind,json.dumps(data))
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
                      'subject_revision':revision,'policy_ids':[policy['id']]},'author':author,
                      'request':{'text':'Review the current target.'},'result':{'text':json.dumps({'items':[{'policy_id':policy['id'],'result':'pass','findings':''}]})}}
            result = process([binary,'review-candidates'],packet)
            assert len(result['candidates']) == 1, result
            return result['candidates'][0]['data']
        record('review-evidence',intake(owner),'self-evidence')
        show()
        refused([args.engine,'--database',database,'--config',config,'--json','event',run_id,event],needle='review-incomplete')
        record('review-evidence',intake({'name':'external-independent','kind':'agent'}),'independent-evidence')
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
        else:
            value = json.loads(target.read_text())
            value['revision'] = '2'
            target.write_text(json.dumps(value))
            revision = '2'
        show()
        refused([args.engine,'--database',database,'--config',config,'--json','event',run_id,event],needle='review-incomplete')
        record('evidence-applicability',{'origin':{'kind':'context-record','id':'independent-evidence'},
               'target':{'subject':subject,'revision':revision},'attesting_driver':owner,
               'reason':'Only immaterial fixture metadata changed; judgment scope remains unchanged.'},'carry-independent')
        move(event)
        if kind == 'research':
            value = research.report()
            value['verification_revision'] = '2'
            (artifacts / 'report.json').write_text(json.dumps(value))
            record_reviews(Path(args.engine),database,config,Path(binary),run_id,'synthesize',
                           [p['id'] for p in profile['review_policies']['synthesize']],'report.json','1')
            move('completed')
        terminal = show()['result']
        assert terminal['current_state'] == 'end' and terminal['lifecycle'] == 'final' and not terminal['has_overrides'], terminal
        print(f'external-work {kind} self-exclusion amendment applicability normal completion passed')


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


def main():
    parser = argparse.ArgumentParser()
    for key in ['engine','software','policy','research','checker','output-root']:
        parser.add_argument('--'+key)
    parser.add_argument('--self-test',action='store_true')
    args = parser.parse_args()
    if args.self_test:
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
    software_runs = [intake_counts_advice(args,root),paired_and_command_proof(args,root),lint_only_reuse(args,root)]
    provider_specific(args,root)
    generated = raw([sys.executable,ROOT / 'scripts/generate-prd-journey.py','--mode','source',
                     '--engine',args.engine,'--provider',args.research,'--checker',args.checker,
                     '--profile',ROOT / 'crates/research-provider/data/configs/generate-prd.json',
                     '--work-root',root / 'generate-prd'])
    (root / 'generate-prd.stdout').write_text(generated.stdout)
    (root / 'generate-prd.stderr').write_text(generated.stderr)
    assert generated.returncode == 0, generated.stderr
    assert 'generate-prd external-work normal completion passed' in generated.stdout
    print('external-work generate-prd checker-backed normal completion passed')
    assert not sentinel.exists(), sentinel.read_text() if sentinel.exists() else ''
    (root / 'outcome.json').write_text(json.dumps({'status':'passed','software_runs':[str(path) for path in software_runs],
                                                'residual_risks':['Scripted reviews prove mechanics, not semantic verdict quality.'],
                                                'execution_sentinel_absent':True},indent=2))
    print('external-work execution traps stayed unused passed')
    print('external-work journey passed')

if __name__ == '__main__':
    main()
