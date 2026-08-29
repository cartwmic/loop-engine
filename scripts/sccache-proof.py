#!/usr/bin/env python3
"""Run the repository's small, credential-free sccache proof.

The proof deliberately uses a fresh local disk cache and two different Cargo
--target-dir paths.  It never runs ``cargo clean`` and never exports
RUSTC_WRAPPER for the caller; the wrapper is present only in the environment of
the two Cargo subprocesses.

Examples:

    python3 scripts/sccache-proof.py --self-test
    python3 scripts/sccache-proof.py start --cache-dir /tmp/loop-engine-sccache
    python3 scripts/sccache-proof.py stats --cache-dir /tmp/loop-engine-sccache
    python3 scripts/sccache-proof.py proof --artifact /tmp/testing-sccache.json
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, NoReturn

ROOT = Path(__file__).resolve().parents[1]
PINNED_SCCACHE_VERSION = "0.17.0"
GHA_CACHE_NAMESPACE = "loop-engine-preflight-v1"
EXPECTED_VERSION = f"sccache {PINNED_SCCACHE_VERSION}"
_VERSION_RE = re.compile(r"\bsccache\s+(\d+\.\d+\.\d+)\b")


class ProofError(RuntimeError):
    """A fail-closed proof or command error."""


def fail(message: str) -> NoReturn:
    raise SystemExit(f"sccache proof failed: {message}")


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat()


def resolved(path: Path) -> Path:
    return path.expanduser().resolve()


def forbid_repository_child(path: Path, repo: Path, label: str) -> Path:
    value = resolved(path)
    try:
        value.relative_to(resolved(repo))
    except ValueError:
        return value
    raise ProofError(f"{label} must remain outside the repository: {value}")


def executable(path_arg: str | None) -> Path:
    candidate = Path(path_arg).expanduser() if path_arg else None
    if candidate is None:
        located = shutil.which("sccache")
        if located is None:
            raise ProofError("sccache was not found on PATH")
        candidate = Path(located)
    candidate = resolved(candidate)
    if not candidate.is_file() or not os.access(candidate, os.X_OK):
        raise ProofError(f"sccache is not an executable file: {candidate}")
    return candidate


def command_text(argv: list[str]) -> str:
    return shlex.join(str(item) for item in argv)


def run_command(
    argv: list[str],
    environment: dict[str, str],
    cwd: Path,
    log_dir: Path,
    label: str,
    timeout_seconds: float = 1800.0,
) -> dict[str, Any]:
    log_dir.mkdir(parents=True, exist_ok=True)
    started = time.monotonic()
    timed_out = False
    try:
        completed = subprocess.run(
            [str(item) for item in argv],
            cwd=str(cwd),
            env=environment,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            encoding="utf-8",
            errors="replace",
            timeout=timeout_seconds,
            check=False,
        )
        exit_status: int | None = completed.returncode
        stdout = completed.stdout
        stderr = completed.stderr
    except subprocess.TimeoutExpired as error:
        timed_out = True
        exit_status = None
        stdout = error.stdout if isinstance(error.stdout, str) else ""
        stderr = error.stderr if isinstance(error.stderr, str) else ""
    elapsed = time.monotonic() - started

    stdout_path = log_dir / f"{label}.stdout"
    stderr_path = log_dir / f"{label}.stderr"
    stdout_path.write_text(stdout, encoding="utf-8")
    stderr_path.write_text(stderr, encoding="utf-8")
    return {
        "command": command_text(argv),
        "argv": [str(item) for item in argv],
        "cwd": str(cwd),
        "exit_status": exit_status,
        "timed_out": timed_out,
        "wall_seconds": round(elapsed, 6),
        "stdout_path": str(stdout_path),
        "stderr_path": str(stderr_path),
        "stdout": stdout,
        "stderr": stderr,
    }


def require_success(result: dict[str, Any], label: str) -> dict[str, Any]:
    if result["exit_status"] != 0:
        detail = result["stderr"].strip() or result["stdout"].strip()
        raise ProofError(f"{label} exited {result['exit_status']}: {detail[-1000:]}")
    return result


def require_stopped_or_success(result: dict[str, Any], label: str) -> dict[str, Any]:
    # sccache returns 2 when --stop-server finds no listener.  That is the
    # expected result for the fresh proof; any other nonzero result is real.
    if result["exit_status"] not in (0, 2):
        detail = result["stderr"].strip() or result["stdout"].strip()
        raise ProofError(f"{label} exited {result['exit_status']}: {detail[-1000:]}")
    result["stopped_or_success"] = True
    result["server_was_running"] = result["exit_status"] == 0
    return result


def local_environment(cache_dir: Path, config_path: Path) -> dict[str, str]:
    """Return an environment that selects only sccache's disk backend."""

    environment = os.environ.copy()
    # Do not let a caller's remote configuration, GHA token, or wrapper alter
    # the local proof.  The explicit config below selects [cache.disk].
    for key in list(environment):
        if key.startswith("SCCACHE_") or key.startswith("ACTIONS_"):
            environment.pop(key, None)
    environment.pop("RUSTC_WRAPPER", None)
    environment.pop("CARGO_TARGET_DIR", None)
    cache_dir.mkdir(parents=True, exist_ok=True)
    config_path.parent.mkdir(parents=True, exist_ok=True)
    config_path.write_text(
        "[cache.disk]\n"
        f"dir = {json.dumps(str(cache_dir))}\n"
        "size = 1073741824\n",
        encoding="utf-8",
    )
    environment.update(
        {
            "SCCACHE_CONF": str(config_path),
            "SCCACHE_DIR": str(cache_dir),
            "SCCACHE_CACHE_SIZE": "1G",
            "SCCACHE_LOCAL_RW_MODE": "READ_WRITE",
            "SCCACHE_IDLE_TIMEOUT": "0",
        }
    )
    return environment


