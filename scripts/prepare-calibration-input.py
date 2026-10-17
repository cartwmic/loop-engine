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
import re
from pathlib import Path
from typing import Any, Iterable


COVERAGE_CASES = ("sufficient", "related-insufficient", "implementation-defect")
COVERAGE_GATES = ("intent-review", "intent-adversarial-review")
NARRATIVE_PHASES = ("design", "plan", "validation")
# Fixture identities are driver metadata, never rendered as reviewer role labels.
NARRATIVE_PAIRINGS = {
    "design": {"narrative-faithful": ("narrative-101", "narrative-111"),
               "narrative-intent-aligned": ("narrative-101", "narrative-113"),
               "narrative-understandable": ("narrative-101", "narrative-112")},
    "plan": {"narrative-faithful": ("narrative-102", "narrative-121"),
             "narrative-intent-aligned": ("narrative-102", "narrative-123"),
             "narrative-understandable": ("narrative-102", "narrative-122")},
    "validation": {"narrative-faithful": ("narrative-103", "narrative-131"),
                   "narrative-intent-aligned": ("narrative-103", "narrative-133"),
                   "narrative-understandable": ("narrative-103", "narrative-132")},
}


def narrative_pairing(profile: str, phase: str, axis: str) -> tuple[str, str]:
    """Keep the paired universe; isolate validation seeds by profile family."""
    pair = NARRATIVE_PAIRINGS.get(phase, {}).get(axis)
    if pair is None:
        die(f"unsupported narrative pairing {phase}/{axis}")
    if profile not in PROFILE_NAMES:
        die(f"unsupported narrative profile family {profile}")
    if phase == "validation" and axis == "narrative-faithful":
        return pair[0], {"minimal": "narrative-131", "standard": "narrative-133", "high-rigor": "narrative-134"}[profile]
    if phase == "validation" and axis == "narrative-intent-aligned":
        return pair[0], "narrative-133"
    return pair


def narrative_gate(gate: str) -> bool:
    return gate in {phase + suffix for phase in NARRATIVE_PHASES
                    for suffix in ("-narrative-review", "-narrative-adversarial-review")}


def narrative_policies(profile: dict[str, Any]) -> Iterable[tuple[str, dict[str, Any]]]:
    templates = profile.get("extra", {}).get("narrative_review")
    if not isinstance(templates, dict) or set(templates) != {"ordinary", "challenge"}:
        die("profile must contain ordinary and challenge narrative templates")
    for phase in NARRATIVE_PHASES:
        for kind, suffix in (("ordinary", "-narrative-review"), ("challenge", "-narrative-adversarial-review")):
            policies = templates[kind]
            if not isinstance(policies, list) or not policies:
                die("empty narrative template coverage")
            seen = set()
            for policy in policies:
                if not isinstance(policy, dict) or policy.get("id") not in NARRATIVE_PAIRINGS[phase]:
                    die("unsupported narrative template axis")
                stage = policy.get("review_stage")
                key = (policy["id"], stage)
                if stage not in ("individual", "aggregate") or key in seen:
                    die("unsupported or duplicate narrative template stage")
                seen.add(key)
            for stage in {stage for _axis, stage in seen}:
                if any((axis, stage) not in seen for axis in NARRATIVE_PAIRINGS[phase]):
                    die("missing narrative template axis coverage")
            for policy in policies:
                yield phase + suffix, policy


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

PARALLELISM_FIXTURES = (
    ("plan-good", "pass"),
    ("plan-defective", "fail"),
    ("plan-serial-honest", "pass"),
    ("plan-missed-parallelism", "fail"),
    ("plan-collision", "fail"),
    ("plan-oversplit", "fail"),
)


def required_predecessors(gate: str, fixture_id: str) -> tuple[str, ...]:
    if gate in ("plan-review", "plan-adversarial-review") and fixture_id == "plan-serial-honest":
        return ("intent-good", "design-layered")
    return SUBJECTS[gate][2]


