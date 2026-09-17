#!/usr/bin/env python3
"""Public outcome proof for the fictional software-change reference workflow."""
from __future__ import annotations

import argparse
import json
import os
import subprocess
from pathlib import Path
from typing import Any


class JourneyFailure(RuntimeError):
    pass


# Each deficit fixture is an otherwise-valid current evidence set with exactly
# the named qualifying contribution absent. The deficient fixture attempts that
# contribution with one configured defect; the repair supplies only that
# contribution. The driver-owned setup supplies these files as fictional data.
EVIDENCE_CASES = (
    (
        "stale revision",
        "intent-evidence-deficit-stale-revision.json",
        "evidence-stale-revision.json",
        "intent-evidence-repair-stale-revision.json",
    ),
    (
        "subject author",
        "intent-evidence-deficit-self-authored.json",
        "evidence-self-authored.json",
        "intent-evidence-repair-self-authored.json",
    ),
    (
        "duplicate author",
        "intent-evidence-deficit-duplicate-author.json",
        "evidence-duplicate-author.json",
        "intent-evidence-repair-duplicate-author.json",
    ),
    (
        "incomplete axis",
        "intent-evidence-deficit-incomplete-axis.json",
        "evidence-incomplete-axis.json",
        "intent-evidence-repair-incomplete-axis.json",
    ),
)

STATIC_FIXTURES = (
    "intent-malformed.json",
    "intent-good.json",
    "intent-three-violations.json",
    "validation-report-good.json",
)


def invoke(engine: Path, config: Path, *args: str) -> dict[str, Any]:
    completed = subprocess.run(
        [str(engine), "--json", "--config", str(config), *args],
        check=False,
        capture_output=True,
        text=True,
    )
    try:
        envelope = json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        raise JourneyFailure(
            f"command {args!r} did not return one JSON envelope: {error}"
        ) from error
    if not isinstance(envelope, dict) or envelope.get("status") not in {
        "completed",
        "rejected",
        "error",
    }:
        raise JourneyFailure(f"invalid command envelope for {args!r}: {envelope!r}")
    expected_exit = {"completed": 0, "rejected": 10, "error": 20}[envelope["status"]]
    if completed.returncode != expected_exit:
        raise JourneyFailure(
            f"command {args!r} returned status={envelope['status']!r} "
            f"with exit {completed.returncode}, expected {expected_exit}"
        )
    envelope["_exit_code"] = completed.returncode
    return envelope


def validate_setup(
    engine: Path, config: Path, profile: Path, fixtures: Path
) -> None:
    """Validate the driver-supplied fictional CLI and fixture handoff."""
    for label, path in (
        ("engine binary", engine),
        ("provider TOML", config),
        ("profile", profile),
    ):
        if not path.is_file():
            raise JourneyFailure(f"{label} does not exist: {path}")
    if not os.access(engine, os.X_OK):
        raise JourneyFailure(f"engine binary is not executable: {engine}")
    if not fixtures.is_dir():
        raise JourneyFailure(f"fixture directory does not exist: {fixtures}")
    try:
        profile_data = json.loads(profile.read_text())
    except (OSError, json.JSONDecodeError) as error:
        raise JourneyFailure(f"profile is not readable JSON: {profile}: {error}") from error
    if profile_data.get("config_version") != "standard-7":
        raise JourneyFailure("fictional journey requires the unchanged standard-7 profile")

    required = list(STATIC_FIXTURES)
    for _reason, deficit, invalid, repair in EVIDENCE_CASES:
        required.extend((deficit, invalid, repair))
    missing = [name for name in required if not (fixtures / name).is_file()]
    validation_records = sorted(fixtures.glob("validation-evidence-good-*.json"))
    if not validation_records:
        missing.append("validation-evidence-good-*.json")
    if missing:
        raise JourneyFailure(
            "fictional journey setup is missing required fixture inputs: "
            + ", ".join(missing)
        )
    for name in required:
        path = fixtures / name
        try:
            json.loads(path.read_text())
        except (OSError, json.JSONDecodeError) as error:
            raise JourneyFailure(f"fixture is not readable JSON: {path}: {error}") from error
    for path in validation_records:
        try:
            json.loads(path.read_text())
        except (OSError, json.JSONDecodeError) as error:
            raise JourneyFailure(f"fixture is not readable JSON: {path}: {error}") from error


def data(envelope: dict[str, Any]) -> dict[str, Any]:
    value = envelope.get("data")
    if not isinstance(value, dict):
        raise JourneyFailure(f"missing envelope data: {envelope!r}")
    return value


