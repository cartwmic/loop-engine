#!/usr/bin/env python3
"""Lean-change journey through the real engine CLI: a scripted dummy worker drives profile B (review) and C (no review) to completion."""
import argparse
import html
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path
from external_work_driver import engine_call

PRINCIPLES = '# Principles\n1. Compose small parts.\n'
GREET_CHECK = 'out=$(sh app.sh Bob)\n[ "$out" = "Hello, Bob!" ] || { echo "got: $out"; exit 1; }\necho ok\n'
QUESTIONS = ['intent', 'scope', 'edge-cases', 'checks', 'docs', 'principles']
# A review with findings_schema 1 also answers the seventh question; a record without it answers the six above.
V1_QUESTIONS = QUESTIONS + ['smaller-design']
CHECK_QUESTIONS = ['use', 'edge', 'control', 'terms', 'criteria-gap', 'principles-gap']
OWNER = {'name': 'owner', 'kind': 'human'}
WORKER = {'name': 'worker', 'kind': 'script'}
REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_SKILL = REPO_ROOT / 'crates/lean-change-provider/skills/using-lean-change-provider/SKILL.md'
PARALLEL_SECTION = '## Parallel builders'
OUTCOME_SECTION = '## Outcome map'
REVIEW_MAP = '$HOME/.local/share/agent-harness/external-skills/accountable-review/review-map/SKILL.md'
FIXTURES = REPO_ROOT / 'scripts/fixtures/outcome-map'
AR_PAGE = FIXTURES / 'ar-page.html'
SHOW_FIXTURE = FIXTURES / 'le-parallel-1-show.json'
RAIL_LINK = '<a data-rail="evidence" href="#evidence">What was approved and what ran</a>'
OUTCOME_GOAL = 'Greet Bob with "Hello, Bob!" & say <b>hi</b>.'
GUARD_REASON = 'a guard on the proof file, which the old tree cannot inform'
REJECT_REASON = 'The goal names no empty name, so the criteria cover it.'
FIX_REASON = 'The intent now names the exact greeting.'
SCENARIOS = ['b', 'c', 'hard', 'old-vs-new', 'parallel', 'outcome-map', 'outcome-map-skill', 'outcome-map-fixture', 'outcome-map-browser', 'findings-v1', 'delegated']
# The stand-in full gate logs every call, then fails when the finished steps word their files differently: each step alone passes, the joined tree does not.
GATE = ('printf "%s %s\\n" "$(pwd -P)" "$(git rev-parse HEAD)" >> "$GATE_LOG"\n'
        'words=$(sed "s/^[a-z]*: //" one.txt two.txt | grep -vx todo | sort -u | wc -l)\n'
        '[ "$words" -le 1 ] || { echo "gate: one.txt and two.txt disagree" >&2; exit 1; }\n')
BUILDER_TIMEOUT = 20


def intent(hard=False):
    check = {'argv': ['sh', '-c', 'grep -q hello greeting.txt'], 'timeout_ms': 10000}
    other = {'argv': ['sh', '-c', 'test -f greeting.txt'], 'timeout_ms': 10000}
    # Both checks fail on the base commit, which has no greeting.txt: a feature, so each expects fails there.
    return {'author': WORKER, 'goal': 'Add a greeting file.', 'constraints': ['Touch only greeting.txt.'], 'hard': hard,
            'criteria': [{'id': 'says-hello', 'statement': 'greeting.txt says hello', 'check': check, 'old': {'expect': 'fails'}},
                         {'id': 'exists', 'statement': 'greeting.txt exists', 'check': other, 'old': {'expect': 'fails'}}]}


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


def v1_review(tree, failed, findings, reviewer='fresh-reviewer'):
    """A review with findings_schema 1: all seven answers; each question in `failed` fails."""
    answers = [{'question': q, 'verdict': 'fail' if q in failed else 'pass', 'note': 'looked'} for q in V1_QUESTIONS]
    return {'findings_schema': 1, 'intent': 'intent-1', 'tree': tree, 'reviewer': {'name': reviewer, 'kind': 'agent'},
            'fresh_context': True, 'answers': answers, 'findings': findings}


def v1_findings():
    """One complete blocking finding with a component, one blocking finding missing its consequence, and one note."""
    return [{'id': 'f1', 'severity': 'blocking', 'question': 'principles', 'text': 'Greeting is hard-coded.',
             'obligation': 'Principles section 1: compose small parts.', 'evidence': 'greeting.txt line 1',
             'consequence': 'Changing the greeting needs a code edit.', 'component': 'greeting file'},
            {'id': 'f2', 'severity': 'blocking', 'question': 'scope', 'text': 'A second file was touched.',
             'obligation': 'Constraint: touch only greeting.txt.', 'evidence': 'notes.txt in the diff', 'component': 'diff'},
            {'id': 'n1', 'severity': 'note', 'question': 'docs', 'text': 'The greeting could be mentioned in a README.'}]


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
        # The intent check also gets the owner's principles and asks which applicable section has no criterion.
        assert PRINCIPLES in packet and 'yet has no criterion or constraint covering it' in packet, packet
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


def old_vs_new_run(engine, provider, work):
    """A fix that must fail on the old tree, a refactor that must print the same, a new command that must be absent, and a relabelled old record."""
    # bookends:LE-210 — the old-vs-new check is on by default: fails needs two real failures on the old tree, same the same output, absent no pass, and verified judges the recorded old runs against the intent's expectation.
    j = Journey(engine, provider, work)
    repo = work / 'repo'
    repo.mkdir()
    git(repo, 'init', '-q')
    git(repo, 'config', 'user.email', 'j@example.invalid')
    git(repo, 'config', 'user.name', 'j')
    (repo / 'README.md').write_text('fixture\n')
    (repo / 'app.sh').write_text('printf "hello, %s\\n" "$1"\n')
    (repo / 'journeys').mkdir()
    (repo / 'journeys' / 'greet.sh').write_text(GREET_CHECK)
    git(repo, 'add', '-A')
    git(repo, 'commit', '-qm', 'base')
    profile_path = work / 'profile.json'
    profile_path.write_text(subprocess.run([str(provider), 'setup', '--profile', 'c', '--repo', str(repo), '--proof-path', 'journeys'],
                                           text=True, capture_output=True, check=True).stdout)
    assert json.loads(profile_path.read_text())['old_vs_new'] == {'proof_paths': ['journeys']}
    j.run_id = 'lean-old-vs-new'
    j.call('start', '--id', j.run_id, 'lean-change', '@' + str(profile_path), 'old vs new')
    timeout = 10000
    criteria = [
        {'id': 'greets', 'statement': 'app.sh greets Bob with Hello, Bob!', 'check': {'argv': ['sh', 'journeys/greet.sh'], 'timeout_ms': timeout},
         'old': {'expect': 'fails'}},
        {'id': 'readme', 'statement': 'README.md is still there and the output does not change', 'check': {'argv': ['sh', '-c', 'test -f README.md && echo readme-ok'], 'timeout_ms': timeout},
         'old': {'expect': 'same'}},
        {'id': 'tool', 'statement': 'tools/new.sh is a new command', 'check': {'argv': ['sh', 'tools/new.sh'], 'timeout_ms': timeout},
         'old': {'expect': 'absent'}},
        {'id': 'guard', 'statement': 'the proof script is intact', 'check': {'argv': ['test', '-f', 'journeys/greet.sh'], 'timeout_ms': timeout},
         'old': {'expect': 'none', 'reason': 'a guard on the proof file, which the old tree cannot inform'}},
    ]
    j.append('intent-1', 'intent', {'author': WORKER, 'goal': 'Greet Bob with Hello, Bob!', 'constraints': ['Touch only app.sh and tools/.'],
                                    'hard': False, 'criteria': criteria})
    j.event('intent-ready')
    j.append('d-ok', 'owner-decision', {'checkpoint': 'intent', 'subject': 'intent', 'subject_revision': 'intent-1',
                                      'decision': 'approve', 'text': 'Yes, greet Bob.', 'actor': OWNER})
    j.event('approved')
    j.event('ready')
    assert j.state() == 'verify'
    j.refused('verified', 'verification-incomplete', 'verify')
    (repo / 'app.sh').write_text('printf "Hello, %s!\\n" "$1"\n')
    (repo / 'tools').mkdir()
    (repo / 'tools' / 'new.sh').write_text('echo new\n')
    genuine = j.verify()
    assert [r['exit_code'] for r in genuine['results']] == [0, 0, 0, 0], genuine
    runs = {r['criterion']: [(run['exit_code'], run['timed_out']) for run in (r['old'] or {}).get('runs', [])] for r in genuine['results']}
    # A command that is missing on the old tree exits 127 or 2 by shell; either is a non-pass, as absent needs.
    assert runs['greets'] == [(1, False)] * 2 and runs['readme'] == [(0, False)] and runs['guard'] == [], runs
    assert len(runs['tool']) == 1 and runs['tool'][0][0] != 0 and not runs['tool'][0][1], runs
    # The old tree's record is judged against the intent: a record that relabels greets as none is refused, and names greets.
    forged = json.loads(json.dumps(genuine))
    forged['results'][0]['old'] = {'expect': 'none', 'runs': []}
    j.append('v-forged', 'verification', forged)
    out = j.refused('verified', 'verification-incomplete', 'verify')
    assert 'greets' in out and 'old (expect fails)' in out, out
    # Only old.expect changed: the genuine runs stay, and the record is still refused for its name.
    relabelled = json.loads(json.dumps(genuine))
    relabelled['results'][0]['old']['expect'] = 'absent'
    j.append('v-relabelled', 'verification', relabelled)
    out = j.refused('verified', 'verification-incomplete', 'verify')
    assert 'greets' in out and 'names expect absent instead of fails' in out, out
    # The none criterion's result with its old removed is refused for guard.
    unrecorded = json.loads(json.dumps(genuine))
    del unrecorded['results'][3]['old']
    j.append('v-unrecorded', 'verification', unrecorded)
    out = j.refused('verified', 'verification-incomplete', 'verify')
    assert 'guard' in out and 'the record has no old runs' in out, out
    j.append('v-1', 'verification', genuine)
    j.event('verified')
    final = j.call('show', '--view', 'full', j.run_id)
    assert final['current_state'] == 'end' and final['lifecycle'] == 'final', final


