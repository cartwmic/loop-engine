"""Periodic snapshot gating, index and per-snapshot hidden-test replay: fake sandbox, fake runner, one real container case."""
import asyncio
import json
from pathlib import Path
import tarfile
from types import SimpleNamespace

import pytest

from evalcore.target import InfrastructureError
from scevals.capture import ReplayContainer
from scevals.cases import load_case
from scevals.image import DEFAULT_IMAGE
from scevals.snapshots import replay_snapshots, snapshot_step

CASE = Path(__file__).resolve().parents[1] / "example-case"


class CannedSandbox:
    """Fake trial sandbox: the snapshot command succeeds or fails as set, and reading the snapshot returns fixed bytes."""

    def __init__(self, archive=b"archive", succeeds=True):
        self.archive, self.succeeds, self.commands = archive, succeeds, []

    async def exec(self, command, *, timeout=None):
        self.commands.append(command)
        return SimpleNamespace(success=self.succeeds)

    async def read_file(self, path, text=True):
        assert path == "/tmp/snapshot.tar" and not text
        return self.archive


def report(*statuses):
    """Hidden-test runner report with one tagged test per (requirement, status) pair."""
    return {"collection_error": False,
            "tests": [{"test_id": f"test{index}", "requirement_id": requirement, "status": status}
                      for index, (requirement, status) in enumerate(statuses)]}


def test_snapshots_are_off_without_an_interval_option(tmp_path):
    sandbox = CannedSandbox()
    directory = tmp_path / "snapshots"
    for options in ({}, {"snapshot_interval_seconds": None}):
        assert asyncio.run(snapshot_step(sandbox, options, directory, driver_started=0.0,
                                         last_taken=0.0, now=10_000.0)) == 0.0
    assert sandbox.commands == [] and not directory.exists()


@pytest.mark.parametrize("interval", [0, -5, True, "60"])
def test_interval_must_be_a_positive_number(tmp_path, interval):
    sandbox = CannedSandbox()
    with pytest.raises(ValueError):
        asyncio.run(snapshot_step(sandbox, {"snapshot_interval_seconds": interval}, tmp_path / "snapshots",
                                  driver_started=0.0, last_taken=0.0, now=100.0))
    assert sandbox.commands == []


def test_snapshots_follow_the_interval_and_index_their_elapsed_time(tmp_path):
    sandbox = CannedSandbox(archive=b"workspace")
    directory = tmp_path / "captures" / "snapshots"
    options = {"snapshot_interval_seconds": 60}

    def step(last_taken, now):
        return asyncio.run(snapshot_step(sandbox, options, directory, driver_started=100.0,
                                         last_taken=last_taken, now=now))

    assert step(100.0, 159.0) == 100.0          # 59 s since driver start: not due
    assert not directory.exists()
    assert step(100.0, 160.0) == 160.0          # 60 s: first snapshot
    assert step(160.0, 219.0) == 160.0          # 59 s since the last snapshot: not due
    assert step(160.0, 221.0) == 221.0          # 61 s: second snapshot
    assert len(sandbox.commands) == 2

    index = json.loads((directory / "snapshots.json").read_text())
    assert [(entry["sequence"], entry["file"], entry["elapsed_seconds"]) for entry in index] == [
        (1, "snapshot-000001.tar", 60.0), (2, "snapshot-000002.tar", 121.0)]
    assert sorted(path.name for path in directory.iterdir()) == [
        "snapshot-000001.tar", "snapshot-000002.tar", "snapshots.json"]
    assert (directory / "snapshot-000001.tar").read_bytes() == b"workspace"


def test_failed_snapshot_is_infrastructure_and_names_no_snapshot(tmp_path):
    sandbox = CannedSandbox(succeeds=False)
    directory = tmp_path / "snapshots"
    with pytest.raises(InfrastructureError):
        asyncio.run(snapshot_step(sandbox, {"snapshot_interval_seconds": 1}, directory,
                                  driver_started=0.0, last_taken=0.0, now=5.0))
    assert not directory.exists()