def version_result(sccache: Path, environment: dict[str, str], log_dir: Path) -> dict[str, Any]:
    result = run_command([sccache, "--version"], environment, ROOT, log_dir, "version")
    output = result["stdout"].strip()
    match = _VERSION_RE.search(output)
    result["reported_version"] = match.group(1) if match else None
    result["expected_version"] = PINNED_SCCACHE_VERSION
    result["exact_version"] = result["exit_status"] == 0 and output == EXPECTED_VERSION
    if not result["exact_version"]:
        raise ProofError(
            f"expected {EXPECTED_VERSION!r}, got {output!r}"
        )
    return result


def counter_total(value: Any) -> int | None:
    if isinstance(value, bool):
        return None
    if isinstance(value, int):
        return value
    if not isinstance(value, dict):
        return None
    counts = value.get("counts")
    if isinstance(counts, dict):
        numbers = [item for item in counts.values() if isinstance(item, int) and not isinstance(item, bool)]
        if len(numbers) == len(counts):
            return sum(numbers)
    total = value.get("total")
    if isinstance(total, int) and not isinstance(total, bool):
        return total
    return None


def parse_stats(raw: str) -> dict[str, Any]:
    try:
        document = json.loads(raw)
    except json.JSONDecodeError as error:
        raise ProofError(f"sccache statistics are not JSON: {error}") from error
    if not isinstance(document, dict):
        raise ProofError("sccache statistics must be a JSON object")
    stats = document.get("stats", document)
    if not isinstance(stats, dict):
        raise ProofError("sccache statistics object is missing `stats`")

    aliases = {
        "cache_hits": ("cache_hits", "cache_read_hits", "hits"),
        "cache_misses": ("cache_misses", "cache_read_misses", "misses"),
        "cache_errors": ("cache_errors", "errors"),
    }
    totals: dict[str, int] = {}
    for name, candidates in aliases.items():
        total: int | None = None
        for candidate in candidates:
            if candidate in stats:
                total = counter_total(stats[candidate])
                if total is not None:
                    break
        if total is None:
            raise ProofError(f"sccache statistics do not expose {name}")
        totals[name] = total
    return {
        "raw": document,
        **totals,
        "cache_attempts": totals["cache_hits"] + totals["cache_misses"] + totals["cache_errors"],
    }


