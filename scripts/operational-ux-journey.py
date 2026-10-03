#!/usr/bin/env python3
"""Public operational UX fixture entry point; case owners add public-path proofs.

This is not a test runner: existing commands remain the proof implementations.
Unimplemented cases fail closed, never produce a passing scenario receipt.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import sys
import subprocess

from test_contract import ContractError, ROOT, repository_proof_identity

CASES = ("packaged", "delivery", "bookends")


def absolute_directory(value: str) -> Path:
    path = Path(value)
    if not path.is_absolute() or not path.is_dir():
        raise argparse.ArgumentTypeError("requires an existing absolute directory")
    return path.resolve()


def output_directory(value: str) -> Path:
    path = Path(value)
    if not path.is_absolute():
        raise argparse.ArgumentTypeError("requires an absolute output directory")
    path = path.resolve()
    if path == ROOT or ROOT in path.parents:
        raise argparse.ArgumentTypeError("fixture output must be outside the checkout")
    if path.exists() and not path.is_dir():
        raise argparse.ArgumentTypeError("output is not a directory")
    return path


def run_case(args):
    if args.case == "delivery":
        from operational_ux_delivery import delivery_case
        delivery_case(args)
    elif args.case == "bookends":
        from operational_ux_bookends import bookends_case
        bookends_case(args)
    else:
        from operational_ux_packaged import packaged_case
        packaged_case(args)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--case", choices=CASES, required=True)
    parser.add_argument("--binary-dir", type=absolute_directory, required=True)
    parser.add_argument("--released-root", type=absolute_directory, required=True)
    parser.add_argument("--output-root", type=output_directory, required=True)
    args = parser.parse_args()
    args.output_root.mkdir(parents=True, exist_ok=True)
    # Never overwrite an earlier failed or successful attempt.
    import tempfile
    attempt = Path(tempfile.mkdtemp(prefix=f"{args.case}-", dir=args.output_root))
    args.attempt = attempt
    before = repository_proof_identity(ROOT)
    try:
        run_case(args)
    except (ContractError, OSError, AssertionError, ValueError, subprocess.SubprocessError) as error:
        outcome = {"case": args.case, "status": "failed", "diagnostic": str(error)}
        exit_code = 1
    else:
        outcome = {"case": args.case, "status": "passed"}
        exit_code = 0
    outcome.update(repository_before=before, repository_after=repository_proof_identity(ROOT))
    (attempt / "outcome.json").write_text(json.dumps(outcome, indent=2) + "\n")
    print(json.dumps({**outcome, "artifact_root": str(attempt)}))
    return exit_code


if __name__ == "__main__":
    sys.exit(main())
