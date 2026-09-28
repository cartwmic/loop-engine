#!/usr/bin/env python3
"""Seed, collect, and compare isolated public-CLI dogfood measurements.

No model calls are made. Every run uses a fresh external SQLite database and
scripted workers; every read is an actual loop-engine/software-change process.
Comparisons use retained sample streams and reject incomplete or mismatched
receipts. This is measurement evidence, not semantic review or approval.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import random
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import Any

SCHEMA_VERSION = 1
DATASET_VERSION = "dogfood-v2"
RECEIPT_SCHEMA_VERSION = 1
RECORD_COUNT = 128
RECORD_PAYLOAD_BYTES = 1024
READ_SAMPLES = 3
REVIEW_SAMPLES = 3
ACTIVE_HOLD_SECONDS = 30
PARALLEL_ACTIVE_RUNS = 4
GRAPH_WORKERS = 14
READ_LANES = ("idle", "serial", "parallel")
READ_SURFACES = ("status", "action", "history", "invocation-progress", "monitor")
REVIEW_SIZES = ("small", "mature")
REVIEW_AUTHOR = "dogfood-scripted-reviewer"
MATURE_HISTORY_KIND = "dogfood-irrelevant-history"
MATURE_HISTORY_BYTES = 65536
BASELINE_CONFIG_VERSION = "minimal-11"
CURRENT_CONFIG_VERSION = "minimal-12"
SCRIPTED_TOKEN_BUDGET = {
    "model_id": "dogfood-scripted-reviewer",
    "context_window_tokens": 200000,
    "system_tokens": 8192,
    "framing_tokens": 32768,
    "output_reserve_tokens": 8192,
    "reasoning_reserve_tokens": 8192,
}
FIXTURE_PATH = Path(
    "crates/software-change-provider/data/calibration/fixtures/intent-good.json"
)
CONFIG_PATH = Path("crates/software-change-provider/data/configs/minimal.json")
MIN_IMPROVED_SURFACES = 2

GENERIC_PROVIDER_SOURCE = r'''#!/usr/bin/env python3
import json, sys
sys.stdin.buffer.read()
workflow = {
    "id": "dogfood-measure-v1",
    "initial_state": "work",
    "states": [
        {"id": "work", "title": "Dogfood work", "instructions": sys.argv[1], "final": False},
        {"id": "done", "title": "Done", "instructions": "Fixture complete", "final": True},
    ],
    "transitions": [{"source": "work", "event": "finish", "target": "done", "kind": "check-free"}],
    "work_slots": [{"id": "scripted-work", "state": "work", "event": "finish", "stdin_context_kinds": ["dogfood-record"]}],
}
print(json.dumps(workflow, separators=(",", ":")))
'''

BOUND_WORKER_SOURCE = r'''#!/usr/bin/env python3
import argparse, json, subprocess, sys, time
from pathlib import Path
p = argparse.ArgumentParser()
p.add_argument("--marker", required=True)
p.add_argument("--sleep", type=float, required=True)
p.add_argument("--children", type=int, default=1)
p.add_argument("--child-sleep", type=float, default=0.0)
a = p.parse_args()
packet = sys.stdin.buffer.read()
Path(a.marker).write_text(json.dumps({"pid": __import__("os").getpid(), "packet_bytes": len(packet)}))
if a.children > 1:
    children = [subprocess.Popen([sys.executable, __file__, "--child", str(a.child_sleep)]) for _ in range(a.children)]
    for child in children:
        if child.wait() != 0:
            raise SystemExit(3)
elif a.sleep:
    time.sleep(a.sleep)
print(json.dumps({"scripted_worker": True, "packet_bytes": len(packet)}))
'''

# The child mode above is handled before argparse only by this wrapper branch;
# keep it explicit so active fixtures never shell-interpolate a command string.
BOUND_WORKER_SOURCE = BOUND_WORKER_SOURCE.replace(
    'p = argparse.ArgumentParser()\n',
    'if len(sys.argv) == 3 and sys.argv[1] == "--child":\n    time.sleep(float(sys.argv[2]))\n    raise SystemExit(0)\np = argparse.ArgumentParser()\n',
)

REVIEW_WORKER_SOURCE = r'''#!/usr/bin/env python3
import hashlib, json, sys, time
from pathlib import Path
raw = sys.stdin.buffer.read()
stdin_received_monotonic_ns = time.perf_counter_ns()
inbox_path, meta_path, subject_path, output_path = map(Path, sys.argv[1:5])
inbox_path.write_bytes(raw)
lines = raw.decode("utf-8", errors="replace").splitlines()
def field(name, required=True):
    prefix = name + ": "
    for line in lines:
        if line.startswith(prefix):
            return line[len(prefix):]
    if required:
        raise SystemExit("missing review assignment field: " + name)
    return None
try:
    policies = json.loads(field("assigned_policies"))
except Exception as error:
    raise SystemExit("invalid assigned_policies: " + str(error))
stage = field("review_stage")
author = field("required_author_claim")
contract = sys.argv[5]
subject_sha256 = "sha256:" + hashlib.sha256(subject_path.read_bytes()).hexdigest()
if not isinstance(policies, list) or not policies:
    raise SystemExit("review packet has no assigned policies")
assignment = {"review_stage": stage, "author": author, "policies": policies}
judgments = [{"axis": policy["id"], "result": "pass", "findings": ""} for policy in policies]
output = {"review_stage": stage, "author": {"name": author, "kind": "agent"}, "judgments": judgments}
if contract == "current-v2":
    output["review_contract_version"] = 2
    for judgment in output["judgments"]:
        judgment["grounds"] = {
            "reason": "Scripted measurement fixture only; no semantic review was performed.",
            "evidence": [{"locator": "intent.json#/problem", "sha256": subject_sha256}],
        }
elif contract != "legacy-v1":
    raise SystemExit("unknown review output contract: " + contract)
encoded_output = json.dumps(output, separators=(",", ":")) + "\n"
output_path.write_text(encoded_output)
budget_line = field("PER-CALL TOKEN WINDOW", required=False)
if budget_line is not None:
    budget_line = "PER-CALL TOKEN WINDOW: " + budget_line
meta_path.write_text(json.dumps({
    "argv": sys.argv, "executable": sys.executable, "stdin_bytes": len(raw),
    "assignment": assignment, "token_budget_line": budget_line,
    "stdin_received_monotonic_ns": stdin_received_monotonic_ns,
    "output_contract": contract, "subject_sha256": subject_sha256,
    "output_path": str(output_path), "output_sha256": "sha256:" + hashlib.sha256(encoded_output.encode()).hexdigest(),
}))
print(encoded_output, end="", flush=True)
'''

GRAPH_WORKER_SOURCE = r'''#!/usr/bin/env python3
import json, sys, time
worker_id, delay = sys.argv[1], float(sys.argv[2])
sys.stdin.buffer.read()
if delay:
    time.sleep(delay)
print(json.dumps({"worker": worker_id, "scripted": True}))
'''


def canonical_bytes(value: Any) -> bytes:
    return (json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False) + "\n").encode()


def sha256(raw: bytes) -> str:
    return "sha256:" + hashlib.sha256(raw).hexdigest()


def file_sha(path: Path) -> str:
    return sha256(path.read_bytes())


def write_json(path: Path, value: Any) -> None:
    path.write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


def require_absolute(path: Path, name: str) -> Path:
    if not path.is_absolute():
        raise ValueError(f"{name} must be an absolute path: {path}")
    return path.resolve()


def ensure_external_output(path: Path, name: str) -> Path:
    path = require_absolute(path, name)
    root = Path(__file__).resolve().parents[1]
    if path == root or root in path.parents:
        raise ValueError(f"{name} must be outside the checkout: {path}")
    if not path.parent.is_dir():
        raise ValueError(f"{name} parent must already exist: {path.parent}")
    if path.exists():
        raise ValueError(f"{name} already exists; refusing to overwrite: {path}")
    return path


def dataset_payload(seed: int, fixture_bytes: bytes) -> dict[str, Any]:
    rng = random.Random(seed)
    alphabet = "abcdefghjkmnpqrstuvwxyz23456789"
    records = []
    for index in range(RECORD_COUNT):
        payload = "".join(rng.choice(alphabet) for _ in range(RECORD_PAYLOAD_BYTES))
        records.append({
            "record_id": f"dogfood-{seed}-{index:04d}",
            "kind": "dogfood-record",
            "data": {"sequence": index, "payload": payload},
        })
    # A fixed, long action is useful to compare large action reads without
    # pretending that every historical record is actionable.
    action_text = (
        "Dogfood fixture duty: inspect the current scripted work and retain the "
        "actual public evidence. This repeated text is inert fixture content. "
    ) * 64
    return {
        "dataset_version": DATASET_VERSION,
        "seed": seed,
        "fixture": {"path": FIXTURE_PATH.as_posix(), "sha256": sha256(fixture_bytes)},
        "record_count": RECORD_COUNT,
        "record_payload_bytes": RECORD_PAYLOAD_BYTES,
        "records": records,
        "large_action_instructions": action_text,
        "samples_per_read_surface": READ_SAMPLES,
        "samples_per_review_size": REVIEW_SAMPLES,
        "active_hold_seconds": ACTIVE_HOLD_SECONDS,
        "parallel_active_runs": PARALLEL_ACTIVE_RUNS,
        "graph_workers": GRAPH_WORKERS,
        "mature_irrelevant_history_kind": MATURE_HISTORY_KIND,
        "mature_irrelevant_history_bytes": MATURE_HISTORY_BYTES,
    }


def validate_dataset_document(document: Any) -> tuple[dict[str, Any], str]:
    if not isinstance(document, dict) or document.get("schema_version") != SCHEMA_VERSION:
        raise ValueError("unsupported or malformed dogfood dataset schema")
    payload = document.get("dataset")
    digest = document.get("dataset_sha256")
    if not isinstance(payload, dict) or not isinstance(digest, str):
        raise ValueError("dataset must contain a payload and content digest")
    calculated = sha256(canonical_bytes(payload))
    if digest != calculated:
        raise ValueError("dataset content digest does not match its bytes")
    if payload.get("dataset_version") != DATASET_VERSION:
        raise ValueError("unsupported dogfood dataset version")
    fixed_workload = {
        "record_count": RECORD_COUNT,
        "record_payload_bytes": RECORD_PAYLOAD_BYTES,
        "samples_per_read_surface": READ_SAMPLES,
        "samples_per_review_size": REVIEW_SAMPLES,
        "active_hold_seconds": ACTIVE_HOLD_SECONDS,
        "parallel_active_runs": PARALLEL_ACTIVE_RUNS,
        "graph_workers": GRAPH_WORKERS,
        "mature_irrelevant_history_kind": MATURE_HISTORY_KIND,
        "mature_irrelevant_history_bytes": MATURE_HISTORY_BYTES,
    }
    if any(payload.get(key) != expected for key, expected in fixed_workload.items()):
        raise ValueError("dataset workload parameters do not match its fixed versioned contract")
    if not isinstance(payload.get("large_action_instructions"), str) or not payload["large_action_instructions"]:
        raise ValueError("dataset is missing the fixed large-action workload")
    if not isinstance(payload.get("seed"), int) or isinstance(payload.get("seed"), bool):
        raise ValueError("dataset seed must be an integer")
    records = payload.get("records")
    if not isinstance(records, list) or len(records) != RECORD_COUNT:
        raise ValueError("dataset record fixture is incomplete")
    ids = [row.get("record_id") for row in records if isinstance(row, dict)]
    if len(ids) != RECORD_COUNT or len(ids) != len(set(ids)):
        raise ValueError("dataset record IDs are missing or duplicated")
    for row in records:
        if row.get("kind") != "dogfood-record" or not isinstance(row.get("data"), dict):
            raise ValueError("dataset contains an invalid public append fixture")
        payload_text = row["data"].get("payload")
        if not isinstance(payload_text, str) or len(payload_text.encode()) != RECORD_PAYLOAD_BYTES:
            raise ValueError("dataset contains a malformed deterministic record payload")
    fixture = payload.get("fixture")
    if not isinstance(fixture, dict) or fixture.get("path") != FIXTURE_PATH.as_posix() or not str(fixture.get("sha256", "")).startswith("sha256:"):
        raise ValueError("dataset fixture identity is missing or invalid")
    return payload, digest


def mature_history_payload(seed: int) -> str:
    rng = random.Random(seed + 995)
    alphabet = "abcdefghjkmnpqrstuvwxyz23456789"
    payload = "".join(rng.choice(alphabet) for _ in range(MATURE_HISTORY_BYTES))
    if len(payload.encode("utf-8")) != MATURE_HISTORY_BYTES:
        raise ValueError("mature irrelevant-history fixture has the wrong byte volume")
    return payload


def oversized_mature_subject(fixture: dict[str, Any], seed: int) -> dict[str, Any]:
    subject = json.loads(json.dumps(fixture))
    subject["problem"] += "\n\nDeterministic mature-fixture context: " + mature_history_payload(seed)
    return subject


def make_distribution(values: list[float | int]) -> dict[str, float | int]:
    if not values:
        raise ValueError("cannot summarize an empty sample set")
    ordered = sorted(values)
    p95_index = max(0, math.ceil(0.95 * len(ordered)) - 1)
    return {
        "count": len(ordered),
        "min": ordered[0],
        "median": statistics.median(ordered),
        "p95": ordered[p95_index],
        "max": ordered[-1],
    }


def sample_distributions(samples: list[dict[str, Any]]) -> dict[str, Any]:
    return {
        "delivered_bytes": make_distribution([int(sample["delivered_bytes"]) for sample in samples]),
        "process_to_last_byte_ms": make_distribution([float(sample["process_to_last_byte_ms"]) for sample in samples]),
    }


def repository_identity(cwd: Path) -> dict[str, str]:
    def git(*args: str) -> str:
        result = subprocess.run(["git", *args], cwd=cwd, capture_output=True, text=True, check=False)
        if result.returncode != 0:
            raise ValueError(f"git {' '.join(args)} failed in execution cwd {cwd}: {result.stderr.strip()}")
        return result.stdout.strip()

    top = Path(git("rev-parse", "--show-toplevel")).resolve()
    if top != cwd.resolve():
        raise ValueError(f"collect must run from repository root; execution cwd is {cwd}, root is {top}")
    # Import the existing project identity helper, which includes index/status,
    # tracked bytes, modes, symlinks, and non-ignored untracked files.
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    try:
        from test_contract import repository_proof_identity
        fingerprint = repository_proof_identity(cwd)
    finally:
        try:
            sys.path.remove(str(Path(__file__).resolve().parent))
        except ValueError:
            pass
    return {"cwd": str(cwd.resolve()), "head": git("rev-parse", "HEAD"), "fingerprint": fingerprint}


class CollectionFailure(RuntimeError):
    pass


class Collector:
    def __init__(self, args: argparse.Namespace, dataset: dict[str, Any], dataset_sha: str):
        self.args = args
        self.dataset = dataset
        self.dataset_sha = dataset_sha
        self.engine = require_absolute(args.engine, "engine")
        self.provider = require_absolute(args.provider, "provider")
        self.data_root = require_absolute(args.data_root, "data-root")
        self.cwd = Path.cwd().resolve()
        self.output = ensure_external_output(args.output, "output")
        self.captures = self.output.parent / ".dogfood-measure-captures" / dataset_sha.removeprefix("sha256:") / ("baseline" if args.phase == "baseline" else "current_")
        self.workspace = self.output.parent / ".dogfood-measure-workspace"
        if self.captures.exists():
            raise ValueError(f"capture directory already exists; refusing to overwrite: {self.captures}")
        if self.workspace.exists():
            raise ValueError(f"isolated measurement workspace already exists; inspect/remove explicitly: {self.workspace}")
        fixture = self.data_root / FIXTURE_PATH
        if not self.engine.is_file() or not os.access(self.engine, os.X_OK):
            raise ValueError(f"engine is not an executable file: {self.engine}")
        if not self.provider.is_file() or not os.access(self.provider, os.X_OK):
            raise ValueError(f"provider is not an executable file: {self.provider}")
        if not self.data_root.is_dir() or not fixture.is_file():
            raise ValueError(f"data-root lacks the required public intent fixture: {fixture}")
        self.fixture_bytes = fixture.read_bytes()
        if sha256(self.fixture_bytes) != dataset["fixture"]["sha256"]:
            raise ValueError("data-root intent fixture does not match the seeded dataset; inputs are not comparable")
        self.captures.mkdir(parents=True)
        self.workspace.mkdir()
        self.stream_dir = self.captures / "streams"
        self.stream_dir.mkdir()
        self.command_rows: list[dict[str, Any]] = []
        self.errors: list[str] = []
        self.sample_counter = 0
        self.generic_provider = self.workspace / "dogfood-provider.py"
        self.worker = self.workspace / "scripted-worker.py"
        self.review_worker = self.workspace / "scripted-review-worker.py"
        self.graph_worker = self.workspace / "scripted-graph-worker.py"
        self.generic_provider.write_text(GENERIC_PROVIDER_SOURCE, encoding="utf-8")
        self.worker.write_text(BOUND_WORKER_SOURCE, encoding="utf-8")
        self.review_worker.write_text(REVIEW_WORKER_SOURCE, encoding="utf-8")
        self.graph_worker.write_text(GRAPH_WORKER_SOURCE, encoding="utf-8")
        self.config = self.workspace / "providers.toml"
        self.config.write_text(
            "[providers.fixture]\ncommand = " + json.dumps(sys.executable) + "\nargs = [" + json.dumps(str(self.generic_provider)) + ", " + json.dumps(dataset["large_action_instructions"]) + "]\n\n"
            "[providers.software-change]\ncommand = " + json.dumps(str(self.provider)) + "\nargs = []\n",
            encoding="utf-8",
        )
        self.report: dict[str, Any] = {
            "schema_version": RECEIPT_SCHEMA_VERSION,
            "receipt_type": "dogfood-measurement",
            "phase": args.phase,
            "complete": False,
            "dataset_sha256": dataset_sha,
            "dataset_version": dataset["dataset_version"],
            "seed": dataset["seed"],
            "collector": {
                "argv": [sys.executable, *sys.argv],
                "cwd": str(self.cwd),
                "script": str(Path(__file__).resolve()),
                "python": sys.version,
            },
            "execution": {"repository": repository_identity(self.cwd)},
            "binaries": {},
            "setup_compatibility": {
                "roster_contract": "legacy-without-token-budget" if args.phase == "baseline" else "current-with-token-budget",
                "advice_contract": "legacy-option-unavailable" if args.phase == "baseline" else "current-explicit-decline",
                "scripted_token_budget": None if args.phase == "baseline" else SCRIPTED_TOKEN_BUDGET,
                "advice_configuration": None,
            },
            "review_output_contract": None,
            "oversized_mandatory_subject_refusal": None,
            "fixtures": {
                "intent_fixture_sha256": sha256(self.fixture_bytes),
                "generic_provider_sha256": sha256(GENERIC_PROVIDER_SOURCE.encode()),
                "bound_worker_sha256": sha256(BOUND_WORKER_SOURCE.encode()),
                "review_worker_sha256": sha256(REVIEW_WORKER_SOURCE.encode()),
                "graph_worker_sha256": sha256(GRAPH_WORKER_SOURCE.encode()),
            },
            "data_root": str(self.data_root),
            "workspace": str(self.workspace),
            "capture_root": str(self.captures),
            "commands": self.command_rows,
            "read_lanes": {lane: {surface: [] for surface in READ_SURFACES} for lane in READ_LANES},
            "review_delivery": {size: {"profile": None, "output_contract": None, "samples": []} for size in REVIEW_SIZES},
            "graph": {"status": "not-run", "workers_expected": GRAPH_WORKERS, "samples": []},
            "errors": self.errors,
        }

    def _stream_capture(self, name: str, stdout: bytes, stderr: bytes) -> dict[str, Any]:
        safe = "".join(ch if ch.isalnum() or ch in "-_." else "_" for ch in name)
        stdout_path = self.stream_dir / f"{safe}.stdout"
        stderr_path = self.stream_dir / f"{safe}.stderr"
        stdout_path.write_bytes(stdout)
        stderr_path.write_bytes(stderr)
        return {
            "stdout_path": str(stdout_path),
            "stdout_bytes": len(stdout),
            "stdout_sha256": sha256(stdout),
            "stderr_path": str(stderr_path),
            "stderr_bytes": len(stderr),
            "stderr_sha256": sha256(stderr),
        }

    def _record_process(
        self,
        label: str,
        argv: list[str],
        *,
        input_bytes: bytes | None = None,
        timeout: float = 30,
        cwd: Path | None = None,
        retain_streams: bool = False,
    ) -> tuple[dict[str, Any], subprocess.CompletedProcess[bytes] | None]:
        started_ns = time.perf_counter_ns()
        row: dict[str, Any] = {
            "label": label,
            "argv": argv,
            "cwd": str((cwd or self.cwd).resolve()),
            "started_monotonic_ns": started_ns,
            "stdin_bytes": len(input_bytes) if input_bytes is not None else None,
            "stdin_sha256": sha256(input_bytes) if input_bytes is not None else None,
            "timeout_seconds": timeout,
            "returncode": None,
            "timed_out": False,
            "spawn_error": None,
        }
        try:
            result = subprocess.run(argv, cwd=cwd or self.cwd, input=input_bytes, capture_output=True, timeout=timeout, check=False)
            elapsed_ms = (time.perf_counter_ns() - started_ns) / 1_000_000
            row.update({"returncode": result.returncode, "elapsed_ms": elapsed_ms, "process_to_last_byte_ms": elapsed_ms})
            row.update({"stdout_bytes": len(result.stdout), "stdout_sha256": sha256(result.stdout), "stderr_bytes": len(result.stderr), "stderr_sha256": sha256(result.stderr)})
            if retain_streams:
                row.update(self._stream_capture(f"{self.sample_counter:05d}-{label}", result.stdout, result.stderr))
                self.sample_counter += 1
            self.command_rows.append(row)
            return row, result
        except subprocess.TimeoutExpired as error:
            elapsed_ms = (time.perf_counter_ns() - started_ns) / 1_000_000
            stdout = error.stdout or b""
            stderr = error.stderr or b""
            if isinstance(stdout, str):
                stdout = stdout.encode()
            if isinstance(stderr, str):
                stderr = stderr.encode()
            row.update({"timed_out": True, "elapsed_ms": elapsed_ms, "process_to_last_byte_ms": elapsed_ms,
                        "stdout_bytes": len(stdout), "stdout_sha256": sha256(stdout), "stderr_bytes": len(stderr), "stderr_sha256": sha256(stderr),
                        "timeout_error": str(error)})
            if retain_streams:
                row.update(self._stream_capture(f"{self.sample_counter:05d}-{label}", stdout, stderr))
                self.sample_counter += 1
            self.command_rows.append(row)
            self.errors.append(f"{label} timed out after {timeout:g}s")
            return row, None
        except OSError as error:
            elapsed_ms = (time.perf_counter_ns() - started_ns) / 1_000_000
            row.update({"spawn_error": str(error), "elapsed_ms": elapsed_ms, "process_to_last_byte_ms": elapsed_ms,
                        "stdout_bytes": 0, "stdout_sha256": sha256(b""), "stderr_bytes": 0, "stderr_sha256": sha256(b"")})
            self.command_rows.append(row)
            self.errors.append(f"{label} could not start: {error}")
            return row, None

    def checked_json(self, label: str, argv: list[str], *, timeout: float = 30, retain_streams: bool = False) -> tuple[dict[str, Any], dict[str, Any]]:
        row, result = self._record_process(label, argv, timeout=timeout, retain_streams=retain_streams)
        if result is None:
            raise CollectionFailure(self.errors[-1])
        if result.returncode != 0:
            detail = result.stderr.decode(errors="replace")[-2000:] or result.stdout.decode(errors="replace")[-2000:]
            self.errors.append(f"{label} exited {result.returncode}: {detail}")
            raise CollectionFailure(self.errors[-1])
        try:
            value = json.loads(result.stdout)
        except (UnicodeDecodeError, json.JSONDecodeError) as error:
            self.errors.append(f"{label} returned non-JSON output: {error}")
            raise CollectionFailure(self.errors[-1]) from error
        if not isinstance(value, dict):
            self.errors.append(f"{label} returned a non-object JSON value")
            raise CollectionFailure(self.errors[-1])
        return row, value

    def metadata_command(self, name: str, binary: Path) -> dict[str, Any]:
        row, result = self._record_process(f"{name}-version", [str(binary), "--version"], timeout=10)
        if result is None or result.returncode != 0:
            detail = self.errors[-1] if self.errors else f"{name} --version failed"
            raise CollectionFailure(detail)
        text = (result.stdout + result.stderr).decode(errors="replace").strip()
        return {"path": str(binary), "sha256": file_sha(binary), "bytes": binary.stat().st_size,
                "version_argv": row["argv"], "version_output": text,
                "version_returncode": result.returncode}

    def write_report(self) -> None:
        write_json(self.output, self.report)

    def fail(self, message: str) -> None:
        self.errors.append(message)
        self.write_report()
        raise CollectionFailure(message)

    def engine_argv(self, database: Path, *args: str) -> list[str]:
        return [str(self.engine), "--database", str(database), "--json", *map(str, args)]

    def create_run(self, database: Path, run_id: str, *, sleep_seconds: float, children: int = 1, child_sleep: float = 0.0) -> dict[str, Any]:
        marker = self.workspace / f"{run_id}.active.json"
        binding = {
            "command": sys.executable,
            "args": [str(self.worker), "--marker", str(marker), "--sleep", str(sleep_seconds),
                     "--children", str(children), "--child-sleep", str(child_sleep)],
        }
        initial = {"work_slot_bindings": {"scripted-work": binding}}
        input_path = self.workspace / f"{run_id}.initial.json"
        write_json(input_path, initial)
        _, started = self.checked_json(
            f"start-{run_id}",
            self.engine_argv(database, "--config", str(self.config), "start", "--id", run_id,
                             "fixture", "@" + str(input_path), "Dogfood read measurement"),
        )
        if started.get("status") != "completed" or started.get("result", {}).get("run", {}).get("id") != run_id:
            self.fail(f"public start did not create expected fixture run {run_id}")
        return {"run_id": run_id, "marker": marker, "sleep_seconds": sleep_seconds, "children": children}

    def wait_for_public_invocation_success(self, database: Path, run_id: str, invocation_id: str,
                                          *, timeout: float, label: str,
                                          marker: Path | None = None) -> dict[str, Any]:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            _, shown = self.checked_json(
                f"{label}-show", self.engine_argv(database, "show", run_id, "--view", "full"), timeout=30,
                retain_streams=True,
            )
            if shown.get("status") != "completed":
                self.fail(f"{label} public full show did not complete")
            result = shown.get("result", {})
            invocations = result.get("invocations")
            rows = invocations.get("items") if isinstance(invocations, dict) else None
            if not isinstance(rows, list):
                rows = result.get("work_slot_invocations", [])
            invocation = next((row for row in rows if row.get("invocation_id") == invocation_id), None)
            state = invocation.get("execution", {}).get("state") if isinstance(invocation, dict) else None
            state = state or (invocation.get("status") if isinstance(invocation, dict) else None)
            if state == "succeeded":
                if marker is not None and not marker.is_file():
                    self.fail(f"{label} invocation succeeded without its scripted worker marker")
                return invocation
            if state in ("failed", "cancelled", "timed_out"):
                self.fail(f"{label} invocation reached terminal {state} instead of succeeding")
            time.sleep(min(1.0, max(0.0, deadline - time.monotonic())))
        self.fail(f"{label} invocation did not durably succeed within {timeout:g} seconds")
        raise AssertionError("self.fail must raise")

    def append_history(self, database: Path, run_id: str) -> None:
        _, observed = self.checked_json(
            f"observe-before-append-{run_id}", self.engine_argv(database, "show", run_id, "--view", "full")
        )
        if observed.get("status") != "completed" or not isinstance(observed.get("result"), dict):
            self.fail(f"public full show did not observe {run_id} before history appends")
        for index, record in enumerate(self.dataset["records"]):
            path = self.workspace / f"{run_id}-record-{index:04d}.json"
            write_json(path, record["data"])
            argv = self.engine_argv(database, "append", "--record-id", record["record_id"], run_id,
                                    record["kind"], "@" + str(path))
            row, result = self._record_process(f"append-{run_id}-{index:04d}", argv, timeout=30, retain_streams=True)
            if result is None or result.returncode != 0:
                detail = "spawn/timeout" if result is None else result.stderr.decode(errors="replace")[-1000:]
                self.errors.append(f"public append failed for {run_id}/{record['record_id']}: {detail}")
                raise CollectionFailure(self.errors[-1])
            try:
                envelope = json.loads(result.stdout)
            except Exception as error:
                self.errors.append(f"public append returned invalid JSON: {error}")
                raise CollectionFailure(self.errors[-1]) from error
            if envelope.get("status") != "completed":
                self.errors.append(f"public append was not completed for {record['record_id']}")
                raise CollectionFailure(self.errors[-1])

    def invoke_process(self, database: Path, run_id: str) -> subprocess.Popen[bytes]:
        action = self.engine_argv(database, "show", run_id, "--view", "action")
        _, shown = self.checked_json(f"arm-{run_id}", action)
        if shown.get("status") != "completed" or not isinstance(shown.get("result"), dict):
            self.fail(f"could not observe the public action view before invoking {run_id}")
        argv = [str(self.engine), "--database", str(database), "--json", "--timeout-ms", "120000",
                "invoke", run_id, "scripted-work"]
        started_ns = time.perf_counter_ns()
        try:
            process = subprocess.Popen(argv, cwd=self.cwd, stdin=subprocess.DEVNULL,
                                       stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        except OSError as error:
            self.errors.append(f"invoke-{run_id} could not start: {error}")
            raise CollectionFailure(self.errors[-1]) from error
        setattr(process, "_dogfood_started_ns", started_ns)
        setattr(process, "_dogfood_argv", argv)
        return process

    def await_active(self, database: Path, actor: dict[str, Any], process: subprocess.Popen[bytes]) -> str:
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            if not actor["marker"].is_file():
                time.sleep(0.02)
                continue
            _, shown = self.checked_json(f"active-show-{actor['run_id']}", self.engine_argv(database, "show", actor["run_id"], "--view", "status"))
            result = shown.get("result", {})
            invocations = result.get("invocations")
            rows = invocations.get("items") if isinstance(invocations, dict) else None
            if not isinstance(rows, list):
                # P01's actual pre-edit 0.22.0 binary uses the older full-view
                # invocation array; keep the matched baseline workload runnable.
                rows = result.get("work_slot_invocations", [])
            active = [row for row in rows if row.get("slot_id") == "scripted-work"
                      and (row.get("execution", {}).get("state") == "running" or row.get("status") == "running")]
            if active:
                return str(active[-1]["invocation_id"])
            time.sleep(0.02)
        detail = ""
        if process.poll() is not None:
            stdout, stderr = process.communicate()
            streams = self._stream_capture(f"invoke-no-active-{actor['run_id']}", stdout, stderr)
            self.command_rows.append({
                "label": f"invoke-no-active-{actor['run_id']}", "argv": getattr(process, "_dogfood_argv"),
                "cwd": str(self.cwd), "returncode": process.returncode,
                "elapsed_ms": (time.perf_counter_ns() - getattr(process, "_dogfood_started_ns")) / 1_000_000,
                **streams,
            })
            detail = ": " + (stderr.decode(errors="replace")[-1000:] or stdout.decode(errors="replace")[-1000:])
        self.errors.append(f"invoke-{actor['run_id']} never reached observed active work{detail}")
        raise CollectionFailure(self.errors[-1])

    def finish_invocations(self, database: Path, workers: list[tuple[dict[str, Any], subprocess.Popen[bytes], str]]) -> None:
        for actor, process, invocation_id in workers:
            try:
                stdout, stderr = process.communicate(timeout=ACTIVE_HOLD_SECONDS + 45)
            except subprocess.TimeoutExpired:
                self.errors.append(f"owned fixture invocation {invocation_id} did not quiesce")
                raise CollectionFailure(self.errors[-1])
            elapsed_ms = (time.perf_counter_ns() - getattr(process, "_dogfood_started_ns")) / 1_000_000
            streams = self._stream_capture(f"invoke-final-{actor['run_id']}", stdout, stderr)
            row = {"label": f"invoke-final-{actor['run_id']}", "argv": getattr(process, "_dogfood_argv"),
                   "cwd": str(self.cwd), "returncode": process.returncode, "elapsed_ms": elapsed_ms,
                   "process_to_last_byte_ms": elapsed_ms, **streams}
            self.command_rows.append(row)
            if process.returncode != 0:
                detail = stderr.decode(errors="replace")[-1500:]
                self.errors.append(f"invoke-{actor['run_id']} exited {process.returncode}: {detail}")
                raise CollectionFailure(self.errors[-1])
            try:
                envelope = json.loads(stdout)
            except Exception as error:
                self.errors.append(f"invoke-{actor['run_id']} returned invalid JSON: {error}")
                raise CollectionFailure(self.errors[-1]) from error
            if envelope.get("status") != "completed" or envelope.get("result", {}).get("invocation_id") != invocation_id:
                self.errors.append(f"invoke-{actor['run_id']} did not complete its observed invocation")
                raise CollectionFailure(self.errors[-1])
            self.wait_for_public_invocation_success(
                database, actor["run_id"], invocation_id, timeout=ACTIVE_HOLD_SECONDS + 45,
                label=f"invoke-completion-{invocation_id}", marker=actor["marker"],
            )

    def sample_read(self, database: Path, lane: str, run_id: str, invocation_id: str,
                    surface: str, index: int, active: list[subprocess.Popen[bytes]]) -> dict[str, Any]:
        active_count = len(active)
        base: list[str]
        if surface == "status":
            base = self.engine_argv(database, "show", run_id, "--view", "status")
        elif surface == "action":
            base = self.engine_argv(database, "show", run_id, "--view", "action")
        elif surface == "history":
            base = self.engine_argv(database, "history", run_id)
        elif surface == "invocation-progress":
            base = self.engine_argv(database, "invocation-progress", run_id, invocation_id)
        elif surface == "monitor":
            sample = self.monitor_sample(database, run_id, lane, index, active_count)
            self.report["read_lanes"][lane][surface].append(sample)
            return sample
        else:
            raise AssertionError(surface)
        label = f"read-{lane}-{surface}-{index:02d}"
        row, result = self._record_process(label, base, timeout=30, retain_streams=True)
        if result is None or result.returncode != 0:
            detail = "spawn/timeout" if result is None else result.stderr.decode(errors="replace")[-1000:]
            self.errors.append(f"{label} failed: {detail}")
            raise CollectionFailure(self.errors[-1])
        try:
            envelope = json.loads(result.stdout)
        except Exception as error:
            self.errors.append(f"{label} returned invalid JSON: {error}")
            raise CollectionFailure(self.errors[-1]) from error
        assertions = self.assert_read_outcome(surface, envelope, run_id, invocation_id)
        sample = {
            "sample_id": label, "lane": lane, "surface": surface, "run_id": run_id, "invocation_id": invocation_id,
            "argv": base, "cwd": str(self.cwd), "returncode": result.returncode,
            "timed_out": False, "process_to_last_byte_ms": row["process_to_last_byte_ms"],
            "delivered_bytes": len(result.stdout), "assertions": assertions,
            "active_work": {"expected_processes": active_count, "observed_processes": active_count},
            "complete": True,
            "stdout_path": row["stdout_path"], "stdout_bytes": row["stdout_bytes"], "stdout_sha256": row["stdout_sha256"],
            "stderr_path": row["stderr_path"], "stderr_bytes": row["stderr_bytes"], "stderr_sha256": row["stderr_sha256"],
        }
        self.report["read_lanes"][lane][surface].append(sample)
        return sample

    def assert_read_outcome(self, surface: str, envelope: Any, run_id: str, invocation_id: str) -> list[str]:
        if not isinstance(envelope, dict) or envelope.get("status") != "completed" or not isinstance(envelope.get("result"), (dict, list)):
            raise CollectionFailure(f"{surface} output has no completed public CLI result")
        result = envelope["result"]
        assertions = ["public-cli-envelope-completed", "public-outcome-asserted"]
        if surface in ("status", "action"):
            if not isinstance(result, dict) or result.get("current_state") not in ("work", "done"):
                raise CollectionFailure(f"{surface} output omitted the observed workflow state")
            assertions.append("target-run-state-observed")
            if surface == "action" and not ("current_state_instructions" in result or "current_action" in result):
                # Old and new CLI view schemas differ; the frozen public action
                # must still expose one inspectable duty-bearing field.
                if not any("instruction" in str(key).lower() or "action" in str(key).lower() for key in result):
                    raise CollectionFailure("action view omitted its public duty/instruction surface")
                assertions.append("action-duty-field-observed")
        elif surface == "history":
            if isinstance(result, list):
                assertions.append("full-history-public-record-list-observed")
            elif isinstance(result, dict) and isinstance(result.get("items"), list) and isinstance(result.get("total"), int):
                if result.get("truncated") and result.get("next_cursor") is None:
                    raise CollectionFailure("paged history read omitted its next cursor")
                assertions.append("history-page-and-exact-total-observed")
            else:
                raise CollectionFailure("history output omitted its public record list or exact page total")
        elif surface == "invocation-progress":
            if not isinstance(result, dict) or result.get("invocation_id") != invocation_id:
                raise CollectionFailure("invocation-progress did not identify the sampled invocation")
            assertions.append("sampled-invocation-identity-observed")
        return assertions

    def monitor_sample(self, database: Path, run_id: str, lane: str, index: int, active_count: int) -> dict[str, Any]:
        argv = [str(self.engine), "monitor", "--run", run_id, "--database", str(database),
                "--json", "--poll-seconds", "0.05"]
        started_ns = time.perf_counter_ns()
        try:
            process = subprocess.Popen(argv, cwd=self.cwd, stdin=subprocess.DEVNULL,
                                       stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        except OSError as error:
            self.errors.append(f"monitor-{lane}-{index:02d} could not start: {error}")
            raise CollectionFailure(self.errors[-1]) from error
        assert process.stdout is not None
        import selectors
        selector = selectors.DefaultSelector()
        selector.register(process.stdout, selectors.EVENT_READ)
        first_line = b""
        first_line_ms: float | None = None
        try:
            events = selector.select(timeout=5)
            if events:
                first_line = process.stdout.readline()
                first_line_ms = (time.perf_counter_ns() - started_ns) / 1_000_000
        finally:
            selector.close()
        if not first_line:
            process.terminate()
            stdout, stderr = process.communicate(timeout=5)
            self._stream_capture(f"monitor-{lane}-{index:02d}", stdout, stderr)
            self.errors.append("monitor produced no complete JSONL snapshot within five seconds")
            raise CollectionFailure(self.errors[-1])
        # The first complete snapshot follows all selected read paths. Stop the
        # continuous observer immediately instead of waiting for another poll.
        process.terminate()
        rest, stderr = process.communicate(timeout=5)
        if process.returncode != -15:
            self.errors.append(f"monitor fixed-window stop was not SIGTERM (returncode={process.returncode})")
            raise CollectionFailure(self.errors[-1])
        stdout = first_line + rest
        elapsed_ms = (time.perf_counter_ns() - started_ns) / 1_000_000
        streams = self._stream_capture(f"read-{lane}-monitor-{index:02d}", stdout, stderr)
        lines = stdout.splitlines(keepends=True)
        packets = []
        partial_final_line = b""
        for line_index, line in enumerate(lines):
            content = line.rstrip(b"\r\n")
            if not content:
                continue
            try:
                packets.append(json.loads(content))
            except json.JSONDecodeError as error:
                if (line_index == len(lines) - 1 and not line.endswith((b"\n", b"\r"))
                        and process.returncode == -15):
                    partial_final_line = line
                    continue
                self.errors.append(f"monitor emitted invalid JSONL before its bounded stop: {error}")
                raise CollectionFailure(self.errors[-1]) from error
        if not packets or not any(packet.get("source") == f"run:{run_id}" for packet in packets):
            self.errors.append("monitor JSONL omitted the selected public run source")
            raise CollectionFailure(self.errors[-1])
        return {
            "sample_id": f"read-{lane}-monitor-{index:02d}", "lane": lane, "surface": "monitor", "run_id": run_id,
            "argv": argv, "cwd": str(self.cwd), "returncode": None,
            "intentional_stop": "fixed_observation_window", "signal": 15,
            "process_to_last_byte_ms": elapsed_ms, "first_output_ms": first_line_ms,
            "delivered_bytes": len(stdout), "assertions": ["public-cli-jsonl-snapshot", "public-outcome-asserted", "selected-run-source-observed"],
            "active_work": {"expected_processes": active_count, "observed_processes": active_count},
            "complete": True, "snapshot_count": len(packets), "complete_jsonl_lines": len(packets),
            "partial_final_line_bytes": len(partial_final_line),
            "partial_final_line_sha256": sha256(partial_final_line) if partial_final_line else None,
            **streams,
        }

    def run_read_lanes(self) -> None:
        database = self.workspace / "read-workload.sqlite"
        idle = self.create_run(database, "dogfood-idle", sleep_seconds=0)
        serial = self.create_run(database, "dogfood-serial", sleep_seconds=ACTIVE_HOLD_SECONDS)
        parallel = self.create_run(database, "dogfood-parallel", sleep_seconds=ACTIVE_HOLD_SECONDS)
        for actor in (idle, serial, parallel):
            self.append_history(database, actor["run_id"])
        actors: dict[str, list[dict[str, Any]]] = {"idle": [idle], "serial": [serial], "parallel": [parallel]}
        parallel_actors = []
        for index in range(PARALLEL_ACTIVE_RUNS - 1):
            parallel_actors.append(self.create_run(database, f"dogfood-parallel-{index:02d}", sleep_seconds=ACTIVE_HOLD_SECONDS))
        actors["parallel"].extend(parallel_actors)
        invocation_ids: dict[str, dict[str, str]] = {lane: {} for lane in READ_LANES}

        # Observe before invoking, then verify the fast worker's durable success;
        # the CLI receipt may arrive before its external worker writes the marker.
        _, idle_action = self.checked_json(
            f"arm-{idle['run_id']}", self.engine_argv(database, "show", idle["run_id"], "--view", "action")
        )
        if idle_action.get("status") != "completed" or idle_action.get("result", {}).get("current_state") != "work":
            self.fail("public action view did not observe the idle fixture before invoking")
        idle_argv = self.engine_argv(database, "--config", str(self.config), "invoke", idle["run_id"], "scripted-work")
        _, idle_result = self.checked_json(f"invoke-{idle['run_id']}", idle_argv, timeout=30, retain_streams=True)
        idle_id = idle_result.get("result", {}).get("invocation_id")
        if idle_result.get("status") != "completed" or not isinstance(idle_id, str):
            self.fail("idle scripted invocation did not return a completed public invocation receipt")
        self.wait_for_public_invocation_success(
            database, idle["run_id"], idle_id, timeout=10,
            label=f"idle-invocation-{idle_id}", marker=idle["marker"],
        )
        invocation_ids["idle"][idle["run_id"]] = idle_id
        idle["invocation_id"] = idle_id
        self.measure_lane(database, "idle", idle["run_id"], idle_id, [], actor_runs=[idle])

        serial_proc = self.invoke_process(database, serial["run_id"])
        serial_id = self.await_active(database, serial, serial_proc)
        serial["invocation_id"] = serial_id
        invocation_ids["serial"][serial["run_id"]] = serial_id
        serial_group = [(serial, serial_proc, serial_id)]
        try:
            self.measure_lane(database, "serial", serial["run_id"], serial_id, [serial_proc], actor_runs=[serial])
        finally:
            self.finish_invocations(database, serial_group)

        parallel_group: list[tuple[dict[str, Any], subprocess.Popen[bytes], str]] = []
        try:
            for actor in actors["parallel"]:
                process = self.invoke_process(database, actor["run_id"])
                invocation_id = self.await_active(database, actor, process)
                actor["invocation_id"] = invocation_id
                invocation_ids["parallel"][actor["run_id"]] = invocation_id
                parallel_group.append((actor, process, invocation_id))
            target_id = invocation_ids["parallel"][parallel["run_id"]]
            self.measure_lane(database, "parallel", parallel["run_id"], target_id,
                              [row[1] for row in parallel_group], actor_runs=actors["parallel"])
        finally:
            if parallel_group:
                self.finish_invocations(database, parallel_group)

    def assert_active_work(self, database: Path, lane: str, actors: list[dict[str, Any]]) -> list[str]:
        observed_ids = []
        for actor in actors:
            invocation_id = actor.get("invocation_id")
            if not isinstance(invocation_id, str) or not actor["marker"].is_file():
                self.fail(f"{lane} active worker lacks its observed invocation ID or scripted marker")
            _, shown = self.checked_json(
                f"assert-active-{lane}-{actor['run_id']}",
                self.engine_argv(database, "show", actor["run_id"], "--view", "status"), retain_streams=True,
            )
            if shown.get("status") != "completed":
                self.fail(f"{lane} status view did not complete for {actor['run_id']}")
            result = shown.get("result", {})
            invocations = result.get("invocations")
            rows = invocations.get("items") if isinstance(invocations, dict) else None
            if not isinstance(rows, list):
                rows = result.get("work_slot_invocations", [])
            invocation = next((row for row in rows if row.get("invocation_id") == invocation_id), None)
            state = invocation.get("execution", {}).get("state") if isinstance(invocation, dict) else None
            state = state or (invocation.get("status") if isinstance(invocation, dict) else None)
            if state != "running":
                self.fail(f"{lane} worker {actor['run_id']} was not publicly observed running")
            observed_ids.append(invocation_id)
        return observed_ids

    def measure_lane(self, database: Path, lane: str, run_id: str, invocation_id: str,
                     active: list[subprocess.Popen[bytes]], actor_runs: list[dict[str, Any]]) -> None:
        observed_start = self.assert_active_work(database, lane, actor_runs) if lane != "idle" else []
        for sample_index in range(READ_SAMPLES):
            for surface in READ_SURFACES:
                self.sample_read(database, lane, run_id, invocation_id, surface, sample_index, active)
        if lane == "idle" and active:
            raise CollectionFailure("idle lane unexpectedly has active work")
        observed_end = self.assert_active_work(database, lane, actor_runs) if lane != "idle" else []
        expected_count = 0 if lane == "idle" else (1 if lane == "serial" else PARALLEL_ACTIVE_RUNS)
        self.report["read_lanes"][lane]["active_work"] = {
            "expected_processes": expected_count,
            "invocation_ids": [actor["invocation_id"] for actor in actor_runs],
            "public_run_ids": [actor["run_id"] for actor in actor_runs],
            "observed_running_before_samples": lane != "idle" and len(observed_start) == expected_count,
            "observed_running_after_samples": lane != "idle" and len(observed_end) == expected_count,
            "observed_invocation_ids_before": observed_start,
            "observed_invocation_ids_after": observed_end,
            "controlled_by": "scripted bound workers with fixed sleep; public status observed before/after samples; no model or live catalog",
        }

    def append_mature_irrelevant_history(self, database: Path, run_id: str, sample_id: str,
                                         area: Path) -> dict[str, Any]:
        payload = mature_history_payload(self.dataset["seed"])
        payload_bytes = payload.encode("utf-8")
        if len(payload_bytes) != self.dataset["mature_irrelevant_history_bytes"]:
            self.fail("mature irrelevant-history payload does not match the versioned exact byte count")
        record_id = f"dogfood-history-{self.dataset['seed']}-{sample_id}"
        record_data = {"payload": payload}
        input_path = area / "irrelevant-history.json"
        write_json(input_path, record_data)
        input_bytes = input_path.read_bytes()
        capture_path = self.captures / f"{sample_id}.irrelevant-history.json"
        capture_path.write_bytes(input_bytes)
        append_argv = self.engine_argv(
            database, "append", "--record-id", record_id, run_id,
            self.dataset["mature_irrelevant_history_kind"], "@" + str(input_path),
        )
        append_row, appended = self._record_process(
            f"append-{sample_id}-irrelevant-history", append_argv, timeout=30, retain_streams=True
        )
        if appended is None or appended.returncode != 0:
            detail = "spawn/timeout" if appended is None else appended.stderr.decode(errors="replace")[-1000:]
            self.fail(f"public irrelevant-history append failed for {sample_id}: {detail}")
        try:
            append_envelope = json.loads(appended.stdout)
        except (UnicodeDecodeError, json.JSONDecodeError) as error:
            self.fail(f"public irrelevant-history append returned invalid JSON for {sample_id}: {error}")
        if append_envelope.get("status") != "completed":
            self.fail(f"public irrelevant-history append was not completed for {sample_id}")
        history_argv = self.engine_argv(database, "history", run_id)
        history_row, history_result = self._record_process(
            f"confirm-{sample_id}-irrelevant-history", history_argv, timeout=30, retain_streams=True
        )
        if history_result is None or history_result.returncode != 0:
            detail = "spawn/timeout" if history_result is None else history_result.stderr.decode(errors="replace")[-1000:]
            self.fail(f"public history confirmation failed for {sample_id}: {detail}")
        try:
            history_envelope = json.loads(history_result.stdout)
        except (UnicodeDecodeError, json.JSONDecodeError) as error:
            self.fail(f"public history confirmation returned invalid JSON for {sample_id}: {error}")
        result = history_envelope.get("result")
        history_items = result if isinstance(result, list) else result.get("items", []) if isinstance(result, dict) else []
        matches = [
            row for row in history_items if isinstance(row, dict)
            and row.get("action", {}).get("kind") == "context_appended"
            and row.get("action", {}).get("context_record_id") == record_id
        ]
        if history_envelope.get("status") != "completed" or len(matches) != 1:
            self.fail(f"public history did not retain exactly one mature fixture record for {sample_id}")
        return {
            "record_id": record_id, "kind": MATURE_HISTORY_KIND,
            "payload_bytes": len(payload_bytes), "payload_sha256": sha256(payload_bytes),
            "record_data_path": str(capture_path), "record_data_bytes": len(input_bytes),
            "record_data_sha256": sha256(input_bytes), "append_argv": append_argv,
            "append_returncode": append_row["returncode"],
            "append_stdout_path": append_row["stdout_path"], "append_stdout_bytes": append_row["stdout_bytes"],
            "append_stdout_sha256": append_row["stdout_sha256"],
            "append_stderr_path": append_row["stderr_path"], "append_stderr_bytes": append_row["stderr_bytes"],
            "append_stderr_sha256": append_row["stderr_sha256"],
            "history_argv": history_argv, "history_returncode": history_row["returncode"],
            "history_stdout_path": history_row["stdout_path"], "history_stdout_bytes": history_row["stdout_bytes"],
            "history_stdout_sha256": history_row["stdout_sha256"],
            "history_stderr_path": history_row["stderr_path"], "history_stderr_bytes": history_row["stderr_bytes"],
            "history_stderr_sha256": history_row["stderr_sha256"],
            "history_event": matches[0],
            "assertions": ["public-append-completed", "public-history-event-observed", "exact-65536-byte-history-payload"],
        }

    def run_review_deliveries(self) -> None:
        fixture = json.loads(self.fixture_bytes)
        current_phase = self.args.phase == "current"
        expected_profile_version = CURRENT_CONFIG_VERSION if current_phase else BASELINE_CONFIG_VERSION
        expected_output_contract = "current-v2" if current_phase else "legacy-v1"
        for size in REVIEW_SIZES:
            for sample_index in range(REVIEW_SAMPLES):
                sample_id = f"review-{size}-{sample_index:02d}"
                area = self.workspace / sample_id
                area.mkdir()
                subject = json.loads(json.dumps(fixture))
                subject_bytes = canonical_bytes(subject)
                inbox_path = self.captures / f"{sample_id}.stdin"
                worker_meta_path = self.captures / f"{sample_id}.worker.json"
                worker_output_path = self.captures / f"{sample_id}.worker-output.json"
                subject_capture = self.captures / f"{sample_id}.intent.json"
                subject_capture.write_bytes(subject_bytes)
                roster_path = area / "roster.json"
                roster_capture = self.captures / f"{sample_id}.roster.json"
                roster_entry = {
                    "author": REVIEW_AUTHOR,
                    "command": sys.executable,
                    "args": [str(self.review_worker), str(inbox_path), str(worker_meta_path),
                             str(subject_capture), str(worker_output_path), expected_output_contract],
                }
                if current_phase:
                    roster_entry["token_budget"] = SCRIPTED_TOKEN_BUDGET
                write_json(roster_path, [roster_entry])
                roster_bytes = roster_path.read_bytes()
                roster_capture.write_bytes(roster_bytes)
                profile_path = area / "profile.json"
                setup_argv = [str(self.provider), "setup", "--rigor", "minimal", "--roster", str(roster_path),
                              "--engine", str(self.engine), "--provider", str(self.provider), "--output", str(profile_path)]
                if current_phase:
                    setup_argv.append("--decline-advice")
                _, setup = self.checked_json(f"setup-{sample_id}", setup_argv, timeout=30)
                if setup.get("status") != "ready" or not profile_path.is_file():
                    self.fail(f"public software-change setup did not produce the {sample_id} profile")
                advice_configuration = setup.get("advice_configuration")
                if current_phase and (not isinstance(advice_configuration, dict)
                                      or advice_configuration.get("decision") != "decline"
                                      or advice_configuration.get("enabled") is not False):
                    self.fail(f"current setup did not retain explicit disabled-advice evidence for {sample_id}")
                if current_phase:
                    self.report["setup_compatibility"]["advice_configuration"] = advice_configuration
                profile_bytes = profile_path.read_bytes()
                profile_capture = self.captures / f"{sample_id}.profile.json"
                profile_capture.write_bytes(profile_bytes)
                profile = json.loads(profile_bytes)
                signature = profile_signature(profile)
                if signature.get("contract_version") != 3 or signature.get("config_version") != expected_profile_version:
                    self.fail(f"{sample_id} did not use the expected preserved review profile identity")
                output_contract = profile_review_output_contract(profile)
                if output_contract["version"] != expected_output_contract:
                    self.fail(f"{sample_id} setup emitted an unrecognized review output contract")
                delivery = self.report["review_delivery"][size]
                if delivery["profile"] is None:
                    delivery["profile"] = signature
                    delivery["output_contract"] = output_contract
                elif delivery["profile"] != signature or delivery["output_contract"] != output_contract:
                    self.fail(f"profile or output-contract identity changed between {size} review samples")
                if self.report["review_output_contract"] is None:
                    self.report["review_output_contract"] = output_contract
                elif self.report["review_output_contract"] != output_contract:
                    self.fail("review output contract changed between public setup samples")
                database = self.workspace / "review-workload.sqlite"
                run_id = sample_id
                _, started = self.checked_json(
                    f"start-{sample_id}",
                    self.engine_argv(database, "--config", str(self.config), "start", "--id", run_id,
                                     "software-change", "@" + str(profile_path), "Dogfood review-delivery measurement"),
                )
                if started.get("status") != "completed" or started.get("result", {}).get("run", {}).get("id") != run_id:
                    self.fail(f"public start did not create software-change review fixture {sample_id}")
                artifact_root = started["result"]["run"]["initial_input"].get("artifact_root")
                if not isinstance(artifact_root, str) or not Path(artifact_root).is_absolute():
                    self.fail(f"software-change start did not return an allocated artifact root for {sample_id}")
                artifact_subject = Path(artifact_root, "intent.json")
                artifact_subject.write_bytes(subject_bytes)
                if artifact_subject.read_bytes() != subject_capture.read_bytes():
                    self.fail(f"captured review subject differs from the public-run subject for {sample_id}")
                _, observed = self.checked_json(f"show-before-ready-{sample_id}", self.engine_argv(database, "show", run_id, "--view", "full"))
                if observed.get("status") != "completed":
                    self.fail(f"could not observe {sample_id} before intent-ready")
                mature_history = None
                if size == "mature":
                    mature_history = self.append_mature_irrelevant_history(database, run_id, sample_id, area)
                    _, observed_after_history = self.checked_json(
                        f"show-after-history-{sample_id}", self.engine_argv(database, "show", run_id, "--view", "full")
                    )
                    if observed_after_history.get("status") != "completed":
                        self.fail(f"could not re-observe {sample_id} after its mature history append")
                _, ready = self.checked_json(f"event-intent-ready-{sample_id}", self.engine_argv(database, "event", run_id, "intent-ready"), timeout=30)
                if ready.get("status") != "completed":
                    self.fail(f"software-change intent-ready did not complete for {sample_id}")
                _, action = self.checked_json(f"show-review-action-{sample_id}", self.engine_argv(database, "show", run_id, "--view", "action"))
                if action.get("status") != "completed" or action.get("result", {}).get("current_state") != "intent-review":
                    self.fail(f"public path did not reach intent-review for {sample_id}")
                invoke_argv = self.engine_argv(database, "--config", str(self.config), "invoke", run_id, "intent-review")
                invoke_row, invoke = self.checked_json(f"invoke-{sample_id}", invoke_argv, timeout=120, retain_streams=True)
                invocation_id = invoke.get("result", {}).get("invocation_id")
                if not isinstance(invocation_id, str):
                    self.fail(f"public invoke did not return an invocation ID for {sample_id}")
                invocation = self.wait_for_public_invocation_success(
                    database, run_id, invocation_id, timeout=120, label=f"review-invocation-{sample_id}"
                )
                if not inbox_path.is_file() or not worker_meta_path.is_file() or not worker_output_path.is_file():
                    self.fail(f"scripted reviewer did not retain actual assignment, output, and delivered stdin for {sample_id}")
                packet = inbox_path.read_bytes()
                meta = json.loads(worker_meta_path.read_bytes())
                output_bytes = worker_output_path.read_bytes()
                if not packet or meta.get("stdin_bytes") != len(packet) or not meta.get("argv"):
                    self.fail(f"captured review packet or actual worker argv is incomplete for {sample_id}")
                assignment = meta.get("assignment")
                expected_assignment = {
                    "review_stage": "aggregate", "author": REVIEW_AUTHOR,
                    "policies": signature["intent_review"],
                }
                if assignment != expected_assignment:
                    self.fail(f"actual {sample_id} assignment does not match its complete named intent-review policy")
                if mature_history is not None:
                    history_payload = json.loads(Path(mature_history["record_data_path"]).read_bytes())["payload"].encode("utf-8")
                    if mature_history["record_id"].encode() in packet or history_payload in packet:
                        self.fail(f"genuinely irrelevant mature history was routed into the {sample_id} review packet")
                received_ns = meta.get("stdin_received_monotonic_ns")
                started_ns = invoke_row.get("started_monotonic_ns")
                if not isinstance(received_ns, int) or not isinstance(started_ns, int) or received_ns < started_ns:
                    self.fail(f"{sample_id} does not retain a valid invocation-start-to-worker-receive clock interval")
                delivery_latency_ms = (received_ns - started_ns) / 1_000_000
                if delivery_latency_ms <= 0:
                    self.fail(f"{sample_id} worker received its review input without a positive measured latency")
                if meta.get("output_contract") != expected_output_contract:
                    self.fail(f"scripted worker selected the wrong output contract for {sample_id}")
                budget_line = meta.get("token_budget_line")
                if current_phase != (isinstance(budget_line, str) and budget_line.startswith("PER-CALL TOKEN WINDOW: ")):
                    self.fail(f"{sample_id} did not expose the expected version-specific token-budget framing")
                validate_scripted_review_output(output_bytes, signature, expected_output_contract, sha256(subject_bytes))
                sample_assertions = ["public-setup-ready", "public-intent-ready", "public-review-slot-invoked",
                                     "bound-invocation-succeeded", "review-stdin-captured", "output-contract-validated"]
                if mature_history is not None:
                    sample_assertions.append("mature-history-stored-and-not-routed")
                sample_assertions.append("public-outcome-asserted")
                sample = {
                    "sample_id": sample_id, "size_class": size, "sample_index": sample_index,
                    "public_run_id": run_id, "invocation_id": invocation_id,
                    "setup_argv": setup_argv, "setup_advice_configuration": advice_configuration,
                    "roster_path": str(roster_capture), "roster_bytes": len(roster_bytes),
                    "roster_sha256": sha256(roster_bytes),
                    "profile_sha256": sha256(profile_bytes), "profile_bytes": len(profile_bytes),
                    "profile_path": str(profile_capture), "profile_signature": signature,
                    "output_contract_version": expected_output_contract,
                    "review_assignment": assignment, "token_budget_line": budget_line,
                    "mature_irrelevant_history": mature_history,
                    "invoke_argv": invoke_argv,
                    "worker_argv": meta["argv"], "worker_executable": meta["executable"],
                    "worker_executable_sha256": file_sha(Path(meta["executable"]).resolve()),
                    "worker_output_path": str(worker_output_path), "worker_output_bytes": len(output_bytes),
                    "worker_output_sha256": sha256(output_bytes),
                    "subject_sha256": sha256(subject_bytes), "subject_bytes": len(subject_bytes),
                    "subject_path": str(subject_capture),
                    "delivered_bytes": len(packet), "stdin_sha256": sha256(packet), "stdin_path": str(inbox_path),
                    "invoke_started_monotonic_ns": started_ns,
                    "worker_stdin_received_monotonic_ns": received_ns,
                    "process_to_last_byte_ms": delivery_latency_ms,
                    "invoke_receipt_process_to_last_byte_ms": invoke_row["process_to_last_byte_ms"],
                    "invocation_returncode": invoke_row["returncode"],
                    "invocation_stdout_path": invoke_row["stdout_path"],
                    "invocation_stdout_bytes": invoke_row["stdout_bytes"],
                    "invocation_stdout_sha256": invoke_row["stdout_sha256"],
                    "invocation_stderr_path": invoke_row["stderr_path"],
                    "invocation_stderr_bytes": invoke_row["stderr_bytes"],
                    "invocation_stderr_sha256": invoke_row["stderr_sha256"],
                    "assertions": sample_assertions,
                    "complete": True,
                }
                delivery["samples"].append(sample)
        for size in REVIEW_SIZES:
            samples = self.report["review_delivery"][size]["samples"]
            self.report["review_delivery"][size]["distribution"] = sample_distributions(samples)

    def run_oversized_mandatory_subject_refusal(self) -> None:
        if self.args.phase != "current":
            return
        fixture = json.loads(self.fixture_bytes)
        subject_bytes = canonical_bytes(oversized_mature_subject(fixture, self.dataset["seed"]))
        if len(subject_bytes) <= 32768:
            self.fail("oversized mandatory-source negative fixture no longer exceeds the current 32 KiB limit")
        area = self.workspace / "oversized-mandatory-subject"
        area.mkdir()
        sample_id = "oversized-mandatory-subject"
        inbox_path = self.captures / f"{sample_id}.stdin"
        worker_meta_path = self.captures / f"{sample_id}.worker.json"
        worker_output_path = self.captures / f"{sample_id}.worker-output.json"
        subject_capture = self.captures / f"{sample_id}.intent.json"
        subject_capture.write_bytes(subject_bytes)
        roster_path = area / "roster.json"
        roster_capture = self.captures / f"{sample_id}.roster.json"
        write_json(roster_path, [{
            "author": REVIEW_AUTHOR,
            "command": sys.executable,
            "args": [str(self.review_worker), str(inbox_path), str(worker_meta_path),
                     str(subject_capture), str(worker_output_path), "current-v2"],
            "token_budget": SCRIPTED_TOKEN_BUDGET,
        }])
        roster_bytes = roster_path.read_bytes()
        roster_capture.write_bytes(roster_bytes)
        profile_path = area / "profile.json"
        setup_argv = [str(self.provider), "setup", "--rigor", "minimal", "--roster", str(roster_path),
                      "--engine", str(self.engine), "--provider", str(self.provider), "--output", str(profile_path),
                      "--decline-advice"]
        setup_row, setup = self.checked_json(
            f"setup-{sample_id}", setup_argv, timeout=30, retain_streams=True
        )
        if setup.get("status") != "ready" or not profile_path.is_file():
            self.fail("current setup did not produce the oversized-subject negative profile")
        advice = setup.get("advice_configuration")
        if not isinstance(advice, dict) or advice.get("decision") != "decline" or advice.get("enabled") is not False:
            self.fail("oversized-subject negative did not explicitly decline current advice")
        profile_bytes = profile_path.read_bytes()
        profile_capture = self.captures / f"{sample_id}.profile.json"
        profile_capture.write_bytes(profile_bytes)
        profile = json.loads(profile_bytes)
        signature = profile_signature(profile)
        if signature.get("contract_version") != 3 or signature.get("config_version") != CURRENT_CONFIG_VERSION:
            self.fail("oversized-subject negative did not use current minimal-12")
        output_contract = profile_review_output_contract(profile)
        if output_contract.get("version") != "current-v2":
            self.fail("oversized-subject negative did not use the current review output contract")
        database = self.workspace / "oversized-subject-workload.sqlite"
        run_id = "dogfood-oversized-mandatory-subject"
        _, started = self.checked_json(
            f"start-{sample_id}",
            self.engine_argv(database, "--config", str(self.config), "start", "--id", run_id,
                             "software-change", "@" + str(profile_path), "Dogfood oversized-source refusal"),
        )
        if started.get("status") != "completed" or started.get("result", {}).get("run", {}).get("id") != run_id:
            self.fail("public start did not create the oversized-subject negative run")
        artifact_root = started["result"]["run"]["initial_input"].get("artifact_root")
        if not isinstance(artifact_root, str) or not Path(artifact_root).is_absolute():
            self.fail("oversized-subject negative did not receive an allocated artifact root")
        artifact_subject = Path(artifact_root, "intent.json")
        artifact_subject.write_bytes(subject_bytes)
        subject_sha256 = sha256(subject_bytes)
        if artifact_subject.read_bytes() != subject_capture.read_bytes():
            self.fail("oversized mandatory subject differs from the preserved negative fixture")
        _, observed = self.checked_json(
            f"show-before-ready-{sample_id}", self.engine_argv(database, "show", run_id, "--view", "full")
        )
        if observed.get("status") != "completed":
            self.fail("could not observe oversized-subject negative run before intent-ready")
        _, ready = self.checked_json(
            f"event-intent-ready-{sample_id}", self.engine_argv(database, "event", run_id, "intent-ready"), timeout=30
        )
        if ready.get("status") != "completed":
            self.fail("oversized-subject negative did not complete intent-ready")
        _, action = self.checked_json(
            f"show-review-action-{sample_id}", self.engine_argv(database, "show", run_id, "--view", "action")
        )
        if action.get("status") != "completed" or action.get("result", {}).get("current_state") != "intent-review":
            self.fail("oversized-subject negative did not reach intent-review")
        invoke_argv = self.engine_argv(database, "--config", str(self.config), "invoke", run_id, "intent-review")
        invoke_row, invoke_result = self._record_process(
            f"invoke-{sample_id}", invoke_argv, timeout=120, retain_streams=True
        )
        if invoke_result is None or invoke_result.returncode != 20:
            detail = "spawn/timeout" if invoke_result is None else invoke_result.stderr.decode(errors="replace")[-1000:]
            self.fail(f"oversized mandatory-source invocation did not fail with the preserved public refusal: {detail}")
        try:
            envelope = json.loads(invoke_result.stdout)
        except (UnicodeDecodeError, json.JSONDecodeError) as error:
            self.fail(f"oversized mandatory-source refusal returned invalid public JSON: {error}")
        message = envelope.get("message", "") if isinstance(envelope, dict) else ""
        if (envelope.get("status") != "error" or envelope.get("code") != "provider-execution-failed"
                or "initial_limit=32768" not in message or "no source was truncated" not in message
                or f"originals={len(subject_bytes)}" not in message):
            self.fail("oversized mandatory-source refusal no longer proves the exact bound and no-truncation outcome")
        subject_after = artifact_subject.read_bytes()
        if sha256(subject_after) != subject_sha256 or len(subject_after) != len(subject_bytes):
            self.fail("oversized mandatory subject changed or was truncated during the public refusal")
        worker_launched = inbox_path.exists() or worker_meta_path.exists() or worker_output_path.exists()
        if worker_launched:
            self.fail("oversized mandatory-source refusal launched the scripted review worker")
        self.report["oversized_mandatory_subject_refusal"] = {
            "complete": True, "expected_refusal": True, "dataset_version": DATASET_VERSION,
            "dataset_sha256": self.dataset_sha,
            "profile_identity": {"contract_version": signature["contract_version"],
                                 "config_version": signature["config_version"]},
            "output_contract_version": output_contract["version"],
            "setup_argv": setup_argv, "setup_returncode": setup_row["returncode"],
            "setup_stdout_path": setup_row["stdout_path"], "setup_stdout_bytes": setup_row["stdout_bytes"],
            "setup_stdout_sha256": setup_row["stdout_sha256"],
            "setup_stderr_path": setup_row["stderr_path"], "setup_stderr_bytes": setup_row["stderr_bytes"],
            "setup_stderr_sha256": setup_row["stderr_sha256"], "setup_advice_configuration": advice,
            "roster_path": str(roster_capture), "roster_bytes": len(roster_bytes),
            "roster_sha256": sha256(roster_bytes), "scripted_token_budget": SCRIPTED_TOKEN_BUDGET,
            "profile_path": str(profile_capture), "profile_bytes": len(profile_bytes),
            "profile_signature": signature,
            "profile_sha256": sha256(profile_bytes),
            "invoke_argv": invoke_argv, "invoke_returncode": invoke_row["returncode"],
            "invoke_stdout_path": invoke_row["stdout_path"], "invoke_stdout_bytes": invoke_row["stdout_bytes"],
            "invoke_stdout_sha256": invoke_row["stdout_sha256"],
            "invoke_stderr_path": invoke_row["stderr_path"], "invoke_stderr_bytes": invoke_row["stderr_bytes"],
            "invoke_stderr_sha256": invoke_row["stderr_sha256"],
            "provider_error": message,
            "subject_path": str(subject_capture), "subject_bytes": len(subject_bytes),
            "subject_sha256_before": subject_sha256,
            "subject_sha256_after": sha256(subject_after),
            "subject_bytes_after": len(subject_after),
            "worker_paths": {"stdin": str(inbox_path), "meta": str(worker_meta_path), "output": str(worker_output_path)},
            "worker_launched": False,
            "assertions": ["public-start-intent-ready-invoke", "initial-32k-limit-enforced",
                           "oversized-mandatory-original-refused", "no-source-truncation-reported",
                           "subject-bytes-unchanged", "review-worker-not-launched"],
        }

    def run_graph(self) -> None:
        dagu = shutil.which("dagu")
        self.report["graph"]["dagu"] = dagu
        self.report["graph"]["dagu_sha256"] = file_sha(Path(dagu).resolve()) if dagu else None
        if not dagu:
            self.report["graph"].update({"status": "incomplete", "error": "dagu executable is not available on PATH"})
            self.errors.append("14-worker scripted graph unavailable: dagu executable missing")
            raise CollectionFailure(self.errors[-1])
        graph_root = self.workspace / "graph"
        graph_root.mkdir()
        instructions = graph_root / "instructions.txt"
        instructions.write_text("Run the fixed 14-worker scripted graph fixture; no review judgment is requested.\n", encoding="utf-8")
        argv = [str(self.engine), "fan-out", "--max-active", str(GRAPH_WORKERS), "--instructions", str(instructions)]
        for worker_id in range(GRAPH_WORKERS):
            spec = {"command": sys.executable, "args": [str(self.graph_worker), f"worker-{worker_id:02d}", "0.02"]}
            argv.extend(["--worker", json.dumps(spec, separators=(",", ":"))])
        row, result = self._record_process("graph-14-workers", argv, timeout=180, cwd=graph_root, retain_streams=True)
        if result is None or result.returncode != 0:
            detail = "spawn/timeout" if result is None else result.stderr.decode(errors="replace")[-2000:]
            self.report["graph"].update({"status": "incomplete", "error": detail, "argv": argv})
            self.errors.append(f"14-worker public graph failed: {detail}")
            raise CollectionFailure(self.errors[-1])
        try:
            summary = json.loads(result.stdout)
        except Exception as error:
            self.report["graph"].update({"status": "incomplete", "error": str(error), "argv": argv})
            self.errors.append(f"14-worker graph returned invalid summary: {error}")
            raise CollectionFailure(self.errors[-1]) from error
        workers = summary.get("workers") if isinstance(summary, dict) else None
        if not isinstance(workers, list) or len(workers) != GRAPH_WORKERS:
            self.report["graph"].update({"status": "incomplete", "argv": argv, "worker_count": len(workers) if isinstance(workers, list) else None})
            self.errors.append("14-worker graph summary omitted the complete declared worker inventory")
            raise CollectionFailure(self.errors[-1])
        expected_specs = {
            (str(self.graph_worker), f"worker-{worker_id:02d}", "0.02")
            for worker_id in range(GRAPH_WORKERS)
        }
        observed_specs = {
            tuple(row.get("args", []))
            for row in workers
            if isinstance(row, dict) and row.get("command") == sys.executable and isinstance(row.get("args"), list)
        }
        if any(not isinstance(worker, dict) or worker.get("exit_code") != 0 for worker in workers) or observed_specs != expected_specs:
            self.report["graph"].update({"status": "incomplete", "argv": argv, "worker_count": len(workers),
                                         "expected_worker_specs": sorted(expected_specs), "observed_worker_specs": sorted(observed_specs)})
            self.errors.append("14-worker graph has a missing, duplicate, changed or nonzero scripted worker")
            raise CollectionFailure(self.errors[-1])
        instructions_capture = self.captures / "graph-instructions.txt"
        instructions_bytes = instructions.read_bytes()
        instructions_capture.write_bytes(instructions_bytes)
        graph_source = graph_root / "fan-out-adhoc"
        if not graph_source.is_dir():
            self.errors.append("14-worker fan-out completed without its isolated public capture directory")
            raise CollectionFailure(self.errors[-1])
        graph_artifacts = self.captures / "graph-artifacts"
        shutil.copytree(graph_source, graph_artifacts)
        artifact_inventory = [
            {"path": str(path), "bytes": path.stat().st_size, "sha256": file_sha(path)}
            for path in sorted(graph_artifacts.rglob("*")) if path.is_file()
        ]
        if not artifact_inventory:
            self.errors.append("14-worker graph capture directory contained no retained output")
            raise CollectionFailure(self.errors[-1])
        sample = {
            "sample_id": "graph-14-workers", "argv": argv, "cwd": str(graph_root),
            "returncode": result.returncode, "delivered_bytes": len(result.stdout), "elapsed_ms": row["elapsed_ms"],
            "process_to_last_byte_ms": row["process_to_last_byte_ms"],
            "workers_expected": GRAPH_WORKERS, "workers_observed": len(workers),
            "worker_ids": [f"worker-{worker_id:02d}" for worker_id in range(GRAPH_WORKERS)],
            "instructions_path": str(instructions_capture), "instructions_sha256": sha256(instructions_bytes),
            "artifacts_directory": str(graph_artifacts), "artifacts": artifact_inventory,
            "assertions": ["public-graph-summary-observed", "14-worker-inventory-complete", "all-worker-exits-zero", "public-outcome-asserted"],
            "complete": True, "stdout_path": row["stdout_path"], "stdout_bytes": row["stdout_bytes"], "stdout_sha256": row["stdout_sha256"],
            "stderr_path": row["stderr_path"], "stderr_bytes": row["stderr_bytes"], "stderr_sha256": row["stderr_sha256"],
        }
        self.report["graph"].update({"status": "complete", "workers_expected": GRAPH_WORKERS, "workers_observed": len(workers), "samples": [sample],
                                     "elapsed_time_role": "descriptive-only; no graph throughput acceptance threshold"})

    def run(self) -> None:
        self.report["binaries"] = {
            "engine": self.metadata_command("engine", self.engine),
            "provider": self.metadata_command("provider", self.provider),
        }
        if self.args.phase == "current":
            self.run_oversized_mandatory_subject_refusal()
        self.run_read_lanes()
        self.run_review_deliveries()
        self.run_graph()
        repository_after = repository_identity(self.cwd)
        self.report["execution"]["repository_after"] = repository_after
        if repository_after != self.report["execution"]["repository"]:
            self.fail("repository identity changed during collection; samples do not share one stable execution tree")
        self.report["sample_distributions"] = {
            lane: {surface: sample_distributions(samples) for surface, samples in self.report["read_lanes"][lane].items()
                   if surface in READ_SURFACES}
            for lane in READ_LANES
        }
        self.report["complete"] = True
        validate_collection(self.report, check_streams=True)
        self.write_report()
        shutil.rmtree(self.workspace)
        self.report["workspace_retained"] = False
        # Workspace state is informational only; avoid rewriting the completed
        # receipt after its stream paths and identities were validated.
        self.write_report()


def profile_signature(profile: dict[str, Any]) -> dict[str, Any]:
    policies = profile.get("review_policies", {}).get("intent-review")
    if not isinstance(policies, list) or not policies:
        raise CollectionFailure("minimal public setup profile lacks intent-review policies")
    workload = []
    for row in policies:
        if not isinstance(row, dict):
            raise CollectionFailure("intent-review profile contains a non-object policy")
        workload.append({
            "id": row.get("id"),
            "description": row.get("description"),
            "example_prompt": row.get("example_prompt"),
            "review_stage": row.get("review_stage", row.get("stage", "aggregate")),
            "required_authors": row.get("required_authors"),
        })
    return {
        "contract_version": profile.get("contract_version"),
        "config_version": profile.get("config_version"),
        "intent_review": workload,
    }


def review_output_contract_version(schema: dict[str, Any]) -> str:
    required = schema.get("required")
    properties = schema.get("properties")
    if not isinstance(required, list) or not isinstance(properties, dict):
        raise CollectionFailure("setup profile contains an incomplete review output schema")
    judgment_schema = properties.get("judgments", {}).get("items", {}) if isinstance(properties.get("judgments"), dict) else {}
    branches = judgment_schema.get("oneOf") if isinstance(judgment_schema, dict) else None
    if not isinstance(branches, list) or not branches or not isinstance(branches[0], dict):
        raise CollectionFailure("setup profile review output schema lacks judgment fields")
    judgment_required = branches[0].get("required")
    if not isinstance(judgment_required, list):
        raise CollectionFailure("setup profile review output schema lacks judgment requirements")
    if (set(required) >= {"review_stage", "author", "judgments"}
            and "review_contract_version" not in required
            and "grounds" not in judgment_required):
        return "legacy-v1"
    contract_property = properties.get("review_contract_version")
    if (set(required) >= {"review_contract_version", "review_stage", "author", "judgments"}
            and isinstance(contract_property, dict) and contract_property.get("const") == 2
            and "grounds" in judgment_required):
        return "current-v2"
    raise CollectionFailure("setup profile has an unsupported review output contract")


def profile_review_output_contract(profile: dict[str, Any]) -> dict[str, Any]:
    binding = profile.get("work_slot_bindings", {}).get("intent-review")
    arguments = binding.get("args") if isinstance(binding, dict) else None
    if not isinstance(arguments, list):
        raise CollectionFailure("setup profile lacks the public intent-review binding")
    workers = []
    for index, argument in enumerate(arguments[:-1]):
        if argument != "--worker":
            continue
        try:
            worker = json.loads(arguments[index + 1])
        except (json.JSONDecodeError, TypeError) as error:
            raise CollectionFailure(f"intent-review binding contains invalid worker JSON: {error}") from error
        if isinstance(worker, dict):
            workers.append(worker)
    if len(workers) != 1:
        raise CollectionFailure("minimal intent-review workload must bind exactly one scripted worker")
    schema = workers[0].get("full_output_schema")
    if not isinstance(schema, dict):
        raise CollectionFailure("intent-review worker binding lacks its full output schema")
    return {
        "version": review_output_contract_version(schema),
        "schema_sha256": sha256(canonical_bytes(schema)),
        "schema": schema,
    }


def parse_review_assignment(raw: bytes) -> tuple[dict[str, Any], str | None]:
    lines = raw.decode("utf-8", errors="replace").splitlines()
    def field(name: str, required: bool = True) -> str | None:
        prefix = name + ": "
        for line in lines:
            if line.startswith(prefix):
                return line[len(prefix):]
        if required:
            raise ValueError(f"review packet is missing assignment field {name}")
        return None
    try:
        policies = json.loads(field("assigned_policies"))
    except (TypeError, json.JSONDecodeError) as error:
        raise ValueError(f"review packet has invalid assigned_policies: {error}") from error
    if not isinstance(policies, list) or not policies:
        raise ValueError("review packet contains no assigned policies")
    assignment = {
        "review_stage": field("review_stage"),
        "author": field("required_author_claim"),
        "policies": policies,
    }
    budget_line = field("PER-CALL TOKEN WINDOW", required=False)
    return assignment, f"PER-CALL TOKEN WINDOW: {budget_line}" if budget_line is not None else None


def validate_scripted_review_output(raw: bytes, signature: dict[str, Any], contract: str,
                                    subject_sha256: str) -> dict[str, Any]:
    try:
        output = json.loads(raw)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError(f"scripted review output is not JSON: {error}") from error
    if not isinstance(output, dict) or output.get("review_stage") != "aggregate":
        raise ValueError("scripted review output does not name the aggregate stage")
    if output.get("author") != {"name": REVIEW_AUTHOR, "kind": "agent"}:
        raise ValueError("scripted review output has the wrong exact author")
    judgments = output.get("judgments")
    policies = signature.get("intent_review")
    if not isinstance(policies, list) or not isinstance(judgments, list):
        raise ValueError("scripted review output/profile omits named judgments")
    if [row.get("axis") for row in judgments if isinstance(row, dict)] != [row.get("id") for row in policies]:
        raise ValueError("scripted review output does not cover the complete named intent-review workload")
    for judgment in judgments:
        if not isinstance(judgment, dict) or judgment.get("result") != "pass" or judgment.get("findings") != "":
            raise ValueError("scripted review fixture changed its deterministic pass output")
    if contract == "legacy-v1":
        if "review_contract_version" in output or any("grounds" in row for row in judgments):
            raise ValueError("legacy review output contains fields outside its v1 schema")
    elif contract == "current-v2":
        if output.get("review_contract_version") != 2:
            raise ValueError("current review output omits review_contract_version 2")
        for judgment in judgments:
            grounds = judgment.get("grounds")
            if not isinstance(grounds, dict) or grounds.get("reason") != "Scripted measurement fixture only; no semantic review was performed.":
                raise ValueError("current review output lacks its scripted-only grounds marker")
            if grounds.get("evidence") != [{"locator": "intent.json#/problem", "sha256": subject_sha256}]:
                raise ValueError("current review output does not cite the exact captured subject bytes")
    else:
        raise ValueError(f"unknown scripted review output contract {contract}")
    return output


def validate_sample_stream(sample: dict[str, Any], *, check_streams: bool) -> None:
    if sample.get("complete") is not True:
        raise ValueError(f"sample {sample.get('sample_id')} is incomplete")
    assertions = sample.get("assertions")
    if not isinstance(assertions, list) or "public-outcome-asserted" not in assertions:
        raise ValueError(f"sample {sample.get('sample_id')} has no asserted public outcome")
    if not isinstance(sample.get("delivered_bytes"), int) or sample["delivered_bytes"] <= 0:
        raise ValueError(f"sample {sample.get('sample_id')} has no delivered bytes")
    latency = sample.get("process_to_last_byte_ms")
    if not isinstance(latency, (int, float)) or not math.isfinite(float(latency)) or latency <= 0:
        raise ValueError(f"sample {sample.get('sample_id')} has invalid process-to-last-byte latency")
    surface = sample.get("surface")
    if surface == "monitor":
        if sample.get("intentional_stop") != "fixed_observation_window" or sample.get("signal") != 15:
            raise ValueError("monitor sample lacks its explicitly bounded continuous-observer stop")
    elif sample.get("returncode") != 0 or sample.get("timed_out") is True:
        raise ValueError(f"sample {sample.get('sample_id')} did not complete its public CLI process")
    stream_bytes: dict[str, bytes] = {}
    for key, path_key, length_key, digest_key in (
        ("stdout", "stdout_path", "stdout_bytes", "stdout_sha256"),
        ("stderr", "stderr_path", "stderr_bytes", "stderr_sha256"),
    ):
        path = sample.get(path_key)
        length = sample.get(length_key)
        digest = sample.get(digest_key)
        if not isinstance(path, str) or not isinstance(length, int) or not isinstance(digest, str) or not digest.startswith("sha256:"):
            raise ValueError(f"sample {sample.get('sample_id')} omits the {key} stream identity")
        if check_streams:
            stream = Path(path)
            if not stream.is_file():
                raise ValueError(f"sample {sample.get('sample_id')} {key} stream is absent: {stream}")
            raw = stream.read_bytes()
            if len(raw) != length or sha256(raw) != digest:
                raise ValueError(f"sample {sample.get('sample_id')} {key} stream differs from its receipt")
            stream_bytes[key] = raw
    if surface != "monitor" and sample["delivered_bytes"] != sample.get("stdout_bytes"):
        raise ValueError(f"sample {sample.get('sample_id')} delivered byte count is not its actual stdout length")
    if check_streams and surface in READ_SURFACES:
        if surface == "monitor":
            raw_monitor = stream_bytes["stdout"]
            lines = raw_monitor.splitlines(keepends=True)
            packets = []
            partial_final_line = b""
            for line_index, line in enumerate(lines):
                content = line.rstrip(b"\r\n")
                if not content:
                    continue
                try:
                    packets.append(json.loads(content))
                except json.JSONDecodeError as error:
                    if (line_index == len(lines) - 1 and not line.endswith((b"\n", b"\r"))
                            and sample.get("signal") == 15):
                        partial_final_line = line
                        continue
                    raise ValueError(f"monitor sample contains invalid JSONL before its bounded stop: {error}") from error
            if not any(packet.get("source") == f"run:{sample.get('run_id')}" for packet in packets):
                raise ValueError("monitor sample has no selected-run public observation")
            if sample.get("complete_jsonl_lines") != len(packets):
                raise ValueError("monitor sample's complete JSONL line count differs from retained bytes")
            if (sample.get("partial_final_line_bytes") != len(partial_final_line)
                    or sample.get("partial_final_line_sha256") != (sha256(partial_final_line) if partial_final_line else None)):
                raise ValueError("monitor sample's explicitly retained partial final line differs from its stream")
        else:
            try:
                envelope = json.loads(stream_bytes["stdout"])
            except (UnicodeDecodeError, json.JSONDecodeError) as error:
                raise ValueError(f"{surface} sample has no public JSON envelope: {error}") from error
            result = envelope.get("result") if isinstance(envelope, dict) else None
            if envelope.get("status") != "completed":
                raise ValueError(f"{surface} sample's public CLI envelope is not completed")
            if surface in ("status", "action"):
                if not isinstance(result, dict) or not isinstance(result.get("current_state"), str):
                    raise ValueError(f"{surface} sample does not expose the current workflow state")
                if surface == "action" and not any("instruction" in str(key).lower() or "action" in str(key).lower() for key in result):
                    raise ValueError("action sample has no duty/instruction field")
            elif surface == "history" and not (isinstance(result, list) or (isinstance(result, dict) and isinstance(result.get("items"), list) and isinstance(result.get("total"), int))):
                raise ValueError("history sample does not contain its record list or exact page total")
            elif surface == "invocation-progress" and (not isinstance(result, dict) or result.get("invocation_id") != sample.get("invocation_id")):
                raise ValueError("invocation-progress sample does not identify the selected invocation")


def validate_oversized_mandatory_subject_refusal(report: dict[str, Any], *, check_streams: bool) -> None:
    refusal = report.get("oversized_mandatory_subject_refusal")
    if report.get("phase") == "baseline":
        if refusal is not None:
            raise ValueError("preserved legacy baseline must not be relabeled as the current bound-refusal proof")
        return
    if not isinstance(refusal, dict) or refusal.get("complete") is not True or refusal.get("expected_refusal") is not True:
        raise ValueError("current collection lacks the explicit oversized mandatory-subject refusal proof")
    if refusal.get("dataset_version") != DATASET_VERSION or refusal.get("dataset_sha256") != report.get("dataset_sha256"):
        raise ValueError("oversized-source refusal is not bound to the current v2 dataset")
    if (refusal.get("profile_identity") != {"contract_version": 3, "config_version": CURRENT_CONFIG_VERSION}
            or refusal.get("output_contract_version") != "current-v2"):
        raise ValueError("oversized-source refusal does not use the exact current profile/output contracts")
    subject_bytes = refusal.get("subject_bytes")
    if (not isinstance(subject_bytes, int) or subject_bytes <= 32768
            or refusal.get("invoke_returncode") != 20
            or refusal.get("subject_bytes_after") != subject_bytes
            or refusal.get("subject_sha256_before") != refusal.get("subject_sha256_after")
            or refusal.get("worker_launched") is not False):
        raise ValueError("oversized-source refusal does not prove an unchanged, unlaunched subject")
    message = refusal.get("provider_error")
    if (not isinstance(message, str) or "initial_limit=32768" not in message
            or f"originals={subject_bytes}" not in message or "no source was truncated" not in message):
        raise ValueError("oversized-source refusal omits its exact enforced limit or no-truncation evidence")
    assertions = refusal.get("assertions")
    required_assertions = {
        "public-start-intent-ready-invoke", "initial-32k-limit-enforced",
        "oversized-mandatory-original-refused", "no-source-truncation-reported",
        "subject-bytes-unchanged", "review-worker-not-launched",
    }
    if not isinstance(assertions, list) or not required_assertions.issubset(assertions):
        raise ValueError("oversized-source refusal lacks its complete public-path assertion set")
    advice = refusal.get("setup_advice_configuration")
    if not isinstance(advice, dict) or advice.get("decision") != "decline" or advice.get("enabled") is not False:
        raise ValueError("oversized-source refusal setup did not preserve explicit current advice decline")
    if not check_streams:
        return
    fixture_path = Path(report.get("data_root", "")) / FIXTURE_PATH
    if not fixture_path.is_file():
        raise ValueError("current data-root fixture is unavailable for oversized-source proof validation")
    expected_subject = canonical_bytes(oversized_mature_subject(json.loads(fixture_path.read_bytes()), report["seed"]))
    subject_path = Path(refusal.get("subject_path", ""))
    subject_raw = subject_path.read_bytes() if subject_path.is_file() else None
    if (subject_raw != expected_subject or len(subject_raw or b"") != subject_bytes
            or sha256(subject_raw or b"") != refusal.get("subject_sha256_before")):
        raise ValueError("oversized-source refusal subject differs from the exact preserved deterministic fixture")
    for prefix in ("setup_stdout", "setup_stderr", "invoke_stdout", "invoke_stderr"):
        path = Path(refusal.get(f"{prefix}_path", ""))
        raw = path.read_bytes() if path.is_file() else None
        if (raw is None or len(raw) != refusal.get(f"{prefix}_bytes")
                or sha256(raw) != refusal.get(f"{prefix}_sha256")):
            raise ValueError(f"oversized-source refusal {prefix} stream is absent or changed")
    envelope = json.loads(Path(refusal["invoke_stdout_path"]).read_bytes())
    if (envelope.get("status") != "error" or envelope.get("code") != "provider-execution-failed"
            or envelope.get("message") != message):
        raise ValueError("oversized-source refusal stdout differs from the asserted public provider error")
    roster_path = Path(refusal.get("roster_path", ""))
    roster_raw = roster_path.read_bytes() if roster_path.is_file() else None
    if (roster_raw is None or len(roster_raw) != refusal.get("roster_bytes")
            or sha256(roster_raw) != refusal.get("roster_sha256")):
        raise ValueError("oversized-source refusal current roster is absent or changed")
    roster = json.loads(roster_raw)
    if not isinstance(roster, list) or len(roster) != 1 or roster[0].get("token_budget") != SCRIPTED_TOKEN_BUDGET:
        raise ValueError("oversized-source refusal current roster omits the required fixture token budget")
    profile_path = Path(refusal.get("profile_path", ""))
    profile_raw = profile_path.read_bytes() if profile_path.is_file() else None
    if (profile_raw is None or len(profile_raw) != refusal.get("profile_bytes")
            or sha256(profile_raw) != refusal.get("profile_sha256")):
        raise ValueError("oversized-source refusal setup profile is absent or changed")
    profile = json.loads(profile_raw)
    if profile_signature(profile) != refusal.get("profile_signature") or profile_review_output_contract(profile).get("version") != "current-v2":
        raise ValueError("oversized-source refusal profile does not match its preserved current setup output")


def validate_collection(report: Any, *, check_streams: bool) -> None:
    if not isinstance(report, dict) or report.get("schema_version") != RECEIPT_SCHEMA_VERSION or report.get("receipt_type") != "dogfood-measurement":
        raise ValueError("measurement receipt schema is unsupported")
    if report.get("phase") not in ("baseline", "current") or report.get("complete") is not True:
        raise ValueError("measurement receipt is not a complete baseline/current collection")
    if report.get("dataset_version") != DATASET_VERSION:
        raise ValueError("measurement receipt uses an obsolete or unsupported dogfood fixture version")
    if report.get("errors"):
        raise ValueError("measurement receipt contains recorded errors")
    for key in ("dataset_sha256", "seed"):
        if report.get(key) is None:
            raise ValueError(f"measurement receipt omits {key}")
    execution = report.get("execution", {}).get("repository", {})
    execution_after = report.get("execution", {}).get("repository_after")
    if not all(isinstance(execution.get(key), str) and execution[key] for key in ("cwd", "head", "fingerprint")):
        raise ValueError("measurement receipt lacks actual execution-tree identity")
    if execution_after != execution:
        raise ValueError("measurement receipt does not prove a stable execution-tree identity before/after collection")
    collector = report.get("collector")
    if not isinstance(collector, dict) or not isinstance(collector.get("python"), str) or not collector["python"] or not isinstance(collector.get("argv"), list):
        raise ValueError("measurement receipt lacks the actual collector argv/Python identity")
    binaries = report.get("binaries")
    if not isinstance(binaries, dict) or not all(isinstance(binaries.get(key), dict) and binaries[key].get("sha256") for key in ("engine", "provider")):
        raise ValueError("measurement receipt lacks engine/provider binary identities")
    validate_oversized_mandatory_subject_refusal(report, check_streams=check_streams)
    phase = report["phase"]
    expected_profile_version = BASELINE_CONFIG_VERSION if phase == "baseline" else CURRENT_CONFIG_VERSION
    expected_output_contract = "legacy-v1" if phase == "baseline" else "current-v2"
    expected_setup_compatibility = {
        "roster_contract": "legacy-without-token-budget" if phase == "baseline" else "current-with-token-budget",
        "advice_contract": "legacy-option-unavailable" if phase == "baseline" else "current-explicit-decline",
    }
    setup_compatibility = report.get("setup_compatibility")
    if not isinstance(setup_compatibility, dict) or any(setup_compatibility.get(key) != value for key, value in expected_setup_compatibility.items()):
        raise ValueError("measurement receipt does not retain the actual phase-specific setup compatibility path")
    if phase == "baseline":
        if setup_compatibility.get("scripted_token_budget") is not None or setup_compatibility.get("advice_configuration") is not None:
            raise ValueError("legacy setup receipt claims unsupported token-budget or advice configuration")
    else:
        if setup_compatibility.get("scripted_token_budget") != SCRIPTED_TOKEN_BUDGET:
            raise ValueError("current setup receipt omits its exact scripted token-budget input")
        advice = setup_compatibility.get("advice_configuration")
        if not isinstance(advice, dict) or advice.get("decision") != "decline" or advice.get("enabled") is not False:
            raise ValueError("current setup receipt does not prove advice was explicitly declined")
    receipt_output_contract = report.get("review_output_contract")
    if not isinstance(receipt_output_contract, dict) or receipt_output_contract.get("version") != expected_output_contract:
        raise ValueError("measurement receipt lacks its actual phase-specific review output contract")
    if receipt_output_contract.get("schema_sha256") != sha256(canonical_bytes(receipt_output_contract.get("schema"))):
        raise ValueError("measurement receipt review output contract digest does not match its schema")
    if review_output_contract_version(receipt_output_contract.get("schema", {})) != expected_output_contract:
        raise ValueError("measurement receipt review output schema identity is not supported for its phase")
    for lane in READ_LANES:
        lane_data = report.get("read_lanes", {}).get(lane)
        if not isinstance(lane_data, dict):
            raise ValueError(f"measurement receipt is missing the {lane} lane")
        active = lane_data.get("active_work")
        expected_count = 0 if lane == "idle" else (1 if lane == "serial" else PARALLEL_ACTIVE_RUNS)
        if not isinstance(active, dict) or active.get("expected_processes") != expected_count:
            raise ValueError(f"measurement receipt lacks controlled {lane} active-work evidence")
        start_ids = active.get("observed_invocation_ids_before")
        end_ids = active.get("observed_invocation_ids_after")
        if lane == "idle":
            if start_ids != [] or end_ids != [] or active.get("observed_running_before_samples") is not False or active.get("observed_running_after_samples") is not False:
                raise ValueError("idle lane falsely claims active scripted work")
        elif (not isinstance(start_ids, list) or len(start_ids) != expected_count
              or not isinstance(end_ids, list) or len(end_ids) != expected_count
              or active.get("observed_running_before_samples") is not True
              or active.get("observed_running_after_samples") is not True
              or start_ids != end_ids):
            raise ValueError(f"{lane} lane lacks matching public running-state observations around every sample")
        for surface in READ_SURFACES:
            samples = lane_data.get(surface)
            if not isinstance(samples, list) or len(samples) != READ_SAMPLES:
                raise ValueError(f"measurement receipt is missing {lane}/{surface} samples")
            for sample in samples:
                if not isinstance(sample, dict) or sample.get("lane") != lane or sample.get("surface") != surface:
                    raise ValueError(f"measurement receipt has a misidentified {lane}/{surface} sample")
                validate_sample_stream(sample, check_streams=check_streams)
                if sample.get("active_work", {}).get("expected_processes") != expected_count or sample.get("active_work", {}).get("observed_processes") != expected_count:
                    raise ValueError(f"measurement receipt lacks observed controlled work for {lane}/{surface}")
            recorded_distribution = report.get("sample_distributions", {}).get(lane, {}).get(surface)
            if recorded_distribution != sample_distributions(samples):
                raise ValueError(f"measurement receipt has a false or missing distribution for {lane}/{surface}")
    reviews = report.get("review_delivery")
    for size in REVIEW_SIZES:
        lane = reviews.get(size) if isinstance(reviews, dict) else None
        if not isinstance(lane, dict) or not isinstance(lane.get("profile"), dict):
            raise ValueError(f"measurement receipt is missing the {size} review profile identity")
        signature = lane["profile"]
        if signature.get("contract_version") != 3 or signature.get("config_version") != expected_profile_version:
            raise ValueError(f"{size} review profile identity differs from the preserved phase-specific version")
        policies = signature.get("intent_review")
        if not isinstance(policies, list) or len(policies) != 7 or any(
                not isinstance(policy, dict) or not all(isinstance(policy.get(field), str) and policy[field]
                                                        for field in ("id", "description", "example_prompt", "review_stage"))
                or policy.get("review_stage") != "aggregate" or policy.get("required_authors") != 1
                for policy in policies):
            raise ValueError(f"{size} review profile does not define the complete named seven-axis workload")
        output_contract = lane.get("output_contract")
        if output_contract != receipt_output_contract or not isinstance(output_contract, dict):
            raise ValueError(f"{size} review lane output contract differs from its setup profile")
        samples = lane.get("samples")
        if not isinstance(samples, list) or len(samples) != REVIEW_SAMPLES:
            raise ValueError(f"measurement receipt is missing {size} review-delivered stdin samples")
        for sample in samples:
            if not isinstance(sample, dict) or sample.get("size_class") != size or sample.get("complete") is not True:
                raise ValueError(f"measurement receipt contains an incomplete {size} review sample")
            if not isinstance(sample.get("assertions"), list) or "public-outcome-asserted" not in sample["assertions"]:
                raise ValueError(f"{size} review sample has no asserted public outcome")
            if sample.get("invocation_returncode") != 0 or not sample.get("stdin_path") or not sample.get("stdin_sha256"):
                raise ValueError(f"{size} review sample is missing actual delivered stdin or successful invocation")
            if sample.get("profile_signature") != signature:
                raise ValueError(f"{size} review sample profile signature is inconsistent")
            if sample.get("output_contract_version") != expected_output_contract:
                raise ValueError(f"{size} review sample output contract identity is inconsistent")
            expected_assignment = {
                "review_stage": "aggregate", "author": REVIEW_AUTHOR, "policies": policies,
            }
            if sample.get("review_assignment") != expected_assignment:
                raise ValueError(f"{size} review sample does not retain its complete named policy assignment")
            mature_history = sample.get("mature_irrelevant_history")
            if size == "small":
                if mature_history is not None:
                    raise ValueError("small review sample unexpectedly contains mature irrelevant history")
            elif (not isinstance(mature_history, dict)
                  or mature_history.get("record_id") != f"dogfood-history-{report['seed']}-{sample.get('sample_id')}"
                  or mature_history.get("kind") != MATURE_HISTORY_KIND
                  or mature_history.get("payload_bytes") != MATURE_HISTORY_BYTES
                  or mature_history.get("payload_sha256") != sha256(mature_history_payload(report["seed"]).encode("utf-8"))
                  or not isinstance(mature_history.get("record_data_bytes"), int)
                  or not str(mature_history.get("record_data_sha256", "")).startswith("sha256:")
                  or mature_history.get("append_returncode") != 0
                  or mature_history.get("history_returncode") != 0
                  or "exact-65536-byte-history-payload" not in mature_history.get("assertions", [])):
                raise ValueError("mature review sample lacks the exact public 65,536-byte irrelevant-history record")
            budget_line = sample.get("token_budget_line")
            if phase == "baseline":
                if budget_line is not None:
                    raise ValueError("legacy review assignment unexpectedly claims a token-budget line")
            else:
                expected_budget_line = (
                    f"PER-CALL TOKEN WINDOW: model_id={SCRIPTED_TOKEN_BUDGET['model_id']} "
                    f"window={SCRIPTED_TOKEN_BUDGET['context_window_tokens']} "
                    f"system_reserve={SCRIPTED_TOKEN_BUDGET['system_tokens']} "
                    f"framing_reserve={SCRIPTED_TOKEN_BUDGET['framing_tokens']} "
                    f"output_reserve={SCRIPTED_TOKEN_BUDGET['output_reserve_tokens']} "
                    f"reasoning_reserve={SCRIPTED_TOKEN_BUDGET['reasoning_reserve_tokens']}. "
                    "This exact model and reserve is frozen; do not substitute or split duties."
                )
                if budget_line != expected_budget_line:
                    raise ValueError("current review assignment does not retain its exact required token-budget framing")
            if not isinstance(sample.get("delivered_bytes"), int) or sample["delivered_bytes"] <= 0:
                raise ValueError(f"{size} review sample has no delivery size")
            latency = sample.get("process_to_last_byte_ms")
            started_ns = sample.get("invoke_started_monotonic_ns")
            received_ns = sample.get("worker_stdin_received_monotonic_ns")
            invoke_latency = sample.get("invoke_receipt_process_to_last_byte_ms")
            if (not isinstance(latency, (int, float)) or not math.isfinite(float(latency)) or latency <= 0
                    or not isinstance(started_ns, int) or not isinstance(received_ns, int) or received_ns <= started_ns
                    or not math.isclose(float(latency), (received_ns - started_ns) / 1_000_000, rel_tol=0, abs_tol=0.001)
                    or not isinstance(invoke_latency, (int, float)) or not math.isfinite(float(invoke_latency)) or invoke_latency <= 0):
                raise ValueError(f"{size} review sample has invalid public-invoke-to-worker-delivery timing")
            if not isinstance(sample.get("worker_argv"), list) or not sample["worker_argv"] or not str(sample.get("worker_executable_sha256", "")).startswith("sha256:"):
                raise ValueError(f"{size} review sample lacks actual scripted worker argv/binary identity")
            roster_path = Path(sample.get("roster_path", ""))
            if not isinstance(sample.get("roster_bytes"), int) or not str(sample.get("roster_sha256", "")).startswith("sha256:"):
                raise ValueError(f"{size} review sample lacks its exact setup roster identity")
            if check_streams:
                if not roster_path.is_file() or len(roster_path.read_bytes()) != sample["roster_bytes"] or sha256(roster_path.read_bytes()) != sample["roster_sha256"]:
                    raise ValueError(f"{size} review setup roster is absent or changed")
                roster = json.loads(roster_path.read_bytes())
                if not isinstance(roster, list) or len(roster) != 1 or roster[0].get("author") != REVIEW_AUTHOR:
                    raise ValueError(f"{size} review setup roster changed its scripted author")
                if phase == "baseline":
                    if "token_budget" in roster[0]:
                        raise ValueError("legacy setup roster contains unsupported token_budget")
                elif roster[0].get("token_budget") != SCRIPTED_TOKEN_BUDGET:
                    raise ValueError("current setup roster omitted its exact scripted token_budget")
                for path_key, bytes_key, digest_key in (("profile_path", "profile_bytes", "profile_sha256"),
                                                        ("subject_path", "subject_bytes", "subject_sha256")):
                    input_path = Path(sample.get(path_key, ""))
                    if not input_path.is_file():
                        raise ValueError(f"{size} review input fixture is absent: {input_path}")
                    input_raw = input_path.read_bytes()
                    if len(input_raw) != sample.get(bytes_key) or sha256(input_raw) != sample.get(digest_key):
                        raise ValueError(f"{size} review input fixture differs from its receipt: {input_path}")
                    if path_key == "profile_path":
                        try:
                            profile_document = json.loads(input_raw)
                        except json.JSONDecodeError as error:
                            raise ValueError(f"{size} captured setup profile is invalid JSON: {error}") from error
                        if profile_signature(profile_document) != signature:
                            raise ValueError(f"{size} profile identity does not match its actual captured setup profile")
                        if profile_review_output_contract(profile_document) != output_contract:
                            raise ValueError(f"{size} output contract does not match its actual captured setup profile")
                stdin_path = Path(sample["stdin_path"])
                if not stdin_path.is_file():
                    raise ValueError(f"{size} review-delivered stdin stream is absent: {stdin_path}")
                raw = stdin_path.read_bytes()
                if len(raw) != sample["delivered_bytes"] or sha256(raw) != sample["stdin_sha256"]:
                    raise ValueError(f"{size} review-delivered stdin differs from its receipt")
                if mature_history is not None:
                    record_id_bytes = mature_history["record_id"].encode("utf-8")
                    payload_bytes = mature_history_payload(report["seed"]).encode("utf-8")
                    if record_id_bytes in raw or payload_bytes in raw:
                        raise ValueError("mature irrelevant history leaked into actual review worker stdin")
                    record_path = Path(mature_history.get("record_data_path", ""))
                    if not record_path.is_file():
                        raise ValueError("mature irrelevant-history input file is absent")
                    record_raw = record_path.read_bytes()
                    if (len(record_raw) != mature_history.get("record_data_bytes")
                            or sha256(record_raw) != mature_history.get("record_data_sha256")):
                        raise ValueError("mature irrelevant-history input differs from its captured identity")
                    record_document = json.loads(record_raw)
                    if record_document != {"payload": payload_bytes.decode("utf-8")}:
                        raise ValueError("mature irrelevant-history input does not contain exactly the approved payload")
                    for prefix in ("append_stdout", "append_stderr", "history_stdout", "history_stderr"):
                        path = Path(mature_history.get(f"{prefix}_path", ""))
                        raw_stream = path.read_bytes() if path.is_file() else None
                        if (raw_stream is None or len(raw_stream) != mature_history.get(f"{prefix}_bytes")
                                or sha256(raw_stream) != mature_history.get(f"{prefix}_sha256")):
                            raise ValueError(f"mature public-history {prefix} stream is absent or changed")
                    append_envelope = json.loads(Path(mature_history["append_stdout_path"]).read_bytes())
                    history_envelope = json.loads(Path(mature_history["history_stdout_path"]).read_bytes())
                    result = history_envelope.get("result")
                    history_items = result if isinstance(result, list) else result.get("items", []) if isinstance(result, dict) else []
                    observed_record = [
                        row for row in history_items if isinstance(row, dict)
                        and row.get("action", {}).get("kind") == "context_appended"
                        and row.get("action", {}).get("context_record_id") == mature_history.get("record_id")
                    ]
                    if (append_envelope.get("status") != "completed" or history_envelope.get("status") != "completed"
                            or len(observed_record) != 1 or observed_record[0] != mature_history.get("history_event")):
                        raise ValueError("public history does not confirm the exact mature irrelevant-history append")
                try:
                    actual_assignment, actual_budget_line = parse_review_assignment(raw)
                except ValueError as error:
                    raise ValueError(f"{size} review stdin has invalid actual assignment fields: {error}") from error
                if actual_assignment != sample["review_assignment"] or actual_budget_line != sample.get("token_budget_line"):
                    raise ValueError(f"{size} review stdin differs from its recorded assignment metadata")
                worker_output_path = Path(sample.get("worker_output_path", ""))
                if not worker_output_path.is_file():
                    raise ValueError(f"{size} review worker output is absent: {worker_output_path}")
                worker_output = worker_output_path.read_bytes()
                if len(worker_output) != sample.get("worker_output_bytes") or sha256(worker_output) != sample.get("worker_output_sha256"):
                    raise ValueError(f"{size} review worker output differs from its receipt")
                validate_scripted_review_output(worker_output, signature, expected_output_contract, sample["subject_sha256"])
                for prefix in ("invocation_stdout", "invocation_stderr"):
                    stream_path = Path(sample.get(f"{prefix}_path", ""))
                    if not stream_path.is_file():
                        raise ValueError(f"{size} review invocation stream is absent: {stream_path}")
                    stream_raw = stream_path.read_bytes()
                    if len(stream_raw) != sample.get(f"{prefix}_bytes") or sha256(stream_raw) != sample.get(f"{prefix}_sha256"):
                        raise ValueError(f"{size} review invocation stream differs from its receipt")
                try:
                    invoke_envelope = json.loads(Path(sample["invocation_stdout_path"]).read_bytes())
                except (OSError, json.JSONDecodeError) as error:
                    raise ValueError(f"{size} review invocation did not retain valid public output: {error}") from error
                if invoke_envelope.get("status") != "completed" or invoke_envelope.get("result", {}).get("invocation_id") != sample.get("invocation_id"):
                    raise ValueError(f"{size} review invocation output does not assert its selected invocation")
        if lane.get("distribution") != sample_distributions(samples):
            raise ValueError(f"measurement receipt has a false or missing distribution for {size} review delivery")
    small_samples = reviews["small"]["samples"]
    mature_samples = reviews["mature"]["samples"]
    for small_sample, mature_sample in zip(small_samples, mature_samples):
        if (small_sample.get("subject_sha256") != mature_sample.get("subject_sha256")
                or small_sample.get("subject_bytes") != mature_sample.get("subject_bytes")):
            raise ValueError("small and mature v2 lanes must deliver the same mandatory review subject")
    graph = report.get("graph", {})
    samples = graph.get("samples") if isinstance(graph, dict) else None
    if graph.get("status") != "complete" or not isinstance(samples, list) or len(samples) != 1:
        raise ValueError("measurement receipt is missing the shared 14-worker scripted graph lane")
    graph_sample = samples[0]
    expected_worker_ids = [f"worker-{worker_id:02d}" for worker_id in range(GRAPH_WORKERS)]
    if graph_sample.get("workers_expected") != GRAPH_WORKERS or graph_sample.get("workers_observed") != GRAPH_WORKERS or graph_sample.get("worker_ids") != expected_worker_ids or "14-worker-inventory-complete" not in graph_sample.get("assertions", []):
        raise ValueError("14-worker graph sample lacks complete asserted worker inventory")
    validate_sample_stream(graph_sample, check_streams=check_streams)
    if not isinstance(graph_sample.get("artifacts"), list) or not graph_sample["artifacts"]:
        raise ValueError("14-worker graph sample omitted retained worker capture artifacts")
    if check_streams:
        graph_summary = json.loads(Path(graph_sample["stdout_path"]).read_bytes())
        workers = graph_summary.get("workers") if isinstance(graph_summary, dict) else None
        observed_ids = {
            value
            for row in workers or []
            for value in row.get("args", [])
            if isinstance(value, str) and value.startswith("worker-")
        }
        if not isinstance(workers, list) or len(workers) != GRAPH_WORKERS or any(row.get("exit_code") != 0 for row in workers) or sorted(observed_ids) != expected_worker_ids:
            raise ValueError("retained public graph stdout does not prove the 14 unique successful workers")
        instructions_path = Path(graph_sample.get("instructions_path", ""))
        if not instructions_path.is_file() or sha256(instructions_path.read_bytes()) != graph_sample.get("instructions_sha256"):
            raise ValueError("14-worker graph instruction fixture is absent or changed")
        for artifact in graph_sample["artifacts"]:
            path = Path(artifact.get("path", ""))
            if not path.is_file() or path.stat().st_size != artifact.get("bytes") or file_sha(path) != artifact.get("sha256"):
                raise ValueError(f"14-worker graph capture artifact is absent or changed: {path}")
    if not graph.get("dagu") or not str(graph.get("dagu_sha256", "")).startswith("sha256:"):
        raise ValueError("14-worker graph receipt lacks the actual Dagu executable identity")
    if graph.get("elapsed_time_role") != "descriptive-only; no graph throughput acceptance threshold":
        raise ValueError("graph elapsed time must remain descriptive, not an AC-2/4 threshold")


def compare_receipts(baseline: Any, current: Any, *, check_streams: bool = True) -> dict[str, Any]:
    if baseline is None or current is None:
        raise ValueError("both baseline and current receipts are required")
    validate_collection(baseline, check_streams=check_streams)
    validate_collection(current, check_streams=check_streams)
    if baseline["phase"] != "baseline" or current["phase"] != "current":
        raise ValueError("compare requires one baseline and one current receipt")
    if baseline["dataset_sha256"] != current["dataset_sha256"] or baseline["seed"] != current["seed"]:
        raise ValueError("baseline/current dataset identity is incomparable")
    if baseline["execution"]["repository"] != current["execution"]["repository"] or baseline["execution"]["repository_after"] != current["execution"]["repository_after"]:
        raise ValueError("baseline/current execution repository identity differs")
    for fixture_name in ("intent_fixture_sha256", "generic_provider_sha256", "bound_worker_sha256", "review_worker_sha256", "graph_worker_sha256"):
        if baseline.get("fixtures", {}).get(fixture_name) != current.get("fixtures", {}).get(fixture_name):
            raise ValueError(f"baseline/current fixture identity differs: {fixture_name}")
    if baseline["collector"].get("python") != current["collector"].get("python"):
        raise ValueError("baseline/current scripted worker Python runtime differs")
    if baseline["graph"].get("dagu_sha256") != current["graph"].get("dagu_sha256"):
        raise ValueError("baseline/current scripted graph Dagu binary differs")
    baseline_profile = baseline["review_delivery"]["small"]["profile"]
    current_profile = current["review_delivery"]["small"]["profile"]
    if baseline_profile["config_version"] != BASELINE_CONFIG_VERSION or current_profile["config_version"] != CURRENT_CONFIG_VERSION:
        raise ValueError("baseline/current review profile identities are not the preserved minimal-11/minimal-12 pair")
    if baseline_profile["contract_version"] != current_profile["contract_version"]:
        raise ValueError("baseline/current named review contract versions differ")
    if baseline_profile["intent_review"] != current_profile["intent_review"]:
        raise ValueError("baseline/current complete named intent-review policies are not equivalent")
    if (baseline_profile["intent_review"] != baseline["review_delivery"]["mature"]["profile"]["intent_review"]
            or current_profile["intent_review"] != current["review_delivery"]["mature"]["profile"]["intent_review"]):
        raise ValueError("small/mature review sizes do not use one equivalent named intent-review workload")
    baseline_output_contract = baseline["review_output_contract"]
    current_output_contract = current["review_output_contract"]
    if baseline_output_contract["version"] != "legacy-v1" or current_output_contract["version"] != "current-v2":
        raise ValueError("baseline/current scripted review output contract pair is unsupported")
    if baseline["setup_compatibility"]["advice_contract"] != "legacy-option-unavailable" or current["setup_compatibility"]["advice_contract"] != "current-explicit-decline":
        raise ValueError("baseline/current advice setup path is not the preserved legacy/explicit-decline pair")
    mature_history_equivalence = []
    for old_sample, new_sample in zip(baseline["review_delivery"]["mature"]["samples"],
                                      current["review_delivery"]["mature"]["samples"]):
        old_history = old_sample.get("mature_irrelevant_history")
        new_history = new_sample.get("mature_irrelevant_history")
        if not isinstance(old_history, dict) or not isinstance(new_history, dict):
            raise ValueError("baseline/current mature workload lacks the approved irrelevant-history record")
        keys = ("record_id", "kind", "payload_bytes", "payload_sha256", "record_data_bytes", "record_data_sha256")
        if any(old_history.get(key) != new_history.get(key) for key in keys):
            raise ValueError("baseline/current mature irrelevant-history bytes are not identical")
        mature_history_equivalence.append({key: old_history[key] for key in keys})
    binary_changes = [name for name in ("engine", "provider") if baseline["binaries"][name]["sha256"] != current["binaries"][name]["sha256"]]
    if not binary_changes:
        raise ValueError("baseline and current receipts use identical engine/provider binaries; no changed-source comparison is evidenced")

    comparisons: dict[str, Any] = {}
    improved_surfaces: set[str] = set()
    current_status_latencies = []
    for lane in READ_LANES:
        comparisons[lane] = {}
        for surface in READ_SURFACES:
            before = baseline["read_lanes"][lane][surface]
            after = current["read_lanes"][lane][surface]
            if len(before) != len(after):
                raise ValueError(f"incomparable sample counts for {lane}/{surface}")
            for old_sample, new_sample in zip(before, after):
                if old_sample.get("sample_id") != new_sample.get("sample_id") or old_sample.get("run_id") != new_sample.get("run_id"):
                    raise ValueError(f"incomparable workload/sample identity for {lane}/{surface}")
            before_stats = sample_distributions(before)
            after_stats = sample_distributions(after)
            if surface == "status":
                current_status_latencies.extend(float(row["process_to_last_byte_ms"]) for row in after)
            bytes_better = after_stats["delivered_bytes"]["median"] < before_stats["delivered_bytes"]["median"]
            latency_better = after_stats["process_to_last_byte_ms"]["median"] < before_stats["process_to_last_byte_ms"]["median"]
            improved = bytes_better and latency_better
            if improved:
                improved_surfaces.add(surface)
            comparisons[lane][surface] = {
                "baseline": before_stats,
                "current": after_stats,
                "bytes_lower": bytes_better,
                "latency_lower": latency_better,
                "both_lower": improved,
            }
    if max(current_status_latencies, default=math.inf) > 2000:
        raise ValueError("current useful status exceeded the 2000ms process-to-last-byte limit")
    # The graph is validated as a complete matched workload, but graph elapsed
    # time is never counted as a throughput result.
    review_comparisons = {}
    for size in REVIEW_SIZES:
        before = baseline["review_delivery"][size]["samples"]
        after = current["review_delivery"][size]["samples"]
        if len(before) != len(after):
            raise ValueError(f"incomparable {size} review-delivery sample counts")
        for old_sample, new_sample in zip(before, after):
            if old_sample.get("sample_id") != new_sample.get("sample_id") or old_sample.get("subject_sha256") != new_sample.get("subject_sha256"):
                raise ValueError(f"incomparable {size} review workload or subject input")
            if old_sample.get("worker_executable_sha256") != new_sample.get("worker_executable_sha256"):
                raise ValueError(f"incomparable {size} scripted review worker executable")
            if old_sample.get("review_assignment") != new_sample.get("review_assignment"):
                raise ValueError(f"incomparable {size} actual named review assignment")
        before_stats = sample_distributions(before)
        after_stats = sample_distributions(after)
        bytes_better = after_stats["delivered_bytes"]["median"] < before_stats["delivered_bytes"]["median"]
        latency_better = after_stats["process_to_last_byte_ms"]["median"] < before_stats["process_to_last_byte_ms"]["median"]
        if bytes_better and latency_better:
            improved_surfaces.add(f"review-delivery-{size}")
        review_comparisons[size] = {"baseline": before_stats, "current": after_stats,
                                    "bytes_lower": bytes_better, "latency_lower": latency_better,
                                    "both_lower": bytes_better and latency_better}
    if len(improved_surfaces) < MIN_IMPROVED_SURFACES:
        raise ValueError(
            f"only {len(improved_surfaces)} read surfaces improved in both median delivered bytes and latency; "
            f"at least {MIN_IMPROVED_SURFACES} distinct surfaces are required"
        )
    return {
        "schema_version": RECEIPT_SCHEMA_VERSION,
        "comparison_type": "dogfood-measurement-comparison",
        "complete": True,
        "semantic_review": "not performed",
        "dataset_version": baseline["dataset_version"],
        "dataset_sha256": baseline["dataset_sha256"],
        "seed": baseline["seed"],
        "execution_repository": baseline["execution"]["repository"],
        "binary_changes": binary_changes,
        "review_workload_equivalence": {
            "gate": "intent-review",
            "baseline_profile_identity": {
                "contract_version": baseline_profile["contract_version"],
                "config_version": baseline_profile["config_version"],
            },
            "current_profile_identity": {
                "contract_version": current_profile["contract_version"],
                "config_version": current_profile["config_version"],
            },
            "same_contract_version": True,
            "baseline_config_version_preserved": BASELINE_CONFIG_VERSION,
            "current_config_version_preserved": CURRENT_CONFIG_VERSION,
            "policy_count": len(baseline_profile["intent_review"]),
            "policy_definitions_sha256": sha256(canonical_bytes(baseline_profile["intent_review"])),
            "complete_policy_definitions_equal": True,
            "captured_assignments_equal": True,
            "small_and_mature_mandatory_subjects_equal": True,
            "mature_irrelevant_history_equal": mature_history_equivalence,
            "oversized_mandatory_subject_refusal": {
                "complete": current["oversized_mandatory_subject_refusal"]["complete"],
                "subject_bytes": current["oversized_mandatory_subject_refusal"]["subject_bytes"],
                "subject_sha256": current["oversized_mandatory_subject_refusal"]["subject_sha256_before"],
                "provider_error": current["oversized_mandatory_subject_refusal"]["provider_error"],
                "worker_launched": current["oversized_mandatory_subject_refusal"]["worker_launched"],
            },
            "review_output_contracts": {
                "baseline": {"version": baseline_output_contract["version"], "schema_sha256": baseline_output_contract["schema_sha256"]},
                "current": {"version": current_output_contract["version"], "schema_sha256": current_output_contract["schema_sha256"]},
                "bridge": "The scripted worker emits the exact v1 schema to the legacy binary and v2 contract_version/grounds fields to the current binary; both retain the same stage, author, axis order, pass results and empty findings, and each public invocation must complete under its own captured schema.",
            },
            "setup_compatibility": {
                "baseline": baseline["setup_compatibility"],
                "current": current["setup_compatibility"],
            },
        },
        "read_surfaces": comparisons,
        "review_delivery": review_comparisons,
        "observed_improvements": sorted(improved_surfaces),
        "current_status_max_ms": max(current_status_latencies),
        "status_limit_ms": 2000,
        "graph_elapsed_time": {
            "baseline_ms": baseline["graph"]["samples"][0]["elapsed_ms"],
            "current_ms": current["graph"]["samples"][0]["elapsed_ms"],
            "role": "descriptive-only; no throughput threshold",
        },
        "source_receipts": {
            "baseline_phase": baseline["phase"], "current_phase": current["phase"],
            "baseline_engine_sha256": baseline["binaries"]["engine"]["sha256"],
            "current_engine_sha256": current["binaries"]["engine"]["sha256"],
            "baseline_provider_sha256": baseline["binaries"]["provider"]["sha256"],
            "current_provider_sha256": current["binaries"]["provider"]["sha256"],
        },
    }


def command_seed(args: argparse.Namespace) -> None:
    output = ensure_external_output(args.output, "output")
    if args.seed < 0:
        raise ValueError("seed must be a nonnegative integer")
    fixture = Path(__file__).resolve().parents[1] / FIXTURE_PATH
    if not fixture.is_file():
        raise ValueError(f"repository fixture is missing: {fixture}")
    payload = dataset_payload(args.seed, fixture.read_bytes())
    document = {"schema_version": SCHEMA_VERSION, "dataset": payload, "dataset_sha256": sha256(canonical_bytes(payload))}
    validate_dataset_document(document)
    write_json(output, document)
    print(json.dumps({"status": "seeded", "dataset": str(output), "dataset_sha256": document["dataset_sha256"],
                      "seed": args.seed, "workloads": [*READ_LANES, *READ_SURFACES, *REVIEW_SIZES, "graph-14-workers"]}, sort_keys=True))


def command_collect(args: argparse.Namespace) -> int:
    dataset_path = require_absolute(args.dataset, "dataset")
    document = json.loads(dataset_path.read_bytes())
    dataset, dataset_sha = validate_dataset_document(document)
    collector: Collector | None = None
    try:
        collector = Collector(args, dataset, dataset_sha)
        collector.run()
        print(json.dumps({"status": "collected", "phase": args.phase, "output": str(collector.output),
                          "dataset_sha256": dataset_sha, "complete": True}, sort_keys=True))
        return 0
    except (CollectionFailure, OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as error:
        if collector is not None:
            collector.errors.append(str(error))
            collector.report["complete"] = False
            collector.report["workspace_retained"] = collector.workspace.exists()
            try:
                collector.write_report()
            except OSError:
                pass
            print(json.dumps({"status": "incomplete", "phase": args.phase, "output": str(collector.output),
                              "error": str(error), "errors": collector.errors}, sort_keys=True), file=sys.stderr)
        else:
            print(f"dogfood_measure collect failed before fixture creation: {error}", file=sys.stderr)
        return 1


def command_compare(args: argparse.Namespace) -> int:
    baseline_path = require_absolute(args.baseline, "baseline")
    current_path = require_absolute(args.current, "current")
    output = ensure_external_output(args.output, "output")
    comparison: dict[str, Any]
    try:
        baseline = json.loads(baseline_path.read_bytes())
        current = json.loads(current_path.read_bytes())
        comparison = compare_receipts(baseline, current, check_streams=True)
        comparison["inputs"] = {
            "baseline_path": str(baseline_path), "baseline_file_sha256": file_sha(baseline_path),
            "current_path": str(current_path), "current_file_sha256": file_sha(current_path),
        }
        comparison["output"] = str(output)
        write_json(output, comparison)
        print(json.dumps({"status": "compared", "output": str(output),
                          "observed_improvements": comparison["observed_improvements"],
                          "current_status_max_ms": comparison["current_status_max_ms"]}, sort_keys=True))
        return 0
    except (ValueError, OSError, KeyError, TypeError, json.JSONDecodeError) as error:
        comparison = {"schema_version": RECEIPT_SCHEMA_VERSION, "comparison_type": "dogfood-measurement-comparison",
                      "complete": False, "semantic_review": "not performed", "errors": [str(error)],
                      "baseline_path": str(baseline_path), "current_path": str(current_path)}
        write_json(output, comparison)
        print(json.dumps({"status": "incomparable", "output": str(output), "error": str(error)}, sort_keys=True), file=sys.stderr)
        return 1


def _fake_receipt(tmp: Path, phase: str, *, latency: float, byte_count: int) -> dict[str, Any]:
    dataset_sha = "sha256:" + "a" * 64
    stream_root = tmp / phase
    stream_root.mkdir()
    def streams(name: str, raw: bytes) -> dict[str, Any]:
        out = stream_root / f"{name}.out"
        err = stream_root / f"{name}.err"
        out.write_bytes(raw)
        err.write_bytes(b"")
        return {"stdout_path": str(out), "stdout_bytes": len(raw), "stdout_sha256": sha256(raw),
                "stderr_path": str(err), "stderr_bytes": 0, "stderr_sha256": sha256(b"")}
    lanes = {}
    for lane in READ_LANES:
        active_count = 0 if lane == "idle" else (1 if lane == "serial" else PARALLEL_ACTIVE_RUNS)
        observed_ids = [] if lane == "idle" else [f"inv-{lane}-{index}" for index in range(active_count)]
        lanes[lane] = {"active_work": {
            "expected_processes": active_count,
            "observed_invocation_ids_before": observed_ids,
            "observed_invocation_ids_after": observed_ids,
            "observed_running_before_samples": lane != "idle",
            "observed_running_after_samples": lane != "idle",
        }}
        for surface in READ_SURFACES:
            samples = []
            for index in range(READ_SAMPLES):
                if surface == "monitor":
                    envelope = {"source": "run:dogfood-idle", "event": "snapshot"}
                elif surface == "history":
                    envelope = {"status": "completed", "result": []}
                elif surface == "invocation-progress":
                    envelope = {"status": "completed", "result": {"invocation_id": f"inv-{lane}"}}
                else:
                    envelope = {"status": "completed", "result": {"current_state": "work", "current_state_instructions": "Inspect the selected fixture."}}
                raw = canonical_bytes(envelope).rstrip(b"\n")
                raw += b" " * max(0, byte_count - len(raw)) + b"\n"
                row = {"sample_id": f"{lane}-{surface}-{index}", "lane": lane, "surface": surface,
                       "run_id": "dogfood-idle", "invocation_id": f"inv-{lane}",
                       "complete": True, "returncode": None if surface == "monitor" else 0,
                       "intentional_stop": "fixed_observation_window" if surface == "monitor" else None,
                       "signal": 15 if surface == "monitor" else None,
                       "complete_jsonl_lines": 1 if surface == "monitor" else None,
                       "partial_final_line_bytes": 0 if surface == "monitor" else None,
                       "partial_final_line_sha256": None,
                       "delivered_bytes": len(raw), "process_to_last_byte_ms": latency,
                       "assertions": ["public-outcome-asserted"],
                       "active_work": {"expected_processes": active_count, "observed_processes": active_count}, **streams(f"{lane}-{surface}-{index}", raw)}
                samples.append(row)
            lanes[lane][surface] = samples
    reviews = {}
    profile_version = BASELINE_CONFIG_VERSION if phase == "baseline" else CURRENT_CONFIG_VERSION
    output_version = "legacy-v1" if phase == "baseline" else "current-v2"
    policies = [{"id": f"axis-{index}", "description": f"Named review obligation {index}.",
                 "example_prompt": f"Judge named review obligation {index}.", "review_stage": "aggregate",
                 "required_authors": 1} for index in range(7)]
    profile_sig = {"contract_version": 3, "config_version": profile_version, "intent_review": policies}
    subject_raw = b"scripted intent fixture"
    if phase == "baseline":
        output_schema = {
            "required": ["review_stage", "author", "judgments"],
            "properties": {"judgments": {"items": {"oneOf": [{"required": ["axis", "result", "findings"]}]}}},
        }
        setup_compatibility = {"roster_contract": "legacy-without-token-budget",
                               "advice_contract": "legacy-option-unavailable", "scripted_token_budget": None,
                               "advice_configuration": None}
    else:
        output_schema = {
            "required": ["review_contract_version", "review_stage", "author", "judgments"],
            "properties": {"review_contract_version": {"const": 2},
                           "judgments": {"items": {"oneOf": [{"required": ["axis", "result", "findings", "grounds"]}]}}},
        }
        setup_compatibility = {"roster_contract": "current-with-token-budget",
                               "advice_contract": "current-explicit-decline", "scripted_token_budget": SCRIPTED_TOKEN_BUDGET,
                               "advice_configuration": {"decision": "decline", "enabled": False}}
    output_contract = {"version": output_version, "schema_sha256": sha256(canonical_bytes(output_schema)),
                       "schema": output_schema}
    profile_document = {
        "contract_version": profile_sig["contract_version"],
        "config_version": profile_sig["config_version"],
        "review_policies": {"intent-review": policies},
        "work_slot_bindings": {"intent-review": {"args": ["fan-out", "--worker", json.dumps({"full_output_schema": output_schema})]}},
    }
    profile_raw = canonical_bytes(profile_document)
    reviews = {}
    for size in REVIEW_SIZES:
        samples = []
        for index in range(REVIEW_SAMPLES):
            assignment = {"review_stage": "aggregate", "author": REVIEW_AUTHOR, "policies": policies}
            packet = ("assigned_policies: " + json.dumps(policies, separators=(",", ":"))
                      + "\nreview_stage: aggregate\nrequired_author_claim: " + REVIEW_AUTHOR + "\n").encode()
            budget_line = None if phase == "baseline" else (
                f"PER-CALL TOKEN WINDOW: model_id={SCRIPTED_TOKEN_BUDGET['model_id']} "
                f"window={SCRIPTED_TOKEN_BUDGET['context_window_tokens']} "
                f"system_reserve={SCRIPTED_TOKEN_BUDGET['system_tokens']} "
                f"framing_reserve={SCRIPTED_TOKEN_BUDGET['framing_tokens']} "
                f"output_reserve={SCRIPTED_TOKEN_BUDGET['output_reserve_tokens']} "
                f"reasoning_reserve={SCRIPTED_TOKEN_BUDGET['reasoning_reserve_tokens']}. "
                "This exact model and reserve is frozen; do not substitute or split duties."
            )
            if budget_line:
                packet += (budget_line + "\n").encode()
            raw = packet + b"x" * max(0, byte_count - len(packet))
            raw += b"\n"
            stdin_path = stream_root / f"{size}-{index}.stdin"
            stdin_path.write_bytes(raw)
            profile_path = stream_root / f"{size}-{index}.profile.json"
            subject_path = stream_root / f"{size}-{index}.intent.json"
            profile_path.write_bytes(profile_raw)
            subject_path.write_bytes(subject_raw)
            mature_history = None
            if size == "mature":
                history_payload = mature_history_payload(145)
                history_payload_bytes = history_payload.encode("utf-8")
                history_record_id = f"dogfood-history-145-review-mature-{index:02d}"
                history_data_raw = canonical_bytes({"payload": history_payload})
                history_data_path = stream_root / f"{size}-{index}.irrelevant-history.json"
                history_data_path.write_bytes(history_data_raw)
                history_append_stdout = streams(f"{size}-{index}.history-append", canonical_bytes({"status": "completed"}))
                history_event = {"sequence": 1, "occurred_at": 1,
                                 "action": {"kind": "context_appended", "context_record_id": history_record_id}}
                history_stdout_raw = canonical_bytes({"status": "completed", "result": [history_event]})
                history_stdout = streams(f"{size}-{index}.history-show", history_stdout_raw)
                mature_history = {
                    "record_id": history_record_id, "kind": MATURE_HISTORY_KIND,
                    "payload_bytes": len(history_payload_bytes), "payload_sha256": sha256(history_payload_bytes),
                    "record_data_path": str(history_data_path), "record_data_bytes": len(history_data_raw),
                    "record_data_sha256": sha256(history_data_raw), "append_argv": ["loop-engine", "append", history_record_id],
                    "append_returncode": 0, "append_stdout_path": history_append_stdout["stdout_path"],
                    "append_stdout_bytes": history_append_stdout["stdout_bytes"], "append_stdout_sha256": history_append_stdout["stdout_sha256"],
                    "append_stderr_path": history_append_stdout["stderr_path"],
                    "append_stderr_bytes": history_append_stdout["stderr_bytes"],
                    "append_stderr_sha256": history_append_stdout["stderr_sha256"],
                    "history_argv": ["loop-engine", "history", f"{size}-{index}"],
                    "history_returncode": 0, "history_stdout_path": history_stdout["stdout_path"],
                    "history_stdout_bytes": history_stdout["stdout_bytes"], "history_stdout_sha256": history_stdout["stdout_sha256"],
                    "history_stderr_path": history_stdout["stderr_path"], "history_stderr_bytes": history_stdout["stderr_bytes"],
                    "history_stderr_sha256": history_stdout["stderr_sha256"], "history_event": history_event,
                    "assertions": ["public-append-completed", "public-history-event-observed", "exact-65536-byte-history-payload"],
                }
            worker_output = {"review_stage": "aggregate", "author": {"name": REVIEW_AUTHOR, "kind": "agent"},
                             "judgments": [{"axis": policy["id"], "result": "pass", "findings": ""} for policy in policies]}
            if phase == "current":
                worker_output["review_contract_version"] = 2
                for judgment in worker_output["judgments"]:
                    judgment["grounds"] = {
                        "reason": "Scripted measurement fixture only; no semantic review was performed.",
                        "evidence": [{"locator": "intent.json#/problem", "sha256": sha256(subject_raw)}],
                    }
            worker_output_raw = (json.dumps(worker_output, separators=(",", ":")) + "\n").encode()
            worker_output_path = stream_root / f"{size}-{index}.worker-output.json"
            worker_output_path.write_bytes(worker_output_raw)
            roster = [{"author": REVIEW_AUTHOR, "command": "/usr/bin/python3", "args": ["scripted-review-worker.py"]}]
            if phase == "current":
                roster[0]["token_budget"] = SCRIPTED_TOKEN_BUDGET
            roster_raw = canonical_bytes(roster)
            roster_path = stream_root / f"{size}-{index}.roster.json"
            roster_path.write_bytes(roster_raw)
            invoke_raw = canonical_bytes({"status": "completed", "result": {"invocation_id": f"{size}-{index}"}})
            invoke_streams = streams(f"invoke-{size}-{index}", invoke_raw)
            invoke_started_ns = 1_000_000_000 + index * 1_000_000
            worker_received_ns = invoke_started_ns + int(latency * 1_000_000)
            samples.append({"sample_id": f"review-{size}-{index:02d}", "size_class": size,
                           "complete": True, "invocation_returncode": 0,
                           "worker_argv": ["/usr/bin/python3", "scripted-review-worker.py"],
                           "worker_executable_sha256": "sha256:" + "7" * 64,
                           "invocation_id": f"{size}-{index}", "subject_sha256": sha256(subject_raw), "subject_bytes": len(subject_raw),
                           "delivered_bytes": len(raw), "process_to_last_byte_ms": latency,
                           "invoke_started_monotonic_ns": invoke_started_ns,
                           "worker_stdin_received_monotonic_ns": worker_received_ns,
                           "invoke_receipt_process_to_last_byte_ms": latency,
                           "stdin_path": str(stdin_path), "stdin_sha256": sha256(raw),
                           "profile_signature": profile_sig, "profile_path": str(profile_path), "profile_bytes": len(profile_raw), "profile_sha256": sha256(profile_raw),
                           "subject_path": str(subject_path), "subject_bytes": len(subject_raw), "subject_sha256": sha256(subject_raw),
                           "output_contract_version": output_version, "review_assignment": assignment,
                           "token_budget_line": budget_line, "mature_irrelevant_history": mature_history,
                           "roster_path": str(roster_path), "roster_bytes": len(roster_raw),
                           "roster_sha256": sha256(roster_raw), "worker_output_path": str(worker_output_path),
                           "worker_output_bytes": len(worker_output_raw), "worker_output_sha256": sha256(worker_output_raw),
                           "invocation_stdout_path": invoke_streams["stdout_path"], "invocation_stdout_bytes": invoke_streams["stdout_bytes"],
                           "invocation_stdout_sha256": invoke_streams["stdout_sha256"],
                           "invocation_stderr_path": invoke_streams["stderr_path"], "invocation_stderr_bytes": invoke_streams["stderr_bytes"],
                           "invocation_stderr_sha256": invoke_streams["stderr_sha256"],
                           "assertions": ["public-outcome-asserted"]})
        reviews[size] = {"profile": profile_sig, "output_contract": output_contract, "samples": samples}
    graph_raw = canonical_bytes({"workers": [
        {"command": "/usr/bin/python3", "args": ["scripted-graph-worker.py", f"worker-{worker_id:02d}", "0.02"], "exit_code": 0}
        for worker_id in range(GRAPH_WORKERS)
    ]})
    graph_instructions = stream_root / "graph-instructions.txt"
    graph_instructions.write_bytes(b"fixed graph instructions\\n")
    graph_artifact = stream_root / "graph-worker-output.json"
    graph_artifact.write_bytes(canonical_bytes({"worker": "fixture"}))
    graph_sample = {"sample_id": "graph", "complete": True, "returncode": 0, "elapsed_ms": 100.0,
                    "process_to_last_byte_ms": 100.0, "delivered_bytes": len(graph_raw),
                    "assertions": ["14-worker-inventory-complete", "public-outcome-asserted"],
                    "workers_expected": GRAPH_WORKERS, "workers_observed": GRAPH_WORKERS,
                    "worker_ids": [f"worker-{worker_id:02d}" for worker_id in range(GRAPH_WORKERS)],
                    "instructions_path": str(graph_instructions), "instructions_sha256": sha256(graph_instructions.read_bytes()),
                    "artifacts_directory": str(stream_root),
                    "artifacts": [{"path": str(graph_artifact), "bytes": graph_artifact.stat().st_size, "sha256": file_sha(graph_artifact)}],
                    **streams("graph", graph_raw)}
    distributions = {
        lane: {surface: sample_distributions(lanes[lane][surface]) for surface in READ_SURFACES}
        for lane in READ_LANES
    }
    for size in REVIEW_SIZES:
        reviews[size]["distribution"] = sample_distributions(reviews[size]["samples"])
    data_root = Path(__file__).resolve().parents[1]
    oversized_refusal = None
    if phase == "current":
        fixture_document = json.loads((data_root / FIXTURE_PATH).read_bytes())
        oversized_subject = canonical_bytes(oversized_mature_subject(fixture_document, 145))
        oversized_subject_path = stream_root / "oversized-mandatory-subject.intent.json"
        oversized_subject_path.write_bytes(oversized_subject)
        oversized_message = (
            "provider-exited-nonzero: provider process exit code 2; stderr: commission: mandatory review sources do not fit "
            f"the starting byte bounds (context=210, originals={len(oversized_subject)}, "
            f"supplied_total={210 + len(oversized_subject)}, initial_limit=32768, total_limit=262144); "
            "no source was truncated; inspect: intent.json"
        )
        oversized_stdout_raw = canonical_bytes({"operation": "invoke", "status": "error",
                                                "code": "provider-execution-failed", "message": oversized_message})
        oversized_invoke_streams = streams("oversized-negative-invoke", oversized_stdout_raw)
        oversized_setup_streams = streams("oversized-negative-setup", canonical_bytes({"status": "ready"}))
        current_sample = reviews["small"]["samples"][0]
        oversized_refusal = {
            "complete": True, "expected_refusal": True, "dataset_version": DATASET_VERSION,
            "dataset_sha256": "sha256:" + "a" * 64,
            "profile_identity": {"contract_version": 3, "config_version": CURRENT_CONFIG_VERSION},
            "output_contract_version": "current-v2", "profile_signature": profile_sig,
            "setup_argv": ["software-change", "setup", "--decline-advice"], "setup_returncode": 0,
            "setup_stdout_path": oversized_setup_streams["stdout_path"],
            "setup_stdout_bytes": oversized_setup_streams["stdout_bytes"],
            "setup_stdout_sha256": oversized_setup_streams["stdout_sha256"],
            "setup_stderr_path": oversized_setup_streams["stderr_path"],
            "setup_stderr_bytes": oversized_setup_streams["stderr_bytes"],
            "setup_stderr_sha256": oversized_setup_streams["stderr_sha256"],
            "setup_advice_configuration": setup_compatibility["advice_configuration"],
            "roster_path": current_sample["roster_path"], "roster_bytes": current_sample["roster_bytes"],
            "roster_sha256": current_sample["roster_sha256"],
            "scripted_token_budget": SCRIPTED_TOKEN_BUDGET,
            "profile_path": current_sample["profile_path"], "profile_bytes": current_sample["profile_bytes"],
            "profile_sha256": current_sample["profile_sha256"],
            "invoke_argv": ["loop-engine", "invoke", "oversized-mandatory-subject", "intent-review"],
            "invoke_returncode": 20, "invoke_stdout_path": oversized_invoke_streams["stdout_path"],
            "invoke_stdout_bytes": oversized_invoke_streams["stdout_bytes"],
            "invoke_stdout_sha256": oversized_invoke_streams["stdout_sha256"],
            "invoke_stderr_path": oversized_invoke_streams["stderr_path"],
            "invoke_stderr_bytes": oversized_invoke_streams["stderr_bytes"],
            "invoke_stderr_sha256": oversized_invoke_streams["stderr_sha256"],
            "provider_error": oversized_message, "subject_path": str(oversized_subject_path),
            "subject_bytes": len(oversized_subject), "subject_sha256_before": sha256(oversized_subject),
            "subject_sha256_after": sha256(oversized_subject), "subject_bytes_after": len(oversized_subject),
            "worker_paths": {"stdin": str(stream_root / "negative-worker.stdin"),
                             "meta": str(stream_root / "negative-worker.meta"),
                             "output": str(stream_root / "negative-worker.output")},
            "worker_launched": False,
            "assertions": ["public-start-intent-ready-invoke", "initial-32k-limit-enforced",
                           "oversized-mandatory-original-refused", "no-source-truncation-reported",
                           "subject-bytes-unchanged", "review-worker-not-launched"],
        }
    repository = {"cwd": "/tmp/repo", "head": "abc", "fingerprint": "sha256:" + "b" * 64}
    return {
        "schema_version": RECEIPT_SCHEMA_VERSION, "receipt_type": "dogfood-measurement", "phase": phase,
        "complete": True, "dataset_version": DATASET_VERSION, "dataset_sha256": dataset_sha,
        "seed": 145, "data_root": str(data_root), "errors": [],
        "execution": {"repository": repository, "repository_after": repository},
        "collector": {"python": "Python 3 test fixture", "argv": ["python3", "dogfood_measure.py"]},
        "sample_distributions": distributions,
        "binaries": {"engine": {"sha256": "sha256:" + ("c" if phase == "baseline" else "d") * 64},
                     "provider": {"sha256": "sha256:" + ("e" if phase == "baseline" else "f") * 64}},
        "fixtures": {name: "sha256:" + "1" * 64 for name in ("intent_fixture_sha256", "generic_provider_sha256", "bound_worker_sha256", "review_worker_sha256", "graph_worker_sha256")},
        "setup_compatibility": setup_compatibility, "review_output_contract": output_contract,
        "oversized_mandatory_subject_refusal": oversized_refusal,
        "read_lanes": lanes, "review_delivery": reviews,
        "graph": {"status": "complete", "dagu": "/usr/bin/dagu", "dagu_sha256": "sha256:" + "8" * 64,
                  "elapsed_time_role": "descriptive-only; no graph throughput acceptance threshold", "samples": [graph_sample]},
    }


def self_test() -> None:
    fixture = b"fixture"
    first = dataset_payload(145, fixture)
    second = dataset_payload(145, fixture)
    if canonical_bytes(first) != canonical_bytes(second):
        raise AssertionError("fixed seed did not produce deterministic dataset bytes")
    valid_dataset = {"schema_version": SCHEMA_VERSION, "dataset": first, "dataset_sha256": sha256(canonical_bytes(first))}
    validate_dataset_document(valid_dataset)
    if first.get("dataset_version") != "dogfood-v2" or first.get("mature_irrelevant_history_bytes") != MATURE_HISTORY_BYTES:
        raise AssertionError("new dataset version did not declare the exact irrelevant-history fixture")
    old_dataset = json.loads(json.dumps(valid_dataset))
    old_dataset["dataset"]["dataset_version"] = "dogfood-v1"
    old_dataset["dataset_sha256"] = sha256(canonical_bytes(old_dataset["dataset"]))
    try:
        validate_dataset_document(old_dataset)
    except ValueError as error:
        if "unsupported dogfood dataset version" not in str(error):
            raise AssertionError(f"old dataset version failed for the wrong reason: {error}") from error
    else:
        raise AssertionError("self-test accepted old dogfood-v1 evidence as the new fixture")
    mature_payload = mature_history_payload(145)
    if len(mature_payload.encode("utf-8")) != MATURE_HISTORY_BYTES:
        raise AssertionError("mature history payload did not contain exactly 65,536 bytes")
    damaged_dataset = json.loads(json.dumps(valid_dataset))
    damaged_dataset["dataset"]["records"].pop()
    damaged_dataset["dataset_sha256"] = sha256(canonical_bytes(damaged_dataset["dataset"]))
    try:
        validate_dataset_document(damaged_dataset)
    except ValueError:
        pass
    else:
        raise AssertionError("incomplete dataset was accepted")
    tampered_dataset = json.loads(json.dumps(valid_dataset))
    tampered_dataset["dataset"]["samples_per_read_surface"] = 1
    tampered_dataset["dataset_sha256"] = sha256(canonical_bytes(tampered_dataset["dataset"]))
    try:
        validate_dataset_document(tampered_dataset)
    except ValueError:
        pass
    else:
        raise AssertionError("tampered versioned workload parameters were accepted")

    with tempfile.TemporaryDirectory(prefix="dogfood-measure-self-test-") as temp:
        root = Path(temp)
        baseline = _fake_receipt(root, "baseline", latency=100.0, byte_count=1000)
        current = _fake_receipt(root, "current", latency=80.0, byte_count=800)
        result = compare_receipts(baseline, current)
        if not result["complete"] or len(result["observed_improvements"]) < MIN_IMPROVED_SURFACES:
            raise AssertionError("complete matched fixture did not produce a measured comparison")
        equivalence = result["review_workload_equivalence"]
        if (equivalence["baseline_profile_identity"]["config_version"] != BASELINE_CONFIG_VERSION
                or equivalence["current_profile_identity"]["config_version"] != CURRENT_CONFIG_VERSION
                or equivalence["policy_count"] != 7 or not equivalence["complete_policy_definitions_equal"]
                or equivalence["review_output_contracts"]["baseline"]["version"] != "legacy-v1"
                or equivalence["review_output_contracts"]["current"]["version"] != "current-v2"):
            raise AssertionError("matched comparison did not preserve both exact profiles and the output-contract bridge")
        refusal_without_truncation = json.loads(json.dumps(current))
        refusal_without_truncation["oversized_mandatory_subject_refusal"]["subject_sha256_after"] = "sha256:" + "9" * 64
        try:
            compare_receipts(baseline, refusal_without_truncation, check_streams=False)
        except ValueError as error:
            if "unchanged, unlaunched subject" not in str(error):
                raise AssertionError(f"truncated oversized subject failed for the wrong reason: {error}") from error
        else:
            raise AssertionError("self-test accepted a mutated oversized mandatory subject")

        changed_mature_subject = json.loads(json.dumps(current))
        changed_mature_subject["review_delivery"]["mature"]["samples"][0]["subject_sha256"] = "sha256:" + "9" * 64
        try:
            compare_receipts(baseline, changed_mature_subject, check_streams=False)
        except ValueError as error:
            if "same mandatory review subject" not in str(error):
                raise AssertionError(f"different small/mature subject failed for the wrong reason: {error}") from error
        else:
            raise AssertionError("self-test accepted a different mandatory subject in the mature lane")

        wrong_history_volume = json.loads(json.dumps(current))
        wrong_history_volume["review_delivery"]["mature"]["samples"][0]["mature_irrelevant_history"]["payload_bytes"] -= 1
        try:
            compare_receipts(baseline, wrong_history_volume, check_streams=False)
        except ValueError as error:
            if "exact public 65,536-byte irrelevant-history" not in str(error):
                raise AssertionError(f"wrong mature history volume failed for the wrong reason: {error}") from error
        else:
            raise AssertionError("self-test accepted a shortened mature history fixture")

        def rewrite_profile(report: dict[str, Any], *, config_version: str | None = None,
                            alter_policy: bool = False) -> dict[str, Any]:
            changed = json.loads(json.dumps(report))
            for size in REVIEW_SIZES:
                lane = changed["review_delivery"][size]
                signature = lane["profile"]
                if config_version is not None:
                    signature["config_version"] = config_version
                if alter_policy:
                    signature["intent_review"][0]["example_prompt"] += " Altered workload."
                lane["profile"] = signature
                for sample in lane["samples"]:
                    sample["profile_signature"] = signature
                    sample["review_assignment"]["policies"] = signature["intent_review"]
            return changed

        for label, left, right, expected in (
            ("wrong preserved profile identity", rewrite_profile(baseline, config_version="minimal-10"), current,
             "preserved phase-specific version"),
            ("same IDs but altered named prompt", baseline, rewrite_profile(current, alter_policy=True),
             "complete named intent-review policies"),
        ):
            try:
                compare_receipts(left, right, check_streams=False)
            except ValueError as error:
                if expected not in str(error):
                    raise AssertionError(f"{label} failed closed for the wrong reason: {error}") from error
            else:
                raise AssertionError(f"self-test accepted {label}")

        unknown_output_contract = json.loads(json.dumps(current))
        unknown_output_contract["review_output_contract"]["version"] = "legacy-v1"
        for size in REVIEW_SIZES:
            unknown_output_contract["review_delivery"][size]["output_contract"]["version"] = "legacy-v1"
        try:
            compare_receipts(baseline, unknown_output_contract, check_streams=False)
        except ValueError as error:
            if "phase-specific review output contract" not in str(error):
                raise AssertionError(f"unknown output contract failed for the wrong reason: {error}") from error
        else:
            raise AssertionError("self-test accepted an unknown current review output contract")

        partial_monitor = json.loads(json.dumps(current))
        monitor_sample = partial_monitor["read_lanes"]["idle"]["monitor"][0]
        monitor_raw = Path(monitor_sample["stdout_path"]).read_bytes()
        partial_tail = b'{"terminated_mid_snapshot":'
        monitor_raw += partial_tail
        partial_path = root / "bounded-monitor-partial.stdout"
        partial_path.write_bytes(monitor_raw)
        monitor_sample.update({
            "stdout_path": str(partial_path), "stdout_bytes": len(monitor_raw),
            "stdout_sha256": sha256(monitor_raw), "delivered_bytes": len(monitor_raw),
            "partial_final_line_bytes": len(partial_tail),
            "partial_final_line_sha256": sha256(partial_tail),
        })
        partial_monitor["sample_distributions"]["idle"]["monitor"] = sample_distributions(
            partial_monitor["read_lanes"]["idle"]["monitor"]
        )
        compare_receipts(baseline, partial_monitor)
        unbound_monitor_tail = json.loads(json.dumps(partial_monitor))
        unbound_monitor_tail["read_lanes"]["idle"]["monitor"][0]["partial_final_line_bytes"] += 1
        try:
            compare_receipts(baseline, unbound_monitor_tail)
        except ValueError as error:
            if "partial final line" not in str(error):
                raise AssertionError(f"unbound monitor tail failed for the wrong reason: {error}") from error
        else:
            raise AssertionError("self-test accepted unbound bytes after a monitor SIGTERM")

        leaked_history = json.loads(json.dumps(current))
        mature_sample = leaked_history["review_delivery"]["mature"]["samples"][0]
        leaked_packet = Path(mature_sample["stdin_path"]).read_bytes() + b"\\n" + mature_sample["mature_irrelevant_history"]["record_id"].encode()
        leaked_path = root / "irrelevant-history-leaked.stdin"
        leaked_path.write_bytes(leaked_packet)
        mature_sample.update({"stdin_path": str(leaked_path), "delivered_bytes": len(leaked_packet),
                              "stdin_sha256": sha256(leaked_packet)})
        leaked_history["review_delivery"]["mature"]["distribution"] = sample_distributions(
            leaked_history["review_delivery"]["mature"]["samples"]
        )
        try:
            compare_receipts(baseline, leaked_history)
        except ValueError as error:
            if "irrelevant history leaked" not in str(error):
                raise AssertionError(f"routed mature history failed for the wrong reason: {error}") from error
        else:
            raise AssertionError("self-test accepted irrelevant mature history routed to the reviewer")

        for label, mutate in (
            ("missing baseline", lambda b, c: (None, c)),
            ("missing serial lane", lambda b, c: (b, {**c, "read_lanes": {k: v for k, v in c["read_lanes"].items() if k != "serial"}})),
            ("missing parallel lane", lambda b, c: (b, {**c, "read_lanes": {k: v for k, v in c["read_lanes"].items() if k != "parallel"}})),
            ("missing read lane", lambda b, c: (b, {**c, "read_lanes": {**c["read_lanes"], "idle": {k: v for k, v in c["read_lanes"]["idle"].items() if k != "history"}}})),
            ("incomparable dataset identity", lambda b, c: (b, {**c, "dataset_sha256": "sha256:" + "9" * 64})),
            ("incomparable execution identity", lambda b, c: (b, {**c, "execution": {"repository": {**c["execution"]["repository"], "fingerprint": "sha256:" + "9" * 64}}})),
            ("incomparable fixture identity", lambda b, c: (b, {**c, "fixtures": {**c["fixtures"], "intent_fixture_sha256": "sha256:" + "9" * 64}})),
            ("incomparable review input", lambda b, c: (b, {**c, "review_delivery": {**c["review_delivery"], "small": {**c["review_delivery"]["small"], "samples": [{**c["review_delivery"]["small"]["samples"][0], "subject_sha256": "sha256:" + "9" * 64}, *c["review_delivery"]["small"]["samples"][1:]]}}})),
            ("exit-zero without assertions", lambda b, c: (b, {**c, "read_lanes": {**c["read_lanes"], "idle": {**c["read_lanes"]["idle"], "status": [{**c["read_lanes"]["idle"]["status"][0], "assertions": []}, *c["read_lanes"]["idle"]["status"][1:]]}}})),
        ):
            left, right = mutate(baseline, current)
            try:
                compare_receipts(left, right, check_streams=True)
            except (ValueError, TypeError, KeyError):
                pass
            else:
                raise AssertionError(f"self-test accepted {label}")

        missing_stream = json.loads(json.dumps(current))
        missing_stream["read_lanes"]["idle"]["status"][0]["stdout_path"] = str(root / "absent.stdout")
        try:
            compare_receipts(baseline, missing_stream)
        except ValueError:
            pass
        else:
            raise AssertionError("self-test accepted absent output streams")

        false_root = root / "false-speedup"
        false_root.mkdir()
        false_speedup = _fake_receipt(false_root, "current", latency=125.0, byte_count=1000)
        false_speedup["claimed_speedup"] = 999.0
        try:
            compare_receipts(baseline, false_speedup)
        except ValueError as error:
            if "improved" not in str(error):
                raise AssertionError(f"false speedup failed for the wrong reason: {error}") from error
        else:
            raise AssertionError("self-test trusted a misleading claimed speedup despite slower raw samples")
    print("dogfood measurement self-test passed: deterministic seed and fail-closed matched-sample comparison")


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    seed = sub.add_parser("seed", help="write one deterministic versioned fixture dataset")
    seed.add_argument("--seed", required=True, type=int)
    seed.add_argument("--output", required=True, type=Path)
    collect = sub.add_parser("collect", help="measure public user paths in an isolated external fixture")
    collect.add_argument("--phase", required=True, choices=("baseline", "current"))
    collect.add_argument("--engine", required=True, type=Path)
    collect.add_argument("--provider", required=True, type=Path)
    collect.add_argument("--data-root", required=True, type=Path)
    collect.add_argument("--dataset", required=True, type=Path)
    collect.add_argument("--output", required=True, type=Path)
    compare = sub.add_parser("compare", help="compare complete matched baseline/current receipts")
    compare.add_argument("--baseline", required=True, type=Path)
    compare.add_argument("--current", required=True, type=Path)
    compare.add_argument("--output", required=True, type=Path)
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    raw = list(sys.argv[1:] if argv is None else argv)
    if raw == ["--self-test"]:
        try:
            self_test()
            return 0
        except (AssertionError, ValueError, OSError, KeyError, TypeError) as error:
            print(f"dogfood measurement self-test failed: {error}", file=sys.stderr)
            return 1
    try:
        args = parse_args(raw)
        if args.command == "seed":
            command_seed(args)
            return 0
        if args.command == "collect":
            return command_collect(args)
        if args.command == "compare":
            return command_compare(args)
        raise AssertionError(args.command)
    except (ValueError, OSError, json.JSONDecodeError) as error:
        print(f"dogfood_measure failed: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
