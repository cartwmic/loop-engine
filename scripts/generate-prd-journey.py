#!/usr/bin/env python3
"""Black-box source journey for the research-provider Generate-PRD path."""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any, NoReturn, Optional, Sequence

import work_slot_journey

ROOT = Path(__file__).resolve().parents[1]
SLOTS = ("scope", "gather", "verify", "synthesize")
STATES = ("scope", "gather", "verify", "synthesize", "end")
EVENTS = {
    "scope": "scoped",
    "gather": "gathered",
    "verify": "verified",
    "synthesize": "completed",
}
NEXT_STATE = {
    "scope": "gather",
    "gather": "verify",
    "verify": "synthesize",
    "synthesize": "end",
}
RUN_ID = "generate-prd-journey"
CANDIDATE_NAME = "prd-candidate.md"
EVIDENCE_NAME = "prd-candidate.evidence.json"
ID_HEADING = re.compile(r"^### (LE-[1-9][0-9]*): (\S.*)$")


class JourneyFailure(RuntimeError):
    """A deterministic journey preflight or assertion failed."""


def fail(message: str) -> NoReturn:
    raise JourneyFailure(message)


def cli_call(engine: Path, database: Path, arguments: Sequence[str]) -> dict[str, Any]:
    completed = subprocess.run(
        [str(engine), "--database", str(database), "--json", *arguments],
        cwd=ROOT,
        text=True,
        capture_output=True,
        check=False,
    )
    if not completed.stdout.strip():
        fail(f"loop-engine returned no JSON: {completed.stderr.strip()}")
    try:
        value = json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        fail(f"loop-engine returned malformed JSON: {error}: {completed.stdout}")
    if not isinstance(value, dict):
        fail(f"loop-engine response is not an object: {value}")
    return value


def show_state(engine: Path, database: Path, state: str) -> dict[str, Any]:
    response = cli_call(engine, database, ["show", RUN_ID])
    if response.get("status") != "completed":
        fail(f"show failed: {response}")
    result = response.get("result")
    if not isinstance(result, dict) or result.get("current_state") != state:
        fail(f"expected state {state}, got {response}")
    return result


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


def exact_line(repository_root: Path, locator: str, needle: str) -> str:
    try:
        lines = (repository_root / locator).read_text(encoding="utf-8").splitlines()
    except OSError as error:
        fail(f"could not read evidence source {locator}: {error}")
    for line in lines:
        if needle in line:
            return line.strip()
    fail(f"evidence needle not found in {locator}: {needle}")


def evidence_records(repository_root: Path) -> list[dict[str, str]]:
    """Return deterministic repository evidence for the proposed records."""
    specifications = (
        (
            "LE-9001",
            "Durable workflow behavior is recorded as a reviewable requirement",
            "docs/PRD.md",
            "### LE-1: A caller cannot directly set current state",
        ),
        (
            "LE-9002",
            "The research public boundary is exercised by a separate journey",
            "scripts/research-journey.py",
            '"""Separate-process public-boundary journey for the research provider."""',
        ),
        (
            "LE-9003",
            "The research workflow has a fixed deterministic topology",
            "crates/research-provider/src/workflow.rs",
            "pub(crate) fn research_workflow() -> Workflow {",
        ),
    )
    return [
        {
            "id": proposal_id,
            "title": title,
            "locator": locator,
            "extract": exact_line(repository_root, locator, needle),
        }
        for proposal_id, title, locator, needle in specifications
    ]


def candidate_text(records: Sequence[dict[str, str]]) -> str:
    lines = [
        "# Proposed PRD candidate",
        "",
        "> Candidate IDs are provisional proposals. A human must accept this file before any commit to the living PRD.",
        "",
    ]
    for record in records:
        lines.extend(
            [
                f"### {record['id']}: {record['title']}",
                "- Status: live",
                "- Coverage: e2e/journey",
                "",
                "This proposal is backed by the repository evidence below; it is not yet PRD authority.",
                "",
                "#### Repository evidence",
                f"- Locator: `{record['locator']}`",
                f"- Extract: `{record['extract']}`",
                "",
            ]
        )
    return "\n".join(lines)


