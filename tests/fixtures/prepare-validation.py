#!/usr/bin/env python3
"""External fixture driver: execute proof commands and submit attributed results.

Synthetic judgments exercise schema/checkpoint mechanics, not semantic review.
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


def _proof_commands(provider, show, *, cwd):
    commission = _run_json(
        [provider, "commission", "--slot", "validation-draft"],
        input_value=show,
        cwd=cwd,
        label="validation commission",
    )
    try:
        commands = commission["commission"]["proof_commands"]
    except (KeyError, TypeError) as error:
        raise RuntimeError(f"validation commission omitted proof_commands: {commission}") from error
    if not isinstance(commands, list) or not commands:
        raise RuntimeError(f"validation commission returned no proof commands: {commission}")
    return commands


def _command_results(provider, cwd, commands):
    results = []
    for command in commands:
        source = _run_json([provider, "source-identity", "--working-directory", cwd], cwd=cwd)
        completed = subprocess.run(
            [command["command"], *command["args"]], cwd=cwd,
            capture_output=True, text=True, check=False, timeout=CAPTURE_TIMEOUT_MS / 1000,
        )
        results.append({
            "id": command["id"], "command": command["command"], "args": command["args"],
            "cwd": str(cwd), "exit_code": completed.returncode,
            "stdout": completed.stdout, "stderr": completed.stderr,
            "producer": {"name": "validation-fixture-driver", "kind": "script"},
            "source": source,
        })
    return results


def prepare(provider, engine, cwd, show, revision):
    root = Path(show["result"]["initial_input"]["artifact_root"])
    cwd = Path(cwd).resolve()
    # Provider admission resolves undeclared primary scope from process cwd,
    # not the packet working_directory. Keep every provider call aligned with
    # the actual proof/checkpoint repository; never broaden the source scope.
    commands = _proof_commands(provider, show, cwd=cwd)
    (root / "validation-report.json").unlink(missing_ok=True)
    results = _command_results(provider, cwd, commands)
    packet = {
        "show": show,
        "working_directory": str(cwd),
        "revision": revision,
        "author": {"name": "software-change journey", "kind": "script"},
        "command_results": results,
        "additions": [],
    }
    prepared = _run_json(
        [provider, "prepare-validation"],
        input_value=packet,
        cwd=cwd,
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
        cwd=cwd,
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
        "command_results": results,
        "checkpoint": checkpoint,
    }


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for option in ("provider", "engine", "working-directory", "revision"):
        parser.add_argument("--" + option, required=True)
    args = parser.parse_args()
    print(json.dumps(prepare(args.provider, args.engine, args.working_directory,
        json.load(sys.stdin), args.revision)))
