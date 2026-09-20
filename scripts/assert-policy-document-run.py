#!/usr/bin/env python3
"""Verify a completed, digest-bound policy-document run.

The driver supplies a saved ``show --view full`` envelope and the approved
profile/reference files.  This checker is deliberately read-only: it never
opens the engine catalog, appends evidence, invokes a worker, or progresses a
run.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path
from typing import Any, Mapping, Sequence


class VerificationError(RuntimeError):
    """The retained policy-document evidence is incomplete or inconsistent."""


def load_json(path: Path, label: str) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise VerificationError(f"could not read {label} {path}: {error}") from error


def sha256(path: Path) -> str:
    try:
        return hashlib.sha256(path.read_bytes()).hexdigest()
    except OSError as error:
        raise VerificationError(f"could not hash {path}: {error}") from error


def require(condition: bool, message: str) -> None:
    if not condition:
        raise VerificationError(message)


def as_object(value: Any, label: str) -> Mapping[str, Any]:
    require(isinstance(value, dict), f"{label} is not an object")
    return value


def show_result(envelope: Any) -> Mapping[str, Any]:
    envelope = as_object(envelope, "show envelope")
    require(envelope.get("status") == "completed", "full show did not complete")
    return as_object(envelope.get("result"), "full show result")


def profile_axes(profile: Mapping[str, Any]) -> list[str]:
    policies = profile.get("semantic_policies")
    require(isinstance(policies, list) and policies, "profile semantic_policies is empty")
    axes: list[str] = []
    for index, policy in enumerate(policies):
        policy = as_object(policy, f"profile semantic policy {index}")
        axis = policy.get("id")
        require(isinstance(axis, str) and axis, f"profile semantic policy {index} has no id")
        require(axis not in axes, f"profile repeats semantic policy {axis}")
        axes.append(axis)
    return axes


def approved_entry(
    references: Mapping[str, Any], confirmations: Mapping[str, Any], run_key: str
) -> tuple[Mapping[str, Any], Mapping[str, Any]]:
    reference = as_object(references.get(run_key), f"run reference {run_key}")
    confirmation = as_object(confirmations.get(run_key), f"confirmation {run_key}")
    return reference, confirmation


def sequence_value(record: Mapping[str, Any], fallback: int) -> tuple[int, int]:
    sequence = record.get("sequence")
    if isinstance(sequence, int) and not isinstance(sequence, bool):
        return sequence, fallback
    if isinstance(sequence, float) and sequence.is_integer():
        return int(sequence), fallback
    return fallback, fallback


def evidence_fields(data: Mapping[str, Any]) -> bool:
    expected = {
        "gate",
        "policy_id",
        "result",
        "findings",
        "author",
        "target_id",
        "target_sha256",
        "profile_version",
    }
    if set(data) != expected:
        return False
    author = data.get("author")
    return (
        isinstance(author, dict)
        and set(author) == {"name", "kind"}
        and isinstance(author.get("name"), str)
        and bool(author["name"].strip())
        and author.get("kind") in {"human", "agent", "script"}
        and data.get("gate") == "semantic-review"
        and data.get("result") in {"pass", "fail"}
        and isinstance(data.get("findings"), str)
        and (data["result"] != "fail" or bool(data["findings"].strip()))
        and isinstance(data.get("target_id"), str)
        and bool(data["target_id"].strip())
        and isinstance(data.get("target_sha256"), str)
        and len(data["target_sha256"]) == 64
        and data["target_sha256"] == data["target_sha256"].lower()
        and all(character in "0123456789abcdef" for character in data["target_sha256"])
        and isinstance(data.get("profile_version"), str)
        and bool(data["profile_version"].strip())
    )


def verify_evidence(
    context: Any,
    axes: Sequence[str],
    *,
    target_id: str,
    target_sha256: str,
    profile_version: str,
) -> None:
    require(isinstance(context, list), "full show context is not an array")
    latest: dict[tuple[str, str, str], tuple[tuple[int, int], Mapping[str, Any]]] = {}
    for index, record_value in enumerate(context):
        if not isinstance(record_value, dict) or record_value.get("kind") != "review-evidence":
            continue
        data_value = record_value.get("data")
        if not isinstance(data_value, dict):
            continue
        axis = data_value.get("policy_id")
        if axis not in axes or data_value.get("gate") != "semantic-review":
            continue
        if not evidence_fields(data_value):
            # A completed allow can contain old malformed/inert records only if
            # a later conforming record repaired the same axis.  Ignore those
            # historical records here and require the current conforming pass
            # below.
            continue
        author = data_value["author"]
        key = (axis, author["name"], author["kind"])
        position = sequence_value(record_value, index)
        previous = latest.get(key)
        if previous is None or position > previous[0]:
            latest[key] = (position, data_value)

    for axis in axes:
        current = [
            (position, data)
            for (candidate_axis, _name, _kind), (position, data) in latest.items()
            if candidate_axis == axis
            and data.get("target_id") == target_id
            and data.get("target_sha256") == target_sha256
            and data.get("profile_version") == profile_version
        ]
        require(current, f"no current digest-bound evidence for semantic axis {axis}")
        require(
            all(data.get("result") == "pass" for _position, data in current),
            f"current standing failure remains for semantic axis {axis}",
        )


def verify_allowed_edge(
    history: Any, source: str, event: str, target: str, label: str
) -> None:
    require(isinstance(history, list), "full show omitted evaluation_history")
    matching = [
        entry
        for entry in history
        if isinstance(entry, dict)
        and isinstance(entry.get("transition"), dict)
        and entry["transition"].get("source") == source
        and entry["transition"].get("event") == event
        and entry["transition"].get("target") == target
    ]
    require(matching, f"full show has no {label} evaluation")
    final_result = as_object(matching[-1].get("result"), f"{label} evaluation")
    require(final_result.get("result") == "allow", f"{label} transition was not allowed")


def verify_final_evaluation(result: Mapping[str, Any]) -> None:
    verify_allowed_edge(
        result.get("evaluation_history"),
        "deterministic-review",
        "passed",
        "semantic-review",
        "deterministic",
    )
    verify_allowed_edge(
        result.get("evaluation_history"),
        "semantic-review",
        "passed",
        "end",
        "final semantic",
    )

    latest = result.get("latest_evaluations")
    require(isinstance(latest, list), "full show omitted latest_evaluations")
    projected = [
        entry
        for entry in latest
        if isinstance(entry, dict)
        and isinstance(entry.get("transition"), dict)
        and entry["transition"].get("source") == "semantic-review"
        and entry["transition"].get("event") == "passed"
        and entry["transition"].get("target") == "end"
    ]
    require(len(projected) == 1, "latest evaluation projection has no unique final semantic edge")
    projected_result = as_object(projected[0].get("result"), "projected final semantic evaluation")
    require(projected_result.get("result") == "allow", "projected final semantic edge was not allowed")


def verify(args: argparse.Namespace) -> None:
    show = load_json(args.show, "full show envelope")
    result = show_result(show)
    references = as_object(load_json(args.run_reference, "run reference file"), "run references")
    confirmations = as_object(load_json(args.confirmation, "confirmation manifest"), "confirmation manifest")
    reference, confirmation = approved_entry(references, confirmations, args.run_key)
    profile = as_object(load_json(args.profile, "approved profile"), "approved profile")

    expected_profile_sha256 = args.profile_sha256.lower()
    require(len(expected_profile_sha256) == 64, "--profile-sha256 must be 64 hexadecimal characters")
    require(
        all(character in "0123456789abcdef" for character in expected_profile_sha256),
        "--profile-sha256 must be lowercase hexadecimal",
    )
    actual_profile_sha256 = sha256(args.profile)
    require(actual_profile_sha256 == expected_profile_sha256, "approved profile digest does not match profile bytes")
    require(reference.get("profile_sha256") == expected_profile_sha256, "run reference profile digest differs")
    require(confirmation.get("sha256") == expected_profile_sha256, "confirmation profile digest differs")
    require(Path(str(confirmation.get("profile"))).resolve() == args.profile.resolve(), "confirmation profile path differs")

    target = as_object(profile.get("target"), "profile target")
    reference_target = reference.get("target")
    confirmation_target = as_object(confirmation.get("target"), "confirmation target")
    require(isinstance(reference_target, str), "run reference target is not a path string")
    require(reference_target == target.get("path"), "run reference target differs from profile")
    require(confirmation_target == target, "confirmation target differs from approved profile target")
    require(target.get("path") == str(args.target), "profile target path differs from --target")
    require(target.get("id") == confirmation_target.get("id"), "profile target id differs from confirmation")
    require(confirmation.get("mode") == profile.get("mode"), "confirmation mode differs from profile")
    require(confirmation.get("profile_version") == profile.get("profile_version"), "confirmation profile version differs")

    axes = profile_axes(profile)
    require(confirmation.get("axes") == axes, "confirmation semantic axes differ from profile order")
    schema = as_object(load_json(args.evidence_schema, "semantic worker output schema"), "semantic worker output schema")
    require(schema.get("required") == ["axis", "author", "result", "findings"], "unexpected semantic worker output schema")

    target_sha256 = sha256(args.target)
    initial_input = as_object(result.get("initial_input"), "full show initial_input")
    require(result.get("run_id") == reference.get("run_id"), "show run ID differs from approved reference")
    require(result.get("current_state") == "end", "policy-document run is not at end")
    require(result.get("lifecycle") == "final", "policy-document run is not final")
    require(initial_input.get("mode") == profile.get("mode"), "run mode differs from approved profile")
    require(initial_input.get("profile_version") == profile.get("profile_version"), "run profile version differs")
    require(initial_input.get("target") == target, "run target differs from approved profile")
    if "work_slot_bindings" in profile:
        require(
            initial_input.get("work_slot_bindings") == profile.get("work_slot_bindings"),
            "run bindings differ from approved profile",
        )

    verify_evidence(
        result.get("context"),
        axes,
        target_id=str(target["id"]),
        target_sha256=target_sha256,
        profile_version=str(profile["profile_version"]),
    )
    verify_final_evaluation(result)
    print(
        json.dumps(
            {
                "status": "verified",
                "run_key": args.run_key,
                "run_id": result["run_id"],
                "state": result["current_state"],
                "lifecycle": result["lifecycle"],
                "target": str(args.target),
                "target_sha256": target_sha256,
                "profile_version": profile["profile_version"],
                "profile_sha256": actual_profile_sha256,
                "axes": axes,
            },
            sort_keys=True,
        )
    )


def parse_args(argv: Sequence[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--show", type=Path, required=True)
    parser.add_argument("--run-reference", type=Path, required=True)
    parser.add_argument("--confirmation", type=Path, required=True)
    parser.add_argument("--profile", type=Path, required=True)
    parser.add_argument("--profile-sha256", required=True)
    parser.add_argument("--target", type=Path, required=True)
    parser.add_argument("--evidence-schema", type=Path, required=True)
    parser.add_argument("--run-key", required=True)
    return parser.parse_args(argv)


def main(argv: Sequence[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    args.show = args.show.resolve()
    args.run_reference = args.run_reference.resolve()
    args.confirmation = args.confirmation.resolve()
    args.profile = args.profile.resolve()
    args.target = args.target.resolve()
    args.evidence_schema = args.evidence_schema.resolve()
    try:
        verify(args)
    except VerificationError as error:
        print(f"policy-document run check failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
