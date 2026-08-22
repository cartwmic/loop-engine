#!/usr/bin/env python3
"""Assert the aggregate implementation report matches repository state and plan."""

from __future__ import annotations

import argparse
import copy
import json
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any, NoReturn, Sequence

ROOT = Path(__file__).resolve().parents[1]

# Plan revision 6 final-validation-and-report command matrix, stored as Python
# data so quoted pipefail commands are not re-encoded through a shell.
FINAL_MATRIX_COMMANDS = [
    "cargo test --workspace",
    "env -u BOOKENDS_BYPASS scripts/bookends-check-gate.sh",
    "isolated temporary GIT_INDEX_FILE with all worktree paths staged, env -u BOOKENDS_BYPASS scripts/bookends-check-gate.sh",
    "cargo clippy --workspace --all-targets -- -D warnings",
    "cargo fmt --all -- --check",
    "cargo build --locked -p loop-cli -p software-change-provider -p policy-document-provider -p research-provider -p bookends-check",
    "cargo test -p bookends-check --offline",
    "cargo test -p software-change-provider --offline",
    r"bash -o pipefail -c 'python3 scripts/software-change-journey.py --self-test | tee /tmp/loop-engine-software-change-self-test.log && rg -q ^worker-data\ skill/root\ policy\ assertions\ passed$ /tmp/loop-engine-software-change-self-test.log'",
    "for mode in draft audit; do python3 scripts/policy-document-journey.py --engine target/debug/loop-engine --provider target/debug/policy-document --profile crates/policy-document-provider/data/readme.json --mode $mode; done",
    "python3 scripts/research-journey.py --self-test",
    "python3 scripts/research-journey.py --mode source --engine target/debug/loop-engine --provider target/debug/research --profile crates/research-provider/data/configs/standard.json",
    "research packaged public journey with an empty temporary data-root: python3 scripts/research-journey.py --mode packaged --engine target/debug/loop-engine --provider target/debug/research --data-root <empty temporary directory> --profile standard.json",
    "python3 scripts/generate-prd-journey.py --self-test",
    "python3 scripts/assert-generate-prd-profile.py",
    r"bash -o pipefail -c 'env -u BOOKENDS_BYPASS python3 scripts/software-change-journey.py --mode source --engine target/debug/loop-engine --provider target/debug/software-change --data-root $PWD --work-root ${TMPDIR:-/tmp}/loop-engine-software-change-journey --profile crates/software-change-provider/data/configs/high-rigor.json --traversal-depth full | tee /tmp/loop-engine-software-change-source.log && rg -q contracted\ fan-out\ failure /tmp/loop-engine-software-change-source.log && rg -q bookends-enabled\ external\ scenario\ passed: /tmp/loop-engine-software-change-source.log && rg -q LE-2\ topology\ scenarios\ passed:\ malformed\ rejected,\ cyclic\ topology\ accepted /tmp/loop-engine-software-change-source.log && rg -q LE-11\ frozen-run\ scenario\ passed:\ changed\ describe\ did\ not\ alter\ show /tmp/loop-engine-software-change-source.log && rg -q LE-12\ unsupported-action\ scenario\ passed:\ explicit\ and\ operational\ errors\ preserved\ state /tmp/loop-engine-software-change-source.log && rg -q LE-13\ final-state\ scenario\ passed:\ outgoing\ transition\ rejected\ before\ run\ creation /tmp/loop-engine-software-change-source.log && rg -q LE-14\ initially-final\ scenario\ passed:\ run\ created\ final /tmp/loop-engine-software-change-source.log && rg -q LE-15\ terminal-mutation\ scenario\ passed:\ append/event/terminate\ rejected\ without\ history\ change /tmp/loop-engine-software-change-source.log'",
    "software-change packaged public journey with an empty temporary data-root: python3 scripts/software-change-journey.py --mode packaged --engine target/debug/loop-engine --provider target/debug/software-change --data-root <empty temporary directory> --work-root <temporary directory> --profile high-rigor.json --traversal-depth checked-prefix",
    "python3 scripts/generate-prd-journey.py --mode source --engine target/debug/loop-engine --provider target/debug/research --checker target/debug/bookends-check --work-root ${TMPDIR:-/tmp}/loop-engine-generate-prd-journey --profile crates/research-provider/data/configs/generate-prd.json",
    "python3 scripts/assert-push-main-preflight.py",
    "dist generate --check",
    "dist plan --output-format=json > /tmp/loop-engine-dist-plan.json",
    "python3 scripts/assert-dist-plan.py --self-test",
    "python3 scripts/assert-dist-plan.py /tmp/loop-engine-dist-plan.json",
    "python3 scripts/assert-release-gates.py",
    "python3 scripts/assert-implementation-report.py --self-test",
    "python3 scripts/assert-implementation-report.py --report /Users/cartwmic/.local/share/loop-engine/runs/run-1787371334879759000-1-70458/implementation-report.json --revision 5 --plan-revision 6",
]
PROOF_MARKERS = [
    "worker-data skill/root policy assertions passed",
    "contracted fan-out failure",
    "bookends-enabled external scenario passed: show instructions, missing/empty/tombstoned IDs, RED, and BYPASS",
    "LE-2 topology scenarios passed: malformed rejected, cyclic topology accepted",
    "LE-11 frozen-run scenario passed: changed describe did not alter show",
    "LE-12 unsupported-action scenario passed: explicit and operational errors preserved state",
    "LE-13 final-state scenario passed: outgoing transition rejected before run creation",
    "LE-14 initially-final scenario passed: run created final",
    "LE-15 terminal-mutation scenario passed: append/event/terminate rejected without history change",
    "LE-68 exact CI wiring proof: source/preflight and packaged/archive-smoke assertions passed",
    "LE-69 exact release proof: cargo-dist plan and release-gate assertions passed",
    "prior exhaustive/independent 90/90 Bookends audit retained",
    "implementation-report provider schema check: passed",
    "current and isolated-index Bookends gates returned GREEN with BOOKENDS_BYPASS unset",
    "GREEN",
    "terminal end reached",
]
EXPECTED_VALIDATION = [f"{command}: passed" for command in FINAL_MATRIX_COMMANDS] + list(
    PROOF_MARKERS
)


