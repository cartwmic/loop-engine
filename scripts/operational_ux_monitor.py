"""Public monitor proof: real waiting JSONL consumers, isolated native runs and scripted read adapter."""
import hashlib
import json
import queue
import subprocess
import sys
import threading
import time
from pathlib import Path


def monitor_case(args):
    root = args.attempt
    engine = args.binary_dir / 'loop-engine'
    commands = []
    outcomes = []

    def record(argv, code, out, err):
        commands.append(dict(argv=list(map(str, argv)), cwd=str(root), exit_code=code,
                             stdout=str(out), stderr=str(err)))
        (root / 'commands.json').write_text(json.dumps(commands, indent=2))

    def call(argv, expected=0):
        argv = list(map(str, argv))
        p = subprocess.run(argv, cwd=root, capture_output=True, timeout=30)
        out = root / f'call-{len(commands)}.stdout'; err = out.with_suffix('.stderr')
        out.write_bytes(p.stdout); err.write_bytes(p.stderr); record(argv, p.returncode, out, err)
        assert p.returncode == expected, (argv, p.returncode, p.stdout, p.stderr)
        return json.loads(p.stdout) if p.stdout.startswith(b'{') else p.stdout

    class Observer:
        def __init__(self, options):
            self.argv = [str(engine), 'monitor', '--json', '--poll-seconds', '0.05', *map(str, options)]
            self.out = root / f'observer-{len(commands)}.jsonl'
            self.err = self.out.with_suffix('.stderr')
            self.errors = self.err.open('wb')
            self.process = subprocess.Popen(self.argv, cwd=root, stdout=subprocess.PIPE, stderr=self.errors)
            self.events = queue.Queue()
            def consume():
                with self.out.open('wb') as retained:
                    for line in self.process.stdout:
                        retained.write(line); retained.flush()
                        if line.endswith(b'\n'):
                            self.events.put(json.loads(line))
            self.reader = threading.Thread(target=consume); self.reader.start()
        def wait(self, event, source=None, reason=None):
            deadline = time.monotonic() + 20
            while time.monotonic() < deadline:
                row = self.events.get(timeout=max(.01, deadline-time.monotonic()))
                if row['event'] == event and (source is None or row['source'] == source) and (reason is None or reason in str(row['boundary'])):
                    assert self.process.poll() is None, 'notification was not received while observer alive'
                    return row
            raise AssertionError('no matching live notification')
        def drain(self):
            rows = []
            while True:
                try: rows.append(self.events.get_nowait())
                except queue.Empty: return rows
        def stop(self):
            self.process.terminate(); self.process.wait(timeout=10); self.reader.join(timeout=5)
            self.errors.close(); record(self.argv, self.process.returncode, self.out, self.err)

    # Native providers expose identical engine mechanics, no semantic/model worker.
    provider = root / 'provider.py'
    provider.write_text('''#!/usr/bin/env python3
import json,sys
r=json.load(sys.stdin)
if r['operation']=='describe':
 print(json.dumps({'id':'monitor-fixture','initial_state':'work','states':[{'id':'work','title':'Work','instructions':'External work','final':False}],'work_slots':[{'id':'worker','state':'work','event':'check'}],'transitions':[{'source':'work','event':'check','target':'work','kind':'checked'}]}))
else: print(json.dumps({'result':'allow'}))
''')
    provider.chmod(0o755)
    config = root / 'providers.toml'
    config.write_text(''.join(f'[providers.{name}]\ncommand = {json.dumps(str(provider))}\n' for name in ('software-change', 'research')))
    db = root / 'native.sqlite'
    base = [engine, '--json', '--database', db, '--config', config]
    ids = []
    for name in ('software-change', 'research'):
        run = name + '-monitor'
        initial = {'work_slot_bindings': {'worker': {'command':sys.executable, 'args':['-c','import sys,time;sys.stdin.read();time.sleep(.3)']}}}
        call([*base, 'start', '--id', run, name, json.dumps(initial)])
        ids.append(run)
    ob = Observer(['--database', db, '--run', ids[0], '--run', ids[1], '--attention-seconds', '.2'])
    try:
        first = ob.wait('snapshot', 'run:'+ids[0])
        for key in ('workflow_lane', 'execution', 'worker_lane', 'conformance', 'acceptance',
                    'evidence', 'freshness', 'uncertainty', 'next_action', 'owner_update'):
            assert key in first, (key, first)
        assert first['acceptance']['state'] == 'unknown'
        assert first['owner_update']['required_before_next_decision']
        assert first['owner_update']['observed_change']
        assert first['owner_update']['needed_action_or_decision']
        ob.wait('snapshot', 'run:'+ids[1])
        denied = call([*base, 'append', ids[0], 'note', '{}'], 10)
        assert denied['code'] == 'run-not-observed'
        ob.wait('attention', reason='deadline')
    finally: ob.stop()
    call([*base, 'show', ids[0]])
    invocation = call([*base, 'invoke', ids[0], 'worker'])['result']
    ob = Observer(['--database', db, '--run', ids[0], '--invocation', invocation['invocation_id']])
    try: ob.wait('completion', 'run:'+ids[0])
    finally: ob.stop()
    outcomes.append({'native_two_providers':ids, 'database':str(db), 'status_did_not_arm':True, 'general_invoke_completion_received':True})

    # Real ad-hoc and bound Dagu graph/fan-out helpers, not fake summaries.
    instructions=root/'instructions.txt'; instructions.write_text('Scripted deterministic worker.')
    worker_spec={'command':sys.executable,'args':['-c','import sys;sys.stdin.read();print(\'{"answer":1}\')']}
    fanout_args=['fan-out','--worker',json.dumps(worker_spec),'--instructions',str(instructions)]
    fanout=call([engine,'--json',*fanout_args])
    fanout_root=Path(fanout['output_dir'])
    ob=Observer(['--capture-dir',fanout_root])
    try:
        completed = ob.wait('completion')
        assert completed['execution']['state'] == 'succeeded', completed
        assert completed['acceptance']['state'] == 'unknown', completed
        assert completed['evidence']['locations'], completed
        assert completed['owner_update']['observed_change']
    finally: ob.stop()
    # A stable completed capture is not a polling heartbeat. The completion
    # notification remains available, but unchanged snapshots are suppressed.
    ob=Observer(['--capture-dir',fanout_root])
    try:
        ob.wait('snapshot')
        time.sleep(.2)
        assert not [row for row in ob.drain() if row.get('event') == 'snapshot'], 'unchanged capture heartbeat'
    finally: ob.stop()
    # Removing an inventoried node remains unknown, never inferred success.
    import shutil
    missing_node=root/'missing-node'; shutil.copytree(fanout_root,missing_node)
    summary=json.loads((missing_node/'summary.json').read_text()); summary['workers']=[]
    (missing_node/'summary.json').write_text(json.dumps(summary))
    ob=Observer(['--capture-dir',missing_node,'--attention-seconds','.1'])
    try:
        row=ob.wait('attention'); assert 'unknown' in str(row['diagnostics'])
    finally: ob.stop()
    bound='bound-graph'
    initial={'work_slot_bindings':{'worker':{'command':str(engine),'args':['fan-out','--worker',json.dumps(worker_spec)]}}}
    call([*base,'start','--id',bound,'research',json.dumps(initial)])
    call([*base,'show',bound]); inv=call([*base,'invoke',bound,'worker'])['result']
    ob=Observer(['--database',db,'--run',bound,'--invocation',inv['invocation_id']])
    try: ob.wait('completion')
    finally: ob.stop()
    bad_spec={'command':sys.executable,'args':['-c','import sys;sys.stdin.read();print("not-json")'],'output_schema':{'required':['answer']}}
    bad_initial={'work_slot_bindings':{'worker':{'command':str(engine),'args':['fan-out','--worker',json.dumps(bad_spec)]}}}
    call([*base,'start','--id','bad-graph','research',json.dumps(bad_initial)])
    call([*base,'show','bad-graph']); bad=call([*base,'invoke','bad-graph','worker'])['result']
    ob=Observer(['--database',db,'--run','bad-graph','--invocation',bad['invocation_id']])
    try: ob.wait('attention',reason='failure')
    finally: ob.stop()
    outcomes.append({'unbound_fanout_completion_received':True,'bound_graph_completion_received':True,'absent_node_unknown':True,'exit_zero_nonconformance_retained':True})

    # Actual software-change graph helper, with deterministic task/summarizer workers.
    # Reuse the source journey's public setup, not a provider-name alias.
    import work_slot_journey as graph
    checkout = Path(__file__).resolve().parents[1]
    real_provider = args.binary_dir / 'software-change'
    graph_work = root / 'plan-graph-work'; graph_work.mkdir()
    call(['git', 'init', graph_work])
    call(['git', '-C', graph_work, '-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid', 'commit', '--allow-empty', '-m', 'fixture'])
    original_engine_json = graph._engine_json
    def logged_engine_json(binary, database, operation):
        return call([binary, '--json', '--database', database, *operation])
    graph._engine_json = logged_engine_json
    try:
        graph_run = 'real-bound-plan-graph'
        # The shipped v3 minimal profile now keeps every review phase live.
        # This transport-only monitor fixture deliberately uses the same v3
        # artifact/config bytes with review obligations empty so the public
        # graph reaches implementation without fabricating semantic verdicts.
        profile_source = root / 'minimal-operational.json'
        profile_value = json.loads(
            (checkout / 'crates/software-change-provider/data/configs/minimal.json').read_text()
        )
        profile_value['review_policies'] = {
            gate: [] for gate in profile_value['review_policies']
        }
        profile_source.write_text(json.dumps(profile_value, indent=2))
        binding = graph.implement_graph_runner_binding(
            provider=real_provider, task_worker=graph.stdin_worker_cli(root/'graph-receipts'),
            working_directory=graph_work)
        graph_call, graph_root, profile = graph._start_isolated_software_change(
            engine=engine, provider=real_provider,
            profile_source=profile_source,
            fixture_root=checkout/'crates/software-change-provider/data/calibration/fixtures',
            work_dir=root/'real-graph-run', run_id=graph_run, extra_bindings={'implement':binding})
        graph.invoke_until_succeeded(graph_call, graph_run, 'intent-draft', timeout_s=45)
        for event, target in [('intent-ready','design'),('design-ready','plan'),('plan-ready','implement')]:
            graph._expect_event_state(graph_call, graph_run, event, target)
        graph_inv = graph.invoke_until_succeeded(graph_call, graph_run, 'implement', timeout_s=45)
        graph_capture = Path(graph_inv['capture_dir'])
        graph_db = root/'real-graph-run/loop.sqlite'
        for options, source in [
            (['--database',graph_db,'--run',graph_run,'--invocation',graph_inv['invocation_id']], 'run:'+graph_run),
            (['--capture-dir',graph_capture], 'capture:'+str(graph_capture))]:
            ob = Observer(options)
            try:
                row = ob.wait('completion', source)
                assert row['judgment']['state'] == 'unknown'
                expected = {task['id'] for task in json.loads((graph_root/'plan.json').read_text())['tasks']} | {'summarizer'}
                assert {worker['assignment_id'] for worker in row['worker']} == expected, row
                assert all(worker['exit_code'] == 0 for worker in row['worker']), row
                if source.startswith('run:'):
                    assert row['helper']['invocation_id'] == graph_inv['invocation_id']
            finally: ob.stop()
        # Unbound public helper with its own actual task/summarizer captures.
        unbound_root = root/'unbound-plan'; graph._write_small_plan(unbound_root)
        unbound_capture = root/'unbound-plan-capture'
        gate = root/'graph-release'
        gated_worker = root/'gated-graph-worker.py'
        gated_worker.write_text('import pathlib,sys,time,subprocess\n'
            'packet=sys.stdin.buffer.read()\n'
            'gate=pathlib.Path(sys.argv[1])\n'
            'deadline=time.monotonic()+30\n'
            'while not gate.exists():\n'
            ' if time.monotonic()>deadline: sys.exit(9)\n'
            ' time.sleep(.02)\n'
            'sys.exit(subprocess.run(sys.argv[2:],input=packet).returncode)\n')
        dummy = graph.stdin_worker_cli(root/'unbound-receipts')
        gated = {'command':sys.executable,'args':[str(gated_worker),str(gate),dummy['command'],*dummy['args']]}
        unbound_binding = graph.implement_graph_runner_binding(
            provider=real_provider, task_worker=gated,
            working_directory=graph_work)
        argv = [unbound_binding['command'], *unbound_binding['args']]
        packet = graph._invoke_packet(run_id='real-unbound-plan-graph', slot_id='implement',
            artifact_root=unbound_root, instruction_body='Deterministic fixture.', capture_dir=unbound_capture)
        (root/'unbound-helper.stdin').write_bytes(packet)
        out=root/'unbound-helper.stdout'; err=root/'unbound-helper.stderr'
        with out.open('wb') as stdout, err.open('wb') as stderr:
            helper=subprocess.Popen(argv,cwd=root,stdin=subprocess.PIPE,stdout=stdout,stderr=stderr)
            helper.stdin.write(packet); helper.stdin.close()
            ob=Observer(['--capture-dir',unbound_capture])
            try:
                ob.wait('snapshot','capture:'+str(unbound_capture))
                ob.stop()
                assert helper.poll() is None, 'observer exit cancelled the graph helper'
                ob=Observer(['--capture-dir',unbound_capture])
                gate.write_text('continue')
                row=ob.wait('completion','capture:'+str(unbound_capture))
                assert {worker['assignment_id'] for worker in row['worker']} == {'alpha','beta','gamma','summarizer'}
                assert row['judgment']['state'] == 'unknown'
            finally:
                gate.touch()
                ob.stop()
                code=helper.wait(timeout=45)
                record(argv,code,out,err)
            assert code == 0
        for capture_root in (graph_capture, unbound_capture):
            assert (capture_root/'summarizer/stdout').is_file()
            ob=Observer(['--capture-dir',capture_root])
            try: ob.wait('completion','capture:'+str(capture_root))
            finally: ob.stop()
        missing_graph=root/'missing-plan-task'; shutil.copytree(unbound_capture,missing_graph)
        summary=json.loads((missing_graph/'summary.json').read_text())
        # Paths in real captures are absolute: rewrite the copied selected path only.
        summary['workers'][0]['selected_output_path']=str(missing_graph/'absent-output')
        (missing_graph/'summary.json').write_text(json.dumps(summary))
        ob=Observer(['--capture-dir',missing_graph,'--capture-dir',unbound_capture])
        try:
            ob.wait('attention','capture:'+str(missing_graph))
            ob.wait('completion','capture:'+str(unbound_capture))
        finally: ob.stop()
        for mode in ('missing-summarizer', 'legacy-inventory'):
            specimen=root/mode; shutil.copytree(unbound_capture,specimen)
            summary=json.loads((specimen/'summary.json').read_text())
            if mode == 'missing-summarizer': summary['auxiliary_workers']=[]
            else: summary.pop('expected_assignment_ids')
            (specimen/'summary.json').write_text(json.dumps(summary))
            ob=Observer(['--capture-dir',specimen,'--attention-seconds','.1'])
            try:
                row=ob.wait('attention','capture:'+str(specimen))
                assert 'unknown' in str(row['diagnostics']), row
            finally: ob.stop()
        outcomes.append({'real_software_change_bound_and_unbound_plan_graph':True,
            'run_id':graph_run,'database':str(graph_db),'captures':[str(graph_capture),str(unbound_capture)],
            'missing_task_not_success':True,'restart_completion':True,'graph_survived_observer_exit':True,
            'task_and_summarizer_receipts_visible':True,'semantic_judgment':'unknown'})
    finally:
        graph._engine_json = original_engine_json

    # Real external command keeps running across observer termination and restart.
    work = root / 'work'; work.mkdir()
    call(['git', 'init', work])
    call(['git', '-C', work, '-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid', 'commit', '--allow-empty', '-m', 'fixture'])
    capture = root / 'capture'
    argv = [engine, 'capture-command', '--working-directory', work, '--output-dir', capture,
            '--', sys.executable, '-c', 'import time;print("started",flush=True);time.sleep(1);print("finished",flush=True)']
    out = root/'worker.stdout'; err = root/'worker.stderr'
    with out.open('wb') as stdout, err.open('wb') as stderr:
        worker = subprocess.Popen(list(map(str,argv)), cwd=root, stdout=stdout, stderr=stderr)
        ob = Observer(['--capture-dir', capture, '--attention-seconds', '.15'])
        try: ob.wait('attention', reason='deadline')
        finally: ob.stop()
        assert worker.poll() is None, 'observer stopped selected work'
        ob = Observer(['--capture-dir', capture])
        try: event = ob.wait('completion', 'capture:'+str(capture))
        finally: ob.stop()
        assert worker.wait(timeout=10) == 0
    record(argv, worker.returncode, out, err)
    original = (capture/'index.json').read_bytes()
    ob = Observer(['--capture-dir', capture])
    try: ob.wait('completion')
    finally: ob.stop()
    assert (capture/'index.json').read_bytes() == original
    outcomes.append({'real_capture_completion_received_live':True, 'restart_retained_evidence':True, 'worker_survived_observer_exit':True})

    # Explicit corrupt specimens cannot borrow a peer's completion.
    import shutil
    for mode in ('partial', 'conflict', 'cleanup'):
        specimen = root / mode; shutil.copytree(capture, specimen)
        if mode == 'partial': (specimen/'index.json').write_text('{')
        else:
            state = json.loads((specimen/'state.json').read_text())
            if mode == 'conflict': state['row_count'] += 1
            else: state['cleanup'] = 'pending'
            (specimen/'state.json').write_text(json.dumps(state))
        ob = Observer(['--capture-dir',specimen,'--capture-dir',capture])
        try:
            ob.wait('attention','capture:'+str(specimen)); ob.wait('completion','capture:'+str(capture))
        finally: ob.stop()
    ob = Observer(['--capture-dir',root/'missing','--attention-seconds','.1'])
    try:
        row=ob.wait('attention'); assert row['worker']['state']=='unknown'
    finally: ob.stop()
    outcomes.append({'partial_conflict_cleanup_attention':True,'missing_never_success':True,'completed_peer_independent':True})

    # Scripted compatibility backend logs every command; it cannot read a catalog.
    backend=root/'released-adapter.py'
    backend.write_text('''#!/usr/bin/env python3
import json,sys,pathlib
root=pathlib.Path(__file__).parent
a=sys.argv[1:]
with (root/'adapter-calls.jsonl').open('a') as f:f.write(json.dumps(a)+'\\n')
assert a[0]=='--json'
if a[1]=='list': result=[{'run_id':'released','lifecycle':'active','provider_id':'research'}]
elif a[1]=='history': result=[{'sequence':2,'occurred_at':123,'action':{'kind':'invocation_status_changed','invocation_id':'i','status':'succeeded'}}]
elif a[1]=='invocation-progress': result={'run_id':'released','invocation_id':'i','capture_dir':'/not-driver-attached','traces':[]}
else: raise AssertionError(a)
print(json.dumps({'status':'completed','result':result}))
''')
    backend.chmod(0o755)
    observation=root/'dated-observation.json'
    observation.write_text(json.dumps({'run_id':'released','sampled_at_ms':123,'attesting_driver':'scripted-driver','judgment':{'author':'fixture-reviewer','result':'opaque-provider-value'}}))
    ob=Observer(['--engine',backend,'--run','released','--invocation','i','--observation',observation])
    try:
        row=ob.wait('completion'); assert row['worker']['state']=='unknown'
        assert row['judgment']['driver_observations'][0]['attribution']['attesting_driver']=='scripted-driver'
        assert 'unknown' in row['judgment']['driver_observations'][0]['freshness']
    finally: ob.stop()
    logged=[json.loads(line) for line in (root/'adapter-calls.jsonl').read_text().splitlines()]
    assert {a[1] for a in logged} == {'list','history','invocation-progress'}
    outcomes.append({'compatibility_read_allowlist':True,'candidate_catalog_access':False,'model_calls':0})
    peer_argv=[sys.executable,'-c','import time,pathlib;time.sleep(.6);pathlib.Path("peer-finished").write_text("finished")']
    peer_out=root/'peer.stdout';peer_err=root/'peer.stderr'
    with peer_out.open('wb') as out,peer_err.open('wb') as err:
        peer=subprocess.Popen(peer_argv,cwd=root,stdout=out,stderr=err)
        ob=Observer(['--capture-dir',capture])
        try: ob.wait('completion')
        finally: ob.stop()
        assert peer.poll() is None
        assert peer.wait(timeout=10)==0
    record(peer_argv,peer.returncode,peer_out,peer_err)
    assert (root/'peer-finished').read_text()=='finished'
    human_argv=[str(engine),'monitor','--capture-dir',str(capture),'--poll-seconds','.05']
    human=subprocess.Popen(human_argv,cwd=root,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
    try:
        first=human.stderr.readline(); assert first.startswith(b'snapshot:')
    finally: human.terminate()
    out,err=human.communicate(timeout=10);assert out==b''
    (root/'human.stdout').write_bytes(out);(root/'human.stderr').write_bytes(first+err)
    record(human_argv,human.returncode,root/'human.stdout',root/'human.stderr')
    call([engine,'monitor','--json'],20)
    outcomes.append({'unselected_live_peer_finished':True,'human_stdout_empty':True,'dated_attribution_not_current_approval':True})
    (root/'scenarios.json').write_text(json.dumps(outcomes,indent=2))