def skill_section(skill, heading):
    """The lines under one '## ' heading of the skill, up to the next '## ' heading."""
    lines = skill.read_text().splitlines()
    if heading not in lines:
        raise SystemExit(f'{skill}: no "{heading}" section')
    section = []
    for line in lines[lines.index(heading) + 1:]:
        if line.startswith('## '):
            break
        section.append(line)
    return section


def section_blocks(skill, heading, expected):
    """The sh blocks of one skill section, in order; there must be exactly expected of them."""
    blocks, block, indent = [], None, 0
    for line in skill_section(skill, heading):
        if block is None and line.strip() == '```sh':
            block, indent = [], len(line) - len(line.lstrip())
        elif block is not None and line.strip() == '```':
            blocks.append('\n'.join(block) + '\n')
            block = None
        elif block is not None:
            block.append(line[indent:])
    if len(blocks) != expected:
        raise SystemExit(f'{skill}: "{heading}" has {len(blocks)} sh blocks, expected {expected}')
    return blocks


def parallel_blocks(skill):
    """The skill's 'Parallel builders' sh blocks, in order: start a step, land a step, clean up a step."""
    return section_blocks(skill, PARALLEL_SECTION, 3)


def run_block(block, repo, work, step):
    env = dict(os.environ, REPO=str(repo), WORK=str(work), STEP=step)
    return subprocess.run(['sh', '-c', block], env=env, text=True, capture_output=True)


def builder(worktree, step, other, name, text, starts):
    """A scripted builder: records its start, waits until the other builder has started, then writes only its own file."""
    (starts / step).touch()
    deadline = time.monotonic() + BUILDER_TIMEOUT
    while not (starts / other).exists():
        if time.monotonic() > deadline:
            raise TimeoutError(f'builder {step}: {other} never started; the builders did not run at the same time')
        time.sleep(0.05)
    (worktree / name).write_text(text)


def build_together(steps, starts):
    failures = []

    def guarded(*args):
        try:
            builder(*args, starts)
        except Exception as error:
            failures.append(repr(error))
    threads = [threading.Thread(target=guarded, args=step) for step in steps]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join()
    assert not failures, failures


def git_out(repo, *args):
    return subprocess.run(['git', *args], cwd=repo, check=True, capture_output=True, text=True).stdout


def parallel_intent():
    def check(name, line):
        return {'argv': ['grep', '-qx', line, name], 'timeout_ms': 10000}
    return {'author': WORKER, 'goal': 'Finish both steps.', 'constraints': ['Touch only one.txt and two.txt.'], 'hard': False,
            'criteria': [{'id': 'one', 'statement': 'one.txt says one: done', 'check': check('one.txt', 'one: done'), 'old': {'expect': 'fails'}},
                         {'id': 'two', 'statement': 'two.txt says two: done', 'check': check('two.txt', 'two: done'), 'old': {'expect': 'fails'}}]}


