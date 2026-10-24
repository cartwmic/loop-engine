"""Repeatable review identity and steering cases through the real public CLIs."""
import copy
import json
from pathlib import Path
from capture_support import require, sha, dump, snapshot, invariant, checked_allow, groups, Capture
LABEL = 'Scripted fixture judgments; no model or human review claim'
def categories(value):
    if isinstance(value, dict):
        if 'category' in value:
            yield value['category']
        for child in value.values():
            yield from categories(child)
    elif isinstance(value, list):
        for child in value:
            yield from categories(child)


def event_name(ctx):
    events = ctx['show']()['result']['requestable_events']
    names = {event['event'] for event in events}
    require('approved' in names or 'passed' in names, events)
    return 'approved' if 'approved' in names else 'passed'


def ledger(ctx, gate, subject, revision):
    response = ctx['append']('finding-ledger', {
        'schema_version': '1', 'gate': gate, 'subject': subject, 'subject_revision': revision,
        'author': {'name': 'synthetic-driver', 'kind': 'agent'}, 'findings': []})
    require(response['status'] == 'completed', response)


def inspect_field(ctx, subject):
    value = json.loads((ctx['artifacts'] / subject).read_bytes())
    # Inspect and retain an actual substantive synthetic subject field, not a forged locator/digest.
    key = next(k for k in value if k not in ('revision', 'author') and not k.endswith('_revision'))
    pointer = '/' + key.replace('~', '~0').replace('/', '~1')
    return value, pointer, value[key]


def candidates(ctx, gate, subject, policies, author, assessment=None):
    value, pointer, inspected = inspect_field(ctx, subject)
    require(author != value['author'], 'Fixture author must be independent of subject author')
    group = policies[0]['group']
    stage = policies[0]['review_stage']
    require(all(p['group'] == group and p['review_stage'] == stage and
                p['required_authors'] == 2 for p in policies), policies)
    grounds = {'reason': LABEL + '; Python inspected actual field ' + pointer + ': ' +
                         json.dumps(inspected, sort_keys=True, ensure_ascii=False),
               'evidence': [{'locator': subject + '#' + pointer}]}
    original = {'review_contract_version': 3, 'review_stage': stage, 'author': author,
                'judgments': [{'axis': p['id'], 'result': 'pass', 'findings': '',
                               'grounds': grounds} for p in policies]}
    if assessment is not None:
        original['assessment_id'] = assessment
    assignment = {'gate': gate, 'stage': stage, 'group': group, 'author': author,
                  'subject': subject, 'subject_revision': value['revision']}
    packet = {'show': ctx['show'](), 'assignment': assignment,
              'request': {'text': LABEL + '; inspect ' + subject + '#' + pointer +
                           ' for assigned axes: ' + ', '.join(p['id'] for p in policies)},
              'original': {'text': json.dumps(original)}}
    result = ctx['invoke']([ctx['provider'], 'review-candidates'], packet)
    require(len(result.get('records', [])) == len(policies), result)
    require(all(not item.get('diagnostics', []) for item in result['items']), result)
    require(len({r['data']['assessment_id'] for r in result['records']}) == 1, result)
    require(all(r['data']['review_contract_version'] == 3 and
                r['data']['grounds'] == grounds for r in result['records']), result)
    return result['records']


def groups(policies):
    selected = {}
    for p in policies:
        selected.setdefault((p['review_stage'], p['group']), []).append(p)
    return sorted(selected.items(), key=lambda pair: (pair[0][0] != 'individual', pair[0]))


def append_candidates(ctx, records):
    stored = []
    for record in records:
        response = ctx['append'](record['kind'], record['data'])
        require(response['status'] == 'completed', response)
        actual = response['result']['context']
        require(actual['data'] == record['data'], actual)
        stored.append(actual)
    return stored