def feedback_details(envelope: dict[str, Any]) -> list[str]:
    feedback = data(envelope).get("feedback", {})
    details = feedback.get("details", []) if isinstance(feedback, dict) else []
    if not isinstance(details, list) or not all(isinstance(item, str) for item in details):
        raise JourneyFailure(f"feedback details are not strings: {details!r}")
    return details


def append_record(
    engine: Path, config: Path, run_id: str, fixture_root: Path, name: str
) -> dict[str, Any]:
    return invoke(engine, config, "append", run_id, f"@{fixture_root / name}")


def append_path(
    engine: Path, config: Path, run_id: str, record_path: Path
) -> dict[str, Any]:
    return invoke(engine, config, "append", run_id, f"@{record_path}")


def frozen_profile_is_inspectable_before_transition(
    engine: Path, config: Path, profile: Path, fixtures: Path
) -> str:
    started = invoke(
        engine,
        config,
        "start",
        "software-change",
        f"@{profile}",
        "public frozen-profile inspection proof",
    )
    assert started["status"] == "completed"
    run_id = data(started)["run_id"]

    shown_before = invoke(engine, config, "show", run_id)
    assert shown_before["status"] == "completed"
    frozen = data(shown_before)["initial_input"]
    assert frozen["config_version"] == "standard-7"
    assert set(frozen["review_policies"]) == {
        "intent-review",
        "intent-adversarial-review",
        "design-review",
        "design-adversarial-review",
        "plan-review",
        "plan-adversarial-review",
        "implementation-review",
        "implementation-adversarial-review",
        "validation-review",
        "validation-adversarial-review",
    }
    assert all(
        policy["required_authors"] >= 1
        for policies in frozen["review_policies"].values()
        for policy in policies
    )
    assert set(frozen["artifact_schemas"]) == {
        "intent.json",
        "design.json",
        "plan.json",
        "implementation-report.json",
        "validation-report.json",
    }
    assert frozen["templates"] and frozen["reviewer_protocol"]
    assert all(policy["example_prompt"] for policies in frozen["review_policies"].values() for policy in policies)

    append_record(engine, config, run_id, fixtures, "intent-malformed.json")
    denied = invoke(engine, config, "event", run_id, "intent-ready")
    assert denied["status"] == "rejected"
    assert data(denied)["current_state"] == "explore"
    shown_after_denial = invoke(engine, config, "show", run_id)
    assert data(shown_after_denial)["initial_input"] == frozen

    append_record(engine, config, run_id, fixtures, "intent-good.json")
    ready = invoke(engine, config, "event", run_id, "intent-ready")
    assert ready["status"] == "completed"
    assert data(ready)["current_state"] == "intent-review"
    shown_after_ready = invoke(engine, config, "show", run_id)
    assert data(shown_after_ready)["initial_input"] == frozen
    return run_id


def malformed_artifact_denial_names_all_rules(
    engine: Path, config: Path, profile: Path, fixtures: Path
) -> None:
    started = invoke(
        engine,
        config,
        "start",
        "software-change",
        f"@{profile}",
        "public structural-denial proof",
    )
    assert started["status"] == "completed"
    run_id = data(started)["run_id"]
    initial = invoke(engine, config, "show", run_id)
    assert data(initial)["current_state"] == "explore"
    append_record(engine, config, run_id, fixtures, "intent-three-violations.json")
    denied = invoke(engine, config, "event", run_id, "intent-ready")
    assert denied["status"] == "rejected"
    assert data(denied)["current_state"] == "explore"
    assert feedback_details(denied) == [
        "/revision|required",
        "/outcome|minLength",
        "/extra|additionalProperties",
    ]
    assert not any("evidence" in detail.lower() for detail in feedback_details(denied))


def evidence_denial_reports_each_configured_reason(
    engine: Path, config: Path, profile: Path, fixtures: Path
) -> None:
    for index, (reason, deficit_fixture, invalid_fixture, repair_fixture) in enumerate(
        EVIDENCE_CASES, start=1
    ):
        started = invoke(
            engine,
            config,
            "start",
            "software-change",
            f"@{profile}",
            f"public evidence proof {index}",
        )
        run_id = data(started)["run_id"]
        append_record(engine, config, run_id, fixtures, "intent-good.json")
        ready = invoke(engine, config, "event", run_id, "intent-ready")
        assert ready["status"] == "completed"
        assert data(ready)["current_state"] == "intent-review"

        # The deficit is deliberate: no complete qualifying evidence is loaded
        # before the invalid contribution. A broken acceptance that counts the
        # deficient record must fail the following unchanged-state assertion.
        append_record(engine, config, run_id, fixtures, deficit_fixture)
        append_record(engine, config, run_id, fixtures, invalid_fixture)
        denied = invoke(engine, config, "event", run_id, "approved")
        assert denied["status"] == "rejected"
        assert data(denied)["current_state"] == "intent-review"
        assert data(denied).get("transition") is None
        details = "\n".join(feedback_details(denied)).lower()
        assert reason in details

        append_record(engine, config, run_id, fixtures, repair_fixture)
        accepted = invoke(engine, config, "event", run_id, "approved")
        assert accepted["status"] == "completed"
        assert data(accepted)["current_state"] == "intent-adversarial-review"


