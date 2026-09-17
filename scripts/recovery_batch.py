"""Public setup and staged-review proof for the software-change provider."""

from __future__ import annotations

import json
import subprocess
import tempfile
import time
from pathlib import Path


def _write_json(path: Path, value: object) -> None:
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


def _worker_body() -> str:
    return r'''#!/usr/bin/env python3
import json
import sys
from pathlib import Path

packet = sys.stdin.read()
def value(prefix, default):
    return next((line[len(prefix):] for line in packet.splitlines() if line.startswith(prefix)), default)
policies = json.loads(value("assigned_policies: ", "[]"))
stage = value("review_stage: ", "aggregate")
author = value("required_author_claim: ", "unknown")
with Path(sys.argv[1]).open("a", encoding="utf-8") as stream:
    stream.write(json.dumps({"stage": stage, "author": author, "stdin": packet}) + "\n")
print(json.dumps({
    "review_stage": stage,
    "author": {"name": author, "kind": "agent"},
    "judgments": [
        {"axis": policy["id"], "result": "pass", "findings": ""}
        for policy in policies
    ],
}))
'''


def _make_executable(path: Path, body: str) -> None:
    path.write_text(body, encoding="utf-8")
    path.chmod(0o755)


def _run(command: list[str], *, cwd: Path, expected: str | None = None) -> dict:
    completed = subprocess.run(command, cwd=cwd, capture_output=True, text=True, check=False)
    try:
        value = json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        raise AssertionError(
            f"non-JSON command output: {command}: {error}: {completed.stdout!r} {completed.stderr!r}"
        ) from error
    if expected is not None and value.get("status") != expected:
        raise AssertionError(f"command {command} returned {value}")
    return value


def _setup(
    journey,
    root: Path,
    rigor: str,
    roster: Path,
    output: Path,
    *extra: str,
) -> dict:
    command = [
        str(journey.provider),
        "setup",
        "--rigor",
        rigor,
        "--roster",
        str(roster),
        "--engine",
        str(journey.engine),
        "--provider",
        str(journey.provider),
        "--output",
        str(output),
        *extra,
    ]
    completed = subprocess.run(command, cwd=root, capture_output=True, text=True, check=False)
    if completed.returncode != 0:
        raise AssertionError(f"setup failed: {completed.stderr or completed.stdout}")
    report = json.loads(completed.stdout)
    assert report["status"] == "ready" and report["started"] is False
    assert report["output_bytes"] == output.read_text(encoding="utf-8")
    return report


def _workers(profile: dict, gate: str) -> list[dict]:
    args = profile["work_slot_bindings"][gate]["args"]
    result = []
    index = 0
    while index < len(args):
        if args[index] == "--worker":
            result.append(json.loads(args[index + 1]))
            index += 2
        elif args[index] == "--max-active":
            index += 2
        else:
            index += 1
    return result


def _expected_worker_count(policies: list[dict], roster_len: int) -> int:
    has_individual = any(
        policy.get("review_stage", "aggregate") == "individual" for policy in policies
    )
    count = 0
    if has_individual:
        count += sum(
            policy.get("required_authors", 1)
            for policy in policies
            if policy.get("review_stage", "aggregate") == "individual"
        )
    for author_index in range(roster_len):
        if any(
            policy.get("review_stage", "aggregate") == "aggregate"
            and policy.get("required_authors", 1) > author_index
            for policy in policies
        ):
            count += 1
    return count


def _profile_checks(profile: dict, report: dict, roster_len: int, *, bookends: bool) -> None:
    assert report["bookends_enabled"] is bookends
    assert report["preview"].get("errors", []) == []
    assert report["preview"]["models"] == []
    assert profile["contract_version"] == 3
    assert profile["work_slot_bindings"]
    for gate, binding in profile["work_slot_bindings"].items():
        if gate == "implement":
            continue
        assert binding["args"][:3] == ["fan-out", "--max-active", "2"]
        assert len(_workers(profile, gate)) == _expected_worker_count(
            report["effective_policy"]["review_policies"][gate], roster_len
        )
        assert binding["context_filter"]["args"] == ["commission"]
        for worker in _workers(profile, gate):
            assert worker["full_output_schema"]["required"] == [
                "review_stage",
                "author",
                "judgments",
            ]
            assert "FROZEN REVIEW ASSIGNMENT" in worker["preamble"]
    if bookends:
        assert profile["extra"]["bookends"]["enabled"] is True
        assert any(
            "ids-grounded" in worker["full_output_schema"]["properties"]["judgments"]["items"]["oneOf"][0]["properties"]["axis"]["enum"]
            for worker in _workers(profile, "intent-review")
        )


