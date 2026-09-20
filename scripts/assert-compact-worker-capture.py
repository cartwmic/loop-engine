#!/usr/bin/env python3
"""Read-only verification for the supported compact-worker journey receipts.

The producer is ``software-change-journey.py --compact-worker-fixture``.  This
checker never starts a worker, opens the engine catalog, or repairs a receipt.
It compares the actual delivered stdin with the independently retained full
routing snapshot and then checks raw output, attempts, selected-output linkage,
and the public invocation/show evidence.
"""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import sys
from pathlib import Path
from typing import Any, Mapping


class CaptureFailure(RuntimeError):
    pass


def load_json(path: Path, label: str) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise CaptureFailure(f"could not read {label} {path}: {error}") from error


def sha256(path: Path) -> str:
    try:
        return hashlib.sha256(path.read_bytes()).hexdigest()
    except OSError as error:
        raise CaptureFailure(f"could not hash {path}: {error}") from error


def require(condition: bool, message: str) -> None:
    if not condition:
        raise CaptureFailure(message)


def projected_context(records: Any) -> Any:
    if not isinstance(records, list):
        raise CaptureFailure("full routed context is not an array")
    result = copy.deepcopy(records)
    for record in result:
        if not isinstance(record, dict):
            raise CaptureFailure(f"routed context contains a non-object record: {record!r}")
        data = record.get("data")
        if isinstance(data, dict):
            data.pop("loop_engine_origin", None)
    return result


def parse_delivered_location(raw: bytes) -> tuple[dict[str, Any], bytes, int]:
    marker = b"---\n\n"
    if marker in raw:
        prefix, body = raw.split(marker, 1)
        require(not body, "bound compact worker stdin contains bytes after the separator")
        lines = prefix.splitlines()
        require(bool(lines), "bound compact worker stdin has no location line")
        location_bytes = lines[-1]
        line_start = prefix.rfind(b"\n") + 1
        envelope_prefix_length = line_start
        separator_length = len(marker)
    else:
        location_bytes = raw.rstrip(b"\n")
        envelope_prefix_length = 0
        separator_length = len(raw) - len(location_bytes)
    try:
        location = json.loads(location_bytes)
    except json.JSONDecodeError as error:
        raise CaptureFailure(f"delivered compact worker location is not JSON: {error}") from error
    require(isinstance(location, dict), "delivered compact worker location is not an object")
    return location, location_bytes, envelope_prefix_length + separator_length


def safe_capture_path(capture: Path, value: str) -> Path:
    path = Path(value)
    if path.is_absolute():
        resolved = path.resolve()
    else:
        resolved = (capture / path).resolve()
    try:
        resolved.relative_to(capture.resolve())
    except ValueError as error:
        raise CaptureFailure(f"capture path escapes capture root: {value}") from error
    return resolved


def worker_declarations(binding: Mapping[str, Any]) -> list[dict[str, Any]]:
    args = binding.get("args")
    require(isinstance(args, list) and args and args[0] == "fan-out", "binding is not the supported fan-out facade")
    require("--instructions" not in args and not any(str(arg).startswith("--instructions=") for arg in args), "binding uses ad-hoc --instructions")
    declarations: list[dict[str, Any]] = []
    index = 1
    while index < len(args):
        token = args[index]
        if token == "--max-active":
            require(index + 1 < len(args), "fan-out --max-active is missing its value")
            index += 2
            continue
        if token == "--then":
            index += 1
            continue
        require(token == "--worker" and index + 1 < len(args), f"malformed fan-out binding near {args[index:]!r}")
        try:
            worker = json.loads(args[index + 1])
        except json.JSONDecodeError as error:
            raise CaptureFailure(f"fan-out worker declaration is not JSON: {error}") from error
        require(isinstance(worker, dict), "fan-out worker declaration is not an object")
        declarations.append(worker)
        index += 2
    require(declarations, "fan-out binding has no workers")
    return declarations


