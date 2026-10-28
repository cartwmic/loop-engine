#!/usr/bin/env python3
"""Black-box external-work outcomes. Scripted reviews establish mechanics only."""
import argparse
import hashlib
import copy
import importlib.util
import json
import os
import shutil
import subprocess
import sys
import sqlite3
import tempfile
import time
from contextlib import contextmanager
from pathlib import Path
from external_work_driver import process

ROOT = Path(__file__).resolve().parents[1]
RETIRED_ENGINE = ['invoke','fan-out','amend-binding','cancel-invocation','invocation-progress',
                  'preview-bindings','capture-command','capture-matrix','capture-abort','advise']
RETIRED_PROVIDER = ['bindings','invoke','run-plan-graph','run-validation','recover-review','repair-review']


def raw(argv, packet=None, cwd=None):
    return subprocess.run([str(a) for a in argv],input=None if packet is None else json.dumps(packet),
                          cwd=cwd,text=True,capture_output=True)


def refused(argv, packet=None, cwd=None, needle=None):
    result = raw(argv,packet,cwd)
    assert result.returncode != 0, result.stdout
    if needle:
        assert needle in result.stdout + result.stderr, result
    return result


@contextmanager


def json_source(work, data):
    # Keep growing evidence out of argv without changing its serialized bytes.
    with tempfile.NamedTemporaryFile(dir=work, suffix='.json') as source:
        source.write(json.dumps(data).encode('utf-8'))
        source.flush()
        yield '@'+source.name


def append(ctx, kind, data, record_id=None):
    args = ['append']
    if record_id:
        args += ['--record-id',record_id]
    with json_source(ctx['work'], data) as source:
        return ctx['call'](*args,ctx['run_id'],kind,source)



