"""Run every evals test exactly once, as concurrent stock pytest units, and prove the partition.

Not a test (no test_ prefix). From the repository root:

    python3 evals/software-change/tests/run_proof_groups.py --log-dir DIR [--baseline-ids FILE] [--jobs N]

DIR must be driver-owned and outside the checkout (logs, summary.json). Each unit is one stock
`uv run --project evals pytest` process over one or more selectors (a test file, or one test with all its parameters, or
one parameter). Units start longest first; at most --jobs run at once (default 6). Nothing is retried.

Why the units are shaped this way (the constraints come from the tests, not from the runner):
- Several tests were written to assert on the whole machine. Each of those now checks only its own campaign's containers
  or processes, so its units can run beside any other unit. The machine-wide checks run once, in the final sweep below.
- test_arms_parity.py and test_snapshots_trial.py start their campaigns together inside one process, so their files stay
  one unit. test_execution_continuation.py's four-carried test is one production chain and stays one unit.
- Ports: the sandbox bridges listen inside their containers, so no unit binds a host port.

After every unit has finished, one serial sweep asserts the machine-wide properties the tests no longer assert one by one:
no container of the default image is left, and no evalcore.publish or `evalctl campaign` process is left.

Exit status is nonzero when a unit fails, when a unit's executed IDs differ from its collected IDs, when a collected test
is in no unit or in two units, when a --baseline-ids test is no longer collected (a removed test), or when the sweep finds
a leftover. Collected tests that the baseline does not list are new: they are counted as extra, reported, and must run too.
"""
import argparse
from concurrent.futures import ThreadPoolExecutor, as_completed
from dataclasses import dataclass, field
import json
from pathlib import Path
import os
import re
import subprocess
import sys
import time

REPO = Path(__file__).resolve().parents[3]
OUTCOME = re.compile(r"^(PASSED|FAILED|ERROR|SKIPPED|XFAIL|XPASS) (\S.*)$")
DEFAULT_JOBS = 6
LIMIT_ENV = {"TIME_SCALE": "0.5"}  # the limit proofs' scaled values (perf/limit-test-notes.md)
SC = "software-change/tests/"
CORE = "core/tests/"


@dataclass(frozen=True)
class Unit:
    name: str
    group: str                    # the reporting group (a test file, or the name of a family of files)
    selectors: tuple[str, ...]    # relative to evals/: a file, `file::test` (all its parameters) or `file::test[param]`
    seconds: float                # expected serial seconds from perf/profile.md; orders the schedule, longest first
    env: dict = field(default_factory=dict)


def unit(name, group, *selectors, seconds, env=None):
    return Unit(name, group, tuple(selectors), float(seconds), dict(env or {}))


def test(file, name):
    return f"{SC}{file}::{name}"


