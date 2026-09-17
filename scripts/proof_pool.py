#!/usr/bin/env python3
"""One bounded process budget for independent public proofs (POSIX hosts)."""
import argparse
import ctypes
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time
import traceback


class PoolFailure(RuntimeError):
    pass


def save(path, value):
    Path(path).write_text(json.dumps(value, indent=2, default=str) + "\n")


def processes():
    rows = subprocess.check_output(
        ["ps", "-axo", "pid=,ppid=,pgid=,stat=,lstart="], text=True
    )
    table = {}
    for line in rows.splitlines():
        fields = line.split(None, 4)
        if len(fields) != 5:
            continue
        pid, parent, group, state, started = fields
        table[int(pid)] = (int(parent), int(group), state, started)
    if sys.platform.startswith("linux"):
        # ps can report the inspection process after check_output has reaped it.
        # /proc's children list is the current kernel-owned direct-child set.
        children_path = Path(f"/proc/self/task/{os.getpid()}/children")
        children = {int(pid) for pid in children_path.read_text().split()}
        table = {
            pid: row for pid, row in table.items()
            if row[0] != os.getpid() or pid in children
        }
    return table


def discover(job, table):
    owned = job["owned"]
    identities = job.setdefault("identities", {})
    owned.add(job["process"].pid)
    if job["process"].pid in table:
        identities.setdefault(job["process"].pid, table[job["process"].pid][3])
    def current(pid):
        row = table.get(pid)
        return row is not None and identities.get(pid, row[3]) == row[3]

    # Retain observed descendants even if a facade exits or they create groups,
    # but do not follow a reused parent PID into an unrelated process tree.
    while True:
        new = {
            pid
            for pid, (parent, group, _, _) in table.items()
            if (parent in owned and current(parent))
            or (group == job["process"].pid and current(job["process"].pid))
        }
        if new <= owned:
            break
        for pid in new - owned:
            identities[pid] = table[pid][3]
        owned.update(new)


def live_owned(job, table):
    """Ignore a PID reused after its recorded process start time."""
    identities = job.get("identities", {})
    return {
        pid
        for pid in job["owned"]
        if pid in table and identities.get(pid, table[pid][3]) == table[pid][3]
    }


def direct_children(table):
    """Return this pool process's direct children by PID and start identity."""
    parent = os.getpid()
    return {pid: row[3] for pid, row in table.items() if row[0] == parent}


def _job_identity_state(jobs):
    identities = {}
    pids = set()
    for job in jobs:
        pids.update(job["owned"])
        for pid, started in job.get("identities", {}).items():
            identities[pid] = started
    return identities, pids


def remember_job_identities(jobs, history):
    for job in jobs:
        history.update(job.get("identities", {}))


def discover_adopted(adopted, baseline, jobs, table, history):
    """Track children reparented to this Linux subreaper.

    A child that was already ours before the pool visit is caller-owned and is
    never waited for.  New direct children are the only kernel-visible record
    left when a descendant escaped ancestry sampling; retain their start-time
    identities so PID reuse cannot turn cleanup into a guess.  Descendants of
    an adopted child are retained as well, without using waitpid(-1).
    """
    if not sys.platform.startswith("linux"):
        return []

    job_identities, job_pids = _job_identity_state(jobs)
    roots = {job["process"].pid for job in jobs}
    new = []

    def protected(pid, started):
        if pid in roots or pid in job_pids:
            expected = job_identities.get(pid)
            if expected is not None and expected != started:
                raise PoolFailure(f"owned PID {pid} changed process identity")
            return True
        if pid in adopted:
            if adopted[pid] != started:
                raise PoolFailure(f"adopted PID {pid} was reused before cleanup")
            return True
        if pid in baseline:
            if baseline[pid] != started:
                raise PoolFailure(f"caller-owned PID {pid} was reused during cleanup")
            return True
        if pid in history:
            if history[pid] != started:
                raise PoolFailure(f"known PID {pid} was reused during cleanup")
            return True
        return False

    for pid, row in table.items():
        if pid in roots or pid in job_pids:
            expected = job_identities.get(pid)
            if expected is not None and expected != row[3]:
                raise PoolFailure(f"owned PID {pid} changed process identity")
        elif pid in adopted and adopted[pid] != row[3]:
            raise PoolFailure(f"adopted PID {pid} was reused before cleanup")
        elif pid in baseline and baseline[pid] != row[3]:
            raise PoolFailure(f"caller-owned PID {pid} was reused during cleanup")
        elif pid in history and history[pid] != row[3]:
            raise PoolFailure(f"known PID {pid} was reused during cleanup")
        if row[0] != os.getpid() or protected(pid, row[3]):
            continue
        adopted[pid] = row[3]
        history[pid] = row[3]
        new.append(pid)

    # A surviving adopted intermediate may retain descendants after it leaves
    # the pool's direct-child list.  Follow parent identities, not PID guesses.
    while True:
        additions = []
        for pid, row in table.items():
            parent, _, _, started = row
            if pid in adopted or protected(pid, started):
                continue
            parent_started = adopted.get(parent)
            parent_row = table.get(parent)
            if parent_started is None or parent_row is None or parent_row[3] != parent_started:
                continue
            adopted[pid] = started
            history[pid] = started
            additions.append(pid)
        if not additions:
            break
        new.extend(additions)
    return new


