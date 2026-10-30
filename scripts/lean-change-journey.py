#!/usr/bin/env python3
"""Lean-change journey through the real engine CLI: a scripted dummy worker drives profile B (review) and C (no review) to completion."""
import argparse
import json
import os
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
CHECK_QUESTIONS = ['use', 'edge', 'control', 'terms', 'criteria-gap', 'principles-gap']
OWNER = {'name': 'owner', 'kind': 'human'}
WORKER = {'name': 'worker', 'kind': 'script'}
REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_SKILL = REPO_ROOT / 'crates/lean-change-provider/skills/using-lean-change-provider/SKILL.md'
PARALLEL_SECTION = '## Parallel builders'
GATE = 'printf "%s %s\\n" "$(pwd -P)" "$(git rev-parse HEAD)" >> "$GATE_LOG"\n'
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


def parallel_blocks(skill):
    """The skill's 'Parallel builders' sh blocks, in order: start a step, land a step, clean up a step."""
    lines = skill.read_text().splitlines()
    if PARALLEL_SECTION not in lines:
        raise SystemExit(f'{skill}: no "{PARALLEL_SECTION}" section')
    section = []
    for line in lines[lines.index(PARALLEL_SECTION) + 1:]:
        if line.startswith('## '):
            break
        section.append(line)
    blocks, block, indent = [], None, 0
    for line in section:
        if block is None and line.strip() == '```sh':
            block, indent = [], len(line) - len(line.lstrip())
        elif block is not None and line.strip() == '```':
            blocks.append('\n'.join(block) + '\n')
            block = None
        elif block is not None:
            block.append(line[indent:])
    if len(blocks) != 3:
        raise SystemExit(f'{skill}: "{PARALLEL_SECTION}" has {len(blocks)} sh blocks, expected three (start, land, clean up)')
    return blocks


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
    """Two steps built at once in worktrees outside the repo, landed and gated once; a third, conflicting step stops at landing."""
    # bookends:LE-211 — the skill's Parallel builders blocks, run as written, build two steps at once, land them, gate once in the repo, and stop a conflicting step at landing.
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
    for step in ['s1', 's2', 's3']:
        result = run_block(start, repo, steps, step)
        assert result.returncode == 0, (step, result.stderr)
        assert (steps / f'wt-{step}').is_dir() and not (steps / f'wt-{step}').resolve().is_relative_to(repo.resolve())
    build_together([(steps / 'wt-s1', 's1', 's2', 'one.txt', 'one: done\n'),
                    (steps / 'wt-s2', 's2', 's1', 'two.txt', 'two: done\n')], starts)
    # The third step edits the line s1 changed.
    (steps / 'wt-s3' / 'one.txt').write_text('one: three\n')
    landed = []
    for step in ['s1', 's2']:
        result = run_block(land, repo, steps, step)
        assert result.returncode == 0 and 'conflict' not in result.stderr, (step, result.stderr)
        landed.append(git_out(repo, 'rev-parse', f'step/{step}').strip())
    head = git_out(repo, 'rev-parse', 'HEAD').strip()
    status = git_out(repo, 'status', '--porcelain', '--untracked-files=all')
    tree = j.verify()['tree']
    result = run_block(land, repo, steps, 's3')
    assert 'conflict: s3' in result.stderr, (result.returncode, result.stderr)
    assert git_out(repo, 'rev-parse', 'HEAD').strip() == head
    assert git_out(repo, 'status', '--porcelain', '--untracked-files=all') == status
    assert j.verify()['tree'] == tree
    assert (repo / 'one.txt').read_text() == 'one: done\n'

    # The driver gates once, in the repository, after the last landing; the builders never ran it.
    assert not gate_log.exists()
    subprocess.run(['sh', 'gate.sh'], cwd=repo, env=dict(os.environ, GATE_LOG=str(gate_log)), check=True)
    assert gate_log.read_text() == f'{repo.resolve()} {head}\n', gate_log.read_text()
    for commit in landed:
        git(repo, 'merge-base', '--is-ancestor', commit, head)

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
    assert '+one: done' in diff and '+two: done' in diff and 'one: three' not in diff, diff
    j.append('rev-1', 'review', review(fresh['tree'], False))
    j.event('reviewed')
    final = j.call('show', '--view', 'full', j.run_id)
    assert final['current_state'] == 'end' and final['lifecycle'] == 'final', final


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--engine')
    parser.add_argument('--provider')
    parser.add_argument('--skill', default=str(DEFAULT_SKILL))
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
    blocks = parallel_blocks(Path(args.skill))
    for name, action in [('b', lambda w: run_profile(engine, provider, 'b', w)),
                         ('c', lambda w: run_profile(engine, provider, 'c', w)),
                         ('hard', lambda w: hard_run(engine, provider, w)),
                         ('old-vs-new', lambda w: old_vs_new_run(engine, provider, w)),
                         ('parallel', lambda w: parallel_run(engine, provider, blocks, w))]:
        with tempfile.TemporaryDirectory(prefix=f'lean-change-{name}-') as temporary:
            action(Path(temporary))
        print(f'lean-change journey {name} passed')


if __name__ == '__main__':
    main()