def parallel_run(engine, provider, blocks, work):
    """Two steps built at once in worktrees outside the repo, landed, gated in the repo and fixed there; a third, conflicting step stops at landing; a fourth, whose commit fails, is kept."""
    # bookends:LE-211 — the skill's Parallel builders blocks, run as written, build two steps at once, land them, gate in the repo, fix the gate's finding there and gate again, stop a conflicting step at landing, and keep a step whose commit fails.
    start, land, clean = blocks
    j = Journey(engine, provider, work)
    repo, steps, starts, gate_log = work / 'repo', work / 'steps', work / 'starts', work / 'gate.log'
    for path in (repo, steps, starts):
        path.mkdir()
    git(repo, 'init', '-q')
    git(repo, 'config', 'user.email', 'j@example.invalid')
    git(repo, 'config', 'user.name', 'j')
    (repo / 'README.md').write_text('fixture\n')
    (repo / 'one.txt').write_text('one: todo\n')
    (repo / 'two.txt').write_text('two: todo\n')
    (repo / 'gate.sh').write_text(GATE)
    git(repo, 'add', '-A')
    git(repo, 'commit', '-qm', 'base')
    principles = work / 'principles.md'
    principles.write_text(PRINCIPLES)
    profile_path = work / 'profile.json'
    profile_path.write_text(subprocess.run([str(provider), 'setup', '--profile', 'b', '--repo', str(repo), '--principles', str(principles)],
                                           text=True, capture_output=True, check=True).stdout)
    j.run_id = 'lean-parallel'
    j.call('start', '--id', j.run_id, 'lean-change', '@' + str(profile_path), 'parallel')
    j.append('intent-1', 'intent', parallel_intent())
    j.append('chk-1', 'intent-check', intent_check())
    j.event('intent-ready')
    j.append('d-ok', 'owner-decision', {'checkpoint': 'intent', 'subject': 'intent', 'subject_revision': 'intent-1',
                                      'decision': 'approve', 'text': 'Yes, both steps.', 'actor': OWNER})
    j.event('approved')
    assert j.state() == 'implement'

    # Every step starts from the same HEAD, before any landing, in a worktree outside the repo.
    for step in ['s1', 's2', 's3', 's4']:
        result = run_block(start, repo, steps, step)
        assert result.returncode == 0, (step, result.stderr)
        assert (steps / f'wt-{step}').is_dir() and not (steps / f'wt-{step}').resolve().is_relative_to(repo.resolve())
    build_together([(steps / 'wt-s1', 's1', 's2', 'one.txt', 'one: done\n'),
                    (steps / 'wt-s2', 's2', 's1', 'two.txt', 'two: finished\n')], starts)
    # Each step alone passes the gate (logged apart from the driver's gate log); only the joined tree fails it.
    for step in ['s1', 's2']:
        alone = subprocess.run(['sh', 'gate.sh'], cwd=steps / f'wt-{step}', env=dict(os.environ, GATE_LOG=str(work / 'step-gate.log')),
                               text=True, capture_output=True)
        assert alone.returncode == 0, (step, alone.stderr)
    # The third step edits the line s1 changed; the fourth adds a file the repository's pre-commit hook will reject.
    (steps / 'wt-s3' / 'one.txt').write_text('one: three\n')
    (steps / 'wt-s4' / 'four.txt').write_text('four\n')
    landed = []
    for step in ['s1', 's2']:
        result = run_block(land, repo, steps, step)
        assert result.returncode == 0 and 'conflict' not in result.stderr and 'not committed' not in result.stderr, (step, result.stderr)
        landed.append(git_out(repo, 'rev-parse', f'step/{step}').strip())
    head = git_out(repo, 'rev-parse', 'HEAD').strip()
    status = git_out(repo, 'status', '--porcelain', '--untracked-files=all')
    tree = j.verify()['tree']
    result = run_block(land, repo, steps, 's3')
    assert result.returncode != 0 and 'conflict: s3' in result.stderr, (result.returncode, result.stderr)
    assert git_out(repo, 'rev-parse', 'HEAD').strip() == head
    assert git_out(repo, 'status', '--porcelain', '--untracked-files=all') == status
    assert j.verify()['tree'] == tree
    assert (repo / 'one.txt').read_text() == 'one: done\n'

    # A pre-commit hook rejects s4's commit: the step does not land, and cleanup refuses to discard its work.
    hook = Path(git_out(repo, 'rev-parse', '--path-format=absolute', '--git-common-dir').strip()) / 'hooks' / 'pre-commit'
    hook.parent.mkdir(exist_ok=True)
    hook.write_text('#!/bin/sh\nif git diff --cached --name-only | grep -qx four.txt; then\n'
                    '  echo "pre-commit: four.txt is rejected" >&2\n  exit 1\nfi\n')
    hook.chmod(0o755)
    result = run_block(land, repo, steps, 's4')
    hook.unlink()
    assert result.returncode != 0 and 'not committed: s4' in result.stderr and 'conflict' not in result.stderr, (result.returncode, result.stderr)
    assert git_out(repo, 'rev-parse', 'HEAD').strip() == head
    assert git_out(repo, 'status', '--porcelain', '--untracked-files=all') == status
    assert j.verify()['tree'] == tree
    assert not (repo / 'four.txt').exists()
    result = run_block(clean, repo, steps, 's4')
    assert result.returncode != 0, result.stderr
    assert (steps / 'wt-s4' / 'four.txt').read_text() == 'four\n'
    # The driver decides to drop s4's work.
    git(repo, 'worktree', 'remove', '--force', str(steps / 'wt-s4'))
    git(repo, 'branch', '-q', '-D', 'step/s4')
    assert not (steps / 'wt-s4').exists()

    # The driver gates in the repository after the last landing; the builders never ran it. The joined tree fails.
    assert not gate_log.exists()
    gate_env = dict(os.environ, GATE_LOG=str(gate_log))
    branches = {step: git_out(repo, 'rev-parse', f'step/{step}').strip() for step in ['s1', 's2', 's3']}
    first = subprocess.run(['sh', 'gate.sh'], cwd=repo, env=gate_env, text=True, capture_output=True)
    assert first.returncode != 0 and 'disagree' in first.stderr, (first.returncode, first.stderr)
    for commit in landed:
        git(repo, 'merge-base', '--is-ancestor', commit, head)

    # The driver fixes the finding in the repository, commits it on the run branch, and gates again: it passes.
    (repo / 'two.txt').write_text('two: done\n')
    git(repo, 'commit', '-qam', 'fix gate finding: two.txt wording')
    fix = git_out(repo, 'rev-parse', 'HEAD').strip()
    assert git_out(repo, 'rev-parse', f'{fix}^@').split() == [head]
    second = subprocess.run(['sh', 'gate.sh'], cwd=repo, env=gate_env, text=True, capture_output=True)
    assert second.returncode == 0, second.stderr
    assert gate_log.read_text() == f'{repo.resolve()} {head}\n{repo.resolve()} {fix}\n', gate_log.read_text()
    # The fix touched no step worktree or branch: step/s2 still holds the original wording.
    assert {step: git_out(repo, 'rev-parse', f'step/{step}').strip() for step in ['s1', 's2', 's3']} == branches
    assert git_out(repo, 'show', 'step/s2:two.txt') == 'two: finished\n'
    assert (steps / 'wt-s2' / 'two.txt').read_text() == 'two: finished\n'
    assert subprocess.run(['git', 'merge-base', '--is-ancestor', fix, 'step/s2'], cwd=repo).returncode == 1

    for step in ['s1', 's2', 's3']:
        result = run_block(clean, repo, steps, step)
        assert result.returncode == 0, (step, result.stderr)
        assert not (steps / f'wt-{step}').exists()
    assert git_out(repo, 'branch', '--list', 'step/*') == ''
    assert git_out(repo, 'worktree', 'list', '--porcelain').count('worktree ') == 1

    j.event('ready')
    fresh = j.verify()
    assert [r['exit_code'] for r in fresh['results']] == [0, 0], fresh
    j.append('v-1', 'verification', fresh)
    j.event('verified')
    assert j.state() == 'review'
    packet = j.tool('review-packet', j.show_text())
    diff = packet.split('## Diff against base', 1)[1]
    assert '+one: done' in diff and '+two: done' in diff and 'one: three' not in diff and 'two: finished' not in diff, diff
    j.append('rev-1', 'review', review(fresh['tree'], False))
    j.event('reviewed')
    final = j.call('show', '--view', 'full', j.run_id)
    assert final['current_state'] == 'end' and final['lifecycle'] == 'final', final


def greet_criterion(criterion_id, statement, argv, old):
    return {'id': criterion_id, 'statement': statement, 'check': {'argv': argv, 'timeout_ms': 10000}, 'old': old}


def greet_intent(criteria, constraints):
    return {'author': WORKER, 'goal': OUTCOME_GOAL, 'constraints': constraints, 'hard': False, 'criteria': criteria}


# One criterion per label. 'app' passes on the old tree too, so its expectation does not match.
LABEL_CRITERIA = [
    greet_criterion('greets', 'app.sh greets Bob with Hello, Bob!', ['sh', 'journeys/greet.sh'], {'expect': 'fails'}),
    greet_criterion('readme', 'README.md is still there and the output does not change', ['sh', '-c', 'test -f README.md && echo readme-ok'],
                    {'expect': 'same'}),
    greet_criterion('tool', 'tools/new.sh is a new command', ['sh', 'tools/new.sh'], {'expect': 'absent'}),
    greet_criterion('app', 'app.sh is still there', ['test', '-f', 'app.sh'], {'expect': 'fails'}),
    greet_criterion('guard', 'the proof script is intact', ['test', '-f', 'journeys/greet.sh'], {'expect': 'none', 'reason': GUARD_REASON}),
    greet_criterion('release-notes', 'RELEASE.md is written', ['test', '-f', 'RELEASE.md'], {'expect': 'fails'}),
]
# The first two pass once app.sh says Hello, Bob!.
PASSING_CRITERIA = LABEL_CRITERIA[:2]
LABELS = {
    'greets': 'changed as intended: failed on the old code, passes now',
    'readme': 'kept: same output on the old and new code',
    'tool': 'new: the old code did not have this',
    'app': 'old code did not behave as expected: expected it to fail',
    'guard': f'not run on the old code: {GUARD_REASON}',
    'release-notes': 'fails on the new code',
}
# The 0.37.0 wording of the page: no section of this journey prints any of it.
OLD_WORDING = ['changed as intended (tested)', 'kept (same output before and after)', 'new: nothing before to compare',
               'old tree did not match', 'not compared:', 'failed now', 'unknown: not run', 'stale: ', 'verification current:',
               'verification stale', 'no verification for', 'verification unreadable', 'verification freshness unknown',
               '<b>Check:</b>', '<b>Current:</b>', '<b>Old tree</b>', '<b>Old run ', '<h3>Findings</h3>', '<b>Text:</b>',
               '<b>Consequence:</b>', '<b>Reason:</b>', 'Criteria:']
# The fields and headings the page prints where they apply: the outcome-map scenario's page must show each one.
PAGE_FIELDS = ['The approved goal, as written:', 'Command that checks it:', 'On the new code:', 'On the old code (expected: it fails):',
               'Old code, run 1:', 'Why the old code did not match:', 'Not run on the old code:',
               'Issues raised, and how each was settled', 'Issue:', 'Evidence:', 'Why it mattered:', 'How it was settled:']


