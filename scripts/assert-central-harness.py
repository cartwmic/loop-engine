#!/usr/bin/env python3
"""Validate the detailed T02 central-harness handoff.

The baseline remains the authority for the predecessor source and named-case
inventory.  This checker verifies the final Cargo target, current test names,
exact source adaptations, and preservation inventories instead of accepting an
aggregate count supplied by the driver.
"""

from __future__ import annotations

import argparse
import difflib
import json
import subprocess
import sys
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parent))
from test_contract import (  # noqa: E402
    ContractError,
    REQUIRED_BINARIES,
    binary_targets,
    cargo_metadata,
    compiler_artifact_targets,
    extracted_inventories,
    integration_targets,
    inventory_manifest_errors,
    load_json,
    load_test_lists,
    sha256_bytes,
    sha256_file,
    source_test_inventory,
    topology_errors,
)


def baseline_source_bytes(repo: Path, baseline: dict[str, Any], path: str) -> bytes:
    revision = baseline.get("repository", {}).get("baseline_git_revision")
    if not isinstance(revision, str) or not revision:
        revision = baseline.get("baseline_git_revision")
    if not isinstance(revision, str) or not revision:
        raise ContractError("baseline lacks baseline_git_revision")
    result = subprocess.run(
        ["git", "show", f"{revision}:{path}"],
        cwd=repo,
        capture_output=True,
        check=False,
    )
    if result.returncode:
        raise ContractError(
            f"could not read baseline source {path} at {revision}: "
            + result.stderr.decode("utf-8", "replace").strip()
        )
    return result.stdout


def exact_diff(old: bytes, new: bytes, old_path: str, new_path: str) -> str:
    try:
        old_lines = old.decode("utf-8").splitlines(keepends=True)
        new_lines = new.decode("utf-8").splitlines(keepends=True)
    except UnicodeDecodeError as error:
        raise ContractError(f"source adaptation is not UTF-8: {error}") from error
    return "".join(
        difflib.unified_diff(
            old_lines,
            new_lines,
            fromfile=old_path,
            tofile=new_path,
            lineterm="\n",
        )
    )