def test_replay_reports_each_snapshot_per_requirement_oldest_first(tmp_path):
    case = load_case(CASE)
    directory = tmp_path / "snapshots"
    options = {"snapshot_interval_seconds": 60}
    last = asyncio.run(snapshot_step(CannedSandbox(archive=b"first"), options, directory,
                                     driver_started=0.0, last_taken=0.0, now=60.0))
    asyncio.run(snapshot_step(CannedSandbox(archive=b"second"), options, directory,
                              driver_started=0.0, last_taken=last, now=120.0))
    reports = {
        b"first": report(("REQ-1", "failed"), ("REQ-1", "failed"), ("REQ-2", "failed"),
                         ("REQ-2", "passed"), ("REQ-3", "error"), ("REQ-3", "failed")),
        b"second": report(("REQ-1", "passed"), ("REQ-1", "passed"), ("REQ-2", "passed"),
                          ("REQ-2", "passed"), ("REQ-3", "passed"), ("REQ-3", "passed")),
    }
    seen = []

    async def fake_runner(archive, case, image):
        seen.append((archive.name, image))
        return reports[archive.read_bytes()]

    trajectory = asyncio.run(replay_snapshots(directory, case, "image:test", replay=fake_runner))

    assert seen == [("snapshot-000001.tar", "image:test"), ("snapshot-000002.tar", "image:test")]
    assert trajectory[0] == {
        "sequence": 1, "elapsed_seconds": 60.0,
        "delivery_recall": {"numerator": 1, "denominator": 6, "value": 1 / 6},
        "delivery_by_requirement": {"REQ-1": {"numerator": 0, "denominator": 2, "value": 0.0},
                                    "REQ-2": {"numerator": 1, "denominator": 2, "value": 0.5},
                                    "REQ-3": {"numerator": 0, "denominator": 2, "value": 0.0}}}
    assert trajectory[1]["sequence"] == 2 and trajectory[1]["elapsed_seconds"] == 120.0
    assert trajectory[1]["delivery_recall"]["numerator"] == 6


def test_container_snapshots_replay_to_a_per_requirement_trajectory(tmp_path):
    """Real tar in the trial image takes the snapshots; a fresh container replays each one with the hidden tests."""
    case = load_case(CASE)
    directory = tmp_path / "captures" / "snapshots"
    options = {"snapshot_interval_seconds": 60}

    async def proof():
        async with ReplayContainer(DEFAULT_IMAGE) as env:
            await env.write_file("/workspace/public/repo/labels.py", (case.repository / "labels.py").read_bytes())
            # A build cache the driver would leave behind: excluded from snapshots like the final capture.
            await env.write_file("/workspace/public/repo/target/CACHEDIR.TAG",
                                 b"Signature: 8a477f597d28d172789f06886806bc55\n")
            await env.write_file("/workspace/public/repo/target/debug/build-output", b"binary")
            last = await snapshot_step(env, options, directory, driver_started=0.0, last_taken=0.0, now=60.0)
            await env.write_file("/workspace/public/repo/labels.py", (case.reference / "labels.py").read_bytes())
            await snapshot_step(env, options, directory, driver_started=0.0, last_taken=last, now=120.0)

    asyncio.run(proof())
    with tarfile.open(directory / "snapshot-000001.tar") as archive:
        names = archive.getnames()
    assert "public/repo/labels.py" in names
    assert not any(name.startswith("public/repo/target") for name in names)

    trajectory = asyncio.run(replay_snapshots(directory, case, DEFAULT_IMAGE))
    assert [point["delivery_recall"]["numerator"] for point in trajectory] == [0, 6]
    assert [point["delivery_by_requirement"]["REQ-1"]["value"] for point in trajectory] == [0.0, 1.0]
    assert [point["elapsed_seconds"] for point in trajectory] == [60.0, 120.0]
