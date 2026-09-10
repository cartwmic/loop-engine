"""Public advisory-summary scenarios using only scripted completion commands."""
import json
import queue
import subprocess
import sys
import threading
import time
from pathlib import Path


def summary_case(args):
    root = args.attempt
    engine = args.binary_dir / 'loop-engine'
    source = root / 'source'
    source.mkdir()
    mode = root / 'mode'
    mode.write_text('valid')
    worker = root / 'completion.py'
    worker.write_text('''import json,sys,time,pathlib
p=json.load(sys.stdin)
mode=pathlib.Path(sys.argv[1]).read_text()
print('scripted stderr '+mode,file=sys.stderr,flush=True)
if mode=='timeout': time.sleep(10)
if mode=='nonzero': print('retained failure');sys.exit(7)
if mode=='empty': sys.exit(0)
if mode=='malformed': print('{}');sys.exit(0)
if mode=='junk': print('not json');sys.exit(0)
if mode in ('extra','bad-correction','bad-usage'):
 v=dict(developments='x',significance='x',uncertainty='x',corrections=[])
 if mode=='extra': v['unexpected']=True
 if mode=='bad-correction': v['corrections']=[dict(prior_claim='x')]
 if mode=='bad-usage': v['usage']=dict(amount=3)
 print(json.dumps(v));sys.exit(0)
previous=p['previous_summary']
print(json.dumps(dict(developments='fixture claim' if not previous else 'corrected fixture claim',significance='execution is not judgment',uncertainty='truth unverified',corrections=[] if not previous else [dict(prior_claim=previous['output']['developments'],correcting_evidence=p['selected_sources'][0])],usage=dict(amount=3,unit='fixture tokens'))))
''')
    commands = []
    live = []

    def change(n, **extra):
        (source / 'summary.json').write_text(json.dumps({'workers':[{'assignment_id':str(n), **extra}]}))

    def config(name, cap=20, interval=.3, executable=None):
        path = root / (name + '.json')
        path.write_text(json.dumps(dict(executable=executable or sys.executable,
            args=[str(worker), str(mode)], timeout_seconds=.2,
            minimum_interval_seconds=interval, max_calls=cap)))
        return path

    def start(name, cfg):
        output = root / name
        argv = [str(engine),'monitor','--capture-dir',str(source),'--json',
                '--poll-seconds','.02','--summary-config',str(cfg),'--output-dir',str(output)]
        index = len(commands)
        stdout = root / f'observer-{index}.stdout'
        stderr = root / f'observer-{index}.stderr'
        err = stderr.open('w')
        p = subprocess.Popen(argv,cwd=root,stdout=subprocess.PIPE,stderr=err,text=True)
        q = queue.Queue()
        def consume():
            with stdout.open('w') as f:
                for line in p.stdout:
                    f.write(line); f.flush(); q.put(json.loads(line))
        thread=threading.Thread(target=consume);thread.start()
        row=dict(argv=argv,cwd=str(root),stdout=str(stdout),stderr=str(stderr),exit_code=None)
        commands.append(row)
        item=(p,q,thread,err,row)
        live.append(item)
        return item

    def wait(item, status):
        deadline=time.monotonic()+8
        while time.monotonic()<deadline:
            try: value=item[1].get(timeout=.1)
            except queue.Empty: continue
            if value.get('status')==status: return value
        raise AssertionError(f'missing {status}; captures: {item[4]}')

    def stop(item):
        p,q,thread,err,row=item
        p.terminate();row['exit_code']=p.wait(timeout=5);thread.join(timeout=2);err.close()
        live.remove(item)
        (root/'commands.json').write_text(json.dumps(commands,indent=2))

    outcomes=[]
    try:
        change(1)
        cfg=config('main')
        item=start('session',cfg)
        first=wait(item,'summary-usable');assert first['attempted_calls']==1
        attempt=root/'session'/'attempt-0001'
        initial=json.loads((attempt/'stdin.json').read_text())
        assert initial['selected_sources']==['capture:'+str(source)]
        assert initial['previous_summary'] is None and initial['evidence_digest'].startswith('sha256:')
        assert initial['current_new_evidence'][0]['worker'][0]['assignment_id']=='1'
        change(1, observed_at=999, elapsed_ms=45)
        time.sleep(.4)
        assert json.loads((root/'session'/'session.json').read_text())['attempted_calls']==1
        outcomes.append('unchanged and observation clocks suppress calls')
        change(2)
        second=wait(item,'summary-usable');assert second['attempted_calls']==2
        inp=json.loads((root/'session'/'attempt-0002'/'stdin.json').read_text())
        assert inp['previous_summary']['output']['developments']=='fixture claim'
        assert inp['previous_summary']['evidence_digest']==initial['evidence_digest']
        assert second['previous_summary']['output']['corrections'][0]['correcting_evidence']=='capture:'+str(source)
        outcomes.append('exact selected evidence, digest, fallible prior claim and correcting locator retained')
        for n, failure in enumerate(['malformed','empty','nonzero','junk','timeout','extra','bad-correction','bad-usage'],3):
            mode.write_text(failure);change(n)
            value=wait(item,'summary-failed')
            assert value['attempted_calls']==n and 'older' in value['previous_summary_label']
            assert value['previous_summary']['output']['developments']=='corrected fixture claim'
            a=root/'session'/f'attempt-{n:04}'
            assert all((a/p).exists() for p in ['stdin.json','stdout','stderr','exit.json','command.json'])
            fact=json.loads((a/'exit.json').read_text());assert not fact['output_conformant']
            if failure=='timeout': assert fact['timed_out']
            if failure=='nonzero': assert fact['exit_code']==7
            time.sleep(.35)
            assert json.loads((root/'session'/'session.json').read_text())['attempted_calls']==n
        outcomes.append('malformed, empty, nonzero, non-JSON and timeout failures retain older summary; unchanged failure never retries')
        # Same session cannot run two advisory streams, but both observers stay live.
        peer=start('session',cfg)
        assert 'already in use' in wait(peer,'summary-failed')['detail']
        stop(peer)
        stop(item)
        outcomes.append('strict unknown-field/correction/usage rejection and session serialization')
        # A changed source remains observable even when no executable can launch.
        missing=start('missing',config('missing-config',executable=str(root/'absent')))
        assert wait(missing,'summary-failed')['detail']['spawn_error']
        change(50)
        deadline=time.monotonic()+5
        while True:
            value=missing[1].get(timeout=max(.01,deadline-time.monotonic()))
            if value.get('event')=='snapshot' and value['worker'][0]['assignment_id']=='50': break
        stop(missing);outcomes.append('missing executable leaves changed deterministic snapshot stream alive')
        mode.write_text('valid');change(60)
        capcfg=config('cap-config',cap=2,interval=.5)
        capped=start('cap',capcfg);wait(capped,'summary-usable')
        # Coalesce two revisions while cadence holds, without launching concurrently.
        change(61);wait(capped,'summary-cadence');change(62)
        value=wait(capped,'summary-usable');assert value['attempted_calls']==2
        inp=json.loads((root/'cap'/'attempt-0002'/'stdin.json').read_text())
        assert inp['current_new_evidence'][0]['worker'][0]['assignment_id']=='62'
        wait(capped,'summary-budget-exhausted');stop(capped)
        change(63);capped=start('cap',capcfg)
        assert wait(capped,'summary-budget-exhausted')['attempted_calls']==2
        assert not (root/'cap'/'attempt-0003').exists()
        stop(capped);outcomes.append('serialized changed-evidence cadence coalesces latest input; persisted cap survives restart')
        # Failed attempts consume the same retained cap.
        mode.write_text('empty');failed=start('failed-cap',config('failed-cap-config',cap=1))
        wait(failed,'summary-failed');stop(failed);change(64)
        failed=start('failed-cap',root/'failed-cap-config.json')
        assert wait(failed,'summary-budget-exhausted')['attempted_calls']==1
        stop(failed);outcomes.append('failed attempts count across restart')
        # Bound packet size: full source locator remains available when omitted.
        (source/'summary.json').write_text(json.dumps({'workers':[{'assignment_id':'large','body':'x'*70000}]}))
        mode.write_text('valid');bounded=start('bounded',config('bounded-config',cap=1))
        wait(bounded,'summary-usable');stop(bounded)
        inp=json.loads((root/'bounded'/'attempt-0001'/'stdin.json').read_text())
        assert inp['omitted_source_count']==1 and inp['omitted_source_locators']==['capture:'+str(source)]
        assert inp['current_new_evidence']==[]
        outcomes.append('oversized source omitted with count and locator, never silently truncated')
        (root/'scenarios.json').write_text(json.dumps(outcomes,indent=2))
    finally:
        for item in list(live): stop(item)
    print('summary public scenarios passed: '+str(root))
