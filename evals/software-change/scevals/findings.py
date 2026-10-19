"""Finding ledger format (G10): one closed-schema record per review finding, validated before it is counted.

A record states how the finding was verified (reproduced or not by a test or the grader), whether it is material, when
it was discovered, whether it was already known, its disposition, and its false-positive and duplicate flags, so review
value can be counted without double-counting. This module validates the format only: it runs no review and no
verification.
"""
import json
from collections import Counter
from datetime import datetime
from pathlib import Path

FINDING_FIELDS = frozenset({
    "finding_id", "source", "summary", "verification", "material", "discovered_at",
    "already_known", "disposition", "false_positive", "duplicate_of",
})
VERIFICATION_FIELDS = frozenset({"status", "method", "evidence"})
VERIFICATION_STATUSES = frozenset({"reproduced", "not_reproduced", "unverified"})
VERIFICATION_METHODS = frozenset({"test", "grader"})
DISPOSITIONS = frozenset({"fixed", "accepted", "rejected", "deferred", "open"})


class FindingError(ValueError):
    """An invalid finding record or finding ledger."""


def validate_finding(record) -> dict:
    if not isinstance(record, dict):
        raise FindingError("finding must be a JSON object")
    _closed(record, FINDING_FIELDS, "finding")
    _text(record["finding_id"], "finding_id")
    _text(record["source"], "source")
    _text(record["summary"], "summary")
    _verification(record["verification"])
    for name in ("material", "already_known", "false_positive"):
        _flag(record[name], name)
    _timestamp(record["discovered_at"])
    _choice(record["disposition"], DISPOSITIONS, "disposition")
    if record["duplicate_of"] is not None:
        _text(record["duplicate_of"], "duplicate_of")
        if record["duplicate_of"] == record["finding_id"]:
            raise FindingError("finding cannot be a duplicate of itself")
    return record


def check_findings(records) -> list[dict]:
    """Validate every record, then the ledger as a whole: finding_id is unique and each duplicate_of names a finding in it."""
    if not isinstance(records, list):
        raise FindingError("finding ledger must be a JSON array of finding records")
    for index, record in enumerate(records, 1):
        try:
            validate_finding(record)
        except FindingError as error:
            raise FindingError(f"record {index}: {error}") from error
    counts = Counter(record["finding_id"] for record in records)
    repeated = sorted(identifier for identifier, count in counts.items() if count > 1)
    if repeated:
        raise FindingError(f"duplicate finding_id: {', '.join(repeated)}")
    for record in records:
        target = record["duplicate_of"]
        if target is not None and target not in counts:
            raise FindingError(f"{record['finding_id']} is a duplicate of unknown finding {target}")
    return records


def check_findings_file(path: Path) -> list[dict]:
    try:
        records = json.loads(path.read_text())
    except json.JSONDecodeError as error:
        raise FindingError(f"{path} is not valid JSON: {error}") from error
    return check_findings(records)


def _verification(value) -> None:
    if not isinstance(value, dict):
        raise FindingError("verification must be a JSON object")
    _closed(value, VERIFICATION_FIELDS, "verification")
    _choice(value["status"], VERIFICATION_STATUSES, "verification status")
    if value["status"] == "unverified":
        if value["method"] is not None:
            raise FindingError("an unverified finding must have verification method null")
    else:
        _choice(value["method"], VERIFICATION_METHODS, "verification method")
    _text(value["evidence"], "verification evidence")


def _closed(record: dict, fields: frozenset, label: str) -> None:
    missing = sorted(fields - set(record))
    if missing:
        raise FindingError(f"{label} is missing field(s): {', '.join(missing)}")
    unknown = sorted(set(record) - fields)
    if unknown:
        raise FindingError(f"{label} has unknown field(s): {', '.join(unknown)}")


def _choice(value, allowed: frozenset, name: str) -> None:
    if not isinstance(value, str) or value not in allowed:
        raise FindingError(f"{name} must be one of {', '.join(sorted(allowed))}")


def _text(value, name: str) -> None:
    if not isinstance(value, str) or not value.strip():
        raise FindingError(f"{name} must be a non-empty string")


def _flag(value, name: str) -> None:
    if not isinstance(value, bool):
        raise FindingError(f"{name} must be true or false")


def _timestamp(value) -> None:
    try:
        parsed = datetime.fromisoformat(value) if isinstance(value, str) else None
    except ValueError:
        parsed = None
    if parsed is None or parsed.tzinfo is None:
        raise FindingError("discovered_at must be an ISO 8601 timestamp with a timezone")
