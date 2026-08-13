#!/usr/bin/env python3
"""Separate-process public-boundary journey for policy-document provider."""
from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import subprocess
import tempfile
from pathlib import Path
from typing import Any


def call(engine: Path, database: Path, arguments: list[str]) -> dict[str, Any]:
    """Invoke one fresh CLI process and parse its public JSON response."""
    completed = subprocess.run(
        [str(engine), "--database", str(database), "--json", *arguments],
        text=True,
        capture_output=True,
        check=False,
    )
    if not completed.stdout.strip():
        raise RuntimeError(f"CLI returned no JSON: {completed.stderr}")
    value = json.loads(completed.stdout)
    if value.get("status") not in (None, "completed", "rejected"):
        raise RuntimeError(value)
    return value


def expect_denial(response: dict[str, Any], code: str, phase: str) -> dict[str, Any]:
    assert response["status"] == "rejected", response
    assert response["code"] == code, response
    assert response["details"]["phase"] == phase, response
    return response["details"]


def show_state(engine: Path, database: Path, run_id: str, state: str) -> dict[str, Any]:
    shown = call(engine, database, ["show", run_id])
    assert shown["status"] == "completed", shown
    assert shown["result"]["current_state"] == state, shown
    return shown["result"]


def evidence(axis: str, digest: str, author: str) -> dict[str, Any]:
    return {
        "gate": "semantic-review",
        "policy_id": axis,
        "result": "pass",
        "findings": "",
        "author": {"name": author, "kind": "script"},
        "target_id": "README.md",
        "target_sha256": digest,
        "profile_version": "readme-1",
    }


def append_evidence(
    engine: Path,
    database: Path,
    run_id: str,
    axes: list[str],
    digest: str,
    prefix: str,
) -> None:
    for index, axis in enumerate(axes):
        response = call(
            engine,
            database,
            [
                "append",
                "--record-id",
                f"{prefix}-{index}",
                run_id,
                "review-evidence",
                json.dumps(evidence(axis, digest, prefix), separators=(",", ":")),
            ],
        )
        assert response["status"] == "completed", response
        assert response["result"]["context"]["id"] == f"{prefix}-{index}", response


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--engine", required=True)
    parser.add_argument("--provider", required=True)
    parser.add_argument("--profile", required=True)
    parser.add_argument("--mode", choices=("draft", "audit"), default="draft")
    args = parser.parse_args()
    engine = Path(args.engine).resolve()
    provider = Path(args.provider).resolve()
    shipped_profile = Path(args.profile).resolve()
    if not shipped_profile.is_file():
        parser.error(f"profile is not a file: {shipped_profile}")

    with tempfile.TemporaryDirectory(prefix="policy-document-journey-") as temporary:
        work = Path(temporary)
        target = work / "README.md"
        target.write_text("", encoding="utf-8")
        database = work / "run.sqlite"
        providers = work / "providers.toml"
        providers.write_text(
            f'[providers.policy-document]\ncommand = {json.dumps(str(provider))}\nargs = []\n',
            encoding="utf-8",
        )

        profile_path = work / "readme.json"
        shutil.copy2(shipped_profile, profile_path)
        profile = json.loads(profile_path.read_text(encoding="utf-8"))
        profile["mode"] = args.mode
        profile["target"]["path"] = str(target)
        profile_path.write_text(json.dumps(profile, indent=2) + "\n", encoding="utf-8")
        axes = [item["id"] for item in profile["semantic_policies"]]
        run_id = f"policy-document-{args.mode}-journey"

        started = call(
            engine,
            database,
            [
                "--config",
                str(providers),
                "start",
                "--id",
                run_id,
                "policy-document",
                "@" + str(profile_path),
                f"README {args.mode} journey",
            ],
        )
        assert started["status"] == "completed", started
        shown = show_state(engine, database, run_id, "prepare")
        assert shown["initial_input"]["mode"] == args.mode, shown

        ready = call(engine, database, ["event", run_id, "ready"])
        assert ready["status"] == "completed", ready
        show_state(engine, database, run_id, "deterministic-review")

        deterministic = expect_denial(
            call(engine, database, ["event", run_id, "passed"]),
            "policy-document-nonconforming",
            "deterministic",
        )
        assert [item["policy_id"] for item in deterministic["violations"]] == [
            "document-present",
            "project-title",
            "purpose",
            "onboarding",
            "usage",
            "validation",
            "onboarding-command",
            "validation-command",
        ]
        show_state(engine, database, run_id, "deterministic-review")

        conforming = (
            "# Product\n\n## Purpose\nUseful tool.\n\n## Installation\n"
            "```sh\ncargo build\n```\n\n## Usage\nRun it.\n\n## Validation\n"
            "```sh\ncargo test\n```\n"
        )
        target.write_text(conforming, encoding="utf-8")
        moved = call(engine, database, ["event", run_id, "passed"])
        assert moved["status"] == "completed", moved
        show_state(engine, database, run_id, "semantic-review")

        missing = expect_denial(
            call(engine, database, ["event", run_id, "passed"]),
            "policy-document-review-incomplete",
            "semantic",
        )
        diagnostics = missing["details"]["diagnostics"]
        assert [(item["policy_id"], item["kind"]) for item in diagnostics] == [
            (axis, "missing") for axis in axes
        ]
        initial_digest = hashlib.sha256(target.read_bytes()).hexdigest()
        assert missing["target_sha256"] == initial_digest
        append_evidence(engine, database, run_id, axes, initial_digest, "initial-review")

        # Finalization must rerun deterministic policy before consulting evidence.
        broken = conforming.replace("## Validation", "Validation")
        target.write_text(broken, encoding="utf-8")
        final_deterministic = expect_denial(
            call(engine, database, ["event", run_id, "passed"]),
            "policy-document-nonconforming",
            "deterministic",
        )
        assert [item["policy_id"] for item in final_deterministic["violations"]] == [
            "validation",
            "validation-command",
        ]

        repaired = conforming + "\nRepair revision.\n"
        target.write_text(repaired, encoding="utf-8")
        stale = expect_denial(
            call(engine, database, ["event", run_id, "passed"]),
            "policy-document-review-incomplete",
            "semantic",
        )
        stale_diagnostics = stale["details"]["diagnostics"]
        assert [item["kind"] for item in stale_diagnostics].count("stale") == len(axes)
        assert [item["kind"] for item in stale_diagnostics].count("missing") == len(axes)
        fresh_digest = hashlib.sha256(target.read_bytes()).hexdigest()
        assert fresh_digest != initial_digest
        assert stale["target_sha256"] == fresh_digest
        append_evidence(engine, database, run_id, axes, fresh_digest, "fresh-review")

        final = call(engine, database, ["event", run_id, "passed"])
        assert final["status"] == "completed", final
        terminal = show_state(engine, database, run_id, "end")
        assert terminal["lifecycle"] == "final", terminal
        assert len(terminal["context"]) == len(axes) * 2, terminal
        print(
            json.dumps(
                {
                    "journey": "policy-document",
                    "result": "passed",
                    "mode": args.mode,
                    "profile": "copied shipped readme.json",
                    "proof": [
                        "deterministic all-findings denial",
                        "missing-evidence denial",
                        "checked deterministic progression",
                        "final deterministic recheck denial",
                        "stale-evidence denial",
                        "fresh-evidence success",
                        "fresh-process show persistence",
                        "terminal completion",
                    ],
                },
                sort_keys=True,
            )
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
