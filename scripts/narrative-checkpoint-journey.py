#!/usr/bin/env python3
"""Change-specific mechanical proof pending LE-185 reconciliation (not a live citation).

SCRIPTED DUMMY BACKENDS supply all reviews and owner decisions. They establish no
semantic review or real human approval. --self-test checks collection/interfaces
only. Real proof requires explicit built binaries and reaches end without override.
"""
import argparse
import copy
import hashlib
import importlib.util
import json
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.dont_write_bytecode = True  # Journey artifacts/captures stay outside the checkout.
SPEC = importlib.util.spec_from_file_location('software_journey', ROOT / 'scripts/software-change-journey.py')
SOFTWARE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SOFTWARE)
PHASES = ['intent', 'design', 'plan', 'implementation', 'validation']
NARRATIVES = ['design', 'plan', 'validation']
SUBJECT = {'intent':'intent.json', 'design':'design.json', 'plan':'plan.json',
           'implementation':'implementation-report.json', 'validation':'validation-report.json'}
LABEL = 'SCRIPTED DUMMY BACKENDS: mechanical proof only, not semantic review or human approval'


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def fact(record, kind):
    if record['kind'] == kind:
        return record['data']
    if kind == 'approved-artifact' and record['kind'] in ('artifact-admitted', 'reconciliation-decision'):
        return record['data'].get('approved_artifact')
    if kind == 'artifact-admitted' and record['kind'] == 'reconciliation-decision':
        return record['data'].get('artifact_admitted')
    return None


def latest(context, kind, phase):
    matches = [(r, fact(r, kind)) for r in context]
    matches = [(r, f) for r, f in matches if f and
               (f.get('subject') == SUBJECT[phase] if kind == 'approved-artifact'
                else f.get('phase') == phase)]
    assert matches, (kind, phase)
    return max(matches, key=lambda pair: pair[0]['sequence'])


