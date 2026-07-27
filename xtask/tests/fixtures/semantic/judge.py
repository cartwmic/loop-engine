#!/usr/bin/env python3
import json
import os
import pathlib
import sys
import time

request = json.load(sys.stdin)
config = json.loads(pathlib.Path("behaviors.json").read_text())
axis = request["axis_id"]
kind = request["request_kind"]
behavior = config.get(axis, config.get("default", {}))

scratch = pathlib.Path(os.environ["SEMANTIC_SCRATCH"])
scratch.mkdir(parents=True, exist_ok=True)
(scratch / f"{kind}-start").write_text(str(time.monotonic()))
(scratch / f"{kind}-request.json").write_text(json.dumps(request, sort_keys=True))
if behavior.get("require_typed_env") and (
    os.environ.get("TYPED_BINDING") != request["candidate_tree"]
    or "REMOVE_ME" in os.environ
):
    raise SystemExit(9)
if behavior.get("require_candidate_cwd") and not pathlib.Path("protected.txt").is_file():
    raise SystemExit(10)

if behavior.get("sleep"):
    time.sleep(float(behavior["sleep"]))
if behavior.get("mutate") == kind:
    target = pathlib.Path("protected.txt")
    target.chmod(0o644)
    target.write_text("mutated\n")
if behavior.get("exit"):
    raise SystemExit(int(behavior["exit"]))
if behavior.get("invalid") == "always" or (
    behavior.get("invalid") == "first" and kind != "correction"
):
    print("not json")
    raise SystemExit(0)

status = behavior.get("status", "pass")
response = {
    "schema_version": 2,
    "request_kind": kind,
    "axis_id": axis,
    "base_revision": request["base_revision"],
    "candidate_revision": request["candidate_revision"],
    "candidate_tree": request["candidate_tree"],
    "status": status,
    "citations": [] if status == "unavailable" else [{
        "kind": "rubric",
        "reference": axis,
        "detail": "fixture rule",
    }],
    "message": behavior.get("message", f"{axis} {status}"),
}
for key, value in behavior.get("response_changes", {}).items():
    if value == "__DELETE__":
        response.pop(key, None)
    else:
        response[key] = value
if behavior.get("duplicate_status"):
    text = json.dumps(response, separators=(",", ":"))
    text = text.replace('"status":"' + status + '"', '"status":"pass","status":"' + status + '"')
    print(text)
else:
    print(json.dumps(response, separators=(",", ":")))
(scratch / f"{kind}-end").write_text(str(time.monotonic()))
