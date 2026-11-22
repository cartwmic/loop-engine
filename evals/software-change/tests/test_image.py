"""Fast mechanics; the separate bridge test proves real Pi and RPC behaviour."""
import importlib.util
import json
from contextlib import contextmanager
from types import SimpleNamespace
from pathlib import Path

import pytest

from scevals import image


def fake_daemon(monkeypatch, arch):
    """Answer `docker version` with this chip; anything else is a test that forgot to fake its own commands."""
    def output(command, **kwargs):
        assert command[:2] == ["docker", "version"], command
        return arch
    monkeypatch.setattr(image, "output", output)


@pytest.mark.parametrize("arch", ["arm64", "amd64"])
def test_creation_has_no_mount_options(monkeypatch, arch):
    fake_daemon(monkeypatch, arch)
    command = image.trial_container_command()
    assert command == ["docker", "create", "--platform", f"linux/{arch}", "--workdir",
                       "/workspace", image.DEFAULT_IMAGE, "sleep", "infinity"]
    assert "VOLUME" not in (image.IMAGE_DIR / "Dockerfile").read_text().split("# No VOLUME")[0]


def test_role_configs_only_contain_their_alias(monkeypatch):
    spec = importlib.util.spec_from_file_location("pi_role", image.IMAGE_DIR / "pi-role.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    monkeypatch.setenv("SC_BRIDGE_URL", "http://localhost:14141/v1")
    for role in module.ROLES.values():
        provider = module.configuration(role)["providers"]["inspect"]
        assert provider["baseUrl"] == "http://localhost:14141/v1"
        assert [model["id"] for model in provider["models"]] == [role]
        assert provider["apiKey"] == "inspect-no-credential"


@pytest.mark.parametrize("reported,arch", [("arm64", "arm64"), ("amd64", "amd64"), ("aarch64", "arm64"),
                                           ("x86_64", "amd64")])
def test_build_uses_committed_archive_and_the_daemons_platform(monkeypatch, reported, arch):
    commands = []
    commit = "a" * 40
    def output(command):
        commands.append(command)
        if command[0] == "docker":
            return reported
        return '[workspace.package]\nversion="0.24.0"' if "show" in command else commit
    def run(command, **kwargs):
        commands.append(command)
        if command[:2] == ["docker", "build"]:
            assert Path(command[-1], "Dockerfile").exists()
    monkeypatch.setattr(image, "output", output)
    monkeypatch.setattr(image, "run", run)
    built = image.build_image()
    assert built["commit"] == commit and built["architecture"] == arch
    archive = next(command for command in commands if "archive" in command)
    assert archive[-1] == commit
    build = next(command for command in commands if command[:2] == ["docker", "build"])
    assert f"linux/{arch}" in build
    assert f"LOOP_ENGINE_COMMIT={commit}" in build


def test_the_daemon_is_asked_once(monkeypatch):
    asked = []
    monkeypatch.setattr(image, "output", lambda command: asked.append(command) or "amd64")
    assert [image.daemon_arch(), image.daemon_arch(), image.daemon_arch()] == ["amd64"] * 3
    assert asked == [["docker", "version", "--format", "{{.Server.Arch}}"]]


def test_mounts_refused_and_container_removed(monkeypatch):
    commands = []
    monkeypatch.setattr(image, "output", lambda command: "amd64" if command[1] == "version" else
                        "container-id" if command[1] == "create"
                        else json.dumps([{"Mounts": [{"Source": "/host"}], "HostConfig": {}}]))
    monkeypatch.setattr(image, "run", lambda command, **kwargs: commands.append(command))
    with pytest.raises(RuntimeError, match="must not have mounts"):
        with image.trial_container():
            pytest.fail("mounted container was yielded")
    assert commands == [["docker", "rm", "--force", "container-id"]]


@pytest.mark.parametrize("daemon,image_arch", [("amd64", "arm64"), ("arm64", "amd64")])
def test_image_check_refuses_an_image_of_another_arch_than_the_daemons(monkeypatch, daemon, image_arch):
    """The pre-flight (the receipt's own check) refuses an image the daemon would have to emulate, in either direction."""
    versions = {"loop-engine": "loop-engine 0.26.0", "software-change": "software-change 0.26.0", "dagu": "2.14.0",
                "pi": "1.0.0", "node": "v24", "git": "git 2", "cargo": "cargo 1.98.0", "rustc": "rustc 1.98.0 (x)"}
    def output(command):
        if command[:2] == ["docker", "version"]:
            return daemon
        if command[:2] == ["docker", "exec"]:
            return versions[command[3]]
        return json.dumps([{"Architecture": image_arch, "Os": "linux", "Config": {"Labels": {}}}])
    @contextmanager
    def container(tag):
        yield "c1", {"Image": "sha256:i", "Mounts": [], "HostConfig": {}}
    monkeypatch.setattr(image, "output", output)
    monkeypatch.setattr(image, "build_image", lambda ref, tag: {"ref": ref, "commit": "a" * 40, "version": "0.26.0",
                                                                "image": tag, "architecture": daemon})
    monkeypatch.setattr(image, "trial_container", container)
    args = SimpleNamespace(ref="v0.26.0", tag="t")
    with pytest.raises(RuntimeError, match=f"trial image is {image_arch}, not the daemon's {daemon}"):
        image.check_image(args)


def test_image_check_receipt_records_the_daemons_arch(monkeypatch, capsys):
    versions = {"loop-engine": "loop-engine 0.26.0", "software-change": "software-change 0.26.0", "dagu": "2.14.0",
                "pi": "1.0.0", "node": "v24", "git": "git 2", "cargo": "cargo 1.98.0", "rustc": "rustc 1.98.0 (x)"}
    roles = json.dumps({"providers": {"inspect": {"models": [{"id": "ROLE"}]}}})
    def output(command):
        if command[:2] == ["docker", "version"]:
            return "amd64"
        if command[:2] == ["docker", "exec"]:
            if command[3] == "cat":
                return roles.replace("ROLE", command[4].split("/")[-2])
            return versions[command[3]]
        return json.dumps([{"Architecture": "amd64", "Os": "linux",
                            "Config": {"Labels": {"org.opencontainers.image.revision": "a" * 40}}}])
    @contextmanager
    def container(tag):
        yield "c1", {"Image": "sha256:i", "Mounts": [], "HostConfig": {}}
    monkeypatch.setattr(image, "output", output)
    monkeypatch.setattr(image, "run", lambda command, **kwargs: None)
    monkeypatch.setattr(image, "build_image", lambda ref, tag: {"ref": ref, "commit": "a" * 40, "version": "0.26.0",
                                                                "image": tag, "architecture": "amd64"})
    monkeypatch.setattr(image, "trial_container", container)
    assert image.check_image(SimpleNamespace(ref="v0.26.0", tag="t")) == 0
    receipt = json.loads(capsys.readouterr().out.split("IMAGE_CHECK ")[1])
    assert receipt["architecture"] == "amd64"
