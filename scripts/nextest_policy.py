"""Shared policy and exact-version checks for the repository's nextest path."""

from __future__ import annotations

import ast
import hashlib
import json
import subprocess
import tempfile
from pathlib import Path
from typing import Any

try:
    import tomllib
except ModuleNotFoundError:  # Python 3.10 on older supported CI images.
    tomllib = None  # type: ignore[assignment]

ROOT = Path(__file__).resolve().parents[1]
CONFIG_PATH = ROOT / ".config/nextest.toml"
PINNED_VERSION = "0.9.143"
TIMEOUT_TEST = "process_timeout_probe"


class PolicyError(RuntimeError):
    """The checked-in nextest policy or installed runner is not usable."""


def _read_config(path: Path) -> tuple[dict[str, Any], str]:
    try:
        text = path.read_text(encoding="utf-8")
    except OSError as error:
        raise PolicyError(f"could not read nextest config {path}: {error}") from error
    try:
        value = tomllib.loads(text) if tomllib is not None else _minimal_toml_loads(text)
    except (ValueError, json.JSONDecodeError) as error:
        raise PolicyError(f"nextest config {path} is invalid TOML: {error}") from error
    if not isinstance(value, dict):
        raise PolicyError("nextest config must be a TOML table")
    return value, text


def _minimal_toml_loads(text: str) -> dict[str, Any]:
    """Parse the small repository config on Python versions without tomllib."""
    result: dict[str, Any] = {}
    default: dict[str, Any] = {}
    result["profile"] = {"default": default}
    overrides: list[dict[str, Any]] = []
    current: dict[str, Any] = result
    for raw_line in text.splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        if line == "[profile.default]":
            current = default
            continue
        if line == "[[profile.default.overrides]]":
            current = {}
            overrides.append(current)
            default["overrides"] = overrides
            continue
        if "=" not in line:
            raise ValueError(f"unsupported TOML line: {raw_line}")
        key, raw_value = (part.strip() for part in line.split("=", 1))
        current[key] = _minimal_toml_value(raw_value)
    return result


def _minimal_toml_value(raw_value: str) -> Any:
    if raw_value.startswith("{") and raw_value.endswith("}"):
        values: dict[str, Any] = {}
        inner = raw_value[1:-1].strip()
        for part in inner.split(","):
            key, value = (item.strip() for item in part.split("=", 1))
            values[key] = _minimal_toml_value(value)
        return values
    if raw_value.isdigit():
        return int(raw_value)
    if raw_value.startswith(("\"", "'")):
        try:
            return json.loads(raw_value) if raw_value.startswith('"') else ast.literal_eval(raw_value)
        except (ValueError, SyntaxError, json.JSONDecodeError) as error:
            raise ValueError(f"invalid TOML string {raw_value!r}: {error}") from error
    raise ValueError(f"unsupported TOML value: {raw_value}")


def validate_config(path: Path = CONFIG_PATH) -> dict[str, Any]:
    """Validate the exact settings required by the repository test contract."""
    value, text = _read_config(path)
    version = value.get("nextest-version")
    if version != PINNED_VERSION:
        raise PolicyError(
            f"nextest config pins {version!r}; expected exact version {PINNED_VERSION}"
        )

    profiles = value.get("profile")
    if not isinstance(profiles, dict):
        raise PolicyError("nextest config must define profile.default")
    default = profiles.get("default")
    if not isinstance(default, dict):
        raise PolicyError("nextest config must define profile.default")
    if default.get("status-level") != "fail":
        raise PolicyError('profile.default.status-level must be "fail"')
    if default.get("final-status-level") != "slow":
        raise PolicyError('profile.default.final-status-level must be "slow"')
    if default.get("test-threads") != 4:
        raise PolicyError("profile.default.test-threads must be 4; the suite must stay parallel")

    ordinary_timeout = default.get("slow-timeout")
    _validate_timeout(
        ordinary_timeout,
        period="30s",
        terminate_after=2,
        label="profile.default.slow-timeout",
    )

    overrides = default.get("overrides")
    if not isinstance(overrides, list):
        raise PolicyError("profile.default.overrides must contain the timeout probe override")
    probe_overrides = [
        override
        for override in overrides
        if isinstance(override, dict)
        and override.get("filter") == f"test({TIMEOUT_TEST})"
    ]
    if len(probe_overrides) != 1:
        raise PolicyError(
            f"profile.default must contain exactly one test({TIMEOUT_TEST}) override"
        )
    _validate_timeout(
        probe_overrides[0].get("slow-timeout"),
        period="1s",
        terminate_after=1,
        label=f"test({TIMEOUT_TEST}) slow-timeout",
    )

    return {
        "path": str(path.resolve()),
        "sha256": hashlib.sha256(text.encode("utf-8")).hexdigest(),
        "pinned_version": PINNED_VERSION,
        "profile": {
            "status_level": default["status-level"],
            "final_status_level": default["final-status-level"],
            "ordinary_slow_timeout": _timeout_summary(ordinary_timeout),
            "timeout_probe_filter": f"test({TIMEOUT_TEST})",
            "timeout_probe_slow_timeout": _timeout_summary(probe_overrides[0]["slow-timeout"]),
            "test_threads": default.get("test-threads", "num-cpus (nextest default)"),
        },
    }


