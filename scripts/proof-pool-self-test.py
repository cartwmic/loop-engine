#!/usr/bin/env python3
"""Exercise the real process pool, ordered hops and owned cleanup."""
import argparse
import json
from pathlib import Path
import subprocess
import sys
import tempfile

import proof_pool


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--work-root", type=Path)
    args = parser.parse_args()
    root = args.work_root or Path(tempfile.mkdtemp(prefix="proof-pool-self-test-"))
    root.mkdir(parents=True, exist_ok=True)
    results = {}
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
    child = ("import os,signal,time; from pathlib import Path; "
             "signal.signal(signal.SIGTERM,signal.SIG_IGN); "
             f"Path({str(pidfile)!r}).write_text(str(os.getpid())); time.sleep(90)")
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
        assert int(pidfile.read_text()) not in proof_pool.processes(), report
        assert any(sig == 9 for _, sig in report["cleanup"]["signals"]), report
        if mode == "failure":
            assert report["jobs"][1]["result"]["status"] == "failed"
            assert "exit status 7" in report["jobs"][1]["result"]["error"]
        else:
            assert report["jobs"][0]["status"] == "timed-out"
        results[mode] = report

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
                      "assertions": "limits 1/2/3, ordered hops, serial, nonzero, queue, timeout, detached resistant descendant reaped, nested refusal, private compile targets"}, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
