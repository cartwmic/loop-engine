#!/usr/bin/env python3
"""Black-box proof of valid-evidence-carry behavior (issues #30, #31, #40, #41).

SCRIPTED DUMMY BACKENDS supply every review, verdict and owner statement; they establish
no semantic review and no real approval. The built loop-engine and software-change
programs run against private catalogs in a driver-owned output root. The installed
v0.26.0 programs are the control only: they must still show the defects this change fixes.
--self-test checks collection and interface mechanics only; it proves no behavior.
"""
# bookends:LE-114 bookends:LE-149 — admission and the checked gate share one command-carry eligibility answer.
# bookends:LE-44 bookends:LE-47 — intake supplies durable rows; it never grants progression.
import argparse
import copy
import importlib.util
import json
import sys
import tempfile
from pathlib import Path
from types import SimpleNamespace

ROOT = Path(__file__).resolve().parents[1]
sys.dont_write_bytecode = True  # Journey artifacts and captures stay outside the checkout.
SPEC = importlib.util.spec_from_file_location('software_journey', ROOT / 'scripts/software-change-journey.py')
SW = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SW)
LABEL = 'SCRIPTED DUMMY BACKENDS: mechanical proof only, not semantic review or human approval'
PROFILES = ROOT / 'crates/software-change-provider/data/configs'
# AC -> scenario names that prove it. --self-test checks every AC has at least one.
COVERAGE = {
    'AC-1': ['review-cases'], 'AC-2': ['review-cases'], 'AC-3': ['review-cases'],
    'AC-4': ['command-carry'], 'AC-5': ['command-carry'],
    'AC-6': ['review-cases', 'intent-cases'], 'AC-7': ['review-cases'],
    'AC-8': ['locator-cases'], 'AC-9': ['locator-cases'], 'AC-10': ['locator-cases'],
    'AC-11': ['command-carry', 'review-cases', 'intent-cases', 'locator-cases'],
    'AC-12': ['intent-cases'],
}


class Failure(Exception):
    pass


def require(condition, detail=None):
    if not condition:
        raise Failure(json.dumps(detail, indent=2, default=str)[:4000] if detail is not None else 'failed')


def dump(path, value):
    Path(path).write_text(json.dumps(value, indent=2, default=str) + '\n')


def snapshot(ctx):
    shown = ctx['show']()['result']
    return {'state': shown['current_state'], 'context': [r['id'] for r in shown['context']],
            'history': ctx['call']('history', ctx['run_id'])['result']}


def unchanged(before, after, what):
    require(before == after, {'refusal changed state/context/history': what})


def append(ctx, kind, data, rid=None):
    argv = ['append'] + (['--record-id', rid] if rid else []) + [ctx['run_id'], kind, json.dumps(data)]
    response = ctx['call'](*argv)
    require(response['status'] == 'completed', response)
    return response


def refuse(ctx, kind, data, rid, rule, message):
    before = snapshot(ctx)
    response = ctx['call']('append', '--record-id', rid, ctx['run_id'], kind, json.dumps(data))
    require(response['status'] == 'rejected' and response['code'] == 'record-refused', response)
    require(response['details']['rule'] == rule and message in response['details']['message'], response)
    unchanged(before, snapshot(ctx), rid)
    return response


def make_args(base, engine, provider):
    return SimpleNamespace(engine=str(engine), provider=str(provider), mode='source', data_root=None)


# ---- AC-4 / AC-5: a fresh verdict citing the original command, selected through a current carry ----

def verdict(report, command, criterion=None):
    data = {'subject': 'validation-report.json', 'subject_revision': report['revision'],
            'checkpoint': 'validation-checkpoint.json',
            'author': {'name': 'scripted-independent-judge', 'kind': 'script'},
            'result': 'pass', 'findings': [], 'evidence_context_ids': [command],
            'reason': 'SCRIPTED FIXTURE: inspected actual helper output; mechanical coverage only.',
            'gate': 'validation-review', 'review_stage': 'aggregate',
            'config_version': 'evidence-carry-journey'}
    if criterion is not None:
        data['criterion_id'] = criterion
    return data


