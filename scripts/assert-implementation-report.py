#!/usr/bin/env python3
"""Check current-plan implementation proof against retained local command receipts.

The matrix is a read-only specification; proof/receipts.json indexes actual
execution. See docs/implementation-report-proof.md for the receipt contract.
This command never executes matrix commands or advances a workflow.
"""
from __future__ import annotations

import argparse
import json
import math
import re
import subprocess
import sys
from pathlib import Path
from typing import Any, Sequence

from test_contract import ContractError, repository_proof_identity

ROOT = Path(__file__).resolve().parents[1]


class ReportError(RuntimeError):
    """The report or its referenced execution evidence is not current proof."""


def load_report(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        raise ReportError(f"could not read {path}: {error}") from error
    if not isinstance(value, dict):
        raise ReportError(f"{path} must contain a JSON object")
    return value


def assert_equal(label: str, actual: Any, expected: Any) -> None:
    if actual != expected:
        raise ReportError(f"{label} mismatch: expected {expected!r}, got {actual!r}")


def git_output(args: Sequence[str]) -> str:
    return subprocess.check_output(["git", *args], cwd=ROOT, stderr=subprocess.PIPE, text=True)


def porcelain_changed_paths(status: str) -> list[str]:
    paths = []
    for line in status.splitlines():
        if len(line) < 4 or line[2] != " ":
            raise ReportError(f"unexpected git status --porcelain=v1 record: {line!r}")
        paths.append(line[3:])
    return paths


def repository_state() -> tuple[str, list[str]]:
    return git_output(["rev-parse", "HEAD"]).strip(), porcelain_changed_paths(
        git_output(["status", "--porcelain=v1", "--untracked-files=all"])
    )


def matrix_rows(matrix: dict[str, Any], section: str) -> list[dict[str, Any]]:
    rows = matrix.get(section)
    if not isinstance(rows, list) or not all(isinstance(row, dict) for row in rows):
        raise ReportError(f"matrix {section} must be an array of objects")
    ids = [row.get("id") for row in rows]
    if not all(isinstance(i, str) and re.fullmatch(r"[A-Za-z0-9_-]+", i) for i in ids):
        raise ReportError(f"matrix {section} has an invalid id")
    if len(ids) != len(set(ids)):
        raise ReportError(f"matrix {section} has duplicate ids")
    return rows


def strings(value: Any, label: str) -> list[str]:
    if not isinstance(value, list) or not all(isinstance(v, str) for v in value):
        raise ReportError(f"{label} must be a string array")
    return value


def resolved_argv(row: dict[str, Any], values: dict[str, str]) -> list[str]:
    command = row.get("command")
    if not isinstance(command, str) or not command:
        raise ReportError(f"{row['id']}: missing command")
    # Replace only declared placeholders, not arbitrary shell/Python braces.
    def expand(text: str) -> str:
        for key, value in values.items():
            text = text.replace("{" + key + "}", value)
        return text
    return [expand(s) for s in [command, *strings(row.get("args"), "matrix args")]]


def reference(base: Path, value: Any, label: str) -> Path:
    if not isinstance(value, str) or not value:
        raise ReportError(f"{label} must name a capture path")
    path = Path(value)
    return path if path.is_absolute() else base / path


def number(value: Any, label: str) -> float:
    if type(value) not in (int, float) or not math.isfinite(value) or value < 0:
        raise ReportError(f"{label} must be a finite nonnegative number")
    return value


def check_receipt(path: Path, row: dict[str, Any], argv: list[str], identity: str) -> tuple[str, float, float]:
    receipt = load_report(path)
    assert_equal("receipt id", receipt.get("id"), row["id"])
    assert_equal("receipt argv", receipt.get("argv"), argv)
    assert_equal("receipt cwd", receipt.get("cwd"), str(ROOT))
    for key in ("repository_before", "repository_after"):
        assert_equal(key, receipt.get(key), identity)
    start = number(receipt.get("started_at"), "started_at")
    finish = number(receipt.get("finished_at"), "finished_at")
    number(receipt.get("wall_seconds"), "wall_seconds")
    if finish < start:
        raise ReportError("receipt finished before it started")
    for key in ("timed_out",):
        if type(receipt.get(key)) is not bool:
            raise ReportError(f"receipt {key} must be boolean")
    if "spawn_error" not in receipt or not (receipt["spawn_error"] is None or isinstance(receipt["spawn_error"], str)):
        raise ReportError("receipt spawn_error must be null or a diagnostic")
    code = receipt.get("exit_code")
    if "exit_code" not in receipt or not (code is None or type(code) is int):
        raise ReportError("receipt exit_code must be an integer or null")
    streams = {}
    for key in ("stdout", "stderr"):
        try:
            streams[key] = reference(path.parent, receipt.get(key), key).read_text(encoding="utf-8", errors="replace")
        except OSError as error:
            raise ReportError(f"missing {key} capture: {error}") from error
    passed = code == 0 and not receipt["timed_out"] and receipt["spawn_error"] is None
    # Common capture preserves the actual child exit (including a cooperative
    # parent's exit 0 during abort). These execution facts must not become a
    # report pass. Legacy receipts without the additive fields retain their form.
    passed = passed and receipt.get("signal") is None and receipt.get("aborted", False) is False
    passed = passed and receipt.get("capture_error") is None and receipt.get("cleanup", "complete") == "complete"
    markers = strings(row.get("expected_stdout_contains", []), "expected_stdout_contains")
    passed = passed and all(marker in streams["stdout"] for marker in markers)
    return ("passed" if passed else "failed"), start, finish


def check_report(path: Path, *, matrix_path: Path, revision: str, plan_revision: str) -> dict[str, Any]:
    report = load_report(path)
    head, changed_paths = repository_state()
    identity = repository_proof_identity(ROOT)
    assert_equal("revision", report.get("revision"), revision)
    assert_equal("plan_revision", report.get("plan_revision"), plan_revision)
    coverage = report.get("coverage")
    if not isinstance(coverage, dict):
        raise ReportError("coverage must be an object")
    assert_equal("coverage.commit", coverage.get("commit"), f"{head}+uncommitted-worktree")
    assert_equal("changed_surface", report.get("changed_surface"), changed_paths)
    matrix_path = matrix_path.resolve()
    matrix = load_report(matrix_path)
    assert_equal("matrix schema_version", matrix.get("schema_version"), 1)
    assert_equal("matrix plan_revision", matrix.get("plan_revision"), plan_revision)
    local = matrix_rows(matrix, "local_final")
    post = matrix_rows(matrix, "post_report")
    later = matrix_rows(matrix, "after_separate_authorization")
    ids = [row["id"] for row in [*local, *post, *later]]
    if not local or len(ids) != len(set(ids)):
        raise ReportError("matrix must have local commands and globally unique ids")
    # Do not smuggle a preexisting result of this very check into local proof.
    for row in local:
        argv = resolved_argv(row, {})
        if any(Path(arg).name == Path(__file__).name for arg in argv) and "--self-test" not in argv:
            raise ReportError("the report check belongs in post_report, not local_final")
    expected_ids = [row["id"] for row in [*local, *later]]
    validation = report.get("validation")
    if not isinstance(validation, list) or not all(isinstance(row, dict) for row in validation):
        raise ReportError("validation rows must be objects with proof and optional criterion_id")
    claims = []
    for row in validation:
        item = row.get("proof")
        if set(row) - {"proof", "criterion_id"} or not isinstance(item, str) or not item:
            raise ReportError("validation row must contain a nonempty proof and only optional criterion_id")
        if "criterion_id" in row:
            criterion = row["criterion_id"]
            if not isinstance(criterion, str) or not re.fullmatch(r"AC-[1-9][0-9]*", criterion):
                raise ReportError("validation row has an invalid criterion_id")
            # Criterion notes are not execution receipts or matrix status claims.
            continue
        match = re.fullmatch(r"([A-Za-z0-9_-]+): (passed|failed|pending)", item)
        if not match:
            raise ReportError(f"invalid validation claim: {item!r}; use 'matrix-id: status'")
        claims.append(match.groups())
    assert_equal("validation ids/order", [i for i, _ in claims], expected_ids)
    claimed = dict(claims)
    for row in later:
        assert_equal(f"after-authorization {row['id']}", claimed[row["id"]], "pending")

    index_path = matrix_path.parent / "proof" / "receipts.json"
    index = load_report(index_path)
    assert_equal("receipt index plan_revision", index.get("plan_revision"), plan_revision)
    target = index.get("target_directory")
    if not isinstance(target, str) or not Path(target).is_absolute():
        raise ReportError("receipt index target_directory must be the resolved absolute Cargo target")
    entries = index.get("commands")
    if not isinstance(entries, list) or not all(isinstance(e, dict) for e in entries):
        raise ReportError("receipt index commands must be an array")
    entry_ids = [entry.get("id") for entry in entries]
    local_ids = [row["id"] for row in local]
    if any(not isinstance(i, str) or i not in local_ids for i in entry_ids) or len(entry_ids) != len(set(entry_ids)):
        raise ReportError("receipt index contains unknown, non-local or duplicate ids")
    references = {entry["id"]: reference(index_path.parent, entry.get("receipt"), "receipt") for entry in entries}
    values = {"artifact_root": str(matrix_path.parent), "checkout": str(ROOT),
              "target_directory": target, "implementation_revision": revision}
    outcomes = []
    errors = []
    previous_finish = None
    for row in local:
        command_id = row["id"]
        status = "pending"
        try:
            if command_id in references:
                status, start, finish = check_receipt(references[command_id], row, resolved_argv(row, values), identity)
                if previous_finish is not None and start < previous_finish:
                    raise ReportError("local matrix receipts are not serialized in matrix order")
                previous_finish = finish
            if claimed[command_id] != status:
                errors.append(f"{command_id}: claims {claimed[command_id]}, evidence is {status}")
            if status != "passed":
                errors.append(f"{command_id}: required local proof is {status}")
        except ReportError as error:
            status = "failed"
            errors.append(f"{command_id}: {error}")
        outcomes.append({"id": command_id, "status": status})
    if errors:
        raise ReportError("\n".join(errors))
    return {"revision": revision, "plan_revision": plan_revision,
            "commit": f"{head}+uncommitted-worktree", "local_final": outcomes,
            "after_separate_authorization": [{"id": row["id"], "status": "pending"} for row in later],
            "post_report": "externally evidenced; not a preexisting report claim"}


def parse_args(argv: Sequence[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--report", type=Path)
    parser.add_argument("--revision")
    parser.add_argument("--plan-revision")
    parser.add_argument("--matrix", type=Path)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--self-test-output", type=Path, help="retain scripted CLI proof fixtures/captures in a new directory")
    args = parser.parse_args(argv)
    report_args = (args.report, args.revision, args.plan_revision, args.matrix)
    if args.self_test:
        if any(arg is not None for arg in report_args):
            parser.error("--self-test cannot be combined with report arguments")
    elif any(arg is None for arg in report_args) or args.self_test_output is not None:
        parser.error("--report, --revision, --plan-revision and --matrix are required; --self-test-output needs --self-test")
    return args


def main(argv: Sequence[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    try:
        if args.self_test:
            from implementation_report_proof import self_test
            return self_test(args.self_test_output)
        result = check_report(args.report, matrix_path=args.matrix, revision=args.revision, plan_revision=args.plan_revision)
    except (ReportError, ContractError, OSError, subprocess.SubprocessError) as error:
        print(f"implementation-report assertion failed: {error}", file=sys.stderr)
        return 1
    print("implementation-report assertions passed: " + json.dumps(result, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
