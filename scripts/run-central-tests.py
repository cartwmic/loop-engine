#!/usr/bin/env python3
"""Build the real workspace binaries, hand them to the central test target, and run it.

This is the supported T02 command.  It deliberately owns the build/test
boundary instead of asking a Cargo build script to invoke nested Cargo.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any

REPO = Path(__file__).resolve().parents[1]
HANDOFF_ENV = "LOOP_ENGINE_TEST_BINARY_HANDOFF"
REQUIRED = (
    ("bookends-check", "bookends-check", "bookends-check"),
    ("loop-cli", "loop-engine", "loop-engine"),
    ("policy-document-provider", "policy-document", "policy-document"),
    ("research-provider", "research", "research"),
    ("software-change-provider", "software-change", "software-change"),
    (
        "loop-reference-fixtures",
        "policy-document-provider",
        "fixture:policy-document-provider",
    ),
    ("loop-reference-fixtures", "research-provider", "fixture:research-provider"),
    (
        "loop-reference-fixtures",
        "software-change-provider",
        "fixture:software-change-provider",
    ),
)


class RunnerError(RuntimeError):
    def __init__(self, message: str, exit_status: int = 1):
        super().__init__(message)
        self.exit_status = exit_status if 1 <= exit_status <= 255 else 1


def package_manifest(package: str) -> Path:
    if package == "loop-reference-fixtures":
        return REPO / "tests/fixtures/Cargo.toml"
    return REPO / "crates" / package / "Cargo.toml"


def run_metadata(environment: dict[str, str]) -> dict[str, Any]:
    result = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--no-deps"],
        cwd=REPO,
        env=environment,
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode:
        raise RunnerError(result.stderr.strip() or "cargo metadata failed")
    try:
        value = json.loads(result.stdout)
    except json.JSONDecodeError as error:
        raise RunnerError(f"cargo metadata returned invalid JSON: {error}") from error
    if not isinstance(value, dict):
        raise RunnerError("cargo metadata result must be an object")
    return value


def active_target(environment: dict[str, str], metadata: dict[str, Any]) -> Path:
    target = metadata.get("target_directory")
    if not isinstance(target, str) or not target:
        raise RunnerError("cargo metadata did not report target_directory")
    path = Path(target)
    if not path.is_absolute():
        path = REPO / path
    return path.resolve()


def package_name_from_artifact(message: dict[str, Any]) -> str | None:
    manifest = message.get("manifest_path")
    if not isinstance(manifest, str):
        return None
    manifest_path = Path(manifest).resolve()
    for package, _target, _alias in REQUIRED:
        if manifest_path == package_manifest(package).resolve():
            return package
    return None


def is_executable(path: Path) -> bool:
    return path.is_file() and os.access(path, os.X_OK)


def build_binaries(environment: dict[str, str], target: Path) -> tuple[dict[str, Any], list[str]]:
    build_command = [
        "cargo",
        "build",
        "--locked",
        "--message-format=json",
        "-p",
        "bookends-check",
        "-p",
        "loop-cli",
        "-p",
        "policy-document-provider",
        "-p",
        "research-provider",
        "-p",
        "software-change-provider",
        "-p",
        "loop-reference-fixtures",
        "--bins",
    ]
    result = subprocess.run(
        build_command,
        cwd=REPO,
        env=environment,
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode:
        if result.stdout:
            print(result.stdout, end="", file=sys.stderr)
        if result.stderr:
            print(result.stderr, end="", file=sys.stderr)
        raise RunnerError(
            f"binary build failed with exit {result.returncode}",
            result.returncode,
        )

    by_pair: dict[tuple[str, str], dict[str, Any]] = {}
    for line_number, line in enumerate(result.stdout.splitlines(), 1):
        if not line.strip():
            continue
        try:
            message = json.loads(line)
        except json.JSONDecodeError as error:
            raise RunnerError(f"cargo build emitted invalid JSON at line {line_number}: {error}") from error
        if message.get("reason") != "compiler-artifact":
            continue
        target_info = message.get("target")
        if not isinstance(target_info, dict) or target_info.get("kind") != ["bin"]:
            continue
        executable = message.get("executable")
        package = package_name_from_artifact(message)
        target_name = target_info.get("name")
        if not isinstance(executable, str) or not isinstance(package, str) or not isinstance(target_name, str):
            continue
        pair = (package, target_name)
        if pair not in {(package_name, target_name) for package_name, target_name, _alias in REQUIRED}:
            continue
        path = Path(executable).resolve()
        if not path.is_relative_to(target):
            raise RunnerError(
                f"Cargo handed off {path}, outside active target {target}; refusing to run it"
            )
        if path.name not in {target_name, f"{target_name}.exe"}:
            raise RunnerError(f"Cargo handed off a non-canonical path for {package}/{target_name}: {path}")
        if not is_executable(path):
            raise RunnerError(f"Cargo build output is not executable: {path}")
        by_pair[pair] = {
            "package": package,
            "target": target_name,
            "path": str(path),
            "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
            "fresh": message.get("fresh"),
            "source": str(Path(str(target_info.get("src_path", ""))).resolve().relative_to(REPO)),
        }

    missing = [
        f"{package}/{target_name}"
        for package, target_name, _alias in REQUIRED
        if (package, target_name) not in by_pair
    ]
    if missing:
        raise RunnerError("current Cargo build did not produce required binaries: " + ", ".join(missing))

    entries = {
        alias: by_pair[(package, target_name)]
        for package, target_name, alias in REQUIRED
    }
    return entries, build_command


def write_handoff(target: Path, entries: dict[str, Any], build_command: list[str]) -> Path:
    target.mkdir(parents=True, exist_ok=True)
    revision = subprocess.run(
        ["git", "rev-parse", "HEAD"],
        cwd=REPO,
        capture_output=True,
        text=True,
        check=True,
    ).stdout.strip()
    state = subprocess.run(
        ["git", "status", "--porcelain=v1", "--untracked-files=all"],
        cwd=REPO,
        capture_output=True,
        text=True,
        check=True,
    ).stdout.splitlines()
    payload = {
        "schema_version": 1,
        "repository_root": str(REPO.resolve()),
        "active_target": str(target),
        "build": {
            "command": build_command,
            "revision": revision,
            "repository_state": revision + "+uncommitted-worktree",
            "status": state,
        },
        "binaries": entries,
    }
    handle = tempfile.NamedTemporaryFile(
        mode="w", encoding="utf-8", prefix="loop-engine-central-handoff-", suffix=".json", dir=target, delete=False
    )
    path = Path(handle.name)
    with handle:
        json.dump(payload, handle, indent=2, sort_keys=True)
        handle.write("\n")
    return path


def run_command(command: list[str], environment: dict[str, str], capture: Path | None = None) -> None:
    if capture is None:
        result = subprocess.run(command, cwd=REPO, env=environment, check=False)
    else:
        result = subprocess.run(
            command,
            cwd=REPO,
            env=environment,
            capture_output=True,
            text=True,
            check=False,
        )
        capture.write_text(result.stdout, encoding="utf-8")
        if result.stderr:
            print(result.stderr, end="", file=sys.stderr)
    if result.returncode:
        raise RunnerError(
            f"command failed with exit {result.returncode}: {' '.join(command)}",
            result.returncode,
        )


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--filter", help="run only central integration tests matching this substring")
    parser.add_argument(
        "--no-run",
        action="store_true",
        help="compile the resolver and central integration target without executing tests",
    )
    parser.add_argument(
        "--compiler-artifacts",
        type=Path,
        help="write the central Cargo compiler-artifact JSON stream to this path with --no-run",
    )
    parser.add_argument(
        "--handoff-output",
        type=Path,
        help=(
            "copy the fresh binary handoff to this external path and keep it after the "
            "command; useful for the direct stock Cargo compatibility gate"
        ),
    )
    parser.add_argument(
        "--nextest",
        action="store_true",
        help="run the central target through cargo-nextest instead of Cargo",
    )
    parser.add_argument(
        "--nextest-run-ignored",
        choices=("only", "all"),
        help="pass --run-ignored to cargo-nextest (requires --nextest)",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    if args.compiler_artifacts is not None and not args.no_run:
        print("--compiler-artifacts requires --no-run", file=sys.stderr)
        return 2
    if args.nextest and args.no_run:
        print("--nextest cannot be combined with --no-run", file=sys.stderr)
        return 2
    if args.nextest_run_ignored and not args.nextest:
        print("--nextest-run-ignored requires --nextest", file=sys.stderr)
        return 2
    environment = os.environ.copy()
    environment["CARGO_TERM_COLOR"] = "never"
    handoff_output = args.handoff_output.resolve() if args.handoff_output is not None else None
    if handoff_output is not None:
        try:
            handoff_output.relative_to(REPO.resolve())
        except ValueError:
            pass
        else:
            print("--handoff-output must remain outside the repository", file=sys.stderr)
            return 2
        try:
            handoff_output.unlink(missing_ok=True)
        except OSError as error:
            print(f"could not remove stale handoff output {handoff_output}: {error}", file=sys.stderr)
            return 1
    metadata = run_metadata(environment)
    target = active_target(environment, metadata)
    handoff: Path | None = None
    try:
        entries, build_command = build_binaries(environment, target)
        handoff = write_handoff(target, entries, build_command)
        if handoff_output is not None:
            handoff_output.parent.mkdir(parents=True, exist_ok=True)
            temporary_output = handoff_output.with_name(
                f".{handoff_output.name}.tmp-{os.getpid()}"
            )
            shutil.copyfile(handoff, temporary_output)
            os.replace(temporary_output, handoff_output)
        environment[HANDOFF_ENV] = str(handoff)
        print(f"central binary handoff: {handoff}")
        if handoff_output is not None:
            print(f"persistent binary handoff: {handoff_output}")
        print(f"active Cargo target: {target}")

        if args.nextest:
            central_command = ["cargo", "nextest", "run", "--locked"]
            if args.nextest_run_ignored:
                central_command.extend(["--run-ignored", args.nextest_run_ignored])
            if args.filter:
                # Focused runs stay on the central target.  The unfiltered
                # ordinary command below covers every workspace unit target.
                central_command.extend(
                    ["-p", "workspace-integration", "--test", "workspace", "--", args.filter]
                )
            else:
                central_command.append("--workspace")
            run_command(central_command, environment)
        else:
            central_command = [
                "cargo",
                "test",
                "--locked",
                "-p",
                "workspace-integration",
                "--test",
                "workspace",
            ]
            if args.no_run:
                central_command.append("--no-run")
                central_command.append("--message-format=json")
            elif args.filter:
                central_command.extend(["--", args.filter, "--test-threads=4"])
            else:
                central_command.extend(["--", "--test-threads=4"])

            if args.no_run:
                run_command(central_command, environment, args.compiler_artifacts)
            else:
                run_command(
                    ["cargo", "test", "--locked", "-p", "workspace-integration", "--lib"],
                    environment,
                )
                run_command(central_command, environment)
        return 0
    except RunnerError as error:
        print(f"central test runner failed closed: {error}", file=sys.stderr)
        return error.exit_status
    finally:
        # Handoff paths are intentionally ephemeral.  No later invocation may
        # accidentally reuse a prior build's path.
        if handoff is not None:
            try:
                handoff.unlink()
            except FileNotFoundError:
                pass


if __name__ == "__main__":
    raise SystemExit(main())
