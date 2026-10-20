"""Run every evals test once, as concurrent stock pytest groups, and prove the partition.

Not a test (no test_ prefix). From the repository root:

    python3 evals/software-change/tests/run_proof_groups.py --log-dir DIR [--baseline-ids FILE]

DIR must be driver-owned and outside the checkout (logs, summary.json). Each group is one stock
`uv run --project evals pytest` process. Waves run one after another; the groups of a wave run concurrently.

Why the waves are shaped this way (the constraints come from the tests, not from the runner):
- test_limit_proofs.py asserts that no container of the default image exists (`docker ps --filter ancestor=...`
  must be empty) and that no evalcore.publish process exists. Only pure host groups run beside it.
- test_fake_campaign.py and test_limits/test_provenance publish reports; test_fake_campaign asserts that no
  `evalctl campaign` or evalcore.publish process is left. It runs alone among campaign-running groups.
- test_lifecycle.py flaked once under load. It runs alone among Docker groups.
- Other Docker and campaign groups use only their own unique capture/report names and no fixed host port, so they
  may run together in the last wave.

Exit status is nonzero when a group fails, when a group's executed IDs differ from its collected IDs, when a
collected test is in no group or in two groups, or when the collected total differs from EXPECTED_TESTS.
"""
import argparse
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass, field
import json
from pathlib import Path
import os
import re
import subprocess
import sys
import time

REPO = Path(__file__).resolve().parents[3]
EXPECTED_TESTS = 699  # collect-only count at the baseline commit (perf/ids-before.txt)
OUTCOME = re.compile(r"^(PASSED|FAILED|ERROR|SKIPPED|XFAIL|XPASS) (\S.*)$")


@dataclass(frozen=True)
class Group:
    name: str
    files: tuple[str, ...]          # relative to evals/
    reason: str
    env: dict = field(default_factory=dict)


@dataclass(frozen=True)
class Wave:
    name: str
    reason: str
    groups: tuple[Group, ...]


WAVES = (
    Wave("limit-with-pure-host",
         "The limit proofs assert on every default-image container and on evalcore.publish processes, so no Docker or "
         "campaign group runs here. The pure host groups start neither.",
         (
             Group("limit", ("software-change/tests/test_limit_proofs.py",),
                   "limit proofs; TIME_SCALE scales the six proofs named in perf/limit-test-notes.md",
                   env={"TIME_SCALE": "0.5"}),
             Group("core-pure", ("core/tests/test_accounting.py", "core/tests/test_codex.py", "core/tests/test_pi.py",
                                 "core/tests/test_pilot.py", "core/tests/test_report.py", "core/tests/test_settings.py"),
                   "no Docker, no campaign, no publish child; temporary directories only"),
             Group("sc-seams", ("software-change/tests/test_target_seams.py",),
                   "fake sandboxes and monkeypatched docker and ReplayContainer; no real container"),
             Group("sc-pure", ("software-change/tests/test_arm_commands.py", "software-change/tests/test_arms_descriptor.py",
                               "software-change/tests/test_calibration.py", "software-change/tests/test_compare.py",
                               "software-change/tests/test_findings.py", "software-change/tests/test_grader.py",
                               "software-change/tests/test_image.py", "software-change/tests/test_image_reuse.py",
                               "software-change/tests/test_ledger.py", "software-change/tests/test_pi_session_and_phases.py",
                               "software-change/tests/test_pilot_inspection.py", "software-change/tests/test_provenance_merge.py",
                               "software-change/tests/test_provider_route.py", "software-change/tests/test_results.py",
                               "software-change/tests/test_scoring_common.py"),
                   "docker mocked or absent; merge, check and compare commands run no campaign"),
         )),
    Wave("lifecycle-alone",
         "test_lifecycle.py flaked once under load: it runs without any other Docker group.",
         (
             Group("lifecycle", ("software-change/tests/test_lifecycle.py",),
                   "campaigns and resumes in Docker; the only Docker group in this wave"),
         )),
    Wave("campaign-process-exclusive",
         "test_fake_campaign.py asserts that no evalctl campaign or evalcore.publish process is left, so no other "
         "campaign-running group runs here.",
         (
             Group("core-campaign", ("core/tests/test_fake_campaign.py", "core/tests/test_limits.py",
                                     "core/tests/test_provenance.py"),
                   "fake target campaigns; the publish and resume paths run in these processes only"),
         )),
    Wave("campaign-and-docker",
         "Each group uses unique capture and report names; none has a global process or container assertion.",
         (
             Group("target", ("software-change/tests/test_target.py",), "scripted campaigns in Docker"),
             Group("scoring", ("software-change/tests/test_scoring.py",), "ac5 demo campaigns and scorer cases"),
             Group("parity", ("software-change/tests/test_arms_parity.py",), "single-trial campaigns in Docker"),
             Group("continuation", ("software-change/tests/test_execution_continuation.py",),
                   "resume campaigns and Docker exec"),
             Group("judge-report", ("software-change/tests/test_judge_report.py",), "judge report campaign"),
             Group("profile-switch", ("software-change/tests/test_profile_switch.py",), "profile switch campaign"),
             Group("snapshots-trial", ("software-change/tests/test_snapshots_trial.py",),
                   "snapshot campaigns and replay in Docker"),
             Group("docker-capture-snapshots-adapters",
                   ("software-change/tests/test_capture.py", "software-change/tests/test_snapshots.py",
                    "software-change/tests/test_arms_adapters.py"),
                   "real replay containers and adapter trial; unique fixed capture paths"),
             Group("docker-cases-bridges",
                   ("software-change/tests/test_cases.py", "software-change/tests/test_image_bridge.py",
                    "software-change/tests/test_pi_bridge.py"),
                   "real case-check containers and bridge containers; unique fixed capture paths"),
         )),
)


