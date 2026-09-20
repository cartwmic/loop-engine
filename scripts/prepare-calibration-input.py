#!/usr/bin/env python3
"""Prepare one fresh, oracle-free supplied-material calibration packet.

The utility only reads shipped procedure/data and writes an isolated packet. It
never invokes a reviewer, records a judgment, or copies manifest oracle fields.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
from typing import Any, Iterable


COVERAGE_CASES = ("sufficient", "related-insufficient", "implementation-defect")
COVERAGE_GATES = ("intent-review", "intent-adversarial-review")
PROFILE_NAMES = ("minimal", "standard", "high-rigor")
SUBJECTS = {
    "intent-review": ("intent.json", "intent.md", ()),
    "intent-adversarial-review": ("intent.json", "intent.md", ()),
    "design-review": ("design.json", "design.md", ("intent-good",)),
    "design-adversarial-review": ("design.json", "design.md", ("intent-good",)),
    "plan-review": ("plan.json", "task-packet.md", ("intent-good", "design-good")),
    "plan-adversarial-review": ("plan.json", "task-packet.md", ("intent-good", "design-good")),
    "implementation-review": (
        "implementation-report.json",
        "implementation-report.md",
        ("intent-good", "design-good", "plan-good"),
    ),
    "implementation-adversarial-review": (
        "implementation-report.json",
        "implementation-report.md",
        ("intent-good", "design-good", "plan-good"),
    ),
    "validation-review": (
        "validation-report.json",
        "validation-report.md",
        ("intent-good", "design-good", "plan-good", "implementation-report-good"),
    ),
    "validation-adversarial-review": (
        "validation-report.json",
        "validation-report.md",
        ("intent-good", "design-good", "plan-good", "implementation-report-good"),
    ),
}

ORDINARY_COVERAGE_PROMPT = (
    "Judge ids-grounded only. Confirm each promised enduring outcome is checked against "
    "the actual accepted requirement text and every authoritative document it explicitly "
    "names. Distinguish sufficient existing wording, missing or changed enduring meaning, "
    "and change-specific proof. A live or related ID, shared topic, or matching token is "
    "not semantic coverage; an implementation defect under sufficient wording does not "
    "require a new requirement. Candidates remain provisional until exact owner acceptance "
    "and separately authorized application and commit. Do not re-judge Bookends checker "
    "red/green."
)
CHALLENGE_COVERAGE_PROMPT = (
    "Falsify ids-grounded only. Attack the ordinary ids-grounded pass by finding a "
    "promised enduring outcome whose cited requirement text or explicit cross-reference "
    "does not demand it, or by showing that a sufficient requirement is being misclassified "
    "as missing when supplied evidence instead shows an implementation defect. A related "
    "ID, shared topic, matching token, candidate flag, or parser success cannot establish "
    "semantic coverage. Do not re-judge Bookends checker red/green."
)


def die(message: str) -> "NoReturn":
    raise ValueError(message)


def read_bytes(path: Path, description: str) -> bytes:
    try:
        return path.read_bytes()
    except OSError as error:
        die(f"could not read {description} {path}: {error}")


def read_json(path: Path, description: str) -> Any:
    try:
        return json.loads(read_bytes(path, description))
    except json.JSONDecodeError as error:
        die(f"invalid JSON in {description} {path}: {error}")


def canonical_json(value: Any) -> bytes:
    """Compact JSON with recursively bytewise UTF-8 sorted object keys."""

    def sorted_value(item: Any) -> Any:
        if isinstance(item, dict):
            return {
                key: sorted_value(item[key])
                for key in sorted(item, key=lambda key: key.encode("utf-8"))
            }
        if isinstance(item, list):
            return [sorted_value(child) for child in item]
        return item

    return json.dumps(
        sorted_value(value), ensure_ascii=False, separators=(",", ":")
    ).encode("utf-8")


def quote_json_string(value: str) -> str:
    # json.dumps emits the RFC 8259 escapes used by the procedure, but its
    # compact object serialization is intentionally kept explicit here so the
    # six-field request order cannot drift when this utility changes.
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"))


def canonical_request(
    gate: str,
    axis: str,
    stage: str,
    subject: str,
    subject_revision: str,
    config_version: str,
) -> bytes:
    fields = (
        ("gate", gate),
        ("policy_id", axis),
        ("review_stage", stage),
        ("subject", subject),
        ("subject_revision", subject_revision),
        ("config_version", config_version),
    )
    return (
        "{"
        + ",".join(f"{quote_json_string(key)}:{quote_json_string(value)}" for key, value in fields)
        + "}"
    ).encode("utf-8")


def framed_digest(records: Iterable[tuple[str, bytes]]) -> str:
    records = list(records)
    stream = bytearray()
    stream.extend(len(records).to_bytes(8, "big"))
    for label, content in records:
        label_bytes = label.encode("utf-8")
        stream.extend(len(label_bytes).to_bytes(8, "big"))
        stream.extend(label_bytes)
        stream.extend(len(content).to_bytes(8, "big"))
        stream.extend(content)
    return hashlib.sha256(stream).hexdigest()


def safe_join(root: Path, relative: str, description: str) -> Path:
    path = (root / relative).resolve()
    if path != root and root not in path.parents:
        die(f"{description} escapes repository root: {relative}")
    return path


def profile_name(profile: Path) -> str:
    name = profile.stem
    if name not in PROFILE_NAMES:
        die(f"profile must be one of {', '.join(PROFILE_NAMES)}: {profile}")
    return name


def parse_row_key(value: str) -> dict[str, str]:
    fields = value.split("|")
    if len(fields) != 5 or any(not field for field in fields):
        die(
            "row key must be `config_version|gate|axis|review_stage|fixture_id`: "
            + value
        )
    config, gate, axis, stage, fixture = fields
    if any("/" in field or "\\" in field for field in fields):
        die(f"row key fields may not contain path separators: {value}")
    return {
        "config_version": config,
        "gate": gate,
        "axis": axis,
        "review_stage": stage,
        "fixture_id": fixture,
        "row_key": value,
    }


def manifest_row(manifest: Any, identity: dict[str, str]) -> dict[str, Any]:
    if not isinstance(manifest, list):
        die("calibration manifest must be an array")
    matches = [
        row
        for row in manifest
        if isinstance(row, dict)
        and all(row.get(field) == identity[field] for field in (
            "config_version", "gate", "axis", "review_stage", "fixture_id"
        ))
    ]
    if len(matches) != 1:
        die(
            f"row key must identify exactly one manifest row, found {len(matches)}: "
            + identity["row_key"]
        )
    return matches[0]


def policy_prompt(profile: dict[str, Any], gate: str, axis: str, stage: str) -> str:
    policies = profile.get("review_policies")
    entries = policies.get(gate) if isinstance(policies, dict) else None
    if not isinstance(entries, list):
        die(f"profile has no policy list for {gate}")
    matches = [
        entry
        for entry in entries
        if isinstance(entry, dict)
        and entry.get("id") == axis
        and entry.get("review_stage", "aggregate") == stage
    ]
    if len(matches) != 1:
        die(f"profile must have exactly one policy for {gate}/{axis}/{stage}")
    prompt = matches[0].get("example_prompt")
    if not isinstance(prompt, str) or not prompt:
        die(f"policy prompt is not a non-empty string for {gate}/{axis}/{stage}")
    return prompt


def fixture_identity(path: Path, label: str) -> tuple[dict[str, Any], bytes]:
    content = read_bytes(path, label)
    value = read_json(path, label)
    if not isinstance(value, dict):
        die(f"fixture must be an object: {path}")
    revision = value.get("revision")
    if not isinstance(revision, str) or not revision:
        die(f"fixture revision must be a non-empty string: {path}")
    return value, content


def make_case(
    *,
    repository: Path,
    procedure: bytes,
    manifest: Any,
    profile_path: Path,
    profile: dict[str, Any],
    output_root: Path,
    identity: dict[str, str],
    fixture_path: Path | None = None,
    companion_path: Path | None = None,
    coverage_case: str | None = None,
    coverage_gate: str | None = None,
) -> Path:
    profile_basename = profile_name(profile_path)
    config_version = identity["config_version"]
    gate = identity["gate"]
    axis = identity["axis"]
    stage = identity["review_stage"]
    fixture_id = identity["fixture_id"]

    if coverage_case is None:
        manifest_row(manifest, identity)
        if profile.get("config_version") != config_version:
            die(
                f"profile config_version {profile.get('config_version')!r} does not match row {config_version!r}"
            )
        if gate not in SUBJECTS:
            die(f"unsupported calibration gate {gate}")
        subject, template, predecessors = SUBJECTS[gate]
        fixture_path = safe_join(
            repository,
            f"crates/software-change-provider/data/calibration/fixtures/{fixture_id}.json",
            "fixture",
        )
        prompt = policy_prompt(profile, gate, axis, stage)
        case_name = identity["row_key"]
        coverage_metadata: dict[str, Any] = {}
    else:
        if coverage_case not in COVERAGE_CASES:
            die(f"unsupported coverage case {coverage_case}")
        if coverage_gate not in COVERAGE_GATES:
            die(f"unsupported coverage gate {coverage_gate}")
        if axis != "ids-grounded" or stage != "aggregate":
            die("coverage packets require ids-grounded at aggregate stage")
        if profile.get("config_version") != config_version:
            die(
                f"profile config_version {profile.get('config_version')!r} does not match coverage row {config_version!r}"
            )
        subject, template, predecessors = SUBJECTS[coverage_gate]
        prompt = (
            ORDINARY_COVERAGE_PROMPT
            if coverage_gate == "intent-review"
            else CHALLENGE_COVERAGE_PROMPT
        )
        case_name = f"coverage-{coverage_case}-{coverage_gate}"
        coverage_metadata = {
            "coverage_case": coverage_case,
            "coverage_gate": coverage_gate,
        }
        fixture_id = Path(fixture_path).stem

    assert fixture_path is not None
    subject_value, subject_bytes = fixture_identity(fixture_path, "subject fixture")
    subject_revision = subject_value["revision"]
    prompt_bytes = prompt.encode("utf-8")
    schema_value = profile.get("artifact_schemas", {}).get(subject)
    if schema_value is None:
        die(f"profile has no artifact schema for {subject}")
    schema_bytes = canonical_json(schema_value)

    instruction_bytes = read_bytes(
        repository / "crates/software-change-provider/data/calibration/reviewer-instruction.txt",
        "reviewer instruction",
    )
    if instruction_bytes.startswith(b"\xef\xbb\xbf") or not instruction_bytes.endswith(b"\n"):
        die("reviewer instruction must be UTF-8 without BOM and have one final LF")
    if b"\r" in instruction_bytes:
        die("reviewer instruction must be LF-only")
    protocol_bytes = read_bytes(
        repository / "crates/software-change-provider/data/reviewer-protocol.md",
        "reviewer protocol",
    )
    template_bytes = read_bytes(
        repository / f"crates/software-change-provider/data/templates/{template}",
        "template",
    )

    records: list[tuple[str, bytes]] = [
        (
            "system-developer-instruction:data/calibration/reviewer-instruction.txt",
            instruction_bytes,
        ),
        ("example_prompt", prompt_bytes),
        ("reviewer-protocol:data/reviewer-protocol.md", protocol_bytes),
        (f"template:data/templates/{template}", template_bytes),
        (
            f"schema:data/configs/{profile_basename}.json#/artifact_schemas/{subject}",
            schema_bytes,
        ),
        (
            f"subject:data/calibration/fixtures/{fixture_id}.json",
            subject_bytes,
        ),
    ]
    for predecessor in predecessors:
        predecessor_path = safe_join(
            repository,
            f"crates/software-change-provider/data/calibration/fixtures/{predecessor}.json",
            "predecessor fixture",
        )
        records.append(
            (
                f"required predecessor:data/calibration/fixtures/{predecessor}.json",
                read_bytes(predecessor_path, "predecessor fixture"),
            )
        )
    if companion_path is not None:
        companion_bytes = read_bytes(companion_path, "companion")
        records.append(("companion:fictional-repo/docs/requirement-coverage.md", companion_bytes))

    request_bytes = canonical_request(
        gate, axis, stage, subject, subject_revision, config_version
    )
    records.append(("request-json", request_bytes))
    input_hash = framed_digest(records)

    # The procedure is an authority input to this producer, but is deliberately
    # not a reviewer source record. Reading it here catches an accidentally
    # substituted procedure without leaking manifest/oracle metadata.
    if b"Canonical digest framing" not in procedure or b"Fresh external review input" not in procedure:
        die("procedure does not contain the shipped supplied-material contract")

    case_dir = output_root / case_name
    if case_dir.exists():
        die(f"refusing to replace existing calibration case directory: {case_dir}")
    case_dir.mkdir(parents=True)
    records_dir = case_dir / "source-records"
    records_dir.mkdir()
    record_metadata = []
    for index, (label, content) in enumerate(records):
        filename = f"{index:03d}.bin"
        (records_dir / filename).write_bytes(content)
        record_metadata.append(
            {
                "label": label,
                "path": f"source-records/{filename}",
                "byte_length": len(content),
                "sha256": hashlib.sha256(content).hexdigest(),
            }
        )

    # Delimiters are packet framing only. The exact records remain available in
    # source-records/ and their unmodified bytes are what input_sha256 covers.
    rendered = bytearray()
    for label, content in records:
        rendered.extend(b"=== source-record: " + label.encode("utf-8") + b" ===\n")
        rendered.extend(content)
        if not content.endswith(b"\n"):
            rendered.extend(b"\n")
        rendered.extend(b"\n")
    instructions_bytes = bytes(rendered)
    (case_dir / "instructions.txt").write_bytes(instructions_bytes)
    (case_dir / "request.json").write_bytes(request_bytes)
    (case_dir / "input_sha256").write_text(input_hash + "\n", encoding="ascii")
    preparation: dict[str, Any] = {
        "schema_version": 1,
        "kind": "calibration-input",
        "row_key": identity["row_key"],
        "config_version": config_version,
        "gate": gate,
        "policy_id": axis,
        "review_stage": stage,
        "subject": subject,
        "subject_revision": subject_revision,
        "fixture_id": fixture_id,
        "profile": profile_basename,
        "template": template,
        "source_records": record_metadata,
        "request_path": "request.json",
        "instructions_path": "instructions.txt",
        "input_sha256": input_hash,
        "prompt_sha256": hashlib.sha256(prompt_bytes).hexdigest(),
        "instructions_sha256": hashlib.sha256(instructions_bytes).hexdigest(),
    }
    preparation.update(coverage_metadata)
    (case_dir / "preparation.json").write_text(
        json.dumps(preparation, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    return case_dir


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--procedure", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--profile", type=Path, required=True)
    parser.add_argument("--repository", type=Path, required=True)
    parser.add_argument("--output-root", type=Path, required=True)
    parser.add_argument("--row-key", action="append", default=[])
    parser.add_argument("--coverage-case", action="append", choices=COVERAGE_CASES, default=[])
    parser.add_argument("--coverage-gate", choices=COVERAGE_GATES)
    parser.add_argument("--coverage-axis", default="ids-grounded")
    parser.add_argument("--coverage-stage", default="aggregate")
    parser.add_argument("--fixture", type=Path)
    parser.add_argument("--companion", type=Path)
    return parser


def main() -> int:
    parser = build_parser()
    args = parser.parse_args()
    try:
        if not args.row_key and not args.coverage_case:
            die("supply at least one --row-key or --coverage-case")
        if args.row_key and args.coverage_case:
            die("do not mix --row-key and --coverage-case in one preparation call")
        repository = args.repository.resolve(strict=True)
        procedure_path = args.procedure.resolve(strict=True)
        manifest_path = args.manifest.resolve(strict=True)
        profile_path = args.profile.resolve(strict=True)
        output_root = args.output_root.resolve()
        output_root.mkdir(parents=True, exist_ok=True)
        procedure = read_bytes(procedure_path, "procedure")
        manifest = read_json(manifest_path, "manifest")
        profile = read_json(profile_path, "profile")
        if not isinstance(profile, dict):
            die("profile must be a JSON object")
        if len(set(args.row_key)) != len(args.row_key):
            die("duplicate --row-key")
        if len(set(args.coverage_case)) != len(args.coverage_case):
            die("duplicate --coverage-case")

        cases: list[Path] = []
        if args.row_key:
            for raw_key in args.row_key:
                identity = parse_row_key(raw_key)
                cases.append(
                    make_case(
                        repository=repository,
                        procedure=procedure,
                        manifest=manifest,
                        profile_path=profile_path,
                        profile=profile,
                        output_root=output_root,
                        identity=identity,
                    )
                )
        else:
            if len(args.coverage_case) != 1:
                die("coverage preparation accepts exactly one --coverage-case per fixture packet")
            if args.coverage_gate is None or args.fixture is None or args.companion is None:
                die("coverage preparation requires --coverage-gate, --fixture, and --companion")
            fixture_path = args.fixture.resolve(strict=True)
            companion_path = args.companion.resolve(strict=True)
            expected_fixture = (
                repository
                / "crates/software-change-provider/data/calibration/fixtures"
                / f"requirement-coverage-{args.coverage_case[0]}.json"
            ).resolve()
            expected_companion = (
                repository
                / "crates/software-change-provider/data/calibration/companions/fictional-repo/docs/requirement-coverage.md"
            ).resolve()
            if fixture_path != expected_fixture:
                die(f"coverage fixture must be the selected shipped fixture: {fixture_path}")
            if companion_path != expected_companion:
                die(f"coverage companion must be the selected shipped companion: {companion_path}")
            for coverage_case in args.coverage_case:
                gate = args.coverage_gate
                config_version = profile.get("config_version")
                if not isinstance(config_version, str):
                    die("profile config_version must be a string")
                identity = {
                    "config_version": config_version,
                    "gate": gate,
                    "axis": args.coverage_axis,
                    "review_stage": args.coverage_stage,
                    "fixture_id": fixture_path.stem,
                    "row_key": f"{config_version}|{gate}|{args.coverage_axis}|{args.coverage_stage}|{fixture_path.stem}",
                }
                cases.append(
                    make_case(
                        repository=repository,
                        procedure=procedure,
                        manifest=manifest,
                        profile_path=profile_path,
                        profile=profile,
                        output_root=output_root,
                        identity=identity,
                        fixture_path=fixture_path,
                        companion_path=companion_path,
                        coverage_case=coverage_case,
                        coverage_gate=gate,
                    )
                )
        for case in cases:
            print(f"prepared {case}")
        return 0
    except (OSError, ValueError) as error:
        parser.error(str(error))
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
