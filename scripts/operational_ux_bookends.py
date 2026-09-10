"""Bookends public full/shallow continuity and durable bypass cases."""
import json
import os
import subprocess
from pathlib import Path

from test_contract import ROOT


def bookends_case(args):
    root = args.attempt
    binary = args.binary_dir / "bookends-check"
    sequence = 0

    def call(argv, cwd=root, status=0, marker=None, env=None):
        nonlocal sequence
        sequence += 1
        prefix = root / f"command-{sequence:03d}"
        result = subprocess.run([str(x) for x in argv], cwd=cwd, capture_output=True, env=env)
        prefix.with_suffix(".stdout").write_bytes(result.stdout)
        prefix.with_suffix(".stderr").write_bytes(result.stderr)
        prefix.with_suffix(".json").write_text(json.dumps({"argv": [str(x) for x in argv], "cwd": str(cwd), "exit_code": result.returncode}))
        assert result.returncode == status, (argv, result.stdout, result.stderr)
        if marker:
            assert result.stdout.decode().splitlines()[0] == marker, result.stdout
        return result

    def write(repo, path, text):
        target = repo / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(text)

    def commit(repo):
        call(["git", "add", "-A"], repo)
        call(["git", "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "-c", "commit.gpgsign=false", "commit", "-m", "fixture"], repo)

    def prd(title="A", tombstone=False):
        return f"### LE-1: {title}\n- Status: {'tombstone' if tombstone else 'live'}\n" + ("" if tombstone else "- Coverage: e2e/journey\n")

    def setup(name):
        repo = root / name
        repo.mkdir()
        call(["git", "init", "-b", "main"], repo)
        write(repo, "bookends.toml", 'prd = "PRD.md"\n[classes.e2e_journey]\npathspecs = ["tests/**"]\nrequired_ci_jobs = ["journey"]\n')
        write(repo, ".github/workflows/ci.yml", "name: ci\non: push\njobs:\n  journey:\n    runs-on: ubuntu-latest\n    steps:\n      - run: python3 tests/journey.py\n")
        write(repo, "tests/journey.py", "# bookends:LE-1\nprint('ok')\n")
        return repo

    outcomes = []
    for name in ("disappearance", "tombstone-removal", "reassignment", "revival"):
        repo = setup(name)
        write(repo, "PRD.md", prd(tombstone=name in ("tombstone-removal", "revival")))
        commit(repo)
        current = prd("B") if name == "reassignment" else prd()
        if name in ("disappearance", "tombstone-removal"):
            current = prd().replace("LE-1", "LE-2")
            write(repo, "tests/journey.py", "# bookends:LE-2\nprint('ok')\n")
        write(repo, "PRD.md", current)
        commit(repo)
        call([binary, "--repo", repo], marker="RED", status=1)
        shallow = root / (name + "-shallow")
        call(["git", "clone", "--depth=1", repo.as_uri(), shallow])
        missing = call([binary, "--repo", shallow], marker="RED", status=1)
        assert b"required parent history unavailable" in missing.stdout
        call(["git", "fetch", "--deepen=1"], shallow)
        resolved = call([binary, "--repo", shallow], marker="RED", status=1)
        assert b"required parent history unavailable" not in resolved.stdout
        outcomes.append(name + ": full RED, depth-1 unavailable RED, depth-2 continuity RED")

    repo = setup("adoption")
    commit(repo)  # resolved parent has no PRD
    write(repo, "PRD.md", prd())
    commit(repo)
    call([binary, "--repo", repo], marker="GREEN")
    shallow = root / "adoption-shallow"
    call(["git", "clone", "--depth=2", repo.as_uri(), shallow])
    call([binary, "--repo", shallow], marker="GREEN")
    verified_root = setup("verified-root")
    write(verified_root, "PRD.md", prd())
    commit(verified_root)
    call([binary, "--repo", verified_root], marker="GREEN")
    root_clone = root / "root-shallow"
    call(["git", "clone", "--depth=1", verified_root.as_uri(), root_clone])
    call([binary, "--repo", root_clone], marker="GREEN")
    # Only the immediate baseline is authoritative, not the whole push range.
    write(repo, "PRD.md", prd("new identity"))
    commit(repo)
    call([binary, "--repo", repo], marker="RED", status=1)
    write(repo, "unrelated", "next commit")
    commit(repo)
    call([binary, "--repo", repo], marker="GREEN")

    # A present tree entry whose blob is unavailable is not adoption.
    damaged = setup("missing-blob")
    write(damaged, "PRD.md", prd())
    commit(damaged)
    blob = call(["git", "rev-parse", "HEAD:PRD.md"], damaged).stdout.decode().strip()
    write(damaged, "PRD.md", prd() + "\ncurrent text\n")
    commit(damaged)
    (damaged / ".git" / "objects" / blob[:2] / blob[2:]).unlink()
    call([binary, "--repo", damaged], marker="RED", status=1)
    outcomes.append("resolved tree with unavailable parent PRD blob RED")

    red = root / "disappearance"
    receipts = root / "receipts"
    call([binary, "--repo", red, "--bypass", "fixture:explicit reason", "--receipt-root", receipts], marker="BYPASS")
    files = list(receipts.glob("*.yaml"))
    assert len(files) == 1
    original = files[0].read_bytes()
    for field in (b"invoked_at_utc_unix_seconds:", b"repository:", b"revision:", b"class: fixture", b"reason: explicit reason", b"outcome: BYPASS"):
        assert field in original, original
    assert not files[0].stat().st_mode & 0o222
    call([binary, "--repo", red, "--bypass", "fixture:second", "--receipt-root", receipts], marker="BYPASS")
    assert len(list(receipts.glob("*.yaml"))) == 2 and files[0].read_bytes() == original
    blocked = root / "not-a-directory"
    blocked.write_text("blocked")
    call([binary, "--repo", red, "--bypass", "fixture:denied", "--receipt-root", blocked], marker="RED", status=1)
    env = dict(os.environ, PATH=str(args.binary_dir) + os.pathsep + os.environ["PATH"], BOOKENDS_BYPASS="fixture:wrapper", XDG_STATE_HOME=str(root / "wrapper-state"))
    call([ROOT / "scripts/bookends-check-gate.sh"], red, marker="BYPASS", env=env)
    assert list((root / "wrapper-state").rglob("*.yaml"))
    env["XDG_STATE_HOME"] = str(blocked)
    call([ROOT / "scripts/bookends-check-gate.sh"], red, status=1, marker="RED", env=env)
    outcomes.extend(["resolved absent PRD and verified root GREEN in full/shallow", "immediate baseline only GREEN after later unchanged commit", "write-once durable BYPASS distinct from GREEN", "CLI and wrapper retention failure RED"])
    (root / "scenarios.json").write_text(json.dumps(outcomes, indent=2))