class Journey:
    def __init__(self, args, root, name, *, reviewless=False, final_narrative=False):
        self.args, self.root, self.name = args, Path(root), name
        self.reviewless, self.final_narrative = reviewless, final_narrative
        self.log = SOFTWARE.CommandLog(self.root / 'captures')
        self.cases, self.decisions, self.flags = [], [], set()
        self.counters, self.pairs, self.saved_artifacts = {}, {}, {}
        self.withheld = []
        self.frozen = None
        self.log.identity(args.engine, args.provider)
        self.receipt = self.root / 'result.json'
        self.result('not-run')

    def result(self, status, **extra):
        write_json(self.receipt, {'label':LABEL, 'status':status,
                   'fixture':self.name, 'reviewless_artifact_fixture':self.reviewless,
                   'high_rigor_proof':not self.reviewless,
                   'work_root':str(self.work_root) if hasattr(self, 'work_root') else None,
                   'actual_command_outcomes':str(self.log.index),
                   'command_payload_root':str(self.log.root),
                   'bookends':'disabled explicitly by setup; no GREEN claim or bypass',
                   'cases':self.cases, 'scripted_owner_decisions':self.decisions, **extra})

    def case(self, name, **details):
        self.cases.append({'case':name, 'label':LABEL, **details})
        self.result('in-progress')

    def configure(self, profile, repository, artifacts):
        self.work_root = artifacts.parent
        base = self.root / 'base.json'
        write_json(base, profile)
        selected = self.root / 'setup.json'
        argv = [self.args.provider, 'setup', '--profile', base, '--decline-advice', '--output', selected]
        enabled_narratives = ['validation'] if self.reviewless and self.final_narrative else NARRATIVES
        if self.reviewless and not self.final_narrative:
            enabled_narratives = ['design']  # Bind the real nested reviewless approval ID.
        checkpoints = PHASES if not self.final_narrative else PHASES[:-1]
        for phase in enabled_narratives:
            argv += ['--set', f'narrative.{phase}=true']
        for phase in checkpoints:
            argv += ['--set', f'checkpoint.{phase}=true']
        response = self.log.execute(argv, cwd=repository, raw=True)
        assert response.returncode == 0, response.stderr
        generated = json.loads(selected.read_text())
        explain = json.loads(Path(str(selected) + '.explain.json').read_text())
        assert generated['extra']['narratives'] == enabled_narratives
        assert generated['extra']['checkpoints'] == checkpoints
        assert explain['effective']['narratives'] == enabled_narratives
        assert explain['effective']['checkpoints'] == checkpoints
        for key in [f'narrative.{p}' for p in enabled_narratives] + [f'checkpoint.{p}' for p in checkpoints]:
            assert any(o['key'] == key and o['value'] is True for o in explain['overrides'])
        if self.reviewless:
            # Separate, explicit collision/completion fixture; NOT high-rigor proof.
            # Omit only artifact rooms; keep materialized narrative floors and the
            # complete command/criterion/goal index and repository checkpoint duties.
            for gate in generated['review_policies']:
                if '-narrative-' not in gate:
                    generated['review_policies'][gate] = []
        profile.clear()
        profile.update(generated)
        self.frozen = copy.deepcopy(profile)
        write_json(self.root / 'declared-frozen-profile.json', profile)
        self.case('AC-2 setup/explain choices emitted', options=profile['extra'],
                  fixture_declaration='Shipped high-rigor-15 floors unchanged' if not self.reviewless
                  else 'Minimal-15 artifact-reviewless fixture; narrative floors and all completion duties retained')

    def append(self, ctx, kind, data, record_id=None):
        argv = ['append']
        if record_id:
            argv += ['--record-id', record_id]
        response = ctx['call'](*argv, ctx['run_id'], kind, json.dumps(data))
        assert response['status'] == 'completed', response
        return response

    def deny(self, ctx, event, name, code=None):
        before = ctx['show']()['result']['current_state']
        response = ctx['call']('event', ctx['run_id'], event)
        assert response['status'] == 'rejected', (name, response)
        if code:
            assert code in json.dumps(response), (name, code, response)
        after = ctx['show']()
        assert after['result']['current_state'] == before, (name, after)
        assert not after['result']['has_overrides'], after
        self.case(name, outcome='scripted request refused', event=event, code=code, response=response)

    def decision(self, ctx, phase, *, decision='approve', target=None, revision=None,
                 subject=None, reason=None, text=None):
        packet = ctx['invoke']([ctx['provider'], 'commission', '--slot', phase + '-checkpoint'],
                               ctx['show'](), cwd=ctx['repository'])
        view = packet['owner_approval_view']
        assert view['owner_approval']['status'] == 'not-performed', view
        expected_subject = phase + '-narrative.json' if phase in self.frozen['extra']['narratives'] else SUBJECT[phase]
        assert view['subject'] == expected_subject, view
        record_id = view['subject_record_id']
        assert any(r['id'] == record_id for r in ctx['show']()['result']['context']), view
        value = {'checkpoint':phase + '-checkpoint', 'subject':subject or view['subject'],
                 'subject_revision':revision or view['subject_revision'], 'decision':decision,
                 'target':target, 'reason':reason,
                 'text':text or f'SCRIPTED owner {decision} {phase}: preserve these exact words.\n  — End.\n',
                 'actor':{'name':'scripted-dummy-owner', 'kind':'script'}}
        appended = self.append(ctx, 'owner-decision', value)
        record = appended['result']['context']
        assert record['data'] == value, record
        self.decisions.append({'record_id':record['id'], 'label':LABEL, 'data':value})
        self.case('AC-5 scripted owner view names exact subject', phase=phase, view=view)
        return value

    def ledger(self, ctx, gate, subject, revision):
        self.append(ctx, 'finding-ledger', {'schema_version':'1', 'gate':gate,
                    'subject':subject, 'subject_revision':revision,
                    # Ledger authors permit human/agent, unlike other scripted records.
                    'author':{'name':'scripted-dummy-driver', 'kind':'agent'}, 'findings':[]})

    def review(self, ctx, gate):
        phase = gate.split('-')[0]
        subject = phase + '-narrative.json'
        revision = json.loads((ctx['artifacts'] / subject).read_text())['revision']
        policies = self.frozen['review_policies'][gate]
        groups = {}
        for policy in policies:
            groups.setdefault((policy.get('review_stage', 'aggregate'), policy['group']), []).append(policy)
        stages = sorted({stage for stage, _ in groups}, key=lambda stage: stage != 'individual')
        for stage in stages:
            shown = ctx['show']()
            commissioned = ctx['invoke']([ctx['provider'], 'commission', '--slot', gate, '--stage', stage],
                                         shown, cwd=ctx['repository'])
            sources = commissioned['review_sources']
            assert {s['locator'] for s in sources} == {'intent.json', SUBJECT[phase], subject, phase + '-narrative.md'}, commissioned
            assert commissioned['review_target']['revision'] == revision, commissioned
            assert commissioned['review_target']['admitted_identity']['record_id'] in {r['id'] for r in shown['result']['context']}, commissioned
            for source in sources:
                assert source['sha256'] == 'sha256:' + digest(Path(source['path'])), source
            for (group_stage, group), axes in groups.items():
                if group_stage != stage:
                    continue
                floor = max(p.get('required_authors', 1) for p in axes)
                for index in range(floor):
                    author = {'name':f'scripted-{gate}-{index}', 'kind':'script'}
                    judgments = [{'axis':p['id'], 'result':'pass', 'findings':'',
                                  'grounds':{'reason':LABEL, 'evidence':[{'locator':subject + '#/revision'}]}}
                                 for p in axes]
                    original = json.dumps({'assessment_id':f'{gate}-{revision}-{stage}-{group}-{index}',
                                           'judgments':judgments})
                    packet = {'show':ctx['show'](), 'assignment':{'gate':gate, 'stage':stage,
                              'group':group, 'subject':subject, 'subject_revision':revision, 'author':author},
                              'author':author, 'request':{'text':LABEL + '\n' + json.dumps({
                                  'gate':gate, 'stage':stage, 'subject_revision':revision,
                                  'sources':[{k:s[k] for k in ('locator','path','sha256')} for s in sources]})},
                              'original':{'text':original}}
                    intake = ctx['invoke']([ctx['provider'], 'review-candidates'], packet, cwd=ctx['repository'])
                    assert intake.get('records') and not intake.get('diagnostics'), intake
                    for candidate in intake['records']:
                        self.append(ctx, candidate['kind'], candidate['data'])
            self.case('AC-3 scripted narrative stage evidence', gate=gate, stage=stage,
                      revision=revision, floors=[p.get('required_authors', 1) for p in policies if p.get('review_stage','aggregate') == stage])
        self.ledger(ctx, gate, subject, revision)

    def pair(self, ctx, phase):
        artifact = ctx['artifacts'] / SUBJECT[phase]
        current = json.loads(artifact.read_text())
        record, approved = latest(ctx['show']()['result']['context'], 'approved-artifact', phase)
        assert approved['revision'] == current['revision'] and approved['sha256'] == digest(artifact)
        counter = self.counters.get(phase, 0) + 1
        self.counters[phase] = counter
        markdown = ctx['artifacts'] / (phase + '-narrative.md')
        markdown.write_text(f'# {phase.title()} — scripted mechanical fixture\n\n'
                            f'The approved {SUBJECT[phase]} revision {current["revision"]} is authoritative.\n'
                            'This fixture checks the durable CLI mechanics with dummy reviewers and owner decisions.\n'
                            'It proves neither semantic fidelity nor real owner approval.\n'
                            f'Narrative rewrite counter: {counter}.\n')
        binding = {'revision':f'{current["revision"]}/{counter}',
                   'author':{'name':'scripted-narrative-writer', 'kind':'script'},
                   'artifact':{'subject':SUBJECT[phase], 'revision':current['revision'], 'sha256':digest(artifact)},
                   'approved_artifact_record_id':record['id'],
                   'markdown':{'file':markdown.name, 'sha256':digest(markdown)}}
        write_json(ctx['artifacts'] / (phase + '-narrative.json'), binding)
        self.case('AC-3 binding written only after actual artifact approval', phase=phase,
                  approval_record=record, binding=binding)
        return binding

    def snapshot_pair(self, ctx, phase):
        saved = {name:(ctx['artifacts'] / name).read_bytes()
                 for name in (phase + '-narrative.json', phase + '-narrative.md')}
        archive = self.root / 'pair-snapshots'
        archive.mkdir(exist_ok=True)
        for name, contents in saved.items():
            path = Path(name)
            (archive / (path.stem + '-' + hashlib.sha256(contents).hexdigest() + path.suffix)).write_bytes(contents)
        return saved

    def restore_pair(self, ctx, saved):
        for name, contents in saved.items():
            (ctx['artifacts'] / name).write_bytes(contents)

    def bump_design(self, ctx, revision):
        path = ctx['artifacts'] / 'design.json'
        design = json.loads(path.read_text())
        design['revision'] = revision
        write_json(path, design)
        plan_path = ctx['artifacts'] / 'plan.json'
        plan = json.loads(plan_path.read_text())
        plan['design_revision'] = revision
        write_json(plan_path, plan)

    def admit_narrative(self, ctx, phase):
        subject = ctx['artifacts'] / SUBJECT[phase]
        before = subject.read_bytes()
        old = self.pairs.get(phase)
        if phase == 'design' and old and 'narrative-rejected' in self.flags and 'old-narrative-ready' not in self.flags:
            self.restore_pair(ctx, old)
            self.deny(ctx, 'narrative-ready', 'AC-3/6 restoring reviewed narrative at narrative-ready refused',
                      'software-change-narrative-admission-invalid')
            self.flags.add('old-narrative-ready')
        if phase == 'design' and old and 'artifact-rejected' in self.flags and 'stale-artifact-prefix' not in self.flags:
            self.restore_pair(ctx, old)
            self.deny(ctx, 'narrative-ready', 'AC-3 old artifact narrative cannot bind NEW artifact revision',
                      'software-change-narrative-binding-invalid')
            self.flags.add('stale-artifact-prefix')
        binding = self.pair(ctx, phase)
        binding_path = ctx['artifacts'] / (phase + '-narrative.json')
        if phase == 'design' and 'binding-negatives' not in self.flags:
            for field, value, name in [
                ('revision', 'unrelated/1', 'wrong composite prefix'),
                ('approved_artifact_record_id', 'not-an-engine-record', 'wrong approved record ID'),
                ('artifact', {**binding['artifact'], 'subject':'plan.json'}, 'wrong phase'),
                ('artifact', {**binding['artifact'], 'sha256':'0'*64}, 'wrong artifact bytes')]:
                invalid = copy.deepcopy(binding)
                invalid[field] = value
                write_json(binding_path, invalid)
                self.deny(ctx, 'narrative-ready', 'AC-3 binding refused: ' + name,
                          'software-change-narrative-binding-invalid')
            write_json(binding_path, binding)
            self.flags.add('binding-negatives')
        ctx['event']('narrative-ready')
        admission, admitted = latest(ctx['show']()['result']['context'], 'narrative-admitted', phase)
        assert admitted['revision'] == binding['revision']
        assert admitted['sha256'] == digest(ctx['artifacts'] / binding['markdown']['file'])
        assert subject.read_bytes() == before, 'Narrative step changed authoritative JSON'
        if phase == 'design' and old and 'old-narrative-ready' in self.flags and 'old-narrative-after-admission' not in self.flags:
            new = self.snapshot_pair(ctx, phase)
            self.restore_pair(ctx, old)
            self.deny(ctx, 'approved', 'AC-3/6 older narrative restored AFTER fresh admission refused',
                      'software-change-narrative-stale')
            self.restore_pair(ctx, new)
            self.flags.add('old-narrative-after-admission')
        self.pairs[phase] = self.snapshot_pair(ctx, phase)
        self.case('AC-3 actual narrative admission persisted with unchanged artifact', phase=phase, record=admission)

    def checkpoint(self, ctx, phase):
        forward = 'passed' if phase == 'validation' else 'approved'
        self.deny(ctx, forward, f'AC-5 {phase} checkpoint missing/fresh owner approval refused',
                  'software-change-owner-decision-required')
        self.decision(ctx, phase, revision='obsolete/revision')
        self.deny(ctx, forward, f'AC-5 {phase} checkpoint stale owner approval refused',
                  'software-change-owner-decision-required')
        if phase == 'design' and not self.reviewless:
            if 'narrative-rejected' not in self.flags:
                self.decision(ctx, phase, decision='reject', target='narrative', reason='', text='SCRIPTED invalid rejection: no reason.')
                self.deny(ctx, 'revise-narrative', 'AC-6 rejection missing reason refused', 'software-change-owner-decision-required')
                self.decision(ctx, phase, decision='reject', reason='SCRIPTED no target.', text='SCRIPTED invalid rejection: no target.')
                self.deny(ctx, 'revise', 'AC-6 rejection missing target refused', 'software-change-owner-decision-required')
                self.decision(ctx, phase)
                self.decision(ctx, phase, decision='reject', target='narrative', revision='another/revision', reason='SCRIPTED other revision rejection.')
                self.deny(ctx, forward, 'AC-5 latest other-revision rejection outranks older matching approval', 'software-change-owner-decision-required')
                self.decision(ctx, phase, decision='reject', target='narrative', reason='SCRIPTED rewrite only the prose.',
                              text='SCRIPTED No — rewrite the narrative only.\n  Keep the artifact unchanged.\n')
                self.artifact_review_ids = {r['id'] for r in ctx['show']()['result']['context']
                                           if r['kind'] == 'review-evidence' and r['data'].get('gate') in ('design-review','design-adversarial-review')}
                (ctx['artifacts'] / 'design-narrative.json').write_text('{early unparseable rewrite')
                self.deny(ctx, forward, 'AC-6 early narrative edit cannot approve')
                ctx['event']('revise-narrative')
                assert (ctx['artifacts'] / 'design.json').read_bytes() == self.saved_artifacts['current-design']
                self.flags.add('narrative-rejected')
                self.case('AC-6 rejection-backed revise-narrative recovers unparseable early edit')
                return
            if 'artifact-rejected' not in self.flags:
                current_ids = {r['id'] for r in ctx['show']()['result']['context']
                               if r['kind'] == 'review-evidence' and r['data'].get('gate') in ('design-review','design-adversarial-review')}
                assert current_ids == self.artifact_review_ids, 'Narrative-only rewrite reopened artifact reviews'
                self.case('AC-6 narrative-only rewrite preserves exact artifact evidence IDs', record_ids=sorted(current_ids))
                self.decision(ctx, phase, decision='reject', target='artifact', reason='SCRIPTED require fresh artifact review.',
                              text='SCRIPTED No — reject the artifact.\n  Reason retained verbatim.\n')
                self.pre_rejection = [r for r in ctx['show']()['result']['context'] if r['kind'] == 'review-evidence'
                                      and r['data'].get('gate') == 'design-review']
                (ctx['artifacts'] / 'design.json').write_text('{early unparseable artifact edit')
                self.deny(ctx, forward, 'AC-6 early artifact edit cannot approve')
                ctx['event']('revise')
                (ctx['artifacts'] / 'design.json').write_bytes(self.saved_artifacts['first-design'])
                self.deny(ctx, 'design-ready', 'AC-6 reviewed but never approved artifact ready refused AFTER rejection',
                          'software-change-artifact-admission-invalid')
                (ctx['artifacts'] / 'design.json').write_bytes(self.saved_artifacts['current-design'])
                self.deny(ctx, 'design-ready', 'AC-6 previously approved artifact ready refused AFTER rejection',
                          'software-change-artifact-admission-invalid')
                self.bump_design(ctx, 'scripted-after-rejection')
                self.flags.add('artifact-rejected')
                self.case('AC-6 rejection-backed revise recovers unparseable early edit; new identity same content')
                return
        self.decision(ctx, phase)
        if phase == 'intent':
            path = ctx['artifacts'] / SUBJECT[phase]
            before = path.read_bytes()
            value = json.loads(before)
            value['outcome'] += ' SCRIPTED same-revision edit.'
            write_json(path, value)
            self.deny(ctx, forward, 'AC-5 current owner approval cannot admit edited artifact bytes',
                      'software-change-artifact-stale')
            path.write_bytes(before)
        if phase == 'validation' and self.withheld:
            self.deny(ctx, forward, 'AC-3/5 final checkpoint retains complete validation index duty', 'software-change-criterion-incomplete')
            self.release_verdicts(ctx)
        ctx['event'](forward)
        self.case('AC-5 current scripted owner approval admits checkpoint', phase=phase)
        if phase == 'implementation':
            history = list((ctx['artifacts'] / 'implementation-proof-history').glob('*.json'))
            assert history, 'No accepted implementation proof on actual validation entry'
            self.case('AC-3/5 accepted implementation proof retained on actual validation entry', files=[str(p) for p in history])

    def release_verdicts(self, ctx):
        for candidate in self.withheld:
            self.append(ctx, candidate['kind'], candidate['data'], candidate['record_id'])
        self.withheld = []

    def exercise(self, state, ctx):
        artifacts = ctx['artifacts']
        if state == 'validation-judgments':
            if not self.reviewless:
                return False
            report = ctx['report']
            for index, batch in enumerate(ctx['prepared']['judgment_batches']):
                verdict = {'subject':'validation-report.json', 'subject_revision':report['revision'],
                           'checkpoint':'validation-checkpoint.json',
                           'author':{'name':f'scripted-dummy-judge-{index}', 'kind':'script'},
                           'result':'pass', 'findings':[], 'reason':LABEL,
                           'evidence_context_ids':report['command_evidence_ids']}
                if batch['kind'] == 'criterion-verdict':
                    verdict['criterion_id'] = batch['criterion_id']
                self.withheld.append({'record_id':batch['record_id'], 'kind':batch['kind'], 'data':verdict})
            self.case('Completion fixture withholds indexed scripted verdicts until final hop', records=self.withheld)
            return True
        if state == 'validation-prepared':
            return False
        shown = ctx['show']()
        assert shown['result']['initial_input'] == self.frozen, 'Started run options/policies changed'
        if 'freeze' not in self.flags:
            edited = copy.deepcopy(self.frozen)
            edited['extra']['narratives'] = []
            edited['extra']['checkpoints'] = []
            write_json(ctx['work'] / 'profile.json', edited)
            assert ctx['show']()['result']['initial_input'] == self.frozen
            self.flags.add('freeze')
            self.case('AC-2 source profile edited after start; exact frozen input unchanged')
        if state in ('design-review', 'design-adversarial-review') and not self.reviewless:
            if state == 'design-adversarial-review' and 'artifact-return-no-rejection' not in self.flags:
                self.saved_artifacts['first-design'] = (artifacts / 'design.json').read_bytes()
                early = ctx['invoke']([ctx['provider'],'commission','--slot','design-narrative-review'], shown, cwd=ctx['repository'])
                assert early.get('missing_inputs') and early.get('approved_artifact') is None, early
                self.case('AC-3 narrative binding has no approval identity before artifact ordinary/challenge clearance')
                ctx['event']('revise')
                self.deny(ctx, 'design-ready', 'AC-3 admitted revision ready refused WITHOUT rejection', 'software-change-artifact-admission-invalid')
                self.bump_design(ctx, 'scripted-no-rejection')
                ctx['event']('design-ready')
                fresh = (artifacts / 'design.json').read_bytes()
                (artifacts / 'design.json').write_bytes(self.saved_artifacts['first-design'])
                self.deny(ctx, 'approved', 'AC-3 older artifact restored during review AFTER new admission refused', 'software-change-artifact-stale')
                (artifacts / 'design.json').write_bytes(fresh)
                self.saved_artifacts['current-design'] = fresh
                self.flags.add('artifact-return-no-rejection')
                return True
            if state == 'design-review' and 'artifact-rejected' in self.flags and 'applicability-cutoff' not in self.flags:
                revision = json.loads((artifacts / 'design.json').read_text())['revision']
                for record in self.pre_rejection:
                    self.append(ctx, 'evidence-applicability', {'origin':{'kind':'context-record','id':record['id']},
                                'target':{'subject':'design.json','revision':revision},
                                'attesting_driver':{'name':'scripted-dummy-driver','kind':'script'},
                                'reason':LABEL + ': original judgment scope declared unchanged, not refreshed.'})
                self.ledger(ctx, state, 'design.json', revision)
                self.deny(ctx, 'approved', 'AC-6 pre-rejection judgment carried later NOT counted', 'software-change-review-incomplete')
                self.flags.add('applicability-cutoff')
                self.case('AC-6 same content NEW revision requires fresh artifact reviews')
        if state in [p + '-narrative' for p in NARRATIVES]:
            self.admit_narrative(ctx, state.split('-')[0])
            return True
        if '-narrative-' in state and state.endswith('review'):
            phase = state.split('-')[0]
            if phase == 'design' and 'prose-changed' not in self.flags:
                old = self.snapshot_pair(ctx, phase)
                markdown = artifacts / 'design-narrative.md'
                markdown.write_text(markdown.read_text() + '\nSCRIPTED changed prose under admitted identity.\n')
                binding_path = artifacts / 'design-narrative.json'
                binding = json.loads(binding_path.read_text())
                binding['markdown']['sha256'] = digest(markdown)
                write_json(binding_path, binding)
                self.deny(ctx, 'approved', 'AC-3 changed prose under admitted identity refused', 'software-change-narrative-stale')
                self.restore_pair(ctx, old)
                self.flags.add('prose-changed')
            revision = json.loads((artifacts / (phase + '-narrative.json')).read_text())['revision']
            self.ledger(ctx, state, phase + '-narrative.json', revision)
            forward = 'passed' if any(e['event'] == 'passed' for e in shown['result']['requestable_events']) else 'approved'
            self.deny(ctx, forward, 'AC-3 narrative cannot leave without fresh configured evidence', 'software-change-review-incomplete')
            self.review(ctx, state)
            if forward == 'passed' and self.withheld:
                self.deny(ctx, forward, 'AC-3 final narrative hop retains complete validation index duty', 'software-change-criterion-incomplete')
                self.release_verdicts(ctx)
            ctx['event'](forward)
            self.case('AC-3 scripted ordinary/challenge narrative clearance', gate=state, revision=revision)
            return True
        if state.endswith('-checkpoint'):
            self.checkpoint(ctx, state.split('-')[0])
            return True
        if state == 'end':
            return False
        return False

    def execute(self):
        profile = ROOT / 'crates/software-change-provider/data/configs' / ('minimal.json' if self.reviewless else 'high-rigor.json')
        try:
            work = SOFTWARE.run(self.args, profile, self.root, configure=self.configure,
                                exercise=self.exercise, command_log=self.log, judgment_reason=LABEL)
            terminal = json.loads((work / 'terminal.json').read_text())
            context = terminal['result']['context']
            history = json.loads((work / 'history.json').read_text())
            # Public history points to durable context IDs, not embedded decision text.
            # Join only actual full-show rows to those IDs; never fabricate history entries.
            for decision in self.decisions:
                assert decision['record_id'] in json.dumps(history), (decision, history)
                record = next(r for r in context if r['id'] == decision['record_id'])
                assert record['data'] == decision['data'], record
            write_json(work / 'owner-decision-history.json', {'public_history':history,
                       'actual_context_records':[r for r in context if r['kind'] == 'owner-decision'], 'label':LABEL})
            for phase in self.frozen['extra']['narratives']:
                markdown = work / 'artifacts' / (phase + '-narrative.md')
                assert markdown.is_file() and markdown.read_text()
                record, admitted = latest(context, 'narrative-admitted', phase)
                assert admitted['sha256'] == digest(markdown), record
            if self.reviewless:
                enclosing, approved = latest(context, 'approved-artifact', 'implementation')
                assert enclosing['kind'] == 'reconciliation-decision', enclosing
                data = enclosing['data']
                assert set(data) == {'revision','sha256','documents','artifact_admitted','approved_artifact'}, data
                reconciliation = json.loads((work / 'artifacts/reconciliation.json').read_text())
                assert data['revision'] == reconciliation['revision']
                assert data['sha256'] == 'sha256:' + digest(work / 'artifacts/reconciliation.json')
                assert data['documents'] == {p:json.loads((work / 'artifacts' / SUBJECT[p]).read_text())['revision'] for p in ('intent','design','plan')}
                assert data['artifact_admitted']['sha256'] == approved['sha256']
                assert data['artifact_admitted']['revision'] == approved['revision']
                if not self.final_narrative:
                    actual, approval = latest(context, 'approved-artifact', 'design')
                    assert actual['kind'] == 'artifact-admitted' and actual['data']['approved_artifact'] == approval
                    binding = json.loads((work / 'artifacts/design-narrative.json').read_text())
                    assert binding['approved_artifact_record_id'] == actual['id']
                self.case('AC-3/5 real engine single-record atomic collisions preserve enclosing IDs/fields', record=enclosing)
            else:
                required = {'binding-negatives','freeze','artifact-return-no-rejection','narrative-rejected',
                            'old-narrative-ready','old-narrative-after-admission','artifact-rejected',
                            'applicability-cutoff','prose-changed','stale-artifact-prefix'}
                assert required <= self.flags, required - self.flags
                assert set(self.pairs) == set(NARRATIVES)
            assert not self.withheld and terminal['result']['current_state'] == 'end'
            assert terminal['result']['lifecycle'] == 'final' and not terminal['result']['has_overrides']
            self.result('passed-terminal', work_root=str(work), database=str(work / 'run.sqlite'),
                        terminal=str(work / 'terminal.json'), history=str(work / 'history.json'),
                        scripted_reviews=[{'label':LABEL, 'actual_context_record':r} for r in context if r['kind'] == 'review-evidence'],
                        scripted_validation_judgments=[{'label':LABEL, 'actual_context_record':r} for r in context if r['kind'] in ('criterion-verdict','goal-verdict','intent-comparison')],
                        scripted_intent_comparison_authorizations=[{'label':LABEL, 'actual_context_record':r} for r in context if r['kind'] == 'user-steering'],
                        actual_command_outcomes=str(self.log.index))
        except Exception as error:
            self.result('failed', error=str(error))
            raise
        finally:
            self.log.finalize()


