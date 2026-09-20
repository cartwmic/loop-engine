#!/usr/bin/env python3
"""Verify that integrated authoritative documents match a committed revision.

This checker is deliberately read-only. It compares each supplied working-tree
path with ``git show HEAD:<path>`` (or another supplied revision) and compares
the exact accepted PRD record text in the approved intent snapshot with the
committed ``docs/PRD.md`` text. It never stages, commits, or edits files.
"""
from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path
from typing import Any, Sequence


class IntegrationError(RuntimeError):
    """The repository or approved requirement snapshot is not integrated."""


def read_json(path: Path) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise IntegrationError(f"could not read intent snapshot {path}: {error}") from error


def approved_records(path: Path, required_ids: Sequence[str]) -> dict[str, bytes]:
    snapshot = read_json(path)
    acceptance = snapshot.get("acceptance") if isinstance(snapshot, dict) else None
    if not isinstance(acceptance, list):
        raise IntegrationError("intent snapshot acceptance must be an array")

    found: dict[str, bytes] = {}
    for entry in acceptance:
        if not isinstance(entry, dict):
            continue
        traceability = entry.get("prd_traceability")
        if not isinstance(traceability, dict):
            continue
        proposed_id = traceability.get("proposed_id")
        record = traceability.get("record_markdown")
        if not isinstance(proposed_id, str) or not isinstance(record, str) or not record:
            continue
        if proposed_id not in required_ids:
            continue
        encoded = record.encode("utf-8")
        previous = found.get(proposed_id)
        if previous is not None and previous != encoded:
            raise IntegrationError(
                f"approved intent contains conflicting record text for {proposed_id}"
            )
        found[proposed_id] = encoded

    missing = [record_id for record_id in required_ids if record_id not in found]
    if missing:
        raise IntegrationError(
            "approved intent snapshot lacks exact record text for: " + ", ".join(missing)
        )
    return found


def repository_path(repository: Path, value: str) -> Path:
    path = Path(value)
    if path.is_absolute():
        raise IntegrationError(f"authoritative path must be repository-relative: {value}")
    resolved = (repository / path).resolve()
    try:
        resolved.relative_to(repository.resolve())
    except ValueError as error:
        raise IntegrationError(f"authoritative path escapes repository: {value}") from error
    return resolved


def committed_bytes(repository: Path, revision: str, path: str) -> bytes:
    result = subprocess.run(
        ["git", "show", f"{revision}:{path}"],
        cwd=repository,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if result.returncode:
        diagnostic = result.stderr.decode("utf-8", "replace").strip()
        raise IntegrationError(f"cannot read {revision}:{path}: {diagnostic}")
    return result.stdout


def parse_args(argv: Sequence[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repository", type=Path, required=True)
    parser.add_argument("--head", required=True, help="committed revision to inspect")
    parser.add_argument("--intent-snapshot", type=Path, required=True)
    parser.add_argument("--path", action="append", required=True, dest="paths")
    parser.add_argument("--require-prd-record", action="append", required=True, dest="record_ids")
    args = parser.parse_args(argv)
    if not args.paths:
        parser.error("at least one --path is required")
    if not args.record_ids:
        parser.error("at least one --require-prd-record is required")
    if len(args.paths) != len(set(args.paths)):
        parser.error("--path values must be unique")
    if len(args.record_ids) != len(set(args.record_ids)):
        parser.error("--require-prd-record values must be unique")
    return args


def verify(args: argparse.Namespace) -> None:
    repository = args.repository.resolve()
    if not repository.is_dir():
        raise IntegrationError(f"repository is not a directory: {repository}")

    records = approved_records(args.intent_snapshot.resolve(), args.record_ids)
    current_by_path: dict[str, bytes] = {}
    for value in args.paths:
        path = repository_path(repository, value)
        try:
            current = path.read_bytes()
        except OSError as error:
            raise IntegrationError(f"could not read current authoritative path {value}: {error}") from error
        committed = committed_bytes(repository, args.head, value)
        if current != committed:
            raise IntegrationError(
                f"{value} does not match {args.head}:{value}; commit the inspected authoritative bytes first"
            )
        current_by_path[value] = current

    prd = current_by_path.get("docs/PRD.md")
    if prd is None:
        raise IntegrationError("--path docs/PRD.md is required for PRD record verification")
    for record_id, record in records.items():
        heading = f"### {record_id}:".encode("utf-8")
        if prd.count(heading) != 1:
            raise IntegrationError(f"docs/PRD.md must contain exactly one {record_id} heading")
        if prd.count(record) != 1:
            raise IntegrationError(
                f"docs/PRD.md does not contain the exact approved {record_id} record once"
            )

    print(
        "committed integration verified: "
        + ", ".join(args.paths)
        + "; PRD records "
        + ", ".join(args.record_ids)
    )


def main(argv: Sequence[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    try:
        verify(args)
    except IntegrationError as error:
        print(f"committed integration check failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