def live_adopted(adopted, table):
    return {
        pid for pid, started in adopted.items()
        if pid in table and table[pid][3] == started
    }


def reap_adopted(adopted):
    if not sys.platform.startswith("linux"):
        return
    for pid in adopted:
        try:
            os.waitpid(pid, os.WNOHANG)
        except ChildProcessError:
            pass


def reap(job):
    job["process"].poll()
    if sys.platform.startswith("linux"):
        for pid in job["owned"] - {job["process"].pid}:
            try:
                os.waitpid(pid, os.WNOHANG)
            except ChildProcessError:
                pass


def cleanup(active, adopted, baseline, history):
    """Stop leaves first, keep parents available to reap, then verify absence."""
    start = time.monotonic()
    sent = []
    adoption_error = None
    adopted_live_seen = False
    while time.monotonic() - start < 8:
        table = processes()
        try:
            for job in active:
                discover(job, table)
            remember_job_identities(active, history)
            discover_adopted(adopted, baseline, active, table, history)
        except PoolFailure as error:
            adoption_error = adoption_error or str(error)
        for job in active:
            reap(job)
        if any(not table[pid][2].startswith("Z") for pid in live_adopted(adopted, table)):
            adopted_live_seen = True
        reap_adopted(adopted)
        # Do not signal a PID from the pre-wait process table: it may already
        # have been reaped or reused by the time cleanup decides what remains.
        table = processes()
        live = set().union(
            *(live_owned(j, table) for j in active), live_adopted(adopted, table)
        ) if active else live_adopted(adopted, table)
        if not live:
            return {"verified": adoption_error is None, "remaining_pids": [],
                    "signals": sent, "adopted_live": adopted_live_seen,
                    "adoption_error": adoption_error,
                    "wall_seconds": time.monotonic() - start}
        elapsed = time.monotonic() - start
        parents = {table[p][0] for p in live}
        for pid in live:
            # Give children a chance to be reaped before stopping their parents.
            if pid in parents and elapsed < 2:
                continue
            sig = signal.SIGTERM if elapsed < 0.5 else signal.SIGKILL
            try:
                os.kill(pid, sig)
                if [pid, int(sig)] not in sent:
                    sent.append([pid, int(sig)])
            except ProcessLookupError:
                pass
        time.sleep(0.05)
    table = processes()
    remaining = sorted(set().union(
        *(live_owned(j, table) for j in active), live_adopted(adopted, table)
    ) if active else live_adopted(adopted, table))
    return {"verified": not remaining and adoption_error is None,
            "remaining_pids": remaining, "signals": sent,
            "adopted_live": adopted_live_seen, "adoption_error": adoption_error,
            "wall_seconds": time.monotonic() - start}


def run(jobs, *, root, limit=2, timeout=1200):
    # Reaping belongs to this pool visit, not to later caller-owned workflows.
    previous = None
    if sys.platform.startswith("linux"):
        libc = ctypes.CDLL(None, use_errno=True)
        previous = ctypes.c_int()
        if libc.prctl(37, ctypes.byref(previous), 0, 0, 0):
            raise PoolFailure("cannot read caller child-subreaper setting")
    try:
        return _run(jobs, root=root, limit=limit, timeout=timeout)
    finally:
        if previous is not None and libc.prctl(36, previous.value, 0, 0, 0):
            raise PoolFailure("cannot restore caller child-subreaper setting")


