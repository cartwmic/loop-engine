"""Real local cargo-dist packages through the shared smoke boundary; no publication."""
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import tarfile

from test_contract import ContractError, ROOT

APPS = {"loop-cli": "loop-engine", "software-change-provider": "software-change",
        "policy-document-provider": "policy-document", "research-provider": "research"}


def packaged_case(args):
    root = args.attempt
    receipts = []

    def run(name, argv, expected=0, cwd=root, env=None):
        out, err = root / f"{name}.stdout", root / f"{name}.stderr"
        with out.open("w") as stdout, err.open("w") as stderr:
            result = subprocess.run(list(map(str, argv)), cwd=cwd, env=env, stdout=stdout, stderr=stderr)
        receipts.append(dict(argv=list(map(str, argv)), cwd=str(cwd), exit_code=result.returncode,
                             stdout=str(out), stderr=str(err)))
        (root / "commands.json").write_text(json.dumps(receipts, indent=2))
        if (expected == 0 and result.returncode != 0) or (expected != 0 and result.returncode == 0):
            raise ContractError(f"{name}: unexpected exit {result.returncode}; see {err}")
        return out

    target = {("Darwin", "arm64"): "aarch64-apple-darwin",
              ("Linux", "x86_64"): "x86_64-unknown-linux-gnu"}.get((platform.system(), platform.machine()))
    if not target:
        raise ContractError("unsupported native package fixture platform")
    manifest = run("dist-build", ["dist", "build", "--artifacts", "local", "--target", target,
                                  "--output-format=json"], cwd=ROOT)
    plan = json.loads(manifest.read_text())
    versions = {r["app_version"] for r in plan["releases"]}
    assert len(versions) == 1
    version = versions.pop()
    archives, binaries, hashes, binary_hashes = [], [], {}, {}
    installed = root / "installed"
    installed.mkdir()
    for app, binary in APPS.items():
        source = ROOT / "target/distrib" / f"{app}-{target}.tar.xz"
        archive = root / source.name
        shutil.copy2(source, archive)
        hashes[app] = hashlib.sha256(archive.read_bytes()).hexdigest()
        # Compare the actual cargo-dist checksum, not just a freshly computed claim.
        assert hashes[app] == source.with_name(source.name + ".sha256").read_text().split()[0]
        archives += ["--archive", f"{app}={archive}", "--checksum", f"{app}={hashes[app]}"]
        with tarfile.open(archive) as package:
            package.extractall(installed, filter="data")
        matches = list(installed.rglob(binary))
        assert len(matches) == 1
        binary_hashes[app] = hashlib.sha256(matches[0].read_bytes()).hexdigest()
        binaries += ["--binary", f"{app}={matches[0]}"]
    archive_identity, installed_identity = root / "archive-identity.json", root / "installed-identity.json"
    for path, values in [(archive_identity, hashes), (installed_identity, binary_hashes)]:
        path.write_text(json.dumps(dict(version=version, platform=target, sha256=values), indent=2))
    base = [sys.executable, ROOT / "scripts/packaged-smoke.py", "--expected-version", version,
            "--platform", target, "--output-root", root / "smokes"]
    outcomes = []
    for mode, inputs, identity in [("archive", archives, archive_identity), ("installed", binaries, installed_identity)]:
        command = base + ["--mode", mode, "--package-identity", identity] + inputs
        run(mode, command)
        outcomes.append(f"{mode}: real packaged journeys passed outside checkout")
        if mode == "archive":
            wrong = command.copy()
            wrong[wrong.index("--checksum") + 1] = "loop-cli=" + "0" * 64
            run("archive-wrong-checksum", wrong, 1)
        wrong = command.copy()
        wrong[wrong.index("--expected-version") + 1] = "0.0.0-wrong"
        run(mode + "-wrong-version", wrong, 1)
        # Match the declaration too: executable --version must independently refuse.
        false_identity = root / f"{mode}-false-version.json"
        value = json.loads(identity.read_text()); value["version"] = "0.0.0-wrong"
        false_identity.write_text(json.dumps(value))
        wrong[wrong.index("--package-identity") + 1] = false_identity
        run(mode + "-false-version", wrong, 1)
        wrong = command.copy(); wrong[wrong.index("--platform") + 1] = "wrong-platform"
        run(mode + "-wrong-platform", wrong, 1)
        wrong = command.copy(); at = wrong.index("--package-identity"); del wrong[at:at+2]
        run(mode + "-missing-provenance", wrong, 1)
        wrong = command.copy(); del wrong[-2:]
        run(mode + "-missing-input", wrong, 1)
        bad_identity = root / f"{mode}-bad-hash.json"
        value = json.loads(identity.read_text()); value["sha256"]["policy-document-provider"] = "0" * 64
        bad_identity.write_text(json.dumps(value))
        wrong = command.copy(); wrong[wrong.index("--package-identity") + 1] = bad_identity
        run(mode + "-bad-hash", wrong, 1)
        # Real binaries pass identity then an existing journey fails with its
        # external tools unavailable. Do not replace a journey with a mock.
        empty_path = root / "empty-path"; empty_path.mkdir(exist_ok=True)
        failed = run(mode + "-failed-journey", command, 1, env={**os.environ, "PATH": str(empty_path)})
        outcome = json.loads(failed.read_text())
        captured = json.loads((Path(outcome["artifact_root"]) / "commands.json").read_text())
        assert captured[-1]["exit_code"] != 0 and captured[-1]["argv"][1].endswith("-journey.py"), captured[-1]
        outcomes.append(f"{mode}: version/platform/provenance/hash/input and real journey failures refused")
    run("missing-required", [sys.executable, ROOT / "scripts/packaged-smoke.py"], 1)
    (root / "scenarios.json").write_text(json.dumps(outcomes, indent=2))
