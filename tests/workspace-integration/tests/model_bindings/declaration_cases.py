"""S4: author and implementer declarations at the real admission boundaries.

The cases drive the shipped checked events (`intent-ready`, `design-ready`, `plan-ready`,
the validation draft, `reconciliation-ready`, `implementation-ready`) of a run whose
author and implementer are bound; every other role is left unbound so the scripted
journey can keep moving.
"""
import json

from harness import ADMISSION_RULE, binding, require, session_refusal_needles, variants

AUTHOR = binding('provider/author-model', 'high')
IMPLEMENTER = binding('provider/implementer-model', 'medium')
BINDINGS = {'roles': {'author': AUTHOR, 'implementer': IMPLEMENTER}}
ARTIFACT_REFUSAL = 'software-change-artifact-admission-invalid'


def revision_of(h, subject):
    return json.loads((h.ctx['artifacts'] / subject).read_bytes())['revision']


def declaration(role, subject, revision, session):
    return {'subject': subject, 'subject_revision': revision, 'role': role, 'session': session}


def boundary(h, role, expected, subject, event, *, advance=True):
    """Missing refused at the event, mismatches refused at the append, matching accepted."""
    revision = revision_of(h, subject)
    needle = f'{subject} revision `{revision}` needs a matching session-declaration'
    h.event(event, deny=[needle, *session_refusal_needles(role, expected, None)], code=ARTIFACT_REFUSAL)
    h.say(f'{event}: no declaration refused at the checked event; run unchanged')
    for label, session, difference in variants(expected)[1:]:
        h.refuse('session-declaration', declaration(role, subject, revision, session),
                 session_refusal_needles(role, expected, session, difference), rule=ADMISSION_RULE)
        h.say(f'{event}: declaration differing in {label} refused at the append; run unchanged')
    # A declaration for another revision of the same subject does not satisfy the boundary.
    stale = declaration(role, subject, revision + '-other', expected)
    h.append('session-declaration', stale)
    h.event(event, deny=[needle], code=ARTIFACT_REFUSAL)
    h.say(f'{event}: a matching declaration for another revision does not satisfy the event')
    stored = h.append('session-declaration', declaration(role, subject, revision, expected))
    require(stored['data']['session'] == expected, stored)
    if advance:
        h.event(event)
        h.say(f'{event}: matching declaration accepted and the checked event committed')
    return revision


def explore(h):
    view = h.bindings()
    rows = {row['role']: row for row in view['roles']}
    require(view['state'] == 'bound' and rows['author']['in_force'] == AUTHOR and
            rows['implementer']['in_force'] == IMPLEMENTER and rows['driver']['in_force'] is None, view)
    require(h.show()['result']['initial_input']['model_bindings'] == BINDINGS, 'frozen input missing')
    h.say('S4 run started with author and implementer bound; views show them in force')
    boundary(h, 'author', AUTHOR, 'intent.json', 'intent-ready')


def design(h):
    boundary(h, 'author', AUTHOR, 'design.json', 'design-ready')


def plan(h):
    boundary(h, 'author', AUTHOR, 'plan.json', 'plan-ready')


def implement(h):
    """implementation-ready is report-free: it succeeds with no declaration while bound."""
    rows = {row['role']: row for row in h.bindings()['roles']}
    require(rows['implementer']['in_force'] == IMPLEMENTER, rows)
    h.ctx['checkpoint']('implementation')
    h.event('implementation-ready')
    require(not [r for r in h.context() if r['kind'] == 'session-declaration' and
                 r['data']['role'] == 'implementer'], 'no implementer declaration was needed')
    h.say('S4 implementation-ready succeeds with no declaration while the implementer is bound')


def reconciliation(h):
    reconciliation = {'revision': 'reconciliation-1', 'author': {'name': 'external-driver', 'kind': 'script'},
                      'mode': 'bookends-disabled', 'branch': 'sufficient-existing-wording',
                      'document_observations': [{'path': 'helper.py', 'status': 'unchanged',
                                                 'observation': 'Fixture source requires no document amendment.'}],
                      'behavior_observations': [{'status': 'matches-intent',
                                                 'observation': 'The helper returns its input.'}],
                      'action': 'no-document-change', 'action_reason': 'No enduring document meaning changes.',
                      'authorization': 'not-required', 'application': 'not-required', 'commit': 'not-required',
                      'traceability': {'status': 'not-applicable', 'references': []},
                      'proof_references': ['plan.json#/proof_commands'], 'blockers': [], 'decision': 'complete'}
    (h.ctx['artifacts'] / 'reconciliation.json').write_text(json.dumps(reconciliation))
    boundary(h, 'implementer', IMPLEMENTER, 'implementation-report.json', 'reconciliation-ready')


def validation_prepared(h):
    """The validation draft: the journey itself fires the final-hop event after this hook."""
    names = {e['event'] for e in h.show()['result']['requestable_events']}
    require('passed' in names and 'validation-ready' not in names, names)
    revision = revision_of(h, 'validation-report.json')
    boundary(h, 'author', AUTHOR, 'validation-report.json', 'passed', advance=False)
    h.validation_revision = revision


def finished(h):
    """After the journey fired `passed` itself, the run is final and the declaration is in history."""
    shown = h.show()['result']
    require(shown['current_state'] == 'end' and shown['lifecycle'] == 'final' and not shown['has_overrides'], shown)
    stored = [r for r in shown['context'] if r['kind'] == 'session-declaration' and
              r['data']['subject'] == 'validation-report.json' and
              r['data']['subject_revision'] == h.validation_revision and r['data']['session'] == AUTHOR]
    require(len(stored) == 1, stored)
    rows = h.ctx['call']('history', h.run_id)['result']
    moved = [r['action'] for r in rows if r['action']['kind'] == 'transition' and
             r['action']['outcome']['outcome'] == 'committed' and r['action']['transition']['target'] == 'end']
    require(len(moved) == 1 and moved[0]['transition']['kind'] == 'checked', moved)
    h.say('validation draft: matching declaration accepted and `passed` committed the run to end')


HOOKS = {'explore': explore, 'design': design, 'plan': plan, 'implement': implement,
         'reconciliation': reconciliation, 'validation-prepared': validation_prepared}
