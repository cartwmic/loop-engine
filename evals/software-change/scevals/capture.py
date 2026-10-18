"""Private durable workspace snapshots and mount-free replay containers."""
import json
import tarfile
from pathlib import Path
from types import SimpleNamespace

import anyio

from evalcore.target import InfrastructureError


def read_capture(path):
    path = Path(path).resolve()
    if path.is_file():
        return path, {}
    return path / "workspace.tar", json.loads((path / "meta.json").read_text())


def captured_lifecycle(path):
    """Read the unmodified post-driver CLI receipt, after raw capture is durable."""
    with tarfile.open(path) as archive:
        for name in ("./trial/show.json", "trial/show.json"):
            try:
                packet = json.load(archive.extractfile(name))
                return packet.get("result", {}).get("lifecycle")
            except (KeyError, ValueError, TypeError, AttributeError):
                pass
    return None


def validate_archive(path):
    # Links themselves are workspace state (venvs often link /usr/bin/python).
    # Permit them in the disposable mount-free container, but NEVER write an
    # archive member through a symlink, irrespective of member order.
    with tarfile.open(path) as archive:
        members = archive.getmembers()
    symlinks = {Path(item.name) for item in members if item.issym()}
    for item in members:
        name = Path(item.name)
        if name.is_absolute() or ".." in name.parts or item.isdev() or item.isfifo():
            raise ValueError("unsafe workspace archive")
        if any(parent in symlinks for parent in name.parents):
            raise ValueError("workspace archive traverses a symlink")
        if item.islnk():
            target = Path(item.linkname)
            if (target.is_absolute() or ".." in target.parts or
                    any(parent in symlinks for parent in target.parents)):
                raise ValueError("unsafe workspace archive hardlink")
        if name in symlinks and not item.issym():
            raise ValueError("workspace archive replaces a symlink")


async def restore(env, archive):
    validate_archive(archive)
    await env.write_file("/tmp/restore-workspace.tar", Path(archive).read_bytes())
    result = await env.exec(["sh", "-c", "mkdir -p /workspace && tar -xf /tmp/restore-workspace.tar -C /workspace && rm /tmp/restore-workspace.tar"], timeout=600)
    if not result.success:
        raise InfrastructureError("workspace restore failed")


class ReplayContainer:
    """Small async file/exec surface shared with Inspect sandboxes; no host mounts."""
    def __init__(self, image, cleanup_seconds=None):
        self.image = image
        self.container = None
        # Seconds, or a callable evaluated when the container is removed (so time spent meanwhile is deducted).
        self.cleanup_seconds = self.CLEANUP_SECONDS if cleanup_seconds is None else cleanup_seconds

    async def __aenter__(self):
        result = await anyio.run_process(["docker", "run", "-d", "--platform", "linux/arm64", "--workdir", "/workspace", self.image, "sleep", "infinity"])
        self.container = result.stdout.decode().strip()
        return self

    CLEANUP_SECONDS = 60  # a stuck removal must not hold the campaign past its cap

    async def __aexit__(self, *args):
        seconds = self.cleanup_seconds() if callable(self.cleanup_seconds) else self.cleanup_seconds
        if seconds is None:
            seconds = self.CLEANUP_SECONDS
        with anyio.CancelScope(shield=True, deadline=anyio.current_time() + seconds):
            await anyio.run_process(["docker", "rm", "-f", self.container], check=False)

    async def exec(self, command, *, cwd=None, timeout=120, env=None):
        argv = ["docker", "exec"]
        if cwd:
            argv += ["-w", cwd]
        for key, value in (env or {}).items():
            argv += ["-e", f"{key}={value}"]
        with anyio.fail_after(timeout):
            result = await anyio.run_process([*argv, self.container, *command], check=False)
        return SimpleNamespace(success=result.returncode == 0, returncode=result.returncode,
                               stdout=result.stdout.decode("utf-8", "replace"), stderr=result.stderr.decode("utf-8", "replace"))

    async def read_file(self, path, text=True):
        result = await anyio.run_process(["docker", "exec", self.container, "cat", path])
        return result.stdout.decode() if text else result.stdout

    async def write_file(self, path, content):
        script = "import pathlib,sys;p=pathlib.Path(sys.argv[1]);p.parent.mkdir(parents=True,exist_ok=True);p.write_bytes(sys.stdin.buffer.read())"
        await anyio.run_process(["docker", "exec", "-i", self.container, "python3", "-c", script, path],
                                input=content.encode() if isinstance(content, str) else content)