def run(args, **kwargs):
    return subprocess.run(args, cwd=REPO, text=True, capture_output=True, **kwargs)


def normalize(test_id: str) -> str:
    return test_id.removeprefix("evals/")


def collect_all() -> list[str]:
    result = run(["uv", "run", "--project", "evals", "pytest", "--collect-only", "-q", "-p", "no:cacheprovider", "evals"])
    if result.returncode != 0:
        raise SystemExit("collect-only failed:\n" + result.stdout + result.stderr)
    return [normalize(line.strip()) for line in result.stdout.splitlines() if "::" in line]


def partition(collected: list[str], waves) -> dict[str, list[str]]:
    """Group name -> collected IDs of its files. Raises when a file is uncovered or covered twice."""
    owner = {}
    for wave in waves:
        for group in wave.groups:
            for file in group.files:
                if file in owner:
                    raise SystemExit(f"{file} is in two groups: {owner[file]} and {group.name}")
                if not (REPO / "evals" / file).is_file():
                    raise SystemExit(f"group {group.name} names a missing file: {file}")
                owner[file] = group.name
    members = {group.name: [] for wave in waves for group in wave.groups}
    for test_id in collected:
        file = test_id.split("::", 1)[0]
        if file not in owner:
            raise SystemExit(f"collected test is in no group: {test_id}")
        members[owner[file]].append(test_id)
    return members