class ReportError(RuntimeError):
    """The implementation report does not match its frozen contract."""


def fail(message: str) -> NoReturn:
    raise SystemExit(f"implementation-report assertion failed: {message}")


def git_output(args: Sequence[str]) -> str:
    try:
        return subprocess.check_output(
            ["git", *args],
            cwd=ROOT,
            stderr=subprocess.PIPE,
            text=True,
        )
    except (OSError, subprocess.CalledProcessError) as error:
        raise ReportError(
            f"could not read repository state with git {' '.join(args)}: {error}"
        ) from error


def porcelain_changed_paths(status: str) -> list[str]:
    changed_paths: list[str] = []
    for line in status.splitlines():
        if not line:
            continue
        if len(line) < 4 or line[2] != " ":
            raise ReportError(f"unexpected git status --porcelain=v1 record: {line!r}")
        changed_paths.append(line[3:])
    return changed_paths


def repository_state() -> tuple[str, list[str]]:
    head = git_output(["rev-parse", "HEAD"]).strip()
    if not head:
        raise ReportError("git rev-parse HEAD returned an empty commit")
    changed_paths = porcelain_changed_paths(
        git_output(["status", "--porcelain=v1", "--untracked-files=all"])
    )
    return head, changed_paths


def load_report(path: Path) -> dict[str, Any]:
    try:
        report = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ReportError(f"could not read {path}: {error}") from error
    if not isinstance(report, dict):
        raise ReportError("report must be a JSON object")
    return report


def assert_equal(label: str, actual: Any, expected: Any) -> None:
    if actual != expected:
        raise ReportError(f"{label} mismatch: expected {expected!r}, got {actual!r}")


def check_report(
    path: Path,
    *,
    revision: str,
    plan_revision: str,
    head: str,
    changed_paths: list[str],
) -> None:
    report = load_report(path)
    assert_equal("revision", report.get("revision"), revision)
    assert_equal("plan_revision", report.get("plan_revision"), plan_revision)

    coverage = report.get("coverage")
    if not isinstance(coverage, dict):
        raise ReportError("coverage must be a JSON object")
    assert_equal("coverage.commit", coverage.get("commit"), f"{head}+uncommitted-worktree")
    assert_equal("changed_surface", report.get("changed_surface"), changed_paths)
    assert_equal("validation", report.get("validation"), EXPECTED_VALIDATION)


def write_report(path: Path, report: dict[str, Any]) -> None:
    path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")


