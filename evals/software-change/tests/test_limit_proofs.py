"""Scripted stop and deadline proofs through the public evalctl path on the stock released image.

No network and no paid call: scripted Inspect backends, real Pi, real Docker. Each proof drives
`evalctl campaign` to a completed outcome and reads the report, the capture and Docker afterwards.

TIME_SCALE (default 1.0 = full size) scales only the proofs that keep their production branch when scaled; the
others run at full size whatever TIME_SCALE is. perf/limit-test-notes.md names the production code behind each choice.

The proofs run concurrently with other test processes, so each one checks only the containers its own campaign created
(own_containers). The machine-wide "no default-image container left" check runs once, after every group, in the runner.
"""
import json
import os
from pathlib import Path
import stat
import subprocess
import time
import tomllib
import uuid

import pytest

from scevals.image import DEFAULT_IMAGE

REPO = Path(__file__).resolve().parents[3]
ROOT = REPO / "evals/software-change"
PROOFS = ROOT / "captures/limit-proofs"
SCRIPTED = {"sc-scripted/driver": None, "sc-scripted/simulator": None, "sc-scripted/judge": None}
HANG_AFTER_TERMINAL = ["sh", "-c", 'sc-driver "$@"; sleep 1000', "sh"]
TIME_SCALE = float(os.environ.get("TIME_SCALE", "1.0"))
if not 0 < TIME_SCALE <= 1:
    raise ValueError("TIME_SCALE must be greater than 0 and at most 1")


def scaled(seconds):
    return seconds * TIME_SCALE


def scaled_int(seconds):
    """A campaign cap or reserve, which must be whole seconds."""
    return max(1, round(scaled(seconds)))


class Outcome:
    def __init__(self, result, elapsed, directory, name):
        self.result, self.elapsed, self.directory, self.name = result, elapsed, directory, name
        self.summary = json.loads((directory / "summary.json").read_text()) if directory else None

    @property
    def trial(self):
        return self.summary["trials"][0]

    @property
    def capture(self):
        return Path(self.summary["capture_run"]) / self.trial["sample_id"]


def drive(prices=None, extra_env=None, expect_exit=0, **overrides):
    PROOFS.mkdir(parents=True, exist_ok=True)
    settings = tomllib.loads((ROOT / "campaigns/scripted-example.toml").read_text())
    settings.update(profile="minimal-16", tier="proof", **overrides)
    settings["target_options"] = {"scripted_driver": "asks-and-builds", **settings.get("target_options", {}), **overrides.get("target_options", {})}
    if prices is not None:
        settings["prices"] = {model: prices for model in SCRIPTED}
    name = "lp-" + uuid.uuid4().hex[:12]
    settings["campaign"] = name
    config = PROOFS / (name + ".json")
    config.write_text(json.dumps(settings))
    begun = time.monotonic()
    result = subprocess.run(["uv", "run", "--project", "evals", "evalctl", "campaign", "--settings", str(config)], cwd=REPO,
                            env={**os.environ, **(extra_env or {})}, text=True, capture_output=True, timeout=1500)
    elapsed = time.monotonic() - begun
    (PROOFS / (name + ".log")).write_text(result.stdout + result.stderr)
    assert result.returncode == expect_exit, result.stdout + result.stderr
    directory = None
    if expect_exit == 0:
        directory = Path(result.stdout.split("Report written: ", 1)[1].strip())
        shown = PROOFS / name
        directory.rename(shown)
        directory.parent.rmdir()
        directory = shown
    return Outcome(result, elapsed, directory, name)


def own_containers(campaign):
    """Default-image containers that this campaign's sandboxes created. Inspect writes each sandbox's compose file under
    captures/<campaign>/, and docker compose records that path in the project's config-files label. One `docker ps` call
    lists the IDs with their labels, so other units' containers coming and going cannot make the listing inconsistent.
    Replay containers carry no label; the runner's final sweep covers them."""
    listed = subprocess.run(["docker", "ps", "-a", "--filter", f"ancestor={DEFAULT_IMAGE}", "--format", "{{.ID}}\t{{.Labels}}"],
                            capture_output=True, text=True, check=True).stdout.splitlines()
    marker = f"/captures/{campaign}/"
    return [line.split("\t", 1)[0] for line in listed if marker in line]


def logged_removals(path):
    """Container IDs of the `docker rm -f` calls that a shim logged (the last argument of each call)."""
    return [line.split()[-1] for line in path.read_text().splitlines()] if path.exists() else []


def tokens(trial):
    return sum(u["total_tokens"] for u in trial["usage_by_role"].values())


@pytest.fixture(scope="module")
def baseline():
    """A normal scripted trial under generous caps: runs to a captured, scoreable result and reports in time."""
    outcome = drive(campaign_wall_limit_seconds=scaled_int(900), post_trial_reserve_seconds=scaled_int(300),
                    publication_reserve_seconds=scaled_int(60))
    return outcome


