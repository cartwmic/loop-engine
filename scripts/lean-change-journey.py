#!/usr/bin/env python3
"""Lean-change journey through the real engine CLI: a scripted dummy worker drives profile B (review) and C (no review) to completion."""
import argparse
import json
import subprocess
import sys
import tempfile
from pathlib import Path
from external_work_driver import engine_call

PRINCIPLES = '# Principles\n1. Compose small parts.\n'
QUESTIONS = ['intent', 'scope', 'edge-cases', 'checks', 'docs', 'principles']
CHECK_QUESTIONS = ['use', 'edge', 'control', 'terms', 'criteria-gap']
OWNER = {'name': 'owner', 'kind': 'human'}
WORKER = {'name': 'worker', 'kind': 'script'}


def intent(hard=False):
    check = {'argv': ['sh', '-c', 'grep -q hello greeting.txt'], 'timeout_ms': 10000}
    other = {'argv': ['sh', '-c', 'test -f greeting.txt'], 'timeout_ms': 10000}
    return {'author': WORKER, 'goal': 'Add a greeting file.', 'constraints': ['Touch only greeting.txt.'], 'hard': hard,
            'criteria': [{'id': 'says-hello', 'statement': 'greeting.txt says hello', 'check': check},
                         {'id': 'exists', 'statement': 'greeting.txt exists', 'check': other}]}


def review(tree, fail_principles):
    answers = [{'question': q, 'verdict': 'pass', 'note': 'looks right'} for q in QUESTIONS]
    findings = []
    if fail_principles:
        answers[-1]['verdict'] = 'fail'
        findings = [{'id': 'f1', 'question': 'principles', 'text': 'Greeting is hard-coded.',
                     'evidence': 'greeting.txt line 1', 'consequence': 'Changing it needs a code edit.'}]
    return {'intent': 'intent-1', 'tree': tree, 'reviewer': {'name': 'fresh-reviewer', 'kind': 'agent'},
            'fresh_context': True, 'answers': answers, 'findings': findings}


def intent_check(reviewer='fresh-reviewer', fail=None):
    answers = [{'question': q, 'verdict': 'fail' if q == fail else 'pass', 'note': 'checked'} for q in CHECK_QUESTIONS]
    findings = []
    if fail:
        findings = [{'id': 'f1', 'question': fail, 'text': 'Which greeting text do you want?', 'evidence': 'The goal says only a greeting.',
                     'consequence': 'A delivery would pick any text.'}]
    return {'intent': 'intent-1', 'reviewer': {'name': reviewer, 'kind': 'agent'}, 'fresh_context': True,
            'answers': answers, 'findings': findings}


class Journey:
    def __init__(self, engine, provider, work):
        self.engine, self.provider, self.work = engine, provider, work
        self.config = work / 'providers.toml'
        self.config.write_text(f'[providers.lean-change]\ncommand = {json.dumps(str(provider))}\nargs = []\n')
        self.database = work / 'run.sqlite'
        self.run_id = None
        self.appended = set()

    def engine_args(self, *args):
        return [str(self.engine), '--database', str(self.database), '--config', str(self.config), '--json', *args]

    def call(self, *args):
        value = engine_call(self.engine, self.database, self.config, list(args))
        assert value['status'] == 'completed', value
        return value['result']

    def state(self):
        return self.call('show', '--view', 'full', self.run_id)['current_state']

    def show_text(self):
        return subprocess.run(self.engine_args('show', '--view', 'full', self.run_id), text=True, capture_output=True, check=True).stdout

    def append(self, record_id, kind, data):
        self.state()  # the engine refuses mutation of a run that was not just observed
        self.call('append', '--record-id', record_id, self.run_id, kind, json.dumps(data))
        self.appended.add(record_id)

    def event(self, name):
        self.state()
        self.call('event', self.run_id, name)

    def refused(self, event, code, expect_state):
        """A checked event must be denied with this code and must leave the state unchanged."""
        self.state()
        result = subprocess.run(self.engine_args('event', self.run_id, event), text=True, capture_output=True)
        assert result.returncode != 0 and code in result.stdout, (event, code, result.stdout, result.stderr)
        assert self.state() == expect_state
        return result.stdout

    def tool(self, command, stdin):
        result = subprocess.run([str(self.provider), command], input=stdin, text=True, capture_output=True)
        assert result.returncode == 0, (command, result.stderr)
        return result.stdout

    def verify(self):
        data = json.loads(self.tool('verify', self.show_text()))
        return data


def git(repo, *args):
    subprocess.run(['git', *args], cwd=repo, check=True, capture_output=True)


