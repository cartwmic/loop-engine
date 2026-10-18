"""S1, S5, S6 and S8: an unbound run, amendments of every kind, every refusal cause, views."""
import copy

from harness import (AMENDMENT_RULE, NEVER_BOUND, ROLES, amendment, binding, change,
                     require, session_refusal_needles, variants)

GATE, SUBJECT = 'intent-review', 'intent.json'
REVIEWER_ONE = binding('provider/reviewer-one', 'high')
REVIEWER_TWO = binding('provider/reviewer-two', 'medium')
DRIVER = binding('provider/driver-model', 'high')
KIND = 'model-binding-amendment'


def view_roles(view):
    return {row['role']: row for row in view['roles']}


def check_views(h, expected_state, *, in_force=None, original=None, history=0, stale=()):
    """S8: status, full and owner approval view carry the same projection."""
    full = h.bindings()
    status = h.status()['model_bindings']
    owner = h.commission(GATE)['owner_approval_view']['model_bindings']
    require(full == status == owner, (full, status, owner))
    require(full['state'] == expected_state, full)
    if expected_state == 'never-bound':
        require(full == {'state': 'never-bound', 'statement': NEVER_BOUND}, full)
        return full
    rows = view_roles(full)
    require(list(rows) == list(ROLES), rows.keys())
    for role in ROLES:
        require(rows[role]['original'] == (original or {}).get(role) and
                rows[role]['in_force'] == (in_force or {}).get(role), (role, rows[role]))
    require(len(full['history']) == history, full['history'])
    for entry in full['history']:
        require({'record_id', 'sequence', 'when', 'actor', 'authority', 'words', 'changes',
                 'driver_session'} <= set(entry), entry)
    sequences = [entry['sequence'] for entry in full['history']]
    require(sequences == sorted(sequences), 'history is ordered')
    require([c['role'] for e in full['history'] for c in e['changes'] if c['mark_stale']] == list(stale), full)
    return full


def refusals_on_unbound(h):
    """S6 on a never-bound run: each cause, named, with the run unchanged."""
    base_change = [change('ordinary-reviewer', None, REVIEWER_ONE)]
    cases = [
        ('actor-not-human', amendment('w', base_change, actor={'name': 'bot', 'kind': 'agent'})),
        ('authority-not-owner', amendment('w', base_change,
                                          authority={'kind': 'standing-delegation', 'reference': 'x'})),
        ('words-blank', amendment('   ', base_change)),
        ('unknown-role', amendment('w', [change('reviewer', None, REVIEWER_ONE)])),
        # changing an unbound role: its `old` binding is not the one in force (none)
        ('old-binding-not-in-force', amendment('w', [change('ordinary-reviewer', REVIEWER_ONE, REVIEWER_TWO)])),
        # a malformed non-null old binding on an unbound role is not 'no binding' (with a stale mark)
        ('old-binding-not-in-force', amendment('w', [change('ordinary-reviewer',
                                                             {'model': 'p/prior', 'thinking_level': 'high'},
                                                             REVIEWER_TWO, mark_stale=True)])),
        # removing an unbound role
        ('old-binding-not-in-force', amendment('w', [change('cold-reader', REVIEWER_ONE, None)])),
        ('new-binding-malformed', amendment('w', [change('ordinary-reviewer', None,
                                                          {'model': '', 'thinking': 'high'})])),
        ('driver-declaration-missing', amendment('w', [change('driver', None, DRIVER)])),
        ('driver-declaration-mismatch', amendment('w', [change('driver', None, DRIVER)],
                                                  driver_session=binding('provider/driver-model', 'low'))),
        ('stale-mark-not-allowed', amendment('w', [change('author', None, binding('provider/a'),
                                                          mark_stale=True)])),
        ('malformed-amendment', amendment('w', [])),
    ]
    for code, data in cases:
        h.refuse(KIND, data, [f'model binding amendment refused ({code})'], rule=AMENDMENT_RULE)
        h.say(f'amendment refused as {code} on a never-bound run; run unchanged')
    # Stale marks on every role outside the four evidence roles.
    for role in ('driver', 'author', 'implementer'):
        data = amendment('w', [change(role, None, binding('provider/x'), mark_stale=True)],
                         driver_session=binding('provider/x') if role == 'driver' else None)
        h.refuse(KIND, data, ['model binding amendment refused (stale-mark-not-allowed)'], rule=AMENDMENT_RULE)
    h.say('stale mark refused for driver, author and implementer')