def self_test() -> int:
    head, changed_paths = repository_state()
    revision = "self-test-revision"
    plan_revision = "self-test-plan-revision"
    passing = {
        "revision": revision,
        "author": {"name": "self-test", "kind": "script"},
        "plan_revision": plan_revision,
        "coverage": {
            "commit": f"{head}+uncommitted-worktree",
            "documents": [{"path": "README.md", "revision": "self-test"}],
        },
        "summary": "Temporary passing implementation report.",
        "changed_surface": changed_paths,
        "validation": EXPECTED_VALIDATION,
    }

    if EXPECTED_VALIDATION != [f"{command}: passed" for command in FINAL_MATRIX_COMMANDS] + list(
        PROOF_MARKERS
    ):
        raise AssertionError("expected passed-string list drifted from matrix and markers")
    pipefail = [command for command in FINAL_MATRIX_COMMANDS if command.startswith("bash -o pipefail")]
    if len(pipefail) != 2:
        raise AssertionError(f"expected two quoted pipefail commands, got {len(pipefail)}")
    if r"^worker-data\ skill/root\ policy\ assertions\ passed$" not in pipefail[0]:
        raise AssertionError("self-test pipefail command lost Python-data backslash quoting")
    if r"contracted\ fan-out\ failure" not in pipefail[1]:
        raise AssertionError("source-journey pipefail command lost Python-data backslash quoting")
    if r"bookends-enabled\ external\ scenario\ passed:" not in pipefail[1]:
        raise AssertionError("source-journey pipefail command lost the enabled-Bookends marker")
    for marker in (
        r"LE-2\ topology\ scenarios\ passed:\ malformed\ rejected,\ cyclic\ topology\ accepted",
        r"LE-11\ frozen-run\ scenario\ passed:\ changed\ describe\ did\ not\ alter\ show",
        r"LE-12\ unsupported-action\ scenario\ passed:\ explicit\ and\ operational\ errors\ preserved\ state",
        r"LE-13\ final-state\ scenario\ passed:\ outgoing\ transition\ rejected\ before\ run\ creation",
        r"LE-14\ initially-final\ scenario\ passed:\ run\ created\ final",
        r"LE-15\ terminal-mutation\ scenario\ passed:\ append/event/terminate\ rejected\ without\ history\ change",
    ):
        if marker not in pipefail[1]:
            raise AssertionError(f"source-journey pipefail command lost focused marker {marker}")

    tampered: list[tuple[str, dict[str, Any]]] = []

    wrong_revision = copy.deepcopy(passing)
    wrong_revision["revision"] = "tampered"
    tampered.append(("revision", wrong_revision))

    wrong_plan_revision = copy.deepcopy(passing)
    wrong_plan_revision["plan_revision"] = "tampered"
    tampered.append(("plan-revision", wrong_plan_revision))

    wrong_commit = copy.deepcopy(passing)
    wrong_commit["coverage"]["commit"] = "tampered+uncommitted-worktree"
    tampered.append(("commit", wrong_commit))

    wrong_surface = copy.deepcopy(passing)
    wrong_surface["changed_surface"] = [*changed_paths, "tampered/path"]
    tampered.append(("changed-surface", wrong_surface))

    wrong_order = copy.deepcopy(passing)
    wrong_order["validation"][0], wrong_order["validation"][1] = (
        wrong_order["validation"][1],
        wrong_order["validation"][0],
    )
    tampered.append(("ordering", wrong_order))

    omission = copy.deepcopy(passing)
    omission["validation"] = omission["validation"][:-1]
    tampered.append(("omission", omission))

    extra = copy.deepcopy(passing)
    extra["validation"].append("unexpected validation: passed")
    tampered.append(("extra", extra))

    with tempfile.TemporaryDirectory(prefix="implementation-report-self-test-") as temp:
        report_path = Path(temp) / "implementation-report.json"
        write_report(report_path, passing)
        try:
            check_report(
                report_path,
                revision=revision,
                plan_revision=plan_revision,
                head=head,
                changed_paths=changed_paths,
            )
        except ReportError as error:
            raise AssertionError(f"self-test rejected passing report: {error}") from error

        rejected: list[str] = []
        for label, report in tampered:
            write_report(report_path, report)
            try:
                check_report(
                    report_path,
                    revision=revision,
                    plan_revision=plan_revision,
                    head=head,
                    changed_paths=changed_paths,
                )
            except ReportError:
                rejected.append(label)
            else:
                raise AssertionError(f"self-test accepted {label} tamper")

    expected_rejections = [label for label, _ in tampered]
    if rejected != expected_rejections:
        raise AssertionError(
            f"self-test rejection coverage mismatch: expected {expected_rejections}, got {rejected}"
        )
    print(
        "implementation-report assertion self-test passed: passing report accepted; "
        + ", ".join(rejected)
        + " tampers rejected"
    )
    return 0


def parse_args(argv: Sequence[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--report", type=Path)
    parser.add_argument("--revision")
    parser.add_argument("--plan-revision")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)
    if args.self_test:
        if args.report is not None or args.revision is not None or args.plan_revision is not None:
            parser.error("--self-test cannot be combined with report arguments")
    elif args.report is None or args.revision is None or args.plan_revision is None:
        parser.error("--report, --revision, and --plan-revision are required")
    return args


def main(argv: Sequence[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    if args.self_test:
        return self_test()

    try:
        head, changed_paths = repository_state()
        check_report(
            args.report,
            revision=args.revision,
            plan_revision=args.plan_revision,
            head=head,
            changed_paths=changed_paths,
        )
    except ReportError as error:
        fail(str(error))
    print(
        "implementation-report assertions passed: "
        f"revision={args.revision} plan_revision={args.plan_revision} "
        f"commit={head}+uncommitted-worktree changed_paths={len(changed_paths)}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
