#!/usr/bin/env python3
"""Assert that the central integration suite retains the frozen test inventory.

The predecessor manifest is intentionally external runtime evidence.  The
checker consumes it but never replaces Cargo's discovered test names with a
hand-authored aggregate count.  In the final topology, T02 may provide a
mapping manifest whose ``baseline_case_id`` values are checked exactly.
"""

from __future__ import annotations

import argparse
import subprocess
import sys
from pathlib import Path
from typing import Any, Sequence

sys.path.insert(0, str(Path(__file__).resolve().parent))
from test_contract import (  # noqa: E402
    ContractError,
    REQUIRED_BINARIES,
    binary_targets,
    cargo_metadata,
    compiler_artifact_targets,
    integration_targets,
    inventory_manifest_errors,
    load_json,
    load_test_lists,
    source_files,
    source_test_inventory,
    topology_errors,
)


def _baseline_cases(baseline: dict[str, Any]) -> list[dict[str, Any]]:
    integration = baseline.get("integration")
    if not isinstance(integration, dict):
        raise ContractError("baseline integration section must be an object")
    cases = integration.get("cases")
    if not isinstance(cases, list) or not all(isinstance(case, dict) for case in cases):
        raise ContractError("baseline integration cases must be an array of objects")
    if not all(isinstance(case.get("id"), str) for case in cases):
        raise ContractError("baseline integration cases must have string ids")
    ids = [str(case["id"]) for case in cases]
    if len(ids) != len(set(ids)):
        raise ContractError("baseline integration case ids must be unique")
    return cases


def case_inventory_errors(
    baseline: dict[str, Any], current_cases: list[dict[str, Any]]
) -> list[str]:
    expected_cases = _baseline_cases(baseline)
    expected_ids = {str(case["id"]) for case in expected_cases}
    current_ids = [case.get("id") for case in current_cases]
    errors: list[str] = []
    if not all(isinstance(case_id, str) for case_id in current_ids):
        errors.append("discovered integration cases must have string ids")
        return errors
    actual_ids = set(current_ids)
    missing = sorted(expected_ids - actual_ids)
    extra = sorted(actual_ids - expected_ids)
    if missing:
        errors.append("missing baseline integration cases: " + ", ".join(missing))
    if extra:
        errors.append("unexpected integration cases: " + ", ".join(extra))
    if len(current_ids) != len(actual_ids):
        errors.append("discovered integration cases contain duplicate source ids")
    return errors


def source_hash_errors(baseline: dict[str, Any], current: list[dict[str, Any]]) -> list[str]:
    integration = baseline.get("integration")
    expected = integration.get("source_files") if isinstance(integration, dict) else None
    if not isinstance(expected, list) or not all(isinstance(item, dict) for item in expected):
        return ["baseline integration source_files must be an array of objects"]
    expected_map = {str(item.get("path")): item for item in expected}
    current_map = {str(item.get("path")): item for item in current}
    errors: list[str] = []
    missing = sorted(set(expected_map) - set(current_map))
    extra = sorted(set(current_map) - set(expected_map))
    if missing:
        errors.append("baseline integration sources missing from checkout: " + ", ".join(missing))
    if extra:
        errors.append("unexpected integration sources in checkout: " + ", ".join(extra))
    changed = sorted(
        path
        for path in set(expected_map) & set(current_map)
        if expected_map[path].get("sha256") != current_map[path].get("sha256")
    )
    if changed:
        errors.append("baseline integration source digest changed: " + ", ".join(changed))
    return errors


