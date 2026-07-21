#!/usr/bin/env python3
"""Generic semantic judge v1 executable adapter (pi / openai-codex)."""

from __future__ import annotations

import json
import os
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parent
CONFIG_PATH = ROOT / "config.json"
VALID_VERDICTS = {"pass", "fail", "indeterminate", "unavailable"}
DETERMINATE_VERDICTS = {"pass", "fail"}


def emit_response(payload: dict[str, Any]) -> None:
    sys.stdout.write(json.dumps(payload, ensure_ascii=False))
    sys.stdout.write("\n")
    sys.stdout.flush()


@dataclass(frozen=True)
class RevisionBinding:
    parent_revision: str | None = None
    candidate_revision: str | None = None

    @classmethod
    def unbound(cls) -> RevisionBinding:
        return cls()

    @classmethod
    def from_request(cls, request: dict[str, Any]) -> RevisionBinding:
        parent = request.get("parent_revision")
        candidate = request.get("candidate_revision")
        parent_revision = parent if isinstance(parent, str) and parent else None
        candidate_revision = candidate if isinstance(candidate, str) and candidate else None
        return cls(parent_revision, candidate_revision)

    @property
    def is_complete(self) -> bool:
        return self.parent_revision is not None and self.candidate_revision is not None

    def as_unavailable_pair(self) -> tuple[str | None, str | None]:
        if self.is_complete:
            return self.parent_revision, self.candidate_revision
        return None, None


def unavailable(message: str, *, binding: RevisionBinding) -> None:
    parent_revision, candidate_revision = binding.as_unavailable_pair()
    payload: dict[str, Any] = {
        "schema_version": 1,
        "verdict": "unavailable",
        "citations": [],
        "message": message,
        "parent_revision": parent_revision,
        "candidate_revision": candidate_revision,
    }
    emit_response(payload)
    raise SystemExit(0)


def load_config(binding: RevisionBinding) -> dict[str, Any]:
    try:
        return json.loads(CONFIG_PATH.read_text(encoding="utf-8"))
    except OSError as exc:
        unavailable(f"cannot read config: {exc}", binding=binding)
    except json.JSONDecodeError as exc:
        unavailable(f"invalid config json: {exc}", binding=binding)
    raise AssertionError("unreachable")


