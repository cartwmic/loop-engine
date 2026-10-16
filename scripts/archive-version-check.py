#!/usr/bin/env python3
"""Run every archived executable; require one exact version token, optionally named."""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import tarfile
import tempfile
import zipfile
from pathlib import Path


class CheckError(RuntimeError):
    pass


def check_binary(binary: Path, expected: str) -> None:
    try:
        result = subprocess.run(
            [str(binary.resolve()), "--version"], capture_output=True, text=True, timeout=30
        )
    except (OSError, UnicodeError, subprocess.TimeoutExpired) as error:
        raise CheckError(f"{binary.name}: could not run --version: {error}") from error
    if result.returncode != 0:
        raise CheckError(f"{binary.name}: --version exited {result.returncode}")
    # loop-engine already prints a bare version; providers print '<name> <version>'.
    # Accept both public forms, but never a prefix, extra version, or extra output.
    match = re.fullmatch(r"(?:(\S+)[ \t]+)?(\S+)\n?", result.stdout)
    semver = r"\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?"
    if (not match or match.group(2) != expected
            or (match.group(1) and re.fullmatch(semver, match.group(1)))):
        raise CheckError(
            f"{binary.name}: expected one exact version token {expected}, got {result.stdout!r}"
        )
    print(f"PASS {binary.name} --version: {result.stdout.strip()}")


def check_directory(directory: Path, expected: str, required: set[str] | None) -> None:
    binaries = sorted(
        path for path in directory.rglob("*") if path.is_file() and os.access(path, os.X_OK)
    )
    if not binaries:
        raise CheckError(f"{directory}: no executables found")
    if required is not None and not required.issubset({path.name for path in binaries}):
        raise CheckError(f"{directory}: missing planned executables {sorted(required)}")
    for binary in binaries:
        check_binary(binary, expected)


def check_archive(path: Path, expected: str, required: set[str] | None = None) -> None:
    if path.is_dir():
        check_directory(path, expected, required)
        return
    if not path.is_file():
        raise CheckError(f"missing archive: {path}")
    with tempfile.TemporaryDirectory(prefix="archive-version-") as temp:
        directory = Path(temp)
        if zipfile.is_zipfile(path):
            with zipfile.ZipFile(path) as archive:
                archive.extractall(directory)
                for member in archive.infolist():
                    mode = member.external_attr >> 16
                    if mode:
                        (directory / member.filename).chmod(mode & 0o777)
        else:
            with tarfile.open(path) as archive:
                archive.extractall(directory, filter="data")
        check_directory(directory, expected, required)


def check_plan(plan: Path, target: str, artifact_dir: Path) -> None:
    manifest = json.loads(plan.read_text())
    checked = 0
    for release in manifest["releases"]:
        archives = []
        for name in release["artifacts"]:
            artifact = manifest["artifacts"][name]
            if artifact["kind"] == "executable-zip" and target in artifact["target_triples"]:
                archives.append((name, artifact))
        if not archives:
            raise CheckError(f"{release['app_name']}: no app archive planned for {target}")
        for name, artifact in archives:
            required = {
                asset["name"] for asset in artifact["assets"] if asset["kind"] == "executable"
            }
            if not required:
                raise CheckError(f"{name}: no executable assets planned")
            print(f"Checking {name} at release version {release['app_version']}")
            check_archive(artifact_dir / name, release["app_version"], required)
            checked += 1
    if not checked:
        raise CheckError(f"no app archives planned for {target}")
    print(f"Archive versions OK: {checked} archives for {target}")


def self_test() -> None:
    version = "0.24.1"
    with tempfile.TemporaryDirectory(prefix="archive-version-self-test-") as temp:
        root = Path(temp)
        cases = [("correct", "fixture " + version, 0), ("bare", version, 0),
                 ("nonzero", "fixture " + version, 7),
                 ("prefix-zero", "fixture " + version + "0", 0),
                 ("prefix-prerelease", "fixture " + version + "-rc.1", 0),
                 ("duplicate", version + " " + version, 0)]
        for name, reported, status in cases:
            directory = root / name
            directory.mkdir()
            binary = directory / "fixture"
            binary.write_text(
                f"#!{sys.executable}\nimport sys\n"
                f"assert sys.argv[1:] == ['--version']\n"
                f"print('{reported}')\nsys.exit({status})\n"
            )
            binary.chmod(0o755)
            archive_path = root / f"{name}.tar.xz"
            with tarfile.open(archive_path, "w:xz") as archive:
                archive.add(directory, arcname=name)
            try:
                check_archive(archive_path, version, {"fixture"})
            except CheckError as error:
                if name in {"correct", "bare"}:
                    raise
                print(f"Expected rejection {name}: {error}")
            else:
                if name not in {"correct", "bare"}:
                    raise CheckError(f"self-test accepted {name}")
                check_archive(directory, version)
        # Do not pass vacuously when an archive/extracted directory lost its binary.
        empty = root / "empty"
        empty.mkdir()
        try:
            check_archive(empty, version)
        except CheckError:
            print("Expected rejection empty directory: no executables")
        else:
            raise CheckError("self-test accepted an empty directory")
    print("Archive checker self-test passed: correct accepted; nonzero and both near-misses rejected")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archives", nargs="*", type=Path)
    parser.add_argument("--expected-version")
    parser.add_argument("--plan", type=Path)
    parser.add_argument("--target")
    parser.add_argument("--artifact-dir", type=Path)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    try:
        if args.self_test:
            self_test()
        elif args.plan:
            if not args.target or not args.artifact_dir:
                parser.error("--plan requires --target and --artifact-dir")
            check_plan(args.plan, args.target, args.artifact_dir)
        else:
            if not args.expected_version or not args.archives:
                parser.error("provide archives and --expected-version, or --plan, or --self-test")
            for archive in args.archives:
                check_archive(archive, args.expected_version)
    except (CheckError, OSError, ValueError, KeyError, tarfile.TarError, zipfile.BadZipFile) as error:
        print(f"Archive version check failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
