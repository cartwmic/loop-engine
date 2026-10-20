"""Snapshots end to end: one scripted arm trial through evalctl campaign, with snapshots on and the same trial off.

The trial is the native arm with the scripted arm driver: it writes an incomplete labels.py, keeps its model traffic
going, then writes the correct one and exits by itself. With snapshot_interval_seconds set, the progress poll takes
private workspace snapshots into the trial's capture area; replay runs the hidden tests on each one in a fresh
container. With it unset, the trial must leave no snapshot, have the same capture members and bytes, and score the
same. No paid model, no network; the scripted models only. Mechanics proof only.
Retained output: tests/proofs/snapshots-trial.txt and the two campaign reports under tests/proofs/campaigns/.
"""
import asyncio
import json
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import tomllib
from types import SimpleNamespace
import uuid

import pytest

from scevals import scripted_arm_driver
from scevals.cases import load_case
from scevals.image import DEFAULT_IMAGE
from scevals.snapshots import replay_snapshots
from scevals.target import captured_files

REPO = Path(__file__).resolve().parents[3]
ROOT = REPO / "evals/software-change"
PROOFS = ROOT / "tests/proofs"
OUTPUT = PROOFS / "snapshots-trial.txt"
CASE = ROOT / "example-case"
DRIVER_SOURCE = (ROOT / "scevals/scripted_arm_driver.py").read_text()
ARM = {"id": "native", "driver_command": ["python3", "-c", DRIVER_SOURCE, "--code", "staged", "--seconds", "4",
                                          "--keepalive"],
       "prompt_template": "native", "workflow_owned_paths": [], "completion_rule": "driver-exit"}
# Progress comes from model traffic every second. The progress poll runs every idle/2 = 2 s and takes a snapshot
# whenever the 1 s snapshot interval has passed since the last one.
CAMPAIGN_IDLE_SECONDS = 4
# Wall-clock and location fields differ between any two runs; everything else must match.
TIMING_OR_LOCATION = {"driver_elapsed_seconds", "copy_log", "copy_out_log", "container"}
RUNS = {}
TRAJECTORIES = {}


def run_campaign(label, target_options):
    settings = tomllib.loads((ROOT / "campaigns/scripted-example.toml").read_text())
    settings["target_options"] = target_options
    settings["idle_limit_seconds"] = CAMPAIGN_IDLE_SECONDS
    name = f"t10-snap-{label}-{uuid.uuid4().hex[:8]}"
    settings["campaign"] = name
    with tempfile.TemporaryDirectory(prefix="t10-snapshots-") as scratch:
        config = Path(scratch) / (name + ".json")
        config.write_text(json.dumps(settings))
        result = subprocess.run(["uv", "run", "--project", "evals", "evalctl", "campaign", "--settings", str(config)],
                                cwd=REPO, text=True, capture_output=True, timeout=650)
    assert result.returncode == 0, result.stdout + result.stderr
    result_dir = Path(result.stdout.split("Report written: ", 1)[1].strip())
    assert result_dir.parent == ROOT / "results" / name
    summary = json.loads((result_dir / "summary.json").read_text())
    # The report is a local receipt under tests/proofs, never an extra published result.
    retained = PROOFS / "campaigns" / f"snapshots-{label}"
    shutil.rmtree(retained, ignore_errors=True)
    retained.parent.mkdir(parents=True, exist_ok=True)
    result_dir.rename(retained)
    result_dir.parent.rmdir()
    (retained / "evalctl.log").write_text(result.stdout + result.stderr)
    assert len(summary["trials"]) == 1, summary
    trial = summary["trials"][0]
    capture = Path(summary["capture_run"]) / trial["sample_id"]
    return trial, capture


def run(label):
    if label not in RUNS:
        options = {"arm": ARM} | ({"snapshot_interval_seconds": 1} if label == "on" else {})
        RUNS[label] = run_campaign(label, options)
    return RUNS[label]


def capture_members(capture):
    """Capture members without git's own object store: its commit hashes depend on the baseline commit's timestamp."""
    with tarfile.open(capture / "workspace.tar") as archive:
        names = (member.name.removeprefix("./") for member in archive.getmembers())
        return sorted(name for name in names if ".git" not in Path(name).parts)


def trajectory_of_the_on_trial():
    """Replay of the snapshots taken in the snapshots-on trial, oldest first; computed once per session."""
    if "on" not in TRAJECTORIES:
        trial, capture = run("on")
        directory = capture.parent / f"{trial['sample_id']}-snapshots"
        TRAJECTORIES["on"] = (len(json.loads((directory / "snapshots.json").read_text())),
                              asyncio.run(replay_snapshots(directory, load_case(CASE), DEFAULT_IMAGE)))
    return TRAJECTORIES["on"]


def comparable(trial):
    metrics = trial["target_metrics"]
    return {key: value for key, value in metrics.items() if key not in TIMING_OR_LOCATION}


