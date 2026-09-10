"""Public capture case implementation for operational-ux-journey.py."""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import selectors
import shutil
import signal
import subprocess
import sys
import time

from test_contract import repository_proof_identity


def _ownership_regression(root, engine, repo, call, streamed):
    """Script process observations, not host PID reuse; exercise real CLI/signals."""
    work = root / "ownership-regression"
    work.mkdir()
    real_ps = shutil.which("ps")
    assert real_ps
    fake_start = "Sat Jan 1 00:00:00 2000"

    def wait_for(predicate):
        deadline = time.monotonic() + 12
        while not predicate():
            assert time.monotonic() < deadline, "ownership fixture deadline"
            time.sleep(.02)

    unrelated_code = '''import os, pathlib, signal, subprocess, sys, time
root = pathlib.Path(sys.argv[1]); role = sys.argv[2]
def terminated(*_):
    with (root / 'term-log').open('a') as f: f.write(role + '\\n')
signal.signal(signal.SIGTERM, terminated)
(root / (role + '-pid')).write_text(str(os.getpid()))
child = None; count = 0
while not (root / 'stop-unrelated').exists():
    count += 1
    tmp = root / (role + '-heartbeat.tmp'); tmp.write_text(str(count))
    tmp.replace(root / (role + '-heartbeat'))
    if role == 'unrelated' and child is None and (root / 'spawn-child').exists():
        child = subprocess.Popen([sys.executable, __file__, str(root), 'unrelated-child'])
    time.sleep(.02)
if child is not None: child.wait(timeout=5)
'''
    unrelated_script = work / "unrelated.py"
    unrelated_script.write_text(unrelated_code)
    unrelated = subprocess.Popen([sys.executable, str(unrelated_script), str(work), "unrelated"],
                                 stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                                 start_new_session=True)
    try:
        wait_for(lambda: (work / "unrelated-pid").exists())
        for mode in ("stale-identity", "signal-check-failure"):
            case = work / mode
            case.mkdir()
            dest = case / "capture"
            backend = case / "backend.py"
            backend.write_text('''import os, pathlib, signal, subprocess, sys, time
root = pathlib.Path(__file__).parent
if len(sys.argv) == 1:
    signal.signal(signal.SIGTERM, lambda *_: None)
    child = subprocess.Popen([sys.executable, __file__, 'child'])
    while not (root / 'owned-ready').exists(): time.sleep(.01)
    print('OWNED-READY', flush=True)
    child.wait()
elif sys.argv[1] == 'child':
    (root / 'owned-ready').write_text(str(os.getpid()))
    while not (root / 'switch-token').exists(): time.sleep(.01)
    os.setsid()
    os.execve(sys.executable, [sys.executable, __file__, 'escaped'],
              dict(os.environ, LOOP_CAPTURE_OWNER='nested-fixture-token'))
else:
    signal.signal(signal.SIGTERM, signal.SIG_IGN)
    (root / 'switched').touch()
    while not (root / 'stop-owned').exists(): time.sleep(.02)
''')
            shim = case / "ps"
            shim.write_text(f"#!{sys.executable}\n" + '''import json, pathlib, subprocess, sys
''' + f"root = pathlib.Path({str(case)!r})\nouter = root.parent\nreal = {real_ps!r}\nfake_start = {fake_start!r}\n" + '''args = sys.argv[1:]
if '-p' in args and (root / 'fail-signal-check').exists():
    print('scripted identity inspection failure', file=sys.stderr); sys.exit(2)
p = subprocess.run([real, *args], capture_output=True)
if args == ['-axo', 'pid=,ppid=,stat=,lstart='] and p.returncode == 0:
    rows = [line.split() for line in p.stdout.decode().splitlines()]
    state = json.loads((root / 'capture/state.json').read_text())
    unrelated = int((outer / 'unrelated-pid').read_text())
    for row in rows:
        if root.name == 'stale-identity' and int(row[0]) == unrelated and not (root / 'replacement').exists():
            row[1] = str(state['root_pid']); row[3:] = fake_start.split()
        if (root / 'replacement').exists() and (root / 'owned-ready').exists() and int(row[0]) == int((root / 'owned-ready').read_text()):
            row[1] = '1'  # Script reparenting after real setsid/token replacement.
    if (root / 'replacement').exists(): (root / 'replacement-read').touch()
    sys.stdout.write(''.join(' '.join(row) + '\\n' for row in rows))
else:
    sys.stdout.buffer.write(p.stdout)
sys.stderr.buffer.write(p.stderr)
sys.exit(p.returncode)
''')
            shim.chmod(0o755)
            env = {**os.environ, "PATH": str(case) + os.pathsep + os.environ["PATH"]}
            argv = [engine, "capture-command", "--working-directory", repo,
                    "--output-dir", dest, "--timeout-ms", "20000", "--",
                    sys.executable, backend]

            def action(controller):
                owned_pid = (case / "owned-ready").read_text()
                def observed():
                    path = dest / "owned.json"
                    if not path.exists(): return False
                    starts = json.loads(path.read_text()).get("process_starts", {})
                    return owned_pid in starts and (mode != "stale-identity" or starts.get(str(unrelated.pid)) == fake_start)
                wait_for(observed)
                (case / "switch-token").touch()
                wait_for(lambda: (case / "switched").exists())
                (case / "replacement").touch()
                wait_for(lambda: (case / "replacement-read").exists())
                (work / "spawn-child").touch()
                wait_for(lambda: (work / "unrelated-child-heartbeat").exists())
                if mode == "signal-check-failure": (case / "fail-signal-check").touch()
                controller.send_signal(signal.SIGINT)

            try:
                streamed(argv, b"OWNED-READY\n", 1, action, env=env)
                state = json.loads((dest / "state.json").read_text())
                receipt_path = Path(json.loads((dest / "index.json").read_text())["receipts"][0]["receipt"])
                receipt_bytes = receipt_path.read_bytes()
                receipt = json.loads(receipt_bytes)
                assert receipt["aborted"] and state["status"] == "failed"
                if mode == "stale-identity":
                    # Scripted reparenting makes both processes cleanup leaves;
                    # either parent exit or signal is valid for this aborted row.
                    assert receipt["cleanup"] == "complete", receipt
                else:
                    assert receipt["cleanup"] == "pending" and receipt["capture_error"]
                    assert "cannot verify owned process" in receipt["capture_error"]
                    call(argv[:argv.index("--")] + ["--resume"] + argv[argv.index("--"):], 20, env=env)
                    assert receipt_path.read_bytes() == receipt_bytes
                for stream in ("stdout", "stderr"):
                    assert receipt[stream + "_sha256"] == hashlib.sha256((receipt_path.parent / stream).read_bytes()).hexdigest()
                assert unrelated.poll() is None and not (work / "term-log").exists()
                for role in ("unrelated", "unrelated-child"):
                    heartbeat_path = work / (role + "-heartbeat")
                    heartbeat = int(heartbeat_path.read_text())
                    wait_for(lambda: int(heartbeat_path.read_text()) > heartbeat)
            finally:
                # Fixture-owned cooperative shutdown, not signaling persisted PID guesses.
                (case / "switch-token").touch()
                (case / "stop-owned").touch()
                ids = []
                ownership_path = dest / "owned.json"
                if ownership_path.exists():
                    ids.append(str(json.loads(ownership_path.read_text())["root_pid"]))
                if (case / "owned-ready").exists():
                    ids.append((case / "owned-ready").read_text())
                wait_for(lambda: not set(ids).intersection(subprocess.check_output([real_ps, "-axo", "pid="], text=True).split()))
    finally:
        (work / "stop-unrelated").touch()
        unrelated.wait(timeout=8)
    return {"stale_pid_and_ancestry": "scripted start-stamp replacement; unrelated parent/child survive without TERM",
            "owned_escape": "real setsid and token replacement, scripted reparenting; owned child gone and reaped",
            "signal_failure": "failed receipt/streams retained, cleanup pending, resume refused",
            "limit": "second-resolution start checks, not atomic signaling or a claim of actual OS PID reuse"}