def prove(journey):
    from work_slot_journey import assert_projected_fan_out_capture

    journey.work_root.mkdir(parents=True, exist_ok=True)
    root = Path(tempfile.mkdtemp(prefix="recovery-batch-", dir=journey.work_root))
    worker_a = root / "worker-a.py"
    worker_b = root / "worker-b.py"
    log_a = root / "worker-a.jsonl"
    log_b = root / "worker-b.jsonl"
    _make_executable(worker_a, _worker_body())
    _make_executable(worker_b, _worker_body())
    roster = root / "roster.json"
    _write_json(
        roster,
        [
            {"author": "reviewer-a", "command": str(worker_a), "args": [str(log_a)]},
            {"author": "reviewer-b", "command": str(worker_b), "args": [str(log_b)]},
        ],
    )

    proof = {"status": "running", "setup": [], "runs": [], "cases": []}
    for rigor in ("minimal", "standard", "high"):
        output = root / f"{rigor}.json"
        report = _setup(journey, root, rigor, roster, output)
        profile = json.loads(output.read_text(encoding="utf-8"))
        _profile_checks(profile, report, 2, bookends=False)
        proof["setup"].append({"rigor": rigor, "output": str(output), "sha256": report["output_sha256"]})

    bookends_output = root / "high-bookends.json"
    report = _setup(journey, root, "high", roster, bookends_output, "--bookends")
    profile = json.loads(bookends_output.read_text(encoding="utf-8"))
    _profile_checks(profile, report, 2, bookends=True)

    duplicate = root / "duplicate.json"
    _write_json(duplicate, [
        {"author": "reviewer-a", "command": str(worker_a), "args": []},
        {"author": "reviewer-a", "command": str(worker_b), "args": []},
    ])
    refused = subprocess.run(
        [
            str(journey.provider), "setup", "--rigor", "standard", "--roster", str(duplicate),
            "--engine", str(journey.engine), "--provider", str(journey.provider),
            "--output", str(root / "duplicate-output.json"),
        ], cwd=root, capture_output=True, text=True, check=False,
    )
    assert refused.returncode != 0 and not (root / "duplicate-output.json").exists()
    proof["cases"].append("all rigor levels, Bookends opt-in, exact bytes, concurrency and invalid roster")

    config = root / "providers.toml"
    config.write_text(
        "[providers.software-change]\n"
        f"command = {json.dumps(str(journey.provider))}\nargs = []\n",
        encoding="utf-8",
    )
    profile_path = root / "high-execution.json"
    _setup(journey, root, "high", roster, profile_path)
    profile = json.loads(profile_path.read_text(encoding="utf-8"))
    database = root / "loop.sqlite"
    run_id = "recovery-batch-high"
    started = _run(
        [
            str(journey.engine), "--database", str(database), "--json", "--config", str(config),
            "start", "--id", run_id, "software-change", f"@{profile_path}", "setup proof",
        ], cwd=journey.data_root, expected="completed",
    )
    artifact_root = Path(started["result"]["run"]["initial_input"]["artifact_root"])
    fixture = journey.data_root / "crates/software-change-provider/data/calibration/fixtures/intent-good.json"
    artifact_root.mkdir(parents=True, exist_ok=True)
    (artifact_root / "intent.json").write_bytes(fixture.read_bytes())
    _run([str(journey.engine), "--database", str(database), "--json", "show", "--view", "full", run_id], cwd=journey.data_root, expected="completed")
    _run([str(journey.engine), "--database", str(database), "--json", "event", run_id, "intent-ready"], cwd=journey.data_root, expected="completed")
    _run([str(journey.engine), "--database", str(database), "--json", "show", "--view", "full", run_id], cwd=journey.data_root, expected="completed")
    invoked = _run([str(journey.engine), "--database", str(database), "--json", "invoke", run_id, "intent-review"], cwd=journey.data_root, expected="completed")
    invocation_id = invoked["result"]["invocation_id"]
    expected_worker_total = len(_workers(profile, "intent-review"))
    shown = None
    for _ in range(600):
        shown = _run([str(journey.engine), "--database", str(database), "--json", "show", "--view", "full", run_id], cwd=journey.data_root, expected="completed")
        invocation = next(row for row in shown["result"]["work_slot_invocations"] if row["invocation_id"] == invocation_id)
        if (
            invocation["status"] in ("succeeded", "failed")
            and len(invocation.get("inner_workers", [])) == expected_worker_total
        ):
            break
        time.sleep(0.05)
    else:
        raise AssertionError("setup execution did not complete with its worker summary")
    assert invocation["status"] == "succeeded", invocation
    assert len(invocation["inner_workers"]) == expected_worker_total
    assert_projected_fan_out_capture(invocation)
    launches = []
    for log in (log_a, log_b):
        if log.exists():
            launches.extend(json.loads(line) for line in log.read_text(encoding="utf-8").splitlines())
    observed = {(row["stage"], row["author"]) for row in launches}
    assert observed == {
        ("individual", "reviewer-a"), ("individual", "reviewer-b"),
        ("aggregate", "reviewer-a"), ("aggregate", "reviewer-b"),
    }
    assert len(launches) == len(_workers(profile, "intent-review"))
    proof["runs"].append({"id": run_id, "database": str(database), "invocation_id": invocation_id})
    proof["cases"].append("fresh individual and aggregate stages executed through captured arbitrary commands")
    proof["status"] = "passed"
    _write_json(root / "proof.json", proof)
    print("recovery batched-review scenario passed: setup profiles, exact captures, staged authors and invalid input", flush=True)