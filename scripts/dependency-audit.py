#!/usr/bin/env python3
"""Run the repository's pinned cargo-machete audit over the whole workspace.

The command deliberately checks the installed cargo-machete version before
running the read-only audit.  It never passes ``--fix``: candidate removal is
a manual, source- and test-validated manifest change.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PINNED_VERSION = "0.9.2"
VERSION_COMMAND = ["cargo", "machete", "--version"]
AUDIT_COMMAND = ["cargo", "machete", "--with-metadata", "--skip-target-dir"]


class AuditError(RuntimeError):
    """The pinned audit tool is unavailable or reports an unexpected version."""


def check_version() -> str:
    try:
        result = subprocess.run(
            VERSION_COMMAND,
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=False,
        )
    except OSError as error:
        raise AuditError(f"could not execute {' '.join(VERSION_COMMAND)}: {error}") from error

    reported = result.stdout.strip()
    if result.returncode != 0:
        detail = result.stderr.strip() or result.stdout.strip() or "no diagnostic"
        raise AuditError(
            f"{' '.join(VERSION_COMMAND)} failed with exit {result.returncode}: {detail}"
        )
    if not re.fullmatch(r"\d+\.\d+\.\d+", reported) or reported != PINNED_VERSION:
        raise AuditError(
            f"cargo-machete version mismatch: expected {PINNED_VERSION!r}, got {reported!r}"
        )
    return reported


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--check-version",
        action="store_true",
        help="check the exact configured cargo-machete version and stop",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        version = check_version()
    except AuditError as error:
        print(f"dependency audit failed closed: {error}", file=sys.stderr)
        return 1

    print(f"cargo-machete version {version} verified")
    if args.check_version:
        return 0

    print("+ " + " ".join(AUDIT_COMMAND), flush=True)
    result = subprocess.run(AUDIT_COMMAND, cwd=ROOT, check=False)
    return result.returncode


if __name__ == "__main__":
    raise SystemExit(main())