def brief_artifact() -> dict[str, Any]:
    return {
        "revision": "1",
        "author": {"name": "generate-prd-worker", "kind": "script"},
        "question": "Extract a schema-valid living markdown PRD candidate for the current repository.",
        "scope": "Tracked documents, tests, and other reviewable evidence in this checkout.",
        "acceptance": [
            "Every proposed requirement carries one reviewable repository locator and exact source extract.",
            "The candidate is parser-valid but remains provisional until human acceptance.",
        ],
        "constraints": [
            "Use the published LE-n ID grammar.",
            "Do not edit docs/PRD.md or commit automatically.",
        ],
        "non_goals": [
            "Certifying semantic completeness.",
            "Calling software-change evaluate or creating another provider.",
        ],
    }


def sources_artifact(records: Sequence[dict[str, str]]) -> dict[str, Any]:
    return {
        "revision": "1",
        "author": {"name": "generate-prd-worker", "kind": "script"},
        "brief_revision": "1",
        "sources": [
            {
                "id": f"source-{record['id']}",
                "title": record["title"],
                "locator": record["locator"],
                "extract": record["extract"],
            }
            for record in records
        ],
    }


def verification_artifact(records: Sequence[dict[str, str]]) -> dict[str, Any]:
    return {
        "revision": "1",
        "author": {"name": "generate-prd-worker", "kind": "script"},
        "sources_revision": "1",
        "claims": [
            {
                "id": f"claim-{record['id']}",
                "statement": record["title"],
                "source_ids": [f"source-{record['id']}"],
                "support": f"The exact extract from {record['locator']} supports this proposed requirement.",
                "challenge": "No contrary repository extract was found after the deterministic search.",
            }
            for record in records
        ],
    }


def report_artifact(records: Sequence[dict[str, str]], candidate: str) -> dict[str, Any]:
    return {
        "revision": "1",
        "author": {"name": "generate-prd-worker", "kind": "script"},
        "verification_revision": "1",
        "conclusion": candidate,
        "citations": [
            {
                "claim_id": f"claim-{record['id']}",
                "source_id": f"source-{record['id']}",
            }
            for record in records
        ],
    }


def run_bound_worker(arguments: argparse.Namespace) -> int:
    """Deterministic worker used only by the source journey."""
    parser = argparse.ArgumentParser()
    parser.add_argument("--worker", action="store_true")
    parser.add_argument("--repository-root", required=True)
    worker_args, _ = parser.parse_known_args(arguments)
    if not worker_args.worker:
        fail("worker mode was not selected")

    raw = sys.stdin.read()
    try:
        packet = json.loads(raw)
    except json.JSONDecodeError as error:
        print(f"worker packet is not JSON: {error}", file=sys.stderr)
        return 1
    if not isinstance(packet, dict):
        print("worker packet must be an object", file=sys.stderr)
        return 1
    required = {"run_id", "slot_id", "artifact_root", "instruction_body", "capture_dir"}
    if set(packet) != required:
        print(f"worker packet keys {sorted(packet)} != {sorted(required)}", file=sys.stderr)
        return 1
    slot = packet["slot_id"]
    artifact_root = packet["artifact_root"]
    if not isinstance(slot, str) or slot not in SLOTS:
        print(f"unexpected worker slot: {slot!r}", file=sys.stderr)
        return 1
    if not isinstance(artifact_root, str) or not artifact_root:
        print("worker packet omitted artifact_root", file=sys.stderr)
        return 1

    repository_root = Path(worker_args.repository_root).resolve()
    artifact_path = Path(artifact_root).resolve()
    artifact_path.mkdir(parents=True, exist_ok=True)
    records = evidence_records(repository_root)
    receipt_dir = artifact_path / ".generate-prd-worker-receipts"
    receipt_dir.mkdir(parents=True, exist_ok=True)
    (receipt_dir / f"{slot}.json").write_text(
        json.dumps(packet, indent=2) + "\n", encoding="utf-8"
    )

    if slot == "scope":
        write_json(artifact_path / "brief.json", brief_artifact())
    elif slot == "gather":
        write_json(artifact_path / "sources.json", sources_artifact(records))
    elif slot == "verify":
        write_json(artifact_path / "verification.json", verification_artifact(records))
    else:
        candidate = candidate_text(records)
        write_json(artifact_path / "report.json", report_artifact(records, candidate))
        (artifact_path / CANDIDATE_NAME).write_text(candidate, encoding="utf-8")
        write_json(
            artifact_path / EVIDENCE_NAME,
            {"candidate_revision": "1", "proposals": records},
        )
    return 0


