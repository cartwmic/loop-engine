#!/usr/bin/env python3
"""Run the shipped packaged journeys outside the checkout.

Identity JSON: {"version":"...", "platform":"...", "sha256":{APP:HEX}}.
Hashes identify archives in archive mode and executables in installed mode.
The identity is caller-supplied package provenance, not a signature or a policy
--version claim. Obtain it from the package build/download, not an assumed version.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import sys
import tarfile
import tempfile

ROOT = Path(__file__).resolve().parents[1]
APPS = {"loop-cli": "loop-engine", "software-change-provider": "software-change",
        "policy-document-provider": "policy-document", "research-provider": "research"}


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def pairs(values):
    result = {}
    for value in values:
        key, value = value.split("=", 1)
        if key not in APPS or key in result:
            raise ValueError(f"unknown or duplicate application: {key}")
        result[key] = value
    if set(result) != set(APPS):
        raise ValueError("all four distributed applications are required")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mode", required=True, choices=("archive", "installed"))
    parser.add_argument("--expected-version", required=True)
    parser.add_argument("--platform", required=True)
    parser.add_argument("--output-root", type=Path, required=True)
    parser.add_argument("--package-identity", type=Path, required=True)
    parser.add_argument("--archive", action="append", default=[], metavar="APP=PATH")
    parser.add_argument("--checksum", action="append", default=[], metavar="APP=SHA256")
    parser.add_argument("--binary", action="append", default=[], metavar="APP=PATH")
    args = parser.parse_args()
    output = args.output_root.resolve()
    if not args.output_root.is_absolute() or output == ROOT or ROOT in output.parents:
        parser.error("output-root must be absolute and outside checkout")
    output.mkdir(parents=True, exist_ok=True)
    attempt = Path(tempfile.mkdtemp(prefix="packaged-", dir=output))
    cwd = attempt / "cwd"
    cwd.mkdir()
    commands = []

    def call(argv):
        index = len(commands)
        out, err = attempt / f"{index:02}.stdout", attempt / f"{index:02}.stderr"
        with out.open("w") as stdout, err.open("w") as stderr:
            process = subprocess.run([str(x) for x in argv], cwd=cwd, stdout=stdout, stderr=stderr)
        commands.append({"argv": [str(x) for x in argv], "cwd": str(cwd),
                         "exit_code": process.returncode, "stdout": str(out), "stderr": str(err)})
        (attempt / "commands.json").write_text(json.dumps(commands, indent=2))
        if process.returncode:
            raise ValueError(f"command failed ({process.returncode}); see {err}")
        return out.read_text()

    try:
        native = {("Darwin", "arm64"): "aarch64-apple-darwin",
                  ("Linux", "x86_64"): "x86_64-unknown-linux-gnu"}.get((platform.system(), platform.machine()))
        if args.platform != native:
            raise ValueError(f"platform mismatch: requested {args.platform}, native {native}")
        identity = json.loads(args.package_identity.read_text())
        if identity["version"] != args.expected_version or identity["platform"] != args.platform:
            raise ValueError("package identity version/platform mismatch")
        (attempt / "package-identity.json").write_text(json.dumps(identity, indent=2))
        inputs = pairs(args.archive if args.mode == "archive" else args.binary)
        if (args.mode == "archive" and args.binary) or (args.mode == "installed" and (args.archive or args.checksum)):
            raise ValueError("mixed archive/installed inputs")
        checksums = pairs(args.checksum) if args.mode == "archive" else identity["sha256"]
        binaries = {}
        for app, name in APPS.items():
            path = Path(inputs[app])
            if not path.is_absolute() or not path.is_file():
                raise ValueError(f"requires absolute existing package input: {path}")
            actual = digest(path)
            if actual != checksums[app] or actual != identity["sha256"][app]:
                raise ValueError(f"checksum/provenance mismatch: {app}")
            if args.mode == "archive":
                extract = attempt / app
                extract.mkdir()
                with tarfile.open(path) as archive:
                    archive.extractall(extract, filter="data")
                matches = [p for p in extract.rglob(name) if p.is_file() and os.access(p, os.X_OK)]
                if len(matches) != 1:
                    raise ValueError(f"expected one executable {name}, got {matches}")
                path = matches[0]
            if not os.access(path, os.X_OK):
                raise ValueError(f"not executable: {path}")
            binaries[app] = path
            if app != "policy-document-provider":
                version = call([path, "--version"]).strip().split()
                expected = [args.expected_version] if app == "loop-cli" else [name, args.expected_version]
                if version != expected:
                    raise ValueError(f"wrong version for {name}: {version}")
        engine = binaries["loop-cli"]
        call([sys.executable, ROOT / "scripts/software-change-journey.py", "--mode", "packaged",
              "--engine", engine, "--provider", binaries["software-change-provider"],
              "--data-root", attempt / "software-data", "--work-root", attempt / "software-work",
              "--profile", "high-rigor.json", "--traversal-depth", "checked-prefix"])
        policy_data = attempt / "policy-data"
        call([binaries["policy-document-provider"], "data-dump", policy_data])
        for mode in ("draft", "audit"):
            call([sys.executable, ROOT / "scripts/policy-document-journey.py", "--engine", engine,
                  "--provider", binaries["policy-document-provider"], "--profile",
                  policy_data / "crates/policy-document-provider/data/readme.json", "--mode", mode])
        research_data = attempt / "research-data"
        research_data.mkdir()
        call([sys.executable, ROOT / "scripts/research-journey.py", "--mode", "packaged",
              "--engine", engine, "--provider", binaries["research-provider"],
              "--data-root", research_data, "--profile", "standard.json"])
        outcome = {"status": "passed"}
    except (OSError, ValueError, KeyError, TypeError, tarfile.TarError) as error:
        outcome = {"status": "failed", "diagnostic": str(error)}
    outcome.update(mode=args.mode, artifact_root=str(attempt))
    (attempt / "outcome.json").write_text(json.dumps(outcome, indent=2))
    print(json.dumps(outcome))
    return 0 if outcome["status"] == "passed" else 1


if __name__ == "__main__":
    sys.exit(main())