def commission(ctx, gate, stage):
    show = ctx['show']()
    require(show['result']['current_state'] == gate, show)
    frozen_input = show['result']['initial_input']
    frozen = frozen_input['review_policies'][gate]
    require(frozen == ctx['profile']['review_policies'][gate], 'Frozen target policy changed')
    assigned = [p for p in frozen if p['review_stage'] == stage]
    require(stage in ('individual', 'aggregate') and assigned and
            all(p['required_authors'] == 2 and p['group'] for p in assigned), assigned)
    subject = {'design-review': 'design.json', 'implementation-review': 'implementation-report.json',
               'validation-review': 'validation-report.json'}[gate]
    require(Path(frozen_input['artifact_root']).resolve() == ctx['artifacts'].resolve(), show)
    subject_bytes = (ctx['artifacts'] / subject).read_bytes()
    revision = json.loads(subject_bytes)['revision']
    argv = [str(ctx['provider']), 'commission', '--slot', gate, '--stage', stage]
    result = ctx['invoke'](argv, show)
    # Ordinary review projection consumes --stage but need not echo it in commission.
    # Bind the witness to the genuine logger's last CLI request/response, never insert an echo.
    capture_root = ctx['work'].parent / 'commands'
    row = json.loads((capture_root / 'commands.jsonl').read_text().splitlines()[-1])
    require(row['argv'] == argv and row['cwd'] == str(ctx['repository'].resolve()) and
            row['exit_code'] == 0, row)
    for stream, value in (('stdin', show), ('stdout', result)):
        retained = (capture_root / row[stream]['path']).read_bytes()
        require(sha(retained) == row[stream]['sha256'] and len(retained) == row[stream]['bytes']
                and json.loads(retained) == value, f'Captured {stream} does not bind commission')
    require(result['slot_id'] == gate and result['review_target'] ==
            {'subject': subject, 'revision': revision} and result['review_contract_version'] == 3, result)
    if result['commission'].get('review_stage') is not None:
        require(result['commission']['review_stage'] == stage, result)
    subject_sources = [s for s in result['review_sources'] if s['locator'] == subject]
    require(len(subject_sources) == 1 and subject_sources[0]['sha256'] == 'sha256:' + sha(subject_bytes)
            and subject_sources[0]['bytes'] == len(subject_bytes), result)
    actual = {r['id']: r for r in show['result']['context']}
    delivered = {r['id']: r for r in result['context']}
    def by_reference(key, record):
        # A commission delivers retained exchange bodies by reference (#59): each stub
        # must name the stored body's exact size, digest and locator.
        stored = copy.deepcopy(actual.get(key))
        exchange = (stored or {}).get('data', {}).get('external_review') or {}
        for part, body in (record.get('data', {}).get('external_review') or {}).items():
            if isinstance(body, dict) and body.get('omitted_from_commission') is True:
                blob = json.dumps(exchange[part], separators=(',', ':'), sort_keys=True,
                                  ensure_ascii=False).encode()
                require(part in ('request', 'original') and body['locator'] ==
                        f'record:{key}#/data/external_review/{part}' and
                        body['bytes'] == len(blob) and body['sha256'] == 'sha256:' + sha(blob), body)
                exchange[part] = body
        return stored == record
    require(len(delivered) == len(result['context']) and
            all(by_reference(key, record) for key, record in delivered.items()), result)
    require(set(result['commission']['record_ids']) == set(delivered) and
            len(result['commission']['record_ids']) == len(delivered), result)
    sources = result['review_record_sources']
    require(len(sources) == len(delivered) and
            {(s['record_id'], s['kind']) for s in sources} ==
            {(r['id'], r['kind']) for r in delivered.values()}, result)
    dump(ctx['work'] / f'commission-request-witness-{row["command_id"]}.json', {
        'label': 'EXTERNAL REQUEST WITNESS; requested stage is not a provider response echo',
        'capture_index': str(capture_root / 'commands.jsonl'), 'command_id': row['command_id'],
        'argv': row['argv'], 'cwd': row['cwd'], 'exit_code': row['exit_code'],
        'stdin_capture': row['stdin'], 'stdout_capture': row['stdout'],
        'requested_stage': stage, 'observed_state': show['result']['current_state'],
        'review_target': result['review_target'],
        'frozen_stage_policies': [{k: p[k] for k in ('id', 'review_stage', 'group', 'required_authors')}
                                  for p in assigned],
        'selected_record_ids': result['commission']['record_ids'],
        'provider_stage_echo': result['commission'].get('review_stage'),
        'limit': 'Request binding alone is not a visibility control; no byte-budget stage inference'})
    return result