def command_carry(args, root, engine, provider, control):
    """Issue #31 sequence. Built programs accept; the v0.26.0 control reproduces two refusals."""
    root.mkdir(parents=True, exist_ok=True)
    state = {}
    outcome = {'engine': str(engine), 'control': control}

    def configure(profile, repository, artifacts):
        profile['review_policies'] = {'validation-review': [{
            'id': 'delivery', 'description': 'Scripted mechanical delivery check',
            'example_prompt': 'Inspect the fixture output.', 'required_authors': 1,
            'review_stage': 'aggregate', 'group': 'validation-review.aggregate', 'self_review': False}]}
        profile['criterion_policy'] = {'required_authors': 1, 'goal_required_authors': 1}
        profile['config_version'] = 'evidence-carry-journey'
        profile['extra'].pop('checkpoints', None)
        profile['extra'].pop('narratives', None)
        path = artifacts / 'intent.json'
        intent = json.loads(path.read_text())
        intent['source_scopes'] = {'primary': {'root': str(repository)}}
        path.write_text(json.dumps(intent))

    class Staged(Exception):
        pass

    def exercise(phase, ctx):
        if phase != 'validation-judgments':
            return False
        state['ctx'] = ctx
        command = ctx['prepared']['command_candidates'][0]
        state['command'] = command
        (ctx['repository'] / 'unrelated-note.txt').write_text('Unrelated source B fixture note\n')
        source_b = ctx['invoke']([ctx['provider'], 'source-identity', '--working-directory', ctx['repository']])
        source_a = command['data']['external_result']['source']
        require(source_a['head'] == source_b['head'] and source_a['state_sha256'] != source_b['state_sha256'],
                (source_a, source_b))
        path = ctx['artifacts'] / 'implementation-report.json'
        implementation = json.loads(path.read_text())
        implementation['revision'] = 'implementation-B'
        path.write_text(json.dumps(implementation))
        ctx['event']('revise-implementation')
        raise Staged()

    log = SW.CommandLog(root / 'commands')
    profile = PROFILES / 'minimal.json'
    try:
        SW.run(make_args(args, engine, provider), profile, root, configure=configure,
               exercise=exercise, command_log=log)
        raise Failure('journey ended before the carry sequence')
    except Staged:
        pass
    ctx, command = state['ctx'], state['command']
    ctx['checkpoint']('implementation')
    ctx['event']('implementation-ready')
    ctx['event']('reconciliation-ready')
    report = {'revision': 'validation-B', 'author': {'name': 'proof-driver', 'kind': 'script'},
              'implementation_revision': 'implementation-B', 'command_evidence_ids': ['carry-A-B'],
              'criteria': [{'criterion_id': row['criterion_id'], 'verdict_ids': ['fresh-' + row['criterion_id']]}
                           for row in ctx['report']['criteria']],
              'goal_verdict_ids': ['fresh-goal']}
    (ctx['artifacts'] / 'validation-report.json').write_text(json.dumps(report))
    ctx['checkpoint']('validation')
    declaration = {'origin': {'kind': 'context-record', 'id': command['record_id']},
                   'target': {'subject': 'validation-report.json', 'revision': 'validation-B',
                              'checkpoint': {'phase': 'validation', 'report_revision': 'validation-B'}},
                   'attesting_driver': {'name': 'fixture-driver', 'kind': 'script'},
                   'reason': 'Only unrelated-note.txt changed; the helper command obligation is unchanged.'}
    wrong = copy.deepcopy(declaration)
    wrong['target']['revision'] = 'wrong-target'
    refuse(ctx, 'evidence-applicability', wrong, 'negative-wrong-target',
           'review-applicability-origin-target', 'target checkpoint is not current')
    append(ctx, 'evidence-applicability', declaration, 'carry-A-B')
    ctx['event']('validation-ready')
    require(ctx['show']()['result']['current_state'] == 'validation-review', 'current index refused')
    # AC-5 controls that must refuse the same way before and after this change.
    criterion = report['criteria'][0]['criterion_id']
    refuse(ctx, 'criterion-verdict', verdict(report, 'carry-A-B', criterion), 'negative-wrapper',
           'criterion-verdict-shape-source', 'unsupported evidence reference')
    refuse(ctx, 'criterion-verdict', verdict(report, 'missing', criterion), 'negative-missing',
           'criterion-verdict-shape-source', 'missing or duplicate context ID')
    attempts = {}
    for kind, crit, rid in [('criterion-verdict', criterion, report['criteria'][0]['verdict_ids'][0]),
                            ('goal-verdict', None, report['goal_verdict_ids'][0])]:
        before = snapshot(ctx)
        response = ctx['call']('append', '--record-id', rid, ctx['run_id'], kind,
                               json.dumps(verdict(report, command['record_id'], crit)))
        attempts[kind] = response
        if response['status'] == 'rejected':
            unchanged(before, snapshot(ctx), kind)
    outcome['verdict_appends'] = {k: {'status': v['status'], 'rule': v.get('details', {}).get('rule'),
                                      'message': v.get('details', {}).get('message')}
                                  for k, v in attempts.items()}
    if control:
        for kind, rule in [('criterion-verdict', 'criterion-verdict-current-target'),
                           ('goal-verdict', 'goal-verdict-current-target')]:
            response = attempts[kind]
            require(response['status'] == 'rejected' and response['details']['rule'] == rule
                    and 'stale-source' in response['details']['message'], response)
        outcome['result'] = 'control reproduced both v0.26.0 refusals'
    else:
        require(all(v['status'] == 'completed' for v in attempts.values()), attempts)
        for row in report['criteria'][1:]:
            append(ctx, 'criterion-verdict', verdict(report, command['record_id'], row['criterion_id']),
                   row['verdict_ids'][0])
        stored = next(r for r in ctx['show']()['result']['context'] if r['id'] == command['record_id'])
        require(stored['data'] == command['data'], 'the original command record changed')
        outcome['result'] = 'accepted; original command record unchanged'
    dump(root / 'outcome.json', outcome)
    log.finalize()
    return outcome



# ---- review-evidence scenarios (AC-1/2/3/6/7/12): one private run per history ----