def greet_run(engine, provider, work, profile, run_id):
    """A run on a repository with app.sh, journeys/greet.sh and README.md; the base commit holds all three."""
    j = Journey(engine, provider, work)
    repo = work / 'repo'
    repo.mkdir()
    git(repo, 'init', '-q')
    git(repo, 'config', 'user.email', 'j@example.invalid')
    git(repo, 'config', 'user.name', 'j')
    (repo / 'README.md').write_text('fixture\n')
    (repo / 'app.sh').write_text('printf "hello, %s\\n" "$1"\n')
    (repo / 'journeys').mkdir()
    (repo / 'journeys' / 'greet.sh').write_text(GREET_CHECK)
    git(repo, 'add', '-A')
    git(repo, 'commit', '-qm', 'base')
    principles = work / 'principles.md'
    principles.write_text(PRINCIPLES)
    setup = [str(provider), 'setup', '--profile', profile, '--repo', str(repo), '--proof-path', 'journeys']
    if profile == 'b':
        setup += ['--principles', str(principles)]
    profile_path = work / 'profile.json'
    profile_path.write_text(subprocess.run(setup, text=True, capture_output=True, check=True).stdout)
    j.run_id = run_id
    j.call('start', '--id', run_id, 'lean-change', '@' + str(profile_path), 'outcome map')
    assert j.state() == 'explore'
    return j, repo


def approve(j, intent_id):
    """The owner approves one intent record, and the run reaches verify."""
    j.append(f'owner-{intent_id}', 'owner-decision', {'checkpoint': 'intent', 'subject': 'intent', 'subject_revision': intent_id,
                                                  'decision': 'approve', 'text': f'Yes, {intent_id}.', 'actor': OWNER})
    j.event('approved')
    j.event('ready')
    assert j.state() == 'verify'


def two_findings_check():
    """An intent check with two findings: f1 on terms, f2 on edge."""
    check = intent_check(fail='terms')
    for answer in check['answers']:
        if answer['question'] == 'edge':
            answer['verdict'] = 'fail'
    check['findings'].append({'id': 'f2', 'question': 'edge', 'text': 'What if the name is empty?', 'evidence': 'The goal names no empty case.',
                              'consequence': 'A delivery would pick its own answer.'})
    return check


def labels_run(engine, provider, work):
    """Profile B with old-vs-new. Intent check f1 is rejected and f2 is fixed by a newer intent; review f1 stays open."""
    j, repo = greet_run(engine, provider, work, 'b', 'lean-outcome-map')
    j.append('intent-1', 'intent', greet_intent(LABEL_CRITERIA, ['Touch only app.sh and tools/.']))
    j.append('chk-1', 'intent-check', two_findings_check())
    j.append('disp-f1', 'finding-disposition', {'review': 'chk-1', 'finding': 'f1', 'disposition': 'rejected', 'reason': REJECT_REASON})
    j.append('disp-f2', 'finding-disposition', {'review': 'chk-1', 'finding': 'f2', 'disposition': 'fixed', 'reason': FIX_REASON})
    # The fix is intent-2. The check names intent-1, so intent-ready needs no new check.
    j.append('intent-2', 'intent', greet_intent(LABEL_CRITERIA, ['Touch only app.sh and tools/.', 'Say "Hello, Bob!" exactly.']))
    j.event('intent-ready')
    approve(j, 'intent-2')
    (repo / 'app.sh').write_text('printf "Hello, %s!\\n" "$1"\n')
    (repo / 'tools').mkdir()
    (repo / 'tools' / 'new.sh').write_text('echo new\n')
    fresh = j.verify()
    j.append('v-1', 'verification', fresh)
    open_review = review(fresh['tree'], True)
    open_review['intent'] = 'intent-2'
    j.append('rev-1', 'review', open_review)
    return j, repo, fresh


def outcome_map(provider, show, *args):
    """Runs lean-change outcome-map with the show text on stdin; stdout is kept as bytes."""
    return subprocess.run([str(provider), 'outcome-map', *args], input=show.encode(), capture_output=True)


def outcome_page(provider, show, *args):
    """The page the command prints; the command must succeed."""
    result = outcome_map(provider, show, *args)
    assert result.returncode == 0 and result.stdout, (args, result.returncode, result.stderr)
    return result.stdout.decode('utf-8')


def visible(markup):
    """The text of some markup: tags dropped, entities unescaped, white space collapsed."""
    return ' '.join(html.unescape(re.sub(r'<[^>]+>', ' ', markup)).split())


def section_of(page):
    """The evidence section, from its opening tag to its closing tag."""
    match = re.search(r'<section id="evidence".*?</section>', page, re.S)
    assert match, 'no evidence section'
    return match.group(0)


def first_paragraph(section):
    return visible(re.search(r'<p\b.*?</p>', section, re.S).group(0))


def rows(markup, cls):
    """The <details> elements of one class in markup, as {id: inner markup}."""
    found = {}
    for attrs, inner in re.findall(r'<details\b([^>]*)>(.*?)</details>', markup, re.S):
        if re.search(rf'class="[^"]*\b{cls}\b', attrs):
            found[re.search(r'\bid="([^"]*)"', attrs).group(1)] = inner
    return found


def labels(section):
    """Each criterion row's label, by criterion id."""
    return {cid: visible(re.search(r'<span class="om-label">(.*?)</span>', row, re.S).group(1))
            for cid, row in rows(section, 'om-criterion').items()}


def summary_text(row):
    return visible(re.search(r'<summary\b.*?</summary>', row, re.S).group(0))


def summary_of(section):
    """The summary paragraph of the section, as visible text."""
    return visible(re.search(r'<p class="om-summary">.*?</p>', section, re.S).group(0))


def old_wording_checks(section):
    """The section prints none of the 0.37.0 wording."""
    for old in OLD_WORDING:
        assert old not in section, old


def check_findings(section, show):
    """Each finding row shows its question, text, evidence and consequence, names its disposition and reason, and the summary links to it."""
    expected = {'chk-1-f1': ('rejected', REJECT_REASON), 'chk-1-f2': ('fixed', FIX_REASON), 'rev-1-f1': ('open', None)}
    findings = rows(section, 'om-finding')
    recorded = {f"{record['id']}-{finding['id']}": finding for record in json.loads(show)['result']['context']
                if record['kind'] in ('intent-check', 'review') for finding in record['data']['findings']}
    assert set(recorded) == set(expected), sorted(recorded)
    for finding, fields in recorded.items():
        text = visible(findings[finding])
        for field in ('question', 'text', 'evidence', 'consequence'):
            assert fields[field] in text, (finding, field, text)
    assert set(findings) == set(expected), sorted(findings)
    assert '<h3>Issues raised, and how each was settled</h3>' in section, 'no findings heading'
    for finding, (disposition, reason) in expected.items():
        for label in ('Issue:', 'Evidence:', 'Why it mattered:', 'How it was settled:'):
            assert label in visible(findings[finding]), (finding, label)
        assert re.search(rf'\b{disposition}\b', summary_text(findings[finding])), (finding, summary_text(findings[finding]))
        assert f'href="#{finding}"' in section, finding
        if reason:
            assert reason in visible(findings[finding]), finding


def self_contained_checks(page, goal, criteria):
    """The standalone page loads nothing from outside, and carries the goal and every criterion."""
    assert page.startswith('<!doctype html>') and '<title>Outcome map: ' in page, page[:200]
    for marker in ['src=', '<link', '@import', 'url(http', '<script']:
        assert marker not in page, marker
    text = html.unescape(page)
    assert goal in text and '<b>hi</b>' not in page, goal
    for criterion in criteria:
        assert 'id="%s"' % criterion['id'] in page and criterion['statement'] in text, criterion['id']


def into_checks(provider, show):
    """--into puts the section after AR's </header> and the rail link after the rail's opening div, and changes no other byte."""
    spliced = outcome_page(provider, show, '--into', str(AR_PAGE))
    section = section_of(spliced)
    page = AR_PAGE.read_bytes().decode('utf-8')
    assert spliced.replace(section, '', 1).replace(RAIL_LINK, '', 1) == page
    header_end = spliced.index('</header>', spliced.index('<main')) + len('</header>')
    assert spliced.startswith(section, header_end)
    rail_start = spliced.index('<div class="rail-links">') + len('<div class="rail-links">')
    assert spliced.startswith(RAIL_LINK, rail_start)