LIMIT = f"{SC}test_limit_proofs.py"
UNITS = (
    # limit proofs: each runs alone in its own process; the machine-wide checks are not made here.
    unit("limit-baseline-normal-dollar", "limit", test("test_limit_proofs.py", "test_normal_scripted_trial_runs_to_a_scoreable_capture_and_reports_inside_the_cap"),
         test("test_limit_proofs.py", "test_dollar_stop_fires_below_the_cap_during_the_trial_and_the_judge_is_never_called"), seconds=140, env=LIMIT_ENV),
    unit("limit-unpriced", "limit", test("test_limit_proofs.py", "test_unpriced_model_with_a_dollar_budget_is_refused_before_any_container_or_call"), seconds=3, env=LIMIT_ENV),
    unit("limit-per-trial", "limit", test("test_limit_proofs.py", "test_per_trial_stop_is_a_limit_with_kept_capture_and_no_leftovers"), seconds=80, env=LIMIT_ENV),
    unit("limit-campaign-wall", "limit", test("test_limit_proofs.py", "test_campaign_wall_time_stop_still_captures_scores_judges_nothing_and_reports_inside_the_cap"), seconds=81, env=LIMIT_ENV),
    unit("limit-render", "limit", test("test_limit_proofs.py", "test_blocking_render_leaves_the_limit_only_record_and_the_command_still_ends_by_the_cap"), seconds=120, env=LIMIT_ENV),
    unit("limit-docker-inspection", "limit", test("test_limit_proofs.py", "test_blocked_docker_inspection_ends_by_the_cap_with_a_limit_before_any_trial"), seconds=62, env=LIMIT_ENV),
    unit("limit-cut-off-capture", "limit", test("test_limit_proofs.py", "test_cut_off_capture_still_leaves_the_isolation_evidence"), seconds=137, env=LIMIT_ENV),
    unit("limit-slow-scoring", "limit", test("test_limit_proofs.py", "test_slow_scoring_is_abandoned_at_its_allowance_with_the_capture_kept"), seconds=136, env=LIMIT_ENV),
    unit("limit-stuck-replay", "limit", test("test_limit_proofs.py", "test_a_stuck_replay_container_removal_cannot_push_publication_past_the_cap"), seconds=140, env=LIMIT_ENV),
    # core
    unit("core-campaign", "core-campaign", f"{CORE}test_fake_campaign.py", seconds=122),
    unit("core-limits-provenance", "core-pure", f"{CORE}test_limits.py", f"{CORE}test_provenance.py", seconds=3),
    unit("core-pure", "core-pure", f"{CORE}test_accounting.py", f"{CORE}test_codex.py", f"{CORE}test_pi.py",
         f"{CORE}test_pilot.py", f"{CORE}test_report.py", f"{CORE}test_settings.py", seconds=14),
    # software-change, no Docker
    unit("sc-seams", "sc-seams", f"{SC}test_target_seams.py", seconds=13),
    unit("sc-pure", "sc-pure", f"{SC}test_arm_commands.py", f"{SC}test_arms_descriptor.py", f"{SC}test_calibration.py",
         f"{SC}test_compare.py", f"{SC}test_findings.py", f"{SC}test_grader.py", f"{SC}test_image.py", f"{SC}test_image_reuse.py",
         f"{SC}test_ledger.py", f"{SC}test_pi_session_and_phases.py", f"{SC}test_pilot_inspection.py",
         f"{SC}test_provenance_merge.py", f"{SC}test_provider_route.py", f"{SC}test_results.py", f"{SC}test_scoring_common.py",
         seconds=100),
    # lifecycle: one unit per test (the file has no shared state between its tests)
    unit("lifecycle-merge-rescored", "lifecycle", test("test_lifecycle.py", "test_merge_rescored_captures_and_resume_only_missing_trial"), seconds=149),
    unit("lifecycle-resume-infrastructure", "lifecycle", test("test_lifecycle.py", "test_campaign_resume_reruns_only_infrastructure_and_merges"), seconds=99),
    unit("lifecycle-capture-first-fault", "lifecycle", test("test_lifecycle.py", "test_capture_first_fault_rescore_and_same_capture_parity"), seconds=87),
    unit("lifecycle-resume-continues-campaign", "lifecycle", test("test_lifecycle.py", "test_resume_continues_restored_db_owner_answers_and_excludes_headline[True]"), seconds=61),
    unit("lifecycle-resume-continues-trial", "lifecycle", test("test_lifecycle.py", "test_resume_continues_restored_db_owner_answers_and_excludes_headline[False]"), seconds=62),
    unit("lifecycle-refused-report", "lifecycle", test("test_lifecycle.py", "test_refused_report_keeps_private_capture_and_recovery_command"), seconds=38),
    unit("lifecycle-terminated-run", "lifecycle", test("test_lifecycle.py", "test_terminated_run_scores_hidden_delivery_without_success"), seconds=28),
    # target: one unit per test; the controlled-driver parameters are separate units
    unit("target-role-routing", "target", test("test_target.py", "test_real_pi_role_routing_swaps_models_in_reports"), seconds=146),
    unit("target-driver-stops-before-limit", "target", test("test_target.py", "test_driver_stops_before_trial_limit_so_the_outcome_is_recorded"), seconds=79),
    unit("target-engine-progress-outlives-idle", "target", test("test_target.py", "test_real_pi_engine_progress_outlives_idle_limit"), seconds=74),
    unit("target-role-token-progress-outlives-idle", "target", test("test_target.py", "test_real_pi_role_token_progress_outlives_idle_limit"), seconds=62),
    unit("target-capture-read-in-parts", "target", test("test_target.py", "test_capture_is_read_in_parts"), seconds=58),
    unit("target-hang-after-terminal", "target", test("test_target.py", "test_real_pi_hang_after_terminal_is_still_a_failure"), seconds=50),
    unit("target-never-asks", "target", test("test_target.py", "test_controlled_drivers_reach_terminal_without_building[never-asks-0]"), seconds=42),
    unit("target-asks-but-does-not-build", "target", test("test_target.py", "test_controlled_drivers_reach_terminal_without_building[asks-but-does-not-build-3]"), seconds=44),
    unit("target-engine-output-larger", "target", test("test_target.py", "test_engine_output_larger_than_exec_limit_is_read_from_files"), seconds=36),
    unit("target-judge-failure", "target", test("test_target.py", "test_judge_failure_does_not_fail_the_trial"), seconds=35),
    unit("target-idle-driver-stopped", "target", test("test_target.py", "test_idle_real_pi_driver_is_stopped_and_captured"), seconds=30),
    unit("target-nonterminal-exit", "target", test("test_target.py", "test_real_pi_nonterminal_driver_exit_retains_evidence"), seconds=24),
    unit("target-quick", "target",
         test("test_target.py", "test_owner_reveals_only_asked_requirement_and_keeps_text_host_side"),
         test("test_target.py", "test_hidden_copy_requires_terminal"),
         test("test_target.py", "test_owner_gate_cli_message_is_clear_without_launch"),
         test("test_target.py", "test_real_settings_record_owner_selection_without_any_generation"),
         test("test_target.py", "test_progress_reading_tolerates_non_json"),
         test("test_target.py", "test_progress_does_not_initialize_a_database_before_driver_start"),
         test("test_target.py", "test_dropped_provider_connection_is_infrastructure"), seconds=3),
    # scoring
    unit("scoring-ac5-demo", "scoring", test("test_scoring.py", "test_public_ac5_demo_separates_recall_and_preserves_terminal_copy_gate"), seconds=188),
    unit("scoring-units", "scoring",
         test("test_scoring.py", "test_collection_mismatch_cannot_award_delivery_credit"),
         test("test_scoring.py", "test_durable_health_counts_denials_revise_overrides_overruns_termination"),
         test("test_scoring.py", "test_reference_diff_scope_and_successful_push_measurement"),
         test("test_scoring.py", "test_honesty_checks_hidden_requirement_goal_claims_and_local_evidence"),
         test("test_scoring.py", "test_latest_revision_review_clearance_not_earlier_failed_review"),
         test("test_scoring.py", "test_finalized_role_flag_and_timeout_use_authoritative_core_accounting"), seconds=10),
    # parity: one process, its campaigns start together (test_arms_parity.py)
    unit("parity", "parity", f"{SC}test_arms_parity.py", seconds=150),
    # continuation
    unit("continuation-four-carried", "continuation", test("test_execution_continuation.py", "test_four_carried_failures_and_exactly_two_new_assignments"), seconds=337),
    unit("continuation-deadline-free-public", "continuation", test("test_execution_continuation.py", "test_deadline_free_public_campaign_survives_old_limits"), seconds=87),
    unit("continuation-quick", "continuation",
         test("test_execution_continuation.py", "test_exception_is_narrow_and_explicit"),
         test("test_execution_continuation.py", "test_deadline_free_pi_exchange_still_cancels_and_cleans_helper"),
         test("test_execution_continuation.py", "test_driver_launcher_cancellation_kills_descendants"), seconds=3),
    unit("judge-report", "judge-report", f"{SC}test_judge_report.py", seconds=81),
    unit("profile-switch", "profile-switch", f"{SC}test_profile_switch.py", seconds=157),
    # snapshots: one process, its on and off campaigns start together (test_snapshots_trial.py)
    unit("snapshots-trial", "snapshots-trial", f"{SC}test_snapshots_trial.py", seconds=110),
    # docker: capture, snapshots, adapters
    unit("dcsa-arms-adapters", "docker-capture-snapshots-adapters", f"{SC}test_arms_adapters.py", seconds=125),
    unit("dcsa-capture", "docker-capture-snapshots-adapters", f"{SC}test_capture.py", seconds=6),
    unit("dcsa-snapshots", "docker-capture-snapshots-adapters", f"{SC}test_snapshots.py", seconds=6),
    # docker: cases and bridges
    unit("dcb-image-bridge-role", "docker-cases-bridges", test("test_image_bridge.py", "test_real_pi_role_bridge_and_owner_service"), seconds=68),
    unit("dcb-image-bridge-codex", "docker-cases-bridges", test("test_image_bridge.py", "test_codex_scripted_stream_through_real_pi_bridge"), seconds=25),
    unit("dcb-pi-bridge", "docker-cases-bridges", f"{SC}test_pi_bridge.py", seconds=74),
    unit("dcb-cases", "docker-cases-bridges", f"{SC}test_cases.py", seconds=60),
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


def covers(selector: str, test_id: str) -> bool:
    """A file selector covers its tests; a function selector covers its parameters; a parameter selector covers one ID."""
    if "::" not in selector:
        return test_id.startswith(selector + "::")
    if test_id == selector:
        return True
    return "[" not in selector and test_id.startswith(selector + "[")


def partition(collected: list[str], units) -> dict[str, list[str]]:
    """Unit name -> collected IDs it covers. Raises when a selector names nothing, a test is covered by no unit or by two."""
    names = [u.name for u in units]
    if len(set(names)) != len(names):
        raise SystemExit("unit names must be unique")
    for u in units:
        for selector in u.selectors:
            file = selector.split("::", 1)[0]
            if not (REPO / "evals" / file).is_file():
                raise SystemExit(f"unit {u.name} names a missing file: {file}")
    members = {u.name: [] for u in units}
    for test_id in collected:
        owners = [u.name for u in units if any(covers(s, test_id) for s in u.selectors)]
        if not owners:
            raise SystemExit(f"collected test is in no unit: {test_id}")
        if len(owners) > 1:
            raise SystemExit(f"collected test is in two units: {test_id}: {owners}")
        members[owners[0]].append(test_id)
    for name, ids in members.items():
        if not ids:
            raise SystemExit(f"unit {name} covers no collected test")
    return members


def new_test_ids(collected: list[str], baseline: list[str]) -> list[str]:
    """Collected IDs that the baseline does not list (new tests).

    A baseline ID that is no longer collected is a removed test, and is refused.
    """
    removed = sorted(set(baseline) - set(collected))
    if removed:
        raise SystemExit(f"{len(removed)} baseline test(s) are no longer collected, e.g. {removed[0]}")
    return sorted(set(collected) - set(baseline))


def run_unit(u: Unit, expected: list[str], log_dir: Path, origin: float) -> dict:
    env = {key: value for key, value in os.environ.items() if key != "TIME_SCALE"}  # only the limit units set it
    env.update(u.env)
    command = ["uv", "run", "--project", "evals", "pytest", "-q", "-rA", "-p", "no:cacheprovider",
               *(f"evals/{selector}" for selector in u.selectors)]
    started = time.monotonic() - origin
    result = subprocess.run(command, cwd=REPO, env=env, text=True, capture_output=True)
    ended = time.monotonic() - origin
    text = result.stdout + result.stderr
    (log_dir / f"{u.name}.log").write_text(
        f"$ {' '.join(command)}\nTIME_SCALE={u.env.get('TIME_SCALE', '(unset)')}\nexit {result.returncode}\n\n" + text)
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
    return {"unit": u.name, "group": u.group, "selectors": list(u.selectors), "exit": result.returncode,
            "start_seconds": round(started, 1), "end_seconds": round(ended, 1), "wall_seconds": round(ended - started, 1),
            "expected": len(expected), "executed": len(outcomes), "outcomes": counts,
            "missing": missing, "unexpected": unexpected, "summary_line": last_line.strip("= ").strip(),
            "log": str(log_dir / f"{u.name}.log"),
            "ok": result.returncode == 0 and not missing and not unexpected and len(outcomes) == len(expected)
                  and set(counts) <= {"PASSED"}}


def sweep(log_dir: Path) -> dict:
    """The one machine-wide check, after every unit has ended: no default-image container, no publish or campaign process."""
    image = run(["uv", "run", "--project", "evals", "python", "-c", "from scevals.image import DEFAULT_IMAGE; print(DEFAULT_IMAGE)"])
    if image.returncode != 0:
        raise SystemExit("could not read DEFAULT_IMAGE:\n" + image.stdout + image.stderr)
    default_image = image.stdout.strip()
    containers = run(["docker", "ps", "-aq", "--filter", f"ancestor={default_image}"]).stdout.split()
    publish = run(["pgrep", "-f", "evalcore.publish"]).stdout.split()
    campaigns = run(["pgrep", "-f", "evalctl campaign"]).stdout.split()
    result = {"image": default_image, "containers_left": containers, "publish_processes_left": publish,
              "campaign_processes_left": campaigns,
              "ok": not (containers or publish or campaigns)}
    (log_dir / "sweep.json").write_text(json.dumps(result, indent=2) + "\n")
    return result


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    parser.add_argument("--log-dir", required=True, type=Path, help="driver-owned directory outside the checkout")
    parser.add_argument("--baseline-ids", type=Path,
                        help="collect-only ID list at the baseline (ids-before-fix7.txt); every baseline ID must still be collected")
    parser.add_argument("--jobs", type=int, default=DEFAULT_JOBS, help=f"units run at once (default {DEFAULT_JOBS})")
    parser.add_argument("--check-only", action="store_true",
                        help="collect, check the partition and the baseline, print the plan, and exit without running any unit")
    args = parser.parse_args(argv)
    if args.jobs < 1:
        raise SystemExit("--jobs must be at least 1")
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
    if len(set(collected)) != len(collected):
        raise SystemExit("collect-only lists a test ID twice")
    baseline_count = None
    new_ids: list[str] = []
    if args.baseline_ids:
        baseline = [line.strip() for line in args.baseline_ids.read_text().splitlines() if "::" in line]
        new_ids = new_test_ids(collected, baseline)
        baseline_count = len(baseline)
        print(f"baseline: {baseline_count} IDs, all still collected; {len(new_ids)} new", flush=True)
    members = partition(collected, UNITS)
    print(f"partition: {len(collected)} collected IDs, each in exactly one of {len(UNITS)} units", flush=True)
    order = sorted(UNITS, key=lambda u: -u.seconds)  # longest first; ThreadPoolExecutor starts queued work in this order
    if args.check_only:
        for u in order:
            print(f"  plan {u.name:<42} {len(members[u.name]):4d} tests  ~{u.seconds:.0f}s", flush=True)
        print(f"check-only: {len(order)} units, {len(collected)} tests; nothing was run", flush=True)
        return 0

    origin = time.monotonic()
    results: list[dict] = []
    with ThreadPoolExecutor(max_workers=args.jobs) as pool:
        futures = [pool.submit(run_unit, u, members[u.name], log_dir, origin) for u in order]
        for future in as_completed(futures):
            result = future.result()
            results.append(result)
            status = "ok" if result["ok"] else "FAILED"
            print(f"  t={result['end_seconds']:7.1f}s {result['unit']:<40} {status:<6} exit={result['exit']} "
                  f"executed={result['executed']}/{result['expected']} {result['outcomes']} wall={result['wall_seconds']}s",
                  flush=True)
            if not result["ok"]:
                print(f"    summary: {result['summary_line']}", flush=True)
                for test_id in result["missing"][:20]:
                    print(f"    not executed: {test_id}", flush=True)
                for line in result["unexpected"][:20]:
                    print(f"    unexpected: {line}", flush=True)
    total_wall = time.monotonic() - origin
    sweep_result = sweep(log_dir)
    print(f"sweep: containers left={len(sweep_result['containers_left'])} publish={len(sweep_result['publish_processes_left'])} "
          f"campaign={len(sweep_result['campaign_processes_left'])} -> {'ok' if sweep_result['ok'] else 'FAILED'}", flush=True)

    groups: dict[str, dict] = {}
    for r in results:
        g = groups.setdefault(r["group"], {"group": r["group"], "units": 0, "first_start": r["start_seconds"],
                                           "last_end": r["end_seconds"], "sum_seconds": 0.0, "executed": 0})
        g["units"] += 1
        g["first_start"] = min(g["first_start"], r["start_seconds"])
        g["last_end"] = max(g["last_end"], r["end_seconds"])
        g["sum_seconds"] = round(g["sum_seconds"] + r["wall_seconds"], 1)
        g["executed"] += r["executed"]
    executed = sum(r["executed"] for r in results)
    all_units_ok = all(r["ok"] for r in results)
    partition_ok = executed == len(collected)
    if not partition_ok:
        print(f"executed {executed} of {len(collected)} collected IDs", flush=True)
    report = {"head": head, "checkout_changes_before": dirty, "jobs": args.jobs,
              "collected": len(collected), "baseline": baseline_count,
              "extra": None if baseline_count is None else len(new_ids), "extra_ids": new_ids,
              "executed": executed, "total_wall_seconds": round(total_wall, 1),
              "sum_of_unit_wall_seconds": round(sum(r["wall_seconds"] for r in results), 1),
              "groups": [{**g, "span_seconds": round(g["last_end"] - g["first_start"], 1)} for g in groups.values()],
              "units": results, "sweep": sweep_result,
              "ok": all_units_ok and partition_ok and sweep_result["ok"]}
    (log_dir / "summary.json").write_text(json.dumps(report, indent=2) + "\n")
    baseline_text, extra_text = ("n/a", "n/a") if baseline_count is None else (baseline_count, len(new_ids))
    print(f"\ntotal: collected={len(collected)} baseline={baseline_text} extra={extra_text} executed={executed} "
          f"wall={total_wall:.0f}s (sum of unit walls {report['sum_of_unit_wall_seconds']:.0f}s, jobs={args.jobs}) "
          f"-> {'PASS' if report['ok'] else 'FAIL'}", flush=True)
    print(f"logs and summary: {log_dir}", flush=True)
    return 0 if report["ok"] else 1


if __name__ == "__main__":
    sys.exit(main())
