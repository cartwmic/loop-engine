#!/usr/bin/env python3
"""Assert the one-workspace-integration-target preservation contract.

The final contract is deliberately based on Cargo metadata and compiler
artifacts, not on a count of source files.  A package's nested Rust modules
are sources, not additional Cargo integration targets.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path
from typing import Any, Sequence

# Allow this script to be run directly from any working directory.
sys.path.insert(0, str(Path(__file__).resolve().parent))
from test_contract import (  # noqa: E402
    ContractError,
    REQUIRED_BINARIES,
    binary_targets,
    cargo_metadata,
    compiler_artifact_targets,
    integration_targets,
    topology_errors,
)


def self_test() -> int:
    expected_binaries = {
        (package, name): {"package": package, "target": name}
        for package, name, _kind in REQUIRED_BINARIES
    }
    one_target = [
        {
            "id": "workspace-integration/workspace",
            "explicit": True,
        }
    ]
    emitted = [{"id": "workspace-integration/workspace"}]
    if topology_errors(one_target, expected_binaries, emitted):
        raise AssertionError("valid one-target topology was rejected")

    many_targets = [
        {"id": "bookends-check/cli", "explicit": False},
        {"id": "loop-reference-fixtures/reference_providers", "explicit": False},
    ]
    errors = topology_errors(many_targets, expected_binaries, [*many_targets])
    joined = "\n".join(errors)
    if "expected exactly one Cargo integration-test target across the workspace; found 2" not in joined:
        raise AssertionError("self-test did not reject multiple integration targets")
    if "extra integration targets: bookends-check/cli, loop-reference-fixtures/reference_providers" not in joined:
        raise AssertionError("self-test did not name every extra target")
    if "package auto-discovered integration roots" not in joined:
        raise AssertionError("self-test did not reject auto-discovered roots")

    missing_binaries = dict(expected_binaries)
    missing_binaries.pop(("loop-cli", "loop-engine"))
    errors = topology_errors(one_target, missing_binaries, emitted)
    if not any("loop-cli/loop-engine" in error for error in errors):
        raise AssertionError("self-test did not reject a missing required binary")

    artifact_mismatch = topology_errors(
        one_target,
        expected_binaries,
        [{"id": "workspace-integration/other"}],
    )
    if not any("metadata/compiler-artifact integration target mismatch" in error for error in artifact_mismatch):
        raise AssertionError("self-test did not reject metadata/artifact mismatch")

    print(
        "integration-test topology assertion self-test passed: "
        "multi-root, auto-root, binary, and compiler-artifact failures rejected"
    )
    return 0


def parse_args(argv: Sequence[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, default=Path.cwd())
    parser.add_argument(
        "--compiler-artifacts",
        type=Path,
        help="Cargo --message-format=json output to inspect for emitted integration executables",
    )
    parser.add_argument("--self-test", action="store_true")
    return parser.parse_args(argv)


def run(args: argparse.Namespace) -> int:
    repo = args.repo.resolve()
    metadata: dict[str, Any] = cargo_metadata(repo)
    targets = integration_targets(metadata, repo)
    binaries = binary_targets(metadata, repo)
    emitted = (
        compiler_artifact_targets(args.compiler_artifacts.resolve(), repo)
        if args.compiler_artifacts is not None
        else None
    )
    errors = topology_errors(targets, binaries, emitted)
    if errors:
        raise ContractError("\n".join(errors))
    print(
        "integration-test topology ok: "
        f"targets=1 executable={emitted[0]['executable'] if emitted else 'not inspected'}"
    )
    return 0


def main(argv: Sequence[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    if args.self_test:
        if args.compiler_artifacts is not None or args.repo != Path.cwd():
            print("--self-test cannot be combined with --repo or --compiler-artifacts", file=sys.stderr)
            return 2
        return self_test()
    try:
        return run(args)
    except ContractError as error:
        print(f"integration-test topology contract failed:\n{error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