def capture_case(args):
    root = args.attempt
    engine = args.binary_dir / "loop-engine"
    repo = root / "fixture-repository"
    repo.mkdir()
    commands = []

    def record(argv, cwd, code, stdout, stderr, start):
        commands.append(dict(argv=list(map(str, argv)), cwd=str(cwd), exit_code=code,
                             stdout=str(stdout), stderr=str(stderr), started_at=start,
                             finished_at=time.time()))
        (root / "commands.json").write_text(json.dumps(commands, indent=2))

    def call(argv, expected=0, cwd=repo, env=None):
        argv = list(map(str, argv))
        n = len(commands)
        out, err = root / f"call-{n:03}.stdout", root / f"call-{n:03}.stderr"
        start = time.time()
        with out.open("wb") as o, err.open("wb") as e:
            p = subprocess.run(argv, cwd=cwd, stdout=o, stderr=e, env=env, timeout=35)
        record(argv, cwd, p.returncode, out, err, start)
        assert p.returncode == expected, (p.returncode, expected, err.read_text())
        return out

    # This fixture owns a disposable repository, never source Git history.
    call(["git", "init", "--quiet"])
    (repo / "source").write_text("fixture tree\n")
    call(["git", "add", "source"])
    call(["git", "-c", "user.name=Capture Fixture", "-c", "user.email=fixture@example.invalid",
          "-c", "commit.gpgsign=false", "commit", "--quiet", "-m", "fixture"])
    backend = root / "backend.py"
    backend.write_text('''import json, os, pathlib, signal, subprocess, sys, time
root = pathlib.Path(__file__).parent
row = sys.argv[1]
with (root / "launches").open("a") as f: f.write(row + "\\n")
print("STREAM-" + row, flush=True)
print("ERR-" + row, file=sys.stderr, flush=True)
if row == "detach" and not (root / "detach-success").exists():
    code = "import os,pathlib,signal,time; signal.signal(signal.SIGTERM,signal.SIG_IGN); pathlib.Path(" + repr(str(root / "detached-ready")) + ").write_text(str(os.getpid())); time.sleep(120)"
    signal.signal(signal.SIGTERM, lambda *_: None)
    child = subprocess.Popen([sys.executable, "-c", code], start_new_session=True)
    while not (root / "detached-ready").exists(): time.sleep(.01)
    print("DETACHED-READY", flush=True)
    child.wait()
elif row == "timeout":
    time.sleep(120)
else:
    time.sleep(.6)
if row == "two" and not (root / "success").exists(): sys.exit(7)
''')
    matrix = {"rows": [{"id": name, "argv": [sys.executable, str(backend), name],
                         "environment": {"CAPTURE_EXPLICIT": "fixture"},
                         "inherit_environment": ["CAPTURE_FIXTURE_SETTING"],
                         "timeout_ms": 10000, "obligations": ["AC-4"]}
                        for name in ("one", "two", "three")]}
    matrix_path = root / "matrix.json"
    matrix_path.write_text(json.dumps(matrix))
    capture = root / "serial"

    def matrix_argv(output=capture, path=matrix_path, resume=False):
        return [engine, "capture-matrix", "--matrix", path, "--working-directory", repo,
                "--output-dir", output] + (["--resume"] if resume else [])

    def streamed(argv, marker, expected, action=lambda p: None, env=None):
        argv = list(map(str, argv))
        n = len(commands)
        out, err = root / f"stream-{n:03}.stdout", root / f"stream-{n:03}.stderr"
        start = time.time()
        with err.open("wb") as e, out.open("wb") as o:
            p = subprocess.Popen(argv, cwd=repo, stdout=subprocess.PIPE, stderr=e, start_new_session=True, env=env)
            seen = bytearray()
            try:
                with selectors.DefaultSelector() as selector:
                    selector.register(p.stdout, selectors.EVENT_READ)
                    deadline = time.monotonic() + 20
                    while marker not in seen:
                        assert time.monotonic() < deadline, "waiting stream consumer deadline"
                        if not selector.select(1):
                            continue
                        chunk = os.read(p.stdout.fileno(), 8192)
                        assert chunk, "executor ended before live stream marker"
                        seen.extend(chunk)
                        o.write(chunk)
                        o.flush()
                    assert p.poll() is None, "marker arrived only after exit"
                    action(p)
                    while True:
                        chunk = p.stdout.read(8192)
                        if not chunk:
                            break
                        o.write(chunk)
                    code = p.wait(timeout=20)
            finally:
                if p.poll() is None:
                    # Control only this fixture-owned capture through its public abort path.
                    subprocess.run([str(engine), "capture-abort", "--output-dir", argv[argv.index("--output-dir")+1]],
                                   cwd=repo, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=15)
                    p.wait(timeout=15)
                p.stdout.close()
        record(argv, repo, code, out, err, start)
        assert code == expected, (code, expected, err.read_text())

    streamed(matrix_argv(), b"STREAM-one\n", 7)
    assert (root / "launches").read_text().splitlines() == ["one", "two"]
    original_index = json.loads((capture / "index.json").read_text())
    selected = [Path(v["receipt"]) for v in original_index["receipts"]]
    assert len(selected) == 2
    failures = {file: file.read_bytes() for p in selected for file in p.parent.iterdir()}
    identity = repository_proof_identity(repo)
    import importlib.util
    spec = importlib.util.spec_from_file_location("capture_report_checker", Path(__file__).with_name("assert-implementation-report.py"))
    checker = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(checker)
    # Scope the existing reader to this disposable proof repository. Receipts
    # and result logic are unchanged; no evidence export or success adapter.
    checker.ROOT = repo
    assert checker.check_receipt(selected[0], matrix["rows"][0], matrix["rows"][0]["argv"], identity)[0] == "passed"
    assert checker.check_receipt(selected[1], matrix["rows"][1], matrix["rows"][1]["argv"], identity)[0] == "failed"
    for path in selected:
        receipt = json.loads(path.read_text())
        assert receipt["repository_before"] == receipt["repository_after"] == identity
        assert receipt["cleanup"] == "complete"
        assert receipt["settings"]["environment"] == {"CAPTURE_EXPLICIT": "fixture"}
        assert "CAPTURE_FIXTURE_SETTING" in receipt["settings"]["inherited_environment"]
        for stream in ("stdout", "stderr"):
            assert receipt[stream + "_sha256"] == hashlib.sha256((path.parent / stream).read_bytes()).hexdigest()
    assert json.loads(selected[1].read_text())["exit_code"] == 7

    def corrupt_copy(name):
        dest = root / name
        shutil.copytree(capture, dest)
        # Copies are explicitly corrupt fixture specimens; original attempts never change.
        for path in (dest / "index.json", dest / "state.json"):
            path.write_text(path.read_text().replace(str(capture), str(dest)))
        return dest

    launches = (root / "launches").read_bytes()
    bad_matrix = root / "changed-argv.json"
    changed = json.loads(json.dumps(matrix))
    changed["rows"][1]["argv"].append("changed")
    bad_matrix.write_text(json.dumps(changed))
    call(matrix_argv(path=bad_matrix, resume=True), 20)
    changed["rows"][1]["argv"].pop()
    changed["rows"][1]["timeout_ms"] += 1
    bad_matrix.write_text(json.dumps(changed))
    call(matrix_argv(path=bad_matrix, resume=True), 20)
    call(matrix_argv(resume=True), 20, env={**os.environ, "CAPTURE_FIXTURE_SETTING": "changed"})
    (repo / "source").write_text("changed fixture tree\n")
    call(matrix_argv(resume=True), 20)
    (repo / "source").write_text("fixture tree\n")
    for name, damage in (("missing-stream", "missing"), ("changed-stream", "digest"),
                         ("partial-receipt", "partial"), ("partial-started", "partial-started"),
                         ("incomplete-started", "started"),
                         ("pending-cleanup", "cleanup")):
        dest = corrupt_copy(name)
        receipt_path = Path(json.loads((dest / "index.json").read_text())["receipts"][0]["receipt"])
        if damage == "missing":
            (receipt_path.parent / "stdout").unlink()
        elif damage == "digest":
            (receipt_path.parent / "stdout").write_bytes(b"corruption")
        elif damage == "partial":
            r = json.loads(receipt_path.read_text())
            del r["finished_at"]
            receipt_path.write_text(json.dumps(r))
        elif damage == "partial-started":
            (receipt_path.parent / "started.json").write_text("{}")
        elif damage == "started":
            orphan = dest / "attempts" / "interrupted-before-finish"
            orphan.mkdir()
            (orphan / "started.json").write_bytes((receipt_path.parent / "started.json").read_bytes())
        else:
            state = json.loads((dest / "state.json").read_text())
            state["cleanup"] = "pending"
            (dest / "state.json").write_text(json.dumps(state))
        call(matrix_argv(output=dest, resume=True), 20)
    assert (root / "launches").read_bytes() == launches, "refusal launched a command"
    (root / "success").touch()
    call(matrix_argv(resume=True))
    assert (root / "launches").read_text().splitlines() == ["one", "two", "two", "three"]
    for path, original in failures.items():
        assert path.read_bytes() == original
    resumed = json.loads((capture / "index.json").read_text())
    assert resumed["receipts"][0] == original_index["receipts"][0]
    assert resumed["receipts"][1] != original_index["receipts"][1]
    assert len(list((capture / "attempts").iterdir())) == 4
    call(matrix_argv(resume=True))
    assert (root / "launches").read_text().splitlines() == ["one", "two", "two", "three"]

    for mode in ("abort", "sigint"):
        dest = root / mode
        ready = root / "detached-ready"
        ready.unlink(missing_ok=True)
        (root / "detach-success").unlink(missing_ok=True)
        rows = {"rows": [{**matrix["rows"][0], "id": name,
                           "argv": [sys.executable, str(backend), name]}
                          for name in ("detach", "must-not-launch")]}
        path = root / (mode + "-matrix.json")
        path.write_text(json.dumps(rows))
        later_count = (root / "launches").read_text().splitlines().count("must-not-launch")
        def action(p):
            assert ready.exists()
            # Concurrent admission must refuse while ownership is live.
            call(matrix_argv(dest, path, True), 20)
            if mode == "abort":
                call([engine, "capture-abort", "--output-dir", dest])
            else:
                p.send_signal(signal.SIGINT)
        streamed(matrix_argv(dest, path), b"DETACHED-READY\n", 1, action)
        pid = int(ready.read_text())
        inventory = subprocess.check_output(["ps", "-axo", "pid="], text=True)
        assert str(pid) not in inventory.split(), "detached descendant survived/reaping unverified"
        state = json.loads((dest / "state.json").read_text())
        assert state["cleanup"] == "complete" and state["status"] == "failed"
        rpath = Path(json.loads((dest / "index.json").read_text())["receipts"][0]["receipt"])
        before_retry = rpath.read_bytes()
        receipt = json.loads(before_retry)
        assert receipt["aborted"] and receipt["cleanup"] == "complete"
        assert receipt["exit_code"] == 0, "cooperative parent should reap and exit normally; abort stays distinct"
        assert checker.check_receipt(rpath, rows["rows"][0], rows["rows"][0]["argv"], identity)[0] == "failed"
        assert (root / "launches").read_text().splitlines().count("must-not-launch") == later_count
        call([engine, "capture-abort", "--output-dir", dest])
        (root / "detach-success").touch()
        # Same argv, explicit restart after verified interruption; original retained.
        call(matrix_argv(dest, path, True))
        assert rpath.read_bytes() == before_retry
        assert (root / "launches").read_text().splitlines().count("must-not-launch") == later_count + 1

    ownership = _ownership_regression(root, engine, repo, call, streamed)

    single = root / "single"
    call([engine, "capture-command", "--working-directory", repo, "--output-dir", single,
          "--", sys.executable, "-c", "import sys;print('single');print('single-err',file=sys.stderr)"])
    sr = json.loads(Path(json.loads((single / "index.json").read_text())["receipts"][0]["receipt"]).read_text())
    assert sr["argv"][0] == sys.executable and sr["exit_code"] == 0
    call([engine, "capture-command", "--working-directory", repo, "--output-dir", root / "timeout",
          "--timeout-ms", "150", "--", sys.executable, str(backend), "timeout"], 1)
    tr = json.loads(Path(json.loads((root / "timeout/index.json").read_text())["receipts"][0]["receipt"]).read_text())
    assert tr["timed_out"] and tr["cleanup"] == "complete" and tr["signal"] is not None
    call([engine, "capture-command", "--working-directory", repo, "--output-dir", root / "spawn-failure",
          "--", str(root / "does-not-exist")], 1)
    sp = json.loads(Path(json.loads((root / "spawn-failure/index.json").read_text())["receipts"][0]["receipt"]).read_text())
    assert sp["spawn_error"] and sp["exit_code"] is None
    (root / "scenarios.json").write_text(json.dumps({
        "host": sys.platform, "stream_before_exit": True, "failure_stops_third": True,
        "ownership_regression": ownership,
        "same_argv_retry_skips_first": True, "failures_immutable": True,
        "identity_parity": identity, "pre_spawn_refusals": ["argv", "timeout", "inherited-setting", "tree", "missing-stream", "stream-digest", "partial-receipt", "partial-started", "incomplete-started", "cleanup-pending", "live-controller"],
        "explicit_abort_and_sigint": "detached descendants absent from ps including zombies; interrupted attempts retained and restart completed",
        "single_timeout_spawn_failure": True, "report_reader": "direct actual receipts: successful=passed, exit7=failed, aborted exit0=failed; no pass adapter",
        "external_semantic_approval": "not claimed"
    }, indent=2))
