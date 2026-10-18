#!/usr/bin/env python3
"""Black-box proof of provider-checked locator repair (issue #45, intent AC-1..AC-3, AC-8).

SCRIPTED DUMMY REVIEWERS supply every return; they establish no semantic review and no real
approval. The built loop-engine and software-change programs run against a private catalog in a
driver-owned output root. The installed v0.26.0 provider is the control only: it has no repair
route and must still discard the whole return. --self-test checks collection and interface
mechanics only; it proves no behavior.
"""
# bookends:LE-151 bookends:LE-155 — a meaning-preserving, separately attributed correction keeps the reviewer's identity, result, findings and reason, and a wrong reference is never admitted.
# bookends:LE-170 bookends:LE-109 — the guided correction keeps the original return and is an inert candidate view until the driver appends it.
import argparse
import copy
import hashlib
import importlib.util
import json
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.dont_write_bytecode = True  # Journey artifacts and captures stay outside the checkout.
SPEC = importlib.util.spec_from_file_location('evidence_carry_journey', ROOT / 'scripts/evidence-carry-journey.py')
EC = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(EC)
LABEL = 'SCRIPTED DUMMY REVIEWERS: mechanical proof only, not semantic review or human approval'
GATE, SUBJECT = 'design-review', 'design.json'
Failure, require = EC.Failure, EC.require
COVERAGE = {'AC-1': ['repair'], 'AC-2': ['repair'], 'AC-3': ['repair'], 'AC-8': ['repair']}


def judgments(ctx, bad):
    """One aggregate return covering every axis; bad maps axis -> extra (unresolvable) locators."""
    rows = []
    for axis, policy in EC.policies(ctx, GATE).items():
        evidence = [SUBJECT + '#/revision'] + bad.get(axis, [])
        rows.append({'axis': axis, 'result': 'pass', 'findings': '',
                     'grounds': {'reason': 'Scripted current-subject inspection of ' + axis + '.',
                                 'evidence': [{'locator': l} for l in evidence]}})
    return rows


def batch(rows, author):
    return {'review_contract_version': 3, 'review_stage': 'aggregate', 'author': author, 'judgments': rows}


def packet(ctx, original, repaired=None, author=None, **extra):
    policy = next(iter(EC.policies(ctx, GATE).values()))
    author = author or {'name': 'scripted-reviewer', 'kind': 'agent'}
    stdin = {'show': ctx['show'](), 'request': {'text': 'Review ' + SUBJECT + '.'},
             'assignment': {'gate': GATE, 'stage': 'aggregate', 'subject': SUBJECT,
                            'subject_revision': EC.revision_of(ctx, SUBJECT), 'group': policy['group'],
                            'author': author},
             'original': {'text': json.dumps(original)}}
    if repaired is not None:
        stdin['locator_repair'] = {'driver': {'name': 'scripted-driver', 'kind': 'script'},
                                   'repaired': {'text': json.dumps(repaired)}}
    stdin.update(extra)
    return stdin


def run_intake(provider, stdin, cwd):
    """The public command exactly as a driver runs it; returns (exit code, stdout, stderr)."""
    done = subprocess.run([str(provider), 'review-candidates'], input=json.dumps(stdin), text=True,
                          capture_output=True, cwd=str(cwd))
    return done.returncode, done.stdout, done.stderr


def refused(ctx, stdin, fragments, label, table):
    """A refused repair: exit 2, nothing released, state/context/history unchanged, what differed named."""
    before = EC.snapshot(ctx)
    code, out, err = run_intake(ctx['provider'], stdin, ctx['repository'])
    EC.unchanged(before, EC.snapshot(ctx), label)
    row = {'exit': code, 'stdout': out, 'stderr': err.strip()}
    require(code == 2 and out.strip() == '' and 'locator repair refused' in err, (label, row))
    for fragment in fragments:
        require(fragment in err, (label, fragment, err))
    table[label] = {'exit': code, 'stderr': err.strip()}


