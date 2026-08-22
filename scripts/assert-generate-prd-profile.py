#!/usr/bin/env python3
"""Assert Generate-PRD is data for the existing research provider."""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any, NoReturn

ROOT = Path(__file__).resolve().parents[1]
GENERATE = ROOT / "crates/research-provider/data/configs/generate-prd.json"
STANDARD = ROOT / "crates/research-provider/data/configs/standard.json"
SKILL = ROOT / "crates/research-provider/skills/using-generate-prd/SKILL.md"
TEMPLATES = ROOT / "crates/research-provider/data/templates/generate-prd"


def fail(message: str) -> NoReturn:
    raise SystemExit(f"generate-prd profile assertion failed: {message}")


def load(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        fail(f"could not read {path}: {error}")
    if not isinstance(value, dict):
        fail(f"{path} must contain a JSON object")
    return value


def policy_axis_ids(value: Any) -> dict[str, list[str]]:
    policies = value.get("review_policies") if isinstance(value, dict) else None
    if not isinstance(policies, dict):
        fail("review_policies must be an object")
    result: dict[str, list[str]] = {}
    for gate, entries in policies.items():
        if not isinstance(entries, list):
            fail(f"review_policies.{gate} must be an array")
        result[gate] = [
            entry["id"]
            for entry in entries
            if isinstance(entry, dict) and isinstance(entry.get("id"), str)
        ]
        if len(result[gate]) != len(entries):
            fail(f"review_policies.{gate} has a malformed axis")
    return result


def binary() -> Path:
    configured = os.environ.get("RESEARCH_PROVIDER")
    candidates = [Path(configured).expanduser()] if configured else []
    candidates.extend((ROOT / "target/debug/research", ROOT / "target/release/research"))
    for candidate in candidates:
        if candidate.is_file() and os.access(candidate, os.X_OK):
            return candidate
    cargo = shutil.which("cargo")
    if cargo is None:
        fail("target/debug/research is missing and cargo is unavailable")
    built = subprocess.run(
        [cargo, "build", "-p", "research-provider"],
        cwd=ROOT,
        text=True,
        capture_output=True,
        check=False,
    )
    if built.returncode != 0:
        fail(f"could not build research-provider: {built.stderr}")
    result = ROOT / "target/debug/research"
    if not result.is_file():
        fail("research-provider build did not produce target/debug/research")
    return result


def invoke(provider: Path, request: dict[str, Any]) -> dict[str, Any]:
    result = subprocess.run(
        [str(provider)],
        cwd=ROOT,
        input=json.dumps(request),
        text=True,
        capture_output=True,
        check=False,
    )
    if result.returncode != 0:
        fail(f"research rejected request: {result.stderr}")
    try:
        value = json.loads(result.stdout)
    except json.JSONDecodeError as error:
        fail(f"research returned non-JSON output: {error}")
    if not isinstance(value, dict):
        fail("research response must be an object")
    return value


def main() -> int:
    generate = load(GENERATE)
    standard = load(STANDARD)
    for key in ("config_version", "artifact_schemas", "revision_links"):
        if generate.get(key) != standard.get(key):
            fail(f"generate-prd must preserve standard {key}")
    if policy_axis_ids(generate) != policy_axis_ids(standard):
        fail("generate-prd must preserve the standard research policy axes")
    if generate.get("extra") != {
        "profile": "generate-prd",
        "template_root": "crates/research-provider/data/templates/generate-prd",
    }:
        fail("generate-prd extra must identify only its template tree")
    if "work_slot_bindings" in generate or "artifact_root" in generate:
        fail("shipped generate-prd profile must omit per-run fields")
    for gate, template, subject in (
        ("verify", "verification.md", "verification.json"),
        ("synthesize", "report.md", "report.json"),
    ):
        for entry in generate["review_policies"][gate]:
            prompt = entry.get("example_prompt", "")
            if f"data/templates/generate-prd/{template}" not in prompt:
                fail(f"{gate} prompt does not use the Generate-PRD template")
            if f"data/configs/generate-prd.json#/artifact_schemas/{subject}" not in prompt:
                fail(f"{gate} prompt does not use the Generate-PRD schema")

    provider_crates = {
        path.parent.name for path in (ROOT / "crates").glob("*-provider/Cargo.toml")
    }
    if provider_crates != {
        "policy-document-provider",
        "research-provider",
        "software-change-provider",
    }:
        fail(f"unexpected provider crate set: {sorted(provider_crates)}")

    for name in ("brief.md", "sources.md", "verification.md", "report.md"):
        path = TEMPLATES / name
        if not path.is_file():
            fail(f"missing generate-prd template {path}")
    skill = SKILL.read_text(encoding="utf-8")
    for phrase in (
        "A human must accept or reject the candidate before any commit to docs/PRD.md.",
        "Never auto-commit.",
        "Never mint IDs outside the published grammar.",
        "Never call software-change evaluate.",
        "Never invoke a model from the research provider binary itself.",
        "prd-candidate.md",
        "bookends-check candidate",
    ):
        if phrase not in skill:
            fail(f"skill is missing {phrase!r}")

    described = invoke(provider := binary(), {"operation": "describe", "initial_input": generate})
    states = [item.get("id") for item in described.get("states", [])]
    if described.get("id") != "research" or states != ["scope", "gather", "verify", "synthesize", "end"]:
        fail(f"generate-prd does not use the research topology: {described}")
    with tempfile.TemporaryDirectory(prefix="generate-prd-profile-") as directory:
        artifacts = Path(directory)
        (artifacts / "brief.json").write_text(
            json.dumps(
                {
                    "revision": "1",
                    "author": {"name": "assertion", "kind": "script"},
                    "question": "Extract a candidate.",
                    "scope": "This repository.",
                    "acceptance": ["Each proposal has repository evidence."],
                    "constraints": ["Human acceptance precedes commit."],
                    "non_goals": ["Automatic commit."],
                }
            ),
            encoding="utf-8",
        )
        initial = dict(generate)
        initial["artifact_root"] = str(artifacts)
        evaluated = invoke(
            provider,
            {
                "operation": "evaluate",
                "workflow": {"id": "research", "initial_state": "scope", "states": [], "transitions": []},
                "initial_input": initial,
                "context": [],
                "transition": {"source": "scope", "event": "scoped", "target": "gather", "kind": "checked"},
                "prior_evaluations": [],
            },
        )
        if evaluated.get("result") != "allow":
            fail(f"existing research binary did not evaluate generate-prd scope: {evaluated}")

    print(
        "generate-prd profile assertion passed: research topology, existing research binary, "
        "human accept/reject, no auto-commit, published ID grammar, no fourth provider"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