def stage_context_controls(ctx, gate, subject):
    # Four read-only controls: two per AC5 slot, after existing
    # legitimate judgments. With an aggregate present and no findings/applicability,
    # the current public selector delivers exactly same-review evidence at --stage.
    for stage in ('individual', 'aggregate'):
        before = snapshot(ctx)
        revision = json.loads((ctx['artifacts'] / subject).read_bytes())['revision']
        reviews = [r for r in before['show']['result']['context'] if r['kind'] == 'review-evidence'
                   and r['data']['gate'] == gate and r['data']['subject'] == subject
                   and r['data']['subject_revision'] == revision]
        require({r['data']['review_stage'] for r in reviews} == {'individual', 'aggregate'}, reviews)
        expected = {r['id'] for r in reviews if r['data']['review_stage'] == stage}
        omitted = {r['id'] for r in reviews} - expected
        packet = commission(ctx, gate, stage)
        after = snapshot(ctx)
        dump(ctx['work'] / f'{gate}-{stage}-postwrite-control.json', {
            'before': before, 'packet': packet, 'after': after, 'requested_stage': stage,
            'exact_selected_review_ids': sorted(expected), 'exact_other_stage_omissions': sorted(omitted)})
        delivered_reviews = [r for r in packet['context'] if r['kind'] == 'review-evidence']
        require(expected and omitted and {r['id'] for r in delivered_reviews} == expected and
                all(r['data']['gate'] == gate and r['data']['subject'] == subject and
                    r['data']['subject_revision'] == revision and r['data']['review_stage'] == stage
                    for r in delivered_reviews), packet)
        require(not omitted.intersection(r['id'] for r in packet['context']) and
                not omitted.intersection(s['record_id'] for s in packet['review_record_sources']), packet)
        invariant(before, after)
        require(before['history'] == after['history'], 'Read-only stage control mutated history')


def ac4_hook(case, finished):
    def hook(state, ctx):
        if state != 'design-review':
            return False
        gate, subject = state, 'design.json'
        policies = ctx['profile']['review_policies'][gate]
        revision = json.loads((ctx['artifacts'] / subject).read_bytes())['revision']
        packet = commission(ctx, gate, 'aggregate')
        dump(ctx['work'] / 'ac4-initial-commission.json', packet)
        grouped = groups(policies)
        target_key, target = max((g for g in grouped if g[0][0] == 'aggregate'), key=lambda g: len(g[1]))
        require(len(target) > 1, 'Canonical design aggregate must be multi-axis')
        normal = [{'name': f'synthetic-A-{i}', 'kind': 'agent'} for i in range(2)]
        alternate = [{'name': f'synthetic-B-{i}', 'kind': 'agent'} for i in range(2)]
        stored = []
        if case == 'ac4-missing':
            preview = candidates(ctx, gate, subject, target, normal[0])
            missing = copy.deepcopy(preview[0]['data'])
            del missing['assessment_id']
            before = snapshot(ctx)
            response = ctx['append']('review-evidence', missing)
            after = snapshot(ctx)
            dump(ctx['work'] / 'negative.json', {'before': before, 'refused_data': missing,
                                               'response': response, 'after': after})
            require(response['status'] == 'rejected' and response['code'] == 'record-refused', response)
            details = response['details']
            require(details['rule'] == 'review-assessment-identity', response)
            require('assessment_id' in json.dumps(details) and 'independence' not in json.dumps(details), response)
            require(details['record_id'] and all(r['id'] != details['record_id']
                    for r in after['show']['result']['context']), response)
            invariant(before, after)
            require(before['history'] == after['history'], 'Refused append changed history/sequence')
        else:
            for key, batch in grouped:
                authors = alternate if case == 'ac4-mixed-authors' and key[0] == 'aggregate' else normal
                for author in authors:
                    # Separate valid one-axis returns produce inconsistent group identities;
                    # never forge helper metadata or seed a historical record.
                    subsets = [[p] for p in batch] if case == 'ac4-inconsistent' and key == target_key else [batch]
                    for subset in subsets:
                        stored.extend(append_candidates(ctx, candidates(ctx, gate, subject, subset, author)))
            ledger(ctx, gate, subject, revision)
            before = snapshot(ctx)
            name = event_name(ctx)
            response = ctx['call']('event', ctx['run_id'], name, allow_rejected=True)
            after = snapshot(ctx)
            dump(ctx['work'] / 'negative.json', {'before': before, 'response': response,
                                               'after': after, 'individual_writes': stored})
            expected = 'assessment_inconsistent' if case == 'ac4-inconsistent' else 'mixed_authors'
            found = set(categories(response))
            require(response['status'] == 'rejected' and expected in found, response)
            require('independence' not in found and 'missing' not in found and
                    'malformed' not in found and 'unverified' not in found, response)
            invariant(before, after)
            prior, following = before['history']['result'], after['history']['result']
            require(isinstance(prior, list) and isinstance(following, list), after['history'])
            require(following[:-1] == prior and len(following) == len(prior) + 1,
                    'Checked deny must preserve prefix and add actual durable denial')
            action = following[-1]['action']
            require(action['kind'] == 'transition' and action['transition']['kind'] == 'checked' and
                    action['outcome']['outcome'] == 'denied' and
                    expected in set(categories(following[-1])), following[-1])
            if case == 'ac4-mixed-authors':
                obsolete = [r['id'] for r in stored if r['data']['review_stage'] == 'aggregate']
                withdrawn = ctx['append']('record-withdrawal', {
                    'record_ids': obsolete, 'reason': LABEL + '; wrong synthetic aggregate author set; replace fully',
                    'attesting_driver': {'name': 'synthetic-driver', 'kind': 'agent'}})
                require(withdrawn['status'] == 'completed', withdrawn)
                dump(ctx['work'] / 'repair-withdrawal.json', withdrawn)
        # Legitimate new complete returns, same two independent authors at both stages.
        # New IDs, no record overwrite/override/reset, all in current ordinary review phase.
        for key, batch in grouped:
            for author in normal:
                append_candidates(ctx, candidates(ctx, gate, subject, batch, author))
        ledger(ctx, gate, subject, revision)
        before_allow = snapshot(ctx)
        allowed = ctx['event'](event_name(ctx))
        after_allow = snapshot(ctx)
        checked_allow(before_allow, allowed, after_allow)
        require(after_allow['show']['result']['current_state'] == 'plan', allowed)
        old_by_id = {r['id']: r for r in before_allow['show']['result']['context']}
        now_by_id = {r['id']: r for r in after_allow['show']['result']['context']}
        require(all(now_by_id.get(k) == v for k, v in old_by_id.items()), 'Repair lost original records')
        dump(ctx['work'] / 'allow.json', {'before': before_allow, 'response': allowed, 'after': after_allow})
        finished.add(case)
        return True
    return hook


