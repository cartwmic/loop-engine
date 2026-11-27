#!/usr/bin/env python3
"""Print one result line per check that shows a change kept behavior.

Run it from a repository checkout on the tree before a change and again on the
tree after it, then compare the two outputs. It runs format, lint and build,
the unit tests, the central tests and the six boundary journeys, hashes the
help text of every `loop-engine` command and the SQL schema statements, and
counts non-test source lines and Cargo.lock packages. It exits 1 if any check
fails.
"""
from __future__ import annotations

import hashlib
import io
import json
import os
import re
import shutil
import sqlite3
import subprocess
import sys
import tarfile
import tempfile
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DIST_URL = "https://github.com/axodotdev/cargo-dist/releases/download/v0.32.0/cargo-dist-x86_64-unknown-linux-gnu.tar.xz"
COUNTED = ("crates/loop-core/src", "crates/loop-integrations/src")
SEARCHED = ("crates", "tests", "Cargo.toml", "Cargo.lock")
RESULT = re.compile(r"test result: \w+\. (\d+) passed; (\d+) failed")
failed = False


def run(*cmd: str, env: dict[str, str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(cmd, cwd=ROOT, env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)


def report(name: str, result: subprocess.CompletedProcess[str]) -> None:
    global failed
    if result.returncode:
        failed = True
        print(f"{name}: FAILED")
        print("\n".join(result.stdout.splitlines()[-5:]))
    else:
        print(f"{name}: ok")


def counts(name: str, result: subprocess.CompletedProcess[str]) -> None:
    global failed
    found = RESULT.findall(result.stdout)
    passed = sum(int(p) for p, _ in found)
    bad = sum(int(f) for _, f in found)
    failed |= result.returncode != 0 or not found
    print(f"{name}: passed {passed} failed {bad}")


def non_test_lines(path: Path) -> int:
    lines = path.read_text().splitlines()
    for i, line in enumerate(lines[:-1]):
        if line.strip() == "#[cfg(test)]" and lines[i + 1].strip().startswith("mod tests"):
            return i
    return len(lines)


def matches(pattern: str) -> list[tuple[Path, int]]:
    expression = re.compile(pattern)
    files = [p for name in SEARCHED for p in ([ROOT / name] if (ROOT / name).is_file() else sorted((ROOT / name).rglob("*"))) if p.is_file()]
    return [(p, n) for p in files for n, line in enumerate(p.read_text(errors="replace").splitlines(), 1) if expression.search(line)]


def environment(scratch: Path) -> dict[str, str]:
    env = dict(os.environ)
    if not os.access(env.get("HOME", "/nonexistent"), os.W_OK):
        env["HOME"] = str(scratch / "home")
        Path(env["HOME"]).mkdir()
    env["PROOF_ROOT"] = str(scratch / "proof")
    Path(env["PROOF_ROOT"]).mkdir()
    if not shutil.which("dist", path=env["PATH"]):
        tarfile.open(fileobj=io.BytesIO(urllib.request.urlopen(DIST_URL).read())).extractall(scratch / "dist")
        env["PATH"] = f"{next((scratch / 'dist').rglob('dist')).parent}:{env['PATH']}"
    return env


def main() -> int:
    with tempfile.TemporaryDirectory(prefix="behavior-snapshot-") as tmp:
        env = environment(Path(tmp))
        report("format", run("cargo", "fmt", "--all", "--", "--check", env=env))
        report("lint", run("cargo", "clippy", "--workspace", "--all-targets", "--locked", "--", "-D", "warnings", env=env))
        report("build", run("cargo", "build", "--workspace", "--all-targets", "--locked", env=env))
        counts("unit tests", run("cargo", "test", "--workspace", "--locked", env=env))
        counts("central tests", run("python3", "scripts/run-central-tests.py", env=env))

        meta = json.loads(run("cargo", "metadata", "--format-version", "1", "--no-deps", env=env).stdout)
        bin_dir = Path(meta["target_directory"]) / "debug"
        engine = str(bin_dir / "loop-engine")
        usage = run(engine, "--help", env=env).stdout.split("Commands:")[1].split("\n\n")[0]
        commands = [line.split()[0] for line in usage.splitlines() if line.strip()]
        help_text = "".join(run(engine, *command, "--help", env=env).stdout for command in [(), *((name,) for name in commands)])
        print(f"help text of loop-engine and {len(commands)} commands ({' '.join(commands)}) sha256: {hashlib.sha256(help_text.encode()).hexdigest()}")
        database = Path(tmp) / "schema" / "loop.db"
        database.parent.mkdir()
        created = run(engine, "--database", str(database), "list", env=env)
        schema = sqlite3.connect(database)
        objects = schema.execute("select type, name, sql from sqlite_master order by type, name").fetchall()
        version = schema.execute("pragma user_version").fetchone()[0]
        report("new database", created)
        print(f"schema of a new database: {len(objects)} objects, user_version {version}, sha256: {hashlib.sha256(json.dumps([version, objects]).encode()).hexdigest()}")

        def tool(name: str) -> str:
            return str(bin_dir / name)

        policy = ["--engine", engine, "--provider", tool("policy-document"), "--profile", "crates/policy-document-provider/data/readme.json"]
        research = ["--mode", "source", "--engine", engine, "--provider", tool("research")]
        journeys = {
            "external-work": ["external-work-journey.py", "--engine", engine, "--lean", tool("lean-change"), "--policy", tool("policy-document"), "--research", tool("research"), "--checker", tool("bookends-check"), "--output-root", f"{tmp}/proof/external-work"],
            "lean-change": ["lean-change-journey.py", "--engine", engine, "--provider", tool("lean-change")],
            "policy-draft": ["policy-document-journey.py", *policy, "--mode", "draft"],
            "policy-audit": ["policy-document-journey.py", *policy, "--mode", "audit"],
            "research": ["research-journey.py", *research, "--profile", "crates/research-provider/data/configs/standard.json"],
            "generate-prd": ["generate-prd-journey.py", *research, "--checker", tool("bookends-check"), "--profile", "crates/research-provider/data/configs/generate-prd.json"],
        }
        for name, (script, *args) in journeys.items():
            report(f"journey {name}", run("python3", f"scripts/{script}", *args, env=env))

    sources = sorted(p for d in COUNTED for p in (ROOT / d).rglob("*.rs"))
    print(f"non-test lines in {' and '.join(Path(d).parts[1] for d in COUNTED)}: {sum(non_test_lines(p) for p in sources)}")
    print(f"Cargo.lock packages: {(ROOT / 'Cargo.lock').read_text().count('[[package]]')}")
    found = [(path, number, pattern) for pattern in sys.argv[1:] for path, number in matches(pattern)]
    print(f"patterns found in {', '.join(SEARCHED)}: {len(found)} matches for {len(sys.argv) - 1} patterns")
    for path, number, pattern in found[:20]:
        print(f"  {path.relative_to(ROOT)}:{number}: {pattern}")
    return 1 if failed or found else 0


if __name__ == "__main__":
    sys.exit(main())