def run_profile(engine, provider, profile, work):
    j = Journey(engine, provider, work)
    repo = work / 'repo'
    repo.mkdir()
    git(repo, 'init', '-q')
    git(repo, 'config', 'user.email', 'j@example.invalid')
    git(repo, 'config', 'user.name', 'j')
    (repo / 'README.md').write_text('fixture\n')
    git(repo, 'add', '-A')
    git(repo, 'commit', '-qm', 'base')
    principles = work / 'principles.md'
    principles.write_text(PRINCIPLES)
    setup = [str(provider), 'setup', '--profile', profile, '--repo', str(repo)]
    if profile == 'b':
        setup += ['--principles', str(principles)]
    profile_path = work / 'profile.json'
    profile_path.write_text(subprocess.run(setup, text=True, capture_output=True, check=True).stdout)
    # bookends:LE-203 bookends:LE-208 — setup freezes repo, base and (for B) the principles snapshot; the run starts in explore.
    j.run_id = f'lean-{profile}'
    j.call('start', '--id', j.run_id, 'lean-change', '@' + str(profile_path), 'lean journey')
    assert j.state() == 'explore'
    shown = j.call('show', '--view', 'full', j.run_id)['workflow_graph']
    states = [s['id'] for s in shown['states']]
    # bookends:LE-202 — B has a review state; C is the same workflow without it.
    assert ('review' in states) == (profile == 'b'), states

    # bookends:LE-203 — no intent, no way out of explore.
    j.refused('intent-ready', 'intent-missing', 'explore')
    j.append('intent-1', 'intent', intent())
    if profile == 'b':
        # bookends:LE-209 — profile b: no intent-ready without one independent check with every finding disposed.
        j.refused('intent-ready', 'intent-check-incomplete', 'explore')
        packet = j.tool('intent-packet', j.show_text())
        assert all(f'- {q}: ' in packet for q in CHECK_QUESTIONS) and 'intent-1' in packet, packet
        assert 'pass every criterion\'s check and still leave the goal' in packet, packet
        j.append('chk-own', 'intent-check', intent_check('worker'))
        out = j.refused('intent-ready', 'intent-check-incomplete', 'explore')
        assert 'someone other' in out, out
        j.append('chk-1', 'intent-check', intent_check(fail='terms'))
        out = j.refused('intent-ready', 'intent-check-incomplete', 'explore')
        assert 'disposition' in out, out
        j.append('chk-disp-1', 'finding-disposition', {'review': 'chk-1', 'finding': 'f1', 'disposition': 'rejected',
                                                       'reason': 'Any greeting text is fine; the criteria check for hello.'})
    j.event('intent-ready')
    assert j.state() == 'intent-approval'

    # bookends:LE-204 — only the owner's verbatim words, bound to this intent record, approve it.
    j.refused('approved', 'owner-decision-missing', 'intent-approval')
    j.append('d-blank', 'owner-decision', {'checkpoint': 'intent', 'subject': 'intent', 'subject_revision': 'intent-1',
                                         'decision': 'approve', 'text': ' ', 'actor': OWNER})
    j.refused('approved', 'owner-decision-missing', 'intent-approval')
    j.append('d-ok', 'owner-decision', {'checkpoint': 'intent', 'subject': 'intent', 'subject_revision': 'intent-1',
                                      'decision': 'approve', 'text': 'Yes, add the greeting.', 'actor': OWNER})
    j.refused('approved-hard', 'hard-mismatch', 'intent-approval')
    j.event('approved')
    assert j.state() == 'implement'
    j.event('ready')
    assert j.state() == 'verify'

    # bookends:LE-206 — every criterion needs a passing check on the current tree: none, a missing one, a stale one.
    j.refused('verified', 'verification-incomplete', 'verify')
    (repo / 'greeting.txt').write_text('hello\n')
    partial = j.verify()
    assert [r['exit_code'] for r in partial['results']] == [0, 0] and partial['intent'] == 'intent-1'
    dropped = dict(partial, results=partial['results'][:1])
    j.append('v-partial', 'verification', dropped)
    out = j.refused('verified', 'verification-incomplete', 'verify')
    assert 'missing' in out, out
    j.append('v-1', 'verification', partial)
    (repo / 'notes.txt').write_text('late edit\n')
    out = j.refused('verified', 'verification-incomplete', 'verify')
    assert 'stale' in out, out
    fresh = j.verify()
    j.append('v-2', 'verification', fresh)

    if profile == 'c':
        j.event('verified')
    else:
        j.event('verified')
        assert j.state() == 'review'
        # bookends:LE-207 — one independent review: refused without a review, without independence, without dispositions.
        j.refused('reviewed', 'review-incomplete', 'review')
        packet = j.tool('review-packet', j.show_text())
        assert all(q in packet for q in QUESTIONS) and 'Compose small parts' in packet and 'greeting.txt' in packet
        assert 'living PRD' in packet and 'the check that proves it' in packet, packet
        own = review(fresh['tree'], True)
        own['reviewer']['name'] = 'worker'
        j.append('rev-own', 'review', own)
        out = j.refused('reviewed', 'review-incomplete', 'review')
        assert 'someone other' in out, out
        j.append('rev-1', 'review', review(fresh['tree'], True))
        out = j.refused('reviewed', 'review-incomplete', 'review')
        assert 'disposition' in out, out
        j.append('disp-1', 'finding-disposition', {'review': 'rev-1', 'finding': 'f1', 'disposition': 'rejected',
                                                   'reason': 'The greeting is fixed by the intent.'})
        j.event('reviewed')
    final = j.call('show', '--view', 'full', j.run_id)
    # bookends:LE-202 — both profiles reach the final state through the real engine.
    assert final['current_state'] == 'end' and final['lifecycle'] == 'final', final
    # bookends:LE-107 — minimum provenance: the finished run holds only the driver's own records, each once.
    ids = [record['id'] for record in final['context']]
    assert len(ids) == len(set(ids)) and set(ids) == j.appended, (ids, j.appended)
    return j