def missing_anchor_checks(provider, show, work):
    """Without <main>, its </header> or the rail div, --into exits 1 with nothing on stdout and names the part."""
    page = AR_PAGE.read_bytes()
    damaged = {'main': page.replace(b'<main>', b'', 1),
               'header': page.replace(b'</header>', b'', 1),
               'rail': page.replace(b'<div class="rail-links">', b'<div class="rail">', 1)}
    # A </header> after </main> is not the masthead's: the page still has no anchor inside <main>.
    main = page.index(b'<main')
    masthead_end = page.index(b'</header>', main)
    outside = page[:masthead_end] + page[masthead_end + len(b'</header>'):]
    damaged['header'] = outside + b'<footer><header>f</header></footer>\n'
    for name, content in damaged.items():
        path = work / f'ar-without-{name}.html'
        path.write_bytes(content)
        result = outcome_map(provider, show, '--into', str(path))
        assert result.returncode == 1 and result.stdout == b'' and name in result.stderr.decode('utf-8'), (name, result.returncode, result.stderr)


def refusal_checks(provider, show):
    """Input that is not a completed lean-change show, and an unknown argument, exit 1 with nothing on stdout."""
    other = json.loads(show)
    other['result']['workflow_id'] = 'policy-document'
    for stdin in ['{}', json.dumps(other)]:
        result = subprocess.run([str(provider), 'outcome-map'], input=stdin.encode(), capture_output=True)
        assert result.returncode == 1 and result.stdout == b'' and result.stderr, (stdin[:60], result.returncode, result.stderr)
    result = outcome_map(provider, show, '--bogus')
    assert result.returncode == 1 and result.stdout == b'', (result.returncode, result.stderr)


def status_unchanged_run(engine, provider, work):
    # bookends:LE-212 — the command leaves the repository as it was, and the next event is still accepted.
    j, repo = greet_run(engine, provider, work, 'c', 'lean-status')
    j.append('intent-1', 'intent', greet_intent(PASSING_CRITERIA, ['Touch only app.sh.']))
    j.event('intent-ready')
    approve(j, 'intent-1')
    (repo / 'app.sh').write_text('printf "Hello, %s!\\n" "$1"\n')
    j.append('v-1', 'verification', j.verify())
    before = git_out(repo, 'status', '--porcelain')
    assert before, 'the change should leave the repository dirty'
    outcome_page(provider, j.show_text())
    outcome_page(provider, j.show_text(), '--into', str(AR_PAGE))
    assert git_out(repo, 'status', '--porcelain') == before
    j.event('verified')
    assert j.state() == 'end'


def outcome_map_scenario(engine, provider, work):
    # bookends:LE-212 — labels, freshness, findings, --into, refusals and a profile C page.
    j, repo, fresh = labels_run(engine, provider, work)
    page = outcome_page(provider, j.show_text())
    self_contained_checks(page, OUTCOME_GOAL, LABEL_CRITERIA)
    section = section_of(page)
    assert first_paragraph(section).startswith('These results are current: the last run of the checks (v-1) used the code as it is now.'), first_paragraph(section)
    assert labels(section) == LABELS, labels(section)
    old_wording_checks(section)
    assert all(field in visible(section) for field in PAGE_FIELDS), [field for field in PAGE_FIELDS if field not in visible(section)]
    assert summary_of(section).startswith('6 checks were approved. 1 changed as intended: failed on the old code, passes now: greets'), summary_of(section)
    assert '3 issues were raised: 1 fixed, 1 rejected, 1 open.' in summary_of(section), summary_of(section)
    criteria = rows(section, 'om-criterion')
    # greets shows its check, the current pass and two old runs that print their output and exit 1.
    greets = criteria['greets']
    assert 'journeys/greet.sh' in visible(greets) and 'got: hello, Bob' in visible(greets), visible(greets)
    assert '<b>On the new code:</b> exit 0 (pass).' in greets, greets
    assert '<b>Old code, run 1:</b> exit 1 (fail).' in greets and '<b>Old code, run 2:</b> exit 1 (fail).' in greets, greets
    # release-notes has no RELEASE.md, so its current result is a fail.
    assert '<b>On the new code:</b> exit 1 (fail).' in criteria['release-notes'], criteria['release-notes']
    assert 'readme-ok' in visible(criteria['readme']) and 'RELEASE.md' in visible(criteria['release-notes'])
    check_findings(section, j.show_text())
    # An untracked file makes the tree differ from the verified one: every row with a result is stale.
    (repo / 'notes.txt').write_text('late edit\n')
    stale = section_of(outcome_page(provider, j.show_text()))
    assert first_paragraph(stale).startswith('These results are out of date: the code changed after the last run of the checks (v-1).'), first_paragraph(stale)
    assert labels(stale) == {name: f'out of date: {label}' for name, label in LABELS.items()}, labels(stale)
    old_wording_checks(stale)
    (repo / 'notes.txt').unlink()
    assert first_paragraph(section_of(outcome_page(provider, j.show_text()))).startswith('These results are current: the last run of the checks (v-1)')
    # A second verification without the tool's result, and with another readme check hash: both rows are unknown.
    partial = json.loads(json.dumps(fresh))
    partial['results'] = [r for r in partial['results'] if r['criterion'] != 'tool']
    for result in partial['results']:
        if result['criterion'] == 'readme':
            result['check_sha256'] = '0' * 64
    j.append('v-2', 'verification', partial)
    unknown = section_of(outcome_page(provider, j.show_text()))
    assert first_paragraph(unknown).startswith('These results are current: the last run of the checks (v-2)'), first_paragraph(unknown)
    assert labels(unknown) == dict(LABELS, tool='not run yet', readme='not run yet'), labels(unknown)
    old_wording_checks(unknown)
    # The readme result for another check still shows what ran: its current exit and output, and its old run.
    readme = visible(rows(unknown, 'om-criterion')['readme'])
    recorded = next(r for r in partial['results'] if r['criterion'] == 'readme')
    assert 'a different check command' in readme and recorded['output_tail'].strip() in readme, readme
    for run in recorded['old']['runs']:
        assert run['output_tail'].strip() in readme, (run, readme)
    assert 'Old code, run 1:' in readme, readme
    check_findings(unknown, j.show_text())
    # Stale and recorded for another check: readme has a result, so it is stale too; tool has none, so it is not.
    (repo / 'notes.txt').write_text('late edit\n')
    stale_unknown = section_of(outcome_page(provider, j.show_text()))
    (repo / 'notes.txt').unlink()
    assert first_paragraph(stale_unknown).startswith('These results are out of date: the code changed after the last run of the checks (v-2).'), first_paragraph(stale_unknown)
    assert labels(stale_unknown) == dict({name: f'out of date: {label}' for name, label in LABELS.items()},
                                         tool='not run yet', readme='out of date: not run yet'), labels(stale_unknown)
    # A latest verification that cannot be read is not replaced by the older one: every row is unknown.
    broken = json.loads(j.show_text())
    context = broken['result']['context']
    context.append({'id': 'v-bad', 'kind': 'verification', 'sequence': max(r['sequence'] for r in context) + 1,
                    'data': {'intent': 'intent-2', 'tree': fresh['tree'], 'results': None}})
    unreadable = section_of(outcome_page(provider, json.dumps(broken)))
    assert first_paragraph(unreadable).startswith('The last run of the checks (v-bad) could not be read: '), first_paragraph(unreadable)
    assert set(labels(unreadable).values()) == {'not run yet'}, labels(unreadable)
    old_wording_checks(unreadable)
    show = j.show_text()
    into_checks(provider, show)
    missing_anchor_checks(provider, show, work)
    refusal_checks(provider, show)
    # A profile C run has no review and no intent check, so its page has no findings part.
    (work / 'profile-c').mkdir()
    c_page = outcome_page(provider, run_profile(engine, provider, 'c', work / 'profile-c').show_text())
    assert set(labels(section_of(c_page))) == {'says-hello', 'exists'}, labels(section_of(c_page))
    assert 'om-finding' not in re.sub(r'<style\b.*?</style>', '', c_page, flags=re.S) and 'Issues raised' not in c_page
    (work / 'status').mkdir()
    status_unchanged_run(engine, provider, work / 'status')


def disposition_reason(context, review_id, finding):
    """The reason of the latest finding-disposition record for one finding of one review or intent check."""
    records = [r for r in context if r['kind'] == 'finding-disposition' and r['data']['review'] == review_id and r['data']['finding'] == finding]
    return max(records, key=lambda r: r['sequence'])['data']['reason']


