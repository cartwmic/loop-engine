"""Workspace replay link fidelity and pre-extraction write-traversal checks."""
import asyncio
import io
import json
import os
from pathlib import Path
import tarfile
import uuid

import anyio
import pytest

from scevals.capture import ReplayContainer, restore, validate_archive
from scevals.image import DEFAULT_IMAGE

ROOT = Path(__file__).resolve().parents[1] / "captures/F4/extraction"


def archive(entries):
    ROOT.mkdir(parents=True, exist_ok=True)
    path = ROOT / (uuid.uuid4().hex + ".tar")
    with tarfile.open(path, "w") as tar:
        for name, kind, value in entries:
            item = tarfile.TarInfo(name)
            item.type = kind
            if kind == tarfile.REGTYPE:
                payload = value.encode()
                item.size = len(payload)
                tar.addfile(item, io.BytesIO(payload))
            else:
                item.linkname = value
                tar.addfile(item)
    return path


def test_replay_restores_absolute_and_parent_symlinks_and_workspace_hardlinks():
    path = archive([
        ("public/data.txt", tarfile.REGTYPE, "retained\n"),
        ("public/repo", tarfile.DIRTYPE, ""),
        ("public/repo/relative-link", tarfile.SYMTYPE, "../data.txt"),
        ("public/repo/python", tarfile.SYMTYPE, "/usr/bin/python3"),
        ("trial/source", tarfile.REGTYPE, "durable\n"),
        ("trial/hardcopy", tarfile.LNKTYPE, "trial/source"),
    ])
    async def proof():
        async with ReplayContainer(DEFAULT_IMAGE) as env:
            await restore(env, path)
            result = await env.exec(["python3", "-c",
                "import os,json; from pathlib import Path; "
                "print(json.dumps({'relative':os.readlink('/workspace/public/repo/relative-link'),"
                "'absolute':os.readlink('/workspace/public/repo/python'),"
                "'text':Path('/workspace/public/repo/relative-link').read_text(),"
                "'hardlink':os.stat('/workspace/trial/source').st_ino==os.stat('/workspace/trial/hardcopy').st_ino}))"])
            assert result.success
            return json.loads(result.stdout)
    assert asyncio.run(proof()) == {"relative": "../data.txt", "absolute": "/usr/bin/python3",
                                    "text": "retained\n", "hardlink": True}


@pytest.mark.parametrize("entries", [
    [("/tmp/outside", tarfile.REGTYPE, "bad")],
    [("../outside", tarfile.REGTYPE, "bad")],
    [("trial/device", tarfile.CHRTYPE, "")],
    [("trial/fifo", tarfile.FIFOTYPE, "")],
    [("public/link", tarfile.SYMTYPE, "/tmp"), ("public/link/outside", tarfile.REGTYPE, "bad")],
    [("public/link/outside", tarfile.REGTYPE, "bad"), ("public/link", tarfile.SYMTYPE, "../elsewhere")],
    [("trial/hardcopy", tarfile.LNKTYPE, "../outside")],
    [("trial/hardcopy", tarfile.LNKTYPE, "/tmp/outside")],
    [("public/link", tarfile.SYMTYPE, "/tmp"), ("trial/hardcopy", tarfile.LNKTYPE, "public/link/outside")],
    [("public/link", tarfile.SYMTYPE, "/tmp"), ("public/link", tarfile.REGTYPE, "bad")],
])
def test_unsafe_archive_is_refused_before_any_container_write(entries):
    path = archive(entries)
    class UnusedEnvironment:
        async def write_file(self, *args):
            pytest.fail("unsafe archive was written to a container")
    with pytest.raises(ValueError):
        validate_archive(path)
    with pytest.raises(ValueError):
        asyncio.run(restore(UnusedEnvironment(), path))


def test_capture_keeps_vendored_target_dirs_and_skips_build_caches():
    """A dir named target is source in vendored crates; only CACHEDIR.TAG dirs are caches."""
    import subprocess
    from scevals.image import DEFAULT_IMAGE
    from scevals.target import CAPTURE_TAR_COMMAND
    script = (
        "mkdir -p /workspace/public/repo/vendor/cc/src/target /workspace/public/repo/target/debug /workspace/.cache && "
        "echo src > /workspace/public/repo/vendor/cc/src/target/parser.rs && "
        "echo bin > /workspace/public/repo/target/debug/big && "
        "echo 'Signature: 8a477f597d28d172789f06886806bc55' > /workspace/public/repo/target/CACHEDIR.TAG && "
        "echo c > /workspace/.cache/x && cd /workspace && " + CAPTURE_TAR_COMMAND + " && tar -tf /tmp/workspace.tar")
    listed = subprocess.run(["docker", "run", "--rm", DEFAULT_IMAGE, "sh", "-c", script],
                            capture_output=True, text=True, check=True).stdout.split()
    assert "./public/repo/vendor/cc/src/target/parser.rs" in listed
    assert not any(name.startswith("./public/repo/target") for name in listed)
    assert not any(".cache" in name for name in listed)


def test_replay_container_cleanup_is_bounded(monkeypatch, tmp_path):
    """A removal that never returns is abandoned at its limit instead of holding the campaign."""
    import stat
    import time
    shim = tmp_path / "docker"
    shim.write_text("#!/bin/sh\nsleep 600\n")
    shim.chmod(shim.stat().st_mode | stat.S_IEXEC)
    monkeypatch.setenv("PATH", f"{tmp_path}:{os.environ['PATH']}")
    container = ReplayContainer("image", cleanup_seconds=1)
    container.container = "c"
    begun = time.monotonic()
    anyio.run(container.__aexit__, None, None, None)
    assert time.monotonic() - begun < 10


def test_cleanup_allowance_never_spends_the_publication_reserve():
    from types import SimpleNamespace
    from scevals.target import cleanup_allowance
    import time
    near = SimpleNamespace(guard=SimpleNamespace(remaining_to_deadline=lambda: 35.0))
    plenty = SimpleNamespace(guard=SimpleNamespace(remaining_to_deadline=lambda: 600.0))
    spent = SimpleNamespace(guard=SimpleNamespace(remaining_to_deadline=lambda: 0.0))
    assert cleanup_allowance(near) == 5.0
    assert cleanup_allowance(plenty) == ReplayContainer.CLEANUP_SECONDS
    assert cleanup_allowance(spent) == 1.0
    assert cleanup_allowance(SimpleNamespace(guard=None)) is None


def test_cleanup_deadline_is_computed_when_the_container_is_removed_not_when_it_is_created(monkeypatch, tmp_path):
    """Time spent in scoring between creation and removal must be deducted from the cleanup allowance."""
    import stat
    import time
    shim = tmp_path / "docker"
    shim.write_text("#!/bin/sh\nsleep 600\n")
    shim.chmod(shim.stat().st_mode | stat.S_IEXEC)
    monkeypatch.setenv("PATH", f"{tmp_path}:{os.environ['PATH']}")
    calls = []
    def allowance():
        calls.append(time.monotonic())
        return 1.0
    container = ReplayContainer("image", cleanup_seconds=allowance)
    container.container = "c"
    assert calls == []            # not evaluated at construction
    begun = time.monotonic()
    anyio.run(container.__aexit__, None, None, None)
    assert len(calls) == 1 and calls[0] >= begun and time.monotonic() - begun < 10
