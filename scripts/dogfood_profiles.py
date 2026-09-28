"""Public setup and Bookends pre-plan fixtures for standalone profiles."""
from __future__ import annotations

import copy
import hashlib
import json
import shutil
import subprocess
import time
from pathlib import Path
from typing import Any

import dogfood_observation

OCCASIONS = [
    "review-candidates",
    "accepted-defect",
    "implementation-correction",
    "execution-or-authority-issue",
    "evidence-applicability",
    "requirements-reconciliation",
    "review-round-departure",
    "final-completion",
]


def _root(journey) -> Path:
    return dogfood_observation._fresh_root(journey, "sol-profiles")


def _capture(
    root: Path,
    argv: list[str],
    *,
    expected_code: int = 0,
    parse_json: bool = True,
    timeout: float = 90,
) -> tuple[Any, subprocess.CompletedProcess[bytes]]:
    logs = root / "commands"
    logs.mkdir(exist_ok=True)
    ordinal = len(list(logs.glob("*.argv.json")))
    stem = f"{ordinal:04d}"
    started = time.perf_counter_ns()
    try:
        completed = subprocess.run(argv, cwd=root, capture_output=True, timeout=timeout, check=False)
    except (OSError, subprocess.TimeoutExpired) as error:
        raise ValueError(f"public command did not complete: {argv!r}: {error}") from error
    elapsed_ms = (time.perf_counter_ns() - started) / 1_000_000
    (logs / f"{stem}.argv.json").write_text(
        json.dumps({"argv": argv, "cwd": str(root)}) + "\n", encoding="utf-8"
    )
    (logs / f"{stem}.stdout").write_bytes(completed.stdout)
    (logs / f"{stem}.stderr").write_bytes(completed.stderr)
    (logs / f"{stem}.exit.json").write_text(
        json.dumps({"returncode": completed.returncode, "elapsed_ms": elapsed_ms}) + "\n",
        encoding="utf-8",
    )
    if completed.returncode != expected_code:
        raise ValueError(
            f"public command exit mismatch: expected {expected_code}, got {completed.returncode}: "
            f"{argv!r}; stderr={completed.stderr[-1200:]!r}"
        )
    if not parse_json:
        return None, completed
    try:
        value = json.loads(completed.stdout)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError(f"public command returned invalid JSON: {argv!r}: {error}") from error
    return value, completed


def _engine(journey, root: Path, database: Path, *args: str, expected_code: int = 0):
    value, _ = _capture(
        root,
        [str(journey.engine), "--database", str(database), "--json", *args],
        expected_code=expected_code,
    )
    return value


def _provider(
    journey,
    root: Path,
    *args: str,
    expected_code: int = 0,
    parse_json: bool = True,
):
    return _capture(
        root,
        [str(journey.provider), *args],
        expected_code=expected_code,
        parse_json=parse_json,
    )


def _git_init(root: Path) -> None:
    for args in (["init", "-q"],):
        result = subprocess.run(["git", *args], cwd=root, capture_output=True, check=False)
        if result.returncode:
            raise ValueError(f"git init failed in {root}: {result.stderr[-800:]!r}")


def _git_add(root: Path) -> None:
    result = subprocess.run(["git", "add", "."], cwd=root, capture_output=True, check=False)
    if result.returncode:
        raise ValueError(f"git add failed in {root}: {result.stderr[-800:]!r}")


def _write_preview_repo(
    root: Path,
    *,
    config: bool = True,
    live_requirement: bool = True,
    recognized_ci: bool = True,
    root_cwd: bool = True,
    matching_location: bool = True,
) -> None:
    root.mkdir(parents=True)
    (root / "docs").mkdir()
    (root / "scripts").mkdir()
    (root / ".github/workflows").mkdir(parents=True)
    (root / "docs/PRD.md").write_text(
        "### 4. Intent\n\n"
        + (
            "### LE-1: Supplied user outcome\n- Status: live\n- Coverage: e2e/journey\n\n"
            "The operator can inspect the selected policy before start. The applicable "
            "contract is in [docs/contract.md](contract.md).\n"
            if live_requirement
            else "### 4. Human prose only\nNo live requirement is accepted here.\n"
        ),
        encoding="utf-8",
    )
    (root / "docs/contract.md").write_text("# Explicit requirement companion\n", encoding="utf-8")
    (root / "scripts/journey.py").write_text(
        "# public assertion: selected policy bytes control setup output\n", encoding="utf-8"
    )
    if config:
        pathspec = "scripts/**" if matching_location else "uncovered/**"
        (root / "bookends.toml").write_text(
            'prd = "docs/PRD.md"\n\n'
            "[classes.e2e_journey]\n"
            f'pathspecs = ["{pathspec}"]\n'
            'required_ci_jobs = ["journey"]\n',
            encoding="utf-8",
        )
    run = "python3 scripts/journey.py" if recognized_ci else "python3 scripts/journey.py --mode source"
    cwd = "" if root_cwd else "        working-directory: nested\n"
    (root / ".github/workflows/test.yml").write_text(
        "jobs:\n  journey:\n    steps:\n      - name: collect\n"
        + cwd
        + f"        run: {run}\n",
        encoding="utf-8",
    )
    _git_init(root)
    _git_add(root)