def run_group(group: Group, expected: list[str], log_dir: Path) -> dict:
    env = {**os.environ, **group.env}
    if not group.env:
        env.pop("TIME_SCALE", None)  # only the limit group reads it
    command = ["uv", "run", "--project", "evals", "pytest", "-q", "-rA", "-p", "no:cacheprovider",
               *(f"evals/{file}" for file in group.files)]
    started = time.monotonic()
    result = subprocess.run(command, cwd=REPO, env=env, text=True, capture_output=True)
    wall = time.monotonic() - started
    text = result.stdout + result.stderr
    (log_dir / f"{group.name}.log").write_text(
        f"$ {' '.join(command)}\nTIME_SCALE={group.env.get('TIME_SCALE', '(unset)')}\nexit {result.returncode}\n\n" + text)
    summary_text = text.split("short test summary info", 1)[-1]
    expected_set = set(expected)
    outcomes: dict[str, str] = {}
    unexpected = []
    for line in summary_text.splitlines():
        match = OUTCOME.match(line.strip())
        if not match:
            continue
        rest = normalize(match.group(2))
        test_id = next((e for e in expected if rest == e or rest.startswith(e + " - ")), None)
        if test_id is None:
            unexpected.append(line.strip())
        elif test_id in outcomes:
            unexpected.append("duplicate result: " + line.strip())
        else:
            outcomes[test_id] = match.group(1)
    counts: dict[str, int] = {}
    for kind in outcomes.values():
        counts[kind] = counts.get(kind, 0) + 1
    missing = sorted(expected_set - set(outcomes))
    last_line = next((line for line in reversed(text.splitlines()) if re.search(r"\d+ (passed|failed|error)", line)), "")
    return {"group": group.name, "exit": result.returncode, "wall_seconds": round(wall, 1),
            "expected": len(expected), "executed": len(outcomes), "outcomes": counts,
            "missing": missing, "unexpected": unexpected, "summary_line": last_line.strip("= ").strip(),
            "log": str(log_dir / f"{group.name}.log"),
            "ok": result.returncode == 0 and not missing and not unexpected and len(outcomes) == len(expected)
                  and set(counts) <= {"PASSED"}}


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    parser.add_argument("--log-dir", required=True, type=Path, help="driver-owned directory outside the checkout")
    parser.add_argument("--baseline-ids", type=Path,
                        help="collect-only ID list at the baseline (ids-before.txt); the collected list must equal it")
    args = parser.parse_args(argv)
    log_dir = args.log_dir.resolve()
    if REPO in log_dir.parents or log_dir == REPO:
        raise SystemExit("--log-dir must be outside the checkout")
    log_dir.mkdir(parents=True, exist_ok=True)
    head = run(["git", "rev-parse", "HEAD"]).stdout.strip()
    dirty = run(["git", "status", "--porcelain"]).stdout.splitlines()
    print(f"HEAD {head}; checkout changes before the run: {len(dirty)}", flush=True)
    sync = run(["uv", "sync", "--project", "evals", "--locked"])
    if sync.returncode != 0:
        raise SystemExit("uv sync --locked failed:\n" + sync.stdout + sync.stderr)

    collected = collect_all()
    if len(collected) != EXPECTED_TESTS:
        raise SystemExit(f"collected {len(collected)} tests, expected {EXPECTED_TESTS}")
    if len(set(collected)) != len(collected):
        raise SystemExit("collect-only lists a test ID twice")
    if args.baseline_ids:
        baseline = [line.strip() for line in args.baseline_ids.read_text().splitlines() if "::" in line]
        if sorted(baseline) != sorted(collected):
            raise SystemExit("collect-only differs from the baseline ID list")
        print(f"collect-only equals the baseline list ({len(baseline)} IDs)", flush=True)
    members = partition(collected, WAVES)
    print(f"partition: {len(collected)} collected IDs in {sum(len(v) for v in members.values())} group slots, "
          f"each in exactly one group", flush=True)

    started = time.monotonic()
    report = {"head": head, "checkout_changes_before": dirty, "expected_tests": EXPECTED_TESTS,
              "waves": [], "groups": []}
    all_ok = True
    for wave in WAVES:
        print(f"\nwave {wave.name}: {wave.reason}", flush=True)
        wave_started = time.monotonic()
        with ThreadPoolExecutor(max_workers=len(wave.groups)) as pool:
            futures = [pool.submit(run_group, group, members[group.name], log_dir) for group in wave.groups]
            results = [future.result() for future in futures]
        wave_wall = time.monotonic() - wave_started
        for group, result in zip(wave.groups, results):
            result["reason"] = group.reason
            status = "ok" if result["ok"] else "FAILED"
            print(f"  {group.name:<36} {status:<6} exit={result['exit']} executed={result['executed']}/{result['expected']} "
                  f"{result['outcomes']} wall={result['wall_seconds']}s", flush=True)
            if not result["ok"]:
                all_ok = False
                print(f"    summary: {result['summary_line']}", flush=True)
                for test_id in result["missing"][:20]:
                    print(f"    not executed: {test_id}", flush=True)
                for line in result["unexpected"][:20]:
                    print(f"    unexpected: {line}", flush=True)
            report["groups"].append(result)
        report["waves"].append({"wave": wave.name, "wall_seconds": round(wave_wall, 1),
                                "groups": [g.name for g in wave.groups]})
    total_wall = time.monotonic() - started
    executed = sum(g["executed"] for g in report["groups"])
    serial = sum(g["wall_seconds"] for g in report["groups"])
    partition_ok = executed == len(collected) == EXPECTED_TESTS
    if not partition_ok:
        all_ok = False
    report.update({"collected": len(collected), "executed": executed, "total_wall_seconds": round(total_wall, 1),
                   "sum_of_group_wall_seconds": round(serial, 1), "ok": all_ok and partition_ok})
    (log_dir / "summary.json").write_text(json.dumps(report, indent=2) + "\n")
    print(f"\ntotal: collected={len(collected)} executed={executed} expected={EXPECTED_TESTS} "
          f"wall={total_wall:.0f}s (sum of group walls {serial:.0f}s) -> {'PASS' if report['ok'] else 'FAIL'}", flush=True)
    print(f"logs and summary: {log_dir}", flush=True)
    return 0 if report["ok"] else 1


if __name__ == "__main__":
    sys.exit(main())
