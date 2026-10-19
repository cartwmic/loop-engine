"""Stage ledger (G10): one JSON file per stage that admits each command against the remaining allowance.

Every command of the stage is admitted with its class cap, then recorded with its reported dollars and hours, outcome
and attempt. admit() launches nothing: the caller runs the command only after admit() returns. The stage cap is best
effort (spec section 7), so a command admitted within the allowance can still overshoot; record() and status() report
the overshoot instead of preventing it. Open entries reserve their handed cap until they are recorded.
"""
import json
import math
import os
from datetime import datetime, timezone
from pathlib import Path

SCHEMA_VERSION = 1

# Class caps from spec section 7: dollars (best effort) and hours from each class's hard_limit_seconds (4500, 13500, 23400).
CLASS_CAPS = {
    "T1": {"dollars": 12.0, "hours": 1.25},
    "T2": {"dollars": 45.0, "hours": 3.75},
    "T3": {"dollars": 120.0, "hours": 6.5},
}


class LedgerError(ValueError):
    """A refused or invalid ledger operation. Every refusal is raised before anything is written."""


def init_ledger(path: Path, stage_dollars: float, stage_hours: float) -> dict:
    if path.exists():
        raise LedgerError(f"ledger already exists: {path}")
    ledger = {
        "schema_version": SCHEMA_VERSION,
        "stage_cap": {"dollars": _positive(stage_dollars, "stage dollars"), "hours": _positive(stage_hours, "stage hours")},
        "entries": [],
    }
    _save(path, ledger)
    return _summary(ledger)


def admit(path: Path, command: str, command_class: str, reason: str | None = None, now: datetime | None = None) -> dict:
    ledger = _load(path)
    _text(command, "command")
    if command_class not in CLASS_CAPS:
        raise LedgerError(f"unknown command class {command_class!r}; expected one of {', '.join(CLASS_CAPS)}")
    prior = [entry for entry in ledger["entries"] if entry["command"] == command]
    if any(entry["end"] is None for entry in prior):
        raise LedgerError(f"command {command!r} has an open entry; record it before admitting it again")
    attempt = len(prior) + 1
    if attempt > 1:
        _text(reason, f"reason for rerun attempt {attempt} (owner-approved)")
    summary = _summary(ledger)
    if summary["unpriced_commands"]:
        raise LedgerError(
            f"refused: {summary['unpriced_commands']} recorded command(s) have no reported dollars, so remaining dollars are unknown")
    remaining = summary["remaining"]
    cap = CLASS_CAPS[command_class]
    if remaining["dollars"] < cap["dollars"] or remaining["hours"] < cap["hours"]:
        raise LedgerError(
            f"refused: remaining ${remaining['dollars']:.2f} and {remaining['hours']:.2f} h; "
            f"{command_class} needs ${cap['dollars']:.2f} and {cap['hours']:.2f} h")
    entry = {
        "id": len(ledger["entries"]) + 1,
        "command": command,
        "class": command_class,
        "attempt": attempt,
        "reason": reason,
        "start": _stamp(now),
        "end": None,
        "handed_cap": {name: min(cap[name], remaining[name]) for name in ("dollars", "hours")},
        "dollars": None,
        "hours": None,
        "outcome": None,
    }
    ledger["entries"].append(entry)
    _save(path, ledger)
    return entry


def record(path: Path, entry_id: int, outcome: str, hours: float, dollars: float | None = None,
           now: datetime | None = None) -> dict:
    ledger = _load(path)
    entry = _entry(ledger, entry_id)
    if entry["end"] is not None:
        raise LedgerError(f"entry {entry_id} is already recorded")
    _text(outcome, "outcome")
    hours = _number(hours, "hours")
    dollars = None if dollars is None else _number(dollars, "dollars")
    entry.update(end=_stamp(now), outcome=outcome, hours=hours, dollars=dollars)
    _save(path, ledger)
    return {"entry": entry, "status": _summary(ledger)}


def status(path: Path) -> dict:
    return _summary(_load(path))


def _summary(ledger: dict) -> dict:
    """Spent figures sum reported values only; unpriced commands make the dollar spend a lower bound."""
    cap = ledger["stage_cap"]
    spent_dollars = spent_hours = reserved_dollars = reserved_hours = 0.0
    unpriced = 0
    open_entries = []
    for entry in ledger["entries"]:
        if entry["end"] is None:
            open_entries.append(entry["id"])
            reserved_dollars += entry["handed_cap"]["dollars"]
            reserved_hours += entry["handed_cap"]["hours"]
            continue
        spent_hours += entry["hours"]
        if entry["dollars"] is None:
            unpriced += 1
        else:
            spent_dollars += entry["dollars"]
    return {
        "stage_cap": cap,
        "spent": {"dollars": spent_dollars, "hours": spent_hours},
        "reserved": {"dollars": reserved_dollars, "hours": reserved_hours},
        "remaining": {
            "dollars": None if unpriced else cap["dollars"] - spent_dollars - reserved_dollars,
            "hours": cap["hours"] - spent_hours - reserved_hours,
        },
        "unpriced_commands": unpriced,
        "open_entries": open_entries,
        "overshoot": {"dollars": max(0.0, spent_dollars - cap["dollars"]), "hours": max(0.0, spent_hours - cap["hours"])},
    }


def _entry(ledger: dict, entry_id: int) -> dict:
    for entry in ledger["entries"]:
        if entry["id"] == entry_id:
            return entry
    raise LedgerError(f"no ledger entry {entry_id}")


def _load(path: Path) -> dict:
    if not path.is_file():
        raise LedgerError(f"ledger does not exist: {path}")
    ledger = json.loads(path.read_text())
    if not isinstance(ledger, dict) or ledger.get("schema_version") != SCHEMA_VERSION:
        raise LedgerError(f"not a schema version {SCHEMA_VERSION} stage ledger: {path}")
    return ledger


def _save(path: Path, ledger: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(path.name + ".tmp")
    temporary.write_text(json.dumps(ledger, indent=2, allow_nan=False) + "\n")
    os.replace(temporary, path)


def _stamp(now: datetime | None) -> str:
    return (now or datetime.now(timezone.utc)).isoformat(timespec="seconds")


def _text(value, name: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise LedgerError(f"{name} must be a non-empty string")
    return value


def _number(value, name: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value) or value < 0:
        raise LedgerError(f"{name} must be a finite number >= 0")
    return float(value)


def _positive(value, name: str) -> float:
    number = _number(value, name)
    if number == 0:
        raise LedgerError(f"{name} must be greater than 0")
    return number