def inventory_signature(inventory: dict[str, Any], category: str) -> str:
    section = inventory.get(category)
    if not isinstance(section, dict) or not isinstance(section.get("entries"), list):
        raise ContractError(f"preservation inventory lacks {category}.entries")
    fields = ("source", "kind", "match") if category != "bookends_citations" else ("source", "match")
    normalized = [
        tuple(entry.get(field) for field in fields)
        for entry in section["entries"]
        if isinstance(entry, dict)
    ]
    normalized.sort(key=lambda entry: tuple(str(value) for value in entry))
    return sha256_bytes(
        json.dumps(normalized, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
    )


def source_mapping_errors(repo: Path, baseline: dict[str, Any], manifest: dict[str, Any]) -> list[str]:
    integration = baseline.get("integration")
    baseline_sources = integration.get("source_files") if isinstance(integration, dict) else None
    mappings = manifest.get("source_mappings")
    if not isinstance(baseline_sources, list) or not all(isinstance(item, dict) for item in baseline_sources):
        return ["baseline integration source_files must be an array of objects"]
    if not isinstance(mappings, list) or not all(isinstance(item, dict) for item in mappings):
        return ["central harness source_mappings must be an array of objects"]

    expected = {str(item.get("path")) for item in baseline_sources}
    actual = [item.get("baseline_path") for item in mappings]
    errors: list[str] = []
    if not all(isinstance(path, str) for path in actual):
        return ["central harness source mappings must have string baseline_path values"]
    if len(actual) != len(set(actual)):
        errors.append("central harness source mappings contain duplicate baseline paths")
    missing = sorted(expected - set(actual))
    extra = sorted(set(actual) - expected)
    if missing:
        errors.append("central harness missing source mappings: " + ", ".join(missing))
    if extra:
        errors.append("central harness has unexpected source mappings: " + ", ".join(extra))

    baseline_by_path = {str(item["path"]): item for item in baseline_sources}
    for mapping in mappings:
        path = mapping.get("baseline_path")
        if not isinstance(path, str) or path not in baseline_by_path:
            continue
        final_path = mapping.get("final_path")
        if not isinstance(final_path, str) or not final_path:
            errors.append(f"central harness mapping {path} lacks final_path")
            continue
        final_file = (repo / final_path).resolve()
        try:
            final_file.relative_to(repo.resolve())
        except ValueError:
            errors.append(f"central harness final path escapes repository: {final_path}")
            continue
        if not final_file.is_file():
            errors.append(f"central harness final source is missing: {final_path}")
            continue
        old_sha = baseline_by_path[path].get("sha256")
        new_sha = sha256_file(final_file)
        if mapping.get("baseline_sha256") != old_sha:
            errors.append(f"central harness baseline digest mismatch: {path}")
        if mapping.get("final_sha256") != new_sha:
            errors.append(f"central harness final digest mismatch: {final_path}")
        identical = old_sha == new_sha
        if mapping.get("byte_identical") is not identical:
            errors.append(f"central harness byte_identical flag is wrong: {path}")
        adaptations = mapping.get("adaptations")
        if not isinstance(adaptations, list):
            errors.append(f"central harness adaptations must be an array: {path}")
            continue
        diff = exact_diff(baseline_source_bytes(repo, baseline, path), final_file.read_bytes(), path, final_path)
        supplied = [item.get("unified_diff") for item in adaptations if isinstance(item, dict)]
        if identical:
            if adaptations:
                errors.append(f"byte-identical source has an adaptation: {path}")
        elif not adaptations:
            errors.append(f"changed source lacks a reviewed adaptation: {path}")
        elif diff not in supplied and "".join(str(value) for value in supplied) != diff:
            errors.append(f"central harness adaptation hunk is not exact: {path}")
    return errors


def support_adaptation_errors(repo: Path, baseline: dict[str, Any], manifest: dict[str, Any]) -> list[str]:
    adaptations = manifest.get("support_adaptations")
    if not isinstance(adaptations, list) or not all(isinstance(item, dict) for item in adaptations):
        return ["central harness support_adaptations must be an array of objects"]
    errors: list[str] = []
    revision = baseline.get("repository", {}).get("baseline_git_revision")
    if not isinstance(revision, str) or not revision:
        return ["baseline lacks baseline_git_revision for support adaptation proof"]
    for adaptation in adaptations:
        path = adaptation.get("path")
        if not isinstance(path, str) or not path:
            errors.append("central harness support adaptation lacks path")
            continue
        old_result = subprocess.run(
            ["git", "show", f"{revision}:{path}"],
            cwd=repo,
            capture_output=True,
            check=False,
        )
        if old_result.returncode:
            errors.append(f"central harness support baseline source is unavailable: {path}")
            continue
        final_file = (repo / path).resolve()
        if not final_file.is_file():
            errors.append(f"central harness support final source is missing: {path}")
            continue
        old = old_result.stdout
        new = final_file.read_bytes()
        diff = exact_diff(old, new, path, path)
        if adaptation.get("baseline_sha256") != sha256_bytes(old):
            errors.append(f"central harness support baseline digest mismatch: {path}")
        if adaptation.get("final_sha256") != sha256_bytes(new):
            errors.append(f"central harness support final digest mismatch: {path}")
        if adaptation.get("unified_diff") != diff:
            errors.append(f"central harness support adaptation hunk is not exact: {path}")
    return errors


def case_mapping_errors(
    repo: Path,
    baseline: dict[str, Any],
    manifest: dict[str, Any],
    current_cases: list[dict[str, Any]],
) -> list[str]:
    mappings = manifest.get("cases")
    if not isinstance(mappings, list) or not all(isinstance(item, dict) for item in mappings):
        return ["central harness cases must be an array of objects"]
    baseline_cases = baseline.get("integration", {}).get("cases", [])
    baseline_by_id = {str(item.get("id")): item for item in baseline_cases if isinstance(item, dict)}
    current_by_id = {str(item.get("id")): item for item in current_cases}
    errors: list[str] = []
    if len(mappings) != len(baseline_by_id):
        errors.append(
            f"central harness case mapping count {len(mappings)} != baseline {len(baseline_by_id)}"
        )
    for mapping in mappings:
        case_id = mapping.get("baseline_case_id", mapping.get("id"))
        if not isinstance(case_id, str):
            continue
        current = current_by_id.get(case_id)
        if current is None:
            continue
        for manifest_key, current_key in (
            ("final_target", "target"),
            ("final_cargo_name", "cargo_name"),
            ("final_source", "source"),
            ("function", "function"),
        ):
            if mapping.get(manifest_key) != current.get(current_key):
                errors.append(f"case {case_id}: {manifest_key} does not match Cargo/source inventory")
        expected_module = str(current["cargo_name"]).rsplit("::", 1)[0]
        if mapping.get("final_module") != expected_module:
            errors.append(f"case {case_id}: final_module does not match final_cargo_name")
        source = current.get("source")
        if isinstance(source, str) and mapping.get("final_sha256") != sha256_file(repo / source):
            errors.append(f"case {case_id}: final source digest mismatch")
    return errors


def binary_handoff_errors(manifest: dict[str, Any]) -> list[str]:
    section = manifest.get("binary_handoff")
    if not isinstance(section, dict):
        return ["central harness binary_handoff is required"]
    artifact = section.get("artifact")
    if not isinstance(artifact, str) or not artifact:
        return ["central harness binary_handoff artifact is required"]
    path = Path(artifact)
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        return [f"central harness binary handoff cannot be read: {error}"]
    if not isinstance(value, dict):
        return ["central harness binary handoff must be an object"]
    errors: list[str] = []
    if value.get("schema_version") != 1:
        errors.append("central harness binary handoff schema_version must be 1")
    if value.get("active_target") != section.get("active_target"):
        errors.append("central harness binary handoff active_target mismatch")
    expected = section.get("entries")
    actual = value.get("binaries")
    if not isinstance(expected, dict) or not isinstance(actual, dict):
        return errors + ["central harness binary handoff entries must be objects"]
    if set(expected) != set(actual):
        errors.append("central harness binary handoff names do not match")
    for name, entry in expected.items():
        handed = actual.get(name)
        if not isinstance(entry, dict) or not isinstance(handed, dict):
            errors.append(f"central harness binary handoff entry is malformed: {name}")
            continue
        for key in ("package", "target", "path", "sha256"):
            if handed.get(key) != entry.get(key):
                errors.append(f"central harness binary handoff mismatch for {name}: {key}")
        handed_path = handed.get("path")
        if isinstance(handed_path, str):
            try:
                if sha256_file(Path(handed_path)) != handed.get("sha256"):
                    errors.append(f"central harness binary handoff digest is stale: {name}")
            except OSError as error:
                errors.append(f"central harness binary handoff path is unreadable for {name}: {error}")
    return errors


def preservation_errors(baseline: dict[str, Any], manifest: dict[str, Any], repo: Path) -> list[str]:
    before = baseline.get("extracted_preservation_inventory")
    recorded = manifest.get("preservation_inventory")
    if not isinstance(before, dict) or not isinstance(recorded, dict):
        return ["central harness preservation_inventory is required"]
    source_entries = [
        item
        for item in baseline.get("integration", {}).get("source_files", [])
        if isinstance(item, dict) and isinstance(item.get("path"), str)
    ]
    after = extracted_inventories(source_entries, repo)
    errors: list[str] = []
    for category in ("assertions", "failure_injection", "bookends_citations"):
        before_sig = inventory_signature(before, category)
        after_sig = inventory_signature(after, category)
        if before[category].get("count") != after[category].get("count") or before_sig != after_sig:
            errors.append(f"central harness {category} inventory changed")
        proof = recorded.get(category)
        if (
            not isinstance(proof, dict)
            or proof.get("baseline_count") != before[category].get("count")
            or proof.get("final_count") != after[category].get("count")
            or proof.get("normalized_sha256") != after_sig
            or proof.get("unchanged") is not True
        ):
            errors.append(f"central harness {category} inventory proof is incomplete")
    return errors


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, default=Path.cwd())
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--compiler-artifacts", type=Path, required=True)
    parser.add_argument("--test-lists", type=Path, required=True)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    repo = args.repo.resolve()
    baseline = load_json(args.baseline.resolve())
    manifest = load_json(args.manifest.resolve())
    if not isinstance(baseline, dict) or not isinstance(manifest, dict):
        raise ContractError("baseline and central harness manifest must be JSON objects")
    metadata = cargo_metadata(repo)
    targets = integration_targets(metadata, repo)
    binaries = binary_targets(metadata, repo)
    emitted = compiler_artifact_targets(args.compiler_artifacts.resolve(), repo)
    errors = topology_errors(targets, binaries, emitted)
    errors.extend(inventory_manifest_errors(baseline, manifest))
    if manifest.get("central_target") != "workspace-integration/workspace":
        errors.append("central harness central_target must be workspace-integration/workspace")

    listings = load_test_lists(args.test_lists.resolve())
    _target_inventory, current_cases = source_test_inventory(metadata, repo, targets, listings)
    expected_ids = {str(item.get("id")) for item in baseline["integration"]["cases"]}
    actual_ids = {str(item.get("id")) for item in current_cases}
    if expected_ids != actual_ids:
        errors.append("central Cargo named/source inventory does not equal the baseline case IDs")
    errors.extend(source_mapping_errors(repo, baseline, manifest))
    errors.extend(support_adaptation_errors(repo, baseline, manifest))
    errors.extend(case_mapping_errors(repo, baseline, manifest, current_cases))
    errors.extend(binary_handoff_errors(manifest))
    errors.extend(preservation_errors(baseline, manifest, repo))
    if errors:
        raise ContractError("\n".join(errors))
    print(
        "central harness equivalence ok: one integration executable, "
        f"{len(current_cases)} named cases, exact source mappings, and unchanged preservation inventories"
    )
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except ContractError as error:
        print(f"central harness contract failed:\n{error}", file=sys.stderr)
        raise SystemExit(1)
