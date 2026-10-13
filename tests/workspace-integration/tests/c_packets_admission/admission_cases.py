"""Admission, correction, citation and checkpoint public-CLI cases."""
import copy
import importlib.util
import json
import os
from pathlib import Path
import shutil
import signal
import sys
import subprocess
import time
from types import SimpleNamespace
from capture_support import Capture, dump, groups, invariant, require, sha, snapshot, checked_allow
HERE = Path(__file__).resolve().parent
LABEL = 'Scripted fixture judgments; no model or human review claim'

def load_module(path, name):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module

class Witness:
    def __init__(self, ctx, log, case):
        self.ctx, self.log, self.case = ctx, log, case
        self.counter = len(list(log.root.parent.glob('witness-*.json')))
        self.edges = []

    def retain(self, edge, data):
        self.counter += 1
        path = self.log.root.parent / f'witness-{self.counter:04d}.json'
        dump(path, {'edge': edge, 'label': LABEL, **data})
        self.edges.append({'edge': edge, 'path': str(path), 'sha256': sha(path.read_bytes())})

    def inspect(self, argv, packet=None, *, refused=None):
        before = snapshot(self.ctx)
        response = self.log.execute(argv, packet, cwd=self.ctx['repository'], raw=True)
        after = snapshot(self.ctx)
        invariant(before, after)
        require(before['history'] == after['history'], 'Read-only preview/commission changed history')
        if refused is None:
            require(response.returncode == 0, response.stderr)
            value = json.loads(response.stdout)
        else:
            require(response.returncode != 0 and refused in response.stdout + response.stderr, response)
            value = {'exit_code': response.returncode, 'stdout': response.stdout, 'stderr': response.stderr}
        self.retain('read-only', {'argv': list(map(str, argv)), 'before': before, 'after': after, 'response': value})
        return value

    def append(self, kind, data, rid=None):
        before = snapshot(self.ctx)
        argv = ['append'] + (['--record-id', rid] if rid else [])
        path = self.log.root.parent / f'input-{len(self.log.rows):06d}.json'
        dump(path, data)
        response = self.ctx['call'](*argv, self.ctx['run_id'], kind, '@' + str(path))
        after = snapshot(self.ctx)
        require(response['status'] == 'completed', response)
        old, new = before['show']['result'], after['show']['result']
        require(new['context'][:-1] == old['context'] and new['context'][-1]['data'] == data and
                new['context'][-1]['kind'] == kind and
                (rid is None or new['context'][-1]['id'] == rid), response)
        require(before['history']['result'] == after['history']['result'][:-1] and
                after['history']['result'][-1]['action']['kind'] == 'context_appended', after)
        require(new['current_state'] == old['current_state'] and new['state_visit'] == old['state_visit'] and
                not new['has_overrides'], after)
        self.retain('accepted-append', {'before': before, 'after': after, 'response': response})
        return response['result']['context']

    def refuse(self, kind, data, rid, rule, needle, *, code='record-refused'):
        before = snapshot(self.ctx)
        path = self.log.root.parent / f'{rid}.candidate.json'
        dump(path, data)
        base = [self.ctx['engine'], '--database', self.ctx['database'], '--config', self.ctx['config'], '--json']
        response = self.log.execute([*base, 'append', '--record-id', rid, self.ctx['run_id'], kind,
                                     '@' + str(path)], cwd=self.ctx['repository'], raw=True)
        value = json.loads(response.stdout)
        after = snapshot(self.ctx)
        invariant(before, after)
        require(before['history'] == after['history'] and response.returncode != 0, value)
        expected_status = 'error' if code == 'provider-invalid-response' else 'rejected'
        require(value['status'] == expected_status and value['code'] == code and needle in json.dumps(value), value)
        if rule is not None:
            require(value['status'] == 'rejected' and value['details']['record_id'] == rid and
                    value['details']['rule'] == rule, value)
        require(all(r['id'] != rid for r in after['show']['result']['context']), value)
        self.retain(rid, {'kind': kind, 'candidate': data, 'before': before, 'after': after, 'response': value})

    def event(self, event, *, deny=None, code=None):
        before = snapshot(self.ctx)
        value = self.ctx['call']('event', self.ctx['run_id'], event)
        after = snapshot(self.ctx)
        if deny:
            invariant(before, after)
            require(value['status'] == 'rejected' and deny in json.dumps(value), value)
            if code is not None:
                require(value['code'] == code, value)
            rows = after['history']['result']
            require(rows[:-1] == before['history']['result'] and
                    rows[-1]['action']['kind'] == 'transition' and
                    rows[-1]['action']['outcome']['outcome'] == 'denied' and
                    rows[-1]['action']['transition']['kind'] == 'checked', rows)
        else:
            checked_allow(before, value, after)
        self.retain('checked-deny' if deny else 'checked-allow', {'before': before, 'after': after, 'response': value})
        return value

    def preview(self, gate, subject, policies, author, *, locators=None, fail_axis=None):
        value = json.loads((self.ctx['artifacts'] / subject).read_bytes())
        require(author != value['author'], 'Reviewer is subject author')
        original = {'review_contract_version': 3, 'review_stage': policies[0]['review_stage'], 'author': author,
                    'judgments': [{'axis': p['id'], 'result': 'fail' if p['id'] == fail_axis else 'pass',
                    'findings': 'Exact toy finding.\nPreserve this wording.' if p['id'] == fail_axis else '',
                    'grounds': {'reason': LABEL, 'evidence': [{'locator': l} for l in
                               (locators or [subject + '#/revision'])]}} for p in policies]}
        require('assessment_id' not in original and all('assessment_id' not in j for j in original['judgments']), original)
        assignment = {'gate': gate, 'stage': policies[0]['review_stage'], 'group': policies[0]['group'],
                      'author': author, 'subject': subject, 'subject_revision': value['revision']}
        result = self.inspect([self.ctx['provider'], 'review-candidates'], {'show': self.ctx['show'](),
                   'assignment': assignment, 'request': {'text': LABEL}, 'original': {'text': json.dumps(original)}})
        return result

    def review(self, gate, subject, policies, *, fail_axis=None, locators=None):
        records = []
        for _, batch in groups(policies):
            for index in range(max(p['required_authors'] for p in batch)):
                author = {'name': f'new-toy-{gate}-{index}', 'kind': 'agent'}
                preview = self.preview(gate, subject, batch, author, locators=locators, fail_axis=fail_axis if batch[0]['review_stage'] == 'aggregate' and index == 0 else None)
                require(len(preview['records']) == len(batch) and all(not i['diagnostics'] for i in preview['items']), preview)
                require(len({r['data']['assessment_id'] for r in preview['records']}) == 1 and
                        all(r['data']['assessment_id'].startswith('assessment:sha256:') for r in preview['records']), preview)
                for row in preview['records']:
                    records.append(self.append(row['kind'], row['data']))
        return records

    def ledger(self, gate, subject, findings=None):
        return self.append('finding-ledger', {'schema_version': '1', 'gate': gate, 'subject': subject,
            'subject_revision': json.loads((self.ctx['artifacts'] / subject).read_bytes())['revision'],
            'author': {'name': 'new-toy-driver', 'kind': 'agent'}, 'findings': findings or []})

    def commission(self, gate, stage='aggregate'):
        return self.inspect([self.ctx['provider'], 'commission', '--slot', gate, '--stage', stage], self.ctx['show']())

    def carry(self, source, subject):
        return {'origin': {'kind': 'context-record', 'id': source['id']},
                'target': {'subject': subject, 'revision': json.loads((self.ctx['artifacts'] / subject).read_bytes())['revision'],
                           'checkpoint': None}, 'attesting_driver': {'name': 'new-toy-driver', 'kind': 'agent'},
                'reason': LABEL + '; explicit unchanged bounded judgment scope'}


