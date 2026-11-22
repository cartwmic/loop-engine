"""Case validation must not implicitly rebuild the shared trial image."""
import json
import subprocess
from types import SimpleNamespace

import pytest
from scevals import cases, image


def fake_docker(daemon, image_arch, revision="a" * 40):
    """`output` for a daemon of one chip holding an image of another (or the same)."""
    def output(command):
        if command[0] == "git":
            return "a" * 40
        if command[:2] == ["docker", "version"]:
            return daemon
        return json.dumps([{"Architecture": image_arch, "Os": "linux", "Config": {
            "Labels": {"org.opencontainers.image.revision": revision}, "Volumes": None}}])
    return output


@pytest.mark.parametrize("daemon,image_arch", [("arm64", "arm64"), ("amd64", "amd64"), ("aarch64", "arm64"),
                                               ("x86_64", "amd64"), ("arm64", "aarch64")])
def test_verify_image_is_read_only_and_takes_an_image_of_the_daemons_arch(monkeypatch, daemon, image_arch):
    commands = []
    inner = fake_docker(daemon, image_arch)
    monkeypatch.setattr(image, "output", lambda command: commands.append(command) or inner(command))
    verified = image.verify_image()
    assert verified["commit"] == "a" * 40 and verified["architecture"] == image.normalise_arch(daemon)
    assert all(command[:2] != ["docker", "build"] for command in commands)


@pytest.mark.parametrize("daemon,image_arch,revision", [
    ("arm64", "amd64", "a" * 40), ("amd64", "arm64", "a" * 40),  # an image of the other chip is refused, both ways
    ("arm64", "arm64", "wrong"), ("amd64", "amd64", "wrong")])
def test_mismatched_image_requires_explicit_image_check(monkeypatch, daemon, image_arch, revision):
    monkeypatch.setattr(image, "output", fake_docker(daemon, image_arch, revision))
    with pytest.raises(RuntimeError, match="run image-check first"):
        image.verify_image()


def test_missing_image_requires_explicit_image_check(monkeypatch):
    def output(command):
        if command[0] == "git":
            return "a" * 40
        if command[:2] == ["docker", "version"]:
            return "amd64"
        raise subprocess.CalledProcessError(1, command)
    monkeypatch.setattr(image, "output", output)
    with pytest.raises(RuntimeError, match="run image-check first"):
        image.verify_image()


def test_case_check_missing_image_explains_next_step(monkeypatch, capsys):
    monkeypatch.setattr(cases, "load_case", lambda path: object())
    monkeypatch.setattr(cases, "unstated_interfaces", lambda case: [])
    def missing(*args):
        raise RuntimeError("private backend details")
    monkeypatch.setattr(cases, "verify_image", missing)
    assert cases.case_check(SimpleNamespace(path="ignored", ref="v0.24.0", tag=image.DEFAULT_IMAGE)) == 2
    assert capsys.readouterr().out == "Trial image missing or mismatched; run image-check first.\n"
