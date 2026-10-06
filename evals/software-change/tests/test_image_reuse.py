"""Case validation must not implicitly rebuild the shared trial image."""
import json
import subprocess
from types import SimpleNamespace

import pytest
from scevals import cases, image


def test_verify_image_is_read_only(monkeypatch):
    commands = []
    def output(command):
        commands.append(command)
        if command[0] == "git":
            return "a" * 40
        return json.dumps([{"Architecture": "arm64", "Os": "linux", "Config": {
            "Labels": {"org.opencontainers.image.revision": "a" * 40}, "Volumes": None}}])
    monkeypatch.setattr(image, "output", output)
    assert image.verify_image()["commit"] == "a" * 40
    assert all(command[:2] != ["docker", "build"] for command in commands)


@pytest.mark.parametrize("architecture,revision", [("amd64", "a" * 40), ("arm64", "wrong")])
def test_mismatched_image_requires_explicit_image_check(monkeypatch, architecture, revision):
    monkeypatch.setattr(image, "output", lambda command: "a" * 40 if command[0] == "git" else json.dumps([
        {"Architecture": architecture, "Os": "linux", "Config": {
            "Labels": {"org.opencontainers.image.revision": revision}}}]))
    with pytest.raises(RuntimeError, match="run image-check first"):
        image.verify_image()


def test_missing_image_requires_explicit_image_check(monkeypatch):
    def output(command):
        if command[0] == "git":
            return "a" * 40
        raise subprocess.CalledProcessError(1, command)
    monkeypatch.setattr(image, "output", output)
    with pytest.raises(RuntimeError, match="run image-check first"):
        image.verify_image()


def test_case_check_missing_image_explains_next_step(monkeypatch, capsys):
    monkeypatch.setattr(cases, "load_case", lambda path: object())
    def missing(*args):
        raise RuntimeError("private backend details")
    monkeypatch.setattr(cases, "verify_image", missing)
    assert cases.case_check(SimpleNamespace(path="ignored", ref="v0.24.0", tag=image.DEFAULT_IMAGE)) == 2
    assert capsys.readouterr().out == "Trial image missing or mismatched; run image-check first.\n"