def refusals_on_bound(h):
    """S6 on a bound run: adding to a bound role, a wrong old value, a missing driver declaration."""
    for data, why in [
        (amendment('w', [change('ordinary-reviewer', None, REVIEWER_TWO)], driver_session=DRIVER), 'adding to a bound role'),
        (amendment('w', [change('ordinary-reviewer', REVIEWER_TWO, binding('p/z'))], driver_session=DRIVER),
         'wrong old binding'),
        (amendment('w', [change('ordinary-reviewer', binding('provider/reviewer-one', 'low'), None)],
                   driver_session=DRIVER), 'removing with a wrong old binding'),
    ]:
        h.refuse(KIND, data, ['model binding amendment refused (old-binding-not-in-force)'], rule=AMENDMENT_RULE)
        h.say(f'amendment refused as old-binding-not-in-force: {why}')
    # The driver is bound: even a change to other roles needs a matching driver declaration.
    other = [change('ordinary-reviewer', REVIEWER_ONE, REVIEWER_TWO)]
    h.refuse(KIND, amendment('w', other), ['model binding amendment refused (driver-declaration-missing)'],
             rule=AMENDMENT_RULE)
    h.refuse(KIND, amendment('w', other, driver_session=binding('provider/driver-model', 'low')),
             ['model binding amendment refused (driver-declaration-mismatch)'], rule=AMENDMENT_RULE)
    h.say('bound driver: amendment of other roles refused without or with a mismatching driver_session')


def return_refusals(h, role, expected):
    """Missing, model-only, thinking-only and both-differ returns are refused with the role named."""
    for label, session, difference in variants(expected):
        h.refused_return(GATE, SUBJECT, f'refused-{role}-{label}', in_force=expected, session=session,
                         needles=session_refusal_needles(role, expected, session, difference))
        h.say(f'{role} return refused ({label}): names role, expected and actual; run unchanged')


def prefix_kept(before, after):
    """Earlier records stay exactly as stored: an amendment rewrites no earlier record."""
    require(after[:len(before)] == before, 'an earlier record changed')