def stats_result(
    sccache: Path,
    environment: dict[str, str],
    log_dir: Path,
    label: str,
) -> tuple[dict[str, Any], dict[str, Any]]:
    result = run_command(
        [sccache, "--show-stats", "--stats-format=json"],
        environment,
        ROOT,
        log_dir,
        f"{label}-json",
    )
    require_success(result, f"{label} statistics")
    parsed = parse_stats(result["stdout"])
    result["parsed"] = {key: value for key, value in parsed.items() if key != "raw"}
    return result, parsed


def human_stats_result(
    sccache: Path,
    environment: dict[str, str],
    log_dir: Path,
    label: str,
) -> dict[str, Any]:
    return require_success(
        run_command([sccache, "--show-stats"], environment, ROOT, log_dir, label),
        f"{label} statistics",
    )


def check_positive_hit(first: dict[str, Any], second: dict[str, Any]) -> dict[str, Any]:
    first_hits = first["cache_hits"]
    second_hits = second["cache_hits"]
    delta = second_hits - first_hits
    assertion = {
        "criterion": "second compile cache hit delta > 0",
        "first_compile_cumulative_cache_hits": first_hits,
        "second_compile_cumulative_cache_hits": second_hits,
        "second_compile_cache_hit_delta": delta,
        "passed": delta > 0,
    }
    if not assertion["passed"]:
        raise ProofError(f"second compile produced no positive cache-hit delta: {assertion}")
    return assertion


def base_artifact(repo: Path, sccache: Path) -> dict[str, Any]:
    return {
        "schema_version": 1,
        "artifact_kind": "sccache-proof",
        "captured_at_utc": utc_now(),
        "repository": {
            "root": str(repo),
            "git_revision": git_revision(repo),
            "repository_state": git_state(repo),
        },
        "tool": {
            "name": "sccache",
            "path": str(sccache),
            "pinned_version": PINNED_SCCACHE_VERSION,
            "backend_documentation": [
                f"https://github.com/mozilla/sccache/blob/v{PINNED_SCCACHE_VERSION}/docs/GHA.md",
                f"https://github.com/mozilla/sccache/blob/v{PINNED_SCCACHE_VERSION}/docs/Local.md",
                f"https://github.com/mozilla/sccache/blob/v{PINNED_SCCACHE_VERSION}/docs/Configuration.md",
            ],
        },
        "local_backend": {
            "backend": "disk",
            "credential_free": True,
            "configuration": "SCCACHE_CONF with [cache.disk], SCCACHE_DIR, and READ_WRITE local mode",
            "gha_backend_disabled": True,
            "rustc_wrapper_scope": "only the two proof Cargo subprocesses",
        },
        "github_hosted_backend": {
            "backend": "gha",
            "credential_free": True,
            "configuration": {
                "SCCACHE_GHA_ENABLED": "true",
                "SCCACHE_GHA_VERSION": GHA_CACHE_NAMESPACE,
                "SCCACHE_GHA_RW_MODE": "READ_WRITE",
                "SCCACHE_IDLE_TIMEOUT": "0",
                "RUSTC_WRAPPER": "sccache",
                "runtime_credentials": "ephemeral GitHub Actions runtime variables exported by actions/github-script; no repository secret",
            },
            "setup_action": "mozilla-actions/sccache-action@v0.0.11",
            "setup_action_version": "v0.0.11",
            "runtime_environment_export": {
                "action": "actions/github-script@v7",
                "script": [
                    "core.exportVariable('ACTIONS_RESULTS_URL', process.env.ACTIONS_RESULTS_URL || '');",
                    "core.exportVariable('ACTIONS_RUNTIME_TOKEN', process.env.ACTIONS_RUNTIME_TOKEN || '');",
                ],
                "purpose": "export the ephemeral GitHub Actions cache service variables required by sccache 0.17.0",
            },
            "sccache_version_input": f"v{PINNED_SCCACHE_VERSION}",
        },
        "hosted_runtime_limitation": (
            "This local macOS session cannot execute the GitHub-hosted required preflight against "
            "the uncommitted reviewed bytes. Doing so before the required terminal-then-one-commit "
            "lifecycle would require a second, unreviewed commit; this artifact claims no hosted run."
        ),
        "post_push_obligation": (
            "After the owner-approved single commit, run the required push-to-main preflight for "
            "that exact commit, verify the checkout SHA equals git rev-parse HEAD, inspect the "
            "hosted sccache startup and final statistics and total wall time, and require success "
            "before Package 7b starts."
        ),
    }