def _preview(journey, root: Path, repository: Path) -> dict[str, Any]:
    value, _ = _provider(
        journey,
        root,
        "bookends-preview",
        "--working-directory",
        str(repository),
    )
    if value.get("schema_version") != 1 or value.get("evaluation_cwd") != str(repository):
        raise ValueError(f"Bookends preview lost its current evaluation cwd: {value!r}")
    if value.get("gate_executed") is not False or value.get("tests_executed") is not False:
        raise ValueError(f"pre-plan preview executed a gate or test: {value!r}")
    return value


def _phase_coverage_gaps(artifacts: dict[str, dict[str, Any]]) -> list[str]:
    """Check the phase-specific fixture handoff, not matching IDs alone."""
    gaps: list[str] = []
    intent = artifacts["intent"]
    criteria = intent.get("acceptance", [])
    if not any(
        row.get("id") == "AC-1"
        and row.get("prd_traceability", {}).get("live_ids") == ["LE-1"]
        and "operator" in row.get("statement", "").lower()
        for row in criteria
    ) or not any(
        row.get("source") == "docs/contract.md"
        for row in intent.get("operating_context", {}).get("outside_obligations", [])
    ):
        gaps.append("intent traceability, named document, and operator outcome")

    design_rows = artifacts["design"].get("coverage", [])
    if not any(
        row.get("criterion_id") == "AC-1"
        and "docs/contract.md" in row.get("delivered_by", "")
        and "operator-visible" in row.get("delivered_by", "")
        for row in design_rows
    ):
        gaps.append("design phase explanation and named requirement document")

    plan = artifacts["plan"]
    plan_tasks = plan.get("tasks", [])
    commands = {row.get("id"): row for row in plan.get("proof_commands", [])}
    if not any(
        row.get("criterion_ids") == ["AC-1"]
        and "docs/contract.md" in json.dumps(row.get("source_of_truth", []))
        and "assertion" in json.dumps(row.get("validation", [])).lower()
        and any(command_id in commands for command_id in row.get("proof_command_ids", []))
        for row in plan_tasks
    ):
        gaps.append("plan task-to-public-assertion relationship")

    report = artifacts["implementation-report"]
    documents = report.get("coverage", {}).get("documents", [])
    if not any(row.get("path") == "docs/contract.md" for row in documents) or not any(
        row.get("criterion_id") == "AC-1"
        and "operator-visible assertion" in row.get("proof", "")
        for row in report.get("validation", [])
    ):
        gaps.append("implementation report document and focused proof links")

    validation = artifacts["validation-report"]
    evidence_records = artifacts.get("validation-evidence-records", {})
    evidence = {row.get("id"): row for row in evidence_records.get("command_evidence", [])}
    verdicts = {row.get("id"): row for row in evidence_records.get("verdicts", [])}
    if "profile-setup" not in validation.get("command_evidence_ids", []) or not any(
        row.get("criterion_id") == "AC-1"
        and any(
            "profile-setup" in verdicts.get(verdict_id, {}).get("evidence_context_ids", [])
            and "selected profile bytes drive effective policy"
            in evidence.get("profile-setup", {}).get("assertion", "")
            and "docs/contract.md" in evidence.get("profile-setup", {}).get("assertion", "")
            for verdict_id in row.get("verdict_ids", [])
        )
        for row in validation.get("criteria", [])
    ):
        gaps.append("validation criterion-to-observed-command assertion relationship")
    return gaps