def ac5_hook(finished):
    steering = {}
    checked = set()
    def hook(state, ctx):
        if state not in ('implementation-review', 'validation-review'):
            return False
        if not steering:
            specs = [('old-all', {'kind': 'all'}, None),
                     ('all-current', {'kind': 'all'}, 'old-all'),
                     ('old-implementation', {'kind': 'slots', 'ids': ['implementation-review']}, None),
                     ('implementation-current', {'kind': 'slots', 'ids': ['implementation-review']}, 'old-implementation'),
                     ('old-validation', {'kind': 'slots', 'ids': ['validation-review']}, None),
                     ('validation-current', {'kind': 'slots', 'ids': ['validation-review']}, 'old-validation'),
                     ('unrelated-design', {'kind': 'slots', 'ids': ['design-draft']}, None)]
            for label, target, prior in specs:
                data = {'target': target, 'instruction': LABEL + '; steering fixture ' + label}
                if prior:
                    data['supersedes'] = [steering[prior]['id']]
                response = ctx['append']('user-steering', data)
                require(response['status'] == 'completed', response)
                steering[label] = response['result']['context']
                require(steering[label]['data'] == data, response)
            dump(ctx['work'] / 'steering-stored.json', steering)
        subject = 'implementation-report.json' if state == 'implementation-review' else 'validation-report.json'
        matching = 'implementation-current' if state == 'implementation-review' else 'validation-current'
        expected = {steering[label]['id'] for label in ('all-current', matching)}
        omissions = {r['id'] for r in steering.values()} - expected
        for stage in ('individual', 'aggregate'):
            before = snapshot(ctx)
            packet = commission(ctx, state, stage)
            after = snapshot(ctx)
            dump(ctx['work'] / f'{state}-{stage}-packet.json',
                 {'before': before, 'packet': packet, 'after': after,
                  'exact_selected_ids': sorted(expected), 'exact_omitted_ids': sorted(omissions)})
            require(not packet.get('missing_inputs', []), packet)
            rows = [r for r in packet['context'] if r['kind'] == 'user-steering']
            require({r['id'] for r in rows} == expected and len(rows) == 2, packet)
            actual = {r['id']: r for r in after['show']['result']['context']}
            require(all(actual[r['id']] == r for r in rows), 'Packet changed actual stored steering bytes/data')
            require(all(actual[r['id']] == r for r in steering.values()), 'History lost omitted steering')
            require(set(packet['commission']['steering_ids']) == expected, packet)
            sources = {s['record_id'] for s in packet['review_record_sources']}
            require(expected <= sources and not (omissions & sources), packet)
            invariant(before, after)
            require(before['history'] == after['history'], 'Commission mutated durable history')
        value = json.loads((ctx['artifacts'] / subject).read_bytes())
        for key, batch in groups(ctx['profile']['review_policies'][state]):
            for index in range(2):
                author = {'name': f'synthetic-{state}-{index}', 'kind': 'agent'}
                append_candidates(ctx, candidates(ctx, state, subject, batch, author))
        stage_context_controls(ctx, state, subject)
        ledger(ctx, state, subject, value['revision'])
        before = snapshot(ctx)
        allowed = ctx['event'](event_name(ctx))
        after = snapshot(ctx)
        checked_allow(before, allowed, after)
        dump(ctx['work'] / f'{state}-allow.json', {'before': before, 'response': allowed, 'after': after})
        checked.add(state)
        if checked == {'implementation-review', 'validation-review'}:
            finished.add('ac5-review-steering')
        return True
    return hook