def repair(args, root):
    ctx = EC.start_at(args, root / 'repair', 'minimal', GATE)
    intent = json.loads((ctx['artifacts'] / 'intent.json').read_text())
    count = len(intent['acceptance'])
    axes = list(EC.policies(ctx, GATE))
    first, second = axes[0], axes[1]
    author = {'name': 'scripted-reviewer', 'kind': 'agent'}
    # The two 2026-10-14 mistakes: one past the end of an array, one at the wrong level.
    off_by_one = f'intent.json#/acceptance/{count}'
    wrong_level = 'intent.json#/outside_obligations/0'
    original = batch(judgments(ctx, {first: [off_by_one], second: [wrong_level]}), author)
    good = copy.deepcopy(original)
    good['judgments'][0]['grounds']['evidence'][1]['locator'] = f'intent.json#/acceptance/{count - 1}'  # replaced
    del good['judgments'][1]['grounds']['evidence'][1]                                                  # dropped
    out = {}
    # Without a repair request the two rows with a bad locator are refused (the baseline behavior):
    # the other rows are released, but the group is incomplete and no reviewer assessment is whole.
    code, stdout, _ = run_intake(ctx['provider'], packet(ctx, original), ctx['repository'])
    result = json.loads(stdout)
    require(code == 0 and len(result['records']) == len(axes) - 2 and
            'does not resolve' in json.dumps(result['items']) and
            sum(1 for i in result['items'] if i['diagnostics']) == 2, result)
    # Control: v0.26.0 has no repair route; the request is an ordinary intake and the whole return is refused.
    if args.control_provider:
        code, stdout, _ = run_intake(args.control_provider, packet(ctx, original, good), ctx['repository'])
        result = json.loads(stdout)
        require(code == 0 and len(result['records']) == len(axes) - 2 and
                'does not resolve' in json.dumps(result['items']), result)
        out['AC-8-control-v0_26_0'] = {'records': len(result['records']), 'expected_if_repaired': len(axes),
                                       'refused_rows': sum(1 for i in result['items'] if i['diagnostics'])}
    # AC-2: one refused request per category; each leaves state, context and history unchanged.
    table = {}

    def variant(edit):
        changed = copy.deepcopy(good)
        edit(changed)
        return packet(ctx, original, changed)

    refused(ctx, variant(lambda c: c['judgments'][2].update(result='fail')), ['field result differs'], 'verdict', table)
    refused(ctx, variant(lambda c: c['judgments'][2].update(findings='new finding')), ['field findings differs'], 'findings', table)
    refused(ctx, variant(lambda c: c['judgments'][2]['grounds'].update(reason='different reason')),
            ['reason may not change'], 'reason', table)
    refused(ctx, variant(lambda c: c['judgments'][2].update(axis='renamed-axis')), ['changes axis'], 'axis', table)
    refused(ctx, variant(lambda c: c['author'].update(name='someone-else')), ['top-level field'], 'author', table)
    refused(ctx, variant(lambda c: c.update(assessment_id='another-assessment')), ['top-level field'], 'assessment', table)
    refused(ctx, variant(lambda c: c.update(review_stage='individual')), ['top-level field'], 'stage', table)
    refused(ctx, variant(lambda c: c.update(subject_revision='99')), ['top-level field'], 'subject-revision', table)
    refused(ctx, variant(lambda c: c['judgments'].pop()), ['may not be added, removed or reordered'], 'judgment-removed', table)
    refused(ctx, variant(lambda c: c['judgments'].append(copy.deepcopy(c['judgments'][0]))),
            ['may not be added, removed or reordered'], 'judgment-added', table)
    refused(ctx, variant(lambda c: c['judgments'].reverse()), ['changes axis'], 'judgments-reordered', table)
    refused(ctx, variant(lambda c: c['judgments'][0]['grounds']['evidence'][1].update(locator=f'intent.json#/acceptance/{count + 5}')),
            ['still does not resolve'], 'locator-still-unresolvable', table)
    refused(ctx, variant(lambda c: c['judgments'][0]['grounds'].update(evidence=[])),
            ['no evidence entry remains'], 'last-resolving-entry-removed', table)
    refused(ctx, variant(lambda c: c['judgments'][0]['grounds']['evidence'].append({'locator': 'intent.json#/revision'})),
            ['evidence may not be added'], 'evidence-added', table)
    refused(ctx, variant(lambda c: c['judgments'][3]['grounds']['evidence'].append({'locator': 'intent.json#/revision'})),
            ['may not change'], 'evidence-changed-where-nothing-was-wrong', table)
    refused(ctx, packet(ctx, good, good), ['nothing to repair'], 'nothing-to-repair', table)
    # A defect other than unresolvable locators is never made acceptable by this step.
    duplicated = copy.deepcopy(original)
    duplicated['judgments'][0]['grounds']['evidence'] = [{'locator': SUBJECT + '#/revision'},
                                                         {'locator': off_by_one}, {'locator': off_by_one}]
    fixed = copy.deepcopy(duplicated)
    fixed['judgments'][0]['grounds']['evidence'] = [{'locator': SUBJECT + '#/revision'},
                                                    {'locator': f'intent.json#/acceptance/{count - 1}'}]
    refused(ctx, packet(ctx, duplicated, fixed), ['defect other than unresolvable locators'], 'duplicated-locator-erased', table)
    sibling_original = copy.deepcopy(original)
    sibling_original['judgments'][3]['grounds']['reason'] = ''
    sibling_fixed = copy.deepcopy(good)
    sibling_fixed['judgments'][3]['grounds']['reason'] = ''
    refused(ctx, packet(ctx, sibling_original, sibling_fixed), ['defect other than unresolvable locators'], 'sibling-with-other-defect', table)
    refused(ctx, packet(ctx, original, good, projection=good), ['cannot be combined'], 'with-projection', table)
    # Citations the resolver refuses for a reason other than "the target does not exist" are never repaired,
    # even when an unresolvable locator beside them is: the cause, not the message text, decides.
    artifacts = ctx['artifacts']
    outside_dir = Path(tempfile.mkdtemp(prefix='outside-', dir=root))
    (outside_dir / 'target.json').write_text('{"a": 1}')
    (artifacts / 'does not resolve.json').symlink_to(outside_dir / 'target.json')
    (artifacts / 'does not resolve.txt').write_text('plain text\n')
    (artifacts / 'AGENTS.md').write_text('# title\nline two\n')

    def other_defect(label, locator, fragments):
        mixed = copy.deepcopy(original)
        mixed['judgments'][0]['grounds']['evidence'].append({'locator': locator})
        refused(ctx, packet(ctx, mixed, copy.deepcopy(good)),
                ['defect other than unresolvable locators'] + fragments, label, table)

    other_defect('malformed-line-selector', 'intent.json#L1-7', ['malformed'])
    other_defect('malformed-pointer-escape', 'intent.json#/revision~2', ['escape'])
    other_defect('out-of-root-symlink-named-like-a-message', 'does not resolve.json#/a', ['resolves outside'])
    other_defect('non-json-source-named-like-a-message', 'does not resolve.txt#/a', ['requires a JSON source'])
    other_defect('selector-text-like-a-message', 'AGENTS.md#/does not resolve', ['requires a JSON source'])
    other_defect('unknown-source-scope', 'nope:intent.json#/revision', ['unknown source scope'])
    out['AC-2'] = table
    # AC-1: the repair yields ordinary candidates; the reviewer's judgments are identical to the original.
    before_success = EC.snapshot(ctx)
    code, stdout, stderr = run_intake(ctx['provider'], packet(ctx, original, good), ctx['repository'])
    require(code == 0, (code, stderr))
    EC.unchanged(before_success, EC.snapshot(ctx), 'a successful repair intake is an inert view')
    result = json.loads(stdout)
    require(len(result['records']) == len(axes) and all(not i['diagnostics'] for i in result['items']), result)
    assessments = {r['data']['assessment_id'] for r in result['records']}
    require(len(assessments) == 1 and next(iter(assessments)).startswith('assessment:sha256:'), assessments)
    original_text = json.dumps(original)
    for record, source, repaired in zip(result['records'], original['judgments'], good['judgments']):
        data = record['data']
        require(record['kind'] == 'review-evidence', record)
        require(data['author'] == original['author'] and data['axis'] == source['axis'] and
                data['result'] == source['result'] and data['findings'] == source['findings'] and
                data['grounds']['reason'] == source['grounds']['reason'] and
                data['grounds']['evidence'] == repaired['grounds']['evidence'], (data, source))
        require(data['external_review']['original']['original'] == original_text, data['external_review'])
    corrections = {r['data']['axis']: r['data']['external_review']['locator_repair']['corrections']
                   for r in result['records']}
    require(corrections[first] == [{'before': off_by_one, 'after': f'intent.json#/acceptance/{count - 1}'}] and
            corrections[second] == [{'before': wrong_level, 'after': None}] and
            all(corrections[a] == [] for a in axes[2:]), corrections)
    ids = {}
    for record in result['records']:
        ids[record['data']['axis']] = EC.append(ctx, 'review-evidence', record['data'])['result']['context']['id']
    EC.ledger(ctx, GATE, SUBJECT)
    gate = EC.gate_answer(ctx, 'approved')
    require(gate['status'] == 'completed', gate)
    out['AC-1'] = {'observed_pointers': {'off_by_one': off_by_one, 'wrong_level': wrong_level},
                   'candidates': len(result['records']), 'gate': gate['status'],
                   'state_after': ctx['show']()['result']['current_state']}
    # AC-3: the stored record keeps the original, shows the repair and who made it, and is not a fresh review.
    stored = {r['id']: r for r in ctx['show']()['result']['context']}
    sample = stored[ids[first]]['data']
    review = sample['external_review']
    repair_block = review['locator_repair']
    require(review['original']['original'] == original_text, review)
    require(repair_block['kind'] == 'locator-repair' and
            repair_block['driver'] == {'name': 'scripted-driver', 'kind': 'script'} and
            repair_block['original_sha256'] == 'sha256:' + hashlib.sha256(original_text.encode()).hexdigest() and
            set(repair_block) == {'kind', 'driver', 'original_sha256', 'corrections'} and
            sample['author'] == original['author'], repair_block)
    require({stored[i]['data']['assessment_id'] for i in ids.values()} == assessments, 'assessment identity changed')
    out['AC-3'] = {'stored_record': ids[first], 'locator_repair': {k: repair_block[k] for k in
                   ('kind', 'driver', 'original_sha256', 'corrections')},
                   'original_bytes_recovered': True, 'author': sample['author']}
    # The repaired return counts as the original would: the same gate cleared with one author.
    return out


SCENARIOS = {'repair': repair}


def self_test():
    """Collection and interface mechanics only; proves no behavior."""
    for ac, names in COVERAGE.items():
        require(all(name in SCENARIOS for name in names), ac)
    print('locator-repair-journey --self-test: collection mechanics only (no behavior proved)')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--self-test', action='store_true')
    parser.add_argument('--engine')
    parser.add_argument('--provider')
    parser.add_argument('--control-engine')
    parser.add_argument('--control-provider')
    parser.add_argument('--output-root')
    args = parser.parse_args()
    if args.self_test:
        return self_test()
    for name in ('engine', 'provider', 'output_root'):
        if not getattr(args, name):
            parser.error(f'--{name.replace("_", "-")} is required')
    out = Path(args.output_root).resolve()
    out.mkdir(parents=True, exist_ok=True)
    results = {name: run(args, Path(tempfile.mkdtemp(prefix=name + '-', dir=out))) for name, run in SCENARIOS.items()}
    EC.dump(out / 'result.json', {'label': LABEL, 'status': 'passed', 'scenarios': results})
    print('locator-repair-journey: all scenarios passed')


if __name__ == '__main__':
    main()