def _coverage_fixture() -> dict[str, dict[str, Any]]:
    return {
        "intent": {
            "revision": "intent-r1",
            "author": {"name": "fixture", "kind": "script"},
            "problem": "An operator needs to know which copied profile is effective before starting.",
            "outcome": "The selected file's policy and exact bytes are visible before start.",
            "acceptance": [{
                "id": "AC-1",
                "statement": "An operator sees the copied profile's changed review axis before start.",
                "prd_traceability": {"type": "linked-live", "live_ids": ["LE-1"]},
            }],
            "constraints": [],
            "non_goals": [],
            "operating_context": {
                "operators": ["trusted local operator"],
                "environment": ["local repository"],
                "threat_boundary": {"in_scope": ["trusted operator"], "excluded": ["hostile operators"]},
                "accepted_risks": [],
                "outside_obligations": [{
                    "source": "docs/contract.md",
                    "obligation": "The selected file controls the setup policy.",
                }],
            },
        },
        "design": {
            "revision": "design-r1",
            "author": {"name": "fixture", "kind": "script"},
            "intent_revision": "intent-r1",
            "approach": "Read the selected complete JSON profile and expose its generated policy and binding preview.",
            "elements": [{"name": "selected profile", "responsibility": "Supplies policy, stage, count and schema bytes."}],
            "decisions": [{"choice": "Use the selected standalone file", "rationale": "Its bytes are the operator's explicit policy input."}],
            "risks": [{"risk": "A basename could mislead the operator", "mitigation": "Show exact selected and effective bytes and hashes."}],
            "coverage": [{
                "criterion_id": "AC-1",
                "acceptance": "The operator can inspect selected profile behavior before start.",
                "delivered_by": "The setup output's operator-visible assertion ties the selected policy bytes to effective stages and bindings; docs/contract.md supplies the named obligation.",
            }],
        },
        "plan": {
            "revision": "plan-r1",
            "author": {"name": "fixture", "kind": "script"},
            "design_revision": "design-r1",
            "objective": "Prove selected profile contents determine the pre-start policy.",
            "tasks": [{
                "id": "P08-fixture",
                "objective": "Drive setup and inspect the selected profile's public preview.",
                "dependencies": [],
                "source_of_truth": ["docs/contract.md#selected-profile"],
                "deliverables": ["Exact bytes and stages/counts are observable."],
                "out_of_scope": ["No advice backend or semantic owner attestation."],
                "validation": ["scripts/software-change-journey.py::sol-profiles public assertion: selected policy bytes control effective stage/count and amendment cannot raise the frozen floor."],
                "handoff": "The selected profile and hash remain visible before start.",
                "criterion_ids": ["AC-1"],
                "proof_command_ids": ["sol-profiles"],
            }],
            "dependency_graph": [],
            "proof_commands": [{
                "id": "sol-profiles",
                "command": "python3",
                "args": ["scripts/software-change-journey.py", "--scenario", "sol-profiles"],
                "owner": "fixture",
                "obligation": "Assert actual selected bytes, effective stage/count and post-start frozen floor through public CLI.",
            }],
        },
        "implementation-report": {
            "revision": "implementation-r1",
            "author": {"name": "fixture", "kind": "script"},
            "plan_revision": "plan-r1",
            "coverage": {
                "commit": "fixture-current",
                "documents": [{"path": "docs/contract.md", "revision": "current"}],
            },
            "summary": "The public setup path used selected profile bytes to assemble changed stages and bindings.",
            "changed_surface": ["selected profile setup and preview"],
            "validation": [{
                "criterion_id": "AC-1",
                "proof": "sol-profiles retained the operator-visible assertion that selected profile bytes drive the effective policy.",
            }],
        },
        "validation-report": {
            "revision": "validation-r1",
            "author": {"name": "fixture", "kind": "script"},
            "implementation_revision": "implementation-r1",
            "command_evidence_ids": ["profile-setup"],
            "criteria": [{"criterion_id": "AC-1", "verdict_ids": ["ac1-pass"]}],
            "goal_verdict_ids": ["goal-pass"],
        },
        "validation-evidence-records": {
            "command_evidence": [{
                "id": "profile-setup",
                "assertion": "selected profile bytes drive effective policy and setup reports those exact bytes and bindings; docs/contract.md names the applicable obligation",
            }],
            "verdicts": [{
                "id": "ac1-pass",
                "evidence_context_ids": ["profile-setup"],
                "reason": "The retained public setup assertion demonstrates selected policy behavior.",
            }],
        },
    }