def _run(jobs, *, root, limit, timeout):
    if os.environ.get("LOOP_PROOF_POOL_ACTIVE"):
        raise PoolFailure("nested proof pools are forbidden")
    if type(limit) is not int or limit < 1 or timeout <= 0:
        raise PoolFailure("jobs and timeout must be positive")
    names = [j["name"] for j in jobs]
    if not names or len(names) != len(set(names)):
        raise PoolFailure("proof inventory must be nonempty and unique")
    # Existing direct children belong to the caller.  They may be zombies whose
    # exit status the caller still needs, so never wait for them in this pool.
    baseline = direct_children(processes())
    adopted = {}
    history = {}
    root = Path(root).resolve()
    root.mkdir(parents=True, exist_ok=False)
    # Adopt orphaned owned descendants on Linux so verification includes reaping.
    if sys.platform.startswith("linux"):
        if ctypes.CDLL(None, use_errno=True).prctl(36, 1, 0, 0, 0):
            raise PoolFailure("cannot enable owned descendant reaping")
    report = {"status": "running", "job_limit": limit, "peak_jobs": 0,
              "inventory": names, "activity": [],
              "caller_children": [{"pid": pid, "started": started}
                                  for pid, started in sorted(baseline.items())],
              "jobs": [{"name": name, "status": "not-run"} for name in names]}
    active = []
    cursor = 0
    started = time.monotonic()
    failure = None

    def activity(name, event):
        report["activity"].append({"name": name, "event": event,
                                   "monotonic": time.monotonic(), "active_jobs": len(active)})
        report["peak_jobs"] = max(report["peak_jobs"], len(active))

    def interrupted(signum, frame):
        raise PoolFailure(f"pool interrupted by signal {signum}")

    old_signals = {s: signal.signal(s, interrupted) for s in (signal.SIGTERM, signal.SIGINT)}
    try:
        while active or cursor < len(jobs):
            # Inspect failures before admitting additional queued work.
            table = processes()
            for job in list(active):
                discover(job, table)
            remember_job_identities(active, history)
            # A descendant can be reparented directly to this subreaper before
            # its former ancestry is sampled.  Account for that kernel-owned
            # child before checking any worker's exit.
            discover_adopted(adopted, baseline, active, table, history)
            for job in list(active):
                # Reap adopted descendants while the job is still running.
                reap(job)
            reap_adopted(adopted)
            for job in list(active):
                code = job["process"].poll()
                elapsed = time.monotonic() - job["started"]
                if code is None and elapsed < timeout:
                    continue
                row = job["row"]
                row.update(exit_code=code, wall_seconds=elapsed)
                if code is None:
                    row["status"] = "timed-out"
                    raise PoolFailure(f"{row['name']}: timed out after {timeout}s")
                reap(job)
                current_table = processes()
                discover_adopted(adopted, baseline, active, current_table, history)
                reap_adopted(adopted)
                current_table = processes()
                remaining = live_owned(job, current_table) - {job["process"].pid}
                adopted_remaining = live_adopted(adopted, current_table)
                result_path = Path(row["result_path"])
                result = json.loads(result_path.read_text()) if result_path.exists() else {}
                row["result"] = result
                if code or result.get("status") != "passed" or remaining:
                    row["status"] = "failed"
                    raise PoolFailure(
                        f"{row['name']}: exit={code}, remaining={sorted(remaining)}, "
                        f"adopted_remaining={sorted(adopted_remaining)}, result={result}"
                    )
                if adopted_remaining:
                    # Adoption is intentionally unattributed.  Keep this
                    # completed primary in the active set while another
                    # primary is still running; only the final primary
                    # completion can establish that the adopted work leaked.
                    if any(other is not job and other["process"].poll() is None
                           for other in active):
                        row["status"] = "waiting-adopted"
                        continue
                    row["status"] = "failed"
                    raise PoolFailure(
                        f"{row['name']}: exit={code}, remaining={sorted(remaining)}, "
                        f"adopted_remaining={sorted(adopted_remaining)}, result={result}"
                    )
                row["status"] = "passed"
                job["stdout"].close()
                job["stderr"].close()
                active.remove(job)
                activity(row["name"], "end")
            while cursor < len(jobs) and len(active) < limit:
                spec = jobs[cursor]
                row = report["jobs"][cursor]
                directory = root / row["name"]
                directory.mkdir()
                packet = directory / "packet.json"
                save(packet, spec)
                row.update(status="running", stdout=str(directory / "stdout"),
                           stderr=str(directory / "stderr"), result_path=str(directory / "result.json"),
                           started_monotonic=time.monotonic())
                env = dict(os.environ, LOOP_PROOF_POOL_ACTIVE="1", PYTHONUNBUFFERED="1")
                if spec.get("compiles"):
                    env["CARGO_TARGET_DIR"] = str(directory / "target")
                stdout = open(row["stdout"], "wb")
                stderr = open(row["stderr"], "wb")
                process = subprocess.Popen([sys.executable, str(Path(__file__).resolve()),
                    "--worker", str(packet)], stdout=stdout, stderr=stderr, env=env, start_new_session=True)
                # A reaped adopted PID may later be reused by a fresh worker;
                # the Popen identity now owns that PID instead.
                adopted.pop(process.pid, None)
                row["pid"] = process.pid
                active.append({"process": process, "owned": {process.pid}, "identities": {},
                               "row": row, "started": row["started_monotonic"],
                               "stdout": stdout, "stderr": stderr})
                cursor += 1
                activity(row["name"], "start")
            save(root / "summary.json", report)
            time.sleep(0.05)
    except BaseException as exc:
        failure = str(exc)
    finally:
        # Cleanup must itself not be interrupted into a false success.
        for sig in old_signals:
            signal.signal(sig, signal.SIG_IGN)
        remember_job_identities(active, history)
        receipt = cleanup(active, adopted, baseline, history)
        report["cleanup"] = receipt
        report["adopted_children"] = [{"pid": pid, "started": started}
                                       for pid, started in sorted(adopted.items())]
        for job in active:
            row = job["row"]
            if row["status"] == "running":
                row["status"] = "cancelled"
            row.update(exit_code=job["process"].poll(), wall_seconds=time.monotonic() - job["started"])
            job["stdout"].close()
            job["stderr"].close()
        for sig, handler in old_signals.items():
            signal.signal(sig, handler)
        report.update(status="passed" if not failure and receipt["verified"] and
                      not receipt["adopted_live"] and
                      all(r["status"] == "passed" for r in report["jobs"]) else "failed",
                      error=failure, wall_seconds=time.monotonic() - started)
        save(root / "summary.json", report)
    return report