def test_normal_scripted_trial_runs_to_a_scoreable_capture_and_reports_inside_the_cap(baseline):
    trial = baseline.trial
    assert trial["outcome"] == "success" and trial["terminal"] is True
    assert trial["target_metrics"]["hidden_test_outcome"]["passed"] is True
    assert (baseline.capture / "workspace.tar").is_file() and (baseline.capture / "isolation.json").is_file()
    budget = baseline.summary["budget"]
    assert budget["wall_within_cap"] is True and budget["cost_verdict"] == "not_budgeted"
    assert baseline.summary["campaign_wall_seconds"] <= scaled(900) and own_containers(baseline.name) == []
    assert baseline.summary["versions"]["target"]["loop_engine"] == "v0.26.0"


def test_unpriced_model_with_a_dollar_budget_is_refused_before_any_container_or_call():
    outcome = drive(expect_exit=2, campaign_budget_usd=5, dollar_margin_usd=1)
    assert "needs a price for every model" in outcome.result.stderr
    assert own_containers(outcome.name) == [] and "Captures retained" not in outcome.result.stdout


# Full size under every TIME_SCALE: the driver limit is max(60, hard - 300), so a 120 s trial's driver floor and margin are production constants.
def test_per_trial_stop_is_a_limit_with_kept_capture_and_no_leftovers():
    outcome = drive(hard_limit_seconds=120, target_options={"driver_command": ["sh", "-c", "sleep 1000", "sh"]})
    trial = outcome.trial
    assert (trial["outcome"], trial["reason"]) == ("limit", "trial_time")
    assert (outcome.capture / "workspace.tar").is_file() and (outcome.capture / "isolation.json").is_file()
    assert own_containers(outcome.name) == []


def test_campaign_wall_time_stop_still_captures_scores_judges_nothing_and_reports_inside_the_cap():
    """Trial work reaches a terminal state, then hangs until the campaign stop at S. Everything after still completes by D."""
    outcome = drive(campaign_wall_limit_seconds=scaled_int(300), post_trial_reserve_seconds=scaled_int(150),
                    publication_reserve_seconds=scaled_int(30), guard_poll_seconds=scaled(2),
                    target_options={"driver_command": HANG_AFTER_TERMINAL})
    trial = outcome.trial
    assert (trial["outcome"], trial["reason"]) == ("limit", "campaign_wall_time")
    metrics = trial["target_metrics"]
    assert metrics["hidden_test_outcome"]["passed"] is True            # scoring (no model call) still ran
    assert metrics["intent_judge"]["status"] == "limit"                # the judge model was never called
    assert trial["usage_by_role"].get("judge", {}).get("total_tokens", 0) == 0
    assert outcome.elapsed < scaled(300) and outcome.summary["budget"]["wall_within_cap"] is True
    assert (outcome.capture / "workspace.tar").is_file() and own_containers(outcome.name) == []


def test_dollar_stop_fires_below_the_cap_during_the_trial_and_the_judge_is_never_called(baseline):
    """Price so the normal pre-terminal spend is small; after the terminal state the driver keeps buying review calls."""
    per_token = 2.0e6 / tokens(baseline.trial)           # about 2 dollars for the whole baseline trial
    prices = {"input_per_million": per_token, "output_per_million": per_token}
    outcome = drive(prices=prices, campaign_budget_usd=10, dollar_margin_usd=1, guard_poll_seconds=scaled(1),
                    campaign_wall_limit_seconds=scaled_int(900), post_trial_reserve_seconds=scaled_int(300),
                    publication_reserve_seconds=scaled_int(60),
                    target_options={"driver_command": ["sh", "-c", f'sc-driver "$@"; for i in $(seq 300); do sc-review "buy"; sleep {scaled(1):g}; done', "sh"]})
    trial = outcome.trial
    assert (trial["outcome"], trial["reason"]) == ("limit", "campaign_dollars")
    assert trial["target_metrics"]["intent_judge"]["status"] == "limit"
    assert trial["usage_by_role"].get("judge", {}).get("total_tokens", 0) == 0
    budget = outcome.summary["budget"]
    assert budget["stop_events"][0]["reason"] == "campaign_dollars" and budget["actual"]["usd"] >= 9   # stop line = cap - margin
    assert budget["cost_verdict"] in ("pass", "fail_over_cap") and own_containers(outcome.name) == []


def test_blocking_render_leaves_the_limit_only_record_and_the_command_still_ends_by_the_cap(tmp_path):
    # TMPDIR is this test's own directory: the publish child's argv names it, so pgrep finds only this test's child.
    outcome = drive(campaign_wall_limit_seconds=scaled_int(240), post_trial_reserve_seconds=scaled_int(100),
                    publication_reserve_seconds=scaled_int(30),
                    extra_env={"EVALCORE_PROOF_RENDER_DELAY_SECONDS": "3600", "TMPDIR": str(tmp_path)})
    assert outcome.elapsed < scaled(240 + 30)
    assert outcome.summary["record"] == "limit-only" and outcome.summary["trials"][0]["outcome"] == "success"
    assert outcome.summary["publication_cut_off"] is True and outcome.summary["cut_off_steps"][-1]["step"] == "publication"
    # The fallback is the result, so it must report the real command-to-written time (the renderer was waited on until the cap).
    assert scaled(240 - 10) <= outcome.summary["campaign_wall_seconds"] <= scaled(240) and outcome.summary["wall_within_cap"] is True
    publication = json.loads((outcome.directory / "publication.json").read_text())
    assert publication["full_report"] is False and publication["within_cap"] is True
    assert abs(publication["command_start_to_report_written_seconds"] - outcome.summary["campaign_wall_seconds"]) < 1
    assert subprocess.run(["pgrep", "-f", str(tmp_path)], capture_output=True, text=True).stdout.strip() == ""


