#!/usr/bin/env python3
"""NEW ordinary line reader: bounded 2000-line/50KiB windows, streaming bytes.
No engine/provider access. The generated fixture checkpoint is supplied by the public test.
"""
import argparse
import hashlib
import json
from pathlib import Path


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('path', type=Path)
    parser.add_argument('--max-line-bytes', type=int, required=True)
    args = parser.parse_args()
    if not 0 < args.max_line_bytes <= 50 * 1024:
        raise ValueError('Line must fit ordinary 50KiB read window')
    digest = hashlib.sha256()
    windows, lines, total, longest, window_bytes, window_lines = 0, 0, 0, 0, 0, 0
    with args.path.open('rb') as source:
        while True:
            line = source.readline(args.max_line_bytes + 1)
            if not line:
                break
            if len(line) > args.max_line_bytes:
                raise ValueError('Oversized line; ordinary file reader refused it')
            line.decode('utf-8')
            if window_bytes + len(line) > 50 * 1024 or window_lines == 2000:
                windows += 1
                window_bytes, window_lines = 0, 0
            window_bytes += len(line); window_lines += 1
            lines += 1; total += len(line); longest = max(longest, len(line))
            digest.update(line)
    if window_lines:
        windows += 1
    if lines <= 1 or total == 0:
        raise ValueError('No readable multi-line checkpoint')
    print(json.dumps({'path': str(args.path.resolve()), 'bytes': total, 'lines': lines,
        'windows': windows, 'max_line_bytes': longest, 'sha256': digest.hexdigest(),
        'window_limits': {'lines': 2000, 'bytes': 50 * 1024}}))

if __name__ == '__main__':
    main()
