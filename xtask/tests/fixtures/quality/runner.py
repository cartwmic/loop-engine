#!/usr/bin/env python3
import json
import os
import pathlib
import sys

mode = sys.argv[1]
if mode == "emit":
    print(sys.argv[2])
elif mode == "record":
    roots = [os.environ[name] for name in ("SCRATCH", "CACHE", "TARGET")]
    for index, root in enumerate(roots):
        path = pathlib.Path(root)
        path.mkdir(parents=True, exist_ok=True)
        (path / f"probe-{index}").write_text("writable\n", encoding="utf-8")
    print(json.dumps({
        "args": sys.argv[2:],
        "cwd": os.getcwd(),
        "default": os.environ.get("DEFAULT_VALUE"),
        "override": os.environ.get("OVERRIDE_VALUE"),
        "removed": os.environ.get("REMOVE_ME"),
        "roots": roots,
    }, sort_keys=True))
elif mode == "fail":
    print(sys.argv[2], file=sys.stderr)
    sys.exit(int(sys.argv[3]))
elif mode == "install":
    pathlib.Path(sys.argv[2]).write_text("install hint executed\n", encoding="utf-8")
elif mode == "mutate-content":
    path = pathlib.Path(sys.argv[2])
    path.chmod(0o644)
    path.write_text("mutated\n", encoding="utf-8")
elif mode == "mutate-mode":
    pathlib.Path(sys.argv[2]).chmod(0o755)
else:
    raise SystemExit(f"unknown mode: {mode}")
