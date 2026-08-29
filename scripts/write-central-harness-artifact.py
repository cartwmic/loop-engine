#!/usr/bin/env python3
"""Write the reviewable T02 central-harness equivalence artifact."""

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
    REQUIRED_BINARIES,
    cargo_metadata,
    extracted_inventories,
    integration_targets,
    load_json,
    load_test_lists,
    sha256_bytes,
    sha256_file,
    source_files,
    source_test_inventory,
)


def baseline_bytes(repo: Path, revision: str, path: str) -> bytes:
    return subprocess.run(
        ["git", "show", f"{revision}:{path}"],
        cwd=repo,
        capture_output=True,
        check=True,
    ).stdout


def unified_diff(old: bytes, new: bytes, old_path: str, new_path: str) -> str:
    return "".join(
        difflib.unified_diff(
            old.decode("utf-8").splitlines(keepends=True),
            new.decode("utf-8").splitlines(keepends=True),
            fromfile=old_path,
            tofile=new_path,
            lineterm="\n",
        )
    )


def inventory_signature(inventory: dict[str, Any], category: str) -> str:
    fields = ("source", "kind", "match") if category != "bookends_citations" else ("source", "match")
    entries = inventory[category]["entries"]
    normalized = [tuple(entry.get(field) for field in fields) for entry in entries]
    normalized.sort(key=lambda entry: tuple(str(value) for value in entry))
    return sha256_bytes(
        json.dumps(normalized, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
    )


def git_revision(repo: Path) -> str:
    return subprocess.run(
        ["git", "rev-parse", "HEAD"], cwd=repo, capture_output=True, text=True, check=True
    ).stdout.strip()


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, default=Path.cwd())
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--compiler-artifacts", type=Path, required=True)
    parser.add_argument("--test-lists", type=Path, required=True)
    parser.add_argument("--binary-handoff", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    repo = args.repo.resolve()
    baseline = load_json(args.baseline.resolve())
    metadata = cargo_metadata(repo)
    targets = integration_targets(metadata, repo)
    listings = load_test_lists(args.test_lists.resolve())
    _target_inventory, cases = source_test_inventory(metadata, repo, targets, listings)
    baseline_sources = baseline["integration"]["source_files"]
    baseline_revision = baseline["repository"]["baseline_git_revision"]
    current_by_path = {entry["path"]: entry for entry in source_files(metadata, repo)}

    source_mappings: list[dict[str, Any]] = []
    for entry in baseline_sources:
        path = str(entry["path"])
        final_entry = current_by_path[path]
        final_path = path
        final_bytes = (repo / final_path).read_bytes()
        old_bytes = baseline_bytes(repo, baseline_revision, path)
        diff = unified_diff(old_bytes, final_bytes, path, final_path)
        if path == "crates/software-change-provider/tests/contracts/process_timeout_probe.rs":
            adaptation_reason = (
                "pre-existing nextest timeout-proof adaptation retained outside the central-harness move"
            )
        else:
            adaptation_reason = "central package binary handoff or owning-package path adaptation"
        source_mappings.append(
            {
                "baseline_path": path,
                "final_path": final_path,
                "baseline_sha256": entry["sha256"],
                "final_sha256": sha256_file(repo / final_path),
                "byte_identical": not bool(diff),
                "adaptations": []
                if not diff
                else [
                    {
                        "reason": adaptation_reason,
                        "unified_diff": diff,
                    }
                ],
                "baseline_size_bytes": entry["size_bytes"],
                "final_size_bytes": final_entry["size_bytes"],
            }
        )

    support_adaptations = []
    fixture_source = "tests/fixtures/src/lib.rs"
    fixture_old = baseline_bytes(repo, baseline_revision, fixture_source)
    fixture_new = (repo / fixture_source).read_bytes()
    fixture_diff = unified_diff(fixture_old, fixture_new, fixture_source, fixture_source)
    support_adaptations.append(
        {
            "path": fixture_source,
            "reason": "remove the fixture crate's permissive binary fallback; central tests use the shared handoff resolver",
            "baseline_sha256": sha256_bytes(fixture_old),
            "final_sha256": sha256_bytes(fixture_new),
            "unified_diff": fixture_diff,
        }
    )

    baseline_cases = {str(case["id"]): case for case in baseline["integration"]["cases"]}
    case_mappings: list[dict[str, Any]] = []
    for case in cases:
        case_id = str(case["id"])
        cargo_name = str(case["cargo_name"])
        source = str(case["source"])
        case_mappings.append(
            {
                "baseline_case_id": case_id,
                "baseline_cargo_name": baseline_cases[case_id]["cargo_name"],
                "final_target": case["target"],
                "final_cargo_name": cargo_name,
                "final_module": cargo_name.rsplit("::", 1)[0],
                "final_source": source,
                "function": case["function"],
                "final_line": case["line"],
                "final_sha256": sha256_file(repo / source),
            }
        )

    current_inventory = extracted_inventories(baseline_sources, repo)
    preservation: dict[str, Any] = {}
    for category in ("assertions", "failure_injection", "bookends_citations"):
        before = baseline["extracted_preservation_inventory"][category]
        after = current_inventory[category]
        before_signature = inventory_signature(baseline["extracted_preservation_inventory"], category)
        after_signature = inventory_signature(current_inventory, category)
        preservation[category] = {
            "baseline_count": before["count"],
            "final_count": after["count"],
            "baseline_normalized_sha256": before_signature,
            "normalized_sha256": after_signature,
            "unchanged": before["count"] == after["count"] and before_signature == after_signature,
        }

    required_binary_names = [f"{package}/{target}" for package, target, _kind in REQUIRED_BINARIES]
    compiler_artifacts = args.compiler_artifacts.resolve()
    target_directory = Path(str(metadata["target_directory"])).resolve()
    def binary_alias(package: str, target_name: str) -> str:
        if package == "loop-reference-fixtures":
            return f"fixture:{target_name}"
        return target_name

    binary_handoff = {
        binary_alias(package, target_name): {
            "package": package,
            "target": target_name,
            "path": str((target_directory / "debug" / target_name).resolve()),
            "sha256": sha256_file((target_directory / "debug" / target_name).resolve()),
        }
        for package, target_name, _kind in REQUIRED_BINARIES
    }
    artifact = {
        "schema_version": 1,
        "artifact_kind": "workspace-integration-central-harness",
        "repository": {
            "root": str(repo),
            "baseline_artifact": str(args.baseline.resolve()),
            "baseline_git_revision": baseline_revision,
            "final_git_revision": git_revision(repo),
            "final_repository_state": git_revision(repo) + "+uncommitted-worktree",
        },
        "central_target": "workspace-integration/workspace",
        "required_binaries": required_binary_names,
        "runner": {
            "command": "python3 scripts/run-central-tests.py",
            "focused_form": "python3 scripts/run-central-tests.py --filter PATTERN",
            "binary_handoff_environment": "LOOP_ENGINE_TEST_BINARY_HANDOFF",
            "compiler_artifacts": str(compiler_artifacts),
            "test_lists": str(args.test_lists.resolve()),
            "build_command": [
                "cargo",
                "build",
                "--locked",
                "--message-format=json",
                "-p",
                "bookends-check",
                "-p",
                "loop-cli",
                "-p",
                "policy-document-provider",
                "-p",
                "research-provider",
                "-p",
                "software-change-provider",
                "-p",
                "loop-reference-fixtures",
                "--bins",
            ],
            "central_test_command": [
                "cargo",
                "test",
                "--locked",
                "-p",
                "workspace-integration",
                "--test",
                "workspace",
                "--",
                "--test-threads=4",
            ],
        },
        "target_contract": {
            "metadata_target_count": len(targets),
            "emitted_integration_executable_count": 1,
            "compiler_artifact_sha256": sha256_file(compiler_artifacts),
            "required_binaries": required_binary_names,
        },
        "binary_handoff": {
            "environment": "LOOP_ENGINE_TEST_BINARY_HANDOFF",
            "artifact": str(args.binary_handoff.resolve()) if args.binary_handoff else None,
            "active_target": str(target_directory),
            "entries": binary_handoff,
        },
        "resolver_contract": {
            "outer_command": "python3 scripts/run-central-tests.py",
            "inner_contract_command": "cargo test --locked -p workspace-integration --lib",
            "cases": [
                {
                    "case": "missing handoff environment",
                    "test": "missing_handoff_environment_is_rejected_before_any_binary_lookup",
                    "outcome": "rejected before binary lookup",
                },
                {
                    "case": "missing handoff file",
                    "test": "missing_handoff_file_is_rejected_before_any_binary_lookup",
                    "outcome": "rejected before binary lookup",
                },
                {
                    "case": "non-executable path",
                    "test": "non_executable_path_is_rejected_before_spawn",
                    "outcome": "rejected before spawn",
                },
                {
                    "case": "outside active target",
                    "test": "path_outside_active_target_is_rejected_before_spawn",
                    "outcome": "rejected before spawn",
                },
                {
                    "case": "stale direct candidate digest",
                    "test": "stale_direct_candidate_digest_is_rejected_before_spawn",
                    "outcome": "digest mismatch rejected before spawn",
                },
                {
                    "case": "stale hashed fallback",
                    "test": "stale_hashed_candidate_is_never_selected_as_a_fallback",
                    "outcome": "hashed candidate not selected and not executed",
                },
                {
                    "case": "direct name under deps",
                    "test": "direct_name_under_deps_is_rejected_before_spawn",
                    "outcome": "direct deps candidate rejected before spawn",
                },
            ],
        },
        "module_roots": [
            {
                "module": module,
                "baseline_root": source,
                "final_target": "workspace-integration/workspace",
            }
            for module, source in (
                ("bookends_check_cli", "crates/bookends-check/tests/cli.rs"),
                ("bookends_check_graph", "crates/bookends-check/tests/graph.rs"),
                ("loop_cli_dagu", "crates/loop-cli/tests/dagu.rs"),
                ("loop_cli_engine", "crates/loop-cli/tests/engine.rs"),
                ("loop_cli_workers", "crates/loop-cli/tests/workers.rs"),
                ("loop_integrations_concurrency", "crates/loop-integrations/tests/concurrency.rs"),
                ("loop_integrations_provider_gateway", "crates/loop-integrations/tests/provider_gateway.rs"),
                ("loop_integrations_sqlite_persistence", "crates/loop-integrations/tests/sqlite_persistence.rs"),
                ("reference_fixture_providers", "tests/fixtures/tests/reference_providers.rs"),
                ("reference_fixture_workflows", "tests/fixtures/tests/reference_workflows.rs"),
                ("policy_document_describe_protocol", "crates/policy-document-provider/tests/describe_protocol.rs"),
                ("research_cli", "crates/research-provider/tests/cli.rs"),
                ("research_describe_protocol", "crates/research-provider/tests/describe_protocol.rs"),
                ("research_embedded_data", "crates/research-provider/tests/embedded_data.rs"),
                ("research_evaluate", "crates/research-provider/tests/evaluate.rs"),
                ("research_shipped_data", "crates/research-provider/tests/shipped_data.rs"),
                ("software_change_cli", "crates/software-change-provider/tests/cli.rs"),
                ("software_change_contracts", "crates/software-change-provider/tests/contracts.rs"),
                ("software_change_plan_graph", "crates/software-change-provider/tests/plan_graph.rs"),
                ("software_change_provider", "crates/software-change-provider/tests/provider.rs"),
            )
        ],
        "source_mappings": source_mappings,
        "support_adaptations": support_adaptations,
        "cases": case_mappings,
        "case_count": len(case_mappings),
        "test_listing": {
            "entries": listings,
            "case_names_sha256": sha256_bytes(
                json.dumps(
                    sorted(str(name) for entry in listings for name in entry.get("tests", [])),
                    separators=(",", ":"),
                ).encode()
            ),
        },
        "preservation_inventory": preservation,
        "central_sources": [
            {
                "path": path,
                "sha256": sha256_file(repo / path),
            }
            for path in (
                "tests/fixtures/src/lib.rs",
                "tests/workspace-integration/Cargo.toml",
                "tests/workspace-integration/src/lib.rs",
                "tests/workspace-integration/tests/workspace.rs",
                "scripts/run-central-tests.py",
                "scripts/assert-central-harness.py",
            )
        ],
        "assumptions": [
            "The supported local platform is Unix-like: executable-bit validation and the existing process-group helper use Unix process semantics.",
            "The Cargo target directory reported by metadata is the active target root; the runner canonicalizes every build output and the resolver rejects paths outside it or under debug/deps.",
            "The fixture package remains the owner of its three binaries; only their test invocation paths move to the central handoff.",
        ],
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(artifact, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(f"wrote {args.output} ({len(case_mappings)} cases, {len(source_mappings)} source mappings)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