def verify_attempts(capture: Path, row: Mapping[str, Any], index: int) -> None:
    attempts_path = row.get("attempts_path")
    if not attempts_path:
        return
    manifest_path = safe_capture_path(capture, str(attempts_path))
    manifest = load_json(manifest_path, "attempts manifest")
    require(manifest.get("schema_version") == "1", f"worker {index} attempts manifest has the wrong schema")
    attempts = manifest.get("attempts")
    require(isinstance(attempts, list) and attempts, f"worker {index} attempts manifest is empty")
    worker_dir = manifest_path.parent
    for position, attempt in enumerate(attempts, start=1):
        require(isinstance(attempt, dict) and attempt.get("number") == position, f"worker {index} attempts are not ordered")
        attempt_dir = worker_dir / "attempts" / str(position)
        stdout = attempt_dir / "stdout"
        stderr = attempt_dir / "stderr"
        require(stdout.is_file() and stderr.is_file(), f"worker {index} attempt {position} raw streams are missing")
        require("sha256:" + sha256(stdout) == attempt.get("stdout_sha256"), f"worker {index} attempt {position} stdout digest mismatch")
        require("sha256:" + sha256(stderr) == attempt.get("stderr_sha256"), f"worker {index} attempt {position} stderr digest mismatch")
    selected = manifest.get("selected_attempt")
    if row.get("selected_attempt") is not None:
        require(selected == row["selected_attempt"], f"worker {index} selected attempt disagrees with attempts manifest")


def verify_worker(
    *,
    capture: Path,
    worker_index: int,
    spec_worker: Mapping[str, Any],
    summary_worker: Mapping[str, Any],
    full_context: list[Any],
    artifact_root: str,
    positive: bool,
) -> dict[str, Any]:
    expected_id = f"worker-{worker_index}"
    require(spec_worker.get("assignment_id") == expected_id, f"spec assignment ordering changed at worker {worker_index}")
    require(summary_worker.get("assignment_id") == expected_id, f"summary assignment ordering changed at worker {worker_index}")
    require(spec_worker.get("routed_inputs") == full_context, f"spec lost full routed context at worker {worker_index}")
    require(summary_worker.get("routed_inputs") == full_context, f"summary lost full routed context at worker {worker_index}")

    stdin_path = Path(str(spec_worker.get("stdin_path")))
    require(stdin_path.is_file(), f"worker {worker_index} stdin capture is missing")
    raw = stdin_path.read_bytes()
    require(raw, f"worker {worker_index} delivered stdin is empty")
    location, location_bytes, envelope_prefix_length = parse_delivered_location(raw)
    require(location.get("artifact_root") == artifact_root, f"worker {worker_index} delivered the wrong artifact_root")
    require("run_id" not in location and "slot_id" not in location and "capture_dir" not in location and "instruction_body" not in location, f"worker {worker_index} received redundant engine envelope fields")
    require(location.get("context") == projected_context(full_context), f"worker {worker_index} context was not the narrow engine-origin projection")

    stdout_path = Path(str(summary_worker.get("stdout_path")))
    stderr_path = Path(str(summary_worker.get("stderr_path")))
    require(stdout_path.is_file() and stderr_path.is_file(), f"worker {worker_index} raw stdout/stderr is missing")
    stdout = stdout_path.read_bytes()
    stderr = stderr_path.read_bytes()
    if positive:
        require(stdout, f"worker {worker_index} produced empty output")
        selected_path = summary_worker.get("selected_output_path")
        selected_digest = summary_worker.get("selected_output_sha256")
        require(isinstance(selected_path, str) and isinstance(selected_digest, str), f"worker {worker_index} omitted selected-output linkage")
        selected = safe_capture_path(capture, selected_path)
        require(selected.is_file(), f"worker {worker_index} selected output is missing")
        require("sha256:" + sha256(selected) == selected_digest, f"worker {worker_index} selected-output digest mismatch")
        require(summary_worker.get("status") in (None, "succeeded"), f"worker {worker_index} output conformance failed")
    else:
        require(not stdout, f"negative worker {worker_index} unexpectedly produced stdout")
        require(summary_worker.get("status") == "failed", f"negative worker {worker_index} did not fail output conformance")
        require("context" in stderr.decode("utf-8", "replace").lower(), f"negative worker {worker_index} lost context-window rejection evidence")
    verify_attempts(capture, summary_worker, worker_index)
    full_location = json.dumps(
        {"artifact_root": artifact_root, "context": full_context},
        separators=(",", ":"),
        ensure_ascii=False,
    ).encode("utf-8")
    equivalent_full_bytes = envelope_prefix_length + len(full_location)
    if b"---\n\n" in raw:
        equivalent_full_bytes += len(b"---\n\n")
    return {
        "assignment_id": expected_id,
        "stdin_bytes": len(raw),
        "equivalent_full_bytes": equivalent_full_bytes,
        "stdout_bytes": len(stdout),
        "stderr_bytes": len(stderr),
        "location_bytes": len(location_bytes),
        "selected_output_sha256": summary_worker.get("selected_output_sha256"),
        "selected_output_path": summary_worker.get("selected_output_path"),
    }


