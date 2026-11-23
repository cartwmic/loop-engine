#!/usr/bin/env python3
"""Model-to-code sync check for the formal model in formal/.

formal/COVERS lists the source files the model covers, one per line:

    <path from the repo root> <git blob hash at the last confirmation>

The model was last confirmed against those versions. This check recomputes each
file's hash with `git hash-object` and fails when one differs or is gone. It needs
no git history, so a shallow clone works.

    formal/check-sync.py           exit 0: in sync; 1: a covered file changed (named); 2: could not run
    formal/check-sync.py --update  rewrite the hashes to the current files; exit 0

Run --update only after re-reading formal/LoopRun.tla against the change. Standard
library only.
"""

import argparse
import pathlib
import subprocess
import sys

FORMAL = pathlib.Path(__file__).resolve().parent
ROOT = FORMAL.parent
COVERS = FORMAL / "COVERS"


def could_not_run(message):
    print(f"formal/check-sync.py: {message}", file=sys.stderr)
    sys.exit(2)


def read_covers():
    """Return every line of COVERS, and (line index, path, recorded hash) per entry."""
    try:
        lines = COVERS.read_text().splitlines()
    except OSError as error:
        could_not_run(f"cannot read {COVERS}: {error.strerror}")
    entries = []
    for index, line in enumerate(lines):
        text = line.strip()
        if not text or text.startswith("#"):
            continue
        fields = text.split()
        if len(fields) != 2:
            could_not_run(f"formal/COVERS line {index + 1} must be '<path> <blob-hash>': {text!r}")
        entries.append((index, fields[0], fields[1]))
    if not entries:
        could_not_run("formal/COVERS lists no files")
    return lines, entries


def current_hash(path):
    """The git blob hash of the working-tree file, or None when it is not there."""
    if not (ROOT / path).is_file():
        return None
    try:
        result = subprocess.run(
            ["git", "hash-object", "--", path], cwd=ROOT, capture_output=True, text=True
        )
    except OSError as error:
        could_not_run(f"cannot run git to hash {path}: {error.strerror}")
    if result.returncode != 0:
        could_not_run(f"git hash-object failed for {path}: {result.stderr.strip()}")
    return result.stdout.strip()


def check(entries):
    """Return (path, reason) for each covered file that no longer matches."""
    changed = []
    for _, path, recorded in entries:
        found = current_hash(path)
        if found is None:
            changed.append((path, "missing"))
        elif found != recorded:
            changed.append((path, "changed"))
    return changed


def update(lines, entries):
    new_lines = list(lines)
    updated = []
    for index, path, recorded in entries:
        found = current_hash(path)
        if found is None:
            could_not_run(f"formal/COVERS names {path}, which is not a file; fix COVERS by hand")
        if found != recorded:
            new_lines[index] = f"{path} {found}"
            updated.append(path)
    if not updated:
        print("formal/check-sync.py: every hash in formal/COVERS is already current")
        return 0
    COVERS.write_text("\n".join(new_lines) + "\n")
    print("formal/check-sync.py: updated formal/COVERS for:")
    for path in updated:
        print(f"  {path}")
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument(
        "--update",
        action="store_true",
        help="rewrite the hashes in formal/COVERS to the current files (only after re-reading the model)",
    )
    args = parser.parse_args()
    lines, entries = read_covers()
    if args.update:
        return update(lines, entries)
    changed = check(entries)
    if not changed:
        print(f"formal/check-sync.py: all {len(entries)} covered files match the model's last confirmation")
        return 0
    print("formal/check-sync.py: a file the model covers changed since the model was last confirmed:", file=sys.stderr)
    for path, reason in changed:
        print(f"  {path} ({reason})", file=sys.stderr)
    print(
        "Re-read formal/LoopRun.tla against the change and update it from the code (not from a check result). "
        "Rerun formal/check.sh, then run formal/check-sync.py --update in the same PR.",
        file=sys.stderr,
    )
    return 1


if __name__ == "__main__":
    sys.exit(main())