def finding(source, *, accepted=False):
    return {'id': 'F-new-toy', 'source': {'kind': 'context-record', 'id': source['id']},
            'policy_id': source['data']['policy_id'], 'summary': 'Separate driver summary; not source wording.',
            'disposition': 'accepted' if accepted else 'rejected', 'reason': LABEL,
            'owner_phase': 'intent' if accepted else None, 'task_ids': [],
            'review_axes': [source['data']['policy_id']] if accepted else [],
            'status': 'unresolved' if accepted else 'recorded'}


def cas_control(w):
    ctx = w.ctx
    root = w.log.root.parent
    ready, release = root / 'cas-ready', root / 'cas-release'
    payload = root / 'cas-input.json'
    payload.write_text(json.dumps({'fixture': 'paused append'}))
    argv = [str(ctx['engine']), '--database', str(ctx['database']), '--config', str(ctx['config']),
            '--json', 'append', '--record-id', 'cas-stale', ctx['run_id'], 'fixture-note', '@' + str(payload)]
    before = snapshot(ctx)
    child = subprocess.Popen(argv, cwd=ctx['repository'], env=w.log.environment,
                             stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
    try:
        deadline = time.monotonic() + 30
        while not ready.exists():
            require(child.poll() is None and time.monotonic() < deadline, 'Admission barrier not reached')
            time.sleep(0.01)
        stored = w.append('fixture-note', {'fixture': 'concurrent append'}, 'cas-concurrent')
        after_concurrent = snapshot(ctx)
    finally:
        try:
            release.write_text('release')
            stdout, stderr = child.communicate(timeout=60)
        finally:
            if child.poll() is None:
                os.killpg(child.pid, signal.SIGKILL)
                child.communicate()
    result = json.loads(stdout)
    after = snapshot(ctx)
    require(child.returncode != 0 and result['code'] == 'append-sequence-changed', result)
    invariant(after_concurrent, after)
    require(after_concurrent['history'] == after['history'], 'Stale admission wrote lineage')
    require(all(r['id'] != 'cas-stale' for r in after['show']['result']['context']), result)
    w.retain('public-admission-cas', {'argv': argv, 'pid':child.pid, 'exit_code':child.returncode,
             'stdout':stdout.decode(), 'stderr':stderr.decode(), 'before':before,
             'concurrent':stored, 'after':after})
    w.append('fixture-note', {'fixture': 'paused append'}, 'cas-stale')


def metadata_case(w):
    ctx = w.ctx
    cas_control(w)
    gate, subject = 'design-review', 'design.json'
    policies = ctx['profile']['review_policies'][gate]
    target = max((batch for key, batch in groups(policies) if key[0] == 'aggregate'), key=len)
    require(len(target) > 1, 'Need real canonical multi-axis group')
    preview = w.preview(gate, subject, target, {'name': 'new-toy-design-review-0', 'kind': 'agent'})
    good = preview['records'][0]['data']
    # The original six cases already cover exact-byte digest mismatch and revision mismatch.
    raw = (ctx['artifacts'] / 'intent.json').read_text()
    baseline = {'revision': json.loads(raw)['revision'], 'locator': 'intent.json',
                'sha256': 'sha256:' + sha(raw.encode()), 'json_bytes': raw}
    steering = {'target': {'kind': 'all'}, 'instruction': LABEL, 'intent_baseline': baseline}
    for tag, digest in [('short', 'sha256:bad'), ('uppercase', 'sha256:' + 'A' * 64)]:
        bad = copy.deepcopy(steering); bad['intent_baseline']['sha256'] = digest
        w.refuse('user-steering', bad, 'digest-format-' + tag, 'steering-shape-references', 'retained intent baseline locator, revision, or exact-byte digest is invalid')
        w.append('user-steering', steering)
    outside = ctx['work'] / 'outside.txt'; outside.write_text('NEW real toy source outside both allowed roots\n')
    link = ctx['artifacts'] / 'escaped-link.txt'; link.symlink_to(outside)
    w.retain('outside-file-stimulus', {'path': str(outside), 'sha256': sha(outside.read_bytes()),
                                     'bytes': outside.read_text(), 'symlink': str(link), 'target': os.readlink(link)})
    for tag, locator, needle in [('escape', '../outside.txt#L1', 'beneath a source root'),
                                  ('symlink', 'escaped-link.txt#L1', 'not found beneath'),
                                  ('missing', 'not-present.json#/x', 'not found beneath'),
                                  ('pointer', 'design.json#/not-present', 'does not resolve')]:
        bad = copy.deepcopy(good); bad['grounds']['evidence'] = [{'locator': locator}]
        preview = w.preview(gate, subject, [target[0]], {'name': 'new-toy-design-review-0', 'kind': 'agent'}, locators=[locator])
        require(not preview['records'] and needle in json.dumps(preview['items']), preview)
        w.refuse('review-evidence', bad, 'file-' + tag, 'review-shape-policy', needle)
        w.append('review-evidence', good)
    # Valid when written historical source becomes invalid specifically at current-target author eligibility.
    path = ctx['artifacts'] / subject
    original = path.read_bytes(); current = json.loads(original)
    stored = w.append('review-evidence', good)
    current['author'] = good['author']; path.write_text(json.dumps(current))
    try:
        w.refuse('evidence-applicability', w.carry(stored, subject), 'origin-current-self',
                 'review-applicability-current-target', 'self')
    finally:
        path.write_bytes(original)
    w.append('evidence-applicability', w.carry(stored, subject))
    # Missing engine origin, before any provider admission, gets the engine's typed error.
    bad = w.carry(stored, subject); bad['origin']['id'] = 'absent-engine-origin'
    w.refuse('evidence-applicability', bad, 'engine-origin-missing', None, 'not found', code='evidence-applicability-refused')
    w.append('evidence-applicability', w.carry(stored, subject))
    marker = ctx['_transport_marker']
    marker.write_text('NEW fixture: admit answers valid JSON with incompatible shape only.\n')
    try:
        w.refuse('user-steering', steering, 'required-validator-incompatible', None,
                 'admit response must be a JSON object', code='provider-invalid-response')
    finally:
        marker.unlink()
    # Independent real executable control, using only the actual public full show.
    shown = ctx['show']()['result']
    real_admit = w.inspect([ctx['_real_provider']], {'operation': 'admit',
        'workflow': shown['workflow_graph'], 'initial_input': shown['initial_input'],
        'context': shown['context'], 'current_state': shown['current_state'], 'state_visit': shown['state_visit'],
        'record': {'id': 'required-validator-incompatible', 'kind': 'user-steering', 'data': steering}})
    require(real_admit == {} and not marker.exists(), 'Real provider must admit repaired conforming candidate')
    repaired = w.append('user-steering', steering, 'required-validator-incompatible')
    w.retain('validator-real-provider-repair', {'real_provider': str(ctx['_real_provider']),
             'marker_present': marker.exists(), 'admission_response': real_admit, 'stored_record': repaired})
    rows = w.review(gate, subject, policies, fail_axis=target[0]['id'])
    source = next(r for r in rows if r['data']['review_stage'] == 'aggregate' and r['data']['policy_id'] == target[0]['id'])
    f = finding(source)
    require('statement' not in f, 'Driver must not copy reviewer source text')
    ledger = w.ledger(gate, subject, [f])
    require(ledger['data']['findings'][0]['summary'] == f['summary'] and 'statement' not in ledger['data']['findings'][0], ledger)
    # Fresh full coherent returns satisfy both floors; exact old source/summary remain.
    w.review(gate, subject, policies)
    # Gate projects the source statement; driver summary stays separate in the accepted snapshot.
    w.event('approved')
    packet = w.commission('plan-draft')
    actual = {r['id']: r for r in ctx['show']()['result']['context']}
    require(actual[ledger['id']] == ledger and actual[source['id']] == source, 'Ledger/source lost on progression')
    w.retain('ac3-assessment-free-ledger-forward', {'ledger': ledger, 'source': source, 'next_packet': packet})


def correction_case(w, *, scope=False):
    ctx = w.ctx
    path = ctx['artifacts'] / 'intent.json'
    prior_bytes = path.read_text(); prior = json.loads(prior_bytes)
    policies = ctx['profile']['review_policies']['intent-review']
    affected = next(p['id'] for p in policies if p['review_stage'] == 'aggregate' and p['id'] == 'solution-agnostic')
    rows = w.review('intent-review', 'intent.json', policies, fail_axis=affected)
    source = next(r for r in rows if r['data']['policy_id'] == affected and r['data']['review_stage'] == 'aggregate')
    f = finding(source, accepted=True)
    ledger = w.ledger('intent-review', 'intent.json', [f])
    revised = copy.deepcopy(prior); revised['revision'] += '-new-correction' if not scope else '-new-scope'
    revised['outcome'] += ' Toy clarification of accepted finding.' if not scope else ' NEW OUTCOME beyond prior accepted scope.'
    path.write_text(json.dumps(revised))
    for artifact in ctx['artifacts'].glob('*.json'):
        if artifact == path: continue
        value = json.loads(artifact.read_bytes())
        if value.get('intent_revision') == prior['revision']:
            value['intent_revision'] = revised['revision']; artifact.write_text(json.dumps(value))
    w.event('intent-ready', deny='exact qualified prior bytes', code='software-change-intent-baseline-required')
    w.inspect([ctx['provider'], 'commission', '--slot', 'intent-review'], ctx['show'](), refused='exact qualified prior baseline')
    baseline = {'revision': prior['revision'], 'locator': 'intent.json@prior', 'sha256': 'sha256:' + sha(prior_bytes.encode()),
                'json_bytes': prior_bytes}
    if scope:
        # Honest scope change: repair with explicitly labelled scripted owner qualification, never a correction waiver.
        owner = w.append('user-steering', {'target': {'kind': 'all'}, 'instruction': LABEL + '; scripted owner scope qualification',
                                         'intent_baseline': baseline})
        w.event('intent-ready')
        packet = w.commission('intent-review')
        require(packet['owner_approval_view']['exact_substantive_wording_delta']['source_kind'] == 'user-steering', packet)
        w.retain('scope-early-refusal-qualified-repair', {'owner': owner, 'packet': packet})
        return
    correction = w.append('intent-correction', {'subject': 'intent.json', 'subject_revision': revised['revision'],
        'prior_intent': baseline, 'findings': [{'ledger_id': ledger['id'], 'finding_id': f['id']}],
        'author': {'name': 'new-toy-driver', 'kind': 'agent'}, 'reason': LABEL + '; declared within accepted intent'})
    w.event('intent-ready')
    packet = w.commission('intent-review')
    view = packet['owner_approval_view']; delta = view['exact_substantive_wording_delta']
    require(delta['source_kind'] == 'intent-correction' and delta['from_revision'] == prior['revision'] and
            {'pointer': '/outcome', 'change': 'replace', 'before': prior['outcome'], 'after': revised['outcome']} in delta['changes'], packet)
    require(not any(r['kind'] == 'user-steering' and 'intent_baseline' in r['data']
                    for r in ctx['show']()['result']['context']), 'Owner-qualified baseline must be absent')
    # One explicitly unaffected axis keeps actual earlier judgments; all remaining axes refreshed.
    unaffected = 'constraints-are-limits'  # Real complete individual group; its constraint text is unchanged.
    require(revised['constraints'] == prior['constraints'], 'Unaffected constraint group needs exact unchanged constraints')
    require(any(p['id'] == unaffected for p in policies), policies)
    carried = [w.append('evidence-applicability', w.carry(r, 'intent.json')) for r in rows
               if r['data']['policy_id'] == unaffected and r['data']['review_stage'] == 'individual']
    require(carried, 'Need an actual unaffected complete individual group')
    carry_floor = next(p['required_authors'] for p in policies
                       if p['id'] == unaffected and p['review_stage'] == 'individual')
    require(len(carried) == carry_floor == 2, 'Both actual canonical individual authors must be carried')
    fresh = w.review('intent-review', 'intent.json', [p for p in policies
                     if not (p['id'] == unaffected and p['review_stage'] == 'individual')])
    f.update(status='resolved', reason=LABEL + '; exact source corrected and fresh current judgments retained')
    w.ledger('intent-review', 'intent.json', [f])
    cold = [r for r in fresh if r['data']['policy_id'] == 'intent-sufficient']
    require(cold, 'Cold reconstruction duty intact')
    auth = w.append('user-steering', {'target': {'kind': 'slots', 'ids': ['intent-review']},
           'instruction': LABEL + '; simulated authorization for toy driver comparison only'})
    comparison = w.append('intent-comparison', {'subject_revision': revised['revision'], 'baseline': revised['baseline'],
        'reconstruction_ids': [r['id'] for r in cold], 'actor': {'name': 'new-toy-driver', 'kind': 'agent'},
        'authority': auth['id'], 'dispositions': [{'reconstruction_id': r['id'], 'alternative': LABEL,
        'disposition': 'implementation-freedom', 'note': LABEL} for r in cold]})
    w.event('approved')
    packet = w.commission('intent-adversarial-review')
    # AC7 delivery has typed routes, not one generic context/source-ID union.
    stored_rows = ctx['show']()['result']['context']
    stored = {r['id']: r for r in stored_rows}
    require(len(stored) == len(stored_rows), 'Canonical stored IDs must be unique')
    selected = {r['id']: r for r in packet['context']}
    sources = {r['record_id']: r for r in packet['review_record_sources']}
    require(len(selected) == len(packet['context']) and len(sources) == len(packet['review_record_sources']) and
            set(sources) == set(selected) == set(packet['commission']['record_ids']) and
            len(packet['commission']['record_ids']) == len(selected), 'Selected projection IDs disagree')
    for record in cold + [correction, auth]:
        require(stored.get(record['id']) == record and selected.get(record['id']) == record and
                sources.get(record['id']) == {'record_id': record['id'], 'kind': record['kind']},
                'Required cold/correction/authorization must be canonical complete selected records')
    cold_ids = [r['id'] for r in cold]
    require(len(cold_ids) == len(set(cold_ids)) == 2 and all(
        r['kind'] == 'review-evidence' and r['data']['gate'] == 'intent-review' and
        r['data']['review_stage'] == 'aggregate' and r['data']['policy_id'] == 'intent-sufficient' and
        r['data']['subject_revision'] == revised['revision'] and r['data']['baseline'] == revised['baseline'] and
        isinstance(r['data']['grounds']['reason'], str) and r['data']['grounds']['reason'].strip()
        for r in cold), 'Both declared-baseline cold sources and /data/grounds/reason must resolve')
    comparisons = [r for r in packet['intent_comparisons'] if r['id'] == comparison['id']]
    require(len(comparisons) == 1 and comparisons[0] == stored.get(comparison['id']) == comparison and
            comparison['kind'] == 'intent-comparison' and
            comparison['data']['subject_revision'] == revised['revision'] and
            comparison['data']['baseline'] == revised['baseline'] and
            comparison['data']['authority'] == auth['id'] and
            comparison['data']['reconstruction_ids'] == cold_ids, 'Complete typed comparison differs from stored current source')
    dispositions = comparisons[0]['data']['dispositions']  # actual /data/dispositions
    require(isinstance(dispositions, list) and [d['reconstruction_id'] for d in dispositions] == cold_ids and
            all(d['disposition'] == 'implementation-freedom' and d['alternative'].strip() and d['note'].strip()
                for d in dispositions), 'Comparison must address exactly both actual cold IDs')
    require(auth['kind'] == 'user-steering' and isinstance(auth['data']['instruction'], str) and
            auth['data']['instruction'].strip() and correction['kind'] == 'intent-correction' and
            correction['data']['subject_revision'] == revised['revision'] and
            correction['data']['prior_intent'] == baseline and
            stored[correction['id']]['data']['prior_intent']['json_bytes'] == prior_bytes,
            'Authorization /data/instruction and correction /data/prior_intent/json_bytes must resolve')
    qualified = packet['intent_revision_baseline']
    require(qualified['record'] == stored[correction['id']] and qualified['delivered_in_context'] is True and
            qualified['source']['record_id'] == correction['id'] and
            qualified['source']['source_kind'] == 'intent-correction' and qualified['source']['baseline'] == baseline,
            'Qualified baseline provenance or delivery changed')
    challenge_delta = packet['owner_approval_view']['exact_substantive_wording_delta']
    require(challenge_delta['source_kind'] == 'intent-correction' and challenge_delta['source_record_id'] == correction['id'] and
            challenge_delta['from_revision'] == prior['revision'] and challenge_delta['to_revision'] == revised['revision'] and
            {'pointer': '/outcome', 'change': 'replace', 'before': prior['outcome'], 'after': revised['outcome']} in challenge_delta['changes'] and
            packet['owner_approval_view']['actors']['owner_approval']['status'] == 'not-performed', 'Exact delta/owner status changed')
    require(packet['missing_inputs'] == [] and packet['record_diagnostics'] == [] and
            packet['commission']['diagnostics'] == [], 'Packet diagnostics need explicit repair; presence is not clearance')
    w.retain('typed-challenge-packet-delivery', {'packet': packet, 'cold': cold,
             'comparison': comparison, 'correction': correction, 'authorization': auth,
             'canonical_prior_bytes': prior_bytes, 'comparison_route': 'intent_comparisons',
             'selected_routes': 'context + review_record_sources',
             'limit': 'Exact equality is for these required source kinds; ledger projections may trim findings'})
    policies = ctx['profile']['review_policies']['intent-adversarial-review']
    _, batch = next((k, b) for k, b in groups(policies) if k[0] == 'aggregate')
    good_preview = w.preview('intent-adversarial-review', 'intent.json', [batch[0]], {'name': 'new-toy-intent-adversarial-review-0', 'kind': 'agent'},
                            locators=[f"record:{cold[0]['id']}#/data/grounds/reason", f"record:{comparison['id']}#/data/dispositions",
                                      f"record:{auth['id']}#/data/instruction"])
    require(len(good_preview['records']) == 1, good_preview)
    good = good_preview['records'][0]['data']
    wrong = w.append('opaque-toy-note', {'x': 'wrong source kind'})
    for tag, locator, needle in [('malformed', 'record:bad/id#/data', 'ID is malformed'),
                                ('missing', 'record:no-such-record#/data', 'missing or ambiguous'),
                                ('wrong-kind', f"record:{wrong['id']}#/data/x", 'disallowed kind'),
                                ('pointer', f"record:{comparison['id']}#/data/no-such-field", 'does not resolve')]:
        bad = copy.deepcopy(good); bad['grounds']['evidence'] = [{'locator': locator}]
        w.refuse('review-evidence', bad, 'record-citation-' + tag, 'review-shape-policy', needle)
        w.append('review-evidence', good)
    w.review('intent-adversarial-review', 'intent.json', policies,
             locators=[f"record:{cold[0]['id']}#/data/grounds/reason", f"record:{comparison['id']}#/data/dispositions"])
    w.ledger('intent-adversarial-review', 'intent.json')
    w.event('approved')
    actual = {r['id']: r for r in ctx['show']()['result']['context']}
    require(all(actual[r['id']] == r for r in rows + carried), 'Unaffected/original judgments changed')
    w.retain('ac7-ac8-correction-challenge-allow', {'prior_bytes': prior_bytes, 'current': revised,
             'correction': correction, 'comparison': comparison, 'cold': cold, 'unaffected_carries': carried, 'packet': packet})


def overlay_policies(ctx):
    gate = 'validation-review'
    policies = copy.deepcopy(ctx['profile']['review_policies'][gate])
    for stage in sorted({p['review_stage'] for p in policies}):
        floor = next(p['required_authors'] for p in policies if p['review_stage'] == stage)
        for axis in ('ids-grounded', 'bypass-not-green'):
            policies.append({'id': axis, 'review_stage': stage, 'group': f'{gate}.{stage}.{axis}', 'required_authors': floor})
    return policies


def overlay_case(w, args):
    ctx = w.ctx
    policies = overlay_policies(ctx)
    w.refuse('approved-artifact', {'subject': 'design.json'}, 'bad-approval',
             'approved-artifact-shape', 'malformed')
    for kind in ('criterion-verdict', 'goal-verdict'):
        original = next(r['data'] for r in ctx['show']()['result']['context'] if r['kind'] == kind)
        bad = copy.deepcopy(original)
        bad['gate'] = 'design-review'
        w.refuse(kind, bad, 'bad-' + kind, kind + '-shape-source', 'incompatible gate/stage/frozen config')
    counts = w.inspect([ctx['provider'], 'author-counts'], ctx['show']())
    for p in policies:
        require(counts['effective'][f"review.validation-review.{p['review_stage']}.{p['id']}"] == p['required_authors'], counts)
    packet = w.commission('validation-review')
    _, batch = next((k, b) for k, b in groups(policies) if b[0]['id'] == 'ids-grounded' and k[0] == 'aggregate')
    result = w.preview('validation-review', 'validation-report.json', batch, {'name': 'new-toy-validation-review-0', 'kind': 'agent'})
    require(len(result['records']) == 1, result)
    good = result['records'][0]['data']; bad = copy.deepcopy(good)
    bad['axis'] = bad['policy_id'] = 'not-configured-overlay'
    w.refuse('review-evidence', bad, 'unknown-overlay-axis', 'review-shape-policy', 'not configured')
    w.append('review-evidence', good)
    # A generated checkpoint is read by an ordinary bounded line reader, not json.load alone.
    checkpoint = ctx['artifacts'] / 'validation-checkpoint.json'
    copied = w.log.root.parent / 'checkpoint-original.json'
    shutil.copyfile(checkpoint, copied)
    read = w.inspect([sys.executable, '-B', HERE / 'line_reader.py', checkpoint, '--max-line-bytes', '8192'], None)
    value = json.loads(checkpoint.read_bytes())
    require(read['bytes'] >= 130000 and read['windows'] > 1 and read['sha256'] == sha(copied.read_bytes()), read)
    report = ctx['artifacts'] / value['report']['file']
    require(value['report']['sha256'] == 'sha256:' + sha(report.read_bytes()), value)
    repo = value['repository']; without = {k: repo[k] for k in ('head', 'index_sha256', 'status_sha256', 'entries')}
    require(repo['state_sha256'] == 'sha256:' + sha(json.dumps(without, ensure_ascii=False, separators=(',', ':')).encode()), value)
    for field, argv in [('index_sha256', ['git', 'ls-files', '--stage', '-z']),
                        ('status_sha256', ['git', 'status', '--porcelain=v2', '-z', '--untracked-files=all', '--ignored=no'])]:
        observed = w.log.execute(argv, cwd=ctx['repository'], raw=True)
        require(observed.returncode == 0 and repo[field] == 'sha256:' + sha(observed.stdout.encode()), (field, repo))
    head = w.log.execute(['git', 'rev-parse', 'HEAD'], cwd=ctx['repository'], raw=True)
    require(head.returncode == 0 and repo['head'] == head.stdout.strip(), repo)
    for e in repo['entries']:
        p = ctx['repository'] / e['path']
        data = os.fsencode(os.readlink(p)) if e['kind'] == 'symlink' else p.read_bytes() if e['kind'] == 'regular' else None
        require(data is not None and e['content_sha256'] == 'sha256:' + sha(data), e)
    w.retain('checkpoint-line-reader-digests', {'reader': read, 'checkpoint_path': str(copied),
                                             'checkpoint_sha256': sha(copied.read_bytes()), 'checkpoint': value})
    # Deliberate internal-digest damage stays a negative, pristine generated bytes restored.
    bad = copy.deepcopy(value); bad['repository']['state_sha256'] = 'sha256:' + '0' * 64
    checkpoint.write_text(json.dumps(bad, indent=2))
    try:
        w.event('passed', deny='repository state digest changed at `state_sha256`',
                code='software-change-checkpoint-invalid')
    finally:
        checkpoint.write_bytes(copied.read_bytes())
    w.review('validation-review', 'validation-report.json', policies)
    w.ledger('validation-review', 'validation-report.json')
    w.event('passed')
    w.retain('overlay-helper-write-gate-parity', {'effective_counts': counts, 'commission': packet})



def run_case(args, journey, case):
    profile_source = args.source_root / 'crates/software-change-provider/data/configs/high-rigor.json'
    original = json.loads(profile_source.read_bytes())
    root = args.output_root
    case_args = SimpleNamespace(**vars(args))
    marker = root / 'incompatible-admit-marker'
    if case == 'admission-metadata':
        shim = root / 'provider-transport.py'
        shim.write_text('#!' + sys.executable + '\nimport json, os, pathlib, subprocess, sys\n' +
            'real = ' + repr(str(args.provider)) + '\n' +
            'if sys.argv[1:]: os.execv(real, [real, *sys.argv[1:]])\n' +
            'raw = sys.stdin.buffer.read()\np = json.loads(raw)\n' +
            'if p.get("operation") == "admit" and p.get("record", {}).get("id") == "cas-stale":\n' +
            '    root = pathlib.Path(' + repr(str(root)) + ')\n' +
            '    (root / "cas-ready").write_text("ready")\n' +
            '    import time\n' +
            '    while not (root / "cas-release").exists(): time.sleep(0.01)\n' +
            'if p.get("operation") == "admit" and pathlib.Path(' + repr(str(marker)) + ').exists():\n' +
            '    print("[]"); sys.exit(0)\n' +
            'sys.exit(subprocess.run([real], input=raw).returncode)\n')
        shim.chmod(0o755)
        case_args.provider = shim
        dump(root / 'transport-identity.json', {'label': LABEL, 'path': str(shim), 'sha256': sha(shim.read_bytes()),
             'scope': 'all operations byte-delegated; admit barrier for CAS and incompatible JSON array response while marker exists'})
    log = Capture(root / 'commands', args)
    log.transport_marker = marker
    state = {'done': False}
    targets = {'design-review'} if case == 'admission-metadata' else {'validation-review'} if case == 'overlay-checkpoint' else {'intent-review', 'intent-adversarial-review'}
    def configure(profile, repository, artifacts):
        for gate in profile['review_policies']:
            if gate not in targets: profile['review_policies'][gate] = []
        for gate in targets:
            require(profile['review_policies'][gate] == original['review_policies'][gate], 'Target policy altered')
        profile['extra']['advice']['enabled'] = False
        templates = artifacts.parent / 'templates'
        shutil.copytree(args.source_root / 'crates/software-change-provider/data/templates', templates)
        profile['extra']['template_root'] = str(templates)
        if case == 'overlay-checkpoint':
            fixture = load_module(HERE / 'bookends_fixture.py', 'new_bookends_fixture')
            fixture.configure(profile, repository, artifacts, log, args)
        dump(root / 'fixture-profile.json', {'label': LABEL, 'base': original, 'actual': profile,
              'authorized_isolation': 'Only non-target rooms removed; target policies/floors + criterion/goal duties retained; toy Bookends only'})
    def hook(phase, ctx):
        selected = 'design-review' if case == 'admission-metadata' else 'validation-review' if case == 'overlay-checkpoint' else 'explore'
        if phase == 'reconciliation' and case == 'overlay-checkpoint':
            from bookends_fixture import reconcile
            reconcile(ctx)
            w = Witness(ctx, log, case)
            report_path = ctx['artifacts'] / 'implementation-report.json'
            saved = report_path.read_bytes(); bad = json.loads(saved)
            bad['residuals'] = [{'kind': 'accepted-boundary', 'statement': LABEL,
                                 'references': ['intent.json#/outcome'], 'accepted_by': 'wrong scalar'}]
            report_path.write_text(json.dumps(bad)); ctx['checkpoint']('implementation')
            w.event('reconciliation-ready', deny='accepted_by', code='software-change-schema-invalid')
            report_path.write_bytes(saved); ctx['checkpoint']('implementation')
            w.event('reconciliation-ready')
            state.setdefault('extra_edges', []).extend(w.edges)
            return True
        if phase == 'validation-judgments' and case == 'overlay-checkpoint':
            w = Witness(ctx, log, case)
            original_candidate = ctx['prepared']['command_candidates'][0]
            bad = copy.deepcopy(original_candidate['data'])
            bad['external_result']['source']['head'] = '0' * 40
            results = [dict(bad['external_result'], id=bad['proof_id'])]
            prepared = w.inspect([ctx['provider'], 'prepare-validation'], {'show': ctx['show'](),
                'working_directory': str(ctx['repository']), 'revision': 'new-negative-validation-helper',
                'author': {'name': 'new-toy-proof-owner', 'kind': 'agent'}, 'command_results': results})
            require(not prepared['commands_complete'] and not prepared['command_candidates'] and
                    any(d.get('rule') == 'command-evidence-current-source' for d in prepared['diagnostics']), prepared)
            w.refuse('command-evidence', bad, 'validation-helper-stale-source',
                     'command-evidence-current-source', 'stale-source')
            w.append(original_candidate['kind'], original_candidate['data'], 'validation-helper-stale-source')
            w.retain('validation-helper-current-candidate', {'prepared': ctx['prepared'], 'candidate': original_candidate})
            state.setdefault('extra_edges', []).extend(w.edges)
            return False
        if phase != selected or state['done']: return False
        state['done'] = True
        shown = ctx['show']()['result']
        require(shown['workflow_graph'].get('admit') is True and
                Path(shown['initial_input']['artifact_root']).resolve() == ctx['artifacts'].resolve(),
                'Actual integrated public admission/artifact_root required')
        ctx['_transport_marker'] = marker
        ctx['_real_provider'] = args.provider
        w = Witness(ctx, log, case)
        if case == 'admission-metadata': metadata_case(w)
        elif case in ('correction-citations', 'scope-change'): correction_case(w, scope=case == 'scope-change')
        else: overlay_case(w, args)
        state['witness'] = w
        return True
    stop = 'plan' if case == 'admission-metadata' else 'intent-review' if case == 'scope-change' else 'design' if case == 'correction-citations' else 'end'
    work = journey.run(case_args, profile_source, root, configure=configure, exercise=hook,
                       stop_at=stop, command_log=log, judgment_reason=LABEL)
    require(state['done'] and 'witness' in state, 'Missing case controls')
    shown = log.execute([args.engine, '--database', work / 'run.sqlite', '--config', work / 'providers.toml',
                         '--json', 'show', '--view', 'full', 'software-external'], cwd=work / 'repository')
    history = log.execute([args.engine, '--database', work / 'run.sqlite', '--config', work / 'providers.toml',
                          '--json', 'history', 'software-external'], cwd=work / 'repository')
    require(shown['result']['current_state'] == stop and not shown['result']['has_overrides'], shown)
    result = {'case': case, 'status': 'pass-mechanics', 'terminal': shown, 'history': history,
              'edges': state.get('extra_edges', []) + state['witness'].edges, 'command_index': str(log.index)}
    dump(root / 'case-result.json', result)

    log.finalize()