def fixture_show(repo):
    """The saved le-parallel-1 show, with its repository moved to repo."""
    show = json.loads(SHOW_FIXTURE.read_text())
    show['result']['initial_input']['repo'] = str(repo)
    return show


def outcome_map_fixture_scenario(provider, work):
    # bookends:LE-212 — the saved LE-211 show with no repository: freshness unknown, labels unprefixed, fixed findings, AR's sections kept.
    show = fixture_show(work / 'gone' / 'repo')
    page = outcome_page(provider, json.dumps(show), '--into', str(AR_PAGE))
    section = section_of(page)
    context = show['result']['context']
    latest = max((r for r in context if r['kind'] == 'intent'), key=lambda r: r['sequence'])
    c2_reason = next(c for c in latest['data']['criteria'] if c['id'] == 'c2')['old']['reason']
    assert first_paragraph(section).startswith('It is unknown whether these results are current: '), first_paragraph(section)
    assert labels(section) == {'c1': 'changed as intended: failed on the old code, passes now', 'c2': f'not run on the old code: {c2_reason}',
                               'c3': 'changed as intended: failed on the old code, passes now'}, labels(section)
    findings = rows(section, 'om-finding')
    for finding, review_id in [('intent-check-1-f1', 'intent-check-1'), ('review-1-f1', 'review-1')]:
        reason = disposition_reason(context, review_id, 'f1')
        assert re.search(r'\bfixed\b', summary_text(findings[finding])) and reason in visible(findings[finding]), (finding, reason)
    for name in ['changed', 'context', 'attention', 'start', 'impact']:
        assert 'id="%s"' % name in page, name


def blank_external_links(markup):
    """Anchors with an http(s) href and target="_blank", whatever the attribute order."""
    return sum(1 for tag in re.findall(r'<a\b[^>]*>', markup) if re.search(r'\bhref="https?://', tag) and 'target="_blank"' in tag)


def dump_dom(chromium, path):
    """The DOM headless Chromium prints for a file after its scripts run.

    On a loaded machine Chromium can print the whole DOM and then fail to shut down ("Teardown watchdog expired",
    exit 2, or no exit before the timeout). The DOM is what the check reads, so a complete DOM counts whatever the exit.
    """
    command = [chromium, '--headless=new', '--disable-gpu', '--virtual-time-budget=3000', '--dump-dom', 'file://' + str(path)]
    try:
        done = subprocess.run(command, capture_output=True, timeout=120)
        out, err, ended = done.stdout, done.stderr, f'exit {done.returncode}'
    except subprocess.TimeoutExpired as expired:
        out, err, ended = expired.stdout or b'', expired.stderr or b'', 'timed out'
    dom = out.decode('utf-8')
    assert dom.rstrip().endswith('</html>'), (ended, err.decode('utf-8', 'replace')[-2000:], dom[-200:])
    return dom


def outcome_map_browser_scenario(provider, chromium, work):
    # bookends:LE-212 — headless Chromium opens the spliced page from file://: the section comes first and AR's script runs.
    page = outcome_page(provider, json.dumps(fixture_show(work / 'gone' / 'repo')), '--into', str(AR_PAGE))
    spliced = work / 'spliced.html'
    spliced.write_bytes(page.encode('utf-8'))
    dumped = dump_dom(chromium, spliced)
    assert dumped.index('id="evidence"') < dumped.index('id="changed"')
    tags = re.findall(r'<([a-zA-Z0-9]+)\b[^>]*\bclass="[^"]*\bom-criterion\b', dumped)
    assert tags and set(tags) == {'details'}, tags
    assert 'href="#evidence"' in dumped
    assert blank_external_links(dumped) > blank_external_links(page), (blank_external_links(dumped), blank_external_links(page))


def outcome_map_skill_scenario(engine, provider, skill, work):
    # bookends:LE-212 — the skill's Outcome map blocks, run as written with stub open and curl.
    default, link = section_blocks(skill, OUTCOME_SECTION, 2)
    text = '\n'.join(skill_section(skill, OUTCOME_SECTION))
    for needle in [REVIEW_MAP, 'status --porcelain', '--output "$WORK/ar"']:
        assert needle in text, needle
    run_dir = work / 'run'
    run_dir.mkdir()
    finished = run_profile(engine, provider, 'c', run_dir)
    repo = run_dir / 'repo'
    git(repo, 'add', '-A')
    git(repo, 'commit', '-qm', 'change')
    show = finished.show_text()
    (work / 'show-full.json').write_text(show)
    stubs = work / 'bin'
    stubs.mkdir()
    (stubs / 'lean-change').symlink_to(provider)
    for name in ['open', 'curl']:
        stub = stubs / name
        # curl also saves the request body and answers like shartifacts: {"id": ...}.
        body = 'cat > "$WORK/curl.body"\nprintf \'{"id":"abc123"}\'\n' if name == 'curl' else ''
        stub.write_text('#!/bin/sh\nprintf "%s\\n" "$*" >> "$WORK/' + name + '.log"\n' + body)
        stub.chmod(0o755)
    path = os.environ['PATH']
    env = dict(os.environ, REPO=str(repo), WORK=str(work), PATH=f'{stubs}{os.pathsep}{path}')
    out = work / 'outcome-map.html'
    ar_index = work / 'ar' / 'index.html'

    def run_sh(block):
        for name in ['open', 'curl']:
            (work / f'{name}.log').unlink(missing_ok=True)
        result = subprocess.run(['sh', '-c', block], env=env, text=True, capture_output=True)
        assert result.returncode == 0, (result.returncode, result.stderr)
        return result.stdout

    def log(name):
        log_path = work / f'{name}.log'
        return log_path.read_text() if log_path.exists() else ''

    def written():
        return out.read_bytes().decode('utf-8')

    # AR's page and a clean repository: the default block passes --into.
    ar_index.parent.mkdir()
    ar_index.write_bytes(AR_PAGE.read_bytes())
    assert git_out(repo, 'status', '--porcelain') == ''
    expected = outcome_page(provider, show, '--into', str(ar_index))
    run_sh(default)
    assert written() == expected and log('open') == f'{out}\n' and log('curl') == '', (log('open'), log('curl'))
    # No AR page: the standalone page.
    shutil.rmtree(work / 'ar')
    expected = outcome_page(provider, show)
    run_sh(default)
    assert written() == expected and log('open') == f'{out}\n' and log('curl') == ''
    # AR's page again, but an untracked file: the standalone page.
    ar_index.parent.mkdir()
    ar_index.write_bytes(AR_PAGE.read_bytes())
    (repo / 'untracked.txt').write_text('uncommitted\n')
    expected = outcome_page(provider, show)
    run_sh(default)
    assert written() == expected and log('open') == f'{out}\n' and log('curl') == ''
    # The link block posts the page to shartifacts as an html artifact and prints its /view link.
    printed = run_sh(link)
    base = 'https://shartifacts.internal.cartwmic.com'
    assert printed == f'{base}/view/abc123\n', printed
    assert log('open') == '', log('open')
    assert log('curl').split() == ['-sS', '--fail-with-body', '-X', 'POST', '-H', 'Content-Type:', 'application/json', '--data-binary', '@-', f'{base}/api/artifacts'], log('curl')
    sent = json.loads((work / 'curl.body').read_text())
    assert sent == {'title': 'Outcome map', 'kind': 'html', 'content': written()}, sent


def v1_run_to_review(engine, provider, work, run_id):
    """Profile b with a v1 intent check whose only finding is a note; the run stops in review with a current verification."""
    j, repo = greet_run(engine, provider, work, 'b', run_id)
    j.append('intent-1', 'intent', greet_intent(LABEL_CRITERIA[:2], ['Touch only app.sh.']))
    packet = j.tool('intent-packet', j.show_text())
    assert '"severity": "blocking|note"' in packet and '"findings_schema": 1' in packet, packet
    check = intent_check(fail='terms')
    check['findings_schema'] = 1
    check['findings'] = [{'id': 'f1', 'severity': 'note', 'question': 'terms', 'text': 'Which greeting text do you want?'}]
    j.append('chk-1', 'intent-check', check)
    j.event('intent-ready')  # a note needs no disposition
    approve(j, 'intent-1')
    (repo / 'app.sh').write_text('printf "Hello, %s!\\n" "$1"\n')
    fresh = j.verify()
    j.append('v-1', 'verification', fresh)
    j.event('verified')
    assert j.state() == 'review'
    return j, repo, fresh


