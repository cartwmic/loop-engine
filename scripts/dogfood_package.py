#!/usr/bin/env python3
"""Validate externally prepared cargo-dist packages and run packaged-smoke.

This adapter never builds, installs, extracts, or copies packages. Installed
mode reads an external temporary prefix; archive mode reads cargo-dist archives
and their sidecars. Identity hashes are computed from the exact supplied bytes.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tarfile
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
APPS = {
    "loop-cli": "loop-engine",
    "software-change-provider": "software-change",
    "policy-document-provider": "policy-document",
    "research-provider": "research",
}


def digest(path: Path) -> str:
    hasher = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            hasher.update(block)
    return hasher.hexdigest()


def parse_checksum(path: Path, archive: Path) -> str:
    text = path.read_text(encoding="utf-8")
    matches = re.findall(r"(?<![0-9A-Fa-f])[0-9A-Fa-f]{64}(?![0-9A-Fa-f])", text)
    if len(matches) != 1:
        raise ValueError(f"{path} must contain exactly one SHA-256 digest")
    declared = matches[0].lower()
    actual = digest(archive)
    if declared != actual:
        raise ValueError(
            f"checksum mismatch for {archive.name}: sidecar={declared}, archive={actual}"
        )
    return actual


def archive_has_executable(archive_path: Path, binary: str) -> bool:
    with tarfile.open(archive_path, mode="r:xz") as archive:
        matches = [
            member
            for member in archive.getmembers()
            if member.isfile()
            and Path(member.name).name == binary
            and member.mode & 0o111
        ]
    if len(matches) != 1:
        raise ValueError(
            f"{archive_path} must contain exactly one executable named {binary}; found {len(matches)}"
        )
    return True


def supplied_inputs(mode: str, version: str, platform: str, package_root: Path):
    paths: dict[str, Path] = {}
    hashes: dict[str, str] = {}
    checksums: dict[str, str] = {}
    if not package_root.is_absolute() or not package_root.is_dir():
        raise ValueError(f"package-root must be an existing absolute directory: {package_root}")

    for app, binary in APPS.items():
        if mode == "installed":
            path = package_root / "bin" / binary
            if path.is_symlink() or not path.is_file() or not os.access(path, os.X_OK):
                raise ValueError(f"requires a genuinely supplied executable at {path}")
            paths[app] = path.resolve()
            hashes[app] = digest(path)
        else:
            path = package_root / f"{app}-{platform}.tar.xz"
            sidecar = Path(str(path) + ".sha256")
            if not path.is_file() or not sidecar.is_file():
                raise ValueError(f"requires cargo-dist archive and checksum sidecar: {path} and {sidecar}")
            archive_has_executable(path, binary)
            actual = parse_checksum(sidecar, path)
            paths[app] = path.resolve()
            hashes[app] = actual
            checksums[app] = actual
    return paths, hashes, checksums


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mode", required=True, choices=("installed", "archive"))
    parser.add_argument("--version", required=True)
    parser.add_argument("--platform", required=True)
    parser.add_argument("--package-root", required=True, type=Path)
    parser.add_argument("--output-root", required=True, type=Path)
    args = parser.parse_args(argv)

    if not args.version.strip() or not args.platform.strip():
        parser.error("--version and --platform must be non-empty")
    if not args.package_root.is_absolute():
        parser.error("--package-root must be absolute")
    if not args.output_root.is_absolute():
        parser.error("--output-root must be absolute")
    output_root = args.output_root.resolve()
    if output_root == ROOT or ROOT in output_root.parents:
        parser.error("--output-root must be outside the checkout")

    output_root.mkdir(parents=True, exist_ok=True)
    identity_path = output_root / f"package-identity-{args.mode}.json"
    outcome_path = output_root / "dogfood-package-outcome.json"
    capture_root = output_root / "adapter-capture"
    smoke_root = output_root / "packaged-smoke"
    if os.path.lexists(outcome_path):
        print(json.dumps({
            "mode": args.mode,
            "status": "failed",
            "diagnostic": "output-root already contains a package outcome; refusing to overwrite evidence",
        }))
        return 1
    outcome: dict[str, Any] = {
        "mode": args.mode,
        "version": args.version,
        "platform": args.platform,
        "package_root": str(args.package_root),
        "output_root": str(output_root),
        "identity_path": str(identity_path),
        "status": "failed",
    }
    try:
        if any(path.exists() for path in (identity_path, outcome_path, capture_root, smoke_root)):
            raise ValueError("output-root already contains evidence for this package mode; use a fresh output root")
        capture_root.mkdir()
        paths, hashes, checksums = supplied_inputs(
            args.mode, args.version, args.platform, args.package_root
        )
        identity = {
            "version": args.version,
            "platform": args.platform,
            "sha256": hashes,
        }
        identity_path.write_text(json.dumps(identity, indent=2) + "\n", encoding="utf-8")

        child_argv = [
            sys.executable,
            str(ROOT / "scripts/packaged-smoke.py"),
            "--mode",
            args.mode,
            "--expected-version",
            args.version,
            "--platform",
            args.platform,
            "--output-root",
            str(smoke_root),
            "--package-identity",
            str(identity_path),
        ]
        for app in APPS:
            if args.mode == "installed":
                child_argv.extend(["--binary", f"{app}={paths[app]}"])
            else:
                child_argv.extend(
                    ["--archive", f"{app}={paths[app]}", "--checksum", f"{app}={checksums[app]}"]
                )
        stdout_path = capture_root / "packaged-smoke.stdout"
        stderr_path = capture_root / "packaged-smoke.stderr"
        argv_path = capture_root / "packaged-smoke.argv.json"
        argv_path.write_text(json.dumps(child_argv, indent=2) + "\n", encoding="utf-8")
        outcome["child_argv"] = child_argv
        outcome["child_stdout"] = str(stdout_path)
        outcome["child_stderr"] = str(stderr_path)
        outcome["child_argv_capture"] = str(argv_path)
        try:
            with stdout_path.open("wb") as stdout, stderr_path.open("wb") as stderr:
                child = subprocess.run(
                    child_argv,
                    cwd=ROOT,
                    stdout=stdout,
                    stderr=stderr,
                    timeout=7200,
                    check=False,
                )
        except (OSError, subprocess.TimeoutExpired) as error:
            raise ValueError(f"packaged-smoke could not complete: {error}") from error
        outcome["child_exit_code"] = child.returncode
        outcome["child_captures"] = [str(stdout_path), str(stderr_path), str(argv_path)]

        if not smoke_root.is_dir():
            raise ValueError("packaged-smoke did not create its output root")
        child_outcomes = sorted(smoke_root.glob("packaged-*/outcome.json"))
        if len(child_outcomes) != 1:
            raise ValueError(f"expected one packaged-smoke outcome.json, found {len(child_outcomes)}")
        child_outcome_path = child_outcomes[0]
        child_outcome = json.loads(child_outcome_path.read_text(encoding="utf-8"))
        outcome["packaged_smoke_outcome"] = str(child_outcome_path)
        outcome["packaged_smoke"] = child_outcome
        if child_outcome.get("mode") != args.mode:
            raise ValueError("packaged-smoke outcome mode differs from the requested package mode")
        artifact_root = Path(child_outcome.get("artifact_root", ""))
        if artifact_root.resolve() != child_outcome_path.parent.resolve():
            raise ValueError("packaged-smoke outcome points at a different artifact root")
        commands_path = child_outcome_path.parent / "commands.json"
        outcome["packaged_smoke_commands"] = str(commands_path)
        if not commands_path.is_file():
            raise ValueError("packaged-smoke child command capture index is missing")
        commands = json.loads(commands_path.read_text(encoding="utf-8"))
        if not commands or any(
            not Path(record.get(key, "")).is_file()
            for record in commands
            for key in ("stdout", "stderr")
        ):
            raise ValueError("packaged-smoke child command captures are missing")
        if child_outcome.get("status") != "passed" or child.returncode != 0:
            raise ValueError(
                f"packaged-smoke did not pass (exit={child.returncode}, status={child_outcome.get('status')!r})"
            )
        echoed_identity = json.loads(
            (child_outcome_path.parent / "package-identity.json").read_text(encoding="utf-8")
        )
        if echoed_identity != identity:
            raise ValueError("packaged-smoke used package identity bytes that differ from the supplied identity")
        outcome["status"] = "passed"
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError, tarfile.TarError) as error:
        outcome["diagnostic"] = str(error)
    try:
        with outcome_path.open("x", encoding="utf-8") as stream:
            json.dump(outcome, stream, indent=2)
            stream.write("\n")
    except FileExistsError:
        print(json.dumps({
            "mode": args.mode,
            "status": "failed",
            "diagnostic": "package outcome appeared concurrently; refusing to overwrite evidence",
        }))
        return 1
    print(json.dumps(outcome))
    return 0 if outcome["status"] == "passed" else 1


if __name__ == "__main__":
    sys.exit(main())