def _validate_timeout(
    value: Any, *, period: str, terminate_after: int, label: str
) -> None:
    if not isinstance(value, dict):
        raise PolicyError(f"{label} must be an inline timeout table")
    if value.get("period") != period:
        raise PolicyError(f"{label}.period must be {period!r}")
    if value.get("terminate-after") != terminate_after:
        raise PolicyError(f"{label}.terminate-after must be {terminate_after}")
    if value.get("grace-period") != "0s":
        raise PolicyError(f"{label}.grace-period must be '0s'")
    if value.get("on-timeout") != "fail":
        raise PolicyError(f"{label}.on-timeout must be 'fail'")


def _timeout_summary(value: dict[str, Any]) -> dict[str, Any]:
    return {
        "period": value["period"],
        "terminate_after": value["terminate-after"],
        "grace_period": value["grace-period"],
        "on_timeout": value["on-timeout"],
    }


def check_installed_version(cwd: Path = ROOT) -> dict[str, Any]:
    """Require cargo-nextest's reported version to equal the repository pin."""
    command = ["cargo", "nextest", "--version"]
    try:
        result = subprocess.run(
            command,
            cwd=cwd,
            capture_output=True,
            text=True,
            check=False,
        )
    except OSError as error:
        raise PolicyError(f"could not execute {' '.join(command)}: {error}") from error
    stdout = result.stdout.strip()
    stderr = result.stderr.strip()
    expected = f"cargo-nextest {PINNED_VERSION}"
    first_line = stdout.splitlines()[0].strip() if stdout else ""
    reported = first_line.removeprefix("cargo-nextest ").split(maxsplit=1)[0]
    if result.returncode != 0:
        detail = stderr or stdout or "no diagnostic"
        raise PolicyError(
            f"{' '.join(command)} failed with exit {result.returncode}: {detail}"
        )
    if not first_line.startswith("cargo-nextest ") or reported != PINNED_VERSION:
        raise PolicyError(
            f"cargo-nextest version mismatch: expected {expected!r}, got {first_line!r}"
        )
    return {
        "command": command,
        "exit_status": result.returncode,
        "expected": expected,
        "reported_version": reported,
        "stdout": result.stdout,
        "stderr": result.stderr,
    }


def self_test() -> None:
    """Exercise policy parsing and fail-closed rejection without running Cargo."""
    valid = """\
nextest-version = \"0.9.143\"
[profile.default]
status-level = \"fail\"
final-status-level = \"slow\"
test-threads = 4
slow-timeout = { period = \"30s\", terminate-after = 2, grace-period = \"0s\", on-timeout = \"fail\" }
[[profile.default.overrides]]
filter = 'test(process_timeout_probe)'
slow-timeout = { period = \"1s\", terminate-after = 1, grace-period = \"0s\", on-timeout = \"fail\" }
"""
    with tempfile.TemporaryDirectory(prefix="loop-engine-nextest-policy-") as directory:
        path = Path(directory) / "nextest.toml"
        path.write_text(valid, encoding="utf-8")
        summary = validate_config(path)
        if summary["pinned_version"] != PINNED_VERSION:
            raise PolicyError("policy self-test did not preserve the pinned version")

        invalid = Path(directory) / "invalid.toml"
        invalid.write_text(valid.replace('terminate-after = 2', 'terminate-after = 1'), encoding="utf-8")
        try:
            validate_config(invalid)
        except PolicyError:
            pass
        else:
            raise PolicyError("policy self-test accepted an invalid timeout")