def keep_record(j, keep, name, record_id):
    """Keeps the review exactly as the run holds it: the record the engine stored, with its envelope."""
    if keep is None:
        return
    record = next(r for r in j.call('show', '--view', 'full', j.run_id)['context'] if r['id'] == record_id)
    keep.mkdir(parents=True, exist_ok=True)
    (keep / f'{name}.json').write_text(json.dumps(record['data'], indent=2) + '\n')
    (keep / f'{name}-record.json').write_text(json.dumps(record, indent=2) + '\n')
    print(f'lean-change journey kept {keep / (name + ".json")}')


def findings_v1_scenario(engine, provider, work, keep):
    # bookends:LE-207 bookends:LE-209 bookends:LE-212 — findings_schema 1: severity, notes, demotion, the seventh question,
    # a changed tree after a review of notes alone, and the page that shows notes apart.
    first = work / 'blocking'
    first.mkdir()
    j, repo, fresh = v1_run_to_review(engine, provider, first, 'lean-findings-v1')
    packet = j.tool('review-packet', j.show_text())
    for needle in ['severity', 'Do not invent', 'If the intent and the checks settle a question, pass it.', 'smaller-design',
                   'Pass every question if nothing material remains.', 'names its component']:
        assert needle in packet, needle
    sentence = re.search(r'If the intent and the checks settle a question.*?remains\.', packet).group(0)
    print(f'lean-change journey review-packet says: {sentence}')
    j.append('rev-1', 'review', v1_review(fresh['tree'], ['principles', 'scope', 'docs'], v1_findings()))
    out = j.refused('reviewed', 'review-incomplete', 'review')
    # Only the complete blocking finding needs a disposition; the one missing its consequence and the note do not.
    assert 'finding f1 needs a disposition' in out and 'f2' not in out and 'n1' not in out, out
    print(f'lean-change journey reviewed refused: {re.search(r"finding f1 needs a disposition[^\"]*", out).group(0)}')
    page = section_of(outcome_page(provider, j.show_text()))
    notes_heading = 'Notes: not blocking, no response needed'
    assert notes_heading in page, page
    before, after = page.split(notes_heading)
    assert 'id="rev-1-f1"' in before and 'id="rev-1-f2"' not in before and 'id="rev-1-n1"' not in before, page
    assert 'id="rev-1-f2"' in after and 'id="rev-1-n1"' in after and 'id="rev-1-f1"' not in after, page
    found = rows(page, 'om-finding')
    assert 'note (demoted: missing consequence)' in summary_text(found['rev-1-f2']), summary_text(found['rev-1-f2'])
    assert summary_text(found['rev-1-n1']).endswith(' note'), summary_text(found['rev-1-n1'])
    assert 'Part of the system: greeting file' in visible(found['rev-1-f1']), visible(found['rev-1-f1'])
    assert '1 issue was raised: 0 fixed, 0 rejected, 1 open.' in summary_of(page) and '3 notes were left that need no response' in summary_of(page), summary_of(page)
    print(f'lean-change journey outcome-map: {summary_of(page)[summary_of(page).index("1 issue"):]}')
    state = visible(re.search(r'<span class="om-disposition">(.*?)</span>', found['rev-1-f2'], re.S).group(1))
    print(f'lean-change journey outcome-map shows "{notes_heading}" with rev-1-f2 as "{state}"')
    j.append('disp-f1', 'finding-disposition', {'review': 'rev-1', 'finding': 'f1', 'disposition': 'rejected',
                                                'reason': 'The greeting is fixed by the intent.'})
    j.event('reviewed')
    assert j.state() == 'end'
    print('lean-change journey reviewed granted once f1 was disposed')
    keep_record(j, keep, 'v1-review', 'rev-1')

    # A review of notes alone, then a change to the tree: accepted after the new verification, no fixed finding needed.
    second = work / 'notes-only'
    second.mkdir()
    j, repo, fresh = v1_run_to_review(engine, provider, second, 'lean-findings-v1-notes')
    notes = [{'id': 'n1', 'severity': 'note', 'question': 'docs', 'text': 'The greeting could be mentioned in a README.'}]
    # A review answers seven questions: a v1 review with six is not well-formed, and the first well-formed one counts.
    short = v1_review(fresh['tree'], ['docs'], notes, 'six-answer-reviewer')
    short['answers'].pop()
    j.append('rev-six', 'review', short)
    out = j.refused('reviewed', 'review-incomplete', 'review')
    assert 'smaller-design is unanswered' in out, out
    j.append('rev-1', 'review', v1_review(fresh['tree'], ['docs'], notes))
    (repo / 'notes.txt').write_text('late edit\n')
    out = j.refused('reviewed', 'verification-incomplete', 'review')
    assert 'stale' in out, out
    j.append('v-2', 'verification', j.verify())
    j.event('reviewed')
    assert j.state() == 'end'
    print('lean-change journey notes-only review accepted after a tree change and re-verification')


def delegation_example(skill):
    """The first ```json block of the skill that carries a delegation, as a decision; the scenario appends it verbatim."""
    lines = skill.read_text().splitlines()
    block, indent = None, 0
    for line in lines:
        if block is None and line.strip() == '```json':
            block, indent = [], len(line) - len(line.lstrip())
        elif block is not None and line.strip() == '```':
            text = '\n'.join(block)
            if '"delegation"' in text:
                return checked_example(skill, text)
            block = None
        elif block is not None:
            block.append(line[indent:])
    raise SystemExit(f'{skill}: no ```json block with "delegation"')


def checked_example(skill, text):
    """The example must fit this scenario: an intent approval whose covers name the criteria the scenario builds."""
    try:
        decision = json.loads(text)
    except ValueError as error:
        raise SystemExit(f'{skill}: the delegation example is not valid JSON: {error}')
    delegation = decision.get('delegation') if isinstance(decision, dict) else None
    if not isinstance(delegation, dict) or (decision.get('checkpoint'), decision.get('subject'), decision.get('decision')) != ('intent', 'intent', 'approve'):
        raise SystemExit(f'{skill}: the delegation example is not an intent approval with a delegation object')
    covers, criteria = delegation.get('covers'), len(intent()['criteria'])
    if not isinstance(covers, list) or not all(isinstance(c, str) for c in covers) or len(set(covers)) != criteria:
        raise SystemExit(f'{skill}: the delegation example covers {covers!r}, but the scenario intent has {criteria} criteria')
    if not isinstance(decision.get('subject_revision'), str):
        raise SystemExit(f'{skill}: the delegation example has no subject_revision')
    return decision


def delegated_intent(covers):
    """The scenario's intent, with its two criteria named by the ids the delegation covers."""
    doc = intent()
    for criterion, criterion_id in zip(doc['criteria'], covers):
        criterion['id'] = criterion_id
    return doc


def changed_intent(covers):
    """The same intent with the goal changed, so the approved outcome differs."""
    doc = delegated_intent(covers)
    doc['goal'] = doc['goal'] + ', twice'
    return doc


def reworded_intent(covers):
    """The same intent with one criterion statement reworded: the outcome is the same, the wording is not."""
    doc = delegated_intent(covers)
    doc['criteria'][0]['statement'] = 'greeting.txt says hello, twice'
    return doc


def delegated_decision(example, revision, covers=None):
    """The example decision aimed at another intent revision, with its covers replaced when given."""
    decision = json.loads(json.dumps(example))
    decision['subject_revision'] = revision
    decision['text'] = f'Lead statement for {revision}.'
    if covers is not None:
        decision['delegation']['covers'] = covers
    return decision


def plain_run(engine, provider, work, run_id):
    """A run on a repository with only README.md and profile c, set up as run_profile sets it up; the run starts in explore."""
    work.mkdir(parents=True)
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
    j.run_id = run_id
    j.call('start', '--id', run_id, 'lean-change', '@' + str(profile_path), 'delegated')
    assert j.state() == 'explore'
    return j, repo


