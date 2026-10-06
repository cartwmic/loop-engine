"""Workspace replay link fidelity and pre-extraction write-traversal checks."""
import asyncio
import io
import json
from pathlib import Path
import tarfile
import uuid

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