def _phase_coverage_proof(root: Path) -> dict[str, Any]:
    artifacts = _coverage_fixture()
    if "docs/contract.md" not in artifacts["design"]["coverage"][0]["delivered_by"]:
        raise ValueError("fixture omitted its accepted requirement cross-reference")
    if len(artifacts["plan"]["tasks"]) != 1 or artifacts["plan"]["tasks"][0]["criterion_ids"] != ["AC-1"]:
        raise ValueError("phase coverage copied IDs to every task or lost the AC spine")
    valid_gaps = _phase_coverage_gaps(artifacts)
    if valid_gaps:
        raise ValueError(f"phase-specific five-artifact fixture is incomplete: {valid_gaps}")

    missing_reference = copy.deepcopy(artifacts)
    missing_reference["validation-evidence-records"]["command_evidence"][0]["assertion"] = (
        "selected profile bytes drive effective policy, but the named applicable document is omitted"
    )
    missing_reference_gaps = _phase_coverage_gaps(missing_reference)
    if not any("validation" in gap for gap in missing_reference_gaps):
        raise ValueError(f"validation coverage silently omitted a required live cross-reference: {missing_reference_gaps}")

    id_only = copy.deepcopy(artifacts)
    id_only["design"]["coverage"] = [{"criterion_id": "AC-1", "acceptance": "LE-1", "delivered_by": "LE-1"}]
    id_only["validation-evidence-records"]["command_evidence"][0]["assertion"] = "LE-1"
    id_only_gaps = _phase_coverage_gaps(id_only)
    if not any("design" in gap for gap in id_only_gaps) or not any("validation" in gap for gap in id_only_gaps):
        raise ValueError(f"ID-only coverage was treated as proof: {id_only_gaps}")

    (root / "phase-coverage-fixture.json").write_text(
        json.dumps({"valid": artifacts, "id_only_rejected_gaps": id_only_gaps}, indent=2) + "\n",
        encoding="utf-8",
    )
    return {
        "missing_validation_reference_gaps": missing_reference_gaps,
        "phase_artifacts": [
            "intent",
            "design",
            "plan",
            "implementation-report",
            "validation-report",
        ],
        "valid_phase_gaps": valid_gaps,
        "id_only_rejected_gaps": id_only_gaps,
        "task_ids_are_ac_only": True,
    }