def git_revision(repo: Path) -> str | None:
    result = subprocess.run(
        ["git", "-C", str(repo), "rev-parse", "HEAD"],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        check=False,
    )
    return result.stdout.strip() if result.returncode == 0 else None


def git_state(repo: Path) -> str | None:
    result = subprocess.run(
        ["git", "-C", str(repo), "status", "--porcelain=v1", "--untracked-files=all"],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        check=False,
    )
    return result.stdout if result.returncode == 0 else None


def static_workflow_proof(log_dir: Path) -> dict[str, Any]:
    command = [sys.executable, str(ROOT / "scripts/assert-push-main-preflight.py")]
    environment = os.environ.copy()
    environment.pop("RUSTC_WRAPPER", None)
    result = run_command(command, environment, ROOT, log_dir, "workflow-static")
    require_success(result, "workflow static assertion")
    release_workflow = ROOT / ".github/workflows/release.yml"
    release_diff = subprocess.run(
        ["git", "-C", str(ROOT), "diff", "HEAD", "--quiet", "--", ".github/workflows/release.yml"],
        check=False,
    )
    if release_diff.returncode != 0:
        raise ProofError("generated .github/workflows/release.yml has an uncommitted diff")
    return {
        "command": result["command"],
        "exit_status": result["exit_status"],
        "stdout": result["stdout"],
        "stderr": result["stderr"],
        "passed": True,
        "workflow_path": str(ROOT / ".github/workflows/preflight.yml"),
        "assertion_path": str(ROOT / "scripts/assert-push-main-preflight.py"),
        "generated_release_workflow": str(release_workflow),
        "generated_release_workflow_check": "git diff HEAD --quiet -- .github/workflows/release.yml",
        "generated_release_workflow_edited": False,
    }


