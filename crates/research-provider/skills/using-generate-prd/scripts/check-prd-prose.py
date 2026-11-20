#!/usr/bin/env python3
"""Check that a converted PRD keeps the accepted prose word for word.

Usage: check-prd-prose.py ACCEPTED.md CONVERTED.md

Every non-blank line of the accepted file must appear, exactly, as a whole line
of the converted file, in the same order. The converted file may have any other
lines anywhere, even between two lines of one accepted list: requirement ids,
status lines, an index.

Only line endings, trailing spaces and a leading UTF-8 byte order mark are
ignored. Anything else, even a reflowed line, is a change, because a paraphrase
can hide in a reflow. A repeated accepted line needs its own copy in the
converted file.

A kept line must also stay visible. Each line counts as plain text, as inside an
HTML comment, or as inside a fenced code block, and an accepted line only
matches a converted line of the same kind. Prose the accepted file shows as
plain text but the converted file wraps in a comment or a fence is reported as
hidden. Prose that is a comment or a fence in the accepted file may stay one.

Exit 0: every accepted line is kept. Exit 1: the report lists each line that
was dropped, changed (with the closest converted line), moved out of order or
hidden. When one line fits several labels the order is moved, hidden, changed,
dropped. Exit 2: a file could not be read. The script reports what differs; it
does not judge what should be a requirement.
"""
from __future__ import annotations

import difflib
import re
import sys
from collections import Counter
from pathlib import Path

# Closest-line hint only: below this a missing line is reported as dropped.
CLOSE = 0.5

FENCE = re.compile(r"\s*(`{3,}|~{3,})")


def comment_open_after(line: str, inside: bool) -> bool:
    """Whether an HTML comment is still open at the end of the line, given whether one was open at its start."""
    position = 0
    while True:
        marker = "-->" if inside else "<!--"
        found = line.find(marker, position)
        if found == -1:
            return inside
        inside = not inside
        position = found + len(marker)


def numbered_lines(text: str) -> list[tuple[int, str, str]]:
    """(line number, kind, text) for each non-blank line, ignoring line endings and trailing spaces.

    kind is "plain", "comment" (inside, opening or closing an HTML comment) or "fence" (inside a fenced code block).
    """
    lines = text.replace("\r\n", "\n").replace("\r", "\n").split("\n")
    found = []
    state = "plain"
    marker = ""
    for number, raw in enumerate(lines, 1):
        line = raw.rstrip()
        kind = state
        if state == "fence":
            closing = FENCE.fullmatch(line)
            if closing and closing.group(1)[0] == marker[0] and len(closing.group(1)) >= len(marker):
                state = "plain"
        else:
            opening = FENCE.match(line) if state == "plain" else None
            if opening:
                state, marker = "fence", opening.group(1)
            elif state == "comment" or "<!--" in line:
                kind = "comment"
                state = "comment" if comment_open_after(line, state == "comment") else "plain"
        if line.strip():
            found.append((number, kind, line))
    return found


def shorten(text: str) -> str:
    return text if len(text) <= 100 else text[:97] + "..."


def closest_lines(lines: list[str], offered: list[tuple[int, str]]) -> dict[int, tuple[int, str]]:
    """For each index of lines, its closest offered line, if any. A converted line is named for one accepted line only."""
    pairs = []
    for index, line in enumerate(lines):
        for number, text in offered:
            ratio = difflib.SequenceMatcher(None, line, text).ratio()
            if ratio >= CLOSE:
                pairs.append((ratio, index, number, text))
    named: dict[int, tuple[int, str]] = {}
    taken: set[int] = set()
    for _ratio, index, number, text in sorted(pairs, key=lambda pair: (-pair[0], pair[2], pair[1])):
        if index not in named and number not in taken:
            named[index] = (number, text)
            taken.add(number)
    return named


def check(accepted: str, converted: str) -> tuple[int, list[str]]:
    wanted = numbered_lines(accepted)
    offered = numbered_lines(converted)
    keys = [(kind, text) for _, kind, text in offered]
    copies = Counter(keys)
    visible_as = {text: {kind for _, kind, t in offered if t == text} for _, _, text in wanted}
    matched: Counter[tuple[str, str]] = Counter()
    used: set[int] = set()
    cursor = 0
    # One entry per accepted line that was not matched in order: position, number, text, kind of loss.
    lost: list[tuple[int, int, str, str]] = []
    for position, (number, kind, line) in enumerate(wanted):
        key = (kind, line)
        try:
            at = keys.index(key, cursor)
        except ValueError:
            at = -1
        if at != -1:
            cursor = at + 1
            matched[key] += 1
            used.add(at)
        elif copies[key] > matched[key]:
            lost.append((position, number, line, "moved"))
        elif kind == "plain" and visible_as[line] - {kind}:
            lost.append((position, number, line, "hidden"))
        elif visible_as[line] - {kind}:
            lost.append((position, number, line, "other block"))
        else:
            lost.append((position, number, line, "missing"))
    # A converted line may be named as the closest one only if no accepted line already claims it as written.
    accepted_texts = {text for _, _, text in wanted}
    pool = [(number, text) for index, (number, _, text) in enumerate(offered) if index not in used and text not in accepted_texts]
    # One entry per run of neighbouring lost lines of the same kind: kind, numbers, lines, hint.
    runs: list[tuple[str, list[int], list[str], str]] = []
    last_position = -2
    near_by_index = closest_lines([line for _, _, line, kind in lost if kind == "missing"], pool)
    missing_seen = 0
    for position, number, line, kind in lost:
        hint = ""
        if kind == "other block":
            kind, hint = "changed", "\n         the line is in the converted file, but in a different block (HTML comment or code fence) than in the accepted file"
        elif kind == "missing":
            near = near_by_index.get(missing_seen)
            missing_seen += 1
            if near is None:
                kind = "dropped"
            else:
                kind, hint = "changed", f"\n         closest converted line {near[0]}: {shorten(near[1])}"
        if kind != "changed" and runs and runs[-1][0] == kind and position == last_position + 1:
            runs[-1][1].append(number)
            runs[-1][2].append(line)
        else:
            runs.append((kind, [number], [line], hint))
        last_position = position
    report = []
    for kind, numbers, lines, hint in runs:
        where = f"line {numbers[0]}" if len(numbers) == 1 else f"lines {numbers[0]}-{numbers[-1]}"
        text = shorten(" / ".join(lines))
        if kind == "moved":
            report.append(f"moved: accepted {where} is in the converted file, but out of order: {text}")
        elif kind == "hidden":
            report.append(f"hidden: accepted {where} is in the converted file, but only inside an HTML comment or a code fence: {text}")
        else:
            report.append(f"{kind}: accepted {where}: {text}{hint}")
    return len(wanted), report


def main(argv: list[str]) -> int:
    if len(argv) != 3:
        print("usage: check-prd-prose.py ACCEPTED.md CONVERTED.md", file=sys.stderr)
        return 2
    try:
        accepted = Path(argv[1]).read_text(encoding="utf-8-sig")
        converted = Path(argv[2]).read_text(encoding="utf-8-sig")
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