def delegated_scenario(engine, provider, skill, work):
    # bookends:LE-204: a delegation is judged on its covers, and a delegated approval never changes an approved outcome.
    example = delegation_example(skill)
    revision, covers = example['subject_revision'], example['delegation']['covers']

    # Covers must match the criteria exactly: a missing id and an extra id are each refused, and the run stays in intent-approval.
    j, repo = plain_run(engine, provider, work / 'end', 'lean-delegated-end')
    j.append(revision, 'intent', delegated_intent(covers))
    j.event('intent-ready')
    j.append('d-missing', 'owner-decision', delegated_decision(example, revision, covers[:1]))
    j.refused('approved', 'delegation-not-covering', 'intent-approval')
    j.append('d-extra', 'owner-decision', delegated_decision(example, revision, covers + ['extra']))
    j.refused('approved', 'delegation-not-covering', 'intent-approval')
    j.append('d-ok', 'owner-decision', example)
    j.event('approved')
    assert j.state() == 'implement'
    j.event('ready')
    (repo / 'greeting.txt').write_text('hello\n')
    fresh = j.verify()
    assert {r['criterion'] for r in fresh['results']} == set(covers), fresh
    j.append('v-1', 'verification', fresh)
    j.event('verified')
    final = j.call('show', '--view', 'full', j.run_id)
    assert final['current_state'] == 'end' and final['lifecycle'] == 'final', final

    # The delegated approval of intent-1 is the baseline. intent-2 changes a criterion and its delegation is refused during implement.
    # intent-3, the same as intent-2, is refused too: the refused approval of intent-2 is not a baseline.
    j, repo = plain_run(engine, provider, work / 'revised', 'lean-delegated-revised')
    j.append(revision, 'intent', delegated_intent(covers))
    j.event('intent-ready')
    j.append('d-ok', 'owner-decision', example)
    j.event('approved')
    assert j.state() == 'implement'
    j.append('intent-2', 'intent', changed_intent(covers))
    j.append('d-2', 'owner-decision', delegated_decision(example, 'intent-2'))
    j.event('ready')
    (repo / 'greeting.txt').write_text('hello\n')
    j.append('v-2', 'verification', j.verify())
    j.refused('verified', 'delegation-changes-approved-outcome', 'verify')
    j.append('intent-3', 'intent', changed_intent(covers))
    j.append('d-3', 'owner-decision', delegated_decision(example, 'intent-3'))
    j.append('v-3', 'verification', j.verify())
    j.refused('verified', 'delegation-changes-approved-outcome', 'verify')
    # The owner's own words need no delegation, so they make intent-3 the baseline and let the run finish.
    j.append('d-own-3', 'owner-decision', {'checkpoint': 'intent', 'subject': 'intent', 'subject_revision': 'intent-3',
                                       'decision': 'approve', 'text': 'Yes, say hello twice.', 'actor': OWNER})
    j.append('v-4', 'verification', j.verify())
    j.event('verified')
    final = j.call('show', '--view', 'full', j.run_id)
    assert final['current_state'] == 'end' and final['lifecycle'] == 'final', final

    # A delegation cannot approve a changed outcome at the approval event either. The owner's words approve intent-1, then revise goes back to explore.
    j, _ = plain_run(engine, provider, work / 'approval', 'lean-delegated-approval')
    j.append(revision, 'intent', delegated_intent(covers))
    j.event('intent-ready')
    j.append('d-own', 'owner-decision', {'checkpoint': 'intent', 'subject': 'intent', 'subject_revision': revision,
                                      'decision': 'approve', 'text': 'Yes, add the greeting.', 'actor': OWNER})
    j.event('approved')
    assert j.state() == 'implement'
    j.event('revise')
    assert j.state() == 'explore'
    j.append('intent-2', 'intent', changed_intent(covers))
    j.event('intent-ready')
    assert j.state() == 'intent-approval'
    j.append('d-2', 'owner-decision', delegated_decision(example, 'intent-2'))
    j.refused('approved', 'delegation-changes-approved-outcome', 'intent-approval')

    # A delegation may reword a statement only as its rewrites declare. Without them the approval is refused; with
    # them it goes through, and the same intent is then judged against the reworded text.
    j, _ = plain_run(engine, provider, work / 'rewrite', 'lean-delegated-rewrite')
    j.append(revision, 'intent', delegated_intent(covers))
    j.event('intent-ready')
    j.append('d-own', 'owner-decision', {'checkpoint': 'intent', 'subject': 'intent', 'subject_revision': revision,
                                      'decision': 'approve', 'text': 'Yes, add the greeting.', 'actor': OWNER})
    j.event('approved')
    j.event('revise')
    reworded = reworded_intent(covers)
    j.append('intent-2', 'intent', reworded)
    j.event('intent-ready')
    j.append('d-2', 'owner-decision', delegated_decision(example, 'intent-2'))
    j.refused('approved', 'delegation-rewrite-mismatch', 'intent-approval')
    wrong = delegated_decision(example, 'intent-2')
    wrong['delegation']['rewrites'] = [{'criterion': covers[0], 'before': 'not the old text', 'after': reworded['criteria'][0]['statement'], 'reason': 'wording'}]
    j.append('d-3', 'owner-decision', wrong)
    j.refused('approved', 'delegation-rewrite-mismatch', 'intent-approval')
    right = delegated_decision(example, 'intent-2')
    right['delegation']['rewrites'] = [{'criterion': covers[0], 'before': delegated_intent(covers)['criteria'][0]['statement'],
                                        'after': reworded['criteria'][0]['statement'], 'reason': 'wording only'}]
    j.append('d-4', 'owner-decision', right)
    j.event('approved')
    assert j.state() == 'implement'


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--engine')
    parser.add_argument('--provider')
    parser.add_argument('--skill', default=str(DEFAULT_SKILL))
    parser.add_argument('--only', choices=SCENARIOS, help='run one scenario')
    parser.add_argument('--chromium', help='headless Chromium for outcome-map-browser; without it that scenario is skipped')
    parser.add_argument('--keep-records', help='directory to keep the findings-v1 review record in (the record as appended, and as the run stores it)')
    parser.add_argument('--self-test', action='store_true')
    args = parser.parse_args()
    # Bare invocation is the recognized CI collection form; full proof uses the parameterized run below.
    if args.self_test or len(sys.argv) == 1:
        assert [q for q in QUESTIONS] == sorted(set(QUESTIONS), key=QUESTIONS.index)
        assert len(intent()['criteria']) == 2 and review('t', True)['findings'] and intent_check(fail='terms')['findings']
        assert len(v1_review('t', [], v1_findings())['answers']) == 7 and len(review('t', False)['answers']) == 6
        assert [f.get('severity') for f in v1_findings()] == ['blocking', 'blocking', 'note'] and 'consequence' not in v1_findings()[1]
        assert 'delegated' in SCENARIOS
        print('lean-change journey self-test passed')
        return
    for name in ['engine', 'provider']:
        if not getattr(args, name):
            parser.error(f'--{name} is required')
    engine, provider, skill = Path(args.engine).resolve(), Path(args.provider).resolve(), Path(args.skill)
    scenarios = [
        ('b', lambda w: run_profile(engine, provider, 'b', w)),
        ('c', lambda w: run_profile(engine, provider, 'c', w)),
        ('hard', lambda w: hard_run(engine, provider, w)),
        ('old-vs-new', lambda w: old_vs_new_run(engine, provider, w)),
        ('parallel', lambda w: parallel_run(engine, provider, parallel_blocks(skill), w)),
        ('outcome-map', lambda w: outcome_map_scenario(engine, provider, w)),
        ('outcome-map-skill', lambda w: outcome_map_skill_scenario(engine, provider, skill, w)),
        ('outcome-map-fixture', lambda w: outcome_map_fixture_scenario(provider, w)),
        ('outcome-map-browser', lambda w: outcome_map_browser_scenario(provider, args.chromium, w)),
        ('findings-v1', lambda w: findings_v1_scenario(engine, provider, w, Path(args.keep_records).resolve() if args.keep_records else None)),
        ('delegated', lambda w: delegated_scenario(engine, provider, skill, w)),
    ]
    for name, action in scenarios:
        if args.only not in (None, name):
            continue
        if name == 'outcome-map-browser' and not args.chromium:
            print('lean-change journey outcome-map-browser skipped (no --chromium)')
            continue
        with tempfile.TemporaryDirectory(prefix=f'lean-change-{name}-') as temporary:
            action(Path(temporary))
        print(f'lean-change journey {name} passed')


if __name__ == '__main__':
    main()