def self_test():
    """Deterministic stub-backed interface checks; zero real engine/provider calls."""
    import subprocess
    from unittest.mock import patch
    SOFTWARE.self_test()
    # Exercise the emitted helper payload, not a separate hand-written ledger.
    # finding_ledger::parse_author accepts human/agent only; keep the scripted name.
    ledger_driver = Journey.__new__(Journey)
    emitted = []
    ledger_driver.append = lambda ctx, kind, data: emitted.append((kind, data))
    for phase in NARRATIVES:
        for suffix in ('review','adversarial-review'):
            gate = phase + '-narrative-' + suffix
            subject = phase + '-narrative.json'
            ledger_driver.ledger({}, gate, subject, 'fixture-artifact/2')
            kind, data = emitted[-1]
            assert kind == 'finding-ledger'
            assert set(data) == {'schema_version','gate','subject','subject_revision','author','findings'}
            assert data['schema_version'] == '1' and data['findings'] == []
            assert (data['gate'], data['subject'], data['subject_revision']) == (gate, subject, 'fixture-artifact/2')
            assert data['author'] == {'name':'scripted-dummy-driver','kind':'agent'}
    ledger_driver.ledger({}, 'design-review', 'design.json', 'fixture-artifact')
    assert emitted[-1][1]['author']['kind'] == 'agent'
    # Criterion/goal parser DOES accept script. Audit the actual withheld-record
    # path so this ledger-only correction cannot become a blanket actor rewrite.
    judgment_driver = Journey.__new__(Journey)
    judgment_driver.reviewless, judgment_driver.withheld = True, []
    judgment_driver.case = lambda *args, **kwargs: None
    report = {'revision':'fixture-validation','command_evidence_ids':['stub-retained-command']}
    prepared = {'judgment_batches':[{'kind':'criterion-verdict','record_id':'stub-criterion','criterion_id':'AC-1'},
                                   {'kind':'goal-verdict','record_id':'stub-goal'}]}
    assert judgment_driver.exercise('validation-judgments', {'artifacts':Path('/unused-stub-artifacts'),
                                   'report':report, 'prepared':prepared})
    for candidate in judgment_driver.withheld:
        data = candidate['data']
        expected_fields = {'subject','subject_revision','checkpoint','author','result','findings','reason','evidence_context_ids'}
        if candidate['kind'] == 'criterion-verdict':
            expected_fields.add('criterion_id')
            assert data['criterion_id'] == 'AC-1'
        assert set(data) == expected_fields
        assert data['author']['kind'] == 'script'
        assert data['subject'] == 'validation-report.json' and data['checkpoint'] == 'validation-checkpoint.json'
        assert data['subject_revision'] == report['revision'] and data['evidence_context_ids'] == report['command_evidence_ids']
        assert data['result'] == 'pass' and data['findings'] == [] and data['reason'] == LABEL
    with tempfile.TemporaryDirectory(prefix='narrative-interface-') as directory:
        log = SOFTWARE.CommandLog(Path(directory) / 'captures')
        completed = subprocess.CompletedProcess(['stub'], 1, json.dumps({'status':'rejected','result':{'code':'fixture-denial'}}).encode('utf-8'), b'scripted stderr')
        with patch.object(SOFTWARE.subprocess, 'run', return_value=completed) as stub:
            value = log.execute(['stub', 'event'], {'scripted':True}, cwd=directory)
            assert value['status'] == 'rejected' and len(log.rows) == 1
            assert log.rows[0]['exit_code'] == 1
            assert (log.root / log.rows[0]['stderr']['path']).read_bytes() == b'scripted stderr'
            assert json.loads(log.index.read_text()) == log.rows[0]
            assert not (log.root / 'commands.json').exists()
            env = stub.call_args.kwargs['env']
            assert not any(k.startswith(('LOOP_', 'XDG_')) for k in env)
        bad = subprocess.CompletedProcess(['stub'], 1, json.dumps({'status':'error'}).encode('utf-8'), b'failure')
        with patch.object(SOFTWARE.subprocess, 'run', return_value=bad):
            try:
                log.execute(['stub'])
            except RuntimeError:
                pass
            else:
                raise AssertionError('Unexpected command error accepted')
        assert len(log.index.read_text().splitlines()) == 2
        log.finalize()
        assert json.loads((log.root / 'commands.json').read_text()) == log.rows
        row = {'id':'real-enclosing-id','kind':'reconciliation-decision','sequence':9,
               'data':{'revision':'reconciliation-r1','sha256':'sha256:original','documents':{},
                       'artifact_admitted':{'phase':'implementation','subject':SUBJECT['implementation'],'revision':'r1','sha256':'a'*64},
                       'approved_artifact':{'subject':SUBJECT['implementation'],'revision':'r1','sha256':'a'*64}}}
        selected, approved = latest([row], 'approved-artifact', 'implementation')
        assert selected is row and selected['id'] == 'real-enclosing-id' and approved is row['data']['approved_artifact']
        # Refusal assertions fail closed on a disabled check, wrong denial code,
        # changed state, or override. These are interface negatives, NOT AC proof.
        journey = Journey.__new__(Journey)
        recorded = []
        journey.case = lambda name, **details: recorded.append((name, details))
        shown = {'result':{'current_state':'design-checkpoint','has_overrides':False}}
        response = {'status':'rejected','result':{'code':'fixture-denial'}}
        ctx = {'show':lambda:shown, 'call':lambda *argv:response, 'run_id':'stub'}
        journey.deny(ctx, 'approved', 'scripted interface refusal', 'fixture-denial')
        assert len(recorded) == 1
        for bad_response in [{'status':'completed'}, {'status':'rejected','result':{'code':'wrong-code'}}]:
            ctx['call'] = lambda *argv, value=bad_response:value
            try:
                journey.deny(ctx, 'approved', 'disabled-check stub', 'fixture-denial')
            except AssertionError:
                pass
            else:
                raise AssertionError('Disabled/wrong check escaped refusal assertion')
        ctx['call'] = lambda *argv:response
        shown['result']['has_overrides'] = True
        try:
            journey.deny(ctx, 'approved', 'override stub', 'fixture-denial')
        except AssertionError:
            pass
        else:
            raise AssertionError('Override escaped refusal assertion')
    with tempfile.TemporaryDirectory(prefix='off-profile-interface-') as directory:
        root = Path(directory)
        source = ROOT / 'crates/software-change-provider/data/configs/minimal.json'
        predecessor = json.loads(source.read_text())
        predecessor['config_version'] = 'minimal-14'
        for key in ('narratives','checkpoints','narrative_review'):
            predecessor['extra'].pop(key, None)
        calls, runs = [], []
        class StubLog:
            def __init__(self):
                self.root = root / 'captures'
                self.index = self.root / 'commands.jsonl'
            def execute(self, argv, packet=None, **kwargs):
                argv = list(map(str, argv))
                calls.append(argv)
                if argv[-1] == '--version':
                    return subprocess.CompletedProcess(argv, 0, 'software-change 0.24.1\n', '')
                if 'data-dump' in argv:
                    target = Path(argv[-1]) / 'crates/software-change-provider/data/configs/minimal.json'
                    target.parent.mkdir(parents=True)
                    write_json(target, predecessor)
                    return subprocess.CompletedProcess(argv, 0, '', '')
                if argv[:2] == ['git','show']:
                    return subprocess.CompletedProcess(argv, 0, json.dumps(predecessor), '')
                return {'scripted':'same complete graph'}
        def stub_run(args, profile_source, root, **kwargs):
            runs.append(args.provider)
            work = Path(root) / str(len(runs))
            work.mkdir()
            write_json(work / 'states.json', ['explore','end'])
            return work
        args = argparse.Namespace(provider='current-stub', predecessor_provider='old-stub')
        with patch.object(SOFTWARE, 'run', side_effect=stub_run):
            SOFTWARE.compare_off(args, source, root, StubLog())
        parity = json.loads((root / 'minimal-off-comparison.json').read_text())
        assert parity['historical_binary_comparison'] and runs == [str(Path('old-stub').resolve()), 'current-stub']
        assert parity['actual_command_outcomes'] == str(root / 'captures/commands.jsonl')
        assert not any(argv[0] == 'git' for argv in calls), 'Historical mode guessed git source instead of explicit embedded data'
        fallback = root / 'profile-only'
        fallback.mkdir()
        args.predecessor_provider = None
        with patch.object(SOFTWARE, 'run', side_effect=stub_run):
            SOFTWARE.compare_off(args, source, fallback, StubLog())
        parity = json.loads((fallback / 'minimal-off-comparison.json').read_text())
        assert not parity['ac1_historical_mechanical_proof'] and 'NOT complete historical AC-1' in parity['limitation']
    print('narrative/checkpoint collection and stub interface checks passed; NOT progression proof; ' + LABEL)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--engine', help='Built loop-engine executable (required for real progression)')
    parser.add_argument('--provider', help='Built software-change executable (required for real progression)')
    parser.add_argument('--output-root', '--work-root', dest='work_root', help='Fresh external capture/catalog root')
    parser.add_argument('--self-test', action='store_true', help='Collection/stub interface checks ONLY; no real progression')
    args = parser.parse_args()
    if sys.flags.optimize:
        parser.error('Optimized Python disables proof assertions; run without -O')
    if args.self_test:
        self_test()
        return
    if not args.engine or not args.provider:
        parser.error('--engine and --provider are required for real proof')
    root = Path(args.work_root).resolve() if args.work_root else Path(tempfile.mkdtemp(prefix='narrative-checkpoint-journey-'))
    if root == ROOT or root.is_relative_to(ROOT):
        parser.error('Capture/catalog root must be outside the checkout')
    if args.work_root and root.exists() and any(root.iterdir()):
        parser.error('Use a new/empty external output root; receipts are never overwritten')
    root.mkdir(parents=True, exist_ok=True)
    print(f'Retained narrative/checkpoint proof root: {root}; {LABEL}', flush=True)
    args.engine, args.provider = str(Path(args.engine).resolve()), str(Path(args.provider).resolve())
    args.mode, args.data_root = 'source', None
    for name, reviewless, final_narrative in [('high-rigor-all-options',False,False),
                                             ('minimal-reviewless-checkpoints',True,False),
                                             ('minimal-reviewless-final-narrative',True,True)]:
        Journey(args, root / name, name, reviewless=reviewless, final_narrative=final_narrative).execute()
    write_json(root / 'result.json', {'status':'passed-terminal', 'label':LABEL,
               'fixtures':['high-rigor-all-options','minimal-reviewless-checkpoints','minimal-reviewless-final-narrative'],
               'off_profile_comparisons':'Run software-change-journey.py separately on each shipped -15 profile',
               'semantic_review':'not-performed'})
    print(f'narrative/checkpoint journeys reached actual end; receipts: {root}; {LABEL}')


if __name__ == '__main__':
    main()