def start_at(args, root, profile_name, gate, configure=None):
    """Private run driven by the built programs up to the first visit of review state `gate`."""
    captured = {}

    def exercise(state, ctx):
        if state == gate and not captured:
            captured.update(ctx)
        return False

    root.mkdir(parents=True, exist_ok=True)
    SW.run(make_args(args, args.engine, args.provider), PROFILES / f'{profile_name}.json', root,
           exercise=exercise, stop_at=gate, configure=configure,
           command_log=SW.CommandLog(root / 'commands'))
    require(captured, f'never reached {gate}')
    return captured


def revision_of(ctx, subject):
    return json.loads((ctx['artifacts'] / subject).read_text())['revision']


def policies(ctx, gate):
    return {p['id']: p for p in ctx['profile']['review_policies'][gate]}


def judge(ctx, gate, subject, axes, who, assessment, *, locators=None, kind='agent',
          result='pass', findings=''):
    """Scripted external judgments through public intake; returns {axis: stored review-evidence ID}."""
    revision, ids = revision_of(ctx, subject), {}
    for axis in axes:
        policy = policies(ctx, gate)[axis]
        author = {'name': who, 'kind': kind}
        evidence = [{'locator': value} for value in (locators or {}).get(axis, [subject + '#/revision'])]
        item = {'axis': axis, 'result': result, 'findings': findings,
                'grounds': {'reason': 'Scripted current-subject inspection.', 'evidence': evidence}}
        packet = {'show': ctx['show'](), 'author': author, 'request': {'text': f'Review {subject}.'},
                  'assignment': {'gate': gate, 'stage': policy.get('review_stage', 'aggregate'),
                                 'subject': subject, 'subject_revision': revision,
                                 'group': policy['group'], 'author': author},
                  'original': {'text': json.dumps({'assessment_id': assessment, 'judgments': [item]})}}
        intake = ctx['invoke']([ctx['provider'], 'review-candidates'], packet)
        require(intake.get('records'), intake)
        for candidate in intake['records']:
            ids[axis] = append(ctx, 'review-evidence', candidate['data'])['result']['context']['id']
    return ids


def carry(ctx, record_id, subject, revision):
    declaration = {'origin': {'kind': 'context-record', 'id': record_id},
                   'target': {'subject': subject, 'revision': revision, 'checkpoint': None},
                   'attesting_driver': {'name': 'scripted-driver', 'kind': 'agent'},
                   'reason': 'Scripted driver attests the unchanged part of the subject still applies.'}
    return append(ctx, 'evidence-applicability', declaration)['result']['context']['id']


def ledger(ctx, gate, subject, findings=None):
    data = {'schema_version': '1', 'gate': gate, 'subject': subject,
            'subject_revision': revision_of(ctx, subject),
            'author': {'name': 'scripted-driver', 'kind': 'agent'}, 'findings': findings or []}
    return append(ctx, 'finding-ledger', data)['result']['context']['id']


def commission(ctx, slot):
    return ctx['invoke']([ctx['provider'], 'commission', '--slot', slot], ctx['show']())


def gate_answer(ctx, event):
    """Checked event on the recorded history. Refusals must leave state, context and history unchanged."""
    before = snapshot(ctx)
    response = ctx['call']('event', ctx['run_id'], event)
    if response['status'] == 'rejected':
        after = snapshot(ctx)
        # A refused checked event is itself recorded in history; state and context do not move.
        unchanged({k: before[k] for k in ('state', 'context')}, {k: after[k] for k in ('state', 'context')}, event)
    return response


def gate_causes(response):
    """(axis, category) pairs named by a refused review gate, plus the raw text."""
    pairs = set()
    for entry in response.get('details', {}).get('diagnostics', []):
        for diagnostic in entry.get('diagnostics', []):
            pairs.add((entry['axis'], diagnostic['category']))
    return pairs


def gate_diagnostics(response):
    """Every diagnostic object a refused review gate reports, with its axis."""
    return [(entry['axis'], diagnostic) for entry in response.get('details', {}).get('diagnostics', [])
            for diagnostic in entry.get('diagnostics', [])]


def compare(ctx, challenge_slot, *, expect, label):
    """One AC-3 table row: commission and the checked event, same history, same answer, same cause."""
    packet = commission(ctx, challenge_slot)
    response = gate_answer(ctx, 'approved')
    row = {'scenario': label, 'commission_missing_inputs': packet.get('missing_inputs'),
           'gate_status': response['status'], 'gate_causes': sorted(gate_causes(response)),
           'parent_review': packet.get('parent_review'), 'gate_response': response}
    if expect is None:
        require(response['status'] == 'completed' and not packet['missing_inputs'], row)
    else:
        category, axis = expect
        require(response['status'] == 'rejected', row)
        require((axis, category) in gate_causes(response), row)
        text = json.dumps(packet['missing_inputs'])
        require(f'({category})' in text and f'axis `{axis}`' in text, row)
        # A refusal never implies a fresh review is always the answer.
        require('is missing current parent aggregate claim' not in text, row)
    return row


def trim(row):
    """Keep a compact receipt row (the full gate response is large)."""
    return {k: v for k, v in row.items() if k != 'gate_response'}


