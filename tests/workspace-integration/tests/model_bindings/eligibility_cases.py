"""S7 and S8: what an amendment does to evidence stored before it.

Both cases use a two-author floor on one intent-review axis (a variant of the shipped
policy, never an edit of shipped data).  One return before the amendment and one after
exercise the unmarked default; a `mark_stale` amendment excludes the earlier returns.
"""
import json

from amendment_cases import GATE, KIND, SUBJECT, check_views
from harness import amendment, binding, change, require

REVIEWER = binding('provider/new-reviewer', 'high')
AXIS = 'solution-agnostic'
SHORTAGE = 'missing'


def carry_of(h, origin):
    revision = json.loads((h.ctx['artifacts'] / SUBJECT).read_bytes())['revision']
    return {'origin': {'kind': 'context-record', 'id': origin['id']},
            'target': {'subject': SUBJECT, 'revision': revision, 'checkpoint': None},
            'attesting_driver': {'name': 'synthetic-driver', 'kind': 'agent'},
            'reason': 'Scripted fixture judgments; no model or human review claim; unchanged bounded scope'}


def stale_cause(amendment_id, origin):
    """The documented cause text: amendment id, role, author, assessment and axis."""
    data = origin['data']
    return (f"record `{origin['id']}` is excluded: model binding amendment `{amendment_id}` marked role "
            f"`ordinary-reviewer` stale; author {data['author']['name']} ({data['author']['kind']}), "
            f"assessment `{data['assessment_id']}`, axis `{data['policy_id']}`")


def axis_diagnostics(response):
    """The evidence diagnostics of the refused checked event, by category."""
    require(response['code'] == 'software-change-review-incomplete', response)
    axes = [d for d in response['details']['diagnostics'] if d['axis'] == AXIS]
    require(len(axes) == 1, response)
    return {d['category']: d for d in axes[0]['diagnostics']}


def event_causes(response):
    return axis_diagnostics(response)['excluded_by_stale_mark']['causes']


def unmarked(h):
    """S7(a): an unmarked amendment keeps earlier evidence eligible; mixed bindings clear the gate."""
    first = h.returned(GATE, SUBJECT, 'elig-unmarked-0')[0]
    h.ledger(GATE, SUBJECT)
    shortage = axis_diagnostics(h.event('approved', deny=[AXIS]))
    require(shortage['independence'] == {'category': 'independence', 'distinct_present': 1, 'required': 2}, shortage)
    added = amendment('Bind ordinary review to provider/new-reviewer at high; keep earlier returns.',
                      [change('ordinary-reviewer', None, REVIEWER)])
    h.append(KIND, added, 'amend-unmarked')
    h.say('S7a unmarked amendment accepted after one return')
    commissioned = h.commission(GATE, stage='aggregate')
    require(first['id'] in {r['id'] for r in commissioned['context']} and
            commissioned['record_diagnostics'] == [], commissioned)
    h.say('S7a commission still delivers the pre-amendment return, with no exclusion cause')
    shortage = axis_diagnostics(h.event('approved', deny=[AXIS]))
    require('excluded_by_stale_mark' not in shortage and shortage['independence']['distinct_present'] == 1, shortage)
    h.say('S7a the checked event still counts the pre-amendment return (one distinct author), names no stale cause')
    carry = h.append('evidence-applicability', carry_of(h, first), 'carry-unmarked')
    commissioned = h.commission(GATE, stage='aggregate')
    delivered = {r['id'] for r in commissioned['context']}
    require(first['id'] in delivered and carry['id'] in delivered and commissioned['record_diagnostics'] == [],
            commissioned)
    shortage = axis_diagnostics(h.event('approved', deny=[AXIS]))
    require('excluded_by_stale_mark' not in shortage, shortage)
    h.say('S7a carry of the pre-amendment return accepted at the append, delivered by commission and '
          'not excluded by the checked event')
    h.returned(GATE, SUBJECT, 'elig-unmarked-1', session=REVIEWER)
    h.event('approved')
    h.say('S7a the gate clears through the checked event with a pre-amendment and a post-amendment return')
    check_views(h, 'bound', in_force={'ordinary-reviewer': REVIEWER}, history=1)
    h.say('S8 views after the unmarked amendment: original none, in force new, no stale mark')


def marked(h):
    """S7(b): a marked amendment excludes the earlier returns, with one cause everywhere."""
    first = h.returned(GATE, SUBJECT, 'elig-marked-0')[0]
    second = h.returned(GATE, SUBJECT, 'elig-marked-1')[0]
    h.ledger(GATE, SUBJECT)
    require(h.commission(GATE, stage='aggregate')['record_diagnostics'] == [], 'nothing excluded yet')
    before = h.context()
    marked_change = amendment('Rebind ordinary review to provider/new-reviewer at high; discard earlier reviews.',
                              [change('ordinary-reviewer', None, REVIEWER, mark_stale=True)])
    h.append(KIND, marked_change, 'amend-marked')
    require(h.context()[:len(before)] == before, 'the amendment rewrote an earlier record')
    check_views(h, 'bound', in_force={'ordinary-reviewer': REVIEWER}, history=1, stale=('ordinary-reviewer',))
    h.say('S7b/S8 marked amendment accepted; views show the stale mark in the history')
    causes = [stale_cause('amend-marked', record) for record in (first, second)]
    commissioned = h.commission(GATE, stage='aggregate')
    require(commissioned['context'] == [] and commissioned['review_record_sources'] == [], commissioned)
    in_commission = [d['reason'] for d in commissioned['record_diagnostics']]
    require(in_commission == causes, in_commission)
    refusal = h.event('approved', deny=[AXIS, *causes])
    at_event = event_causes(refusal)
    require(at_event == in_commission == causes, (at_event, in_commission))
    h.say('S7b earlier returns excluded: commission and the checked event give the SAME cause text '
          '(amendment id, role, author, assessment, axis)')
    h.refuse('evidence-applicability', carry_of(h, first),
             ['carries a stale-marked origin', causes[0]], rule='a-applicability-origin-not-withdrawn')
    h.say('S7b a new carry of an excluded return is refused with the cause')
    h.returned(GATE, SUBJECT, 'elig-marked-2', session=REVIEWER)
    again = h.event('approved', deny=[AXIS, *causes])
    require(axis_diagnostics(again)['independence']['distinct_present'] == 1 and event_causes(again) == causes, again)
    h.returned(GATE, SUBJECT, 'elig-marked-3', session=REVIEWER)
    h.event('approved')
    h.say('S7b fresh returns under the new binding clear the gate; one fresh return alone did not')
