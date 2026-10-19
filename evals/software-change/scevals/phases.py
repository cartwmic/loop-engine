"""Phase attribution for a Pi session record: tokens and time by skill or command marker, and by role.

`SC_PI_SESSION_DIR` keeps each Pi process's JSONL record (image/pi-role.py). Entries are read in file order.
A marker starts a phase that lasts until the next marker in the same record:
  - a `read` of a file named SKILL.md starts `skill:<directory name>`;
  - a user message whose text begins with `/name` starts `command:name`, or `skill:<name>` for `/skill:<name>`.
Work before any marker is `unmarked`. Each assistant message names its model, the role alias (for example
`role-driver`); work before any model is named is `unknown`.

Tokens are the usage objects in the record (assistant messages, tool results and usage entries); turns count
assistant messages. Time is the wall time between consecutive message entries, attributed to the phase and role
in effect at the earlier entry, so model latency and tool execution count toward the phase that issued them.

Attribution is unavailable, never zero, when the record is missing or unreadable, when it has no session header
or a line that is not a JSON object, or when a message lacks a timestamp, goes back in time, or lacks usage.
"""
import argparse
from dataclasses import asdict, dataclass
import json
from pathlib import Path
import re
import sys

UNMARKED = "unmarked"
UNKNOWN_ROLE = "unknown"
SKILL_FILE = re.compile(r"(?:^|/)([^/]+)/SKILL\.md$")
COMMAND = re.compile(r"/([A-Za-z0-9][\w.:-]*)(?=\s|$)")
USAGE_FIELDS = (("input_tokens", "input"), ("output_tokens", "output"), ("cache_read_tokens", "cacheRead"),
                ("cache_write_tokens", "cacheWrite"), ("total_tokens", "totalTokens"))


class UnparseableRecord(ValueError):
    pass


@dataclass
class PhaseTotals:
    turns: int = 0
    input_tokens: int = 0
    output_tokens: int = 0
    cache_read_tokens: int = 0
    cache_write_tokens: int = 0
    total_tokens: int = 0
    seconds: float = 0.0

    def add_usage(self, usage):
        if not isinstance(usage, dict) or not all(isinstance(usage.get(key), int) for _, key in USAGE_FIELDS):
            raise UnparseableRecord("usage is missing a token count")
        for field, key in USAGE_FIELDS:
            setattr(self, field, getattr(self, field) + usage[key])


@dataclass(frozen=True)
class PhaseAttribution:
    available: bool
    reason: str | None = None
    # Keyed by (phase, role). None when unavailable, so an unavailable record never reads as zero.
    phases: dict[tuple[str, str], PhaseTotals] | None = None

    def to_record(self) -> dict:
        if not self.available:
            return {"available": False, "reason": self.reason, "phases": None}
        rows = []
        for (phase, role), totals in sorted(self.phases.items()):
            row = asdict(totals)
            row["seconds"] = round(row["seconds"], 3)
            rows.append({"phase": phase, "role": role, **row})
        return {"available": True, "reason": None, "phases": rows}


@dataclass
class _Cursor:
    """Where the walk is: the marker phase and role in effect, and the entry that the next gap starts from."""
    phase: str = UNMARKED
    role: str = UNKNOWN_ROLE
    previous_key: tuple[str, str] | None = None
    previous_time: int | None = None

    def key(self):
        return (self.phase, self.role)


def attribute(path: Path) -> PhaseAttribution:
    """Attribute one session file, or every .jsonl session file in a directory, or say why that is impossible."""
    files = sorted(path.glob("*.jsonl")) if path.is_dir() else [path]
    if not files:
        return PhaseAttribution(False, "session record missing: no .jsonl files")
    totals: dict[tuple[str, str], PhaseTotals] = {}
    for file in files:
        try:
            lines = file.read_text(encoding="utf-8").splitlines()
        except FileNotFoundError:
            return PhaseAttribution(False, f"session record missing: {file.name}")
        except (OSError, UnicodeDecodeError):
            return PhaseAttribution(False, f"session record unreadable: {file.name}")
        try:
            _accumulate(lines, totals)
        except UnparseableRecord as error:
            return PhaseAttribution(False, f"unparseable session record {file.name}: {error}")
    return PhaseAttribution(True, phases=totals)


def _accumulate(lines, totals):
    cursor = _Cursor()
    header_seen = False
    for number, line in enumerate(lines, 1):
        if not line.strip():
            continue
        try:
            entry = json.loads(line)
        except json.JSONDecodeError as error:
            raise UnparseableRecord(f"line {number} is not JSON") from error
        if not isinstance(entry, dict):
            raise UnparseableRecord(f"line {number} is not an object")
        if not header_seen:
            if entry.get("type") != "session":
                raise UnparseableRecord("first line is not a session header")
            header_seen = True
            continue
        if entry.get("type") == "model_change" and isinstance(entry.get("modelId"), str):
            cursor.role = entry["modelId"]
        elif entry.get("type") == "usage":
            _target(totals, cursor.key()).add_usage(entry.get("usage"))
        elif entry.get("type") == "message":
            _message(entry, number, cursor, totals)
    if not header_seen:
        raise UnparseableRecord("no session header")


def _message(entry, number, cursor, totals):
    message = entry.get("message")
    timestamp = message.get("timestamp") if isinstance(message, dict) else None
    if not isinstance(message, dict) or not isinstance(timestamp, int) or isinstance(timestamp, bool):
        raise UnparseableRecord(f"line {number} has no message timestamp")
    if cursor.previous_time is not None:
        gap = timestamp - cursor.previous_time
        if gap < 0:
            raise UnparseableRecord(f"line {number} goes back in time")
        if gap:
            _target(totals, cursor.previous_key).seconds += gap / 1000
    role = message.get("role")
    if role == "assistant":
        if isinstance(message.get("model"), str) and message["model"]:
            cursor.role = message["model"]
        for block in _blocks(message.get("content")):
            arguments = block.get("arguments")
            if block.get("type") == "toolCall" and block.get("name") == "read" and isinstance(arguments, dict):
                path = arguments.get("path")
                skill = SKILL_FILE.search(path) if isinstance(path, str) else None
                if skill:
                    cursor.phase = f"skill:{skill.group(1)}"
    elif role == "user":
        command = COMMAND.match(_text(message.get("content")).lstrip())
        if command:
            name = command.group(1)
            cursor.phase = name if name.startswith("skill:") else f"command:{name}"
    key = cursor.key()
    if role == "assistant":
        target = _target(totals, key)
        target.turns += 1
        target.add_usage(message.get("usage"))
    elif "usage" in message:
        _target(totals, key).add_usage(message["usage"])
    cursor.previous_key = key
    cursor.previous_time = timestamp


def _target(totals, key):
    return totals.setdefault(key, PhaseTotals())


def _blocks(content):
    return [block for block in content if isinstance(block, dict)] if isinstance(content, list) else []


def _text(content):
    if isinstance(content, str):
        return content
    for block in _blocks(content):
        if block.get("type") == "text" and isinstance(block.get("text"), str):
            return block["text"]
    return ""


def main(argv=None):
    parser = argparse.ArgumentParser(description="Split a Pi session record's tokens and time by marker and role.")
    parser.add_argument("record", type=Path, help="a session .jsonl file, or a directory of them")
    arguments = parser.parse_args(argv)
    print(json.dumps(attribute(arguments.record).to_record(), indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