PLAN = 'plan.json'
PLAN_GATE, PLAN_CHALLENGE = 'plan-review', 'plan-adversarial-review'


def revise_plan(ctx, suffix='-2'):
    ctx['event']('revise')
    path = ctx['artifacts'] / PLAN
    plan = json.loads(path.read_text())
    plan['revision'] += suffix
    path.write_text(json.dumps(plan))
    ctx['event']('plan-ready')
    return plan['revision']


def plan_prefix(ctx, reviewers=(('reviewer-a', 'A1'),)):
    """Fresh judgments at the first plan revision, then a revision-only edit to the next.
    Returns ({author: {axis: record ID}}, new revision)."""
    judged = {who: judge(ctx, PLAN_GATE, PLAN, list(policies(ctx, PLAN_GATE)), who, assessment)
              for who, assessment in reviewers}
    return judged, revise_plan(ctx)


def review_cases(args, root):
    """AC-1/2/3 table rows and the review-group rows of AC-6/AC-7 on the plan gate."""
    rows = []

    def start(name, profile='minimal'):
        return start_at(args, root / name, profile, PLAN_GATE)

    # AC-1: the complete ordinary aggregate, carried to the unchanged revision.
    ctx = start('carried-eligible')
    judged, revision = plan_prefix(ctx)
    first = judged['reviewer-a']
    for record in first.values():
        carry(ctx, record, PLAN, revision)
    ledger(ctx, PLAN_GATE, PLAN)
    row = compare(ctx, PLAN_CHALLENGE, expect=None, label='AC-1 carried complete aggregate')
    labels = [p['label'] for p in row['parent_review']['parts']]
    require(labels.count('original judgment') == len(first) and 'fresh review' not in labels and
            labels.count('current applicability attestation') == len(first), labels)
    original = next(p for p in row['parent_review']['parts'] if p['record_id'] == first['task-sized'])
    require(original['author'] == {'name': 'reviewer-a', 'kind': 'agent'} and original['assessment_id'] == 'A1'
            and original['group'] == 'plan-review.aggregate' and original['originally_for_revision'] != revision,
            original)
    rows.append(trim(row))

    # AC-2 (one refusing history per cause) and the AC-6(d) uncovered axis.
    ctx = start('missing-member')
    judged, revision = plan_prefix(ctx)
    axes = list(judged['reviewer-a'])
    for axis in axes[:-1]:
        carry(ctx, judged['reviewer-a'][axis], PLAN, revision)
    ledger(ctx, PLAN_GATE, PLAN)
    rows.append(trim(compare(ctx, PLAN_CHALLENGE, expect=('missing', axes[-1]),
                             label='AC-2/AC-6(d) missing group member')))

    ctx = start('assessment-mismatch')
    judged, revision = plan_prefix(ctx)
    axes = list(judged['reviewer-a'])
    for axis in axes[:5]:
        carry(ctx, judged['reviewer-a'][axis], PLAN, revision)
    judge(ctx, PLAN_GATE, PLAN, [axes[5]], 'reviewer-a', 'A2')
    judge(ctx, PLAN_GATE, PLAN, [axes[6]], 'reviewer-a', 'A3')
    ledger(ctx, PLAN_GATE, PLAN)
    row = compare(ctx, PLAN_CHALLENGE, expect=('assessment_inconsistent', axes[5]),
                  label='AC-2/AC-7 assessment mismatch')
    inconsistent = next(d for a, d in gate_diagnostics(row['gate_response'])
                        if d['category'] == 'assessment_inconsistent')
    require(inconsistent['author'] == {'name': 'reviewer-a', 'kind': 'agent'} and
            inconsistent['assessment_ids'] == ['A2', 'A3'] and inconsistent['group'] == 'plan-review.aggregate',
            inconsistent)
    rows.append(trim(row))

    ctx = start('author-split')
    axes = list(policies(ctx, PLAN_GATE))
    judge(ctx, PLAN_GATE, PLAN, axes[:4], 'reviewer-a', 'A1')
    judge(ctx, PLAN_GATE, PLAN, axes[4:], 'reviewer-b', 'B1')
    ledger(ctx, PLAN_GATE, PLAN)
    row = compare(ctx, PLAN_CHALLENGE, expect=('independence', axes[0]),
                  label='AC-2/AC-6/AC-7 author mismatch: fresh-only split across authors')
    # AC-7: the refusal names the rule, author, assessment and axis, and why the pass was excluded.
    excluded = [e for a, d in gate_diagnostics(row['gate_response']) if d['category'] == 'independence'
                for e in d.get('excluded', [])]
    require(len(excluded) == len(axes), excluded)
    require(all(e['rule'] == 'review-group-complete' and e['author']['name'] in ('reviewer-a', 'reviewer-b')
                and e['assessment_id'] in ('A1', 'B1') and e['axis'] in axes and 'judged every axis' in e['reason']
                for e in excluded), excluded)
    rows.append(trim(row))

    ctx = start('stale-target')
    judged, revision = plan_prefix(ctx)
    axes = list(judged['reviewer-a'])
    for record in judged['reviewer-a'].values():
        carry(ctx, record, PLAN, revision)
    revise_plan(ctx, '-3')  # the carry still names the previous revision
    ledger(ctx, PLAN_GATE, PLAN)
    rows.append(trim(compare(ctx, PLAN_CHALLENGE, expect=('unverified', axes[0]),
                             label='AC-2 wrong revision target (carry names a superseded revision)')))

    ctx = start('withdrawn-origin')
    judged, revision = plan_prefix(ctx)
    axes = list(judged['reviewer-a'])
    for record in judged['reviewer-a'].values():
        carry(ctx, record, PLAN, revision)
    append(ctx, 'record-withdrawal', {'attesting_driver': {'name': 'scripted-driver', 'kind': 'agent'},
                                      'reason': 'Scripted withdrawal of one carried origin.',
                                      'record_ids': [judged['reviewer-a'][axes[0]]]})
    ledger(ctx, PLAN_GATE, PLAN)
    rows.append(trim(compare(ctx, PLAN_CHALLENGE, expect=('missing', axes[0]),
                             label='AC-2 invalid source reference (withdrawn origin)')))

    ctx = start('unresolved-finding')
    axes = list(policies(ctx, PLAN_GATE))
    judge(ctx, PLAN_GATE, PLAN, axes, 'reviewer-a', 'A1')
    failing = judge(ctx, PLAN_GATE, PLAN, [axes[0]], 'reviewer-b', 'B1', result='fail',
                    findings='A scripted material defect.')
    task = json.loads((ctx['artifacts'] / PLAN).read_text())['tasks'][0]['id']
    ledger(ctx, PLAN_GATE, PLAN, [{'id': 'F-open', 'source': {'kind': 'context-record', 'id': failing[axes[0]]},
                                   'policy_id': axes[0], 'disposition': 'accepted', 'reason': 'Scripted: valid.',
                                   'owner_phase': 'plan', 'task_ids': [task], 'review_axes': [axes[0]],
                                   'status': 'unresolved'}])
    packet = commission(ctx, PLAN_CHALLENGE)
    response = gate_answer(ctx, 'approved')
    text = json.dumps(packet['missing_inputs'])
    require(response['status'] == 'rejected' and 'F-open' in json.dumps(response) and
            response['details']['status'] == 'accepted_unresolved', response)
    require('(unresolved_finding)' in text and 'F-open' in text, text)
    rows.append({'scenario': 'AC-2 unresolved accepted finding', 'commission_missing_inputs': packet['missing_inputs'],
                 'gate_status': response['status'], 'gate_code': response['code']})

    # AC-3: an unchanged challenge carried after ordinary clearance (eligible) and a raw re-append (not fresh).
    for variant in ('challenge-carried', 'challenge-reappended'):
        ctx = start(variant)
        first = judge(ctx, PLAN_GATE, PLAN, list(policies(ctx, PLAN_GATE)), 'reviewer-a', 'A1')
        ledger(ctx, PLAN_GATE, PLAN)
        ctx['event']('approved')
        challenge_axes = list(policies(ctx, PLAN_CHALLENGE))
        early = judge(ctx, PLAN_CHALLENGE, PLAN, challenge_axes, 'challenger-a', 'C1')
        revision = revise_plan(ctx)
        for record in first.values():
            carry(ctx, record, PLAN, revision)
        ledger(ctx, PLAN_GATE, PLAN)
        ctx['event']('approved')
        if variant == 'challenge-carried':
            for record in early.values():
                carry(ctx, record, PLAN, revision)
        else:
            judge(ctx, PLAN_CHALLENGE, PLAN, challenge_axes, 'challenger-a', 'C1')
        ledger(ctx, PLAN_CHALLENGE, PLAN)
        packet = commission(ctx, PLAN_CHALLENGE)
        response = gate_answer(ctx, 'approved')
        standing = packet['challenge_standing']
        if variant == 'challenge-carried':
            counted = [s for s in standing if s['standing'] == 'counts']
            require(response['status'] == 'completed' and len(counted) == len(challenge_axes) and
                    all(s['kind'] == 'evidence-applicability' for s in counted) and
                    not packet['missing_inputs'], (standing, response['status']))
        else:
            require(response['status'] == 'rejected' and 'challenge-before-ordinary-clearance' in response['message']
                    and all(s['standing'] == 'early' for s in standing) and
                    all('challenge-before-ordinary-clearance' in s['reason'] for s in standing),
                    (standing, response['message']))
        rows.append({'scenario': 'AC-3 ' + variant, 'gate_status': response['status'],
                     'standing': [(s['axis'], s['kind'], s['standing']) for s in standing]})

    # AC-6(c): a floor-2 axis with one distinct author is refused, naming axis, floor and count.
    ctx = start('floor-two', 'standard')
    judged, revision = plan_prefix(ctx, (('reviewer-a', 'A1'), ('reviewer-b', 'B1')))
    axes = list(judged['reviewer-a'])
    for axis in axes:
        carry(ctx, judged['reviewer-a'][axis], PLAN, revision)
    for axis in axes[:-1]:
        carry(ctx, judged['reviewer-b'][axis], PLAN, revision)
    ledger(ctx, PLAN_GATE, PLAN)
    row = compare(ctx, PLAN_CHALLENGE, expect=('independence', axes[-1]),
                  label='AC-6(c)/AC-7 floor-2 axis with one distinct author')
    diagnostic = next(d for a, d in gate_diagnostics(row['gate_response'])
                      if a == axes[-1] and d['category'] == 'independence')
    require(diagnostic['required'] == 2 and diagnostic['distinct_present'] == 1, diagnostic)
    require('1 of 2 required distinct authors count' in json.dumps(row['commission_missing_inputs']), row)
    rows.append(trim(row))
    return rows


