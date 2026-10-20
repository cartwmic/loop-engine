"""Periodic private workspace snapshots and per-snapshot hidden-test replay.

Off unless a target sets snapshot_interval_seconds. A snapshot holds the
delivered public repository only; hidden tests enter a fresh replay container
at replay time. Snapshots and replay results are private evidence, never published.
"""
import json
from pathlib import Path

from evalcore.target import InfrastructureError

from .capture import ReplayContainer, restore
from .cases import RUNTIME, EvalCase
from .image import DEFAULT_IMAGE
from .scoring import recall

INDEX_NAME = "snapshots.json"
SNAPSHOT_TIMEOUT_SECONDS = 120  # own limit of one snapshot step during the driver run

# Same cache exclusions and tar tolerance as the final capture: a live driver can
# make tar exit 1 ("file changed while read"), which is harmless here.
SNAPSHOT_TAR_COMMAND = ("cd /workspace && test -d public/repo && "
                        "{ tar --exclude-caches-all --exclude=.cache -cf /tmp/snapshot.tar public/repo; [ $? -le 1 ]; }")


def write_private(path: Path, data: bytes):
    """Write whole or not at all, so a partial file is never named as a snapshot."""
    partial = path.with_name(path.name + ".partial")
    partial.write_bytes(data)
    partial.rename(path)


def read_index(directory: Path):
    path = directory / INDEX_NAME
    return json.loads(path.read_text()) if path.is_file() else []


async def snapshot_step(env, options, directory: Path, *, driver_started: float, last_taken: float, now: float):
    """Take a snapshot when snapshot_interval_seconds has passed since last_taken; return the newest snapshot time.

    Call from the progress poll with last_taken=driver_started until the first snapshot. Times are
    time.monotonic() readings. `directory` must not be the trial capture directory, which capture_raw
    creates fresh after the driver stops.
    """
    interval = options.get("snapshot_interval_seconds")
    if interval is None:
        return last_taken
    if isinstance(interval, bool) or not isinstance(interval, (int, float)) or interval <= 0:
        raise ValueError("snapshot_interval_seconds must be a positive number")
    if now - last_taken < interval:
        return last_taken
    index = read_index(directory)
    sequence = len(index) + 1
    result = await env.exec(["sh", "-c", SNAPSHOT_TAR_COMMAND], timeout=SNAPSHOT_TIMEOUT_SECONDS)
    if not result.success:
        raise InfrastructureError("workspace snapshot failed")
    archive = await env.read_file("/tmp/snapshot.tar", text=False)
    directory.mkdir(mode=0o700, parents=True, exist_ok=True)
    name = f"snapshot-{sequence:06d}.tar"
    write_private(directory / name, archive)
    index.append({"sequence": sequence, "file": name, "elapsed_seconds": now - driver_started})
    write_private(directory / INDEX_NAME, (json.dumps(index, indent=2) + "\n").encode())
    return now


async def replay_hidden_tests(archive: Path, case: EvalCase, image: str):
    """Restore one snapshot into a fresh mount-free container and run the hidden tests as final scoring does."""
    async with ReplayContainer(image) as env:
        await restore(env, archive)
        for file in sorted(case.tests.rglob("*")):
            if file.is_file():
                await env.write_file("/opt/scoring/tests/" + file.relative_to(case.tests).as_posix(), file.read_bytes())
        await env.write_file("/opt/scoring/case_runtime.py", RUNTIME.read_bytes())
        result = await env.exec(["python3", "/opt/scoring/case_runtime.py", "--tests", "/opt/scoring/tests",
                                 "--output", "/tmp/hidden-results.json"],
                                cwd="/workspace/public/repo", timeout=case.test_timeout_seconds)
        if not result.success:
            raise InfrastructureError("hidden-test runner failed")
        return json.loads(await env.read_file("/tmp/hidden-results.json"))


async def replay_snapshots(directory: Path, case: EvalCase, image: str = DEFAULT_IMAGE, *, replay=replay_hidden_tests):
    """Delivery trajectory: hidden tests on each snapshot, oldest first, one fresh container per snapshot.

    Each point carries only the sequence, elapsed time and the recall() fractions, never test names or output.
    """
    trajectory = []
    for entry in read_index(directory):
        tests = await replay(directory / entry["file"], case, image)
        scored = recall(case, tests, revealed=())
        trajectory.append({"sequence": entry["sequence"], "elapsed_seconds": entry["elapsed_seconds"],
                           "delivery_recall": scored["delivery_recall"],
                           "delivery_by_requirement": scored["delivery_by_requirement"]})
    return trajectory