def validate_request(request: dict[str, Any], binding: RevisionBinding) -> None:
    def reject(message: str) -> None:
        unavailable(message, binding=binding)

    required = {
        "schema_version",
        "mode",
        "parent_revision",
        "candidate_revision",
        "diff",
        "rubrics",
        "deterministic_evidence",
    }
    allowed = required | {"relevant_docs", "timeout_seconds"}
    missing = sorted(required - set(request))
    extra = sorted(set(request) - allowed)
    if missing or extra:
        reject(f"request fields mismatch: missing={missing} extra={extra}")

    schema_version = request["schema_version"]
    if isinstance(schema_version, bool) or not isinstance(schema_version, int):
        reject("request schema_version must be integer")
    if schema_version != 1:
        reject("unsupported request schema_version")
    if request["mode"] not in {"local", "publication"}:
        reject("mode must be local or publication")
    for key in ("parent_revision", "candidate_revision"):
        if not isinstance(request[key], str) or not request[key]:
            reject(f"{key} must be a non-empty string")
    if not isinstance(request["diff"], str):
        reject("diff must be a string")

    if "timeout_seconds" in request:
        timeout = request["timeout_seconds"]
        if isinstance(timeout, bool) or not isinstance(timeout, int) or timeout < 1:
            reject("timeout_seconds must be an integer >= 1")

    relevant_docs = request.get("relevant_docs", [])
    if not isinstance(relevant_docs, list):
        reject("relevant_docs must be an array")
    for doc in relevant_docs:
        if not isinstance(doc, dict) or set(doc) != {"path", "content"}:
            reject("relevant_doc must contain only path and content")
        if not isinstance(doc["path"], str) or not doc["path"]:
            reject("relevant_doc path must be a non-empty string")
        if not isinstance(doc["content"], str):
            reject("relevant_doc content must be a string")

    rubrics = request["rubrics"]
    if not isinstance(rubrics, list) or not rubrics:
        reject("rubrics must be a non-empty array")
    for rubric in rubrics:
        if not isinstance(rubric, dict) or set(rubric) != {"id", "content"}:
            reject("rubric must contain only id and content")
        if not isinstance(rubric["id"], str) or not rubric["id"]:
            reject("rubric id must be a non-empty string")
        if not isinstance(rubric["content"], str) or not rubric["content"]:
            reject("rubric content must be a non-empty string")

    evidence_items = request["deterministic_evidence"]
    if not isinstance(evidence_items, list):
        reject("deterministic_evidence must be an array")
    evidence_required = {"command", "exit_code", "stdout", "stderr"}
    evidence_allowed = evidence_required | {"candidate_revision"}
    for item in evidence_items:
        if not isinstance(item, dict):
            reject("deterministic evidence item must be object")
        if set(item) - evidence_allowed or evidence_required - set(item):
            reject("deterministic evidence item has invalid fields")
        if not isinstance(item["command"], str) or not item["command"]:
            reject("deterministic evidence command must be a non-empty string")
        exit_code = item["exit_code"]
        if isinstance(exit_code, bool) or not isinstance(exit_code, int):
            reject("deterministic evidence exit_code must be integer")
        for key in ("stdout", "stderr"):
            if not isinstance(item[key], str):
                reject(f"deterministic evidence {key} must be a string")
        if "candidate_revision" in item and (
            not isinstance(item["candidate_revision"], str)
            or not item["candidate_revision"]
        ):
            reject("deterministic evidence candidate_revision must be non-empty string")

    migration_rubrics = [
        rubric for rubric in rubrics
        if rubric["id"] == "publication-checkpoint-migration"
    ]
    if migration_rubrics:
        foundation = "7552af5968b4a2c10aefd01fbfa6c351817e1b8b"
        bindings = [
            item for item in evidence_items
            if item["command"].startswith("full publication diff sha256 ")
        ]
        if (
            len(migration_rubrics) != 1
            or request["mode"] != "publication"
            or request["parent_revision"] != foundation
            or relevant_docs
            or len(bindings) != 1
        ):
            reject("migration projection scope or complete-range binding is invalid")
        binding = bindings[0]
        fields = dict(
            token.split("=", 1)
            for token in binding["stdout"].split()
            if "=" in token
        )
        digest = fields.get("sha256", "")
        if (
            binding["exit_code"] != 0
            or len(digest) != 64
            or any(char not in "0123456789abcdef" for char in digest)
            or not fields.get("bytes", "").isdigit()
            or not fields.get("changed_paths", "").isdigit()
            or fields.get("semantic_projection_base")
            != "30f210d2a064c679c44f7880b67958fc23efe21e"
        ):
            reject("migration complete-range binding payload is invalid")


def validate_response(payload: dict[str, Any]) -> str | None:
    response_fields = {
        "schema_version",
        "parent_revision",
        "candidate_revision",
        "verdict",
        "citations",
        "message",
    }
    if set(payload) != response_fields:
        missing = sorted(response_fields - set(payload))
        extra = sorted(set(payload) - response_fields)
        return f"response fields mismatch: missing={missing} extra={extra}"

    schema_version = payload["schema_version"]
    if isinstance(schema_version, bool) or not isinstance(schema_version, int):
        return "response schema_version must be integer"
    if schema_version != 1:
        return "unsupported response schema_version"

    verdict = payload["verdict"]
    if not isinstance(verdict, str) or verdict not in VALID_VERDICTS:
        return "invalid verdict"

    parent_revision = payload["parent_revision"]
    candidate_revision = payload["candidate_revision"]
    if parent_revision is None or candidate_revision is None:
        if parent_revision is not None or candidate_revision is not None:
            return "parent_revision and candidate_revision must both be present or both null"
        if verdict != "unavailable":
            return "only unavailable may use null revision binding"
    else:
        if not isinstance(parent_revision, str) or not parent_revision:
            return "parent_revision must be a non-empty string"
        if not isinstance(candidate_revision, str) or not candidate_revision:
            return "candidate_revision must be a non-empty string"

    message = payload["message"]
    if not isinstance(message, str) or not message:
        return "message must be a non-empty string"

    citations = payload["citations"]
    if not isinstance(citations, list):
        return "citations must be an array"
    if verdict == "unavailable":
        return None if not citations else "unavailable verdict must have empty citations"
    if not citations:
        return "pass/fail/indeterminate require at least one citation"

    citation_fields = {"rubric_id", "rule", "lines"}
    for citation in citations:
        if not isinstance(citation, dict):
            return "citation must be object"
        if set(citation) != citation_fields:
            missing = sorted(citation_fields - set(citation))
            extra = sorted(set(citation) - citation_fields)
            return f"citation fields mismatch: missing={missing} extra={extra}"
        for key in ("rubric_id", "rule"):
            if not isinstance(citation[key], str) or not citation[key]:
                return f"citation {key} must be a non-empty string"
        lines = citation["lines"]
        if not isinstance(lines, list) or not lines:
            return "citation lines must be non-empty array"
        if any(not isinstance(line, str) or not line for line in lines):
            return "citation lines entries must be non-empty strings"
    return None