# ---- intent revisions with a changed criterion (AC-6 a/b, AC-12) ----

INTENT = 'intent.json'
INTENT_GATE, INTENT_CHALLENGE = 'intent-review', 'intent-adversarial-review'
CITES_ACCEPTANCE = ('acceptance-granularity', 'acceptance-sufficient')


def intent_locators(axes):
    return {axis: ['intent.json#/acceptance'] if axis in CITES_ACCEPTANCE else ['intent.json#/revision']
            for axis in axes}


def intent_revise(ctx, change_criterion):
    """Revise to the next intent revision, optionally changing one criterion's text. The exact prior
    bytes travel in an owner-steering baseline, as the provider requires for any later revision."""
    import hashlib
    path = ctx['artifacts'] / INTENT
    prior = path.read_text()
    intent = json.loads(prior)
    ctx['event']('revise')
    if change_criterion:
        intent['acceptance'][1]['statement'] += ' It must also hold for every caller.'
    intent['revision'] += '-2'
    path.write_text(json.dumps(intent))
    append(ctx, 'user-steering', {
        'target': {'kind': 'all'},
        'instruction': 'SCRIPTED DUMMY BACKENDS: a simulated owner scopes the next intent revision.',
        'intent_baseline': {'revision': json.loads(prior)['revision'], 'locator': 'intent.json (retained prior bytes)',
                            'sha256': 'sha256:' + hashlib.sha256(prior.encode()).hexdigest(), 'json_bytes': prior}})
    ctx['event']('intent-ready')
    return intent['revision']


