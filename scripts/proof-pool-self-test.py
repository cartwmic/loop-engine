#!/usr/bin/env python3
"""Exercise the real process pool, ordered hops and owned cleanup."""
import argparse
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import time

import proof_pool


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--work-root", type=Path)
    args = parser.parse_args()
    root = args.work_root or Path(tempfile.mkdtemp(prefix="proof-pool-self-test-"))
    root.mkdir(parents=True, exist_ok=True)
    results = {}
    if sys.platform.startswith("linux"):
        import ctypes
        libc = ctypes.CDLL(None, use_errno=True)
        def subreaper():
            value = ctypes.c_int()
            assert libc.prctl(37, ctypes.byref(value), 0, 0, 0) == 0
            return value.value
        original = subreaper()
        try:
            for prior in (0, 1):
                assert libc.prctl(36, prior, 0, 0, 0) == 0
                for mode, code in (("success", "pass"), ("failure", "raise SystemExit(7)"),
                                   ("timeout", "import time; time.sleep(2)")):
                    name = f"subreaper-{prior}-{mode}"
                    report = proof_pool.run([{"name": "probe", "command": [sys.executable, "-c", code]}],
                                            root=root / name, timeout=.2 if mode == "timeout" else 10)
                    assert report["status"] == ("passed" if mode == "success" else "failed"), report
                    assert report["cleanup"]["verified"], report
                    assert subreaper() == prior, (name, "caller subreaper setting changed")
                    results[name] = report
        finally:
            assert libc.prctl(36, original, 0, 0, 0) == 0
    for limit in (1, 2, 3):
        jobs = []
        for n in range(5):
            log = root / f"hops-{limit}-{n}.json"
            code = ("import json,time; from pathlib import Path; rows=[]; "
                    "rows.append(['first-start',time.monotonic()]); time.sleep(.15); "
                    "rows.append(['first-end',time.monotonic()]); "
                    "rows.append(['second-start',time.monotonic()]); time.sleep(.15); "
                    "rows.append(['second-end',time.monotonic()]); "
                    f"Path({str(log)!r}).write_text(json.dumps(rows))")
            jobs.append({"name": f"ordered-{n}", "command": [sys.executable, "-c", code]})
        report = proof_pool.run(jobs, root=root / f"jobs-{limit}", limit=limit)
        assert report["status"] == "passed", report
        assert report["peak_jobs"] == limit
        for n in range(5):
            rows = json.loads((root / f"hops-{limit}-{n}.json").read_text())
            assert rows[1][1] <= rows[2][1] and rows[0][1] < rows[1][1] < rows[3][1]
        results[f"jobs-{limit}"] = report

    # A detached TERM-resistant grandchild is owned too, not only the worker group.
    pidfile = root / "resistant.pid"
    child = ("import os,signal,subprocess,time; from pathlib import Path; "
             "signal.signal(signal.SIGTERM,signal.SIG_IGN); "
             f"p=os.getpid(); group=subprocess.check_output(['ps','-p',str(p),'-o','pgid='],text=True).strip(); "
             f"started=subprocess.check_output(['ps','-p',str(p),'-o','lstart='],text=True).strip(); "
             f"Path({str(pidfile)!r}).write_text(f'{{p}}\\n{{group}}\\n{{started}}'); time.sleep(90)")
    wedge = ("import subprocess,sys,time; "
             f"p=subprocess.Popen([sys.executable,'-c',{child!r}],start_new_session=True); p.wait()")
    for mode in ("failure", "timeout"):
        jobs = [{"name": "wedge", "command": [sys.executable, "-c", wedge]},
                {"name": "failure", "command": [sys.executable, "-c", "import time; time.sleep(.7); raise SystemExit(7)" ]},
                {"name": "queued", "command": [sys.executable, "-c", "raise RuntimeError('must not run')"]}]
        if mode == "timeout":
            jobs.pop(1)
        report = proof_pool.run(jobs, root=root / mode, limit=2 if mode == "failure" else 1,
                                timeout=10 if mode == "failure" else 1)
        assert report["status"] == "failed" and report["cleanup"]["verified"], report
        assert report["jobs"][-1]["status"] == "not-run", report
        pid, group, started = pidfile.read_text().splitlines()
        current = proof_pool.processes().get(int(pid))
        assert current is None or (current[1], current[3]) != (int(group), started), report
        assert any(sig == 9 for _, sig in report["cleanup"]["signals"]), report
        if mode == "failure":
            assert report["jobs"][1]["result"]["status"] == "failed"
            assert "exit status 7" in report["jobs"][1]["result"]["error"]
        else:
            assert report["jobs"][0]["status"] == "timed-out"
        results[mode] = report

    # A descendant observed before it creates a new session remains owned even
    # after its process group changes. The marker is a readiness handshake from
    # the public pool entry point: the child cannot call setsid until discover
    # has recorded it.
    delayed_root = root / "delayed-session-change"
    delayed_root.mkdir()
    child_pid_path = delayed_root / "child-pid"
    discovered_path = delayed_root / "discovered"
    switched_path = delayed_root / "switched"
    finish_path = delayed_root / "finish"
    delayed_job = """\
import os
import sys
import time
from pathlib import Path

root = Path(sys.argv[1])
child = os.fork()
if child == 0:
    deadline = time.monotonic() + 15
    while not (root / "discovered").exists() and time.monotonic() < deadline:
        time.sleep(.01)
    if not (root / "discovered").exists():
        os._exit(3)
    os.setsid()
    (root / "switched").write_text(str(os.getpid()))
    while not (root / "finish").exists() and time.monotonic() < deadline:
        time.sleep(.01)
    os._exit(0)

(root / "child-pid").write_text(str(child))
deadline = time.monotonic() + 10
while not (root / "switched").exists() and time.monotonic() < deadline:
    time.sleep(.01)
assert (root / "switched").exists(), "readiness handshake timed out"
"""
    original_discover = proof_pool.discover
    observations = []

    def observed_discover(job, table):
        original_discover(job, table)
        if child_pid_path.exists() and not discovered_path.exists():
            child_pid = int(child_pid_path.read_text())
            if child_pid in job["owned"] and child_pid in table:
                observations.append({"pid": child_pid, "row": table[child_pid]})
                discovered_path.write_text("pool observed child before session change\\n")

    proof_pool.discover = observed_discover
    try:
        delayed_report = proof_pool.run(
            [{"name": "delayed-session-change", "command": [
                sys.executable, "-c", delayed_job, str(delayed_root)
            ]}],
            root=delayed_root / "pool",
            limit=1,
            timeout=12,
        )
    finally:
        proof_pool.discover = original_discover
        finish_path.write_text("driver requests orderly diagnostic child exit\\n")
    assert observations, "delayed session-change child was not discovered before setsid"
    assert switched_path.is_file(), "delayed session-change child never changed session"
    assert delayed_report["status"] == "failed", delayed_report
    assert delayed_report["cleanup"]["verified"], delayed_report
    assert delayed_report["jobs"][0]["status"] == "failed", delayed_report
    assert delayed_report["jobs"][0]["result"]["status"] == "passed", delayed_report
    child_pid = int(child_pid_path.read_text())
    deadline = time.monotonic() + 20
    while child_pid in proof_pool.processes() and time.monotonic() < deadline:
        time.sleep(.05)
    assert child_pid not in proof_pool.processes(), delayed_report
    results["delayed-session-change"] = delayed_report

    # A running Linux job waits for an adopted orphan to disappear. The pool
    # must reap that zombie during active polling, before the job's deadline.
    if sys.platform.startswith("linux"):
        active_reap_root = root / "active-adopted-orphan"
        active_reap_root.mkdir()
        orphan_pid_path = active_reap_root / "orphan-pid"
        waiting_path = active_reap_root / "waiting"
        reaped_path = active_reap_root / "reaped"
        active_reap_job = """\\
import os
import sys
import time
from pathlib import Path

root = Path(sys.argv[1])
intermediate = os.fork()
if intermediate == 0:
    orphan = os.fork()
    if orphan == 0:
        (root / "orphan-pid").write_text(str(os.getpid()))
        os._exit(0)
    os._exit(0)

os.waitpid(intermediate, 0)
deadline = time.monotonic() + 5
while not (root / "orphan-pid").exists() and time.monotonic() < deadline:
    time.sleep(.01)
if not (root / "orphan-pid").exists():
    raise SystemExit("orphan pid handshake timed out")
(root / "waiting").write_text("job is waiting for adopted orphan disappearance\\n")
pid = int((root / "orphan-pid").read_text())
while time.monotonic() < deadline:
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        (root / "reaped").write_text("adopted orphan disappeared\\n")
        break
    time.sleep(.01)
else:
    raise SystemExit("adopted orphan remained present until job deadline")
"""
        active_reap_report = proof_pool.run(
            [{"name": "active-adopted-orphan", "command": [
                sys.executable, "-c", active_reap_job, str(active_reap_root)
            ]}],
            root=active_reap_root / "pool", limit=1, timeout=10,
        )
        assert active_reap_report["status"] == "passed", active_reap_report
        assert active_reap_report["cleanup"]["verified"], active_reap_report
        assert active_reap_report["jobs"][0]["status"] == "passed", active_reap_report
        assert orphan_pid_path.is_file() and waiting_path.is_file() and reaped_path.is_file(), active_reap_report
        assert active_reap_report["jobs"][0]["wall_seconds"] < 5, active_reap_report
        results["active-adopted-orphan"] = active_reap_report

    nested = {"name": "nested", "command": [sys.executable, "-c",
        f"import sys;sys.path.insert(0,{str(Path(__file__).resolve().parent)!r}); import proof_pool; "
        f"proof_pool.run([{{'name':'no'}}],root={str(root / 'forbidden')!r})"]}
    report = proof_pool.run([nested], root=root / "nested", limit=2)
    assert report["status"] == "failed" and not (root / "forbidden").exists(), report
    results["nested-refused"] = report
    # Only compiling jobs get private target directories; no Cargo suite runs here.
    for compiles in (False, True):
        code = "import os; from pathlib import Path; "
        code += "assert Path(os.environ['CARGO_TARGET_DIR']).is_absolute()" if compiles else "assert 'LOOP_PROOF_POOL_ACTIVE' in os.environ"
        report = proof_pool.run([{"name": "target", "compiles": compiles,
            "command": [sys.executable, "-c", code]}], root=root / f"target-{compiles}")
        assert report["status"] == "passed", report
        results[f"target-{compiles}"] = report
    proof_pool.save(root / "self-test.json", results)
    print(json.dumps({"status": "passed", "proof": str(root / "self-test.json"),
                      "assertions": "limits 1/2/3, ordered hops, serial, nonzero, queue, timeout, detached resistant descendant reaped, delayed session-change cleanup, active adopted orphan reaped before deadline, nested refusal, private compile targets"}, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
