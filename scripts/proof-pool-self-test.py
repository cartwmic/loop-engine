#!/usr/bin/env python3
"""Exercise the real process pool, ordered hops and owned cleanup."""
import argparse
import json
import os
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

        # The pool's process-table inspection itself is a short-lived child.
        # A successful command must not turn a stale ps row into caller-owned
        # or adopted work.
        inspection_report = proof_pool.run(
            [{"name": "inspection-success", "command": [
                sys.executable, "-c",
                "import os,subprocess; "
                "subprocess.check_output(['ps','-p',str(os.getpid()),'-o','pid='], text=True)",
            ]}],
            root=root / "inspection-success", limit=1, timeout=10,
        )
        assert inspection_report["status"] == "passed", inspection_report
        assert inspection_report["cleanup"]["verified"], inspection_report
        assert inspection_report["caller_children"] == [], inspection_report
        results["inspection-success"] = inspection_report

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

        # A detached orphan can escape the ancestry sample before the pool
        # sees it.  The hook deliberately suppresses discover() after the
        # child announces its PID; only the kernel-parent relationship may
        # establish ownership here.  The child changes session before it is
        # adopted, so process groups cannot accidentally prove the case.
        def run_unobserved_adoption(name, survivor):
            case_root = root / name
            case_root.mkdir()
            pid_path = case_root / "orphan-pid"
            suppress_path = case_root / "suppress-ancestry"
            escaped_path = case_root / "escaped-before-discover"
            release_path = case_root / "release"
            finished_path = case_root / "finished"
            code = r"""
import os
import signal
import sys
import time
from pathlib import Path

root = Path(sys.argv[1])
pool_pid = int(sys.argv[2])
survivor = sys.argv[3] == "survivor"
(root / "suppress-ancestry").write_text("suppress sampled ancestry\n")
intermediate = os.fork()
if intermediate == 0:
    orphan = os.fork()
    if orphan == 0:
        if survivor:
            signal.signal(signal.SIGTERM, signal.SIG_IGN)
        os.setsid()
        deadline = time.monotonic() + 8
        while os.getppid() != pool_pid and time.monotonic() < deadline:
            time.sleep(.01)
        if os.getppid() != pool_pid:
            os._exit(3)
        (root / "orphan-pid").write_text(str(os.getpid()))
        if survivor:
            while True:
                time.sleep(1)
        while not (root / "release").exists():
            time.sleep(.01)
        os._exit(0)
    os._exit(0)

os.waitpid(intermediate, 0)
deadline = time.monotonic() + 6
while not (root / "orphan-pid").exists() and time.monotonic() < deadline:
    time.sleep(.01)
if not (root / "orphan-pid").exists():
    raise SystemExit("adopted orphan handshake timed out")
(root / "ready").write_text("the worker observed the adopted child\n")
if survivor:
    raise SystemExit(0)
pid = int((root / "orphan-pid").read_text())
while time.monotonic() < deadline:
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        (root / "finished").write_text("pool reaped adopted orphan\n")
        break
    time.sleep(.01)
else:
    raise SystemExit("adopted orphan remained until job deadline")
"""
            original_discover = proof_pool.discover

            def suppress_ancestry_discovery(job, table):
                if suppress_path.exists():
                    if pid_path.exists():
                        pid = int(pid_path.read_text())
                        if pid in table and table[pid][0] == os.getpid() and not escaped_path.exists():
                            escaped_path.write_text("direct child before discover\n")
                            if not survivor:
                                release_path.write_text("release after kernel adoption\n")
                    # Do not let the sampled former ancestry claim this
                    # child; discover_adopted must account for it.
                    return
                original_discover(job, table)

            proof_pool.discover = suppress_ancestry_discovery
            try:
                report = proof_pool.run(
                    [{"name": name, "command": [
                        sys.executable, "-c", code, str(case_root),
                        str(os.getpid()), "survivor" if survivor else "cooperative",
                    ]}],
                    root=case_root / "pool", limit=1, timeout=10,
                )
            finally:
                proof_pool.discover = original_discover
            orphan_pid = int(pid_path.read_text())
            gone_deadline = time.monotonic() + 8
            while orphan_pid in proof_pool.processes() and time.monotonic() < gone_deadline:
                time.sleep(.05)
            assert escaped_path.is_file(), (name, report)
            assert orphan_pid not in proof_pool.processes(), (name, report)
            assert any(item["pid"] == orphan_pid for item in report["adopted_children"]), report
            assert report["cleanup"]["verified"], report
            if survivor:
                assert report["status"] == "failed", report
                assert report["cleanup"]["adopted_live"], report
                assert any(pid == orphan_pid and sig == 9
                           for pid, sig in report["cleanup"]["signals"]), report
            else:
                assert report["status"] == "passed", report
                assert finished_path.is_file(), report
            return report

        results["unobserved-adopted-setsid"] = run_unobserved_adoption(
            "unobserved-adopted-setsid", survivor=False
        )
        results["unobserved-surviving-detached"] = run_unobserved_adoption(
            "unobserved-surviving-detached", survivor=True
        )

        # An unattributed adopted child can belong to a still-running primary.
        # A completed quick job must retain its slot until that primary also
        # finishes, rather than failing the quick job on a global leak check.
        barrier_root = root / "concurrent-adopted-barrier"
        barrier_root.mkdir()
        barrier_suppress = barrier_root / "suppress-ancestry"
        barrier_suppress.write_text("force kernel adoption for this regression\n")
        barrier_waiter = r"""
import os
import sys
import time
from pathlib import Path

root = Path(sys.argv[1])
pool_pid = int(sys.argv[2])
intermediate = os.fork()
if intermediate == 0:
    orphan = os.fork()
    if orphan == 0:
        os.setsid()
        deadline = time.monotonic() + 8
        while os.getppid() != pool_pid and time.monotonic() < deadline:
            time.sleep(.01)
        if os.getppid() != pool_pid:
            os._exit(3)
        (root / "orphan-pid").write_text(str(os.getpid()))
        (root / "orphan-ready").write_text("adopted child is live\n")
        while not (root / "release").exists():
            time.sleep(.01)
        os._exit(0)
    os._exit(0)

os.waitpid(intermediate, 0)
deadline = time.monotonic() + 8
while not (root / "orphan-ready").exists() and time.monotonic() < deadline:
    time.sleep(.01)
if not (root / "orphan-ready").exists():
    raise SystemExit("adopted child handshake timed out")
while not (root / "quick-completed").exists() and time.monotonic() < deadline:
    time.sleep(.01)
if not (root / "quick-completed").exists():
    raise SystemExit("quick primary did not complete")
# Leave the adopted child live long enough for the pool to observe the
# completed quick primary before this waiter releases it.
time.sleep(.25)
(root / "release").write_text("release after quick completion\n")
pid = int((root / "orphan-pid").read_text())
while time.monotonic() < deadline:
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        (root / "waiter-finished").write_text("adopted child was reaped\n")
        break
    time.sleep(.01)
else:
    raise SystemExit("adopted child remained until waiter deadline")
"""
        barrier_quick = r"""
import sys
import time
from pathlib import Path

root = Path(sys.argv[1])
deadline = time.monotonic() + 8
while not (root / "orphan-ready").exists() and time.monotonic() < deadline:
    time.sleep(.01)
if not (root / "orphan-ready").exists():
    raise SystemExit("orphan-ready handshake timed out")
(root / "quick-completed").write_text("quick primary completed first\n")
"""
        barrier_original_discover = proof_pool.discover

        def barrier_discover(job, table):
            if barrier_suppress.exists():
                return
            barrier_original_discover(job, table)

        proof_pool.discover = barrier_discover
        try:
            barrier_report = proof_pool.run(
                [{"name": "waiter", "command": [
                    sys.executable, "-c", barrier_waiter,
                    str(barrier_root), str(os.getpid()),
                ]}, {"name": "quick", "command": [
                    sys.executable, "-c", barrier_quick, str(barrier_root),
                ]}],
                root=barrier_root / "pool", limit=2, timeout=10,
            )
        finally:
            proof_pool.discover = barrier_original_discover
        assert barrier_report["status"] == "passed", barrier_report
        assert barrier_report["cleanup"]["verified"], barrier_report
        assert all(row["status"] == "passed" for row in barrier_report["jobs"]), barrier_report
        assert (barrier_root / "quick-completed").is_file(), barrier_report
        assert (barrier_root / "waiter-finished").is_file(), barrier_report
        orphan_pid = int((barrier_root / "orphan-pid").read_text())
        assert any(item["pid"] == orphan_pid
                   for item in barrier_report["adopted_children"]), barrier_report
        results["concurrent-adopted-barrier"] = barrier_report

        # A direct child that existed before the pool visit remains caller
        # owned, including its exit status.  The pool must not waitpid it.
        caller_owned = subprocess.Popen([
            sys.executable, "-c", "import time; time.sleep(.25); raise SystemExit(23)"
        ])
        caller_report = proof_pool.run(
            [{"name": "caller-owned", "command": [
                sys.executable, "-c", "import time; time.sleep(.5)"
            ]}],
            root=root / "caller-owned", limit=1, timeout=10,
        )
        assert caller_report["status"] == "passed", caller_report
        assert any(item["pid"] == caller_owned.pid
                   for item in caller_report["caller_children"]), caller_report
        assert caller_owned.wait(timeout=5) == 23, caller_report
        results["caller-owned-exit-status"] = caller_report

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
    linux_note = ("Linux adoption regressions executed"
                  if sys.platform.startswith("linux")
                  else "Linux adoption regressions skipped on non-Linux")
    print(json.dumps({"status": "passed", "proof": str(root / "self-test.json"),
                      "assertions": "limits 1/2/3, ordered hops, serial, nonzero, queue, timeout, detached resistant descendant reaped, delayed session-change cleanup, active adopted orphan reaped before deadline, inspection subprocess success, unobserved setsid adoption, surviving adopted descendant fails closed, concurrent adopted barrier, caller child exit preserved, nested refusal, private compile targets; " + linux_note}, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