IMPLEMENTATION_COMPANION_LABEL = "companion:fictional-repo/implementation-evidence/repository-state.txt"
IMPLEMENTATION_COMPANIONS = {
    "repo-state-2026-08-12": "calibration/companions/fictional-repo/implementation-evidence/repo-state-2026-08-12.txt",
    "repo-state-2026-08-13": "calibration/companions/fictional-repo/implementation-evidence/repo-state-2026-08-13.txt",
}
# Owner-approved preparation-only replacement case selectors; not shipped calibration.
IMPLEMENTATION_COMPANIONS.update({'9bc785ea61812bd63c8641698ffba8162c1a3883': 'calibration/companions/fictional-repo/implementation-evidence/repository-state-replacement-case-01.txt', '1a24892b24a425ff470a6063839e41d4ac68f229': 'calibration/companions/fictional-repo/implementation-evidence/repository-state-replacement-case-02.txt'})
IMPLEMENTATION_COMPANIONS['b39ad2594aad48ba0698f22848f2fb64631c9ea9'] = 'calibration/companions/fictional-repo/implementation-evidence/repository-state-replacement-case-01.txt'
DOC_COMPANIONS = {
    'fictional-repo/implementation-evidence/current-proof-harness/new-current-proof.py',
    'fictional-repo/implementation-evidence/current-proof-harness/proof_support.py',
    'fictional-repo/scripts/assert_new_current_proof.py',
    'fictional-repo/implementation-evidence/current-mechanical-proof.json',
    "fictional-repo/README.md",
    "fictional-repo/provider/README.md",
    "fictional-repo/docs/PRD.md",
    "fictional-repo/docs/review-contract.md",
    "fictional-repo/implementation-evidence/requirement-to-proof.md",
    "fictional-repo/loop-engine-software-change-provider-prd.md",
    "fictional-repo/loop-engine-software-change-provider-task-packets.md",
    "fictional-repo/loop-engine-software-change-provider-technical-design.md",
    "fictional-repo/scripts/assert-doc-authority.py",
    "fictional-repo/scripts/assert-requirement-proof.py",
    "fictional-repo/scripts/production-journey.py",
}
REQUIREMENT_PROOF_COMPANIONS = (
    'fictional-repo/implementation-evidence/current-proof-harness/new-current-proof.py',
    'fictional-repo/implementation-evidence/current-proof-harness/proof_support.py',
    'fictional-repo/scripts/assert_new_current_proof.py',
    'fictional-repo/implementation-evidence/current-mechanical-proof.json',
    "fictional-repo/docs/PRD.md",
    "fictional-repo/implementation-evidence/requirement-to-proof.md",
    "fictional-repo/scripts/assert-requirement-proof.py",
    "fictional-repo/scripts/production-journey.py",
)

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


# ---- Versioned reviewer framing record "locator-line-counts/v1" (non-evidential, derived, deterministic) ----
FRAMING_V1 = "locator-line-counts/v1"
FRAMING_V1_LABEL = "framing:" + FRAMING_V1
FRAMING_V1_DISPLAY = "locator line-count note"
# v1.1 adds ONE fixed, non-derived record after the v1 note (prompt-format clarification of the FILE#Lstart-Lend locator syntax). It is the same bytes in every row.
FRAMING_V11 = FRAMING_V1 + "+locator-syntax/v1.1"
FRAMING_V11_LABEL = "framing:locator-syntax/v1.1"
FRAMING_V11_DISPLAY = "locator syntax requirement"
FRAMING_V11_RECORD = "This framing record is added by the calibration driver (locator-syntax/v1.1), after the locator line-count note. It is not evidence and says nothing about verdicts, criteria or what to cite.\n\nA line-range locator must be written exactly as FILE#Lstart-Lend: the letter L comes before the first number and again before the second number.\nValid:   FILE#L3-L7\nInvalid: FILE#L3-7  (no L before the second number)\nJSON-pointer locators (FILE#/path) are unchanged. The line counts in the previous note are unchanged.\n\nBefore you return your record, check each evidence locator against this syntax and against the line counts, and correct any that do not conform. This check changes nothing else about what you inspect or conclude.\n".encode("utf-8")
FRAMINGS = (FRAMING_V1, FRAMING_V11)
CLASS_CODED_TOKENS = ("good", "defective", "overbuilt", "bad")
EMBEDDED_MARKER = re.compile(
    r"^=== (?:COVERED FILE|ACTUAL EXECUTED EXTERNAL PROOF HARNESS|BASELINE AND COVERED UNCHANGED FILE|COVERED ADDITION; BASELINE ABSENT FILE): ([^;\n]+); sha256:([a-f0-9]{64}) ===\n",
    re.M,
)
NEUTRAL_PREDECESSOR_NAMES = {
    "intent-good": "intent.json", "design-good": "design.json", "plan-good": "plan.json",
    "implementation-report-good": "implementation report.json",
}