def current_test_lists(emitted: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """Discover the current central libtest names from the emitted executable."""
    entries: list[dict[str, Any]] = []
    for target in emitted:
        executable = target.get("executable")
        if not isinstance(executable, str) or not executable:
            raise ContractError("central compiler artifact lacks an executable path")
        try:
            result = subprocess.run(
                [executable, "--list"],
                capture_output=True,
                text=True,
                check=False,
                timeout=120,
            )
        except (OSError, subprocess.TimeoutExpired) as error:
            raise ContractError(f"could not list current integration executable {executable}: {error}") from error
        if result.returncode != 0:
            detail = result.stderr.strip() or result.stdout.strip() or "no diagnostic"
            raise ContractError(
                f"current integration executable --list failed with exit {result.returncode}: {detail}"
            )
        names = [
            line.strip()[: -len(": test")]
            for line in result.stdout.splitlines()
            if line.strip().endswith(": test")
        ]
        if not names:
            raise ContractError(f"current integration executable listed no tests: {executable}")
        entries.append(
            {
                "package": target.get("package"),
                "target": target.get("target"),
                "tests": names,
            }
        )
    return entries


def self_test() -> int:
    baseline = {
        "integration": {
            "cases": [
                {"id": "tests/a.rs::kept"},
                {"id": "tests/b.rs::also_kept"},
            ]
        }
    }
    good = {
        "cases": [
            {"baseline_case_id": "tests/a.rs::kept"},
            {"baseline_case_id": "tests/b.rs::also_kept"},
        ],
        "required_binaries": sorted(
            f"{package}/{name}" for package, name, _kind in REQUIRED_BINARIES
        ),
    }
    if inventory_manifest_errors(baseline, good):
        raise AssertionError("valid final inventory manifest was rejected")
    bad = {
        "cases": [
            {"baseline_case_id": "tests/a.rs::kept"},
            {"baseline_case_id": "tests/a.rs::kept"},
            {"baseline_case_id": "tests/new.rs::not_frozen"},
        ],
        "required_binaries": ["loop-cli/loop-engine"],
    }
    errors = inventory_manifest_errors(baseline, bad)
    joined = "\n".join(errors)
    for expected in (
        "missing baseline integration cases: tests/b.rs::also_kept",
        "unexpected integration cases: tests/new.rs::not_frozen",
        "final inventory contains duplicate baseline integration cases",
        "final inventory missing required binaries",
    ):
        if expected not in joined:
            raise AssertionError(f"self-test did not report {expected!r}")

    current_errors = case_inventory_errors(
        baseline,
        [{"id": "tests/a.rs::kept"}],
    )
    if "missing baseline integration cases: tests/b.rs::also_kept" not in current_errors:
        raise AssertionError("self-test did not reject a missing discovered case")

    print(
        "integration-test inventory assertion self-test passed: "
        "missing, extra, duplicate, and binary-preservation failures rejected"
    )
    return 0


def parse_args(argv: Sequence[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, default=Path.cwd())
    parser.add_argument("--baseline", type=Path, required=False)
    parser.add_argument("--compiler-artifacts", type=Path)
    parser.add_argument("--test-lists", type=Path)
    parser.add_argument(
        "--final-manifest",
        type=Path,
        help="T02 mapping manifest with cases[].baseline_case_id and required_binaries[]",
    )
    parser.add_argument(
        "--current-only",
        action="store_true",
        help="validate the current emitted central executable against its source declarations",
    )
    parser.add_argument("--self-test", action="store_true")
    return parser.parse_args(argv)


def run(args: argparse.Namespace) -> int:
    if args.compiler_artifacts is None:
        raise ContractError("--compiler-artifacts is required")
    if args.current_only and any(
        value is not None for value in (args.baseline, args.test_lists, args.final_manifest)
    ):
        raise ContractError("--current-only cannot be combined with baseline, test-list, or final-manifest inputs")
    if not args.current_only and args.baseline is None:
        raise ContractError("--baseline is required")
    if not args.current_only and args.final_manifest is None and args.test_lists is None:
        raise ContractError("one of --test-lists or --final-manifest is required")

    repo = args.repo.resolve()
    metadata = cargo_metadata(repo)
    targets = integration_targets(metadata, repo)
    binaries = binary_targets(metadata, repo)
    emitted = compiler_artifact_targets(args.compiler_artifacts.resolve(), repo)
    errors = topology_errors(targets, binaries, emitted)

    if args.current_only:
        listings = current_test_lists(emitted)
        _target_inventory, current_cases = source_test_inventory(metadata, repo, targets, listings)
        if errors:
            raise ContractError("\n".join(errors))
        print(
            "integration-test inventory ok: current central executable lists "
            f"{len(current_cases)} source-mapped cases and all declarations are discoverable"
        )
        return 0

    baseline = load_json(args.baseline.resolve())
    if not isinstance(baseline, dict):
        raise ContractError("baseline must be a JSON object")

    if args.final_manifest is not None:
        final_manifest = load_json(args.final_manifest.resolve())
        if not isinstance(final_manifest, dict):
            errors.append("final inventory manifest must be a JSON object")
        else:
            errors.extend(inventory_manifest_errors(baseline, final_manifest))
            if args.test_lists is not None:
                listings = load_test_lists(args.test_lists.resolve())
                _target_inventory, current_cases = source_test_inventory(
                    metadata, repo, targets, listings
                )
                errors.extend(case_inventory_errors(baseline, current_cases))
    else:
        listings = load_test_lists(args.test_lists.resolve())
        _target_inventory, current_cases = source_test_inventory(
            metadata, repo, targets, listings
        )
        errors.extend(case_inventory_errors(baseline, current_cases))
        errors.extend(source_hash_errors(baseline, source_files(metadata, repo)))

    if errors:
        raise ContractError("\n".join(errors))
    print(
        "integration-test inventory ok: baseline named cases retained, "
        "source inventory checked, and required binary names preserved"
    )
    return 0


def main(argv: Sequence[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    if args.self_test:
        if any(
            value is not None
            for value in (args.baseline, args.compiler_artifacts, args.test_lists, args.final_manifest)
        ) or args.repo != Path.cwd():
            print("--self-test cannot be combined with contract arguments", file=sys.stderr)
            return 2
        return self_test()
    try:
        return run(args)
    except ContractError as error:
        print(f"integration-test inventory contract failed:\n{error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