def extract_json_object(text: str) -> dict[str, Any] | None:
    text = text.strip()
    if not text:
        return None
    try:
        payload = json.loads(text)
    except json.JSONDecodeError:
        return None
    return payload if isinstance(payload, dict) else None


def validate_response_against_request(
    payload: dict[str, Any], request: dict[str, Any]
) -> str | None:
    if payload["verdict"] == "unavailable":
        return None

    rubrics = {rubric["id"]: rubric["content"] for rubric in request["rubrics"]}
    resulting_docs = {
        doc["path"]: doc["content"] for doc in request.get("relevant_docs", [])
    }

    for citation in payload["citations"]:
        rubric_id = citation["rubric_id"]
        rubric_content = rubrics.get(rubric_id)
        if rubric_content is None:
            return f"citation references unknown parent rubric_id: {rubric_id}"
        headings = {
            line.lstrip("#").strip().casefold()
            for line in rubric_content.splitlines()
            if line.startswith("#") and line.lstrip("#").strip()
        }
        identifiers = {
            token.casefold()
            for token in "".join(
                ch if ch.isalnum() or ch in {"_", "-"} else " "
                for ch in rubric_content
            ).split()
            if token[0].isupper() and any(ch.isdigit() for ch in token)
        }
        rule = citation["rule"].strip().casefold()
        if rule not in headings and rule not in identifiers:
            return f"citation rule is not an exact parent-rubric heading or identifier: {citation['rule']}"
        changed_paths: set[str] = set(resulting_docs)
        for diff_line in request["diff"].splitlines():
            if diff_line.startswith("diff --git a/") and " b/" in diff_line:
                left, right = diff_line.split(" b/", 1)
                changed_paths.add(left.removeprefix("diff --git a/"))
                changed_paths.add(right)
        for location in citation["lines"]:
            path, separator, suffix = location.rpartition(":")
            line_number: int | None = None
            if separator and suffix.isdigit() and path:
                line_number = int(suffix)
                if line_number == 0:
                    return f"citation line number must be positive: {location}"
            else:
                path = location
            if path.startswith("/") or any(part == ".." for part in Path(path).parts):
                return f"citation line is not a repository-relative path: {location}"
            if path not in changed_paths:
                return f"citation does not name a changed/resulting path: {location}"
            content = resulting_docs.get(path)
            if line_number is not None:
                if content is None:
                    return f"numbered citation requires resulting-document content: {location}"
                line_count = len(content.splitlines())
                if line_number > line_count:
                    return f"citation line exceeds resulting document length: {location}"
    return None


def extract_pi_assistant_text(output: str) -> str | None:
    stripped = output.strip()
    if not stripped:
        return None
    if stripped.startswith("{"):
        return stripped
    last_text: str | None = None
    for line in output.splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        event_type = event.get("type")
        if event_type in {"message_end", "turn_end", "agent_end"}:
            message = event.get("message") or event.get("assistantMessage")
            if not isinstance(message, dict):
                continue
            if message.get("role") != "assistant":
                continue
            for part in message.get("content", []):
                if part.get("type") == "text" and part.get("text"):
                    last_text = part["text"]
    return last_text