def lean_module():
    spec = importlib.util.spec_from_file_location('lean_journey', ROOT / 'scripts/lean-change-journey.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def lean_ctx(args, root, name):
    """A lean-change run in explore with one intent record: a host for engine-generic observation proofs."""
    from external_work_driver import engine_call
    lean = lean_module()
    work = root / name
    work.mkdir()
    repository = work / 'repo'
    repository.mkdir()
    for argv in [['init','-q'],['config','user.email','j@example.invalid'],['config','user.name','j']]:
        lean.git(repository,*argv)
    (repository / 'README.md').write_text('fixture\n')
    lean.git(repository,'add','-A')
    lean.git(repository,'commit','-qm','base')
    profile = work / 'profile.json'
    profile.write_text(json.dumps(process([args.lean,'setup','--profile','c','--repo',repository])))
    config = work / 'providers.toml'
    config.write_text(f'[providers.lean-change]\ncommand = {json.dumps(args.lean)}\nargs = []\n')
    database = work / 'run.sqlite'
    run_id = 'lean-external'
    def call(*argv):
        return engine_call(args.engine,database,config,list(argv))
    def show():
        return call('show','--view','full',run_id)
    def event(name):
        show()
        result = call('event',run_id,name)
        assert result['status'] == 'completed', result
        return result
    ctx = {'engine':args.engine,'provider':args.lean,'database':database,'config':config,'work':work,
           'repository':repository,'run_id':run_id,'call':call,'show':show,'event':event}
    assert call('start','--id',run_id,'lean-change','@'+str(profile),'engine observation host')['status'] == 'completed'
    show()
    assert append(ctx,'intent',lean.intent(),'intent-1')['status'] == 'completed'
    return ctx


# bookends:LE-176 bookends:LE-119 — retired public commands are rejected and only the primary operations remain; sentinel never runs.
def retirement(args, root):
    for binary, retired in [(args.engine,RETIRED_ENGINE),(args.lean,RETIRED_PROVIDER),
                            (args.policy,RETIRED_PROVIDER),(args.research,RETIRED_PROVIDER)]:
        result = raw([binary,'--help'])
        if binary == args.engine:
            assert result.returncode == 0, result.stderr
        elif result.returncode:
            assert 'unsupported command' in result.stderr, result.stderr
        words = result.stdout.replace(',',' ').split()
        assert not set(words).intersection(retired), result.stdout
        if binary != args.engine:
            for command in retired:
                refused([binary,command],{})
    profile = json.loads((ROOT / 'crates/research-provider/data/configs/standard.json').read_text())
    profile['artifact_root'] = str(root / 'artifacts')
    (root / 'artifacts').mkdir()
    profile['work_slot_bindings'] = {'gather':{'command':'pi','args':[]}}
    selected = root / 'profile.json'
    selected.write_text(json.dumps(profile))
    config = root / 'providers.toml'
    config.write_text(f'[providers.research]\ncommand = {json.dumps(args.research)}\nargs = []\n')
    refused([args.engine,'--database',root / 'run.sqlite','--config',config,'--json','start',
             '--id','retired-bindings','research','@'+str(selected)],needle='binding')
    print('external-work retirement passed')


# bookends:LE-167 bookends:LE-85 — external policy/research judgments complete normally without an engine executor or any binding.
def provider_specific(args, root):
    import hashlib
    from external_work_driver import engine_call, record_reviews, declare_target_authorship
    spec = importlib.util.spec_from_file_location('research_fixture', ROOT / 'scripts/research-journey.py')
    research = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(research)
    spec = importlib.util.spec_from_file_location('policy_fixture', ROOT / 'scripts/policy-document-journey.py')
    policy_fixture = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(policy_fixture)
    for kind, binary in [('policy-document',args.policy),('research',args.research)]:
        work = root / (kind+'-specific')
        work.mkdir()
        artifacts = work / 'artifacts'
        artifacts.mkdir()
        owner = {'name':'owner','kind':'human'}
        if kind == 'policy-document':
            profile = json.loads((ROOT / 'crates/policy-document-provider/data/readme.json').read_text())
            target = work / 'README.md'
            target.write_text(policy_fixture.document())
            profile.update(mode='audit')
            profile['target']['path'] = str(target)
            profile['semantic_policies'] = profile['semantic_policies'][:2]
            policy = profile['semantic_policies'][0]
            gate, event, subject = 'semantic-review','passed',profile['target']['id']
            revision = hashlib.sha256(target.read_bytes()).hexdigest()
        else:
            profile = json.loads((ROOT / 'crates/research-provider/data/configs/standard.json').read_text())
            for gate in ['verify','synthesize']:
                profile['review_policies'][gate] = profile['review_policies'][gate][:2] if gate == 'verify' else [profile['review_policies'][gate][0]]
            policy = profile['review_policies']['verify'][0]
            gate, event, subject, revision = 'verify','verified','verification.json','1'
            for name, make in [('brief.json',research.brief),('sources.json',research.sources),('verification.json',research.verification)]:
                (artifacts / name).write_text(json.dumps(make()))
            target = artifacts / 'verification.json'
        grouped = profile['semantic_policies'] if kind == 'policy-document' else profile['review_policies']['verify']
        for member in grouped:
            member.update(required_authors=1,self_review=False,group='combined-public')
        profile['artifact_root'] = str(artifacts)
        selected = work / 'profile.json'
        selected.write_text(json.dumps(profile))
        config = work / 'providers.toml'
        config.write_text(f'[providers.{kind}]\ncommand = {json.dumps(binary)}\nargs = []\n')
        database = work / 'run.sqlite'
        run_id = kind+'-external-specific'
        def call(*argv):
            return engine_call(args.engine,database,config,list(argv))
        def show():
            return call('show','--view','full',run_id)
        def move(event):
            show()
            return call('event',run_id,event)
        def record(kind,data,id):
            with json_source(work, data) as source:
                result = call('append','--record-id',id,run_id,kind,source)
            if kind == 'review-evidence':
                retained = next(row for row in show()['result']['context'] if row['id'] == id)
                assert retained['data']['external_review'] == data['external_review'], retained
            return result
        call('start','--id',run_id,kind,'@'+str(selected),'provider-specific external evidence')
        if kind == 'policy-document':
            move('ready')
            move('passed')
        else:
            move('scoped')
            move('gathered')
        def intake(author):
            assignment_subject = revision if kind == 'policy-document' else subject
            packet = {'show':show(),'assignment':{'gate':gate,'review_stage':'ordinary','subject':assignment_subject,
                      'subject_revision':revision,'policy_ids':[p['id'] for p in grouped],'assessment_id':'combined-'+author['name']},'author':author,
                      'request':{'text':'Review the current target.'},'result':{'text':json.dumps({'items':[{'policy_id':p['id'],'result':'pass','findings':''} for p in grouped]})}}
            result = process([binary,'review-candidates'],packet)
            # Public combined assignment refusal and faithful correction.
            subset = copy.deepcopy(packet)
            subset['assignment']['policy_ids'] = [policy['id']]
            refused([binary,'review-candidates'],subset,needle='whole review group')
            corrected = copy.deepcopy(packet)
            corrected['projection'] = json.loads(packet['result']['text'])
            # Canonical policy identity cannot contradict the retained original axis.
            alias_conflict = copy.deepcopy(packet)
            axis_original = json.loads(packet['result']['text'])
            for row in axis_original['items']:
                row['axis'] = row.pop('policy_id')
            alias_conflict['result']['text'] = json.dumps(axis_original)
            alias_conflict['projection'] = copy.deepcopy(axis_original)
            alias_conflict['projection']['items'][0]['policy_id'] = grouped[1]['id']
            before_alias = show()['result']['context']
            refused([binary,'review-candidates'],alias_conflict,needle='projection axis and policy_id disagree')
            # A prose original has no decoded rows; its projection still cannot name two obligations.
            prose_conflict = copy.deepcopy(packet)
            prose_conflict['result']['text'] = 'Prose review: every grouped obligation passes.'
            prose_conflict['projection'] = {'items':[{'axis':grouped[0]['id'],'policy_id':grouped[1]['id'],'result':'pass','findings':''}]
                                            +[{'policy_id':p['id'],'result':'pass','findings':''} for p in grouped[1:]]}
            refused([binary,'review-candidates'],prose_conflict,needle='projection axis and policy_id disagree')
            assert show()['result']['context'] == before_alias
            corrected['projection']['items'][0]['locator'] = 'corrected-locator'
            assert len(process([binary,'review-candidates'],corrected)['candidates']) == len(grouped)
            corrected['projection']['items'][0]['result'] = 'fail'
            refused([binary,'review-candidates'],corrected,needle='contradicts decoded result')
            corrected['projection']['items'][0]['result'] = 'pass'
            corrected['projection']['items'][0]['findings'] = 'invented finding'
            refused([binary,'review-candidates'],corrected,needle='contradicts decoded findings')
            failed_original = copy.deepcopy(packet)
            decoded = json.loads(failed_original['result']['text'])
            decoded['items'][0].update(result='fail',findings='material defect')
            failed_original['result']['text'] = json.dumps(decoded)
            failed_original['projection'] = json.loads(packet['result']['text'])
            refused([binary,'review-candidates'],failed_original,needle='contradicts decoded result')
            assert len(result['candidates']) == len(grouped), result
            for candidate in result['candidates']:
                exchange = candidate['data']['external_review']
                assert exchange['request']['original'] == packet['request']['text'], candidate
                assert exchange['original']['original'] == packet['result']['text'], candidate
            return [c['data'] for c in result['candidates']]
        if kind == 'policy-document':
            missing = process([binary, 'commission', json.dumps(profile)], show())
            assert missing['missing_inputs'][0]['input'] == 'target-authorship', missing
            # Complete independent reviews cannot bypass a missing authorship declaration.
            for i, data in enumerate(intake({'name':'external-independent','kind':'agent'})):
                record('review-evidence', data, 'missing-authorship-'+str(i))
            show()
            refused([args.engine,'--database',database,'--config',config,'--json','event',run_id,event],
                    needle='missing-target-authorship')
            declare_target_authorship(Path(args.engine), database, config, run_id, target,
                                      owner, 'authorship-original')
            commissioned = process([binary, 'commission', json.dumps(profile)], show())
            assert commissioned['missing_inputs'] == [], commissioned
            # A different actor rewrites the bytes; the former declaration is now stale.
            target.write_text(target.read_text()+'\n<!-- authored revision -->\n')
            revision = hashlib.sha256(target.read_bytes()).hexdigest()
            owner = {'name':'revision-author','kind':'agent'}
            for i, data in enumerate(intake(owner)):
                record('review-evidence', data, 'stale-authorship-'+str(i))
            show()
            refused([args.engine,'--database',database,'--config',config,'--json','event',run_id,event],
                    needle='missing-target-authorship')
            stale = process([binary, 'commission', json.dumps(profile)], show())
            assert stale['missing_inputs'][0]['input'] == 'target-authorship', stale
            declare_target_authorship(Path(args.engine), database, config, run_id, target,
                                      owner, 'authorship-revision')
            show()
            refused([args.engine,'--database',database,'--config',config,'--json','event',run_id,event],
                    needle='self-excluded')
        for i, data in enumerate(intake(owner)):
            record('review-evidence',data,'self-evidence-'+str(i))
        show()
        refused([args.engine,'--database',database,'--config',config,'--json','event',run_id,event],needle='review-incomplete')
        independent = intake({'name':'external-independent','kind':'agent'})
        for i, data in enumerate(independent):
            singleton = copy.deepcopy(data)
            singleton['assessment_id'] = 'isolated-'+str(i)
            record('review-evidence',singleton,'isolated-'+str(i))
        show()
        refused([args.engine,'--database',database,'--config',config,'--json','event',run_id,event],needle='review-incomplete')
        for i, data in enumerate(independent):
            record('review-evidence',data,'independent-evidence-'+str(i))
        def amend(after,id):
            data = {'targets':[{'gate':gate,'axis':policy['id']}],'after':after,
                    'actor':owner,'authority':{'kind':'owner','reference':'scripted instruction'},'reason':'External review budget.'}
            candidate = process([binary,'author-counts','--propose',json.dumps(data)],show())
            record(candidate['kind'],candidate['data'],id)
        amend(2,'increase')
        show()
        refused([args.engine,'--database',database,'--config',config,'--json','event',run_id,event],needle='review-incomplete')
        amend(1,'decrease')
        if kind == 'policy-document':
            target.write_text(target.read_text()+'\n<!-- immaterial fixture comment -->\n')
            revision = hashlib.sha256(target.read_bytes()).hexdigest()
            declare_target_authorship(Path(args.engine), database, config, run_id, target,
                                      owner, 'authorship-final-revision')
        else:
            value = json.loads(target.read_text())
            value['revision'] = '2'
            target.write_text(json.dumps(value))
            revision = '2'
        show()
        refused([args.engine,'--database',database,'--config',config,'--json','event',run_id,event],needle='review-incomplete')
        # An older carried PASS cannot erase this author's current FAIL.
        failure_packet = {'show':show(), 'assignment':{'gate':gate,'review_stage':'ordinary',
                          'subject':revision if kind == 'policy-document' else subject,
                          'subject_revision':revision,'policy_ids':[p['id'] for p in grouped],
                          'assessment_id':'current-failure'},
                          'author':{'name':'external-independent','kind':'agent'},
                          'request':{'text':'Review the current revision for remaining defects.'},
                          'result':{'text':json.dumps({'items':[{'policy_id':p['id'],'result':'fail',
                          'findings':'standing current-revision defect'} for p in grouped]})}}
        for i, candidate in enumerate(process([binary,'review-candidates'],failure_packet)['candidates']):
            record(candidate['kind'],candidate['data'],'current-failure-'+str(i))
        for i in range(len(grouped)):
            record('evidence-applicability',{'origin':{'kind':'context-record','id':'independent-evidence-'+str(i)},
                   'target':{'subject':subject,'revision':revision},'attesting_driver':owner,
                   'reason':'Only immaterial fixture metadata changed; judgment scope remains unchanged.'},'carry-independent-'+str(i))
        show()
        refused([args.engine,'--database',database,'--config',config,'--json','event',run_id,event],
                needle='standing current-revision defect')
        # Correct the defect and obtain fresh confirmation, preserving all historical records.
        if kind == 'policy-document':
            target.write_text(target.read_text()+'\n<!-- corrected current-revision fixture defect -->\n')
            revision = hashlib.sha256(target.read_bytes()).hexdigest()
            declare_target_authorship(Path(args.engine), database, config, run_id, target,
                                      owner, 'authorship-corrected')
        else:
            value = json.loads(target.read_text())
            value['revision'] = '3'
            target.write_text(json.dumps(value))
            revision = '3'
        for i, data in enumerate(intake({'name':'external-independent','kind':'agent'})):
            record('review-evidence',data,'fresh-confirmation-'+str(i))
        move(event)
        if kind == 'research':
            value = research.report()
            value['verification_revision'] = revision
            (artifacts / 'report.json').write_text(json.dumps(value))
            record_reviews(Path(args.engine),database,config,Path(binary),run_id,'synthesize',
                           [p['id'] for p in profile['review_policies']['synthesize']],'report.json','1')
            move('completed')
        terminal = show()['result']
        assert terminal['current_state'] == 'end' and terminal['lifecycle'] == 'final' and not terminal['has_overrides'], terminal
        print(f'external-work {kind} self-exclusion amendment applicability normal completion passed')


def decode_retained(context):
    """Every retained artifact entry, in order, with its exact bytes (None when absent)."""
    import base64
    by_id = {record['id']: record for record in context}
    entries = []
    for record in sorted((r for r in context if r['kind'] == 'retained-artifacts'), key=lambda r: r['sequence']):
        assert record['data']['schema_version'] == 1, record
        for entry in record['data']['artifacts']:
            data = None
            if not entry.get('absent'):
                source = entry
                if 'same_as' in entry:
                    holder = by_id[entry['same_as']]
                    source = next(x for x in holder['data']['artifacts'] if x['name'] == entry['name'])
                    assert 'content' in source, source
                    assert 'content' not in entry, entry
                data = source['content'].encode() if source['encoding'] == 'utf-8' else base64.b64decode(source['content'])
                assert 'sha256:'+hashlib.sha256(data).hexdigest() == entry['sha256'], entry
                assert len(data) == entry['size'], entry
            entries.append({'record': record['id'], 'sequence': record['sequence'], 'created_at': record['created_at'],
                            'transition': record['data']['transition'], 'exit': record['data']['exit'],
                            'name': entry['name'], 'bytes': data, 'same_as': entry.get('same_as')})
    return entries


def assert_retained_history(call, run_id, entries):
    """Each retained record is a context entry immediately followed by the transition that retained it."""
    history = call('history', run_id)['result']
    by_sequence = {entry['sequence']: entry for entry in history}
    for entry in entries:
        appended = by_sequence[entry['sequence']]['action']
        assert appended == {'kind': 'context_appended', 'context_record_id': entry['record']}, appended
        following = by_sequence[entry['sequence'] + 1]['action']
        assert following['kind'] == 'transition', following
        assert {k: following['transition'][k] for k in ('source', 'event', 'target')} == entry['transition'], following
        expected = 'overridden' if entry['exit'] == 'override' else 'committed'
        assert following['outcome']['outcome'] == expected, following
        assert by_sequence[entry['sequence']]['occurred_at'] == entry['created_at']


def retained_view(call, run_id):
    shown = call('show', '--view', 'full', run_id)
    entries = decode_retained(shown['result']['context'])
    assert_retained_history(call, run_id, entries)
    return entries


# bookends:LE-186 — research and policy-document runs retain each owning phase's exact artifact bytes, readable after the files are deleted.
def retention_provider_runs(args, root):
    import shutil
    from external_work_driver import engine_call
    expectations = {
        'research': [('scope', 'scoped', 'brief.json'), ('gather', 'gathered', 'sources.json'),
                     ('verify', 'verified', 'verification.json'), ('synthesize', 'completed', 'report.json')],
        'policy-document': [('semantic-review', 'passed', 'README.md')],
    }
    for kind, exits in expectations.items():
        work = root / (kind + '-specific')
        database, config, run_id = work / 'run.sqlite', work / 'providers.toml', kind + '-external-specific'
        def call(*argv):
            return engine_call(args.engine, database, config, list(argv))
        files = {'README.md': work / 'README.md'} if kind == 'policy-document' else {
            name: work / 'artifacts' / name for _, _, name in exits}
        before = retained_view(call, run_id)
        assert [(e['transition']['source'], e['transition']['event'], e['name']) for e in before] == exits, before
        for entry in before:
            assert entry['exit'] == 'checked' and entry['same_as'] is None, entry
            assert entry['bytes'] == files[entry['name']].read_bytes(), entry['name']
        shutil.rmtree(work / 'artifacts')
        if kind == 'policy-document':
            files['README.md'].unlink()
        after = retained_view(call, run_id)
        assert [(e['name'], e['bytes'], e['transition']) for e in after] == [(e['name'], e['bytes'], e['transition']) for e in before]
        print(f'external-work {kind} retained artifacts readable after file deletion passed')


def _seed_legacy_invocations(database, run_id, rows, payload_bytes):
    """Insert legacy-format work-slot invocation rows with bulky payloads, as earlier releases wrote them."""
    worker = {'assignment_id': 'worker-0', 'command': '/legacy/bin/pi',
              'args': ['--print', 'x' * payload_bytes], 'exit_code': 0, 'attempts': []}
    connection = sqlite3.connect(database, timeout=30)
    with connection:
        for index in range(rows):
            inner = json.dumps([worker])
            connection.execute(
                """INSERT INTO work_slot_invocations (run_id, invocation_id, slot_id, binding_json,
                       instruction_digest, subject, state_visit, waiter_pid, started_at, allowed_time_ms,
                       status, exit_code, completed_at, capture_dir, inner_workers_json, assignment_selection_json,
                       invocation_input_json, routed_inputs_json, frozen_run_identity_json, completion_snapshot_json)
                   VALUES (?, ?, 'legacy-slot', ?, 'sha256:legacy', 'legacy-subject', 0, 0, 1, 1000,
                           'succeeded', 0, 2, '/legacy/capture', ?, NULL, ?, ?, NULL, ?)""",
                (run_id, f'legacy-{run_id}-{index}-{time.time_ns()}',
                 json.dumps({'command': '/legacy/bin/pi', 'args': ['--print']}),
                 inner, json.dumps({'prompt': 'p' * 1000}), '[]', inner))
    connection.close()


def _invocation_columns(database, run_id):
    connection = sqlite3.connect(database, timeout=30)
    try:
        return connection.execute(
            """SELECT invocation_id, binding_json, inner_workers_json, completion_snapshot_json, routed_inputs_json,
                      invocation_input_json, assignment_selection_json, frozen_run_identity_json
               FROM work_slot_invocations WHERE run_id = ? ORDER BY invocation_id""", (run_id,)).fetchall()
    finally:
        connection.close()


def _file_digest(path):
    # A missing write-ahead log and an empty one hold the same catalog content.
    return hashlib.sha256(path.read_bytes() if path.exists() else b'').hexdigest()


def _interrupt_reclaim_apply(engine, database, run_id):
    """Observe a real WAL apply in progress, then require signal-caused termination."""
    import signal
    holder = sqlite3.connect(database, timeout=30, isolation_level=None)
    observer = sqlite3.connect(database, timeout=1, isolation_level=None)
    victim = None
    try:
        assert holder.execute('PRAGMA journal_mode').fetchone() == ('wal',)
        assert holder.execute('PRAGMA wal_checkpoint(TRUNCATE)').fetchone()[0] == 0
        main_before = database.stat().st_size
        holder.execute('BEGIN')
        holder.execute('SELECT count(*) FROM work_slot_invocations').fetchone()
        pending = "SELECT count(*) FROM work_slot_invocations WHERE run_id=? AND inner_workers_json!='[]'"
        assert observer.execute(pending, (run_id,)).fetchone()[0] > 0
        victim = subprocess.Popen([str(engine), '--database', str(database), '--json', 'reclaim', '--apply'],
                                  stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        # The pinned reader prevents successful checkpoint/shrink. Wait for visible
        # committed clearing, not elapsed-time guessing or mere process existence.
        deadline = time.monotonic() + 4
        while True:
            assert victim.poll() is None, 'apply exited before an interrupted write was observed'
            if observer.execute(pending, (run_id,)).fetchone()[0] == 0:
                break
            assert time.monotonic() < deadline, 'no committed apply write observed'
            time.sleep(0.01)
        wal_bytes = Path(str(database)+'-wal').stat().st_size
        assert wal_bytes > 0 and database.stat().st_size == main_before
        assert victim.poll() is None, 'apply must still be in progress before SIGKILL'
        victim.send_signal(signal.SIGKILL)
        status = victim.wait(timeout=10)
        assert status == -signal.SIGKILL, f'apply was not interrupted by SIGKILL: {status}'
        return {'stage': 'committed clearing, pinned checkpoint unfinished',
                'wal_bytes': wal_bytes, 'main_file_unchanged': True, 'exit_code': status}
    finally:
        if victim is not None and victim.poll() is None:
            victim.kill()
            victim.wait(timeout=10)
        observer.close()
        if holder.in_transaction:
            holder.execute('ROLLBACK')
        holder.close()


# bookends:LE-100 — show reports a deterministic, provider-free change report for retained assignment records; unknown inputs are changed.
def legacy_change_report(args, root):
    work = root / 'legacy-change-report'
    work.mkdir()
    database = work / 'run.sqlite'
    shutil.copyfile(root / 'research-specific' / 'run.sqlite', database)
    run_id = 'research-external-specific'
    _seed_legacy_invocations(database, run_id, 1, 10)
    offline = work / 'offline.toml'
    offline.write_text('[providers.research]\ncommand = "/nonexistent/research"\nargs = []\n')
    def report():
        observed = raw([args.engine,'--database',database,'--config',offline,'--json','show','--view','full',run_id])
        assert observed.returncode == 0, observed.stderr
        shown = json.loads(observed.stdout)['result']
        return shown['change_report'], [row['change_report'] for row in shown['work_slot_invocations']]
    first, second = report(), report()
    assert first == second, 'change report is deterministic'
    summary, rows = first
    assert [row['assignment_id'] for row in summary['assignments']] == ['worker-0'] and summary['plan_task_results'] == []
    dimensions = rows[0]['dimensions']
    assert dimensions['governing_policy_configuration']['changed'] is True and dimensions['governing_policy_configuration']['recorded'] is None
    assert dimensions['worker_assignment']['changed'] is False and rows[0]['standing'] is False, rows[0]
    print('external-work legacy change report deterministic and provider-free passed')


# bookends:LE-187 — explicit reclaim previews without writing, clears only finished runs' legacy payloads, keeps cleaned runs listed and showable, leaves active runs advancing, and shrinks the catalog file.
def reclaim_legacy_catalog(args, root):
    import shutil, signal
    from external_work_driver import engine_call
    work = root / 'reclaim'
    work.mkdir()
    database = work / 'loop.db'
    retained_work = root / 'research-specific'
    shutil.copyfile(retained_work / 'run.sqlite', database)
    config = retained_work / 'providers.toml'
    finished = 'research-external-specific'
    def call(*argv):
        return engine_call(args.engine, database, config, list(argv))
    def engine(*argv):
        return raw([args.engine, '--database', database, '--config', config, '--json', *argv])
    profile = json.loads((retained_work / 'profile.json').read_text())
    active_artifacts = work / 'active-artifacts'
    active_artifacts.mkdir()
    shutil.copyfile(retained_work / 'profile.json', work / 'active-profile.json')
    spec = importlib.util.spec_from_file_location('reclaim_research_fixture', ROOT / 'scripts/research-journey.py')
    research = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(research)
    (active_artifacts / 'brief.json').write_text(json.dumps(research.brief()))
    profile['artifact_root'] = str(active_artifacts)
    (work / 'active-profile.json').write_text(json.dumps(profile))
    call('start', '--id', 'active-run', 'research', '@'+str(work / 'active-profile.json'), 'active during reclaim')
    call('start', '--id', 'terminated-run', 'research', '@'+str(work / 'active-profile.json'), 'terminated before reclaim')
    call('show', 'terminated-run', '--view', 'action')
    assert call('terminate', 'terminated-run')['status'] == 'completed'
    for run_id in [finished, 'terminated-run', 'active-run']:
        _seed_legacy_invocations(database, run_id, 4, 400_000)
    retained_before = retained_view(call, finished)
    for run_id in [finished, 'terminated-run', 'active-run']:
        assert call('show', '--view', 'full', run_id)['status'] == 'completed'
    active_before = _invocation_columns(database, 'active-run')
    digest_before = (_file_digest(database), _file_digest(Path(str(database)+'-wal')))
    preview = call('reclaim')
    assert preview['status'] == 'completed' and preview['result']['mode'] == 'preview', preview
    listed = {run['run_id']: run for run in preview['result']['preview']['runs']}
    assert set(listed) == {finished, 'terminated-run'}, listed
    assert all(run['invocation_rows'] == 4 and run['reclaimable_bytes'] > 3_000_000 for run in listed.values()), listed
    assert (_file_digest(database), _file_digest(Path(str(database)+'-wal'))) == digest_before, 'preview must not write'
    human = raw([args.engine, '--database', database, 'reclaim'])
    assert human.returncode == 0 and 'nothing written' in human.stdout, human
    print('external-work reclaim preview lists only finished runs and writes nothing passed')
    applied = call('reclaim', '--apply')
    assert applied['status'] == 'completed', applied
    result = applied['result']['applied']
    assert result['invocation_rows_cleared'] == 8, result
    assert result['after']['file_bytes'] < result['before']['file_bytes'] - 3_000_000, result
    assert database.stat().st_size == result['after']['file_bytes']
    for run_id in [finished, 'terminated-run']:
        for row in _invocation_columns(database, run_id):
            assert row[2:] == ('[]', '[]', '[]', None, None, None), row
            assert json.loads(row[1])['command'], row
    assert _invocation_columns(database, 'active-run') == active_before, 'active run rows must be unchanged'
    listed_runs = {run['id'] for run in call('list')['result']}
    assert {finished, 'terminated-run', 'active-run'} <= listed_runs, listed_runs
    retained_after = retained_view(call, finished)
    assert [(e['name'], e['bytes']) for e in retained_after] == [(e['name'], e['bytes']) for e in retained_before]
    call('show', 'active-run', '--view', 'action')
    assert call('event', 'active-run', 'scoped')['status'] == 'completed'
    assert sqlite3.connect(database).execute('PRAGMA integrity_check').fetchone() == ('ok',)
    print('external-work reclaim apply clears finished runs only, keeps them showable and shrinks the file passed')

    # Concurrent driver command while reclaim rewrites the file: it completes or fails with a retryable busy error.
    _seed_legacy_invocations(database, 'terminated-run', 6, 2_000_000)
    reclaim_process = subprocess.Popen([args.engine, '--database', str(database), '--json', 'reclaim', '--apply'],
                                       stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    concurrent = engine('show', 'active-run', '--view', 'full')
    reclaim_out, _ = reclaim_process.communicate(timeout=300)
    assert reclaim_process.returncode == 0, reclaim_out
    if concurrent.returncode != 0:
        assert 'catalog-busy' in concurrent.stdout and 'retry' in concurrent.stdout, concurrent
    assert call('show', '--view', 'full', 'active-run')['status'] == 'completed'
    print('external-work reclaim alongside a concurrent driver command passed')

    # Pin a reader's WAL snapshot across clearing and VACUUM. A busy checkpoint returns a row,
    # not a SQLite exception; apply must refuse success until the rewritten pages reach the main file.
    _seed_legacy_invocations(database, 'terminated-run', 4, 400_000)
    holder = sqlite3.connect(database, timeout=30, isolation_level=None)
    assert holder.execute('PRAGMA wal_checkpoint(TRUNCATE)').fetchone()[0] == 0
    main_before = database.stat().st_size
    holder.execute('BEGIN')
    holder.execute('SELECT inner_workers_json FROM work_slot_invocations').fetchall()
    active_before = _invocation_columns(database, 'active-run')
    try:
        pinned = engine('reclaim', '--apply')
        assert pinned.returncode != 0, pinned.stdout
        issue = json.loads(pinned.stdout)
        assert 'catalog-busy' in json.dumps(issue) and 'retry' in json.dumps(issue), issue
        assert database.stat().st_size == main_before, 'pinned snapshot keeps the old main file'
        assert Path(str(database)+'-wal').stat().st_size > 0
    finally:
        holder.execute('ROLLBACK')
        holder.close()
    assert _invocation_columns(database, 'active-run') == active_before
    assert sqlite3.connect(database).execute('PRAGMA integrity_check').fetchone() == ('ok',)
    retried = call('reclaim', '--apply')
    assert retried['status'] == 'completed', retried
    assert database.stat().st_size < main_before - 2_000_000
    assert not Path(str(database)+'-wal').exists() or Path(str(database)+'-wal').stat().st_size == 0
    assert call('show', 'active-run', '--view', 'full')['status'] == 'completed'
    print('external-work reclaim refuses a pinned WAL checkpoint and shrinks on retry passed')

    # Ordinary mutation under a write lock must also explain that contention is retryable.
    holder = sqlite3.connect(database, timeout=30, isolation_level=None)
    holder.execute('BEGIN IMMEDIATE')
    try:
        ordinary_busy = engine('append', 'active-run', 'user-steering',
                               json.dumps({'target': {'kind': 'all'}, 'instruction': 'held lock probe'}))
        assert ordinary_busy.returncode != 0, ordinary_busy.stdout
        issue = json.loads(ordinary_busy.stdout)
        assert 'catalog-busy' in json.dumps(issue) and 'retry' in json.dumps(issue), issue
    finally:
        holder.execute('ROLLBACK')
        holder.close()
    assert call('append', 'active-run', 'user-steering',
                json.dumps({'target': {'kind': 'all'}, 'instruction': 'after held lock'}))['status'] == 'completed'
    print('external-work ordinary command reports retryable catalog contention passed')

    # Another process holds the catalog's write lock: reclaim stops with a clear busy error and changes nothing.
    _seed_legacy_invocations(database, 'terminated-run', 1, 100_000)
    holder = sqlite3.connect(database, timeout=30, isolation_level=None)
    holder.execute('BEGIN IMMEDIATE')
    busy = raw([args.engine, '--database', database, '--json', 'reclaim', '--apply'])
    holder.execute('ROLLBACK')
    holder.close()
    assert busy.returncode != 0, busy
    busy_issue = json.loads(busy.stdout)
    assert 'catalog-busy' in json.dumps(busy_issue) and 'retry' in json.dumps(busy_issue), busy_issue
    assert any(row[2] != '[]' for row in _invocation_columns(database, 'terminated-run')), 'busy reclaim must not clear rows'
    print('external-work reclaim reports a held catalog as busy passed')

    # Interrupt an observed real apply after clearing commits, before pinned WAL shrink finishes.
    _seed_legacy_invocations(database, 'terminated-run', 6, 2_000_000)
    active_before = _invocation_columns(database, 'active-run')
    interruption = _interrupt_reclaim_apply(args.engine, database, 'terminated-run')
    (work / 'interruption.json').write_text(json.dumps(interruption, indent=2))
    assert sqlite3.connect(database).execute('PRAGMA integrity_check').fetchone() == ('ok',)
    assert _invocation_columns(database, 'active-run') == active_before
    assert {finished, 'terminated-run', 'active-run'} <= {run['id'] for run in call('list')['result']}
    assert call('show', '--view', 'full', 'active-run')['status'] == 'completed'
    # Advance through whatever normal transition the active run's profile offers now
    # (review-enabled and review-free profiles reach different states before this point).
    observed = call('show', 'active-run', '--view', 'action')['result']
    offered = observed['requestable_events']
    step = next((e for e in offered if e['kind'] == 'check-free'), None) or next(e for e in offered if e['kind'] == 'checked')
    advanced = call('event', 'active-run', step['event'])
    assert advanced['status'] == 'completed', advanced
    assert advanced['result']['run']['current_state'] == step['target'], (observed['current_state'], step, advanced)
    rerun = call('reclaim', '--apply')
    assert rerun['status'] == 'completed', rerun
    assert all(row[2] == '[]' for row in _invocation_columns(database, 'terminated-run'))
    print('external-work reclaim SIGKILL interrupts an observed apply; catalog and active progression survive passed')

    # Storage runs out while the catalog is rewritten. A POSIX file-size limit makes the real SQLite write path
    # report full storage deterministically; filling an actual volume needs root or platform disk images.
    _seed_legacy_invocations(database, 'active-run', 4, 2_000_000)
    _seed_legacy_invocations(database, 'terminated-run', 1, 50_000)
    call('show', 'active-run', '--view', 'action')
    sqlite3.connect(database).execute('PRAGMA wal_checkpoint(TRUNCATE)').fetchone()
    # VACUUM must rewrite at least half the remaining catalog, so a half-size cap always
    # trips during the rewrite. A cap just above the current size could be satisfied when
    # cleared bytes outweigh WAL frame overhead, letting apply succeed with no storage failure.
    limit = database.stat().st_size // 2
    tmp = work / 'tmp'
    tmp.mkdir()
    def constrain():
        import resource
        signal.signal(signal.SIGXFSZ, signal.SIG_IGN)
        resource.setrlimit(resource.RLIMIT_FSIZE, (limit, limit))
    environment = dict(os.environ, TMPDIR=str(tmp), SQLITE_TMPDIR=str(tmp))
    full = subprocess.run([str(args.engine), '--database', str(database), '--json', 'reclaim', '--apply'],
                          text=True, capture_output=True, env=environment, preexec_fn=constrain)
    assert full.returncode != 0, full.stdout
    full_issue = json.dumps(json.loads(full.stdout))
    assert 'catalog-storage-full' in full_issue and 'storage' in full_issue, full_issue
    assert sqlite3.connect(database).execute('PRAGMA integrity_check').fetchone() == ('ok',)
    assert {finished, 'terminated-run', 'active-run'} <= {run['id'] for run in call('list')['result']}
    call('show', 'active-run', '--view', 'action')
    assert call('append', 'active-run', 'user-steering', json.dumps({'target': {'kind': 'all'}, 'instruction': 'after full storage'}))['status'] == 'completed'
    assert call('reclaim', '--apply')['status'] == 'completed'
    print('external-work reclaim reports full storage clearly and leaves the catalog usable passed')


# bookends:LE-180 — base-derived setup exports inspectable effective policy with small explicit overrides and visible newer-base drift.
def profile_preparation(args, root):
    work = root / 'profile-preparation'
    work.mkdir()
    for provider, semantic, bases in [(args.policy, True, ['readme','agents']),
                                       (args.research, False, ['standard','generate-prd'])]:
        for base_name in bases:
            prefix = ('policy-' if semantic else 'research-') + base_name
            selected = work / (prefix+'.json')
            base = process([provider,'setup','--rigor',base_name,'--output',selected])
            gate = 'semantic-review' if semantic else 'verify'
            def rows(profile):
                return profile['semantic_policies'] if semantic else profile['review_policies'][gate]
            axis = rows(base)[0]['id']
            options = ['--set',f'review.{gate}.{axis}.required_authors=2',
                       '--set',f'review.{gate}.{axis}.self_review=true',
                       '--set',f'review.{gate}.{axis}.group=custom',
                       '--add-axis',f'{gate}.added={gate}.{axis}']
            custom_path = work / (prefix+'-custom.json')
            custom = process([provider,'setup','--profile',selected,'--output',custom_path,*options])
            assert custom == json.loads(custom_path.read_text())
            assert rows(custom)[0]['required_authors'] == 2 and rows(custom)[0]['self_review']
            assert rows(custom)[0]['group'] == 'custom' and rows(custom)[-1]['id'] == 'added'
            explain_path = Path(str(custom_path)+'.explain.json')
            explain = json.loads(explain_path.read_text())
            assert explain['effective'] == custom
            assert explain['base']['sha256'] == 'sha256:'+hashlib.sha256(selected.read_bytes()).hexdigest()
            changed = copy.deepcopy(base)
            rows(changed)[0]['required_authors'] = 3
            rows(changed)[1]['description'] = 'New selected base default'
            newer = work / (prefix+'-newer.json')
            newer.write_text(json.dumps(changed))
            refreshed_path = work / (prefix+'-refreshed.json')
            refreshed = process([provider,'setup','--profile',newer,'--output',refreshed_path,
                                 '--previous-explain',explain_path,*options])
            drift = json.loads(Path(str(refreshed_path)+'.explain.json').read_text())['drift']
            assert any(d['key'].endswith('.required_authors') and d['base_value'] == 3 for d in drift)
            assert rows(refreshed)[1]['description'] == 'New selected base default'
            removed = process([provider,'setup','--profile',custom_path,'--output',work/(prefix+'-removed.json'),
                               '--remove-axis',f'{gate}.added'])
            assert all(row['id'] != 'added' for row in rows(removed))
            for setting in ['required_authors=0','self_review=maybe','group=""']:
                refused([provider,'setup','--profile',selected,'--output',work/(prefix+'-bad.json'),
                         '--set',f'review.{gate}.{axis}.{setting}'])
            refused([provider,'setup','--profile',newer,'--output',work/(prefix+'-missing.json'),
                     '--set',f'review.{gate}.no-longer-present.required_authors=1'])
    print('external-work base-derived profile preparation and drift passed')


def acknowledge_context_record(packet, record_id, receive):
    """Wait for the complete append sample before asserting unchanged silence.

    Monitor reads workflow and history separately. A concurrent append can first
    appear in history while workflow still reflects the preceding observation.
    Accept only that exact one-record catch-up, not a heartbeat or other change.
    """
    old_context = packet['workflow']['context']
    if any(r['id'] == record_id for r in old_context):
        return packet
    appended = [r for r in packet['judgment']['history']['items']
                if r['action'].get('context_record_id') == record_id]
    assert len(appended) == 1
    complete = receive()
    new_context = complete['workflow']['context']
    assert len(new_context) == len(old_context) + 1
    assert new_context[:-1] == old_context
    assert new_context[-1]['id'] == record_id
    assert new_context[-1]['sequence'] == appended[0]['sequence']
    projected = copy.deepcopy(complete)
    projected['workflow']['context'] = old_context
    def without_clocks(value):
        if isinstance(value, dict):
            return {k:without_clocks(v) for k,v in value.items() if k != 'sampled_at_ms'}
        if isinstance(value, list):
            return [without_clocks(v) for v in value]
        return value
    assert without_clocks(projected) == without_clocks(packet), 'Not an isolated context catch-up'
    return complete


# Real changed packets and unchanged stdout silence describe the assistant-owned posting checkpoint; conversation delivery remains external evidence.
def owner_update_proof(ctx):
    import queue
    import threading
    skill = (ROOT / 'skills/using-loop-engine/SKILL.md').read_text()
    for instruction in ['coordinating assistant', 'concise source-backed owner update',
                        'active Pi conversation', 'meaningful observed change',
                        'before the next wait, inspect, or owner-help decision',
                        'stay quiet for unchanged observations']:
        assert instruction in skill, instruction
    before = ctx['call']('history',ctx['run_id'])
    packets = queue.Queue()
    child = subprocess.Popen([ctx['engine'],'monitor','--run',ctx['run_id'],
                              '--database',str(ctx['database']),'--json','--poll-seconds','0.05'],
                             stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
    def collect():
        for line in child.stdout:
            packets.put(json.loads(line))
    reader = threading.Thread(target=collect,daemon=True)
    reader.start()
    def receive():
        try:
            return packets.get(timeout=5)
        except queue.Empty:
            raise AssertionError('monitor did not emit expected packet')
    def quiet():
        try:
            unexpected = packets.get(timeout=0.4)
        except queue.Empty:
            assert child.poll() is None
            return
        raise AssertionError(('unchanged monitor repeated a packet',unexpected))
    try:
        initial = receive()
        assert initial['event'] == 'snapshot', initial
        quiet()
        assert ctx['call']('history',ctx['run_id']) == before
        append(ctx,'driver-note',{'reason':'Non-meaningful owner-update proof annotation.'},'owner-update-note')
        noted = ctx['call']('history',ctx['run_id'])
        noted_packet = acknowledge_context_record(receive(), 'owner-update-note', receive)
        unchanged = noted_packet['owner_update']
        assert unchanged['required_before_next_decision'] is False, unchanged
        assert unchanged['unchanged_observation'] == 'suppress the heartbeat', unchanged
        quiet()
        assert ctx['call']('history',ctx['run_id']) == noted
        ctx['event']('intent-ready')
        changed_history = ctx['call']('history',ctx['run_id'])
        current = ctx['show']()['result']
        changed = receive()
        deadline = time.monotonic() + 5
        while not changed['owner_update']['required_before_next_decision']:
            assert time.monotonic() < deadline, changed
            changed = receive()
        update = changed['owner_update']
        assert update['required_before_next_decision'] is True, changed
        assert 'workflow' in update['observed_change'], update
        assert changed['workflow_lane']['current_state'] == current['current_state'] == 'intent-approval', changed
        assert update['needed_action_or_decision'] == changed['next_action'], changed
        assert changed['workflow']['requestable_events'] == current['requestable_events'], changed
        assert changed['workflow']['current_state_instructions'] == current['current_state_instructions'], changed
        assert update['channel'] == 'active Pi conversation', update
        assert update['machine_notification_is_not_owner_update'] is True, update
        quiet()
    finally:
        child.terminate()
        child.wait(timeout=5)
        reader.join(timeout=5)
        child.stdout.close()
        child.stderr.close()
    assert ctx['call']('history',ctx['run_id']) == changed_history
    print('external-work owner-update changed packets, unchanged silence and shipped assistant guidance passed')


# bookends:LE-146 — a locked source reports unavailability, never arms mutation, and completes in less than two seconds.
def locked_status(ctx):
    before = ctx['show']()['result']
    lock = sqlite3.connect(ctx['database'])
    lock.execute('PRAGMA journal_mode=DELETE')
    lock.execute('BEGIN EXCLUSIVE')
    try:
        started = time.monotonic()
        observed = raw([ctx['engine'],'--database',ctx['database'],'--json','show','--view','status',ctx['run_id']])
        assert time.monotonic() - started < 2, observed
        assert observed.returncode == 0, observed.stderr
        status = json.loads(observed.stdout)['result']
        assert status['uncertainty']['state'] == 'unavailable' and status['mutation_armed'] is False, status
    finally:
        lock.rollback()
        lock.close()
    assert ctx['show']()['result']['current_state'] == before['current_state']
    print('external-work locked-source status bounded and honest passed')


# bookends:LE-120 bookends:LE-145 — passive snapshots/attention expose workflow and source-backed owner-update guidance while external execution stays unknown and observer shutdown changes no run state.
# bookends:LE-127 bookends:LE-148 — focused action is smaller than retained full detail; public history paging returns explicit cursors/totals without mutation.
def passive_views(ctx):
    import fcntl, pty, select, struct, termios
    append(ctx,'retained-detail',{'text':'original external detail '*3000},'large-retained-detail')
    before = ctx['call']('history',ctx['run_id'])
    full = ctx['show']()
    action = ctx['call']('show','--view','action',ctx['run_id'])
    assert len(json.dumps(action)) < len(json.dumps(full))
    assert 'current_state_instructions' in action['result'] and 'locators' in action['result']
    assert next(row['data']['text'] for row in full['result']['context'] if row['id'] == 'large-retained-detail') == 'original external detail '*3000
    page = ctx['call']('read',ctx['run_id'],'--kind','history','--cursor','0','--limit','2')['result']
    assert len(page['items']) == 2 and page['truncated'] and page['total'] > 2, page
    following = ctx['call']('read',ctx['run_id'],'--kind','history','--cursor',str(page['next_cursor']),'--limit','2')['result']
    assert following['items'][0]['sequence'] > page['items'][-1]['sequence'], following
    # Header, exact-record and list reads find retained detail without the whole-run payload.
    headers = ctx['call']('read',ctx['run_id'],'--kind','context','--record-kind','retained-detail')['result']
    assert [h['id'] for h in headers['items']] == ['large-retained-detail'] and headers['items'][0]['data_bytes'] > 70000, headers
    assert len(json.dumps(headers)) * 20 < len(json.dumps(full)), 'context headers must be far smaller than full show'
    stored = next(row for row in full['result']['context'] if row['id'] == 'large-retained-detail')
    assert ctx['call']('read',ctx['run_id'],'--kind','record','--id','large-retained-detail')['result']['value'] == stored
    assert ctx['call']('read',ctx['run_id'],'--kind','record','--id','large-retained-detail','--pointer','/kind')['result']['value'] == 'retained-detail'
    assert any(e.get('context_kind') == 'retained-detail' for e in ctx['call']('history',ctx['run_id'])['result'])
    assert any(r['id'] == ctx['run_id'] for r in ctx['call']('list','--lifecycle','active')['result'])
    assert not any(r['id'] == ctx['run_id'] for r in ctx['call']('list','--lifecycle','terminated')['result'])
    assert ctx['call']('history',ctx['run_id']) == before
    try:
        subprocess.run([ctx['engine'],'monitor','--run',ctx['run_id'],'--database',ctx['database'],'--json',
                        '--poll-seconds','0.05','--attention-seconds','0.1'],text=True,capture_output=True,timeout=2)
        raise AssertionError('monitor unexpectedly exited')
    except subprocess.TimeoutExpired as stopped:
        output = stopped.stdout.decode() if isinstance(stopped.stdout,bytes) else stopped.stdout
    packets = [json.loads(line) for line in output.splitlines()]
    assert any(p['event'] == 'snapshot' for p in packets) and any(p['event'] == 'attention' for p in packets), packets
    assert all(p['execution']['state'] == 'unknown' for p in packets), packets
    assert any(p['owner_update'].get('machine_notification_is_not_owner_update') for p in packets), packets
    assert ctx['call']('history',ctx['run_id']) == before
    # bookends:LE-147 — a real PTY navigates the frozen provider graph, scrolls details and preserves selection across portrait folded/unfolded resize; q only stops observation.
    master, slave = pty.openpty()
    transcript = bytearray()
    child = None
    def size(fd, rows, columns):
        fcntl.ioctl(fd,termios.TIOCSWINSZ,struct.pack('HHHH',rows,columns,0,0))
    def until(marker, offset=0):
        deadline = time.monotonic()+5
        while marker.encode() not in transcript[offset:]:
            assert time.monotonic() < deadline, (marker,bytes(transcript[-4000:]))
            if select.select([master],[],[],0.1)[0]:
                try:
                    transcript.extend(os.read(master,8192))
                except OSError:
                    assert child.poll() is None, bytes(transcript[-4000:])
    def key(value, marker):
        offset = len(transcript)
        os.write(master,value)
        until(marker,offset)
    try:
        size(slave,34,62)
        child = subprocess.Popen([ctx['engine'],'--database',ctx['database'],'explore',ctx['run_id']],
                                 stdin=slave,stdout=slave,stderr=slave,close_fds=True)
        os.close(slave)
        slave = -1
        until('selected=state:intent-approval')
        until('requestable now')
        until('checked-not-allowed')
        key(b'g','selected=state:explore')
        key(b'j','selected=state:intent-approval')
        key(b']','detail-offset=')
        key(b'[','detail-offset=')
        offset = len(transcript)
        size(master,27,114)
        until('terminal-size=27x114',offset)
        until('selected=state:intent-approval',offset)
        key(b'G','selected=state:end')
        os.write(master,b'q')
        deadline = time.monotonic()+5
        while child.poll() is None:
            assert time.monotonic() < deadline, 'explorer failed to quit'
            if select.select([master],[],[],0.1)[0]:
                try:
                    transcript.extend(os.read(master,8192))
                except OSError:
                    pass
        assert child.wait() == 0, bytes(transcript[-4000:])
    finally:
        if child is not None and child.poll() is None:
            child.kill()
            child.wait()
        os.close(master)
        if slave != -1:
            os.close(slave)
    (ctx['work'] / 'explorer.transcript').write_bytes(transcript)
    assert ctx['call']('history',ctx['run_id']) == before
    print('external-work passive monitor, targeted reads and PTY navigation passed')


# bookends:LE-53 — one run moves between distinct actor processes: observation, update and navigation each run in fresh engine processes.
def engine_observation(args, root):
    ctx = lean_ctx(args, root, 'engine-observation')
    locked_status(ctx)
    owner_update_proof(ctx)
    # A denied checked request leaves durable evaluation history for the navigator to show.
    ctx['show']()
    refused([ctx['engine'],'--database',ctx['database'],'--config',ctx['config'],'--json',
             'event',ctx['run_id'],'approved'],needle='owner')
    passive_views(ctx)


# Policy-document and research prepare inert early briefs; missing inputs stay explicit and nothing advances.
def early_review_briefs(args, root):
    from external_work_driver import engine_call, declare_target_authorship
    def durable(show):
        result = copy.deepcopy(show['result'])
        # Read timestamps are observation metadata, not workflow state or evidence.
        def strip(value):
            if isinstance(value, dict):
                value.pop('observed_at', None)
                value.pop('sampled_at_ms', None)
                for child in value.values():
                    strip(child)
            elif isinstance(value, list):
                for child in value:
                    strip(child)
        strip(result)
        return result
    for kind, binary in [('policy-document',args.policy),('research',args.research)]:
        work = root / ('early-'+kind)
        work.mkdir()
        artifacts = work / 'artifacts'
        artifacts.mkdir()
        if kind == 'policy-document':
            spec = importlib.util.spec_from_file_location('early_policy_fixture', ROOT / 'scripts/policy-document-journey.py')
            fixture = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(fixture)
            profile = json.loads((ROOT / 'crates/policy-document-provider/data/readme.json').read_text())
            target = work / 'README.md'
            target.write_text(fixture.document())
            profile.update(mode='audit')
            profile['target']['path'] = str(target)
            command = ['commission',json.dumps(profile)]
            review_state = 'semantic-review'
        else:
            spec = importlib.util.spec_from_file_location('early_research_fixture', ROOT / 'scripts/research-journey.py')
            fixture = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(fixture)
            profile = json.loads((ROOT / 'crates/research-provider/data/configs/standard.json').read_text())
            for name, make in [('brief.json',fixture.brief),('sources.json',fixture.sources),
                               ('verification.json',fixture.verification),('report.json',fixture.report)]:
                (artifacts / name).write_text(json.dumps(make()))
            target = artifacts / 'verification.json'
            command = ['commission','verify']
            review_state = 'verify'
        profile['artifact_root'] = str(artifacts)
        selected = work / 'profile.json'
        selected.write_text(json.dumps(profile))
        config = work / 'providers.toml'
        config.write_text(f'[providers.{kind}]\ncommand = {json.dumps(binary)}\nargs = []\n')
        def call(*argv):
            return engine_call(args.engine,work / 'run.sqlite',config,list(argv))
        run_id = 'early-'+kind
        call('start','--id',run_id,kind,'@'+str(selected),'Early preparation public proof')
        if kind == 'policy-document':
            declare_target_authorship(Path(args.engine),work / 'run.sqlite',config,run_id,target,
                                      {'name':'owner','kind':'human'},'early-target-author')
        before = call('show','--view','full',run_id)
        assert before['result']['current_state'] != review_state, before
        complete = process([binary,*command],before)
        assert complete['missing_inputs'] == [], complete
        assert durable(call('show','--view','full',run_id)) == durable(before)
        saved = target.read_bytes()
        target.unlink()
        missing = process([binary,*command],before)
        assert any(row['input'] == ('target' if kind == 'policy-document' else 'verification.json')
                   for row in missing['missing_inputs']), missing
        assert durable(call('show','--view','full',run_id)) == durable(before)
        target.write_bytes(saved)
    print('external-work early review briefs and partial-parent public proof passed')


def self_test():
    import ast
    for filename in ['external-work-journey.py','external_work_driver.py','lean-change-journey.py',
                     'policy-document-journey.py','research-journey.py','generate-prd-journey.py']:
        ast.parse((ROOT / 'scripts' / filename).read_text(),filename=filename)
    retired_modules = {'work_slot_journey','recovery_execution','recovery_cancellation',
                       'operational_ux_capture','operational_ux_summary','operational_ux_monitor'}
    for path in (ROOT / 'scripts').glob('*.py'):
        tree = ast.parse(path.read_text(),filename=str(path))
        for node in ast.walk(tree):
            imported = ([name.name for name in node.names] if isinstance(node,ast.Import)
                        else [node.module] if isinstance(node,ast.ImportFrom) else [])
            assert not retired_modules.intersection(imported), path
    assert all(not (ROOT / 'scripts' / (name+'.py')).exists() for name in retired_modules)
    assert len(RETIRED_ENGINE) == len(set(RETIRED_ENGINE))
    assert 'invoke' in RETIRED_ENGINE and 'run-validation' in RETIRED_PROVIDER
    assert callable(lean_module().intent)
    # Concurrent append: history can observe it before the workflow read does.
    partial = {'workflow':{'context':[], 'current_state':'explore'},
               'judgment':{'history':{'items':[{'sequence':2, 'action':{'context_record_id':'note'}}]}},
               'sampled_at_ms':1, 'freshness':{'sampled_at_ms':1}}
    complete = copy.deepcopy(partial)
    complete['workflow']['context'] = [{'id':'note', 'sequence':2}]
    complete['sampled_at_ms'] = complete['freshness']['sampled_at_ms'] = 2
    assert acknowledge_context_record(partial, 'note', lambda:complete) == complete
    assert acknowledge_context_record(complete, 'note', lambda:None) == complete
    for corruption in ['unchanged', 'another-context', 'semantic-change']:
        bad = copy.deepcopy(complete)
        if corruption == 'unchanged':
            bad = copy.deepcopy(partial)
        elif corruption == 'another-context':
            bad['workflow']['context'].append({'id':'other', 'sequence':3})
        else:
            bad['workflow']['current_state'] = 'intent-review'
        try:
            acknowledge_context_record(partial, 'note', lambda:bad)
        except AssertionError:
            pass
        else:
            raise AssertionError('Context catch-up accepted ' + corruption)
    print('external-work journey self-test passed')


def main():
    parser = argparse.ArgumentParser()
    for key in ['engine','lean','policy','research','checker','output-root']:
        parser.add_argument('--'+key)
    parser.add_argument('--self-test',action='store_true')
    args = parser.parse_args()
    # Bare invocation is the recognized CI collection form; full proof uses the parameterized run below.
    if args.self_test or len(sys.argv) == 1:
        self_test()
        return
    for key in ['engine','lean','policy','research','checker','output_root']:
        if not getattr(args,key):
            parser.error('--'+key.replace('_','-')+' is required')
        setattr(args,key,str(Path(getattr(args,key)).resolve()))
    root = Path(args.output_root)
    assert root != ROOT and not root.is_relative_to(ROOT), 'Proof root must be external.'
    assert not root.exists() or not any(root.iterdir()), 'Use a fresh proof root; preserve original captures.'
    root.mkdir(parents=True,exist_ok=True)
    trap = root / 'trap-bin'
    trap.mkdir(exist_ok=True)
    sentinel = root / 'forbidden-execution'
    assert not sentinel.exists(), 'Use a fresh output root: prior execution sentinel exists.'
    for command in ['pi','dagu','cargo']:
        script = trap / command
        script.write_text('#!/bin/sh\nprintf "%s\\n" "$0" >> '+json.dumps(str(sentinel))+'\nexit 99\n')
        script.chmod(0o755)
    os.environ['PATH'] = str(trap)+os.pathsep+os.environ['PATH']
    retirement_root = root / 'retirement'
    retirement_root.mkdir()
    retirement(args,retirement_root)
    profile_preparation(args,root)
    provider_specific(args,root)
    retention_provider_runs(args,root)
    legacy_change_report(args,root)
    reclaim_legacy_catalog(args,root)
    early_review_briefs(args,root)
    engine_observation(args,root)
    generated = raw([sys.executable,ROOT / 'scripts/generate-prd-journey.py','--mode','source',
                     '--engine',args.engine,'--provider',args.research,'--checker',args.checker,
                     '--profile',ROOT / 'crates/research-provider/data/configs/generate-prd.json',
                     '--work-root',root / 'generate-prd'])
    (root / 'generate-prd.stdout').write_text(generated.stdout)
    (root / 'generate-prd.stderr').write_text(generated.stderr)
    assert generated.returncode == 0, generated.stderr
    assert 'generate-prd external-work normal completion passed' in generated.stdout
    print('external-work generate-prd checker-backed normal completion passed')
    # bookends:LE-139 — meaningful workflows and changed provider boundaries complete through real binaries with scripted judgments and no model/executor launches.
    assert not sentinel.exists(), sentinel.read_text() if sentinel.exists() else ''
    (root / 'outcome.json').write_text(json.dumps({'status':'passed',
                                                'residual_risks':['Scripted reviews prove mechanics, not semantic verdict quality.'],
                                                'execution_sentinel_absent':True},indent=2))
    print('external-work execution traps stayed unused passed')
    print('external-work journey passed')

if __name__ == '__main__':
    main()
