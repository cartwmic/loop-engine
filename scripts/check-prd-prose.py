#!/usr/bin/env python3
"""Check that a converted PRD keeps the accepted prose word for word.

Usage: check-prd-prose.py ACCEPTED.md CONVERTED.md

Every non-blank line of the accepted file must appear, exactly, as a whole line
of the converted file, in the same order. The converted file may have any other
lines anywhere, even between two lines of one accepted list: requirement ids,
status lines, an index.

Only line endings and trailing spaces are ignored. Anything else, even a
reflowed line, is a change, because a paraphrase can hide in a reflow.
A repeated accepted line needs its own copy in the converted file.

Exit 0: every accepted line is kept. Exit 1: the report lists each line that
was dropped, changed (with the closest converted line) or moved out of order.
Exit 2: a file could not be read. The script reports what differs; it does not
judge what should be a requirement.
"""
from __future__ import annotations

import difflib
import sys
from collections import Counter
from pathlib import Path

# Closest-line hint only: below this a missing line is reported as dropped.
CLOSE = 0.5


def numbered_lines(text: str) -> list[tuple[int, str]]:
    """(line number, text) for each non-blank line, ignoring line endings and trailing spaces."""
    lines = text.replace("\r\n", "\n").replace("\r", "\n").split("\n")
    return [(number, line.rstrip()) for number, line in enumerate(lines, 1) if line.strip()]


def shorten(text: str) -> str:
    return text if len(text) <= 100 else text[:97] + "..."


def closest(line: str, offered: list[tuple[int, str]]) -> tuple[int, str] | None:
    best, best_ratio = None, CLOSE
    for number, text in offered:
        ratio = difflib.SequenceMatcher(None, line, text).ratio()
        if ratio >= best_ratio:
            best, best_ratio = (number, text), ratio
    return best


def check(accepted: str, converted: str) -> tuple[int, list[str]]:
    wanted = numbered_lines(accepted)
    offered = numbered_lines(converted)
    texts = [text for _, text in offered]
    copies = Counter(texts)
    matched: Counter[str] = Counter()
    cursor = 0
    # One entry per run of neighbouring lost lines of the same kind: kind, numbers, lines, hint.
    runs: list[tuple[str, list[int], list[str], str]] = []
    last_index = -2
    for index, (number, line) in enumerate(wanted):
        try:
            at = texts.index(line, cursor)
        except ValueError:
            at = -1
        if at != -1:
            cursor = at + 1
            matched[line] += 1
            continue
        near = closest(line, offered)
        if copies[line] > matched[line]:
            kind, hint = "moved", ""
        elif copies[line] == 0 and near is not None:
            kind, hint = "changed", f"\n         closest converted line {near[0]}: {shorten(near[1])}"
        else:
            kind, hint = "dropped", ""
        if kind != "changed" and runs and runs[-1][0] == kind and last_index == index - 1:
            runs[-1][1].append(number)
            runs[-1][2].append(line)
        else:
            runs.append((kind, [number], [line], hint))
        last_index = index
    report = []
    for kind, numbers, lines, hint in runs:
        where = f"line {numbers[0]}" if len(numbers) == 1 else f"lines {numbers[0]}-{numbers[-1]}"
        text = shorten(" / ".join(lines))
        if kind == "moved":
            report.append(f"moved: accepted {where} is in the converted file, but out of order: {text}")
        else:
            report.append(f"{kind}: accepted {where}: {text}{hint}")
    return len(wanted), report


def main(argv: list[str]) -> int:
    if len(argv) != 3:
        print("usage: check-prd-prose.py ACCEPTED.md CONVERTED.md", file=sys.stderr)
        return 2
    try:
        accepted = Path(argv[1]).read_text(encoding="utf-8")
        converted = Path(argv[2]).read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError) as error:
        print(f"check-prd-prose: cannot read input: {error}", file=sys.stderr)
        return 2
    total, report = check(accepted, converted)
    for entry in report:
        print(entry)
    if report:
        print(f"check-prd-prose: {len(report)} problem(s) in {total} accepted lines; the prose is not kept verbatim")
        return 1
    print(f"check-prd-prose: all {total} accepted lines kept verbatim and in order")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