def line_count(text: str) -> int:
    """Number of lines exactly as Rust str::lines().count() (the engine's validate_line_selector rule) for any text."""
    return text.count("\n") + (1 if text and not text.endswith("\n") else 0)


def framing_citation_name(label: str, subject: str, template: str) -> str | None:
    """Neutral locator name for a citable source record, or None when the record is not line-citable framing-wise.
    Class-coded canonical fixture labels are never used (see reviewer_display_label)."""
    if label in ("example_prompt", "request-json", FRAMING_V1_LABEL, FRAMING_V11_LABEL) or label.startswith("system-developer-instruction:"):
        return None
    if label.startswith(("reviewer-protocol:", "template:", "schema:data/configs/")):
        # Full data path (never the bare basename: embedded copies of the same basenames make a bare name ambiguous).
        return label.split(":", 1)[1].split("#", 1)[0]
    if label.startswith("subject:"):
        return subject
    if label.startswith("required predecessor:data/calibration/fixtures/"):
        fixture = label.rsplit("/", 1)[-1].removesuffix(".json")
        if fixture not in NEUTRAL_PREDECESSOR_NAMES:
            die(f"locator framing v1 does not define a neutral name for {label}")
        return NEUTRAL_PREDECESSOR_NAMES[fixture]
    if label.startswith("companion:"):
        return label.split(":", 1)[1]
    die(f"locator framing v1 does not define source record label {label}")


def locator_line_count_note(records: list[tuple[str, bytes]], subject: str, template: str) -> bytes:
    """Pure function of the ordered source records that precede the framing record. Same bytes in producer, asserter and consumer."""
    rows: list[tuple[str, str, int]] = []
    seen: set[str] = set()
    for index, (label, content) in enumerate(records):
        name = framing_citation_name(label, subject, template)
        if name is None:
            continue
        text = content.decode("utf-8")
        header = reviewer_display_label(records, index, subject, template)
        rows.append((header, name, line_count(text)))
        for marker in EMBEDDED_MARKER.finditer(text):
            after = text[marker.end():]
            following = re.search(r"^=== ", after, re.M)
            part = after[: following.start()] if following else after
            body = next((part[:-n] if n else part for n in range(5)
                         if hashlib.sha256((part[:-n] if n else part).encode("utf-8")).hexdigest() == marker.group(2)), None)
            if body is None:
                die(f"embedded body hash mismatch in {name}: {marker.group(1)}")
            rows.append(("embedded in " + header, marker.group(1), line_count(body)))
    for header, name, _ in rows:
        for token in CLASS_CODED_TOKENS:
            if token in name.lower() or token in header.lower():
                die(f"class-coded token {token!r} in locator framing name {name!r}")
    names = [name for _, name, _ in rows]
    if len(set(names)) != len(names):
        die("locator framing names are not unique")
    lines = [
        "This framing record is added by the calibration driver (" + FRAMING_V1 + "). It is not evidence and says nothing about verdicts, criteria or what to cite.",
        "",
        "A line-range locator FILE#Lstart-Lend counts the physical lines of the exact body of FILE: 1-based and inclusive; a line ends at a line feed and a final line without one still counts; start must be at least 1 and no greater than end, and end must not exceed the line count listed below. This is the engine's own rule: the number of lines in the source text. JSON-pointer locators are unchanged.",
        "Never counted: the '=== supplied material: ... ===' header line, the blank line after each supplied material and, for a file embedded in a record, the blank lines that follow the embedded body.",
        "",
        "Line counts (supplied material as shown : name used in a locator : lines):",
    ]
    lines += [f"- {header} : {name} : {count} line" + ("" if count == 1 else "s") for header, name, count in rows]
    return ("\n".join(lines) + "\n").encode("utf-8")


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
        FRAMING_V1_LABEL: FRAMING_V1_DISPLAY,
        FRAMING_V11_LABEL: FRAMING_V11_DISPLAY,
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
    die(f"unsupported source record label for reviewer display: {label}")


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
    if narrative_gate(gate):
        matches = [p for g, p in narrative_policies(profile) if g == gate
                   and p.get("id") == axis and p.get("review_stage") == stage]
        if len(matches) != 1 or not isinstance(matches[0].get("example_prompt"), str) or not matches[0]["example_prompt"]:
            die(f"missing or duplicate narrative policy {gate}/{axis}/{stage}")
        return matches[0]["example_prompt"]
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