def test_snapshots_on_produce_a_per_requirement_trajectory_of_the_same_delivery():
    trial, capture = run("on")
    assert (trial["outcome"], trial["reason"], trial["terminal"]) == ("success", "passed", True), trial["target_metrics"]
    directory = capture.parent / f"{trial['sample_id']}-snapshots"
    index = json.loads((directory / "snapshots.json").read_text())
    assert len(index) >= 2, index
    assert all((directory / entry["file"]).is_file() for entry in index)
    assert not any(path.name.startswith("snapshot") for path in capture.iterdir())
    count, trajectory = trajectory_of_the_on_trial()
    assert count == len(index) == len(trajectory)
    assert [point["sequence"] for point in trajectory] == [entry["sequence"] for entry in index]
    assert [point["elapsed_seconds"] for point in trajectory] == sorted(point["elapsed_seconds"] for point in trajectory)
    first, last = trajectory[0], trajectory[-1]
    # The first snapshot misses the empty-label fallback (REQ-3); the last one matches the delivered final workspace.
    assert first["delivery_by_requirement"]["REQ-3"]["value"] == 0.0, first
    assert last["delivery_recall"] == {"numerator": 6, "denominator": 6, "value": 1.0}, last
    assert first["delivery_recall"]["numerator"] < last["delivery_recall"]["numerator"], trajectory
    assert "snapshot" not in json.dumps(trial["target_metrics"]), "snapshot evidence must stay out of the exported metrics"


def test_snapshots_off_leave_the_capture_and_the_scores_as_the_same_trial_without_them():
    on, on_capture = run("on")
    off, off_capture = run("off")
    assert (off["outcome"], off["reason"], off["terminal"]) == ("success", "passed", True), off["target_metrics"]
    assert not (off_capture.parent / f"{off['sample_id']}-snapshots").exists()
    assert not any(path.name.startswith("snapshot") for path in off_capture.iterdir())
    assert capture_members(off_capture) == capture_members(on_capture)
    assert captured_files(off_capture / "workspace.tar") == captured_files(on_capture / "workspace.tar")
    assert comparable(off) == comparable(on)
    assert off["target_metrics"]["hidden_test_outcome"] == on["target_metrics"]["hidden_test_outcome"]
    assert off["target_metrics"]["delivery_recall"] == on["target_metrics"]["delivery_recall"]
    assert off["target_metrics"]["discipline"] == on["target_metrics"]["discipline"]
    assert on["outcome"] == off["outcome"] == "success"


def test_retained_snapshot_output_records_both_trials():
    on, _ = run("on")
    off, _ = run("off")
    count, trajectory = trajectory_of_the_on_trial()
    OUTPUT.parent.mkdir(parents=True, exist_ok=True)
    OUTPUT.write_text(
        "Snapshots trial proof: one scripted native-arm trial through evalctl campaign, snapshots on and off.\n"
        "No model and no network; the models are scripted. Mechanics proof only, not evidence about any real arm.\n"
        f"snapshots-on trial: outcome {on['outcome']} ({on['reason']}); snapshots taken: {count}\n"
        "replay in fresh containers, one line per snapshot (sequence, elapsed seconds, delivery recall, REQ-3):\n"
        + "\n".join(json.dumps({"sequence": point["sequence"], "elapsed_seconds": point["elapsed_seconds"],
                                "delivery_recall": point["delivery_recall"],
                                "REQ-3": point["delivery_by_requirement"]["REQ-3"]}, sort_keys=True)
                    for point in trajectory) + "\n"
        f"snapshots-off trial: outcome {off['outcome']} ({off['reason']}); no snapshot directory, capture members and\n"
        "bytes identical to the snapshots-on capture, and metrics identical apart from driver timing and container ids.\n"
        "Baseline note: 'today's' capture is this same trial with the snapshot option absent, run in this worktree;\n"
        "no byte comparison against a pre-change main capture was made.\n")


@pytest.mark.parametrize(("seconds", "call_seconds", "calls"), [
    pytest.param(4, 0.0, 4, id="fast-calls"),
    pytest.param(4, 2.5, 4, id="slow-calls-none-dropped"),
    pytest.param(0.5, 0.0, 1, id="short-wait-one-call"),
    pytest.param(0, 0.0, 1, id="zero-wait-one-call"),
])
def test_keepalive_wait_makes_the_same_calls_whatever_their_latency(monkeypatch, seconds, call_seconds, calls):
    """max(1, ceil(seconds)) calls per wait, so the driver's token count does not depend on call latency."""
    clock = [0.0]
    made = []

    def model_call():
        made.append(clock[0])
        clock[0] += call_seconds

    def sleep(duration):
        clock[0] += duration

    monkeypatch.setattr(scripted_arm_driver, "model_call", model_call)
    monkeypatch.setattr(scripted_arm_driver, "time", SimpleNamespace(monotonic=lambda: clock[0], sleep=sleep))
    scripted_arm_driver.pause(seconds, keepalive=True)
    assert len(made) == calls


def test_a_wait_without_keepalive_only_sleeps(monkeypatch):
    slept = []
    monkeypatch.setattr(scripted_arm_driver, "model_call",
                        lambda: pytest.fail("a wait without keepalive makes no model call"))
    monkeypatch.setattr(scripted_arm_driver, "time", SimpleNamespace(sleep=slept.append))
    scripted_arm_driver.pause(4, keepalive=False)
    assert slept == [4]
