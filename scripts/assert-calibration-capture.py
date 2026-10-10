#!/usr/bin/env python3
"""Read-only verification for retained external calibration captures.

This consumer never starts a model and never edits preparation, receipt, or
manifest files. It verifies mechanical source identity, capture linkage, raw
returned output, and pending owner-attestation state.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
from typing import Any


COVERAGE_CASES = ("sufficient", "related-insufficient", "implementation-defect")
COVERAGE_GATES = ("intent-review", "intent-adversarial-review")
FORBIDDEN_METADATA_KEYS = {"expected", "observed", "oracle", "fixture_class", "seed_label", "expected_result"}
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


def fail(message: str) -> "NoReturn":
    raise ValueError(message)


def read_json(path: Path, description: str) -> Any:
    try:
        return json.loads(path.read_bytes())
    except OSError as error:
        fail(f"could not read {description} {path}: {error}")
    except json.JSONDecodeError as error:
        fail(f"invalid JSON in {description} {path}: {error}")


def read_bytes(path: Path, description: str) -> bytes:
    try:
        return path.read_bytes()
    except OSError as error:
        fail(f"could not read {description} {path}: {error}")


def reject_oracle_keys(value: Any, location: str) -> None:
    if isinstance(value, dict):
        for key, child in value.items():
            if key in FORBIDDEN_METADATA_KEYS:
                fail(f"oracle metadata key {key!r} in {location}")
            reject_oracle_keys(child, f"{location}.{key}")
    elif isinstance(value, list):
        for index, child in enumerate(value):
            reject_oracle_keys(child, f"{location}[{index}]")


def framed_digest(records: list[tuple[str, bytes]]) -> str:
    stream = bytearray()
    stream.extend(len(records).to_bytes(8, "big"))
    for label, content in records:
        label_bytes = label.encode("utf-8")
        stream.extend(len(label_bytes).to_bytes(8, "big"))
        stream.extend(label_bytes)
        stream.extend(len(content).to_bytes(8, "big"))
        stream.extend(content)
    return hashlib.sha256(stream).hexdigest()


def reviewer_display_label(
    records: list[tuple[str, bytes]], index: int, subject: str, template: str
) -> str:
    """Render a neutral role label without exposing canonical fixture labels."""
    label = records[index][0]
    fixed_labels = {
        "system-developer-instruction:data/calibration/reviewer-instruction.txt": "reviewer instruction",
        "example_prompt": "selected review prompt",
        "reviewer-protocol:data/reviewer-protocol.md": "review protocol",
        "request-json": "canonical request JSON",
        "review-worker-preamble:data/review-worker-preamble.txt": "shipped review-worker preamble",
        "assignment-json": "frozen narrative assignment",
        "schema:data/narrative-schema.json": "narrative binding schema",
        "markdown": f"narrative markdown: {subject.removesuffix('.json')}.md",
        "approved-artifact": "exact approved artifact",
        "frozen-intent": "frozen intent: intent.json",
        "challenge-context": "ordinary review and cleared finding dispositions",
    }
    if label in fixed_labels:
        return fixed_labels[label]
    if label.startswith("template:"):
        return f"artifact template: {template}"
    if label.startswith("schema:"):
        return f"artifact schema: {subject}"
    if label.startswith("subject:"):
        return f"subject artifact: {subject}"
    predecessor_labels = {
        "required predecessor:data/calibration/fixtures/intent-good.json": "required predecessor: intent",
        "required predecessor:data/calibration/fixtures/design-good.json": "required predecessor: design",
        "required predecessor:data/calibration/fixtures/design-layered.json": "required predecessor design",
        "required predecessor:data/calibration/fixtures/plan-good.json": "required predecessor: plan",
        "required predecessor:data/calibration/fixtures/implementation-report-good.json": "required predecessor: implementation report",
    }
    if label in predecessor_labels:
        return predecessor_labels[label]
    companion_labels = {
        "companion:validation-evidence.json": "validation evidence companion",
        "companion:fictional-repo/implementation-evidence/repository-state.txt": "implementation evidence companion",
        "companion:fictional-repo/docs/requirement-coverage.md": "requirement coverage companion",
    }
    if label in companion_labels:
        return companion_labels[label]
    if label.startswith("companion:fictional-repo/"):
        ordinal = sum(
            1
            for prior, _content in records[:index]
            if prior.startswith("companion:fictional-repo/")
            and prior not in companion_labels
        ) + 1
        return f"supporting document {ordinal}"
    fail(f"unsupported source record label for reviewer display: {label}")


def contained(root: Path, path: Path, description: str) -> Path:
    resolved = path.resolve()
    if resolved != root and root not in resolved.parents:
        fail(f"{description} escapes capture root: {path}")
    return resolved


def parse_row_key(value: str) -> dict[str, str]:
    fields = value.split("|")
    if len(fields) != 5 or any(not field for field in fields):
        fail(f"invalid row key: {value}")
    return dict(
        zip(
            ("config_version", "gate", "axis", "review_stage", "fixture_id"),
            fields,
        )
    ) | {"row_key": value}


def expected_specs(args: argparse.Namespace) -> list[dict[str, str]]:
    specs = [parse_row_key(value) for value in args.row_key]
    for case in args.coverage_case:
        for gate in COVERAGE_GATES:
            config = "high-rigor-15"
            fixture = f"requirement-coverage-{case}"
            specs.append(
                {
                    "config_version": config,
                    "gate": gate,
                    "axis": "ids-grounded",
                    "review_stage": "aggregate",
                    "fixture_id": fixture,
                    "row_key": f"{config}|{gate}|ids-grounded|aggregate|{fixture}",
                    "coverage_case": case,
                    "coverage_gate": gate,
                }
            )
    if not specs:
        fail("supply at least one --row-key or --coverage-case")
    if len({spec["row_key"] for spec in specs}) != len(specs):
        fail("duplicate requested calibration case")
    return specs


def preparation_for(root: Path, spec: dict[str, str]) -> tuple[Path, dict[str, Any]]:
    candidates = []
    for child in root.iterdir():
        if not child.is_dir():
            continue
        path = child / "preparation.json"
        if not path.is_file():
            continue
        metadata = read_json(path, "preparation")
        if isinstance(metadata, dict) and metadata.get("row_key") == spec["row_key"]:
            candidates.append((child, metadata))
    if len(candidates) != 1:
        fail(
            f"expected one preparation for {spec['row_key']}, found {len(candidates)}"
        )
    return candidates[0]


NARRATIVE_PREDECESSORS = {
    # Mirrors prepare-calibration-input.py required_narrative_predecessors; the
    # frozen intent is supplied as its own "frozen-intent" record, not a predecessor.
    "design": (),
    "plan": ("design-good",),
    "validation": ("design-good", "plan-good", "implementation-report-good"),
}


def required_predecessors(gate: str, fixture_id: str) -> tuple[str, ...]:
    if "-narrative-" in gate:
        phase = gate.split("-narrative-", 1)[0]
        if phase not in NARRATIVE_PREDECESSORS:
            fail(f"unsupported narrative calibration gate: {gate}")
        return NARRATIVE_PREDECESSORS[phase]
    family = gate.split("-", 1)[0]
    predecessors = {
        "intent": (),
        "design": ("intent-good",),
        "plan": ("intent-good", "design-good"),
        "implementation": ("intent-good", "design-good", "plan-good"),
        "validation": ("intent-good", "design-good", "plan-good", "implementation-report-good"),
    }
    if family == "plan" and fixture_id == "plan-serial-honest":
        return ("intent-good", "design-layered")
    if family not in predecessors:
        fail(f"unsupported calibration gate: {gate}")
    return predecessors[family]


def verify_preparation(root: Path, case_dir: Path, metadata: dict[str, Any], spec: dict[str, str]) -> dict[str, Any]:
    reject_oracle_keys(metadata, str(case_dir / "preparation.json"))
    allowed_metadata = {
        "schema_version", "kind", "row_key", "config_version", "gate", "policy_id",
        "review_stage", "subject", "subject_revision", "fixture_id", "profile", "template",
        "source_records", "request_path", "instructions_path", "input_sha256", "prompt_sha256",
        "instructions_sha256", "coverage_case", "coverage_gate", "output_format",
        "challenge_context_provenance",
    }
    if set(metadata) - allowed_metadata:
        fail(f"unsupported preparation metadata fields: {case_dir}")
    for field in (
        "schema_version",
        "kind",
        "row_key",
        "config_version",
        "gate",
        "policy_id",
        "review_stage",
        "subject",
        "subject_revision",
        "fixture_id",
        "profile",
        "source_records",
        "input_sha256",
        "prompt_sha256",
        "instructions_sha256",
    ):
        if field not in metadata:
            fail(f"preparation missing {field}: {case_dir}")
    for field in ("config_version", "gate", "policy_id", "review_stage", "fixture_id"):
        expected = {
            "config_version": spec["config_version"],
            "gate": spec["gate"],
            "policy_id": spec["axis"],
            "review_stage": spec["review_stage"],
            "fixture_id": spec["fixture_id"],
        }[field]
        if metadata[field] != expected:
            fail(f"preparation {field} mismatch for {spec['row_key']}")
    if metadata["row_key"] != spec["row_key"]:
        fail(f"preparation row_key mismatch: {case_dir}")
    records_meta = metadata["source_records"]
    if not isinstance(records_meta, list) or not records_meta:
        fail(f"preparation source_records must be a non-empty list: {case_dir}")
    records: list[tuple[str, bytes]] = []
    for index, record in enumerate(records_meta):
        if not isinstance(record, dict):
            fail(f"source record {index} is not an object: {case_dir}")
        for field in ("label", "path", "byte_length", "sha256"):
            if field not in record:
                fail(f"source record {index} missing {field}: {case_dir}")
        relative = record["path"]
        if not isinstance(relative, str) or Path(relative).is_absolute():
            fail(f"source record path is not relative: {case_dir}")
        path = contained(case_dir, case_dir / relative, "source record")
        content = read_bytes(path, "source record")
        if len(content) != record["byte_length"]:
            fail(f"source record byte length mismatch: {path}")
        digest = hashlib.sha256(content).hexdigest()
        if digest != record["sha256"]:
            fail(f"source record sha256 mismatch: {path}")
        label = record["label"]
        if not isinstance(label, str) or not label:
            fail(f"source record label is not non-empty: {path}")
        records.append((label, content))
    predecessor_labels = [label for label, _ in records if label.startswith("required predecessor:")]
    expected_predecessor_labels = [
        f"required predecessor:data/calibration/fixtures/{fixture}.json"
        for fixture in required_predecessors(spec["gate"], spec["fixture_id"])
    ]
    if predecessor_labels != expected_predecessor_labels:
        fail(f"required predecessor selection mismatch: {case_dir}")
    if records[0][0] != "system-developer-instruction:data/calibration/reviewer-instruction.txt":
        fail(f"reviewer instruction is not first source record: {case_dir}")
    if records[-1][0] != "request-json":
        fail(f"request-json is not final source record: {case_dir}")
    input_hash = framed_digest(records)
    if input_hash != metadata["input_sha256"]:
        fail(f"mechanical input_sha256 mismatch: {case_dir}")
    hash_file = read_bytes(case_dir / "input_sha256", "input_sha256")
    if hash_file != (input_hash + "\n").encode("ascii"):
        fail(f"input_sha256 file mismatch: {case_dir}")
    request = read_bytes(case_dir / "request.json", "request")
    if request != records[-1][1] or request.endswith(b"\n"):
        fail(f"canonical request bytes mismatch: {case_dir}")
    instructions = read_bytes(case_dir / "instructions.txt", "instructions")
    if hashlib.sha256(instructions).hexdigest() != metadata["instructions_sha256"]:
        fail(f"instructions_sha256 mismatch: {case_dir}")
    expected_instructions = bytearray()
    for index, (_label, content) in enumerate(records):
        display_label = reviewer_display_label(
            records, index, str(metadata["subject"]), str(metadata["template"])
        )
        expected_instructions.extend(
            b"=== supplied material: " + display_label.encode("utf-8") + b" ===\n"
        )
        expected_instructions.extend(content)
        if not content.endswith(b"\n"):
            expected_instructions.extend(b"\n")
        expected_instructions.extend(b"\n")
    if instructions != bytes(expected_instructions):
        fail(f"instructions do not preserve the prepared source framing: {case_dir}")
    prompt = next((content for label, content in records if label == "example_prompt"), None)
    if prompt is None or hashlib.sha256(prompt).hexdigest() != metadata["prompt_sha256"]:
        fail(f"preparation prompt hash does not match its source record: {case_dir}")
    if spec.get("coverage_case"):
        expected_prompt = (
            ORDINARY_COVERAGE_PROMPT
            if spec["coverage_gate"] == "intent-review"
            else CHALLENGE_COVERAGE_PROMPT
        ).encode("utf-8")
        if prompt != expected_prompt:
            fail(f"coverage packet has wrong or mixed rubric prompt: {case_dir}")
        if sum(label == "example_prompt" for label, _ in records) != 1:
            fail(f"coverage packet must contain exactly one rubric: {case_dir}")
    if "-narrative-" in spec["gate"]:
        verify_current_narrative(metadata, spec)
    return {"records": records, "instructions": instructions}


def producer_module():
    import importlib.util
    path = Path(__file__).with_name("prepare-calibration-input.py")
    spec = importlib.util.spec_from_file_location("calibration_producer", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def verify_current_narrative(metadata: dict, spec: dict) -> None:
    producer = producer_module()
    repository = Path(__file__).resolve().parent.parent
    data = repository / "crates/software-change-provider/data"
    profile_path = data / f"configs/{metadata['profile']}.json"
    identity = producer.parse_row_key(spec["row_key"])
    manifest = read_json(data / "calibration/manifest.json", "manifest")
    row = producer.manifest_row(manifest, identity)
    current_hash = producer.make_case(repository=repository,
        procedure=(data / "calibration/PROCEDURE.md").read_bytes(),
        manifest=manifest,
        profile_path=profile_path, profile=read_json(profile_path, "profile"),
        output_root=repository, identity=identity, hash_only=True)
    if current_hash != metadata["input_sha256"] or current_hash != row.get("input_sha256"):
        fail("narrative packet differs from actual current shipped sources or selected manifest row")
    if metadata.get("output_format") != "review-worker-batch-v3":
        fail("narrative packet missing explicit shipped output contract")
    provenance = "synthetic-corpus-stimulus-not-genuine-review" if spec["gate"].endswith("adversarial-review") else "not-supplied"
    if metadata.get("challenge_context_provenance") != provenance:
        fail("narrative challenge context provenance mismatch")


def narrative_judgment(returned: bytes, prep: dict) -> dict:
    """Select the actual raw batch judgment; do not project a fabricated record."""
    value = json.loads(returned)
    if (not isinstance(value, dict) or set(value) != {"review_contract_version", "review_stage", "author", "judgments"}
        or value["review_contract_version"] != 3 or value["review_stage"] != prep["review_stage"]
        or value["author"] != {"name": "calibration-reviewer", "kind": "agent"}):
        fail("narrative raw batch contract/assignment mismatch")
    rows = value["judgments"]
    if not isinstance(rows, list) or len(rows) != 1:
        fail("narrative raw batch must contain the one assigned axis")
    row = rows[0]
    if (not isinstance(row, dict) or set(row) != {"axis", "result", "findings", "grounds"}
        or row["axis"] != prep["policy_id"] or row["result"] not in ("pass", "fail")
        or not isinstance(row["findings"], str)
        or (row["result"] == "pass" and row["findings"] != "")
        or (row["result"] == "fail" and not row["findings"].strip())):
        fail("narrative raw judgment identity/result/findings invalid")
    grounds = row["grounds"]
    if (not isinstance(grounds, dict) or set(grounds) != {"reason", "evidence"}
        or not isinstance(grounds["reason"], str) or not grounds["reason"].strip()
        or not isinstance(grounds["evidence"], list) or not grounds["evidence"]
        or any(not isinstance(e, dict) or set(e) != {"locator"} or not isinstance(e["locator"], str) or not e["locator"].strip() for e in grounds["evidence"])):
        fail("narrative raw judgment grounds incomplete")
    return row


def receipt_candidates(root: Path) -> list[tuple[Path, dict[str, Any]]]:
    candidates = []
    for index_path in sorted(root.glob("driver-capture-index-*.json"), key=lambda p: p.stat().st_mtime, reverse=True):
        index = read_json(index_path, "driver capture index")
        reject_oracle_keys(index, str(index_path))
        if not isinstance(index, dict) or index.get("owner_attestation") != "pending":
            continue
        cases = index.get("cases")
        if not isinstance(cases, list):
            fail(f"capture index cases must be a list: {index_path}")
        for item in cases:
            if not isinstance(item, dict) or not isinstance(item.get("receipt"), str):
                fail(f"capture index has malformed receipt locator: {index_path}")
            receipt_path = Path(item["receipt"])
            if not receipt_path.is_absolute():
                receipt_path = index_path.parent / receipt_path
            receipt_path = contained(root, receipt_path, "capture receipt")
            receipt = read_json(receipt_path, "capture receipt")
            if not isinstance(receipt, dict):
                fail(f"capture receipt must be an object: {receipt_path}")
            reject_oracle_keys(receipt, str(receipt_path))
            candidates.append((receipt_path, receipt))
    return candidates


def verify_receipt(
    root: Path,
    prep_dir: Path,
    prep: dict[str, Any],
    receipt_path: Path,
    receipt: dict[str, Any],
) -> dict[str, Any]:
    if receipt.get("owner_attestation") != "pending":
        fail(f"owner attestation is not pending: {receipt_path}")
    if receipt.get("semantic_disposition") != "pending-driver-inspection":
        fail(f"semantic disposition is not pending-driver-inspection: {receipt_path}")
    if receipt.get("exit_code") != 0:
        fail(f"capture did not complete successfully: {receipt_path}")
    if receipt.get("instructions_sha256") != prep["instructions_sha256"]:
        fail(f"receipt instructions hash does not match preparation: {receipt_path}")
    command = receipt.get("command")
    if not isinstance(command, list) or not all(isinstance(arg, str) for arg in command):
        fail(f"capture command is not an argv array: {receipt_path}")
    if len(command) < 2 or command[1] != "fan-out" or command.count("--worker") != 1:
        fail(f"capture must declare exactly one fan-out worker: {receipt_path}")
    try:
        declared_worker = json.loads(command[command.index("--worker") + 1])
    except (IndexError, ValueError):
        fail(f"capture has invalid nested worker JSON: {receipt_path}")
    if not isinstance(declared_worker, dict) or set(declared_worker) != {"command", "args"}:
        fail(f"capture has invalid nested worker fields: {receipt_path}")
    worker_args = declared_worker["args"]
    if not isinstance(worker_args, list) or not all(isinstance(arg, str) for arg in worker_args):
        fail(f"nested worker args are not strings: {receipt_path}")
    if "--no-context-files" not in worker_args:
        fail(f"capture worker did not disable inherited context files: {receipt_path}")
    for flag, expected in (("--tools", ""), ("--model", receipt.get("model")), ("--thinking", receipt.get("thinking"))):
        if worker_args.count(flag) != 1:
            fail(f"capture worker must declare {flag} once: {receipt_path}")
        position = worker_args.index(flag) + 1
        if position >= len(worker_args) or worker_args[position] != expected:
            fail(f"capture worker {flag} differs from required metadata: {receipt_path}")
    attempt_dir = receipt_path.parent
    raw_stdout = read_bytes(attempt_dir / "stdout", "raw facade stdout")
    raw_stderr = read_bytes(attempt_dir / "stderr", "raw facade stderr")
    if not raw_stdout:
        fail(f"raw facade stdout is empty: {receipt_path}")
    returned_path = attempt_dir / "returned-output"
    returned = read_bytes(returned_path, "returned output")
    if not returned.strip():
        fail(f"returned output is empty: {receipt_path}")
    if receipt.get("returned_output_sha256") != hashlib.sha256(returned).hexdigest():
        fail(f"returned output hash mismatch: {receipt_path}")
    summary = json.loads(raw_stdout)
    workers = summary.get("workers") if isinstance(summary, dict) else None
    if not isinstance(workers, list) or len(workers) != 1 or not isinstance(workers[0], dict):
        fail(f"facade summary must contain exactly one captured worker: {receipt_path}")
    worker = workers[0]
    if worker.get("exit_code") != 0:
        fail(f"captured worker failed: {receipt_path}")
    if any(worker.get(key) != declared_worker[key] for key in ("command", "args")):
        fail(f"captured worker differs from declared command/args: {receipt_path}")
    for field, summary_field in (("captured_stdout", "stdout_path"), ("captured_stderr", "stderr_path")):
        value = receipt.get(field)
        if not isinstance(value, str) or not Path(value).is_absolute():
            fail(f"receipt {field} must be an absolute raw capture path: {receipt_path}")
        if value != worker.get(summary_field):
            fail(f"receipt {field} differs from actual worker capture: {receipt_path}")
        content = read_bytes(Path(value), field)
        if field == "captured_stdout" and content != returned:
            fail(f"returned judgment differs from captured worker stdout: {receipt_path}")
    if "-narrative-" in prep["gate"]:
        narrative_judgment(returned, prep)
    # Preserve the facade's own raw streams and the worker's selected output;
    # this check never parses or changes the returned judgment.
    return {
        "receipt": str(receipt_path),
        "instructions_sha256": prep["instructions_sha256"],
        "input_sha256": prep["input_sha256"],
        "returned_output_sha256": receipt["returned_output_sha256"],
        "raw_stdout_bytes": len(raw_stdout),
        "raw_stderr_bytes": len(raw_stderr),
        "returned_output": str(returned_path),
        "owner_attestation": receipt["owner_attestation"],
        "semantic_disposition": receipt["semantic_disposition"],
    }


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--preparation-only", action="store_true", help="Verify supplied packets without requiring or inventing reviewer returns")
    parser.add_argument("--row-key", action="append", default=[])
    parser.add_argument("--coverage-case", action="append", choices=COVERAGE_CASES, default=[])
    return parser


def main() -> int:
    parser = build_parser()
    args = parser.parse_args()
    try:
        root = args.root.resolve(strict=True)
        specs = expected_specs(args)
        preparations = []
        for spec in specs:
            case_dir, metadata = preparation_for(root, spec)
            verify_preparation(root, case_dir, metadata, spec)
            preparations.append((spec, case_dir, metadata))
        if args.preparation_only:
            print(json.dumps({"status": "prepared-only-observations-pending", "cases": [
                {"row_key": spec["row_key"], "input_sha256": metadata["input_sha256"], "instructions_sha256": metadata["instructions_sha256"]}
                for spec, _directory, metadata in preparations]}, indent=2))
            return 0
        receipts = receipt_candidates(root)
        if not receipts:
            fail(f"no pending driver capture indexes found under {root}")
        used: set[Path] = set()
        results = []
        for spec, case_dir, metadata in preparations:
            matches = [
                (path, receipt)
                for path, receipt in receipts
                if path not in used
                and receipt.get("instructions_sha256") == metadata["instructions_sha256"]
                and receipt.get("case") in (case_dir.name, spec["row_key"])
            ]
            if len(matches) != 1:
                fail(
                    f"expected one retained capture for {spec['row_key']}, found {len(matches)}"
                )
            receipt_path, receipt = matches[0]
            used.add(receipt_path)
            result = verify_receipt(root, case_dir, metadata, receipt_path, receipt)
            result.update(
                {
                    "row_key": spec["row_key"],
                    "case_directory": str(case_dir),
                    "gate": spec["gate"],
                    "policy_id": spec["axis"],
                    "review_stage": spec["review_stage"],
                }
            )
            results.append(result)
        print(json.dumps({"status": "verified", "cases": results}, indent=2))
        return 0
    except (OSError, ValueError) as error:
        parser.error(str(error))
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