def verify_run(run_root: Path, expected_fixture: str) -> dict[str, Any]:
    metadata_path = run_root / "compact-capture.json"
    metadata = load_json(metadata_path, "compact capture metadata")
    require(metadata.get("schema_version") == 1, f"{run_root} has an unsupported compact capture schema")
    require(metadata.get("fixture") == expected_fixture, f"{run_root} fixture label does not match {expected_fixture}")
    setup_report = load_json(Path(metadata["setup_report"]), "setup report")
    profile_path = Path(metadata["setup_profile"])
    profile = load_json(profile_path, "generated setup profile")
    require(setup_report.get("status") == "ready" and setup_report.get("started") is False, "setup was not inert and ready")
    require(profile_path.read_text(encoding="utf-8") == setup_report.get("output_bytes"), "setup profile bytes differ from setup report")
    require(sha256(profile_path) == setup_report.get("output_sha256") == metadata.get("setup_profile_sha256"), "setup/profile digest evidence disagrees")
    require(profile.get("config_version") == "high-rigor-11", "compact capture did not retain high-rigor-11")
    require(profile.get("criterion_policy") == {"required_authors": 2, "goal_required_authors": 2}, "compact capture changed criterion/goal floors")
    for gate in ("intent-review", "intent-adversarial-review"):
        axes = profile.get("review_policies", {}).get(gate, [])
        ids = {entry.get("id") for entry in axes if isinstance(entry, dict)}
        require({"acceptance-granularity", "owner-comprehensible"}.issubset(ids), f"compact capture omitted shipped {gate} intent questions")
        require(all(entry.get("review_stage", "aggregate") == "aggregate" for entry in axes if entry.get("id") in {"acceptance-granularity", "owner-comprehensible"}), f"compact capture changed {gate} intent-question stage")
    contract = metadata.get("profile_contract")
    require(isinstance(contract, dict) and contract.get("config_version") == "high-rigor-11", "compact capture omitted profile contract snapshot")
    draft_worker = load_json(Path(metadata["draft_worker"]), "draft-worker JSON")
    require(set(draft_worker) == {"command", "args"}, "draft-worker JSON is not the closed command/args shape")
    require(profile.get("work_slot_bindings", {}).get("intent-draft") == draft_worker, "generated profile did not retain draft-worker input")
    review_binding = profile.get("work_slot_bindings", {}).get("intent-review")
    require(isinstance(review_binding, dict), "generated profile omitted intent-review binding")
    final_binding = metadata.get("binding")
    require(isinstance(final_binding, dict), "capture metadata omitted the final bound worker binding")
    declarations = worker_declarations(final_binding)

    start_response = load_json(Path(metadata["start"]), "start response")
    require(start_response.get("status") == "completed", "public start did not complete")
    start_input = load_json(Path(metadata["start_input"]), "start input")
    require(start_input.get("work_slot_bindings") == profile.get("work_slot_bindings"), "start changed constructor bindings")
    artifact_root = start_input.get("artifact_root")
    require(isinstance(artifact_root, str) and artifact_root, "start input omitted artifact_root")

    complete = load_json(Path(metadata["show_full_complete"]), "completed full show")
    require(complete.get("status") == "completed", "completed full show failed")
    result = complete.get("result", {})
    invocations = result.get("work_slot_invocations", [])
    invocation_id = metadata.get("invocation_id")
    invocation = next((item for item in invocations if item.get("invocation_id") == invocation_id), None)
    require(isinstance(invocation, dict), "full show omitted the retained invocation")
    require(invocation.get("capture_dir") == metadata.get("capture_dir"), "full show capture locator disagrees")
    require(invocation.get("routed_inputs") == metadata.get("routed_inputs"), "full show routed context disagrees with retained metadata")

    capture = Path(metadata["capture_dir"])
    spec = load_json(Path(metadata["fan_out_spec"]), "fan-out spec")
    summary = load_json(Path(metadata["summary"]), "fan-out summary")
    require(spec.get("capture_format") == "bound-context-projection-v1", "compact fan-out capture format is missing")
    spec_workers = spec.get("workers")
    summary_workers = summary.get("workers")
    require(isinstance(spec_workers, list) and isinstance(summary_workers, list), "fan-out receipts omitted workers")
    require(len(spec_workers) == len(summary_workers) == len(declarations), "assignment count/order changed between setup and capture")
    for index, (spec_worker, declaration, summary_worker) in enumerate(
        zip(spec_workers, declarations, summary_workers)
    ):
        require(
            spec_worker.get("command") == declaration.get("command")
            and spec_worker.get("args") == declaration.get("args"),
            f"worker {index} command/args changed between setup and capture",
        )
        require(
            summary_worker.get("command") == spec_worker.get("command")
            and summary_worker.get("args") == spec_worker.get("args"),
            f"worker {index} command/args changed between routing and summary",
        )
    full_context = metadata.get("routed_inputs")
    require(isinstance(full_context, list), "metadata omitted full routed context")
    positive = expected_fixture != "negative-empty"
    worker_results = [
        verify_worker(
            capture=capture,
            worker_index=index,
            spec_worker=spec_worker,
            summary_worker=summary_worker,
            full_context=full_context,
            artifact_root=artifact_root,
            positive=positive,
        )
        for index, (spec_worker, summary_worker) in enumerate(zip(spec_workers, summary_workers))
    ]
    inner = invocation.get("inner_workers", [])
    if isinstance(inner, list) and inner:
        require([item.get("assignment_id") for item in inner] == [f"worker-{i}" for i in range(len(inner))], "show inner worker assignment order changed")

    if positive:
        require(invocation.get("status") == "succeeded", "positive compact delivery did not complete successfully")
        require(all(row["stdout_bytes"] > 0 for row in worker_results), "positive compact delivery has empty actual worker output")
        origin_count = sum(
            isinstance(record, dict)
            and isinstance(record.get("data"), dict)
            and "loop_engine_origin" in record["data"]
            for record in full_context
        )
        if origin_count:
            require(
                all(row["stdin_bytes"] < row["equivalent_full_bytes"] for row in worker_results),
                "supported compact delivery was not smaller than the equivalent direct full packet",
            )
        if expected_fixture == "review":
            require(any(isinstance(record.get("data"), dict) and "loop_engine_origin" in record["data"] for record in full_context if isinstance(record, dict)), "review fixture did not retain engine-owned routed evidence")
    else:
        require(invocation.get("status") == "failed", "negative compact delivery did not fail closed")
        require(metadata.get("accepted_artifacts_before") == metadata.get("accepted_artifacts_after"), "negative compact delivery changed accepted artifacts")
    verification = load_json(Path(metadata["verification"]), "verification snapshot")
    require(verification.get("invocation_id") == invocation_id, "verification snapshot points at another invocation")
    require(verification.get("selected_outputs") == [
        {
            "assignment_id": row.get("assignment_id"),
            "selected_attempt": summary_workers[index].get("selected_attempt"),
            "selected_output_sha256": row.get("selected_output_sha256"),
            "selected_output_path": row.get("selected_output_path"),
            "attempts_path": summary_workers[index].get("attempts_path"),
        }
        for index, row in enumerate(worker_results)
    ], "verification selected-output snapshot disagrees with summary")
    return {
        "fixture": expected_fixture,
        "run_id": metadata.get("run_id"),
        "invocation_id": invocation_id,
        "capture_dir": str(capture),
        "workers": len(worker_results),
        "positive": positive,
        "routed_records": len(full_context),
        "engine_origin_records": sum(
            isinstance(record, dict)
            and isinstance(record.get("data"), dict)
            and "loop_engine_origin" in record["data"]
            for record in full_context
        ),
    }


def self_test() -> None:
    source = [{"id": "x", "kind": "evidence", "data": {"loop_engine_origin": {"x": 1}, "nested": {"loop_engine_origin": "keep"}}}]
    assert projected_context(source) == [{"id": "x", "kind": "evidence", "data": {"nested": {"loop_engine_origin": "keep"}}}]
    assert source[0]["data"]["loop_engine_origin"] == {"x": 1}
    print("compact worker capture checker self-test passed")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path)
    parser.add_argument("--draft", type=Path)
    parser.add_argument("--review", type=Path)
    parser.add_argument("--negative", type=Path)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)
    try:
        if args.self_test:
            self_test()
            return 0
        for name, path in (("root", args.root), ("draft", args.draft), ("review", args.review), ("negative", args.negative)):
            if path is None:
                raise CaptureFailure(f"--{name} is required")
        results = {
            "draft": verify_run(args.draft, "draft"),
            "review": verify_run(args.review, "review"),
            "negative-empty": verify_run(args.negative, "negative-empty"),
        }
        print(json.dumps({"status": "passed", "root": str(args.root), "fixtures": results}, indent=2))
        return 0
    except CaptureFailure as error:
        print(f"compact capture check failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
