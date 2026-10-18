"""Shared helpers for the model-binding black-box scenario.

Everything here drives the built loop-engine and software-change programs on private
catalogs; reviewer judgments are scripted and carry no semantic-review claim.
"""
import copy
import itertools
import json
import sys
from pathlib import Path

LABEL = 'Scripted fixture judgments; no model or human review claim'
HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / 'c_packets_admission'))
from capture_support import dump, require  # noqa: E402  (shared fixture assertions)

ROLES = ('driver', 'author', 'implementer', 'ordinary-reviewer', 'challenge-reviewer',
         'cold-reader', 'criterion-author')
OWNER = {'name': 'owner', 'kind': 'human'}
AUTHORITY = {'kind': 'owner', 'reference': 'owner chat, 2026-10-17'}
ADMISSION_RULE = 'model-binding-session'
AMENDMENT_RULE = 'model-binding-amendment-valid'
UNIQUE = itertools.count(1)
NEVER_BOUND = 'This run never had model bindings and has no model binding amendments.'


def binding(model, thinking='high'):
    return {'model': model, 'thinking': thinking}


def amendment(words, changes, *, actor=None, authority=None, driver_session=None):
    """The documented closed `model-binding-amendment` data."""
    data = {'actor': copy.deepcopy(actor or OWNER), 'authority': copy.deepcopy(authority or AUTHORITY),
            'words': words, 'changes': changes}
    if driver_session is not None:
        data['driver_session'] = driver_session
    return data


def change(role, old, new, mark_stale=None):
    value = {'role': role, 'old': old, 'new': new}
    if mark_stale is not None:
        value['mark_stale'] = mark_stale
    return value


def trim_profile(profile, keep):
    """Keep only the named gates' review policies (others become empty rooms)."""
    for gate in profile['review_policies']:
        profile['review_policies'][gate] = keep.get(gate, [])


def policy_of(profile, gate, axis, stage='aggregate', *, required_authors=None):
    """A copy of one shipped policy, optionally with a changed author floor."""
    found = next(p for p in profile['review_policies'][gate]
                 if p['id'] == axis and p['review_stage'] == stage)
    found = copy.deepcopy(found)
    if required_authors is not None:
        found['required_authors'] = required_authors
    return found


def configure_with(args, *, keep_from_shipped, bindings=None):
    """`configure` callback for the journey: keep chosen policies, optional frozen bindings.

    keep_from_shipped maps gate -> list of (axis, stage, required_authors); the policy
    text is the shipped one so the scenario stays a variant, never a shipped-data edit.
    """
    def configure(profile, repository, artifacts):
        shipped = copy.deepcopy(profile)
        keep = {gate: [policy_of(shipped, gate, axis, stage, required_authors=floor)
                       for axis, stage, floor in rows]
                for gate, rows in keep_from_shipped.items()}
        trim_profile(profile, keep)
        profile['extra']['advice']['enabled'] = False
        templates = artifacts.parent / 'templates'
        import shutil
        shutil.copytree(args.source_root / 'crates/software-change-provider/data/templates', templates)
        profile['extra']['template_root'] = str(templates)
        if bindings is not None:
            profile['model_bindings'] = copy.deepcopy(bindings)
    return configure


def json_text(value):
    return json.dumps(value, ensure_ascii=False)