def intent_sufficient(ctx, who='reviewer-c'):
    """The separate cold-reconstruction group, freshly judged, with its scripted owner-authorized comparison."""
    judge(ctx, INTENT_GATE, INTENT, ['intent-sufficient'], who, 'S1')
    intent = json.loads((ctx['artifacts'] / INTENT).read_text())
    cold = [r for r in ctx['show']()['result']['context'] if r['kind'] == 'review-evidence'
            and r['data'].get('gate') == INTENT_GATE and r['data'].get('policy_id') == 'intent-sufficient'
            and r['data'].get('subject_revision') == intent['revision']]
    SW.append_scripted_intent_comparison(ctx['call'], ctx['show'], ctx['run_id'], intent, cold)


def intent_cases(args, root):
    rows = []

    def prepared(name, change):
        ctx = start_at(args, root / name, 'minimal', INTENT_GATE)
        policy = policies(ctx, INTENT_GATE)
        aggregate = [a for a, p in policy.items() if p['group'] == 'intent-review.aggregate']
        require(len(aggregate) == 8, aggregate)
        first = judge(ctx, INTENT_GATE, INTENT, aggregate, 'reviewer-a', 'A1', locators=intent_locators(aggregate))
        stale = [a for a in aggregate if a in CITES_ACCEPTANCE]
        return ctx, aggregate, first, [a for a in aggregate if a not in stale], stale, intent_revise(ctx, change)

    def finish(ctx, label, expect=None):
        intent_sufficient(ctx)
        ledger(ctx, INTENT_GATE, INTENT)
        return compare(ctx, INTENT_CHALLENGE, expect=expect, label=label)

    # (a) floor 1: six axes carried from one author, two freshly passed by another.
    ctx, aggregate, first, kept, stale, revision = prepared('six-carried-two-fresh', True)
    for axis in kept:
        carry(ctx, first[axis], INTENT, revision)
    judge(ctx, INTENT_GATE, INTENT, stale, 'reviewer-b', 'B1', locators=intent_locators(stale))
    row = finish(ctx, 'AC-6(a) six carried from one author, two fresh from another, criterion text changed')
    parts = row['parent_review']['parts']
    require(len([p for p in parts if p['label'] == 'original judgment']) == 6 and
            len([p for p in parts if p['label'] == 'fresh review' and p['gate'] == INTENT_GATE]) >= 2, parts)
    rows.append(trim(row))
    # (b) floor 1: three carried, five fresh from the same reviewer name under a new assessment.
    ctx, aggregate, first, kept, stale, revision = prepared('three-carried-five-fresh', True)
    for axis in kept[:3]:
        carry(ctx, first[axis], INTENT, revision)
    fresh = stale + kept[3:]
    judge(ctx, INTENT_GATE, INTENT, fresh, 'reviewer-a', 'A2', locators=intent_locators(fresh))
    rows.append(trim(finish(ctx, 'AC-6(b) three carried, five fresh from the same reviewer under a new assessment')))
    # Six carried plus two fresh axes from two different authors.
    ctx, aggregate, first, kept, stale, revision = prepared('six-carried-two-authors', True)
    for axis in kept:
        carry(ctx, first[axis], INTENT, revision)
    judge(ctx, INTENT_GATE, INTENT, [stale[0]], 'reviewer-b', 'B1', locators=intent_locators(stale))
    judge(ctx, INTENT_GATE, INTENT, [stale[1]], 'reviewer-d', 'D1', locators=intent_locators(stale))
    rows.append(trim(finish(ctx, 'AC-6 six carried, two fresh axes from two different authors')))
    # AC-12: a revision-only edit keeps every carry ...
    ctx, aggregate, first, kept, stale, revision = prepared('revision-only', False)
    for axis in aggregate:
        carry(ctx, first[axis], INTENT, revision)
    rows.append(trim(finish(ctx, 'AC-12 revision-only edit keeps carried evidence for unchanged criteria')))
    # ... and changed criterion text makes carried evidence about it stale, naming the criterion.
    # Write-time admission already refuses the stale carry itself, leaving everything unchanged.
    ctx, aggregate, first, kept, stale, revision = prepared('criterion-changed-append', True)
    before = snapshot(ctx)
    declaration = {'origin': {'kind': 'context-record', 'id': first[stale[0]]},
                   'target': {'subject': INTENT, 'revision': revision, 'checkpoint': None},
                   'attesting_driver': {'name': 'scripted-driver', 'kind': 'agent'}, 'reason': 'Scripted carry.'}
    response = ctx['call']('append', ctx['run_id'], 'evidence-applicability', json.dumps(declaration))
    require(response['status'] == 'rejected' and response['details']['rule'] == 'review-applicability-origin-target'
            and 'stale carry' in response['details']['message'] and 'AC-2' in response['details']['message'], response)
    unchanged(before, snapshot(ctx), 'stale carry append')
    rows.append({'scenario': 'AC-12 stale carry refused at append', 'message': response['details']['message']})
    # A carry that was valid when stored becomes stale if the criterion is edited afterwards:
    # packet construction and the checked event refuse it identically.
    ctx, aggregate, first, kept, stale, revision = prepared('criterion-changed-after-carry', False)
    for axis in aggregate:
        carry(ctx, first[axis], INTENT, revision)
    path = ctx['artifacts'] / INTENT
    edited = json.loads(path.read_text())
    edited['acceptance'][1]['statement'] += ' It must also hold for every caller.'
    path.write_text(json.dumps(edited))
    row = finish(ctx, 'AC-12 criterion text changed after the carry: stale for packet and gate',
                 expect=('unverified', stale[0]))
    require({axis for axis, _ in gate_causes(row['gate_response'])} == set(stale), row['gate_causes'])
    gate_text, commission_text = json.dumps(row['gate_response']), json.dumps(row['commission_missing_inputs'])
    require('stale carry' in gate_text and 'AC-2' in gate_text and 'stale carry' in commission_text and
            'AC-2' in commission_text, row)
    rows.append(trim(row))
    return rows