def run_case(args, journey, case):
    import shutil
    profile_source = args.source_root / 'crates/software-change-provider/data/configs/high-rigor.json'
    original = json.loads(profile_source.read_bytes())
    finished = set()
    root = args.output_root
    log = Capture(root / 'commands', args)
    targets = {'design-review'} if case.startswith('ac4') else {'implementation-review', 'validation-review'}
    def configure(profile, repository, artifacts):
        for gate in profile['review_policies']:
            if gate not in targets:
                profile['review_policies'][gate] = []
        require(profile['contract_version'] == original['contract_version'] == 4, profile)
        require(profile['criterion_policy'] == original['criterion_policy'] ==
                {'required_authors': 2, 'goal_required_authors': 2}, profile)
        for gate in targets:
            require(profile['review_policies'][gate] == original['review_policies'][gate], gate)
        templates = artifacts.parent / 'templates'
        shutil.copytree(args.source_root / 'crates/software-change-provider/data/templates', templates)
        profile['extra']['template_root'] = str(templates)
        dump(artifacts.parent / 'synthetic-profile-delta.json', {
            'label': LABEL, 'original_profile_sha256': sha(profile_source.read_bytes()),
            'original_profile': original, 'synthetic_profile': profile,
            'changes': ['artifact_root=test-local', 'template_root=byte-identical test-local copy',
                        'only non-target review policies=[]'],
            'target_gates': sorted(targets), 'target_review_author_floor': 2,
            'criterion_author_floor': 2, 'goal_author_floor': 2,
            'external_roles': 'Python scripted dummy reviewers; no model or human invoked',
            'canonical_advice_bookends': 'disabled/unset; unchanged'})
    hook = ac4_hook(case, finished) if case.startswith('ac4') else ac5_hook(finished)
    stop = 'plan' if case.startswith('ac4') else 'end'
    work = journey.run(args, profile_source, root, configure=configure, exercise=hook,
                       stop_at=stop, command_log=log,
                       judgment_reason=LABEL + '; actual toy proof result inspected')
    require(case in finished, f'Case did not finish: {case}')
    terminal = log.execute([args.engine, '--database', work / 'run.sqlite', '--config',
                  work / 'providers.toml', '--json', 'show', '--view', 'full', 'software-external'],
                  cwd=work / 'repository')
    history = log.execute([args.engine, '--database', work / 'run.sqlite', '--config',
                  work / 'providers.toml', '--json', 'history', 'software-external'],
                  cwd=work / 'repository')
    require(terminal['status'] == history['status'] == 'completed', (terminal, history))
    require(not terminal['result']['has_overrides'], terminal)
    require(terminal['result']['current_state'] == stop, terminal)
    if case == 'ac5-review-steering':
        require(terminal['result']['lifecycle'] == 'final', terminal)
    dump(root / 'case-result.json', {'status': 'pass-mechanics', 'case': case,
        'catalog': str(work / 'run.sqlite'), 'work': str(work),
        'terminal': terminal, 'history': history, 'semantic_acceptance': False})

    log.finalize()