def worker(packet):
    spec = json.loads(packet.read_text())
    start = time.monotonic()
    # Preserve every real subprocess.run command and result, including expected denials.
    original = subprocess.run
    def captured(*args, **kwargs):
        begin = time.monotonic()
        value = None
        error = None
        try:
            value = original(*args, **kwargs)
            return value
        except BaseException as exc:
            error = exc
            raise
        finally:
            def text(v):
                return v.decode(errors="replace") if isinstance(v, bytes) else v
            with (packet.parent / "commands.jsonl").open("a") as out:
                out.write(json.dumps({"argv": args[0] if args else kwargs.get("args"),
                    "cwd": str(kwargs.get("cwd", Path.cwd())), "wall_seconds": time.monotonic() - begin,
                    "exit_code": getattr(value, "returncode", None), "stdout": text(getattr(value, "stdout", None)),
                    "stderr": text(getattr(value, "stderr", None)), "error": str(error) if error else None}, default=str) + "\n")
    subprocess.run = captured
    result = {"status": "failed"}
    try:
        if "command" in spec:  # instrumented self-test jobs use the same process path
            subprocess.run(spec["command"], check=True)
            value = []
        else:
            import work_slot_journey
            value = getattr(work_slot_journey, spec["name"])(**{k: Path(v) for k, v in spec["kwargs"].items()})
        result.update(status="passed", returned_assertions=value)
    except BaseException:
        result["error"] = traceback.format_exc()
        print(result["error"], file=sys.stderr)
    result["wall_seconds"] = time.monotonic() - start
    save(packet.parent / "result.json", result)
    print(json.dumps(result, indent=2, default=str))
    return 0 if result["status"] == "passed" else 1


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--worker", type=Path, required=True)
    raise SystemExit(worker(parser.parse_args().worker))
