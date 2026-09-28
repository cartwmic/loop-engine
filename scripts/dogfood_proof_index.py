"""Public proof-index and finding-attribution fixture for AC-13/AC-14.

This runs only on disposable scripted inputs and a temporary real Git checkout.
It proves capture/index mechanics and keeps human merits and owner decisions
explicitly pending; it is not a semantic verdict or a historical source run.
"""
from __future__ import annotations

import copy
import hashlib
import json
import subprocess
import sys
import time
from pathlib import Path
from typing import Any

import dogfood_observation
from test_contract import repository_proof_identity


def _write(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


def _sha(raw: bytes) -> str:
    return "sha256:" + hashlib.sha256(raw).hexdigest()


def _git(root: Path, *args: str) -> str:
    result = subprocess.run(
        ["git", *args], cwd=root, text=True, capture_output=True, check=False
    )
    if result.returncode != 0:
        raise ValueError(f"fixture git {' '.join(args)} failed: {result.stderr.strip()}")
    return result.stdout.strip()


def _changed_paths(root: Path) -> list[str]:
    raw = subprocess.check_output(
        ["git", "status", "--porcelain=v1", "--untracked-files=all"], cwd=root, text=True
    )
    result = []
    for line in raw.splitlines():
        if len(line) < 4 or line[2] != " ":
            raise ValueError(f"unexpected fixture Git status record: {line!r}")
        result.append(line[3:])
    return result


def _current_plan(root: Path) -> dict[str, Any]:
    # One authoritative AC spine, with distinct task observations and commands.
    plan = {
        "revision": "future-proof-index-r1",
        "acceptance": [
            {"id": "AC-13", "statement": "Index current assertions separately from actual historical receipts."},
            {"id": "AC-14", "statement": "Keep factual correction, substantive reconsideration and owner exception distinct."},
        ],
        "tasks": [
            {
                "id": "proof-index-current-negative",
                "criterion_ids": ["AC-13"],
                "validation": [
                    {
                        "criterion_id": "AC-13",
                        "operator_path_observation": "A deliberately invalid current proof admission is rejected before it can enter the fixed passing index.",
                        "assertion": "The public checker refuses a zero-exit command with a missing asserted-outcome marker and refuses a nonzero historical receipt as current evidence.",
                        "proof_command_ids": ["ac13-current-negative"],
                        "historical_before_fix": {
                            "required_when_practical": True,
                            "practical": False,
                            "limitation": "This portable mechanics fixture does not rerun the pre-edit product binary; actual run-specific before-fix captures remain separate driver evidence.",
                            "nearest_current_negative_proof_command_id": "ac13-current-negative",
                            "capture": None,
                        },
                    }
                ],
            },
            {
                "id": "finding-attribution-examples",
                "criterion_ids": ["AC-14"],
                "validation": [
                    {
                        "criterion_id": "AC-14",
                        "operator_path_observation": "The driver preserves the original finding while recording a source-linked factual correction or a separately attributed reconsideration request.",
                        "assertion": "A cost-only or parent-pass-only argument remains a visible non-merits counterexample, and any residual owner exception remains pending until a scoped owner act exists.",
                        "proof_command_ids": ["ac14-merit-comparison"],
                        "owner_comparison": "For the owner, compare the unchanged original claim and cited source fact, the named driver-authored merits disposition, any separately authored focused independent reconsideration and status, and the remaining defect's consequence, repair choices and exact residual scope/reason versus ordinary completion.",
                    }
                ],
            },
        ],
        "proof_commands": [
            {
                "id": "ac13-current-negative",
                "command": sys.executable,
                "args": ["scripts/assert-index-fixture.py", "ac13"],
                "owner": "driver",
                "obligation": "Retain the distinct AC-13 current-negative assertion and its public checker receipt.",
                "expected_stdout_contains": ["ASSERTION: AC-13 current negative refused"],
            },
            {
                "id": "ac14-merit-comparison",
                "command": sys.executable,
                "args": ["scripts/assert-index-fixture.py", "ac14"],
                "owner": "driver",
                "obligation": "Retain the actor-attributed AC-14 comparison fixture for independent inspection.",
                "expected_stdout_contains": ["ASSERTION: AC-14 actor comparison retained"],
            },
        ],
    }
    _write(root / "plan.json", plan)
    return plan


def _validate_history_coverage(plan: dict[str, Any], root: Path) -> None:
    proof_ids = {row["id"] for row in plan["proof_commands"]}
    for task in plan["tasks"]:
        for case in task["validation"]:
            history = case.get("historical_before_fix")
            if not isinstance(history, dict) or not history.get("required_when_practical"):
                continue
            if history.get("practical"):
                capture = history.get("capture")
                path = Path(capture) if isinstance(capture, str) else Path()
                if not path.is_absolute():
                    path = root / path
                if not capture or not path.is_file():
                    raise ValueError(f"practical historical capture is missing: {path}")
            elif (
                not isinstance(history.get("limitation"), str)
                or not history["limitation"].strip()
                or history.get("nearest_current_negative_proof_command_id") not in proof_ids
            ):
                raise ValueError("impractical historical case needs a concrete limitation and nearest current negative")


def _merit_examples(root: Path) -> dict[str, Any]:
    fact_source = {
        "historical_receipt": "The fixture's historical receipt exits 7 and is not a current command capture.",
        "scope": "proof-index fixture only",
    }
    _write(root / "fact-source.json", fact_source)
    historical_digest = _sha((root / "fact-source.json").read_bytes())
    assertion_digest = _sha((root / "plan.json").read_bytes())
    examples = {
        "classification": "synthetic-review-fixture-not-a-product-finding",
        "factual_error": {
            "finding_id": "fixture-factual-error",
            "original_author": {"name": "fixture-reviewer-a", "kind": "agent"},
            "claim": "The historical negative receipt passed as a current proof command.",
            "source": {"locator": "fact-source.json#/historical_receipt", "sha256": historical_digest},
            "driver_disposition": {
                "action": "reject-factual-error",
                "author": {"name": "fixture-driver", "kind": "script"},
                "reason": "The cited historical receipt exits 7 and is absent from the current index.",
            },
        },
        "substantive_dispute": {
            "finding_id": "fixture-substantive-dispute",
            "original_author": {"name": "fixture-reviewer-b", "kind": "agent"},
            "claim": "The source-linked assertion is sufficient to establish the user outcome.",
            "source": {"locator": "plan.json#/tasks/0/validation/0/assertion", "sha256": assertion_digest},
            "driver_action": {
                "action": "request-focused-independent-reconsideration",
                "author": {"name": "fixture-driver", "kind": "script"},
                "reason": "Whether this exact assertion establishes the user outcome is a substantive dispute; preserve the original claim and request focused independent review.",
            },
            "reconsideration": {
                "status": "scripted-attribution-only-not-semantic-review",
                "author": {"name": "fixture-independent-reviewer", "kind": "script"},
                "action": "reassess-original-claim-against-original-evidence",
                "evidence": {"locator": "plan.json#/tasks/0/validation/0/assertion", "sha256": assertion_digest},
                "result": "unresolved",
                "original_finding_rewritten": False,
            },
        },
        "non_merits_counterexample": {
            "claim": "Reject the finding because rerunning costs time and the parent review passed.",
            "author": {"name": "fixture-driver", "kind": "script"},
            "classification": "reviewable-non-merits-counterexample",
            "accepted_as_disposition": False,
            "human_merits_review": "pending",
        },
        "scoped_residual": {
            "status": "illustrative-only-no-owner-act",
            "owner": {"name": "fixture-owner", "kind": "script"},
            "finding_id": "fixture-substantive-dispute",
            "scope": "one synthetic report-entry field only",
            "reason": "Example shape only; no actual residual is accepted.",
            "owner_attestation": {"status": "not-performed"},
            "completion_mode": "exceptional-only-if-a-real-scoped-owner-act-exists",
            "ordinary_completion": False,
        },
        "semantic_limit": "Fixture actor labels and source bytes do not establish truthful human merits or owner attestation.",
    }
    _write(root / "finding-attribution-fixtures.json", examples)
    return examples


def _checker_case(
    *, root: Path, checkout: Path, label: str, report: dict[str, Any],
    matrix: dict[str, Any], index: dict[str, Any], expected: int,
    diagnostic: str | None = None,
) -> dict[str, Any]:
    case = root / "checker-cases" / label
    case.mkdir(parents=True)
    report_path = case / "implementation-report.json"
    matrix_path = case / "proof-matrix.json"
    _write(report_path, report)
    _write(matrix_path, matrix)
    _write(case / "proof" / "receipts.json", index)
    argv = [
        sys.executable,
        str(checkout / "scripts" / "assert-implementation-report.py"),
        "--report", str(report_path),
        "--revision", str(report["revision"]),
        "--plan-revision", str(report["plan_revision"]),
        "--matrix", str(matrix_path),
    ]
    started = time.time()
    monotonic = time.monotonic()
    completed = subprocess.run(argv, cwd=checkout, capture_output=True, timeout=30, check=False)
    receipt = {
        "argv": argv,
        "cwd": str(checkout),
        "started_at": started,
        "finished_at": time.time(),
        "wall_seconds": time.monotonic() - monotonic,
        "exit_code": completed.returncode,
    }
    (case / "stdout").write_bytes(completed.stdout)
    (case / "stderr").write_bytes(completed.stderr)
    _write(case / "cli-receipt.json", receipt)
    if completed.returncode != expected:
        raise ValueError(
            f"checker {label} expected exit {expected}, got {completed.returncode}: "
            f"{completed.stderr.decode('utf-8', 'replace')}"
        )
    error = completed.stderr.decode("utf-8", "replace")
    if diagnostic and diagnostic not in error:
        raise ValueError(f"checker {label} omitted {diagnostic!r}: {error}")
    return {"case": label, "exit_code": completed.returncode, "capture": str(case)}


def proof_index_case(journey) -> None:
    root = dogfood_observation._fresh_root(journey, "sol-proof-index")
    checkout = root / "checkout"
    scripts = checkout / "scripts"
    scripts.mkdir(parents=True)
    repository_scripts = Path(__file__).resolve().parent
    for name in ("assert-implementation-report.py", "test_contract.py"):
        (scripts / name).write_bytes((repository_scripts / name).read_bytes())
    _git(checkout, "init", "-q")
    _git(checkout, "config", "user.name", "Proof index fixture")
    _git(checkout, "config", "user.email", "proof-index@example.invalid")
    _git(checkout, "config", "core.hooksPath", "/dev/null")
    (checkout / ".gitignore").write_text("__pycache__/\n", encoding="utf-8")

    assertion_script = scripts / "assert-index-fixture.py"
    assertion_script.write_text(
        "import hashlib,json,sys\n"
        "from pathlib import Path\n"
        "root=Path(__file__).resolve().parents[1]\n"
        "plan=json.loads((root/'plan.json').read_text())\n"
        "if sys.argv[1]=='ac13':\n"
        " a=plan['tasks'][0]['validation'][0]; assert a['criterion_id']=='AC-13'; assert a['proof_command_ids']==['ac13-current-negative']; assert 'historical_before_fix' in a; assert a['assertion'] != plan['tasks'][1]['validation'][0]['assertion']; print('ASSERTION: AC-13 current negative refused; actual historical receipts remain outside current evidence')\n"
        "elif sys.argv[1]=='ac14':\n"
        " a=plan['tasks'][1]['validation'][0]; x=json.loads((root/'finding-attribution-fixtures.json').read_text()); assert a['criterion_id']=='AC-14'; assert all(term in a['owner_comparison'].lower() for term in ('owner','original claim','source fact','driver-authored','independent','consequence','repair choices','residual')); fact=x['factual_error']; dispute=x['substantive_dispute']; assert fact['source']['locator']=='fact-source.json#/historical_receipt'; assert fact['source']['sha256']=='sha256:'+hashlib.sha256((root/'fact-source.json').read_bytes()).hexdigest(); assert fact['driver_disposition']['author']['name']=='fixture-driver'; assert dispute['source']['locator']=='plan.json#/tasks/0/validation/0/assertion'; assert dispute['source']['sha256']=='sha256:'+hashlib.sha256((root/'plan.json').read_bytes()).hexdigest(); assert dispute['reconsideration']['evidence']==dispute['source']; assert dispute['reconsideration']['author']!={'name':'fixture-driver','kind':'script'}; assert dispute['reconsideration']['status']=='scripted-attribution-only-not-semantic-review'; assert not dispute['reconsideration']['original_finding_rewritten']; nm=x['non_merits_counterexample']; assert nm['classification']=='reviewable-non-merits-counterexample'; assert 'costs time' in nm['claim'] and 'parent review passed' in nm['claim']; assert not nm['accepted_as_disposition']; assert x['scoped_residual']['scope']=='one synthetic report-entry field only'; assert x['scoped_residual']['owner_attestation']['status']=='not-performed'; assert x['scoped_residual']['completion_mode']=='exceptional-only-if-a-real-scoped-owner-act-exists'; assert not x['scoped_residual']['ordinary_completion']; print('ASSERTION: AC-14 actor comparison retained; semantic and owner judgments pending')\n"
        "else: raise SystemExit('unknown fixture assertion')\n",
        encoding="utf-8",
    )
    plan = _current_plan(checkout)
    examples = _merit_examples(checkout)
    _validate_history_coverage(plan, checkout)
    missing_history_plan = copy.deepcopy(plan)
    missing_history = missing_history_plan["tasks"][0]["validation"][0]["historical_before_fix"]
    missing_history["practical"] = True
    missing_history["capture"] = "historical/missing-before-fix-receipt.json"
    missing_capture_refusal = ""
    try:
        _validate_history_coverage(missing_history_plan, checkout)
    except ValueError as error:
        missing_capture_refusal = str(error)
    if "practical historical capture is missing" not in missing_capture_refusal:
        raise ValueError("proof plan did not fail closed for a missing practical historical capture")
    _write(root / "negative-cases" / "missing-practical-history.json", {
        "status": "refused",
        "diagnostic": missing_capture_refusal,
        "case_specific_limitation": plan["tasks"][0]["validation"][0]["historical_before_fix"]["limitation"],
    })
    index_fixture = {
        "schema_version": 1,
        "plan_revision": plan["revision"],
        "current_command_evidence_ids": [
            "validation-r1-command-ac13-current-negative",
            "validation-r1-command-ac14-merit-comparison",
        ],
        "criteria": [
            {"criterion_id": "AC-13", "current_proof_command_ids": ["ac13-current-negative"], "fresh_verdicts": "pending-independent-review"},
            {"criterion_id": "AC-14", "current_proof_command_ids": ["ac14-merit-comparison"], "fresh_verdicts": "pending-independent-review"},
        ],
        "goal_verdicts": "pending-independent-review",
        "historical_receipts_are_current_evidence": False,
    }
    _write(root / "fixed-validation-index-fixture.json", index_fixture)

    # This is deliberately a labeled mechanics-only failure, not an actual old
    # product receipt. The real before-fix captures are separately retained by
    # the driver and must never be backfilled or relabeled by this scenario.
    historical_fixture = {
        "classification": "synthetic-negative-for-index-separation-test-only",
        "exit_code": 7,
        "timed_out": False,
        "spawn_error": None,
        "current_command_evidence_id": None,
        "may_enter_passing_current_index": False,
    }
    _write(root / "synthetic-noncurrent-negative.json", historical_fixture)
    if historical_fixture["exit_code"] == 0 or historical_fixture["may_enter_passing_current_index"]:
        raise ValueError("noncurrent failure fixture was promoted into passing current evidence")
    if any("synthetic-noncurrent" in item for item in index_fixture["current_command_evidence_ids"]):
        raise ValueError("historical negative fixture leaked into current command evidence IDs")

    _git(checkout, "add", ".gitignore", "scripts", "plan.json", "fact-source.json", "finding-attribution-fixtures.json")
    _git(checkout, "commit", "-q", "-m", "future plan and proof fixtures")
    head = _git(checkout, "rev-parse", "HEAD")
    (checkout / "tracked.txt").write_text("current proof target\n", encoding="utf-8")
    commands = plan["proof_commands"]
    initial_identity = repository_proof_identity(checkout)
    receipts = []
    captures = []
    for spec in commands:
        argv = [spec["command"], *spec["args"]]
        started = time.time()
        monotonic = time.monotonic()
        completed = subprocess.run(argv, cwd=checkout, capture_output=True, timeout=30, check=False)
        after = time.time()
        after_identity = repository_proof_identity(checkout)
        stdout = completed.stdout.decode("utf-8", "replace")
        stderr = completed.stderr.decode("utf-8", "replace")
        if completed.returncode != 0 or any(marker not in stdout for marker in spec["expected_stdout_contains"]):
            raise ValueError(f"current proof command {spec['id']} failed its assertion: {stdout!r} {stderr!r}")
        if repository_proof_identity(checkout) != initial_identity:
            raise ValueError("current proof command changed the stable fixture Git identity")
        capture_dir = root / "current-captures" / spec["id"]
        capture_dir.mkdir(parents=True)
        (capture_dir / "stdout").write_bytes(completed.stdout)
        (capture_dir / "stderr").write_bytes(completed.stderr)
        receipt = {
            "id": spec["id"],
            "argv": argv,
            "cwd": str(checkout),
            "started_at": started,
            "finished_at": after,
            "wall_seconds": time.monotonic() - monotonic,
            "exit_code": completed.returncode,
            "timed_out": False,
            "spawn_error": None,
            "stdout": str(capture_dir / "stdout"),
            "stderr": str(capture_dir / "stderr"),
            "repository_before": initial_identity,
            "repository_after": after_identity,
        }
        receipt_path = capture_dir / "receipt.json"
        _write(receipt_path, receipt)
        receipts.append({"id": spec["id"], "receipt": str(receipt_path)})
        captures.append({
            "proof_command_id": spec["id"],
            "assertion": spec["expected_stdout_contains"][0],
            "command_evidence_id": f"validation-r1-command-{spec['id']}",
            "receipt": str(receipt_path),
            "repository_identity": initial_identity,
        })

    index_path = root / "current-captures" / "receipts-index.json"
    _write(index_path, {"plan_revision": plan["revision"], "target_directory": str(checkout / "target"), "commands": receipts})
    current_ids = [item["id"] for item in receipts]
    fixed_ids = index_fixture["current_command_evidence_ids"]
    expected_fixed_ids = [f"validation-r1-command-{proof_id}" for proof_id in current_ids]
    task_proofs: dict[str, list[str]] = {}
    for task in plan["tasks"]:
        for validation in task["validation"]:
            task_proofs[validation["criterion_id"]] = validation["proof_command_ids"]
    indexed_proofs = {
        row["criterion_id"]: row["current_proof_command_ids"]
        for row in index_fixture["criteria"]
    }
    if (
        len(current_ids) != len(set(current_ids))
        or fixed_ids != expected_fixed_ids
        or task_proofs != indexed_proofs
        or any(proof_id not in current_ids for ids in indexed_proofs.values() for proof_id in ids)
    ):
        raise ValueError("plan assertions, current proof commands and fixed validation index disagree")

    matrix = {
        "schema_version": 1,
        "plan_revision": plan["revision"],
        "local_final": [
            {"id": row["id"], "command": row["command"], "args": row["args"], "expected_stdout_contains": row["expected_stdout_contains"]}
            for row in commands
        ],
        "post_report": [{"id": "implementation-report-check"}],
        "after_separate_authorization": [],
    }
    changed = _changed_paths(checkout)
    report = {
        "revision": "implementation-r1",
        "plan_revision": plan["revision"],
        "coverage": {"commit": head + "+uncommitted-worktree"},
        "changed_surface": changed,
        "validation": [
            {"proof": "ac13-current-negative: passed"},
            {"proof": "ac14-merit-comparison: passed"},
            {"criterion_id": "AC-13", "proof": "See captured AC-13 current-negative assertion; fresh independent verdicts remain pending."},
            {"criterion_id": "AC-14", "proof": "See actor-attributed fixture comparison; semantic reconsideration and owner exception remain pending."},
        ],
    }
    base_index = {"plan_revision": plan["revision"], "target_directory": str(checkout / "target"), "commands": receipts}
    checker = _checker_case(
        root=root, checkout=checkout, label="current-positive", report=report,
        matrix=matrix, index=base_index, expected=0,
    )

    negative_cases = []
    missing_assertion = copy.deepcopy(matrix)
    missing_assertion["local_final"][0]["expected_stdout_contains"] = ["ASSERTION: deliberately absent"]
    negative_cases.append(_checker_case(
        root=root, checkout=checkout, label="green-exit-without-current-assertion", report=report,
        matrix=missing_assertion, index=base_index, expected=1, diagnostic="required local proof is failed",
    ))

    stale_index = copy.deepcopy(base_index)
    stale_receipt_path = root / "negative-receipts" / "stale.json"
    stale_receipt = json.loads(Path(receipts[0]["receipt"]).read_text(encoding="utf-8"))
    stale_receipt["repository_before"] = "sha256:" + "0" * 64
    _write(stale_receipt_path, stale_receipt)
    stale_index["commands"][0]["receipt"] = str(stale_receipt_path)
    negative_cases.append(_checker_case(
        root=root, checkout=checkout, label="stale-current-tree-identity", report=report,
        matrix=matrix, index=stale_index, expected=1, diagnostic="repository_before mismatch",
    ))

    wrong_target_index = copy.deepcopy(base_index)
    wrong_target_path = root / "negative-receipts" / "wrong-target.json"
    wrong_target = json.loads(Path(receipts[0]["receipt"]).read_text(encoding="utf-8"))
    wrong_target["cwd"] = str(root)
    _write(wrong_target_path, wrong_target)
    wrong_target_index["commands"][0]["receipt"] = str(wrong_target_path)
    negative_cases.append(_checker_case(
        root=root, checkout=checkout, label="wrong-target-cwd", report=report,
        matrix=matrix, index=wrong_target_index, expected=1, diagnostic="receipt cwd mismatch",
    ))

    missing_index = copy.deepcopy(base_index)
    missing_index["commands"] = missing_index["commands"][1:]
    negative_cases.append(_checker_case(
        root=root, checkout=checkout, label="missing-current-indexed-assertion", report=report,
        matrix=matrix, index=missing_index, expected=1, diagnostic="required local proof is pending",
    ))

    old_as_current_matrix = copy.deepcopy(matrix)
    old_as_current_matrix["local_final"].append({
        "id": "historical-before-fix", "command": sys.executable,
        "args": ["-c", "raise SystemExit(7)"],
    })
    old_as_current_report = copy.deepcopy(report)
    old_as_current_report["validation"].insert(2, {"proof": "historical-before-fix: passed"})
    negative_cases.append(_checker_case(
        root=root, checkout=checkout, label="nonzero-historical-receipt-cannot-pass-current-index",
        report=old_as_current_report, matrix=old_as_current_matrix, index=base_index,
        expected=1, diagnostic="historical-before-fix: required local proof is pending",
    ))

    proof = {
        "schema_version": 1,
        "scenario": "sol-proof-index",
        "status": "passed-scripted-proof-index-mechanics",
        "future_plan": str(checkout / "plan.json"),
        "plan_revision": plan["revision"],
        "current_tree": {
            "head": head,
            "repository_identity": initial_identity,
            "changed_surface": changed,
            "captures": captures,
            "fixed_validation_command_evidence_ids": fixed_ids,
            "historical_negative_current_evidence_id": None,
        },
        "public_report_checker": checker,
        "fail_closed_cases": negative_cases,
        "missing_practical_history_refusal": str(root / "negative-cases" / "missing-practical-history.json"),
        "finding_attribution_fixture": str(checkout / "finding-attribution-fixtures.json"),
        "historical_limit": "The synthetic noncurrent exit-7 fixture tests index separation only; it is not an actual product before-fix receipt. Actual historical captures remain separate driver-owned evidence.",
        "semantic_limit": "No real finding merits judgment, focused independent reconsideration, owner residual exception or final criterion/goal verdict was performed.",
        "fresh_independent_verdict_relationship": {
            "current_index_is_fixed_before_review": True,
            "required_ac_verdicts": ["AC-13", "AC-14"],
            "verdict_status": "pending-independent-review",
            "required_goal_verdict": "pending-independent-review",
            "only_current_command_evidence_is_eligible": True,
        },
    }
    _write(root / "sol-proof-index-proof.json", proof)
    print("sol-proof-index public CLI passed: distinct assertions, actual fixture Git identities, historical/current separation and merit actor captures retained")
    print("sol-proof-index semantic limit: independent merits/reconsideration, owner exception and final verdicts remain pending")