def run(h):
    """S1, S5, S6, S8 on a run started with no bindings, driven at intent-review."""
    # S1: an unbound run behaves as before.
    require(h.show()['result']['initial_input'].get('model_bindings') is None, 'run must start unbound')
    check_views(h, 'never-bound')
    h.say('S1/S8 unbound run: status, full and owner views state never-bound explicitly')
    first = h.returned(GATE, SUBJECT, 'review-0')
    require('session' not in first[0]['data'], first)
    h.say('S1 unbound run accepts a return with no session; no new refusal')
    refusals_on_unbound(h)

    # S5 add: an amendment on a run that started with none (also adds the driver with its declaration).
    before = h.context()
    add = amendment('Bind ordinary review to provider/reviewer-one at high; bind the driver.',
                    [change('ordinary-reviewer', None, REVIEWER_ONE), change('driver', None, DRIVER)],
                    driver_session=DRIVER)
    added = h.append(KIND, add, 'amend-1')
    prefix_kept(before, h.context())
    view = check_views(h, 'bound', in_force={'ordinary-reviewer': REVIEWER_ONE, 'driver': DRIVER}, history=1)
    entry = view['history'][0]
    require(entry['record_id'] == 'amend-1' and entry['actor'] == add['actor'] and
            entry['authority'] == add['authority'] and entry['words'] == add['words'] and
            entry['driver_session'] == DRIVER and
            entry['changes'] == [{'role': 'ordinary-reviewer', 'old': None, 'new': REVIEWER_ONE, 'mark_stale': False},
                                 {'role': 'driver', 'old': None, 'new': DRIVER, 'mark_stale': False}], entry)
    require(view['original_driver_session'] is None, view)
    require(added['kind'] == KIND, added)
    h.say('S5 add accepted on a run started with none; views show original none, in force new, history entry')
    refusals_on_bound(h)
    return_refusals(h, 'ordinary-reviewer', REVIEWER_ONE)
    second = h.returned(GATE, SUBJECT, 'review-1', session=REVIEWER_ONE)
    require(second[0]['data']['session'] == REVIEWER_ONE, second)
    stored = {r['id']: r for r in h.context()}
    require(stored[first[0]['id']] == first[0], 'the earlier unsessioned return is still stored, not re-judged')
    h.say('S5 matching return accepted after the add; the earlier return is still stored as it was')

    # S5 change: `old` is required and checked.
    before = h.context()
    change_data = amendment('Move ordinary review to provider/reviewer-two at medium.',
                            [change('ordinary-reviewer', REVIEWER_ONE, REVIEWER_TWO)], driver_session=DRIVER)
    h.append(KIND, change_data, 'amend-2')
    prefix_kept(before, h.context())
    view = check_views(h, 'bound', in_force={'ordinary-reviewer': REVIEWER_TWO, 'driver': DRIVER}, history=2)
    require(view['history'][1]['changes'][0]['old'] == REVIEWER_ONE and
            view['history'][1]['changes'][0]['new'] == REVIEWER_TWO, view)
    h.refused_return(GATE, SUBJECT, 'review-old-binding', in_force=REVIEWER_TWO, session=REVIEWER_ONE,
                     needles=session_refusal_needles('ordinary-reviewer', REVIEWER_TWO, REVIEWER_ONE, 'both'))
    third = h.returned(GATE, SUBJECT, 'review-2', session=REVIEWER_TWO)
    stored = {r['id']: r for r in h.context()}
    require(stored[second[0]['id']] == second[0] and stored[first[0]['id']] == first[0], 'earlier returns re-judged')
    h.say('S5 change accepted; a return under the old binding is refused, one under the new is accepted; '
          'earlier returns (unbound era and old binding) are untouched')

    # S5 remove, with the bare-string authority the docs allow.
    before = h.context()
    remove = amendment('Stop requiring a model for ordinary review.',
                       [change('ordinary-reviewer', REVIEWER_TWO, None)], authority='owner', driver_session=DRIVER)
    h.append(KIND, remove, 'amend-3')
    prefix_kept(before, h.context())
    view = check_views(h, 'bound', in_force={'driver': DRIVER}, history=3)
    require(view['history'][2]['changes'][0]['new'] is None, view)
    fourth = h.returned(GATE, SUBJECT, 'review-3')
    require('session' not in fourth[0]['data'], fourth)
    h.say('S5 remove accepted; a later return needs no session; a run amended to no reviewer binding is still "bound"')
    # Removing the driver needs no declaration.
    h.append(KIND, amendment('Release the driver.', [change('driver', DRIVER, None)]), 'amend-4')
    view = check_views(h, 'bound', history=4)
    require(view['history'][3]['driver_session'] is None, view)
    h.say('S5/S6 removing the driver needs no driver declaration')

    # All amendments are in the history view with actor, words, old and new.
    history = h.bindings()['history']
    require([e['record_id'] for e in history] == ['amend-1', 'amend-2', 'amend-3', 'amend-4'], history)
    require(all(e['actor'] == {'name': 'owner', 'kind': 'human'} and e['words'] for e in history), history)
    require(history[2]['authority'] == 'owner', history)
    h.say('S8 the ordered history lists every amendment with actor, authority, words, old and new')
    # Return counts and the unchanged earlier records at the end.
    require([r['id'] for r in h.context() if r['kind'] == 'review-evidence'] ==
            [first[0]['id'], second[0]['id'], third[0]['id'], fourth[0]['id']], 'returns kept in order')
    return copy.deepcopy(history)