# Full size under every TIME_SCALE: the image-inspection probe is PROBE_SECONDS = 60, a production constant that must stay below the cap.
def test_blocked_docker_inspection_ends_by_the_cap_with_a_limit_before_any_trial(tmp_path):
    real = subprocess.run(["which", "docker"], capture_output=True, text=True).stdout.strip()
    shim = tmp_path / "docker"
    shim.write_text(f'#!/bin/sh\nif [ "$1" = image ] && [ "$2" = inspect ]; then sleep 600; fi\nexec {real} "$@"\n')
    shim.chmod(shim.stat().st_mode | stat.S_IEXEC)
    outcome = drive(campaign_wall_limit_seconds=90, post_trial_reserve_seconds=40, publication_reserve_seconds=10,
                    extra_env={"PATH": f"{tmp_path}:{os.environ['PATH']}"})
    assert outcome.elapsed < 90
    record = outcome.summary
    assert record["record"] == "limit-only" and record["setup_limit"]["step"] == "image_inspection"
    assert {(t["outcome"], t["limit_step"]) for t in record["trials"]} == {("limit", "image_inspection")}
    assert own_containers(outcome.name) == []


def test_cut_off_capture_still_leaves_the_isolation_evidence():
    outcome = drive(campaign_wall_limit_seconds=scaled_int(300), post_trial_reserve_seconds=scaled_int(200),
                    publication_reserve_seconds=scaled_int(30), target_options={"scripted_capture_delay_seconds": 3000})
    trial = outcome.trial
    assert (trial["outcome"], trial["limit_step"]) == ("limit", "capture")
    capture = Path(outcome.summary["capture_run"]) / trial["sample_id"]
    isolation = json.loads((capture / "isolation.json").read_text())
    assert isolation["mounts"] == [] and isolation["binds"] == [] and isolation["id"] and isolation["image_id"]
    assert not (capture / "workspace.tar").exists()
    assert outcome.elapsed < scaled(300) and own_containers(outcome.name) == []


def test_slow_scoring_is_abandoned_at_its_allowance_with_the_capture_kept():
    outcome = drive(campaign_wall_limit_seconds=scaled_int(300), post_trial_reserve_seconds=scaled_int(200),
                    publication_reserve_seconds=scaled_int(30), target_options={"scripted_scoring_delay_seconds": 3000})
    trial = outcome.trial
    assert (trial["outcome"], trial["limit_step"]) == ("limit", "scoring")
    assert (outcome.capture / "workspace.tar").is_file()
    assert outcome.summary["budget"]["cut_off_steps"][0]["step"] == "scoring"
    assert outcome.elapsed < scaled(300) and outcome.summary["budget"]["wall_within_cap"] is True and own_containers(outcome.name) == []


# The publication reserve stays at full size under every TIME_SCALE: the removal allowance is publication reserve - 30 s
# (PUBLICATION_HEADROOM_SECONDS), which is 10 s here as at full size. The cap and post-trial reserve scale, so the scoring
# window H - now and the removal allowance keep their full-size branch.
def test_a_stuck_replay_container_removal_cannot_push_publication_past_the_cap(tmp_path):
    """Scoring runs until the hard end H, then removing the replay container blocks. The shielded removal must be cut at its
    allowance (time left before D minus headroom), so the report still lands before D."""
    real = subprocess.run(["which", "docker"], capture_output=True, text=True).stdout.strip()
    shim = tmp_path / "docker"
    removals = tmp_path / "removals.txt"
    shim.write_text(f'#!/bin/sh\nif [ "$1" = rm ]; then echo "$*" >> {removals}; sleep 90; fi\nexec {real} "$@"\n')
    shim.chmod(shim.stat().st_mode | stat.S_IEXEC)
    try:
        outcome = drive(campaign_wall_limit_seconds=scaled_int(300), post_trial_reserve_seconds=scaled_int(200),
                        publication_reserve_seconds=40,
                        target_options={"scripted_scoring_delay_seconds": 3000}, extra_env={"PATH": f"{tmp_path}:{os.environ['PATH']}"})
    finally:  # the shim leaked this campaign's replay container; remove exactly the containers it was asked to remove
        for container in logged_removals(removals):
            subprocess.run([real, "rm", "-f", container], capture_output=True)
    assert outcome.elapsed < scaled(300)
    assert outcome.summary["budget"]["wall_within_cap"] is True
    assert (outcome.trial["outcome"], outcome.trial["limit_step"]) == ("limit", "scoring")
    steps = [c["step"] for c in outcome.summary["budget"]["cut_off_steps"]]
    assert "replay_cleanup" in steps          # the abandoned removal is named in the report, and was handed off detached
