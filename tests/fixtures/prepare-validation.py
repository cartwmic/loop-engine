#!/usr/bin/env python3
"""Synthetic v2 fixture migration: real command capture, frozen index, fake judgments.

Only tests call this helper. It never appends, advances, or claims semantic review.
The caller supplies a show snapshot and appends returned genuine record candidates.
"""
import argparse
import json
from pathlib import Path
import subprocess
import sys


def prepare(provider, engine, cwd, show, revision):
    result = subprocess.run([str(provider), "run-validation", "--engine", str(engine),
        "--working-directory", str(cwd), "--revision", revision], input=json.dumps(show),
        text=True, capture_output=True, check=False)
    if result.returncode:
        raise RuntimeError(f"fixture run-validation failed: {result.stdout}\n{result.stderr}")
    execution = json.loads(result.stdout)
    assert execution["commands_passed"], execution
    root = Path(show["result"]["initial_input"]["artifact_root"])
    report = json.loads((root / "validation-report.json").read_text())
    checkpoint = subprocess.run([str(provider), "checkpoint", "--phase", "validation",
        "--artifact-root", str(root), "--working-directory", str(cwd)],
        text=True, capture_output=True, check=False)
    if checkpoint.returncode:
        raise RuntimeError(f"fixture checkpoint failed: {checkpoint.stderr}")
    records = [{k: row[k] for k in ("record_id", "kind", "data")}
        for row in execution["command_candidates"]]
    groups = [(row["criterion_id"], row["verdict_ids"]) for row in report["criteria"]]
    groups.append((None, report["goal_verdict_ids"]))
    for criterion, names in groups:
        for number, name in enumerate(names):
            data = {"subject": "validation-report.json", "subject_revision": revision,
                "checkpoint": "validation-checkpoint.json",
                "author": {"name": f"synthetic-independent-criterion-{number}", "kind": "script"},
                "result": "pass", "findings": [],
                "evidence_context_ids": report["command_evidence_ids"]}
            if criterion:
                data["criterion_id"] = criterion
            records.append({"record_id": name,
                "kind": "criterion-verdict" if criterion else "goal-verdict", "data": data})
    return {"report": report, "records": records, "execution": execution,
            "checkpoint": json.loads(checkpoint.stdout)}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for option in ("provider", "engine", "working-directory", "revision"):
        parser.add_argument("--" + option, required=True)
    args = parser.parse_args()
    print(json.dumps(prepare(args.provider, args.engine, args.working_directory,
        json.load(sys.stdin), args.revision)))