# ---- locator guidance, nearest-pointer diagnostics, invocation-directory independence (AC-8/9/10) ----

def expected_arrays(value, pointer='', depth=0, out=None):
    """Independent walk of a document's array lengths (same bound the packet documents)."""
    out = {} if out is None else out
    if depth > 4:
        return out
    if isinstance(value, list):
        out[pointer] = len(value)
        for index, item in enumerate(value):
            expected_arrays(item, f'{pointer}/{index}', depth + 1, out)
    elif isinstance(value, dict):
        for key, item in value.items():
            expected_arrays(item, f"{pointer}/{key.replace('~', '~0').replace('/', '~1')}", depth + 1, out)
    return out


def intake(ctx, locator, *, provider=None, cwd=None):
    shown = ctx['show']()
    policy = policies(ctx, INTENT_GATE)['solution-agnostic']
    author = {'name': 'reviewer-a', 'kind': 'agent'}
    item = {'axis': 'solution-agnostic', 'result': 'pass', 'findings': '',
            'grounds': {'reason': 'Scripted inspection.', 'evidence': [{'locator': locator}]}}
    packet = {'show': shown, 'author': author, 'request': {'text': 'Review.'},
              'assignment': {'gate': INTENT_GATE, 'stage': 'aggregate', 'subject': INTENT,
                             'subject_revision': revision_of(ctx, INTENT), 'group': policy['group'],
                             'author': author},
              'original': {'text': json.dumps({'assessment_id': 'X1', 'judgments': [item]})}}
    kwargs = {'cwd': cwd} if cwd else {}
    return ctx['invoke']([provider or ctx['provider'], 'review-candidates'], packet, **kwargs)


def declare_scope(profile, repository, artifacts):
    path = artifacts / INTENT
    intent = json.loads(path.read_text())
    intent['source_scopes'] = {'primary': {'root': str(repository)}}
    path.write_text(json.dumps(intent))