def build_prompt(request: dict[str, Any]) -> str:
    rubric_blocks = []
    for rubric in request["rubrics"]:
        rubric_id = rubric.get("id", "unknown")
        content = rubric.get("content", "")
        rubric_blocks.append(f"### Rubric {rubric_id}\n\n{content}")

    evidence_blocks = []
    for item in request["deterministic_evidence"]:
        command = item.get("command", "<unknown>")
        exit_code = item.get("exit_code")
        stdout = item.get("stdout", "")
        stderr = item.get("stderr", "")
        evidence_blocks.append(
            f"- command: {command}\n  exit_code: {exit_code}\n  stdout: {stdout!r}\n  stderr: {stderr!r}"
        )

    docs_blocks = []
    for doc in request.get("relevant_docs", []):
        path = doc.get("path", "<unknown>")
        content = doc.get("content", "")
        docs_blocks.append(f"#### {path}\n\n{content}")

    migration_projection = any(
        rubric.get("id") == "publication-checkpoint-migration"
        for rubric in request["rubrics"]
    )
    semantic_scope = (
        "the exact owner-authorized governance-repair delta supplied below; "
        "use full foundation-range digest evidence only as boundary binding"
        if migration_projection
        else "the exact parent-to-candidate diff and resulting relevant docs"
    )
    diff_heading = (
        "Owner-authorized exact governance-repair delta"
        if migration_projection
        else "Exact diff"
    )
    example_rubric_id = request["rubrics"][0]["id"]
    example_content = request["rubrics"][0]["content"]
    example_identifiers = [
        token
        for token in "".join(
            ch if ch.isalnum() or ch in {"_", "-"} else " "
            for ch in example_content
        ).split()
        if token[0].isupper() and any(ch.isdigit() for ch in token)
    ]
    example_headings = [
        line.lstrip("#").strip()
        for line in example_content.splitlines()
        if line.startswith("#") and line.lstrip("#").strip()
    ]
    example_rule = next(iter(example_identifiers or example_headings), "rule")
    changed_paths = []
    for diff_line in request["diff"].splitlines():
        if diff_line.startswith("diff --git a/") and " b/" in diff_line:
            left, right = diff_line.split(" b/", 1)
            changed_paths.extend([left.removeprefix("diff --git a/"), right])
    example_path = (
        request.get("relevant_docs", [{}])[0].get("path")
        if request.get("relevant_docs")
        else None
    ) or next(iter(dict.fromkeys(changed_paths)), "path")
    valid_rubric_ids = ", ".join(rubric["id"] for rubric in request["rubrics"])
    response_example = json.dumps(
        {
            "schema_version": 1,
            "parent_revision": request["parent_revision"],
            "candidate_revision": request["candidate_revision"],
            "verdict": "pass|fail|indeterminate",
            "citations": [
                {
                    "rubric_id": example_rubric_id,
                    "rule": example_rule,
                    "lines": [example_path],
                }
            ],
            "message": "...",
        },
        ensure_ascii=False,
        separators=(",", ":"),
    )

    return f"""You are a documentation semantic judge for loop-engine.

Evaluate whether the candidate revision is documentation-coherent under the parent revision's rubric.

Rules:
- Use ONLY the parent rubric text provided below.
- Judge ONLY {semantic_scope}.
- Use deterministic evidence for build/test/check claims; never invent CI or test results.
- Emit exactly one JSON object and no other text.
- verdict must be one of: pass, fail, indeterminate.
- pass/fail/indeterminate require at least one citation with rubric_id, rule, and non-empty lines array.
- Valid rubric_id values are exactly: {valid_rubric_ids}. rubric_id names the rubric container (for example `{example_rubric_id}`), never a rule identifier such as `{example_rule}`.
- rule must be an exact identifier or heading present in the cited rubric.
- lines entries must use unnumbered changed/resulting repository paths from the supplied semantic diff or resulting-doc snapshots. Do not append :line numbers.
- If evidence is insufficient, use indeterminate.

Request mode: {request['mode']}
Parent revision: {request['parent_revision']}
Candidate revision: {request['candidate_revision']}

## Parent rubrics

{chr(10).join(rubric_blocks)}

## Deterministic evidence

{chr(10).join(evidence_blocks) if evidence_blocks else '(none)'}

## Relevant resulting docs

{chr(10).join(docs_blocks) if docs_blocks else '(none)'}

## {diff_heading}

```diff
{request['diff']}
```

Respond with JSON only using this shape:
{response_example}

parent_revision and candidate_revision in the response must exactly match the request values above.
"""


