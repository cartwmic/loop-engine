#!/usr/bin/env python3
"""Run the release-after-merge Python tests in parallel shards.

Same tests as `python3 -m unittest scripts/test_release_after_merge.py`, split
into N groups that run as separate unittest processes at the same time. Each
shard writes its own log. The stage passes only if every shard passes and the
shards together ran every test that was discovered.
"""
from __future__ import annotations

import argparse
import re
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DEFAULT_MODULE = "scripts.test_release_after_merge"


def test_ids(module: str) -> list[str]:
    sys.path.insert(0, str(ROOT))

    def walk(suite: unittest.TestSuite):
        for item in suite:
            if isinstance(item, unittest.TestSuite):
                yield from walk(item)
            else:
                yield item.id()

    loaded = unittest.defaultTestLoader.loadTestsFromName(module)
    return sorted(walk(loaded))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--shards", type=int, default=8)
    parser.add_argument("--module", default=DEFAULT_MODULE)
    parser.add_argument("--log-dir", type=Path, help="default: a new directory under the system temporary directory")
    args = parser.parse_args()
    if args.shards < 1:
        parser.error("--shards must be at least 1")

    ids = test_ids(args.module)
    if not ids:
        print(f"FAIL: no tests found in {args.module}", file=sys.stderr)
        return 1
    groups = [ids[i :: args.shards] for i in range(args.shards)]
    groups = [g for g in groups if g]
    log_dir = args.log_dir or Path(tempfile.mkdtemp(prefix="release-tests-"))
    log_dir.mkdir(parents=True, exist_ok=True)

    procs = []
    for n, group in enumerate(groups, 1):
        log = log_dir / f"shard-{n}.log"
        handle = log.open("w")
        proc = subprocess.Popen(
            [sys.executable, "-m", "unittest", *group],
            cwd=ROOT, stdout=handle, stderr=subprocess.STDOUT,
        )
        procs.append((n, group, log, handle, proc))

    ran_total, failed = 0, []
    for n, group, log, handle, proc in procs:
        code = proc.wait()
        handle.close()
        match = re.search(r"^Ran (\d+) tests?", log.read_text(), re.M)
        ran = int(match.group(1)) if match else 0
        ran_total += ran
        ok = code == 0 and ran == len(group)
        print(f"shard {n}/{len(groups)}: {'PASS' if ok else 'FAIL'} ({ran}/{len(group)} tests) {log}")
        if not ok:
            failed.append(n)
            print(log.read_text(), end="")

    if failed:
        print(f"FAIL: shards {failed} failed; ran {ran_total} of {len(ids)} tests", file=sys.stderr)
        return 1
    print(f"PASS: {ran_total} tests in {len(groups)} shards (logs in {log_dir})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
