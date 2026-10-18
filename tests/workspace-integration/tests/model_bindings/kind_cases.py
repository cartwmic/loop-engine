"""S3: a declaration per return kind in a run started with bindings.

Ordinary-reviewer, cold-reader and challenge-reviewer returns go through the real
review-candidates intake and the real gates; criterion and goal verdicts go through the
real validation preparation, with the actual criterion/goal batches of the shipped graph.
"""
import copy

from harness import ADMISSION_RULE, binding, require, session_refusal_needles, variants

ORDINARY = binding('provider/ordinary-model', 'high')
CHALLENGE = binding('provider/challenge-model', 'xhigh')
COLD = binding('provider/cold-model', 'medium')
CRITERION = binding('provider/criterion-model', 'low')
ROLES = {'ordinary-reviewer': ORDINARY, 'challenge-reviewer': CHALLENGE, 'cold-reader': COLD,
         'criterion-author': CRITERION}
BINDINGS = {'roles': ROLES}


def gate_return_cases(h, gate, subject, role, expected, other_roles_sessions):
    """Refused for missing/model/thinking/both and for another role's binding; then accepted."""
    for label, session, difference in variants(expected):
        h.refused_return(gate, subject, f'{gate}-{label}', in_force=expected, session=session,
                         needles=session_refusal_needles(role, expected, session, difference))
        h.say(f'{gate} ({role}) return refused ({label}); diagnostic names role, expected, actual')
    for other in other_roles_sessions:
        h.refused_return(gate, subject, f'{gate}-other-role', in_force=expected, session=other,
                         needles=session_refusal_needles(role, expected, other, 'both'))
    h.say(f'{gate}: a session matching another role\'s binding is refused for role {role}')
    stored = h.returned(gate, subject, f'{gate}-matching', session=expected)
    require(all(r['data']['session'] == expected for r in stored), stored)
    h.say(f'{gate} ({role}) matching return accepted and stored with its session')


def advance(h, gate, subject):
    h.ledger(gate, subject)
    names = {e['event'] for e in h.show()['result']['requestable_events']}
    h.event('approved' if 'approved' in names else 'passed')


def cold_reader_gate(h):
    """intent-sufficient at intent-review is the cold-reader role, not the ordinary reviewer."""
    gate_return_cases(h, 'intent-review', 'intent.json', 'cold-reader', COLD, [ORDINARY])


def ordinary_gate(h):
    gate_return_cases(h, 'design-review', 'design.json', 'ordinary-reviewer', ORDINARY, [CHALLENGE, COLD])
    advance(h, 'design-review', 'design.json')
    h.say('design-review cleared by its sessioned return through the checked event')


def challenge_gate(h):
    gate_return_cases(h, 'design-adversarial-review', 'design.json', 'challenge-reviewer', CHALLENGE, [ORDINARY])
    advance(h, 'design-adversarial-review', 'design.json')
    h.say('design-adversarial-review cleared by its sessioned return through the checked event')


def verdicts(h):
    """criterion-author: every criterion and goal verdict batch of the validation index."""
    ctx = h.ctx
    prepared, report = ctx['prepared'], ctx['report']
    batches = prepared['judgment_batches']
    kinds = {batch['kind'] for batch in batches}
    require(kinds == {'criterion-verdict', 'goal-verdict'}, kinds)
    rows = {row['role']: row for row in h.bindings()['roles']}
    require(rows['criterion-author']['in_force'] == CRITERION, rows)
    seen = set()
    for index, batch in enumerate(batches):
        verdict = {'subject': 'validation-report.json', 'subject_revision': report['revision'],
                   'checkpoint': 'validation-checkpoint.json',
                   'author': {'name': f'external-judge-{index}', 'kind': 'agent'},
                   'result': 'pass', 'findings': [], 'reason': 'Scripted fixture judgments; no model or human review claim',
                   'evidence_context_ids': report['command_evidence_ids']}
        if batch['kind'] == 'criterion-verdict':
            verdict['criterion_id'] = batch['criterion_id']
        if batch['kind'] not in seen:
            seen.add(batch['kind'])
            for label, session, difference in variants(CRITERION):
                candidate = copy.deepcopy(verdict)
                if session is not None:
                    candidate['session'] = session
                h.refuse(batch['kind'], candidate,
                         session_refusal_needles('criterion-author', CRITERION, session, difference),
                         rule=ADMISSION_RULE)
                h.say(f"{batch['kind']} refused ({label}); role criterion-author named; run unchanged")
        verdict['session'] = CRITERION
        stored = h.append(batch['kind'], verdict, batch['record_id'])
        require(stored['data']['session'] == CRITERION, stored)
    h.say('criterion-verdict and goal-verdict accepted with the matching session')


HOOKS_COLD = {'intent-review': cold_reader_gate}
HOOKS_REVIEW = {'design-review': ordinary_gate, 'design-adversarial-review': challenge_gate,
                'validation-judgments': verdicts}