def invoke_pi(
    prompt: str,
    config: dict[str, Any],
    timeout_seconds: float,
    *,
    binding: RevisionBinding,
) -> str:
    pi_executable = os.environ.get("LOOP_ENGINE_SEMANTIC_JUDGE_PI", config.get("pi_executable", "pi"))
    provider = config.get("provider", "openai-codex")
    model = config.get("model", "gpt-5.6-sol")
    cmd = [
        pi_executable,
        "--provider",
        provider,
        "--model",
        model,
        "--print",
        "--no-session",
        "--no-tools",
    ]
    try:
        completed = subprocess.run(
            cmd,
            input=prompt,
            check=False,
            capture_output=True,
            text=True,
            timeout=timeout_seconds,
            env=os.environ.copy(),
        )
    except FileNotFoundError:
        unavailable(
            f"pi executable not found: {pi_executable}",
            binding=binding,
        )
    except subprocess.TimeoutExpired:
        unavailable(
            f"judge timeout after {timeout_seconds}s",
            binding=binding,
        )
    if completed.returncode != 0:
        stderr = (completed.stderr or "").strip()
        unavailable(
            f"pi exited {completed.returncode}: {stderr or 'no stderr'}",
            binding=binding,
        )
    text = extract_pi_assistant_text(completed.stdout)
    return text if text is not None else completed.stdout


def judge(request: dict[str, Any], binding: RevisionBinding) -> dict[str, Any]:
    validate_request(request, binding)
    parent_revision = request["parent_revision"]
    candidate_revision = request["candidate_revision"]
    config = load_config(binding)
    timeout_seconds = int(request.get("timeout_seconds") or config.get("timeout_seconds") or 300)
    deadline = time.monotonic() + timeout_seconds

    def remaining_budget() -> float:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            unavailable(
                f"judge timeout after {timeout_seconds}s",
                binding=binding,
            )
        return remaining

    prompt = build_prompt(request)
    raw = invoke_pi(
        prompt,
        config,
        remaining_budget(),
        binding=binding,
    )

    for attempt in range(2):
        payload = extract_json_object(raw)
        failure = "judge returned non-json response" if payload is None else None
        if payload is not None:
            error = validate_response(payload)
            if error:
                failure = f"invalid judge response: {error}"
            else:
                semantic_error = validate_response_against_request(payload, request)
                if semantic_error:
                    failure = f"invalid judge citations: {semantic_error}"
                elif (
                    payload.get("parent_revision") != parent_revision
                    or payload.get("candidate_revision") != candidate_revision
                ):
                    failure = "response revision binding mismatch"
                else:
                    return payload

        assert failure is not None
        if attempt == 1:
            unavailable(failure, binding=binding)
        correction_prompt = f"""{prompt}

Your previous response failed validation and was rejected fail-closed. Return one corrected JSON object. Preserve exact revision bindings. Use one of the explicitly listed rubric_id values, an exact rule from that rubric, and unnumbered changed/resulting paths only. Do not repeat prose or any prior response text.
"""
        raw = invoke_pi(
            correction_prompt,
            config,
            remaining_budget(),
            binding=binding,
        )

    raise AssertionError("unreachable")


def cmd_validate_fixtures() -> int:
    paths = sorted((ROOT / "fixtures").glob("response-*.v1.json"))
    ok = True
    for path in paths:
        payload = json.loads(path.read_text(encoding="utf-8"))
        error = validate_response(payload)
        if error:
            print(f"INVALID {path.name}: {error}", file=sys.stderr)
            ok = False
        else:
            print(f"OK {path.name}: {payload['verdict']}")
    return 0 if ok else 1


def main() -> int:
    if len(sys.argv) > 1 and sys.argv[1] == "--validate-fixtures":
        return cmd_validate_fixtures()

    if sys.stdin.isatty():
        unavailable(
            "expected one JSON request on stdin",
            binding=RevisionBinding.unbound(),
        )
    try:
        request = json.load(sys.stdin)
    except json.JSONDecodeError as exc:
        unavailable(
            f"invalid request json: {exc}",
            binding=RevisionBinding.unbound(),
        )
    if not isinstance(request, dict):
        unavailable(
            "request must be a JSON object",
            binding=RevisionBinding.unbound(),
        )
    binding = RevisionBinding.from_request(request)
    try:
        response = judge(request, binding)
    except SystemExit:
        raise
    except Exception as exc:  # noqa: BLE001 - adapter boundary
        unavailable(
            f"judge adapter error: {exc}",
            binding=binding,
        )
    emit_response(response)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