def locator_cases(args, root):
    out = {}
    # AC-8: the packet states the forms, the roots and the subject's real structure.
    ctx = start_at(args, root / 'guidance', 'minimal', INTENT_GATE)
    guidance = commission(ctx, INTENT_GATE)['locator_guidance']
    intent = json.loads((ctx['artifacts'] / INTENT).read_text())
    source = next(s for s in guidance['json_sources'] if s['locator'] == INTENT)
    require(sorted(source['top_level_keys']) == sorted(intent), (source['top_level_keys'], list(intent)))
    require(source['array_lengths'] == expected_arrays(intent), (source['array_lengths'], expected_arrays(intent)))
    require(len(guidance['accepted_forms']) == 4 and 'never depends' in guidance['resolution']['rule'], guidance)
    out['AC-8'] = {'top_level_keys': source['top_level_keys'], 'array_lengths': source['array_lengths']}
    # AC-9: a pointer that does not resolve is refused and the closest real one is named.
    count = len(intent['acceptance'])
    refused = {}
    for locator, wanted in [(f'intent.json#/acceptance/{count}', f'closest existing pointer: `intent.json#/acceptance/{count - 1}`'),
                            ('intent.json#/outside_obligations/0',
                             'closest existing pointer: `intent.json#/operating_context/outside_obligations/0`'),
                            ('intent.json#/nothing_like_it/9', 'no close existing pointer was found')]:
        result = intake(ctx, locator)
        require(not result['records'] and wanted in json.dumps(result['items']), (locator, result))
        refused[locator] = wanted
    require(intake(ctx, 'intent.json#/revision')['records'], 'a real pointer must still be accepted')
    out['AC-9'] = refused
    # AC-10: with a declared source root the same intake accepts and refuses the same locators from any directory.
    other = Path(tempfile.mkdtemp(prefix='elsewhere-', dir=root))
    ctx = start_at(args, root / 'scoped', 'minimal', INTENT_GATE, configure=declare_scope)
    locators = ['helper.py#L1-L1', 'missing.py#L1-L1', 'intent.json#/revision', 'intent.json#/nope']

    def verdicts(context, provider=None):
        table = {}
        for cwd in (context['repository'], other):
            table[str(cwd)] = {loc: ('accepted' if intake(context, loc, provider=provider, cwd=cwd)['records']
                                     else 'refused') for loc in locators}
        return table
    built = verdicts(ctx)
    first, second = list(built.values())
    require(first == second and first['helper.py#L1-L1'] == 'accepted' and first['missing.py#L1-L1'] == 'refused' and
            first['intent.json#/nope'] == 'refused', built)
    # Without a declared root and outside a git work tree there is no source root: refused from anywhere.
    ctx = start_at(args, root / 'unscoped', 'minimal', INTENT_GATE)
    now = verdicts(ctx)
    nfirst, nsecond = list(now.values())
    require(nfirst == nsecond and nfirst['helper.py#L1-L1'] == 'refused', now)
    out['AC-10'] = {'built_declared_scope': built, 'built_undeclared': now}
    if args.control_provider:
        # Control: v0.26.0 resolves an undeclared root against the directory it runs from.
        control = verdicts(ctx, provider=args.control_provider)
        cfirst, csecond = list(control.values())
        require(cfirst['helper.py#L1-L1'] == 'accepted' and csecond['helper.py#L1-L1'] == 'refused', control)
        out['AC-10']['control_v0_26_0_undeclared'] = control
    return out

# ---- driver ----

SCENARIOS = {
    'command-carry': lambda a, r: {
        'built': command_carry(a, r / 'built', a.engine, a.provider, False),
        'control': (command_carry(a, r / 'control', a.control_engine, a.control_provider, True)
                    if a.control_engine and a.control_provider else 'not run: no v0.26.0 control programs supplied')},
    'review-cases': review_cases,
    'intent-cases': intent_cases,
    'locator-cases': locator_cases,
}


def self_test():
    """Collection and interface mechanics only; proves no behavior."""
    for ac in [f'AC-{n}' for n in range(1, 13)]:
        require(ac in COVERAGE and all(name in SCENARIOS for name in COVERAGE[ac]), ac)
    print('evidence-carry-journey --self-test: collection mechanics only (no behavior proved)')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--self-test', action='store_true')
    parser.add_argument('--engine')
    parser.add_argument('--provider')
    parser.add_argument('--control-engine')
    parser.add_argument('--control-provider')
    parser.add_argument('--output-root')
    parser.add_argument('--only', action='append', choices=sorted(SCENARIOS))
    args = parser.parse_args()
    if args.self_test:
        return self_test()
    for name in ('engine', 'provider', 'output_root'):
        if not getattr(args, name):
            parser.error(f'--{name.replace("_", "-")} is required')
    out = Path(args.output_root).resolve()
    out.mkdir(parents=True, exist_ok=True)
    results = {}
    for name in (args.only or sorted(SCENARIOS)):
        print(f'== {name}', flush=True)
        results[name] = SCENARIOS[name](args, Path(tempfile.mkdtemp(prefix=name + '-', dir=out)))
    dump(out / 'result.json', {'label': LABEL, 'status': 'passed', 'scenarios': results})
    print('evidence-carry-journey: all scenarios passed')


if __name__ == '__main__':
    main()