def hard_run(engine, provider, work):
    """A hard intent routes through plan, is completed, and a replaced intent must be approved, planned and verified again."""
    # bookends:LE-205 — hard work takes the plan edge, and the plan edge needs a plan for the current intent.
    j = Journey(engine, provider, work)
    repo = work / 'repo'
    repo.mkdir()
    git(repo, 'init', '-q')
    git(repo, 'config', 'user.email', 'j@example.invalid')
    git(repo, 'config', 'user.name', 'j')
    (repo / 'README.md').write_text('fixture\n')
    git(repo, 'add', '-A')
    git(repo, 'commit', '-qm', 'base')
    profile_path = work / 'profile.json'
    profile_path.write_text(subprocess.run([str(provider), 'setup', '--profile', 'c', '--repo', str(repo)], text=True, capture_output=True, check=True).stdout)
    j.run_id = 'lean-hard'
    j.call('start', '--id', j.run_id, 'lean-change', '@' + str(profile_path), 'hard')
    j.append('intent-1', 'intent', intent(hard=True))
    j.event('intent-ready')
    j.append('d', 'owner-decision', {'checkpoint': 'intent', 'subject': 'intent', 'subject_revision': 'intent-1',
                                    'decision': 'approve', 'text': 'Go, with a plan.', 'actor': OWNER})
    j.refused('approved', 'hard-mismatch', 'intent-approval')
    j.event('approved-hard')
    assert j.state() == 'plan'
    j.refused('planned', 'plan-missing', 'plan')
    j.append('plan-1', 'plan', {'intent': 'intent-1', 'text': 'Write greeting.txt, then verify.'})
    j.event('planned')
    assert j.state() == 'implement'
    j.event('ready')
    (repo / 'greeting.txt').write_text('hello\n')
    j.append('v-1', 'verification', j.verify())
    # bookends:LE-206 — a later intent record does not inherit the earlier approval, plan or verification.
    swapped = intent(hard=True)
    swapped['criteria'][1]['check']['argv'] = ['sh', '-c', 'test -s greeting.txt']
    j.append('intent-2', 'intent', swapped)
    j.refused('verified', 'owner-decision-missing', 'verify')
    j.append('d2', 'owner-decision', {'checkpoint': 'intent', 'subject': 'intent', 'subject_revision': 'intent-2',
                                     'decision': 'approve', 'text': 'Yes, the revised checks.', 'actor': OWNER})
    j.refused('verified', 'plan-missing', 'verify')
    j.append('plan-2', 'plan', {'intent': 'intent-2', 'text': 'Same plan, stricter check.'})
    j.refused('verified', 'verification-incomplete', 'verify')
    j.append('v-2', 'verification', j.verify())
    j.event('verified')
    final = j.call('show', '--view', 'full', j.run_id)
    assert final['current_state'] == 'end' and final['lifecycle'] == 'final', final


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--engine')
    parser.add_argument('--provider')
    parser.add_argument('--self-test', action='store_true')
    args = parser.parse_args()
    # Bare invocation is the recognized CI collection form; full proof uses the parameterized run below.
    if args.self_test or len(sys.argv) == 1:
        assert [q for q in QUESTIONS] == sorted(set(QUESTIONS), key=QUESTIONS.index)
        assert len(intent()['criteria']) == 2 and review('t', True)['findings'] and intent_check(fail='terms')['findings']
        print('lean-change journey self-test passed')
        return
    for name in ['engine', 'provider']:
        if not getattr(args, name):
            parser.error(f'--{name} is required')
    engine, provider = Path(args.engine).resolve(), Path(args.provider).resolve()
    for name, action in [('b', lambda w: run_profile(engine, provider, 'b', w)),
                         ('c', lambda w: run_profile(engine, provider, 'c', w)),
                         ('hard', lambda w: hard_run(engine, provider, w))]:
        with tempfile.TemporaryDirectory(prefix=f'lean-change-{name}-') as temporary:
            action(Path(temporary))
        print(f'lean-change journey {name} passed')


if __name__ == '__main__':
    main()
