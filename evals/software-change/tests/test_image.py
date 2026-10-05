"""Fast mechanics; the separate bridge test proves real Pi and RPC behaviour."""
import importlib.util
import json
from pathlib import Path

import pytest

from scevals import image


def test_creation_has_no_mount_options():
    command = image.trial_container_command()
    assert command == ["docker", "create", "--platform", "linux/arm64", "--workdir",
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


def test_build_uses_committed_archive_and_native_platform(monkeypatch):
    commands = []
    commit = "a" * 40
    def output(command):
        commands.append(command)
        if command[0] == "docker":
            return "arm64"
        return '[workspace.package]\nversion="0.24.0"' if "show" in command else commit
    def run(command, **kwargs):
        commands.append(command)
        if command[:2] == ["docker", "build"]:
            assert Path(command[-1], "Dockerfile").exists()
    monkeypatch.setattr(image, "output", output)
    monkeypatch.setattr(image, "run", run)
    assert image.build_image()["commit"] == commit
    archive = next(command for command in commands if "archive" in command)
    assert archive[-1] == commit
    build = next(command for command in commands if command[:2] == ["docker", "build"])
    assert "linux/arm64" in build
    assert f"LOOP_ENGINE_COMMIT={commit}" in build


def test_non_arm64_is_refused(monkeypatch):
    monkeypatch.setattr(image, "output", lambda command: "amd64")
    with pytest.raises(RuntimeError, match="native arm64"):
        image.build_image()


def test_mounts_refused_and_container_removed(monkeypatch):
    commands = []
    monkeypatch.setattr(image, "output", lambda command: "container-id" if command[1] == "create"
                        else json.dumps([{"Mounts": [{"Source": "/host"}], "HostConfig": {}}]))
    monkeypatch.setattr(image, "run", lambda command, **kwargs: commands.append(command))
    with pytest.raises(RuntimeError, match="must not have mounts"):
        with image.trial_container():
            pytest.fail("mounted container was yielded")
    assert commands == [["docker", "rm", "--force", "container-id"]]