def worker_command(repository_root: Path) -> dict[str, Any]:
    return {
        "command": sys.executable,
        "args": [
            str(Path(__file__).resolve()),
            "--worker",
            "--repository-root",
            str(repository_root),
        ],
    }


def assert_packet(artifact_root: Path, slot: str, overlay: dict[str, Any]) -> None:
    path = artifact_root / ".generate-prd-worker-receipts" / f"{slot}.json"
    try:
        packet = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        fail(f"bound worker did not write {path}: {error}")
    if not isinstance(packet, dict) or packet.get("slot_id") != slot:
        fail(f"bound worker packet has wrong identity: {packet}")
    if packet.get("run_id") != RUN_ID:
        fail(f"bound worker packet has wrong run id: {packet}")
    if packet.get("capture_dir") != overlay.get("capture_dir"):
        fail(f"worker capture_dir does not match invocation: {packet}")
    if not packet.get("instruction_body") or "Legal start: loop-engine invoke" in packet["instruction_body"]:
        fail("worker received redacted or empty instruction body")


def expect_rejected(response: dict[str, Any], code: str) -> None:
    if response.get("status") != "rejected" or response.get("code") != code:
        fail(f"expected rejection {code}, got {response}")


def candidate_records(text: str) -> dict[str, str]:
    """Extract only candidate headings; the real checker validates full grammar."""
    records: dict[str, str] = {}
    for line in text.splitlines():
        if not line.startswith("### LE-"):
            continue
        match = ID_HEADING.fullmatch(line)
        if match is None:
            fail(f"malformed candidate requirement heading: {line}")
        proposal_id, title = match.groups()
        if proposal_id in records:
            fail(f"candidate contains duplicate proposal id: {proposal_id}")
        records[proposal_id] = title
    if not records:
        fail("candidate contains no parseable live proposal headings")
    return records


