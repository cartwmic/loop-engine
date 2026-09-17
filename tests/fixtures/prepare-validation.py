#!/usr/bin/env python3
"""Prepare the validation fixture from one real common-capture matrix.

Only tests call this helper.  It executes the commissioned proof commands
through loop-engine's existing capture-matrix path, then asks the provider's
inert prepare-validation command to index those fresh receipts.  It never
appends, advances, or claims semantic review.
"""
import argparse
import json
from pathlib import Path
import subprocess
import sys


CAPTURE_TIMEOUT_MS = 1_200_000


def _run_json(command, *, input_value=None, cwd=None, label="command"):
    result = subprocess.run(
        [str(value) for value in command],
        input=None if input_value is None else json.dumps(input_value),
        text=True,
        cwd=cwd,
        capture_output=True,
        check=False,
    )
    if result.returncode:
        raise RuntimeError(f"{label} failed: {result.stdout}\n{result.stderr}")
    try:
        return json.loads(result.stdout)
    except json.JSONDecodeError as error:
        raise RuntimeError(f"{label} returned non-JSON: {result.stdout!r}") from error


def _proof_commands(provider, show):
    commission = _run_json(
        [provider, "commission", "--slot", "validation-draft"],
        input_value=show,
        label="validation commission",
    )
    try:
        commands = commission["commission"]["proof_commands"]
    except (KeyError, TypeError) as error:
        raise RuntimeError(f"validation commission omitted proof_commands: {commission}") from error
    if not isinstance(commands, list) or not commands:
        raise RuntimeError(f"validation commission returned no proof commands: {commission}")
    return commands


def _capture_matrix(engine, root, cwd, commands, revision):
    matrix = {
        "rows": [
            {
                "id": command["id"],
                "argv": [command["command"], *command["args"]],
                "environment": {},
                "inherit_environment": [],
                "timeout_ms": CAPTURE_TIMEOUT_MS,
                "obligations": [command["obligation"]],
            }
            for command in commands
        ]
    }
    matrix_path = root / f"validation-capture-matrix-{revision}.json"
    capture_root = root / f"validation-capture-matrix-{revision}"
    matrix_path.write_text(json.dumps(matrix, indent=2) + "\n")
    # The report is intentionally invalidated before fresh admission.  A
    # failed matrix therefore cannot leave an older passing report current.
    (root / "validation-report.json").unlink(missing_ok=True)
    result = subprocess.run(
        [
            str(engine),
            "capture-matrix",
            "--matrix",
            str(matrix_path),
            "--working-directory",
            str(cwd),
            "--output-dir",
            str(capture_root),
        ],
        cwd=cwd,
        text=True,
        capture_output=True,
        check=False,
    )
    (root / "validation-capture-matrix.stdout").write_text(result.stdout)
    (root / "validation-capture-matrix.stderr").write_text(result.stderr)
    if result.returncode:
        raise RuntimeError(
            f"capture-matrix failed ({result.returncode}): {result.stdout}\n{result.stderr}"
        )
    index = capture_root / "index.json"
    if not index.is_file():
        raise RuntimeError(f"capture-matrix omitted its index: {index}")
    return matrix, capture_root, index


def prepare(provider, engine, cwd, show, revision):
    root = Path(show["result"]["initial_input"]["artifact_root"])
    cwd = Path(cwd).resolve()
    commands = _proof_commands(provider, show)
    matrix, capture_root, capture_index = _capture_matrix(engine, root, cwd, commands, revision)
    settings = {
        "environment": {},
        "inherit_environment": [],
        "timeout_ms": CAPTURE_TIMEOUT_MS,
    }
    packet = {
        "show": show,
        "working_directory": str(cwd),
        "revision": revision,
        "author": {"name": "software-change journey", "kind": "script"},
        "capture_indexes": [str(capture_index)],
        "execution_settings": settings,
        "additions": [],
    }
    prepared = _run_json(
        [provider, "prepare-validation"],
        input_value=packet,
        label="prepare-validation",
    )
    if prepared.get("status") != "draft-only" or not prepared.get("commands_complete"):
        raise RuntimeError(f"prepare-validation did not produce a complete draft: {prepared}")
    report = prepared.get("report_draft")
    records = prepared.get("command_candidates")
    if not isinstance(report, dict) or not isinstance(records, list):
        raise RuntimeError(f"prepare-validation omitted report or command candidates: {prepared}")
    command_ids = {command["id"] for command in commands}
    record_ids = {row.get("data", {}).get("proof_id") for row in records}
    if len(records) != len(commands) or record_ids != command_ids:
        raise RuntimeError(
            f"prepare-validation indexed proof IDs {sorted(record_ids)}, "
            f"expected {sorted(command_ids)}"
        )
    (root / "validation-report.json").write_text(json.dumps(report, indent=2) + "\n")
    checkpoint = _run_json(
        [
            provider,
            "checkpoint",
            "--phase",
            "validation",
            "--artifact-root",
            str(root),
            "--working-directory",
            str(cwd),
        ],
        label="validation checkpoint",
    )
    # The checkpoint precedes the same deterministic independent-record
    # fixtures as the former helper.  These records exercise provider schema,
    # author-floor and checkpoint binding; they are not semantic judgments.
    complete_records = [
        {key: row[key] for key in ("record_id", "kind", "data")}
        for row in records
    ]
    groups = [(row["criterion_id"], row["verdict_ids"]) for row in report["criteria"]]
    groups.append((None, report["goal_verdict_ids"]))
    for criterion, names in groups:
        for number, name in enumerate(names):
            data = {
                "subject": "validation-report.json",
                "subject_revision": revision,
                "checkpoint": "validation-checkpoint.json",
                "author": {"name": f"synthetic-independent-criterion-{number}", "kind": "script"},
                "result": "pass",
                "findings": [],
                "evidence_context_ids": report["command_evidence_ids"],
            }
            if criterion:
                data["criterion_id"] = criterion
            complete_records.append({
                "record_id": name,
                "kind": "criterion-verdict" if criterion else "goal-verdict",
                "data": data,
            })
    return {
        "report": report,
        "records": complete_records,
        "execution": prepared,
        "matrix": matrix,
        "capture_root": str(capture_root),
        "capture_index": str(capture_index),
        "checkpoint": checkpoint,
    }


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for option in ("provider", "engine", "working-directory", "revision"):
        parser.add_argument("--" + option, required=True)
    args = parser.parse_args()
    print(json.dumps(prepare(args.provider, args.engine, args.working_directory,
        json.load(sys.stdin), args.revision)))
