"""Deterministic public CLI proof for assert-implementation-report.py.

Only tiny scripted commands are executed. No production matrix row is run or
claimed complete by these fixtures. Kept separate from the checker so fixtures
cannot become a second policy inventory.
"""
from __future__ import annotations

import copy
import json
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

from test_contract import ROOT, repository_proof_identity


def write(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


def prove_committed(root: Path) -> list[dict]:
    """Drive the copied public checker against real clean fixture commits."""
    repo = root / "checkout"
    (repo / "scripts").mkdir(parents=True)
    for name in ("assert-implementation-report.py", "test_contract.py"):
        shutil.copy2(ROOT / "scripts" / name, repo / "scripts" / name)
    (repo / ".gitignore").write_text("__pycache__/\n")

    def git(*args: str) -> str:
        return subprocess.check_output(["git", *args], cwd=repo, text=True, stderr=subprocess.PIPE).strip()

    git("init", "-q")
    git("config", "user.name", "Checker fixture")
    git("config", "user.email", "checker@example.invalid")
    git("config", "core.hooksPath", "/dev/null")
    (repo / "a.txt").write_text("before\n")
    (repo / "removed.txt").write_text("removed by delivery\n")
    git("add", ".")
    git("commit", "-qm", "baseline")
    baseline = git("rev-parse", "HEAD")
    (repo / "a.txt").write_text("after\n")
    (repo / "removed.txt").unlink()
    (repo / "z.txt").write_text("delivered\n")
    git("add", "-A")
    git("commit", "-qm", "delivery")
    head = git("rev-parse", "HEAD")
    unrelated = git("commit-tree", "HEAD^{tree}", "-m", "unrelated fixture commit")
    identity = repository_proof_identity(repo)
    argv = [sys.executable, "-c", "print('committed fixture proof')"]
    started = time.time()
    proc = subprocess.run(argv, cwd=repo, capture_output=True, check=True)
    finished = time.time()
    (root / "stdout").write_bytes(proc.stdout)
    (root / "stderr").write_bytes(proc.stderr)
    receipt = {"id": "committed-proof", "argv": argv, "cwd": str(repo),
               "started_at": started, "finished_at": finished, "wall_seconds": finished - started,
               "exit_code": proc.returncode, "timed_out": False, "spawn_error": None,
               "stdout": str(root / "stdout"), "stderr": str(root / "stderr"),
               "repository_before": identity, "repository_after": repository_proof_identity(repo)}
    matrix = {"schema_version": 1, "plan_revision": "5", "local_final": [
        {"id": "committed-proof", "command": argv[0], "args": argv[1:]}],
        "post_report": [{"id": "implementation-report-check"}], "after_separate_authorization": []}
    report = {"revision": "committed", "plan_revision": "5", "coverage": {"commit": head},
              "changed_surface": ["a.txt", "removed.txt", "z.txt"],
              "validation": [{"proof": "committed-proof: passed"}]}
    results = []

    def case(label: str, expected: int, *, base=baseline, value=None, missing=False, stale=False, diagnostic=None):
        directory = root / label
        directory.mkdir()
        write(directory / "report.json", report if value is None else value)
        write(directory / "matrix.json", matrix)
        actual = copy.deepcopy(receipt)
        if stale:
            actual["repository_before"] = "sha256:stale"
        write(directory / "receipt.json", actual)
        write(directory / "proof/receipts.json", {"plan_revision": "5", "target_directory": str(repo / "target"),
              "commands": [] if missing else [{"id": "committed-proof", "receipt": str(directory / "receipt.json")}]})
        command = [sys.executable, str(repo / "scripts/assert-implementation-report.py"),
                   "--report", str(directory / "report.json"), "--revision", "committed", "--plan-revision", "5",
                   "--matrix", str(directory / "matrix.json"), "--baseline-commit", base]
        result = subprocess.run(command, cwd=repo, text=True, capture_output=True, timeout=30)
        (directory / "stdout").write_text(result.stdout)
        (directory / "stderr").write_text(result.stderr)
        write(directory / "cli-receipt.json", {"argv": command, "cwd": str(repo), "exit_code": result.returncode})
        assert result.returncode == expected, (label, result.returncode, result.stderr)
        assert diagnostic is None or diagnostic in result.stderr, (label, result.stderr)
        results.append({"case": label, "expected_exit": expected, "actual_exit": result.returncode, "capture": str(directory)})

    case("committed-positive", 0)
    value = copy.deepcopy(report)
    value["coverage"]["commit"] += "+uncommitted-worktree"
    case("committed-wrong-head", 1, value=value, diagnostic="coverage.commit mismatch")
    value = copy.deepcopy(report)
    value["changed_surface"] = list(reversed(value["changed_surface"]))
    case("committed-wrong-path-order", 1, value=value, diagnostic="changed_surface mismatch")
    case("committed-short-baseline", 1, base=baseline[:7], diagnostic="full commit SHA")
    case("committed-unknown-baseline", 1, base="0" * 40)
    case("committed-nonancestor", 1, base=unrelated, diagnostic="ancestor")
    case("committed-empty-delivery", 1, base=head, diagnostic="no changed paths")
    case("committed-missing-proof", 1, missing=True, diagnostic="required local proof is pending")
    case("committed-stale-proof", 1, stale=True, diagnostic="repository_before mismatch")
    (repo / "a.txt").write_text("uncommitted correction\n")
    case("committed-dirty-tree", 1, diagnostic="clean checkout")
    return results


def prove(root: Path) -> int:
    head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    changed = [line[3:] for line in subprocess.check_output(
        ["git", "status", "--porcelain=v1", "--untracked-files=all"], cwd=ROOT, text=True
    ).splitlines()]
    identity = repository_proof_identity(ROOT)
    # Names exercise final measurement obligations, but the script explicitly
    # labels its output synthetic. The real benchmark comparison stays T14-owned.
    ids = ["focused-proof", "benchmark-final-serial", "benchmark-final-parallel", "benchmark-compare"]
    later = ["commit-and-push", "hosted-exact-commit-preflight", "later-real-dogfood"]
    matrix = {"schema_version": 1, "plan_revision": "5", "local_final": [
        {"id": i, "command": sys.executable,
         "args": ["-c", "import sys; print('synthetic proof completed'); print('diagnostic capture', file=sys.stderr)"],
         "expected_stdout_contains": ["synthetic proof completed"]} for i in ids
    ], "post_report": [{"id": "implementation-report-check"}, {"id": "bootstrap-implementation-checkpoint"}],
        "after_separate_authorization": [{"id": i, "status": "pending separate authorization"} for i in later]}
    report = {"revision": "fixture-current", "plan_revision": "5", "author": {"name": "fixture", "kind": "script"},
              "coverage": {"commit": head + "+uncommitted-worktree"}, "changed_surface": changed,
              "summary": "Scripted checker proof only; not final measurements or hosted proof.",
              "validation": [{"proof": i + ": passed"} for i in ids] + [{"proof": i + ": pending"} for i in later]}
    # Derive the public row contract from shipped data, not a checker-only fixture.
    profile = json.loads((ROOT / "crates/software-change-provider/data/configs/high-rigor.json").read_text())
    row_schema = profile["artifact_schemas"]["implementation-report.json"]["properties"]["validation"]["items"]
    assert row_schema["type"] == "object" and row_schema["required"] == ["proof"]
    assert row_schema["additionalProperties"] is False
    assert set(row_schema["properties"]) == {"proof", "criterion_id"}

    def execute(row: dict, directory: Path) -> Path:
        directory.mkdir(parents=True, exist_ok=True)
        argv = [row["command"], *row["args"]]
        before = repository_proof_identity(ROOT)
        start, monotonic = time.time(), time.monotonic()
        proc = subprocess.run(argv, cwd=ROOT, capture_output=True, timeout=10)
        elapsed, finish = time.monotonic() - monotonic, time.time()
        (directory / "stdout").write_bytes(proc.stdout)
        (directory / "stderr").write_bytes(proc.stderr)
        path = directory / "receipt.json"
        write(path, {"id": row["id"], "argv": argv, "cwd": str(ROOT),
                     "started_at": start, "finished_at": finish, "wall_seconds": elapsed,
                     "exit_code": proc.returncode, "timed_out": False, "spawn_error": None,
                     "stdout": "stdout", "stderr": "stderr",
                     "repository_before": before, "repository_after": repository_proof_identity(ROOT)})
        return path

    receipts = [execute(row, root / "commands" / row["id"]) for row in matrix["local_final"]]
    index = {"plan_revision": "5", "target_directory": str(ROOT / "target"),
             "commands": [{"id": i, "receipt": str(path)} for i, path in zip(ids, receipts)]}
    results = []

    def case(label: str, expected: int, *, r=None, m=None, ix=None, receipt_change=None, diagnostic=None):
        directory = root / "cases" / label
        directory.mkdir(parents=True)
        effective_index = copy.deepcopy(index if ix is None else ix)
        if receipt_change:
            row_index, mutation = receipt_change
            raw = json.loads(receipts[row_index].read_text())
            # Keep original streams visible when a receipt is relocated.
            raw["stdout"] = str(receipts[row_index].parent / "stdout")
            raw["stderr"] = str(receipts[row_index].parent / "stderr")
            mutation(raw)
            altered = directory / "altered-receipt.json"
            write(altered, raw)
            effective_index["commands"][row_index]["receipt"] = str(altered)
        write(directory / "matrix.json", matrix if m is None else m)
        write(directory / "report.json", report if r is None else r)
        write(directory / "proof" / "receipts.json", effective_index)
        argv = [sys.executable, str(ROOT / "scripts/assert-implementation-report.py"),
                "--report", str(directory / "report.json"), "--revision", "fixture-current",
                "--plan-revision", "5", "--matrix", str(directory / "matrix.json")]
        proc = subprocess.run(argv, cwd=ROOT, capture_output=True, text=True, timeout=30)
        (directory / "stdout").write_text(proc.stdout)
        (directory / "stderr").write_text(proc.stderr)
        write(directory / "cli-receipt.json", {"argv": argv, "cwd": str(ROOT), "exit_code": proc.returncode})
        if proc.returncode != expected or (diagnostic and diagnostic not in proc.stderr):
            raise AssertionError(f"{label}: expected exit {expected}, diagnostic {diagnostic!r}; got {proc.returncode}: {proc.stderr}")
        results.append({"case": label, "expected_exit": expected, "actual_exit": proc.returncode,
                        "capture": str(directory)})

    case("current-plan-positive", 0)
    value = copy.deepcopy(report)
    value["validation"].append({"criterion_id": "AC-1", "proof": "Retained criterion evidence; not a matrix receipt."})
    case("canonical-criterion-note", 0, r=value)
    value = copy.deepcopy(report)
    value["validation"][0]["criterion_id"] = "AC-1"
    case("criterion-note-cannot-replace-matrix-proof", 1, r=value, diagnostic="validation ids/order mismatch")
    value = copy.deepcopy(report)
    value["validation"] = [row["proof"] for row in value["validation"]]
    case("noncanonical-string-rows", 1, r=value, diagnostic="validation rows must")
    for label, row in [("missing-proof", {}), ("nonstring-proof", {"proof": 3}),
                       ("unknown-row-field", {"proof": "focused-proof: passed", "extra": True}),
                       ("invalid-criterion-id", {"proof": "note", "criterion_id": "AC-0"})]:
        value = copy.deepcopy(report)
        value["validation"].append(row)
        case(label, 1, r=value, diagnostic="validation row")
    for field in ("revision", "plan_revision"):
        value = copy.deepcopy(report)
        value[field] = "wrong"
        case("wrong-" + field, 1, r=value, diagnostic=field + " mismatch")
    value = copy.deepcopy(report)
    value["coverage"]["commit"] = head
    case("wrong-head-suffix", 1, r=value, diagnostic="coverage.commit mismatch")
    value = copy.deepcopy(report)
    value["changed_surface"] = [*changed, "wrong/path"]
    case("wrong-path", 1, r=value, diagnostic="changed_surface mismatch")
    value = copy.deepcopy(report)
    # The live worktree has multiple entries. Also retain a deterministic pure
    # order check if the checkout is clean when the reusable self-test runs.
    value["changed_surface"] = list(reversed(changed)) if len(changed) > 1 else ["b", "a"]
    case("wrong-path-order", 1, r=value, diagnostic="changed_surface mismatch")
    value = copy.deepcopy(matrix)
    value["plan_revision"] = "old"
    case("wrong-matrix-revision", 1, m=value, diagnostic="matrix plan_revision mismatch")
    value = copy.deepcopy(report)
    value["validation"] = [{"proof": "all tests passed; benchmark speedup achieved"}]
    case("report-only-success-string", 1, r=value, diagnostic="invalid validation claim")
    value = copy.deepcopy(index)
    value["commands"] = []
    case("success-claims-without-execution", 1, ix=value, diagnostic="required local proof is pending")
    for i in ids[1:]:
        value = copy.deepcopy(index)
        value["commands"] = [row for row in value["commands"] if row["id"] != i]
        case("absent-" + i, 1, ix=value, diagnostic=i + ": required local proof is pending")
    value = copy.deepcopy(report)
    value["validation"][0]["proof"] = "focused-proof: pending"
    missing = copy.deepcopy(index)
    missing["commands"] = missing["commands"][1:]
    case("honest-local-pending", 1, r=value, ix=missing, diagnostic="required local proof is pending")
    missing_receipt = copy.deepcopy(index)
    missing_receipt["commands"][1]["receipt"] = str(root / "absent-receipt.json")
    case("missing-collection-receipt-file", 1, ix=missing_receipt, diagnostic="could not read")
    for position in (1, 2):
        case("missing-collection-stdout-" + ids[position], 1,
             receipt_change=(position, lambda r: r.update(stdout=str(root / "absent"))),
             diagnostic="missing stdout capture")
    for stream in ("stdout", "stderr"):
        case("missing-" + stream, 1, receipt_change=(3, lambda r, key=stream: r.update({key: str(root / "absent")})), diagnostic="missing " + stream + " capture")
    case("wrong-argv", 1, receipt_change=(0, lambda r: r.update(argv=["true"])), diagnostic="receipt argv mismatch")
    case("wrong-cwd", 1, receipt_change=(0, lambda r: r.update(cwd="/")), diagnostic="receipt cwd mismatch")
    case("stale-tree-same-paths", 1, receipt_change=(0, lambda r: r.update(repository_before="sha256:old")), diagnostic="repository_before mismatch")
    case("tree-changed-during-command", 1, receipt_change=(0, lambda r: r.update(repository_after="sha256:changed")), diagnostic="repository_after mismatch")
    case("timeout", 1, receipt_change=(0, lambda r: r.update(timed_out=True)), diagnostic="required local proof is failed")
    case("spawn-error", 1, receipt_change=(0, lambda r: r.update(exit_code=None, spawn_error="not found")), diagnostic="required local proof is failed")
    case("overlapping-matrix", 1, receipt_change=(1, lambda r: r.update(started_at=0)), diagnostic="not serialized")
    value = copy.deepcopy(matrix)
    value["local_final"][0]["expected_stdout_contains"] = ["marker not emitted by script"]
    case("missing-required-marker", 1, m=value, diagnostic="required local proof is failed")
    for state in ("passed", "failed"):
        value = copy.deepcopy(report)
        value["validation"][-2]["proof"] = "hosted-exact-commit-preflight: " + state
        case("hosted-not-local-" + state, 1, r=value, diagnostic="after-authorization")
    value = copy.deepcopy(report)
    value["validation"].append({"proof": "implementation-report-check: passed"})
    case("circular-report-claim", 1, r=value, diagnostic="validation ids/order mismatch")
    for label, extra in (("duplicate", index["commands"][0]),
                         ("unknown", {"id": "old-task-proof", "receipt": str(receipts[0])}),
                         ("hosted", {"id": later[1], "receipt": str(receipts[0])})):
        value = copy.deepcopy(index)
        value["commands"].append(extra)
        case(label + "-receipt-id", 1, ix=value, diagnostic="unknown, non-local or duplicate ids")
    # Actual nonzero benchmark-comparison command, not just a rewritten receipt.
    failed_matrix = copy.deepcopy(matrix)
    failed_matrix["local_final"][-1]["args"] = ["-c", "import sys; print('synthetic comparison: no improvement'); sys.exit(7)"]
    failed_path = execute(failed_matrix["local_final"][-1], root / "failed-comparison")
    failed_index = copy.deepcopy(index)
    failed_index["commands"][-1]["receipt"] = str(failed_path)
    speedup_claim = copy.deepcopy(report)
    speedup_claim["summary"] = "Claimed benchmark speedup: all comparisons passed."
    case("nonzero-comparison-despite-speedup-claim", 1, r=speedup_claim, m=failed_matrix, ix=failed_index, diagnostic="benchmark-compare: required local proof is failed")
    failed_report = copy.deepcopy(report)
    failed_report["validation"][3]["proof"] = "benchmark-compare: failed"
    case("honest-failed-comparison", 1, r=failed_report, m=failed_matrix, ix=failed_index, diagnostic="benchmark-compare: required local proof is failed")
    results.extend(prove_committed(root / "committed"))
    assert repository_proof_identity(ROOT) == identity, "proof changed maintained checkout"
    write(root / "outcomes.json", {"scope": "scripted public checker mechanics only", "cases": results,
                                    "production_matrix_executed": False, "after_authorization": "pending"})
    print(f"implementation-report assertion self-test passed: {len(results)} public CLI cases; real scripted receipts; missing final measurement captures and nonzero comparison refused; after-authorization pending")
    return 0


def self_test(output: Path | None = None) -> int:
    if output is not None:
        output = output.resolve()
        if output == ROOT or ROOT in output.parents:
            raise ValueError("self-test output must be outside the maintained checkout")
        output.mkdir(parents=True, exist_ok=False)
        result = prove(output)
        print(f"public checker proof retained: {output}")
        return result
    with tempfile.TemporaryDirectory(prefix="implementation-report-proof-") as temp:
        return prove(Path(temp))