def terminal_validation_gate(
    engine: Path, config: Path, terminal_run_id: str, fixtures: Path
) -> None:
    shown = invoke(engine, config, "show", terminal_run_id)
    run = data(shown)
    assert run["current_state"] == "validation-adversarial-review"
    frozen = run["initial_input"]
    assert frozen["config_version"] == "standard-7"
    required = {
        policy["id"]: policy["required_authors"]
        for policy in frozen["review_policies"]["validation-adversarial-review"]
    }
    assert required and all(count >= 1 for count in required.values())

    subject = json.loads((fixtures / "validation-report-good.json").read_text())
    subject_revision = subject["revision"]
    denied = invoke(engine, config, "event", terminal_run_id, "passed")
    assert denied["status"] == "rejected"
    assert data(denied)["current_state"] == "validation-adversarial-review"
    assert data(denied).get("transition") is None
    denial_text = "\n".join(feedback_details(denied))
    for policy_id in required:
        assert policy_id in denial_text
    denial_id = data(denied)["evaluation_id"]

    evidence_paths = sorted(fixtures.glob("validation-evidence-good-*.json"))
    assert len(evidence_paths) > 1
    authors: dict[str, set[tuple[str, str]]] = {policy_id: set() for policy_id in required}

    def append_validation_record(record_path: Path) -> dict[str, Any]:
        record = json.loads(record_path.read_text())
        review = record["data"]
        assert record["kind"] == "review-evidence"
        assert review["gate"] == "validation-adversarial-review"
        assert review["policy_id"] in required
        assert review["subject"] == "validation-report.json"
        assert review["subject_revision"] == subject_revision
        assert review["config_version"] == frozen["config_version"]
        assert review["result"] == "pass" and review["findings"] == ""
        author = (review["author"]["name"], review["author"]["kind"])
        assert author != (subject["author"]["name"], subject["author"]["kind"])
        authors[review["policy_id"]].add(author)
        appended = append_path(engine, config, terminal_run_id, record_path)
        assert appended["status"] == "completed"
        return review

    for record_path in evidence_paths[:-1]:
        append_validation_record(record_path)
    partial = invoke(engine, config, "event", terminal_run_id, "passed")
    assert partial["status"] == "rejected"
    assert data(partial)["current_state"] == "validation-adversarial-review"
    assert data(partial).get("transition") is None
    missing_policy = json.loads(evidence_paths[-1].read_text())["data"]["policy_id"]
    assert missing_policy in "\n".join(feedback_details(partial))

    append_validation_record(evidence_paths[-1])
    for policy_id, count in required.items():
        assert len(authors[policy_id]) >= count

    accepted = invoke(engine, config, "event", terminal_run_id, "passed")
    assert accepted["status"] == "completed"
    assert data(accepted)["current_state"] == "end"

    final_show = invoke(engine, config, "show", terminal_run_id)
    assert data(final_show)["current_state"] == "end"
    assert data(final_show)["requestable_events"] == []
    assert denial_id in {item["id"] for item in data(final_show)["evaluations"]}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--engine", type=Path, required=True)
    parser.add_argument("--config", type=Path, required=True)
    parser.add_argument("--profile", type=Path, required=True)
    parser.add_argument("--fixtures", type=Path, required=True)
    parser.add_argument("--terminal-run-id", required=True)
    args = parser.parse_args()
    validate_setup(args.engine, args.config, args.profile, args.fixtures)
    frozen_profile_is_inspectable_before_transition(args.engine, args.config, args.profile, args.fixtures)
    malformed_artifact_denial_names_all_rules(args.engine, args.config, args.profile, args.fixtures)
    evidence_denial_reports_each_configured_reason(args.engine, args.config, args.profile, args.fixtures)
    terminal_validation_gate(args.engine, args.config, args.terminal_run_id, args.fixtures)
    print("public software-change outcome scenarios passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