def write_artifact(path: Path, artifact: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(artifact, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def run_proof(args: argparse.Namespace) -> int:
    repo = resolved(args.repo)
    artifact_path = forbid_repository_child(Path(args.artifact), repo, "proof artifact")
    sccache = executable(args.sccache)
    work_parent: Path | None = None
    if args.work_root:
        work_parent = forbid_repository_child(Path(args.work_root), repo, "proof work root")
        work_parent.mkdir(parents=True, exist_ok=True)
    proof_dir = Path(tempfile.mkdtemp(prefix="loop-engine-sccache-proof-", dir=str(work_parent) if work_parent else None))
    proof_dir = resolved(proof_dir)
    artifact = base_artifact(repo, sccache)
    initial_repository_state = artifact["repository"]["repository_state"]
    artifact["proof"] = {
        "work_directory": str(proof_dir),
        "temporary_runtime_data": True,
        "cargo_clean_used": False,
        "target_directories": [str(proof_dir / "target-a"), str(proof_dir / "target-b")],
        "target_directories_are_distinct": True,
        "commands": [],
    }
    error: str | None = None
    environment: dict[str, str] | None = None
    server_started = False
    try:
        cache_dir = proof_dir / "cache"
        config_path = proof_dir / "sccache.toml"
        target_a = proof_dir / "target-a"
        target_b = proof_dir / "target-b"
        log_dir = proof_dir / "logs"
        environment = local_environment(cache_dir, config_path)
        artifact["local_backend"]["proof_configuration"] = {
            "SCCACHE_CONF": str(config_path),
            "SCCACHE_DIR": str(cache_dir),
            "SCCACHE_CACHE_SIZE": "1G",
            "SCCACHE_LOCAL_RW_MODE": "READ_WRITE",
            "SCCACHE_IDLE_TIMEOUT": "0",
            "config_file_contents": (
                "[cache.disk]\n"
                f"dir = {json.dumps(str(cache_dir))}\n"
                "size = 1073741824\n"
            ),
            "credentials": [],
        }
        artifact["proof"]["cache_directory"] = str(cache_dir)

        parser_self_test = require_success(
            run_command(
                [sys.executable, str(Path(__file__).resolve()), "--self-test"],
                environment,
                ROOT,
                log_dir,
                "parser-self-test",
            ),
            "sccache parser self-test",
        )
        artifact["parser_assertion_self_test"] = {
            "command": parser_self_test["command"],
            "exit_status": parser_self_test["exit_status"],
            "stdout": parser_self_test["stdout"],
            "stderr": parser_self_test["stderr"],
            "passed": True,
        }
        artifact["proof"]["commands"].append(parser_self_test)

        version = version_result(sccache, environment, log_dir)
        artifact["tool"]["version_check"] = version
        stop = require_stopped_or_success(
            run_command([sccache, "--stop-server"], environment, ROOT, log_dir, "stop-before-start"),
            "sccache stop before startup",
        )
        artifact["proof"]["commands"].append(stop)
        start = require_success(
            run_command([sccache, "--start-server"], environment, ROOT, log_dir, "start-server"),
            "sccache startup",
        )
        server_started = True
        artifact["proof"]["commands"].append(start)
        zero = require_success(
            run_command([sccache, "--zero-stats"], environment, ROOT, log_dir, "zero-stats"),
            "sccache zero stats",
        )
        artifact["proof"]["commands"].append(zero)
        startup_stats_result, startup_stats = stats_result(sccache, environment, log_dir, "stats-after-start")
        artifact["proof"]["commands"].append(startup_stats_result)
        artifact["proof"]["startup"] = {
            "start_command": start["command"],
            "start_exit_status": start["exit_status"],
            "stats_command": startup_stats_result["command"],
            "stats_exit_status": startup_stats_result["exit_status"],
            "stats": startup_stats["raw"],
            "passed": True,
        }

        cargo = shutil.which("cargo")
        if cargo is None:
            raise ProofError("cargo was not found on PATH")
        # Keep the Cargo shim's basename.  Resolving ~/.cargo/bin/cargo to
        # the rustup executable would make rustup interpret `check` as its
        # own subcommand instead of dispatching to Cargo.
        cargo = str(Path(cargo))
        compile_environment = environment.copy()
        compile_environment["RUSTC_WRAPPER"] = str(sccache)
        compile_command_base = [cargo, "test", "--workspace", "--locked", "--no-run"]
        for label, target in (("compile-target-a", target_a), ("compile-target-b", target_b)):
            command = [*compile_command_base, "--target-dir", str(target)]
            result = require_success(
                run_command(command, compile_environment, repo, log_dir, label, args.timeout),
                label,
            )
            result["environment"] = {
                "SCCACHE_CONF": str(config_path),
                "SCCACHE_DIR": str(cache_dir),
                "RUSTC_WRAPPER": str(sccache),
            }
            result["target_directory"] = str(target)
            result["equivalent_workspace_input"] = True
            artifact["proof"]["commands"].append(result)

            snapshot_result, snapshot = stats_result(sccache, environment, log_dir, f"stats-after-{label}")
            artifact["proof"]["commands"].append(snapshot_result)
            artifact["proof"]["stats_after_" + label.removeprefix("compile-").replace("-", "_")] = snapshot["raw"]
            if label == "compile-target-a":
                first_stats = snapshot
            else:
                second_stats = snapshot

        assertion = check_positive_hit(first_stats, second_stats)
        artifact["proof"]["positive_hit_assertion"] = assertion
        artifact["proof"]["cache_behavior"] = {
            "first_compile": "fresh local disk cache population",
            "first_compile_cache_hits": first_stats["cache_hits"],
            "first_compile_cache_misses": first_stats["cache_misses"],
            "second_compile_cache_hit_delta": assertion["second_compile_cache_hit_delta"],
            "eviction_or_cold_cache_note": (
                "The first compile is expected to be cold for this fresh local cache. "
                "GitHub Actions entries may be cold or evicted under normal backend retention; "
                "no universal speedup or permanent retention is claimed."
            ),
        }
        artifact["proof"]["passed"] = True
        human = human_stats_result(sccache, environment, log_dir, "final-stats-human")
        artifact["proof"]["final_human_statistics"] = human["stdout"]
        artifact["proof"]["commands"].append(human)
        final_json_result, final_json = stats_result(sccache, environment, log_dir, "final-stats-json")
        artifact["proof"]["final_statistics"] = final_json["raw"]
        artifact["proof"]["commands"].append(final_json_result)
        artifact["workflow_static_proof"] = static_workflow_proof(log_dir)
    except (OSError, ProofError, subprocess.SubprocessError) as caught:
        error = str(caught)
        artifact["proof"]["passed"] = False
        artifact["error"] = error
    finally:
        if server_started and environment is not None:
            stop_after = run_command([sccache, "--stop-server"], environment, ROOT, proof_dir / "logs", "stop-after-proof")
            artifact["proof"]["commands"].append(stop_after)
        artifact["cleanup"] = {
            "target_and_cache_paths_are_temporary": True,
            "removed_work_directory": str(proof_dir),
            "repository_cache_or_target_created": False,
        }
        try:
            shutil.rmtree(proof_dir)
        except OSError as cleanup_error:
            artifact["cleanup"]["error"] = str(cleanup_error)
            if error is None:
                error = f"could not remove temporary proof directory: {cleanup_error}"

        final_repository_state = git_state(repo)
        repository_status_unchanged = (
            initial_repository_state is not None
            and final_repository_state is not None
            and initial_repository_state == final_repository_state
        )
        artifact["repository_safety"] = {
            "command": "git status --porcelain=v1 --untracked-files=all",
            "before": initial_repository_state,
            "after": final_repository_state,
            "status_unchanged": repository_status_unchanged,
            "cache_and_target_paths_outside_repository": True,
            "generated_release_workflow_edited": False,
        }
        if not repository_status_unchanged:
            artifact["proof"]["passed"] = False
            if error is None:
                error = "repository status changed while running the temporary sccache proof"
        write_artifact(artifact_path, artifact)

    if error is not None:
        fail(f"{error}; artifact: {artifact_path}")
    print(json.dumps({"status": "passed", "artifact": str(artifact_path)}, separators=(",", ":")) )
    return 0


def persistent_paths(args: argparse.Namespace, repo: Path) -> tuple[Path, Path]:
    cache_dir = forbid_repository_child(Path(args.cache_dir), repo, "cache directory")
    config = Path(args.config) if args.config else cache_dir.parent / f".{cache_dir.name}-sccache.toml"
    config = forbid_repository_child(config, repo, "sccache config")
    return cache_dir, config


def run_start(args: argparse.Namespace) -> int:
    repo = resolved(args.repo)
    sccache = executable(args.sccache)
    cache_dir, config = persistent_paths(args, repo)
    log_dir = cache_dir.parent / f".{cache_dir.name}-logs"
    environment = local_environment(cache_dir, config)
    version_result(sccache, environment, log_dir)
    require_stopped_or_success(
        run_command([sccache, "--stop-server"], environment, ROOT, log_dir, "stop-before-start"),
        "sccache stop before startup",
    )
    start = require_success(
        run_command([sccache, "--start-server"], environment, ROOT, log_dir, "start-server"),
        "sccache startup",
    )
    require_success(
        run_command([sccache, "--zero-stats"], environment, ROOT, log_dir, "zero-stats"),
        "sccache zero stats",
    )
    print(f"sccache {PINNED_SCCACHE_VERSION} started")
    print(f"local disk cache: {cache_dir}")
    print(f"start command: {start['command']}")
    return 0


def run_stats(args: argparse.Namespace) -> int:
    repo = resolved(args.repo)
    sccache = executable(args.sccache)
    cache_dir, config = persistent_paths(args, repo)
    log_dir = cache_dir.parent / f".{cache_dir.name}-logs"
    environment = local_environment(cache_dir, config)
    version_result(sccache, environment, log_dir)
    human = human_stats_result(sccache, environment, log_dir, "stats-human")
    print(human["stdout"], end="" if human["stdout"].endswith("\n") else "\n")
    json_result, parsed = stats_result(sccache, environment, log_dir, "stats-json")
    print(json.dumps(parsed["raw"], indent=2, sort_keys=True))
    print(f"stats command: {json_result['command']}")
    return 0


def self_test() -> int:
    positive = {
        "stats": {
            "cache_hits": {"counts": {"Rust": 4}},
            "cache_misses": {"counts": {"Rust": 7}},
            "cache_errors": {"counts": {}},
        }
    }
    first = parse_stats(json.dumps({**positive, "stats": {**positive["stats"], "cache_hits": {"counts": {"Rust": 0}}}}))
    second = parse_stats(json.dumps(positive))
    assertion = check_positive_hit(first, second)
    if not assertion["passed"]:
        raise ProofError("self-test did not accept a positive cache hit")
    zero = parse_stats(json.dumps({
        "stats": {
            "cache_hits": {"counts": {"Rust": 0}},
            "cache_misses": {"counts": {"Rust": 1}},
            "cache_errors": {"counts": {}},
        }
    }))
    try:
        check_positive_hit(zero, zero)
    except ProofError:
        pass
    else:
        raise ProofError("self-test accepted a zero-hit proof")
    print("sccache statistics assertion self-test passed: positive hit accepted, zero hit rejected")
    return 0


def add_common(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--sccache", help="exact sccache executable (default: PATH lookup)")
    parser.add_argument("--repo", type=Path, default=ROOT, help=argparse.SUPPRESS)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true", help="test the statistics parser and positive-hit assertion")
    subparsers = parser.add_subparsers(dest="operation")

    for operation in ("start", "stats"):
        child = subparsers.add_parser(operation, help=f"{operation} a local disk-backed sccache server")
        add_common(child)
        child.add_argument("--cache-dir", type=Path, required=True)
        child.add_argument("--config", type=Path)

    proof = subparsers.add_parser("proof", help="compile the workspace twice into isolated target directories")
    add_common(proof)
    proof.add_argument("--artifact", type=Path, required=True)
    proof.add_argument("--work-root", type=Path)
    proof.add_argument("--timeout", type=float, default=1800.0)
    return parser


def main() -> int:
    parser = build_parser()
    args = parser.parse_args()
    if args.self_test:
        return self_test()
    if args.operation == "start":
        return run_start(args)
    if args.operation == "stats":
        return run_stats(args)
    if args.operation == "proof":
        return run_proof(args)
    parser.error("choose start, stats, or proof (or use --self-test)")
    return 2


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (ProofError, OSError) as error:
        fail(str(error))