class Harness:
    """One live run: snapshots, unchanged-run checks and the three admission shapes."""

    def __init__(self, ctx, log, name, trail):
        self.ctx, self.log, self.name, self.trail = ctx, log, name, trail
        self.run_id = ctx['run_id']

    # -- observation ---------------------------------------------------------------
    def say(self, text):
        line = f'[{self.name}] {text}'
        self.trail.append(line)
        print('ASSERT ' + line, flush=True)

    def show(self):
        return self.ctx['show']()

    def snap(self):
        return {'show': self.show(), 'history': self.ctx['call']('history', self.run_id)}

    def status(self):
        value = self.ctx['call']('show', '--view', 'status', self.run_id)
        require(value['status'] == 'completed', value)
        return value['result']

    def bindings(self):
        """The full-view `model_bindings` projection."""
        return self.show()['result']['model_bindings']

    def context(self):
        return self.show()['result']['context']

    # -- invariants ----------------------------------------------------------------
    @staticmethod
    def unchanged(before, after, *, history=True):
        a, b = before['show']['result'], after['show']['result']
        for key in ('context', 'current_state', 'state_visit', 'initial_input', 'lifecycle',
                    'has_overrides', 'model_bindings'):
            require(a[key] == b[key], f'unexpected mutation of {key}')
        require(not b['has_overrides'], 'overrides are never fixture clearance')
        if history:
            require(before['history'] == after['history'], 'history changed')

    @staticmethod
    def only_appended(before, after, kind):
        a, b = before['show']['result'], after['show']['result']
        require(b['context'][:-1] == a['context'] and b['context'][-1]['kind'] == kind, 'append shape')
        require(a['current_state'] == b['current_state'] and a['state_visit'] == b['state_visit'], 'state moved')
        rows = after['history']['result']
        require(rows[:-1] == before['history']['result'] and
                rows[-1]['action']['kind'] == 'context_appended', 'history shape')

    # -- appends -------------------------------------------------------------------
    def raw_append(self, kind, data, record_id=None):
        """`loop-engine append` of exact JSON data from a file (the documented `@file` form)."""
        path = self.ctx['work'] / f'append-input-{self.name}-{next(UNIQUE):04d}.json'
        dump(path, data)
        argv = ['append'] + (['--record-id', record_id] if record_id else [])
        return self.ctx['call'](*argv, self.run_id, kind, '@' + str(path))

    def append(self, kind, data, record_id=None):
        before = self.snap()
        response = self.raw_append(kind, data, record_id)
        require(response['status'] == 'completed', response)
        after = self.snap()
        self.only_appended(before, after, kind)
        stored = response['result']['context']
        require(stored['data'] == data and stored['kind'] == kind and
                (record_id is None or stored['id'] == record_id), stored)
        return stored

    def refuse(self, kind, data, needles, *, rule, record_id=None, code='record-refused'):
        """A refused append: every needle named, rule identified, run exactly unchanged."""
        record_id = record_id or f'refused-{self.name}-{next(UNIQUE)}'
        before = self.snap()
        response = self.raw_append(kind, data, record_id)
        after = self.snap()
        require(response['status'] == 'rejected' and response['code'] == code, response)
        text = json_text(response)
        for needle in needles:
            require(needle in text, (needle, response))
        if rule is not None:
            require(response['details']['rule'] == rule and
                    response['details']['record_id'] == record_id, response)
        self.unchanged(before, after)
        require(all(r['id'] != record_id for r in after['show']['result']['context']), response)
        return response

    def event(self, name, *, deny=None, code=None):
        before = self.snap()
        self.show()  # one driver: a fresh arming observation before each mutation
        response = self.ctx['call']('event', self.run_id, name)
        after = self.snap()
        if deny is None:
            require(response['status'] == 'completed', response)
            old, new = before['show']['result'], after['show']['result']
            require(old['current_state'] != new['current_state'] and
                    new['state_visit'] > old['state_visit'] and not new['has_overrides'], after)
            prior, following = before['history']['result'], after['history']['result']
            require(following[:len(prior)] == prior and len(following) > len(prior), 'history prefix')
            moved = [r['action'] for r in following[len(prior):] if r['action']['kind'] == 'transition']
            require(len(moved) == 1 and moved[0]['outcome']['outcome'] == 'committed' and
                    moved[0]['transition']['kind'] == 'checked', moved)
            return response
        text = json_text(response)
        require(response['status'] == 'rejected', response)
        if code is not None:
            require(response['code'] == code, response)
        for needle in ([deny] if isinstance(deny, str) else deny):
            require(needle in text, (needle, response))
        self.unchanged(before, after, history=False)
        rows = after['history']['result']
        require(rows[:-1] == before['history']['result'] and
                rows[-1]['action']['kind'] == 'transition' and
                rows[-1]['action']['outcome']['outcome'] == 'denied' and
                rows[-1]['action']['transition']['kind'] == 'checked', rows)
        return response

    # -- reviewer intake through the real review-candidates path -------------------------
    def intake(self, gate, subject, policies, author, *, session=None, assessment=None):
        """The raw review-candidates result for one assignment (no assertions on the outcome)."""
        value = json.loads((self.ctx['artifacts'] / subject).read_bytes())
        group, stage = policies[0]['group'], policies[0]['review_stage']
        pointer = '/revision'
        grounds = {'reason': LABEL + '; Python inspected actual field ' + pointer,
                   'evidence': [{'locator': subject + '#' + pointer}]}
        original = {'review_contract_version': 3, 'review_stage': stage, 'author': author,
                    'judgments': [{'axis': p['id'], 'result': 'pass', 'findings': '',
                                   'grounds': grounds} for p in policies]}
        if assessment is not None:
            original['assessment_id'] = assessment
        assignment = {'gate': gate, 'stage': stage, 'group': group, 'author': author,
                      'subject': subject, 'subject_revision': value['revision']}
        if session is not None:
            assignment['session'] = session
        packet = {'show': self.show(), 'assignment': assignment,
                  'request': {'text': LABEL + '; inspect ' + subject + '#' + pointer},
                  'original': {'text': json.dumps(original)}}
        return self.ctx['invoke']([self.ctx['provider'], 'review-candidates'], packet)

    def candidates(self, gate, subject, policies, author, *, session=None, assessment=None):
        result = self.intake(gate, subject, policies, author, session=session, assessment=assessment)
        require(len(result.get('records', [])) == len(policies) and
                all(not item.get('diagnostics') for item in result['items']), result)
        return result['records']

    def group_policies(self, gate):
        """The gate's frozen policies, one batch (all share the single group of this scenario)."""
        policies = self.show()['result']['initial_input']['review_policies'][gate]
        require(policies and len({(p['group'], p['review_stage']) for p in policies}) == 1, policies)
        return policies

    def returned(self, gate, subject, author_name, *, session=None, policies=None):
        """One accepted reviewer return: real intake, then append; the record carries `session`."""
        policies = policies or self.group_policies(gate)
        author = {'name': author_name, 'kind': 'agent'}
        records = self.candidates(gate, subject, policies, author, session=session)
        require(all(r['data'].get('session') == session for r in records) and
                (session is not None or all('session' not in r['data'] for r in records)), records)
        return [self.append(record['kind'], record['data']) for record in records]

    def refused_return(self, gate, subject, author_name, *, in_force, session, needles, policies=None):
        """A return whose session misses the binding in force: refused at intake and at append.

        The intake (review-candidates) carries the same admission diagnostics; the append is
        proven with the otherwise-valid records from a matching intake whose session is
        replaced, so the refusal is the session check alone.
        """
        policies = policies or self.group_policies(gate)
        author = {'name': author_name, 'kind': 'agent'}
        before = self.snap()
        intake = self.intake(gate, subject, policies, author, session=session)
        require(not intake['records'], intake)
        text = json_text(intake['items'])
        for needle in needles:
            require(needle in text, (needle, intake))
        self.unchanged(before, self.snap())
        good = self.candidates(gate, subject, policies, author, session=in_force)
        for record in good:
            data = copy.deepcopy(record['data'])
            if session is None:
                data.pop('session', None)
            else:
                data['session'] = session
            self.refuse(record['kind'], data, needles, rule=ADMISSION_RULE)

    def ledger(self, gate, subject):
        revision = json.loads((self.ctx['artifacts'] / subject).read_bytes())['revision']
        return self.append('finding-ledger', {
            'schema_version': '1', 'gate': gate, 'subject': subject, 'subject_revision': revision,
            'author': {'name': 'synthetic-driver', 'kind': 'agent'}, 'findings': []})

    def commission(self, gate, *, stage=None):
        argv = [str(self.ctx['provider']), 'commission', '--slot', gate]
        if stage:
            argv += ['--stage', stage]
        before = self.snap()
        value = self.ctx['invoke'](argv, self.show())
        self.unchanged(before, self.snap())
        return value

    def commission_raw(self, gate, *, stage=None):
        """Commission as a raw process result (stdout JSON plus stderr diagnostics)."""
        argv = [str(self.ctx['provider']), 'commission', '--slot', gate]
        if stage:
            argv += ['--stage', stage]
        packet = self.show()
        before = self.snap()
        result = self.log.execute(argv, packet, cwd=self.ctx['repository'], raw=True)
        self.unchanged(before, self.snap())
        return result


DIFFERENCE = {'model': 'model only', 'thinking': 'thinking level only',
              'both': 'model and thinking level'}


def session_refusal_needles(role, expected, actual, difference=None):
    """The documented shape: role, expected, actual (or missing) and the differing part."""
    needles = [f'model binding for role `{role}` is not met',
               f"expected model `{expected['model']}` at thinking `{expected['thinking']}`"]
    if actual is None:
        needles.append('actual: no session declared (statement missing)')
    else:
        needles.append(f"actual model `{actual['model']}` at thinking `{actual['thinking']}`")
        needles.append('differs in ' + DIFFERENCE[difference])
    return needles


def variants(expected):
    """Missing, model-only, thinking-only and both-differ sessions for one binding."""
    return [('missing', None, None),
            ('model', binding(expected['model'] + '-other', expected['thinking']), 'model'),
            ('thinking', binding(expected['model'], expected['thinking'] + '-other'), 'thinking'),
            ('both', binding(expected['model'] + '-other', expected['thinking'] + '-other'), 'both')]