def selected_validation_evidence(
    repository: Path, subject: dict[str, Any]
) -> tuple[dict[str, Any], list[tuple[str, bytes]]]:
    evidence_ids = subject.get("command_evidence_ids")
    if evidence_ids is None:
        return subject, []
    if not isinstance(evidence_ids, list) or not evidence_ids or not isinstance(evidence_ids[0], str):
        die("validation subject command_evidence_ids must select a supplied narrative")
    suffix = evidence_ids[0].removeprefix("calibration-")
    if suffix not in ("2026-08-12", "2026-08-13"):
        die(f"unsupported selected validation evidence identity: {evidence_ids[0]}")
    path = safe_join(
        repository,
        f"crates/software-change-provider/data/calibration/fixtures/validation-evidence-{suffix}.json",
        "validation evidence companion",
    )
    value = read_json(path, "validation evidence companion")
    if not isinstance(value, dict):
        die(f"validation evidence companion must be an object: {path}")
    return value, [("companion:validation-evidence.json", read_bytes(path, "validation evidence companion"))]


def calibration_companions(
    repository: Path, subject: dict[str, Any], gate: str, axis: str
) -> list[tuple[str, bytes]]:
    selected, records = selected_validation_evidence(repository, subject)
    family = gate.split("-", 1)[0]
    if family == "implementation" or (family == "validation" and axis in ("intent-delivered", "requirement-proof-mapping")):
        coverage = selected.get("coverage")
        commit = coverage.get("commit") if isinstance(coverage, dict) else None
        relative = IMPLEMENTATION_COMPANIONS.get(commit) if isinstance(commit, str) else None
        if relative is None:
            die(f"missing or unknown coverage.commit for {gate}/{axis}: {commit!r}")
        records.append((
            IMPLEMENTATION_COMPANION_LABEL,
            read_bytes(safe_join(repository, f"crates/software-change-provider/data/{relative}", "repository-state companion"), "repository-state companion"),
        ))

    if family == "validation" and axis == "docs-integrated":
        coverage = selected.get("coverage")
        documents = coverage.get("documents") if isinstance(coverage, dict) else None
        if not isinstance(documents, list):
            die("docs-integrated selected validation evidence has no coverage.documents")
        labels: list[str] = []
        for document in documents:
            label = document.get("path") if isinstance(document, dict) else None
            if not isinstance(label, str) or label not in DOC_COMPANIONS:
                die(f"unmapped docs companion path: {label!r}")
            labels.append(label)
        if len(set(labels)) != len(labels):
            die("docs-integrated coverage has duplicate companion paths")
        for label in sorted(labels, key=lambda value: value.encode("utf-8")):
            relative = f"crates/software-change-provider/data/calibration/companions/{label}"
            records.append((f"companion:{label}", read_bytes(safe_join(repository, relative, "docs companion"), "docs companion")))

    if family == "validation" and axis == "requirement-proof-mapping":
        for label in REQUIREMENT_PROOF_COMPANIONS:
            relative = f"crates/software-change-provider/data/calibration/companions/{label}"
            records.append((f"companion:{label}", read_bytes(safe_join(repository, relative, "requirement-proof companion"), "requirement-proof companion")))
    if family == "intent" and axis in ("intent-sufficient", "acceptance-sufficient"):
        labels = []
        if axis == "intent-sufficient":
            labels.append("baseline-2026-08-12.txt")
        if gate == "intent-adversarial-review":
            labels.append("intent-sufficient-comparison.md" if axis == "intent-sufficient" else "acceptance-sufficient-coverage.md")
        for name in labels:
            label = f"fictional-repo/implementation-evidence/{name}"
            records.append((f"companion:{label}", read_bytes(repository / f"crates/software-change-provider/data/calibration/companions/{label}", "sufficiency companion")))
    return records


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
    hash_only: bool = False,
    input_framing: str | None = None,
) -> Path | str:
    profile_basename = profile_name(profile_path)
    config_version = identity["config_version"]
    gate = identity["gate"]
    axis = identity["axis"]
    stage = identity["review_stage"]
    fixture_id = identity["fixture_id"]

    if coverage_case is None:
        selected_row = manifest_row(manifest, identity)
        if selected_row.get("input_framing") != input_framing:
            die(
                f"input framing {input_framing!r} disagrees with the manifest row marker {selected_row.get('input_framing')!r}: "
                + identity["row_key"]
            )
        if profile.get("config_version") != config_version:
            die(
                f"profile config_version {profile.get('config_version')!r} does not match row {config_version!r}"
            )
        if narrative_gate(gate):
            phase = gate.split("-", 1)[0]
            if fixture_id not in narrative_pairing(profile_basename, phase, axis):
                die("fixture does not belong to narrative axis pairing")
            subject, template, predecessors = phase + "-narrative.json", "narrative.md", ()
        elif gate not in SUBJECTS:
            die(f"unsupported calibration gate {gate}")
        else:
            subject, template, _ = SUBJECTS[gate]
            predecessors = required_predecessors(gate, fixture_id)
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
    schema_value = read_json(repository / "crates/software-change-provider/data/narrative-schema.json", "narrative schema") if narrative_gate(gate) else profile.get("artifact_schemas", {}).get(subject)
    if schema_value is None:
        die(f"profile has no artifact schema for {subject}")
    schema_bytes = read_bytes(repository / "crates/software-change-provider/data/narrative-schema.json", "narrative schema") if narrative_gate(gate) else canonical_json(schema_value)

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
    if narrative_gate(gate):
        records = narrative_records(repository, profile, identity, subject_value, records)
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
    if coverage_case is None and not narrative_gate(gate):
        records.extend(calibration_companions(repository, subject_value, gate, axis))
    elif companion_path is not None:
        companion_bytes = read_bytes(companion_path, "companion")
        records.append(("companion:fictional-repo/docs/requirement-coverage.md", companion_bytes))

    if input_framing is not None:
        if input_framing not in FRAMINGS:
            die(f"unsupported input framing {input_framing!r}")
        if coverage_case is not None or narrative_gate(gate):
            die("input framing v1 is defined only for ordinary non-narrative review packets")
        records.append((FRAMING_V1_LABEL, locator_line_count_note(records, subject, template)))
        if input_framing == FRAMING_V11:
            records.append((FRAMING_V11_LABEL, FRAMING_V11_RECORD))
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

    if hash_only:
        return input_hash

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

    # Canonical provenance labels remain in preparation.json and input_sha256;
    # reviewer-facing framing uses only neutral roles and subject names.
    rendered = bytearray()
    for index, (label, content) in enumerate(records):
        display_label = reviewer_display_label(records, index, subject, template)
        rendered.extend(b"=== supplied material: " + display_label.encode("utf-8") + b" ===\n")
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
    if input_framing is not None:
        preparation["input_framing"] = input_framing
    if narrative_gate(gate):
        preparation["output_format"] = "review-worker-batch-v3"
        preparation["challenge_context_provenance"] = "synthetic-corpus-stimulus-not-genuine-review" if gate.endswith("adversarial-review") else "not-supplied"
    preparation.update(coverage_metadata)
    (case_dir / "preparation.json").write_text(
        json.dumps(preparation, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    return case_dir


def narrative_records(repository: Path, profile: dict, identity: dict, binding: dict,
                      records: list[tuple[str, bytes]]) -> list[tuple[str, bytes]]:
    data = repository / "crates/software-change-provider/data"
    phase = identity["gate"].split("-", 1)[0]
    fixture = identity["fixture_id"]
    artifact_id = {"design": "design-good", "plan": "plan-good", "validation": "validation-report-good"}[phase]
    artifact = read_bytes(data / f"calibration/fixtures/{artifact_id}.json", "approved artifact")
    markdown = read_bytes(data / f"calibration/fixtures/{fixture}.md", "narrative markdown")
    if (binding.get("revision") != "r15/1" or binding.get("artifact") != {
            "subject": SUBJECTS[phase + "-review"][0], "revision": "r15",
            "sha256": hashlib.sha256(artifact).hexdigest()}
        or binding.get("markdown") != {"file": phase + "-narrative.md", "sha256": hashlib.sha256(markdown).hexdigest()}
        or binding.get("approved_artifact_record_id") != "approved-" + phase + "-r15"
        or set(binding) != {"revision", "author", "artifact", "markdown", "approved_artifact_record_id"}
        or binding.get("author") != {"name": "narrative-writer", "kind": "agent"}):
        die("invalid narrative binding identity or exact byte hashes")
    assignment = {
        "author": {"name": "calibration-reviewer", "kind": "agent"},
        "gate": identity["gate"], "review_stage": identity["review_stage"],
        "axes": [{"id": identity["axis"], "example_prompt": policy_prompt(profile, identity["gate"], identity["axis"], identity["review_stage"])}],
        "subject": phase + "-narrative.json", "subject_revision": "r15/1",
        "instructions": "Review only supplied bytes; do not resolve fictional-repo labels or live paths. The shipped preamble governs output: return its batch-v3 object for this one axis and stage, not the older calibration instruction's single review-evidence record. No validation_verdicts: this is narrative review. Use supplied role/file names for locators. Ordinary context when supplied is a controlled corpus stimulus, not actual live-gate fulfillment. Never infer a verdict from its existence.",
    }
    # Existing-key streams retain their original instruction and schema records.
    records.insert(1, ("review-worker-preamble:data/review-worker-preamble.txt", read_bytes(data / "review-worker-preamble.txt", "preamble")))
    records.insert(2, ("assignment-json", canonical_json(assignment)))
    records[6] = ("schema:data/narrative-schema.json", read_bytes(data / "narrative-schema.json", "narrative schema"))
    records.extend([("markdown", markdown), ("approved-artifact", artifact),
                    ("frozen-intent", read_bytes(data / "calibration/fixtures/intent-good.json", "frozen intent"))])
    for predecessor in required_narrative_predecessors(phase):
        records.append((f"required predecessor:data/calibration/fixtures/{predecessor}.json", read_bytes(data / f"calibration/fixtures/{predecessor}.json", "predecessor")))
    if phase == "validation":
        records.extend(calibration_companions(repository, read_json(data / f"calibration/fixtures/{artifact_id}.json", "approved artifact"), "validation-review", "intent-delivered"))
        records.extend(requirement_companions_for_narrative(repository))
    if identity["gate"].endswith("adversarial-review"):
        records.append(("challenge-context", read_bytes(data / f"calibration/fixtures/{phase}-narrative-context-{identity['axis']}.json", "ordinary context")))
    return records


def required_narrative_predecessors(phase: str) -> tuple[str, ...]:
    return {"design": (), "plan": ("design-good",), "validation": ("design-good", "plan-good", "implementation-report-good")}[phase]


def requirement_companions_for_narrative(repository: Path) -> list[tuple[str, bytes]]:
    return [(f"companion:{label}", read_bytes(repository / f"crates/software-change-provider/data/calibration/companions/{label}", "proof companion")) for label in REQUIREMENT_PROOF_COMPANIONS]


def prepare_successor_manifest(repository: Path, manifest: list, procedure: bytes) -> list:
    """Derive complete successor identities, never observations/applicability.

    An attested current row is kept only when its recorded input hash equals the
    freshly derived packet hash; otherwise preparation stops for review."""
    key_of = lambda row: tuple(row[k] for k in ("config_version", "gate", "axis", "review_stage", "fixture_id"))
    output = [row for row in manifest if not row["config_version"].endswith("-15")]
    existing = {key_of(row): row for row in manifest if row["config_version"].endswith("-15")}
    def keep_or_new(row: dict) -> dict:
        prior = existing.get(key_of(row))
        if prior and prior["observed"] != "pending":
            if prior["input_sha256"] != row["input_sha256"]:
                die("attested current row has changed inputs: " + "|".join(key_of(row)))
            return prior
        return row
    for name in PROFILE_NAMES:
        path = repository / f"crates/software-change-provider/data/configs/{name}.json"
        profile = read_json(path, "successor profile")
        if profile["config_version"] != name + "-15": die("successor preparation requires v15 profiles")
        for gate, policies in profile["review_policies"].items():
            for policy in policies:
                sources = [row for row in manifest if row["config_version"] == name + "-14"
                           and row["gate"] == gate and row["axis"] == policy["id"]
                           and row["review_stage"] == policy["review_stage"]]
                if policy["id"] in ("intent-sufficient", "acceptance-sufficient"):
                    sources = [dict(fixture_id=policy["id"] + "-" + quality, gate=gate, axis=policy["id"], review_stage=policy["review_stage"], expected=result) for quality, result in (("good", "pass"), ("defective", "fail"))]
                if policy["id"] == "parallelism-honest":
                    sources = [dict(fixture_id=fixture, gate=gate, axis=policy["id"], review_stage=policy["review_stage"], expected=result) for fixture, result in PARALLELISM_FIXTURES]
                if not sources: die(f"missing predecessor fixtures for {name}/{gate}/{policy['id']}")
                for source in sources:
                    output.append(keep_or_new(pending_successor(repository, procedure, path, profile, source)))
        for gate, policy in narrative_policies(profile):
            phase = gate.split("-", 1)[0]
            for fixture, result in zip(narrative_pairing(name, phase, policy["id"]), ("pass", "fail")):
                output.append(keep_or_new(pending_successor(repository, procedure, path, profile,
                    dict(fixture_id=fixture, gate=gate, axis=policy["id"], review_stage=policy["review_stage"], expected=result))))
    if not output or len({key_of(row) for row in output}) != len(output):
        die("empty or duplicate calibration universe")
    return output


def pending_successor(repository: Path, procedure: bytes, path: Path, profile: dict, source: dict) -> dict:
    row = {k: source[k] for k in ("fixture_id", "gate", "axis", "review_stage", "expected")}
    row.update(config_version=profile["config_version"], observed="pending", model="", attested_by="",
               invocation="Fresh review pending: mechanical rehash complete; owner must perform exact fresh review and attest returned evidence before green calibration.")
    key = "|".join(row[k] for k in ("config_version", "gate", "axis", "review_stage", "fixture_id"))
    row["input_sha256"] = make_case(repository=repository, procedure=procedure, manifest=[row],
        profile_path=path, profile=profile, output_root=repository, identity=parse_row_key(key), hash_only=True)
    return row


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--procedure", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--profile", type=Path)
    parser.add_argument("--prepare-successor-manifest", action="store_true")
    parser.add_argument("--repository", type=Path, required=True)
    parser.add_argument("--output-root", type=Path, required=True)
    parser.add_argument("--row-key", action="append", default=[])
    parser.add_argument("--coverage-case", action="append", choices=COVERAGE_CASES, default=[])
    parser.add_argument("--coverage-gate", choices=COVERAGE_GATES)
    parser.add_argument("--coverage-axis", default="ids-grounded")
    parser.add_argument("--coverage-stage", default="aggregate")
    parser.add_argument("--fixture", type=Path)
    parser.add_argument("--input-framing", choices=FRAMINGS, default=None)
    parser.add_argument("--companion", type=Path)
    return parser


def main() -> int:
    parser = build_parser()
    args = parser.parse_args()
    try:
        repo = args.repository.resolve(strict=True)
        dest = args.output_root.resolve()
        if dest == repo or repo in dest.parents:
            die("packet output root must be isolated outside the checkout")
        if args.prepare_successor_manifest:
            if args.row_key or args.coverage_case or args.profile: die("successor manifest preparation takes no row/coverage/profile selection")
            rows = prepare_successor_manifest(args.repository.resolve(), read_json(args.manifest, "manifest"), read_bytes(args.procedure, "procedure"))
            args.output_root.mkdir(parents=True, exist_ok=False)
            (args.output_root / "manifest.json").write_text(json.dumps(rows, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
            print(f"prepared complete successor manifest with pending uncovered rows: {args.output_root / 'manifest.json'}")
            return 0
        if args.profile is None: die("packet preparation requires --profile")
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
                        input_framing=args.input_framing,
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
        inventory = [{"row_key": read_json(case / "preparation.json", "preparation")["row_key"],
                      "input_sha256": read_json(case / "preparation.json", "preparation")["input_sha256"],
                      "instructions_path": str(case / "instructions.txt"),
                      "preparation_path": str(case / "preparation.json"),
                      "source_paths": [str(case / record["path"]) for record in read_json(case / "preparation.json", "preparation")["source_records"]]}
                     for case in cases]
        inventory_path = output_root / ("dispatch-" + hashlib.sha256("|".join(item["row_key"] for item in inventory).encode()).hexdigest()[:16] + ".json")
        if inventory_path.exists(): die("refusing to replace dispatch inventory")
        inventory_path.write_text(json.dumps({"cases": inventory}, indent=2) + "\n", encoding="utf-8")
        for case in cases:
            print(f"prepared {case}")
        return 0
    except (OSError, ValueError) as error:
        parser.error(str(error))
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