def profiles_case(journey) -> None:
    root = _root(journey)
    export = root / "embedded-export"
    _provider(journey, root, "data-dump", str(export), parse_json=False)
    config_root = export / "crates/software-change-provider/data/configs"
    profiles = {
        "minimal": json.loads((config_root / "minimal.json").read_text(encoding="utf-8")),
        "standard": json.loads((config_root / "standard.json").read_text(encoding="utf-8")),
        "high-rigor": json.loads((config_root / "high-rigor.json").read_text(encoding="utf-8")),
    }
    if set(profiles) != {"minimal", "standard", "high-rigor"}:
        raise ValueError("data-dump did not export all three standalone profiles")
    for name, profile in profiles.items():
        if profile.get("contract_version") != 3 or not profile.get("review_policies") or not profile.get("artifact_schemas"):
            raise ValueError(f"exported {name} profile is not a complete standalone profile")
        if set(profile["artifact_schemas"]) != {
            "intent.json", "design.json", "plan.json", "implementation-report.json", "validation-report.json"
        }:
            raise ValueError(f"exported {name} profile has inconsistent phase-schema coverage")
        expected_floor = {"minimal": 1, "standard": 2, "high-rigor": 2}[name]
        if profile["criterion_policy"].get("required_authors") != expected_floor or profile["criterion_policy"].get("goal_required_authors") != expected_floor:
            raise ValueError(f"exported {name} profile changed its frozen criterion/goal author floor")
        if set(profile["review_policies"]) != {
            "intent-review", "intent-adversarial-review", "design-review", "design-adversarial-review",
            "plan-review", "plan-adversarial-review", "implementation-review",
            "implementation-adversarial-review", "validation-review", "validation-adversarial-review",
        }:
            raise ValueError(f"exported {name} profile omitted phase policy coverage")
        for gate in ("intent-review", "design-review", "plan-review", "implementation-review", "validation-review"):
            challenge = gate.replace("-review", "-adversarial-review")
            ordinary_axes = {
                (row["id"], row["review_stage"]): row["required_authors"]
                for row in profile["review_policies"][gate]
            }
            challenge_axes = {
                (row["id"], row["review_stage"]): row["required_authors"]
                for row in profile["review_policies"][challenge]
            }
            if ordinary_axes != challenge_axes:
                raise ValueError(f"exported {name} profile has inconsistent review/challenge stages or counts at {gate}")
            if any(stage not in {"individual", "aggregate"} or count < 1 for (_, stage), count in ordinary_axes.items()):
                raise ValueError(f"exported {name} profile has an unsupported stage or author count at {gate}")
        if profile.get("extra", {}).get("advice", {}).get("occasion_map") != OCCASIONS:
            raise ValueError(f"exported {name} profile has a different advisory occasion map")
    expected_versions = {
        "minimal": "minimal-12",
        "standard": "standard-12",
        "high-rigor": "high-rigor-12",
    }
    if {name: profile["config_version"] for name, profile in profiles.items()} != expected_versions:
        raise ValueError("exported profiles do not use the selected v12 successor identities")

    selected = copy.deepcopy(profiles["minimal"])
    selected["artifact_schemas"]["design.json"]["properties"]["approach"]["minLength"] = 7
    selected_axis = selected["review_policies"]["intent-review"][0]
    selected_axis.update(
        description="CUSTOM PROFILE MARKER: exact selected policy controls setup",
        example_prompt="CUSTOM PROFILE PROMPT MARKER: inspect the selected policy bytes",
        review_stage="individual",
        required_authors=2,
    )
    selected_path = root / "edited" / "minimal.json"
    selected_path.parent.mkdir()
    selected_bytes = (json.dumps(selected, indent=2, ensure_ascii=False) + "\n").encode()
    selected_path.write_bytes(selected_bytes)
    selected_hash = hashlib.sha256(selected_bytes).hexdigest()

    roster = [
        {
            "author": name,
            "command": "/bin/echo",
            "args": [f"reviewer-{name}"],
            "token_budget": {
                "model_id": "scripted-profile-fixture",
                "context_window_tokens": 64000,
                "system_tokens": 1000,
                "framing_tokens": 1000,
                "output_reserve_tokens": 1000,
                "reasoning_reserve_tokens": 1000,
            },
        }
        for name in ("alpha", "beta")
    ]
    roster_path = root / "roster.json"
    roster_path.write_text(json.dumps(roster, indent=2) + "\n", encoding="utf-8")
    bundled_setup_dir = root / "bundled-setup"
    bundled_setup_dir.mkdir()
    for name, profile in profiles.items():
        source_path = config_root / f"{name}.json"
        output_path = bundled_setup_dir / f"{name}.effective.json"
        bundled_report, _ = _capture(
            root,
            [
                str(journey.provider), "setup", "--profile", str(source_path),
                "--roster", str(roster_path), "--engine", str(journey.engine),
                "--provider", str(journey.provider), "--output", str(output_path),
                "--decline-advice",
            ],
        )
        source_bytes = source_path.read_bytes()
        effective_bytes = output_path.read_bytes()
        if (
            bundled_report["profile_selection"].get("bytes", "").encode() != source_bytes
            or bundled_report["profile_selection"].get("sha256") != hashlib.sha256(source_bytes).hexdigest()
            or bundled_report["effective_policy"].get("config_version") != profile["config_version"]
            or bundled_report["effective_policy"].get("review_policies") != profile["review_policies"]
            or bundled_report["effective_policy"].get("artifact_schemas") != profile["artifact_schemas"]
            or bundled_report["output_bytes"].encode() != effective_bytes
            or bundled_report["bookends_enabled"] is not False
        ):
            raise ValueError(f"public setup did not expose complete selected {name} policy/schema bytes")
    draft_path = root / "draft-worker.json"
    draft_path.write_text(json.dumps({"command": "/bin/echo", "args": ["draft-command-marker"]}) + "\n", encoding="utf-8")
    implementation_path = root / "implementation.json"
    implementation_path.write_text(
        json.dumps({"command": "/bin/echo", "args": ["implementation-command-marker"], "working_directory": str(root)}) + "\n",
        encoding="utf-8",
    )
    generated = root / "generated" / "profile.json"
    generated.parent.mkdir()
    setup_argv = [
        str(journey.provider), "setup", "--profile", str(selected_path), "--roster", str(roster_path),
        "--engine", str(journey.engine), "--provider", str(journey.provider), "--output", str(generated),
        "--decline-advice", "--bookends", "--draft-worker", str(draft_path), "--implementation", str(implementation_path),
    ]
    setup, _ = _capture(root, setup_argv)
    output_profile = json.loads(generated.read_text(encoding="utf-8"))
    selected_report = setup["profile_selection"]
    if (
        selected_report.get("kind") != "file"
        or selected_report.get("basename") != "minimal.json"
        or selected_report.get("sha256") != selected_hash
        or selected_report.get("byte_length") != len(selected_bytes)
        or selected_report.get("bytes", "").encode() != selected_bytes
    ):
        raise ValueError("setup did not retain exact selected-file basename, bytes and identity")
    effective_bytes = setup["output_bytes"].encode()
    if hashlib.sha256(effective_bytes).hexdigest() != setup["output_sha256"]:
        raise ValueError("setup effective output hash does not match its exact bytes")
    if effective_bytes != generated.read_bytes() or setup["effective_bindings"] != output_profile["work_slot_bindings"]:
        raise ValueError("setup effective bytes/bindings differ from the selected output file")
    if output_profile["review_policies"]["intent-review"][0] != selected_axis:
        raise ValueError("setup substituted embedded policy for the selected copied policy")
    if setup["effective_policy"].get("artifact_schemas", {}).get("design.json") != selected["artifact_schemas"]["design.json"]:
        raise ValueError("effective setup policy omitted the selected-file artifact schema")
    if setup["effective_policy"]["artifact_schemas"]["intent.json"]["properties"]["acceptance"]["items"]["properties"].get("prd_traceability") is None:
        raise ValueError("Bookends-enabled effective policy omitted its intent traceability schema")
    if not setup["bookends_enabled"] or setup["enablement"]["bookends"]["enabled"] is not True:
        raise ValueError("setup omitted effective Bookends enablement")
    advice = setup["enablement"]["advice"]
    if advice.get("enabled") is not False or advice.get("configured") is not False or advice.get("occasion_map") != OCCASIONS:
        raise ValueError("setup claimed advice configured or omitted the common occasion map")
    bindings = setup["effective_bindings"]
    if bindings["intent-draft"] != {"command": "/bin/echo", "args": ["draft-command-marker"]}:
        raise ValueError("setup did not use the selected draft worker command")
    implementation_binding = bindings["implement"]
    implementation_worker = json.loads(implementation_binding["args"][implementation_binding["args"].index("--task-worker") + 1])
    if implementation_worker != {"command": "/bin/echo", "args": ["implementation-command-marker"]}:
        raise ValueError("setup did not use the selected unwrapped implementation command")

    reviewer_binding = bindings["intent-review"]
    reviewer_args = reviewer_binding["args"]
    review_workers = [
        json.loads(reviewer_args[index + 1])
        for index, token in enumerate(reviewer_args[:-1])
        if token == "--worker"
    ]
    selected_workers = [
        worker for worker in review_workers
        if "CUSTOM PROFILE PROMPT MARKER" in worker.get("preamble", "")
    ]
    if len(selected_workers) != 2:
        raise ValueError(f"selected individual/count policy did not bind both authors: {selected_workers!r}")
    if any(worker.get("full_output_schema", {}).get("properties", {}).get("review_stage", {}).get("const") != "individual" for worker in selected_workers):
        raise ValueError("selected-file review stage was replaced during binding assembly")

    # The public start/show/amend-binding path proves that a future executor
    # correction cannot rewrite the started policy or raise its author floor.
    provider_config = root / "providers.toml"
    provider_config.write_text(
        "[providers.software-change]\n"
        f"command = {json.dumps(str(journey.provider))}\nargs = []\n",
        encoding="utf-8",
    )
    database = root / "profile-start.sqlite"
    run_id = "sol-profiles-floor-freeze"
    started = _engine(
        journey, root, database, "--config", str(provider_config), "start", "--id", run_id,
        "software-change", "@" + str(generated),
    )
    before = _engine(journey, root, database, "show", "--view", "full", run_id)["result"]
    initial_input = before.get("initial_input") or before.get("run", {}).get("initial_input")
    expected_floor = selected_axis["required_authors"]
    frozen_floor = next(
        row["required_authors"]
        for row in initial_input["review_policies"]["intent-review"]
        if row["id"] == selected_axis["id"]
    )
    if frozen_floor != expected_floor:
        raise ValueError("started initial input did not freeze the selected policy floor")
    visit = before.get("state_visit", before.get("run", {}).get("state_visit"))
    amended = _engine(
        journey,
        root,
        database,
        "amend-binding",
        run_id,
        "intent-review",
        json.dumps({
            "state_visit": visit,
            "owner": "fixture-owner",
            "reason": "Change only the future command in an isolated policy-freeze fixture",
            "binding": {"command": "/bin/echo", "args": ["amended-future-binding"]},
        }, separators=(",", ":")),
    )
    after = _engine(journey, root, database, "show", "--view", "full", run_id)["result"]
    after_input = after.get("initial_input") or after.get("run", {}).get("initial_input")
    after_floor = next(
        row["required_authors"]
        for row in after_input["review_policies"]["intent-review"]
        if row["id"] == selected_axis["id"]
    )
    current_binding = after.get("effective_bindings", {}).get("intent-review")
    if after_floor != expected_floor or current_binding != {"command": "/bin/echo", "args": ["amended-future-binding"]}:
        raise ValueError("binding amendment changed policy floor or failed to change only the future binding")

    # Refusal paths are all public setup invocations and must not write output.
    mutual_output = root / "invalid-mutual.json"
    _, mutual = _capture(
        root,
        [str(journey.provider), "setup", "--profile", str(selected_path), "--rigor", "minimal",
         "--roster", str(roster_path), "--engine", str(journey.engine), "--provider", str(journey.provider),
         "--output", str(mutual_output), "--decline-advice"],
        expected_code=2,
        parse_json=False,
    )
    if b"mutually exclusive" not in mutual.stderr or mutual_output.exists():
        raise ValueError("setup did not refuse mixed --profile/--rigor before writing")

    bad_coverage = copy.deepcopy(selected)
    bad_coverage["review_policies"]["intent-review"][0]["required_authors"] = 3
    bad_path = root / "edited" / "bad-coverage.json"
    bad_path.write_text(json.dumps(bad_coverage) + "\n", encoding="utf-8")
    bad_output = root / "invalid-coverage.json"
    _, bad = _capture(
        root,
        [str(journey.provider), "setup", "--profile", str(bad_path), "--roster", str(roster_path),
         "--engine", str(journey.engine), "--provider", str(journey.provider), "--output", str(bad_output),
         "--decline-advice"],
        expected_code=1,
        parse_json=False,
    )
    if b"roster has 2 authors" not in bad.stderr or bad_output.exists():
        raise ValueError("setup did not reject unsupported required-author coverage before output")

    legacy_versions = [
        "minimal-10", "standard-10", "high-rigor-10",
        "minimal-11", "standard-11", "high-rigor-11",
    ]
    legacy_dir = root / "edited" / "legacy"
    legacy_dir.mkdir()
    for version in legacy_versions:
        legacy_profile = copy.deepcopy(selected)
        legacy_profile["config_version"] = version
        legacy_path = legacy_dir / "minimal.json"
        legacy_path.write_text(json.dumps(legacy_profile) + "\n", encoding="utf-8")
        legacy_output = root / "generated" / f"refused-{version}.json"
        _, refusal = _capture(
            root,
            [str(journey.provider), "setup", "--profile", str(legacy_path), "--roster", str(roster_path),
             "--engine", str(journey.engine), "--provider", str(journey.provider), "--output", str(legacy_output),
             "--decline-advice"],
            expected_code=1,
            parse_json=False,
        )
        if b"known historical shipped profile config_version" not in refusal.stderr or legacy_output.exists():
            raise ValueError(f"setup did not refuse legacy shipped identity {version} before output")

    custom = copy.deepcopy(selected)
    custom["config_version"] = "caller-managed-profile-11"
    custom_path = root / "edited" / "caller-managed" / "minimal.json"
    custom_path.parent.mkdir()
    custom_path.write_text(json.dumps(custom) + "\n", encoding="utf-8")
    custom_output = root / "generated" / "caller-managed-custom.json"
    custom_setup, _ = _capture(
        root,
        [str(journey.provider), "setup", "--profile", str(custom_path), "--roster", str(roster_path),
         "--engine", str(journey.engine), "--provider", str(journey.provider), "--output", str(custom_output),
         "--decline-advice"],
    )
    if custom_setup["effective_policy"]["config_version"] != "caller-managed-profile-11":
        raise ValueError("setup guessed a refusal from an arbitrary caller-managed version suffix")

    prewrapped = root / "prewrapped-implementation.json"
    prewrapped.write_text(json.dumps({
        "command": str(journey.engine),
        "args": ["fan-out", "--worker", "{}"],
        "working_directory": str(root),
    }) + "\n", encoding="utf-8")
    prewrapped_output = root / "invalid-prewrapped.json"
    _, wrapped = _capture(
        root,
        [str(journey.provider), "setup", "--profile", str(selected_path), "--roster", str(roster_path),
         "--engine", str(journey.engine), "--provider", str(journey.provider), "--output", str(prewrapped_output),
         "--implementation", str(prewrapped), "--decline-advice"],
        expected_code=1,
        parse_json=False,
    )
    if b"unwrapped task-worker" not in wrapped.stderr or prewrapped_output.exists():
        raise ValueError("setup did not clearly refuse a pre-wrapped task worker")

    # Explicit Bookends-off setup carries no injected PRD-ID policy.
    off_output = root / "generated" / "profile-bookends-off.json"
    off, _ = _capture(
        root,
        [str(journey.provider), "setup", "--profile", str(selected_path), "--roster", str(roster_path),
         "--engine", str(journey.engine), "--provider", str(journey.provider), "--output", str(off_output),
         "--decline-advice"],
    )
    off_profile = json.loads(off_output.read_text(encoding="utf-8"))
    if off["bookends_enabled"] or off["enablement"]["advice"]["enabled"] or "bookends" in off_profile.get("extra", {}):
        raise ValueError("Bookends-off setup added overlay or PRD-ID duties")
    if off_profile["artifact_schemas"] != selected["artifact_schemas"]:
        raise ValueError("Bookends-off setup changed artifact schemas")
    if off["effective_policy"].get("artifact_schemas") != selected["artifact_schemas"]:
        raise ValueError("Bookends-off effective policy changed or hid selected artifact schemas")

    current_preview = _preview(journey, root, journey.data_root)
    current_e2e = next((row for row in current_preview["proof_classes"] if row["name"] == "e2e/journey"), None)
    if (
        not current_preview["bookends_toml"]["present"]
        or not current_preview["prd"]["live_requirements"]
        or not current_e2e
        or "scripts/software-change-journey.py" not in current_e2e["eligible_public_locations"]
        or current_preview["evaluation_cwd"] != str(journey.data_root)
    ):
        raise ValueError("read-only Bookends preview omitted live text, cwd, or recognized public CI collection")

    fixture_root = root / "bookends-preview-fixtures"
    fixture_root.mkdir()
    cases: dict[str, Any] = {}
    cases_to_make = {
        "missing-config": {"config": False},
        "missing-live-wording": {"live_requirement": False},
        "missing-ci-collection": {"recognized_ci": False},
        "wrong-ci-cwd": {"root_cwd": False},
        "missing-eligible-location": {"matching_location": False},
    }
    expected_fragments = {
        "missing-config": "bookends.toml is missing",
        "missing-live-wording": "no parsed live requirement text",
        "missing-ci-collection": "no recognized root-cwd collection",
        "wrong-ci-cwd": "no recognized root-cwd collection",
        "missing-eligible-location": "pathspecs match no tracked files",
    }
    for name, changes in cases_to_make.items():
        repo = fixture_root / name
        _write_preview_repo(repo, **changes)
        preview = _preview(journey, root, repo)
        missing = "\n".join(preview["missing_prerequisites"])
        if expected_fragments[name] not in missing:
            raise ValueError(f"{name} prerequisite was not reported actionably: {missing}")
        if preview["tests_executed"] is not False or preview["gate_executed"] is not False:
            raise ValueError(f"{name} preview demanded or ran unfinished behavior proof")
        first_job = next(
            (job for row in preview["proof_classes"] for job in row["ci_jobs"]),
            None,
        )
        observed_commands = first_job["observed_run_commands"] if first_job else []
        if name == "wrong-ci-cwd" and not any(
            row["working_directory"] == "nested" and row["recognized_collection"] is None
            for row in observed_commands
        ):
            raise ValueError(f"wrong-CWD preview did not show the actual cwd/collection refusal: {observed_commands!r}")
        cases[name] = {
            "evaluation_cwd": preview["evaluation_cwd"],
            "missing_prerequisites": preview["missing_prerequisites"],
            "live_requirement_count": len(preview["prd"]["live_requirements"]),
            "observed_ci_run_commands": observed_commands,
        }

    good_repo = fixture_root / "complete-prerequisites"
    _write_preview_repo(good_repo)
    good_preview = _preview(journey, root, good_repo)
    live = good_preview["prd"]["live_requirements"][0]
    if live["id"] != "LE-1" or not any("docs/contract.md" in link for link in live["explicit_references"]):
        raise ValueError(f"preview omitted accepted live text or its explicit reference: {live!r}")
    if good_preview["missing_prerequisites"]:
        raise ValueError(f"complete read-only prerequisite fixture reported gaps: {good_preview['missing_prerequisites']}")

    phase_proof = _phase_coverage_proof(root)
    setup_guidance = (
        journey.data_root
        / "crates/software-change-provider/skills/using-software-change-provider/SKILL.md"
    ).read_text(encoding="utf-8")
    for guidance in (
        "An owner-attested binding amendment changes only a future executor; it cannot raise or lower a frozen policy or criterion/goal author floor.",
        "If a floor or policy must change, start a fresh run from the corrected profile; do not use `amend-binding` as a policy edit.",
    ):
        if guidance not in setup_guidance:
            raise ValueError(f"profile procedure omitted frozen-floor/restart guidance: {guidance}")
    amend_action = amended.get("result", {}).get("action", {})
    binding_amendment_observation = {
        "kind": amend_action.get("kind"),
        "slot_id": amend_action.get("amendment", {}).get("slot_id"),
        "future_command": current_binding,
    }
    result = {
        "status": "passed",
        "scenario": "sol-profiles",
        "setup": {
            "selected_profile": str(selected_path),
            "selected_sha256": selected_hash,
            "effective_sha256": setup["output_sha256"],
            "selected_policy_stage": selected_axis["review_stage"],
            "selected_policy_required_authors": selected_axis["required_authors"],
            "selected_reviewer_assignments": len(selected_workers),
            "binding_amendment": binding_amendment_observation,
            "floor_before": expected_floor,
            "floor_after": after_floor,
            "refusals": ["profile-and-rigor-exclusive", "insufficient-roster", "known-legacy-shipped-profile-identities", "pre-wrapped-task-worker"],
            "bookends_off_has_no_overlay_ids": True,
            "advice_enabled": advice["enabled"],
            "occasion_map": advice["occasion_map"],
        },
        "bookends_preview": {
            "current_repository_cwd": current_preview["evaluation_cwd"],
            "current_live_requirements": len(current_preview["prd"]["live_requirements"]),
            "current_eligible_public_locations": current_e2e["eligible_public_locations"],
            "missing_prerequisite_cases": cases,
            "complete_fixture_references": live["explicit_references"],
        },
        "phase_coverage": phase_proof,
        "semantic_limit": "This scripted fixture proves operator mechanics, not owner acceptance, semantic requirement coverage, a live GREEN gate, or advisory quality.",
    }
    (root / "sol-profiles-result.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print("sol-profiles public setup/coverage/Bookends prerequisite assertions passed")