def assert_evidence(candidate: Path, sidecar: Path, repository_root: Path) -> list[str]:
    """Check candidate-to-sidecar equality and exact tracked source extracts."""
    text = candidate.read_text(encoding="utf-8")
    candidate_titles = candidate_records(text)
    ids = list(candidate_titles)

    try:
        value = json.loads(sidecar.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        fail(f"could not read evidence sidecar: {error}")
    if not isinstance(value, dict) or set(value) != {"candidate_revision", "proposals"}:
        fail("evidence sidecar has the wrong top-level shape")
    proposals = value.get("proposals")
    if not isinstance(proposals, list) or value.get("candidate_revision") != "1":
        fail("evidence sidecar has invalid revision or proposals")
    mapped: dict[str, dict[str, str]] = {}
    for proposal in proposals:
        if not isinstance(proposal, dict) or set(proposal) != {"id", "title", "locator", "extract"}:
            fail(f"malformed evidence proposal: {proposal}")
        proposal_id = proposal.get("id")
        if not isinstance(proposal_id, str) or proposal_id in mapped:
            fail(f"duplicate or invalid evidence id: {proposal_id!r}")
        if not all(isinstance(proposal.get(key), str) and proposal[key] for key in ("title", "locator", "extract")):
            fail(f"evidence fields are not non-empty strings: {proposal}")
        if proposal["title"] != candidate_titles.get(proposal_id):
            fail(f"candidate/evidence titles differ for {proposal_id}: {proposal}")
        mapped[proposal_id] = proposal
    if set(mapped) != set(ids) or len(proposals) != len(ids):
        fail(f"candidate/evidence id sets differ: candidate={ids}, evidence={list(mapped)}")

    tracked = set(
        subprocess.run(
            ["git", "ls-files", "--cached", "--", *[item["locator"] for item in mapped.values()]],
            cwd=repository_root,
            text=True,
            capture_output=True,
            check=False,
        ).stdout.splitlines()
    )
    for proposal_id in ids:
        proposal = mapped[proposal_id]
        locator = proposal["locator"]
        locator_path = Path(locator)
        if locator_path.is_absolute() or ".." in locator_path.parts or locator not in tracked:
            fail(f"evidence locator is not a tracked relative path: {proposal}")
        try:
            source = (repository_root / locator).read_text(encoding="utf-8")
        except OSError as error:
            fail(f"could not read tracked evidence {locator}: {error}")
        if proposal["extract"] not in source:
            fail(f"evidence extract is not exact for {proposal_id}: {proposal}")
        if f"- Locator: `{locator}`" not in text or f"- Extract: `{proposal['extract']}`" not in text:
            fail(f"candidate does not carry evidence mapping for {proposal_id}")
    return ids


def assert_terminal(shown: dict[str, Any]) -> None:
    if shown.get("current_state") != "end" or shown.get("lifecycle") != "final":
        fail(f"Generate-PRD run did not reach terminal end: {shown}")


def resolve_checker(value: Optional[str]) -> Path:
    candidates = [Path(value).expanduser()] if value else []
    candidates.extend((ROOT / "target/debug/bookends-check", ROOT / "target/release/bookends-check"))
    for candidate in candidates:
        if candidate.is_file() and os.access(candidate, os.X_OK):
            return candidate.resolve()
    found = shutil.which("bookends-check")
    if found:
        return Path(found).resolve()
    fail("bookends-check executable is required for the real candidate parser command")


def validate_with_checker(checker: Path, candidate: Path) -> str:
    completed = subprocess.run(
        [str(checker), "candidate", str(candidate)],
        cwd=ROOT,
        text=True,
        capture_output=True,
        check=False,
    )
    first = completed.stdout.splitlines()[0] if completed.stdout.splitlines() else ""
    if completed.returncode != 0 or first != "GREEN":
        fail(
            "parser-only candidate validation failed: "
            f"exit={completed.returncode}, stdout={completed.stdout!r}, stderr={completed.stderr!r}"
        )
    return " ".join([str(checker), "candidate", str(candidate)])


def source_journey(args: argparse.Namespace) -> dict[str, Any]:
    engine = Path(args.engine).expanduser().resolve()
    provider = Path(args.provider).expanduser().resolve()
    profile_source = Path(args.profile).expanduser().resolve()
    checker = resolve_checker(args.checker)
    for label, path in (("engine", engine), ("provider", provider), ("profile", profile_source)):
        if not path.is_file() or (label != "profile" and not os.access(path, os.X_OK)):
            fail(f"{label} is not usable: {path}")
    profile = json.loads(profile_source.read_text(encoding="utf-8"))
    if not isinstance(profile, dict) or profile.get("extra", {}).get("profile") != "generate-prd":
        fail("source journey requires the shipped generate-prd profile")

    work_base = Path(args.work_root).expanduser().resolve() if args.work_root else Path(tempfile.mkdtemp(prefix="generate-prd-journey-"))
    work_base.mkdir(parents=True, exist_ok=True)
    run_dir = Path(tempfile.mkdtemp(prefix="source-", dir=str(work_base)))
    artifact_root = run_dir / "artifacts"
    artifact_root.mkdir()
    database = run_dir / "run.sqlite"
    profile_path = run_dir / "generate-prd.json"
    profile["artifact_root"] = str(artifact_root)
    binding = worker_command(ROOT)
    profile["work_slot_bindings"] = {slot: binding for slot in SLOTS}
    write_json(profile_path, profile)
    providers = run_dir / "providers.toml"
    providers.write_text(
        f"[providers.research]\ncommand = {json.dumps(str(provider))}\nargs = []\n",
        encoding="utf-8",
    )
    before_prd = (ROOT / "docs/PRD.md").read_bytes()

    started = cli_call(
        engine,
        database,
        [
            "--config",
            str(providers),
            "start",
            "--id",
            RUN_ID,
            "research",
            "@" + str(profile_path),
            "Generate-PRD source journey",
        ],
    )
    if started.get("status") != "completed":
        fail(f"Generate-PRD start failed: {started}")
    engine_call = lambda operation: cli_call(engine, database, operation)

    initial = show_state(engine, database, "scope")
    if initial.get("initial_input", {}).get("work_slot_bindings") != profile["work_slot_bindings"]:
        fail("frozen Generate-PRD worker bindings differ from the selected profile")
    expect_rejected(engine_call(["event", RUN_ID, "scoped"]), "bound-slot-invocation-required")

    proof: list[str] = ["started existing research provider with generate-prd profile", "bound event was gated before worker invocation"]
    config_version = profile["config_version"]
    verify_axes = [item["id"] for item in profile["review_policies"]["verify"]]
    synthesize_axes = [item["id"] for item in profile["review_policies"]["synthesize"]]
    for slot in SLOTS:
        shown = show_state(engine, database, slot)
        if "Bound work slot" not in shown.get("current_state_instructions", ""):
            fail(f"{slot} did not expose bound-worker instructions")
        overlay = work_slot_journey.invoke_until_succeeded(
            engine_call,
            RUN_ID,
            slot,
            timeout_s=15.0,
        )
        assert_packet(artifact_root, slot, overlay)
        event = EVENTS[slot]
        response = engine_call(["event", RUN_ID, event])
        if slot in ("verify", "synthesize"):
            expect_rejected(response, "research-review-incomplete")
            axes = verify_axes if slot == "verify" else synthesize_axes
            subject = "verification.json" if slot == "verify" else "report.json"
            gate = "verify" if slot == "verify" else "synthesize"
            for index, axis in enumerate(axes):
                evidence = {
                    "gate": gate,
                    "policy_id": axis,
                    "result": "pass",
                    "findings": "",
                    "author": {"name": f"generate-prd-worker-{slot}-{index}", "kind": "script"},
                    "subject": subject,
                    "subject_revision": "1",
                    "config_version": config_version,
                }
                appended = engine_call(
                    [
                        "append",
                        "--record-id",
                        f"{slot}-evidence-{index}",
                        RUN_ID,
                        "review-evidence",
                        json.dumps(evidence, separators=(",", ":")),
                    ]
                )
                if appended.get("status") != "completed":
                    fail(f"could not append deterministic {slot} evidence: {appended}")
            response = engine_call(["event", RUN_ID, event])
        if response.get("status") != "completed":
            fail(f"{slot} event failed: {response}")
        show_state(engine, database, NEXT_STATE[slot])
        proof.append(f"deterministic bound {slot} worker wrote its artifact and advanced {event}")

    terminal = show_state(engine, database, "end")
    assert_terminal(terminal)
    candidate = artifact_root / CANDIDATE_NAME
    sidecar = artifact_root / EVIDENCE_NAME
    if not candidate.is_file() or not sidecar.is_file():
        fail(f"Generate-PRD worker omitted candidate outputs: {candidate}, {sidecar}")
    ids = assert_evidence(candidate, sidecar, ROOT)
    checker_command = validate_with_checker(checker, candidate)
    if (ROOT / "docs/PRD.md").read_bytes() != before_prd:
        fail("Generate-PRD journey modified docs/PRD.md")
    proof.extend(["terminal end reached", "every proposal mapped to exact tracked evidence", "real parser-only checker returned GREEN"])
    result = {
        "journey": "generate-prd",
        "mode": "source",
        "result": "passed",
        "run_id": RUN_ID,
        "database": str(database),
        "artifact_root": str(artifact_root),
        "candidate": str(candidate),
        "evidence": str(sidecar),
        "candidate_ids": ids,
        "candidate_authority": "provisional; human acceptance and commit are required",
        "checker_command": checker_command,
        "proof": proof,
    }
    print(json.dumps(result, sort_keys=True))
    return result


def self_test() -> int:
    """Exercise the journey's fail-closed candidate/evidence contract."""
    records = evidence_records(ROOT)
    candidate = candidate_text(records)
    with tempfile.TemporaryDirectory(prefix="generate-prd-self-test-") as directory:
        root = Path(directory)
        candidate_path = root / CANDIDATE_NAME
        evidence_path = root / EVIDENCE_NAME
        candidate_path.write_text(candidate, encoding="utf-8")
        write_json(evidence_path, {"candidate_revision": "1", "proposals": records})
        assert_evidence(candidate_path, evidence_path, ROOT)

        malformed = candidate.replace("### LE-9001:", "### LE-0:", 1)
        candidate_path.write_text(malformed, encoding="utf-8")
        try:
            assert_evidence(candidate_path, evidence_path, ROOT)
        except JourneyFailure:
            pass
        else:
            raise JourneyFailure("malformed candidate self-test did not fail closed")
        candidate_path.write_text(candidate, encoding="utf-8")

        duplicate = candidate.replace(
            "### LE-9002:", "### LE-9001: duplicate\n- Status: live\n- Coverage: e2e/journey\n\n### LE-9002:", 1
        )
        candidate_path.write_text(duplicate, encoding="utf-8")
        try:
            assert_evidence(candidate_path, evidence_path, ROOT)
        except JourneyFailure:
            pass
        else:
            raise JourneyFailure("duplicate candidate self-test did not fail closed")
        candidate_path.write_text(candidate, encoding="utf-8")

        missing = {"candidate_revision": "1", "proposals": records[:-1]}
        write_json(evidence_path, missing)
        try:
            assert_evidence(candidate_path, evidence_path, ROOT)
        except JourneyFailure:
            pass
        else:
            raise JourneyFailure("missing evidence self-test did not fail closed")

        extra = {"candidate_revision": "1", "proposals": records + [dict(records[0], id="LE-9999")]}
        write_json(evidence_path, extra)
        try:
            assert_evidence(candidate_path, evidence_path, ROOT)
        except JourneyFailure:
            pass
        else:
            raise JourneyFailure("extra evidence self-test did not fail closed")

        untracked = {"candidate_revision": "1", "proposals": [dict(records[0], locator="not-tracked.md"), *records[1:]]}
        write_json(evidence_path, untracked)
        try:
            assert_evidence(candidate_path, evidence_path, ROOT)
        except JourneyFailure:
            pass
        else:
            raise JourneyFailure("untracked evidence self-test did not fail closed")

        mismatched = {"candidate_revision": "1", "proposals": [dict(records[0], extract="not the source extract"), *records[1:]]}
        write_json(evidence_path, mismatched)
        try:
            assert_evidence(candidate_path, evidence_path, ROOT)
        except JourneyFailure:
            pass
        else:
            raise JourneyFailure("mismatched evidence self-test did not fail closed")

        try:
            assert_terminal({"current_state": "verify", "lifecycle": "active"})
        except JourneyFailure:
            pass
        else:
            raise JourneyFailure("nonterminal run self-test did not fail closed")
        assert_terminal({"current_state": "end", "lifecycle": "final"})
    print(
        "generate-prd journey self-test passed: terminal-state, malformed-candidate, "
        "missing/duplicate/mismatched evidence guards"
    )
    return 0


def parse_args(argv: Optional[Sequence[str]] = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mode", choices=("source",), required=True)
    parser.add_argument("--engine", required=True)
    parser.add_argument("--provider", required=True)
    parser.add_argument("--profile", required=True)
    parser.add_argument("--checker", help="bookends-check executable")
    parser.add_argument("--work-root", help="retain the source journey output beneath this directory")
    return parser.parse_args(argv)


def main(argv: Optional[Sequence[str]] = None) -> int:
    raw = list(sys.argv[1:] if argv is None else argv)
    try:
        if not raw or raw == ["--self-test"]:
            return self_test()
        if "--worker" in raw:
            return run_bound_worker(raw)
        parsed = parse_args(raw)
        source_journey(parsed)
        return 0
    except JourneyFailure as error:
        print(f"generate-prd journey failed: {error}", file=sys.stderr)
        return 1
    except (OSError, subprocess.SubprocessError, json.JSONDecodeError) as error:
        print(f"generate-prd journey failed before assertion: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
